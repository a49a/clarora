//! Model installation has its own application-owned lane, independent of learning materials.
use super::*;

pub(super) enum Request {
    Asr(String),
    Kokoro,
}
#[derive(Default)]
pub(super) struct State {
    pub(super) lane: task_runtime::Lane<ModelProgress, &'static str>,
    pub(super) progress: Option<ModelProgress>,
    pub(super) notice: Option<TaskNotice>,
    #[cfg(debug_assertions)]
    pub(super) fixture: Option<String>,
}
impl State {
    pub(super) fn cancel(&mut self) {
        if self.lane.is_running() {
            self.lane.cancel();
            if let Some(notice) = &mut self.notice {
                notice.state = TaskState::Cancelling;
                notice.detail = "正在停止模型下载，已安装的模型会保留…".into();
            }
        }
    }
}
impl AppView {
    pub(super) fn begin_model_download(&mut self, request: Request) -> Result<()> {
        ensure!(
            !self.tasks.models.lane.is_running(),
            "已有模型正在下载，请等待完成或取消后再下载"
        );
        let title = match &request {
            Request::Asr(model) => format!("下载模型 · {model}"),
            Request::Kokoro => "下载模型 · Kokoro 中英语音".into(),
        };
        let success = match &request {
            Request::Asr(_) => "模型已安装。可在设置中选用并保存；当前学习内容保持不变。",
            Request::Kokoro => "Kokoro 中英模型已安装，可离线生成音频；当前学习内容保持不变。",
        };
        let lib = self.lib.clone();
        #[cfg(debug_assertions)]
        let fixture = self.tasks.models.fixture.clone();
        self.tasks
            .models
            .lane
            .spawn(TaskScope::Application, move |cancel, report| {
                let result = (|| -> Result<()> {
                    let mut notify = |p| {
                        let _ = report.send(p);
                    };
                    #[cfg(debug_assertions)]
                    if let Some(host) = fixture {
                        let Request::Asr(model) = &request else {
                            anyhow::bail!("诊断下载仅支持 ASR");
                        };
                        return clarora_gpui::model_download::download(
                            model,
                            &lib.root.join("models"),
                            &ai::local_model_files(model)?,
                            &[&host],
                            &cancel,
                            &mut notify,
                        );
                    }
                    match request {
                        Request::Asr(model) => ai::download_model(&lib, &model, &cancel, notify),
                        Request::Kokoro => tts::install(&lib, &cancel, &mut notify),
                    }
                })()
                .map_err(|e| format!("{e:#}"));
                result.map(|()| success)
            })?;
        self.tasks.models.progress = None;
        self.tasks.models.notice = Some(TaskNotice::new(
            &title,
            "正在连接下载源，可继续切换视频和音频",
        ));
        Ok(())
    }
    pub(super) fn poll_model_download(&mut self) -> bool {
        let models = &mut self.tasks.models;
        let mut changed = false;
        if let Some(notice) = &mut models.notice {
            changed |= notice.tick();
        }
        let mut completed = false;
        while let Some(event) = models.lane.poll() {
            changed = true;
            match event {
                task_runtime::Event::Progress { value: p, .. } => {
                    if models
                        .lane
                        .cancel_token()
                        .is_some_and(|t| !t.load(Ordering::SeqCst))
                    {
                        if let Some(notice) = &mut models.notice {
                            notice.detail = p.label();
                        }
                        models.progress = Some(p);
                    }
                }
                task_runtime::Event::Completed(done) => {
                    let result = done.result;
                    // A successful atomic installation wins over a cancel pressed
                    // after the worker finished; never report an installed model as cancelled.
                    let cancelled = result.is_err() && done.cancelled;
                    models.progress = None;
                    if let Some(notice) = &mut models.notice {
                        notice.elapsed = notice.started.elapsed().as_secs();
                        (notice.state, notice.detail) = match result {
                            Ok(detail) => (TaskState::Succeeded, detail.into()),
                            Err(_) if cancelled => (
                                TaskState::Cancelled,
                                "已停止下载，已安装的模型会保留。".into(),
                            ),
                            Err(error) => (
                                TaskState::Failed,
                                format!("模型下载失败：{error}。可在设置中重新下载。"),
                            ),
                        };
                    }
                    completed = true;
                }
            }
        }
        if completed {
            // Refresh availability for the current data key; do not reopen media,
            // clear chat output, or finish the unrelated foreground task.
            self.refresh_view_data();
        }
        changed
    }
}
