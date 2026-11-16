//! App integration for local-only per-video/per-part progress.
use super::*;
use crate::app_state::PART_ROW_HEIGHT;
use clarora_gpui::{
    playback_session::Mode as PlaybackMode,
    video_progress::{self, Item, PartsAnchor, Reading, Resume},
};

impl AppView {
    fn parts_anchor_snapshot(&self) -> Option<(String, PartsAnchor)> {
        let series = self.video.series.as_ref()?;
        if series.parts.is_empty() {
            return None;
        }
        let state = self.video.parts_scroll.0.borrow();
        // `None` means this handle has not laid out a part list yet. Do not
        // overwrite a restored anchor with a synthetic zero before then.
        state.last_item_size?;
        let offset = f32::from(state.base_handle.offset().y);
        if !offset.is_finite() {
            return None;
        }
        let top = (-offset / PART_ROW_HEIGHT).max(0.);
        let index = (top.floor() as usize).min(series.parts.len() - 1);
        Some((
            video_progress::series_key(series),
            PartsAnchor {
                index,
                offset: (top - index as f32).clamp(0., 0.999_9),
            },
        ))
    }

    pub(super) fn save_parts_anchor(&mut self) -> bool {
        let Some((key, anchor)) = self.parts_anchor_snapshot() else {
            return false;
        };
        self.video_progress.record_parts_anchor(key, anchor).is_ok()
    }

    pub(super) fn reset_parts_scroll(&self) {
        let mut state = self.video.parts_scroll.0.borrow_mut();
        state.deferred_scroll_to_item = None;
        state.last_item_size = None;
        state.base_handle.set_offset(point(px(0.), px(0.)));
    }

    /// Install the saved top row for the current series, or fall back to the
    /// playing P. The scroll handle is shared by all series, so this must run
    /// whenever the series identity changes.
    pub(super) fn restore_parts_anchor(&self, key: Option<&str>) {
        let Some(series) = &self.video.series else {
            self.reset_parts_scroll();
            return;
        };
        if series.parts.is_empty() {
            self.reset_parts_scroll();
            return;
        }
        let fallback = series
            .current
            .unwrap_or(series.requested)
            .min(series.parts.len() - 1);
        let anchor = key
            .and_then(|key| self.video_progress.parts_anchor(key))
            .filter(|anchor| anchor.index < series.parts.len());
        self.reset_parts_scroll();
        if let Some(anchor) = anchor {
            let y = -((anchor.index as f32 + anchor.offset) * PART_ROW_HEIGHT);
            self.video
                .parts_scroll
                .0
                .borrow_mut()
                .base_handle
                .set_offset(point(px(0.), px(y)));
        } else {
            self.video
                .parts_scroll
                .scroll_to_item_strict(fallback, ScrollStrategy::Top);
        }
    }

    /// Probe/self-check helper: place the part list at a manual position.
    #[cfg(debug_assertions)]
    pub(super) fn scroll_parts_to(&self, index: usize, offset: f32) {
        let Some(series) = &self.video.series else {
            return;
        };
        if series.parts.is_empty() {
            return;
        }
        let index = index.min(series.parts.len() - 1);
        let offset = offset.clamp(0., 0.999);
        let mut state = self.video.parts_scroll.0.borrow_mut();
        state.deferred_scroll_to_item = None;
        state.base_handle.set_offset(point(
            px(0.),
            px(-((index as f32 + offset) * PART_ROW_HEIGHT)),
        ));
    }

    #[cfg(debug_assertions)]
    pub(super) fn parts_top(&self) -> Option<(usize, f32)> {
        let series = self.video.series.as_ref()?;
        if series.parts.is_empty() {
            return None;
        }
        let state = self.video.parts_scroll.0.borrow();
        let top = (-f32::from(state.base_handle.offset().y) / PART_ROW_HEIGHT).max(0.);
        let index = (top.floor() as usize).min(series.parts.len() - 1);
        Some((index, top - index as f32))
    }

    fn valid_video_position(&self) -> Option<(PathBuf, f64, Playback)> {
        if self.page != "video" {
            return None;
        }
        let path = self.video.path.clone()?;
        let playback = self.media.player.snapshot();
        (self.media.session.is_video()
            && self.media.session.path.as_ref() == Some(&path)
            && self.media.matches_snapshot(&playback)
            && playback.path == path.to_string_lossy()
            && playback.duration > 0.
            && playback.position.is_finite()
            && playback.position >= 0.)
            .then_some((path, playback.position, playback))
    }

    fn reading_snapshot(&self, position: f64) -> Reading {
        let active_start = self
            .video
            .cues
            .iter()
            .find(|cue| position < cue.end)
            .or_else(|| self.video.cues.last())
            .map(|cue| cue.start);
        let visible = self.video.scroll.logical_scroll_top();
        let cue_start = if self.video.follow {
            active_start
        } else {
            self.video
                .cues
                .get(visible.item_ix)
                .map(|cue| cue.start)
                .or(active_start)
        };
        let transcript = self.video.transcript_scroll.logical_scroll_top();
        Reading {
            tab: self.video.tab,
            transcript_open: self.video.transcript_open,
            follow: self.video.follow,
            cue_start,
            active_start,
            cue_offset: f32::from(visible.offset_in_item),
            transcript_item: Some(transcript.item_ix),
            transcript_cue_start: self
                .video
                .transcript
                .paragraph_cues
                .get(transcript.item_ix)
                .copied(),
            transcript_offset: f32::from(transcript.offset_in_item),
            transcript_revision: self.video.transcript.revision(),
        }
    }

