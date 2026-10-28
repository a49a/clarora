use super::*;
impl AppView {
    pub(super) fn select_source_only(&mut self, key: &str, play: bool) -> Result<bool> {
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
        videos.selected = Some(key.into());
        videos.save(&self.lib)?;
        series.save(&self.lib)?;
        self.reset_video_sentence_request();
        self.media.player.command(["stop"]);
        self.media.path = None;
        self.video.path = None;
        self.video.set_cues(vec![]);
        self.video.active = None;
        self.video.caption_source = VideoCaptionSource::Hidden;
        self.video.material = None;
        self.video.material_error.clear();
        self.video.series = Some(series);
        self.video.collection = videos;
        self.video.tab = 1;
        self.video.tools_open = false;
        if play {
            self.start_part(index);
        }
        Ok(true)
    }
    pub(super) fn video_collection_panel(&self, cx: &Context<Self>) -> AnyElement {
        let busy = self.tasks.cancel.is_some();
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
                div()
                    .flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        self.button(
                            "video-collection-add",
                            "＋ 导入视频",
                            Action::Pick("video".into()),
                            cx,
                        )
                        .primary()
                        .disabled(busy),
                    )
                    .child(
                        self.button(
                            "video-collection-online",
                            "bilibili 导入",
                            Action::VideoTools,
                            cx,
                        )
                        .disabled(busy),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("每个视频收藏一条 · 分P在选集中切换"),
            );
        for video in &self.video.collection.items {
            let key = video.id.clone();
            let selected = self.video.collection.selected.as_ref() == Some(&key);
            let missing = video.playable_path().is_none();
            let can_download = video.series.is_some();
            let source = if video.series.is_some() {
                "bilibili"
            } else if video.path.starts_with(self.lib.root.join("downloads")) {
                "已下载视频"
            } else {
                "本地文件"
            };
            let path_label = video
                .path
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            let click_key = key.clone();
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
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(video.title.clone()),
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
                                    if missing && can_download {
                                        "下载并播放"
                                    } else if selected {
                                        "从头播放"
                                    } else {
                                        "选择播放"
                                    },
                                    Action::VideoSelect(key.clone()),
                                    cx,
                                )
                                .disabled(busy || (missing && !can_download)),
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
            body=body.child(div().p_4().flex().flex_col().gap_2().child(div().text_lg().child("收藏想学的视频"))
   .child(div().text_sm().text_color(cx.theme().muted_foreground).child("可以一次导入多个本地视频，或从 bilibili 下载。所有导入结果都会保存在这里。")));
        }
        body.into_any_element()
    }
}
