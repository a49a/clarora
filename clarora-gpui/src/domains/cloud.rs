//! Cloud commands own their action effects and execution.
use super::*;
pub(super) enum Command {
    CloudList,
    CloudBackup,
    CloudRestore(String),
    CloudSync,
    CloudAutoSync,
    CloudRetention(u8, usize),
    CloudCleanupPreview,
    CloudCleanup,
}
impl Command {
    pub(super) fn effects(&self, _app: &AppView) -> Effects {
        Effects::default()
    }
    pub(super) fn execute(
        self,
        app: &mut AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Result<()> {
        match self {
            Command::CloudList => app.begin_scoped(
                TaskScope::Application,
                "正在读取备份版本…",
                move |lib, cancel, _| {
                    Ok(Outcome::CloudVersions(
                        backup::Storage::load(&lib)?.versions(&cancel)?,
                    ))
                },
            ),

            Command::CloudBackup => {
                ensure!(
                    !app.tasks.foreground.is_running(),
                    "已有任务正在执行，请完成或取消后重试"
                );
                #[cfg(debug_assertions)]
                let fixture = app.tasks.backup_fixture;
                app.begin_scoped(
                    TaskScope::Application,
                    "正在整理云备份…",
                    move |lib, cancel, tx| {
                        #[cfg(debug_assertions)]
                        let storage = fixture
                            .map(backup::Storage::loopback_fixture)
                            .unwrap_or_else(|| backup::Storage::load(&lib))?;
                        #[cfg(not(debug_assertions))]
                        let storage = backup::Storage::load(&lib)?;
                        let incremental = storage.incremental_supported();
                        let progress = move |progress| {
                            let _ = tx.send(TaskUpdate::BackupProgress(progress));
                        };
                        let result = if incremental {
                            storage.upload_incremental(&lib, &cancel, progress)
                        } else {
                            storage.upload_with_progress(&lib, &cancel, progress)
                        };
                        Ok(match result {
                            Ok(key) => Outcome::BackupCreated(key, if incremental { 3 } else { 2 }),
                            Err(error) => Outcome::BackupFailed(format!("{error:#}")),
                        })
                    },
                );
                app.tasks.backup_progress = Some(backup::UploadProgress {
                    stage: backup::UploadStage::Preparing,
                    bytes_sent: 0,
                    bytes_total: 0,
                    bytes_reused: 0,
                    files_done: 0,
                    files_total: 0,
                    current_name: String::new(),
                });
                app.tasks.notice = Some(TaskNotice::new("云备份", "正在整理资料和附件…"));
            }

            Command::CloudRestore(key) => app.begin_scoped(
                TaskScope::Application,
                "正在校验并恢复备份…",
                move |lib, cancel, _| {
                    let count =
                        backup::Storage::load(&lib)?.restore_checked(&lib, &key, &cancel)?;
                    let videos = video_library::Collection::load(&lib)
                        .context("资料已恢复，但重新载入视频库失败，请刷新")?;
                    Ok(Outcome::LibraryRestored(count, videos))
                },
            ),

            Command::CloudSync => app.cloud_sync(window, cx)?,

            Command::CloudAutoSync => {
                app.cloud.automatic = !app.cloud.automatic;
                app.lib.set_setting(
                    "cloud_auto_sync_v3",
                    if app.cloud.automatic { "true" } else { "false" },
                )?;
                if app.cloud.automatic {
                    app.cloud.last_attempt = Instant::now() - Duration::from_secs(300);
                }
                app.report_status(
                    FeedbackKind::Info,
                    if app.cloud.automatic {
                        "自动同步已开启：每5分钟在页面空闲时检查"
                    } else {
                        "自动同步已关闭，仍可手动同步"
                    }
                    .into(),
                );
            }

            Command::CloudRetention(which, value) => {
                match which {
                    0 => {
                        ensure!((1..=10000).contains(&value), "保留份数无效");
                        app.cloud.retention.keep_last = value;
                    }
                    1 => {
                        ensure!(value <= 36500, "保留天数无效");
                        app.cloud.retention.keep_days = value as u64;
                    }
                    2 => {
                        ensure!((1..=36500).contains(&value), "孤立对象宽限无效");
                        app.cloud.retention.orphan_days = value as u64;
                    }
                    _ => anyhow::bail!("保留策略无效"),
                }
                app.lib.set_setting(
                    "cloud_retention_v3",
                    &serde_json::to_string(&app.cloud.retention)?,
                )?;
                app.cloud.plan = None;
            }

            Command::CloudCleanupPreview => {
                let policy = app.cloud.retention.clone();
                app.begin_scoped(
                    TaskScope::Application,
                    "正在检查可清理版本和附件…",
                    move |lib, cancel, _| {
                        Ok(Outcome::CloudCleanupPlan(
                            backup::Storage::load(&lib)?.preview_cleanup(policy, &cancel)?,
                        ))
                    },
                );
            }

            Command::CloudCleanup => {
                let plan = app.cloud.plan.clone().context("请先预览可清理内容")?;
                app.begin_scoped(
                    TaskScope::Application,
                    "正在按预览清理云端对象…",
                    move |lib, cancel, _| {
                        let storage = backup::Storage::load(&lib)?;
                        let result = storage.cleanup(&plan, &cancel)?;
                        Ok(Outcome::CloudCleaned(result, storage.versions(&cancel)?))
                    },
                );
                app.cloud.plan = None;
            }
        }
        Ok(())
    }
}

// Completion policy belongs to this domain: cancellation may race a durable commit.
pub(super) enum Completed {
    LibraryRestored(usize, video_library::Collection),
    BackupCreated(String, u32),
    BackupFailed(String),
    CloudVersions(Vec<backup::BackupVersion>),
    CloudCleanupPlan(backup::CleanupPlan),
    CloudCleaned(backup::CleanupResult, Vec<backup::BackupVersion>),
    CloudSynced(backup::SyncResult, video_library::Collection),
}
impl Completed {
    pub(super) fn apply(
        self,
        app: &mut AppView,
        context: &mut Completion,
        _window: &mut Window,
        _cx: &mut Context<AppView>,
    ) {
        match self {
            Completed::CloudVersions(v) => {
                app.backups = v.iter().map(|x| x.key.clone()).collect();
                app.cloud.versions = v;
                app.report_status(
                    FeedbackKind::Info,
                    format!("已读取 {} 个云备份版本", app.cloud.versions.len()),
                );
            }

            Completed::CloudCleanupPlan(plan) => {
                app.report_status(
                    FeedbackKind::Info,
                    format!(
                        "清理预览：{} 个旧版本、{} 个未引用对象；确认后才会删除",
                        plan.snapshots.len(),
                        plan.objects.len()
                    ),
                );
                app.cloud.plan = Some(plan);
            }

            Completed::CloudCleaned(report, versions) => {
                app.backups = versions.iter().map(|v| v.key.clone()).collect();
                app.cloud.versions = versions;
                app.cloud.plan = None;
                app.report_status(
                    FeedbackKind::Info,
                    format!(
                        "清理完成：{} 个版本、{} 个对象，释放约 {:.1} MiB",
                        report.snapshots_deleted,
                        report.objects_deleted,
                        report.bytes_freed as f64 / 1048576.
                    ),
                );
            }

            Completed::CloudSynced(report, videos) => {
                context.cancelled = false;
                app.video.collection = videos;
                app.cloud.plan = None;
                app.cloud.status = report.summary.clone();
                app.refresh_view_data();
                app.report_status(FeedbackKind::Info, report.summary);
            }

            Completed::BackupCreated(key, format_version) => {
                app.cloud.versions.insert(
                    0,
                    backup::BackupVersion {
                        key: key.clone(),
                        created_at: chrono::Utc::now().to_rfc3339(),
                        format_version,
                    },
                );
                app.cloud.plan = None;
                // A confirmed remote commit wins over a simultaneous cancel.
                context.cancelled = false;
                if !app.backups.contains(&key) {
                    app.backups.insert(0, key);
                }
                if let Some(progress) = &mut app.tasks.backup_progress {
                    progress.stage = backup::UploadStage::Completed;
                    progress.bytes_sent = progress.bytes_total;
                    progress.files_done = progress.files_total;
                    progress.current_name.clear();
                }
                let detail = app
                    .tasks
                    .backup_progress
                    .as_ref()
                    .map(backup::UploadProgress::label)
                    .unwrap_or_else(|| "云备份完成".into());
                app.report_status(FeedbackKind::Info, detail);
            }

            Completed::BackupFailed(message) => {
                // Preserve an uncertain manifest publication or HTTP error,
                // even if cancellation was requested at the same time.
                context.cancelled = message == "已取消";
                app.report_status(
                    if context.cancelled {
                        FeedbackKind::Info
                    } else {
                        FeedbackKind::Error
                    },
                    if context.cancelled {
                        "云备份已取消".into()
                    } else {
                        message
                    },
                );
            }

            Completed::LibraryRestored(_count, videos) => {
                if app.video.path.as_ref().is_none_or(|path| !path.is_file()) {
                    if let Some(video) = videos.active().filter(|v| v.path.is_file()) {
                        app.video.path = Some(video.path.clone());
                        app.video.series = video.series.clone();
                    }
                }
                app.video.collection = videos;
                if app.pdf_path.as_ref().is_none_or(|path| !path.is_file()) {
                    if let Ok(saved) = app.lib.setting("pdf_current") {
                        if let Ok(value) = serde_json::from_str::<Value>(&saved) {
                            let path = PathBuf::from(value["path"].as_str().unwrap_or(""));
                            if path.is_file() && ebook::supported(&path) {
                                app.pdf_path = Some(path);
                                app.pdf_page =
                                    value["page"].as_u64().unwrap_or(0).min(u16::MAX as u64) as u16;
                                app.pdf_width =
                                    value["width"].as_i64().unwrap_or(1000).clamp(300, 3000) as i32;
                                app.pdf_image = None;
                                app.ebook_reading = None;
                                app.document_text.clear();
                            }
                        }
                    }
                }
                app.report_status(FeedbackKind::Info, "资料、附件与学习进度已恢复".into());
            }
        }
    }
}

impl AppView {
    pub(crate) fn update_backup_progress(&mut self, progress: backup::UploadProgress) {
        if self.tasks.foreground.cancel_token().is_some_and(|t| {
            !t.load(Ordering::SeqCst) || progress.stage == backup::UploadStage::Completed
        }) {
            let label = progress.label();
            self.report_status(FeedbackKind::Progress, label.clone());
            if let Some(notice) = self
                .tasks
                .notice
                .as_mut()
                .filter(|n| n.state == TaskState::Running)
            {
                notice.detail = label;
            }
            self.tasks.backup_progress = Some(progress);
        }
    }
}
