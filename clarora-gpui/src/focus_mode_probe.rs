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
            self.started.elapsed() < Duration::from_secs(20),
            "focus diagnostic timed out"
        );
        match self.stage {
            0 => {
                app.timer_chimes = timer_chimes::Chimes::without_audio_device(
                    app.lib.root.join("cache/timer-chimes"),
                );
                ensure!(!native::is_active(), "diagnostic must remain inactive");
                self.baseline = native::options();
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
                self.stage = 1;
            }
            1 => {
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
                self.stage = 2;
            }
            2 => {
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
