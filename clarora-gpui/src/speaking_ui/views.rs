//! Speaking page views, separated from persistence and playback control.
use super::*;

impl AppView {
    pub(crate) fn speaking_library_view(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let available =
            f32::from(window.viewport_size().width) - 224. * self.navigation_progress() - 64.;
        let detail_width = (available - 42.).max(0.);
        let compare_reviews = detail_width >= 760.;
        let review_height = (f32::from(window.viewport_size().height) - 320.).clamp(360., 720.);
        let locked = self.speaking_locked();
        let muted = cx.theme().muted_foreground;
        let layout = div()
            .w_full()
            .min_w_0()
            .child(panel(cx).w_full().min_w_0().when_else(
                self.speaking.current.is_some(),
                |detail| {
                    let question = self.speaking.current.as_ref().unwrap();
                    detail
                        .child(self.speaking_question_heading(question, locked, cx))
                        .children(
                            (self.speaking_question_dirty(cx) || self.speaking.saving)
                                .then(|| self.speaking_question_actions(question, locked, cx)),
                        )
                        .child(self.speaking_prompt_card(locked, cx))
                        .child({
                            let history = (self.speaking.show_reviews
                                && !question.reviews.is_empty())
                            .then(|| self.speaking_review_history(question, window, cx));
                            match history {
                                Some(history) if compare_reviews => {
                                    let left_width = (detail_width * 0.5).clamp(400., 800.);
                                    let review_width = (detail_width - left_width - 16.).max(360.);
                                    let draft_height = (review_height - 150.).max(200.);
                                    let answer = self.speaking_answer_draft_view(
                                        question,
                                        locked,
                                        Some(px(draft_height)),
                                        cx,
                                    );
                                    div()
                                        .w_full()
                                        .flex()
                                        .items_start()
                                        .gap_4()
                                        .child(
                                            div().w(px(left_width)).flex_shrink_0().child(answer),
                                        )
                                        .child(
                                            div()
                                                .relative()
                                                .w(px(review_width))
                                                .h(px(review_height))
                                                .overflow_hidden()
                                                .child(
                                                    div()
                                                        .id("speaking-review-panel")
                                                        .size_full()
                                                        .overflow_y_scroll()
                                                        .track_scroll(&self.speaking.review_scroll)
                                                        .on_scroll_wheel(|_, _, cx| {
                                                            cx.stop_propagation()
                                                        })
                                                        .child(history),
                                                )
                                                .child(
                                                    div()
                                                        .absolute()
                                                        .top_0()
                                                        .left_0()
                                                        .size_full()
                                                        .child(
                                                            Scrollbar::vertical(
                                                                &self.speaking.review_scroll,
                                                            )
                                                            .scrollbar_show(ScrollbarShow::Always),
                                                        ),
                                                ),
                                        )
                                        .into_any_element()
                                }
                                Some(history) => div()
                                    .w_full()
                                    .flex()
                                    .flex_col()
                                    .gap_4()
                                    .child(
                                        self.speaking_answer_draft_view(question, locked, None, cx),
                                    )
                                    .child(
                                        div()
                                            .relative()
                                            .w_full()
                                            .h(px(420.))
                                            .overflow_hidden()
                                            .child(
                                                div()
                                                    .id("speaking-review-panel")
                                                    .size_full()
                                                    .overflow_y_scroll()
                                                    .track_scroll(&self.speaking.review_scroll)
                                                    .on_scroll_wheel(|_, _, cx| {
                                                        cx.stop_propagation()
                                                    })
                                                    .child(history),
                                            )
                                            .child(
                                                div()
                                                    .absolute()
                                                    .top_0()
                                                    .left_0()
                                                    .size_full()
                                                    .child(
                                                        Scrollbar::vertical(
                                                            &self.speaking.review_scroll,
                                                        )
                                                        .scrollbar_show(ScrollbarShow::Always),
                                                    ),
                                            ),
                                    )
                                    .into_any_element(),
                                None => self
                                    .speaking_answer_draft_view(question, locked, None, cx)
                                    .into_any_element(),
                            }
                        })
                },
                |detail| {
                    detail
                        .child(
                            div()
                                .text_lg()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("题目详情"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(muted)
                                .child("创建一个分组，即可添加音频题目。"),
                        )
                },
            ));
        #[cfg(debug_assertions)]
        let layout = if let Some(bounds) = self.speaking.layout_probe.clone() {
            layout
                .on_children_prepainted(move |children, _, _| bounds.set(children.first().copied()))
        } else {
            layout
        };
        layout
    }

    pub(crate) fn speaking_question_heading(
        &self,
        question: &Question,
        locked: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        div()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("题目详情"),
            )
            .when_else(
                self.speaking.editing_title,
                |heading| {
                    let saved_title = question.title.clone();
                    heading.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div().flex_1().min_w_0().child(
                                    Input::new(&self.inputs["speaking_title"]).disabled(locked),
                                ),
                            )
                            .child(
                                Button::new("speaking-cancel-rename")
                                    .small()
                                    .outline()
                                    .label("取消改名")
                                    .disabled(locked)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.set("speaking_title", saved_title.clone(), window, cx);
                                        this.speaking.editing_title = false;
                                        cx.notify();
                                    })),
                            ),
                    )
                },
                |heading| {
                    heading.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .whitespace_normal()
                                    .text_xl()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(question.label()),
                            )
                            .child(
                                Button::new("speaking-rename-question")
                                    .small()
                                    .outline()
                                    .label("改名")
                                    .disabled(locked)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.speaking.editing_title = true;
                                        this.inputs["speaking_title"]
                                            .update(cx, |input, cx| input.focus(window, cx));
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("speaking-question-more")
                                    .small()
                                    .ghost()
                                    .label("更多")
                                    .tooltip("删除当前题目")
                                    .disabled(locked)
                                    .dropdown_menu({
                                        let owner = cx.entity().downgrade();
                                        move |mut menu, _, _| {
                                            let owner = owner.clone();
                                            menu =
                                                menu.item(PopupMenuItem::new("删除题目").on_click(
                                                    move |_, window, cx| {
                                                        let _ = owner.update(cx, |this, cx| {
                                                            this.handle(
                                                                Action::SpeakingDelete,
                                                                window,
                                                                cx,
                                                            );
                                                        });
                                                    },
                                                ));
                                            menu
                                        }
                                    }),
                            ),
                    )
                },
            )
    }

    pub(crate) fn open_speaking_batch_import_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        ensure!(self.page == "speaking", "请先进入口语练习");
        ensure!(!self.speaking_locked(), "请先等待当前任务完成");
        if self.import_command.trim().is_empty() {
            self.handle(Action::ConfigureCommandImport, window, cx);
            return Ok(());
        }
        let group_id = self
            .speaking
            .current
            .as_ref()
            .context("请先选择口语分组")?
            .group_id
            .clone();
        let group_name = self
            .speaking
            .groups
            .iter()
            .find(|group| group.id == group_id)
            .map(|group| group.name.clone())
            .unwrap_or_default();
        let view = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, window, cx| {
            let view = view.clone();
            let group_id = group_id.clone();
            dialog
                .title("按命令导入题目")
                .confirm()
                .w((window.viewport_size().width - px(32.)).min(px(520.)))
                .button_props(
                    DialogButtonProps::default()
                        .cancel_text("取消")
                        .ok_text("继续导入")
                        .ok_variant(ButtonVariant::Primary),
                )
                .child(div().text_sm().child(format!("当前分组：{group_name}")))
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("将执行「设置 → 悬浮窗与命令导入」中的本机命令。命令返回的每个音频都会在当前分组新建一道题；如果返回多个音频，会创建多道题。原始命令和音频文件不会被删除。"),
                )
                .on_ok(move |_, window, cx| {
                    view.update(cx, |this, cx| {
                        this.start_speaking_command_import_to(group_id.clone(), None, window, cx);
                        true
                    })
                    .unwrap_or(true)
                })
        });
        Ok(())
    }

    pub(crate) fn speaking_header_controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let locked = self.speaking_locked();
        let question = self.speaking.current.as_ref();
        let group_id = question.map(|question| question.group_id.as_str());
        let active_group = self
            .speaking
            .groups
            .iter()
            .find(|group| Some(group.id.as_str()) == group_id);
        let filter = self.speaking_tag_filter().map(str::to_owned);
        let groups: Vec<_> = self
            .speaking
            .groups
            .iter()
            .filter(|group| {
                filter.as_ref().is_none_or(|filter| {
                    group
                        .tags
                        .iter()
                        .any(|tag| tag.eq_ignore_ascii_case(filter))
                })
            })
            .cloned()
            .collect();
        let mut available_tags = std::collections::BTreeMap::new();
        for tag in self.speaking.groups.iter().flat_map(|group| &group.tags) {
            available_tags
                .entry(tag.to_ascii_lowercase())
                .or_insert_with(|| tag.clone());
        }
        let owner = cx.entity().downgrade();
        let filter_owner = cx.entity().downgrade();
        let active_id = group_id.map(str::to_owned);
        let questions: Vec<_> = active_group
            .map(|group| {
                self.speaking
                    .questions
                    .iter()
                    .filter(|question| question.group_id == group.id)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let count = questions.len();
        let answered = questions
            .iter()
            .filter(|question| !question.answer_draft.trim().is_empty())
            .count();
        let mut controls = div()
            .min_w_0()
            .flex_1()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .rounded_xl()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("分组"),
                    )
                    .child(
                        Button::new("speaking-select-group")
                            .outline()
                            .small()
                            .h(px(30.))
                            .disabled(locked || groups.is_empty())
                            .w(px(190.))
                            .child(div().flex_1().min_w_0().truncate().child(
                                active_group.map_or_else(
                                    || "选择分组".to_owned(),
                                    |group| group.name.clone(),
                                ),
                            ))
                            .dropdown_menu(move |mut menu, _, _| {
                                for group in &groups {
                                    let id = group.id.clone();
                                    let owner = owner.clone();
                                    menu = menu.item(
                                        PopupMenuItem::new(group.name.clone())
                                            .checked(active_id.as_deref() == Some(id.as_str()))
                                            .on_click(move |_, window, cx| {
                                                let _ = owner.update(cx, |this, cx| {
                                                    if this.may_switch_speaking_question(cx) {
                                                        this.open_speaking_group(&id, window, cx);
                                                    }
                                                    cx.notify();
                                                });
                                            }),
                                    );
                                }
                                menu.max_h(px(300.)).scrollable(true)
                            }),
                    )
                    .when(!available_tags.is_empty(), |row| {
                        row.child(
                            Button::new("speaking-filter-groups-by-tag")
                                .small()
                                .outline()
                                .h(px(30.))
                                .w(px(150.))
                                .child(div().flex_1().min_w_0().truncate().child(
                                    filter.as_ref().map_or_else(
                                        || "标签筛选".to_owned(),
                                        |tag| format!("#{tag} · 筛选"),
                                    ),
                                ))
                                .dropdown_menu(move |mut menu, _, _| {
                                    for tag in std::iter::once(None)
                                        .chain(available_tags.values().cloned().map(Some))
                                    {
                                        let owner = filter_owner.clone();
                                        let checked = match (&tag, &filter) {
                                            (Some(tag), Some(filter)) => {
                                                tag.eq_ignore_ascii_case(filter)
                                            }
                                            _ => tag == filter,
                                        };
                                        let label = tag.as_ref().map_or_else(
                                            || "全部分组".to_owned(),
                                            |tag| format!("#{tag}"),
                                        );
                                        menu = menu.item(
                                            PopupMenuItem::new(label).checked(checked).on_click(
                                                move |_, _, cx| {
                                                    let _ = owner.update(cx, |this, cx| {
                                                        this.speaking.tag_filter = tag.clone();
                                                        cx.notify();
                                                    });
                                                },
                                            ),
                                        );
                                    }
                                    menu.max_h(px(300.)).scrollable(true)
                                }),
                        )
                    })
                    .child(
                        Button::new("speaking-new-group")
                            .ghost()
                            .small()
                            .h(px(30.))
                            .disabled(locked)
                            .label("＋ 新建组")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.begin_speaking_group(window, cx);
                            })),
                    )
                    .when(active_group.is_some(), |row| {
                        row.child(
                            Button::new("speaking-toggle-group-tags")
                                .ghost()
                                .small()
                                .h(px(30.))
                                .label(if self.speaking.editing_group_tags {
                                    "收起标签"
                                } else {
                                    "管理标签"
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.speaking.editing_group_tags =
                                        !this.speaking.editing_group_tags;
                                    cx.notify();
                                })),
                        )
                    })
                    .when_some(
                        active_group.filter(|group| !group.tags.is_empty()),
                        |row, group| {
                            row.child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(
                                        group
                                            .tags
                                            .iter()
                                            .map(|tag| format!("#{tag}"))
                                            .collect::<Vec<_>>()
                                            .join("  "),
                                    ),
                            )
                        },
                    ),
            )
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("题目 · {count}")),
                    )
                    .child(
                        div()
                            .id("speaking-question-strip")
                            .flex_1()
                            .min_w(px(180.))
                            .flex()
                            .items_center()
                            .overflow_x_scroll()
                            .when(count == 0, |strip| {
                                strip.child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("本组还没有题目"),
                                )
                            })
                            .when(count > 0, |strip| {
                                strip.child(
                                    div()
                                        .flex_none()
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .p_1()
                                        .rounded_full()
                                        .bg(cx.theme().muted)
                                        .children(questions.into_iter().map(|item| {
                                            let selected = question
                                                .is_some_and(|question| question.id == item.id);
                                            let question_id = item.id.clone();
                                            let kind = match (
                                                item.audio.is_some(),
                                                item.text.trim().is_empty(),
                                            ) {
                                                (true, false) => "已转写",
                                                (true, true) => "待转写",
                                                (false, false) => "已存文本",
                                                (false, true) => "待添加音频",
                                            };
                                            let button = Button::new(SharedString::from(format!(
                                                "speaking-question-{}",
                                                item.id
                                            )))
                                            .small()
                                            .h(px(30.))
                                            .px_3()
                                            .rounded_full();
                                            let button = if selected {
                                                button.primary()
                                            } else {
                                                button.ghost()
                                            };
                                            button
                                                .disabled(locked)
                                                .label(item.label())
                                                .tooltip(format!("{} · {}", item.label(), kind))
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        if this
                                                            .speaking
                                                            .current
                                                            .as_ref()
                                                            .is_none_or(|current| {
                                                                current.id != question_id
                                                            })
                                                            && this.may_switch_speaking_question(cx)
                                                            && let Some(question) = this
                                                                .speaking
                                                                .questions
                                                                .iter()
                                                                .find(|question| {
                                                                    question.id == question_id
                                                                })
                                                                .cloned()
                                                        {
                                                            this.set_speaking_question(
                                                                question, window, cx,
                                                            );
                                                        }
                                                        cx.notify();
                                                    },
                                                ))
                                        })),
                                )
                            }),
                    )
                    .when(active_group.is_some(), |row| {
                        row.child(
                            div()
                                .flex_shrink_0()
                                .w(px(1.))
                                .h(px(20.))
                                .mx_1()
                                .bg(cx.theme().border),
                        )
                        .child(
                            Button::new("speaking-group-check")
                                .small()
                                .outline()
                                .h(px(30.))
                                .label("整组批改")
                                .tooltip(format!(
                                    "把本组 {answered} 道已写草稿的题目一起加入引用，检查前后答案是否重复"
                                ))
                                .disabled(locked || answered < 2)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.handle(Action::SpeakingGroupCheck, window, cx);
                                })),
                        )
                        .child(
                            Button::new("speaking-command-import-group")
                                .small()
                                .ghost()
                                .h(px(30.))
                                .disabled(locked)
                                .label("按命令导入")
                                .tooltip("用本机命令导入音频；返回多个音频时会创建多道题，执行前需确认")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.handle(Action::SpeakingBatchImport, window, cx);
                                })),
                        )
                        .child(
                            Button::new("speaking-new-question")
                                .small()
                                .primary()
                                .h(px(30.))
                                .disabled(locked)
                                .label("＋ 题目")
                                .tooltip("新建题目")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    if this.may_switch_speaking_question(cx)
                                        && let Some(group_id) = this
                                            .speaking
                                            .current
                                            .as_ref()
                                            .map(|q| q.group_id.clone())
                                        {
                                            this.set_speaking_question(
                                                Question::draft(group_id),
                                                window,
                                                cx,
                                            );
                                        }
                                    cx.notify();
                                })),
                        )
                    }),
            );
        if self.speaking.creating_group {
            controls = controls.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .w(px(220.))
                            .max_w_full()
                            .child(Input::new(&self.inputs["speaking_group"]).disabled(locked)),
                    )
                    .child(
                        Button::new("speaking-create-group")
                            .small()
                            .primary()
                            .label("创建分组")
                            .disabled(locked)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.create_speaking_group(window, cx)
                            })),
                    )
                    .child(
                        Button::new("speaking-cancel-group")
                            .small()
                            .outline()
                            .label("取消")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.speaking.creating_group = false;
                                cx.notify();
                            })),
                    ),
            );
        }
        if let Some(group) = active_group.filter(|_| self.speaking.editing_group_tags) {
            controls = controls.child(self.speaking_group_tags(group, locked, cx));
        }
        controls
    }

    fn pick_speaking_answer_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.speaking_locked() {
            return;
        }
        let Some(question) = &self.speaking.current else {
            return;
        };
        let question_id = question.id.clone();
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("导入作答草稿（.txt 或 .md）".into()),
        });
        cx.spawn_in(window, async move |this, window| {
            let path = match picker.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                _ => None,
            };
            let Some(path) = path else { return };
            let text = window
                .background_executor()
                .spawn(async move { speaking::read_answer_draft(&path) })
                .await;
            let _ = this.update_in(window, |this, window, cx| {
                if this.page != "speaking"
                    || this.speaking_locked()
                    || this
                        .speaking
                        .current
                        .as_ref()
                        .is_none_or(|question| question.id != question_id)
                {
                    return;
                }
                if let Err(error) =
                    text.and_then(|text| this.append_speaking_answer_draft(&text, window, cx))
                {
                    this.report_status(FeedbackKind::Error, format!("导入作答草稿失败：{error:#}"));
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn append_speaking_answer_draft(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let mut draft = self.value("speaking_answer_draft", cx);
        if !draft.is_empty() {
            draft.push_str("\n\n");
        }
        draft.push_str(text);
        speaking::validate_answer_draft(&draft)?;
        self.set("speaking_answer_draft", draft, window, cx);
        Ok(())
    }

    pub(crate) fn speaking_answer_draft_view(
        &self,
        question: &Question,
        locked: bool,
        editor_height: Option<Pixels>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let editor_height = editor_height.unwrap_or(px(160.));
        let draft = self.inputs["speaking_answer_draft"].read(cx).value();
        let words = clarora_gpui::writing::answer_word_count(&draft);
        let chars = draft.chars().count();
        let limit = speaking::MAX_ANSWER_DRAFT_CHARS;
        let target_words = self.speaking_target_words(cx);
        let word_status = match target_words {
            Some(target) if words < target => {
                format!("{words} / {target} 词 · 还差 {} 词", target - words)
            }
            Some(target) => format!("{words} / {target} 词 · 已达到目标"),
            None => format!("{words} 词"),
        };
        let has_draft = !draft.trim().is_empty();
        let has_prompt = !self.inputs["reference"].read(cx).value().trim().is_empty();
        let draft_dirty =
            draft.as_ref() != question.answer_draft || target_words != question.target_words;
        let save_error = self.speaking.draft_save_error.clone();
        let save_status = if let Some(error) = &save_error {
            format!("保存失败 · {error}")
        } else if self.speaking.saving {
            "正在保存…".into()
        } else if draft_dirty {
            "等待自动保存…".into()
        } else if question.answer_draft.is_empty() && question.target_words.is_none() {
            "输入后自动保存".into()
        } else {
            "草稿已保存".into()
        };
        let owner = cx.entity().downgrade();
        let draft_audio = tts::cached_preview(&self.lib, &draft, self.tts_voice, self.tts_speed);
        let playback = self.media.player.snapshot();
        let draft_audio_loaded = draft_audio
            .as_deref()
            .is_some_and(|path| self.speaking_playback_matches(path, &playback));
        let draft_audio_playing = draft_audio_loaded && !playback.paused && !playback.eof;
        let draft_audio_eof = draft_audio_loaded && playback.eof;
        let draft_audio_fraction = if draft_audio_loaded && playback.duration > 0. {
            (playback.position / playback.duration).clamp(0., 1.) as f32
        } else {
            0.
        };
        let draft_audio_time = if draft_audio_loaded {
            format!(
                "{} / {}",
                clock(playback.position),
                clock(playback.duration)
            )
        } else {
            String::new()
        };
        let tts_ready = self.data.tts_ready;
        let tts_busy = self.tasks.foreground.is_running();
        div()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("作答草稿"))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("目标词数"),
                            )
                            .child(
                                div().w(px(88.)).child(
                                    Input::new(&self.inputs["speaking_target_words"])
                                        .disabled(self.speaking.saving),
                                ),
                            ),
                    )
                    .child(
                        Button::new("speaking-import-answer-draft")
                            .small()
                            .outline()
                            .label("导入文本")
                            .tooltip("支持 UTF-8 的 .txt 和 .md，有内容时追加到末尾")
                            .disabled(locked)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.pick_speaking_answer_draft(window, cx)
                            })),
                    ),
            )
            .child(
                Input::new(&self.inputs["speaking_answer_draft"])
                    .w_full()
                    .h(editor_height)
                    .text_size(writing_ui::BODY_FONT_SIZE)
                    .line_height(relative(writing_ui::BODY_LINE_SPACING))
                    .disabled(self.speaking.saving),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .whitespace_normal()
                            .when(save_error.is_some(), |status| {
                                status.text_color(cx.theme().danger)
                            })
                            .child(save_status),
                    )
                    .when_some(save_error, |row, error| {
                        row.child(
                            Button::new("speaking-copy-draft-error")
                                .small()
                                .outline()
                                .label("复制错误")
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                        error.clone(),
                                    ));
                                })),
                        )
                    })
                    .child(
                        div()
                            .flex_shrink_0()
                            .when(chars > limit, |count| count.text_color(cx.theme().danger))
                            .child(format!("{word_status} · {chars} / {limit} 字符")),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("speaking-save-draft")
                            .small()
                            .outline()
                            .label("保存草稿")
                            .disabled(!draft_dirty || self.speaking.saving || chars > limit)
                            .on_click(cx.listener(|this, _, _, cx| {
                                let _ = this.flush_speaking_draft(cx);
                            })),
                    )
                    .when(has_draft, |bar| {
                        bar.child(
                            Button::new("speaking-review-draft")
                                .small()
                                .primary()
                                .label("AI 批改 ▾")
                                .tooltip(if has_prompt {
                                    "先保存草稿，再发送题目文字和草稿给设置中的聊天模型"
                                } else {
                                    "请先转写题目音频"
                                })
                                .disabled(locked || !has_prompt || chars > limit)
                                .dropdown_menu(move |mut menu, _, _| {
                                    for mode in [ReviewMode::Overview, ReviewMode::LineEdits] {
                                        let owner = owner.clone();
                                        menu =
                                            menu.item(PopupMenuItem::new(mode.label()).on_click(
                                                move |_, _, cx| {
                                                    let _ = owner.update(cx, |app, cx| {
                                                        app.start_speaking_review(mode, cx)
                                                    });
                                                },
                                            ));
                                    }
                                    menu
                                }),
                        )
                    })
                    .when(!question.reviews.is_empty(), |bar| {
                        bar.child(
                            Button::new("speaking-show-reviews")
                                .small()
                                .outline()
                                .label(if self.speaking.show_reviews {
                                    "收起批改结果".to_owned()
                                } else {
                                    format!("批改结果 · {}", question.reviews.len())
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.speaking.show_reviews = !this.speaking.show_reviews;
                                    cx.notify();
                                })),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .when_some(draft_audio.clone(), |row, path| {
                        row.child(
                            Button::new("speaking-draft-audio-play")
                                .small()
                                .outline()
                                .label(if draft_audio_playing {
                                    "暂停读音"
                                } else if draft_audio_eof {
                                    "重新播放"
                                } else {
                                    "播放读音"
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Err(error) = this.play_speaking_draft_audio(path.clone())
                                    {
                                        this.report_status(
                                            FeedbackKind::Error,
                                            format!("播放草稿读音失败：{error:#}"),
                                        );
                                    }
                                    cx.notify();
                                })),
                        )
                        .when(draft_audio_loaded, |row| {
                            row.child(
                                div().flex_1().min_w(px(120.)).child(
                                    Progress::new()
                                        .value(draft_audio_fraction)
                                        .bg(cx.theme().primary),
                                ),
                            )
                            .child(div().flex_shrink_0().child(draft_audio_time.clone()))
                        })
                    })
                    .when(draft_audio.is_none(), |row| {
                        row.child(
                            Button::new("speaking-draft-audio-generate")
                                .small()
                                .outline()
                                .label("生成读音")
                                .tooltip(if tts_ready {
                                    "使用音频页选择的 Kokoro 音色和语速，生成本稿读音并播放"
                                } else {
                                    "请先在音频页下载 Kokoro 模型"
                                })
                                .disabled(
                                    !tts_ready
                                        || !has_draft
                                        || chars > 10_000
                                        || tts_busy
                                        || locked,
                                )
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Err(error) = this.start_speaking_draft_audio(cx) {
                                        this.report_status(
                                            FeedbackKind::Error,
                                            format!("生成草稿读音失败：{error:#}"),
                                        );
                                    }
                                    cx.notify();
                                })),
                        )
                    })
                    .when(!tts_ready && draft_audio.is_none(), |row| {
                        row.child(div().child("需先在音频页下载 Kokoro 模型"))
                    }),
            )
    }

    pub(crate) fn speaking_review_history(
        &self,
        question: &Question,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        writing_ui::correction_history(
            ReviewHistory {
                reviews: &question.reviews,
                selected: self.speaking.selected_review.as_deref(),
                show_submission: self.speaking.show_review_source,
                show_notes: self.speaking.show_review_notes,
            },
            Self::speaking_review_action,
            window,
            cx,
        )
    }

    pub(crate) fn speaking_waveform_view(
        &self,
        path: &Path,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let state = &self.speaking.waveform;
        let result = (state.path.as_deref() == Some(path))
            .then_some(state.result.as_ref())
            .flatten();
        let waveform = result.and_then(|result| result.as_ref().ok());
        let playback = &self.media.snapshot;
        let loaded = self.speaking_playback_matches(path, playback);
        let position = if loaded {
            playback.position
        } else if state.path.as_deref() == Some(path) {
            state.start_at
        } else {
            0.
        };
        let duration = if loaded && playback.duration > 0. {
            playback.duration
        } else {
            waveform.map_or(0., |waveform| waveform.duration)
        };
        let cursor = if duration > 0. {
            (position / duration).clamp(0., 1.) as f32
        } else {
            0.
        };
        let bounds_cell = state.bounds.clone();
        let primary = cx.theme().primary;
        let rail_color = cx.theme().border;
        let peaks = waveform
            .map(|waveform| waveform.peaks.clone())
            .unwrap_or_default();
        let path = path.to_owned();
        let can_seek = duration > 0. && self.recording.is_none();
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .id("speaking-waveform")
                    .relative()
                    .h(px(52.))
                    .flex_shrink_0()
                    .rounded_md()
                    .bg(cx.theme().muted)
                    .when(can_seek, |track| track.cursor_pointer())
                    .child(
                        // Anchor to the laid-out track. Relative-sized children can be
                        // measured at their intrinsic width inside the scroll panel.
                        canvas(
                            move |bounds, _, _| bounds_cell.set(bounds),
                            move |bounds, _, window, _| {
                                audio_player_ui::paint_waveform_peaks(
                                    window,
                                    Bounds::new(bounds.origin, size(bounds.size.width, px(38.))),
                                    &peaks,
                                    primary,
                                );
                                let rail = Bounds::new(
                                    point(bounds.left(), bounds.bottom() - px(8.)),
                                    size(bounds.size.width, px(3.)),
                                );
                                window.paint_quad(fill(rail, rail_color));
                                if duration > 0. {
                                    let offset = rail.size.width * cursor;
                                    window.paint_quad(fill(
                                        Bounds::new(rail.origin, size(offset, rail.size.height)),
                                        primary,
                                    ));
                                    let marker = (offset - px(2.))
                                        .clamp(px(0.), (rail.size.width - px(4.)).max(px(0.)));
                                    window.paint_quad(fill(
                                        Bounds::new(
                                            point(rail.left() + marker, rail.top() - px(3.)),
                                            size(px(4.), px(9.)),
                                        ),
                                        primary,
                                    ));
                                }
                            },
                        )
                        .absolute()
                        .inset_0(),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            let bounds = this.speaking.waveform.bounds.get();
                            if can_seek && bounds.size.width > px(0.) {
                                let fraction = ((event.position.x - bounds.left())
                                    / bounds.size.width)
                                    .clamp(0., 1.);
                                let time =
                                    (f64::from(fraction) * duration).min((duration - 0.01).max(0.));
                                this.seek_speaking_prompt(&path, time);
                                cx.notify();
                            }
                        }),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(if duration > 0. {
                        format!("{} / {} · 点击音轨定位", clock(position), clock(duration))
                    } else if result.is_some() {
                        "暂时无法读取波形".into()
                    } else {
                        "正在读取波形…".into()
                    }),
            )
    }

    pub(crate) fn speaking_prompt_card(
        &self,
        locked: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let audio = self.speaking_prompt_audio();
        let transcript = self.inputs["reference"].read(cx).value();
        let has_text = !transcript.trim().is_empty();
        let translation = self.speaking_translation(cx);
        let playing = audio.as_ref().is_some_and(|(path, _)| {
            self.speaking_playback_matches(path, &self.media.snapshot)
                && !self.media.snapshot.paused
                && !self.media.snapshot.eof
        });
        let mut card = div()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().border)
            .child(div().font_weight(FontWeight::SEMIBOLD).child("题目音频"));

        if let Some((path, name)) = &audio {
            card = card
                .child(
                    div()
                        .w_full()
                        .truncate()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(name.clone()),
                )
                .child(self.speaking_waveform_view(path, cx))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child(
                            Button::new("speaking-play-prompt")
                                .small()
                                .primary()
                                .disabled(self.recording.is_some())
                                .label(if playing {
                                    "暂停播放"
                                } else {
                                    "▶ 播放题目"
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if playing {
                                        this.media.player.set("pause", "yes");
                                    } else if let Err(error) = this.play_speaking_prompt() {
                                        this.report_status(FeedbackKind::Error, error.to_string());
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(self.speaking_rate_picker(cx))
                        .child(
                            Button::new("speaking-remove-prompt")
                                .small()
                                .outline()
                                .label("移除音频")
                                .disabled(locked)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.remove_speaking_prompt(window, cx);
                                })),
                        ),
                );
        } else {
            card = card
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("尚未添加音频"),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            Button::new("speaking-pick-prompt")
                                .small()
                                .primary()
                                .disabled(locked)
                                .label("添加音频")
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.pick_speaking_prompt(cx)),
                                ),
                        )
                        .child(
                            Button::new("speaking-command-import-question")
                                .small()
                                .outline()
                                .disabled(locked)
                                .label("用命令导入")
                                .tooltip("使用设置中的本机导入命令，命令需返回一个音频")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.start_speaking_command_import(window, cx);
                                })),
                        ),
                );
        }

        if audio.is_some() || has_text {
            card = card.child(
                div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_3()
                    .rounded_lg()
                    .bg(cx.theme().muted)
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .child(div().flex_1().font_weight(FontWeight::SEMIBOLD).child(
                                if audio.is_some() {
                                    "题目转写"
                                } else {
                                    "已保存的题目文字"
                                },
                            ))
                            .when(audio.is_some(), |bar| {
                                bar.child(
                                    Button::new("speaking-transcribe-prompt")
                                        .small()
                                        .outline()
                                        .label(if has_text {
                                            "重新转写"
                                        } else {
                                            "音频转文字"
                                        })
                                        .disabled(locked)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.start_speaking_text_task(false, cx);
                                        })),
                                )
                            })
                            .when(has_text, |bar| {
                                bar.child(
                                    Button::new("speaking-translate-prompt")
                                        .small()
                                        .outline()
                                        .label(if translation.is_some() {
                                            "重新翻译"
                                        } else {
                                            "翻译成中文"
                                        })
                                        .disabled(locked)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.start_speaking_text_task(true, cx);
                                        })),
                                )
                            }),
                    )
                    .when_else(
                        has_text,
                        |panel| {
                            panel.child(speaking_result_body("speaking-transcript", &transcript))
                        },
                        |panel| {
                            panel.child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("转写后，原文会显示在这里，也可用于跟读评分。"),
                            )
                        },
                    )
                    .when_some(translation, |panel, text| {
                        panel.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .border_t_1()
                                .border_color(cx.theme().border)
                                .pt_2()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("中文译文"),
                                )
                                .child(speaking_result_body("speaking-translation", text)),
                        )
                    }),
            );
        }

        card
    }

    pub(crate) fn speaking_question_actions(
        &self,
        question: &Question,
        locked: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let can_save = self.speaking_prompt_audio().is_some()
            || !self.value("reference", cx).trim().is_empty()
            || self
                .speaking
                .questions
                .iter()
                .any(|saved| saved.id == question.id);

        div()
            .flex()
            .flex_wrap()
            .gap_2()
            .child(
                Button::new("speaking-save-question")
                    .small()
                    .primary()
                    .label(if self.speaking.saving {
                        "正在保存…"
                    } else {
                        "保存题目"
                    })
                    .disabled(locked || !can_save)
                    .on_click(cx.listener(|this, _, _, cx| this.save_speaking_question(cx))),
            )
            .child(
                Button::new("speaking-revert-question")
                    .small()
                    .outline()
                    .label("撤销编辑")
                    .disabled(locked)
                    .on_click(cx.listener(|this, _, window, cx| {
                        let Some(question) = &this.speaking.current else {
                            return;
                        };
                        let (text, title, draft, translation) = (
                            question.text.clone(),
                            question.title.clone(),
                            question.answer_draft.clone(),
                            question.translation.clone(),
                        );
                        this.stop_speaking_prompt();
                        this.set("reference", text.clone(), window, cx);
                        this.set("speaking_title", title, window, cx);
                        this.set("speaking_answer_draft", draft, window, cx);
                        this.speaking.translation =
                            translation.map(|translation| (text, translation));
                        this.speaking.audio_edit = AudioEdit::Keep;
                        this.speaking.editing_title = false;
                        cx.notify();
                    })),
            )
    }
}
