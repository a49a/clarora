//! A cached history viewport. Typing in the independent composer must not rebuild Markdown.
use super::*;
use clarora_gpui::assistant::{self, Reference};
#[cfg(debug_assertions)]
use std::{cell::Cell, rc::Rc};

struct Model {
    chat_id: String,
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
            chat_id: app.chat.id.clone(),
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
        self.chat_id == app.chat.id
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
    rendered: (String, usize, usize),
    #[cfg(debug_assertions)]
    builds: Rc<Cell<u64>>,
}
impl HistoryView {
    pub fn new(owner: WeakEntity<AppView>, app: &AppView) -> Self {
        Self {
            owner,
            model: Model::capture(app),
            scroll: app.assistant.scroll.clone(),
            rendered: Default::default(),
            #[cfg(debug_assertions)]
            builds: app.assistant.debug_history_builds.clone(),
        }
    }
    /// Called by the parent before including the child, never while child Render runs.
    /// Comparison is allocation-free and compares actual values, including equal-length replacements.
    pub fn sync(&mut self, app: &AppView, cx: &mut Context<Self>) {
        if self.model.matches(app) {
            return;
        }
        self.model = Model::capture(app);
        cx.notify();
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
impl Render for HistoryView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(debug_assertions)]
        self.builds.set(self.builds.get().saturating_add(1));
        let signature = (
            self.model.chat_id.clone(),
            self.model.history.len(),
            self.model.output.len(),
        );
        if signature != self.rendered {
            let follow = self.rendered.0 != signature.0
                || f32::from(self.scroll.offset().y).abs() + 60.
                    >= f32::from(self.scroll.max_offset().height);
            if follow {
                self.scroll.scroll_to_bottom();
            }
            self.rendered = signature;
        }
        let mut history = div()
            .id("assistant-history")
            .track_scroll(&self.scroll)
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .p_4()
            .flex()
            .flex_col()
            .gap_4()
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation());
        if self.model.history.is_empty() && self.model.pending.is_none() {
            history=history.child(div().py_6().flex().flex_col().gap_3()
                .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child("选一段文字，从这里问起"))
                .child(div().text_sm().text_color(cx.theme().muted_foreground).child("在字幕、电子书、闪卡或写作中拖选文字，点击浮出的「问 AI」。引用会先放在下方，确认后再发送。"))
                .child(div().text_sm().text_color(cx.theme().muted_foreground).child("也可以直接提问。切换栏目仍保留这段对话。")));
        }
        for (i, message) in self.model.history.iter().enumerate() {
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
                bubble = bubble.child(self.button(
                    format!("turn-ref-{i}"),
                    format!(
                        "{} 份引用 · {}",
                        refs.len(),
                        if self.model.expanded_turn == Some(i) {
                            "收起"
                        } else {
                            "查看原文"
                        }
                    ),
                    Action::AssistantTurnReference(i),
                    cx,
                ));
                if self.model.expanded_turn == Some(i) {
                    for (r, reference) in refs.iter().enumerate() {
                        bubble = bubble.child(self.quote(
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
                bubble = bubble.child(self.button(
                    format!("copy-answer-{i}"),
                    "复制回答",
                    Action::AssistantCopy(message["content"].as_str().unwrap_or_default().into()),
                    cx,
                ));
            }
            history = history.child(bubble);
        }
        if let Some((question, refs)) = &self.model.pending {
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
            for (i, reference) in refs.iter().enumerate() {
                pending =
                    pending.child(self.quote(format!("pending-ref-{i}"), reference, window, cx));
            }
            history = history.child(pending);
        }
        if self.model.running || (self.model.pending.is_some() && !self.model.error.is_empty()) {
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
                            .child(if self.model.running {
                                "✦ Clarora · 正在回答…"
                            } else {
                                "✦ Clarora · 未完成"
                            }),
                    )
                    .child(
                        TextView::markdown(
                            "assistant-stream",
                            if self.model.running {
                                if self.model.output.is_empty() {
                                    "正在等待回复…".into()
                                } else {
                                    self.model.output.clone()
                                }
                            } else {
                                format!(
                                    "以下为未完成的输出，未保存为回答：\n\n{}",
                                    self.model.failed_output
                                )
                            },
                            window,
                            cx,
                        )
                        .selectable(true),
                    ),
            );
        }
        if !self.model.reasoning.is_empty() {
            history = history.child(self.button(
                "toggle-reasoning",
                if self.model.reasoning_open {
                    "收起思考过程"
                } else {
                    "查看思考过程（非正式回答）"
                },
                Action::ToggleReasoning,
                cx,
            ));
            if self.model.reasoning_open {
                history = history.child(
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
        history
    }
}
