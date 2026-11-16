use super::*;
impl AppView {
    pub(super) fn select_source_only(
        &mut self,
        key: &str,
        play: bool,
        from_start: bool,
    ) -> Result<bool> {
        let mut videos = video_library::Collection::load(&self.lib)?;
        let video = videos
            .items
            .iter()
            .find(|v| v.id == key)
            .context("该视频已移出收藏")?
            .clone();
        if video.playable_path().is_some() || video.series.is_none() {
            return Ok(false);
        }
        let series = video.series.unwrap();
        let index = series.current.unwrap_or(series.requested);
        self.save_media_position();
        self.save_video_progress();
        videos.selected = Some(key.into());
        videos.save(&self.lib)?;
        series.save(&self.lib)?;
        self.reset_video_sentence_request();
        self.media.player.command(["stop"]);
        self.media.release();
        self.video.path = None;
        self.video.set_cues(vec![]);
        self.video.active = None;
        self.video.caption_source = VideoCaptionSource::Hidden;
        self.video.material = None;
        self.video.material_error.clear();
        self.video.series = Some(series);
        self.video.collection = videos;
        let key = self
            .video
            .series
            .as_ref()
            .map(video_progress::series_key)
            .unwrap_or_default();
        if self.video.parts_series_key.as_deref() != Some(&key) {
            self.video.parts_series_key = Some(key.clone());
            self.restore_parts_anchor(Some(&key));
        }
        self.video.tab = 1;
        self.video.tools_open = false;
        if play {
            if from_start {
                self.start_part_from_start(index);
            } else {
                self.start_part(index);
            }
        }
        Ok(true)
    }
    pub(super) fn video_collection_panel(&self, cx: &Context<Self>) -> AnyElement {
        let busy = self.tasks.foreground.is_running();
        let filter = self.video.group.clone();
        let groups = self.video.collection.groups.clone();
        let mut group_chips = div().flex().gap_2().flex_wrap().child(self.button(
            "video-group-all",
            format!("全部 · {}", self.video.collection.items.len()),
            Action::VideoGroupFilter(String::new()),
            cx,
        ));
        for group in &groups {
            group_chips = group_chips.child(self.button(
                SharedString::from(format!("video-group-{group}")),
                format!(
                    "{} · {}",
                    group.chars().take(12).collect::<String>(),
                    self.video.collection.group_count(group)
                ),
                Action::VideoGroupFilter(group.clone()),
                cx,
            ));
        }
        group_chips = group_chips
            .child(self.button(
                "video-group-ungrouped",
                format!(
                    "未分组 · {}",
                    self.video.collection.group_count(video_library::UNGROUPED)
                ),
                Action::VideoGroupFilter(video_library::UNGROUPED.into()),
                cx,
            ))
            .child(
                self.button("video-group-new", "＋ 新建分组", Action::VideoGroupNew, cx)
                    .disabled(busy),
            );
        if !filter.is_empty() && filter != video_library::UNGROUPED {
            group_chips = group_chips.child(self.button_menu(
                "video-group-manage",
                "分组管理".into(),
                vec![
                    (
                        "重命名当前分组…".into(),
                        Action::VideoGroupRename,
                        false,
                        busy,
                    ),
                    (
                        "删除当前分组…".into(),
                        Action::VideoGroupDelete,
                        false,
                        busy,
                    ),
                ],
                cx,
            ));
        }
        let mut body = div()
            .id("video-collection")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div().flex().gap_2().flex_wrap().child(
                    self.button(
                        "video-collection-add",
                        "＋ 导入视频",
                        Action::VideoImport,
                        cx,
                    )
                    .primary()
                    .disabled(busy),
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("视频分组"),
                    )
                    .child(group_chips),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("B 站分 P 视频可在「选集」中切换"),
            );
        let mut shown = 0;
        for video in self
            .video
            .collection
            .items
            .iter()
            .filter(|video| video_library::matches_group(video, &filter))
        {
            shown += 1;
            let key = video.id.clone();
            let selected = self.video.collection.selected.as_ref() == Some(&key);
            let missing = video.playable_path().is_none();
            let can_download = video.series.is_some();
            let resume = self.resume_state(video);
            let primary = if missing && can_download {
                ("下载并播放", Action::VideoSelect(key.clone()))
            } else {
                match resume {
                    clarora_gpui::video_progress::Resume::Continue => {
                        ("继续播放", Action::VideoSelect(key.clone()))
                    }
                    clarora_gpui::video_progress::Resume::Replay => {
                        ("重播", Action::VideoSelectFromStart(key.clone()))
                    }
                    clarora_gpui::video_progress::Resume::None if selected => {
                        ("从头播放", Action::VideoSelect(key.clone()))
                    }
                    clarora_gpui::video_progress::Resume::None => {
                        ("选择播放", Action::VideoSelect(key.clone()))
                    }
                }
            };
            let source = if video.series.is_some() {
                "bilibili"
            } else if let Ok(download) = video.path.strip_prefix(self.lib.root.join("downloads")) {
                download
                    .iter()
                    .next()
                    .and_then(|folder| folder.to_str())
                    .and_then(process::VideoSource::from_cache_folder)
                    .map_or("在线视频", process::VideoSource::label)
            } else {
                "本地文件"
            };
            let group_label = if video.group.is_empty() {
                "未分组"
            } else {
                video.group.as_str()
            };
            let path_label = video
                .path
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            let click_key = key.clone();
            let mut video_groups = vec![(
                "未分组".to_string(),
                Action::VideoSetGroup(key.clone(), String::new()),
                video.group.is_empty(),
                busy,
            )];
            for group in &groups {
                video_groups.push((
                    group.clone(),
                    Action::VideoSetGroup(key.clone(), group.clone()),
                    video.group == *group,
                    busy,
                ));
            }
            body = body.child(
                div()
                    .id(SharedString::from(format!("video-favorite-{key}")))
                    .when(!busy && (!missing || can_download), |v| {
                        v.cursor_pointer()
                            .hover(|v| v.bg(cx.theme().accent))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.handle(Action::VideoSelect(click_key.clone()), window, cx)
                            }))
                    })
                    .p_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(if selected {
                        cx.theme().primary
                    } else {
                        cx.theme().border
                    })
                    .bg(if selected {
                        cx.theme().accent
                    } else {
                        cx.theme().background
                    })
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(video.title.clone()),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .px_2()
                                    .py_1()
                                    .rounded_full()
                                    .bg(if video.group.is_empty() {
                                        cx.theme().accent
                                    } else {
                                        cx.theme().primary
                                    })
                                    .text_color(if video.group.is_empty() {
                                        cx.theme().muted_foreground
                                    } else {
                                        cx.theme().background
                                    })
                                    .text_xs()
                                    .child(group_label.chars().take(14).collect::<String>()),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!(
                                "{} · {source}",
                                if missing && can_download {
                                    "仅保存来源链接 · 播放时下载"
                                } else if missing {
                                    "文件已移动或删除"
                                } else if selected {
                                    "当前选中"
                                } else {
                                    "已收藏"
                                }
                            )),
                    )
                    .when_some(video.series.as_ref(), |v, series| {
                        v.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!(
                                    "共 {} P · 已下载 {} P{}",
                                    series.parts.len(),
                                    (0..series.parts.len())
                                        .filter(|i| series.cached(*i).is_some())
                                        .count(),
                                    series
                                        .current
                                        .map(|i| format!(" · 上次 P{}", i + 1))
                                        .unwrap_or_default()
                                )),
                        )
                    })
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .truncate()
                            .child(path_label),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .mt_2()
                            .flex_wrap()
                            .child(
                                self.button(
                                    SharedString::from(format!("video-select-{key}")),
                                    primary.0,
                                    primary.1,
                                    cx,
                                )
                                .disabled(busy || (missing && !can_download)),
                            )
                            .when(
                                resume == clarora_gpui::video_progress::Resume::Continue,
                                |v| {
                                    v.child(
                                        self.button(
                                            SharedString::from(format!("video-from-start-{key}")),
                                            "从头播放",
                                            Action::VideoSelectFromStart(key.clone()),
                                            cx,
                                        )
                                        .disabled(busy),
                                    )
                                },
                            )
                            .when(video.series.is_some(), |v| {
                                v.child(
                                    self.button(
                                        SharedString::from(format!("video-episodes-{key}")),
                                        "查看选集",
                                        Action::VideoEpisodes(key.clone()),
                                        cx,
                                    )
                                    .disabled(busy),
                                )
                            })
                            .child(self.button_menu(
                                SharedString::from(format!("video-group-{key}")),
                                "移动到分组".into(),
                                video_groups,
                                cx,
                            ))
                            .child(
                                self.button(
                                    SharedString::from(format!("video-remove-{key}")),
                                    "移除收藏",
                                    Action::VideoRemove(key),
                                    cx,
                                )
                                .disabled(busy),
                            ),
                    ),
            );
        }
        if self.video.collection.items.is_empty() {
            body = body.child(
                div()
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().text_lg().child("收藏想学的视频"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("点击「导入视频」添加本地文件或粘贴视频链接。"),
                    ),
            );
        } else if shown == 0 {
            body = body
                .child(
                    div()
                        .p_4()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("这个分组还没有视频。可以切换分组，或导入新视频到当前分组。"),
                )
                .child(self.button(
                    "video-group-show-all",
                    "查看全部视频",
                    Action::VideoGroupFilter(String::new()),
                    cx,
                ));
        }
        body.into_any_element()
    }
}
