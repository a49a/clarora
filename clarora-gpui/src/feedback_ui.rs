//! Transient feedback overlays; task progress stays in its own visible task area.
use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum FeedbackKind {
    Info,
    Error,
    Progress,
}

#[derive(Clone)]
pub(super) struct Retry {
    action: Action,
    page: String,
    audio: String,
    media: Option<PathBuf>,
    document: Option<PathBuf>,
}
impl Retry {
    pub(super) fn capture(action: &Action, app: &AppView) -> Option<Self> {
        // Only repeatable reload/generation actions. Never replay imports, deletion or payment.
        matches!(
            action,
            Action::Refresh
                | Action::Transcribe
                | Action::Translate
                | Action::VideoTranslate
                | Action::VideoTranscribe
                | Action::PdfOpen(_)
        )
        .then(|| Self {
            action: action.clone(),
            page: app.page.clone(),
            audio: app.audio.current_id.clone(),
            media: app.media.path.clone(),
            document: app.pdf_path.clone(),
        })
    }
    fn matches(&self, app: &AppView) -> bool {
        self.page == app.page
            && self.audio == app.audio.current_id
            && self.media == app.media.path
            && self.document == app.pdf_path
    }
}

pub(super) struct ErrorFeedback {
    message: String,
    retry: Option<Retry>,
}

#[derive(Default)]
pub(super) struct FeedbackState {
    transient: Option<(String, Instant)>,
    error: Option<ErrorFeedback>,
    last_error: Option<String>,
    media_error: String,
    dirty: bool,
    pub(super) retry_candidate: Option<Retry>,
}
impl FeedbackState {
    pub(super) fn attach_retry(&mut self, retry: Option<Retry>) {
        if let Some(error) = &mut self.error {
            error.retry = retry;
        }
    }
}

impl AppView {
    pub(super) fn report_status(&mut self, kind: FeedbackKind, message: String) {
        self.status = message.clone();
        if message.trim().is_empty() {
            return;
        }
        match kind {
            FeedbackKind::Info => {
                self.feedback.transient = Some((message, Instant::now()));
                self.feedback.dirty = true;
            }
            FeedbackKind::Error if self.feedback.last_error.as_ref() != Some(&message) => {
                self.feedback.last_error = Some(message.clone());
                self.feedback.error = Some(ErrorFeedback {
                    message,
                    retry: None,
                });
                self.feedback.dirty = true;
            }
            FeedbackKind::Error => {}
            FeedbackKind::Progress => {
                self.tasks.progress_label = message;
            }
        }
    }

    pub(super) fn begin_feedback_action(
        &mut self,
        action: &Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match action {
            Action::DismissFeedback(error) => {
                if *error {
                    self.feedback.error = None;
                } else {
                    self.feedback.transient = None;
                }
                cx.notify();
                return true;
            }
            Action::RetryFeedback => {
                let retry = self.feedback.error.as_ref().and_then(|e| e.retry.clone());
                if let Some(retry) = retry {
                    if retry.matches(self) {
                        self.feedback.error = None;
                        self.handle(retry.action, window, cx);
                    } else {
                        self.report_status(
                            FeedbackKind::Info,
                            "材料已切换，请在当前页面重新操作".into(),
                        );
                    }
                }
                cx.notify();
                return true;
            }
            _ => {}
        }
        // A new user intent may retry the same failure; timer polling does not reopen dismissed errors.
        self.feedback.last_error = None;
        self.feedback.retry_candidate = Retry::capture(action, self);
        false
    }

    pub(super) fn tick_feedback(&mut self) -> bool {
        if self
            .feedback
            .transient
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() >= Duration::from_secs(5))
        {
            self.feedback.transient = None;
            self.feedback.dirty = true;
        }
        let error = self.media.snapshot.error.clone();
        if error != self.feedback.media_error {
            if error.is_empty() {
                let previous = format!("播放失败：{}", self.feedback.media_error);
                if self
                    .feedback
                    .error
                    .as_ref()
                    .is_some_and(|e| e.message == previous)
                {
                    self.feedback.error = None;
                    self.feedback.dirty = true;
                }
            } else {
                self.report_status(FeedbackKind::Error, format!("播放失败：{error}"));
            }
            self.feedback.media_error = error;
        }
        std::mem::take(&mut self.feedback.dirty)
    }

    pub(super) fn feedback_view(&self, window: &Window, cx: &Context<Self>) -> Option<AnyElement> {
        if self.feedback.transient.is_none() && self.feedback.error.is_none() {
            return None;
        }
        Some(
            div()
                .id("feedback-overlay")
                .absolute()
                .top_4()
                .right_4()
                .w(px(
                    (f32::from(window.viewport_size().width) - 32.).clamp(160., 440.)
                ))
                .flex()
                .flex_col()
                .gap_2()
                .children(
                    self.feedback
                        .error
                        .as_ref()
                        .map(|e| self.feedback_card(&e.message, true, e.retry.is_some(), cx)),
                )
                .children(
                    self.feedback
                        .transient
                        .as_ref()
                        .map(|(s, _)| self.feedback_card(s, false, false, cx)),
                )
                .into_any_element(),
        )
    }

    fn feedback_card(
        &self,
        message: &str,
        error: bool,
        retry: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let color = if error {
            cx.theme().danger
        } else {
            cx.theme().primary
        };
        div()
            .id(if error {
                "feedback-error"
            } else {
                "feedback-info"
            })
            .p_4()
            .rounded_lg()
            .border_1()
            .border_color(color.opacity(0.5))
            .bg(cx.theme().background)
            .shadow_md()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(color)
                            .child(if error { "操作未完成" } else { "提示" }),
                    )
                    .when(retry, |v| {
                        v.child(
                            self.button("feedback-retry", "重试", Action::RetryFeedback, cx)
                                .ghost(),
                        )
                    })
                    .child(
                        self.button(
                            if error {
                                "feedback-dismiss-error"
                            } else {
                                "feedback-dismiss-info"
                            },
                            "关闭",
                            Action::DismissFeedback(error),
                            cx,
                        )
                        .ghost(),
                    ),
            )
            .child(
                div()
                    .id(if error {
                        "feedback-error-message"
                    } else {
                        "feedback-info-message"
                    })
                    .max_h(px(160.))
                    .overflow_y_scroll()
                    .text_sm()
                    .child(message.to_string()),
            )
            .into_any_element()
    }
}
