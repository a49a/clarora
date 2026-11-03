//! Frequent subtitle actions stay visible; occasional controls live in a short menu.
use super::*;

impl AppView {
    pub(super) fn audio_group_subtitle_actions(
        &self,
        location: &str,
        total: usize,
        cx: &Context<Self>,
    ) -> AnyElement {
        let disabled = self.tasks.cancel.is_some()
            || self.tasks.audio_subtitles.cancel.is_some()
            || self.audio.practice_id.is_empty()
            || total == 0;
        div()
            .id(SharedString::from(format!(
                "{location}-group-subtitle-actions"
            )))
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().accent.opacity(0.35))
            .px_3()
            .py_2()
            .flex()
            .items_center()
            .flex_wrap()
            .gap_2()
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!("整个组 · {total} 条音频")),
            )
            .child(
                self.button(
                    format!("{location}-group-english"),
                    "整组生成英文字幕",
                    Action::AudioGroupSubtitles(clarora_gpui::audio_batch::Mode::English),
                    cx,
                )
                .outline()
                .h(px(32.))
                .disabled(disabled)
                .tooltip("一键处理当前整个组，已有英文字幕跳过；不受搜索和筛选影响"),
            )
            .child(
                self.button(
                    format!("{location}-group-chinese"),
                    "整组翻译中文字幕",
                    Action::AudioGroupSubtitles(clarora_gpui::audio_batch::Mode::Chinese),
                    cx,
                )
                .outline()
                .h(px(32.))
                .disabled(disabled)
                .tooltip("只补缺少中文的句子，已有中文保留；没有英文时先自动生成英文"),
            )
            .into_any_element()
    }

    pub(super) fn audio_subtitle_toolbar(&self, cx: &Context<Self>) -> impl IntoElement {
        let available = !self.audio.current_id.is_empty();
        let busy = self.tasks.cancel.is_some() || self.tasks.audio_subtitles.cancel.is_some();
        let has_cues = !self.audio.cues.is_empty();
        let generating = self.tasks.audio_subtitles.working(
            &self.audio.current_id,
            clarora_gpui::audio_batch::Mode::English,
        );
        let translating = self.tasks.audio_subtitles.working(
            &self.audio.current_id,
            clarora_gpui::audio_batch::Mode::Chinese,
        );
        div()
            .id("audio-subtitle-toolbar")
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .px_4()
            .pb_3()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("当前音频"),
            )
            .child(
                self.button(
                    "audio-generate-subtitles",
                    if generating {
                        "生成中…"
                    } else if has_cues {
                        "重新生成字幕"
                    } else {
                        "生成字幕"
                    },
                    Action::Transcribe,
                    cx,
                )
                .primary()
                .h(px(32.))
                .disabled(!available || busy)
                .tooltip(if !available {
                    "请先选择音频"
                } else if busy {
                    "已有任务正在处理，可切换音频试听；完成或在顶部取消后可再次生成"
                } else if has_cues {
                    "重新识别音频并替换现有字幕"
                } else {
                    "识别当前音频，生成带时间轴的原文字幕"
                }),
            )
            .child(
                self.button(
                    "audio-translate-subtitles",
                    if translating {
                        "翻译中…"
                    } else {
                        "翻译成中文"
                    },
                    Action::Translate,
                    cx,
                )
                .outline()
                .h(px(32.))
                .disabled(!available || !has_cues || busy)
                .tooltip(if !available {
                    "请先选择音频"
                } else if busy {
                    "已有任务正在处理，可切换音频试听；完成或在顶部取消后可再次生成"
                } else if !has_cues {
                    "请先生成字幕，或从“更多”导入原文字幕"
                } else {
                    "翻译已有字幕，保留原文与时间轴"
                }),
            )
            .child(self.audio_subtitle_more(available, busy, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(if has_cues {
                        format!("已载入 {} 句字幕", self.audio.cues.len())
                    } else if available {
                        "先生成字幕，再翻译成中文".into()
                    } else {
                        "选择音频后生成字幕".into()
                    }),
            )
    }

    fn audio_subtitle_more(
        &self,
        available: bool,
        busy: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let view = cx.weak_entity();
        let current = self.audio.show_current;
        let font = self.audio.font_size;
        Button::new("audio-subtitle-options")
            .label("更多")
            .icon(IconName::Ellipsis)
            .small()
            .ghost()
            .h(px(32.))
            .tooltip("导入字幕、显示方式与字号")
            .dropdown_menu(move |mut menu, window, cx| {
                for (label, action) in [
                    ("导入英文字幕", Action::SubtitleImport(false)),
                    ("导入中文字幕", Action::SubtitleImport(true)),
                    ("刷新字幕", Action::Refresh),
                ] {
                    let view = view.clone();
                    menu = menu.item(
                        PopupMenuItem::new(label)
                            .disabled(!available || busy)
                            .on_click(move |_, window, cx| {
                                let _ = view
                                    .update(cx, |this, cx| this.handle(action.clone(), window, cx));
                            }),
                    );
                }
                let current_view = view.clone();
                let font_view = view.clone();
                menu.min_w(px(200.))
                    .separator()
                    .item(
                        PopupMenuItem::new("单独显示当前句")
                            .checked(current)
                            .on_click(move |_, window, cx| {
                                let _ = current_view.update(cx, |this, cx| {
                                    this.handle(Action::AudioCurrent, window, cx)
                                });
                            }),
                    )
                    .submenu(
                        format!("字幕字号 · {font}px"),
                        window,
                        cx,
                        move |mut menu, _, _| {
                            for size in [14, 16, 18, 20, 24, 28, 32] {
                                let view = font_view.clone();
                                menu = menu.item(
                                    PopupMenuItem::new(format!("{size}px"))
                                        .checked(font == size)
                                        .on_click(move |_, window, cx| {
                                            let _ = view.update(cx, |this, cx| {
                                                this.handle(Action::AudioFont(size), window, cx)
                                            });
                                        }),
                                );
                            }
                            menu.max_h(px(280.))
                        },
                    )
            })
    }
}
