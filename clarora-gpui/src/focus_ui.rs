use super::*;

impl AppView {
    pub(super) fn focus_control_label(&self) -> &'static str {
        if self.focus_mode.active() {
            "紧急结束"
        } else if cfg!(target_os = "macos") && self.timer.strict() && self.timer.is_running() {
            "继续电脑专注"
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
        } else if cfg!(target_os = "macos") && self.timer.strict() && self.timer.is_running() {
            Action::FocusResume
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
                    .child("电脑专注中"),
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
            .items_center()
            .flex_wrap()
            .gap_2()
            .child(design::icon("meditation", primary))
            .child(div().text_sm().child(if self.timer.awaiting_focus() {
                "下一轮专注"
            } else if self.timer.focus() {
                "专注时间"
            } else {
                "休息时间"
            }))
            .child(
                div()
                    .min_w(px(54.))
                    .flex_shrink_0()
                    .text_base()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(primary)
                    .child(clock(self.timer_remaining as f64)),
            )
            .child(div().text_xs().child(if self.timer.awaiting_focus() {
                "休息已结束，等待开始"
            } else if self.timer.is_running() {
                "进行中"
            } else if self.timer.paused() {
                "已暂停"
            } else {
                "待开始"
            }))
            .into_any_element()
    }
}
