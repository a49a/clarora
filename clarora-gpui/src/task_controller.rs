//! Foreground delivery and lifecycle; domain controllers apply completed results.
use super::*;
pub(super) enum Outcome {
    CredentialMode(bool),
    LibraryRestored(usize, video_library::Collection),
    Message(String),
    Issue(String),
    Parts(bilibili::Series),
    PartVideo(bilibili::Series, usize, PathBuf, bool, u64, bool),
    Output(String),
    VideoTranslated(PathBuf, String, Vec<Cue>, Vec<Cue>),
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
        self.tasks.backup_progress = None;
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
        changed |= self.poll_video_titles();
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
                    let failed = result.is_err()
                        || matches!(&result, Ok(Outcome::Issue(_)))
                        || matches!(&result, Ok(Outcome::BackupFailed(_)));
                    let completed_backup = self.tasks.backup_progress.is_some();
                    let completed_sync = matches!(&result, Ok(Outcome::CloudSynced(..)));
                    let completed_chat = request.is_some();
                    if let Some(request) = &request {
                        if !self.accept_chat_task(request, done.cancelled, &result) {
                            continue;
                        }
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
}
