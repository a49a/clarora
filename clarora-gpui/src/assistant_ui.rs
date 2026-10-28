//! App-wide sidecar: explicit references, readable transcript and a fixed composer.
use super::*;
use clarora_gpui::assistant::{self, Reference};

#[derive(Default)]
pub(super) struct State {
    pub enabled: bool,
    pub legacy: bool,
    pub references: Vec<Reference>,
    pub pending: Option<(String, Vec<Reference>)>,
    pub error: String,
    pub failed_output: String,
    pub expanded: bool,
    pub expanded_turn: Option<usize>,
    pub sessions_open: bool,
    pub wide: bool,
    pub scroll: ScrollHandle,
    rendered: (String, usize, usize),
    pub press: Option<Point<Pixels>>,
    pub dragging: bool,
    pub offer: Option<(Reference, Point<Pixels>)>,
}
impl AppView {
    /// Keep assistant actions visibly distinct from selectable learning text.
    fn assistant_button(
        &self,
        key: impl Into<SharedString>,
        label: impl Into<SharedString>,
        action: Action,
        cx: &Context<Self>,
    ) -> Button {
        let icon = match &action {
            Action::AssistantSessions => Some("history"),
            Action::ChatSession(id) if id.is_empty() => Some("plus"),
            Action::AssistantWide => Some(if self.assistant.wide {
                "fullscreen-exit"
            } else {
                "fullscreen"
            }),
            Action::Assistant => Some("assistant-close"),
            Action::AssistantCurrent => Some("plus"),
            Action::AssistantRemove(_) => Some("close"),
            Action::AssistantCopy(_) => Some("copy"),
            Action::SaveAnswer => Some("bookmark"),
            Action::Chat => Some("send"),
            Action::Cancel => Some("close"),
            Action::AssistantReferenceDetails => Some(if self.assistant.expanded {
                "chevron-up"
            } else {
                "chevron-down"
            }),
            _ => None,
        };
        let selected = match &action {
            Action::AssistantSessions => self.assistant.sessions_open,
            Action::ChatSession(id) => !id.is_empty() && *id == self.chat.id,
            _ => false,
        };
        self.button(key, label, action, cx)
            .with_size(gpui_component::Size::Medium)
            .px_3()
            .custom(
                ButtonCustomVariant::new(cx)
                    .color(cx.theme().muted)
                    .foreground(cx.theme().foreground)
                    .border(cx.theme().primary.opacity(0.3))
                    .hover(cx.theme().accent)
                    .active(cx.theme().primary.opacity(0.2)),
            )
            // Preserve the button shape even when its action is disabled.
            .border_color(cx.theme().primary.opacity(0.3))
            .selected(selected)
            .when_some(icon, |button, icon| {
                button.icon(Icon::default().path(format!("ui/{icon}.svg")))
            })
    }
    pub(super) fn persist_assistant_references(&self) -> Result<()> {
        self.lib.set_setting(
            "assistant_reference_draft",
            &serde_json::to_string(&self.assistant.references)?,
        )
    }
    pub(super) fn open_assistant(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.assistant.enabled = true;
        self.assistant_open = true;
        if let Err(error) = self.lib.set_setting("assistant_open", "true") {
            self.report_status(FeedbackKind::Error, format!("助手状态保存失败：{error:#}"));
        }
        self.sync_chat(window, cx);
    }
    pub(super) fn add_assistant_reference(
        &mut self,
        reference: Reference,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        ensure!(!reference.text.trim().is_empty(), "请先用鼠标选取文字");
        let mut next = self.assistant.references.clone();
        if !next.contains(&reference) {
            next.push(reference);
        }
        assistant::validate(&next)?;
        self.lib
            .set_setting("assistant_reference_draft", &serde_json::to_string(&next)?)?;
        self.assistant.references = next;
        self.assistant.expanded = true;
        self.assistant.error.clear();
        self.assistant.offer = None;
        self.open_assistant(window, cx);
        self.inputs["question"].update(cx, |state, cx| state.focus(window, cx));
        Ok(())
    }
    /// Called after an actual mouse drag, before another click can clear TextView's
    /// native selection. Copy targets the original focused element, then restores
    /// the clipboard. No stale clipboard contents can become an offered reference.
    pub(super) fn offer_assistant_selection(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.page == "settings"
            || self.page == "meditation"
            || (self.page == "listening" && self.audio.blind.enabled)
        {
            return;
        }
        let selected = if self
            .subtitle_text
            .borrow()
            .selection_from_press(self.assistant.press)
            || (self.assistant.press.is_none()
                && self.subtitle_text.borrow().focus.is_focused(window))
        {
            self.subtitle_text.borrow().selection_text()
        } else {
            String::new()
        };
        if matches!(self.page.as_str(), "video" | "listening") && !selected.is_empty() {
            self.assistant.offer = Some((Reference { text: selected }, position));
            cx.notify();
            return;
        }
        let view = cx.entity().downgrade();
        let focus = window.focused(cx);
        let page = self.page.clone();
        let handle = window.window_handle();
        // Finish the input callback before rendering the final selection. Drawing
        // while AppView is leased would recurse; requesting animation here would
        // access GPUI's paint-only stack. This also works in hidden/minimized windows.
        cx.defer(move |cx| {
            let _ = handle.update(cx, move |_, window, cx| {
                window.draw(cx).clear();
                if !focus.as_ref().is_some_and(|focus| focus.is_focused(window)) {
                    return;
                }
                let previous = cx.read_from_clipboard();
                let sentinel = format!("clarora-no-selection-{}", id());
                cx.write_to_clipboard(ClipboardItem::new_string(sentinel.clone()));
                window.dispatch_action(Box::new(gpui_component::input::Copy), cx);
                cx.defer(move |cx| {
                    let selected = cx
                        .read_from_clipboard()
                        .and_then(|item| item.text())
                        .unwrap_or_default();
                    cx.write_to_clipboard(
                        previous.unwrap_or_else(|| ClipboardItem::new_string(String::new())),
                    );
                    if selected != sentinel
                        && !selected.trim().is_empty()
                        && selected.chars().count() <= 120_000
                    {
                        let _ = view.update(cx, |this, cx| {
                            if this.page == page {
                                this.assistant.offer =
                                    Some((Reference { text: selected }, position));
                                cx.notify();
                            }
                        });
                    }
                });
            });
        });
        cx.notify();
    }
    pub(super) fn selection_offer(
        &self,
        window: &Window,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let (reference, position) = self.assistant.offer.as_ref()?;
        Some(
            div()
                .absolute()
                .left(
                    position
                        .x
                        .min(window.viewport_size().width - px(150.))
                        .max(px(8.)),
                )
                .top(
                    (position.y + px(12.))
                        .min(window.viewport_size().height - px(60.))
                        .max(px(8.)),
                )
                .p_1()
                .rounded_lg()
                .shadow_lg()
                .bg(cx.theme().background)
                .border_1()
                .border_color(cx.theme().border)
                .child(
                    self.assistant_button(
                        "selection-ask-ai",
                        "✦ 问 AI",
                        Action::AssistantQuote(reference.clone()),
                        cx,
                    )
                    .primary(),
                )
                .into_any_element(),
        )
    }
    fn assistant_quote_view(
        &self,
        key: String,
        reference: &Reference,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .p_3()
            .rounded_lg()
            .bg(cx.theme().muted)
            .flex()
            .flex_col()
            .gap_2()
            .child(
                TextView::html(
                    SharedString::from(key),
                    format!(
                        "<p>{}</p>",
                        reference
                            .text
                            .replace('&', "&amp;")
                            .replace('<', "&lt;")
                            .replace('>', "&gt;")
                            .replace('\n', "<br/>")
                    ),
                    window,
                    cx,
                )
                .selectable(true),
            )
            .into_any_element()
    }
    pub(super) fn assistant_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let signature = (
            self.chat.id.clone(),
            self.chat.history.len(),
            self.chat.output.len(),
        );
        if signature != self.assistant.rendered {
            let follow = self.assistant.rendered.0 != signature.0
                || f32::from(self.assistant.scroll.offset().y).abs() + 60.
                    >= f32::from(self.assistant.scroll.max_offset().height);
            if follow {
                self.assistant.scroll.scroll_to_bottom();
            }
            self.assistant.rendered = signature;
        }
        let mut history = div()
            .id("assistant-history")
            .track_scroll(&self.assistant.scroll)
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_4()
            .flex()
            .flex_col()
            .gap_4()
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation());
        if self.chat.history.is_empty() && self.assistant.pending.is_none() {
            history=history.child(div().py_6().flex().flex_col().gap_3()
                .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child("选一段文字，从这里问起"))
                .child(div().text_sm().text_color(cx.theme().muted_foreground).child("在字幕、电子书、闪卡或写作中拖选文字，点击浮出的「问 AI」。引用会先放在下方，确认后再发送。"))
                .child(div().text_sm().text_color(cx.theme().muted_foreground).child("也可以直接提问。切换栏目仍保留这段对话。")));
        }
        for (i, message) in self.chat.history.clone().iter().enumerate() {
            let user = message["role"] == "user";
            let mut bubble = div()
                .id(SharedString::from(format!("assistant-turn-{i}")))
                .p_4()
                .rounded_xl()
                .flex()
                .flex_col()
                .gap_3()
                .bg(if user {
                    cx.theme().accent
                } else {
                    cx.theme().background
                })
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(cx.theme().primary)
                        .child(if user {
                            "你 · 已发送"
                        } else {
                            "✦ Clarora · 回答"
                        }),
                );
            let mut refs = assistant::references(message);
            if refs.is_empty() {
                if let Some(text) = message["reference"].as_str().filter(|s| !s.is_empty()) {
                    refs.push(Reference {
                        text: assistant::plain_reference(text),
                    });
                }
            }
            if !refs.is_empty() {
                bubble = bubble.child(self.assistant_button(
                    format!("turn-ref-{i}"),
                    format!(
                        "{} 份引用 · {}",
                        refs.len(),
                        if self.assistant.expanded_turn == Some(i) {
                            "收起"
                        } else {
                            "查看原文"
                        }
                    ),
                    Action::AssistantTurnReference(i),
                    cx,
                ));
                if self.assistant.expanded_turn == Some(i) {
                    for (r, reference) in refs.iter().enumerate() {
                        bubble = bubble.child(self.assistant_quote_view(
                            format!("sent-ref-{i}-{r}"),
                            reference,
                            window,
                            cx,
                        ));
                    }
                }
            }
            bubble = bubble.child(
                TextView::markdown(
                    ("assistant-message", i),
                    message["content"].as_str().unwrap_or_default().to_string(),
                    window,
                    cx,
                )
                .selectable(true),
            );
            if !user {
                bubble = bubble.child(self.assistant_button(
                    format!("copy-answer-{i}"),
                    "复制回答",
                    Action::AssistantCopy(message["content"].as_str().unwrap_or_default().into()),
                    cx,
                ));
            }
            history = history.child(bubble);
        }
        if let Some((question, refs)) = &self.assistant.pending {
            let mut pending = div()
                .p_4()
                .rounded_xl()
                .bg(cx.theme().accent)
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("你 · 本次发送"),
                )
                .child(
                    TextView::markdown("assistant-pending-question", question.clone(), window, cx)
                        .selectable(true),
                );
            for (i, reference) in refs.clone().iter().enumerate() {
                pending = pending.child(self.assistant_quote_view(
                    format!("pending-ref-{i}"),
                    reference,
                    window,
                    cx,
                ));
            }
            history = history.child(pending);
        }
        if self.chat.running
            || (self.assistant.pending.is_some() && !self.assistant.error.is_empty())
        {
            history = history.child(
                div()
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(cx.theme().primary)
                            .child(if self.chat.running {
                                "✦ Clarora · 正在回答…"
                            } else {
                                "✦ Clarora · 未完成"
                            }),
                    )
                    .child(
                        TextView::markdown(
                            "assistant-stream",
                            if self.chat.running {
                                if self.chat.output.is_empty() {
                                    "正在等待回复…".into()
                                } else {
                                    self.chat.output.clone()
                                }
                            } else {
                                format!(
                                    "以下为未完成的输出，未保存为回答：\n\n{}",
                                    self.assistant.failed_output
                                )
                            },
                            window,
                            cx,
                        )
                        .selectable(true),
                    ),
            );
        }
        if !self.chat.reasoning.is_empty() {
            history = history.child(self.assistant_button(
                "toggle-reasoning",
                if self.chat.reasoning_open {
                    "收起思考过程"
                } else {
                    "查看思考过程（非正式回答）"
                },
                Action::ToggleReasoning,
                cx,
            ));
            if self.chat.reasoning_open {
                history = history.child(
                    TextView::markdown(
                        "assistant-reasoning",
                        self.chat.reasoning.clone(),
                        window,
                        cx,
                    )
                    .selectable(true),
                );
            }
        }
        let mut composer = div()
            .flex_shrink_0()
            .p_4()
            .border_t_1()
            .border_color(cx.theme().border)
            .flex()
            .flex_col()
            .gap_3();
        if !self.assistant.references.is_empty() {
            let refs = self.assistant.references.clone();
            let mut attachments = div()
                .id("assistant-draft-references")
                .max_h(px(if self.assistant.expanded { 200. } else { 94. }))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap_2();
            for (i, reference) in refs.iter().enumerate() {
                let row = div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w_0().text_xs().child(format!(
                        "引用 {} · {} 字符",
                        i + 1,
                        reference.text.chars().count()
                    )))
                    .child(self.assistant_button(
                        format!("remove-ref-{i}"),
                        "移除",
                        Action::AssistantRemove(i),
                        cx,
                    ));
                attachments = attachments.child(row);
                if self.assistant.expanded {
                    attachments = attachments.child(self.assistant_quote_view(
                        format!("draft-ref-{i}"),
                        reference,
                        window,
                        cx,
                    ));
                }
            }
            if !self.assistant.references.is_empty() {
                composer = composer.child(
                    div()
                        .flex()
                        .gap_2()
                        .flex_wrap()
                        .child(
                            self.assistant_button(
                                "prompt-explain",
                                "解释用法",
                                Action::AssistantPrompt(
                                    "请解释选中的文字，说明重点词语和用法。".into(),
                                ),
                                cx,
                            )
                            .disabled(self.chat.running || self.chat.is_loading()),
                        )
                        .child(
                            self.assistant_button(
                                "prompt-translate",
                                "翻译成中文",
                                Action::AssistantPrompt("请把选中的文字翻译成自然的中文。".into()),
                                cx,
                            )
                            .disabled(self.chat.running || self.chat.is_loading()),
                        )
                        .child(
                            self.assistant_button(
                                "prompt-examples",
                                "举个例子",
                                Action::AssistantPrompt(
                                    "请解释选中的表达，并给出两个使用例句。".into(),
                                ),
                                cx,
                            )
                            .disabled(self.chat.running || self.chat.is_loading()),
                        ),
                );
            }
            composer = composer
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(format!("待发送引用 · {} 份", refs.len())),
                        )
                        .child(self.assistant_button(
                            "expand-reference",
                            if self.assistant.expanded {
                                "收起原文"
                            } else {
                                "查看完整原文"
                            },
                            Action::AssistantReferenceDetails,
                            cx,
                        )),
                )
                .child(attachments);
        } else {
            composer = composer.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("本次未附加引用，仅发送你的问题和本会话历史。"),
            );
        }
        composer = composer
            .child(
                Input::new(&self.inputs["question"])
                    .disabled(self.chat.running || self.chat.is_loading()),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_between()
                    .items_center()
                    .gap_2()
                    .child(
                        self.assistant_button(
                            "assistant-reference-current",
                            "引用当前材料",
                            Action::AssistantCurrent,
                            cx,
                        )
                        .disabled(self.chat.running),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(if cfg!(target_os = "macos") {
                                "⌘ Enter 发送"
                            } else {
                                "Ctrl Enter 发送"
                            }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        self.assistant_button("ask", "发送问题", Action::Chat, cx)
                            .primary()
                            .disabled(self.tasks.cancel.is_some() || self.chat.is_loading()),
                    )
                    .child(
                        self.assistant_button("cancel-ai", "停止回答", Action::Cancel, cx)
                            .disabled(!self.chat.running),
                    )
                    .child(
                        self.assistant_button("save-answer", "存为问答卡", Action::SaveAnswer, cx)
                            .disabled(
                                self.chat.running
                                    || self.chat.is_loading()
                                    || self.chat.history.is_empty()
                                    || !self.assistant.error.is_empty(),
                            ),
                    ),
            );
        let mut header = div()
            .flex_shrink_0()
            .p_4()
            .border_b_1()
            .border_color(cx.theme().border)
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("✦ 学习助手"),
                    )
                    .child(self.assistant_button("close-assistant", "收起", Action::Assistant, cx)),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(if self.assistant.legacy {
                        "当前材料的历史会话"
                    } else {
                        "全局对话 · 切换栏目继续聊"
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(self.assistant_button(
                        "chat-history",
                        "历史",
                        Action::AssistantSessions,
                        cx,
                    ))
                    .child(
                        self.assistant_button(
                            "chat-new",
                            "新对话",
                            Action::ChatSession(String::new()),
                            cx,
                        )
                        .disabled(self.chat.running || self.chat.is_loading()),
                    )
                    .child(self.assistant_button(
                        "assistant-wide",
                        if self.assistant.wide {
                            "紧凑"
                        } else {
                            "加宽"
                        },
                        Action::AssistantWide,
                        cx,
                    )),
            );
        if self.assistant.sessions_open {
            let mut sessions = div()
                .id("assistant-sessions")
                .max_h(px(160.))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap_2()
                .child(self.assistant_button(
                    "assistant-scope",
                    if self.assistant.legacy {
                        "返回全局会话"
                    } else {
                        "查看当前材料的旧会话"
                    },
                    Action::AssistantLegacy,
                    cx,
                ));
            for (i, session) in self.chat.sessions.iter().enumerate() {
                sessions = sessions.child(
                    self.assistant_button(
                        format!("session-{i}"),
                        session.title.clone(),
                        Action::ChatSession(session.id.clone()),
                        cx,
                    )
                    .disabled(self.chat.running),
                );
            }
            header = header.child(sessions);
        }
        let error = if !self.assistant.error.is_empty() {
            self.assistant.error.clone()
        } else {
            self.chat.error.clone()
        };
        div()
            .id("global-assistant")
            .absolute()
            .right_0()
            .top_0()
            .h_full()
            .w(px((if self.assistant.wide {
                660.0_f32
            } else {
                480.0_f32
            })
            .min(f32::from(window.viewport_size().width) - 16.)))
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .border_l_1()
            .border_color(cx.theme().border)
            .shadow_lg()
            .overflow_hidden()
            .child(header)
            .child(history)
            .when(!error.is_empty(), |v| {
                v.child(
                    div()
                        .px_4()
                        .py_2()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(error),
                )
            })
            .when(self.chat.is_loading(), |v| {
                v.child(div().px_4().text_xs().child("正在恢复会话…"))
            })
            .child(composer)
            .into_any_element()
    }
}
