//! Video subtitle controls and the in-place English reading view.
use super::*;

impl AppView {
    pub(super) fn video_subtitle_toolbar(&self, cx: &Context<Self>) -> AnyElement {
        let busy = self.tasks.cancel.is_some() || self.tasks.video_job.is_some();
        let no_media = self.media.path.is_none();
        let no_cues = self.video.cues.is_empty();
        let no_english = self.video.transcript.text.is_empty();
        let scope = self
            .video
            .series
            .as_ref()
            .and_then(|series| {
                series.current.filter(|index| {
                    series
                        .parts
                        .get(*index)
                        .is_some_and(|part| part.path.is_some() && part.path == self.video.path)
                })
            })
            .map(|index| format!("当前 P{}", index + 1))
            .unwrap_or_else(|| "本视频".into());
        let collection = self
            .video
            .series
            .as_ref()
            .map(|series| format!("整个合集 · {} P", series.parts.len()))
            .unwrap_or_else(|| "整个合集".into());
        let more = vec![
            (
                "当前视频 · 导入英文字幕".into(),
                Action::Pick("video-subtitle".into()),
                false,
                no_media || busy,
            ),
            (
                "复制选中文字".into(),
                Action::CopySelectedSubtitle,
                false,
                !self.has_subtitle_selection(),
            ),
            (
                "当前视频 · 复制双语字幕".into(),
                Action::VideoCopySubtitle(None),
                false,
                no_cues,
            ),
            (
                "当前视频 · 导出 SRT".into(),
                Action::VideoSubtitleExport,
                false,
                no_cues,
            ),
            (
                format!("{collection} · 生成英文字幕"),
                Action::VideoTranscribeAll,
                false,
                self.video.series.is_none() || busy || no_media,
            ),
            (
                format!("{collection} · 翻译中文字幕"),
                Action::VideoTranslateAll,
                false,
                self.video.series.is_none() || busy,
            ),
        ];
        div()
            .id("video-subtitle-toolbar")
            .flex_shrink_0()
            .px_3()
            .py_2()
            .flex()
            .flex_col()
            .gap_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                self.button(
                                    "video-subtitles-mode-cues",
                                    "逐句字幕",
                                    Action::VideoTranscriptMode(false),
                                    cx,
                                )
                                .outline()
                                .selected(!self.video.transcript_open)
                                .h(px(28.))
                                .px_2(),
                            )
                            .child(
                                self.button(
                                    "video-subtitles-mode-english",
                                    "英文全文",
                                    Action::VideoTranscriptMode(true),
                                    cx,
                                )
                                .outline()
                                .selected(self.video.transcript_open)
                                .h(px(28.))
                                .px_2(),
                            ),
                    )
                    .when(!self.video.transcript_open, |view| {
                        view.child(
                            self.button(
                                "video-follow",
                                if self.video.follow {
                                    "跟随 ✓"
                                } else {
                                    "跟随"
                                },
                                Action::VideoFollow,
                                cx,
                            )
                            .outline()
                            .selected(self.video.follow)
                            .h(px(28.))
                            .px_2()
                            .tooltip("播放时将当前字幕句保持在列表中间"),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        self.button(
                            "video-transcribe",
                            if no_cues {
                                "生成字幕"
                            } else {
                                "重新生成"
                            },
                            Action::VideoTranscribe,
                            cx,
                        )
                        .outline()
                        .when(no_cues, |button| button.primary())
                        .flex_1()
                        .min_w_0()
                        .h(px(32.))
                        .px_2()
                        .disabled(busy || no_media),
                    )
                    .child(
                        self.button("video-translate-zh", "翻译中文", Action::VideoTranslate, cx)
                            .outline()
                            .flex_1()
                            .min_w_0()
                            .h(px(32.))
                            .px_2()
                            .disabled(no_cues || busy || no_media),
                    )
                    .child(self.surface_menu("video-subtitle-more", "更多 ▾".into(), more, cx)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        self.button(
                            "video-transcript-copy",
                            "复制英文全文",
                            Action::VideoCopyTranscript,
                            cx,
                        )
                        .outline()
                        .icon(Icon::default().path("ui/copy.svg").size(px(14.)))
                        .flex_1()
                        .min_w_0()
                        .h(px(32.))
                        .px_2()
                        .disabled(no_english)
                        .tooltip("复制当前视频的全部英文字幕，不含中文与时间戳"),
                    )
                    .child(
                        self.button(
                            "video-transcript-ask",
                            "向全文提问",
                            Action::VideoAskTranscript,
                            cx,
                        )
                        .primary()
                        .icon(Icon::default().path("ui/discover.svg").size(px(14.)))
                        .flex_1()
                        .min_w_0()
                        .h(px(32.))
                        .px_2()
                        .disabled(no_english)
                        .tooltip("将当前英文全文放入学习助手，检查引用后再发送问题"),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(if no_english {
                        format!("{scope} · 生成或导入英文字幕后可复制与提问")
                    } else {
                        format!(
                            "{scope} · 英文 {} 句 · 约 {} 词",
                            self.video.transcript.cue_count, self.video.transcript.word_count
                        )
                    }),
            )
            .into_any_element()
    }

    pub(super) fn video_transcript_view(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id("video-english-panel")
            .flex_1()
            .min_h_0()
            .w_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .id(("video-transcript-scroll", self.video.transcript_revision))
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_4()
                    .when(self.video.transcript.text.is_empty(), |view| {
                        view.child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("暂无可读取的英文全文。请先生成字幕，或从「更多」导入英文字幕。"),
                        )
                    })
                    .when(!self.video.transcript.text.is_empty(), |view| {
                        view.child(
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
                            .text_size(px(16.)),
                        )
                    }),
            )
            .into_any_element()
    }
}
