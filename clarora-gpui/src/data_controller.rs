//! Accept complete background snapshots at the UI boundary.
use super::*;
use clarora_gpui::library_controller::{Controller, Key, Snapshot};
use std::collections::HashSet;

#[derive(Default)]
pub(super) struct Loads {
    controller: Controller,
    requested: Option<Key>,
    accepted: Option<Key>,
    play_pending: bool,
    pub(super) restore_pending: bool,
    pub(super) restore_audio: Option<(String, f64)>,
    pub(super) discover_due: HashSet<String>,
    pub(super) discover_favorites: HashSet<String>,
    pub(super) error: String,
}

impl AppView {
    fn data_key(&self) -> Key {
        Key {
            page: self.page.clone(),
            practice_id: self.audio.practice_id.clone(),
            card_filter: self.card_filter.clone(),
            due: self.due,
            saved_only: self.saved_only,
            selected_audio_id: self.audio.current_id.clone(),
            selected_subtitle_uri: self
                .selected()
                .map(|r| text(&r, "subtitle_uri"))
                .unwrap_or_default(),
            library_selected: self.audio.library.selected.clone(),
            library_filter: self.audio.library.filter,
            library_sort: self.audio.library.sort,
            video_path: if self.page == "video" {
                self.video
                    .path
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default()
            } else {
                String::new()
            },
        }
    }

    pub(super) fn refresh(&mut self) {
        self.media.preloaded_index = None;
        let key = self.data_key();
        let runtime = self.page == "settings" || self.settings.runtime.components.is_empty();
        self.loads
            .controller
            .request(&self.lib, key.clone(), runtime);
        self.loads.requested = Some(key);
        self.loads.error.clear();
    }

    pub(super) fn refresh_view_data(&mut self) {
        self.refresh();
    }

    pub(super) fn cards_ready(&self) -> bool {
        !self.loads.controller.is_loading()
            && self.loads.accepted.as_ref().is_some_and(|key| {
                key.page == self.page
                    && key.card_filter == self.card_filter
                    && key.due == self.due
                    && key.saved_only == self.saved_only
            })
    }

    pub(super) fn defer_card_play(&mut self) -> bool {
        if self.loads.controller.is_loading() || !self.cards_ready() {
            self.loads.play_pending = true;
            self.media.player.set("pause", "yes");
            return true;
        }
        false
    }

    pub(super) fn poll_data(&mut self) -> bool {
        let key = self.data_key();
        if self.loads.requested.as_ref() != Some(&key) {
            self.refresh();
        }
        let Some(result) = self.loads.controller.poll(&key) else {
            return false;
        };
        match result {
            Ok(snapshot) => self.accept_data(snapshot),
            Err(error) => {
                self.loads.error = error.clone();
                self.report_status(
                    FeedbackKind::Error,
                    format!("资料读取失败，保留上次内容：{error}"),
                );
            }
        }
        true
    }

    fn accept_data(&mut self, mut snapshot: Snapshot) {
        self.audio.practices = snapshot.practices;
        self.audio.audios = snapshot.audios;
        self.audio.practice_id = snapshot.practice_id;
        self.audio.checked = audio_library::scoped_ids(
            &self.audio.audios,
            &self.audio.practice_id,
            &self.audio.checked,
        )
        .into_iter()
        .collect();
        if self.audio.library.renaming.as_ref().is_some_and(|target| {
            target.group != self.audio.practice_id
                || !self.audio.audios.iter().any(|row| {
                    text(row, "id") == target.id && text(row, "practice_id") == target.group
                })
        }) {
            self.audio.library.cancel_rename();
        }
        self.audio.library.selected = snapshot.audio_library.selected;
        self.audio.library.subtitle_ids = snapshot.audio_library.subtitle_ids;
        self.audio.library.preview = snapshot.audio_library.preview;
        self.audio.library.subtitle_count = snapshot.audio_library.subtitle_count;
        if let Some(cards) = snapshot.cards {
            self.cards = cards;
            if self.index >= self.cards.len() {
                self.index = 0;
            }
        }
        if let Some(cues) = snapshot.cues {
            if !same_cues(&cues, &self.audio.cues) {
                self.audio.cues = cues;
                self.invalidate_blind();
                self.audio.scroll.reset(self.audio.cues.len());
                self.audio.active = None;
                self.audio.current_scroll.set_offset(Point::default());
            }
        }
        if let Some(points) = snapshot.audio_key_points {
            self.audio.key_points = points;
        }
        if let Some(cues) = snapshot.video_cues {
            if !same_cues(&cues, &self.video.cues) {
                self.video.set_cues(cues);
                self.video.active = None;
                self.video.material = None;
                self.video.material_error.clear();
            }
        }
        self.clip_catalog = snapshot.clip_catalog;
        self.refresh_saved_ranges();
        snapshot.data.revision = self.data.revision.wrapping_add(1);
        self.data = snapshot.data;
        if let Some(runtime) = snapshot.runtime {
            self.settings.runtime = runtime;
        }
        if let Some(due) = snapshot.discover_due {
            self.loads.discover_due = due.into_iter().collect();
        }
        if let Some(favorites) = snapshot.discover_favorites {
            self.loads.discover_favorites = favorites.into_iter().collect();
        }
        self.loads.restore_audio = snapshot.restore_audio;
        self.loads.error.clear();
        // Normalising a removed group/selection is part of accepting this snapshot.
        let current = self.data_key();
        self.loads.requested = Some(current.clone());
        self.loads.accepted = Some(current);
        if self.loads.restore_pending {
            self.loads.restore_pending = false;
            if self.page == "listening" {
                self.restore_listening_cached();
            }
        }
        if self.loads.play_pending {
            self.loads.play_pending = false;
            if self.page == "discover" || self.page == "flashcard" {
                self.play_card();
            }
        }
    }
}

fn same_cues(a: &[Cue], b: &[Cue]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(a, b)| a.start == b.start && a.end == b.end && a.text == b.text)
}
