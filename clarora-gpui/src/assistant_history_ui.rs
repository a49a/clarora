//! A cached history viewport. Typing in the independent composer must not rebuild Markdown.
use super::*;
use clarora_gpui::assistant::{self, Reference};
#[cfg(debug_assertions)]
use std::{cell::Cell, rc::Rc};

struct Model {
    context: String,
    chat_id: String,
    request: Option<clarora_gpui::chat_store::Request>,
    history: Vec<Value>,
    output: String,
    reasoning: String,
    reasoning_open: bool,
    running: bool,
    pending: Option<(String, Vec<Reference>)>,
    error: String,
    failed_output: String,
    expanded_turn: Option<usize>,
}
impl Model {
    fn capture(app: &AppView) -> Self {
        Self {
            context: app.chat.context.clone(),
            chat_id: app.chat.id.clone(),
            request: app.chat.current_request(),
            history: app.chat.history.clone(),
            output: app.chat.output.clone(),
            reasoning: app.chat.reasoning.clone(),
            reasoning_open: app.chat.reasoning_open,
            running: app.chat.running,
            pending: app.assistant.pending.clone(),
            error: app.assistant.error.clone(),
            failed_output: app.assistant.failed_output.clone(),
            expanded_turn: app.assistant.expanded_turn,
        }
    }
    fn matches(&self, app: &AppView) -> bool {
        self.context == app.chat.context
            && self.chat_id == app.chat.id
            && self.request == app.chat.current_request()
            && self.history == app.chat.history
            && self.output == app.chat.output
            && self.reasoning == app.chat.reasoning
            && self.reasoning_open == app.chat.reasoning_open
            && self.running == app.chat.running
            && self.pending == app.assistant.pending
            && self.error == app.assistant.error
            && self.failed_output == app.assistant.failed_output
            && self.expanded_turn == app.assistant.expanded_turn
    }
}
pub(super) struct HistoryView {
    owner: WeakEntity<AppView>,
    model: Model,
    scroll: ScrollHandle,
    show_conversation: bool,
    #[cfg(debug_assertions)]
    mode_bounds: Rc<Cell<Option<[Bounds<Pixels>; 2]>>>,
    #[cfg(debug_assertions)]
    builds: Rc<Cell<u64>>,
}
impl HistoryView {
    pub fn new(owner: WeakEntity<AppView>, app: &AppView) -> Self {
        Self {
            owner,
            model: Model::capture(app),
            scroll: app.assistant.scroll.clone(),
            show_conversation: false,
            #[cfg(debug_assertions)]
            mode_bounds: Default::default(),
            #[cfg(debug_assertions)]
            builds: app.assistant.debug_history_builds.clone(),
        }
    }
    /// Called by the parent before including the child, never while child Render runs.
    /// Compare actual values, including equal-length replacements.
    pub fn sync(&mut self, app: &AppView, cx: &mut Context<Self>) {
        if self.model.matches(app) {
            return;
        }
        let next = Model::capture(app);
        if self.model.chat_id != next.chat_id
            || self.model.context != next.context
            || (next.request.is_some() && self.model.request != next.request)
        {
            self.show_conversation = false;
            self.scroll.set_offset(Point::default());
        }
        self.model = next;
        cx.notify();
    }
    #[cfg(debug_assertions)]
    pub(super) fn mode_bounds(&self) -> Option<[Bounds<Pixels>; 2]> {
        self.mode_bounds.get()
    }
    fn button(
        &self,
        key: impl Into<SharedString>,
        label: impl Into<SharedString>,
        action: Action,
        cx: &Context<Self>,
    ) -> Button {
        let icon = match &action {
            Action::AssistantCopy(_) => Some("copy"),
            _ => None,
        };
        let owner = self.owner.clone();
        Button::new(ElementId::Name(key.into()))
            .label(label)
            .small()
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
            .border_color(cx.theme().primary.opacity(0.3))
            .when_some(icon, |button, icon| {
                button.icon(Icon::default().path(format!("ui/{icon}.svg")))
            })
            .on_click(move |_, window, cx| {
                let _ = owner.update(cx, |app, cx| app.handle(action.clone(), window, cx));
            })
    }
    fn quote(
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
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(reference.kind.label()),
            )
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
}
struct Turn<'a> {
    index: usize,
    question: Option<&'a str>,
    references: Vec<Reference>,
    answer: Option<&'a str>,
    status: &'static str,
    current: bool,
    complete: bool,
}

