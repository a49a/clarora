//! Audio selection, restoration and waveform coordination.
use super::*;

impl AppView {
    pub(super) fn open_audio(&mut self, row: &Row) {
        self.loads.restore_pending = false;
        self.reset_audio_sentence_request();
        self.audio.pending_repeat_pause = Some(false);
        self.audio.pending_repeat_position = Some(0.);
        let replacement = self
            .audio
            .library
            .merges
            .replacement(&text(row, "id"), &self.audio.audios)
            .cloned();
        let row = replacement.as_ref().unwrap_or(row);
        self.audio.library.cancel_rename();
        self.invalidate_blind();
        self.audio.blind_settle_until = Some(Instant::now() + Duration::from_millis(250));
        let path = PathBuf::from(text(row, "audio_uri"));
        self.audio.current_id = text(row, "id");
        if self.audio.repeat_mode == PlaybackMode::Ab {
            self.audio.repeat_mode = PlaybackMode::Normal;
        }
        self.media.open(
            PlaybackOwner::Audio(self.audio.current_id.clone()),
            path,
            0.,
            0.,
            false,
            self.audio.repeat_mode,
        );
        let _ = self
            .lib
            .set_setting("last_listening_audio", &self.audio.current_id);
        self.load_waveform();
        self.audio.key_points.clear();
        self.audio.practice_id = text(row, "practice_id");
        if let Err(error) = self
            .lib
            .set_setting("library_group", &self.audio.practice_id)
        {
            self.report_status(
                FeedbackKind::Error,
                format!("保存所选练习组失败：{error:#}"),
            );
        }
        self.audio.cues.clear();
        self.invalidate_blind();
        self.audio.scroll.reset(self.audio.cues.len());
        self.audio.active = None;
        self.audio.current_scroll.set_offset(Point::default());
        self.audio.clip_mode = false;
        self.audio.tools_open = false;
        self.audio.review = None;
        if self.audio.repeat_mode == PlaybackMode::Ab {
            self.audio.repeat_mode = PlaybackMode::Normal;
        }
        self.media.session.mode = self.audio.repeat_mode;
        self.media.session.loop_a = None;
        self.media.session.loop_b = None;
        self.refresh_saved_ranges();
        self.refresh();
    }
    pub(super) fn restore_listening(&mut self) {
        self.loads.restore_pending = true;
        self.refresh();
    }
    pub(super) fn restore_listening_cached(&mut self) {
        let Some((last, position)) = self.loads.restore_audio.clone() else {
            return;
        };
        if let Some(row) = self
            .audio
            .audios
            .iter()
            .find(|r| text(r, "id") == last)
            .cloned()
        {
            self.open_audio(&row);
            self.audio.pending_repeat_pause = Some(true);
            self.audio.pending_repeat_position = Some(position);
            self.media.player.set("pause", "yes");
            self.media.player.command([
                "seek".into(),
                position.to_string(),
                "absolute+exact".into(),
            ]);
        }
    }
    pub(super) fn load_waveform(&mut self) {
        if let Some(cancel) = self.audio.waveform_cancel.take() {
            cancel.store(true, Ordering::SeqCst);
        }
        self.audio.waveform = None;
        self.audio.waveform_rx = None;
        if let Some(path) = self.media.session.path.clone() {
            let lib = self.lib.clone();
            let cancel = process::cancel_token();
            self.audio.waveform_cancel = Some(cancel.clone());
            let (tx, rx) = mpsc::channel();
            self.audio.waveform_rx = Some(rx);
            std::thread::spawn(move || {
                let _ =
                    tx.send(waveform::extract(&lib, &path, &cancel).map_err(|e| format!("{e:#}")));
            });
        }
    }
    pub(super) fn selected(&self) -> Option<Row> {
        self.audio
            .audios
            .iter()
            .find(|r| text(r, "id") == self.audio.current_id)
            .cloned()
    }
    pub(super) fn invalidate_blind(&mut self) {
        self.audio.blind.invalidate();
        self.audio.blind_pending = None;
    }
}

