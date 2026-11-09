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
    FocusFinish,
    FocusDistraction,
    FocusReview(String, Option<u8>),
}
impl Command {
    pub(super) fn effects(&self, app: &AppView) -> Effects {
        Effects {
            restore_focus: matches!(app.page.as_str(), "listening" | "video")
                && matches!(
                    self,
                    Self::Timer(..) | Self::TimerPause | Self::TimerReset | Self::FocusFinish
                ),
            ..Effects::default()
        }
    }
    pub(super) fn execute(
        self,
        app: &mut AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Result<()> {
        let now = focus_timer::now_ms();
        match self {
            Command::Timer(seconds) => app.timer.set_goal(&app.lib, seconds)?,
            Command::FocusStart(seconds) => app.start_focus_mode(Some(seconds), window, cx)?,
            Command::FocusResume => app.start_focus_mode(None, window, cx)?,
            Command::FocusEmergency => app.emergency_focus_end(window, cx)?,
            Command::FocusKeep => app.focus_mode.escape = None,
            Command::TimerPause => match app.timer.toggle(&app.lib, now)? {
                focus_timer::Toggle::Started => app.play_timer_chime(timer_chimes::Cue::Start),
                focus_timer::Toggle::Completed(event) => app.focus_phase_completed(event),
                focus_timer::Toggle::Paused | focus_timer::Toggle::Resumed => {}
            },
            Command::TimerReset => {
                app.timer.reset(&app.lib)?;
                app.timer_chimes.stop();
            }
            Command::FocusFinish => {
                if app.focus_mode.active() {
                    ensure!(
                        app.timer.goal_met(now),
                        "尚未达到目标，可用「紧急结束」解除电脑专注并暂停计时"
                    );
                    app.stop_focus_mode(window, cx);
                }
                app.timer.finish(&app.lib, now)?;
                app.play_timer_chime(timer_chimes::Cue::End);
                if app.page == "stats" {
                    app.refresh_view_data();
                }
            }
            Command::FocusDistraction => app.timer.record_distraction(&app.lib)?,
            Command::FocusReview(id, fatigue) => {
                app.timer.review(&app.lib, &id, fatigue)?;
                if fatigue.is_some() {
                    app.report_status(FeedbackKind::Info, app.focus_advice());
                }
            }
        }
        app.timer_seconds = app.timer.display_seconds(focus_timer::now_ms());
        app.timer_retry = None;
        Ok(())
    }
}
