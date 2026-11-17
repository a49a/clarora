//! Video subtitle controls and the in-place English reading view.
use super::*;
use crate::app_state::TranscriptSelection;

fn transcript_selected_bytes(
    selection: Option<TranscriptSelection>,
    index: usize,
    len: usize,
) -> Option<std::ops::Range<usize>> {
    let selection = selection?;
    let (start_paragraph, start_byte, end_paragraph, end_byte) =
        if selection.anchor <= selection.focus {
            (
                selection.anchor.0,
                selection.anchor.1,
                selection.focus.0,
                selection.focus.1,
            )
        } else {
            (
                selection.focus.0,
                selection.focus.1,
                selection.anchor.0,
                selection.anchor.1,
            )
        };
    if index < start_paragraph || index > end_paragraph {
        return None;
    }
    if start_paragraph == end_paragraph {
        return (start_byte < end_byte).then_some(start_byte..end_byte);
    }
    if index == start_paragraph {
        return (start_byte < len).then_some(start_byte..len);
    }
    if index == end_paragraph {
        return (end_byte > 0).then_some(0..end_byte);
    }
    Some(0..len)
}

fn transcript_runs(
    text: &str,
    selection: Option<TranscriptSelection>,
    index: usize,
    window: &Window,
    selection_color: Hsla,
) -> Vec<TextRun> {
    let style = window.text_style();
    let font = style.font();
    let selected = transcript_selected_bytes(selection, index, text.len());
    let mut runs = Vec::new();
    let mut push = |range: std::ops::Range<usize>, selected: bool| {
        let Some(slice) = text.get(range) else {
            return;
        };
        if slice.is_empty() {
            return;
        }
        runs.push(TextRun {
            len: slice.len(),
            font: font.clone(),
            color: style.color,
            background_color: selected.then_some(selection_color),
            underline: None,
            strikethrough: None,
        });
    };
    if let Some(selected) = selected {
        push(0..selected.start, false);
        push(selected.clone(), true);
        push(selected.end..text.len(), false);
    } else {
        push(0..text.len(), false);
    }
    runs
}

