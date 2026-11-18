//! Domain state owners. Views render snapshots; resource lifetimes live here.
use super::*;

/// Fixed height of one part row in the series list.
pub(super) const PART_ROW_HEIGHT: f32 = 64.;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TranscriptSelection {
    pub(super) anchor: (usize, usize),
    pub(super) focus: (usize, usize),
}

pub(super) struct AudioClipReview {
    pub(super) audio_id: String,
    pub(super) ranges: Vec<clip_marks::Range>,
    pub(super) index: usize,
    pub(super) started: Instant,
}
pub(super) struct AudioState {
    pub(super) library: audio_library::State,
    pub(super) rename_subscription: Option<Subscription>,
    pub(super) library_scroll: ScrollHandle,
    pub(super) practices: Vec<Row>,
    pub(super) audios: Vec<Row>,
    pub(super) practice_id: String,
    pub(super) checked: std::collections::BTreeSet<String>,
    pub(super) current_id: String,
    pub(super) waveform: Option<Waveform>,
    pub(super) waveform_rx: Option<mpsc::Receiver<std::result::Result<Waveform, String>>>,
    pub(super) waveform_cancel: Option<Cancel>,
    pub(super) waveform_bounds: std::rc::Rc<std::cell::Cell<Bounds<Pixels>>>,
    pub(super) waveform_drag: Option<bool>,
    pub(super) text_edge_drag: Option<bool>,
    pub(super) review: Option<AudioClipReview>,
    /// Review queue restricted to starred audio in the current practice group.
    pub(super) starred_review: bool,
    pub(super) cues: Vec<Cue>,
    pub(super) scroll: ListState,
    pub(super) center: subtitle_follow_ui::Follow,
    pub(super) active: Option<usize>,
    pub(super) follow: bool,
    pub(super) blind: blind_listening::State,
    pub(super) blind_chinese: bool,
    pub(super) blind_pending: Option<(Instant, String, blind_listening::Revealed)>,
    pub(super) blind_settle_until: Option<Instant>,
    pub(super) clip_mode: bool,
    pub(super) tools_open: bool,
    pub(super) show_current: bool,
    pub(super) current_scroll: ScrollHandle,
    pub(super) font_size: u8,
    pub(super) key_points: Vec<ai::KeyPoint>,
    pub(super) repeat_mode: PlaybackMode,
    pub(super) pending_repeat_pause: Option<bool>,
    pub(super) pending_repeat_position: Option<f64>,
}
impl AudioState {
    pub(super) fn new(lib: &Library) -> Self {
        Self {
            library: Default::default(),
            rename_subscription: None,
            library_scroll: ScrollHandle::new(),
            practices: vec![],
            audios: vec![],
            practice_id: String::new(),
            checked: Default::default(),
            current_id: String::new(),
            waveform: None,
            waveform_rx: None,
            waveform_cancel: None,
            waveform_bounds: Default::default(),
            waveform_drag: None,
            text_edge_drag: None,
            review: None,
            starred_review: false,
            cues: vec![],
            scroll: ListState::new(0, ListAlignment::Top, px(120.)),
            center: Default::default(),
            active: None,
            follow: true,
            blind: blind_listening::State::default(),
            blind_chinese: false,
            blind_pending: None,
            blind_settle_until: None,
            clip_mode: false,
            tools_open: false,
            show_current: true,
            current_scroll: ScrollHandle::new(),
            font_size: 18,
            key_points: vec![],
            repeat_mode: lib
                .setting("audio_repeat_mode")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|n| [0, 1, 3, 4, 5, 6, 7].contains(n))
                .map(PlaybackMode::from_index)
                .unwrap_or_default(),
            pending_repeat_pause: None,
            pending_repeat_position: None,
        }
    }
}

