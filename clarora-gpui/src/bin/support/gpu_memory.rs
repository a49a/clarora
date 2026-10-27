//! Real native-window/Metal regression; libmpv-only tests never upload GPU textures.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::{fs, path::Path, time::Duration};

pub fn run(output: &Path, results: &mut Vec<Value>) -> Result<()> {
    if !cfg!(target_os = "macos") || std::env::var("CLARORA_GUI_MEMORY_CHECK").as_deref() != Ok("1")
    {
        results.push(
            json!({"workflow":"GPUI窗口GPU纹理生命周期与物理footprint", "passed":true,
            "skipped":"Requires a macOS graphical session and CLARORA_GUI_MEMORY_CHECK=1"}),
        );
        return Ok(());
    }
    let root = output.join("gpu-memory");
    fs::create_dir_all(&root)?;
    let exe = std::env::current_exe()?;
    clarora_gpui::process::run(
        &exe,
        &[
            root.to_string_lossy().into_owned(),
            "--gpu-memory-worker".into(),
        ],
        &clarora_gpui::process::cancel_token(),
        Duration::from_secs(60),
    )?;
    let evidence = root.join("gpu-memory.json");
    let report: Value = serde_json::from_slice(&fs::read(&evidence)?)?;
    ensure!(report["passed"] == true, "GPU memory regression: {report}");
    results.push(json!({"workflow":"GPUI窗口GPU纹理生命周期与物理footprint", "passed":true,"evidence":evidence}));
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn worker(_: &Path, _: bool) -> Result<()> {
    anyhow::bail!("GPU footprint probe requires macOS")
}

#[cfg(target_os = "macos")]
pub fn worker(output: &Path, unbounded: bool) -> Result<()> {
    use clarora_gpui::render_images::RenderedImages;
    use gpui::{prelude::*, *};
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
        sync::Arc,
        time::Instant,
    };
    fn footprint() -> u64 {
        unsafe {
            let mut info: libc::rusage_info_v2 = std::mem::zeroed();
            let status = libc::proc_pid_rusage(
                std::process::id() as i32,
                libc::RUSAGE_INFO_V2,
                (&mut info as *mut libc::rusage_info_v2).cast(),
            );
            if status == 0 {
                info.ri_phys_footprint
            } else {
                0
            }
        }
    }
    struct Probe {
        started: Instant,
        last_sample: Duration,
        samples: Vec<Value>,
        peak: u64,
        warm: Option<u64>,
        images: RenderedImages,
        frame: Option<Arc<RenderImage>>,
        painted: Rc<Cell<u64>>,
        serial: u32,
        phase_paints: [u64; 5],
        output: std::path::PathBuf,
        result: Rc<RefCell<Value>>,
        unbounded: bool,
    }
    impl Render for Probe {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let elapsed = self.started.elapsed();
            let seconds = elapsed.as_secs_f64();
            let phase = if seconds < 6. {
                0
            } else if seconds < 8. {
                1
            } else if seconds < 11. {
                2
            } else if seconds < 14. {
                3
            } else {
                4
            };
            self.phase_paints[phase] += 1;
            let bytes = footprint();
            self.peak = self.peak.max(bytes);
            if seconds >= 2. && self.warm.is_none() {
                self.warm = Some(bytes);
            }
            if elapsed.saturating_sub(self.last_sample) >= Duration::from_millis(500) {
                self.samples.push(json!({"seconds":seconds,"physical_footprint":bytes,"painted":self.painted.get(),"retained":self.images.len()}));
                self.last_sample = elapsed;
            }
            if seconds >= 22. || bytes > 640 * 1024 * 1024 {
                self.images.retain([], window, cx);
                let growth = self.peak.saturating_sub(self.warm.unwrap_or(self.peak));
                let report = json!({"passed":seconds>=22. && self.painted.get()>350 && bytes>0
                        && self.peak<=640*1024*1024 && growth<=192*1024*1024 && self.phase_paints.iter().all(|count| *count > 10),
                    "seconds":seconds,"painted":self.painted.get(),"peak_physical_footprint":self.peak,
                    "growth_after_warmup":growth,"unbounded_negative_control":self.unbounded,"phase_paints":self.phase_paints,
                    "driver":"hidden native window; explicit Window::draw; no foreground presentation",
                    "samples":self.samples,"stages":"new frames; paused image reuse; resized; hidden; restored; duplicate image references"});
                fs::write(&self.output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
                *self.result.borrow_mut() = report;
                cx.defer(|cx| cx.quit());
                return div().into_any_element();
            }
            let hidden = (11. ..14.).contains(&seconds);
            let paused = (6. ..8.).contains(&seconds);
            if !hidden && (!paused || self.frame.is_none()) {
                self.serial += 1;
                let (w, h) = if (8. ..11.).contains(&seconds) && self.serial % 2 == 0 {
                    (1280, 720)
                } else {
                    (1024, 768)
                };
                let image = image::RgbaImage::from_pixel(
                    w,
                    h,
                    image::Rgba([self.serial as u8, 77, 120, 255]),
                );
                self.frame = Some(Arc::new(RenderImage::new(vec![image::Frame::new(image)])));
            }
            if hidden {
                self.frame = None;
            }
            let frame = self.frame.clone();
            if !self.unbounded {
                // Include a duplicate reference: retirement must be exactly once per image.
                self.images
                    .retain(frame.clone().into_iter().chain(frame.clone()), window, cx);
            }
            let painted = self.painted.clone();
            div()
                .size_full()
                .bg(rgb(0x121212))
                .when_some(frame, |view, image| {
                    view.child(img(image).size_full().object_fit(ObjectFit::Contain))
                })
                .child(
                    canvas(
                        |_, _, _| {},
                        move |_, _, _, _| painted.set(painted.get() + 1),
                    )
                    .absolute()
                    .size_full(),
                )
                .into_any_element()
        }
    }
    fs::create_dir_all(output)?;
    let result = Rc::new(RefCell::new(Value::Null));
    let shared = result.clone();
    let output = output.join("gpu-memory.json");
    Application::new().run(move |cx| {
        let bounds = Bounds::centered(None, size(px(800.), px(600.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                focus: false,
                show: false,
                ..Default::default()
            },
            move |window, cx| {
                cx.new(|cx| {
                    // macOS stops its display link when a window is occluded.
                    // The public draw API still executes real Metal texture uploads;
                    // keep the native window hidden and drive it independently.
                    let view = cx.entity().downgrade();
                    window.spawn(cx, async move |cx| {
                        loop {
                            gpui::Timer::after(Duration::from_millis(25)).await;
                            let done = cx.update(|window, cx| {
                                let finished = view.update(cx, |this: &mut Probe, cx| {
                                    let bytes = footprint();
                                    this.peak = this.peak.max(bytes);
                                    if this.started.elapsed() >= Duration::from_secs(25) || bytes > 640*1024*1024 {
                                        let report = json!({"passed":false,"reason":"native drawing stopped or exceeded memory limit",
                                            "seconds":this.started.elapsed().as_secs_f64(),"painted":this.painted.get(),
                                            "peak_physical_footprint":this.peak,"samples":this.samples,
                                            "driver":"hidden native window; explicit Window::draw; no foreground presentation"});
                                        fs::write(&this.output,serde_json::to_vec_pretty(&report).unwrap()).unwrap();
                                        *this.result.borrow_mut()=report;
                                        this.images.clear(cx);
                                        cx.defer(|cx|cx.quit());
                                        true
                                    } else {cx.notify();false}
                                }).unwrap_or(true);
                                if !finished {
                                    window.refresh();
                                    window.draw(cx).clear();
                                }
                                finished
                            }).unwrap_or(true);
                            if done {break}
                        }
                    }).detach();
                    cx.on_release(|this: &mut Probe, cx| this.images.clear(cx))
                        .detach();
                    Probe {
                        started: Instant::now(),
                        last_sample: Duration::ZERO,
                        samples: vec![],
                        peak: 0,
                        warm: None,
                        images: Default::default(),
                        frame: None,
                        painted: Rc::new(Cell::new(0)),
                        serial: 0,
                        phase_paints: [0; 5],
                        output,
                        result: shared,
                        unbounded,
                    }
                })
            },
        )
        .unwrap();
    });
    ensure!(
        result.borrow()["passed"] == true,
        "Native GPU memory probe failed: {}",
        result.borrow()
    );
    Ok(())
}
