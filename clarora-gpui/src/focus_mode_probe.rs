//! Inactive hidden native self-check. Never activates a kiosk on the user's desktop.
use super::*;
use focus_mode_ui::native;
use std::sync::{Arc, Mutex};
static LABEL_COLORS: std::sync::OnceLock<Mutex<Vec<Value>>> = std::sync::OnceLock::new();
fn label_colors() -> Vec<Value> {
    LABEL_COLORS
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .clone()
}
pub(super) fn label_color_probe(background: Hsla) -> Option<AnyElement> {
    if !std::env::args().any(|arg| arg == "--focus-mode-check")
        || std::env::var("CLARORA_FOCUS_DIAGNOSTIC_INACTIVE").as_deref() != Ok("1")
    {
        return None;
    }
    Some(canvas(|_, _, _| (), move |_, _, window, cx| {
        let foreground: Rgba = window.text_style().color.into();
        let background: Rgba = background.into();
        let luminance = |c: Rgba| {
            let linear = |v: f32| { let v = v as f64; if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) } };
            0.2126 * linear(c.r) + 0.7152 * linear(c.g) + 0.0722 * linear(c.b)
        };
        let a = luminance(foreground);
        let b = luminance(background);
        let row = json!({"theme":format!("{:?}",cx.theme().mode),"foreground":[foreground.r,foreground.g,foreground.b,foreground.a],"background":[background.r,background.g,background.b,background.a],"contrast":(a.max(b)+0.05)/(a.min(b)+0.05)});
        let mut values = LABEL_COLORS.get_or_init(Default::default).lock().unwrap();
        if !values.contains(&row) { values.push(row); }
    }).absolute().size_full().into_any_element())
}
type LayoutResult = Arc<Mutex<Option<std::result::Result<Vec<Value>, String>>>>;
struct FocusLayoutView {
    app: Entity<AppView>,
    result: LayoutResult,
}
impl Render for FocusLayoutView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.result.lock().unwrap().is_none() {
            let value = self
                .app
                .update(cx, |app, cx| Probe::measure_bars(app, window, cx));
            *self.result.lock().unwrap() = Some(value.map_err(|error| format!("{error:#}")));
        }
        div().size_full().child(self.app.clone())
    }
}
pub(super) struct Probe {
    out: PathBuf,
    stage: usize,
    started: Instant,
    cases: Vec<Value>,
    baseline: usize,
    layouts: LayoutResult,
    original_theme: Option<ThemeMode>,
}
impl Probe {
    pub fn new(out: &Path) -> Self {
        Self {
            out: out.into(),
            stage: 0,
            started: Instant::now(),
            cases: vec![],
            baseline: 0,
            layouts: Arc::new(Mutex::new(None)),
            original_theme: None,
        }
    }
    fn note(&mut self, name: &str, app: &AppView) -> Result<()> {
        ensure!(
            !native::is_active(),
            "diagnostic unexpectedly became active; refusing to restrict desktop"
        );
        self.cases.push(json!({"case":name,"passed":true,"presentation_options":native::options(),"active":app.focus_mode.active(),"deadline":app.timer.deadline_ms()}));
        Ok(())
    }
    fn step(
        &mut self,
        app: &mut AppView,
        w: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Result<bool> {
        ensure!(
            self.started.elapsed() < Duration::from_secs(30),
            "focus diagnostic timed out at stage {} (position={}, paused={}, sentence={:?}, video_focus={}, status={})",
            self.stage,
            app.media.player.snapshot().position,
            app.media.player.snapshot().paused,
            app.video.sentence_repeat.map(|s| s.index),
            w.focused(cx) == Some(app.focus.clone()),
            app.status
        );
        match self.stage {
            0 => {
                app.timer_chimes = timer_chimes::Chimes::without_audio_device(
                    app.lib.root.join("cache/timer-chimes"),
                );
                ensure!(!native::is_active(), "diagnostic must remain inactive");
                self.original_theme = Some(cx.theme().mode);
                Theme::change(ThemeMode::Light, None, cx);
                design::apply(cx);
                self.baseline = native::options();
                let now = focus_timer::now_ms();
                app.lib.set_setting(focus_timer::KEY, &json!({"version":1,"strict":false,"focus":false,"paused":false,"duration_ms":300000,"remaining_ms":300000,"deadline_ms":now-1}).to_string())?;
                app.timer = focus_timer::Timer::load(&app.lib, now)?.timer;
                app.timer_remaining = app.timer.remaining_seconds(now);
                ensure!(
                    app.focus_timer_toggle_label() == "开始下一轮专注" && !app.timer.is_running(),
                    "waiting confirmation is not explicit in the timer UI"
                );
                let handle = w.window_handle();
                let app = cx.entity();
                let result = self.layouts.clone();
                cx.defer(move |cx| {
                    let _ = handle.update(cx, move |_, w, cx| {
                        w.replace_root(cx, |w, cx| {
                            let view = cx.new(|_| FocusLayoutView { app, result });
                            Root::new(view, w, cx)
                        });
                        w.draw(cx).clear();
                    });
                });
                self.stage = 9;
            }
            9 => {
                let layout = self.layouts.lock().unwrap().clone();
                let Some(layout) = layout else {
                    return Ok(false);
                };
                layout.map_err(anyhow::Error::msg)?;
                self.note("header focus control and waiting-confirmation timer lay out in light/dark, normal/fullscreen and narrow/wide widths", app)?;
                app.handle(Action::TimerPause, w, cx);
                ensure!(
                    app.timer.is_running()
                        && app.timer.focus()
                        && app.timer_remaining == 1500
                        && app.focus_timer_toggle_label() == "暂停",
                    "actual confirmation action did not begin a full new focus round"
                );
                self.note("rest completion waits for actual AppView confirmation before starting the next focus", app)?;
                app.handle(Action::TimerReset, w, cx);
                ensure!(
                    !app.timer.is_running() && app.focus_timer_toggle_label() == "开始",
                    "reset did not clear waiting state"
                );
                app.media.player = Player::without_audio_device();
                video_resume_probe::prepare(&app.lib)?;
                app.handle(Action::Page("video".into()), w, cx);
                ensure!(
                    app.open_at(app.lib.root.join("media/resume-video.mp4"), 0., true),
                    "video fixture failed to open"
                );
                self.stage = 3;
            }
            3 => {
                app.media.player.set_video_target(320, 180, true);
                let state = app.media.player.snapshot();
                if state.duration < 11.9 || state.frame.is_none() || app.video.cues.len() != 3 {
                    return Ok(false);
                }
                app.handle(Action::VideoSentenceRepeat(Some(0), true), w, cx);
                app.media.player.set("pause", "yes");
                self.stage = 8;
            }
            8 => {
                if !app.media.player.snapshot().paused || !Self::at_sentence(app, 0) {
                    return Ok(false);
                }
                if !app.assistant_open {
                    app.handle(Action::Assistant, w, cx);
                }
                app.inputs["question"].update(cx, |state, cx| state.focus(w, cx));
                app.handle(Action::FocusStart(60), w, cx);
                ensure!(
                    app.focus_mode.active() && w.focused(cx) == Some(app.focus.clone()),
                    "FocusStart left video keyboard focus in the old input/menu"
                );
                Self::keys(w, cx, &["right"]);
                self.stage = 4;
            }
            4 => {
                if !Self::at_sentence(app, 1) {
                    return Ok(false);
                }
                self.note("FocusStart → actual right key changes sentence and keeps three repeats without a video click", app)?;
                Self::keys(w, cx, &["space"]);
                self.stage = 5;
            }
            5 => {
                if !app.media.player.snapshot().paused {
                    return Ok(false);
                }
                self.note(
                    "actual space key pauses video after computer focus starts",
                    app,
                )?;
                app.inputs["question"].update(cx, |state, cx| state.focus(w, cx));
                Self::keys(w, cx, &["right", "space"]);
                self.stage = 6;
            }
            6 => {
                if !app.media.player.snapshot().paused {
                    return Ok(false);
                }
                let question_focus = app.inputs["question"].read(cx).focus_handle(cx);
                ensure!(
                    w.focused(cx) == Some(question_focus.clone())
                        && app.video.sentence_repeat.is_some_and(|s| s.index == 1),
                    "focus mode intercepted editor arrows/space"
                );
                app.poll_focus_mode(w, cx);
                ensure!(
                    w.focused(cx) == Some(question_focus),
                    "poll stole editor focus"
                );
                self.note("question input keeps actual arrows/space and focus polling does not steal focus", app)?;
                app.stop_focus_mode(w, cx);
                app.handle(Action::FocusResume, w, cx);
                ensure!(
                    app.focus_mode.active() && w.focused(cx) == Some(app.focus.clone()),
                    "FocusResume did not restore video keyboard focus"
                );
                Self::keys(w, cx, &["left"]);
                self.stage = 7;
            }
            7 => {
                if !Self::at_sentence(app, 0) {
                    return Ok(false);
                }
                self.note(
                    "FocusResume → actual left key works without clicking the video",
                    app,
                )?;
                app.stop_focus_mode(w, cx);
                app.handle(Action::VideoStopSentenceRepeat, w, cx);
                app.handle(Action::Page("writing".into()), w, cx);
                app.inputs["question"].update(cx, |state, cx| state.focus(w, cx));
                let input = w.focused(cx);
                app.handle(Action::FocusStart(60), w, cx);
                ensure!(
                    app.focus_mode.active() && w.focused(cx) == input,
                    "starting focus on a non-video page stole editor focus"
                );
                self.note(
                    "starting computer focus outside video preserves editor focus",
                    app,
                )?;
                app.stop_focus_mode(w, cx);
                self.stage = 10;
            }
            10 => {
                app.handle(Action::FocusStart(60), w, cx);
                ensure!(
                    app.focus_mode.active()
                        && app.timer.strict()
                        && native::options() & native::REQUIRED == native::REQUIRED,
                    "actual native start failed: {}",
                    app.status
                );
                let deadline = app.timer.deadline_ms();
                app.handle(Action::TimerPause, w, cx);
                app.handle(Action::TimerReset, w, cx);
                app.handle(Action::Timer(5), w, cx);
                ensure!(
                    app.timer.deadline_ms() == deadline && app.timer.strict(),
                    "ordinary timer actions bypassed focus"
                );
                self.note(
                    "actual AppKit kiosk flags and guarded pause/reset/duration",
                    app,
                )?;
                ensure!(
                    !app.prepare_close(cx) && app.chat_closing.is_none(),
                    "ordinary close bypassed focus"
                );
                self.note(
                    "native close and Quit handler's shared close path is blocked",
                    app,
                )?;
                app.handle(Action::Page("writing".into()), w, cx);
                ensure!(
                    app.page == "writing" && app.focus_mode.active(),
                    "in-app learning navigation blocked or exited kiosk"
                );
                self.note("Clarora modules stay usable inside computer focus", app)?;
                app.handle(Action::FocusEmergency, w, cx);
                app.handle(Action::FocusEmergency, w, cx);
                ensure!(app.focus_mode.active(), "emergency countdown bypassed");
                self.note("emergency exit requires deliberate countdown", app)?;
                self.stage = 11;
            }
            11 => {
                if app.focus_mode.escape_remaining() != Some(0) {
                    return Ok(false);
                }
                app.handle(Action::FocusEmergency, w, cx);
                ensure!(
                    !app.focus_mode.active()
                        && !app.timer.strict()
                        && app.timer.paused()
                        && native::options() == self.baseline,
                    "emergency exit failed to restore native/timer state"
                );
                self.note(
                    "delayed emergency release restores original presentation and pauses timer",
                    app,
                )?;
                app.handle(Action::FocusStart(1), w, cx);
                ensure!(app.focus_mode.active(), "second session failed");
                self.stage = 12;
            }
            12 => {
                if app.focus_mode.active() {
                    return Ok(false);
                }
                ensure!(
                    native::options() == self.baseline,
                    "deadline did not restore presentation options"
                );
                self.note(
                    "deadline releases native restrictions before persistence/phase polling",
                    app,
                )?;
                app.lib.db()?.execute_batch("CREATE TRIGGER reject_focus BEFORE UPDATE ON app_settings WHEN NEW.key='focus_timer_v1' BEGIN SELECT RAISE(ABORT,'focus fixture failure'); END;")?;
                app.handle(Action::FocusStart(60), w, cx);
                ensure!(
                    !app.focus_mode.active() && native::options() == self.baseline,
                    "start persistence failure leaked kiosk flags"
                );
                app.lib.db()?.execute_batch("DROP TRIGGER reject_focus;")?;
                self.note(
                    "failed start restores native options and never leaves half-active session",
                    app,
                )?;
                Theme::change(ThemeMode::Dark, None, cx);
                design::apply(cx);
                Self::keys(w, cx, &[]);
                self.stage = 13;
            }
            13 => {
                let colors = label_colors();
                ensure!(
                    ["Light", "Dark"]
                        .iter()
                        .all(|mode| colors.iter().any(|r| r["theme"] == *mode)),
                    "missing painted label color evidence: {colors:?}"
                );
                ensure!(
                    colors
                        .iter()
                        .all(|r| r["contrast"].as_f64().is_some_and(|c| c >= 4.5)),
                    "painted focus label has insufficient contrast: {colors:?}"
                );
                self.note("actual painted focus label contrast is at least 4.5:1 in light and dark themes", app)?;
                if let Some(mode) = self.original_theme {
                    Theme::change(mode, None, cx);
                    design::apply(cx);
                }
                return Ok(true);
            }
            _ => {}
        }
        Ok(false)
    }
    fn measure_bars(
        app: &mut AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Result<Vec<Value>> {
        let original = cx.theme().mode;
        let fullscreen = app.video.fullscreen;
        let result = (|| {
            let mut rows = vec![];
            for mode in [ThemeMode::Light, ThemeMode::Dark] {
                Theme::change(mode, None, cx);
                design::apply(cx);
                for full in [false, true] {
                    app.video.fullscreen = full;
                    for width in [420., 760., 1240.] {
                        for (name, mut bar) in [
                            ("computer-focus", app.computer_focus_control(cx)),
                            ("timer-confirmation", app.focus_timer_bar(cx)),
                        ] {
                            let measured = bar.layout_as_root(
                                size(
                                    AvailableSpace::Definite(px(width)),
                                    AvailableSpace::MaxContent,
                                ),
                                window,
                                cx,
                            );
                            let height = f32::from(measured.height);
                            ensure!(
                                f32::from(measured.width) <= width + 1.
                                    && height >= 30.
                                    && height <= 160.,
                                "focus bar overflow: {name}, {mode:?}, fullscreen={full}, width={width}, measured={measured:?}"
                            );
                            rows.push(json!({"bar":name,"theme":format!("{mode:?}"),"fullscreen":full,"available_width":width,"width":f32::from(measured.width),"height":height}));
                        }
                    }
                }
            }
            Ok(rows)
        })();
        app.video.fullscreen = fullscreen;
        Theme::change(original, None, cx);
        design::apply(cx);
        result
    }
    fn at_sentence(app: &AppView, index: usize) -> bool {
        let state = app.media.player.snapshot();
        app.video.repeat_three
            && app.video.sentence_repeat.is_some_and(|s| {
                s.index == index
                    && state.position >= s.range.0 - 0.05
                    && state.position < s.range.0 + 1.
                    && state
                        .sentence_repeat
                        .as_ref()
                        .is_some_and(|r| r.request_id == s.request_id)
            })
    }
    fn keys(window: &Window, cx: &mut Context<AppView>, keys: &[&str]) {
        let handle = window.window_handle();
        let keys = keys
            .iter()
            .map(|key| Keystroke::parse(key).expect("probe key"))
            .collect::<Vec<_>>();
        cx.defer(move |cx| {
            let _ = handle.update(cx, move |_, w, cx| {
                w.draw(cx).clear();
                for key in keys {
                    w.dispatch_keystroke(key, cx);
                }
            });
        });
    }
    fn report(&self, passed: bool, error: Option<String>) -> Result<()> {
        fs::write(
            self.out.join("focus-mode-report.json"),
            serde_json::to_vec_pretty(
                &json!({"passed":passed,"error":error,"cases":self.cases,"top_bar_layouts":self.layouts.lock().unwrap().clone(),"focus_label_colors":label_colors(),"inactive_hidden_window":true,"real_appkit_options":true,"actual_appview_actions":true,"physical_shortcuts_fullscreen_multimonitor_verified":false}),
            )?,
        )?;
        Ok(())
    }
}
impl AppView {
    pub(super) fn poll_focus_probe(&mut self, w: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(mut probe) = self.focus_mode_probe.take() else {
            return false;
        };
        match probe.step(self, w, cx) {
            Ok(true) => {
                self.stop_focus_mode(w, cx);
                let _ = probe.report(true, None);
                w.remove_window();
                true
            }
            Err(error) => {
                self.stop_focus_mode(w, cx);
                let _ = probe.report(false, Some(format!("{error:#}")));
                w.remove_window();
                true
            }
            Ok(false) => {
                self.focus_mode_probe = Some(probe);
                false
            }
        }
    }
}
