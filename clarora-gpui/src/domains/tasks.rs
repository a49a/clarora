//! Task commands own their action effects and execution.
use super::*;
pub(super) enum Command {
    Cancel,
    DismissTaskNotice,
    CancelModel,
    CancelAudioSubtitles,
    DismissAudioSubtitles,
    DismissModelNotice,
    FeedbackHandled,
}
impl Command {
    pub(super) fn effects(&self, _app: &AppView) -> Effects {
        Effects::default()
    }
    pub(super) fn execute(
        self,
        app: &mut AppView,
        _window: &mut Window,
        _cx: &mut Context<AppView>,
    ) -> Result<()> {
        match self {
            // Handled by the feedback guard before domain dispatch.
            Command::FeedbackHandled => {}

            Command::DismissTaskNotice => {
                if app.tasks.notice.as_ref().is_some_and(|n| !n.active()) {
                    app.tasks.notice = None;
                    app.tasks.backup_progress = None;
                }
            }

            Command::Cancel => {
                if app.tasks.foreground.is_running() {
                    app.tasks.foreground.cancel();
                    app.report_status(FeedbackKind::Progress, "正在取消…".into());
                    if let Some(notice) = app.tasks.notice.as_mut().filter(|n| n.active()) {
                        notice.state = TaskState::Cancelling;
                        notice.detail = "正在停止当前处理，请稍候…".into();
                    }
                }
            }

            Command::CancelModel => app.tasks.models.cancel(),

            Command::CancelAudioSubtitles => app.tasks.audio_subtitles.cancel(),

            Command::DismissAudioSubtitles => {
                if !app.tasks.audio_subtitles.lane.is_running() {
                    app.tasks.audio_subtitles.notice = None;
                }
            }

            Command::DismissModelNotice => {
                if !app.tasks.models.lane.is_running() {
                    app.tasks.models.notice = None;
                }
            }
        }
        Ok(())
    }
}

// Completion policy belongs to this domain: cancellation may race a durable commit.
pub(super) enum Completed {
    Message(String),
    Issue(String),
}
impl Completed {
    pub(super) fn apply(
        self,
        app: &mut AppView,
        _context: &mut Completion,
        _window: &mut Window,
        _cx: &mut Context<AppView>,
    ) {
        match self {
            Completed::Message(s) => app.report_status(FeedbackKind::Info, s),

            Completed::Issue(s) => app.report_status(FeedbackKind::Error, s),
        }
    }
}

pub(super) fn leave(app: &mut AppView) {
    app.tasks.foreground.cancel_material();
}
