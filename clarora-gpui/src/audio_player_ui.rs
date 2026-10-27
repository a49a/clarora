//! Audio workspace renders the current playback and library snapshots.
use super::*;

impl AppView {
    pub(super) fn waveform_view(&self, cx: &Context<Self>) -> AnyElement {
        let Some(waveform) = &self.audio.waveform else {
            return div()
                .text_sm()
                .child(if self.audio.waveform_rx.is_some() {
                    "正在读取真实音频波形…"
                } else {
                    "选择音频后显示波形"
                })
                .into_any_element();
        };
        let peaks = waveform.peaks.clone();
        let duration = waveform.duration;
        let position = self.media.snapshot.position;
        let a = self.media.loop_a;
        let b = self.media.loop_b;
        let bounds_cell = self.audio.waveform_bounds.clone();
        let color = cx.theme().primary;
        div()
            .w_full()
            .child(
                div()
                    .id("audio-waveform")
                    .relative()
                    .w_full()
                    .h(px(48.))
                    .cursor_pointer()
                    .child(
                        canvas(
                            move |bounds, _, _| bounds_cell.set(bounds),
                            move |bounds, _, window, _| {
                                let width = f32::from(bounds.size.width);
                                let height = f32::from(bounds.size.height);
                                if let (Some(a), Some(b)) = (a, b) {
                                    window.paint_quad(fill(
                                        Bounds::new(
                                            point(
                                                bounds.left() + px((a / duration) as f32 * width),
                                                bounds.top(),
                                            ),
                                            size(
                                                px(((b - a) / duration) as f32 * width),
                                                bounds.size.height,
                                            ),
                                        ),
                                        rgba(0x36a58022),
                                    ));
                                }
                                let step = width / peaks.len().max(1) as f32;
                                for (i, peak) in peaks.iter().enumerate() {
                                    let h = (peak * height * 0.8).max(1.);
                                    window.paint_quad(fill(
                                        Bounds::new(
                                            point(
                                                bounds.left() + px(i as f32 * step),
                                                bounds.top() + px((height - h) / 2.),
                                            ),
                                            size(px(step.max(1.) * 0.7), px(h)),
                                        ),
                                        color,
                                    ));
                                }
                                for (time, color) in [
                                    (Some(position), rgb(0xe87240)),
                                    (a, clip_selection_ui::edge_color(true)),
                                    (b, clip_selection_ui::edge_color(false)),
                                ] {
                                    if let Some(time) = time {
                                        window.paint_quad(fill(
                                            Bounds::new(
                                                point(
                                                    bounds.left()
                                                        + px((time / duration).clamp(0., 1.)
                                                            as f32
                                                            * width),
                                                    bounds.top(),
                                                ),
                                                size(px(2.), bounds.size.height),
                                            ),
                                            color,
                                        ));
                                    }
                                }
                            },
                        )
                        .size_full(),
                    )
                    .when_some(a, |v, time| {
                        v.child(clip_selection_ui::waveform_edge(time, duration, true))
                    })
                    .when_some(b, |v, time| {
                        v.child(clip_selection_ui::waveform_edge(time, duration, false))
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, window, cx| {
                            let bounds = this.audio.waveform_bounds.get();
                            if bounds.size.width <= px(0.) {
                                return;
                            }
                            let duration = this
                                .audio
                                .waveform
                                .as_ref()
                                .map(|w| w.duration)
                                .unwrap_or(0.);
                            let time = ((event.position.x - bounds.left()) / bounds.size.width)
                                .clamp(0., 1.) as f64
                                * duration;
                            let threshold = duration * 8. / f64::from(f32::from(bounds.size.width));
                            this.audio.waveform_drag = if !this.audio.clip_mode {
                                None
                            } else if event.modifiers.shift {
                                Some(true)
                            } else if event.modifiers.alt {
                                Some(false)
                            } else if this
                                .media
                                .loop_a
                                .is_some_and(|a| (a - time).abs() < threshold)
                            {
                                Some(true)
                            } else if this
                                .media
                                .loop_b
                                .is_some_and(|b| (b - time).abs() < threshold)
                            {
                                Some(false)
                            } else {
                                None
                            };
                            this.handle(
                                if let Some(start) = this.audio.waveform_drag {
                                    Action::Boundary(time, start)
                                } else {
                                    Action::SeekTo(time)
                                },
                                window,
                                cx,
                            );
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                        if !event.dragging() {
                            return;
                        }
                        let bounds = this.audio.waveform_bounds.get();
                        if bounds.size.width <= px(0.) {
                            return;
                        }
                        let duration = this
                            .audio
                            .waveform
                            .as_ref()
                            .map(|w| w.duration)
                            .unwrap_or(0.);
                        let time = ((event.position.x - bounds.left()) / bounds.size.width)
                            .clamp(0., 1.) as f64
                            * duration;
                        this.handle(
                            if let Some(start) = this.audio.waveform_drag {
                                Action::Boundary(time, start)
                            } else {
                                Action::SeekTo(time)
                            },
                            window,
                            cx,
                        );
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, _| this.audio.waveform_drag = None),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this, _, _, _| this.audio.waveform_drag = None),
                    ),
            )
            .into_any_element()
    }
    pub(super) fn blind_panel(&self, cx: &Context<Self>) -> AnyElement {
        let ready = !self.audio.current_id.is_empty()
            && self.tasks.cancel.is_none()
            && self.audio.blind_pending.is_none()
            && self
                .audio
                .blind_settle_until
                .is_none_or(|t| Instant::now() >= t)
            && self
                .media
                .path
                .as_ref()
                .is_some_and(|p| p.to_string_lossy() == self.media.snapshot.path);
        let target = blind_listening::resolve(
            &self.audio.cues,
            self.media.snapshot.position,
            self.media.snapshot.duration,
        );
        let loop_range = self
            .media
            .loop_a
            .zip(self.media.loop_b)
            .filter(|(a, b)| self.media.repeat == 2 && b > a);
        let can_sentence = |direction| {
            ready
                && blind_listening::sentence_target(
                    &self.audio.cues,
                    self.media.snapshot.position,
                    self.media.snapshot.duration,
                    self.audio.blind.revealed.as_ref(),
                    direction,
                    loop_range,
                )
                .is_some()
        };
        div()
            .id("blind-listening-controls")
            .flex()
            .flex_col()
            .gap_2()
            .px_5()
            .py_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        self.player_menu(
                            "blind-caption-display",
                            if self.audio.blind_chinese {
                                "查看：仅中文 ▾"
                            } else {
                                "查看：中英字幕 ▾"
                            }
                            .into(),
                            vec![
                                (
                                    "中英字幕".into(),
                                    Action::AudioBlindChinese(false),
                                    !self.audio.blind_chinese,
                                    false,
                                ),
                                (
                                    "仅中文".into(),
                                    Action::AudioBlindChinese(true),
                                    self.audio.blind_chinese,
                                    false,
                                ),
                            ],
                            cx,
                        ),
                    )
                    .child(
                        self.button("blind-back-five", "回退 5 秒", Action::Seek(-5.), cx)
                            .disabled(!ready),
                    )
                    .child(
                        self.button(
                            "blind-previous-sentence",
                            "上一句 ↑",
                            Action::AudioBlindSentence(-1),
                            cx,
                        )
                        .disabled(!can_sentence(-1)),
                    )
                    .child(
                        self.button(
                            "blind-repeat-sentence",
                            "从句首重听 R",
                            Action::AudioBlindSentence(0),
                            cx,
                        )
                        .disabled(!can_sentence(0)),
                    )
                    .child(
                        self.button(
                            "blind-next-sentence",
                            "下一句 ↓",
                            Action::AudioBlindSentence(1),
                            cx,
                        )
                        .disabled(!can_sentence(1)),
                    )
                    .child(
                        self.button(
                            "blind-reveal",
                            if self.audio.blind_pending.is_some() {
                                "正在暂停…"
                            } else if self.audio.blind.revealed.is_some() {
                                "收起文字 H"
                            } else if self.audio.blind_chinese {
                                "看中文 H"
                            } else {
                                "看刚才一句 H"
                            },
                            if self.audio.blind.revealed.is_some() {
                                Action::AudioBlindHide
                            } else {
                                Action::AudioBlindReveal
                            },
                            cx,
                        )
                        .primary()
                        .disabled(
                            !ready || (self.audio.blind.revealed.is_none() && target.is_none()),
                        ),
                    )
                    .when(self.audio.blind.revealed.is_some(), |v| {
                        v.child(
                            self.button(
                                "blind-resume",
                                "继续盲听 C",
                                Action::AudioBlindResume(false),
                                cx,
                            )
                            .disabled(!ready),
                        )
                    })
                    .when(self.media.snapshot.eof, |v| {
                        v.child(
                            self.button(
                                "blind-restart",
                                "从头再听",
                                Action::AudioBlindResume(true),
                                cx,
                            )
                            .disabled(!ready),
                        )
                    })
                    .child(
                        self.button("blind-full", "查看全文", Action::AudioBlind(false), cx)
                            .ghost()
                            .disabled(self.tasks.cancel.is_some()),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(if self.audio.cues.is_empty() {
                        "暂无字幕，仍可盲听；切回字幕模式可导入字幕。"
                    } else if self.audio.blind_chinese {
                        "查看时仅显示刚才一句的中文，英文保持遮挡；继续播放后重新遮住中文。"
                    } else if self.audio.blind.revealed.as_ref().is_some_and(|r| r.sparse) {
                        "已显示上一条字幕；继续播放会自动遮住文字。"
                    } else {
                        "点击词块从对应单词播放，点击时间戳从句首播放；词时间为估算。"
                    }),
            )
            .into_any_element()
    }
    pub(super) fn audio_study(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let group = self
            .audio
            .practices
            .iter()
            .find(|r| text(r, "id") == self.audio.practice_id)
            .map(|r| text(r, "name"))
            .unwrap_or_else(|| "选择练习组".into());
        let audios = self
            .audio
            .audios
            .iter()
            .filter(|r| text(r, "practice_id") == self.audio.practice_id)
            .collect::<Vec<_>>();
        let index = audios
            .iter()
            .position(|r| text(r, "id") == self.audio.current_id);
        let title = index
            .map(|i| text(audios[i], "name"))
            .unwrap_or_else(|| "选择音频".into());
        let group_options = self
            .audio
            .practices
            .iter()
            .map(|r| {
                let key = text(r, "id");
                (
                    text(r, "name"),
                    Action::AudioPractice(key.clone()),
                    key == self.audio.practice_id,
                    self.tasks.cancel.is_some(),
                )
            })
            .collect();
        let audio_options = audios
            .iter()
            .enumerate()
            .map(|(i, r)| {
                (
                    format!("{}  {}", i + 1, text(r, "name")),
                    Action::Audio(text(r, "id")),
                    Some(i) == index,
                    self.tasks.cancel.is_some(),
                )
            })
            .collect();
        let previous = index
            .and_then(|i| i.checked_sub(1))
            .map(|i| text(audios[i], "id"));
        let next = index
            .filter(|i| i + 1 < audios.len())
            .map(|i| text(audios[i + 1], "id"));
        let page = index
            .map(|i| format!("{} / {}", i + 1, audios.len()))
            .unwrap_or_else(|| format!("0 / {}", audios.len()));
        let available = !self.audio.current_id.is_empty();
        let navigation = div()
            .flex()
            .items_center()
            .gap_2()
            .flex_wrap()
            .px_4()
            .py_3()
            .child(
                self.button("audio-subtitle-mode", "字幕", Action::AudioBlind(false), cx)
                    .selected(!self.audio.blind.enabled)
                    .disabled(self.tasks.cancel.is_some()),
            )
            .child(
                self.button("audio-blind-mode", "盲听", Action::AudioBlind(true), cx)
                    .selected(self.audio.blind.enabled)
                    .disabled(self.tasks.cancel.is_some()),
            )
            .child(self.player_menu(
                "audio-practice-picker",
                format!("{} ▾", group.chars().take(20).collect::<String>()),
                group_options,
                cx,
            ))
            .child(self.player_menu(
                "audio-picker",
                format!("{} ▾", title.chars().take(24).collect::<String>()),
                audio_options,
                cx,
            ))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(page),
            )
            .child(
                self.button(
                    "audio-previous",
                    "‹",
                    Action::Audio(previous.clone().unwrap_or_default()),
                    cx,
                )
                .disabled(previous.is_none() || self.tasks.cancel.is_some()),
            )
            .child(
                self.button(
                    "audio-next",
                    "›",
                    Action::Audio(next.clone().unwrap_or_default()),
                    cx,
                )
                .disabled(next.is_none() || self.tasks.cancel.is_some()),
            )
            .child(div().flex_1())
            .child(
                self.button(
                    "audio-follow",
                    if self.audio.follow {
                        "跟随 ✓"
                    } else {
                        "跟随"
                    },
                    Action::AudioFollow,
                    cx,
                )
                .ghost(),
            )
            .when(!self.audio.blind.enabled, |v| {
                v.child(
                    self.button(
                        "audio-copy-selected",
                        "复制选中",
                        Action::CopySelectedSubtitle,
                        cx,
                    )
                    .ghost()
                    .disabled(!self.has_subtitle_selection()),
                )
                .child(
                    self.button(
                        "audio-copy-all",
                        "复制全部",
                        Action::AudioCopySubtitle(self.audio.current_id.clone(), None),
                        cx,
                    )
                    .ghost()
                    .disabled(self.audio.cues.is_empty()),
                )
            })
            .when(self.audio.blind.enabled, |v| {
                v.child(
                    self.button(
                        "audio-manage-subtitles",
                        "管理字幕",
                        Action::AudioBlind(false),
                        cx,
                    )
                    .ghost()
                    .disabled(self.tasks.cancel.is_some()),
                )
            })
            .child(
                self.button(
                    "audio-manage",
                    "管理音频",
                    Action::Page("library".into()),
                    cx,
                )
                .ghost(),
            );
        let header = div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(navigation)
            .when(!self.audio.blind.enabled, |v| {
                v.child(self.audio_subtitle_toolbar(cx))
            });
        let active = video_subtitles::active(&self.audio.cues, self.media.snapshot.position);
        let transcript = centered_list(
            &self.audio.center,
            self.audio.scroll.clone(),
            self.audio.follow && !self.has_subtitle_selection(),
            self.audio.active,
            cx.processor(|this, i: usize, _, cx| {
                let Some(cue) = this.audio.cues.get(i) else {
                    return div().into_any_element();
                };
                let active =
                    video_subtitles::active(&this.audio.cues, this.media.snapshot.position)
                        == Some(i);
                if this.audio.blind.enabled {
                    let revealed = this.audio.blind.revealed.as_ref().filter(|r| {
                        r.cue.start == cue.start && r.cue.end == cue.end && r.cue.text == cue.text
                    });
                    let mut word_line = div().flex().flex_wrap().gap_x_1().gap_y_2().items_center();
                    let show_original = revealed.is_some() && !this.audio.blind_chinese;
                    let words = subtitles::words(cue);
                    let no_words = words.is_empty();
                    for (j, word) in words.into_iter().enumerate() {
                        let audio_id = this.audio.current_id.clone();
                        let source_cue = cue.clone();
                        let current = active
                            && word.start <= this.media.snapshot.position
                            && this.media.snapshot.position < word.end;
                        let label = format!("第 {} 个词，从这里播放", j + 1);
                        let mut token = div()
                            .id(("blind-audio-word", i * 100000 + j))
                            .cursor_pointer()
                            .rounded_sm()
                            .px_1()
                            .py_1()
                            .text_size(px(this.audio.font_size as f32))
                            .bg(if current {
                                cx.theme().accent
                            } else {
                                cx.theme().border
                            })
                            .hover(|v| v.bg(cx.theme().accent))
                            .tooltip(move |window, cx| {
                                Tooltip::new(label.clone()).build(window, cx)
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.handle(
                                    Action::AudioBlindRow(
                                        audio_id.clone(),
                                        source_cue.clone(),
                                        Some(j),
                                    ),
                                    window,
                                    cx,
                                );
                            }));
                        if show_original {
                            token = token.child(word.text);
                        } else {
                            token = token
                                .w(px((word.text.chars().count() as f32
                                    * this.audio.font_size as f32
                                    * 0.55
                                    + 8.)
                                    .clamp(24., 280.)))
                                .h(px(this.audio.font_size as f32 + 12.));
                        }
                        word_line = word_line.child(token);
                    }
                    let mut content = div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .py_1()
                        .child(word_line)
                        .when(no_words, |v| {
                            v.child(
                                div()
                                    .w(px(180.))
                                    .h(px(this.audio.font_size as f32 + 4.))
                                    .rounded_sm()
                                    .bg(cx.theme().border),
                            )
                        });
                    if let Some(revealed) = revealed {
                        let chinese = if this.audio.blind_chinese {
                            blind_listening::chinese_hint(&revealed.cue.text)
                        } else {
                            subtitles::languages(std::slice::from_ref(&revealed.cue)).1
                        };
                        if !chinese.is_empty() || this.audio.blind_chinese {
                            content = content.child(
                                div()
                                    .text_size(px((this.audio.font_size as f32 - 2.).max(12.)))
                                    .text_color(cx.theme().muted_foreground)
                                    .whitespace_normal()
                                    .child(if chinese.is_empty() {
                                        "本句暂无中文".into()
                                    } else {
                                        chinese
                                    }),
                            );
                        }
                    } else {
                        content = content.child(
                            div()
                                .w_full()
                                .max_w(px(380.))
                                .h(px((this.audio.font_size as f32 - 2.).max(12.)))
                                .rounded_sm()
                                .bg(cx.theme().border),
                        );
                    }
                    let clicked_audio = this.audio.current_id.clone();
                    let clicked_cue = cue.clone();
                    let reveal_audio = this.audio.current_id.clone();
                    let reveal_cue = cue.clone();
                    return div()
                        .id(("blind-audio-cue", i))
                        .cursor_pointer()
                        .hover(|v| v.bg(cx.theme().accent))
                        .tooltip(|window, cx| {
                            Tooltip::new("左键播放，右键查看／收起这句").build(window, cx)
                        })
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                let is_revealed = reveal_audio == this.audio.current_id
                                    && this.audio.blind.revealed.as_ref().is_some_and(|r| {
                                        r.cue.start == reveal_cue.start
                                            && r.cue.end == reveal_cue.end
                                            && r.cue.text == reveal_cue.text
                                    });
                                this.handle(
                                    if is_revealed {
                                        Action::AudioBlindHide
                                    } else {
                                        Action::AudioBlindRevealCue(
                                            reveal_audio.clone(),
                                            reveal_cue.clone(),
                                        )
                                    },
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.handle(
                                Action::AudioBlindRow(
                                    clicked_audio.clone(),
                                    clicked_cue.clone(),
                                    None,
                                ),
                                window,
                                cx,
                            );
                        }))
                        .w_full()
                        .flex()
                        .gap_3()
                        .px_5()
                        .py_3()
                        .border_l_2()
                        .border_color(if active {
                            cx.theme().primary
                        } else {
                            cx.theme().background
                        })
                        .when(active, |v| v.bg(cx.theme().accent))
                        .child(
                            div()
                                .w(px(40.))
                                .flex_shrink_0()
                                .pt_1()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(clock(cue.start)),
                        )
                        .child(div().flex_1().min_w_0().child(content))
                        .child(
                            this.button(
                                format!("blind-row-reveal-{i}"),
                                if revealed.is_some() {
                                    "收起"
                                } else {
                                    "查看"
                                },
                                if revealed.is_some() {
                                    Action::AudioBlindHide
                                } else {
                                    Action::AudioBlindRevealCue(
                                        this.audio.current_id.clone(),
                                        cue.clone(),
                                    )
                                },
                                cx,
                            )
                            .ghost()
                            .disabled(
                                this.tasks.cancel.is_some() || this.audio.blind_pending.is_some(),
                            ),
                        )
                        .when(revealed.is_some(), |v| {
                            v.child(
                                this.button(
                                    format!("blind-row-copy-{i}"),
                                    "复制",
                                    Action::AudioCopySubtitle(
                                        this.audio.current_id.clone(),
                                        Some(cue.clone()),
                                    ),
                                    cx,
                                )
                                .ghost(),
                            )
                        })
                        .into_any_element();
                }
                let (_, translation) = subtitles::languages(std::slice::from_ref(cue));
                let edges = this
                    .audio
                    .clip_mode
                    .then(|| {
                        subtitles::selection_edges(
                            &this.audio.cues,
                            this.media.loop_a,
                            this.media.loop_b,
                        )
                    })
                    .flatten();
                let mut line = div().flex().flex_wrap().gap_x_1().gap_y_1().items_center();
                for (j, word) in subtitles::words(cue).into_iter().enumerate() {
                    let current = active
                        && word.start <= this.media.snapshot.position
                        && this.media.snapshot.position < word.end;
                    let marked = this.audio.key_points.iter().find(|p| {
                        p.text.split_whitespace().any(|w| {
                            w.trim_matches(|c: char| !c.is_alphanumeric())
                                .eq_ignore_ascii_case(
                                    word.text.trim_matches(|c: char| !c.is_alphanumeric()),
                                )
                        })
                    });
                    let saved = this.saved_at(true, word.start, word.end);
                    let is_saved = !saved.is_empty();
                    let tip = if is_saved {
                        Some("已加入闪卡 · 右键管理收藏复习；左键定位播放或选取片段".into())
                    } else {
                        marked.map(|p| format!("{} · {}", p.text, p.translation))
                    };
                    let selected = this.audio.clip_mode
                        && this
                            .media
                            .loop_a
                            .zip(this.media.loop_b)
                            .is_some_and(|(a, b)| word.start < b && word.end > a);
                    line = line.child(
                        div()
                            .id(("audio-word", i * 100000 + j))
                            .relative()
                            .when(this.audio.clip_mode, |v| v.py(px(14.)))
                            .when_some(
                                edges.filter(|e| e.start.cue == i && e.start.word == j),
                                |v, e| v.child(clip_selection_ui::text_edge(e.start, true)),
                            )
                            .when_some(
                                edges.filter(|e| e.end.cue == i && e.end.word == j),
                                |v, e| v.child(clip_selection_ui::text_edge(e.end, false)),
                            )
                            .text_size(px(this.audio.font_size as f32))
                            .px_0p5()
                            .rounded_sm()
                            .cursor_pointer()
                            .text_color(if current {
                                cx.theme().primary
                            } else {
                                cx.theme().foreground
                            })
                            .when(current || selected, |v| v.bg(cx.theme().accent))
                            .when(is_saved, |v| v.underline().text_color(cx.theme().primary))
                            .when(marked.is_some(), |v| {
                                v.border_b_2().border_color(rgb(0xd8a947))
                            })
                            .when_some(tip, |v, tip| {
                                v.tooltip(move |window, cx| {
                                    Tooltip::new(tip.clone()).build(window, cx)
                                })
                            })
                            .hover(|v| v.bg(cx.theme().accent))
                            .child(this.subtitle_fragment(
                                subtitle_text_ui::Surface::Audio,
                                i,
                                Some(j),
                                cx,
                            ))
                            .when(is_saved, |v| {
                                v.on_mouse_down(
                                    MouseButton::Right,
                                    cx.listener(move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.handle(Action::SavedClips(saved.clone()), window, cx);
                                    }),
                                )
                            })
                            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                                if this.subtitle_dragged() {
                                    cx.stop_propagation();
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
                            })),
                    );
                }
                div()
                    .id(("audio-cue", i))
                    .w_full()
                    .flex()
                    .gap_3()
                    .px_5()
                    .py_3()
                    .border_l_2()
                    .border_color(if active {
                        cx.theme().primary
                    } else {
                        cx.theme().background
                    })
                    .child(
                        div()
                            .w(px(40.))
                            .flex_shrink_0()
                            .pt_1()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(clock(cue.start)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(line)
                            .when(!translation.is_empty(), |v| {
                                v.child(
                                    div()
                                        .text_size(px((this.audio.font_size as f32 - 2.).max(12.)))
                                        .text_color(cx.theme().muted_foreground)
                                        .child(this.subtitle_fragment(
                                            subtitle_text_ui::Surface::Audio,
                                            i,
                                            None,
                                            cx,
                                        )),
                                )
                            }),
                    )
                    .child(
                        this.button(
                            format!("audio-row-copy-{i}"),
                            "复制",
                            Action::AudioCopySubtitle(
                                this.audio.current_id.clone(),
                                Some(cue.clone()),
                            ),
                            cx,
                        )
                        .ghost(),
                    )
                    .into_any_element()
            }),
        )
        .flex_1()
        .min_h_0()
        .w_full();
        let reading = {
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .overflow_hidden()
                .when(
                    !self.audio.blind.enabled
                        && self.audio.show_current
                        && !self.audio.cues.is_empty(),
                    |v| v.child(self.audio_current_cue(active, window, cx)),
                )
                .when(self.audio.cues.is_empty(), |v| {
                    v.child(
                        div()
                            .flex_1()
                            .p_8()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(if available && self.audio.blind.enabled {
                                "暂无字幕；可继续盲听，或点击“管理字幕”退出盲听后生成字幕。"
                            } else if available {
                                "点击上方“生成字幕”开始逐句学习，再点“翻译成中文”添加译文。已有字幕可从“更多”导入。"
                            } else {
                                "选择练习组和音频后开始学习，也可前往“管理音频”导入材料。"
                            }),
                    )
                })
                .when(!self.audio.cues.is_empty(), |v| v.child(transcript))
                .when(self.audio.blind.enabled, |v| v.child(self.blind_panel(cx)))
                .into_any_element()
        };
        let valid = self
            .media
            .loop_a
            .zip(self.media.loop_b)
            .is_some_and(|(a, b)| b > a);
        let rates = [0.6, 0.7, 0.8, 0.9, 1., 1.2, 1.25, 1.5, 2.]
            .into_iter()
            .map(|r| {
                (
                    format!("{r}×"),
                    Action::Rate(r),
                    (self.media.rate - r).abs() < 0.001,
                    false,
                )
            })
            .collect();
        let repeat = ["不循环", "单曲循环", "A–B 循环", "组循环", "单次连播"]
            .into_iter()
            .enumerate()
            .map(|(i, n)| {
                (
                    n.into(),
                    Action::AudioRepeat(i),
                    self.media.repeat == i,
                    i == 2 && !valid,
                )
            })
            .collect();
        let mut footer = div()
            .flex()
            .flex_col()
            .gap_1()
            .px_4()
            .py_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().muted)
            .flex_shrink_0()
            .when(available, |v| {
                v.child(self.waveform_view(cx))
                    .child(self.video_timeline(cx))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        self.player_icon(
                            "audio-back10",
                            "rewind",
                            "后退 10 秒",
                            Action::Seek(-10.),
                            cx,
                        )
                        .disabled(!available),
                    )
                    .child(
                        self.player_icon(
                            "audio-play",
                            "play",
                            if self.media.snapshot.paused {
                                "播放 · 空格"
                            } else {
                                "暂停 · 空格"
                            },
                            Action::Play,
                            cx,
                        )
                        .icon(Icon::new(IconName::ChevronDown).path(
                            if self.media.snapshot.paused {
                                "ui/play.svg"
                            } else {
                                "ui/pause.svg"
                            },
                        ))
                        .w(px(40.))
                        .h(px(40.))
                        .rounded_full()
                        .primary()
                        .disabled(!available),
                    )
                    .child(
                        self.player_icon(
                            "audio-forward10",
                            "forward",
                            "前进 10 秒",
                            Action::Seek(10.),
                            cx,
                        )
                        .disabled(!available),
                    )
                    .child(div().text_xs().child(if available {
                        format!(
                            "{} / {}",
                            clock(self.media.snapshot.position),
                            clock(self.media.snapshot.duration)
                        )
                    } else {
                        "00:00 / 00:00".into()
                    }))
                    .child(self.player_menu(
                        "audio-speed",
                        format!("{}× ▾", self.media.rate),
                        rates,
                        cx,
                    ))
                    .child(div().flex_1())
                    .child(self.player_menu(
                        "audio-repeat",
                        format!(
                            "{} ▾",
                            ["不循环", "单曲循环", "A–B 循环", "组循环", "单次连播"]
                                [self.media.repeat]
                        ),
                        repeat,
                        cx,
                    ))
                    .child(
                        self.button(
                            "audio-clip-mode",
                            if self.audio.clip_mode && !self.audio.blind.enabled {
                                "截取片段 ✓"
                            } else {
                                "截取片段"
                            },
                            Action::AudioClipMode,
                            cx,
                        )
                        .selected(self.audio.clip_mode)
                        .disabled(!available || self.audio.blind.enabled),
                    )
                    .child(
                        self.button(
                            "audio-highlights",
                            if self.audio.key_points.is_empty() {
                                "✦ 标重点"
                            } else {
                                "清除重点"
                            },
                            if self.audio.key_points.is_empty() {
                                Action::Highlight
                            } else {
                                Action::ClearHighlights
                            },
                            cx,
                        )
                        .ghost()
                        .disabled(
                            self.tasks.cancel.is_some()
                                || self.audio.cues.is_empty()
                                || self.audio.blind.enabled,
                        ),
                    )
                    .child(self.player_icon(
                        "audio-mute",
                        if self.media.volume == 0. {
                            "muted"
                        } else {
                            "volume"
                        },
                        "静音 / 恢复音量",
                        Action::Mute,
                        cx,
                    ))
                    .child(self.player_volume(cx)),
            );
        if self.audio.clip_mode && !self.audio.blind.enabled {
            footer = footer.child(
                div()
                    .flex()
                    .gap_2()
                    .items_center()
                    .flex_wrap()
                    .child(
                        self.button(
                            "audio-mark-a",
                            format!(
                                "A ▶ {}",
                                self.media
                                    .loop_a
                                    .map(clip_clock)
                                    .unwrap_or_else(|| "起点".into())
                            ),
                            Action::Mark(true),
                            cx,
                        )
                        .text_color(clip_selection_ui::edge_color(true)),
                    )
                    .child(
                        self.button(
                            "audio-mark-b",
                            format!(
                                "◀ B {}",
                                self.media
                                    .loop_b
                                    .map(clip_clock)
                                    .unwrap_or_else(|| "终点".into())
                            ),
                            Action::Mark(false),
                            cx,
                        )
                        .text_color(clip_selection_ui::edge_color(false)),
                    )
                    .child(
                        self.button("audio-clear-ab", "清除", Action::VideoClearLoop, cx)
                            .ghost(),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("点击单词选片段；Shift 点击扩展，拖动波形 A/B 调整"),
                    )
                    .child(self.button(
                        "audio-clip-editor",
                        if self.audio.tools_open {
                            "收起编辑"
                        } else {
                            "编辑片段"
                        },
                        Action::AudioTools,
                        cx,
                    ))
                    .child(
                        self.button("audio-save-clip", "保存为闪卡", Action::Clip, cx)
                            .primary()
                            .disabled(!valid || self.tasks.cancel.is_some()),
                    ),
            );
            if self.audio.tools_open {
                footer = footer.child(
                    div()
                        .flex()
                        .gap_3()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(self.field("front", "片段原文")),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(self.field("back", "片段翻译")),
                        ),
                );
            }
        }
        div()
            .w_full()
            .h(px(
                (f32::from(window.viewport_size().height) - 206.).max(360.)
            ))
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded_2xl()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(header)
            .child(reading)
            .child(footer)
            .into_any_element()
    }
    pub(super) fn transport(&self, cx: &Context<Self>) -> AnyElement {
        if self.page == "video" {
            return self.video_controls(cx);
        }
        div()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .when(self.page != "video", |v| v.rounded_lg())
            .bg(if self.page == "video" {
                rgb(0x14241e).into()
            } else {
                cx.theme().muted
            })
            .when(self.page == "video", |v| v.text_color(rgb(0xe8f0eb)))
            .when(self.page == "video", |v| v.child(self.video_timeline(cx)))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .flex_wrap()
                    .items_center()
                    .child(self.transport_button("back10", "−10秒", Action::Seek(-10.), cx))
                    .child(
                        self.transport_button(
                            "play",
                            if self.media.snapshot.paused {
                                "播放"
                            } else {
                                "暂停"
                            },
                            Action::Play,
                            cx,
                        )
                        .primary(),
                    )
                    .child(self.transport_button("forward10", "+10秒", Action::Seek(10.), cx))
                    .child(format!(
                        "{} / {}",
                        clock(self.media.snapshot.position),
                        clock(self.media.snapshot.duration)
                    ))
                    .child(self.transport_button(
                        "repeat",
                        ["不循环", "单曲循环", "A/B循环", "组循环", "单次连播"][self.media.repeat],
                        Action::Repeat,
                        cx,
                    )),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .flex_wrap()
                    .children([0.6, 0.8, 0.9, 1., 1.25, 1.5, 2.].into_iter().map(|r| {
                        self.transport_button(
                            format!("speed-{r}"),
                            format!("{r}×"),
                            Action::Rate(r),
                            cx,
                        )
                    }))
                    .child(self.transport_button(
                        "volume-down",
                        "音量 −",
                        Action::Volume(self.media.volume - 10.),
                        cx,
                    ))
                    .child(self.transport_button(
                        "volume-up",
                        "音量 +",
                        Action::Volume(self.media.volume + 10.),
                        cx,
                    )),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .flex_wrap()
                    .child(self.transport_button(
                        "point-a",
                        format!(
                            "A {}",
                            self.media.loop_a.map(clock).unwrap_or("未设置".into())
                        ),
                        Action::Mark(true),
                        cx,
                    ))
                    .child(self.transport_button(
                        "point-b",
                        format!(
                            "B {}",
                            self.media.loop_b.map(clock).unwrap_or("未设置".into())
                        ),
                        Action::Mark(false),
                        cx,
                    ))
                    .child(
                        self.transport_button(
                            "export-clip",
                            "保存 A/B 片段为闪卡",
                            Action::Clip,
                            cx,
                        )
                        .disabled(self.tasks.cancel.is_some()),
                    ),
            )
            .into_any_element()
    }
    pub(super) fn tts_panel(&self, cx: &Context<Self>) -> AnyElement {
        let ready = self.data.tts_ready;
        let voice = tts::VOICES
            .iter()
            .find(|(sid, _)| *sid == self.tts_voice)
            .map(|(_, name)| name.to_string())
            .unwrap_or_else(|| format!("音色 {}", self.tts_voice));
        let mut content = panel(cx)
            .child(div().text_lg().child("文字转音频 · Kokoro ONNX"))
            .child(div().text_sm().child(if ready {
                "中英模型已就绪 · 本机离线生成 WAV 和逐句字幕 · 自动保存到当前练习组"
            } else {
                "首次下载约 334 MiB，下载后可离线生成；需安装 clarora-tts 运行程序"
            }))
            .child(self.field("tts_name", "音频名称（留空自动命名）"))
            .child(self.field("tts_text", "要合成的文字"))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        self.player_menu(
                            "tts-voice",
                            voice,
                            tts::VOICES
                                .iter()
                                .map(|(sid, name)| {
                                    (
                                        name.to_string(),
                                        Action::TtsVoice(*sid),
                                        *sid == self.tts_voice,
                                        false,
                                    )
                                })
                                .collect(),
                            cx,
                        ),
                    )
                    .child(
                        self.player_menu(
                            "tts-speed",
                            format!("语速 {:.2}×", self.tts_speed),
                            [0.5, 0.75, 1., 1.25, 1.5, 2.]
                                .into_iter()
                                .map(|rate| {
                                    (
                                        format!("{rate:.2}×"),
                                        Action::TtsSpeed(rate),
                                        rate == self.tts_speed,
                                        false,
                                    )
                                })
                                .collect(),
                            cx,
                        ),
                    )
                    .child(
                        self.button(
                            "tts-download",
                            if ready {
                                "模型已下载"
                            } else {
                                "下载中英模型"
                            },
                            Action::TtsDownload,
                            cx,
                        )
                        .disabled(ready || self.tasks.cancel.is_some()),
                    )
                    .child(
                        self.button("tts-generate", "生成并保存音频", Action::TtsGenerate, cx)
                            .primary()
                            .disabled(
                                !ready
                                    || self.audio.practice_id.is_empty()
                                    || self.tasks.cancel.is_some(),
                            ),
                    ),
            );
        if let Some(row) = &self.tts_last {
            content = content
                .child(
                    div()
                        .text_sm()
                        .child(format!("最近生成：{}", text(row, "name"))),
                )
                .child(
                    self.button("tts-play", "试听 / 开始学习", Action::TtsPlay, cx)
                        .disabled(self.tasks.cancel.is_some()),
                );
        }
        content.into_any_element()
    }
}
