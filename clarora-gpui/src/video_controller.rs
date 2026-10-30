//! Video playback and sentence-repeat coordination.
use super::*;

#[derive(PartialEq, Eq)]
pub(super) struct PartConfirmation {
    next: bool,
    path: PathBuf,
    base: String,
    current: usize,
    target: usize,
    number: u32,
    title: String,
}

impl AppView {
    pub(super) fn start_part(&mut self, index: usize) {
        self.start_part_from(index, false);
    }
    pub(super) fn start_part_from(&mut self, index: usize, from_end: bool) {
        self.video.part_confirmation = None;
        if self.tasks.cancel.is_some() {
            self.report_status(FeedbackKind::Error, "请先完成或取消当前前台任务".into());
            return;
        }
        let Some(mut series) = self.video.series.clone() else {
            return;
        };
        if index >= series.parts.len() {
            return;
        }
        if let Some(path) = self.tasks.downloads.cached(&series, index) {
            series.parts[index].path = Some(path.clone());
            series.current = Some(index);
            self.video.series = Some(series);
            let panel = self.video.tab;
            if self.open_part_video(path, from_end) {
                self.video.tab = panel;
                if !self.video.repeat_three && !from_end {
                    self.media.player.set("pause", "no");
                }
                self.report_status(
                    FeedbackKind::Info,
                    if from_end {
                        format!("正在定位 P{} 的最后一句", index + 1)
                    } else if self.video.repeat_three {
                        format!("正在准备 P{} 的每句三次播放", index + 1)
                    } else {
                        format!("正在播放 P{}", index + 1)
                    },
                );
            }
            return;
        }
        let origin = self.media.path.clone();
        let downloads = self.tasks.downloads.clone();
        self.begin(
            &format!("正在准备 P{}…", index + 1),
            move |lib, cancel, _| {
                let path = downloads.obtain(&lib, &series, index, &cancel)?;
                // Completed files belong to the shared cache, even if this foreground request is cancelled.
                Ok(Outcome::PartVideo(
                    series, index, path, false, origin, from_end,
                ))
            },
        );
    }
    pub(super) fn open(&mut self, path: PathBuf) -> bool {
        self.open_at(path, 0., false)
    }
    pub(super) fn open_at(&mut self, path: PathBuf, position: f64, paused: bool) -> bool {
        self.open_at_start(path, position, paused, false)
    }
    pub(super) fn open_part_video(&mut self, path: PathBuf, from_end: bool) -> bool {
        self.open_at_start(path, 0., false, from_end)
    }
    fn open_at_start(
        &mut self,
        path: PathBuf,
        position: f64,
        paused: bool,
        from_end: bool,
    ) -> bool {
        self.video.picture_click = None;
        self.video.seek_dragging = false;
        let result = (|| -> Result<_> {
            let mut videos = video_library::Collection::load(&self.lib)?;
            let ids = videos.add(&[path], self.video.series.as_ref())?;
            let video = videos.select(&ids[0])?;
            videos.save(&self.lib)?;
            self.video.collection = videos;
            Ok(video)
        })();
        let video = match result {
            Ok(video) => video,
            Err(e) => {
                self.report_status(FeedbackKind::Error, format!("无法选择视频：{e:#}"));
                return false;
            }
        };
        let path = video.path;
        self.reset_video_sentence_request();
        self.video.series = video.series;
        self.video.remove = None;
        if let Some(cancel) = self.audio.waveform_cancel.take() {
            cancel.store(true, Ordering::SeqCst);
        }
        self.audio.waveform = None;
        self.audio.waveform_rx = None;
        self.audio.key_points.clear();
        if let Some(series) = &mut self.video.series {
            series.current = series
                .parts
                .iter()
                .position(|p| p.path.as_ref() == Some(&path));
            let _ = series.save(&self.lib);
            if let Some(index) = series.current {
                self.video
                    .parts_scroll
                    .scroll_to_item(index, ScrollStrategy::Center);
            }
        }
        if self.media.repeat >= 2 || self.video.repeat_three {
            self.media.repeat = 0;
        }
        self.video.path = Some(path.clone());
        self.media.player.open_range(
            &path,
            position,
            0.,
            paused || self.video.repeat_three || from_end,
        );
        self.media.path = Some(path.clone());
        self.media.loop_a = None;
        self.media.loop_b = None;
        self.audio.cues.clear();
        self.audio.current_id.clear();
        self.load_video_subtitles(&path);
        if self.video.repeat_three || from_end {
            self.video.pending_start = Some(app_state::PendingVideoStart {
                path: path.clone(),
                paused,
                from_end,
                seek_to: None,
                selected_time: (!from_end && position.is_finite() && position >= 0.)
                    .then_some(position),
                within_sentence_seek: None,
            });
            if from_end {
                self.report_status(
                    FeedbackKind::Info,
                    "正在等待上一视频字幕，从最后一句开始播放".into(),
                );
            }
        }
        if self.video.series.is_none() {
            let _ = self.lib.set_setting("bilibili_series", "");
        }
        true
    }
    pub(super) fn prepare_video_material(&mut self, scope: video_qa::Scope) -> Result<()> {
        let path = self.media.path.as_ref().context("请先打开视频")?;
        match video_qa::build(
            path,
            &self.video.cues,
            scope,
            self.media.snapshot.position,
            &self.media.snapshot.subtitle,
        ) {
            Ok(material) => {
                self.video.material = Some(material);
                self.video.material_error.clear();
                Ok(())
            }
            Err(error) => {
                self.video.material = None;
                self.video.material_error = error.to_string();
                Err(error)
            }
        }
    }
    pub(super) fn load_video_subtitles(&mut self, path: &std::path::Path) {
        let selected_time = self
            .video
            .pending_start
            .as_ref()
            .and_then(|p| p.selected_time);
        let within_sentence_seek = self
            .video
            .pending_start
            .as_ref()
            .and_then(|p| p.within_sentence_seek);
        let from_end = self
            .video
            .pending_start
            .as_ref()
            .is_some_and(|p| p.from_end);
        let paused = self
            .video
            .pending_start
            .as_ref()
            .map(|p| p.paused)
            .unwrap_or_else(|| self.media.player.snapshot().paused);
        self.reset_video_sentence_request();
        if (self.video.repeat_three || from_end) && self.media.path.as_deref() == Some(path) {
            self.video.pending_start = Some(app_state::PendingVideoStart {
                path: path.into(),
                paused,
                from_end,
                seek_to: None,
                selected_time,
                within_sentence_seek,
            });
            self.media.player.set("pause", "yes");
            self.report_status(
                FeedbackKind::Info,
                if from_end {
                    "正在等待当前视频字幕，以定位最后一句"
                } else {
                    "已保留每句三遍，等待当前视频字幕；无字幕时可生成或导入"
                }
                .into(),
            );
        }
        self.video.caption_source = VideoCaptionSource::Transcript;
        self.video.material = None;
        self.video.material_error.clear();
        self.video.set_cues(vec![]);
        self.refresh_saved_ranges();
        self.refresh();
        self.video.tab = 0;
    }
    pub(super) fn leave_video_fullscreen(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
        self.video.picture_click = None;
        if !self.video.fullscreen {
            return;
        }
        if self.video.fullscreen_owned && window.is_fullscreen() {
            window.toggle_fullscreen();
        }
        self.video.fullscreen = false;
        self.video.fullscreen_owned = false;
        self.video.fullscreen_seen = false;
        window.focus(&self.focus);
    }
    pub(super) fn clear_video_sentence_repeat(&mut self) {
        self.reset_video_sentence_request();
        self.video.repeat_three = false;
    }
    pub(super) fn reset_video_sentence_request(&mut self) {
        self.video.part_confirmation = None;
        self.video.seek_drag_target = None;
        self.video.pending_start = None;
        if self.video.sentence_repeat.take().is_some() {
            self.media.player.clear_sentence_repeat();
        }
    }
    pub(super) fn video_playback_paused(&self) -> bool {
        self.video
            .pending_start
            .as_ref()
            .map(|p| p.paused)
            .unwrap_or_else(|| self.media.player.snapshot().paused)
    }
    pub(super) fn toggle_pending_video_start(&mut self) -> bool {
        if self.page == "video" {
            if let Some(pending) = &mut self.video.pending_start {
                pending.paused = !pending.paused;
                return true;
            }
        }
        false
    }
    pub(super) fn poll_pending_video_start(&mut self) -> bool {
        let Some(pending) = self.video.pending_start.as_ref() else {
            return false;
        };
        if self.page != "video"
            || self.video.path.as_ref() != Some(&pending.path)
            || self.media.path.as_ref() != Some(&pending.path)
        {
            self.video.pending_start = None;
            return true;
        }
        let state = self.media.player.snapshot();
        if state.path != pending.path.to_string_lossy()
            || !state.duration.is_finite()
            || state.duration <= 0.
            || !state.error.is_empty()
        {
            return false;
        }
        if let Some(position) = pending.seek_to {
            if (state.position - position).abs() <= 0.09 {
                let paused = pending.paused;
                self.video.pending_start = None;
                self.media
                    .player
                    .set("pause", if paused { "yes" } else { "no" });
                return true;
            }
            return false;
        }
        let index = if let Some(time) = pending.selected_time {
            clarora_gpui::video_repeat::at_or_next(
                &self.video.cues,
                time.clamp(0., state.duration),
                state.duration,
            )
        } else if pending.from_end {
            clarora_gpui::video_repeat::adjacent(
                &self.video.cues,
                state.duration,
                false,
                state.duration,
            )
        } else {
            clarora_gpui::video_repeat::first(&self.video.cues, state.duration)
        };
        let Some(index) = index else {
            return false;
        };
        let paused = pending.paused;
        let within_sentence_seek = pending.within_sentence_seek;
        if !self.video.repeat_three {
            let position = self.video.cues[index].start;
            self.video.pending_start.as_mut().unwrap().seek_to = Some(position);
            self.media.player.command([
                "seek".into(),
                position.to_string(),
                "absolute+exact".into(),
            ]);
            self.video.active = Some(index);
            if self.video.follow {
                self.video.center.request(index);
            }
            return true;
        }
        self.video.pending_start = None;
        let started = (|| -> Result<()> {
            self.start_video_sentence_repeat(Some(index), true)?;
            if let Some(position) = within_sentence_seek {
                self.video.sentence_repeat.context("尚未建立重复句")?.seek(
                    &self.media.player,
                    &PathBuf::from(&state.path),
                    &self.video.cues,
                    position,
                    state.duration,
                )?;
            }
            Ok(())
        })();
        match started {
            Ok(()) => {
                if paused {
                    self.media.player.set("pause", "yes");
                }
            }
            Err(error) => {
                self.clear_video_sentence_repeat();
                self.report_status(FeedbackKind::Error, format!("恢复每句三次失败：{error:#}"));
            }
        }
        true
    }
    pub(super) fn seek_video_timeline_three(&mut self, time: f64) -> Result<()> {
        ensure!(time.is_finite(), "播放位置无效");
        let path = self.video.path.clone().context("请先打开视频")?;
        ensure!(self.media.path.as_ref() == Some(&path), "视频正在切换");
        let paused = self.video_playback_paused();
        let state = self.media.player.snapshot();
        let session = self.video.sentence_repeat.filter(|session| session.three);
        let pending = self.video.pending_start.as_ref();
        let from_end = session.is_none() && pending.is_some_and(|pending| pending.from_end);
        let anchor = session
            .map(|session| session.range.0)
            .or_else(|| pending.and_then(|pending| pending.selected_time))
            .or_else(|| {
                (state.path == path.to_string_lossy() && state.position.is_finite())
                    .then_some(state.position.max(0.))
            })
            .unwrap_or(0.);
        let index = (state.path == path.to_string_lossy()
            && state.duration.is_finite()
            && state.duration > 0.
            && state.error.is_empty())
        .then(|| {
            session.map(|session| session.index).or_else(|| {
                if from_end {
                    clarora_gpui::video_repeat::adjacent(
                        &self.video.cues,
                        state.duration,
                        false,
                        state.duration,
                    )
                } else {
                    clarora_gpui::video_repeat::at_or_next(
                        &self.video.cues,
                        anchor.clamp(0., state.duration),
                        state.duration,
                    )
                }
            })
        })
        .flatten();
        if let Some(index) = index {
            if session.is_none()
                || session.is_some_and(|session| {
                    state.sentence_repeat.as_ref().is_some_and(|repeat| {
                        repeat.request_id == session.request_id && repeat.finished
                    })
                })
            {
                self.start_video_sentence_repeat(Some(index), true)?;
            }
            let session = self.video.sentence_repeat.context("尚未建立重复句")?;
            let target =
                clarora_gpui::media::sentence_seek_target(session.range.0, session.range.1, time)?;
            if self.video.seek_dragging
                && self
                    .video
                    .seek_drag_target
                    .is_some_and(|(request, previous)| {
                        request == session.request_id && (previous - target).abs() < 0.000_001
                    })
            {
                return Ok(());
            }
            session.seek(
                &self.media.player,
                &path,
                &self.video.cues,
                target,
                state.duration,
            )?;
            self.video.seek_drag_target = Some((session.request_id, target));
            if paused {
                self.media.player.set("pause", "yes");
            }
        } else {
            self.reset_video_sentence_request();
            self.video.pending_start = Some(app_state::PendingVideoStart {
                path,
                paused,
                from_end,
                seek_to: None,
                selected_time: (!from_end).then_some(anchor),
                within_sentence_seek: Some(time),
            });
            self.media.player.set("pause", "yes");
            self.report_status(
                FeedbackKind::Info,
                "已保留当前句和音轨选点，等待字幕后在本句内定位".into(),
            );
        }
        Ok(())
    }
    pub(super) fn follow_video_repeat(&mut self) {
        if let Some(session) = self.video.sentence_repeat {
            self.video.active = Some(session.index);
            if self.video.follow {
                self.video.center.request(session.index);
            }
            self.report_status(
                FeedbackKind::Info,
                "每句三遍，播完自动下一句；← / → 切句后从第一遍开始".into(),
            );
        }
    }
    pub(super) fn poll_video_sentence_repeat(&mut self) -> bool {
        if self.page != "video" || self.video.part_confirmation.is_some() {
            return false;
        }
        let Some(mut session) = self.video.sentence_repeat else {
            return false;
        };
        let Some(path) = self.video.path.as_ref() else {
            return false;
        };
        let snapshot = self.media.player.snapshot();
        match session.poll(&self.media.player, path, &self.video.cues, &snapshot) {
            Ok(clarora_gpui::video_repeat::Transition::Unchanged) => false,
            Ok(transition) => {
                self.video.sentence_repeat = Some(session);
                if transition == clarora_gpui::video_repeat::Transition::Advanced {
                    self.follow_video_repeat();
                } else {
                    self.report_status(
                        FeedbackKind::Info,
                        "最后一句已播放三遍，已暂停；← 切上一句，→ 确认下一 P".into(),
                    );
                }
                true
            }
            Err(error) => {
                self.clear_video_sentence_repeat();
                self.report_status(FeedbackKind::Error, error.to_string());
                true
            }
        }
    }
    pub(super) fn clear_finished_video_sentence_repeat(&mut self) {
        self.poll_video_sentence_repeat();
        if self.video.sentence_repeat.is_some_and(|session| {
            self.media
                .player
                .snapshot()
                .sentence_repeat
                .as_ref()
                .is_some_and(|state| state.request_id == session.request_id && state.finished)
        }) {
            self.clear_video_sentence_repeat();
        }
    }
    pub(super) fn video_horizontal_action(&self, next: bool) -> Action {
        if self.page == "video" {
            Action::VideoSentence(next)
        } else {
            Action::Seek(if next { 10. } else { -10. })
        }
    }
    /// Use raw key events in normal and fullscreen views so OS key repeat cannot
    /// turn one held arrow into two confirmations. Inputs keep their own arrows.
    pub(super) fn handle_video_arrow_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let modifiers = event.keystroke.modifiers;
        if self.page != "video"
            || modifiers.control
            || modifiers.platform
            || modifiers.alt
            || modifiers.shift
            || !window.focused(cx).is_some_and(|focus| {
                focus == self.focus || focus == self.subtitle_text.borrow().focus
            })
        {
            return false;
        }
        let action = match event.keystroke.key.as_str() {
            "right" => Action::VideoArrow(true, event.is_held),
            "left" => Action::VideoArrow(false, event.is_held),
            "escape" if self.video.part_confirmation.is_some() => Action::VideoCancelPart,
            _ => return false,
        };
        self.handle(action, window, cx);
        true
    }
    pub(super) fn video_arrow(
        &mut self,
        next: bool,
        held: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.video.pending_start.is_some() {
            self.report_status(
                FeedbackKind::Info,
                "正在准备当前视频的字幕播放位置，请稍候".into(),
            );
            return;
        }
        if self
            .video
            .part_confirmation
            .as_ref()
            .is_some_and(|p| p.next != next)
        {
            self.video.part_confirmation = None;
            self.handle(Action::VideoSentence(next), window, cx);
            return;
        }
        let state = self.media.player.snapshot();
        let first = clarora_gpui::video_repeat::first(&self.video.cues, state.duration);
        let last = clarora_gpui::video_repeat::adjacent(
            &self.video.cues,
            state.duration,
            false,
            state.duration,
        );
        let current = self
            .video
            .sentence_repeat
            .filter(|s| s.three)
            .map(|s| s.index)
            .or_else(|| video_subtitles::active(&self.video.cues, state.position))
            .or_else(|| {
                last.filter(|&i| state.position >= self.video.cues[i].end.min(state.duration))
            })
            .or_else(|| first.filter(|&i| state.position <= self.video.cues[i].start));
        let boundary = if next { last } else { first };
        if boundary.is_none() || current != boundary {
            self.video.part_confirmation = None;
            self.handle(Action::VideoSentence(next), window, cx);
            return;
        }
        if held {
            return;
        }
        let target = (|| {
            let path = self.video.path.as_ref()?;
            if self.page != "video"
                || self.media.path.as_ref() != Some(path)
                || state.path != path.to_string_lossy()
            {
                return None;
            }
            let series = self.video.series.as_ref()?;
            let current = series.current?;
            if series.parts.get(current)?.path.as_ref() != Some(path) {
                return None;
            }
            let target = if next {
                series.next_for(path)?
            } else {
                current.checked_sub(1)?
            };
            let part = series.parts.get(target)?;
            Some(PartConfirmation {
                next,
                path: path.clone(),
                base: series.base.clone(),
                current: series.current?,
                target,
                number: part.number,
                title: part.title.clone(),
            })
        })();
        let Some(target) = target else {
            self.video.part_confirmation = None;
            self.report_status(
                FeedbackKind::Info,
                if next {
                    "已经是最后一句，当前没有下一 P"
                } else {
                    "已经是第一句，当前没有上一 P"
                }
                .into(),
            );
            return;
        };
        if self.tasks.cancel.is_some() {
            self.video.part_confirmation = None;
            self.report_status(
                FeedbackKind::Info,
                "请先完成或取消当前任务，再切换视频".into(),
            );
            return;
        }
        if self.video.part_confirmation.as_ref() == Some(&target) {
            self.video.part_confirmation = None;
            self.handle(
                if next {
                    Action::SelectPart(target.target)
                } else {
                    Action::SelectPartAtEnd(target.target)
                },
                window,
                cx,
            );
        } else {
            self.media.player.set("pause", "yes");
            self.video.part_confirmation = Some(target);
        }
    }
    pub(super) fn dismiss_video_part_confirmation(&mut self) {
        let Some(prompt) = self.video.part_confirmation.take() else {
            return;
        };
        // A completed first cue must not auto-advance after cancelling its prompt.
        if !prompt.next {
            if let Some(session) = self.video.sentence_repeat.filter(|s| s.three) {
                if self
                    .media
                    .player
                    .snapshot()
                    .sentence_repeat
                    .as_ref()
                    .is_some_and(|s| s.request_id == session.request_id && s.finished)
                {
                    if let Err(error) = self.start_video_sentence_repeat(Some(session.index), true)
                    {
                        self.report_status(FeedbackKind::Error, error.to_string());
                    }
                    self.media.player.set("pause", "yes");
                }
            }
        }
    }
    pub(super) fn video_part_confirmation_view(
        &self,
        window: &Window,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let target = self
            .video
            .part_confirmation
            .as_ref()
            .filter(|_| self.page == "video")?;
        let width = (f32::from(window.viewport_size().width) - 32.).clamp(160., 460.);
        let (boundary, direction, arrow, button) = if target.next {
            ("最后一句", "下一", "→", "切换下一 P →")
        } else {
            ("第一句", "上一", "←", "← 切换上一 P")
        };
        Some(
            div()
                .id("video-part-confirmation")
                .absolute()
                .bottom_8()
                .left(px((f32::from(window.viewport_size().width) - width) / 2.))
                .w(px(width))
                .p_5()
                .rounded_xl()
                .shadow_lg()
                .bg(cx.theme().background)
                .border_2()
                .border_color(cx.theme().primary)
                .flex()
                .flex_col()
                .gap_3()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format!("已经是{boundary}")),
                )
                .child(div().text_sm().child(format!(
                    "{direction}视频：P{} · {}",
                    target.number, target.title
                )))
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().primary)
                        .child(if target.next {
                            format!("再次按 {arrow} 从下一视频首句播放；按 Esc 取消")
                        } else {
                            format!("再次按 {arrow} 从上一视频末句播放；按 Esc 取消")
                        }),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            self.button(
                                "video-part-confirm",
                                button,
                                Action::VideoArrow(target.next, false),
                                cx,
                            )
                            .primary(),
                        )
                        .child(
                            self.button(
                                "video-part-cancel",
                                "取消 · Esc",
                                Action::VideoCancelPart,
                                cx,
                            )
                            .outline(),
                        ),
                )
                .into_any_element(),
        )
    }
    pub(super) fn can_move_video_sentence(&self, next: bool) -> bool {
        if let Some(session) = self.video.sentence_repeat.filter(|s| s.three) {
            session
                .neighbor(&self.video.cues, next, self.media.snapshot.duration)
                .is_some()
        } else {
            clarora_gpui::video_repeat::adjacent(
                &self.video.cues,
                self.media.snapshot.position,
                next,
                self.media.snapshot.duration,
            )
            .is_some()
        }
    }
    pub(super) fn video_repeat_cue(&self) -> Option<usize> {
        self.video
            .sentence_repeat
            .map(|session| session.index)
            .or_else(|| video_subtitles::active(&self.video.cues, self.media.snapshot.position))
            .filter(|index| self.video.cues.get(*index).is_some())
    }
    pub(super) fn start_video_sentence_repeat(
        &mut self,
        index: Option<usize>,
        three: bool,
    ) -> Result<()> {
        ensure!(self.page == "video", "请先进入视频学习");
        let path = self.video.path.clone().context("请先打开视频")?;
        let playback = self.media.player.snapshot();
        ensure!(
            self.media.path.as_ref() == Some(&path),
            "视频正在切换，请稍候再重复本句"
        );
        if three && index.is_none() {
            // The mode menu is independent of a selected/active subtitle. Bind
            // its intent to this file and let the existing readiness path pick
            // the current cue, the next cue in a gap, or the last valid cue.
            let position =
                if playback.path == path.to_string_lossy() && playback.position.is_finite() {
                    playback.position.max(0.)
                } else {
                    0.
                };
            let pending = self.video.pending_start.as_ref().filter(|p| p.path == path);
            let from_end = pending.is_some_and(|p| p.from_end);
            let selected_time = pending
                .and_then(|p| p.selected_time)
                .or_else(|| (!from_end).then_some(position));
            self.reset_video_sentence_request();
            self.video.repeat_three = true;
            self.video.pending_start = Some(app_state::PendingVideoStart {
                path,
                paused: false,
                from_end,
                seek_to: None,
                selected_time,
                within_sentence_seek: None,
            });
            self.media.repeat = 0;
            self.media.loop_a = None;
            self.media.loop_b = None;
            self.media.end_handled = true;
            self.media.player.set("pause", "yes");
            self.poll_pending_video_start();
            if self.video.pending_start.is_some() {
                self.report_status(
                    FeedbackKind::Info,
                    "已开启每句三次，等待当前视频和字幕就绪；可生成或导入字幕".into(),
                );
            }
            return Ok(());
        }
        ensure!(
            playback.path == path.to_string_lossy()
                && playback.duration.is_finite()
                && playback.duration > 0.,
            "视频正在切换，请稍候再重复本句"
        );
        let index = index
            .or_else(|| self.video_repeat_cue())
            .context("当前没有带时间轴的字幕，请先选择一句字幕")?;
        self.video.sentence_repeat = Some(clarora_gpui::video_repeat::Session::start(
            &self.media.player,
            &path,
            &self.video.cues,
            index,
            three,
            playback.duration,
        )?);
        self.video.repeat_three = three;
        self.video.pending_start = None;
        self.media.repeat = 0;
        self.media.loop_a = None;
        self.media.loop_b = None;
        self.media.end_handled = true;
        if three {
            self.follow_video_repeat();
        } else {
            self.report_status(
                FeedbackKind::Info,
                "已开启单句循环；点击「单句循环」菜单可退出".into(),
            );
        }
        Ok(())
    }
}
