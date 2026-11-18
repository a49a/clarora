use super::*;
use gpui_component::spinner::Spinner;

impl AppView {
    pub(super) fn subtitle_task_banner(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let foreground =
            self.tasks.foreground.is_running() && self.tasks.notice.is_none() && !self.chat.running;
        if self.tasks.notice.is_none()
            && self.tasks.cloud_notice.is_none()
            && self.tasks.video_notice.is_none()
            && self.tasks.models.notice.is_none()
            && self.tasks.audio_subtitles.notice.is_none()
            && self.video.import_recovery.is_none()
            && !foreground
        {
            return None;
        }
        Some(
            div()
                .flex_shrink_0()
                .flex()
                .flex_col()
                .child(
                    div().px_4().pt_2().flex().justify_end().child(
                        self.button(
                            "tasks-collapse",
                            if self.tasks.collapsed {
                                "展开任务"
                            } else {
                                "收起任务"
                            },
                            Action::ToggleTasksCollapsed,
                            cx,
                        )
                        .small()
                        .ghost(),
                    ),
                )
                .children(
                    self.video
                        .import_recovery
                        .as_ref()
                        .map(|_| self.video_import_recovery_card(cx)),
                )
                .children(self.tasks.audio_subtitles.notice.as_ref().map(|n| {
                    self.task_notice_card(
                        n,
                        "background-audio",
                        Action::CancelAudioSubtitles,
                        Action::DismissAudioSubtitles,
                        cx,
                    )
                }))
                .children(self.tasks.models.notice.as_ref().map(|n| {
                    self.task_notice_card(
                        n,
                        "background-model",
                        Action::CancelModel,
                        Action::DismissModelNotice,
                        cx,
                    )
                }))
                .children(foreground.then(|| {
                    let started = self.tasks.foreground.started().unwrap_or_else(Instant::now);
                    let notice = TaskNotice {
                        title: "处理任务".into(),
                        detail: self.tasks.progress_label.clone(),
                        model: None,
                        state: if self
                            .tasks
                            .foreground
                            .cancel_token()
                            .is_some_and(|token| token.load(Ordering::SeqCst))
                        {
                            TaskState::Cancelling
                        } else {
                            TaskState::Running
                        },
                        started,
                        elapsed: started.elapsed().as_secs(),
                    };
                    self.task_notice_card(
                        &notice,
                        "foreground",
                        Action::Cancel,
                        Action::DismissTaskNotice,
                        cx,
                    )
                }))
                .children(self.tasks.video_notice.as_ref().map(|n| {
                    self.task_notice_card(
                        n,
                        "background-video",
                        Action::CancelVideoSubtitles,
                        Action::DismissVideoSubtitles,
                        cx,
                    )
                }))
                .children(self.tasks.cloud_notice.as_ref().map(|n| {
                    self.task_notice_card(
                        n,
                        "cloud-backup",
                        Action::CancelCloud,
                        Action::DismissCloudNotice,
                        cx,
                    )
                }))
                .children(self.tasks.notice.as_ref().map(|n| {
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
    fn video_import_recovery_card(&self, cx: &Context<Self>) -> AnyElement {
        let draft = self
            .video
            .import_recovery
            .as_ref()
            .expect("import recovery");
        let target = if draft.group.is_empty() {
            "未分组".to_string()
        } else {
            draft.group.clone()
        };
        let part = draft
            .part
            .map(|index| format!(" · P{}", index + 1))
            .unwrap_or_default();
        div()
            .id("video-import-recovery")
            .flex_shrink_0()
            .mx_4()
            .mb_3()
            .p_4()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().primary.opacity(0.45))
            .bg(cx.theme().accent)
            .text_color(cx.theme().foreground)
            .flex()
            .items_center()
            .flex_wrap()
            .gap_3()
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
                            .child("视频导入未完成"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .child(format!("已保留链接、目标分组 · {target}{part}")),
                    ),
            )
            .child(
                self.button(
                    "video-import-recovery-retry",
                    "重试导入",
                    Action::VideoImportRetry,
                    cx,
                )
                .primary(),
            )
            .child(
                self.button(
                    "video-import-recovery-edit",
                    "编辑链接",
                    Action::VideoImportEdit,
                    cx,
                )
                .outline(),
            )
            .child(
                self.button(
                    "video-import-recovery-dismiss",
                    "关闭提示",
                    Action::DismissVideoImport,
                    cx,
                )
                .ghost(),
            )
            .into_any_element()
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
        if self.tasks.collapsed {
            let detail = notice.detail.lines().next().unwrap_or_default().to_string();
            let detail = notice
                .model
                .as_ref()
                .map_or(detail.clone(), |model| format!("{model} · {detail}"));
            let action = if notice.active() {
                (
                    cancel,
                    if notice.state == TaskState::Cancelling {
                        "取消中…".to_string()
                    } else {
                        "取消".to_string()
                    },
                )
            } else {
                (dismiss, "关闭".to_string())
            };
            return div()
                .id(SharedString::from(format!("{key}-task-notice-compact")))
                .flex_shrink_0()
                .mx_4()
                .mb_2()
                .px_3()
                .py_2()
                .rounded_lg()
                .border_1()
                .border_color(color.opacity(0.4))
                .bg(cx.theme().accent)
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(heading),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(detail),
                )
                .when(notice.active(), |row| {
                    row.child(
                        Spinner::new()
                            .icon(Icon::new(IconName::Loader).path("ui/task-spinner.svg"))
                            .with_size(px(16.))
                            .color(color),
                    )
                })
                .child(
                    self.button(format!("{key}-compact-action"), action.1, action.0, cx)
                        .small()
                        .outline(),
                )
                .into_any_element();
        }
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
                } else if key == "cloud-backup" {
                    "取消备份"
                } else if key == "background-model" {
                    "取消模型下载"
                } else if key == "background-audio" {
                    "取消字幕任务"
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
                    .when_some(notice.model.as_ref(), |v, model| {
                        v.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("使用模型：{model}")),
                        )
                    })
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
                    )
                    .when(key == "background-model", |v| {
                        v.when_some(
                            self.tasks
                                .models
                                .progress
                                .as_ref()
                                .and_then(|p| p.percent()),
                            |v, p| {
                                v.child(Progress::new().value(p).bg(cx.theme().primary).w_full())
                            },
                        )
                    })
                    .when(key == "cloud-backup", |v| {
                        v.when_some(
                            self.tasks
                                .backup_progress
                                .as_ref()
                                .and_then(|p| p.percent()),
                            |v, percent| {
                                v.child(
                                    div()
                                        .id("cloud-backup-progress")
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child(Progress::new().value(percent).bg(color).flex_1())
                                        .child(
                                            div()
                                                .text_xs()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .child(format!("{percent:.0}%")),
                                        ),
                                )
                            },
                        )
                    }),
            )
            .when(
                notice.state == TaskState::Failed && !notice.detail.trim().is_empty(),
                |v| {
                    v.child(
                        self.button(
                            format!("{key}-task-copy"),
                            "复制错误",
                            Action::CopySelection(notice.detail.clone()),
                            cx,
                        )
                        .ghost()
                        .h(px(40.))
                        .px_4()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD),
                    )
                },
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
