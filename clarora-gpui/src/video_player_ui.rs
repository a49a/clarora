//! Video workspace renders cached player and subtitle state.
use super::*;

impl AppView {
    pub(super) fn video_timeline(&self, cx: &Context<Self>) -> impl IntoElement {
        let bounds = std::rc::Rc::new(std::cell::Cell::new(Bounds::<Pixels>::default()));
        let paint_bounds = bounds.clone();
        let drag_bounds = bounds.clone();
        let duration = self.media.snapshot.duration;
        let ratio = if duration.is_finite() && duration > 0. {
            (self.media.snapshot.position / duration).clamp(0., 1.) as f32
        } else {
            0.
        };
        let listening = self.page == "listening";
        let sentence_range = self.video.sentence_repeat.map(|session| session.range);
        let loop_range = sentence_range.or_else(|| {
            self.media
                .loop_a
                .zip(self.media.loop_b)
                .filter(|(a, b)| b > a)
        });
        let markers = loop_range
            .map(|(a, b)| [Some(a), Some(b)])
            .unwrap_or([self.media.loop_a, self.media.loop_b]);
        let color: Hsla = if self.page == "listening" {
            cx.theme().primary
        } else {
            rgb(0x7dd8b5).into()
        };
        let background: Hsla = if self.page == "listening" {
            cx.theme().border
        } else {
            rgb(0x345045).into()
        };
        div()
            .id("video-timeline")
            .w_full()
            .h(px(20.))
            .cursor_pointer()
            .child(
                canvas(
                    move |area, _, _| paint_bounds.set(area),
                    move |area, _, window, _| {
                        let width = f32::from(area.size.width);
                        let y = area.top() + px(8.);
                        window.paint_quad(fill(
                            Bounds::new(point(area.left(), y), size(area.size.width, px(4.))),
                            background,
                        ));
                        if duration > 0. {
                            if let Some((a, b)) = loop_range {
                                let left = (a / duration).clamp(0., 1.) as f32 * width;
                                let right = (b / duration).clamp(0., 1.) as f32 * width;
                                window.paint_quad(fill(
                                    Bounds::new(
                                        point(area.left() + px(left), y - px(3.)),
                                        size(px(right - left), px(10.)),
                                    ),
                                    rgb(0x3c7260),
                                ));
                            }
                            for (index, time) in markers
                                .into_iter()
                                .enumerate()
                                .filter_map(|(i, t)| t.map(|t| (i, t)))
                            {
                                let x =
                                    (time / duration).clamp(0., 1.) as f32 * (width - 2.).max(0.);
                                window.paint_quad(fill(
                                    Bounds::new(
                                        point(area.left() + px(x), y - px(4.)),
                                        size(px(2.), px(12.)),
                                    ),
                                    if listening {
                                        clip_selection_ui::edge_color(index == 0)
                                    } else {
                                        rgb(0xe7bd7b)
                                    },
                                ));
                            }
                        }
                        window.paint_quad(fill(
                            Bounds::new(point(area.left(), y), size(px(width * ratio), px(4.))),
                            color,
                        ));
                        window.paint_quad(fill(
                            Bounds::new(
                                point(area.left() + px((width - 8.).max(0.) * ratio), y - px(2.)),
                                size(px(8.), px(8.)),
                            ),
                            color,
                        ));
                    },
                )
                .size_full(),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    let area = bounds.get();
                    if area.size.width > px(0.) && duration.is_finite() && duration > 0. {
                        this.video.seek_dragging = true;
                        this.video.seek_drag_target = None;
                        let time = ((event.position.x - area.left()) / area.size.width)
                            .clamp(0., 1.) as f64
                            * duration;
                        this.handle(
                            if this.page == "video" {
                                Action::VideoTimelineSeek(time)
                            } else {
                                Action::SeekTo(time)
                            },
                            window,
                            cx,
                        );
                    }
                }),
            )
            .on_mouse_move(
                cx.listener(move |this, event: &MouseMoveEvent, window, cx| {
                    let area = drag_bounds.get();
                    if this.video.seek_dragging
                        && event.dragging()
                        && area.size.width > px(0.)
                        && duration.is_finite()
                        && duration > 0.
                    {
                        let time = ((event.position.x - area.left()) / area.size.width)
                            .clamp(0., 1.) as f64
                            * duration;
                        this.handle(
                            if this.page == "video" {
                                Action::VideoTimelineSeek(time)
                            } else {
                                Action::SeekTo(time)
                            },
                            window,
                            cx,
                        );
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.video.seek_dragging = false;
                    this.video.seek_drag_target = None;
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.video.seek_dragging = false;
                    this.video.seek_drag_target = None;
                }),
            )
    }
    pub(super) fn video_frame_path(&self) -> Option<PathBuf> {
        let path = self.media.path.as_ref()?;
        let visible = if self.page == "video" {
            self.video.path.as_ref() == Some(path)
        } else if self.page == "flashcard" || self.page == "discover" {
            self.current_card()
                .is_some_and(|card| card.kind == "video" && Path::new(&card.media) == path)
        } else {
            false
        };
        visible.then(|| path.clone())
    }
    pub(super) fn video_viewport_probe(&self, cx: &Context<Self>) -> impl IntoElement {
        let view = cx.entity().downgrade();
        let expected = self.video_frame_path();
        canvas(
            move |bounds, window, cx| {
                let scale = window.scale_factor();
                let width = (f32::from(bounds.size.width) * scale).ceil().max(0.) as u32;
                let height = (f32::from(bounds.size.height) * scale).ceil().max(0.) as u32;
                let _ = view.update(cx, |this, _| {
                    if expected.is_some() && this.video_frame_path() == expected {
                        this.media
                            .player
                            .set_video_target(width, height, width > 0 && height > 0);
                    }
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }
    pub(super) fn video_picture(
        &self,
        height: Option<Pixels>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let active = self.video.path.as_ref().is_some_and(|p| {
            self.media.path.as_ref() == Some(p) && self.media.snapshot.path == p.to_string_lossy()
        });
        let frame = self.media.snapshot.frame.clone().filter(|_| active);
        let active_caption = if !active
            || !self.video.captions_enabled
            || self.media.snapshot.subtitle_is_bitmap
            || self.video.caption_source == VideoCaptionSource::Hidden
        {
            String::new()
        } else if self.video.caption_source == VideoCaptionSource::Track {
            self.media.snapshot.subtitle.clone()
        } else {
            video_subtitles::active(&self.video.cues, self.media.snapshot.position)
                .map(|i| self.video.cues[i].text.clone())
                .filter(|text| !text.is_empty())
                .unwrap_or_else(|| self.media.snapshot.subtitle.clone())
        };
        div()
            .relative()
            .w_full()
            .id("video-picture")
            .when(active, |view| view.cursor_pointer())
            .when_some(height, |view, height| view.h(height).flex_shrink_0())
            .when(height.is_none(), |view| view.flex_1().min_h_0())
            .overflow_hidden()
            .bg(rgb(0x090e0c))
            .child(self.video_viewport_probe(cx))
            .when_some(frame.clone(), |v, image| {
                // Keep the intrinsic image ratio out of the flex flow. The
                // decoder frame must fit entirely inside the available viewport.
                v.child(
                    img(image)
                        .absolute()
                        .top_0()
                        .left_0()
                        .w_full()
                        .when_some(height, |image, height| image.h(height))
                        .when(height.is_none(), |image| image.h_full())
                        .object_fit(ObjectFit::Contain),
                )
            })
            .when(frame.is_none(), |v| {
                v.child(
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(rgb(0x9bafa4))
                        .child("打开本地视频，或从 bilibili 导入"),
                )
            })
            .when(!active_caption.is_empty(), |v| {
                v.child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .right_0()
                        .px_5()
                        .py_3()
                        .id("video-current-caption")
                        .max_h(px(f32::from(self.video.subtitle_size) * 3.8 + 24.))
                        .overflow_y_scroll()
                        .bg(rgba(0x050a08c9))
                        .text_color(rgb(0xffffff))
                        .text_size(px(f32::from(self.video.subtitle_size)))
                        .text_center()
                        .child(active_caption),
                )
            })
            .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                if !event.standard_click() || this.page != "video" || window.has_active_dialog(cx) {
                    this.video.picture_click = None;
                    return;
                }
                let Some(path) = this.video.path.clone().filter(|path| {
                    this.media.path.as_ref() == Some(path)
                        && this.media.player.snapshot().path == path.to_string_lossy()
                }) else {
                    this.video.picture_click = None;
                    return;
                };
                cx.stop_propagation();
                window.focus(&this.focus);
                match event.click_count() {
                    1 => {
                        if this.toggle_pending_video_start() {
                            this.video.picture_click = Some(path);
                            cx.notify();
                            return;
                        }
                        let paused = this.media.player.snapshot().paused;
                        this.clear_finished_video_sentence_repeat();
                        this.video.picture_click = Some(path);
                        this.media
                            .player
                            .set("pause", if paused { "no" } else { "yes" });
                    }
                    2 => {
                        if this.video.picture_click.take().as_ref() == Some(&path) {
                            // Undo the first click without relying on the 50 ms UI snapshot.
                            if !this.toggle_pending_video_start() {
                                this.media.player.command(["cycle", "pause"]);
                            }
                        }
                        this.handle(Action::VideoFullscreen, window, cx);
                    }
                    _ => this.video.picture_click = None,
                }
                cx.notify();
            }))
    }
    pub(super) fn fullscreen_video(&self, cx: &Context<Self>) -> AnyElement {
        div()
            .id("video-fullscreen")
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x090e0c))
            .track_focus(&self.focus)
            .key_context("VideoFullscreen")
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.handle_video_arrow_key(event, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .child(self.subtitle_copy_anchor(cx))
            .capture_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, _, _| {
                if event.button == MouseButton::Left {
                    this.assistant.press = Some(event.position);
                    this.assistant.dragging = false;
                }
            }))
            .capture_any_mouse_up(cx.listener(|this, event: &MouseUpEvent, window, cx| {
                if event.button == MouseButton::Left && this.assistant.dragging {
                    this.offer_assistant_selection(event.position, window, cx);
                } else if event.button == MouseButton::Left {
                    this.assistant.offer = None;
                    cx.notify();
                }
                this.assistant.press = None;
                this.assistant.dragging = false;
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.subtitle_text.borrow_mut().block_click = false),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                this.subtitle_drag_update(event, cx);
                if event.dragging()
                    && this
                        .assistant
                        .press
                        .is_some_and(|p| (event.position - p).magnitude() >= 4.)
                {
                    this.assistant.dragging = true;
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleAssistant, window, cx| {
                this.handle(Action::Assistant, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &AskAi, window, cx| this.handle(Action::Chat, window, cx)),
            )
            .on_action(cx.listener(|this, _: &QuoteSelectedText, window, cx| {
                this.offer_assistant_selection(point(px(400.), px(120.)), window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleVideoFullscreen, window, cx| {
                this.handle(Action::VideoFullscreen, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ExitVideoFullscreen, window, cx| {
                this.handle(
                    if this.video.part_confirmation.is_some() {
                        Action::VideoCancelPart
                    } else {
                        Action::VideoExitFullscreen
                    },
                    window,
                    cx,
                )
            }))
            .on_action(cx.listener(|this, _: &FullscreenPlayPause, window, cx| {
                this.handle(Action::Play, window, cx)
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_4()
                    .py_1()
                    .flex_shrink_0()
                    .text_color(rgb(0xb9c9c1))
                    .child(
                        div().flex_1().min_w_0().text_xs().truncate().child(
                            self.media
                                .path
                                .as_ref()
                                .and_then(|p| p.file_name())
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_default(),
                        ),
                    )
                    .child(self.transport_button(
                        "fullscreen-assistant",
                        "✦ 学习助手",
                        Action::Assistant,
                        cx,
                    ))
                    .child(self.transport_button(
                        "exit-video-fullscreen",
                        "退出全屏 · Esc",
                        Action::VideoExitFullscreen,
                        cx,
                    )),
            )
            .child(self.focus_timer_bar(cx))
            .children(self.subtitle_task_banner(cx))
            .child(self.video_picture(None, cx))
            .child(self.video_controls(cx))
            .into_any_element()
    }
    pub(super) fn video_controls(&self, cx: &Context<Self>) -> AnyElement {
        let valid_loop = self
            .media
            .loop_a
            .zip(self.media.loop_b)
            .is_some_and(|(a, b)| b > a);
        let current_cue = self.video_repeat_cue();
        let repeat_label = if let Some(session) = self.video.sentence_repeat {
            if session.three {
                match self
                    .media
                    .snapshot
                    .sentence_repeat
                    .as_ref()
                    .filter(|state| state.request_id == session.request_id)
                {
                    Some(state) if state.finished => "逐句三次：已完成 ▾".into(),
                    Some(state) => format!("逐句三次：{}/3 ▾", state.current_play),
                    None => "逐句三次：准备中 ▾".into(),
                }
            } else {
                "本句：无限重复 ▾".into()
            }
        } else if self.video.repeat_three {
            "逐句三次：等待就绪 ▾".into()
        } else if self.video.pending_start.is_some() {
            "正在定位末句 ▾".into()
        } else {
            "重复模式 ▾".into()
        };
        let sentence_options = vec![
            (
                "本句无限重复".into(),
                Action::VideoSentenceRepeat(None, false),
                self.video.sentence_repeat.is_some_and(|s| !s.three),
                current_cue.is_none(),
            ),
            (
                "每句重复三次 · 自动下一句".into(),
                Action::VideoSentenceRepeat(None, true),
                self.video.repeat_three,
                self.video.path.is_none(),
            ),
            (
                "停止本句重复".into(),
                Action::VideoStopSentenceRepeat,
                false,
                self.video.sentence_repeat.is_none()
                    && !self.video.repeat_three
                    && self.video.pending_start.is_none(),
            ),
        ];
        let rates = [0.6, 0.7, 0.8, 0.9, 1., 1.25, 1.5, 2.]
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
        let mut options = [14, 16, 20, 24, 28, 32]
            .into_iter()
            .map(|size| {
                (
                    format!("字幕 {size}px"),
                    Action::VideoSubtitleSize(size),
                    self.video.subtitle_size == size,
                    false,
                )
            })
            .collect::<Vec<_>>();
        options.extend(
            [("关闭循环", 0), ("单集循环", 1), ("A–B 片段循环", 2)]
                .into_iter()
                .map(|(label, mode)| {
                    (
                        label.into(),
                        Action::LoopMode(mode),
                        self.media.repeat == mode,
                        mode == 2 && !valid_loop,
                    )
                }),
        );
        options.push((
            "A–B 精听工具".into(),
            Action::VideoPrecision,
            self.video.precision,
            false,
        ));
        div()
            .flex_shrink_0()
            .px_4()
            .pt_1()
            .pb_3()
            .flex()
            .flex_col()
            .gap_1()
            .bg(rgb(0x171f1c))
            .text_color(rgb(0xe8f0eb))
            .child(self.video_timeline(cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .flex_wrap()
                    .child(self.player_icon(
                        "video-back",
                        "rewind",
                        "后退 10 秒",
                        Action::Seek(-10.),
                        cx,
                    ))
                    .child(
                        self.player_icon(
                            "video-play",
                            if self.video_playback_paused() {
                                "play"
                            } else {
                                "pause"
                            },
                            if self.video_playback_paused() {
                                "播放 · 空格"
                            } else {
                                "暂停 · 空格"
                            },
                            Action::Play,
                            cx,
                        )
                        .w(px(40.))
                        .h(px(40.))
                        .rounded_full(),
                    )
                    .child(self.player_icon(
                        "video-forward",
                        "forward",
                        "前进 10 秒",
                        Action::Seek(10.),
                        cx,
                    ))
                    .child(div().text_xs().text_color(rgb(0xb9c9c1)).child(format!(
                        "{} / {}",
                        clock(self.media.snapshot.position),
                        clock(self.media.snapshot.duration)
                    )))
                    .child(div().flex_1())
                    .child(self.player_menu(
                        "video-sentence-repeat",
                        repeat_label,
                        sentence_options,
                        cx,
                    ))
                    .child(self.player_menu(
                        "video-speed",
                        format!("{}× ▾", self.media.rate),
                        rates,
                        cx,
                    ))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(self.player_icon(
                                "video-mute",
                                if self.media.volume == 0. {
                                    "muted"
                                } else {
                                    "volume"
                                },
                                &format!("音量 {:.0}% · 点击静音 / 恢复", self.media.volume),
                                Action::Mute,
                                cx,
                            ))
                            .child(self.player_volume(cx)),
                    )
                    .child(
                        self.transport_button(
                            "video-captions-toggle",
                            if self.video.captions_enabled {
                                "字幕：开"
                            } else {
                                "字幕：关"
                            },
                            Action::VideoCaptions,
                            cx,
                        )
                        .selected(self.video.captions_enabled),
                    )
                    .child(self.player_menu("video-options", "设置 ▾".into(), options, cx))
                    .child(
                        self.player_icon(
                            "video-fullscreen-toggle",
                            if self.video.fullscreen {
                                "fullscreen-exit"
                            } else {
                                "fullscreen"
                            },
                            "全屏 / 退出全屏 · F11；双击画面也可全屏",
                            Action::VideoFullscreen,
                            cx,
                        )
                        .disabled(self.media.path.is_none()),
                    ),
            )
            .when(self.video.precision, |v| {
                v.child(
                    div()
                        .mt_2()
                        .pt_3()
                        .border_t_1()
                        .border_color(rgb(0x34463d))
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .flex_wrap()
                                .child(
                                    self.transport_button(
                                        "video-set-a",
                                        format!(
                                            "A  {}",
                                            self.media
                                                .loop_a
                                                .map(clip_clock)
                                                .unwrap_or("设起点".into())
                                        ),
                                        Action::Mark(true),
                                        cx,
                                    )
                                    .tooltip("将当前播放位置设为 A 点"),
                                )
                                .child(
                                    self.transport_button(
                                        "video-set-b",
                                        format!(
                                            "B  {}",
                                            self.media
                                                .loop_b
                                                .map(clip_clock)
                                                .unwrap_or("设终点".into())
                                        ),
                                        Action::Mark(false),
                                        cx,
                                    )
                                    .disabled(self.media.loop_a.is_none())
                                    .tooltip("将当前位置设为 B 点，自动循环此片段"),
                                )
                                .child(
                                    self.transport_button(
                                        "video-clear-loop",
                                        "清除",
                                        Action::VideoClearLoop,
                                        cx,
                                    )
                                    .disabled(
                                        self.media.loop_a.is_none()
                                            && self.media.loop_b.is_none()
                                            && self.video.sentence_repeat.is_none(),
                                    ),
                                )
                                .child(div().flex_1())
                                .child(
                                    self.transport_button(
                                        "video-clip",
                                        "存为片段闪卡",
                                        Action::Clip,
                                        cx,
                                    )
                                    .disabled(!valid_loop || self.tasks.cancel.is_some()),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .flex_wrap()
                                .child(
                                    self.transport_button(
                                        "video-prev-sentence",
                                        "上一句",
                                        Action::VideoSentence(false),
                                        cx,
                                    )
                                    .disabled(!self.can_move_video_sentence(false)),
                                )
                                .child(
                                    self.transport_button(
                                        "video-repeat-sentence",
                                        "重听本句",
                                        Action::VideoRepeatCue,
                                        cx,
                                    )
                                    .disabled(current_cue.is_none()),
                                )
                                .child(
                                    self.transport_button(
                                        "video-next-sentence",
                                        "下一句",
                                        Action::VideoSentence(true),
                                        cx,
                                    )
                                    .disabled(!self.can_move_video_sentence(true)),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(0x9bafa4))
                                        .child("A → B 自动循环 · 进度条金色标记为边界"),
                                ),
                        ),
                )
            })
            .into_any_element()
    }
}
