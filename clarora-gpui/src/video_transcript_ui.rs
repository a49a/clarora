use super::*;

impl AppView {
    pub(super) fn video_transcript_view(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        panel(cx)
            .w_full()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::BOLD)
                            .child("英文字幕文稿"),
                    )
                    .child(div().flex_1())
                    .child(
                        self.button(
                            "video-transcript-copy",
                            "复制英文全文",
                            Action::VideoCopyTranscript,
                            cx,
                        )
                        .disabled(self.video.transcript.text.is_empty()),
                    )
                    .child(
                        self.button(
                            "video-transcript-close",
                            "收起文稿",
                            Action::VideoTranscript,
                            cx,
                        )
                        .ghost(),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("整篇英文 · 无时间戳与中文翻译 · 可选中文字复制"),
            )
            .child(
                div()
                    .id(("video-transcript-scroll", self.video.transcript_revision))
                    .w_full()
                    .h(px(440.))
                    .overflow_y_scroll()
                    .p_4()
                    .bg(cx.theme().muted)
                    .rounded_lg()
                    .when(self.video.transcript.text.is_empty(), |v| {
                        v.child(div().text_color(cx.theme().muted_foreground).child(
                            "暂无独立英文原文，请先导入英文字幕或生成字幕。双语字幕仅显示英文行。",
                        ))
                    })
                    .when(!self.video.transcript.text.is_empty(), |v| {
                        v.child(
                            TextView::html(
                                SharedString::from(format!(
                                    "video-english-transcript-{}",
                                    self.video.transcript_revision
                                )),
                                self.video.transcript_html.clone(),
                                window,
                                cx,
                            )
                            .selectable(true)
                            .text_size(px(18.)),
                        )
                    }),
            )
            .into_any_element()
    }
}
