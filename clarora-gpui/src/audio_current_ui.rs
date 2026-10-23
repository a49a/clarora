//! The listening focus card stays separate from the scrolling subtitle transcript.
use super::*;

impl AppView {
    pub(super) fn audio_current_cue(
        &self,
        active: Option<usize>,
        window: &Window,
        cx: &Context<Self>,
    ) -> AnyElement {
        let cue = active.and_then(|index| self.audio.cues.get(index));
        let (original, translation) = cue
            .map(|cue| subtitles::languages(std::slice::from_ref(cue)))
            .unwrap_or_else(|| ("·  ·  ·".into(), String::new()));
        let font = f32::from(self.audio.font_size);
        let body_height = (f32::from(window.viewport_size().height) * 0.24).clamp(80., 220.);
        div()
            .id("audio-current-card")
            .mx_4()
            .mt_4()
            .mb_3()
            .min_w_0()
            .flex_shrink_0()
            .rounded_xl()
            .border_1()
            .border_l_4()
            .border_color(cx.theme().primary)
            .bg(cx.theme().background)
            .shadow_md()
            .child(
                div()
                    .px_4()
                    .pt_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .py_1()
                            .rounded_full()
                            .bg(cx.theme().accent)
                            .text_color(cx.theme().primary)
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(design::icon("listening", cx.theme().primary))
                            .child(if cue.is_some() {
                                "当前句"
                            } else {
                                "等待下一句"
                            }),
                    )
                    .child(div().flex_1())
                    .when_some(cue.zip(active), |v, (cue, index)| {
                        v.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!(
                                    "{} – {}  ·  {} / {}",
                                    clock(cue.start),
                                    clock(cue.end),
                                    index + 1,
                                    self.audio.cues.len()
                                )),
                        )
                    }),
            )
            .child(
                div()
                    .relative()
                    .overflow_hidden()
                    .child(
                        div()
                            .id(("audio-current-content", active.unwrap_or(usize::MAX)))
                            .px_5()
                            .py_3()
                            .h(px((font * 4. + 40.).clamp(112., 180.).min(body_height)))
                            .overflow_y_scroll()
                            .track_scroll(&self.audio.current_scroll)
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .w_full()
                                    .flex_shrink_0()
                                    .max_w(px(1000.))
                                    .min_w_0()
                                    .text_center()
                                    .text_size(px((font + 8.).clamp(24., 36.)))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(if cue.is_some() {
                                        cx.theme().foreground
                                    } else {
                                        cx.theme().muted_foreground
                                    })
                                    .child(original),
                            )
                            .when(!translation.is_empty(), |v| {
                                v.child(
                                    div()
                                        .w_full()
                                        .flex_shrink_0()
                                        .max_w(px(1000.))
                                        .min_w_0()
                                        .text_center()
                                        .text_size(px((font + 2.).clamp(18., 24.)))
                                        .text_color(cx.theme().muted_foreground)
                                        .child(translation),
                                )
                            }),
                    )
                    .child(
                        div().absolute().top_0().left_0().size_full().child(
                            Scrollbar::vertical(&self.audio.current_scroll)
                                .scrollbar_show(ScrollbarShow::Always),
                        ),
                    ),
            )
            .into_any_element()
    }
}
