use super::*;

impl AppView {
    pub(super) fn focus_control_label(&self) -> &'static str {
        if self.focus_mode.active() {
            "紧急结束"
        } else if self.timer.awaiting_focus() {
            "开始下一轮专注"
        } else if self.timer.is_running() {
            if self.timer.focus() {
                "暂停专注"
            } else {
                "暂停休息"
            }
        } else if self.timer.paused() {
            if self.timer.focus() {
                "继续专注"
            } else {
                "继续休息"
            }
        } else {
            "开始专注"
        }
    }

    pub(super) fn focus_control_primary_action(&self) -> Action {
        if self.focus_mode.active() {
            Action::FocusEmergency
        } else {
            Action::TimerPause
        }
    }

    pub(super) fn computer_focus_control(&self, cx: &Context<Self>) -> AnyElement {
        if !self.focus_mode.active() {
            return self.focus_mode_button(cx);
        }
        let primary: Hsla = if self.video.fullscreen {
            rgb(0x7dd8b5).into()
        } else {
            cx.theme().primary
        };
        div()
            .id("computer-focus-control")
            .flex()
            .items_center()
            .flex_wrap()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::BOLD)
                    .px_3()
                    .py_1()
                    .rounded_lg()
                    .border_1()
                    .border_color(primary.opacity(0.4))
                    .bg(primary.opacity(0.12))
                    .text_color(primary)
                    .child(format!(
                        "电脑专注中 · {} 退出",
                        focus_mode_ui::EXIT_SHORTCUT
                    )),
            )
            .child(self.focus_mode_button(cx))
            .into_any_element()
    }

    pub(super) fn focus_timer_bar(&self, cx: &Context<Self>) -> AnyElement {
        let fullscreen = self.video.fullscreen;
        let foreground = if fullscreen {
            rgb(0xe8f0eb).into()
        } else {
            cx.theme().foreground
        };
        let primary = if fullscreen {
            rgb(0x7dd8b5).into()
        } else {
            cx.theme().primary
        };
        let reached = self.timer.goal_met(focus_timer::now_ms());
        let status = if self.timer.awaiting_focus() {
            "休息已结束，等待开始"
        } else if reached {
            if self.timer.paused() {
                "已达标 · 已暂停"
            } else {
                "已达标 · 自由延长"
            }
        } else if self.timer.is_running() {
            "进行中"
        } else if self.timer.paused() {
            "已暂停"
        } else {
            "待开始"
        };
        let row = div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap_2()
            .child(design::icon("meditation", primary))
            .child(div().text_sm().child(if self.timer.focus() {
                "已专注"
            } else {
                "休息剩余"
            }))
            .child(
                div()
                    .min_w(px(54.))
                    .text_base()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(primary)
                    .child(clock(self.timer_seconds as f64)),
            )
            .when(self.timer.focus(), |row| {
                row.child(
                    div()
                        .text_xs()
                        .child(format!("最低目标 {} 分钟", self.timer.goal_seconds() / 60)),
                )
            })
            .child(div().text_xs().child(status))
            .when(self.timer.active_round(), |row| {
                row.child(
                    self.button(
                        "focus-add-distraction",
                        format!("分心 {} · +1", self.timer.distractions()),
                        Action::FocusDistraction,
                        cx,
                    )
                    .outline(),
                )
            })
            .when(
                self.timer.active_round() && (!self.focus_mode.active() || reached),
                |row| {
                    row.child(
                        self.button("focus-finish-round", "结束本轮", Action::FocusFinish, cx)
                            .outline(),
                    )
                },
            );
        div()
            .id("global-focus-timer")
            .flex_shrink_0()
            .min_w_0()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(if fullscreen {
                rgb(0x30463d).into()
            } else {
                cx.theme().border
            })
            .bg(if fullscreen {
                rgb(0x171f1c).into()
            } else {
                cx.theme().background
            })
            .text_color(foreground)
            .flex()
            .flex_col()
            .gap_2()
            .child(row)
            .when_some(
                self.timer.recent().first().filter(|r| r.review_pending),
                |bar, round| bar.child(self.focus_review(round, true, cx)),
            )
            .into_any_element()
    }

    fn focus_review(
        &self,
        round: &focus_timer::Round,
        prompt: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let mut row = div()
            .id(if prompt {
                "focus-review-prompt"
            } else {
                "focus-review-history"
            })
            .flex()
            .items_center()
            .flex_wrap()
            .gap_2()
            .child(div().text_sm().child(if prompt {
                format!(
                    "本轮 {} · 分心 {} 次 · 现在感觉如何？",
                    clock((round.actual_ms / 1000) as f64),
                    round.distractions
                )
            } else {
                "补记本轮疲劳".into()
            }));
        for (level, label) in [(1, "轻松"), (2, "一般"), (3, "疲劳")] {
            row = row.child(
                self.button(
                    format!("focus-fatigue-{level}"),
                    label,
                    Action::FocusReview(round.id.clone(), Some(level)),
                    cx,
                )
                .outline(),
            );
        }
        row.when(prompt, |row| {
            row.child(
                self.button(
                    "focus-review-later",
                    "稍后评价",
                    Action::FocusReview(round.id.clone(), None),
                    cx,
                )
                .ghost(),
            )
        })
        .into_any_element()
    }

    pub(super) fn focus_advice(&self) -> String {
        let minutes = self.timer.goal_seconds() / 60;
        match self.timer.recommendation() {
            focus_timer::Recommendation::NeedMore => format!(
                "记录 3 轮以 {minutes} 分钟为目标的专注，并评价每轮的疲劳程度，就能看到建议。"
            ),
            focus_timer::Recommendation::Stay => {
                format!("建议继续练习 {minutes} 分钟，先找到能稳定达标、较少分心的节奏。")
            }
            focus_timer::Recommendation::IncreaseTo(seconds) => format!(
                "最近 3 轮都达到了 {minutes} 分钟、每轮分心至多 1 次且感觉轻松。可以试试 {} 分钟。",
                seconds / 60
            ),
            focus_timer::Recommendation::AtMaximum => {
                "当前已是最高目标档位，可根据本轮感觉调整节奏。".into()
            }
        }
    }

    pub(super) fn focus_training(&self, cx: &Context<Self>) -> impl IntoElement {
        let mut view = panel(cx)
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("专注训练"),
            )
            .child(div().text_sm().child(self.focus_advice()));
        if let focus_timer::Recommendation::IncreaseTo(seconds) = self.timer.recommendation() {
            view = view.child(
                div().child(
                    self.button(
                        "focus-apply-suggestion",
                        format!("采用 {} 分钟目标", seconds / 60),
                        Action::Timer(seconds),
                        cx,
                    )
                    .outline()
                    .disabled(self.timer.active_round()),
                ),
            );
        }
        for (i, round) in self.timer.recent().iter().enumerate() {
            let date = chrono::DateTime::from_timestamp_millis(round.ended_ms as i64)
                .map(|date| {
                    date.with_timezone(&chrono::Local)
                        .format("%m-%d %H:%M")
                        .to_string()
                })
                .unwrap_or_default();
            let fatigue = match round.fatigue {
                Some(1) => "轻松",
                Some(2) => "一般",
                Some(3) => "疲劳",
                _ => "未评价",
            };
            view = view.child(
                div()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .pt_2()
                    .text_sm()
                    .child(format!(
                        "{date} · 实际 {} / 目标 {} 分钟 · 分心 {} 次 · {fatigue}",
                        clock((round.actual_ms / 1000) as f64),
                        round.goal_ms / 60_000,
                        round.distractions
                    ))
                    .when(
                        i == 0 && round.fatigue.is_none() && !round.review_pending,
                        |row| row.child(self.focus_review(round, false, cx)),
                    ),
            );
        }
        view
    }
}
