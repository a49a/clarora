//! Focus commands own their action effects and execution.
use super::*;
pub(super) enum Command {
    Timer(u64),
    TimerPause,
    TimerReset,
    FocusStart(u64),
    FocusResume,
    FocusEmergency,
    FocusKeep,
}
impl Command {
    pub(super) fn effects(&self, app: &AppView) -> Effects {
        Effects {
            restore_focus: (app.page == "listening"
                && matches!(
                    self,
                    Command::Timer(..) | Command::TimerPause | Command::TimerReset
                ))
                || (app.page == "video"
                    && matches!(
                        self,
                        Command::Timer(..) | Command::TimerPause | Command::TimerReset
                    )),
            ..Effects::default()
        }
    }
    pub(super) fn execute(
        self,
        app: &mut AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Result<()> {
        match self {
            Command::Timer(seconds) => {
                app.timer.start(&app.lib, focus_timer::now_ms(), seconds)?;
                app.timer_remaining = app.timer.remaining_seconds(focus_timer::now_ms());
                app.timer_retry = None;
                app.play_timer_chime(timer_chimes::Cue::Start);
            }

            Command::FocusStart(seconds) => app.start_focus_mode(Some(seconds), window, cx)?,

            Command::FocusResume => app.start_focus_mode(None, window, cx)?,

            Command::FocusEmergency => app.emergency_focus_end(window, cx)?,

            Command::FocusKeep => {
                app.focus_mode.escape = None;
            }

            Command::TimerPause => {
                match app.timer.toggle(&app.lib, focus_timer::now_ms())? {
                    focus_timer::Toggle::Started => app.play_timer_chime(timer_chimes::Cue::Start),
                    focus_timer::Toggle::Completed(focused) => app.focus_phase_completed(focused),
                    focus_timer::Toggle::Paused | focus_timer::Toggle::Resumed => {}
                }
                app.timer_remaining = app.timer.remaining_seconds(focus_timer::now_ms());
                app.timer_retry = None;
            }

            Command::TimerReset => {
                app.timer.reset(&app.lib)?;
                app.timer_chimes.stop();
                app.timer_remaining = app.timer.remaining_seconds(focus_timer::now_ms());
                app.timer_retry = None;
            }
        }
        Ok(())
    }
}
