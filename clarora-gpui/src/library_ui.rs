//! Native audio-library workspace. Selection does not touch player state.
use super::*;
impl AppView {
    pub(super) fn audio_library_view(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let width = (f32::from(window.viewport_size().width)
            - if self.navigation_open { 224. } else { 0. }
            - if self.assistant_open { 400. } else { 0. }
            - 64.)
            .max(260.);
        let tall = f32::from(window.viewport_size().height) >= 680.;
        let group_rail = width >= 1000. && tall;
        let split = width >= 720. && tall;
        let height = (f32::from(window.viewport_size().height) - 330.).max(320.);
        let group_name = self
            .practices
            .iter()
            .find(|r| text(r, "id") == self.practice_id)
            .map(|r| text(r, "name"))
            .unwrap_or_else(|| "还没有练习组".into());
        let busy = self.busy.is_some();
        let group_total = self
            .audios
            .iter()
            .filter(|r| text(r, "practice_id") == self.practice_id)
            .count();
        let attached = self
            .audios
            .iter()
            .filter(|r| {
                text(r, "practice_id") == self.practice_id
                    && self.library.subtitle_ids.contains(&text(r, "id"))
            })
            .count();
        let mut root = div().flex().flex_col().gap_4().w_full().min_w_0();
        let group_query = self.value("library_groups", cx).trim().to_lowercase();
        let group_options = self
            .practices
            .iter()
            .filter(|r| text(r, "name").to_lowercase().contains(&group_query))
            .map(|r| {
                (
                    format!(
                        "{} · {} 条",
                        text(r, "name"),
                        self.audios
                            .iter()
                            .filter(|a| text(a, "practice_id") == text(r, "id"))
                            .count()
                    ),
                    Action::Practice(text(r, "id")),
                    text(r, "id") == self.practice_id,
                    false,
                )
            })
            .collect();
        let mut toolbar = div().flex().items_center().gap_2().flex_wrap();
        if !group_rail {
            toolbar = toolbar
                .child(
                    div()
                        .w(px(160.))
                        .child(Input::new(&self.inputs["library_groups"])),
                )
                .child(self.player_menu(
                    "library-group-menu",
                    format!(
                        "练习组：{}",
                        group_name.chars().take(22).collect::<String>()
                    ),
                    group_options,
                    cx,
                ));
        }
        toolbar = toolbar
            .child(
                self.button(
                    "library-new-group",
                    "＋ 新建组",
                    Action::LibraryTool("group-new".into()),
                    cx,
                )
                .disabled(busy),
            )
            .child(
                self.button(
                    "library-rename-group",
                    "组设置",
                    Action::LibraryTool("group-rename".into()),
                    cx,
                )
                .disabled(busy || self.practice_id.is_empty()),
            )
            .child(
                self.button(
                    "library-import",
                    "＋ 导入音频",
                    Action::Pick("audio".into()),
                    cx,
                )
                .primary()
                .disabled(busy || self.practice_id.is_empty()),
            )
            .child(
                self.button(
                    "library-tts",
                    "文字转音频",
                    Action::LibraryTool("tts".into()),
                    cx,
                )
                .selected(self.library.tool == "tts")
                .disabled(self.practice_id.is_empty()),
            )
            .child(self.button(
                "library-advanced",
                "更多导入",
                Action::LibraryTool("advanced".into()),
                cx,
            ));
        root = root.child(toolbar);
        let mut board = div()
            .flex()
            .gap_4()
            .min_w_0()
            .when(!split, |v| v.flex_col());
        if group_rail {
            let group_query = self.value("library_groups", cx).trim().to_lowercase();
            let mut groups = div()
                .id("library-group-list")
                .overflow_y_scroll()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .gap_1();
            for row in self
                .practices
                .iter()
                .filter(|r| text(r, "name").to_lowercase().contains(&group_query))
            {
                let key = text(row, "id");
                let count = self
                    .audios
                    .iter()
                    .filter(|a| text(a, "practice_id") == key)
                    .count();
                groups = groups.child(
                    div()
                        .id(SharedString::from(format!("library-group-{key}")))
                        .px_3()
                        .py_3()
                        .rounded_lg()
                        .bg(if key == self.practice_id {
                            cx.theme().accent
                        } else {
                            cx.theme().background
                        })
                        .cursor_pointer()
                        .hover(|v| v.bg(cx.theme().accent))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.handle(Action::Practice(key.clone()), window, cx)
                        }))
                        .child(div().text_sm().truncate().child(text(row, "name")))
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("{count} 条音频")),
                        ),
                );
            }
            if !self
                .practices
                .iter()
                .any(|r| text(r, "name").to_lowercase().contains(&group_query))
            {
                groups = groups.child(div().p_3().text_sm().child("没有匹配的练习组"));
            }
            board = board.child(
                panel(cx)
                    .w(px(180.))
                    .flex_shrink_0()
                    .p_3()
                    .h(px(height))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("练习组 · {}", self.practices.len())),
                    )
                    .child(Input::new(&self.inputs["library_groups"]))
                    .child(groups),
            );
        }
        let query = self.value("library_search", cx);
        let visible = self
            .library
            .visible(&self.audios, &self.practice_id, &query);
        let mut list = panel(cx)
            .flex_1()
            .min_w_0()
            .p_0()
            .overflow_hidden()
            .when(split, |v| v.h(px(height)));
        let mut list_head = div()
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(group_name.clone()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "{group_total} 条音频 · {attached} 条附字幕 · {} 条待配字幕",
                        group_total - attached
                    )),
            )
            .child(Input::new(&self.inputs["library_search"]));
        let mut filters = div().flex().gap_1().items_center().flex_wrap();
        for (i, label) in [(0, "全部"), (1, "附字幕"), (2, "待配字幕")] {
            filters = filters.child(
                self.button(
                    SharedString::from(format!("library-filter-{i}")),
                    label,
                    Action::LibraryFilter(i),
                    cx,
                )
                .selected(self.library.filter == i),
            );
        }
        filters = filters
            .child(
                self.player_menu(
                    "library-sort",
                    ["最近添加", "名称 A–Z", "最早添加"][self.library.sort.min(2) as usize].into(),
                    [(0, "最近添加"), (1, "名称 A–Z"), (2, "最早添加")]
                        .into_iter()
                        .map(|(i, s)| {
                            (
                                s.into(),
                                Action::LibrarySort(i),
                                self.library.sort == i,
                                false,
                            )
                        })
                        .collect(),
                    cx,
                ),
            )
            .child(
                self.button(
                    "library-batch",
                    if self.library.batch {
                        "退出多选"
                    } else {
                        "多选"
                    },
                    Action::LibraryBatch,
                    cx,
                )
                .selected(self.library.batch)
                .disabled(busy || group_total == 0),
            );
        if !query.trim().is_empty() || self.library.filter != 0 {
            filters =
                filters.child(self.button("library-reset", "重置筛选", Action::LibraryReset, cx));
        }
        list_head = list_head.child(filters);
        if self.library.batch {
            let ids =
                audio_library::scoped_ids(&self.audios, &self.practice_id, &self.checked_audio);
            list_head = list_head.child(
                div()
                    .flex()
                    .gap_2()
                    .flex_wrap()
                    .items_center()
                    .child(div().text_xs().child(format!("已选 {} 条", ids.len())))
                    .child(
                        self.button(
                            "library-select-all",
                            "全选 / 取消当前结果",
                            Action::LibrarySelectAll,
                            cx,
                        )
                        .disabled(busy || visible.is_empty()),
                    )
                    .child(
                        self.button(
                            "library-move",
                            "移动到…",
                            Action::LibraryTool("move".into()),
                            cx,
                        )
                        .disabled(busy || ids.is_empty()),
                    )
                    .child(
                        self.button("library-merge", "合并", Action::MergeAudios, cx)
                            .disabled(busy || ids.len() < 2),
                    )
                    .child(
                        self.button(
                            "library-delete-batch",
                            "移除",
                            Action::LibraryDelete(ids.clone()),
                            cx,
                        )
                        .disabled(busy || ids.is_empty()),
                    ),
            );
        }
        list = list.child(list_head);
        let mut rows = div()
            .id("library-audio-list")
            .flex()
            .flex_col()
            .min_h_0()
            .when(split, |v| v.flex_1().overflow_y_scroll());
        for (index, row) in visible.iter().enumerate() {
            let key = text(row, "id");
            let selected = key == self.library.selected;
            let subtitle = self.library.subtitle_ids.contains(&key);
            let missing = !PathBuf::from(text(row, "audio_uri")).is_file();
            let click_key = key.clone();
            let mut item = div()
                .id(SharedString::from(format!("library-row-{key}")))
                .flex()
                .items_center()
                .gap_3()
                .px_4()
                .py_3()
                .min_w_0()
                .border_b_1()
                .border_color(cx.theme().border)
                .bg(if selected {
                    cx.theme().accent
                } else {
                    cx.theme().background
                })
                .cursor_pointer()
                .hover(|v| v.bg(cx.theme().accent))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.handle(Action::LibrarySelect(click_key.clone()), window, cx)
                }));
            if self.library.batch {
                item = item.child(
                    self.button(
                        SharedString::from(format!("library-check-{key}")),
                        if self.checked_audio.contains(&key) {
                            "☑"
                        } else {
                            "□"
                        },
                        Action::CheckAudio(key.clone()),
                        cx,
                    )
                    .disabled(busy),
                );
            } else {
                item = item.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("{:02}", index + 1)),
                );
            }
            rows = rows.child(
                item.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .truncate()
                                .child(text(row, "name")),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(if missing {
                                    cx.theme().danger
                                } else {
                                    cx.theme().muted_foreground
                                })
                                .child(if missing {
                                    "音频文件缺失"
                                } else if subtitle {
                                    "附字幕"
                                } else {
                                    "待配字幕"
                                }),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(if selected { "已选" } else { "›" }),
                ),
            );
        }
        if visible.is_empty() {
            rows = rows.child(
                div()
                    .p_6()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(div().text_lg().child(if self.practice_id.is_empty() {
                        "先创建一个练习组"
                    } else if group_total == 0 {
                        "把想学的内容放进来"
                    } else {
                        "没有匹配的音频"
                    }))
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(if group_total == 0 {
                                "导入本地音频，或把文字生成带字幕的语音。"
                            } else {
                                "调整搜索词或字幕筛选条件。"
                            }),
                    ),
            );
        }
        list = list.child(rows);
        let detail = self.library_detail(cx).when(split, |v| {
            v.w(px(if group_rail { 290. } else { 310. }))
                .flex_shrink_0()
                .h(px(height))
                .overflow_y_scroll()
        });
        // Explicit editors stay visible above the list in a small window.
        if !split && !self.library.tool.is_empty() {
            board = board.child(detail).child(list);
        } else {
            board = board.child(list).child(detail);
        }
        root.child(board).into_any_element()
    }
    fn library_detail(&self, cx: &Context<Self>) -> Stateful<Div> {
        let busy = self.busy.is_some();
        let mut detail = div()
            .id("library-detail")
            .flex()
            .flex_col()
            .gap_3()
            .min_w_0();
        let close = self.button(
            "library-tool-close",
            "返回音频列表",
            Action::LibraryTool(String::new()),
            cx,
        );
        match self.library.tool.as_str() {
            "tts" => return detail.child(close).child(self.tts_panel(cx)),
            "group-new" | "group-rename" | "audio-rename" => {
                let title = match self.library.tool.as_str() {
                    "group-new" => "新建练习组",
                    "group-rename" => "重命名练习组",
                    _ => "重命名音频",
                };
                return detail.child(
                    panel(cx)
                        .child(div().text_lg().child(title))
                        .child(self.field("library_name", "名称"))
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .flex_wrap()
                                .child(
                                    self.button(
                                        "library-save-name",
                                        "保存名称",
                                        Action::LibrarySaveName,
                                        cx,
                                    )
                                    .primary()
                                    .disabled(busy),
                                )
                                .child(close),
                        ),
                );
            }
            "move" => {
                let count =
                    audio_library::scoped_ids(&self.audios, &self.practice_id, &self.checked_audio)
                        .len();
                let mut targets = panel(cx)
                    .child(div().text_lg().child(format!("将 {count} 条音频移动到")))
                    .child(close);
                for row in self
                    .practices
                    .iter()
                    .filter(|r| text(r, "id") != self.practice_id)
                {
                    targets = targets.child(
                        self.button(
                            SharedString::from(format!("library-destination-{}", text(row, "id"))),
                            text(row, "name"),
                            Action::LibraryMove(text(row, "id")),
                            cx,
                        )
                        .disabled(busy || count == 0),
                    );
                }
                if self.practices.len() < 2 {
                    targets = targets.child("先新建另一个练习组，再移动音频。");
                }
                return detail.child(targets);
            }
            "delete" => {
                let mut confirm = panel(cx)
                    .child(div().text_lg().child(format!(
                        "移除 {} 条音频记录？",
                        self.library.pending_delete.len()
                    )))
                    .child(
                        div()
                            .text_sm()
                            .child("记录会从音频库移除，本机媒体和字幕附件保留。"),
                    );
                for key in &self.library.pending_delete {
                    if let Some(row) = self.audios.iter().find(|r| text(r, "id") == *key) {
                        confirm = confirm.child(div().text_sm().child(text(row, "name")));
                    }
                }
                return detail.child(
                    confirm
                        .child(
                            self.button(
                                "library-confirm-delete",
                                "确认移除",
                                Action::LibraryConfirmDelete,
                                cx,
                            )
                            .disabled(busy || self.library.pending_delete.is_empty()),
                        )
                        .child(close),
                );
            }
            "advanced" => {
                return detail.child(
                    panel(cx)
                        .child(div().text_lg().child("更多导入方式"))
                        .child(
                            div().text_sm().child(
                                "按设置中的本机导入命令执行，将返回的音频路径导入当前练习组。",
                            ),
                        )
                        .child(
                            self.button(
                                "library-command",
                                "运行本机导入命令",
                                Action::CommandImport,
                                cx,
                            )
                            .disabled(busy || self.practice_id.is_empty()),
                        )
                        .child(self.button(
                            "library-settings",
                            "打开设置",
                            Action::Page("settings".into()),
                            cx,
                        ))
                        .child(close),
                );
            }
            _ => {}
        }
        if self.library.tool == "detail" {
            detail = detail.child(close);
        }
        let Some(row) = self.audios.iter().find(|r| {
            text(r, "id") == self.library.selected && text(r, "practice_id") == self.practice_id
        }) else {
            return detail.child(
                panel(cx)
                    .child(div().text_lg().child("选择一条音频"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("单击列表查看详情，再开始学习。"),
                    ),
            );
        };
        let key = text(row, "id");
        let missing = !PathBuf::from(text(row, "audio_uri")).is_file();
        let mut card = panel(cx)
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("所选音频"),
            )
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(text(row, "name")),
            )
            .child(
                self.button(
                    "library-start",
                    "▶ 开始学习",
                    Action::LibraryStudy(false),
                    cx,
                )
                .primary()
                .disabled(busy || missing),
            )
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(
                if self.library.subtitle_count > 0 {
                    format!("字幕 · {} 句", self.library.subtitle_count)
                } else {
                    "字幕尚未就绪".into()
                },
            ))
            .child(
                self.button(
                    "library-captions",
                    if self.library.subtitle_count > 0 {
                        "查看 / 管理字幕"
                    } else {
                        "生成 / 导入字幕"
                    },
                    Action::LibraryStudy(true),
                    cx,
                )
                .disabled(busy || missing),
            );
        if self.library.subtitle_count == 0 {
            card = card.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("进入学习页的“字幕”菜单，可生成原文、导入字幕或添加中文翻译。"),
            );
        } else {
            for cue in &self.library.preview {
                card = card.child(
                    div()
                        .text_sm()
                        .py_2()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(cue.text.clone()),
                );
            }
            if self.library.subtitle_count > 5 {
                card = card.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("显示前 5 句，完整字幕见学习页。"),
                );
            }
        }
        card = card
            .child(
                div()
                    .flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        self.button(
                            "library-rename-audio",
                            "重命名",
                            Action::LibraryTool("audio-rename".into()),
                            cx,
                        )
                        .disabled(busy),
                    )
                    .child(
                        self.button(
                            "library-remove-audio",
                            "移除记录",
                            Action::LibraryDelete(vec![key]),
                            cx,
                        )
                        .disabled(busy),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("文件位置"),
            )
            .child(
                div()
                    .text_xs()
                    .overflow_hidden()
                    .child(text(row, "audio_uri")),
            );
        detail = detail.child(card);
        detail
    }
}