impl AppView {
    pub(super) fn start_audio_review(&mut self) -> Result<()> {
        ensure!(!self.audio.blind.enabled, "请先切回字幕模式");
        if self.audio.review.is_some() {
            self.stop_audio_review(true);
            return Ok(());
        }
        ensure!(!self.audio_saved.is_empty(), "本篇还没有收藏的重点句");
        self.audio.review = Some(crate::app_state::AudioClipReview {
            audio_id: self.audio.current_id.clone(),
            ranges: self.audio_saved.clone(),
            index: 0,
            started: Instant::now(),
        });
        self.audio.clip_mode = false;
        self.audio.tools_open = false;
        self.audio.repeat_mode = PlaybackMode::Normal;
        self.media.session.mode = PlaybackMode::Normal;
        self.media.session.loop_a = None;
        self.media.session.loop_b = None;
        self.play_audio_review_clip(0)
    }

    pub(super) fn play_audio_review_clip(&mut self, index: usize) -> Result<()> {
        let (audio_id, range, total) = {
            let review = self.audio.review.as_ref().context("尚未开始重点句复习")?;
            let range = review.ranges.get(index).context("重点句已不存在")?.clone();
            (review.audio_id.clone(), range, review.ranges.len())
        };
        ensure!(audio_id == self.audio.current_id, "音频已切换");
        let row = self.selected().context("音频不存在")?;
        let path = PathBuf::from(text(&row, "audio_uri"));
        self.media.open(
            PlaybackOwner::Audio(audio_id),
            path,
            range.start,
            range.end,
            false,
            PlaybackMode::Normal,
        );
        if let Some(review) = self.audio.review.as_mut() {
            review.index = index;
            review.started = Instant::now();
        }
        self.report_status(
            FeedbackKind::Info,
            format!("重点句 {}/{} · {}", index + 1, total, range.saved.front),
        );
        Ok(())
    }

    pub(super) fn step_audio_review(&mut self, delta: i8) -> Result<()> {
        let Some(review) = self.audio.review.as_ref() else {
            return Ok(());
        };
        let next = (review.index as i64 + i64::from(delta))
            .clamp(0, review.ranges.len().saturating_sub(1) as i64) as usize;
        if next == review.index {
            return Ok(());
        }
        self.play_audio_review_clip(next)
    }

    pub(super) fn stop_audio_review(&mut self, restore: bool) {
        let Some(review) = self.audio.review.take() else {
            return;
        };
        let position = review
            .ranges
            .get(review.index)
            .map(|range| range.start)
            .unwrap_or(0.);
        if restore && let Some(row) = self.selected() {
            self.media.open(
                PlaybackOwner::Audio(review.audio_id),
                PathBuf::from(text(&row, "audio_uri")),
                position,
                0.,
                true,
                PlaybackMode::Normal,
            );
        }
        self.media.player.set("pause", "yes");
    }

    pub(super) fn poll_audio_review(&mut self) -> bool {
        let Some(review) = self.audio.review.as_ref() else {
            return false;
        };
        if self.page != "listening" || review.audio_id != self.audio.current_id {
            let stale = review.audio_id.clone();
            self.stop_audio_review(false);
            if stale != self.audio.current_id {
                self.report_status(FeedbackKind::Info, "音频已切换，已退出重点句复习".into());
            }
            return true;
        }
        let (index, total, range, elapsed) = (
            review.index,
            review.ranges.len(),
            review.ranges[review.index].clone(),
            review.started.elapsed(),
        );
        let Some(row) = self.selected() else {
            self.stop_audio_review(false);
            return true;
        };
        let state = self.media.snapshot.clone();
        if state.path != text(&row, "audio_uri") || elapsed < Duration::from_millis(250) {
            return false;
        }
        if state.position + 0.5 < range.start || state.position > range.end + 0.5 {
            self.stop_audio_review(true);
            self.report_status(FeedbackKind::Info, "已退出重点句复习，恢复整篇播放".into());
            return true;
        }
        if !(state.eof || state.position >= range.end - 0.03) {
            return false;
        }
        if index + 1 < total {
            if let Err(error) = self.play_audio_review_clip(index + 1) {
                self.stop_audio_review(true);
                self.report_status(FeedbackKind::Error, format!("重点句复习中断：{error:#}"));
            }
        } else {
            self.stop_audio_review(true);
            self.report_status(FeedbackKind::Info, "本篇重点句复习完成".into());
        }
        true
    }
}
