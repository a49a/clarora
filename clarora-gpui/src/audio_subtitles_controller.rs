//! Audio subtitle work belongs to captured sources, independently of playback or chat.
use super::*;
use clarora_gpui::audio_batch::{Mode, Plan, Report};

pub(super) struct Target {
    pub audio: Option<String>,
    pub mode: Mode,
    pub name: String,
}
#[derive(Default)]
pub(super) struct State {
    pub lane: task_runtime::Lane<String, Report>,
    pub notice: Option<TaskNotice>,
    target: Option<Target>,
}
impl State {
    pub(super) fn working(&self, audio: &str, mode: Mode) -> bool {
        self.lane.is_running()
            && self
                .target
                .as_ref()
                .is_some_and(|t| t.audio.as_deref() == Some(audio) && t.mode == mode)
    }
    pub(super) fn cancel(&mut self) {
        if self.lane.is_running() {
            self.lane.cancel();
            if let Some(n) = &mut self.notice {
                n.state = TaskState::Cancelling;
                n.detail = "正在停止字幕处理，已保存的结果会保留…".into();
            }
        }
    }
}
impl AppView {
    pub(super) fn begin_audio_subtitles(&mut self, plan: Plan, target: Target) -> Result<()> {
        ensure!(
            !self.tasks.audio_subtitles.lane.is_running(),
            "已有音频字幕任务正在处理，可继续切换音频；请等待完成或在顶部取消后再生成"
        );
        let title = if target.audio.is_some() {
            match target.mode {
                Mode::English => "生成音频字幕",
                Mode::Chinese => "翻译音频字幕",
            }
        } else {
            target.mode.label()
        };
        let title = format!("{title} · {}", target.name);
        let lib = self.lib.clone();
        self.tasks
            .audio_subtitles
            .lane
            .spawn(TaskScope::Application, move |cancel, report| {
                Ok(plan.run(&lib, &cancel, |stage| {
                    let _ = report.send(stage.into());
                }))
            })?;
        self.tasks.audio_subtitles.target = Some(target);
        self.tasks.audio_subtitles.notice = Some(TaskNotice::new(
            &title,
            "正在准备字幕处理，可继续学习其它音频；完成后自动保存。",
        ));
        Ok(())
    }
    pub(super) fn poll_audio_subtitles(&mut self) -> bool {
        let state = &mut self.tasks.audio_subtitles;
        let mut changed = false;
        if let Some(n) = &mut state.notice {
            changed |= n.tick();
        }
        let mut completed = false;
        while let Some(event) = state.lane.poll() {
            changed = true;
            match event {
                task_runtime::Event::Progress { value: stage, .. } => {
                    if let Some(n) = state
                        .notice
                        .as_mut()
                        .filter(|n| n.state == TaskState::Running)
                    {
                        n.detail = stage;
                    }
                }
                task_runtime::Event::Completed(done) => {
                    let result = done.result;
                    if let Some(n) = &mut state.notice {
                        n.elapsed = n.started.elapsed().as_secs();
                        (n.state, n.detail) = match result {
                            Ok(report) => {
                                let status = if report.cancelled {
                                    TaskState::Cancelled
                                } else if !report.failed.is_empty() {
                                    TaskState::Failed
                                } else {
                                    TaskState::Succeeded
                                };
                                let message = if let Some(target) =
                                    state.target.as_ref().filter(|t| t.audio.is_some())
                                {
                                    if report.cancelled {
                                        "已停止字幕处理，已保存的结果会保留。".into()
                                    } else if let Some((_, error)) = report.failed.first() {
                                        format!("「{}」处理失败：{error}", target.name)
                                    } else if report.skipped > 0 {
                                        format!("「{}」已有所需字幕，无需重复处理。", target.name)
                                    } else {
                                        format!("「{}」字幕已保存，当前播放保持不变。", target.name)
                                    }
                                } else {
                                    report.message()
                                };
                                (status, message)
                            }
                            Err(error) => (TaskState::Failed, error),
                        };
                    }
                    completed = true;
                }
            }
        }
        if completed {
            self.refresh_view_data();
        }
        changed
    }
}