pub(super) struct PendingVideoStart {
    pub(super) revision: u64,
    pub(super) path: PathBuf,
    pub(super) paused: bool,
    pub(super) from_end: bool,
    pub(super) seek_to: Option<f64>,
    pub(super) selected_time: Option<f64>,
    pub(super) within_sentence_seek: Option<f64>,
}
/// Parameters confirmed by the import dialog. Kept local so a failure or
/// cancellation can reopen the same link, part and target group.
#[derive(Clone, Debug)]
pub(super) struct VideoImportDraft {
    pub(super) raw: String,
    pub(super) normalized: Option<String>,
    pub(super) part: Option<usize>,
    pub(super) group: String,
}
pub(super) struct VideoState {
    pub(super) path: Option<PathBuf>,
    pub(super) series: Option<bilibili::Series>,
    pub(super) auto_next_part: bool,
    pub(super) cues: Vec<Cue>,
    pub(super) caption_source: VideoCaptionSource,
    pub(super) transcript: clarora_gpui::video_transcript::Transcript,
    pub(super) transcript_open: bool,
    pub(super) transcript_paragraphs: std::rc::Rc<Vec<SharedString>>,
    pub(super) transcript_revision: u64,
    pub(super) captions_enabled: bool,
    pub(super) material: Option<video_qa::Material>,
    pub(super) material_error: String,
    pub(super) scroll: ListState,
    pub(super) center: subtitle_follow_ui::Follow,
    pub(super) tab: usize,
    pub(super) tools_open: bool,
    pub(super) panel_width: f32,
    pub(super) resize: Option<(Pixels, f32)>,
    pub(super) parts_scroll: UniformListScrollHandle,
    /// Series whose manual part-list position is currently installed in
    /// `parts_scroll`; avoids resetting the list when only the playing P changes.
    pub(super) parts_series_key: Option<String>,
    pub(super) active: Option<usize>,
    pub(super) sentence_repeat: Option<u32>,
    /// Preserve a sentence-repeat mode while background subtitles replace cues.
    pub(super) repeat_resume: Option<Option<u32>>,
    pub(super) repeat_mode: PlaybackMode,
    pub(super) pending_start: Option<PendingVideoStart>,
    pub(super) part_confirmation: Option<video_controller::PartConfirmation>,
    pub(super) follow: bool,
    pub(super) fullscreen: bool,
    pub(super) fullscreen_owned: bool,
    pub(super) fullscreen_seen: bool,
    pub(super) picture_click: Option<PathBuf>,
    pub(super) precision: bool,
    pub(super) subtitle_size: u8,
    pub(super) repeat_tail_ms: u16,
    pub(super) seek_dragging: bool,
    pub(super) seek_drag_target: Option<(u64, f64)>,
    pub(super) volume_dragging: bool,
    pub(super) collection: video_library::Collection,
    pub(super) remove: Option<String>,
    pub(super) youtube_browser: Option<process::YoutubeBrowser>,
    pub(super) group: String,
    pub(super) progress_key: Option<String>,
    pub(super) import_group: Option<String>,
    pub(super) reading_restore: Option<clarora_gpui::video_progress::Reading>,
    pub(super) transcript_scroll: ListState,
    /// Cross-paragraph selection in the English transcript.
    pub(super) transcript_selection: Option<TranscriptSelection>,
    pub(super) transcript_drag_anchor: Option<(usize, usize)>,
    pub(super) transcript_focus: FocusHandle,
    pub(super) completion_marked: bool,
    pub(super) import_draft: Option<VideoImportDraft>,
    pub(super) import_recovery: Option<VideoImportDraft>,
    pub(super) import_active: bool,
    pub(super) import_edit: Option<VideoImportDraft>,
    /// Group renames are name-based in storage; keep the old name resolvable
    /// for an import task that was confirmed before the rename.
    pub(super) group_aliases: crate::BTreeMap<String, String>,
}
impl VideoState {
    /// Snapshot the reader before a background subtitle update replaces the
    /// cue list. These callbacks do not pass through `open_with_item`, so the
    /// previous position must be carried forward explicitly.
    fn reading_for_cue_change(&self) -> clarora_gpui::video_progress::Reading {
        let cue_top = self.scroll.logical_scroll_top();
        let transcript_top = self.transcript_scroll.logical_scroll_top();
        let active_start = self
            .active
            .and_then(|index| self.cues.get(index))
            .or_else(|| self.cues.get(cue_top.item_ix))
            .map(|cue| cue.start);
        clarora_gpui::video_progress::Reading {
            tab: self.tab,
            transcript_open: self.transcript_open,
            follow: self.follow,
            cue_start: self.cues.get(cue_top.item_ix).map(|cue| cue.start),
            active_start,
            cue_offset: f32::from(cue_top.offset_in_item),
            transcript_item: Some(transcript_top.item_ix),
            transcript_cue_start: self
                .transcript
                .paragraph_cues
                .get(transcript_top.item_ix)
                .copied(),
            transcript_offset: f32::from(transcript_top.offset_in_item),
            transcript_revision: self.transcript.revision(),
        }
    }

