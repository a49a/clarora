//! Chat editor and asynchronous repository coordination.
use super::*;

pub(super) struct Closing {
    reply: mpsc::Receiver<std::result::Result<(), String>>,
    editor: String,
    context: String,
    requested_context: String,
    revision: u64,
    started: Instant,
}

impl AppView {
    pub(super) fn chat_context_key(&self, cx: &App) -> String {
        if self.assistant.enabled && !self.assistant.legacy {
            return clarora_gpui::assistant::CONTEXT.into();
        }
        use sha2::{Digest, Sha256};
        let material = match self.page.as_str() {
            "flashcard" | "discover" => self
                .current_card()
                .filter(|_| self.cards_ready())
                .map(|c| format!("{}:{}", c.kind, c.id))
                .unwrap_or_default(),
            "listening" if self.audio.blind.enabled => format!(
                "blind:{}:{}:{}",
                self.audio.current_id,
                self.audio.blind_chinese,
                self.audio
                    .blind
                    .revealed
                    .as_ref()
                    .map(|r| format!("{}:{}:{}", r.cue.start, r.cue.end, r.cue.text))
                    .unwrap_or_else(|| "hidden".into())
            ),
            "listening" => self.audio.current_id.clone(),
            "video" if self.video.material.is_some() => {
                self.video.material.as_ref().unwrap().key.clone()
            }
            "video" => self
                .media
                .path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
            "pdf" => self
                .pdf_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
            "ocr" => self
                .ocr_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
            "speaking" => self
                .recorded
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
            "writing" => self
                .writing
                .as_ref()
                .map(|view| view.read(cx).context_key())
                .unwrap_or_default(),
            _ => self.page.clone(),
        };
        hex::encode(Sha256::digest(
            format!("{}:{material}", self.page).as_bytes(),
        ))
    }
    pub(super) fn sync_chat(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let context = self.chat_context_key(cx);
        if context == self.chat_requested_context {
            return;
        }
        if self.chat.running && self.chat.requested_context() != context {
            if let Some(cancel) = &self.tasks.cancel {
                cancel.store(true, Ordering::SeqCst);
            }
        }
        let draft = self.value("question", cx);
        match self.chat.switch_context(&context, &draft) {
            Ok(_) if self.chat.requested_context() == context => {
                self.chat_requested_context = context
            }
            Ok(_) => {} // Retry backoff has not queued a request yet.
            Err(error) => {
                self.report_status(FeedbackKind::Error, format!("读取材料会话失败：{error:#}"))
            }
        }
    }
    pub(super) fn sync_assistant(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.assistant.enabled
            && self.page == "video"
            && (self.assistant_open || self.video.tab == 2)
            && self.video.material.is_none()
            && self.video.material_error.is_empty()
        {
            let _ = self.prepare_video_material(video_qa::Scope::All);
        }
        self.sync_chat(window, cx);
    }
    pub(super) fn poll_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let draft = self.value("question", cx);
        self.chat.observe_draft(&draft);
        let update = self.chat.poll();
        if let Some(draft) = update.draft {
            self.set("question", draft, window, cx);
        }
        for error in update.errors {
            self.report_status(FeedbackKind::Error, error);
        }
        if self.assistant.enabled
            && !self.chat.running
            && self.assistant.error.is_empty()
            && !self.chat.is_loading()
        {
            if let Some((question, references)) = &self.assistant.pending {
                let sent = self.chat.history.iter().rev().find(|m| m["role"] == "user");
                if sent.is_some_and(|m| {
                    m["content"].as_str() == Some(question)
                        && clarora_gpui::assistant::references(m) == *references
                }) {
                    if self.value("question", cx) == *question {
                        self.set("question", String::new(), window, cx);
                    }
                    if self.assistant.references == *references {
                        self.assistant.references.clear();
                        if let Err(error) = self.persist_assistant_references() {
                            self.report_status(
                                FeedbackKind::Error,
                                format!("引用草稿保存失败：{error:#}"),
                            );
                        }
                    }
                    self.assistant.pending = None;
                }
            }
        }
        update.changed
    }
    pub(super) fn prepare_close(&mut self, cx: &mut Context<Self>) -> bool {
        if self.focus_mode.active() {
            self.report_status(
                FeedbackKind::Info,
                "电脑专注尚未结束。需要提前离开时，请点击顶部「紧急结束」".into(),
            );
            cx.notify();
            return false;
        }
        if self.chat_closing.is_some() {
            return false;
        }
        if let Err(error) = self.flush_writing(cx) {
            self.report_status(
                FeedbackKind::Error,
                format!("写作草稿尚未保存，窗口保持打开：{error:#}"),
            );
            cx.notify();
            return false;
        }
        let editor = self.value("question", cx);
        self.chat.observe_draft(&editor);
        self.chat_closing = Some(Closing {
            reply: self.chat.flush_async(),
            editor,
            context: self.chat.context.clone(),
            requested_context: self.chat.requested_context().into(),
            revision: self.chat.draft_revision(),
            started: Instant::now(),
        });
        cx.notify();
        false
    }
    pub(super) fn poll_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(closing) = &self.chat_closing else {
            return false;
        };
        let result = match closing.reply.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty)
                if closing.started.elapsed() < Duration::from_secs(12) =>
            {
                return false;
            }
            Err(mpsc::TryRecvError::Empty) => {
                Err("等待问题草稿保存超时，窗口保持打开，请重试".into())
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("问题草稿保存线程已结束，窗口保持打开".into())
            }
        };
        let closing = self.chat_closing.take().unwrap();
        match result {
            Ok(())
                if closing.editor == self.value("question", cx)
                    && closing.context == self.chat.context
                    && closing.requested_context == self.chat.requested_context()
                    && closing.revision == self.chat.draft_revision() =>
            {
                if let Err(error) = self.flush_writing(cx) {
                    self.report_status(
                        FeedbackKind::Error,
                        format!("写作草稿尚未保存，窗口保持打开：{error:#}"),
                    );
                    cx.notify();
                    return false;
                }
                #[cfg(debug_assertions)]
                if self.probe.is_some() {
                    ui_controller_probe::close_verified(self, window, cx);
                }
                window.remove_window();
                true
            }
            Ok(()) => {
                self.prepare_close(cx);
                false
            }
            Err(error) => {
                self.report_status(FeedbackKind::Error, error);
                cx.notify();
                false
            }
        }
    }
}