impl HistoryView {
    fn turn(
        &self,
        turn: Turn<'_>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut card = div()
            .id(("assistant-turn", turn.index))
            .p_4()
            .rounded_lg()
            .border_1()
            .border_color(if turn.current {
                cx.theme().primary.opacity(0.55)
            } else {
                cx.theme().border
            })
            .when(turn.current, |card| card.border_l_4())
            .bg(cx.theme().background)
            .flex()
            .flex_col()
            .gap_3();
        if let Some(question) = turn.question {
            let expanded = self.model.expanded_turn == Some(turn.index);
            let label = if expanded {
                "收起".into()
            } else if turn.references.is_empty() {
                "查看问题".into()
            } else {
                format!("问题与引用 · {}", turn.references.len())
            };
            card = card.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .truncate()
                            .child(format!(
                                "问题 · {}",
                                question.split_whitespace().collect::<Vec<_>>().join(" ")
                            )),
                    )
                    .child(
                        self.button(
                            format!("turn-ref-{}", turn.index),
                            label,
                            Action::AssistantTurnReference(turn.index),
                            cx,
                        )
                        .small()
                        .flex_shrink_0(),
                    ),
            );
            if expanded {
                card = card.child(
                    TextView::markdown(
                        ("assistant-question", turn.index),
                        question.to_string(),
                        window,
                        cx,
                    )
                    .selectable(true),
                );
                for (i, reference) in turn.references.iter().enumerate() {
                    card = card.child(self.quote(
                        format!("sent-ref-{}-{i}", turn.index),
                        reference,
                        window,
                        cx,
                    ));
                }
            }
        }
        card = card.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(cx.theme().primary)
                        .child(if turn.current {
                            "✦ 本轮回答"
                        } else {
                            "✦ Clarora · 回答"
                        }),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(turn.status),
                ),
        );
        if let Some(answer) = turn.answer.filter(|s| !s.is_empty()) {
            card = card.child(
                TextView::markdown(
                    ("assistant-answer", turn.index),
                    answer.to_string(),
                    window,
                    cx,
                )
                .selectable(true),
            );
            if turn.complete {
                card = card.child(
                    div().flex().child(
                        self.button(
                            format!("copy-answer-{}", turn.index),
                            "复制回答",
                            Action::AssistantCopy(answer.to_string()),
                            cx,
                        )
                        .small(),
                    ),
                );
            }
        }
        if turn.current && !self.model.reasoning.is_empty() {
            card = card.child(
                div().flex().child(
                    self.button(
                        "toggle-reasoning",
                        if self.model.reasoning_open {
                            "收起思考过程"
                        } else {
                            "查看思考过程"
                        },
                        Action::ToggleReasoning,
                        cx,
                    )
                    .small(),
                ),
            );
            if self.model.reasoning_open {
                card = card.child(
                    TextView::markdown(
                        "assistant-reasoning",
                        self.model.reasoning.clone(),
                        window,
                        cx,
                    )
                    .selectable(true),
                );
            }
        }
        card
    }
    fn view_button(&self, conversation: bool, cx: &mut Context<Self>) -> Button {
        let active = self.show_conversation == conversation;
        let owner = self.owner.clone();
        let history = cx.weak_entity();
        Button::new(if conversation {
            "assistant-view-conversation"
        } else {
            "assistant-view-current"
        })
        .label(if conversation {
            "整段对话"
        } else {
            "本轮回答 ↑"
        })
        .small()
        .outline()
        .when(active, |b| b.primary())
        .on_click(move |_, _, cx| {
            let _ = owner.update(cx, |app, cx| {
                if !conversation {
                    app.assistant.expanded_turn = None;
                }
                let _ = history.update(cx, |this, cx| {
                    this.show_conversation = conversation;
                    this.scroll.set_offset(Point::default());
                    this.sync(app, cx);
                    cx.notify();
                });
            });
        })
    }
}