    pub(super) fn set_cues(&mut self, cues: Vec<Cue>) {
        // Async subtitle loading may deliver the same list again after a
        // progress restore; keep the restored active cue instead of clearing it.
        if !cues.is_empty()
            && cues.len() == self.cues.len()
            && cues
                .iter()
                .zip(&self.cues)
                .all(|(a, b)| a.start == b.start && a.end == b.end && a.text == b.text)
        {
            return;
        }
        let preserve =
            (self.reading_restore.is_none() && !cues.is_empty() && !self.cues.is_empty())
                .then(|| self.reading_for_cue_change());
        self.part_confirmation = None;
        self.transcript = clarora_gpui::video_transcript::Transcript::from_cues(&cues);
        self.transcript_paragraphs = std::rc::Rc::new(
            self.transcript
                .paragraphs
                .iter()
                .cloned()
                .map(SharedString::from)
                .collect(),
        );
        self.transcript_revision = self.transcript_revision.wrapping_add(1);
        self.scroll.reset(cues.len());
        self.transcript_scroll
            .reset(self.transcript.paragraphs.len());
        self.transcript_selection = None;
        self.transcript_drag_anchor = None;
        self.active = None;
        let empty = cues.is_empty();
        self.cues = cues;
        // Subtitles are loaded in a second step after the player opens. Keep a
        // cue-bound snapshot until the real cue list arrives, otherwise the
        // empty reset consumes it and the sentence position is lost.
        let defer = self
            .reading_restore
            .as_ref()
            .is_some_and(|reading| empty && reading.cue_start.is_some());
        if !defer && let Some(reading) = self.reading_restore.take().or(preserve) {
            self.tab = reading.tab.min(3);
            self.transcript_open = reading.transcript_open;
            self.follow = reading.follow;
            if let Some(start) = reading.active_start.or(reading.cue_start)
                && let Some(index) = self
                    .cues
                    .iter()
                    .position(|cue| (cue.start - start).abs() < 0.05)
                    .or_else(|| self.cues.iter().position(|cue| cue.start > start))
                    .or_else(|| self.cues.len().checked_sub(1))
            {
                self.active = Some(index);
            }
            if let Some(start) = reading.cue_start
                && let Some(index) = self
                    .cues
                    .iter()
                    .position(|cue| (cue.start - start).abs() < 0.05)
                    .or_else(|| self.cues.iter().position(|cue| cue.start > start))
                    .or_else(|| self.cues.len().checked_sub(1))
            {
                self.scroll.scroll_to(ListOffset {
                    item_ix: index,
                    offset_in_item: px(reading.cue_offset),
                });
            }
            let revision_matches = reading.transcript_revision.is_empty()
                || reading.transcript_revision == self.transcript.revision();
            let exact_item = reading
                .transcript_item
                .filter(|item| *item < self.transcript.paragraphs.len());
            let anchored_item = if revision_matches {
                exact_item
            } else {
                // The English text changed. Re-locate the same reading content
                // by its cue time before considering the old paragraph index.
                reading
                    .transcript_cue_start
                    .map(|time| self.transcript.paragraph_for_time(time))
            };
            if let Some(item) = anchored_item {
                self.transcript_scroll.scroll_to(ListOffset {
                    item_ix: item,
                    offset_in_item: if revision_matches {
                        px(reading.transcript_offset)
                    } else {
                        px(0.)
                    },
                });
            } else if let Some(item) = exact_item {
                self.transcript_scroll.scroll_to(ListOffset {
                    item_ix: item,
                    offset_in_item: px(0.),
                });
            } else if !self.transcript.paragraphs.is_empty() {
                let item = self
                    .transcript
                    .paragraph_for_time(reading.active_start.unwrap_or(0.));
                self.transcript_scroll.scroll_to(ListOffset {
                    item_ix: item,
                    offset_in_item: px(0.),
                });
            }
        }
    }

