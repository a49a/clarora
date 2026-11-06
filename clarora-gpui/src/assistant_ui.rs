//! App-wide sidecar: explicit references, readable transcript and a fixed composer.
use super::*;
use clarora_gpui::assistant::{self, Reference};

#[derive(Default)]
pub(super) struct State {
    #[cfg(debug_assertions)]
    pub debug_history_builds: std::rc::Rc<std::cell::Cell<u64>>,
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
    history_view: Option<Entity<super::assistant_history_ui::HistoryView>>,
    pub press: Option<Point<Pixels>>,
    pub dragging: bool,
    pub offer: Option<(Reference, Point<Pixels>)>,
}
impl AppView {
    // Synchronize at action/tick boundaries, before drawing. Notifying a cached
    // child from inside parent Render can leave it stale until another event.
    pub(super) fn sync_assistant_history(&self, cx: &mut Context<Self>) {
        if let Some(view) = &self.assistant.history_view {
            view.update(cx, |history, cx| history.sync(self, cx));
        }
    }
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
        let history_view = if let Some(view) = &self.assistant.history_view {
            view.clone()
        } else {
            let owner = cx.entity().downgrade();
            let view = cx.new(|_| super::assistant_history_ui::HistoryView::new(owner, self));
            self.assistant.history_view = Some(view.clone());
            view
        };
        let history = AnyView::from(history_view)
            .cached(StyleRefinement::default().flex_1().min_h_0().min_w_0());
        let mut composer = div()
            .id("assistant-composer-content")
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
                div()
                    .id("assistant-question-click-target")
                    .flex_shrink_0()
                    .child(
                        Input::new(&self.inputs["question"])
                            .disabled(self.chat.running || self.chat.is_loading()),
                    )
                    .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                        let ClickEvent::Mouse(click) = event else {
                            return;
                        };
                        // Leave ordinary caret placement and drag selection to the editor.
                        if click.up.button != MouseButton::Left
                            || click.up.click_count != 2
                            || (click.up.position - click.down.position).magnitude() > 4.
                            || this.chat.running
                            || this.chat.is_loading()
                        {
                            return;
                        }
                        let selectable = this.inputs["question"].update(cx, |state, cx| {
                            if state.marked_text_range(window, cx).is_some() {
                                return false;
                            }
                            state.focus(window, cx);
                            true
                        });
                        if selectable {
                            window.dispatch_action(Box::new(gpui_component::input::SelectAll), cx);
                        }
                    })),
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
                                "Enter 换行 · ⌘ Enter 发送"
                            } else {
                                "Enter 换行 · Ctrl Enter 发送"
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
        let composer = div()
            .id("assistant-composer-scroll")
            .flex_shrink_0()
            .max_h(px(
                (f32::from(window.viewport_size().height) * 0.62).max(150.)
            ))
            .overflow_y_scroll()
            .child(composer);
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
            // The sidebar overlays selectable content with independent mouse hitboxes.
            .on_any_mouse_down(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .id("assistant-header-scroll")
                    .flex_shrink_0()
                    .max_h(px((f32::from(window.viewport_size().height) * 0.3).max(72.)))
                    .overflow_y_scroll()
                    .child(header),
            )
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
