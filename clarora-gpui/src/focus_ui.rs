use super::*;

impl AppView {
    pub(super) fn focus_timer_toggle_label(&self) -> &'static str {
        if self.timer_end.is_some() {
            "暂停"
        } else if self.timer_paused {
            "继续"
        } else {
            "开始"
        }
    }

    pub(super) fn focus_timer_bar(&self, cx: &Context<Self>) -> AnyElement {
        let fullscreen = self.video_fullscreen;
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
        let button_style = || {
            ButtonCustomVariant::new(cx)
                .foreground(foreground)
                .hover(primary.opacity(0.12))
                .active(primary.opacity(0.2))
        };
        let view = cx.entity().downgrade();
        let duration = Button::new("global-focus-duration")
            .label("专注时长")
            .small()
            .ghost()
            .custom(button_style())
            .tooltip("选择时长，开始新一轮专注")
            .dropdown_menu_with_anchor(Corner::BottomRight, move |mut menu, _, _| {
                for minutes in [5u64, 10, 15, 25, 45, 60] {
                    let view = view.clone();
                    menu = menu.item(PopupMenuItem::new(format!("{minutes} 分钟")).on_click(
                        move |_, window, cx| {
                            let _ = view.update(cx, |this, cx| {
                                this.handle(Action::Timer(minutes * 60), window, cx)
                            });
                        },
                    ));
                }
                menu
            });
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
            .child(div().text_sm().child(if self.timer_focus {
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
            .child(div().text_xs().child(if self.timer_end.is_some() {
                "进行中"
            } else if self.timer_paused {
                "已暂停"
            } else {
                "待开始"
            }))
            .child(div().flex_1())
            .child(
                self.button(
                    "global-focus-toggle",
                    self.focus_timer_toggle_label(),
                    Action::TimerPause,
                    cx,
                )
                .custom(button_style()),
            )
            .child(duration)
            .child(
                self.button("global-focus-reset", "重置", Action::TimerReset, cx)
                    .ghost()
                    .custom(button_style())
                    .tooltip("回到 25 分钟，等待开始"),
            )
            .into_any_element()
    }
}