    pub(super) fn new(cx: &mut Context<AppView>) -> Self {
        Self {
            path: None,
            series: None,
            auto_next_part: true,
            cues: vec![],
            caption_source: VideoCaptionSource::Transcript,
            transcript: Default::default(),
            transcript_open: false,
            transcript_paragraphs: std::rc::Rc::new(Vec::new()),
            transcript_revision: 0,
            captions_enabled: true,
            material: None,
            material_error: String::new(),
            scroll: ListState::new(0, ListAlignment::Top, px(200.)),
            center: Default::default(),
            tab: 3,
            tools_open: false,
            panel_width: 360.,
            resize: None,
            parts_scroll: UniformListScrollHandle::new(),
            parts_series_key: None,
            active: None,
            sentence_repeat: None,
            repeat_resume: None,
            repeat_mode: PlaybackMode::Normal,
            pending_start: None,
            part_confirmation: None,
            follow: true,
            fullscreen: false,
            fullscreen_owned: false,
            fullscreen_seen: false,
            picture_click: None,
            precision: false,
            subtitle_size: 16,
            repeat_tail_ms: 350,
            seek_dragging: false,
            seek_drag_target: None,
            volume_dragging: false,
            collection: Default::default(),
            remove: None,
            youtube_browser: None,
            group: String::new(),
            progress_key: None,
            import_group: None,
            reading_restore: None,
            transcript_scroll: ListState::new(0, ListAlignment::Top, px(400.)),
            transcript_selection: None,
            transcript_drag_anchor: None,
            transcript_focus: cx.focus_handle(),
            completion_marked: false,
            import_draft: None,
            import_recovery: None,
            import_active: false,
            import_edit: None,
            group_aliases: Default::default(),
        }
    }
}

pub(super) struct PlaybackState {
    pub(super) player: Player,
    pub(super) preloader: Player,
    pub(super) preloaded_index: Option<usize>,
    pub(super) snapshot: Playback,
    load_after: Option<u64>,
    preloaded_after: Option<u64>,
    pub(super) session: clarora_gpui::playback_session::Session,
    pub(super) rate: f64,
    pub(super) volume: f64,
    pub(super) volume_before_mute: f64,
    pub(super) stats: clarora_gpui::media_stats::MediaStats,
}
impl PlaybackState {
    pub(super) fn poll(&mut self) -> bool {
        let snapshot = self.player.snapshot();
        let snapshot = if self.matches_snapshot(&snapshot) {
            snapshot
        } else {
            // Failed loads never publish FILE_LOADED. Keep errors visible while
            // withholding the previous material's position, EOF and video frame.
            Playback {
                error: if self.session.owner().is_some() {
                    snapshot.error
                } else {
                    String::new()
                },
                ..Playback::default()
            }
        };
        let changed = clarora_gpui::media::controls_changed(&self.snapshot, &snapshot);
        self.snapshot = snapshot;
        changed
    }
    pub(super) fn matches_snapshot(&self, snapshot: &Playback) -> bool {
        self.session
            .path
            .as_ref()
            .is_some_and(|path| path.to_string_lossy() == snapshot.path)
            && self
                .load_after
                .is_none_or(|revision| snapshot.load_revision > revision)
    }
    pub(super) fn activate(&mut self, owner: PlaybackOwner, path: PathBuf, mode: PlaybackMode) {
        self.player.clear_sentence_repeat();
        self.player.set("loop-file", "no");
        self.session.claim(owner, path, mode);
        self.load_after = None;
        self.snapshot = Playback::default();
    }
    pub(super) fn open(
        &mut self,
        owner: PlaybackOwner,
        path: PathBuf,
        start: f64,
        end: f64,
        paused: bool,
        mode: PlaybackMode,
    ) {
        self.activate(owner, path.clone(), mode);
        self.load_after = Some(self.player.snapshot().load_revision);
        self.player.open_range(&path, start, end, paused);
    }
    pub(super) fn preload_card(&mut self, index: usize, path: &Path, start: f64, end: f64) {
        self.preloaded_index = Some(index);
        self.preloaded_after = Some(self.preloader.snapshot().load_revision);
        self.preloader.open_range(path, start, end, true);
    }
    pub(super) fn play_preloaded(&mut self, owner: PlaybackOwner, path: PathBuf) {
        self.activate(owner, path, PlaybackMode::Normal);
        std::mem::swap(&mut self.player, &mut self.preloader);
        self.load_after = self.preloaded_after.take();
        self.preloader.set_video_target(0, 0, false);
        self.preloader.set("pause", "yes");
        self.player.set("volume", self.volume);
        self.player.set("speed", self.rate);
        self.player.set("pause", "no");
    }
    pub(super) fn release(&mut self) {
        self.player.clear_sentence_repeat();
        self.player.set("loop-file", "no");
        self.player.set("pause", "yes");
        self.preloader.set("pause", "yes");
        self.preloaded_index = None;
        self.preloaded_after = None;
        self.session.release();
        self.load_after = None;
        self.snapshot = Playback::default();
    }
    pub(super) fn new() -> Self {
        Self {
            player: Player::new(),
            preloader: Player::new(),
            preloaded_index: None,
            snapshot: Playback::default(),
            load_after: None,
            preloaded_after: None,
            session: Default::default(),
            rate: 1.,
            volume: 100.,
            volume_before_mute: 100.,
            stats: Default::default(),
        }
    }
}

