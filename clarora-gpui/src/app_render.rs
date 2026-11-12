//! Root AppView render tree, separated from startup and helpers.
use super::*;

impl Render for AppView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(debug_assertions)]
        self.capture_chatgpt_settings_layouts(window, cx);
        let mut images = Vec::new();
        if self.video_frame_path().is_some() {
            images.extend(self.media.snapshot.frame.clone());
        }
        if self.page == "pdf" {
            if let Some(reading) = &self.ebook_reading {
                images.extend(reading.images.values().cloned());
            } else {
                images.extend(self.pdf_image.clone());
            }
        }
        self.rendered_images.retain(images, window, cx);
        self.subtitle_text.borrow_mut().begin_frame();
        let (title, subtitle) = PAGES
            .iter()
            .find(|(key, _, _)| *key == self.page)
            .map(|(_, a, b)| (*a, *b))
            .unwrap_or(("Clarora", ""));
        if self.video.fullscreen {
            if window.is_fullscreen() {
                if !self.video.fullscreen_seen {
                    window.focus(&self.focus);
                }
                self.video.fullscreen_seen = true;
            } else if self.video.fullscreen_seen {
                // The user may exit through the native window controls.
                self.video.fullscreen = false;
                self.video.fullscreen_owned = false;
                self.video.fullscreen_seen = false;
            }
        }
        if self.video.fullscreen && self.page == "video" {
            let assistant = self.assistant_open.then(|| self.assistant_view(window, cx));
            let selection = self.selection_offer(window, cx);
            return div()
                .relative()
                .size_full()
                .child(self.fullscreen_video(window, cx))
                .children(assistant)
                .children(selection)
                .children(self.feedback_view(window, cx))
                .children(self.video_part_confirmation_view(window, cx))
                .into_any_element();
        }
        let navigation_progress = self.navigation_progress();
        if let Some((started, _)) = self.navigation_motion {
            if started.elapsed() < Duration::from_millis(240) {
                window.request_animation_frame();
            } else {
                self.navigation_motion = None;
            }
        }
        let content = self.content(window, cx);
        let assistant = if self.assistant_open {
            Some(self.assistant_view(window, cx))
        } else {
            None
        };
        let mut links = div()
            .id("navigation-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_1();
        // Keep existing Cmd/Ctrl+number assignments while showing review spaces last.
        for key in NAV_ORDER.iter().copied() {
            if let Some(section) = match key {
                "listening" => Some("学习空间"),
                "library" => Some("资料库"),
                "stats" => Some("我的"),
                "discover" => Some("复习"),
                _ => None,
            } {
                links = links.child(
                    div()
                        .px_3()
                        .pt_5()
                        .pb_2()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(section),
                );
            }
            let selected = self.page == key;
            let label = PAGES
                .iter()
                .find(|(page, _, _)| *page == key)
                .map(|(_, label, _)| *label)
                .unwrap_or(key);
            let key = key.to_string();
            links = links.child(
                div()
                    .id(SharedString::from(format!("nav-{key}")))
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_2p5()
                    .rounded_lg()
                    .cursor_pointer()
                    .text_sm()
                    .font_weight(if selected {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::NORMAL
                    })
                    .bg(if selected {
                        cx.theme().accent
                    } else {
                        transparent_black()
                    })
                    .text_color(if selected {
                        cx.theme().accent_foreground
                    } else {
                        cx.theme().foreground
                    })
                    .hover(|s| s.bg(cx.theme().accent))
                    .child(design::icon(
                        &key,
                        if selected {
                            cx.theme().primary
                        } else {
                            cx.theme().muted_foreground
                        },
                    ))
                    .child(div().flex_1().child(label))
                    .when(selected, |v| {
                        v.child(div().size(px(5.)).rounded_full().bg(cx.theme().primary))
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.handle(Action::Page(key.clone()), window, cx)
                    })),
            );
        }
        let nav = div()
            .w(px(224.))
            .flex_shrink_0()
            .h_full()
            .px_4()
            .py_6()
            .flex()
            .flex_col()
            .bg(design::rail(cx))
            .child(
                div()
                    .px_2()
                    .pb_4()
                    .flex()
                    .gap_3()
                    .items_center()
                    .child(
                        div()
                            .size(px(36.))
                            .rounded_xl()
                            .bg(cx.theme().primary)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                design::icon("meditation", cx.theme().primary_foreground)
                                    .size(px(24.)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(FontWeight::BOLD)
                                    .child("Clarora"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("把世界，听得更清楚"),
                            ),
                    ),
            )
            .child(links)
            .child(
                div()
                    .mt_4()
                    .p_4()
                    .rounded_xl()
                    .bg(cx.theme().background)
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .mb_2()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(design::icon("meditation", cx.theme().primary))
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child("留一点时间给自己"),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("每天一点，积累新的表达。"),
                    ),
            )
            .child(
                div()
                    .pt_4()
                    .px_2()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("本机学习空间"),
                    )
                    .child(
                        self.button("nav-theme", "明 / 暗", Action::Theme, cx)
                            .ghost(),
                    ),
            );
        div()
            .id("app")
            .relative()
            .track_focus(&self.focus)
            .key_context("Clarora")
            .child(self.subtitle_copy_anchor(cx))
            .capture_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, _, _| {
                if event.button == MouseButton::Left {
                    this.assistant.press = Some(event.position);
                    this.assistant.dragging = false;
                }
            }))
            .capture_any_mouse_up(cx.listener(|this, event: &MouseUpEvent, window, cx| {
                this.audio.text_edge_drag = None;
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
            .on_action(cx.listener(|this, _: &ToggleFloatingImport, window, cx| {
                this.toggle_floating_import(window, cx);
            }))
            .on_action(cx.listener(|this, _: &ImportAudioCommand, window, cx| {
                this.handle(Action::CommandImport, window, cx);
            }))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                if let Some(start) = this.audio.text_edge_drag {
                    if event.dragging() {
                        let time = this
                            .subtitle_text
                            .borrow()
                            .audio_time_at(event.position, &this.audio.cues);
                        if let Some(time) = time {
                            this.handle(Action::Boundary(time, start), window, cx);
                        }
                    }
                    return;
                }
                this.subtitle_drag_update(event, cx);
                if event.dragging()
                    && this
                        .assistant
                        .press
                        .is_some_and(|p| (event.position - p).magnitude() >= 4.)
                {
                    this.assistant.dragging = true;
                }
                if let Some((start, width)) = this.video.resize {
                    if event.dragging() {
                        let available = (f32::from(window.viewport_size().width)
                            - if this.navigation_open { 224. } else { 0. }
                            - 32.)
                            .max(300.);
                        this.video.panel_width = (width - f32::from(event.position.x - start))
                            .clamp(300., (available * 0.48).max(300.));
                        cx.notify();
                    } else {
                        this.video.resize = None;
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &OpenFile, _, cx| {
                if matches!(this.page.as_str(), "writing" | "sentences") {
                    return;
                }
                let kind = match this.page.as_str() {
                    "pdf" => "pdf",
                    "video" => "video",
                    "ocr" => "ocr",
                    "speaking" => "recording",
                    "library" | "listening" => "audio",
                    _ => "words",
                };
                this.pick(kind, cx);
            }))
            .on_action(cx.listener(|this, _: &Quit, _, cx| {
                cx.stop_propagation();
                this.prepare_close(cx);
            }))
            .on_action(cx.listener(|this, _: &NextVideoPart, window, cx| {
                if this.page == "video"
                    && let Some(index) = this
                        .video
                        .series
                        .as_ref()
                        .and_then(|s| s.current)
                        .map(|i| i + 1)
                {
                    this.handle(Action::SelectPart(index), window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &PreviousVideoPart, window, cx| {
                if this.page == "video"
                    && let Some(index) = this
                        .video
                        .series
                        .as_ref()
                        .and_then(|s| s.current)
                        .and_then(|i| i.checked_sub(1))
                {
                    this.handle(Action::SelectPart(index), window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ReadVideoParts, window, cx| {
                if this.page == "video" {
                    this.handle(Action::ReadParts, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleVideoFullscreen, window, cx| {
                this.handle(Action::VideoFullscreen, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleVideoPrecision, window, cx| {
                if this.page == "video" {
                    this.handle(Action::VideoPrecision, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &MarkVideoA, window, cx| {
                if this.page == "video" {
                    this.handle(Action::Mark(true), window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &MarkVideoB, window, cx| {
                if this.page == "video" {
                    this.handle(Action::Mark(false), window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ClearVideoLoop, window, cx| {
                if this.page == "video" {
                    this.handle(Action::VideoClearLoop, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &RepeatVideoCue, window, cx| {
                if this.page == "video" {
                    this.handle(Action::VideoRepeatCue, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &PreviousVideoSentence, window, cx| {
                if this.page == "video" {
                    this.handle(Action::VideoSentence(false), window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &NextVideoSentence, window, cx| {
                if this.page == "video" {
                    this.handle(Action::VideoSentence(true), window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &AskVideoCurrent, window, cx| {
                this.handle(Action::VideoAsk(video_qa::Scope::Current), window, cx);
            }))
            .on_action(cx.listener(|this, _: &AskVideoAll, window, cx| {
                this.handle(Action::VideoAskTranscript, window, cx);
            }))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| {
                this.handle(Action::Page("settings".into()), window, cx);
            }))
            .on_action(cx.listener(|this, _: &DownloadLocalModel, window, cx| {
                if this.page == "settings" {
                    this.handle(Action::Models(this.value("local_model", cx)), window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &CancelTask, window, cx| {
                this.handle(
                    if this.tasks.foreground.is_running() {
                        Action::Cancel
                    } else if this.tasks.audio_subtitles.lane.is_running() {
                        Action::CancelAudioSubtitles
                    } else {
                        Action::CancelModel
                    },
                    window,
                    cx,
                );
            }))
            .on_action(cx.listener(|this, _: &TranscribeVideo, window, cx| {
                this.handle(Action::VideoTranscribe, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ComposeSentence, window, cx| {
                if this.page != "flashcard" {
                    this.handle(Action::Page("flashcard".into()), window, cx);
                }
                this.composer_open = true;
                this.page_scroll.scroll_to_bottom();
                this.inputs["front"].update(cx, |state, cx| state.focus(window, cx));
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleAppearance, window, cx| {
                this.handle(Action::Theme, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleNavigation, window, cx| {
                this.handle(Action::Navigation, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleAssistant, window, cx| {
                this.handle(Action::Assistant, window, cx)
            }))
            .on_action(cx.listener(|this, _: &QuoteSelectedText, window, cx| {
                this.offer_assistant_selection(point(px(360.), px(180.)), window, cx);
            }))
            .on_action(cx.listener(|this, _: &PdfZoomIn, window, cx| {
                if this.page == "pdf" {
                    this.handle(Action::PdfZoom(200), window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &PdfZoomOut, window, cx| {
                if this.page == "pdf" {
                    this.handle(Action::PdfZoom(-200), window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &AskAi, window, cx| {
                if !this.assistant.enabled {
                    this.open_assistant(window, cx);
                }
                this.handle(Action::Chat, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleBlindListening, window, cx| {
                this.handle(Action::AudioBlind(!this.audio.blind.enabled), window, cx);
            }))
            .on_action(cx.listener(|this, _: &RevealBlindSentence, window, cx| {
                this.handle(Action::AudioBlindReveal, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ResumeBlindListening, window, cx| {
                this.handle(Action::AudioBlindResume(false), window, cx);
            }))
            .on_action(cx.listener(|this, _: &RestartBlindListening, window, cx| {
                this.handle(Action::AudioBlindResume(true), window, cx);
            }))
            .on_action(cx.listener(|this, _: &ViewAudioTranscript, window, cx| {
                this.handle(Action::AudioBlind(false), window, cx);
            }))
            .on_action(cx.listener(|this, _: &NewChat, window, cx| {
                this.open_assistant(window, cx);
                this.handle(Action::ChatSession(String::new()), window, cx);
            }))
            .on_action(cx.listener(|this, _: &Forward, window, cx| {
                if this.page == "writing" {
                    return;
                }
                this.handle(
                    if this.page == "pdf" {
                        Action::PdfPage(1)
                    } else {
                        Action::Next
                    },
                    window,
                    cx,
                )
            }))
            .on_action(cx.listener(|this, _: &Backward, window, cx| {
                if this.page == "writing" {
                    return;
                }
                this.handle(
                    if this.page == "pdf" {
                        Action::PdfPage(-1)
                    } else {
                        Action::Previous
                    },
                    window,
                    cx,
                )
            }))
            .size_full()
            .flex()
            .bg(design::canvas(cx))
            .text_color(cx.theme().foreground)
            .text_size(px(14.))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.handle_card_study_key(event, window, cx) {
                    cx.stop_propagation();
                    return;
                }
                if this.handle_sentence_search_key(event, window, cx) {
                    cx.stop_propagation();
                    return;
                }
                if this.handle_audio_arrow_key(event, window, cx) {
                    cx.stop_propagation();
                    return;
                }
                if this.handle_video_arrow_key(event, window, cx) {
                    cx.stop_propagation();
                    return;
                }
                if event.keystroke.modifiers.control
                    || (cfg!(target_os = "macos") && event.keystroke.modifiers.platform)
                {
                    if let Ok(index) = event.keystroke.key.parse::<usize>()
                        && index >= 1
                        && index <= PAGES.len()
                    {
                        this.handle(Action::Page(PAGES[index - 1].0.into()), window, cx);
                        return;
                    }
                    return;
                }
                if matches!(this.page.as_str(), "writing" | "sentences") {
                    return;
                }
                let subtitle_focus = this.subtitle_text.borrow().focus.clone();
                if window.focused(cx) == Some(this.focus.clone())
                    || window.focused(cx) == Some(subtitle_focus)
                {
                    if this.page == "listening"
                        && this.audio.blind.enabled
                        && !event.keystroke.modifiers.alt
                        && !event.keystroke.modifiers.shift
                    {
                        let action = match event.keystroke.key.as_str() {
                            "r" => Some(Action::AudioBlindSentence(0)),
                            "up" => Some(Action::AudioBlindSentence(-1)),
                            "down" => Some(Action::AudioBlindSentence(1)),
                            "h" => Some(if this.audio.blind.revealed.is_some() {
                                Action::AudioBlindHide
                            } else {
                                Action::AudioBlindReveal
                            }),
                            "c" => Some(Action::AudioBlindResume(false)),
                            _ => None,
                        };
                        if let Some(action) = action {
                            this.handle(action, window, cx);
                            cx.stop_propagation();
                            return;
                        }
                    }
                    match event.keystroke.key.as_str() {
                        "space" if !matches!(this.page.as_str(), "flashcard" | "discover") => {
                            this.handle(Action::Play, window, cx)
                        }
                        "right" if this.page != "listening" => this.handle(
                            if ["video", "listening"].contains(&this.page.as_str()) {
                                this.video_horizontal_action(true)
                            } else {
                                Action::Next
                            },
                            window,
                            cx,
                        ),
                        "left" if this.page != "listening" => this.handle(
                            if ["video", "listening"].contains(&this.page.as_str()) {
                                this.video_horizontal_action(false)
                            } else {
                                Action::Previous
                            },
                            window,
                            cx,
                        ),
                        "1" => this.handle(Action::Grade("again".into()), window, cx),
                        "2" => this.handle(Action::Grade("hard".into()), window, cx),
                        "3" => this.handle(Action::Grade("good".into()), window, cx),
                        _ => {}
                    }
                }
            }))
            .when(navigation_progress > 0., |view| {
                view.child(
                    div()
                        .id("navigation-slide")
                        .relative()
                        .w(px(224. * navigation_progress))
                        .flex_shrink_0()
                        .h_full()
                        .overflow_hidden()
                        .child(
                            nav.absolute()
                                .top_0()
                                .left(px(224. * (navigation_progress - 1.))),
                        ),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .px_8()
                            .py_6()
                            .when(self.page == "video" || self.page == "listening", |v| {
                                v.py_3()
                            })
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .bg(design::canvas(cx))
                            .flex()
                            .justify_between()
                            .items_center()
                            .flex_wrap()
                            .gap_4()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_3()
                                            .child(
                                                self.button(
                                                    "navigation-toggle",
                                                    if self.navigation_open {
                                                        "收起导航"
                                                    } else {
                                                        "展开导航"
                                                    },
                                                    Action::Navigation,
                                                    cx,
                                                )
                                                .icon(
                                                    Icon::new(IconName::ChevronDown)
                                                        .path(if self.navigation_open {
                                                            "ui/sidebar-close.svg"
                                                        } else {
                                                            "ui/sidebar-open.svg"
                                                        })
                                                        .size(px(22.)),
                                                )
                                                .h(px(36.))
                                                .px_3()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .cursor_pointer()
                                                .custom(
                                                    ButtonCustomVariant::new(cx)
                                                        .color(cx.theme().accent)
                                                        .foreground(cx.theme().primary)
                                                        .border(cx.theme().primary.opacity(0.45))
                                                        .hover(cx.theme().primary.opacity(0.16))
                                                        .active(cx.theme().primary.opacity(0.24)),
                                                )
                                                .tooltip(if cfg!(target_os = "macos") {
                                                    "显示或隐藏左侧导航 · ⌘B"
                                                } else {
                                                    "显示或隐藏左侧导航 · Ctrl+B"
                                                }),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child("CLARORA  /  学习空间"),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(27.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(title),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(subtitle),
                                    ),
                            )
                            .when(self.page == "speaking", |header| {
                                header.child(self.speaking_header_controls(cx))
                            })
                            .child(
                                div()
                                    .flex()
                                    .gap_4()
                                    .items_center()
                                    .flex_wrap()
                                    .child(self.computer_focus_control(cx))
                                    .child(
                                        self.button(
                                            "floating-import-toggle",
                                            "悬浮窗",
                                            Action::FloatingImport,
                                            cx,
                                        )
                                        .outline(),
                                    )
                                    .child(
                                        self.button(
                                            "assistant-toggle",
                                            "✦  学习助手",
                                            Action::Assistant,
                                            cx,
                                        )
                                        .outline(),
                                    ),
                            ),
                    )
                    .child(self.focus_timer_bar(window, cx))
                    .children(self.subtitle_task_banner(cx))
                    .child(
                        div()
                            .id("page-scroll")
                            .track_scroll(&self.page_scroll)
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .when(self.page == "pdf", |view| view.overflow_x_scroll())
                            .p_8()
                            .when(self.page == "video" || self.page == "listening", |v| {
                                v.p_4()
                            })
                            .child(content),
                    ),
            )
            .when_some(assistant, |view, assistant| view.child(assistant))
            .children(self.selection_offer(window, cx))
            .children(self.feedback_view(window, cx))
            .children(self.video_part_confirmation_view(window, cx))
            .children(Root::render_dialog_layer(window, cx))
            .into_any_element()
    }
}
