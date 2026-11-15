//! Foreground delivery and lifecycle; domain controllers apply completed results.
use super::*;
pub(super) enum Outcome {
    CredentialMode(bool),
    LibraryRestored(usize, video_library::Collection),
    Message(String),
    ClipSaved,
    VideoClipSaved,
    Issue(String),
    Parts(bilibili::Series),
    PartVideo {
        series: bilibili::Series,
        index: usize,
        path: PathBuf,
        origin: u64,
        from_end: bool,
        resume: bool,
        group: String,
    },
    Output(String),
    VideoTranslated(PathBuf, String, Vec<Cue>, Vec<Cue>),
    VideoDownloaded(PathBuf, u64, String, process::VideoSource),
    VideoAuthRequired(String, u64, String, process::VideoSource),
    Pdf(PathBuf, i32, documents::PdfPage),
    Ebook(ebook::Reading),
    Ocr(String),
    KeyPoints(String, Vec<ai::KeyPoint>),
    BackupCreated(String, u32),
    BackupFailed(String),
    CloudVersions(Vec<backup::BackupVersion>),
    CloudCleanupPlan(backup::CleanupPlan),
    CloudCleaned(backup::CleanupResult, Vec<backup::BackupVersion>),
    CloudSynced(backup::SyncResult, video_library::Collection),
    Recording(PathBuf),
    TtsReady(Row),
    CommandImported(clarora_gpui::command_import::Report),
    SpeakingCommandImported(clarora_gpui::command_import::SpeakingReport),
    SpeakingTextReady(String, speaking_ui::PromptText),
    SpeakingGroupTagsReady(String, Vec<String>),
    SpeakingReviewed(String, clarora_gpui::writing::Review),
    SpeakingDraftAudio(PathBuf),
    AudioMerged(clarora_gpui::audio_tools::Merged),
}
pub(super) enum TaskUpdate {
    SubtitleProgress(String),
    Chunk(clarora_gpui::chat_store::Request, ai::ChatReply),
    BackupProgress(backup::UploadProgress),
}

type Reporter = task_runtime::Reporter<TaskUpdate, Outcome>;

