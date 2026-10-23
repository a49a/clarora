//! Domain state owners. Views render snapshots; resource lifetimes live here.
use super::*;

pub(super) struct AudioState {
    pub(super) library: audio_library::State,
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
    pub(super) cues: Vec<Cue>,
    pub(super) scroll: ListState,
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
}
impl AudioState {
    pub(super) fn new() -> Self {
        Self {
            library: Default::default(),
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
            cues: vec![],
            scroll: ListState::new(0, ListAlignment::Top, px(120.)),
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
        }
    }
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
    pub(super) tab: usize,
    pub(super) tools_open: bool,
    pub(super) panel_width: f32,
    pub(super) resize: Option<(Pixels, f32)>,
    pub(super) parts_scroll: UniformListScrollHandle,
    pub(super) active: Option<usize>,
    pub(super) sentence_repeat: Option<clarora_gpui::video_repeat::Session>,
    pub(super) follow: bool,
    pub(super) fullscreen: bool,
    pub(super) fullscreen_owned: bool,
    pub(super) fullscreen_seen: bool,
    pub(super) picture_click: Option<PathBuf>,
    pub(super) precision: bool,
    pub(super) subtitle_size: u8,
    pub(super) seek_dragging: bool,
    pub(super) volume_dragging: bool,
    pub(super) collection: video_library::Collection,
    pub(super) remove: Option<String>,
}
impl VideoState {
    pub(super) fn set_cues(&mut self, cues: Vec<Cue>) {
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
            tab: 3,
            tools_open: false,
            panel_width: 360.,
            resize: None,
            parts_scroll: UniformListScrollHandle::new(),
            active: None,
            sentence_repeat: None,
            follow: true,
            fullscreen: false,
            fullscreen_owned: false,
            fullscreen_seen: false,
            picture_click: None,
            precision: false,
            subtitle_size: 16,
            seek_dragging: false,
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
    pub(super) path: Option<PathBuf>,
    pub(super) loop_a: Option<f64>,
    pub(super) loop_b: Option<f64>,
    pub(super) repeat: usize,
    pub(super) rate: f64,
    pub(super) volume: f64,
    pub(super) volume_before_mute: f64,
    pub(super) end_handled: bool,
    pub(super) stats_ticks: f64,
}
impl PlaybackState {
    pub(super) fn poll(&mut self) -> bool {
        let snapshot = self.player.snapshot();
        let changed = clarora_gpui::media::visible_changed(&self.snapshot, &snapshot);
        self.snapshot = snapshot;
        changed
    }
    pub(super) fn new() -> Self {
        Self {
            player: Player::new(),
            preloader: Player::new(),
            preloaded_index: None,
            snapshot: Playback::default(),
            path: None,
            loop_a: None,
            loop_b: None,
            repeat: 0,
            rate: 1.,
            volume: 100.,
            volume_before_mute: 100.,
            end_handled: false,
            stats_ticks: 0.,
        }
    }
}

pub(super) struct TasksState {
    pub(super) cancel: Option<Cancel>,
    pub(super) scope: TaskScope,
    pub(super) model_progress: Option<ModelProgress>,
    pub(super) progress_label: String,
    pub(super) started: Option<Instant>,
    pub(super) progress_elapsed: u64,
    pub(super) retry: Option<feedback_ui::Retry>,
    pub(super) notice: Option<TaskNotice>,
    pub(super) video_notice: Option<TaskNotice>,
    pub(super) video_job: Option<video_jobs::Job>,
    pub(super) video_error: String,
    pub(super) downloads: video_jobs::Downloads,
    pub(super) events: Option<mpsc::Receiver<Event>>,
    pub(super) title_events: Option<mpsc::Receiver<(String, String)>>,
}
impl TasksState {
    pub(super) fn new() -> Self {
        Self {
            cancel: None,
            scope: TaskScope::Material,
            model_progress: None,
            progress_label: String::new(),
            started: None,
            progress_elapsed: 0,
            retry: None,
            notice: None,
            video_notice: None,
            video_job: None,
            video_error: String::new(),
            downloads: Default::default(),
            events: None,
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
        if let Some(cancel) = &self.cancel {
            cancel.store(true, Ordering::SeqCst);
        }
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
