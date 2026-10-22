use super::*;
use gpui_component::spinner::Spinner;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum TaskState {
    Running,
    Cancelling,
    Succeeded,
    Failed,
    Cancelled,
}
pub(super) struct TaskNotice {
    pub title: String,
    pub detail: String,
    pub state: TaskState,
    pub started: Instant,
    pub elapsed: u64,
}
impl TaskNotice {
    pub fn new(title: &str, detail: &str) -> Self {
        Self {
            title: title.into(),
            detail: detail.into(),
            state: TaskState::Running,
            started: Instant::now(),
            elapsed: 0,
        }
    }
    pub fn active(&self) -> bool {
        matches!(self.state, TaskState::Running | TaskState::Cancelling)
    }
}
impl AppView {
    pub(super) fn subtitle_task_banner(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if self.task_notice.is_none() && self.video_job_notice.is_none() {
            return None;
        }
        Some(
            div()
                .flex_shrink_0()
                .flex()
                .flex_col()
                .children(self.video_job_notice.as_ref().map(|n| {
                    self.task_notice_card(
                        n,
                        "background-video",
                        Action::CancelVideoSubtitles,
                        Action::DismissVideoSubtitles,
                        cx,
                    )
                }))
                .children(self.task_notice.as_ref().map(|n| {
                    self.task_notice_card(
                        n,
                        "subtitle",
                        Action::Cancel,
                        Action::DismissTaskNotice,
                        cx,
                    )
                }))
                .into_any_element(),
        )
    }
    fn task_notice_card(
        &self,
        notice: &TaskNotice,
        key: &str,
        cancel: Action,
        dismiss: Action,
        cx: &Context<Self>,
    ) -> AnyElement {
        let color = if notice.state == TaskState::Failed {
            cx.theme().danger
        } else {
            cx.theme().primary
        };
        let heading = match notice.state {
            TaskState::Running => format!("正在{}", notice.title),
            TaskState::Cancelling => format!("{} · 正在取消", notice.title),
            TaskState::Succeeded => format!("{} · 已完成", notice.title),
            TaskState::Failed => format!("{} · 失败", notice.title),
            TaskState::Cancelled => format!("{} · 已取消", notice.title),
        };
        let indicator = if notice.active() {
            Spinner::new()
                .icon(Icon::new(IconName::Loader).path("ui/task-spinner.svg"))
                .with_size(px(24.))
                .color(color)
                .into_any_element()
        } else {
            div()
                .text_lg()
                .text_color(color)
                .child(match notice.state {
                    TaskState::Succeeded => "✓",
                    TaskState::Failed => "!",
                    _ => "—",
                })
                .into_any_element()
        };
        let button = if notice.active() {
            self.button(
                format!("{key}-task-cancel"),
                if notice.state == TaskState::Cancelling {
                    "正在取消…"
                } else if key == "background-video" {
                    "取消后台字幕"
                } else {
                    "取消任务"
                },
                cancel,
                cx,
            )
            .outline()
            .disabled(notice.state == TaskState::Cancelling)
        } else {
            self.button(format!("{key}-task-dismiss"), "关闭提示", dismiss, cx)
                .ghost()
        };
        div()
            .id(SharedString::from(format!("{key}-task-notice")))
            .flex_shrink_0()
            .mx_4()
            .mb_3()
            .p_4()
            .rounded_lg()
            .border_1()
            .border_color(color.opacity(0.45))
            .bg(cx.theme().accent)
            .text_color(cx.theme().foreground)
            .flex()
            .items_center()
            .flex_wrap()
            .gap_3()
            .child(indicator)
            .child(
                div()
                    .flex_1()
                    .min_w(px(180.))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(heading),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("{key}-task-detail")))
                            .text_sm()
                            .max_h(px(96.))
                            .overflow_y_scroll()
                            .child(notice.detail.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!(
                                "{} {:02}:{:02}",
                                if notice.active() {
                                    "已用时"
                                } else {
                                    "耗时"
                                },
                                notice.elapsed / 60,
                                notice.elapsed % 60
                            )),
                    ),
            )
            .child(
                button
                    .h(px(40.))
                    .px_4()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD),
            )
            .into_any_element()
    }
}