pub(super) struct TasksState {
    pub(super) foreground: task_runtime::Lane<TaskUpdate, Outcome>,
    pub(super) cloud: task_runtime::Lane<TaskUpdate, Outcome>,
    pub(super) chat_request: Option<clarora_gpui::chat_store::Request>,
    pub(super) models: model_controller::State,
    pub(super) audio_subtitles: audio_subtitles_controller::State,
    pub(super) backup_progress: Option<backup::UploadProgress>,
    #[cfg(debug_assertions)]
    pub(super) backup_fixture: Option<std::net::SocketAddr>,
    pub(super) progress_label: String,
    pub(super) progress_elapsed: u64,
    pub(super) retry: Option<feedback_ui::Retry>,
    pub(super) notice: Option<TaskNotice>,
    pub(super) cloud_notice: Option<TaskNotice>,
    pub(super) collapsed: bool,
    pub(super) video_notice: Option<TaskNotice>,
    pub(super) video_job: Option<video_jobs::Job>,
    pub(super) video_error: String,
    pub(super) downloads: video_jobs::Downloads,
    pub(super) title_events: Option<mpsc::Receiver<(String, String)>>,
}
impl TasksState {
    pub(super) fn new() -> Self {
        Self {
            foreground: Default::default(),
            cloud: Default::default(),
            chat_request: None,
            models: Default::default(),
            audio_subtitles: Default::default(),
            backup_progress: None,
            #[cfg(debug_assertions)]
            backup_fixture: None,
            progress_label: String::new(),
            progress_elapsed: 0,
            retry: None,
            notice: None,
            cloud_notice: None,
            collapsed: false,
            video_notice: None,
            video_job: None,
            video_error: String::new(),
            downloads: Default::default(),
            title_events: None,
        }
    }
}

pub(super) struct SettingsState {
    pub(super) secure_credentials: bool,
    pub(super) runtime: clarora_gpui::runtime::Report,
}
impl SettingsState {
    pub(super) fn new(lib: &Library) -> Self {
        Self {
            secure_credentials: credentials::secure_mode(lib).unwrap_or(false),
            runtime: clarora_gpui::runtime::inspect(),
        }
    }
}
impl Drop for TasksState {
    fn drop(&mut self) {
        self.foreground.cancel();
        self.cloud.cancel();
        if let Some(job) = &self.video_job {
            job.cancel.store(true, Ordering::SeqCst);
        }
    }
}
impl Drop for AudioState {
    fn drop(&mut self) {
        if let Some(cancel) = &self.waveform_cancel {
            cancel.store(true, Ordering::SeqCst);
        }
    }
}