impl Render for HistoryView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(debug_assertions)]
        self.builds.set(self.builds.get().saturating_add(1));
        let messages = &self.model.history;
        let end = messages.len();
        let latest = if self.model.pending.is_some() {
            end
        } else if end >= 2
            && messages[end - 2]["role"] == "user"
            && messages[end - 1]["role"] == "assistant"
        {
            end - 2
        } else {
            end.saturating_sub(1)
        };
        let mut history = div()
            .id("assistant-history")
            .track_scroll(&self.scroll)
            .flex_1()
            .w_full()
            .min_h_0()
            .overflow_y_scroll()
            .p_4()
            .flex()
            .flex_col()
            .gap_4()
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation());
        let empty = end == 0 && self.model.pending.is_none();
        if empty {
            history = history.child(div().py_6().flex().flex_col().gap_3()
                .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child("选一段文字，从这里问起"))
                .child(div().text_sm().text_color(cx.theme().muted_foreground).child("在字幕、电子书、闪卡或写作中拖选文字，点击浮出的「问 AI」。引用会先放在下方，确认后再发送。"))
                .child(div().text_sm().text_color(cx.theme().muted_foreground).child("也可以直接提问。切换栏目仍保留这段对话。")));
        }
        let mut i = if self.show_conversation { 0 } else { latest };
        while let Some(message) = messages.get(i) {
            let user = message["role"] == "user";
            let paired = user
                && messages
                    .get(i + 1)
                    .is_some_and(|m| m["role"] == "assistant");
            let answer = if paired {
                Some(&messages[i + 1])
            } else {
                (!user).then_some(message)
            };
            let mut references = assistant::references(message);
            if references.is_empty()
                && let Some(text) = message["reference"].as_str().filter(|s| !s.is_empty())
            {
                references.push(Reference {
                    text: assistant::plain_reference(text),
                    ..Default::default()
                });
            }
            history = history.child(self.turn(
                Turn {
                    index: i,
                    question: user.then(|| message["content"].as_str().unwrap_or_default()),
                    references,
                    answer: answer.and_then(|m| m["content"].as_str()),
                    status: if answer.is_some() {
                        "已完成"
                    } else {
                        "暂无回答"
                    },
                    current: i == latest,
                    complete: answer.is_some(),
                },
                window,
                cx,
            ));
            i += if paired { 2 } else { 1 };
        }
        if let Some((question, references)) = &self.model.pending {
            let (answer, status) = if !self.model.error.is_empty() {
                (self.model.failed_output.as_str(), "未完成")
            } else if self.model.output.is_empty() {
                ("", "等待回复")
            } else if self.model.running {
                (self.model.output.as_str(), "正在回答…")
            } else {
                (self.model.output.as_str(), "正在保存…")
            };
            history = history.child(self.turn(
                Turn {
                    index: end,
                    question: Some(question),
                    references: references.clone(),
                    answer: Some(answer),
                    status,
                    current: true,
                    complete: false,
                },
                window,
                cx,
            ));
        }
        let controls = div()
            .flex_shrink_0()
            .flex()
            .gap_2()
            .px_4()
            .py_2()
            .bg(cx.theme().muted)
            .border_b_1()
            .border_color(cx.theme().border)
            .child(self.view_button(false, cx))
            .child(self.view_button(true, cx));
        #[cfg(debug_assertions)]
        let controls = {
            let bounds = self.mode_bounds.clone();
            controls.on_children_prepainted(move |children, _, _| {
                bounds.set(children.try_into().ok());
            })
        };
        div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .when(!empty, |v| v.child(controls))
            .child(history)
    }
}
