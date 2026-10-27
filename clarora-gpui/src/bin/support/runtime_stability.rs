//! Real-time native GPUI/Metal + libmpv soak. The window never takes focus.
//! Failure scenarios are recorded before implementation in the task artifacts.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{fs, path::Path, time::Duration};

pub fn run(output: &Path, results: &mut Vec<Value>, seconds: u64) -> Result<()> {
    if !cfg!(target_os = "macos") || std::env::var("CLARORA_GUI_MEMORY_CHECK").as_deref() != Ok("1")
    {
        results.push(
            json!({"workflow":"真实视频原生窗口稳定性、footprint与CPU曲线","passed":true,
            "skipped":"Requires a macOS graphical session and CLARORA_GUI_MEMORY_CHECK=1"}),
        );
        return Ok(());
    }
    ensure!(
        (32..=86400).contains(&seconds),
        "soak duration must be 32..86400 seconds"
    );
    let root = output.join("runtime-stability");
    fs::create_dir_all(&root)?;
    let ffmpeg = clarora_gpui::process::tool("ffmpeg")?;
    for (name, source) in [
        ("landscape.mp4", "testsrc2=size=1280x720:rate=30"),
        ("portrait.mp4", "testsrc2=size=360x640:rate=24"),
    ] {
        let file = root.join(name);
        clarora_gpui::process::run(
            &ffmpeg,
            &[
                "-hide_banner".into(),
                "-loglevel".into(),
                "error".into(),
                "-f".into(),
                "lavfi".into(),
                "-i".into(),
                source.into(),
                "-t".into(),
                "12".into(),
                "-c:v".into(),
                "libx264".into(),
                "-preset".into(),
                "ultrafast".into(),
                "-crf".into(),
                "28".into(),
                "-pix_fmt".into(),
                "yuv420p".into(),
                "-an".into(),
                "-y".into(),
                file.to_string_lossy().into_owned(),
            ],
            &clarora_gpui::process::cancel_token(),
            Duration::from_secs(60),
        )?;
    }
    fs::write(
        root.join("command.json"),
        serde_json::to_vec_pretty(&json!({
            "exe":std::env::current_exe()?, "seconds":seconds,
            "args":[root.to_string_lossy(),"--runtime-stability-worker",&seconds.to_string()],
            "window":"hidden native NSWindow; public Window::draw executes actual GPUI paint/Metal atlas upload; no foreground presentation",
            "audio":"libmpv null audio output", "fixtures":"FFmpeg-generated H.264, landscape 1280x720@30 and portrait 360x640@24",
            "sample":"proc_pid_rusage physical footprint; getrusage user/system CPU microseconds converted to nanoseconds; 2 second JSONL",
            "limits":{"physical_footprint":768_u64*1024*1024,"growth_after_warmup":256_u64*1024*1024},
        }))?,
    )?;
    let report_path = root.join("runtime-stability.json");
    if report_path.exists() {
        fs::remove_file(&report_path)?;
    }
    let exe = std::env::current_exe()?;
    let outcome = clarora_gpui::process::run(
        &exe,
        &[
            root.to_string_lossy().into_owned(),
            "--runtime-stability-worker".into(),
            seconds.to_string(),
        ],
        &clarora_gpui::process::cancel_token(),
        Duration::from_secs(seconds + seconds / 10 + 60),
    );
    let report: Value = serde_json::from_slice(
        &fs::read(&report_path).context("native worker left no final report")?,
    )?;
    outcome?;
    ensure!(
        report["passed"] == true,
        "native video stability failed: reason={}, seconds={}, painted={}, peak={}; evidence={}",
        report["error"],
        report["seconds"],
        report["painted"],
        report["peak_physical_footprint"],
        report_path.display()
    );
    ensure!(
        report["requested_seconds"].as_u64() == Some(seconds)
            && report["seconds"]
                .as_f64()
                .is_some_and(|elapsed| elapsed >= seconds as f64),
        "native worker did not complete requested duration; evidence={}",
        report_path.display()
    );
    // A machine asleep for part of the soak must not pass just because the
    // requested wall time elapsed. Check independent measurement continuity.
    let samples = report["samples"]
        .as_array()
        .context("missing native measurements")?;
    let mut previous = 0.;
    for sample in samples {
        let seconds = sample["seconds"].as_f64().context("invalid sample time")?;
        ensure!(
            seconds >= previous && seconds - previous <= 15.,
            "native measurements interrupted for {:.1} seconds; sleep/wake is not a completed soak",
            seconds - previous
        );
        previous = seconds;
    }
    ensure!(
        seconds as f64 - previous <= 15.,
        "missing final native measurement"
    );
    results.push(json!({"workflow":"真实视频原生窗口稳定性、footprint与CPU曲线", "passed":true, "evidence":report_path,"seconds":seconds}));
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn worker(_: &Path, _: u64) -> Result<()> {
    anyhow::bail!("native stability requires macOS")
}

#[cfg(target_os = "macos")]
pub fn worker(output: &Path, requested_seconds: u64) -> Result<()> {
    use clarora_gpui::{media::Player, render_images::RenderedImages};
    use gpui::{prelude::*, *};
    use std::{
        cell::{Cell, RefCell},
        io::Write,
        path::PathBuf,
        rc::Rc,
        sync::Arc,
        time::Instant,
    };
    fn usage() -> Result<(u64, u64)> {
        let mut info: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
        let status = unsafe {
            libc::proc_pid_rusage(
                std::process::id() as i32,
                libc::RUSAGE_INFO_V2,
                (&mut info as *mut libc::rusage_info_v2).cast(),
            )
        };
        ensure!(
            status == 0 && info.ri_phys_footprint > 0,
            "proc_pid_rusage failed: {status}"
        );
        let mut cpu: libc::rusage = unsafe { std::mem::zeroed() };
        ensure!(
            unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut cpu) } == 0,
            "getrusage failed"
        );
        let nanoseconds = (cpu.ru_utime.tv_sec as u64 + cpu.ru_stime.tv_sec as u64) * 1_000_000_000
            + (cpu.ru_utime.tv_usec as u64 + cpu.ru_stime.tv_usec as u64) * 1_000;
        Ok((info.ri_phys_footprint, nanoseconds))
    }
    const LIMIT: u64 = 768 * 1024 * 1024;
    const GROWTH_LIMIT: u64 = 256 * 1024 * 1024;
    const STAGES: [&str; 6] = [
        "playing landscape",
        "paused",
        "resized landscape",
        "video surface hidden",
        "restored landscape",
        "switched portrait",
    ];
    struct Probe {
        player: Player,
        started: Instant,
        requested: u64,
        output: PathBuf,
        stream: fs::File,
        samples: Vec<Value>,
        transitions: Vec<Value>,
        stage: Option<(u64, usize)>,
        stage_started: Instant,
        stage_ready: Option<Instant>,
        entry_revision: u64,
        pause_candidate: Option<(u64, Instant)>,
        completed_cycles: u64,
        complete: bool,
        painted: Rc<Cell<u64>>,
        phase_paints: [u64; 6],
        phase_frames: [u64; 6],
        hidden_clear: u64,
        paused_reuses: u64,
        paused_frame: Option<u64>,
        revision: u64,
        fresh_frames: u64,
        last_fresh: Instant,
        images: RenderedImages,
        frame: Option<Arc<RenderImage>>,
        footprint_peak: u64,
        warm: Option<u64>,
        initial_cpu: u64,
        previous_usage: (Duration, u64),
        last_sample: Duration,
        samples_cpu_peak: f64,
        result: Rc<RefCell<Value>>,
        error: Option<String>,
        delay_phase: bool,
        delay_injected: bool,
        late_paused_frame_requested: bool,
        late_paused_frame_revision: Option<u64>,
        late_paused_frame_observed: bool,
    }
    impl Probe {
        fn enter(&mut self, cycle: u64, phase: usize) {
            self.transitions.push(json!({
                "seconds":self.started.elapsed().as_secs_f64(),
                "cycle":cycle,"stage":STAGES[phase],"painted":self.painted.get()
            }));
            self.stage_started = Instant::now();
            self.stage_ready = None;
            self.entry_revision = self.player.snapshot().frame_revision;
            self.pause_candidate = None;
            self.paused_frame = None;
            self.last_fresh = Instant::now();
            match phase {
                0 | 5 => {
                    let name = if phase == 0 {
                        "landscape.mp4"
                    } else {
                        "portrait.mp4"
                    };
                    self.player.set_video_target(960, 540, true);
                    self.player.open(&self.output.join(name));
                    self.player.set("loop-file", "inf");
                }
                1 => self.player.set("pause", "yes"),
                2 => {
                    self.player.set_video_target(1280, 720, true);
                    self.player.set("pause", "no");
                }
                3 => self.player.set_video_target(0, 0, false),
                4 => self.player.set_video_target(640, 360, true),
                _ => unreachable!(),
            }
            self.stage = Some((cycle, phase));
        }
        fn advance(&mut self) -> Result<()> {
            if self.stage.is_none() {
                self.enter(0, 0);
            }
            let (cycle, phase) = self.stage.unwrap();
            let state = self.player.snapshot();
            ensure!(state.error.is_empty(), "libmpv: {}", state.error);
            if state.frame_revision != self.revision {
                self.revision = state.frame_revision;
                if state.frame.is_some() {
                    self.fresh_frames += 1;
                    self.phase_frames[phase] += 1;
                    self.last_fresh = Instant::now();
                }
            }
            if self.stage_ready.is_none() {
                // mpv commands and decoded-frame publication are asynchronous.
                // Start measured residence only after the requested state exists.
                let ready = match phase {
                    1 => {
                        // Deterministic native regression: seek while paused once,
                        // publishing an actual asynchronous mpv frame after pause
                        // acknowledgement, while the barrier is still settling.
                        if self.delay_phase
                            && !self.late_paused_frame_requested
                            && state.paused
                            && self.pause_candidate.is_some_and(|(_, since)| {
                                since.elapsed() >= Duration::from_millis(150)
                            })
                        {
                            self.late_paused_frame_requested = true;
                            self.late_paused_frame_revision = Some(state.frame_revision);
                            self.player.command(["seek", "1", "absolute+exact"]);
                        }
                        if self
                            .late_paused_frame_revision
                            .is_some_and(|revision| revision != state.frame_revision)
                            && state.frame.is_some()
                        {
                            self.late_paused_frame_observed = true;
                        }
                        if state.paused && state.frame.is_some() {
                            match self.pause_candidate {
                                Some((revision, since)) if revision == state.frame_revision => {
                                    if since.elapsed() >= Duration::from_millis(750) {
                                        self.paused_frame = Some(revision);
                                        true
                                    } else {
                                        false
                                    }
                                }
                                _ => {
                                    self.pause_candidate =
                                        Some((state.frame_revision, Instant::now()));
                                    false
                                }
                            }
                        } else {
                            self.pause_candidate = None;
                            false
                        }
                    }
                    3 => state.frame.is_none(),
                    _ => {
                        let name = if phase == 5 {
                            "portrait.mp4"
                        } else {
                            "landscape.mp4"
                        };
                        !state.paused
                            && state.frame.is_some()
                            && state.frame_revision != self.entry_revision
                            && state.path == self.output.join(name).to_string_lossy()
                    }
                };
                if ready {
                    let ready_at = Instant::now();
                    self.stage_ready = Some(ready_at);
                    self.transitions.last_mut().unwrap()["ready_seconds"] =
                        json!(self.started.elapsed().as_secs_f64());
                } else {
                    ensure!(
                        self.stage_started.elapsed() < Duration::from_secs(6),
                        "player did not confirm {} within 6 seconds",
                        STAGES[phase]
                    );
                }
            }
            if self.started.elapsed().as_secs_f64() > 5. && phase != 1 && phase != 3 {
                ensure!(
                    self.last_fresh.elapsed() < Duration::from_secs(6),
                    "decoder made no fresh frames for 6 seconds in {}",
                    STAGES[phase]
                );
            }
            if let Some(ready) = self.stage_ready {
                if phase == 3 && ready.elapsed() >= Duration::from_secs(1) {
                    ensure!(
                        state.frame.is_none(),
                        "hidden video surface retained a frame"
                    );
                    self.hidden_clear += 1;
                }
                if phase == 1 {
                    ensure!(state.paused, "player resumed during paused stage");
                    ensure!(
                        self.paused_frame == Some(state.frame_revision),
                        "paused frame kept changing after confirmed stable barrier"
                    );
                    self.paused_reuses += 1;
                }
            }
            self.frame = state.frame;
            self.phase_paints[phase] += 1;
            const RESIDENCE: [u64; 6] = [8, 2, 5, 3, 8, 4];
            if self
                .stage_ready
                .is_some_and(|ready| ready.elapsed() >= Duration::from_secs(RESIDENCE[phase]))
            {
                self.transitions.last_mut().unwrap()["completed_seconds"] =
                    json!(self.started.elapsed().as_secs_f64());
                if phase == 5 {
                    self.completed_cycles += 1;
                    let required_cycles = (self.requested / 30).saturating_sub(1).max(1);
                    if self.started.elapsed() >= Duration::from_secs(self.requested)
                        && self.completed_cycles >= required_cycles
                    {
                        self.complete = true;
                    } else {
                        self.enter(cycle + 1, 0);
                    }
                } else {
                    self.enter(cycle, phase + 1);
                }
            }
            Ok(())
        }
        fn sample(&mut self, force: bool) -> Result<()> {
            let elapsed = self.started.elapsed();
            let (bytes, cpu) = usage()?;
            self.footprint_peak = self.footprint_peak.max(bytes);
            if elapsed.as_secs() >= 10 && self.warm.is_none() {
                self.warm = Some(bytes);
            }
            ensure!(
                bytes <= LIMIT,
                "footprint {} exceeds {} safety ceiling",
                bytes,
                LIMIT
            );
            if let Some(warm) = self.warm {
                ensure!(
                    self.footprint_peak.saturating_sub(warm) <= GROWTH_LIMIT,
                    "footprint grows beyond warmup allowance"
                );
            }
            if !force && elapsed.saturating_sub(self.last_sample) < Duration::from_secs(2) {
                return Ok(());
            }
            let interval = elapsed.saturating_sub(self.previous_usage.0).as_secs_f64();
            let percent = if interval > 0. {
                cpu.saturating_sub(self.previous_usage.1) as f64 / 1e9 / interval * 100.
            } else {
                0.
            };
            self.samples_cpu_peak = self.samples_cpu_peak.max(percent);
            let s = self.player.snapshot();
            let sample = json!({"seconds":elapsed.as_secs_f64(),"physical_footprint":bytes,"peak_physical_footprint":self.footprint_peak,
                "cpu_percent":percent,"cpu_seconds":cpu.saturating_sub(self.initial_cpu) as f64/1e9,
                "painted":self.painted.get(),"fresh_frames":self.fresh_frames,"stage":self.stage.map(|(_,i)|STAGES[i]),
                "retained_images":self.images.len(),"frame":[s.frame_width,s.frame_height],"position":s.position,
                "decoder":s.render_stats.decoder,"rendered_frames":s.render_stats.rendered_frames});
            writeln!(self.stream, "{}", serde_json::to_string(&sample)?)?;
            self.stream.flush()?;
            self.samples.push(sample);
            self.previous_usage = (elapsed, cpu);
            self.last_sample = elapsed;
            Ok(())
        }
        fn finish(&mut self, cx: &mut App) {
            if let Err(error) = self.sample(true) {
                self.error.get_or_insert(error.to_string());
            }
            let elapsed = self.started.elapsed().as_secs_f64();
            let stages = self.phase_paints.iter().all(|count| *count > 15)
                && [0, 2, 4, 5].iter().all(|i| self.phase_frames[*i] > 10)
                && self.hidden_clear > 10
                && self.paused_reuses > 10;
            let report = json!({"passed":self.error.is_none() && self.complete && elapsed>=self.requested as f64
                && self.painted.get()>self.requested*10 && self.fresh_frames>self.requested*5 && stages
                && (!self.delay_phase || (self.delay_injected && self.late_paused_frame_observed)),
                "error":self.error,"requested_seconds":self.requested,"seconds":elapsed,
                "completed_cycles":self.completed_cycles,
                "required_cycles":(self.requested / 30).saturating_sub(1).max(1),
                "phase_clock":"residence begins after player confirmation; pause requires an unchanged frame for 750 ms",
                "injected_driver_delay":self.delay_injected,
                "injected_late_paused_frame":self.late_paused_frame_requested,
                "observed_late_paused_frame":self.late_paused_frame_observed,
                "painted":self.painted.get(),"fresh_frames":self.fresh_frames,
                "stage_paints":self.phase_paints,"stage_fresh_frames":self.phase_frames,
                "stages":STAGES,"hidden_empty_checks":self.hidden_clear,"paused_frame_reuses":self.paused_reuses,
                "peak_physical_footprint":self.footprint_peak,"warmup_physical_footprint":self.warm,
                "growth_after_warmup":self.footprint_peak.saturating_sub(self.warm.unwrap_or(self.footprint_peak)),
                "mean_cpu_percent":self.samples.last().and_then(|s|s["cpu_seconds"].as_f64()).unwrap_or(0.)/elapsed*100.,
                "peak_sample_cpu_percent":self.samples_cpu_peak,"samples":self.samples,"transitions":self.transitions,
                "driver":"hidden native NSWindow; public Window::draw and actual Metal texture atlas uploads; no foreground presentation",
                "limits":{"physical_footprint":LIMIT,"growth_after_warmup":GROWTH_LIMIT},
                "not_verified":["foreground present/fullscreen mouse and keyboard","sleep/wake","system sound device","PDF/EPUB interaction","Windows","remote services"]});
            match fs::write(
                self.output.join("runtime-stability.json"),
                serde_json::to_vec_pretty(&report).unwrap(),
            ) {
                Ok(()) => *self.result.borrow_mut() = report,
                Err(error) => {
                    *self.result.borrow_mut() = json!({"passed":false,"error":error.to_string()})
                }
            }
            self.images.clear(cx);
            cx.quit();
        }
    }
    impl Render for Probe {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            if let Err(error) = self.advance() {
                self.error = Some(format!("{error:#}"));
            }
            let frame = self.frame.clone();
            self.images.retain(frame.clone(), window, cx);
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
        }
    }
    ensure!(
        (32..=86400).contains(&requested_seconds),
        "invalid soak duration"
    );
    fs::create_dir_all(output)?;
    let output = output.to_path_buf();
    ensure!(
        output.join("landscape.mp4").is_file() && output.join("portrait.mp4").is_file(),
        "missing real video fixtures"
    );
    let result = Rc::new(RefCell::new(Value::Null));
    let shared = result.clone();
    let report_path = output.join("runtime-stability.json");
    let (_, initial_cpu) = usage()?;
    Application::new().run(move |cx| {
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(800.), px(600.)));
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                focus: false,
                show: false,
                ..Default::default()
            },
            move |window, cx| {
                cx.new(|cx| {
                    let view = cx.entity().downgrade();
                    window
                        .spawn(cx, async move |cx| {
                            loop {
                                gpui::Timer::after(Duration::from_millis(33)).await;
                                let delay = cx
                                    .update(|_, cx| {
                                        view.update(cx, |this: &mut Probe, _| {
                                            if this.delay_phase
                                                && !this.delay_injected
                                                && this.started.elapsed().as_secs_f64() >= 7.8
                                            {
                                                this.delay_injected = true;
                                                true
                                            } else {
                                                false
                                            }
                                        })
                                        .unwrap_or(false)
                                    })
                                    .unwrap_or(false);
                                if delay {
                                    gpui::Timer::after(Duration::from_millis(2500)).await;
                                }
                                let finished = cx
                                    .update(|window, cx| {
                                        let done = view
                                            .update(cx, |this: &mut Probe, cx| {
                                                if let Err(error) = this.sample(false) {
                                                    this.error = Some(format!("{error:#}"));
                                                }
                                                if this.error.is_some() || this.complete {
                                                    this.finish(cx);
                                                    true
                                                } else {
                                                    cx.notify();
                                                    false
                                                }
                                            })
                                            .unwrap_or(true);
                                        if !done {
                                            window.refresh();
                                            window.draw(cx).clear();
                                        }
                                        done
                                    })
                                    .unwrap_or(true);
                                if finished {
                                    break;
                                }
                            }
                        })
                        .detach();
                    cx.on_release(|this: &mut Probe, cx| this.images.clear(cx))
                        .detach();
                    Probe {
                        player: Player::without_audio_device(),
                        started: Instant::now(),
                        requested: requested_seconds,
                        stream: fs::File::create(output.join("samples.jsonl")).unwrap(),
                        output,
                        samples: vec![],
                        transitions: vec![],
                        stage: None,
                        stage_started: Instant::now(),
                        stage_ready: None,
                        entry_revision: 0,
                        pause_candidate: None,
                        completed_cycles: 0,
                        complete: false,
                        painted: Rc::new(Cell::new(0)),
                        phase_paints: [0; 6],
                        phase_frames: [0; 6],
                        hidden_clear: 0,
                        paused_reuses: 0,
                        paused_frame: None,
                        revision: 0,
                        fresh_frames: 0,
                        last_fresh: Instant::now(),
                        images: Default::default(),
                        frame: None,
                        footprint_peak: 0,
                        warm: None,
                        initial_cpu,
                        previous_usage: (Duration::ZERO, initial_cpu),
                        last_sample: Duration::ZERO,
                        samples_cpu_peak: 0.,
                        result: shared,
                        error: None,
                        delay_phase: std::env::var("CLARORA_RUNTIME_DELAY_PHASE").as_deref()
                            == Ok("1"),
                        delay_injected: false,
                        late_paused_frame_requested: false,
                        late_paused_frame_revision: None,
                        late_paused_frame_observed: false,
                    }
                })
            },
        )
        .expect("open native GPUI stability window");
    });
    ensure!(
        result.borrow()["passed"] == true,
        "native video stability failed: reason={}; evidence={}",
        result.borrow()["error"],
        report_path.display()
    );
    Ok(())
}
