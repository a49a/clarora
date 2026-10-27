//! Module pages consume view data; they do not refresh the repository.
use super::*;

impl AppView {
    pub(super) fn content(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let mut content = div().flex().flex_col().gap_5().w_full();
        if matches!(self.page.as_str(), "flashcard" | "discover") && !self.cards_ready() {
            return content
                .child(
                    panel(cx)
                        .child(if self.loads.error.is_empty() {
                            "正在读取学习卡片…".to_string()
                        } else {
                            format!("读取失败：{}", self.loads.error)
                        })
                        .child(self.button("retry-card-load", "重新读取", Action::Refresh, cx)),
                )
                .into_any_element();
        }
        if !self.loads.error.is_empty() {
            content = content.child(panel(cx).text_color(cx.theme().danger).child(format!(
                "资料读取失败，正在显示上次内容：{}",
                self.loads.error
            )));
        }
        if !self.data.error.is_empty() {
            content = content.child(panel(cx).text_color(cx.theme().danger).child(format!(
                "读取页面资料失败，正在显示上次内容：{}",
                self.data.error
            )));
        }
        match self.page.as_str() {
            "flashcard" | "discover" => {
                let discover = self.page == "discover";
                content = content.child(
                    div()
                        .p_6()
                        .rounded_2xl()
                        .bg(cx.theme().accent)
                        .flex()
                        .justify_between()
                        .items_center()
                        .gap_4()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child(div().text_xs().text_color(cx.theme().primary).child(
                                    if discover {
                                        "EXPLORE & LEARN"
                                    } else {
                                        "A LITTLE, EVERY DAY"
                                    },
                                ))
                                .child(div().text_xl().font_weight(FontWeight::SEMIBOLD).child(
                                    if discover {
                                        "让好奇心，带你学下一句。"
                                    } else {
                                        "把遇见的表达，变成熟悉。"
                                    },
                                )),
                        )
                        .child(
                            div()
                                .text_right()
                                .flex_shrink_0()
                                .child(
                                    div()
                                        .text_2xl()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(cx.theme().primary)
                                        .child(self.cards.len().to_string()),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(if self.due && !discover {
                                            "待学卡片"
                                        } else {
                                            "可学卡片"
                                        }),
                                ),
                        ),
                );
                content = content.child(
                    div()
                        .flex()
                        .gap_2()
                        .flex_wrap()
                        .child(self.button("previous", "上一条", Action::Previous, cx))
                        .child(self.button("next", "下一条", Action::Next, cx).primary())
                        .child(self.button("favorite", "收藏 / 取消", Action::Favorite, cx))
                        .child(format!(
                            "{} / {}",
                            if self.cards.is_empty() {
                                0
                            } else {
                                self.index + 1
                            },
                            self.cards.len()
                        )),
                );
                if discover {
                    content = content.child(
                        div()
                            .flex()
                            .gap_2()
                            .child(self.button(
                                "discover-all",
                                "全部内容",
                                Action::Saved(false),
                                cx,
                            ))
                            .child(self.button(
                                "discover-saved",
                                "我的收藏",
                                Action::Saved(true),
                                cx,
                            )),
                    );
                }
                if !discover {
                    content = content.child(
                        div()
                            .flex()
                            .gap_2()
                            .flex_wrap()
                            .child(self.button("due", "计划复习", Action::ReviewMode(true), cx))
                            .child(self.button("browse", "自由浏览", Action::ReviewMode(false), cx))
                            .children(
                                [
                                    ("all", "全部"),
                                    ("word", "单词"),
                                    ("sentence", "句子"),
                                    ("clip", "听力片段"),
                                    ("video", "视频片段"),
                                    ("ai", "AI 问答"),
                                ]
                                .into_iter()
                                .map(|(kind, label)| {
                                    self.button(
                                        format!("filter-{kind}"),
                                        label,
                                        Action::Filter(kind.into()),
                                        cx,
                                    )
                                }),
                            ),
                    );
                }
                if let Some(card) = self.current_card() {
                    let back = if self.revealed {
                        card.back.clone()
                    } else {
                        "点击翻面查看释义".into()
                    };
                    content = content.child(
                        div()
                            .p_8()
                            .min_h(px(310.))
                            .rounded_2xl()
                            .border_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().background)
                            .shadow_sm()
                            .flex()
                            .flex_col()
                            .gap_6()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .text_xs()
                                            .text_color(cx.theme().primary)
                                            .child(design::icon("flashcard", cx.theme().primary))
                                            .child(match card.kind.as_str() {
                                                "word" => "单词卡片",
                                                "sentence" => "句子卡片",
                                                "clip" => "听力片段",
                                                "video" => "视频片段",
                                                _ => "AI 问答",
                                            }),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(format!(
                                                "{:02}  /  {:02}",
                                                self.index + 1,
                                                self.cards.len()
                                            )),
                                    ),
                            )
                            .child(
                                div()
                                    .py_4()
                                    .text_size(px(if card.kind == "word" { 36. } else { 23. }))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(TextView::markdown(
                                        "card-front",
                                        card.front.clone(),
                                        window,
                                        cx,
                                    )),
                            )
                            .child(
                                div()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(TextView::markdown("card-back", back, window, cx)),
                            )
                            .child(
                                div()
                                    .mt_4()
                                    .pt_5()
                                    .border_t_1()
                                    .border_color(cx.theme().border)
                                    .flex()
                                    .justify_between()
                                    .items_center()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(if self.revealed {
                                                "回想一次，再选择熟悉程度"
                                            } else {
                                                "先回想，再查看答案 · Space"
                                            }),
                                    )
                                    .child(
                                        self.button(
                                            "flip",
                                            if self.revealed {
                                                "收起答案"
                                            } else {
                                                "查看答案  →"
                                            },
                                            Action::Flip,
                                            cx,
                                        )
                                        .primary(),
                                    ),
                            ),
                    );
                    if !card.media.is_empty() {
                        if card.kind == "video" {
                            content = content.child(
                                div()
                                    .id("card-video-picture")
                                    .relative()
                                    .w_full()
                                    .h(px(300.))
                                    .overflow_hidden()
                                    .bg(rgb(0x090e0c))
                                    .child(self.video_viewport_probe(cx))
                                    .when_some(
                                        self.media
                                            .snapshot
                                            .frame
                                            .clone()
                                            .filter(|_| self.media.snapshot.path == card.media),
                                        |view, image| {
                                            view.child(
                                                img(image)
                                                    .absolute()
                                                    .top_0()
                                                    .left_0()
                                                    .size_full()
                                                    .object_fit(ObjectFit::Contain),
                                            )
                                        },
                                    ),
                            );
                        }
                        content = content.child(self.transport(cx));
                    }
                    if !discover && self.due {
                        content = content.child(
                            div().flex().gap_3().children(
                                [
                                    ("again", "忘了 · 10 分钟"),
                                    ("hard", "模糊"),
                                    ("good", "认识"),
                                ]
                                .into_iter()
                                .map(|(grade, label)| {
                                    self.button(
                                        format!("grade-{grade}"),
                                        label,
                                        Action::Grade(grade.into()),
                                        cx,
                                    )
                                    .disabled(!self.revealed)
                                }),
                            ),
                        );
                    }
                } else {
                    content =
                        content.child(div().p_8().text_lg().child(if self.due && !discover {
                            "今日待复习内容已完成，可切换自由浏览或导入新材料。"
                        } else {
                            "还没有学习内容，请导入词条、句子或带字幕的音频。"
                        }));
                }
                content = content.child(
                    div()
                        .flex()
                        .gap_2()
                        .flex_wrap()
                        .child(self.button(
                            "import-words",
                            "导入文本",
                            Action::Pick("words".into()),
                            cx,
                        ))
                        .child(self.button(
                            "import-dir",
                            "导入目录",
                            Action::Pick("directory".into()),
                            cx,
                        ))
                        .child(self.button(
                            "import-anki",
                            "Anki 导入",
                            Action::Pick("anki".into()),
                            cx,
                        )),
                );
                if !discover {
                    content = content.child(
                        div()
                            .flex()
                            .gap_2()
                            .child(self.button("edit-card", "编辑此卡", Action::EditCard, cx))
                            .child(self.button(
                                "suspend-card",
                                "暂停此卡复习",
                                Action::Suspend(true),
                                cx,
                            ))
                            .child(self.button(
                                "resume-card",
                                "恢复此卡复习",
                                Action::Suspend(false),
                                cx,
                            )),
                    );
                }
                content = content.child(
                    self.button(
                        "toggle-composer",
                        if self.composer_open {
                            "收起句子编辑"
                        } else {
                            "＋ 添加句子卡"
                        },
                        Action::Composer,
                        cx,
                    )
                    .ghost(),
                );
                if self.composer_open || self.editing.is_some() {
                    content = content.child(
                        panel(cx)
                            .child(if self.editing.is_some() {
                                "编辑卡片"
                            } else {
                                "添加句子卡"
                            })
                            .child(self.field("front", "原文"))
                            .child(self.field("back", "翻译"))
                            .child(if self.editing.is_some() {
                                self.button(
                                    "save-edited-card",
                                    "保存修改",
                                    Action::SaveEditedCard,
                                    cx,
                                )
                            } else {
                                self.button("sentence-save", "保存句子卡", Action::SaveSentence, cx)
                            }),
                    );
                }
            }
            "video" => {
                let download_panel = panel(cx)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .p_2()
                                    .rounded_lg()
                                    .bg(cx.theme().accent)
                                    .child(design::icon("video", cx.theme().primary)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child("从 bilibili 开始一堂课"),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child("收藏喜欢的视频，慢慢听懂每一句。"),
                                    ),
                            ),
                    )
                    .child(self.field("bilibili", "完整视频链接或 BV / av 编号"))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                self.button("download", "下载并导入", Action::Download, cx)
                                    .primary()
                                    .disabled(self.tasks.cancel.is_some()),
                            )
                            .child(
                                self.button("read-parts", "读取分P / 选集", Action::ReadParts, cx)
                                    .disabled(self.tasks.cancel.is_some()),
                            )
                            .child(
                                self.button("cancel-download", "取消下载", Action::Cancel, cx)
                                    .disabled(self.tasks.cancel.is_none()),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("自动识别分P；点击选集，未下载的集数会先下载再播放。"),
                    );
                let available = (f32::from(window.viewport_size().width)
                    - if self.navigation_open { 224. } else { 0. }
                    - 32.)
                    .max(300.);
                let stacked = available < 780.;
                let side_width = self
                    .video
                    .panel_width
                    .clamp(300., (available * 0.48).max(300.));
                let picture_height = if stacked {
                    (available * 9. / 16.).clamp(220., 360.)
                } else {
                    ((available - side_width - 12.) * 9. / 16.).clamp(260., 410.)
                };
                let board_height = picture_height + 114.;
                let mut stage = div()
                    .flex_1()
                    .when(stacked, |v| v.w_full().flex_none())
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .rounded_2xl()
                    .overflow_hidden()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(rgb(0x0c1612))
                    .child(
                        div()
                            .flex_shrink_0()
                            .px_4()
                            .py_2()
                            .bg(cx.theme().muted)
                            .flex()
                            .items_center()
                            .flex_wrap()
                            .gap_2()
                            .child(
                                div().flex_1().min_w_0().text_sm().truncate().child(
                                    self.media
                                        .path
                                        .as_ref()
                                        .and_then(|p| p.file_name())
                                        .map(|v| v.to_string_lossy().to_string())
                                        .unwrap_or_else(|| "视频播放器".into()),
                                ),
                            )
                            .when_some(
                                self.video
                                    .series
                                    .as_ref()
                                    .and_then(|s| s.current.map(|i| (i, s.parts.len()))),
                                |v, (i, count)| {
                                    v.child(self.button(
                                        "player-parts",
                                        format!("P{} / {} ▾", i + 1, count),
                                        Action::VideoPanel(1),
                                        cx,
                                    ))
                                    .child(
                                        self.button(
                                            "player-prev-part",
                                            "‹",
                                            Action::SelectPart(i.saturating_sub(1)),
                                            cx,
                                        )
                                        .tooltip("上一 P")
                                        .disabled(self.tasks.cancel.is_some() || i == 0),
                                    )
                                    .child(
                                        self.button(
                                            "player-next-part",
                                            "›",
                                            Action::SelectPart(i + 1),
                                            cx,
                                        )
                                        .tooltip("下一 P")
                                        .disabled(self.tasks.cancel.is_some() || i + 1 >= count),
                                    )
                                },
                            )
                            .child(
                                self.player_menu(
                                    "video-collection-picker",
                                    format!("切换收藏 · {}", self.video.collection.items.len()),
                                    self.video
                                        .collection
                                        .items
                                        .iter()
                                        .map(|v| {
                                            (
                                                if self
                                                    .video
                                                    .collection
                                                    .items
                                                    .iter()
                                                    .filter(|other| other.title == v.title)
                                                    .count()
                                                    > 1
                                                {
                                                    format!(
                                                        "{} · {}",
                                                        v.title,
                                                        v.path
                                                            .parent()
                                                            .and_then(|p| p.file_name())
                                                            .unwrap_or_default()
                                                            .to_string_lossy()
                                                    )
                                                } else {
                                                    v.title.clone()
                                                },
                                                Action::VideoSelect(v.id.clone()),
                                                self.video.collection.selected.as_ref()
                                                    == Some(&v.id),
                                                self.tasks.cancel.is_some()
                                                    || (v.playable_path().is_none()
                                                        && v.series.is_none()),
                                            )
                                        })
                                        .collect(),
                                    cx,
                                ),
                            )
                            .child(
                                self.button(
                                    "video-english-transcript",
                                    "英文文稿",
                                    Action::VideoTranscript,
                                    cx,
                                )
                                .selected(self.video.transcript_open),
                            )
                            .child(self.button(
                                "video-more",
                                if self.video.tools_open {
                                    "收起更多 ▴"
                                } else {
                                    "更多 ▾"
                                },
                                Action::VideoTools,
                                cx,
                            )),
                    );
                stage = stage.child(self.video_picture(Some(px(picture_height)), cx));
                stage = stage.child(self.transport(cx));
                let count = self.video.series.as_ref().map_or(0, |s| s.parts.len());
                let tabs = div()
                    .flex()
                    .items_center()
                    .flex_wrap()
                    .gap_1()
                    .p_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .children(
                        [
                            (
                                3,
                                format!("我的视频 · {}", self.video.collection.items.len()),
                            ),
                            (0, format!("字幕 · {}", self.video.cues.len())),
                            (1, format!("选集 · {count}")),
                            (2, "AI 提问".into()),
                        ]
                        .into_iter()
                        .map(|(tab, label)| {
                            self.button(
                                format!("video-tab-{tab}"),
                                label,
                                Action::VideoPanel(tab),
                                cx,
                            )
                            .selected(self.video.tab == tab)
                            .flex_none()
                        }),
                    );
                let mut study = div()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .rounded_2xl()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().background)
                    .when(stacked, |v| v.w_full().h(px(480.)))
                    .when(!stacked, |v| {
                        v.w(px(side_width)).h(px(board_height)).flex_shrink_0()
                    })
                    .child(tabs);
                match self.video.tab {
                    3 => {
                        study = study.child(self.video_collection_panel(cx));
                    }
                    0 => {
                        let transcript = centered_list(
                            &self.video.center,
                            self.video.scroll.clone(),
                            self.video.follow && !self.has_subtitle_selection(),
                            self.video.active,
                            cx.processor(|this, i: usize, _, cx| {
                                let Some(cue) = this.video.cues.get(i) else {
                                    return div().into_any_element();
                                };
                                let active = Some(i)
                                    == video_subtitles::active(
                                        &this.video.cues,
                                        this.media.snapshot.position,
                                    );
                                div()
                                    .id(("video-cue", i))
                                    .w_full()
                                    .px_3()
                                    .py_2()
                                    .flex()
                                    .gap_2()
                                    .cursor_pointer()
                                    .border_l_2()
                                    .border_color(if active {
                                        cx.theme().primary
                                    } else {
                                        cx.theme().background
                                    })
                                    .bg(if active {
                                        cx.theme().accent
                                    } else {
                                        cx.theme().background
                                    })
                                    .child(
                                        div()
                                            .w(px(38.))
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
                                            .gap_1()
                                            .child(
                                                {
                                                    let (_,translation)=subtitles::languages(std::slice::from_ref(cue));
                                                    div().flex().flex_col().gap_1()
                                                        .child(this.marked_cue(false,i,cue,14.,false,cx))
                                                        .when(!translation.is_empty(),|v|v.child(div().text_sm().text_color(cx.theme().muted_foreground).child(this.subtitle_fragment(subtitle_text_ui::Surface::Video,i,None,cx))))
                                                        .into_any_element()
                                                },
                                            )
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_wrap()
                                                    .gap_2()
                                                    .child(
                                                        this.button(
                                                            format!("video-cue-copy-{i}"),
                                                            "复制",
                                                            Action::VideoCopySubtitle(Some(i)),
                                                            cx,
                                                        )
                                                        .ghost()
                                                        .on_mouse_down(
                                                            MouseButton::Left,
                                                            |_, _, cx| cx.stop_propagation(),
                                                        ),
                                                    )
                                                    .child(
                                                        this.button(
                                                            format!(
                                                                "video-cue-repeat-infinite-{i}"
                                                            ),
                                                            "无限重复",
                                                            Action::VideoSentenceRepeat(
                                                                Some(i),
                                                                false,
                                                            ),
                                                            cx,
                                                        )
                                                        .ghost()
                                                        .on_mouse_down(
                                                            MouseButton::Left,
                                                            |_, _, cx| cx.stop_propagation(),
                                                        ),
                                                    )
                                                    .child(
                                                        this.button(
                                                            format!("video-cue-repeat-three-{i}"),
                                                            "重复三次",
                                                            Action::VideoSentenceRepeat(
                                                                Some(i),
                                                                true,
                                                            ),
                                                            cx,
                                                        )
                                                        .ghost()
                                                        .on_mouse_down(
                                                            MouseButton::Left,
                                                            |_, _, cx| cx.stop_propagation(),
                                                        ),
                                                    )
                                                    .child(
                                                        this.button(
                                                            format!("video-cue-ask-{i}"),
                                                            "问 AI",
                                                            Action::VideoAsk(video_qa::Scope::Cue(
                                                                i,
                                                            )),
                                                            cx,
                                                        )
                                                        .ghost()
                                                        .disabled(this.chat.running)
                                                        .on_mouse_down(
                                                            MouseButton::Left,
                                                            |_, _, cx| cx.stop_propagation(),
                                                        ),
                                                    ),
                                            ),
                                    )
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        if this.subtitle_dragged(){cx.stop_propagation();return;}
                                        this.handle(Action::VideoPlayCue(i), window, cx)
                                    }))
                                    .into_any_element()
                            }),
                        )
                        .flex_1()
                        .min_h_0()
                        .w_full();
                        study = study.child(div().px_3().py_2().flex().items_center().gap_1().flex_wrap()
                            .child(self.button("video-follow", if self.video.follow { "跟随 ✓" } else { "跟随" }, Action::VideoFollow, cx).ghost())
                            .child(self.button("video-import-english", "导入英文字幕", Action::Pick("video-subtitle".into()),cx).ghost().disabled(self.media.path.is_none()||self.tasks.cancel.is_some()||self.tasks.video_job.is_some()))
                            .child(self.button("video-translate-zh", "英文生成中文", Action::VideoTranslate,cx).disabled(self.video.cues.is_empty()||self.tasks.cancel.is_some()||self.tasks.video_job.is_some()||self.media.path.is_none()))
                            .child(div().flex_1())
                            .child(self.button("video-copy-selected","复制选中",Action::CopySelectedSubtitle,cx).ghost().disabled(!self.has_subtitle_selection()))
                            .child(self.button("video-copy-all", "复制全部", Action::VideoCopySubtitle(None), cx).ghost().disabled(self.video.cues.is_empty()))
                            .child(self.button("video-translate-all", "全部P翻译中文", Action::VideoTranslateAll,cx)
                                .disabled(self.video.series.is_none()||self.tasks.cancel.is_some()||self.tasks.video_job.is_some()))
                            .child(self.button("video-transcribe-all", "全部P生成字幕", Action::VideoTranscribeAll,cx)
                                .disabled(self.tasks.cancel.is_some()||self.tasks.video_job.is_some()||self.video.series.is_none()||self.media.path.is_none()))
                            .child(self.button("video-transcribe", if self.video.cues.is_empty() { "生成字幕" } else { "重新生成" }, Action::VideoTranscribe, cx)
                                .disabled(self.tasks.cancel.is_some() || self.tasks.video_job.is_some() || self.media.path.is_none())))
                            .when(self.video.cues.is_empty(), |v| v.child(div().p_5().text_sm().text_color(cx.theme().muted_foreground)
                                .child("生成或导入字幕后，点击句子即可播放。复制与 AI 提问位于每句下方。")))
                            .when(!self.video.cues.is_empty(), |v| v.child(transcript));
                    }
                    1 => {
                        if let Some(series) = &self.video.series {
                            let current = series.current;
                            let parts = uniform_list(
                                "bilibili-parts",
                                count,
                                cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                                    range
                                        .map(|i| {
                                            let series = this.video.series.as_ref().unwrap();
                                            let part = &series.parts[i];
                                            let playing = series.current == Some(i)
                                                && part.path.as_ref() == this.media.path.as_ref();
                                            div()
                                                .id(("bili-part", i))
                                                .h(px(64.))
                                                .w_full()
                                                .px_3()
                                                .py_2()
                                                .flex()
                                                .items_center()
                                                .gap_2()
                                                .cursor_pointer()
                                                .bg(if playing {
                                                    cx.theme().accent
                                                } else {
                                                    cx.theme().background
                                                })
                                                .child(
                                                    div()
                                                        .w(px(32.))
                                                        .flex_shrink_0()
                                                        .text_sm()
                                                        .text_color(cx.theme().primary)
                                                        .child(format!("P{}", part.number)),
                                                )
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .min_w_0()
                                                        .flex()
                                                        .flex_col()
                                                        .gap_1()
                                                        .child(
                                                            div()
                                                                .text_sm()
                                                                .truncate()
                                                                .child(part.title.clone()),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_xs()
                                                                .text_color(
                                                                    cx.theme().muted_foreground,
                                                                )
                                                                .child(format!(
                                                                    "{} · {}",
                                                                    clock(part.duration),
                                                                    if playing {
                                                                        "正在播放"
                                                                    } else if series
                                                                        .cached(i)
                                                                        .is_some()
                                                                    {
                                                                        "已下载"
                                                                    } else {
                                                                        "点击下载并播放"
                                                                    }
                                                                )),
                                                        ),
                                                )
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        if this.tasks.cancel.is_none() {
                                                            this.handle(
                                                                Action::SelectPart(i),
                                                                window,
                                                                cx,
                                                            );
                                                        }
                                                    },
                                                ))
                                                .into_any_element()
                                        })
                                        .collect::<Vec<_>>()
                                }),
                            )
                            .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::FitList)
                            .track_scroll(self.video.parts_scroll.clone())
                            .size_full()
                            .pr_3();
                            study = study
                                .child(div().px_3().py_2().flex().flex_col().gap_1()
                                    .child(self.button("parts-translate-all", "全部P翻译中文",Action::VideoTranslateAll,cx).outline().disabled(self.tasks.cancel.is_some()||self.tasks.video_job.is_some()||self.video.series.is_none()))
                                    .child(self.button("parts-transcribe-all", "全部P生成字幕",Action::VideoTranscribeAll,cx).primary().disabled(self.tasks.cancel.is_some()||self.tasks.video_job.is_some()||self.media.path.is_none()))
                                    .child(div().text_xs().text_color(cx.theme().muted_foreground).child("已有字幕跳过 · 未下载的P先下载 · 可切换播放，退出应用会停止任务")))
                                .child(
                                    div()
                                        .px_3()
                                        .py_2()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .flex_wrap()
                                        .child(
                                            self.button(
                                                "previous-part",
                                                "上一P",
                                                Action::SelectPart(
                                                    current.unwrap_or(0).saturating_sub(1),
                                                ),
                                                cx,
                                            )
                                            .disabled(
                                                self.tasks.cancel.is_some()
                                                    || current.is_none_or(|i| i == 0),
                                            ),
                                        )
                                        .child(
                                            self.button(
                                                "next-part",
                                                "下一P",
                                                Action::SelectPart(current.unwrap_or(0) + 1),
                                                cx,
                                            )
                                            .disabled(
                                                self.tasks.cancel.is_some()
                                                    || current.is_none_or(|i| i + 1 >= count),
                                            ),
                                        )
                                        .child(
                                            self.button(
                                                "auto-next-part",
                                                if self.video.auto_next_part {
                                                    "自动连播 ✓"
                                                } else {
                                                    "自动连播"
                                                },
                                                Action::AutoNextPart,
                                                cx,
                                            )
                                            .ghost(),
                                        ),
                                )
                                .child(
                                    div()
                                        .id("bilibili-parts-viewport")
                                        .relative()
                                        .flex_1()
                                        .min_h_0()
                                        .overflow_hidden()
                                        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                                        .child(parts)
                                        .child(
                                            Scrollbar::vertical(&self.video.parts_scroll)
                                                .scrollbar_show(ScrollbarShow::Always),
                                        ),
                                );
                        } else {
                            study = study
                                .child(
                                    div()
                                        .p_5()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("从 bilibili 导入或读取分P后，在这里选择集数。"),
                                )
                                .child(self.button(
                                    "show-import",
                                    "导入视频",
                                    Action::VideoTools,
                                    cx,
                                ));
                        }
                    }
                    _ => study = study.child(self.assistant_view(window, cx)),
                }
                let divider = div()
                    .id("video-panel-resize")
                    .w(px(12.))
                    .h(px(board_height))
                    .flex_shrink_0()
                    .cursor(CursorStyle::ResizeLeftRight)
                    .flex()
                    .justify_center()
                    .items_center()
                    .child(
                        div()
                            .w(px(3.))
                            .h(px(40.))
                            .rounded_full()
                            .bg(cx.theme().border),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, _, _| {
                            this.video.resize = Some((event.position.x, side_width));
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, _| this.video.resize = None),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this, _, _, _| this.video.resize = None),
                    );
                content = content.child(
                    div()
                        .flex()
                        .items_start()
                        .when(stacked, |v| v.flex_col().gap_3())
                        .child(stage)
                        .when(!stacked, |v| v.child(divider))
                        .child(study),
                );
                if self.video.transcript_open {
                    content = content.child(self.video_transcript_view(window, cx));
                }
                if self.video.tools_open || self.media.path.is_none() {
                    content = content.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(self.button(
                                "open-video",
                                "打开本地视频",
                                Action::Pick("video".into()),
                                cx,
                            ))
                            .child(
                                self.button(
                                    "video-export-srt",
                                    "导出 SRT",
                                    Action::VideoSubtitleExport,
                                    cx,
                                )
                                .disabled(self.video.cues.is_empty()),
                            ),
                    );
                    content = content.child(download_panel);
                    let mut tracks = div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child("字幕轨道 · 主字幕 / 第二字幕")
                        .child(self.button(
                            "video-external-sub",
                            "加载外部字幕",
                            Action::Pick("video-subtitle".into()),
                            cx,
                        ))
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(self.button(
                                    "sub-off",
                                    "关闭主字幕",
                                    Action::Subtitle(0, false),
                                    cx,
                                ))
                                .child(self.button(
                                    "secondary-off",
                                    "关闭第二字幕",
                                    Action::Subtitle(0, true),
                                    cx,
                                )),
                        );
                    for track in &self.media.snapshot.tracks {
                        if track["type"] == "sub" {
                            let key = track["id"].as_i64().unwrap_or(0);
                            let label = track["title"]
                                .as_str()
                                .or(track["lang"].as_str())
                                .unwrap_or("字幕");
                            tracks = tracks.child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(self.button(
                                        format!("sub-{key}"),
                                        format!("主：{label}"),
                                        Action::Subtitle(key, false),
                                        cx,
                                    ))
                                    .child(self.button(
                                        format!("secondary-{key}"),
                                        format!("副：{label}"),
                                        Action::Subtitle(key, true),
                                        cx,
                                    )),
                            );
                        }
                    }
                    content = content
                        .child(tracks)
                        .child(self.field("front", "片段原文"))
                        .child(self.field("back", "片段翻译"));
                }
            }
            "library" => {
                content = content.child(self.audio_library_view(window, cx));
            }
            "listening" => {
                content = content.child(self.audio_study(window, cx));
            }
            "speaking" => {
                content = content
                    .child(self.field("reference", "朗读参考原文"))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .flex_wrap()
                            .child(
                                self.button("record", "开始录音", Action::Record, cx)
                                    .primary()
                                    .disabled(self.recording.is_some()),
                            )
                            .child(
                                self.button("stop-record", "停止录音", Action::StopRecord, cx)
                                    .disabled(self.recording.is_none()),
                            )
                            .child(self.button(
                                "pick-record",
                                "选择已有录音",
                                Action::Pick("recording".into()),
                                cx,
                            ))
                            .child(
                                self.button("score-record", "转写并对齐评分", Action::Score, cx)
                                    .disabled(self.tasks.cancel.is_some()),
                            ),
                    );
                if let Some(path) = &self.recorded {
                    content = content.child(
                        path.file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string(),
                    );
                }
                if !self.output.is_empty() {
                    content = content.child(TextView::markdown(
                        "speaking-output",
                        self.output.clone(),
                        window,
                        cx,
                    ));
                }
                for (i, record) in self.data.speaking.iter().enumerate() {
                    content = content.child(
                        panel(cx)
                            .child(format!(
                                "记录 {} · {:.0}%",
                                i + 1,
                                record["accuracy"].as_f64().unwrap_or(0.) * 100.
                            ))
                            .child(record["reference"].as_str().unwrap_or("").to_string())
                            .child(record["transcript"].as_str().unwrap_or("").to_string()),
                    );
                }
            }
            "writing" => {
                if let Some(view) = &self.writing {
                    content = content.child(view.clone());
                }
            }
            "pdf" => {
                content = content.child(self.document_view(window, cx));
            }
            "ocr" => {
                content = content.child(
                    self.button(
                        "ocr-image",
                        "导入图片并识字",
                        Action::Pick("ocr".into()),
                        cx,
                    )
                    .primary()
                    .disabled(self.tasks.cancel.is_some()),
                );
                if !self.output.is_empty() {
                    content = content.child(TextView::markdown(
                        "ocr-output",
                        self.output.clone(),
                        window,
                        cx,
                    ));
                }
                for record in &self.data.ocr {
                    content = content.child(
                        panel(cx)
                            .child(record["name"].as_str().unwrap_or("图片").to_string())
                            .child(record["text"].as_str().unwrap_or("").to_string()),
                    );
                }
            }
            "graph" => {
                let graph = clarora_gpui::view_data::graph();
                let nodes = graph["nodes"].as_array().map(Vec::as_slice).unwrap_or(&[]);
                content = content.child(Input::new(&self.inputs["search"]));
                let mut lists = std::collections::BTreeSet::new();
                for n in nodes {
                    if let Some(list) = n["list_num"].as_i64() {
                        lists.insert(list);
                    }
                }
                let mut filters = div().flex().gap_2().flex_wrap().child(self.button(
                    "graph-all",
                    "全部",
                    Action::GraphList(0),
                    cx,
                ));
                for list in lists {
                    filters = filters.child(self.button(
                        format!("graph-list-{list}"),
                        format!("List {list}"),
                        Action::GraphList(list),
                        cx,
                    ));
                }
                content = content.child(filters);
                let query = self.value("search", cx).to_lowercase();
                let selected = nodes.iter().find(|n| n["id"] == self.graph_node);
                if let Some(node) = selected {
                    let mut detail = panel(cx).child(
                        div()
                            .text_3xl()
                            .child(node["label"].as_str().unwrap_or("").to_string()),
                    );
                    for key in ["pron", "category", "kaofa", "etymology", "mnemonic"] {
                        if let Some(text) = node[key].as_str().filter(|s| !s.is_empty()) {
                            detail = detail.child(text.to_string());
                        }
                    }
                    for sense in node["senses"].as_array().into_iter().flatten() {
                        detail = detail.child(format!(
                            "{}  {}",
                            sense["en"].as_str().unwrap_or(""),
                            sense["zh"].as_str().unwrap_or("")
                        ));
                    }
                    let mut related = div().flex().gap_2().flex_wrap();
                    for (i, edge) in graph["links"].as_array().into_iter().flatten().enumerate() {
                        let other = if edge["source"] == self.graph_node {
                            &edge["target"]
                        } else if edge["target"] == self.graph_node {
                            &edge["source"]
                        } else {
                            continue;
                        };
                        if let Some(target) = nodes.iter().find(|n| n["id"] == *other) {
                            related = related.child(self.button(
                                format!("graph-edge-{i}"),
                                format!(
                                    "{} · {}",
                                    edge["link_type"].as_str().unwrap_or("关联"),
                                    target["label"].as_str().unwrap_or("")
                                ),
                                Action::GraphSelect(other.as_str().unwrap_or("").into()),
                                cx,
                            ));
                        }
                    }
                    content = content.child(detail.child(related));
                }
                let mut nodes_view = div().flex().gap_2().flex_wrap();
                for node in nodes.iter().filter(|n| {
                    (self.graph_list == 0 || n["list_num"] == self.graph_list)
                        && n["label"]
                            .as_str()
                            .unwrap_or("")
                            .to_lowercase()
                            .contains(&query)
                }) {
                    nodes_view = nodes_view.child(self.button(
                        format!("graph-node-{}", node["id"]),
                        node["label"].as_str().unwrap_or("").to_string(),
                        Action::GraphSelect(node["id"].as_str().unwrap_or("").into()),
                        cx,
                    ));
                }
                content = content.child(nodes_view);
                for row in self
                    .data
                    .words
                    .iter()
                    .filter(|r| text(r, "word").to_lowercase().contains(&query))
                    .take(300)
                {
                    content = content.child(
                        div()
                            .flex()
                            .gap_4()
                            .child(text(row, "word"))
                            .child(text(row, "meaning")),
                    );
                }
            }
            "stats" => {
                let rows = &self.data.stats;
                let listening: f64 = rows
                    .iter()
                    .filter_map(|r| r["listen_seconds"].as_f64())
                    .sum();
                let review: f64 = rows.iter().filter_map(|r| r["review_count"].as_f64()).sum();
                let pomodoro: f64 = rows
                    .iter()
                    .filter_map(|r| r["pomodoro_count"].as_f64())
                    .sum();
                content = content.child(
                    div()
                        .flex()
                        .gap_4()
                        .flex_wrap()
                        .child(
                            panel(cx)
                                .child("累计听力")
                                .child(format!("{:.1} 分钟", listening / 60.)),
                        )
                        .child(panel(cx).child("复习卡片").child(format!("{review:.0} 张")))
                        .child(
                            panel(cx)
                                .child("完成番茄钟")
                                .child(format!("{pomodoro:.0} 轮")),
                        ),
                );
                let mut heatmap = div().flex().gap_1().flex_wrap().max_w(px(650.));
                for day in (0..119).rev() {
                    let date = (chrono::Local::now() - chrono::Duration::days(day))
                        .format("%Y-%m-%d")
                        .to_string();
                    let count = rows
                        .iter()
                        .find(|r| text(r, "day") == date)
                        .and_then(|r| r["review_count"].as_f64())
                        .unwrap_or(0.);
                    heatmap =
                        heatmap.child(div().size(px(20.)).rounded_sm().bg(rgb(if count > 30. {
                            0x17604c
                        } else if count > 10. {
                            0x459d7a
                        } else if count > 0. {
                            0x9ed6b5
                        } else {
                            0xe6e9e4
                        })));
                }
                content = content.child("最近 17 周复习热力图").child(heatmap);
                let mut listening_chart = div().flex().gap_1().items_end().h(px(160.));
                let days = (0..14)
                    .rev()
                    .map(|day| {
                        let date = (chrono::Local::now() - chrono::Duration::days(day))
                            .format("%Y-%m-%d")
                            .to_string();
                        let seconds = rows
                            .iter()
                            .find(|r| text(r, "day") == date)
                            .and_then(|r| r["listen_seconds"].as_f64())
                            .unwrap_or(0.);
                        (date, seconds)
                    })
                    .collect::<Vec<_>>();
                let maximum = days.iter().map(|(_, seconds)| *seconds).fold(60., f64::max);
                for (day, seconds) in days {
                    listening_chart = listening_chart.child(
                        div()
                            .flex()
                            .flex_col()
                            .justify_end()
                            .items_center()
                            .w(px(45.))
                            .gap_1()
                            .child(format!("{:.0}m", seconds / 60.))
                            .child(
                                div()
                                    .w(px(24.))
                                    .h(px((seconds / maximum * 100.).max(2.) as f32))
                                    .bg(cx.theme().primary),
                            )
                            .child(day[5..].to_string()),
                    );
                }
                content = content.child("最近 14 天听力时长").child(listening_chart);
                let attempts = &self.data.speaking;
                let mut trend = div().flex().gap_2().items_end().h(px(140.));
                for record in attempts.iter().rev() {
                    let accuracy = record["score"]["accuracy"]
                        .as_f64()
                        .or_else(|| record["accuracy"].as_f64().map(|v| v * 100.))
                        .unwrap_or(0.);
                    trend = trend.child(
                        div()
                            .flex()
                            .flex_col()
                            .justify_end()
                            .items_center()
                            .gap_1()
                            .child(format!("{accuracy:.0}%"))
                            .child(
                                div()
                                    .w(px(20.))
                                    .h(px(accuracy.clamp(2., 100.) as f32))
                                    .bg(cx.theme().primary),
                            ),
                    );
                }
                content = content.child("最近 20 次跟读准确度").child(trend);

                for row in rows.iter().rev().take(14) {
                    content = content.child(format!(
                        "{} · 复习 {} · 听力 {:.1} 分钟",
                        text(row, "day"),
                        text(row, "review_count"),
                        row["listen_seconds"].as_f64().unwrap_or(0.) / 60.
                    ));
                }
            }
            "meditation" => {
                content = content.child(
                    panel(cx)
                        .items_center()
                        .p_10()
                        .child(div().text_3xl().child(clock(self.timer_remaining as f64)))
                        .child(if self.timer.focus() {
                            "专注时间"
                        } else {
                            "休息时间"
                        })
                        .child(
                            div()
                                .flex()
                                .gap_3()
                                .child(self.button(
                                    "timer-pause",
                                    self.focus_timer_toggle_label(),
                                    Action::TimerPause,
                                    cx,
                                ))
                                .child(self.button("timer-reset", "重置", Action::TimerReset, cx)),
                        )
                        .child(div().flex().gap_2().flex_wrap().children(
                            [5u64, 10, 15, 25, 45, 60].into_iter().map(|minutes| {
                                self.button(
                                    format!("timer-{minutes}"),
                                    format!("{minutes} 分钟"),
                                    Action::Timer(minutes * 60),
                                    cx,
                                )
                            }),
                        ))
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("新一轮开始响一声，每轮结束响两声；暂停和继续不重复提醒。"),
                        )
                        .child(self.button(
                            "background-sound",
                            "选择本机背景音乐",
                            Action::Pick("background".into()),
                            cx,
                        )),
                );
            }
            "settings" => {
                content = content.child(self.settings_view(window, cx));
            }
            _ => {}
        }
        content.into_any_element()
    }
}