impl AppView {
    pub(super) fn begin(
        &mut self,
        label: &str,
        task: impl FnOnce(Library, Cancel, Reporter) -> Result<Outcome> + Send + 'static,
    ) {
        self.begin_scoped(TaskScope::Material, label, task);
    }
    pub(super) fn begin_scoped(
        &mut self,
        scope: TaskScope,
        label: &str,
        task: impl FnOnce(Library, Cancel, Reporter) -> Result<Outcome> + Send + 'static,
    ) {
        self.begin_correlated(scope, label, None, task);
    }
    pub(super) fn begin_cloud(
        &mut self,
        label: &str,
        task: impl FnOnce(Library, Cancel, Reporter) -> Result<Outcome> + Send + 'static,
    ) -> bool {
        if self.tasks.cloud.is_running() {
            self.report_status(
                FeedbackKind::Error,
                "已有云任务正在执行，请完成或取消后重试".into(),
            );
            return false;
        }
        let lib = self.lib.clone();
        if let Err(error) = self
            .tasks
            .cloud
            .spawn(TaskScope::Application, move |cancel, report| {
                task(lib, cancel, report).map_err(|e| format!("{e:#}"))
            })
        {
            self.report_status(FeedbackKind::Error, format!("无法启动云任务：{error:#}"));
            return false;
        }
        self.tasks.cloud_notice = Some(TaskNotice::new(label, "正在准备…"));
        self.report_status(FeedbackKind::Progress, label.into());
        true
    }
    pub(super) fn begin_chat_task(
        &mut self,
        scope: TaskScope,
        label: &str,
        request: clarora_gpui::chat_store::Request,
        task: impl FnOnce(Library, Cancel, Reporter) -> Result<Outcome> + Send + 'static,
    ) {
        self.begin_correlated(scope, label, Some(request), task);
    }
    fn begin_correlated(
        &mut self,
        scope: TaskScope,
        label: &str,
        request: Option<clarora_gpui::chat_store::Request>,
        task: impl FnOnce(Library, Cancel, Reporter) -> Result<Outcome> + Send + 'static,
    ) {
        if self.tasks.foreground.is_running() {
            self.report_status(
                FeedbackKind::Error,
                "已有任务正在执行，请完成或取消后重试".into(),
            );
            return;
        }
        let lib = self.lib.clone();
        if let Err(error) = self.tasks.foreground.spawn(scope, move |cancel, report| {
            task(lib, cancel, report).map_err(|e| format!("{e:#}"))
        }) {
            if let Some(request) = request.as_ref() {
                self.chat.finish_request(request);
            }
            self.report_status(FeedbackKind::Error, format!("无法启动任务：{error:#}"));
            return;
        }
        self.tasks.chat_request = request;
        self.tasks.notice = None;
        self.tasks.progress_elapsed = 0;
        self.tasks.retry = self.feedback.retry_candidate.clone();
        self.report_status(FeedbackKind::Progress, label.into());
    }
    pub(super) fn begin_subtitle(
        &mut self,
        title: &str,
        detail: &str,
        task: impl FnOnce(Library, Cancel, Reporter) -> Result<Outcome> + Send + 'static,
    ) {
        if self.tasks.foreground.is_running() {
            self.report_status(
                FeedbackKind::Error,
                "已有任务正在执行，请完成或取消后重试".into(),
            );
            return;
        }
        self.begin(&format!("正在{title}…"), task);
        if self.tasks.foreground.is_running() {
            self.tasks.notice = Some(TaskNotice::new(title, detail));
        }
    }
    pub(super) fn poll_tasks(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let mut changed = false;
        if let Some(notice) = &mut self.tasks.notice {
            if notice.state == TaskState::Running
                && self
                    .tasks
                    .foreground
                    .cancel_token()
                    .is_some_and(|t| t.load(Ordering::SeqCst))
            {
                notice.state = TaskState::Cancelling;
                notice.detail = "正在停止当前处理，请稍候…".into();
                changed = true;
            }
            changed |= notice.tick();
        }
        if let Some(notice) = &mut self.tasks.cloud_notice {
            changed |= notice.tick();
        }
        changed |= self.poll_video_titles();
        changed |= self.poll_cloud_tasks(window, cx);
        while let Some(event) = self.tasks.foreground.poll() {
            changed = true;
            match event {
                task_runtime::Event::Progress { value, .. } => match value {
                    TaskUpdate::SubtitleProgress(stage) => {
                        if let Some(notice) = self
                            .tasks
                            .notice
                            .as_mut()
                            .filter(|n| n.state == TaskState::Running)
                        {
                            notice.detail = stage;
                        }
                    }
                    TaskUpdate::BackupProgress(progress) => self.update_backup_progress(progress),
                    TaskUpdate::Chunk(request, chunk) => self.update_chat_task(&request, chunk),
                },
                task_runtime::Event::Completed(done) => {
                    let request = self.tasks.chat_request.take();
                    let result = done.result;
                    let task_retry = self.tasks.retry.take();
                    let import_task = self.video.import_active;
                    if import_task {
                        let unfinished = done.cancelled
                            || result.is_err()
                            || matches!(&result, Ok(Outcome::Issue(_)));
                        if unfinished {
                            self.video.import_recovery = self.video.import_draft.clone();
                        } else {
                            self.video.import_recovery = None;
                        }
                    }
                    let failed = result.is_err()
                        || matches!(&result, Ok(Outcome::Issue(_)))
                        || matches!(&result, Ok(Outcome::BackupFailed(_)));
                    let completed_backup = matches!(
                        &result,
                        Ok(Outcome::BackupCreated(..))
                            | Ok(Outcome::BackupFailed(_))
                            | Ok(Outcome::CloudSynced(..))
                    );
                    let completed_sync = matches!(&result, Ok(Outcome::CloudSynced(..)));
                    let completed_chat = request.is_some();
                    if let Some(request) = &request
                        && !self.accept_chat_task(request, done.cancelled, &result)
                    {
                        continue;
                    }
                    let mut completion = domains::Completion {
                        cancelled: done.cancelled,
                        chat: completed_chat,
                    };
                    match result {
                        Ok(outcome) => {
                            domains::complete(outcome, self, &mut completion, window, cx)
                        }
                        Err(_) if completion.cancelled => {
                            self.report_status(FeedbackKind::Info, "已取消当前任务".into());
                        }
                        Err(e) => {
                            self.report_status(FeedbackKind::Error, e);
                            self.feedback.attach_retry(task_retry);
                        }
                    }
                    if import_task {
                        self.video.import_active = false;
                    }
                    if let Some(notice) = self.tasks.notice.as_mut().filter(|n| n.active()) {
                        notice.state = if completion.cancelled {
                            TaskState::Cancelled
                        } else if failed {
                            TaskState::Failed
                        } else {
                            TaskState::Succeeded
                        };
                        notice.elapsed = notice.started.elapsed().as_secs();
                        notice.detail = self.status.clone();
                    }
                    // Chat only updates its persisted conversation. Refreshing materials
                    // here would invalidate the revealed cue and switch its chat context.
                    if !completed_chat && !completed_backup {
                        self.refresh();
                    }
                    if let Err(error) = if completed_backup && !completed_sync {
                        Ok(())
                    } else {
                        self.chat.refresh()
                    } {
                        self.report_status(FeedbackKind::Error, format!("刷新会话失败：{error:#}"));
                    }
                }
            }
        }
        changed
    }

    fn poll_cloud_tasks(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let mut changed = false;
        while let Some(event) = self.tasks.cloud.poll() {
            changed = true;
            match event {
                task_runtime::Event::Progress { value, .. } => {
                    if let TaskUpdate::BackupProgress(progress) = value {
                        self.update_backup_progress(progress);
                    }
                }
                task_runtime::Event::Completed(done) => {
                    let cancelled = done.cancelled;
                    let result = done.result;
                    let mut completion = domains::Completion {
                        cancelled,
                        chat: false,
                    };
                    let failure_detail = match &result {
                        Err(error) => Some(error.clone()),
                        Ok(Outcome::BackupFailed(error) | Outcome::Issue(error)) => {
                            Some(error.clone())
                        }
                        _ => None,
                    };
                    let failed = failure_detail.is_some();
                    match result {
                        Ok(outcome) => {
                            domains::complete(outcome, self, &mut completion, window, cx)
                        }
                        Err(error) => self.report_status(FeedbackKind::Error, error),
                    }
                    let detail = if completion.cancelled {
                        "已取消".to_string()
                    } else if let Some(error) = failure_detail {
                        error
                    } else {
                        self.tasks
                            .backup_progress
                            .as_ref()
                            .map(backup::UploadProgress::label)
                            .unwrap_or_else(|| "云任务完成".into())
                    };
                    if let Some(notice) = self.tasks.cloud_notice.as_mut() {
                        notice.state = if completion.cancelled {
                            TaskState::Cancelled
                        } else if failed {
                            TaskState::Failed
                        } else {
                            TaskState::Succeeded
                        };
                        notice.detail = detail;
                        notice.elapsed = notice.started.elapsed().as_secs();
                    }
                    cx.notify();
                }
            }
        }
        changed
    }
}