    fn progress_item(&self, position: f64, completed: bool) -> Item {
        Item {
            position,
            completed,
            sentence_repeat: self.video.sentence_repeat,
            repeat_mode: self.video.repeat_mode.index(),
            reading: self.reading_snapshot(position),
            updated_ms: 0,
        }
    }

    pub(super) fn save_video_progress(&mut self) {
        let parts_saved = self.save_parts_anchor();
        let Some((path, raw_position, playback)) = self.valid_video_position() else {
            if parts_saved {
                let _ = self.video_progress.save(&self.lib);
            }
            return;
        };
        let position = raw_position;
        let completed = playback.eof
            && self.media.session.mode == PlaybackMode::Normal
            && self.media.session.video_sentence().is_none()
            && self.video.sentence_repeat.is_none();
        let key = video_progress::item_key(&self.video.collection, &path);
        let item = self.progress_item(position, completed);
        if self.video_progress.record(key.clone(), item).is_err() {
            if parts_saved {
                let _ = self.video_progress.save(&self.lib);
            }
            return;
        }
        let _ = self.video_progress.save(&self.lib);
        self.video.progress_key = Some(key);
    }

    fn saved_item(&mut self, path: &Path) -> (String, Option<Item>) {
        let key = video_progress::item_key(&self.video.collection, path);
        self.video_progress.seed_legacy(&self.lib, &key, path);
        let item = self.video_progress.get(&key).cloned();
        (key, item)
    }

    fn open_with_item(
        &mut self,
        path: PathBuf,
        item: Option<Item>,
        from_start: bool,
        autoplay: bool,
    ) -> bool {
        let key = video_progress::item_key(&self.video.collection, &path);
        let reading = item
            .as_ref()
            .map(|item| item.reading.clone())
            .unwrap_or_default();
        let repeat = item.as_ref().and_then(|item| item.sentence_repeat);
        let mode = item
            .as_ref()
            .map(|item| match item.repeat_mode {
                1 => PlaybackMode::Track,
                2 => PlaybackMode::Ab,
                _ => PlaybackMode::Normal,
            })
            .unwrap_or(PlaybackMode::Normal);
        self.video.sentence_repeat = repeat;
        self.video.repeat_mode = mode;
        // A completed item behaves like Replay: start at zero while keeping
        // the chosen reading tab, but do not jump back to the last cue.
        let restart = from_start || item.as_ref().is_some_and(|item| item.completed);
        let position = if restart {
            0.
        } else {
            item.as_ref().map_or(0., |item| item.position)
        };
        if !self.open_at_start(path, position, !autoplay, false, None) {
            return false;
        }
        self.video.progress_key = Some(key);
        self.video.reading_restore = Some(if restart {
            Reading {
                tab: reading.tab,
                transcript_open: reading.transcript_open,
                ..Reading::default()
            }
        } else {
            reading.clone()
        });
        self.video.tab = reading.tab.min(3);
        self.video.transcript_open = reading.transcript_open;
        true
    }

    pub(super) fn open_continue(&mut self, path: PathBuf, autoplay: bool) -> bool {
        self.video_progress = video_progress::Progress::load(&self.lib);
        let (_, item) = self.saved_item(&path);
        self.open_with_item(path, item, false, autoplay)
    }

    pub(super) fn open_from_start(&mut self, path: PathBuf, autoplay: bool) -> bool {
        self.video_progress = video_progress::Progress::load(&self.lib);
        let (_, item) = self.saved_item(&path);
        self.open_with_item(path, item, true, autoplay)
    }

    pub(super) fn mark_video_completed(&mut self) {
        let Some((path, _, playback)) = self.valid_video_position() else {
            return;
        };
        let key = video_progress::item_key(&self.video.collection, &path);
        let item = self.progress_item(playback.duration, true);
        if self.video_progress.record(key.clone(), item).is_ok() {
            let _ = self.video_progress.save(&self.lib);
            self.video.progress_key = Some(key);
        }
    }

    pub(super) fn resume_state(&self, video: &video_library::Video) -> Resume {
        let path = video
            .series
            .as_ref()
            .and_then(|series| series.current.and_then(|index| series.parts.get(index)))
            .and_then(|part| part.path.clone())
            .unwrap_or_else(|| video.path.clone());
        let key = video_progress::item_key(&self.video.collection, &path);
        let item = self.video_progress.get(&key).cloned().or_else(|| {
            video_progress::legacy_position(&self.lib, &path).map(|position| Item {
                position,
                completed: false,
                sentence_repeat: None,
                repeat_mode: 0,
                reading: Reading::default(),
                updated_ms: 0,
            })
        });
        video_progress::resume(item.as_ref())
    }

    fn part_key(&self, series: &clarora_gpui::bilibili::Series, index: usize) -> Option<String> {
        video_progress::series_item_key(&self.video.collection, series, index).or_else(|| {
            series
                .parts
                .get(index)?
                .path
                .as_ref()
                .map(|path| video_progress::item_key(&self.video.collection, path))
        })
    }

    pub(super) fn part_resume_position(
        &self,
        series: &clarora_gpui::bilibili::Series,
        index: usize,
    ) -> Option<f64> {
        let key = self.part_key(series, index)?;
        self.video_progress
            .get(&key)
            .filter(|item| !item.completed && item.position > 0.)
            .map(|item| item.position)
    }

    pub(super) fn part_reading(
        &self,
        series: &clarora_gpui::bilibili::Series,
        index: usize,
    ) -> Option<Reading> {
        let key = self.part_key(series, index)?;
        self.video_progress.get(&key).map(|item| {
            if item.completed {
                Reading {
                    tab: item.reading.tab,
                    transcript_open: item.reading.transcript_open,
                    ..Reading::default()
                }
            } else {
                item.reading.clone()
            }
        })
    }
}
