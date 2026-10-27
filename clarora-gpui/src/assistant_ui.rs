//! Chat rendering reads the cached conversation state only.
use super::*;

impl AppView {
    pub(super) fn assistant_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut history = div()
            .id("assistant-history")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_3();
        let mut sessions = div().flex().gap_2().flex_wrap().child(self.button(
            "chat-new",
            "新会话",
            Action::ChatSession(String::new()),
            cx,
        ));
        for (i, session) in self.chat.sessions.iter().enumerate() {
            sessions = sessions.child(self.button(
                format!("chat-session-{i}"),
                session.title.chars().take(18).collect::<String>(),
                Action::ChatSession(session.id.clone()),
                cx,
            ));
        }
        if !self.chat.error.is_empty() {
            history = history.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child(format!("会话读取失败：{}", self.chat.error)),
            );
        }
        history = history.child(sessions);
        for (i, message) in self.chat.history.iter().enumerate() {
            history = history.child(
                panel(cx)
                    .bg(if message["role"] == "user" {
                        cx.theme().muted
                    } else {
                        cx.theme().background
                    })
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(cx.theme().primary)
                            .child(if message["role"] == "user" {
                                "你"
                            } else {
                                "✦ Clarora"
                            }),
                    )
                    .child(TextView::markdown(
                        ("chat-message", i),
                        message["content"].as_str().unwrap_or("").to_string(),
                        window,
                        cx,
                    )),
            );
        }
        if !self.chat.reasoning.is_empty() {
            history = history.child(self.button(
                "toggle-reasoning",
                if self.chat.reasoning_open {
                    "收起推理内容"
                } else {
                    "查看推理内容"
                },
                Action::ToggleReasoning,
                cx,
            ));
            if self.chat.reasoning_open {
                history = history.child(TextView::markdown(
                    "ai-reasoning",
                    self.chat.reasoning.clone(),
                    window,
                    cx,
                ));
            }
        }
        if self.tasks.cancel.is_some() || self.chat.history.is_empty() {
            history = history.child(TextView::markdown(
                "ai-output",
                self.chat.output.clone(),
                window,
                cx,
            ));
        }
        div()
            .w(px(400.))
            .max_w(relative(0.45))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_4()
            .p_5()
            .bg(cx.theme().background)
            .border_l_1()
            .border_color(cx.theme().border)
            .when(self.page == "video", |v| {
                v.w_full().max_w(relative(1.)).border_l_0().p_3().gap_2()
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .items_center()
                            .child(design::icon("discover", cx.theme().primary))
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("学习助手"),
                            ),
                    )
                    .child(
                        self.button("close-assistant", "收起", Action::Assistant, cx)
                            .ghost(),
                    ),
            )
            .child(
                div()
                    .p_3()
                    .rounded_lg()
                    .bg(cx.theme().muted)
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(if self.reference(cx).is_empty() {
                        "围绕当前学习材料，随时提问。".to_string()
                    } else {
                        self.reference(cx).chars().take(140).collect::<String>()
                    }),
            )
            .when(self.page == "video", |v| {
                v.child(
                    div()
                        .flex()
                        .gap_2()
                        .flex_wrap()
                        .child(
                            self.button(
                                "ai-video-current",
                                "引用当前句",
                                Action::VideoAsk(video_qa::Scope::Current),
                                cx,
                            )
                            .disabled(self.chat.running),
                        )
                        .child(
                            self.button(
                                "ai-video-all",
                                "引用整段字幕",
                                Action::VideoAsk(video_qa::Scope::All),
                                cx,
                            )
                            .disabled(self.chat.running || self.video.cues.is_empty()),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(
                        self.video
                            .material
                            .as_ref()
                            .map(|m| {
                                format!(
                                    "{} · 引用已固定，播放不会改变本次材料。使用设置中的聊天 API。",
                                    m.label
                                )
                            })
                            .unwrap_or_else(|| {
                                if self.video.material_error.is_empty() {
                                    "请先生成或加载字幕，再选择引用范围。".into()
                                } else {
                                    self.video.material_error.clone()
                                }
                            }),
                    ),
                )
            })
            .when(self.chat.is_loading(), |v| {
                v.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("正在读取材料会话…"),
                )
            })
            .child(self.field("question", "向当前材料提问"))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        self.button("ask", "发送", Action::Chat, cx)
                            .primary()
                            .disabled(
                                self.tasks.cancel.is_some()
                                    || self.chat.is_loading()
                                    || (matches!(self.page.as_str(), "flashcard" | "discover")
                                        && !self.cards_ready())
                                    || (self.page == "video" && self.video.material.is_none()),
                            ),
                    )
                    .child(
                        self.button("cancel-ai", "取消", Action::Cancel, cx)
                            .disabled(self.tasks.cancel.is_none()),
                    )
                    .child(
                        self.button("save-answer", "存为问答卡", Action::SaveAnswer, cx)
                            .disabled(
                                self.tasks.cancel.is_some()
                                    || self.chat.is_loading()
                                    || self.chat.context != self.chat_context_key(cx)
                                    || self.chat.history.is_empty(),
                            ),
                    ),
            )
            .child(history)
            .into_any_element()
    }
}