impl AppView {
    pub(super) fn video_subtitle_toolbar(&self, cx: &Context<Self>) -> AnyElement {
        let busy = self.tasks.foreground.is_running() || self.tasks.video_job.is_some();
        let no_media = !self.media.session.is_video() || self.media.session.path.is_none();
        let no_cues = self.video.cues.is_empty();
        let has_chinese = video_subtitles::has_chinese(&self.video.cues);
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
        let stronger = self.stronger_local_model();
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
                        self.button(
                            "video-translate-zh",
                            if has_chinese {
                                "重新翻译"
                            } else {
                                "翻译中文"
                            },
                            Action::VideoTranslate,
                            cx,
                        )
                        .outline()
                        .flex_1()
                        .min_w_0()
                        .h(px(32.))
                        .px_2()
                        .disabled(no_cues || busy || no_media)
                        .tooltip(if has_chinese {
                            "已有中文字幕；重新翻译会覆盖现有译文"
                        } else {
                            "将英文字幕翻译为中文，保留原文与时间轴"
                        }),
                    )
                    .child(
                        self.button(
                            "video-transcribe-stronger",
                            "更强模型",
                            Action::VideoTranscribeStronger,
                            cx,
                        )
                        .outline()
                        .flex_none()
                        .h(px(32.))
                        .px_2()
                        .disabled(stronger.is_none() || busy || no_media)
                        .tooltip(
                            stronger
                                .as_ref()
                                .map(|(model, _)| {
                                    format!(
                                        "用 {} 重新生成当前视频字幕，覆盖英文字幕和已有中文译文",
                                        model.name
                                    )
                                })
                                .unwrap_or_else(|| "请先在设置中下载更强的同语言本机模型".into()),
                        ),
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
                            if self.video.transcript_selection.is_some() {
                                "复制选中"
                            } else {
                                "复制英文全文"
                            },
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
                            if self.video.transcript_selection.is_some() {
                                "向选中提问"
                            } else {
                                "向全文提问"
                            },
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

    fn transcript_selection_bounds(&self) -> Option<((usize, usize), (usize, usize))> {
        let selection = self.video.transcript_selection?;
        Some(if selection.anchor <= selection.focus {
            (selection.anchor, selection.focus)
        } else {
            (selection.focus, selection.anchor)
        })
    }

    pub(super) fn transcript_selection_text(&self) -> Option<String> {
        let ((start_paragraph, start_byte), (end_paragraph, end_byte)) =
            self.transcript_selection_bounds()?;
        let paragraphs = &self.video.transcript.paragraphs;
        if start_paragraph >= paragraphs.len() || end_paragraph >= paragraphs.len() {
            return None;
        }
        if start_paragraph == end_paragraph {
            return paragraphs[start_paragraph]
                .get(start_byte..end_byte)
                .filter(|text| !text.trim().is_empty())
                .map(str::to_owned);
        }
        let mut parts = Vec::new();
        if let Some(text) = paragraphs[start_paragraph].get(start_byte..)
            && !text.trim().is_empty()
        {
            parts.push(text);
        }
        for paragraph in &paragraphs[start_paragraph + 1..end_paragraph] {
            if !paragraph.trim().is_empty() {
                parts.push(paragraph.as_str());
            }
        }
        if let Some(text) = paragraphs[end_paragraph].get(..end_byte)
            && !text.trim().is_empty()
        {
            parts.push(text);
        }
        let text = parts.join("\n\n");
        (!text.trim().is_empty()).then_some(text)
    }

    fn transcript_paragraph_at(&self, y: Pixels) -> Option<usize> {
        let top = self.video.transcript_scroll.logical_scroll_top().item_ix;
        for index in top..self.video.transcript.paragraphs.len() {
            let Some(bounds) = self.video.transcript_scroll.bounds_for_item(index) else {
                continue;
            };
            if y >= bounds.top() && y <= bounds.bottom() {
                return Some(index);
            }
            if bounds.top() > y {
                break;
            }
        }
        None
    }

    fn transcript_position_at(
        &self,
        position: Point<Pixels>,
        window: &Window,
    ) -> Option<(usize, usize)> {
        let paragraph = self.transcript_paragraph_at(position.y)?;
        let bounds = self.video.transcript_scroll.bounds_for_item(paragraph)?;
        let text = self.video.transcript.paragraphs.get(paragraph)?;
        let style = window.text_style();
        let run = TextRun {
            len: text.len(),
            font: style.font(),
            color: style.color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let lines = window
            .text_system()
            .shape_text(
                SharedString::from(text.clone()),
                px(16.),
                &[run],
                Some(bounds.size.width),
                None,
            )
            .ok()?;
        let line = lines.first()?;
        let local = point(position.x - bounds.left(), position.y - bounds.top());
        let byte = match line.index_for_position(local, px(24.)) {
            Ok(byte) | Err(byte) => byte,
        }
        .min(text.len());
        Some((paragraph, byte))
    }

    pub(super) fn video_transcript_view(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let paragraphs = self.video.transcript_paragraphs.clone();
        let selection = self.video.transcript_selection;
        let body = if self.video.transcript.text.is_empty() {
            div()
                .p_4()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("暂无可读取的英文全文。请先生成字幕，或从「更多」导入英文字幕。")
                .into_any_element()
        } else {
            let selection_color = cx.theme().selection;
            gpui::list(
                self.video.transcript_scroll.clone(),
                move |index, window, _cx| {
                    let paragraph = paragraphs.get(index).cloned().unwrap_or_default();
                    let runs =
                        transcript_runs(&paragraph, selection, index, window, selection_color);
                    div()
                        .w_full()
                        .pb_3()
                        .text_size(px(16.))
                        .line_height(px(24.))
                        .child(StyledText::new(paragraph).with_runs(runs))
                        .into_any_element()
                },
            )
            .size_full()
            .into_any_element()
        };
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
                    .relative()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .p_4()
                    .track_focus(&self.video.transcript_focus)
                    .key_context("VideoTranscript")
                    .on_action(cx.listener(|this, _: &CopySubtitleSelection, _, cx| {
                        if let Some(text) = this.transcript_selection_text() {
                            cx.write_to_clipboard(ClipboardItem::new_string(text));
                        }
                        cx.stop_propagation();
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, window, cx| {
                            if let Some(position) =
                                this.transcript_position_at(event.position, window)
                            {
                                this.assistant.last_source =
                                    crate::assistant_ui::SelectionSource::Transcript;
                                this.video.transcript_drag_anchor = Some(position);
                                this.video.transcript_selection = None;
                                window.focus(&this.video.transcript_focus);
                                cx.notify();
                            }
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                        if !event.dragging() {
                            return;
                        }
                        let Some(anchor) = this.video.transcript_drag_anchor else {
                            return;
                        };
                        let focus = this.transcript_position_at(event.position, window);
                        if let Some(focus) = focus {
                            this.video.transcript_selection =
                                (focus != anchor).then_some(TranscriptSelection { anchor, focus });
                            cx.notify();
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseUpEvent, _, cx| {
                            if this.video.transcript_drag_anchor.take().is_some()
                                && this
                                    .video
                                    .transcript_selection
                                    .is_some_and(|selection| selection.anchor == selection.focus)
                            {
                                this.video.transcript_selection = None;
                            }
                            cx.notify();
                        }),
                    )
                    .child(body)
                    .child(Scrollbar::vertical(&self.video.transcript_scroll)),
            )
            .into_any_element()
    }
}
