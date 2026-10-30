//! App-wide cloud operations and opt-in scheduling; all network work is background.
use super::*;

pub(super) struct State {
    pub versions: Vec<backup::BackupVersion>,
    pub plan: Option<backup::CleanupPlan>,
    pub retention: backup::Retention,
    pub automatic: bool,
    pub last_attempt: Instant,
    pub status: String,
}
impl State {
    pub fn new(lib: &Library) -> Self {
        let retention =
            serde_json::from_str(&lib.setting("cloud_retention_v3").unwrap_or_default())
                .unwrap_or_default();
        Self {
            versions: vec![],
            plan: None,
            retention,
            automatic: lib.setting("cloud_auto_sync_v3").ok().as_deref() == Some("true"),
            last_attempt: Instant::now(),
            status: String::new(),
        }
    }
}
impl AppView {
    pub(super) fn cloud_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Result<()> {
        ensure!(
            self.tasks.cancel.is_none() && !self.chat.running,
            "请先完成当前任务再同步"
        );
        self.flush_writing(cx)?;
        self.cloud.last_attempt = Instant::now();
        #[cfg(debug_assertions)]
        let fixture = self.tasks.backup_fixture;
        self.begin_scoped(
            TaskScope::Application,
            "正在同步云端资料…",
            move |lib, cancel, tx| {
                #[cfg(debug_assertions)]
                let storage = if let Some(address) = fixture {
                    backup::Storage::loopback_fixture(address)?
                } else {
                    backup::Storage::load(&lib)?
                };
                #[cfg(not(debug_assertions))]
                let storage = backup::Storage::load(&lib)?;
                let result = storage.synchronize(&lib, &cancel, move |progress| {
                    let _ = tx.send(Event::BackupProgress(progress));
                })?;
                let videos = video_library::Collection::load(&lib)?;
                Ok(Outcome::CloudSynced(result, videos))
            },
        );
        self.tasks.backup_progress = Some(backup::UploadProgress {
            stage: backup::UploadStage::Preparing,
            bytes_sent: 0,
            bytes_total: 0,
            bytes_reused: 0,
            files_done: 0,
            files_total: 0,
            current_name: String::new(),
        });
        self.tasks.notice = Some(TaskNotice::new("云端同步", "正在比较变更、删除标记与附件…"));
        let _ = window;
        Ok(())
    }
    pub(super) fn poll_auto_cloud_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.cloud.automatic
            || self.cloud.last_attempt.elapsed() < Duration::from_secs(300)
            || self.tasks.cancel.is_some()
            || self.chat.running
            || self.assistant_open
            || self.composer_open
            || !matches!(
                self.page.as_str(),
                "video" | "listening" | "meditation" | "settings"
            )
        {
            return;
        }
        // Active editors keep their unsaved in-memory text. Try when focus returns
        // to the page rather than reloading an editor under the user's cursor.
        if !self.focus.is_focused(window) {
            return;
        }
        self.cloud.last_attempt = Instant::now();
        if let Err(error) = self.cloud_sync(window, cx) {
            self.cloud.status = format!("自动同步未开始：{error:#}");
        }
    }
    pub(super) fn cloud_sync_settings(&self, cx: &Context<Self>) -> AnyElement {
        let busy = self.tasks.cancel.is_some();
        let menu = |which: u8, values: &[usize], unit: &str, current: usize| {
            values
                .iter()
                .map(|value| {
                    (
                        format!("{value}{unit}"),
                        Action::CloudRetention(which, *value),
                        *value == current,
                        false,
                    )
                })
                .collect()
        };
        let mut view=div().flex().flex_col().gap_3()
            .child(div().text_sm().child("新版备份按内容分块去重，重试会复用已完成分块；恢复前校验完整内容。同步按每条资料的最后修改时间处理覆盖和删除。"))
            .child(div().flex().flex_wrap().gap_2()
                .child(self.button("cloud-sync-now","立即同步",Action::CloudSync,cx).primary().disabled(busy))
                .child(self.button("cloud-sync-auto",if self.cloud.automatic{"自动同步：开 · 5分钟"}else{"自动同步：关"},Action::CloudAutoSync,cx).outline().selected(self.cloud.automatic).disabled(busy)))
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child("自动同步默认关闭；开启后在页面空闲且未编辑文字时运行，切换模块后任务继续。修改时间相同则按设备标识和序号稳定决定。"))
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child("首次同步前的旧资料没有修改记录，以初始版本合并；启用后持续记录离线修改。新版去重、同步与清理需要支持条件写入的 S3／R2；OSS 继续使用兼容备份。"))
            .when(!self.cloud.status.is_empty(),|v|v.child(div().text_sm().child(self.cloud.status.clone())))
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("备份保留与清理"))
            .child(div().flex().flex_wrap().gap_2()
                .child(self.surface_menu("cloud-retain-count",format!("至少留 {} 份 ▾",self.cloud.retention.keep_last),menu(0,&[1,3,5,10,20,50],"份",self.cloud.retention.keep_last),cx))
                .child(self.surface_menu("cloud-retain-days",format!("保留近 {} 天 ▾",self.cloud.retention.keep_days),menu(1,&[0,7,30,90,365],"天",self.cloud.retention.keep_days as usize),cx))
                .child(self.surface_menu("cloud-orphan-days",format!("孤立附件宽限 {} 天 ▾",self.cloud.retention.orphan_days),menu(2,&[1,7,30],"天",self.cloud.retention.orphan_days as usize),cx)))
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child("最近份数与保留天数同时生效；仍被备份、同步状态或活动任务引用的附件不会清理。先预览，再确认执行。"))
            .child(self.button("cloud-cleanup-preview","预览可清理内容",Action::CloudCleanupPreview,cx).outline().disabled(busy));
        if let Some(plan) = &self.cloud.plan {
            view = view.child(
                div()
                    .p_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().border)
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(format!(
                        "将清理 {} 个旧版本、{} 个未引用对象，约 {:.1} MiB。",
                        plan.snapshots.len(),
                        plan.objects.len(),
                        plan.bytes as f64 / 1048576.
                    ))
                    .child(
                        self.button(
                            "cloud-cleanup-apply",
                            "确认按预览清理",
                            Action::CloudCleanup,
                            cx,
                        )
                        .outline()
                        .disabled(busy || plan.snapshots.is_empty() && plan.objects.is_empty()),
                    ),
            );
        }
        view.into_any_element()
    }
}
