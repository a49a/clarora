//! Domain state owners. Views render snapshots; resource lifetimes live here.
use super::*;

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
pub(super) struct VideoState {
    pub(super) path: Option<PathBuf>,
    pub(super) series: Option<bilibili::Series>,
    pub(super) auto_next_part: bool,
    pub(super) cues: Vec<Cue>,
    pub(super) caption_source: VideoCaptionSource,
    pub(super) transcript: clarora_gpui::video_transcript::Transcript,
    pub(super) transcript_open: bool,
    pub(super) transcript_html: SharedString,
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
    pub(super) active: Option<usize>,
    pub(super) sentence_repeat: Option<u32>,
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
    pub(super) seek_dragging: bool,
    pub(super) seek_drag_target: Option<(u64, f64)>,
    pub(super) volume_dragging: bool,
    pub(super) collection: video_library::Collection,
    pub(super) remove: Option<String>,
}
impl VideoState {
    pub(super) fn set_cues(&mut self, cues: Vec<Cue>) {
        self.part_confirmation = None;
        self.transcript = clarora_gpui::video_transcript::Transcript::from_cues(&cues);
        self.transcript_html = self.transcript.html.clone().into();
        self.transcript_revision = self.transcript_revision.wrapping_add(1);
        self.scroll.reset(cues.len());
        self.active = None;
        self.cues = cues;
    }

    pub(super) fn new() -> Self {
        Self {
            path: None,
            series: None,
            auto_next_part: true,
            cues: vec![],
            caption_source: VideoCaptionSource::Transcript,
            transcript: Default::default(),
            transcript_open: false,
            transcript_html: "".into(),
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
            active: None,
            sentence_repeat: None,
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
            seek_dragging: false,
            seek_drag_target: None,
            volume_dragging: false,
            collection: Default::default(),
            remove: None,
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
