//! Playback clock and native-player events; no page rendering.
use super::*;

impl AppView {
    pub(super) fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focus_changed = self.poll_focus_mode(window, cx);
        let data_changed = self.poll_data();
        self.media.preloader.set_video_target(0, 0, false);
        if self.video_frame_path().is_none() {
            self.media.player.set_video_target(0, 0, false);
        }
        let chat_changed = self.poll_chat(window, cx);
        self.poll_auto_cloud_sync(window, cx);
        if self.poll_close(window, cx) {
            return;
        }
        if self.poll_speaking_waveform() {
            cx.notify();
        }
        #[cfg(debug_assertions)]
        if self.poll_diagnostic(window, cx) {
            return;
        }
        if let Some(rx) = &self.audio.waveform_rx {
            if let Ok(result) = rx.try_recv() {
                match result {
                    Ok(waveform) => self.audio.waveform = Some(waveform),
                    Err(error) => {
                        self.report_status(FeedbackKind::Error, format!("波形读取失败：{error}"))
                    }
                }
                self.audio.waveform_rx = None;
                cx.notify();
            }
        }
        let playback_changed = self.media.poll();
        if self.page == "listening" {
            let active = video_subtitles::active(&self.audio.cues, self.media.snapshot.position);
            if active != self.audio.active {
                self.audio.active = active;
                self.audio.current_scroll.set_offset(Point::default());
                if self.audio.follow && !self.has_subtitle_selection() {
                    if let Some(i) = active {
                        self.audio.center.request(i);
                    }
                }
            }
        }
        if self.page == "video" {
            let active = video_subtitles::active(&self.video.cues, self.media.snapshot.position);
            if active != self.video.active {
                self.video.active = active;
                if self.video.follow && !self.has_subtitle_selection() {
                    if let Some(i) = active {
                        self.video.center.request(i);
                    }
                }
            }
        }
        let mut blind_changed = false;
        if self.audio.blind.enabled {
            if let Some((started, audio, target)) = self.audio.blind_pending.clone() {
                let valid = self.page == "listening"
                    && audio == self.audio.current_id
                    && !self.tasks.foreground.is_running()
                    && self
                        .media
                        .session
                        .path
                        .as_ref()
                        .is_some_and(|p| p.to_string_lossy() == self.media.snapshot.path)
                    && self.audio.cues.iter().any(|c| {
                        c.start == target.cue.start
                            && c.end == target.cue.end
                            && c.text == target.cue.text
                    });
                if !valid
                    || started.elapsed() > Duration::from_secs(2)
                    || !self.media.snapshot.error.is_empty()
                {
                    self.invalidate_blind();
                    self.report_status(FeedbackKind::Error, "未能确认暂停，请重试查看原文".into());
                    blind_changed = true;
                } else if self.media.snapshot.paused
                    && started.elapsed() >= Duration::from_millis(80)
                {
                    if let Some(i) = self.audio.cues.iter().position(|c| {
                        c.start == target.cue.start
                            && c.end == target.cue.end
                            && c.text == target.cue.text
                    }) {
                        self.audio.scroll.scroll_to_reveal_item(i);
                    }
                    self.audio.blind.revealed = Some(target);
                    self.audio.blind_pending = None;
                    self.report_status(FeedbackKind::Info, "已暂停并显示所选字幕".into());
                    blind_changed = true;
                }
            } else if !self.media.snapshot.paused && self.audio.blind.revealed.is_some() {
                self.invalidate_blind();
                blind_changed = true;
            }
        }
        let subtitle_changed = self.poll_background_subtitles();
        let mut changed = self.poll_sentence_search()
            || focus_changed
            || subtitle_changed
            || blind_changed
            || playback_changed
            || data_changed
            || chat_changed;
        if !self.media.snapshot.paused
            && self.media.snapshot.duration > 0.
            && self.page != "meditation"
        {
            self.media.stats_ticks += 0.05;
            if self.media.stats_ticks >= 30. {
                if self.media.session.is_audio(&self.audio.current_id) {
                    let _ = self.lib.set_setting(
                        "last_listening_position",
                        &json!({"id":self.audio.current_id,"position":self.audio_saved_position()})
                            .to_string(),
                    );
                }
                let _ = self.lib.stat("listen_seconds", self.media.stats_ticks);
                self.media.stats_ticks = 0.;
                if self.page == "stats" {
                    self.refresh_view_data();
                }
            }
        }
        changed |= self.poll_tasks(window, cx);
        changed |= self.poll_chatgpt(cx);
        changed |= self.poll_model_download();
        changed |= self.poll_audio_subtitles();
        changed |= self.poll_pending_video_start();
        changed |= self.poll_video_sentence_repeat();
        changed |= self.poll_audio_sentence_repeat();
        changed |= self.poll_audio_review();
        if let (Some(a), Some(b)) = (self.media.session.loop_a, self.media.session.loop_b) {
            if self.media.session.sentence().is_none()
                && self.media.session.mode == PlaybackMode::Ab
                && !self.media.snapshot.paused
                && self.media.snapshot.position >= b
            {
                self.media
                    .player
                    .command(["seek".into(), a.to_string(), "absolute+exact".into()]);
            }
        }
        if self.media.session.owner().is_some()
            && self.media.session.sentence().is_none()
            && (self.page != "video" || self.media.session.mode != PlaybackMode::Normal)
        {
            if self.media.snapshot.duration > 0.
                && self.media.snapshot.position >= self.media.snapshot.duration - 0.1
            {
                if !self.media.session.end_handled {
                    self.media.session.end_handled = true;
                    if self.media.session.mode == PlaybackMode::Track {
                        self.media.player.command(["seek", "0", "absolute"]);
                        self.media.player.set("pause", "no");
                    } else if self.media.session.is_audio(&self.audio.current_id)
                        && matches!(
                            self.media.session.mode,
                            PlaybackMode::Group | PlaybackMode::Continuous
                        )
                    {
                        let group = self
                            .audio
                            .library
                            .merges
                            .learning(&self.audio.audios, &self.audio.practice_id);
                        if let Some(index) = group
                            .iter()
                            .position(|r| text(r, "id") == self.audio.current_id)
                        {
                            let next = if index + 1 < group.len() {
                                Some(group[index + 1].clone())
                            } else if self.media.session.mode == PlaybackMode::Group {
                                group.first().map(|r| (*r).clone())
                            } else {
                                None
                            };
                            if let Some(row) = next {
                                self.open_audio(&row);
                            }
                        }
                    }
                }
            } else {
                self.media.session.end_handled = false;
            }
        }
        if self.page == "video"
            && self.media.session.is_video()
            && self.media.session.mode == PlaybackMode::Normal
            && self.media.session.video_sentence().is_none()
            && self.video.sentence_repeat.is_none()
        {
            if !self.media.snapshot.eof {
                self.media.session.end_handled = false;
            }
            if self.video.auto_next_part
                && self.media.snapshot.error.is_empty()
                && self.media.snapshot.eof
                && !self.media.session.end_handled
                && !self.tasks.foreground.is_running()
                && self
                    .media
                    .session
                    .path
                    .as_ref()
                    .is_some_and(|p| p.to_string_lossy() == self.media.snapshot.path)
            {
                self.media.session.end_handled = true;
                if let Some(next) = self
                    .video
                    .series
                    .as_ref()
                    .and_then(|s| self.media.session.path.as_ref().and_then(|p| s.next_for(p)))
                {
                    self.start_part(next);
                }
            }
        }
        if let Some(error) = self.timer_chimes.take_error() {
            self.report_status(FeedbackKind::Error, format!("计时提示音播放失败：{error}"));
        }
        if self.timer_retry.is_none_or(|retry| Instant::now() >= retry) {
            match self.timer.tick(&self.lib, focus_timer::now_ms()) {
                Ok(Some(focused)) => {
                    self.focus_phase_completed(focused);
                    self.timer_retry = None;
                    changed = true;
                }
                Ok(None) => {}
                Err(error) => {
                    self.timer_retry = Some(Instant::now() + Duration::from_secs(3));
                    self.report_status(FeedbackKind::Error, format!("专注计时保存失败：{error:#}"));
                    changed = true;
                }
            }
        }
        let remaining = self.timer.display_seconds(focus_timer::now_ms());
        if remaining != self.timer_seconds {
            self.timer_seconds = remaining;
            changed = true;
        }
        let previous_context = self.chat.context.clone();
        let previous_error = self.chat.error.clone();
        self.sync_assistant(window, cx);
        changed |= previous_context != self.chat.context || previous_error != self.chat.error;
        if self.tasks.foreground.is_running() {
            if let Some(started) = self.tasks.foreground.started() {
                let elapsed = started.elapsed().as_secs();
                if elapsed != self.tasks.progress_elapsed {
                    self.tasks.progress_elapsed = elapsed;
                    changed = true;
                }
            }
        }
        changed |= self.tick_feedback();
        if changed {
            cx.notify();
        }
    }
}
