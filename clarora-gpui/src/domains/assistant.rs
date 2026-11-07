//! Assistant commands own their action effects and execution.
use super::*;
pub(super) enum Command {
    Assistant,
    Chat,
    AssistantQuote(clarora_gpui::assistant::Reference),
    AssistantCurrent,
    AssistantRemove(usize),
    AssistantReferenceDetails,
    AssistantTurnReference(usize),
    AssistantSessions,
    AssistantWide,
    AssistantLegacy,
    AssistantCopy(String),
    AssistantPrompt(String),
    SaveAnswer,
    ChatSession(String),
    ToggleReasoning,
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
            Command::Assistant => {
                if app.assistant_open {
                    app.assistant_open = false;
                    window.focus(&app.focus);
                    app.lib.set_setting("assistant_open", "false")?;
                } else {
                    app.open_assistant(window, cx);
                    app.inputs["question"].update(cx, |state, cx| state.focus(window, cx));
                }
            }

            Command::AssistantQuote(reference) => {
                app.add_assistant_reference(reference, window, cx)?;
            }

            Command::AssistantCurrent => {
                if app.page == "video" {
                    app.prepare_video_material(video_qa::Scope::Current)?;
                }
                let reference = clarora_gpui::assistant::Reference {
                    text: clarora_gpui::assistant::plain_reference(&app.reference(cx)),
                };
                app.add_assistant_reference(reference, window, cx)?;
            }

            Command::AssistantRemove(index) => {
                if index < app.assistant.references.len() {
                    app.assistant.references.remove(index);
                    app.persist_assistant_references()?;
                }
            }

            Command::AssistantReferenceDetails => app.assistant.expanded = !app.assistant.expanded,

            Command::AssistantTurnReference(index) => {
                app.assistant.expanded_turn = if app.assistant.expanded_turn == Some(index) {
                    None
                } else {
                    Some(index)
                };
            }

            Command::AssistantSessions => {
                app.assistant.sessions_open = !app.assistant.sessions_open
            }

            Command::AssistantWide => app.assistant.wide = !app.assistant.wide,

            Command::AssistantLegacy => {
                ensure!(!app.chat.running, "请先停止当前回答");
                app.assistant.legacy = !app.assistant.legacy;
                app.assistant.pending = None;
                app.assistant.error.clear();
                app.chat_requested_context.clear();
                app.sync_chat(window, cx);
            }

            Command::AssistantCopy(answer) => {
                cx.write_to_clipboard(ClipboardItem::new_string(answer));
            }

            Command::AssistantPrompt(question) => {
                app.set("question", question, window, cx);
                app.inputs["question"].update(cx, |state, cx| state.focus(window, cx));
            }

            Command::ToggleReasoning => app.chat.reasoning_open = !app.chat.reasoning_open,

            Command::ChatSession(session) => {
                app.cancel_chat_task();
                app.chat_requested_context.clear();
                app.sync_chat(window, cx);
                ensure!(
                    !app.chat.is_loading() && app.chat.context == app.chat_context_key(cx),
                    "请先恢复材料会话读取"
                );
                ensure!(!app.chat.is_loading(), "正在读取材料会话，请稍候");
                let draft = app.value("question", cx);
                app.chat.select(&session, &draft)?;
                app.assistant.pending = None;
                app.assistant.error.clear();
                app.assistant.expanded_turn = None;
                app.assistant.sessions_open = false;
                if session.is_empty() {
                    app.assistant.references.clear();
                    app.persist_assistant_references()?;
                }
                app.inputs["question"].update(cx, |state, cx| state.focus(window, cx));
            }

            Command::Chat => {
                ensure!(
                    app.assistant.enabled
                        || !matches!(app.page.as_str(), "flashcard" | "discover")
                        || app.cards_ready(),
                    "正在读取学习卡片，请稍候"
                );
                ensure!(
                    !app.tasks.foreground.is_running(),
                    "已有任务正在执行，请完成或取消后重试"
                );
                app.chat_requested_context.clear();
                app.sync_chat(window, cx);
                ensure!(
                    !app.chat.is_loading() && app.chat.context == app.chat_context_key(cx),
                    "请先恢复材料会话读取"
                );
                if app.chat.id.is_empty() {
                    app.chat.id = id();
                }
                let session_id = app.chat.id.clone();
                let context = app.chat.context.clone();
                let question = app.value("question", cx);
                let references = app.assistant.references.clone();
                clarora_gpui::assistant::validate(&references)?;
                let global = app.assistant.enabled;
                let passage = if global {
                    references
                        .iter()
                        .map(|r| r.text.clone())
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    clarora_gpui::assistant::plain_reference(&app.reference(cx))
                };
                let video_material = if !global && app.page == "video" {
                    Some(app.video.material.clone().context(
                        if app.video.material_error.is_empty() {
                            "请先生成字幕并选择提问范围".to_string()
                        } else {
                            app.video.material_error.clone()
                        },
                    )?)
                } else {
                    None
                };
                let history = app.chat.history.clone();
                ensure!(!question.trim().is_empty(), "请输入问题");
                app.chat.output.clear();
                app.chat.reasoning.clear();
                if global {
                    app.assistant.pending = Some((question.clone(), references.clone()));
                    app.assistant.error.clear();
                    app.assistant.scroll.scroll_to_bottom();
                }
                let request = app.chat.begin_request();
                app.begin_chat_task(if global {TaskScope::Application}else{TaskScope::Material},"AI 正在回答…", request.clone(), move |lib, cancel, tx| {
                    let mut notify=|chunk|{let _=tx.send(TaskUpdate::Chunk(request.clone(), chunk));};
                    let video_material=video_material.map(|mut material|{material.text=clarora_gpui::assistant::plain_reference(&material.text);material});
                    let reply=if global {clarora_gpui::assistant::ask(&lib,&question,&references,&history,&cancel,&mut notify)?}else if let Some(material)=video_material {
                        video_qa::ask(&lib,&question,&material,&history,&cancel,&mut notify)?
                    }else{ai::chat_stream(&lib,&question,&passage,&history,&cancel,&mut notify)?};
                    ensure!(!cancel.load(Ordering::SeqCst), "已取消");
                    let mut next = history;
                    next.push(if global {clarora_gpui::assistant::user_turn(&question,&references)}else{json!({"role":"user","content":question,"reference":passage})});
                    next.push(json!({"role":"assistant","content":reply.answer,"reasoning_content":reply.reasoning}));
                    if next.len() > 60 {
                        next.drain(..next.len() - 60);
                    }
                    ai::save_chat(&lib, &session_id, &context, &question, &next)?;
                    Ok(Outcome::Output(reply.answer))
                });
            }

