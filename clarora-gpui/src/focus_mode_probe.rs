//! Inactive hidden native self-check. Never activates a kiosk on the user's desktop.
use super::*;
use focus_mode_ui::native;
pub(super) struct Probe {
    out: PathBuf,
    stage: usize,
    started: Instant,
    cases: Vec<Value>,
    baseline: usize,
}
impl Probe {
    pub fn new(out: &Path) -> Self {
        Self {
            out: out.into(),
            stage: 0,
            started: Instant::now(),
            cases: vec![],
            baseline: 0,
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
                self.baseline = native::options();
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
                return Ok(true);
            }
            _ => {}
        }
        Ok(false)
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
                &json!({"passed":passed,"error":error,"cases":self.cases,"inactive_hidden_window":true,"real_appkit_options":true,"actual_appview_actions":true,"physical_shortcuts_fullscreen_multimonitor_verified":false}),
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
