//! Subtitle markings use cached card data; cancellation removes the actual review card.
use super::*;
impl AppView {
    pub(super) fn refresh_saved_ranges(&mut self) {
        let audio = self
            .audio
            .audios
            .iter()
            .find(|r| text(r, "id") == self.audio.current_id)
            .map(|r| text(r, "audio_uri"))
            .unwrap_or_default();
        self.audio_saved = self.clip_catalog.ranges(
            &clip_marks::Source::audio(&self.audio.current_id),
            &self.audio.cues,
            &audio,
        );
        self.video_saved = self
            .video
            .path
            .as_ref()
            .map(|p| {
                self.clip_catalog.ranges(
                    &clip_marks::Source::video(p),
                    &self.video.cues,
                    &p.to_string_lossy(),
                )
            })
            .unwrap_or_default();
        self.subtitle_text.borrow_mut().documents(
            clarora_gpui::subtitle_selection::Document::new(
                self.audio.current_id.clone(),
                &self.audio.cues,
            ),
            clarora_gpui::subtitle_selection::Document::new(
                self.video
                    .path
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                &self.video.cues,
            ),
        );
    }
    pub(super) fn saved_at(&self, audio: bool, start: f64, end: f64) -> Vec<clip_marks::Saved> {
        (if audio {
            &self.audio_saved
        } else {
            &self.video_saved
        })
        .iter()
        .filter(|r| r.overlaps(start, end))
        .map(|r| r.saved.clone())
        .collect()
    }
    pub(super) fn show_saved_clips(
        &mut self,
        saved: Vec<clip_marks::Saved>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if saved.is_empty() {
            return;
        }
        let view = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, window, _| {
            let mut body = div()
                .id("saved-clips-list")
                .max_h((window.viewport_size().height - px(220.)).max(px(100.)))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap_3();
            for card in &saved {
                let weak = view.clone();
                let target = card.clone();
                body = body.child(
                    div()
                        .p_3()
                        .rounded_lg()
                        .border_1()
                        .border_color(rgb(0x85a99b))
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(div().text_sm().child(card.front.clone()))
                        .child(
                            Button::new(SharedString::from(format!(
                                "unsave-{}-{}",
                                card.kind, card.id
                            )))
                            .label("取消收藏复习")
                            .danger()
                            .on_click(move |_, window, cx| {
                                let removed = weak
                                    .update(cx, |this, cx| {
                                        match clip_marks::remove(&this.lib, &target) {
                                            Ok(()) => {
                                                this.refresh();
                                                this.report_status(
                                                    FeedbackKind::Info,
                                                    "已取消片段收藏复习，原音频和字幕保留".into(),
                                                );
                                                cx.notify();
                                                true
                                            }
                                            Err(e) => {
                                                this.report_status(
                                                    FeedbackKind::Error,
                                                    format!("取消失败：{e:#}"),
                                                );
                                                cx.notify();
                                                false
                                            }
                                        }
                                    })
                                    .unwrap_or(true);
                                if removed {
                                    window.close_dialog(cx);
                                }
                            }),
                        ),
                );
            }
            dialog
                .title("已收藏的片段")
                .w((window.viewport_size().width - px(32.)).min(px(520.)))
                .child(
                    div()
                        .text_sm()
                        .child("选择要取消的片段，将同时移出闪卡复习。媒体文件与字幕会保留。"),
                )
                .child(body)
        });
    }
    /// Shared marked original for the focus card and video transcript. Hidden blind text never calls this.
    pub(super) fn marked_cue(
        &self,
        audio: bool,
        index: usize,
        cue: &Cue,
        font: f32,
        center: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let selecting = audio && self.audio.clip_mode && !self.audio.blind.enabled;
        let edges = selecting
            .then(|| {
                subtitles::selection_edges(&self.audio.cues, self.media.loop_a, self.media.loop_b)
            })
            .flatten();
        let mut line = div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_wrap()
            .gap_x_1()
            .gap_y_1()
            .when(center, |v| v.justify_center());
        for (j, word) in subtitles::words(cue).into_iter().enumerate() {
            let saved = self.saved_at(audio, word.start, word.end);
            let marked = !saved.is_empty();
            line = line.child(
                div()
                    .id((
                        if audio {
                            "focus-saved-word"
                        } else {
                            "video-saved-word"
                        },
                        index * 100000 + j,
                    ))
                    .relative()
                    .when(selecting, |v| v.py(px(14.)))
                    .when(
                        selecting
                            && self
                                .media
                                .loop_a
                                .zip(self.media.loop_b)
                                .is_some_and(|(a, b)| word.start < b && word.end > a),
                        |v| v.bg(cx.theme().accent).rounded_sm(),
                    )
                    .when_some(
                        edges.filter(|e| e.start.cue == index && e.start.word == j),
                        |v, e| v.child(clip_selection_ui::text_edge(e.start, true)),
                    )
                    .when_some(
                        edges.filter(|e| e.end.cue == index && e.end.word == j),
                        |v, e| v.child(clip_selection_ui::text_edge(e.end, false)),
                    )
                    .text_size(px(font))
                    .child(self.subtitle_fragment(
                        if audio {
                            subtitle_text_ui::Surface::Focus
                        } else {
                            subtitle_text_ui::Surface::Video
                        },
                        index,
                        Some(j),
                        cx,
                    ))
                    .when(audio, |v| {
                        v.cursor_pointer().on_click(cx.listener(
                            move |this, event: &ClickEvent, window, cx| {
                                cx.stop_propagation();
                                if this.subtitle_dragged() {
                                    return;
                                }
                                this.handle(
                                    Action::Word(
                                        word.start,
                                        word.end,
                                        word.text.clone(),
                                        event.modifiers().shift,
                                    ),
                                    window,
                                    cx,
                                );
                            },
                        ))
                    })
                    .when(marked, |v| {
                        v.underline()
                            .cursor_pointer()
                            .text_color(cx.theme().primary)
                            .tooltip(|window, cx| {
                                Tooltip::new("已加入闪卡 · 右键管理收藏复习").build(window, cx)
                            })
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.handle(Action::SavedClips(saved.clone()), window, cx);
                                }),
                            )
                    }),
            );
        }
        line.into_any_element()
    }
}