            Command::SaveAnswer => {
                ensure!(
                    !app.chat.is_loading() && app.chat.context == app.chat_context_key(cx),
                    "请先等待当前材料会话读取完成"
                );
                ensure!(!app.tasks.foreground.is_running(), "请等待 AI 回答完成");
                let answer = app
                    .chat
                    .history
                    .last()
                    .filter(|m| m["role"] == "assistant")
                    .and_then(|m| m["content"].as_str())
                    .context("请先完成一次 AI 回答")?;
                ensure!(
                    !answer.trim().is_empty() && answer == app.chat.output,
                    "当前回答未完整保存，请完成回答后再存卡"
                );
                let question = app
                    .chat
                    .history
                    .iter()
                    .rev()
                    .find(|m| m["role"] == "user")
                    .and_then(|m| m["content"].as_str())
                    .context("会话缺少问题")?;
                let reference = app
                    .chat
                    .history
                    .iter()
                    .rev()
                    .find(|m| m["role"] == "user")
                    .and_then(|m| m["reference"].as_str())
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        app.chat
                            .history
                            .iter()
                            .rev()
                            .find(|m| m["role"] == "user")
                            .map(clarora_gpui::assistant::references)
                            .unwrap_or_default()
                            .iter()
                            .map(|r| r.text.clone())
                            .collect::<Vec<_>>()
                            .join("\n\n")
                    });
                app.lib.put("ai_cards",json!({"id":id(),"question":question,"answer":answer,"context_text":reference,"created_at":now()}))?;
                app.report_status(FeedbackKind::Info, "已存为 AI 问答卡".into());
            }
        }
        Ok(())
    }
}

// Completion policy belongs to this domain: cancellation may race a durable commit.
pub(super) enum Completed {
    Output(String),
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
            Completed::Output(s) if !context.cancelled => {
                if context.chat {
                    app.chat.output = s;
                } else {
                    app.output = s;
                }
                app.report_status(FeedbackKind::Info, "已完成".into());
            }

            _ => app.report_status(FeedbackKind::Info, "已取消".into()),
        }
    }
}

impl AppView {
    pub(crate) fn update_chat_task(
        &mut self,
        request: &clarora_gpui::chat_store::Request,
        chunk: ai::ChatReply,
    ) {
        if self.tasks.chat_request.as_ref() == Some(request)
            && self.chat.accepts(request)
            && self
                .tasks
                .foreground
                .cancel_token()
                .is_some_and(|t| !t.load(Ordering::SeqCst))
        {
            self.chat.output = chunk.answer;
            self.chat.reasoning = chunk.reasoning;
        }
    }
    pub(crate) fn accept_chat_task(
        &mut self,
        request: &clarora_gpui::chat_store::Request,
        cancelled: bool,
        result: &Result<Outcome, String>,
    ) -> bool {
        if !self.chat.finish_request(request) {
            // A displaced request may have committed. Refresh the index without painting it.
            if let Err(error) = self.chat.refresh() {
                self.report_status(FeedbackKind::Error, format!("刷新会话失败：{error:#}"));
            }
            return false;
        }
        if self.assistant.enabled {
            if cancelled || result.is_err() {
                self.assistant.failed_output = self.chat.output.clone();
            }
            self.assistant.error = if cancelled {
                "已停止回答。问题和引用已保留，可再次发送。".into()
            } else if let Err(error) = result {
                format!("回答未完成：{error:#}。问题和引用已保留。")
            } else {
                String::new()
            };
        }
        true
    }
    pub(crate) fn cancel_chat_task(&self) {
        if self
            .tasks
            .chat_request
            .as_ref()
            .is_some_and(|request| self.chat.accepts(request))
        {
            self.tasks.foreground.cancel();
        }
    }
}
