#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod design;
mod library_ui;
mod video_library_ui;
use anyhow::{Context as _, Result, ensure};
use clarora_gpui::{
    ai, audio_library, backup, bilibili, blind_listening,
    data::{Card, Library, Row, id, now, text},
    documents,
    media::{Playback, Player},
    model_download::Progress as ModelProgress,
    process::{self, Cancel},
    recording::Recorder,
    subtitles::{self, Cue},
    tts, video_library, video_qa, video_subtitles,
    waveform::{self, Waveform},
};
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, Root, Selectable, Sizable, Theme, ThemeMode,
    WindowExt,
    button::*,
    dialog::DialogButtonProps,
    input::{Input, InputState},
    menu::{DropdownMenu, PopupMenuItem},
    progress::Progress,
    scroll::{Scrollbar, ScrollbarShow},
    text::TextView,
    tooltip::Tooltip,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::{atomic::Ordering, mpsc},
    time::{Duration, Instant, SystemTime},
};
actions!(
    clarora_gpui_actions,
    [
        Quit,
        OpenFile,
        ToggleAssistant,
        ToggleNavigation,
        AskAi,
        NewChat,
        ToggleBlindListening,
        RevealBlindSentence,
        ResumeBlindListening,
        RestartBlindListening,
        ViewAudioTranscript,
        Forward,
        Backward,
        PdfZoomIn,
        PdfZoomOut,
        ToggleAppearance,
        ComposeSentence,
        TranscribeVideo,
        NextVideoPart,
        PreviousVideoPart,
        ReadVideoParts,
        DownloadLocalModel,
        CancelTask,
        OpenSettings,
        ToggleVideoFullscreen,
        ExitVideoFullscreen,
        FullscreenPlayPause,
        FullscreenSeekBack,
        FullscreenSeekForward,
        ToggleVideoPrecision,
        MarkVideoA,
        MarkVideoB,
        ClearVideoLoop,
        RepeatVideoCue,
        PreviousVideoSentence,
        NextVideoSentence,
        AskVideoCurrent,
        AskVideoAll
    ]
);
const PAGES: &[(&str, &str, &str)] = &[
    ("discover", "随便学学", "打开就开始"),
    ("flashcard", "闪卡复习", "温习与计划"),
    ("listening", "音频学习", "逐句听懂"),
    ("video", "视频学习", "精听与片段"),
    ("speaking", "口语跟读", "录音与对齐"),
    ("library", "音频管理", "整理练习组"),
    ("pdf", "文档阅读", "本机 PDF"),
    ("ocr", "拍照识字", "从图片提取"),
    ("graph", "词汇表", "发现词语关系"),
    ("stats", "学习统计", "记录每一点进步"),
    ("meditation", "冥想放松", "给专注留白"),
    ("settings", "设置", "AI 与存储"),
];
#[derive(Clone)]
enum Action {
    Page(String),
    Pick(String),
    Audio(String),
    Practice(String),
    AudioPractice(String),
    AudioBlind(bool),
    AudioBlindChinese(bool),
    AudioBlindReveal,
    AudioBlindRevealCue(String, Cue),
    AudioBlindHide,
    AudioBlindResume(bool),
    AudioBlindSentence(i8),
    AudioBlindRow(String, Cue, Option<usize>),
    AudioClipMode,
    AudioTools,
    AudioFont(u8),
    AudioCurrent,
    AudioFollow,
    AudioCopySubtitle(String, Option<Cue>),
    AudioRepeat(usize),
    LibrarySelect(String),
    LibraryTool(String),
    LibrarySaveName,
    LibraryFilter(u8),
    LibrarySort(u8),
    LibraryBatch,
    LibrarySelectAll,
    LibraryReset,
    LibraryMove(String),
    LibraryDelete(Vec<String>),
    LibraryConfirmDelete,
    LibraryStudy(bool),
    Flip,
    Next,
    Previous,
    Grade(String),
    ReviewMode(bool),
    Saved(bool),
    Filter(String),
    SaveSentence,
    EditCard,
    Composer,
    SaveEditedCard,
    Play,
    Seek(f64),
    SeekTo(f64),
    Boundary(f64, bool),
    Word(f64, f64, String, bool),
    Score,
    Assistant,
    Navigation,
    Rate(f64),
    Volume(f64),
    Mark(bool),
    Repeat,
    VideoFullscreen,
    VideoExitFullscreen,
    VideoPrecision,
    VideoSubtitleSize(u8),
    VideoCaptions,
    VideoTranslate,
    VideoClearLoop,
    VideoRepeatCue,
    VideoSentence(bool),
    VideoPlayCue(usize),
    VideoCopySubtitle(Option<usize>),
    LoopMode(usize),
    Mute,
    Subtitle(i64, bool),
    SubtitleImport(bool),
    Download,
    ReadParts,
    SelectPart(usize),
    AutoNextPart,
    Cancel,
    Clip,
    Chat,
    SaveAnswer,
    ChatSession(String),
    ToggleReasoning,
    PdfOpen(PathBuf),
    Favorite,
    Transcribe,
    VideoTranscribe,
    VideoAsk(video_qa::Scope),
    VideoFollow,
    VideoPanel(usize),
    VideoTools,
    VideoSelect(String),
    VideoEpisodes(String),
    VideoRemove(String),
    VideoConfirmRemove,
    VideoSubtitleExport,
    Translate,
    Record,
    StopRecord,
    Timer(u64),
    TimerPause,
    TimerReset,
    SaveAi,
    SaveStorage,
    Models(String),
    SelectModel(String),
    CloudList,
    CloudBackup,
    CloudRestore(String),
    PdfPage(i32),
    PdfZoom(i32),
    Theme,
    Highlight,
    ClearHighlights,
    TtsDownload,
    TtsGenerate,
    TtsVoice(i32),
    TtsSpeed(f32),
    TtsPlay,
    MergeAudios,
    CheckAudio(String),
    Suspend(bool),
    CommandImport,
    Refresh,
    GraphSelect(String),
    GraphList(i64),
}
enum Outcome {
    Message(String),
    Parts(bilibili::Series),
    PartVideo(bilibili::Series, usize, PathBuf, bool, Option<PathBuf>),
    Output(String),
    VideoSubtitles(PathBuf, String, Vec<Cue>),
    VideoTranslated(PathBuf, String, Vec<Cue>, Vec<Cue>),
    Pdf(documents::PdfPage),
    Ocr(String),
    KeyPoints(String, Vec<ai::KeyPoint>),
    Backups(Vec<String>),
    Recording(PathBuf),
    TtsReady(Row),
}
enum Event {
    Done(std::result::Result<Outcome, String>),
    Chunk(ai::ChatReply),
    ModelProgress(ModelProgress),
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum TaskScope {
    Material,
    Application,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum VideoCaptionSource {
    Transcript,
    Track,
    Hidden,
}
struct AppView {
    lib: Library,
    library: audio_library::State,
    tts_voice: i32,
    tts_speed: f32,
    tts_last: Option<Row>,
    page: String,
    navigation_open: bool,
    navigation_motion: Option<(Instant, f32)>,
    inputs: BTreeMap<String, Entity<InputState>>,
    status: String,
    busy: Option<Cancel>,
    task_scope: TaskScope,
    model_progress: Option<ModelProgress>,
    rx: Option<mpsc::Receiver<Event>>,
    video_titles_rx: Option<mpsc::Receiver<(String, String)>>,
    cards: Vec<Card>,
    index: usize,
    revealed: bool,
    editing: Option<Card>,
    composer_open: bool,
    page_scroll: ScrollHandle,
    due: bool,
    card_filter: String,
    saved_only: bool,
    history: Vec<usize>,
    practices: Vec<Row>,
    audios: Vec<Row>,
    practice_id: String,
    checked_audio: std::collections::BTreeSet<String>,
    current_audio: String,
    waveform: Option<Waveform>,
    waveform_rx: Option<mpsc::Receiver<std::result::Result<Waveform, String>>>,
    waveform_cancel: Option<Cancel>,
    waveform_bounds: std::rc::Rc<std::cell::Cell<Bounds<Pixels>>>,
    waveform_drag: Option<bool>,
    player: Player,
    preloader: Player,
    preloaded_index: Option<usize>,
    playback: Playback,
    current_path: Option<PathBuf>,
    video_path: Option<PathBuf>,
    video_series: Option<bilibili::Series>,
    auto_next_part: bool,
    video_cues: Vec<Cue>,
    video_caption_source: VideoCaptionSource,
    video_captions_enabled: bool,
    video_material: Option<video_qa::Material>,
    video_material_error: String,
    video_scroll: ListState,
    video_tab: usize,
    video_tools_open: bool,
    video_panel_width: f32,
    video_resize: Option<(Pixels, f32)>,
    parts_scroll: UniformListScrollHandle,
    video_active: Option<usize>,
    video_follow: bool,
    video_fullscreen: bool,
    video_fullscreen_owned: bool,
    video_fullscreen_seen: bool,
    video_precision: bool,
    video_subtitle_size: u8,
    video_seek_dragging: bool,
    video_volume_dragging: bool,
    volume_before_mute: f64,
    cues: Vec<Cue>,
    audio_scroll: ListState,
    audio_active: Option<usize>,
    audio_follow: bool,
    audio_blind: blind_listening::State,
    audio_blind_chinese: bool,
    audio_blind_pending: Option<(Instant, String, blind_listening::Revealed)>,
    audio_blind_settle_until: Option<Instant>,
    audio_clip_mode: bool,
    audio_tools_open: bool,
    audio_show_current: bool,
    audio_font_size: u8,
    key_points: Vec<ai::KeyPoint>,
    videos: video_library::Collection,
    video_remove: Option<String>,
    loop_a: Option<f64>,
    loop_b: Option<f64>,
    repeat: usize,
    rate: f64,
    volume: f64,
    end_handled: bool,
    output: String,
    chat_history: Vec<Value>,
    chat_id: String,
    chat_context: String,
    reasoning: String,
    reasoning_open: bool,
    chat_draft: String,
    chat_running: bool,
    backups: Vec<String>,
    recording: Option<Recorder>,
    recorded: Option<PathBuf>,
    timer_end: Option<SystemTime>,
    timer_remaining: u64,
    timer_paused: bool,
    timer_focus: bool,
    pdf_path: Option<PathBuf>,
    pdf_image: Option<std::sync::Arc<RenderImage>>,
    pdf_page: u16,
    pdf_pages: u16,
    pdf_width: i32,
    pdf_bookmarks: Vec<(String, u16)>,
    document_text: String,
    ocr_text: String,
    ocr_path: Option<PathBuf>,
    dark: bool,
    focus: FocusHandle,
    stats_ticks: f64,
    assistant_open: bool,
    graph_node: String,
    graph_list: i64,
}
impl AppView {
    fn new(lib: Library, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut inputs = BTreeMap::new();
        for (key, placeholder, multi, masked) in [
            ("search", "搜索内容", false, false),
            ("library_search", "搜索当前组的音频", false, false),
            ("library_groups", "搜索练习组", false, false),
            ("library_name", "输入名称", false, false),
            ("group", "练习组名称", false, false),
            ("front", "原文 / 单词", true, false),
            ("back", "翻译 / 释义", true, false),
            (
                "bilibili",
                "https://www.bilibili.com/video/BV…?p=1",
                false,
                false,
            ),
            ("question", "向 AI 提问", true, false),
            ("reference", "朗读参考原文", true, false),
            (
                "tts_text",
                "输入要合成的中文或英文（最多 10,000 字）",
                true,
                false,
            ),
            ("tts_name", "生成音频的名称", false, false),
            ("base_url", "https://api.example.com/v1", false, false),
            ("model", "聊天／转写共用模型 ID", false, false),
            ("vision_model", "视觉模型", false, false),
            ("chat_key", "共用 API Key", false, true),
            ("local_model", "base.en", false, false),
            ("engine", "local / compatible", false, false),
            ("provider", "s3 / oss", false, false),
            ("endpoint", "https://s3.example.com", false, false),
            ("region", "Region", false, false),
            ("bucket", "Bucket", false, false),
            ("access_key", "Access Key ID", false, false),
            ("secret_key", "Secret Access Key", false, true),
            ("session_token", "Session Token（可选）", false, true),
            ("prefix", "clarora", false, false),
            ("path_style", "true / false", false, false),
            ("command", "本机导入命令", true, false),
            ("daily_limit", "每日新卡上限，例如 20", false, false),
        ] {
            inputs.insert(
                key.into(),
                cx.new(|cx| {
                    let mut input = InputState::new(window, cx).placeholder(placeholder);
                    if multi {
                        input = input.multi_line(true).rows(3);
                    }
                    if masked {
                        input = input.masked(true);
                    }
                    input
                }),
            );
        }
        let mut this = Self {
            tts_voice: lib
                .setting("tts_voice")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|v| (0..54).contains(v))
                .unwrap_or(0),
            tts_speed: lib
                .setting("tts_speed")
                .ok()
                .and_then(|v| v.parse::<f32>().ok())
                .filter(|v| v.is_finite() && (0.5..=2.).contains(v))
                .unwrap_or(1.),
            tts_last: None,
            library: Default::default(),
            lib,
            page: "flashcard".into(),
            navigation_open: true,
            navigation_motion: None,
            inputs,
            status: "准备好了，开始今天的学习。".into(),
            busy: None,
            task_scope: TaskScope::Material,
            model_progress: None,
            rx: None,
            video_titles_rx: None,
            cards: vec![],
            index: 0,
            revealed: false,
            editing: None,
            composer_open: false,
            page_scroll: ScrollHandle::new(),
            due: true,
            card_filter: "all".into(),
            saved_only: false,
            history: vec![],
            practices: vec![],
            audios: vec![],
            practice_id: String::new(),
            checked_audio: Default::default(),
            current_audio: String::new(),
            waveform: None,
            waveform_rx: None,
            waveform_cancel: None,
            waveform_bounds: Default::default(),
            waveform_drag: None,
            player: Player::new(),
            preloader: Player::new(),
            preloaded_index: None,
            playback: Playback::default(),
            current_path: None,
            video_path: None,
            video_series: None,
            auto_next_part: true,
            video_cues: vec![],
            video_caption_source: VideoCaptionSource::Transcript,
            video_captions_enabled: true,
            video_material: None,
            video_material_error: String::new(),
            video_scroll: ListState::new(0, ListAlignment::Top, px(200.)),
            video_tab: 3,
            video_tools_open: false,
            video_panel_width: 360.,
            video_resize: None,
            parts_scroll: UniformListScrollHandle::new(),
            video_active: None,
            video_follow: true,
            video_fullscreen: false,
            video_fullscreen_owned: false,
            video_fullscreen_seen: false,
            video_precision: false,
            video_subtitle_size: 16,
            video_seek_dragging: false,
            video_volume_dragging: false,
            volume_before_mute: 100.,
            cues: vec![],
            audio_scroll: ListState::new(0, ListAlignment::Top, px(120.)),
            audio_active: None,
            audio_follow: true,
            audio_blind: blind_listening::State::default(),
            audio_blind_chinese: false,
            audio_blind_pending: None,
            audio_blind_settle_until: None,
            audio_clip_mode: false,
            audio_tools_open: false,
            audio_show_current: true,
            audio_font_size: 18,
            key_points: vec![],
            videos: Default::default(),
            video_remove: None,
            loop_a: None,
            loop_b: None,
            repeat: 0,
            rate: 1.,
            volume: 100.,
            end_handled: false,
            output: String::new(),
            chat_history: vec![],
            chat_id: String::new(),
            chat_context: String::new(),
            reasoning: String::new(),
            reasoning_open: false,
            chat_draft: String::new(),
            chat_running: false,
            backups: vec![],
            recording: None,
            recorded: None,
            timer_end: None,
            timer_remaining: 1500,
            timer_paused: false,
            timer_focus: true,
            pdf_path: None,
            pdf_image: None,
            pdf_page: 0,
            pdf_pages: 0,
            pdf_width: 1000,
            pdf_bookmarks: vec![],
            document_text: String::new(),
            ocr_text: String::new(),
            ocr_path: None,
            dark: cx.theme().is_dark(),
            focus: cx.focus_handle(),
            stats_ticks: 0.,
            assistant_open: false,
            graph_node: "demo-1".into(),
            graph_list: 0,
        };
        this.video_series = bilibili::Series::load(&this.lib).unwrap_or(None);
        if let Some(series) = &this.video_series {
            this.parts_scroll.scroll_to_item(
                series.current.unwrap_or(series.requested),
                ScrollStrategy::Center,
            );
        }
        this.video_captions_enabled =
            this.lib.setting("video_captions_enabled").ok().as_deref() != Some("false");
        this.player
            .set_subtitles_visible(this.video_captions_enabled);
        this.video_subtitle_size = this
            .lib
            .setting("video_subtitle_size")
            .ok()
            .and_then(|value| value.parse::<u8>().ok())
            .filter(|size| [14, 16, 20, 24, 28, 32].contains(size))
            .unwrap_or(16);
        this.audio_font_size = this
            .lib
            .setting("subtitle_font_size")
            .ok()
            .and_then(|v| v.parse::<u8>().ok())
            .filter(|n| [14, 16, 18, 20, 24, 28, 32].contains(n))
            .unwrap_or(18);
        this.audio_blind_chinese =
            this.lib.setting("blind_chinese_hint").ok().as_deref() == Some("true");
        this.auto_next_part = this.lib.setting("video_auto_next").ok().as_deref() != Some("false");
        if let Some(series) = &this.video_series {
            let base = series.base.clone();
            this.set("bilibili", base, window, cx);
        }
        match video_library::Collection::load(&this.lib) {
            Ok(videos) => this.videos = videos,
            Err(e) => this.status = format!("{e:#}"),
        }
        let sources: Vec<_> = this
            .videos
            .items
            .iter()
            .filter_map(|v| v.series.as_ref())
            .filter(|s| s.title.trim().is_empty())
            .map(|s| s.base.clone())
            .collect();
        if !sources.is_empty() {
            let (tx, rx) = mpsc::channel();
            this.video_titles_rx = Some(rx);
            std::thread::spawn(move || {
                for base in sources {
                    if let Ok(series) = bilibili::fetch(&base, &process::cancel_token()) {
                        if tx.send((base, series.title)).is_err() {
                            break;
                        }
                    }
                }
            });
        }
        this.page = this
            .lib
            .setting("last_page")
            .ok()
            .filter(|p| PAGES.iter().any(|(k, _, _)| k == p))
            .unwrap_or("flashcard".into());
        this.navigation_open = this.lib.setting("navigation_open").ok().as_deref() != Some("false");
        if this.page == "library" {
            this.practice_id = this.lib.setting("library_group").unwrap_or_default();
        }
        this.refresh();
        let ai_result = ai::load(&this.lib);
        match ai_result {
            Ok((c, key, asr)) => {
                let (c, key) = ai::shared_settings(c, key, asr);
                for (k, v) in [
                    ("base_url", c.base_url),
                    ("model", c.model),
                    ("vision_model", c.vision_model),
                    ("local_model", c.local_model),
                    ("engine", c.engine),
                    ("chat_key", key),
                ] {
                    this.set(k, v, window, cx);
                }
            }
            Err(e) => this.status = e.to_string(),
        }
        if let Ok(c) = serde_json::from_str::<backup::StorageConfig>(
            &this.lib.setting("storage_config").unwrap_or_default(),
        ) {
            for (k, v) in [
                ("provider", c.provider),
                ("endpoint", c.endpoint),
                ("region", c.region),
                ("bucket", c.bucket),
                ("access_key", c.access_key_id),
                ("prefix", c.prefix),
                ("path_style", c.path_style.to_string()),
            ] {
                this.set(k, v, window, cx);
            }
        }
        for (k, default) in [
            ("provider", "s3"),
            ("prefix", "clarora"),
            ("path_style", "false"),
            ("daily_limit", "20"),
            ("command", ""),
        ] {
            let value = this
                .lib
                .setting(k)
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or(default.into());
            if this.value(k, cx).is_empty() {
                this.set(k, value, window, cx);
            }
        }
        cx.spawn(async move |this, cx| {
            loop {
                Timer::after(Duration::from_millis(50)).await;
                if this.update(cx, |this, cx| this.tick(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        if let Some(video) = this.videos.active().cloned().filter(|v| v.path.is_file()) {
            this.video_series = video.series;
            if let Some(series) = &mut this.video_series {
                series.current = series
                    .parts
                    .iter()
                    .position(|p| p.path.as_ref() == Some(&video.path));
            }
            let position = this
                .lib
                .setting("last_video_position")
                .ok()
                .and_then(|v| serde_json::from_str::<Value>(&v).ok())
                .filter(|v| v["path"].as_str() == video.path.to_str())
                .and_then(|v| v["position"].as_f64())
                .filter(|v| v.is_finite() && *v >= 0.)
                .unwrap_or(0.);
            this.player.open_range(&video.path, position, 0., true);
            this.current_path = Some(video.path.clone());
            this.video_path = Some(video.path.clone());
            this.load_video_subtitles(&video.path);
            this.video_tab = 3;
        } else {
            this.video_series = None;
        }
        if this.page == "listening" {
            this.restore_listening();
        }
        if let Ok(saved) = this.lib.setting("pdf_current") {
            if let Ok(value) = serde_json::from_str::<Value>(&saved) {
                let path = PathBuf::from(value["path"].as_str().unwrap_or(""));
                if path.is_file() {
                    this.pdf_path = Some(path);
                    this.pdf_page = value["page"].as_u64().unwrap_or(0).min(u16::MAX as u64) as u16;
                    this.pdf_width = value["width"].as_i64().unwrap_or(1000) as i32;
                    if this.page == "pdf" {
                        this.render_pdf();
                    }
                }
            }
        }
        this
    }
    fn chat_context_key(&self) -> String {
        use sha2::{Digest, Sha256};
        let material = match self.page.as_str() {
            "flashcard" | "discover" => self
                .current_card()
                .map(|c| format!("{}:{}", c.kind, c.id))
                .unwrap_or_default(),
            "listening" if self.audio_blind.enabled => format!(
                "blind:{}:{}:{}",
                self.current_audio,
                self.audio_blind_chinese,
                self.audio_blind
                    .revealed
                    .as_ref()
                    .map(|r| format!("{}:{}:{}", r.cue.start, r.cue.end, r.cue.text))
                    .unwrap_or_else(|| "hidden".into())
            ),
            "listening" => self.current_audio.clone(),
            "video" if self.video_material.is_some() => {
                self.video_material.as_ref().unwrap().key.clone()
            }
            "video" => self
                .current_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
            "pdf" => self
                .pdf_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
            "ocr" => self
                .ocr_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
            "speaking" => self
                .recorded
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
            _ => self.page.clone(),
        };
        hex::encode(Sha256::digest(
            format!("{}:{material}", self.page).as_bytes(),
        ))
    }
    fn sync_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let context = self.chat_context_key();
        if context != self.chat_context {
            if self.chat_running {
                if let Some(cancel) = &self.busy {
                    cancel.store(true, Ordering::SeqCst);
                }
            }
            if !self.chat_context.is_empty() {
                let _ = self.lib.set_setting(
                    &format!("chat_draft_{}", self.chat_context),
                    &self.value("question", cx),
                );
            }
            self.chat_context = context;
            self.chat_draft = self
                .lib
                .setting(&format!("chat_draft_{}", self.chat_context))
                .unwrap_or_default();
            self.set("question", self.chat_draft.clone(), window, cx);
            self.reasoning.clear();
            self.chat_id.clear();
            self.chat_history.clear();
            self.output.clear();
            if let Some(record) = documents::records(&self.lib, "chat")
                .unwrap_or_default()
                .into_iter()
                .rev()
                .find(|r| r["context"] == self.chat_context)
            {
                self.chat_id = record["id"].as_str().unwrap_or("").into();
                self.chat_history = record["messages"].as_array().cloned().unwrap_or_default();
                self.reasoning = self
                    .chat_history
                    .last()
                    .and_then(|m| m["reasoning_content"].as_str())
                    .unwrap_or("")
                    .into();
                self.output = self
                    .chat_history
                    .last()
                    .and_then(|m| m["content"].as_str())
                    .unwrap_or("")
                    .into();
            }
        }
    }
    fn value(&self, key: &str, cx: &App) -> String {
        self.inputs
            .get(key)
            .map(|input| input.read(cx).value().to_string())
            .unwrap_or_default()
    }
    fn set(
        &self,
        key: &str,
        value: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(input) = self.inputs.get(key) {
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
    }
    fn refresh(&mut self) {
        self.preloaded_index = None;
        self.practices = self.lib.list("listening_practices").unwrap_or_default();
        if self.practice_id.is_empty() {
            self.practice_id = self
                .practices
                .first()
                .map(|r| text(r, "id"))
                .unwrap_or_default();
        }
        self.audios = self.lib.list("listening_audios").unwrap_or_default();
        if !self
            .practices
            .iter()
            .any(|r| text(r, "id") == self.practice_id)
        {
            self.practice_id = self
                .practices
                .first()
                .map(|r| text(r, "id"))
                .unwrap_or_default();
        }
        self.checked_audio =
            audio_library::scoped_ids(&self.audios, &self.practice_id, &self.checked_audio)
                .into_iter()
                .collect();
        self.library.sync(&self.practice_id, &self.audios);
        self.cards = self
            .lib
            .cards(self.due && self.page == "flashcard")
            .unwrap_or_default();
        if self.page == "discover" {
            self.cards
                .retain(|c| c.kind != "video" && c.kind != "sentence");
            for row in self.audios.clone() {
                if let Ok(body) = fs::read_to_string(text(&row, "subtitle_uri")) {
                    let cues = subtitles::parse(&body);
                    let mut group: Vec<Cue> = vec![];
                    for cue in cues {
                        if !group.is_empty() && cue.end - group[0usize].start > 60. {
                            self.segment(&row, &group);
                            group.clear();
                        }
                        group.push(cue.clone());
                        if cue.end - group[0].start >= 20.
                            && cue.text.ends_with(['.', '!', '?', '。', '！', '？'])
                        {
                            self.segment(&row, &group);
                            group.clear();
                        }
                    }
                    self.segment(&row, &group);
                }
            }
        }
        if self.card_filter != "all" {
            self.cards.retain(|c| c.kind == self.card_filter);
        }
        if self.saved_only && self.page == "discover" {
            self.cards = self
                .lib
                .list("discover_favorites")
                .unwrap_or_default()
                .into_iter()
                .filter_map(|r| {
                    let key = text(&r, "key");
                    let (kind, card_id) = key.split_once(':')?;
                    let payload: Value = serde_json::from_str(&text(&r, "payload")).ok()?;
                    Some(Card {
                        kind: kind.into(),
                        id: card_id.into(),
                        front: payload["front"].as_str().unwrap_or("").into(),
                        back: payload["back"].as_str().unwrap_or("").into(),
                        media: payload["audio"]["uri"].as_str().unwrap_or("").into(),
                        start: payload["audio"]["startMs"].as_f64().unwrap_or(0.) / 1000.,
                        end: payload["audio"]["endMs"].as_f64().unwrap_or(0.) / 1000.,
                    })
                })
                .filter(|c| self.card_filter == "all" || self.card_filter == c.kind)
                .collect();
        }
        if self.index >= self.cards.len() {
            self.index = 0;
        }
        if let Some(row) = self.selected() {
            self.cues = fs::read_to_string(text(&row, "subtitle_uri"))
                .ok()
                .map(|body| subtitles::parse(&body))
                .unwrap_or_default();
            self.invalidate_blind();
            self.audio_scroll.reset(self.cues.len());
            self.audio_active = None;
        }
    }
    fn segment(&mut self, row: &Row, cues: &[Cue]) {
        if let (Some(a), Some(b)) = (cues.first(), cues.last()) {
            if b.end - a.start >= 3. {
                let key = format!("segment:{}:{}", text(row, "id"), a.start);
                if self.cards.iter().any(|c| c.kind == "clip" && c.id == key) {
                    return;
                }
                let (front, back) = subtitles::languages(cues);
                self.cards.push(Card {
                    kind: "clip".into(),
                    id: key,
                    front,
                    back,
                    media: text(row, "audio_uri"),
                    start: a.start,
                    end: b.end,
                });
            }
        }
    }
    fn begin(
        &mut self,
        label: &str,
        task: impl FnOnce(Library, Cancel, mpsc::Sender<Event>) -> Result<Outcome> + Send + 'static,
    ) {
        self.begin_scoped(TaskScope::Material, label, task);
    }
    fn begin_scoped(
        &mut self,
        scope: TaskScope,
        label: &str,
        task: impl FnOnce(Library, Cancel, mpsc::Sender<Event>) -> Result<Outcome> + Send + 'static,
    ) {
        if self.busy.is_some() {
            self.status = "已有任务正在执行，请完成或取消后重试".into();
            return;
        }
        self.task_scope = scope;
        let cancel = process::cancel_token();
        let token = cancel.clone();
        let lib = self.lib.clone();
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        self.busy = Some(cancel);
        self.status = label.into();
        std::thread::spawn(move || {
            let result = task(lib, token, tx.clone()).map_err(|e| format!("{e:#}"));
            let _ = tx.send(Event::Done(result));
        });
    }
    fn tick(&mut self, cx: &mut Context<Self>) {
        if !self.chat_context.is_empty() {
            let draft = self.value("question", cx);
            if draft != self.chat_draft {
                self.chat_draft = draft;
                let _ = self.lib.set_setting(
                    &format!("chat_draft_{}", self.chat_context),
                    &self.chat_draft,
                );
            }
        }
        if let Some(rx) = &self.waveform_rx {
            if let Ok(result) = rx.try_recv() {
                match result {
                    Ok(waveform) => self.waveform = Some(waveform),
                    Err(error) => self.status = format!("波形读取失败：{error}"),
                }
                self.waveform_rx = None;
                cx.notify();
            }
        }
        let old_pos = self.playback.position;
        let old_pause = self.playback.paused;
        let old_frame = self.playback.frame.clone();
        self.playback = self.player.snapshot();
        if self.page == "listening" {
            let active = video_subtitles::active(&self.cues, self.playback.position);
            if active != self.audio_active {
                self.audio_active = active;
                if self.audio_follow {
                    if let Some(i) = active {
                        self.audio_scroll.scroll_to_reveal_item(i);
                    }
                }
            }
        }
        if self.page == "video" {
            let active = video_subtitles::active(&self.video_cues, self.playback.position);
            if active != self.video_active {
                self.video_active = active;
                if self.video_follow {
                    if let Some(i) = active {
                        self.video_scroll.scroll_to_reveal_item(i);
                    }
                }
            }
        }
        let mut blind_changed = false;
        if self.audio_blind.enabled {
            if let Some((started, audio, target)) = self.audio_blind_pending.clone() {
                let valid = self.page == "listening"
                    && audio == self.current_audio
                    && self.busy.is_none()
                    && self
                        .current_path
                        .as_ref()
                        .is_some_and(|p| p.to_string_lossy() == self.playback.path)
                    && self.cues.iter().any(|c| {
                        c.start == target.cue.start
                            && c.end == target.cue.end
                            && c.text == target.cue.text
                    });
                if !valid
                    || started.elapsed() > Duration::from_secs(2)
                    || !self.playback.error.is_empty()
                {
                    self.invalidate_blind();
                    self.status = "未能确认暂停，请重试查看原文".into();
                    blind_changed = true;
                } else if self.playback.paused && started.elapsed() >= Duration::from_millis(80) {
                    if let Some(i) = self.cues.iter().position(|c| {
                        c.start == target.cue.start
                            && c.end == target.cue.end
                            && c.text == target.cue.text
                    }) {
                        self.audio_scroll.scroll_to_reveal_item(i);
                    }
                    self.audio_blind.revealed = Some(target);
                    self.audio_blind_pending = None;
                    self.status = "已暂停并显示所选字幕".into();
                    blind_changed = true;
                }
            } else if !self.playback.paused && self.audio_blind.revealed.is_some() {
                self.invalidate_blind();
                blind_changed = true;
            }
        }
        let frame_changed = match (&old_frame, &self.playback.frame) {
            (Some(a), Some(b)) => !std::sync::Arc::ptr_eq(a, b),
            (None, Some(_)) => true,
            _ => false,
        };
        let mut changed = blind_changed
            || frame_changed
            || old_pause != self.playback.paused
            || (old_pos - self.playback.position).abs() > 0.01;
        if !self.playback.paused && self.playback.duration > 0. && self.page != "meditation" {
            self.stats_ticks += 0.05;
            if self.stats_ticks >= 30. {
                if !self.current_audio.is_empty() {
                    let _ = self.lib.set_setting(
                        "last_listening_position",
                        &json!({"id":self.current_audio,"position":self.playback.position})
                            .to_string(),
                    );
                }
                let _ = self.lib.stat("listen_seconds", self.stats_ticks);
                self.stats_ticks = 0.;
            }
        }
        if let Some(rx) = &self.video_titles_rx {
            let titles = rx.try_iter().collect::<Vec<_>>();
            for (base, title) in titles {
                let result = (|| -> Result<()> {
                    let mut videos = video_library::Collection::load(&self.lib)?;
                    if videos.update_title(&base, &title)? {
                        videos.save(&self.lib)?;
                        self.videos = videos;
                        if let Some(series) = self.video_series.as_mut().filter(|s| s.base == base)
                        {
                            series.title = title;
                            // Only persist when the active queue belongs to this source.
                            series.save(&self.lib)?;
                        }
                    }
                    Ok(())
                })();
                if let Err(error) = result {
                    self.status = format!("更新视频标题失败：{error:#}");
                }
                changed = true;
            }
        }
        if let Some(rx) = &self.rx {
            let events = rx.try_iter().collect::<Vec<_>>();
            for event in events {
                changed = true;
                match event {
                    Event::ModelProgress(progress) => {
                        if self
                            .busy
                            .as_ref()
                            .is_some_and(|t| !t.load(Ordering::SeqCst))
                        {
                            self.status = progress.label();
                            self.model_progress = Some(progress);
                        }
                    }
                    Event::Chunk(chunk) => {
                        if self
                            .busy
                            .as_ref()
                            .is_some_and(|cancel| !cancel.load(Ordering::SeqCst))
                        {
                            self.output = chunk.answer;
                            self.reasoning = chunk.reasoning;
                        }
                    }
                    Event::Done(result) => {
                        let completed_chat = self.chat_running;
                        let cancelled =
                            self.busy.as_ref().is_some_and(|t| t.load(Ordering::SeqCst));
                        self.busy = None;
                        self.task_scope = TaskScope::Material;
                        self.model_progress = None;
                        self.chat_running = false;
                        match result {
                            Ok(Outcome::TtsReady(row)) => {
                                self.status =
                                    "音频及逐句字幕已保存到生成时选择的练习组，点击试听查看".into();
                                if text(&row, "practice_id") == self.practice_id {
                                    self.library.selected = text(&row, "id");
                                }
                                self.tts_last = Some(row);
                            }
                            Ok(Outcome::Parts(series)) if !cancelled => {
                                self.status =
                                    format!("已识别 {} P，点击列表选集播放", series.parts.len());
                                if let Err(e) = series.save(&self.lib) {
                                    self.status = format!("保存分P列表失败：{e}");
                                }
                                self.parts_scroll.scroll_to_item(
                                    series.current.unwrap_or(series.requested),
                                    ScrollStrategy::Center,
                                );
                                self.video_series = Some(series);
                                if self.page == "video" {
                                    self.video_tab = 1;
                                }
                            }
                            Ok(Outcome::PartVideo(mut series, index, path, new_file, origin)) => {
                                if cancelled {
                                    if new_file {
                                        if let Some(parent) = path.parent().filter(|p| {
                                            p.starts_with(self.lib.root.join("downloads"))
                                        }) {
                                            let _ = fs::remove_dir_all(parent);
                                        }
                                    }
                                    self.status = "已取消分P下载".into();
                                } else {
                                    series.parts[index].path = Some(path.clone());
                                    let activate =
                                        self.page == "video" && self.current_path == origin;
                                    if activate {
                                        series.current = Some(index);
                                    }
                                    let saved = (|| -> Result<()> {
                                        let mut videos =
                                            video_library::Collection::load(&self.lib)?;
                                        videos.add(&[path.clone()], Some(&series))?;
                                        videos.save(&self.lib)?;
                                        self.videos = videos;
                                        Ok(())
                                    })();
                                    if activate && saved.is_ok() {
                                        self.video_series = Some(series);
                                        // Opening media reloads subtitles; keep the panel the user is viewing.
                                        let panel = self.video_tab;
                                        if self.open(path) {
                                            self.video_tab = panel;
                                            self.player.set("pause", "no");
                                            self.status = format!("正在播放 P{}", index + 1);
                                        }
                                    } else {
                                        self.status = "分P已下载，返回视频页选集播放".into();
                                    }
                                    if let Err(e) = saved {
                                        self.status = format!("分P已导入，但保存列表失败：{e}");
                                    }
                                }
                            }
                            Ok(Outcome::VideoSubtitles(video, path, cues)) if !cancelled => {
                                if self.page == "video"
                                    && self.current_path.as_ref() == Some(&video)
                                {
                                    self.video_cues = cues;
                                    self.video_scroll.reset(self.video_cues.len());
                                    self.video_tab = 0;
                                    self.video_caption_source = VideoCaptionSource::Transcript;
                                    self.video_material = None;
                                    self.video_material_error.clear();
                                    self.video_active = None;
                                    self.page_scroll.set_offset(point(px(0.), px(0.)));
                                    self.player
                                        .command(["sub-add".into(), path, "select".into()]);
                                }
                                self.status = "视频字幕已生成并保存，播放时自动高亮".into();
                            }
                            Ok(Outcome::VideoTranslated(video, path, source, cues))
                                if !cancelled =>
                            {
                                let same_source = self.video_cues.len() == source.len()
                                    && self.video_cues.iter().zip(&source).all(|(a, b)| {
                                        a.start == b.start && a.end == b.end && a.text == b.text
                                    });
                                if self.page == "video"
                                    && self.current_path.as_ref() == Some(&video)
                                    && same_source
                                {
                                    self.video_cues = cues;
                                    self.video_scroll.reset(self.video_cues.len());
                                    self.video_active = None;
                                    self.video_caption_source = VideoCaptionSource::Transcript;
                                    self.video_material = None;
                                    self.video_material_error.clear();
                                    self.player
                                        .command(["sub-add".into(), path, "select".into()]);
                                    self.status = "中文字幕已生成并保存，英文和时间轴已保留".into();
                                } else {
                                    self.status = "翻译已完成，当前材料已改变，未切换显示".into();
                                }
                            }
                            Ok(Outcome::Output(s)) if !cancelled => {
                                self.output = s;
                                self.status = "已完成".into();
                            }
                            Ok(Outcome::KeyPoints(audio, points)) if !cancelled => {
                                let _ = self.lib.set_setting(
                                    &format!("key_points_{audio}"),
                                    &serde_json::to_string(&points).unwrap_or_default(),
                                );
                                if self.current_audio == audio {
                                    self.key_points = points;
                                }
                                self.status = "学习重点已标注，可悬停查看释义".into();
                            }
                            Ok(Outcome::Ocr(text)) if !cancelled => {
                                self.ocr_text = text.clone();
                                self.output = text;
                                self.status = "图片文字已识别".into();
                            }
                            Ok(Outcome::Pdf(page)) if !cancelled => {
                                self.document_text = page.text;
                                self.pdf_image = Some(page.image);
                                self.pdf_page = page.page;
                                self.pdf_pages = page.count;
                                self.pdf_bookmarks = page.bookmarks;
                                let _ = self.lib.set_setting(
                                    "pdf_current",
                                    &json!({"path":self.pdf_path,"page":self.pdf_page,"width":self.pdf_width}).to_string(),
                                );
                                self.status = "PDF 已载入".into();
                            }
                            Ok(Outcome::Backups(v)) => {
                                self.backups = v;
                                self.status = "已读取备份版本".into();
                            }
                            Ok(Outcome::Recording(path)) => {
                                self.recorded = Some(path);
                                self.status = "录音已保存，可转写评分".into();
                            }
                            Ok(Outcome::Message(s)) => self.status = s,
                            Err(e) => self.status = e,
                            _ => self.status = "已取消".into(),
                        }
                        // Chat only updates its persisted conversation. Refreshing materials
                        // here would invalidate the revealed cue and switch its chat context.
                        if !completed_chat {
                            self.refresh();
                        }
                        if !self.chat_id.is_empty() {
                            if let Some(record) = documents::records(&self.lib, "chat")
                                .unwrap_or_default()
                                .into_iter()
                                .find(|r| r["id"] == self.chat_id)
                            {
                                self.chat_history =
                                    record["messages"].as_array().cloned().unwrap_or_default();
                            }
                        }
                    }
                }
            }
        }
        if let (Some(a), Some(b)) = (self.loop_a, self.loop_b) {
            if self.repeat == 2 && !self.playback.paused && self.playback.position >= b {
                self.player
                    .command(["seek".into(), a.to_string(), "absolute+exact".into()]);
            }
        }
        if self.page != "video" || self.repeat != 0 {
            if self.playback.duration > 0. && self.playback.position >= self.playback.duration - 0.1
            {
                if !self.end_handled {
                    self.end_handled = true;
                    if self.repeat == 1 {
                        self.player.command(["seek", "0", "absolute"]);
                        self.player.set("pause", "no");
                    } else if self.repeat >= 3 {
                        let group = self
                            .audios
                            .iter()
                            .filter(|r| text(r, "practice_id") == self.practice_id)
                            .collect::<Vec<_>>();
                        if let Some(index) = group
                            .iter()
                            .position(|r| text(r, "id") == self.current_audio)
                        {
                            let next = if index + 1 < group.len() {
                                Some(group[index + 1].clone())
                            } else if self.repeat == 3 {
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
                self.end_handled = false;
            }
        }
        if self.page == "video" && self.repeat == 0 {
            if !self.playback.eof {
                self.end_handled = false;
            }
            if self.auto_next_part
                && self.playback.error.is_empty()
                && self.playback.eof
                && !self.end_handled
                && self.busy.is_none()
                && self
                    .current_path
                    .as_ref()
                    .is_some_and(|p| p.to_string_lossy() == self.playback.path)
            {
                self.end_handled = true;
                if let Some(next) = self
                    .video_series
                    .as_ref()
                    .and_then(|s| self.current_path.as_ref().and_then(|p| s.next_for(p)))
                {
                    self.start_part(next);
                }
            }
        }
        if let Some(end) = self.timer_end {
            if SystemTime::now() >= end {
                self.timer_end = None;
                if self.timer_focus {
                    let _ = self.lib.stat("pomodoro_count", 1.);
                }
                self.timer_focus = !self.timer_focus;
                self.timer_remaining = if self.timer_focus { 1500 } else { 300 };
                self.timer_end =
                    Some(SystemTime::now() + Duration::from_secs(self.timer_remaining));
                self.status = if self.timer_focus {
                    "休息结束，可以开始下一轮"
                } else {
                    "专注完成，休息一下"
                }
                .into();
                changed = true;
            } else if !self.timer_paused {
                let remaining = end
                    .duration_since(SystemTime::now())
                    .unwrap_or_default()
                    .as_secs();
                if remaining != self.timer_remaining {
                    self.timer_remaining = remaining;
                    changed = true;
                }
            }
        }
        if changed {
            cx.notify();
        }
    }
    fn start_part(&mut self, index: usize) {
        if self.busy.is_some() {
            self.status = "请先完成或取消当前任务".into();
            return;
        }
        let Some(series) = self.video_series.clone() else {
            return;
        };
        if index >= series.parts.len() {
            return;
        }
        let origin = self.current_path.clone();
        self.begin(
            &format!("正在准备 P{}…", index + 1),
            move |lib, cancel, _| {
                let cached = series.cached(index);
                let new_file = cached.is_none();
                let path = if let Some(path) = cached {
                    path
                } else {
                    process::download(&lib.root, &series.url(index)?, &cancel)?
                };
                Ok(Outcome::PartVideo(series, index, path, new_file, origin))
            },
        );
    }
    fn open(&mut self, path: PathBuf) -> bool {
        let result = (|| -> Result<_> {
            let mut videos = video_library::Collection::load(&self.lib)?;
            let ids = videos.add(&[path], self.video_series.as_ref())?;
            let video = videos.select(&ids[0])?;
            videos.save(&self.lib)?;
            self.videos = videos;
            Ok(video)
        })();
        let video = match result {
            Ok(video) => video,
            Err(e) => {
                self.status = format!("无法选择视频：{e:#}");
                return false;
            }
        };
        let path = video.path;
        self.video_series = video.series;
        self.video_remove = None;
        if let Some(cancel) = self.waveform_cancel.take() {
            cancel.store(true, Ordering::SeqCst);
        }
        self.waveform = None;
        self.waveform_rx = None;
        self.key_points.clear();
        if let Some(series) = &mut self.video_series {
            series.current = series
                .parts
                .iter()
                .position(|p| p.path.as_ref() == Some(&path));
            let _ = series.save(&self.lib);
            if let Some(index) = series.current {
                self.parts_scroll
                    .scroll_to_item(index, ScrollStrategy::Center);
            }
        }
        if self.repeat >= 2 {
            self.repeat = 0;
        }
        self.video_path = Some(path.clone());
        self.player.open(&path);
        self.current_path = Some(path.clone());
        self.loop_a = None;
        self.loop_b = None;
        self.cues.clear();
        self.current_audio.clear();
        self.load_video_subtitles(&path);
        if self.video_series.is_none() {
            let _ = self.lib.set_setting("bilibili_series", "");
        }
        true
    }
    fn prepare_video_material(&mut self, scope: video_qa::Scope) -> Result<()> {
        let path = self.current_path.as_ref().context("请先打开视频")?;
        match video_qa::build(
            path,
            &self.video_cues,
            scope,
            self.playback.position,
            &self.playback.subtitle,
        ) {
            Ok(material) => {
                self.video_material = Some(material);
                self.video_material_error.clear();
                Ok(())
            }
            Err(error) => {
                self.video_material = None;
                self.video_material_error = error.to_string();
                Err(error)
            }
        }
    }
    fn load_video_subtitles(&mut self, path: &std::path::Path) {
        self.video_caption_source = VideoCaptionSource::Transcript;
        self.video_material = None;
        self.video_material_error.clear();
        self.video_cues = match video_subtitles::load(&self.lib, path) {
            Ok(cues) => cues,
            Err(e) => {
                self.status = format!("字幕载入失败：{e}");
                vec![]
            }
        };
        self.video_active = None;
        self.video_scroll.reset(self.video_cues.len());
        self.video_tab = 0;
    }

    fn open_audio(&mut self, row: &Row) {
        self.invalidate_blind();
        self.audio_blind_settle_until = Some(Instant::now() + Duration::from_millis(250));
        let path = PathBuf::from(text(row, "audio_uri"));
        self.player.open(&path);
        self.current_path = Some(path);
        self.current_audio = text(row, "id");
        let _ = self
            .lib
            .set_setting("last_listening_audio", &self.current_audio);
        self.load_waveform();
        self.key_points = serde_json::from_str(
            &self
                .lib
                .setting(&format!("key_points_{}", self.current_audio))
                .unwrap_or_default(),
        )
        .unwrap_or_default();
        self.practice_id = text(row, "practice_id");
        self.cues = fs::read_to_string(text(&row, "subtitle_uri"))
            .ok()
            .map(|s| subtitles::parse(&s))
            .unwrap_or_default();
        self.invalidate_blind();
        self.audio_scroll.reset(self.cues.len());
        self.audio_active = None;
        self.audio_clip_mode = false;
        self.audio_tools_open = false;
        if self.repeat == 2 {
            self.repeat = 0;
        }
        self.loop_a = None;
        self.loop_b = None;
    }
    fn restore_listening(&mut self) {
        let last = self.lib.setting("last_listening_audio").unwrap_or_default();
        if let Some(row) = self.audios.iter().find(|r| text(r, "id") == last).cloned() {
            self.open_audio(&row);
            self.player.set("pause", "yes");
            if let Ok(value) = serde_json::from_str::<Value>(
                &self
                    .lib
                    .setting("last_listening_position")
                    .unwrap_or_default(),
            ) {
                if value["id"] == last {
                    if let Some(time) = value["position"].as_f64() {
                        self.player.command([
                            "seek".into(),
                            time.to_string(),
                            "absolute+exact".into(),
                        ]);
                    }
                }
            }
        }
    }
    fn save_media_position(&self) {
        if self.page == "listening" && !self.current_audio.is_empty() {
            let _ = self.lib.set_setting(
                "last_listening_position",
                &json!({"id":self.current_audio,"position":self.playback.position}).to_string(),
            );
        } else if self.page == "video" {
            if let Some(path) = &self.video_path {
                let _ = self.lib.set_setting(
                    "last_video_position",
                    &json!({"path":path,"position":self.playback.position}).to_string(),
                );
            }
        }
    }
    fn load_waveform(&mut self) {
        if let Some(cancel) = self.waveform_cancel.take() {
            cancel.store(true, Ordering::SeqCst);
        }
        self.waveform = None;
        self.waveform_rx = None;
        if let Some(path) = self.current_path.clone() {
            let lib = self.lib.clone();
            let cancel = process::cancel_token();
            self.waveform_cancel = Some(cancel.clone());
            let (tx, rx) = mpsc::channel();
            self.waveform_rx = Some(rx);
            std::thread::spawn(move || {
                let _ =
                    tx.send(waveform::extract(&lib, &path, &cancel).map_err(|e| format!("{e:#}")));
            });
        }
    }
    fn selected(&self) -> Option<Row> {
        self.audios
            .iter()
            .find(|r| text(r, "id") == self.current_audio)
            .cloned()
    }
    fn current_card(&self) -> Option<Card> {
        self.cards.get(self.index).cloned()
    }
    fn play_card(&mut self) {
        self.player.set("volume", self.volume);
        self.player.set("speed", self.rate);
        self.player.set("pause", "yes");
        if let Some(card) = self.current_card() {
            if !card.media.is_empty() {
                if self.preloaded_index == Some(self.index) {
                    std::mem::swap(&mut self.player, &mut self.preloader);
                    self.player.set("pause", "no");
                } else {
                    self.player
                        .open_range(&PathBuf::from(card.media), card.start, card.end, false);
                }
            }
        }
        self.preloaded_index = None;
        if self.page == "discover" && !self.cards.is_empty() {
            let due = self
                .lib
                .cards(true)
                .unwrap_or_default()
                .iter()
                .map(|c| format!("{}:{}", c.kind, c.id))
                .collect();
            let favorites = self
                .lib
                .list("discover_favorites")
                .unwrap_or_default()
                .iter()
                .map(|r| text(r, "key"))
                .collect();
            if let Some(index) = clarora_gpui::discover::next(
                &self.cards,
                &self.history,
                self.index,
                &due,
                &favorites,
            ) {
                self.preloaded_index = Some(index);
                let next = &self.cards[index];
                if !next.media.is_empty() {
                    self.preloader.open_range(
                        &PathBuf::from(&next.media),
                        next.start,
                        next.end,
                        true,
                    );
                } else {
                    self.preloader.set("pause", "yes");
                }
            }
        }
    }
    fn pick(&self, kind: &str, cx: &mut Context<Self>) {
        let kind = kind.to_string();
        let directory = kind == "directory";
        let audio_group = self.practice_id.clone();
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: !directory,
            directories: directory,
            multiple: kind == "audio" || kind == "merge" || kind == "video",
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picker.await {
                let _ = this.update(cx, |this, cx| {
                    if kind == "audio" && this.practice_id != audio_group {
                        this.status = "选择文件期间练习组已切换，请在目标组重新导入".into();
                    } else {
                        this.picked(&kind, paths);
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }
    fn picked(&mut self, kind: &str, paths: Vec<PathBuf>) {
        let Some(path) = paths.first().cloned() else {
            return;
        };
        match kind {
            "video" => {
                let result = (|| -> Result<()> {
                    ensure!(self.busy.is_none(), "请完成或取消当前任务后再导入视频");
                    let mut videos = video_library::Collection::load(&self.lib)?;
                    let ids = videos.add(&paths, None)?;
                    videos.save(&self.lib)?;
                    let first = videos.select(&ids[0])?;
                    self.videos = videos;
                    self.save_media_position();
                    ensure!(self.open(first.path), "{}", self.status);
                    self.video_tab = 3;
                    self.video_tools_open = false;
                    self.status = format!("已收藏 {} 个视频，可在“我的视频”切换或移除", ids.len());
                    Ok(())
                })();
                if let Err(e) = result {
                    self.status = format!("视频导入失败：{e:#}");
                }
            }
            "video-subtitle" => {
                let result = (|| -> Result<()> {
                    let video = self.current_path.as_ref().context("请先打开视频")?;
                    let mut cues = subtitles::parse(&fs::read_to_string(&path)?);
                    ensure!(!cues.is_empty(), "请选择带时间轴的 UTF-8 SRT / VTT 字幕");
                    cues.sort_by(|a, b| a.start.total_cmp(&b.start));
                    video_subtitles::save(&self.lib, video, &cues, &process::cancel_token())?;
                    self.player.command([
                        "sub-add".into(),
                        path.to_string_lossy().into_owned(),
                        "select".into(),
                    ]);
                    self.load_video_subtitles(video.clone().as_path());
                    Ok(())
                })();
                self.status = match result {
                    Ok(()) => "已加载并缓存字幕，可同步高亮及向 AI 提问".into(),
                    Err(error) => error.to_string(),
                };
            }
            "words" => self.begin("正在导入词条…", move |lib, _, _| {
                Ok(Outcome::Message(format!(
                    "已导入 {} 条",
                    lib.import_words(&fs::read_to_string(path)?, "manual", false)?
                )))
            }),
            "directory" => self.begin("正在导入目录…", move |lib, _, _| {
                Ok(Outcome::Message(format!(
                    "已导入 {} 条",
                    lib.import_directory(&path)?
                )))
            }),
            "anki" => self.begin("正在读取 Anki…", move |lib, _, _| {
                Ok(Outcome::Message(format!(
                    "已导入 {} 张卡片",
                    documents::import_anki(&lib, &path)?
                )))
            }),
            "audio" => {
                let group = self.practice_id.clone();
                self.begin("正在导入音频…", move |lib, _, _| {
                    ensure!(!group.is_empty(), "请先创建练习组");
                    let mut errors = vec![];
                    let mut count = 0;
                    for p in paths {
                        match lib.import_audio(&group, &p) {
                            Ok(_) => count += 1,
                            Err(e) => errors.push(e.to_string()),
                        }
                    }
                    Ok(Outcome::Message(format!(
                        "已导入 {count} 条。{}",
                        errors.join("；")
                    )))
                });
            }
            "subtitle-en" | "subtitle-zh" => {
                let zh = kind.ends_with("zh");
                let row = self.selected();
                self.begin("正在合并字幕…", move |lib, _, _| {
                    let row = row.context("请先选择音频")?;
                    let new = subtitles::parse(&fs::read_to_string(&path)?);
                    let old = fs::read_to_string(text(&row, "subtitle_uri"))
                        .ok()
                        .map(|s| subtitles::parse(&s))
                        .unwrap_or_default();
                    let merged = subtitles::merge(&old, &new, zh)?;
                    let target = lib.root.join("media").join(format!("{}.srt", id()));
                    fs::write(&target, subtitles::srt(&merged))?;
                    lib.db()?.execute(
                        "UPDATE listening_audios SET subtitle_uri=? WHERE id=?",
                        rusqlite::params![target.to_string_lossy(), text(&row, "id")],
                    )?;
                    Ok(Outcome::Message("字幕已导入".into()))
                });
            }
            "ocr" => {
                self.ocr_path = Some(path.clone());
                self.begin("正在识别图片…", move |lib, cancel, _| {
                    Ok(Outcome::Ocr(ai::ocr(&lib, &path, &cancel)?))
                });
            }
            "pdf" => {
                if self.busy.is_some() {
                    self.status = "已有任务正在执行，请完成或取消后重试".into();
                    return;
                }
                let lib = self.lib.clone();
                match lib.copy_media(&path, "pdf") {
                    Ok(p) => {
                        self.pdf_path = Some(p);
                        self.pdf_page = 0;
                        self.render_pdf();
                    }
                    Err(e) => self.status = e.to_string(),
                }
            }
            "rn-library" => self.begin("正在读取原版资料快照…", move |lib, _, _| {
                Ok(Outcome::Message(format!(
                    "已从快照合并 {} 条，原库保持只读",
                    backup::import_sqlite(&lib, &path)?
                )))
            }),
            "restore" => self.begin("正在合并资料备份…", move |lib, _, _| {
                Ok(Outcome::Message(format!(
                    "已合并 {} 条资料",
                    backup::import_local(&lib, &path)?
                )))
            }),
            "recording" => {
                self.recorded = Some(path);
                self.status = "已选择录音".into();
            }
            "background" => {
                self.player.open(&path);
                self.player.set("loop-file", "inf");
            }
            _ => {}
        }
    }
    fn render_pdf(&mut self) {
        if self.busy.is_some() {
            self.status = "已有任务正在执行，请完成或取消后重试".into();
            return;
        }
        if let Some(path) = self.pdf_path.clone() {
            self.pdf_image = None;
            self.document_text.clear();
            self.pdf_bookmarks.clear();
            let _ = self.lib.set_setting(
                "pdf_current",
                &json!({"path":path,"page":self.pdf_page,"width":self.pdf_width}).to_string(),
            );
            let page = self.pdf_page;
            let width = self.pdf_width;
            self.begin("正在渲染 PDF…", move |lib, _, _| {
                Ok(Outcome::Pdf(documents::load_pdf_page(
                    &lib, &path, page, width,
                )?))
            });
        }
    }
    fn handle(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        // Modal confirmation owns interaction until it is confirmed or dismissed.
        if window.has_active_dialog(cx) && !matches!(action, Action::VideoConfirmRemove) {
            return;
        }
        let result = (|| -> Result<()> {
            match action {
                Action::Composer => {
                    self.composer_open = !self.composer_open;
                    if self.composer_open {
                        self.page_scroll.scroll_to_bottom();
                    }
                }
                Action::GraphSelect(node) => self.graph_node = node,
                Action::GraphList(list) => self.graph_list = list,
                Action::Page(page) => {
                    self.audio_blind.enabled = false;
                    self.invalidate_blind();
                    let previous = self.page.clone();
                    self.video_remove = None;
                    self.library.pending_delete.clear();
                    if self.library.tool == "delete" {
                        self.library.tool.clear();
                    }
                    self.save_media_position();
                    if self.task_scope == TaskScope::Material {
                        if let Some(cancel) = &self.busy {
                            cancel.store(true, Ordering::SeqCst);
                        }
                    }
                    self.player.set("pause", "yes");
                    self.preloader.set("pause", "yes");
                    self.preloaded_index = None;
                    self.page = page;
                    if self.page == "library" {
                        let group = self.lib.setting("library_group")?;
                        if self.practices.iter().any(|r| text(r, "id") == group) {
                            self.practice_id = group;
                        }
                    }
                    self.page_scroll.set_offset(point(px(0.), px(0.)));
                    if self.page == "discover" {
                        self.card_filter = "all".into();
                    }
                    self.focus.focus(window);
                    self.revealed = false;
                    self.index = 0;
                    self.history.clear();
                    self.lib.set_setting("last_page", &self.page)?;
                    self.refresh();
                    if self.page == "discover" || self.page == "flashcard" {
                        self.play_card();
                    }
                    if self.page != previous {
                        if self.page == "listening" {
                            self.restore_listening();
                        } else if self.page == "video" {
                            if let Some(path) = self.video_path.clone() {
                                self.open(path.clone());
                                self.video_tab = 3;
                                self.player.set("pause", "yes");
                                if let Ok(value) = serde_json::from_str::<Value>(
                                    &self.lib.setting("last_video_position").unwrap_or_default(),
                                ) {
                                    if value["path"].as_str() == path.to_str() {
                                        if let Some(time) = value["position"].as_f64() {
                                            self.player.command([
                                                "seek".into(),
                                                time.to_string(),
                                                "absolute+exact".into(),
                                            ]);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if self.page == "pdf"
                        && self.pdf_image.is_none()
                        && self.pdf_path.is_some()
                        && self.busy.is_none()
                    {
                        self.render_pdf();
                    }
                }
                Action::Refresh => {
                    self.refresh();
                    if let Some(row) = self.selected() {
                        self.cues = fs::read_to_string(text(&row, "subtitle_uri"))
                            .ok()
                            .map(|s| subtitles::parse(&s))
                            .unwrap_or_default();
                        self.invalidate_blind();
                        self.audio_scroll.reset(self.cues.len());
                        self.audio_active = None;
                    }
                }
                Action::Pick(kind) => {
                    if kind == "backup" {
                        let picker =
                            cx.prompt_for_new_path(&self.lib.root, Some("clarora-library.zip"));
                        cx.spawn(async move |this, cx| {
                            if let Ok(Ok(Some(path))) = picker.await {
                                let _ = this.update(cx, |this, cx| {
                                    this.begin("正在备份资料…", move |lib, _, _| {
                                        backup::export_local(&lib, &path)?;
                                        Ok(Outcome::Message("已导出资料与附件".into()))
                                    });
                                    cx.notify();
                                });
                            }
                        })
                        .detach();
                    } else {
                        self.pick(&kind, cx);
                    }
                }
                Action::Audio(key) => {
                    let row = self
                        .audios
                        .iter()
                        .find(|r| text(r, "id") == key)
                        .cloned()
                        .context("音频不存在")?;
                    self.open_audio(&row);
                    self.page = "listening".into();
                    self.lib.set_setting("last_page", "listening")?;
                }
                Action::Practice(key) => {
                    ensure!(
                        self.practices.iter().any(|r| text(r, "id") == key),
                        "练习组不存在"
                    );
                    self.lib.set_setting("library_group", &key)?;
                    self.practice_id = key;
                    self.checked_audio.clear();
                    self.library.selected.clear();
                    self.library.filter = 0;
                    self.library.tool.clear();
                    self.library.pending_delete.clear();
                    self.set("library_search", "", window, cx);
                    self.library.sync(&self.practice_id, &self.audios);
                }
                Action::AudioPractice(key) => {
                    self.invalidate_blind();
                    ensure!(self.busy.is_none(), "请等待当前任务完成");
                    self.practice_id = key;
                    if let Some(row) = self
                        .audios
                        .iter()
                        .find(|r| text(r, "practice_id") == self.practice_id)
                        .cloned()
                    {
                        self.open_audio(&row);
                    } else {
                        self.player.set("pause", "yes");
                        self.current_audio.clear();
                        self.current_path = None;
                        self.cues.clear();
                        self.audio_scroll.reset(0);
                        self.audio_active = None;
                        self.waveform = None;
                        self.key_points.clear();
                        if let Some(cancel) = self.waveform_cancel.take() {
                            cancel.store(true, Ordering::SeqCst);
                        }
                        self.waveform_rx = None;
                        self.lib.set_setting("last_listening_audio", "")?;
                        if self.repeat == 2 {
                            self.repeat = 0;
                        }
                        self.loop_a = None;
                        self.loop_b = None;
                    }
                }
                Action::AudioBlind(enabled) => {
                    ensure!(self.page == "listening", "请先进入音频学习");
                    ensure!(self.busy.is_none(), "请等待当前任务完成");
                    self.audio_blind.enabled = enabled;
                    self.invalidate_blind();
                    self.assistant_open = false;
                    self.output.clear();
                    self.reasoning.clear();
                    if enabled {
                        if self.loop_a.is_none() || self.loop_b.is_none() {
                            self.loop_a = None;
                            self.loop_b = None;
                            if self.repeat == 2 {
                                self.repeat = 0;
                            }
                        }
                        self.audio_tools_open = false;
                        self.status = "盲听已开启，字幕已隐藏".into();
                    } else {
                        self.player.set("pause", "yes");
                        self.status = "已切回字幕模式，保持暂停".into();
                    }
                    self.sync_chat(window, cx);
                    window.focus(&self.focus);
                }
                Action::AudioBlindChinese(enabled) => {
                    ensure!(
                        self.page == "listening" && self.audio_blind.enabled,
                        "请先开启盲听"
                    );
                    self.lib.set_setting(
                        "blind_chinese_hint",
                        if enabled { "true" } else { "false" },
                    )?;
                    self.audio_blind_chinese = enabled;
                    self.invalidate_blind();
                    self.assistant_open = false;
                    self.sync_chat(window, cx);
                    window.focus(&self.focus);
                    self.status = if enabled {
                        "查看时仅显示中文；平时中英文均遮挡"
                    } else {
                        "查看时显示中英字幕；平时均遮挡"
                    }
                    .into();
                }
                action @ (Action::AudioBlindReveal | Action::AudioBlindRevealCue(..)) => {
                    ensure!(
                        self.page == "listening" && self.audio_blind.enabled,
                        "请先开启盲听"
                    );
                    ensure!(
                        self.busy.is_none() && self.audio_blind_pending.is_none(),
                        "请等待当前操作完成"
                    );
                    ensure!(
                        self.audio_blind_settle_until
                            .is_none_or(|t| Instant::now() >= t),
                        "正在定位，请稍候重试"
                    );
                    let playback = self.player.snapshot();
                    ensure!(
                        !self.current_audio.is_empty()
                            && self
                                .current_path
                                .as_ref()
                                .is_some_and(|p| p.to_string_lossy() == playback.path),
                        "音频正在切换，请稍候"
                    );
                    let target = if let Action::AudioBlindRevealCue(audio_id, cue) = action {
                        ensure!(audio_id == self.current_audio, "音频已切换，请重新选择");
                        blind_listening::row_target(&self.cues, &cue, playback.duration)
                            .context("字幕已变化或时间无效，请重新选择")?;
                        blind_listening::Revealed { cue, sparse: false }
                    } else {
                        blind_listening::resolve(&self.cues, playback.position, playback.duration)
                            .context("播放到有字幕的部分后再查看")?
                    };
                    self.invalidate_blind();
                    self.assistant_open = false;
                    self.sync_chat(window, cx);
                    self.audio_blind_pending =
                        Some((Instant::now(), self.current_audio.clone(), target));
                    self.player.set("pause", "yes");
                    self.status = "正在暂停，确认后显示所选字幕…".into();
                }
                Action::AudioBlindHide => {
                    self.invalidate_blind();
                    self.sync_chat(window, cx);
                    window.focus(&self.focus);
                }
                Action::AudioBlindResume(restart) => {
                    ensure!(
                        self.page == "listening"
                            && self.audio_blind.enabled
                            && !self.current_audio.is_empty(),
                        "请先选择音频并开启盲听"
                    );
                    self.invalidate_blind();
                    self.assistant_open = false;
                    if restart {
                        self.player.command(["seek", "0", "absolute+exact"]);
                    }
                    self.player.set("pause", "no");
                    self.status = "正在盲听，字幕已隐藏".into();
                    self.audio_blind_settle_until =
                        Some(Instant::now() + Duration::from_millis(200));
                    self.sync_chat(window, cx);
                    window.focus(&self.focus);
                }
                Action::AudioBlindRow(audio_id, cue, word_index) => {
                    ensure!(
                        self.page == "listening" && self.audio_blind.enabled,
                        "请先开启盲听"
                    );
                    ensure!(self.busy.is_none(), "请等待当前任务完成");
                    let playback = self.player.snapshot();
                    ensure!(
                        audio_id == self.current_audio
                            && !audio_id.is_empty()
                            && self
                                .current_path
                                .as_ref()
                                .is_some_and(|p| p.to_string_lossy() == playback.path),
                        "音频正在切换，请重试"
                    );
                    let start = match word_index {
                        Some(index) => {
                            blind_listening::word_target(&self.cues, &cue, index, playback.duration)
                        }
                        None => blind_listening::row_target(&self.cues, &cue, playback.duration),
                    }
                    .context("字幕已变化或时间无效，请重新选择")?;
                    self.invalidate_blind();
                    self.assistant_open = false;
                    let outside_loop = self.repeat == 2
                        && self
                            .loop_a
                            .zip(self.loop_b)
                            .is_some_and(|(a, b)| start < a || start >= b);
                    if outside_loop {
                        self.repeat = 0;
                        self.loop_a = None;
                        self.loop_b = None;
                    }
                    self.player.command([
                        "seek".into(),
                        start.to_string(),
                        "absolute+exact".into(),
                    ]);
                    self.player.set("pause", "no");
                    self.audio_blind_settle_until =
                        Some(Instant::now() + Duration::from_millis(250));
                    self.status = if outside_loop {
                        "已退出 A–B 循环，从所选句开始盲听"
                    } else {
                        "已从所选位置开始盲听，字幕保持遮挡"
                    }
                    .into();
                    self.sync_chat(window, cx);
                    window.focus(&self.focus);
                }
                Action::AudioBlindSentence(direction) => {
                    ensure!(
                        self.page == "listening" && self.audio_blind.enabled,
                        "请先开启盲听"
                    );
                    ensure!(
                        self.busy.is_none() && self.audio_blind_pending.is_none(),
                        "请等待当前操作完成"
                    );
                    ensure!(
                        self.audio_blind_settle_until
                            .is_none_or(|t| Instant::now() >= t),
                        "正在定位，请稍候重试"
                    );
                    let playback = self.player.snapshot();
                    ensure!(
                        !self.current_audio.is_empty()
                            && self
                                .current_path
                                .as_ref()
                                .is_some_and(|p| p.to_string_lossy() == playback.path),
                        "音频正在切换，请稍候"
                    );
                    let target = blind_listening::sentence_target(
                        &self.cues,
                        playback.position,
                        playback.duration,
                        self.audio_blind.revealed.as_ref(),
                        direction,
                        self.loop_a
                            .zip(self.loop_b)
                            .filter(|(a, b)| self.repeat == 2 && b > a),
                    )
                    .context("当前范围内没有可播放的字幕句")?;
                    self.invalidate_blind();
                    self.assistant_open = false;
                    self.player.command([
                        "seek".into(),
                        target.cue.start.to_string(),
                        "absolute+exact".into(),
                    ]);
                    self.player.set("pause", "no");
                    self.audio_blind_settle_until =
                        Some(Instant::now() + Duration::from_millis(250));
                    self.status = match direction {
                        -1 => "已从上一句开始盲听",
                        1 => "已从下一句开始盲听",
                        _ => "已隐藏文字，从句首重听",
                    }
                    .into();
                    self.sync_chat(window, cx);
                    window.focus(&self.focus);
                }
                Action::AudioClipMode => {
                    ensure!(!self.audio_blind.enabled, "请先切回字幕模式");
                    self.audio_clip_mode = !self.audio_clip_mode;
                    if !self.audio_clip_mode {
                        self.loop_a = None;
                        self.loop_b = None;
                        if self.repeat == 2 {
                            self.repeat = 0;
                        }
                    }
                }
                Action::AudioTools => {
                    ensure!(!self.audio_blind.enabled, "请先切回字幕模式");
                    self.audio_tools_open = !self.audio_tools_open;
                }
                Action::AudioCurrent => self.audio_show_current = !self.audio_show_current,
                Action::AudioFollow => {
                    self.audio_follow = !self.audio_follow;
                    if self.audio_follow {
                        if let Some(i) = self.audio_active {
                            self.audio_scroll.scroll_to_reveal_item(i);
                        }
                    }
                }
                Action::AudioFont(size) => {
                    ensure!([14, 16, 18, 20, 24, 28, 32].contains(&size), "无效字幕字号");
                    self.audio_font_size = size;
                    self.lib
                        .set_setting("subtitle_font_size", &size.to_string())?;
                    self.invalidate_blind();
                    self.audio_scroll.reset(self.cues.len());
                }
                Action::AudioRepeat(mode) => {
                    ensure!(mode <= 4, "无效循环方式");
                    if mode == 2 {
                        ensure!(
                            self.loop_a.zip(self.loop_b).is_some_and(|(a, b)| b > a),
                            "先截取有效片段"
                        );
                    }
                    self.repeat = mode;
                }
                Action::LibrarySelect(key) => {
                    ensure!(
                        self.audios
                            .iter()
                            .any(|r| text(r, "id") == key
                                && text(r, "practice_id") == self.practice_id),
                        "音频已不存在"
                    );
                    self.library.selected = key;
                    self.library.tool = "detail".into();
                    self.page_scroll.set_offset(point(px(0.), px(0.)));
                    self.library.pending_delete.clear();
                    self.library.sync(&self.practice_id, &self.audios);
                }
                Action::LibraryTool(tool) => {
                    self.library.pending_delete.clear();
                    let name = match tool.as_str() {
                        "group-rename" => self
                            .practices
                            .iter()
                            .find(|r| text(r, "id") == self.practice_id)
                            .map(|r| text(r, "name"))
                            .unwrap_or_default(),
                        "audio-rename" => self
                            .audios
                            .iter()
                            .find(|r| text(r, "id") == self.library.selected)
                            .map(|r| text(r, "name"))
                            .unwrap_or_default(),
                        _ => String::new(),
                    };
                    self.set("library_name", name, window, cx);
                    self.library.tool = tool;
                    self.page_scroll.set_offset(point(px(0.), px(0.)));
                    let field = if self.library.tool == "tts" {
                        Some("tts_text")
                    } else if ["group-new", "group-rename", "audio-rename"]
                        .contains(&self.library.tool.as_str())
                    {
                        Some("library_name")
                    } else {
                        None
                    };
                    if let Some(field) = field {
                        self.inputs[field].update(cx, |state, cx| state.focus(window, cx));
                    }
                }
                Action::LibrarySaveName => {
                    ensure!(self.busy.is_none(), "请等待当前任务完成");
                    let name = self.value("library_name", cx);
                    match self.library.tool.as_str() {
                        "group-new" => {
                            ensure!(
                                !name.trim().is_empty() && name.chars().count() <= 120,
                                "名称需为 1–120 字"
                            );
                            ensure!(
                                !self.practices.iter().any(|r| text(r, "name")
                                    .trim()
                                    .eq_ignore_ascii_case(name.trim())),
                                "已有同名练习组，请直接选择或换一个名称"
                            );
                            self.practice_id = self.lib.practice(name.trim())?;
                            self.checked_audio.clear();
                            self.library.selected.clear();
                            self.library.filter = 0;
                            self.set("library_search", "", window, cx);
                        }
                        "group-rename" => audio_library::rename(
                            &self.lib,
                            "listening_practices",
                            &self.practice_id,
                            &name,
                        )?,
                        "audio-rename" => audio_library::rename(
                            &self.lib,
                            "listening_audios",
                            &self.library.selected,
                            &name,
                        )?,
                        _ => anyhow::bail!("请重新打开名称编辑"),
                    }
                    self.lib.set_setting("library_group", &self.practice_id)?;
                    self.library.tool.clear();
                    self.refresh();
                    self.status = "名称已保存".into();
                }
                Action::LibraryFilter(filter) => {
                    self.library.filter = filter;
                }
                Action::LibrarySort(sort) => {
                    self.library.sort = sort;
                }
                Action::LibraryBatch => {
                    self.library.batch = !self.library.batch;
                    self.checked_audio.clear();
                    self.library.pending_delete.clear();
                }
                Action::LibraryReset => {
                    self.library.filter = 0;
                    self.set("library_search", "", window, cx);
                }
                Action::LibrarySelectAll => {
                    let ids = self
                        .library
                        .visible(
                            &self.audios,
                            &self.practice_id,
                            &self.value("library_search", cx),
                        )
                        .into_iter()
                        .map(|r| text(r, "id"))
                        .collect::<Vec<_>>();
                    if !ids.is_empty() && ids.iter().all(|id| self.checked_audio.contains(id)) {
                        for id in ids {
                            self.checked_audio.remove(&id);
                        }
                    } else {
                        self.checked_audio.extend(ids);
                    }
                    self.library.pending_delete.clear();
                }
                Action::LibraryMove(destination) => {
                    ensure!(self.busy.is_none(), "请等待当前任务完成");
                    let ids = audio_library::scoped_ids(
                        &self.audios,
                        &self.practice_id,
                        &self.checked_audio,
                    );
                    audio_library::move_records(&self.lib, &self.practice_id, &destination, &ids)?;
                    self.checked_audio.clear();
                    self.library.tool.clear();
                    self.refresh();
                    self.status = format!("已移动 {} 条音频", ids.len());
                }
                Action::LibraryDelete(ids) => {
                    ensure!(self.busy.is_none(), "请等待当前任务完成");
                    self.library.pending_delete = ids;
                    self.library.tool = "delete".into();
                }
                Action::LibraryConfirmDelete => {
                    ensure!(self.busy.is_none(), "请等待当前任务完成");
                    let ids = self.library.pending_delete.clone();
                    audio_library::delete_records(&self.lib, &self.practice_id, &ids)?;
                    self.checked_audio.clear();
                    self.library.pending_delete.clear();
                    self.library.tool.clear();
                    self.refresh();
                    self.status = format!("已移除 {} 条记录，附件文件保留", ids.len());
                }
                Action::LibraryStudy(paused) => {
                    let key = self.library.selected.clone();
                    ensure!(
                        self.audios
                            .iter()
                            .any(|r| text(r, "id") == key
                                && text(r, "practice_id") == self.practice_id),
                        "请先选择音频"
                    );
                    self.audio_blind.enabled = false;
                    self.handle(Action::Audio(key), window, cx);
                    if paused {
                        self.player.set("pause", "yes");
                    }
                }
                Action::Saved(saved) => {
                    self.saved_only = saved;
                    self.refresh();
                }
                Action::ReviewMode(due) => {
                    self.due = due;
                    self.refresh();
                    self.revealed = false;
                }
                Action::Filter(filter) => {
                    self.card_filter = filter;
                    self.refresh();
                }
                Action::Flip => self.revealed = !self.revealed,
                Action::Next => {
                    if !self.cards.is_empty() {
                        self.history.push(self.index);
                        if self.page == "discover" {
                            self.index = if let Some(index) = self.preloaded_index {
                                index
                            } else {
                                let due = self
                                    .lib
                                    .cards(true)?
                                    .iter()
                                    .map(|c| format!("{}:{}", c.kind, c.id))
                                    .collect();
                                let favorites = self
                                    .lib
                                    .list("discover_favorites")?
                                    .iter()
                                    .map(|r| text(r, "key"))
                                    .collect();
                                clarora_gpui::discover::next(
                                    &self.cards,
                                    &self.history,
                                    self.index,
                                    &due,
                                    &favorites,
                                )
                                .unwrap_or(0)
                            };
                        } else {
                            self.index = (self.index + 1) % self.cards.len();
                        }
                        self.revealed = false;
                        self.play_card();
                    }
                }
                Action::Previous => {
                    self.index = self.history.pop().unwrap_or(self.index.saturating_sub(1));
                    self.revealed = false;
                    self.play_card();
                }
                Action::Grade(grade) => {
                    let card = self.current_card().context("没有待复习卡片")?;
                    ensure!(self.revealed, "请先翻面再评分");
                    let due = self.lib.grade(&card, &grade)?;
                    self.status = format!("下次复习：{due}");
                    self.cards.remove(self.index);
                    if grade == "again" {
                        self.cards.push(card);
                    }
                    if self.index >= self.cards.len() {
                        self.index = 0;
                    }
                    self.revealed = false;
                    self.play_card();
                }
                Action::EditCard => {
                    self.page_scroll.scroll_to_bottom();
                    let card = self.current_card().context("没有可编辑的卡片")?;
                    self.set("front", card.front.clone(), window, cx);
                    self.set("back", card.back.clone(), window, cx);
                    self.editing = Some(card);
                    self.status = "在下方编辑原文与翻译，然后保存修改".into();
                }
                Action::SaveEditedCard => {
                    let card = self.editing.as_ref().context("请先选择编辑卡片")?;
                    self.lib.update_card(
                        card,
                        &self.value("front", cx),
                        &self.value("back", cx),
                    )?;
                    self.editing = None;
                    self.refresh();
                    self.status = "卡片已更新，复习进度保留".into();
                }
                Action::SaveSentence => {
                    documents::save_sentence(
                        &self.lib,
                        &self.value("front", cx),
                        &self.value("back", cx),
                    )?;
                    self.refresh();
                    self.status = "句子卡已保存".into();
                }
                Action::Play => {
                    if self.page == "listening" {
                        self.invalidate_blind();
                    }
                    self.player
                        .set("pause", if self.playback.paused { "no" } else { "yes" });
                }
                Action::Seek(delta) => {
                    if self.page == "listening" {
                        self.invalidate_blind();
                        self.audio_blind_settle_until =
                            Some(Instant::now() + Duration::from_millis(200));
                    }
                    self.player
                        .command(["seek".into(), delta.to_string(), "relative+exact".into()])
                }
                Action::Word(start, end, word, extend) => {
                    ensure!(
                        !(self.page == "listening" && self.audio_blind.enabled),
                        "请先切回字幕模式"
                    );
                    if !self.audio_clip_mode {
                        self.player.command([
                            "seek".into(),
                            start.to_string(),
                            "absolute+exact".into(),
                        ]);
                        self.player.set("pause", "no");
                    } else {
                        let start = if extend {
                            self.loop_a.unwrap_or(start).min(start)
                        } else {
                            start
                        };
                        let end = if extend {
                            self.loop_b.unwrap_or(end).max(end)
                        } else {
                            end
                        };
                        self.loop_a = Some(start);
                        self.loop_b = Some(end);
                        self.repeat = 2;
                        let (front, back) = subtitles::excerpt(&self.cues, start, end);
                        self.set(
                            "front",
                            if front.is_empty() { word } else { front },
                            window,
                            cx,
                        );
                        self.set("back", back, window, cx);
                        self.player.command([
                            "seek".into(),
                            start.to_string(),
                            "absolute+exact".into(),
                        ]);
                        self.player.set("pause", "no");
                        self.status =
                            "已选中词语，可调整 A/B 边界、保存片段或询问 AI；时间按字幕长度估算"
                                .into();
                    }
                }
                Action::Boundary(time, start) => {
                    let time = time.clamp(0., self.playback.duration.max(time));
                    if start {
                        self.loop_a =
                            Some(time.min(self.loop_b.unwrap_or(f64::MAX) - 0.02).max(0.));
                    } else {
                        self.loop_b = Some(time.max(self.loop_a.unwrap_or(0.) + 0.02));
                    }
                    if let (Some(a), Some(b)) = (self.loop_a, self.loop_b) {
                        self.repeat = 2;
                        if self.page == "listening" {
                            let (front, back) = subtitles::excerpt(&self.cues, a, b);
                            self.set("front", front, window, cx);
                            self.set("back", back, window, cx);
                        }
                    }
                }
                Action::SeekTo(time) => {
                    if self.page == "listening" {
                        self.invalidate_blind();
                        self.audio_blind_settle_until =
                            Some(Instant::now() + Duration::from_millis(200));
                    }
                    self.player
                        .command(["seek".into(), time.to_string(), "absolute+exact".into()])
                }
                Action::Navigation => {
                    let open = !self.navigation_open;
                    self.lib
                        .set_setting("navigation_open", if open { "true" } else { "false" })?;
                    let from = self.navigation_progress();
                    self.navigation_open = open;
                    self.navigation_motion = Some((Instant::now(), from));
                }
                Action::Assistant if self.page == "video" => {
                    self.handle(
                        Action::VideoPanel(if self.video_tab == 2 { 0 } else { 2 }),
                        window,
                        cx,
                    );
                }
                Action::Assistant => {
                    self.assistant_open = !self.assistant_open;
                    if self.assistant_open {
                        if self.page == "video" && self.video_material.is_none() {
                            let _ = self.prepare_video_material(video_qa::Scope::All);
                        }
                        self.sync_chat(window, cx);
                        self.inputs["question"].update(cx, |state, cx| state.focus(window, cx));
                    }
                }
                Action::Score => {
                    let path = self.recorded.clone().context("请先录音或选择录音文件")?;
                    let reference = self.value("reference", cx);
                    ensure!(!reference.trim().is_empty(), "请填写参考原文");
                    self.begin("正在转写并对齐录音…", move |lib, cancel, _| {
                        let cues = ai::transcribe(&lib, &path, &cancel)?;
                        let actual = cues
                            .iter()
                            .map(|c| c.text.clone())
                            .collect::<Vec<_>>()
                            .join(" ");
                        let mut score = ai::score(&reference, &actual)?;
                        let duration = cues.last().map(|c| c.end).unwrap_or(0.);
                        let wpm = if duration > 0. { (score["words"].as_f64().unwrap_or(0.) * 60. / duration).round() } else { 0. };
                        let feedback = ai::chat(&lib, "用简短中文给出跟读建议。只能依据识别文本评价，不推断发音音质。", &score.to_string(), &[], &cancel, |_| {}).unwrap_or_else(|_| "已完成识别文本对齐；未取得 AI 点评。".into());
                        ensure!(!cancel.load(Ordering::SeqCst), "已取消");
                        score["attemptId"] = json!(id());
                        score["kind"] = json!("shadow");
                        score["createdAt"] = json!(now());
                        score["status"] = json!("completed");
                        score["feedback"] = json!(feedback);
                        score["score"] = json!({"accuracy":(score["accuracy"].as_f64().unwrap_or(0.)*100.).round(),"completeness":(score["completeness"].as_f64().unwrap_or(0.)*100.).round(),"wpm":wpm,"missed":score["missed"],"wrong":score["wrong"],"extra":score["extra"]});
                        ai::save_record(&lib, "speaking", score.clone())?;
                        Ok(Outcome::Output(format!(
                            "文本对齐准确度：{:.0}% · 完整度：{:.0}% · 语速：{wpm:.0} WPM\n\n识别：{actual}\n\n漏读：{}\n\n读错：{}\n\n多读：{}\n\n{feedback}",
                            score["accuracy"].as_f64().unwrap_or(0.) * 100.,
                            score["completeness"].as_f64().unwrap_or(0.) * 100.,
                            score["missed"], score["wrong"], score["extra"]
                        )))
                    });
                }
                Action::Rate(rate) => {
                    self.rate = rate;
                    self.player.set("speed", rate);
                }
                Action::Volume(volume) => {
                    self.volume = volume.clamp(0., 100.);
                    self.player.set("volume", self.volume);
                }
                Action::VideoFullscreen => {
                    ensure!(
                        self.page == "video" && self.current_path.is_some(),
                        "请先打开视频"
                    );
                    if self.video_fullscreen {
                        self.leave_video_fullscreen(window, cx);
                    } else {
                        self.video_fullscreen_owned = !window.is_fullscreen();
                        self.video_fullscreen_seen = window.is_fullscreen();
                        self.video_fullscreen = true;
                        window.focus(&self.focus);
                        if self.video_fullscreen_owned {
                            window.toggle_fullscreen();
                        }
                    }
                }
                Action::VideoExitFullscreen => self.leave_video_fullscreen(window, cx),
                Action::VideoCaptions => {
                    ensure!(self.page == "video", "请先进入视频学习");
                    let enabled = !self.video_captions_enabled;
                    self.lib.set_setting(
                        "video_captions_enabled",
                        if enabled { "true" } else { "false" },
                    )?;
                    self.video_captions_enabled = enabled;
                    if enabled && self.video_caption_source == VideoCaptionSource::Hidden {
                        self.video_caption_source = if self.video_cues.is_empty() {
                            VideoCaptionSource::Track
                        } else {
                            VideoCaptionSource::Transcript
                        };
                        self.player.set("sid", "auto");
                    }
                    self.player.set_subtitles_visible(enabled);
                    self.status = if enabled {
                        "画面字幕已开启"
                    } else {
                        "画面字幕已关闭，字幕列表仍保留"
                    }
                    .into();
                }
                Action::VideoTranslate => {
                    ensure!(self.page == "video", "请先进入视频学习");
                    ensure!(self.busy.is_none(), "请等待当前任务完成");
                    let video = self.current_path.clone().context("请先打开视频")?;
                    video_subtitles::english_source(&self.video_cues)?;
                    let source = self.video_cues.clone();
                    self.begin(
                        "正在根据英文字幕分批生成中文…",
                        move |lib, cancel, _| {
                            let (path, cues) =
                                video_subtitles::translate(&lib, &video, &source, &cancel)?;
                            Ok(Outcome::VideoTranslated(video, path, source, cues))
                        },
                    );
                }
                Action::VideoSubtitleSize(size) => {
                    ensure!([14, 16, 20, 24, 28, 32].contains(&size), "无效字幕字号");
                    self.lib
                        .set_setting("video_subtitle_size", &size.to_string())?;
                    self.video_subtitle_size = size;
                }
                Action::VideoPrecision => self.video_precision = !self.video_precision,
                Action::VideoClearLoop => {
                    self.loop_a = None;
                    self.loop_b = None;
                    if self.repeat == 2 {
                        self.repeat = 0;
                    }
                }
                Action::LoopMode(mode) => {
                    ensure!(mode <= 2, "循环模式无效");
                    if mode == 2 {
                        ensure!(
                            self.loop_a.zip(self.loop_b).is_some_and(|(a, b)| b > a),
                            "请先设置有效的 A/B 范围"
                        );
                    }
                    self.repeat = mode;
                }
                Action::VideoRepeatCue => {
                    let i = video_subtitles::active(&self.video_cues, self.playback.position)
                        .context("当前没有带时间轴的字幕")?;
                    let cue = &self.video_cues[i];
                    self.loop_a = Some(cue.start);
                    self.loop_b = Some(cue.end);
                    self.repeat = 2;
                    self.player.command([
                        "seek".into(),
                        cue.start.to_string(),
                        "absolute+exact".into(),
                    ]);
                    self.player.set("pause", "no");
                }
                Action::VideoPlayCue(index) => {
                    ensure!(
                        self.page == "video" && self.current_path.is_some(),
                        "请先打开视频"
                    );
                    let cue = self.video_cues.get(index).context("字幕行不存在")?;
                    let start = cue.start;
                    ensure!(start.is_finite() && start >= 0., "字幕时间无效");
                    // A previous sentence loop must not pull playback back to its B point.
                    if self.repeat == 2 {
                        self.repeat = 0;
                    }
                    self.player.command([
                        "seek".into(),
                        start.to_string(),
                        "absolute+exact".into(),
                    ]);
                    self.player.set("pause", "no");
                }
                Action::VideoSentence(next) => {
                    let pos = self.playback.position;
                    let target = if next {
                        self.video_cues.iter().find(|c| c.start > pos + 0.1)
                    } else {
                        self.video_cues.iter().rev().find(|c| c.start < pos - 0.5)
                    }
                    .context("没有更多字幕")?
                    .start;
                    // Moving to another sentence exits the previous A/B loop.
                    if self.repeat == 2 {
                        self.repeat = 0;
                    }
                    self.player.command([
                        "seek".into(),
                        target.to_string(),
                        "absolute+exact".into(),
                    ]);
                }
                Action::Mute => {
                    if self.volume > 0. {
                        self.volume_before_mute = self.volume;
                        self.volume = 0.;
                    } else {
                        self.volume = self.volume_before_mute.max(1.);
                    }
                    self.player.set("volume", self.volume);
                }
                Action::Mark(start) => {
                    if self.page == "video" || self.page == "listening" {
                        ensure!(
                            self.current_path.is_some() && self.playback.duration > 0.,
                            "请先打开可播放的媒体"
                        );
                        if start {
                            self.loop_a = Some(self.playback.position);
                            if self.loop_b.is_some_and(|b| b <= self.playback.position) {
                                self.loop_b = None;
                                if self.repeat == 2 {
                                    self.repeat = 0;
                                }
                            }
                        } else {
                            let a = self.loop_a.context("请先设置 A 点")?;
                            ensure!(self.playback.position > a, "B 点必须晚于 A 点");
                            self.loop_b = Some(self.playback.position);
                        }
                        if self.loop_a.zip(self.loop_b).is_some_and(|(a, b)| b > a) {
                            self.repeat = 2;
                        }
                    } else if start {
                        self.loop_a = Some(self.playback.position);
                    } else {
                        self.loop_b = Some(self.playback.position);
                    }
                    if let (Some(a), Some(b)) = (self.loop_a, self.loop_b) {
                        ensure!(b > a, "B 点必须晚于 A 点");
                        self.repeat = 2;
                        if self.page == "listening" {
                            let (front, back) = subtitles::excerpt(&self.cues, a, b);
                            self.set("front", front, window, cx);
                            self.set("back", back, window, cx);
                        }
                    }
                }
                Action::Repeat => {
                    self.repeat = (self.repeat + 1) % if self.page == "video" { 3 } else { 5 };
                }
                Action::Subtitle(key, secondary) => {
                    self.player.set(
                        if secondary { "secondary-sid" } else { "sid" },
                        if key == 0 {
                            "no".into()
                        } else {
                            key.to_string()
                        },
                    );
                    if !secondary {
                        self.video_caption_source = if key == 0 {
                            VideoCaptionSource::Hidden
                        } else {
                            VideoCaptionSource::Track
                        };
                    }
                }
                Action::SubtitleImport(zh) => {
                    self.pick(if zh { "subtitle-zh" } else { "subtitle-en" }, cx)
                }
                Action::ReadParts => {
                    let link = self.value("bilibili", cx);
                    let previous = self.video_series.clone();
                    self.begin(
                        "正在读取 bilibili 分P列表…",
                        move |_, cancel, _| {
                            let mut series = bilibili::fetch(&link, &cancel)?;
                            if let Some(previous) = previous.filter(|p| p.base == series.base) {
                                for part in &mut series.parts {
                                    part.path = previous
                                        .parts
                                        .iter()
                                        .find(|p| p.number == part.number)
                                        .and_then(|p| p.path.clone());
                                }
                                series.current =
                                    previous.current.filter(|i| *i < series.parts.len());
                            }
                            Ok(Outcome::Parts(series))
                        },
                    );
                }
                Action::SelectPart(index) => self.start_part(index),
                Action::AutoNextPart => {
                    let enabled = !self.auto_next_part;
                    self.lib
                        .set_setting("video_auto_next", if enabled { "true" } else { "false" })?;
                    self.auto_next_part = enabled;
                    self.end_handled = false;
                }
                Action::Download => {
                    let link = self.value("bilibili", cx);
                    let previous = self.video_series.clone();
                    let origin = self.current_path.clone();
                    self.begin(
                        "正在识别分P并导入所选集…",
                        move |lib, cancel, _| {
                            let mut series = bilibili::fetch(&link, &cancel)?;
                            if let Some(previous) = previous.filter(|p| p.base == series.base) {
                                for part in &mut series.parts {
                                    part.path = previous
                                        .parts
                                        .iter()
                                        .find(|p| p.number == part.number)
                                        .and_then(|p| p.path.clone());
                                }
                            }
                            let index = series.requested;
                            let cached = series.cached(index);
                            let new_file = cached.is_none();
                            let path = if let Some(path) = cached {
                                path
                            } else {
                                process::download(&lib.root, &series.url(index)?, &cancel)?
                            };
                            Ok(Outcome::PartVideo(series, index, path, new_file, origin))
                        },
                    );
                }
                Action::Cancel => {
                    if let Some(cancel) = &self.busy {
                        cancel.store(true, Ordering::SeqCst);
                        self.status = "正在取消…".into();
                    }
                }
                Action::Clip => {
                    ensure!(
                        !(self.page == "listening" && self.audio_blind.enabled),
                        "请先切回字幕模式再保存片段"
                    );
                    let path = self.current_path.clone().context("请先打开媒体")?;
                    let a = self.loop_a.context("请先设置 A 点")?;
                    let b = self.loop_b.context("请先设置 B 点")?;
                    let front = self.value("front", cx);
                    let back = self.value("back", cx);
                    let is_audio = self.page == "listening";
                    self.begin("正在导出片段…",move|lib,cancel,_|{let output=lib.root.join("media").join(format!("{}.mp4",id()));process::clip(&path,&output,a,b,&cancel)?;let result=if is_audio{lib.put("clip_cards",json!({"id":id(),"en_text":front,"zh_text":back,"audio_uri":output.to_string_lossy(),"start_ms":0,"end_ms":((b-a)*1000.) as u64,"created_at":now()}))}else{lib.put("video_clips",json!({"id":id(),"en_text":front,"zh_text":back,"video_uri":output.to_string_lossy(),"created_at":now()}))};if let Err(e)=result{let _=fs::remove_file(output);return Err(e)}Ok(Outcome::Message("片段已存为闪卡".into()))});
                }
                Action::ToggleReasoning => self.reasoning_open = !self.reasoning_open,
                Action::ChatSession(session) => {
                    if self.chat_running {
                        if let Some(cancel) = &self.busy {
                            cancel.store(true, Ordering::SeqCst);
                        }
                    }
                    self.sync_chat(window, cx);
                    self.chat_id = session.clone();
                    if session.is_empty() {
                        self.set("question", String::new(), window, cx);
                    }
                    self.inputs["question"].update(cx, |state, cx| state.focus(window, cx));
                    self.chat_history.clear();
                    self.output.clear();
                    self.reasoning.clear();
                    if !session.is_empty() {
                        if let Some(record) = documents::records(&self.lib, "chat")?
                            .into_iter()
                            .find(|r| r["id"] == session && r["context"] == self.chat_context)
                        {
                            self.chat_history =
                                record["messages"].as_array().cloned().unwrap_or_default();
                            self.reasoning = self
                                .chat_history
                                .last()
                                .and_then(|m| m["reasoning_content"].as_str())
                                .unwrap_or("")
                                .into();
                            self.output = self
                                .chat_history
                                .last()
                                .and_then(|m| m["content"].as_str())
                                .unwrap_or("")
                                .into();
                        }
                    }
                }
                Action::PdfOpen(path) => {
                    ensure!(self.busy.is_none(), "已有任务正在执行，请完成或取消后重试");
                    self.pdf_path = Some(path);
                    self.pdf_page = 0;
                    self.render_pdf();
                }
                Action::ClearHighlights => {
                    self.key_points.clear();
                    self.lib
                        .set_setting(&format!("key_points_{}", self.current_audio), "[]")?;
                }
                Action::Highlight => {
                    ensure!(
                        !(self.page == "listening" && self.audio_blind.enabled),
                        "请先切回字幕模式"
                    );
                    let audio = self.current_audio.clone();
                    let cues = self.cues.clone();
                    ensure!(
                        !audio.is_empty() && !cues.is_empty(),
                        "请先选择音频并加载字幕"
                    );
                    self.begin("正在标注学习重点…", move |lib, cancel, _| {
                        Ok(Outcome::KeyPoints(
                            audio,
                            ai::key_points(&lib, &cues, &cancel)?,
                        ))
                    });
                }
                Action::Chat => {
                    ensure!(self.busy.is_none(), "已有任务正在执行，请完成或取消后重试");
                    self.sync_chat(window, cx);
                    if self.chat_id.is_empty() {
                        self.chat_id = id();
                    }
                    let session_id = self.chat_id.clone();
                    let context = self.chat_context.clone();
                    let question = self.value("question", cx);
                    let passage = self.reference(cx);
                    let video_material = if self.page == "video" {
                        Some(self.video_material.clone().context(
                            if self.video_material_error.is_empty() {
                                "请先生成字幕并选择提问范围".to_string()
                            } else {
                                self.video_material_error.clone()
                            },
                        )?)
                    } else {
                        None
                    };
                    let history = self.chat_history.clone();
                    ensure!(!question.trim().is_empty(), "请输入问题");
                    self.output.clear();
                    self.reasoning.clear();
                    self.chat_running = true;
                    self.begin("AI 正在回答…", move |lib, cancel, tx| {
                        let mut notify=|chunk|{let _=tx.send(Event::Chunk(chunk));};
                        let reply=if let Some(material)=video_material {
                            video_qa::ask(&lib,&question,&material,&history,&cancel,&mut notify)?
                        }else{ai::chat_stream(&lib,&question,&passage,&history,&cancel,&mut notify)?};
                        ensure!(!cancel.load(Ordering::SeqCst), "已取消");
                        let mut next = history;
                        next.push(json!({"role":"user","content":question,"reference":passage}));
                        next.push(json!({"role":"assistant","content":reply.answer,"reasoning_content":reply.reasoning}));
                        if next.len() > 60 {
                            next.drain(..next.len() - 60);
                        }
                        ai::save_chat(&lib, &session_id, &context, &question, &next)?;
                        Ok(Outcome::Output(reply.answer))
                    });
                }
                Action::SaveAnswer => {
                    ensure!(self.busy.is_none(), "请等待 AI 回答完成");
                    let answer = self
                        .chat_history
                        .last()
                        .filter(|m| m["role"] == "assistant")
                        .and_then(|m| m["content"].as_str())
                        .context("请先完成一次 AI 回答")?;
                    ensure!(
                        !answer.trim().is_empty() && answer == self.output,
                        "当前回答未完整保存，请完成回答后再存卡"
                    );
                    let question = self
                        .chat_history
                        .iter()
                        .rev()
                        .find(|m| m["role"] == "user")
                        .and_then(|m| m["content"].as_str())
                        .context("会话缺少问题")?;
                    let reference = self
                        .chat_history
                        .iter()
                        .rev()
                        .find(|m| m["role"] == "user")
                        .and_then(|m| m["reference"].as_str())
                        .map(str::to_owned)
                        .unwrap_or_else(|| self.reference(cx));
                    self.lib.put("ai_cards",json!({"id":id(),"question":question,"answer":answer,"context_text":reference,"created_at":now()}))?;
                    self.status = "已存为 AI 问答卡".into();
                }
                Action::Favorite => {
                    let card = self.current_card().context("当前没有内容")?;
                    let key = format!("{}:{}", card.kind, card.id);
                    let exists = self.lib.rows(
                        "SELECT key FROM discover_favorites WHERE key=?",
                        &[key.clone().into()],
                    )?;
                    if exists.is_empty() {
                        self.lib.put("discover_favorites",json!({"key":key,"payload":json!({"key":key,"kind":card.kind,"front":card.front,"back":card.back,"source":"GPUI","audio":if card.media.is_empty(){Value::Null}else{json!({"uri":card.media,"startMs":card.start*1000.,"endMs":card.end*1000.})}}).to_string()}))?;
                        if card.id.starts_with("segment:") {
                            let count=self.lib.rows("SELECT id FROM clip_cards WHERE audio_uri=? AND start_ms=? AND end_ms=?",&[card.media.clone().into(),(card.start*1000.).into(),(card.end*1000.).into()])?;
                            if count.is_empty() {
                                self.lib.put("clip_cards",json!({"id":card.id,"en_text":card.front,"zh_text":card.back,"audio_uri":card.media,"start_ms":card.start*1000.,"end_ms":card.end*1000.,"created_at":now()}))?;
                            }
                        }
                        self.status = "已收藏".into();
                    } else {
                        self.lib.delete("discover_favorites", &key)?;
                        self.status = "已取消收藏".into();
                    }
                }
                Action::VideoAsk(scope) => {
                    ensure!(self.page == "video", "请先进入视频学习");
                    ensure!(!self.chat_running, "请等待当前回答完成或取消后切换引用");
                    self.prepare_video_material(scope)?;
                    if scope != video_qa::Scope::All {
                        self.player.set("pause", "yes");
                    }
                    self.video_tab = 2;
                    self.sync_chat(window, cx);
                    self.inputs["question"].update(cx, |state, cx| state.focus(window, cx));
                    self.status = "已引用字幕；输入问题后发送给设置中的聊天模型".into();
                }
                Action::VideoTranscribe => {
                    ensure!(self.page == "video", "请先进入视频学习");
                    let video = self.current_path.clone().context("请先打开或导入视频")?;
                    self.begin(
                        "正在提取音轨并转写字幕，长视频可能需要几分钟…",
                        move |lib, cancel, _| {
                            let (path, cues) = video_subtitles::generate(&lib, &video, &cancel)?;
                            Ok(Outcome::VideoSubtitles(video, path, cues))
                        },
                    );
                }
                Action::VideoTools => self.video_tools_open = !self.video_tools_open,
                Action::VideoSelect(key) => {
                    ensure!(self.busy.is_none(), "请完成或取消当前任务后再切换视频");
                    let mut videos = video_library::Collection::load(&self.lib)?;
                    let video = videos.select(&key)?;
                    videos.save(&self.lib)?;
                    self.videos = videos;
                    self.save_media_position();
                    if !self.open(video.path) {
                        return Ok(());
                    }
                    self.video_tab = 3;
                    self.video_tools_open = false;
                    self.status = format!("已选择：{}", video.title);
                }
                Action::VideoEpisodes(key) => {
                    ensure!(self.busy.is_none(), "请完成或取消当前任务后再切换视频");
                    if self.videos.selected.as_ref() != Some(&key) || self.video_path.is_none() {
                        let mut videos = video_library::Collection::load(&self.lib)?;
                        let video = videos.select(&key)?;
                        videos.save(&self.lib)?;
                        self.save_media_position();
                        self.videos = videos;
                        if !self.open(video.path) {
                            return Ok(());
                        }
                        self.player.command(["set", "pause", "yes"]);
                    }
                    self.video_tab = 1;
                    self.video_tools_open = false;
                }
                Action::VideoRemove(key) => {
                    ensure!(self.busy.is_none(), "请等待当前任务完成");
                    let title = self
                        .videos
                        .items
                        .iter()
                        .find(|v| v.id == key)
                        .context("该视频已移出收藏")?
                        .title
                        .clone();
                    self.video_remove = Some(key);
                    let view = cx.entity().downgrade();
                    window.open_dialog(cx, move |dialog, window, cx| {
                        let confirm_view = view.clone();
                        let close_view = view.clone();
                        dialog
                            .title("移除视频收藏？")
                            .confirm()
                            .w((window.viewport_size().width - px(32.)).min(px(480.)))
                            .margin_top(
                                ((window.viewport_size().height - px(280.)) / 2.).max(px(16.)),
                            )
                            .button_props(
                                DialogButtonProps::default()
                                    .cancel_text("取消")
                                    .ok_text("确认移除")
                                    .ok_variant(ButtonVariant::Danger),
                            )
                            .child(div().text_sm().child(title.clone()))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("视频文件和字幕缓存会保留。移除当前视频会停止播放。"),
                            )
                            .on_ok(move |_, window, cx| {
                                confirm_view
                                    .update(cx, |this, cx| {
                                        this.handle(Action::VideoConfirmRemove, window, cx);
                                        this.video_remove.is_none()
                                    })
                                    .unwrap_or(true)
                            })
                            .on_close(move |_, _, cx| {
                                let _ = close_view.update(cx, |this, cx| {
                                    this.video_remove = None;
                                    cx.notify();
                                });
                            })
                    });
                }
                Action::VideoConfirmRemove => {
                    ensure!(self.busy.is_none(), "请等待当前任务完成");
                    let key = self
                        .video_remove
                        .clone()
                        .context("请重新选择要移除的视频")?;
                    let mut videos = video_library::Collection::load(&self.lib)?;
                    let video = videos.remove(&key)?;
                    videos.save(&self.lib)?;
                    self.videos = videos;
                    self.video_remove = None;
                    if self
                        .video_path
                        .as_ref()
                        .is_some_and(|path| video.contains(path))
                    {
                        self.leave_video_fullscreen(window, cx);
                        self.player.command(["stop"]);
                        self.playback = Playback::default();
                        self.current_path = None;
                        self.video_path = None;
                        self.video_series = None;
                        self.video_cues.clear();
                        self.video_scroll.reset(0);
                        self.video_active = None;
                        self.video_material = None;
                        self.video_material_error.clear();
                        self.loop_a = None;
                        self.loop_b = None;
                        self.repeat = 0;
                        self.end_handled = true;
                        self.output.clear();
                        self.reasoning.clear();
                        self.lib.set_setting("bilibili_series", "")?;
                        self.lib.set_setting("last_video_position", "")?;
                    }
                    self.video_tab = 3;
                    self.status = format!("已移除「{}」收藏，视频和字幕文件保留", video.title);
                }
                Action::VideoPanel(tab) => {
                    self.video_remove = None;
                    ensure!(tab < 4, "无效学习面板");
                    self.video_tab = tab;
                    if tab == 2 {
                        if self.video_material.is_none() {
                            let _ = self.prepare_video_material(video_qa::Scope::All);
                        }
                        self.sync_chat(window, cx);
                        self.inputs["question"].update(cx, |state, cx| state.focus(window, cx));
                    } else {
                        window.focus(&self.focus);
                    }
                }
                Action::VideoFollow => {
                    self.video_follow = !self.video_follow;
                    if self.video_follow {
                        if let Some(i) = self.video_active {
                            self.video_scroll.scroll_to_reveal_item(i);
                        }
                    }
                }
                Action::AudioCopySubtitle(audio_id, cue) => {
                    ensure!(
                        self.page == "listening"
                            && !audio_id.is_empty()
                            && audio_id == self.current_audio,
                        "音频已切换，请重新复制"
                    );
                    let body = blind_listening::copy_text(
                        &self.cues,
                        cue.as_ref(),
                        &self.audio_blind,
                        self.audio_blind_chinese,
                    )
                    .context("暂无可复制的字幕；盲听时请先翻开对应句子")?;
                    cx.write_to_clipboard(ClipboardItem::new_string(body));
                    self.status = if cue.is_some() {
                        "已复制这句字幕"
                    } else {
                        "已复制全部字幕（不含时间戳）"
                    }
                    .into();
                }
                Action::VideoCopySubtitle(index) => {
                    let body = match index {
                        Some(index) => self
                            .video_cues
                            .get(index)
                            .context("字幕已改变，请重试")?
                            .text
                            .clone(),
                        None => self
                            .video_cues
                            .iter()
                            .map(|cue| cue.text.as_str())
                            .collect::<Vec<_>>()
                            .join("\n"),
                    };
                    ensure!(!body.trim().is_empty(), "没有可复制的字幕");
                    cx.write_to_clipboard(ClipboardItem::new_string(body));
                    self.status = match index {
                        Some(index) => format!("已复制第 {} 句字幕", index + 1),
                        None => format!("已复制全部 {} 句字幕", self.video_cues.len()),
                    };
                }
                Action::VideoSubtitleExport => {
                    ensure!(!self.video_cues.is_empty(), "请先生成字幕");
                    let body = subtitles::srt(&self.video_cues);
                    let picker =
                        cx.prompt_for_new_path(&self.lib.root, Some("video-subtitles.srt"));
                    cx.spawn(async move |this, cx| {
                        if let Ok(Ok(Some(path))) = picker.await {
                            let result = fs::write(path, body);
                            let _ = this.update(cx, |this, cx| {
                                this.status = match result {
                                    Ok(()) => "SRT 字幕已导出".into(),
                                    Err(e) => format!("导出失败：{e}"),
                                };
                                cx.notify();
                            });
                        }
                    })
                    .detach();
                }
                Action::Transcribe => {
                    let row = self.selected().context("请先选择音频")?;
                    self.begin("正在生成字幕…", move |lib, cancel, _| {
                        let path = ai::write_transcription(
                            &lib,
                            &PathBuf::from(text(&row, "audio_uri")),
                            &cancel,
                        )?;
                        lib.db()?.execute(
                            "UPDATE listening_audios SET subtitle_uri=? WHERE id=?",
                            rusqlite::params![path, text(&row, "id")],
                        )?;
                        Ok(Outcome::Message("字幕已生成，点击刷新载入".into()))
                    });
                }
                Action::Translate => {
                    let row = self.selected().context("请先选择音频")?;
                    let cues = self.cues.clone();
                    ensure!(!cues.is_empty(), "请先导入或生成原文字幕");
                    self.begin("正在翻译字幕…", move |lib, cancel, _| {
                        let translated = ai::translate(&lib, &cues, &cancel)?;
                        let path = lib.root.join("media").join(format!("{}.srt", id()));
                        fs::write(&path, subtitles::srt(&translated))?;
                        lib.db()?.execute(
                            "UPDATE listening_audios SET subtitle_uri=? WHERE id=?",
                            rusqlite::params![path.to_string_lossy(), text(&row, "id")],
                        )?;
                        Ok(Outcome::Message("双语字幕已生成，点击刷新载入".into()))
                    });
                }
                Action::Record => {
                    ensure!(self.recording.is_none(), "正在录音");
                    self.player.set("pause", "yes");
                    let path = self
                        .lib
                        .root
                        .join("recordings")
                        .join(format!("{}.wav", id()));
                    self.recording = Some(Recorder::start(path));
                    self.status = "正在录音，完成后点击停止".into();
                }
                Action::StopRecord => {
                    let recorder = self.recording.take().context("没有正在进行的录音")?;
                    self.begin("正在保存录音…", move |_, _, _| {
                        Ok(Outcome::Recording(recorder.finish()?))
                    });
                }
                Action::Timer(seconds) => {
                    self.timer_remaining = seconds;
                    self.timer_end = Some(SystemTime::now() + Duration::from_secs(seconds));
                    self.timer_paused = false;
                    self.timer_focus = true;
                }
                Action::TimerPause => {
                    self.timer_paused = !self.timer_paused;
                    if self.timer_paused {
                        self.timer_end = None;
                    } else {
                        self.timer_end =
                            Some(SystemTime::now() + Duration::from_secs(self.timer_remaining));
                    }
                }
                Action::TimerReset => {
                    self.timer_focus = true;
                    self.timer_end = None;
                    self.timer_remaining = 1500;
                    self.timer_paused = false;
                }
                Action::SaveAi => {
                    let config = ai::Config {
                        base_url: self.value("base_url", cx),
                        model: self.value("model", cx),
                        vision_model: self.value("vision_model", cx),
                        local_model: self.value("local_model", cx),
                        engine: self.value("engine", cx),
                        ..Default::default()
                    };
                    let key = self.value("chat_key", cx);
                    let limit = self.value("daily_limit", cx);
                    ensure!(limit.parse::<usize>().is_ok(), "每日新卡上限无效");
                    self.lib.set_setting("new_cards_per_day", &limit)?;
                    self.lib.set_setting("daily_limit", &limit)?;
                    self.lib
                        .set_setting("command", &self.value("command", cx))?;
                    self.begin("正在保存到系统凭证库…", move |lib, _, _| {
                        ai::save_shared(&lib, config, &key)?;
                        Ok(Outcome::Message(
                            "AI 设置已保存，聊天与远端转写共用配置，密钥保存在系统凭证库".into(),
                        ))
                    });
                }
                Action::SaveStorage => {
                    let config = backup::StorageConfig {
                        provider: self.value("provider", cx),
                        endpoint: self.value("endpoint", cx),
                        region: self.value("region", cx),
                        bucket: self.value("bucket", cx),
                        access_key_id: self.value("access_key", cx),
                        prefix: self.value("prefix", cx),
                        path_style: self.value("path_style", cx) == "true",
                        credential_id: String::new(),
                    };
                    let key = self.value("secret_key", cx);
                    let token = self.value("session_token", cx);
                    self.begin("正在保存存储凭证…", move |lib, _, _| {
                        backup::Storage::save(&lib, config, &key, &token)?;
                        Ok(Outcome::Message("对象存储设置已保存".into()))
                    });
                }
                Action::SelectModel(model) => {
                    let name = model.clone();
                    self.set("local_model", model, window, cx);
                    self.set("engine", "local", window, cx);
                    self.status =
                        format!("已选择 {name} · 待保存；点击「保存 AI 与学习设置」后生效");
                }
                Action::Models(model) => {
                    self.begin_scoped(
                        TaskScope::Application,
                        "正在连接模型下载源…",
                        move |lib, cancel, tx| {
                            ai::download_model(&lib, &model, &cancel, |progress| {
                                let _ = tx.send(Event::ModelProgress(progress));
                            })?;
                            Ok(Outcome::Message(
                                "模型文件已下载；点击「选用」并保存设置后生效".into(),
                            ))
                        },
                    );
                }
                Action::CloudList => self.begin("正在读取备份版本…", move |lib, _, _| {
                    Ok(Outcome::Backups(backup::Storage::load(&lib)?.list()?))
                }),
                Action::CloudBackup => {
                    self.begin("正在上传资料与附件…", move |lib, cancel, _| {
                        Ok(Outcome::Message(format!(
                            "备份完成：{}",
                            backup::Storage::load(&lib)?.upload(&lib, &cancel)?
                        )))
                    })
                }
                Action::CloudRestore(key) => {
                    self.begin("正在恢复并合并…", move |lib, cancel, _| {
                        Ok(Outcome::Message(format!(
                            "已合并 {} 条资料",
                            backup::Storage::load(&lib)?.restore(&lib, &key, &cancel)?
                        )))
                    })
                }
                Action::PdfPage(delta) => {
                    ensure!(self.busy.is_none(), "PDF 仍在载入，请稍后翻页");
                    self.pdf_page = (self.pdf_page as i32 + delta)
                        .clamp(0, self.pdf_pages.saturating_sub(1) as i32)
                        as u16;
                    self.render_pdf();
                }
                Action::PdfZoom(delta) => {
                    ensure!(self.busy.is_none(), "PDF 仍在载入，请稍后缩放");
                    self.pdf_width = (self.pdf_width + delta).clamp(300, 2400);
                    self.render_pdf();
                }
                Action::Theme => {
                    self.dark = !self.dark;
                    Theme::change(
                        if self.dark {
                            ThemeMode::Dark
                        } else {
                            ThemeMode::Light
                        },
                        Some(window),
                        cx,
                    );
                    design::apply(cx);
                }
                Action::CheckAudio(key) => {
                    ensure!(
                        self.audios
                            .iter()
                            .any(|r| text(r, "id") == key
                                && text(r, "practice_id") == self.practice_id),
                        "音频不属于当前组"
                    );
                    self.library.pending_delete.clear();
                    if !self.checked_audio.insert(key.clone()) {
                        self.checked_audio.remove(&key);
                    }
                }
                Action::Suspend(suspend) => {
                    let card = self.current_card().context("当前没有卡片")?;
                    self.lib.db()?.execute("INSERT INTO review_schedule(card_kind,card_id,suspended) VALUES(?,?,?) ON CONFLICT(card_kind,card_id) DO UPDATE SET suspended=excluded.suspended",rusqlite::params![card.kind,card.id,i32::from(suspend)])?;
                    self.refresh();
                    self.status = if suspend {
                        "已暂停此卡复习"
                    } else {
                        "已恢复此卡复习"
                    }
                    .into();
                }
                Action::TtsVoice(voice) => {
                    ensure!((0..54).contains(&voice), "音色编号无效");
                    self.tts_voice = voice;
                    self.lib.set_setting("tts_voice", &voice.to_string())?;
                }
                Action::TtsSpeed(speed) => {
                    ensure!(speed.is_finite() && (0.5..=2.).contains(&speed), "语速无效");
                    self.tts_speed = speed;
                    self.lib.set_setting("tts_speed", &speed.to_string())?;
                }
                Action::TtsDownload => {
                    self.begin_scoped(
                        TaskScope::Application,
                        "正在下载 Kokoro 中英模型…",
                        |lib, cancel, tx| {
                            tts::install(&lib, &cancel, &mut |p| {
                                let _ = tx.send(Event::ModelProgress(p));
                            })?;
                            Ok(Outcome::Message(
                                "Kokoro 中英模型已安装，可离线生成音频".into(),
                            ))
                        },
                    );
                }
                Action::TtsGenerate => {
                    let body = self.value("tts_text", cx);
                    self.library.filter = 0;
                    self.library.sort = 0;
                    self.set("library_search", "", window, cx);
                    ensure!(!body.trim().is_empty(), "请先输入要合成的文字");
                    ensure!(!self.practice_id.is_empty(), "请先新建或选择练习组");
                    let mut name = self.value("tts_name", cx);
                    if name.trim().is_empty() {
                        name =
                            format!("Kokoro · {}", chrono::Local::now().format("%m-%d %H:%M:%S"));
                    }
                    let group = self.practice_id.clone();
                    let voice = self.tts_voice;
                    let speed = self.tts_speed;
                    self.begin_scoped(
                        TaskScope::Application,
                        "正在本机生成语音（首次加载模型需稍候）…",
                        move |lib, cancel, _| {
                            let row = tts::generate(
                                &lib,
                                &tts::model_dir(&lib),
                                &group,
                                &name,
                                &body,
                                voice,
                                speed,
                                &cancel,
                            )?;
                            Ok(Outcome::TtsReady(row))
                        },
                    );
                }
                Action::TtsPlay => {
                    let row = self.tts_last.clone().context("请先生成音频")?;
                    ensure!(
                        PathBuf::from(text(&row, "audio_uri")).is_file(),
                        "生成的音频已被移除"
                    );
                    self.audio_blind.enabled = false;
                    self.page_scroll.set_offset(point(px(0.), px(0.)));
                    self.handle(Action::Audio(text(&row, "id")), window, cx);
                }
                Action::MergeAudios => {
                    let rows = self
                        .audios
                        .iter()
                        .filter(|r| {
                            text(r, "practice_id") == self.practice_id
                                && self.checked_audio.contains(&text(r, "id"))
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    ensure!(rows.len() >= 2, "请至少勾选两条音频");
                    let group = self.practice_id.clone();
                    self.begin("正在合并音频与字幕…", move |lib, cancel, _| {
                        clarora_gpui::audio_tools::merge(&lib, &rows, &group, &cancel)?;
                        Ok(Outcome::Message("已合并音频与字幕，原文件保留".into()))
                    });
                }
                Action::CommandImport => {
                    let command = self.value("command", cx);
                    let practice = self.practice_id.clone();
                    ensure!(!command.is_empty(), "请先在设置中填写本机命令");
                    self.begin("正在运行本机导入命令…",move|lib,cancel,_|{let(exe,args)=if cfg!(windows){(PathBuf::from("C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe"),vec!["-NoProfile".into(),"-Command".into(),command])}else{(PathBuf::from("/bin/zsh"),vec!["-c".into(),command])};let output=process::run(&exe,&args,&cancel,Duration::from_secs(60))?;let mut count=0;for line in output.lines(){let p=PathBuf::from(line.trim());if p.is_file()&&lib.import_audio(&practice,&p).is_ok(){count+=1;}}Ok(Outcome::Message(format!("已导入 {count} 条")))});
                }
            }
            Ok(())
        })();
        if let Err(e) = result {
            self.status = format!("{e:#}");
        }
        cx.notify();
    }
    fn assistant_view(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.page == "video"
            && self.video_material.is_none()
            && self.video_material_error.is_empty()
        {
            let _ = self.prepare_video_material(video_qa::Scope::All);
        }
        self.sync_chat(window, cx);
        let mut history = div()
            .id("assistant-history")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_3();
        let mut sessions = div().flex().gap_2().flex_wrap().child(self.button(
            "chat-new",
            "新会话",
            Action::ChatSession(String::new()),
            cx,
        ));
        for (i, record) in documents::records(&self.lib, "chat")
            .unwrap_or_default()
            .into_iter()
            .rev()
            .filter(|r| r["context"] == self.chat_context)
            .take(12)
            .enumerate()
        {
            sessions = sessions.child(
                self.button(
                    format!("chat-session-{i}"),
                    record["title"]
                        .as_str()
                        .unwrap_or("会话")
                        .chars()
                        .take(18)
                        .collect::<String>(),
                    Action::ChatSession(record["id"].as_str().unwrap_or("").into()),
                    cx,
                ),
            );
        }
        history = history.child(sessions);
        for (i, message) in self.chat_history.iter().enumerate() {
            history = history.child(
                panel(cx)
                    .bg(if message["role"] == "user" {
                        cx.theme().muted
                    } else {
                        cx.theme().background
                    })
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(cx.theme().primary)
                            .child(if message["role"] == "user" {
                                "你"
                            } else {
                                "✦ Clarora"
                            }),
                    )
                    .child(TextView::markdown(
                        ("chat-message", i),
                        message["content"].as_str().unwrap_or("").to_string(),
                        window,
                        cx,
                    )),
            );
        }
        if !self.reasoning.is_empty() {
            history = history.child(self.button(
                "toggle-reasoning",
                if self.reasoning_open {
                    "收起推理内容"
                } else {
                    "查看推理内容"
                },
                Action::ToggleReasoning,
                cx,
            ));
            if self.reasoning_open {
                history = history.child(TextView::markdown(
                    "ai-reasoning",
                    self.reasoning.clone(),
                    window,
                    cx,
                ));
            }
        }
        if self.busy.is_some() || self.chat_history.is_empty() {
            history = history.child(TextView::markdown(
                "ai-output",
                self.output.clone(),
                window,
                cx,
            ));
        }
        div()
            .w(px(400.))
            .max_w(relative(0.45))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_4()
            .p_5()
            .bg(cx.theme().background)
            .border_l_1()
            .border_color(cx.theme().border)
            .when(self.page == "video", |v| {
                v.w_full().max_w(relative(1.)).border_l_0().p_3().gap_2()
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .items_center()
                            .child(design::icon("discover", cx.theme().primary))
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("学习助手"),
                            ),
                    )
                    .child(
                        self.button("close-assistant", "收起", Action::Assistant, cx)
                            .ghost(),
                    ),
            )
            .child(
                div()
                    .p_3()
                    .rounded_lg()
                    .bg(cx.theme().muted)
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(if self.reference(cx).is_empty() {
                        "围绕当前学习材料，随时提问。".to_string()
                    } else {
                        self.reference(cx).chars().take(140).collect::<String>()
                    }),
            )
            .when(self.page == "video", |v| {
                v.child(
                    div()
                        .flex()
                        .gap_2()
                        .flex_wrap()
                        .child(
                            self.button(
                                "ai-video-current",
                                "引用当前句",
                                Action::VideoAsk(video_qa::Scope::Current),
                                cx,
                            )
                            .disabled(self.chat_running),
                        )
                        .child(
                            self.button(
                                "ai-video-all",
                                "引用整段字幕",
                                Action::VideoAsk(video_qa::Scope::All),
                                cx,
                            )
                            .disabled(self.chat_running || self.video_cues.is_empty()),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(
                        self.video_material
                            .as_ref()
                            .map(|m| {
                                format!(
                                    "{} · 引用已固定，播放不会改变本次材料。使用设置中的聊天 API。",
                                    m.label
                                )
                            })
                            .unwrap_or_else(|| {
                                if self.video_material_error.is_empty() {
                                    "请先生成或加载字幕，再选择引用范围。".into()
                                } else {
                                    self.video_material_error.clone()
                                }
                            }),
                    ),
                )
            })
            .child(self.field("question", "向当前材料提问"))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        self.button("ask", "发送", Action::Chat, cx)
                            .primary()
                            .disabled(
                                self.busy.is_some()
                                    || (self.page == "video" && self.video_material.is_none()),
                            ),
                    )
                    .child(
                        self.button("cancel-ai", "取消", Action::Cancel, cx)
                            .disabled(self.busy.is_none()),
                    )
                    .child(
                        self.button("save-answer", "存为问答卡", Action::SaveAnswer, cx)
                            .disabled(self.busy.is_some() || self.chat_history.is_empty()),
                    ),
            )
            .child(history)
            .into_any_element()
    }
    fn waveform_view(&self, cx: &Context<Self>) -> AnyElement {
        let Some(waveform) = &self.waveform else {
            return div()
                .text_sm()
                .child(if self.waveform_rx.is_some() {
                    "正在读取真实音频波形…"
                } else {
                    "选择音频后显示波形"
                })
                .into_any_element();
        };
        let peaks = waveform.peaks.clone();
        let duration = waveform.duration;
        let position = self.playback.position;
        let a = self.loop_a;
        let b = self.loop_b;
        let bounds_cell = self.waveform_bounds.clone();
        let color = cx.theme().primary;
        div()
            .w_full()
            .child(
                div()
                    .id("audio-waveform")
                    .w_full()
                    .h(px(48.))
                    .cursor_pointer()
                    .child(
                        canvas(
                            move |bounds, _, _| bounds_cell.set(bounds),
                            move |bounds, _, window, _| {
                                let width = f32::from(bounds.size.width);
                                let height = f32::from(bounds.size.height);
                                if let (Some(a), Some(b)) = (a, b) {
                                    window.paint_quad(fill(
                                        Bounds::new(
                                            point(
                                                bounds.left() + px((a / duration) as f32 * width),
                                                bounds.top(),
                                            ),
                                            size(
                                                px(((b - a) / duration) as f32 * width),
                                                bounds.size.height,
                                            ),
                                        ),
                                        rgba(0x36a58022),
                                    ));
                                }
                                let step = width / peaks.len().max(1) as f32;
                                for (i, peak) in peaks.iter().enumerate() {
                                    let h = (peak * height * 0.8).max(1.);
                                    window.paint_quad(fill(
                                        Bounds::new(
                                            point(
                                                bounds.left() + px(i as f32 * step),
                                                bounds.top() + px((height - h) / 2.),
                                            ),
                                            size(px(step.max(1.) * 0.7), px(h)),
                                        ),
                                        color,
                                    ));
                                }
                                for (time, color) in [
                                    (Some(position), rgb(0xe87240)),
                                    (a, rgb(0x2a9465)),
                                    (b, rgb(0x3b70be)),
                                ] {
                                    if let Some(time) = time {
                                        window.paint_quad(fill(
                                            Bounds::new(
                                                point(
                                                    bounds.left()
                                                        + px((time / duration).clamp(0., 1.)
                                                            as f32
                                                            * width),
                                                    bounds.top(),
                                                ),
                                                size(px(2.), bounds.size.height),
                                            ),
                                            color,
                                        ));
                                    }
                                }
                            },
                        )
                        .size_full(),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, window, cx| {
                            let bounds = this.waveform_bounds.get();
                            if bounds.size.width <= px(0.) {
                                return;
                            }
                            let duration = this.waveform.as_ref().map(|w| w.duration).unwrap_or(0.);
                            let time = ((event.position.x - bounds.left()) / bounds.size.width)
                                .clamp(0., 1.) as f64
                                * duration;
                            let threshold = duration * 8. / f64::from(f32::from(bounds.size.width));
                            this.waveform_drag = if !this.audio_clip_mode {
                                None
                            } else if event.modifiers.shift {
                                Some(true)
                            } else if event.modifiers.alt {
                                Some(false)
                            } else if this.loop_a.is_some_and(|a| (a - time).abs() < threshold) {
                                Some(true)
                            } else if this.loop_b.is_some_and(|b| (b - time).abs() < threshold) {
                                Some(false)
                            } else {
                                None
                            };
                            this.handle(
                                if let Some(start) = this.waveform_drag {
                                    Action::Boundary(time, start)
                                } else {
                                    Action::SeekTo(time)
                                },
                                window,
                                cx,
                            );
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                        if !event.dragging() {
                            return;
                        }
                        let bounds = this.waveform_bounds.get();
                        if bounds.size.width <= px(0.) {
                            return;
                        }
                        let duration = this.waveform.as_ref().map(|w| w.duration).unwrap_or(0.);
                        let time = ((event.position.x - bounds.left()) / bounds.size.width)
                            .clamp(0., 1.) as f64
                            * duration;
                        this.handle(
                            if let Some(start) = this.waveform_drag {
                                Action::Boundary(time, start)
                            } else {
                                Action::SeekTo(time)
                            },
                            window,
                            cx,
                        );
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, _| this.waveform_drag = None),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this, _, _, _| this.waveform_drag = None),
                    ),
            )
            .into_any_element()
    }
    fn reference(&self, cx: &App) -> String {
        match self.page.as_str() {
            "flashcard" | "discover" => self
                .current_card()
                .map(|c| format!("{}\n{}", c.front, c.back))
                .unwrap_or_default(),
            "listening" if self.audio_blind.enabled => {
                self.audio_blind.reference_for(self.audio_blind_chinese)
            }
            "listening" if self.loop_a.is_some() && self.loop_b.is_some() => {
                let (front, back) =
                    subtitles::excerpt(&self.cues, self.loop_a.unwrap(), self.loop_b.unwrap());
                format!("选中内容：{front}\n翻译：{back}")
            }
            "listening" => self
                .cues
                .iter()
                .map(|c| c.text.clone())
                .collect::<Vec<_>>()
                .join("\n"),
            "pdf" => self.document_text.clone(),
            "ocr" => self.ocr_text.clone(),
            "speaking" => self.value("reference", cx),
            "video" => self
                .video_material
                .as_ref()
                .map(|m| m.text.clone())
                .unwrap_or_default(),
            _ => String::new(),
        }
    }
    fn button(
        &self,
        key: impl Into<SharedString>,
        label: impl Into<SharedString>,
        action: Action,
        cx: &Context<Self>,
    ) -> Button {
        let selected = match &action {
            Action::Filter(kind) => *kind == self.card_filter,
            Action::ReviewMode(due) => *due == self.due,
            Action::Saved(saved) => *saved == self.saved_only,
            _ => false,
        };
        Button::new(ElementId::Name(key.into()))
            .selected(selected)
            .label(label)
            .small()
            .on_click(cx.listener(move |this, _, window, cx| {
                if matches!(
                    action,
                    Action::VideoRemove(_)
                        | Action::VideoSelect(_)
                        | Action::VideoEpisodes(_)
                        | Action::CheckAudio(_)
                        | Action::VideoAsk(_)
                        | Action::VideoCopySubtitle(_)
                        | Action::AudioCopySubtitle(..)
                        | Action::AudioBlindRevealCue(..)
                        | Action::AudioBlindHide
                ) {
                    cx.stop_propagation();
                }
                this.handle(action.clone(), window, cx)
            }))
    }
    fn tts_panel(&self, cx: &Context<Self>) -> AnyElement {
        let ready = tts::validate_model(&tts::model_dir(&self.lib)).is_ok();
        let voice = tts::VOICES
            .iter()
            .find(|(sid, _)| *sid == self.tts_voice)
            .map(|(_, name)| name.to_string())
            .unwrap_or_else(|| format!("音色 {}", self.tts_voice));
        let mut content = panel(cx)
            .child(div().text_lg().child("文字转音频 · Kokoro ONNX"))
            .child(div().text_sm().child(if ready {
                "中英模型已就绪 · 本机离线生成 WAV 和逐句字幕 · 自动保存到当前练习组"
            } else {
                "首次下载约 334 MiB，下载后可离线生成；需安装 clarora-tts 运行程序"
            }))
            .child(self.field("tts_name", "音频名称（留空自动命名）"))
            .child(self.field("tts_text", "要合成的文字"))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        self.player_menu(
                            "tts-voice",
                            voice,
                            tts::VOICES
                                .iter()
                                .map(|(sid, name)| {
                                    (
                                        name.to_string(),
                                        Action::TtsVoice(*sid),
                                        *sid == self.tts_voice,
                                        false,
                                    )
                                })
                                .collect(),
                            cx,
                        ),
                    )
                    .child(
                        self.player_menu(
                            "tts-speed",
                            format!("语速 {:.2}×", self.tts_speed),
                            [0.5, 0.75, 1., 1.25, 1.5, 2.]
                                .into_iter()
                                .map(|rate| {
                                    (
                                        format!("{rate:.2}×"),
                                        Action::TtsSpeed(rate),
                                        rate == self.tts_speed,
                                        false,
                                    )
                                })
                                .collect(),
                            cx,
                        ),
                    )
                    .child(
                        self.button(
                            "tts-download",
                            if ready {
                                "模型已下载"
                            } else {
                                "下载中英模型"
                            },
                            Action::TtsDownload,
                            cx,
                        )
                        .disabled(ready || self.busy.is_some()),
                    )
                    .child(
                        self.button("tts-generate", "生成并保存音频", Action::TtsGenerate, cx)
                            .primary()
                            .disabled(!ready || self.practice_id.is_empty() || self.busy.is_some()),
                    ),
            );
        if let Some(row) = &self.tts_last {
            content = content
                .child(
                    div()
                        .text_sm()
                        .child(format!("最近生成：{}", text(row, "name"))),
                )
                .child(
                    self.button("tts-play", "试听 / 开始学习", Action::TtsPlay, cx)
                        .disabled(self.busy.is_some()),
                );
        }
        content.into_any_element()
    }
    fn field(&self, key: &str, label: &str) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_sm().child(label.to_string()))
            .child(Input::new(self.inputs.get(key).unwrap()))
    }
    fn video_timeline(&self, cx: &Context<Self>) -> impl IntoElement {
        let bounds = std::rc::Rc::new(std::cell::Cell::new(Bounds::<Pixels>::default()));
        let paint_bounds = bounds.clone();
        let drag_bounds = bounds.clone();
        let duration = self.playback.duration;
        let ratio = if duration.is_finite() && duration > 0. {
            (self.playback.position / duration).clamp(0., 1.) as f32
        } else {
            0.
        };
        let loop_range = self.loop_a.zip(self.loop_b).filter(|(a, b)| b > a);
        let markers = [self.loop_a, self.loop_b];
        let color: Hsla = if self.page == "listening" {
            cx.theme().primary
        } else {
            rgb(0x7dd8b5).into()
        };
        let background: Hsla = if self.page == "listening" {
            cx.theme().border
        } else {
            rgb(0x345045).into()
        };
        div()
            .id("video-timeline")
            .w_full()
            .h(px(20.))
            .cursor_pointer()
            .child(
                canvas(
                    move |area, _, _| paint_bounds.set(area),
                    move |area, _, window, _| {
                        let width = f32::from(area.size.width);
                        let y = area.top() + px(8.);
                        window.paint_quad(fill(
                            Bounds::new(point(area.left(), y), size(area.size.width, px(4.))),
                            background,
                        ));
                        if duration > 0. {
                            if let Some((a, b)) = loop_range {
                                let left = (a / duration).clamp(0., 1.) as f32 * width;
                                let right = (b / duration).clamp(0., 1.) as f32 * width;
                                window.paint_quad(fill(
                                    Bounds::new(
                                        point(area.left() + px(left), y - px(3.)),
                                        size(px(right - left), px(10.)),
                                    ),
                                    rgb(0x3c7260),
                                ));
                            }
                            for time in markers.into_iter().flatten() {
                                let x =
                                    (time / duration).clamp(0., 1.) as f32 * (width - 2.).max(0.);
                                window.paint_quad(fill(
                                    Bounds::new(
                                        point(area.left() + px(x), y - px(4.)),
                                        size(px(2.), px(12.)),
                                    ),
                                    rgb(0xe7bd7b),
                                ));
                            }
                        }
                        window.paint_quad(fill(
                            Bounds::new(point(area.left(), y), size(px(width * ratio), px(4.))),
                            color,
                        ));
                        window.paint_quad(fill(
                            Bounds::new(
                                point(area.left() + px((width - 8.).max(0.) * ratio), y - px(2.)),
                                size(px(8.), px(8.)),
                            ),
                            color,
                        ));
                    },
                )
                .size_full(),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    let area = bounds.get();
                    if area.size.width > px(0.) && duration.is_finite() && duration > 0. {
                        this.video_seek_dragging = true;
                        let time = ((event.position.x - area.left()) / area.size.width)
                            .clamp(0., 1.) as f64
                            * duration;
                        this.handle(Action::SeekTo(time), window, cx);
                    }
                }),
            )
            .on_mouse_move(
                cx.listener(move |this, event: &MouseMoveEvent, window, cx| {
                    let area = drag_bounds.get();
                    if this.video_seek_dragging
                        && event.dragging()
                        && area.size.width > px(0.)
                        && duration.is_finite()
                        && duration > 0.
                    {
                        let time = ((event.position.x - area.left()) / area.size.width)
                            .clamp(0., 1.) as f64
                            * duration;
                        this.handle(Action::SeekTo(time), window, cx);
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.video_seek_dragging = false),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.video_seek_dragging = false),
            )
    }
    fn transport_button(
        &self,
        key: impl Into<SharedString>,
        label: impl Into<SharedString>,
        action: Action,
        cx: &Context<Self>,
    ) -> Button {
        let button = self.button(key, label, action, cx);
        if self.page == "video" {
            button.custom(
                ButtonCustomVariant::new(cx)
                    .foreground(rgb(0xe8f0eb).into())
                    .hover(rgb(0x29483c).into())
                    .active(rgb(0x345b4b).into()),
            )
        } else {
            button
        }
    }
    fn player_icon(
        &self,
        key: &'static str,
        icon: &str,
        tip: &str,
        action: Action,
        cx: &Context<Self>,
    ) -> Button {
        Button::new(key)
            .icon(Icon::new(IconName::ChevronDown).path(format!("ui/{icon}.svg")))
            .tooltip(tip.to_string())
            .small()
            .w(px(32.))
            .h(px(32.))
            .custom(
                ButtonCustomVariant::new(cx)
                    .foreground(if self.page == "listening" {
                        cx.theme().foreground
                    } else {
                        rgb(0xe8f0eb).into()
                    })
                    .hover(if self.page == "listening" {
                        cx.theme().accent
                    } else {
                        rgb(0x303c38).into()
                    })
                    .active(if self.page == "listening" {
                        cx.theme().accent
                    } else {
                        rgb(0x41554b).into()
                    }),
            )
            .on_click(
                cx.listener(move |this, _, window, cx| this.handle(action.clone(), window, cx)),
            )
    }
    fn player_menu(
        &self,
        key: &'static str,
        label: String,
        options: Vec<(String, Action, bool, bool)>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let view = cx.entity().downgrade();
        // The menu is transient; the checked value is rebuilt from current playback state.
        Button::new(key)
            .label(label)
            .small()
            .custom(
                ButtonCustomVariant::new(cx)
                    .foreground(if self.page == "listening" || self.page == "library" {
                        cx.theme().foreground
                    } else {
                        rgb(0xe8f0eb).into()
                    })
                    .hover(if self.page == "listening" || self.page == "library" {
                        cx.theme().accent
                    } else {
                        rgb(0x303c38).into()
                    })
                    .active(if self.page == "listening" || self.page == "library" {
                        cx.theme().accent
                    } else {
                        rgb(0x41554b).into()
                    }),
            )
            .dropdown_menu_with_anchor(Corner::BottomRight, move |mut menu, _, _| {
                for (label, action, checked, disabled) in options.clone() {
                    let view = view.clone();
                    menu = menu.item(
                        PopupMenuItem::new(label)
                            .checked(checked)
                            .disabled(disabled)
                            .on_click(move |_, window, cx| {
                                let _ = view
                                    .update(cx, |this, cx| this.handle(action.clone(), window, cx));
                            }),
                    );
                }
                menu
            })
    }
    fn player_volume(&self, cx: &Context<Self>) -> impl IntoElement {
        let bounds = std::rc::Rc::new(std::cell::Cell::new(Bounds::<Pixels>::default()));
        let paint_bounds = bounds.clone();
        let drag_bounds = bounds.clone();
        let volume = self.volume as f32 / 100.;
        div()
            .id("player-volume-slider")
            .w(px(64.))
            .h(px(28.))
            .cursor_pointer()
            .child(
                canvas(
                    move |area, _, _| paint_bounds.set(area),
                    move |area, _, window, _| {
                        let y = area.top() + px(12.);
                        window.paint_quad(fill(
                            Bounds::new(point(area.left(), y), size(area.size.width, px(3.))),
                            rgb(0x485750),
                        ));
                        window.paint_quad(fill(
                            Bounds::new(
                                point(area.left(), y),
                                size(area.size.width * volume, px(3.)),
                            ),
                            rgb(0xc5d9d0),
                        ));
                        window.paint_quad(fill(
                            Bounds::new(
                                point(
                                    area.left() + (area.size.width - px(6.)) * volume,
                                    y - px(2.),
                                ),
                                size(px(6.), px(7.)),
                            ),
                            rgb(0xffffff),
                        ));
                    },
                )
                .size_full(),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    let area = bounds.get();
                    if area.size.width > px(0.) {
                        this.video_volume_dragging = true;
                        this.handle(
                            Action::Volume(
                                ((event.position.x - area.left()) / area.size.width).clamp(0., 1.)
                                    as f64
                                    * 100.,
                            ),
                            window,
                            cx,
                        );
                    }
                    cx.stop_propagation();
                }),
            )
            .on_mouse_move(
                cx.listener(move |this, event: &MouseMoveEvent, window, cx| {
                    let area = drag_bounds.get();
                    if this.video_volume_dragging && event.dragging() && area.size.width > px(0.) {
                        this.handle(
                            Action::Volume(
                                ((event.position.x - area.left()) / area.size.width).clamp(0., 1.)
                                    as f64
                                    * 100.,
                            ),
                            window,
                            cx,
                        );
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.video_volume_dragging = false),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.video_volume_dragging = false),
            )
    }
    fn leave_video_fullscreen(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
        if !self.video_fullscreen {
            return;
        }
        if self.video_fullscreen_owned && window.is_fullscreen() {
            window.toggle_fullscreen();
        }
        self.video_fullscreen = false;
        self.video_fullscreen_owned = false;
        self.video_fullscreen_seen = false;
        window.focus(&self.focus);
    }
    fn navigation_progress(&self) -> f32 {
        let target = if self.navigation_open { 1. } else { 0. };
        let Some((started, from)) = self.navigation_motion else {
            return target;
        };
        let elapsed = (started.elapsed().as_secs_f32() / 0.24).clamp(0., 1.);
        let eased = 1. - (1. - elapsed).powi(3);
        from + (target - from) * eased
    }
    fn video_picture(&self, height: Option<Pixels>, cx: &Context<Self>) -> impl IntoElement {
        let active = self.video_path.as_ref().is_some_and(|p| {
            self.current_path.as_ref() == Some(p) && self.playback.path == p.to_string_lossy()
        });
        let frame = self.playback.frame.clone().filter(|_| active);
        let active_caption = if !active
            || !self.video_captions_enabled
            || self.playback.subtitle_is_bitmap
            || self.video_caption_source == VideoCaptionSource::Hidden
        {
            String::new()
        } else if self.video_caption_source == VideoCaptionSource::Track {
            self.playback.subtitle.clone()
        } else {
            video_subtitles::active(&self.video_cues, self.playback.position)
                .map(|i| self.video_cues[i].text.clone())
                .filter(|text| !text.is_empty())
                .unwrap_or_else(|| self.playback.subtitle.clone())
        };
        div()
            .relative()
            .w_full()
            .id("video-picture")
            .when_some(height, |view, height| view.h(height).flex_shrink_0())
            .when(height.is_none(), |view| view.flex_1().min_h_0())
            .overflow_hidden()
            .bg(rgb(0x090e0c))
            .when_some(frame.clone(), |v, image| {
                // Keep the intrinsic image ratio out of the flex flow. The
                // decoder frame must fit entirely inside the available viewport.
                v.child(
                    img(image)
                        .absolute()
                        .top_0()
                        .left_0()
                        .w_full()
                        .when_some(height, |image, height| image.h(height))
                        .when(height.is_none(), |image| image.h_full())
                        .object_fit(ObjectFit::Contain),
                )
            })
            .when(frame.is_none(), |v| {
                v.child(
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(rgb(0x9bafa4))
                        .child("打开本地视频，或从 bilibili 导入"),
                )
            })
            .when(!active_caption.is_empty(), |v| {
                v.child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .right_0()
                        .px_5()
                        .py_3()
                        .id("video-current-caption")
                        .max_h(px(f32::from(self.video_subtitle_size) * 3.8 + 24.))
                        .overflow_y_scroll()
                        .bg(rgba(0x050a08c9))
                        .text_color(rgb(0xffffff))
                        .text_size(px(f32::from(self.video_subtitle_size)))
                        .text_center()
                        .child(active_caption),
                )
            })
            .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                if event.click_count() == 2 {
                    this.handle(Action::VideoFullscreen, window, cx);
                }
            }))
    }
    fn fullscreen_video(&self, cx: &Context<Self>) -> AnyElement {
        div()
            .id("video-fullscreen")
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x090e0c))
            .track_focus(&self.focus)
            .key_context("VideoFullscreen")
            .on_action(cx.listener(|this, _: &ToggleVideoFullscreen, window, cx| {
                this.handle(Action::VideoFullscreen, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ExitVideoFullscreen, window, cx| {
                this.handle(Action::VideoExitFullscreen, window, cx)
            }))
            .on_action(cx.listener(|this, _: &FullscreenPlayPause, window, cx| {
                this.handle(Action::Play, window, cx)
            }))
            .on_action(cx.listener(|this, _: &FullscreenSeekBack, window, cx| {
                this.handle(Action::Seek(-10.), window, cx)
            }))
            .on_action(cx.listener(|this, _: &FullscreenSeekForward, window, cx| {
                this.handle(Action::Seek(10.), window, cx)
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_4()
                    .py_1()
                    .flex_shrink_0()
                    .text_color(rgb(0xb9c9c1))
                    .child(
                        div().flex_1().min_w_0().text_xs().truncate().child(
                            self.current_path
                                .as_ref()
                                .and_then(|p| p.file_name())
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_default(),
                        ),
                    )
                    .child(self.transport_button(
                        "exit-video-fullscreen",
                        "退出全屏 · Esc",
                        Action::VideoExitFullscreen,
                        cx,
                    )),
            )
            .child(self.video_picture(None, cx))
            .child(self.video_controls(cx))
            .into_any_element()
    }
    fn video_controls(&self, cx: &Context<Self>) -> AnyElement {
        let valid_loop = self.loop_a.zip(self.loop_b).is_some_and(|(a, b)| b > a);
        let current_cue =
            video_subtitles::active(&self.video_cues, self.playback.position).is_some();
        let rates = [0.6, 0.8, 1., 1.25, 1.5, 2.]
            .into_iter()
            .map(|r| {
                (
                    format!("{r}×"),
                    Action::Rate(r),
                    (self.rate - r).abs() < 0.001,
                    false,
                )
            })
            .collect();
        let mut options = [14, 16, 20, 24, 28, 32]
            .into_iter()
            .map(|size| {
                (
                    format!("字幕 {size}px"),
                    Action::VideoSubtitleSize(size),
                    self.video_subtitle_size == size,
                    false,
                )
            })
            .collect::<Vec<_>>();
        options.extend(
            [("关闭循环", 0), ("单集循环", 1), ("A–B 片段循环", 2)]
                .into_iter()
                .map(|(label, mode)| {
                    (
                        label.into(),
                        Action::LoopMode(mode),
                        self.repeat == mode,
                        mode == 2 && !valid_loop,
                    )
                }),
        );
        options.push((
            "A–B 精听工具".into(),
            Action::VideoPrecision,
            self.video_precision,
            false,
        ));
        div()
            .flex_shrink_0()
            .px_4()
            .pt_1()
            .pb_3()
            .flex()
            .flex_col()
            .gap_1()
            .bg(rgb(0x171f1c))
            .text_color(rgb(0xe8f0eb))
            .child(self.video_timeline(cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .flex_wrap()
                    .child(self.player_icon(
                        "video-back",
                        "rewind",
                        "后退 10 秒",
                        Action::Seek(-10.),
                        cx,
                    ))
                    .child(
                        self.player_icon(
                            "video-play",
                            if self.playback.paused {
                                "play"
                            } else {
                                "pause"
                            },
                            if self.playback.paused {
                                "播放 · 空格"
                            } else {
                                "暂停 · 空格"
                            },
                            Action::Play,
                            cx,
                        )
                        .w(px(40.))
                        .h(px(40.))
                        .rounded_full(),
                    )
                    .child(self.player_icon(
                        "video-forward",
                        "forward",
                        "前进 10 秒",
                        Action::Seek(10.),
                        cx,
                    ))
                    .child(div().text_xs().text_color(rgb(0xb9c9c1)).child(format!(
                        "{} / {}",
                        clock(self.playback.position),
                        clock(self.playback.duration)
                    )))
                    .child(div().flex_1())
                    .child(self.player_menu("video-speed", format!("{}× ▾", self.rate), rates, cx))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(self.player_icon(
                                "video-mute",
                                if self.volume == 0. { "muted" } else { "volume" },
                                &format!("音量 {:.0}% · 点击静音 / 恢复", self.volume),
                                Action::Mute,
                                cx,
                            ))
                            .child(self.player_volume(cx)),
                    )
                    .child(
                        self.transport_button(
                            "video-captions-toggle",
                            if self.video_captions_enabled {
                                "字幕：开"
                            } else {
                                "字幕：关"
                            },
                            Action::VideoCaptions,
                            cx,
                        )
                        .selected(self.video_captions_enabled),
                    )
                    .child(self.player_menu("video-options", "设置 ▾".into(), options, cx))
                    .child(
                        self.player_icon(
                            "video-fullscreen-toggle",
                            if self.video_fullscreen {
                                "fullscreen-exit"
                            } else {
                                "fullscreen"
                            },
                            "全屏 / 退出全屏 · F11；双击画面也可全屏",
                            Action::VideoFullscreen,
                            cx,
                        )
                        .disabled(self.current_path.is_none()),
                    ),
            )
            .when(self.video_precision, |v| {
                v.child(
                    div()
                        .mt_2()
                        .pt_3()
                        .border_t_1()
                        .border_color(rgb(0x34463d))
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .flex_wrap()
                                .child(
                                    self.transport_button(
                                        "video-set-a",
                                        format!(
                                            "A  {}",
                                            self.loop_a.map(clip_clock).unwrap_or("设起点".into())
                                        ),
                                        Action::Mark(true),
                                        cx,
                                    )
                                    .tooltip("将当前播放位置设为 A 点"),
                                )
                                .child(
                                    self.transport_button(
                                        "video-set-b",
                                        format!(
                                            "B  {}",
                                            self.loop_b.map(clip_clock).unwrap_or("设终点".into())
                                        ),
                                        Action::Mark(false),
                                        cx,
                                    )
                                    .disabled(self.loop_a.is_none())
                                    .tooltip("将当前位置设为 B 点，自动循环此片段"),
                                )
                                .child(
                                    self.transport_button(
                                        "video-clear-loop",
                                        "清除",
                                        Action::VideoClearLoop,
                                        cx,
                                    )
                                    .disabled(self.loop_a.is_none() && self.loop_b.is_none()),
                                )
                                .child(div().flex_1())
                                .child(
                                    self.transport_button(
                                        "video-clip",
                                        "存为片段闪卡",
                                        Action::Clip,
                                        cx,
                                    )
                                    .disabled(!valid_loop || self.busy.is_some()),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .flex_wrap()
                                .child(
                                    self.transport_button(
                                        "video-prev-sentence",
                                        "上一句",
                                        Action::VideoSentence(false),
                                        cx,
                                    )
                                    .disabled(
                                        !self
                                            .video_cues
                                            .iter()
                                            .any(|c| c.start < self.playback.position - 0.5),
                                    ),
                                )
                                .child(
                                    self.transport_button(
                                        "video-repeat-sentence",
                                        "重听本句",
                                        Action::VideoRepeatCue,
                                        cx,
                                    )
                                    .disabled(!current_cue),
                                )
                                .child(
                                    self.transport_button(
                                        "video-next-sentence",
                                        "下一句",
                                        Action::VideoSentence(true),
                                        cx,
                                    )
                                    .disabled(
                                        !self
                                            .video_cues
                                            .iter()
                                            .any(|c| c.start > self.playback.position + 0.1),
                                    ),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(0x9bafa4))
                                        .child("A → B 自动循环 · 进度条金色标记为边界"),
                                ),
                        ),
                )
            })
            .into_any_element()
    }
    fn blind_panel(&self, cx: &Context<Self>) -> AnyElement {
        let ready = !self.current_audio.is_empty()
            && self.busy.is_none()
            && self.audio_blind_pending.is_none()
            && self
                .audio_blind_settle_until
                .is_none_or(|t| Instant::now() >= t)
            && self
                .current_path
                .as_ref()
                .is_some_and(|p| p.to_string_lossy() == self.playback.path);
        let target =
            blind_listening::resolve(&self.cues, self.playback.position, self.playback.duration);
        let loop_range = self
            .loop_a
            .zip(self.loop_b)
            .filter(|(a, b)| self.repeat == 2 && b > a);
        let can_sentence = |direction| {
            ready
                && blind_listening::sentence_target(
                    &self.cues,
                    self.playback.position,
                    self.playback.duration,
                    self.audio_blind.revealed.as_ref(),
                    direction,
                    loop_range,
                )
                .is_some()
        };
        div()
            .id("blind-listening-controls")
            .flex()
            .flex_col()
            .gap_2()
            .px_5()
            .py_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        self.player_menu(
                            "blind-caption-display",
                            if self.audio_blind_chinese {
                                "查看：仅中文 ▾"
                            } else {
                                "查看：中英字幕 ▾"
                            }
                            .into(),
                            vec![
                                (
                                    "中英字幕".into(),
                                    Action::AudioBlindChinese(false),
                                    !self.audio_blind_chinese,
                                    false,
                                ),
                                (
                                    "仅中文".into(),
                                    Action::AudioBlindChinese(true),
                                    self.audio_blind_chinese,
                                    false,
                                ),
                            ],
                            cx,
                        ),
                    )
                    .child(
                        self.button("blind-back-five", "回退 5 秒", Action::Seek(-5.), cx)
                            .disabled(!ready),
                    )
                    .child(
                        self.button(
                            "blind-previous-sentence",
                            "上一句 ↑",
                            Action::AudioBlindSentence(-1),
                            cx,
                        )
                        .disabled(!can_sentence(-1)),
                    )
                    .child(
                        self.button(
                            "blind-repeat-sentence",
                            "从句首重听 R",
                            Action::AudioBlindSentence(0),
                            cx,
                        )
                        .disabled(!can_sentence(0)),
                    )
                    .child(
                        self.button(
                            "blind-next-sentence",
                            "下一句 ↓",
                            Action::AudioBlindSentence(1),
                            cx,
                        )
                        .disabled(!can_sentence(1)),
                    )
                    .child(
                        self.button(
                            "blind-reveal",
                            if self.audio_blind_pending.is_some() {
                                "正在暂停…"
                            } else if self.audio_blind.revealed.is_some() {
                                "收起文字 H"
                            } else if self.audio_blind_chinese {
                                "看中文 H"
                            } else {
                                "看刚才一句 H"
                            },
                            if self.audio_blind.revealed.is_some() {
                                Action::AudioBlindHide
                            } else {
                                Action::AudioBlindReveal
                            },
                            cx,
                        )
                        .primary()
                        .disabled(
                            !ready || (self.audio_blind.revealed.is_none() && target.is_none()),
                        ),
                    )
                    .when(self.audio_blind.revealed.is_some(), |v| {
                        v.child(
                            self.button(
                                "blind-resume",
                                "继续盲听 C",
                                Action::AudioBlindResume(false),
                                cx,
                            )
                            .disabled(!ready),
                        )
                    })
                    .when(self.playback.eof, |v| {
                        v.child(
                            self.button(
                                "blind-restart",
                                "从头再听",
                                Action::AudioBlindResume(true),
                                cx,
                            )
                            .disabled(!ready),
                        )
                    })
                    .child(
                        self.button("blind-full", "查看全文", Action::AudioBlind(false), cx)
                            .ghost()
                            .disabled(self.busy.is_some()),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(if self.cues.is_empty() {
                        "暂无字幕，仍可盲听；切回字幕模式可导入字幕。"
                    } else if self.audio_blind_chinese {
                        "查看时仅显示刚才一句的中文，英文保持遮挡；继续播放后重新遮住中文。"
                    } else if self.audio_blind.revealed.as_ref().is_some_and(|r| r.sparse) {
                        "已显示上一条字幕；继续播放会自动遮住文字。"
                    } else {
                        "点击词块从对应单词播放，点击时间戳从句首播放；词时间为估算。"
                    }),
            )
            .into_any_element()
    }
    fn invalidate_blind(&mut self) {
        self.audio_blind.invalidate();
        self.audio_blind_pending = None;
    }
    fn audio_study(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let group = self
            .practices
            .iter()
            .find(|r| text(r, "id") == self.practice_id)
            .map(|r| text(r, "name"))
            .unwrap_or_else(|| "选择练习组".into());
        let audios = self
            .audios
            .iter()
            .filter(|r| text(r, "practice_id") == self.practice_id)
            .collect::<Vec<_>>();
        let index = audios
            .iter()
            .position(|r| text(r, "id") == self.current_audio);
        let title = index
            .map(|i| text(audios[i], "name"))
            .unwrap_or_else(|| "选择音频".into());
        let group_options = self
            .practices
            .iter()
            .map(|r| {
                let key = text(r, "id");
                (
                    text(r, "name"),
                    Action::AudioPractice(key.clone()),
                    key == self.practice_id,
                    self.busy.is_some(),
                )
            })
            .collect();
        let audio_options = audios
            .iter()
            .enumerate()
            .map(|(i, r)| {
                (
                    format!("{}  {}", i + 1, text(r, "name")),
                    Action::Audio(text(r, "id")),
                    Some(i) == index,
                    self.busy.is_some(),
                )
            })
            .collect();
        let previous = index
            .and_then(|i| i.checked_sub(1))
            .map(|i| text(audios[i], "id"));
        let next = index
            .filter(|i| i + 1 < audios.len())
            .map(|i| text(audios[i + 1], "id"));
        let page = index
            .map(|i| format!("{} / {}", i + 1, audios.len()))
            .unwrap_or_else(|| format!("0 / {}", audios.len()));
        let available = !self.current_audio.is_empty();
        let mut subtitle_options = vec![
            (
                "导入英文字幕".into(),
                Action::SubtitleImport(false),
                false,
                !available || self.busy.is_some(),
            ),
            (
                "导入中文字幕".into(),
                Action::SubtitleImport(true),
                false,
                !available || self.busy.is_some(),
            ),
            (
                "生成原文字幕".into(),
                Action::Transcribe,
                false,
                !available || self.busy.is_some(),
            ),
            (
                "生成中文字幕".into(),
                Action::Translate,
                false,
                self.cues.is_empty() || self.busy.is_some(),
            ),
            ("刷新字幕".into(), Action::Refresh, false, !available),
            (
                "单独显示当前句".into(),
                Action::AudioCurrent,
                self.audio_show_current,
                false,
            ),
        ];
        subtitle_options.extend([14, 16, 18, 20, 24, 28, 32].into_iter().map(|n| {
            (
                format!("字幕 {n}px"),
                Action::AudioFont(n),
                self.audio_font_size == n,
                false,
            )
        }));
        if self.audio_blind.enabled {
            subtitle_options = vec![(
                "管理字幕（退出盲听）".into(),
                Action::AudioBlind(false),
                false,
                self.busy.is_some(),
            )];
        }
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .flex_wrap()
            .px_4()
            .py_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                self.button("audio-subtitle-mode", "字幕", Action::AudioBlind(false), cx)
                    .selected(!self.audio_blind.enabled)
                    .disabled(self.busy.is_some()),
            )
            .child(
                self.button("audio-blind-mode", "盲听", Action::AudioBlind(true), cx)
                    .selected(self.audio_blind.enabled)
                    .disabled(self.busy.is_some()),
            )
            .child(self.player_menu(
                "audio-practice-picker",
                format!("{} ▾", group.chars().take(20).collect::<String>()),
                group_options,
                cx,
            ))
            .child(self.player_menu(
                "audio-picker",
                format!("{} ▾", title.chars().take(24).collect::<String>()),
                audio_options,
                cx,
            ))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(page),
            )
            .child(
                self.button(
                    "audio-previous",
                    "‹",
                    Action::Audio(previous.clone().unwrap_or_default()),
                    cx,
                )
                .disabled(previous.is_none() || self.busy.is_some()),
            )
            .child(
                self.button(
                    "audio-next",
                    "›",
                    Action::Audio(next.clone().unwrap_or_default()),
                    cx,
                )
                .disabled(next.is_none() || self.busy.is_some()),
            )
            .child(div().flex_1())
            .child(
                self.button(
                    "audio-follow",
                    if self.audio_follow {
                        "跟随 ✓"
                    } else {
                        "跟随"
                    },
                    Action::AudioFollow,
                    cx,
                )
                .ghost(),
            )
            .when(!self.audio_blind.enabled, |v| {
                v.child(
                    self.button(
                        "audio-copy-all",
                        "复制全部",
                        Action::AudioCopySubtitle(self.current_audio.clone(), None),
                        cx,
                    )
                    .ghost()
                    .disabled(self.cues.is_empty()),
                )
            })
            .child(self.player_menu(
                "audio-subtitle-options",
                "字幕 ▾".into(),
                subtitle_options,
                cx,
            ))
            .child(
                self.button(
                    "audio-manage",
                    "管理音频",
                    Action::Page("library".into()),
                    cx,
                )
                .ghost(),
            );
        let active = video_subtitles::active(&self.cues, self.playback.position);
        let transcript = list(
            self.audio_scroll.clone(),
            cx.processor(|this, i: usize, _, cx| {
                let Some(cue) = this.cues.get(i) else {
                    return div().into_any_element();
                };
                let active = video_subtitles::active(&this.cues, this.playback.position) == Some(i);
                if this.audio_blind.enabled {
                    let revealed = this.audio_blind.revealed.as_ref().filter(|r| {
                        r.cue.start == cue.start && r.cue.end == cue.end && r.cue.text == cue.text
                    });
                    let mut word_line = div().flex().flex_wrap().gap_x_1().gap_y_2().items_center();
                    let show_original = revealed.is_some() && !this.audio_blind_chinese;
                    let words = subtitles::words(cue);
                    let no_words = words.is_empty();
                    for (j, word) in words.into_iter().enumerate() {
                        let audio_id = this.current_audio.clone();
                        let source_cue = cue.clone();
                        let current = active
                            && word.start <= this.playback.position
                            && this.playback.position < word.end;
                        let label = format!("第 {} 个词，从这里播放", j + 1);
                        let mut token = div()
                            .id(("blind-audio-word", i * 100000 + j))
                            .cursor_pointer()
                            .rounded_sm()
                            .px_1()
                            .py_1()
                            .text_size(px(this.audio_font_size as f32))
                            .bg(if current {
                                cx.theme().accent
                            } else {
                                cx.theme().border
                            })
                            .hover(|v| v.bg(cx.theme().accent))
                            .tooltip(move |window, cx| {
                                Tooltip::new(label.clone()).build(window, cx)
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.handle(
                                    Action::AudioBlindRow(
                                        audio_id.clone(),
                                        source_cue.clone(),
                                        Some(j),
                                    ),
                                    window,
                                    cx,
                                );
                            }));
                        if show_original {
                            token = token.child(word.text);
                        } else {
                            token = token
                                .w(px((word.text.chars().count() as f32
                                    * this.audio_font_size as f32
                                    * 0.55
                                    + 8.)
                                    .clamp(24., 280.)))
                                .h(px(this.audio_font_size as f32 + 12.));
                        }
                        word_line = word_line.child(token);
                    }
                    let mut content = div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .py_1()
                        .child(word_line)
                        .when(no_words, |v| {
                            v.child(
                                div()
                                    .w(px(180.))
                                    .h(px(this.audio_font_size as f32 + 4.))
                                    .rounded_sm()
                                    .bg(cx.theme().border),
                            )
                        });
                    if let Some(revealed) = revealed {
                        let chinese = if this.audio_blind_chinese {
                            blind_listening::chinese_hint(&revealed.cue.text)
                        } else {
                            subtitles::languages(std::slice::from_ref(&revealed.cue)).1
                        };
                        if !chinese.is_empty() || this.audio_blind_chinese {
                            content = content.child(
                                div()
                                    .text_size(px((this.audio_font_size as f32 - 2.).max(12.)))
                                    .text_color(cx.theme().muted_foreground)
                                    .whitespace_normal()
                                    .child(if chinese.is_empty() {
                                        "本句暂无中文".into()
                                    } else {
                                        chinese
                                    }),
                            );
                        }
                    } else {
                        content = content.child(
                            div()
                                .w_full()
                                .max_w(px(380.))
                                .h(px((this.audio_font_size as f32 - 2.).max(12.)))
                                .rounded_sm()
                                .bg(cx.theme().border),
                        );
                    }
                    let clicked_audio = this.current_audio.clone();
                    let clicked_cue = cue.clone();
                    let reveal_audio = this.current_audio.clone();
                    let reveal_cue = cue.clone();
                    return div()
                        .id(("blind-audio-cue", i))
                        .cursor_pointer()
                        .hover(|v| v.bg(cx.theme().accent))
                        .tooltip(|window, cx| {
                            Tooltip::new("左键播放，右键查看／收起这句").build(window, cx)
                        })
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                let is_revealed = reveal_audio == this.current_audio
                                    && this.audio_blind.revealed.as_ref().is_some_and(|r| {
                                        r.cue.start == reveal_cue.start
                                            && r.cue.end == reveal_cue.end
                                            && r.cue.text == reveal_cue.text
                                    });
                                this.handle(
                                    if is_revealed {
                                        Action::AudioBlindHide
                                    } else {
                                        Action::AudioBlindRevealCue(
                                            reveal_audio.clone(),
                                            reveal_cue.clone(),
                                        )
                                    },
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.handle(
                                Action::AudioBlindRow(
                                    clicked_audio.clone(),
                                    clicked_cue.clone(),
                                    None,
                                ),
                                window,
                                cx,
                            );
                        }))
                        .w_full()
                        .flex()
                        .gap_3()
                        .px_5()
                        .py_3()
                        .border_l_2()
                        .border_color(if active {
                            cx.theme().primary
                        } else {
                            cx.theme().background
                        })
                        .when(active, |v| v.bg(cx.theme().accent))
                        .child(
                            div()
                                .w(px(40.))
                                .flex_shrink_0()
                                .pt_1()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(clock(cue.start)),
                        )
                        .child(div().flex_1().min_w_0().child(content))
                        .child(
                            this.button(
                                format!("blind-row-reveal-{i}"),
                                if revealed.is_some() {
                                    "收起"
                                } else {
                                    "查看"
                                },
                                if revealed.is_some() {
                                    Action::AudioBlindHide
                                } else {
                                    Action::AudioBlindRevealCue(
                                        this.current_audio.clone(),
                                        cue.clone(),
                                    )
                                },
                                cx,
                            )
                            .ghost()
                            .disabled(this.busy.is_some() || this.audio_blind_pending.is_some()),
                        )
                        .when(revealed.is_some(), |v| {
                            v.child(
                                this.button(
                                    format!("blind-row-copy-{i}"),
                                    "复制",
                                    Action::AudioCopySubtitle(
                                        this.current_audio.clone(),
                                        Some(cue.clone()),
                                    ),
                                    cx,
                                )
                                .ghost(),
                            )
                        })
                        .into_any_element();
                }
                let (_, translation) = subtitles::languages(std::slice::from_ref(cue));
                let mut line = div().flex().flex_wrap().gap_x_1().gap_y_1().items_center();
                for (j, word) in subtitles::words(cue).into_iter().enumerate() {
                    let current = active
                        && word.start <= this.playback.position
                        && this.playback.position < word.end;
                    let marked = this.key_points.iter().find(|p| {
                        p.text.split_whitespace().any(|w| {
                            w.trim_matches(|c: char| !c.is_alphanumeric())
                                .eq_ignore_ascii_case(
                                    word.text.trim_matches(|c: char| !c.is_alphanumeric()),
                                )
                        })
                    });
                    let tip = marked.map(|p| format!("{} · {}", p.text, p.translation));
                    let selected = this.audio_clip_mode
                        && this
                            .loop_a
                            .zip(this.loop_b)
                            .is_some_and(|(a, b)| word.start < b && word.end > a);
                    line = line.child(
                        div()
                            .id(("audio-word", i * 100000 + j))
                            .text_size(px(this.audio_font_size as f32))
                            .px_0p5()
                            .rounded_sm()
                            .cursor_pointer()
                            .text_color(if current {
                                cx.theme().primary
                            } else {
                                cx.theme().foreground
                            })
                            .when(current || selected, |v| v.bg(cx.theme().accent))
                            .when(marked.is_some(), |v| {
                                v.border_b_2().border_color(rgb(0xd8a947))
                            })
                            .when_some(tip, |v, tip| {
                                v.tooltip(move |window, cx| {
                                    Tooltip::new(tip.clone()).build(window, cx)
                                })
                            })
                            .hover(|v| v.bg(cx.theme().accent))
                            .child(word.text.clone())
                            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                                this.handle(
                                    Action::Word(
                                        word.start,
                                        word.end,
                                        word.text.clone(),
                                        event.modifiers().shift,
                                    ),
                                    window,
                                    cx,
                                );
                            })),
                    );
                }
                div()
                    .id(("audio-cue", i))
                    .w_full()
                    .flex()
                    .gap_3()
                    .px_5()
                    .py_3()
                    .border_l_2()
                    .border_color(if active {
                        cx.theme().primary
                    } else {
                        cx.theme().background
                    })
                    .child(
                        div()
                            .w(px(40.))
                            .flex_shrink_0()
                            .pt_1()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(clock(cue.start)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(line)
                            .when(!translation.is_empty(), |v| {
                                v.child(
                                    div()
                                        .text_size(px((this.audio_font_size as f32 - 2.).max(12.)))
                                        .text_color(cx.theme().muted_foreground)
                                        .child(translation),
                                )
                            }),
                    )
                    .child(
                        this.button(
                            format!("audio-row-copy-{i}"),
                            "复制",
                            Action::AudioCopySubtitle(
                                this.current_audio.clone(),
                                Some(cue.clone()),
                            ),
                            cx,
                        )
                        .ghost(),
                    )
                    .into_any_element()
            }),
        )
        .flex_1()
        .min_h_0()
        .w_full();
        let reading = {
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .overflow_hidden()
                .when(
                    !self.audio_blind.enabled && self.audio_show_current && active.is_some(),
                    |v| {
                        v.child(
                            div()
                                .px_5()
                                .py_3()
                                .bg(cx.theme().accent)
                                .text_size(px(self.audio_font_size as f32))
                                .child(self.cues[active.unwrap()].text.clone()),
                        )
                    },
                )
                .when(self.cues.is_empty(), |v| {
                    v.child(
                        div()
                            .flex_1()
                            .p_8()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(if available {
                                "在右上角“字幕”菜单导入字幕或生成原文，开始逐句学习。"
                            } else {
                                "选择练习组和音频后开始学习，也可前往“管理音频”导入材料。"
                            }),
                    )
                })
                .when(!self.cues.is_empty(), |v| v.child(transcript))
                .when(self.audio_blind.enabled, |v| v.child(self.blind_panel(cx)))
                .into_any_element()
        };
        let valid = self.loop_a.zip(self.loop_b).is_some_and(|(a, b)| b > a);
        let rates = [0.6, 0.7, 0.8, 1., 1.2, 1.25, 1.5, 2.]
            .into_iter()
            .map(|r| {
                (
                    format!("{r}×"),
                    Action::Rate(r),
                    (self.rate - r).abs() < 0.001,
                    false,
                )
            })
            .collect();
        let repeat = ["不循环", "单曲循环", "A–B 循环", "组循环", "单次连播"]
            .into_iter()
            .enumerate()
            .map(|(i, n)| {
                (
                    n.into(),
                    Action::AudioRepeat(i),
                    self.repeat == i,
                    i == 2 && !valid,
                )
            })
            .collect();
        let mut footer = div()
            .flex()
            .flex_col()
            .gap_1()
            .px_4()
            .py_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().muted)
            .flex_shrink_0()
            .when(available, |v| {
                v.child(self.waveform_view(cx))
                    .child(self.video_timeline(cx))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        self.player_icon(
                            "audio-back10",
                            "rewind",
                            "后退 10 秒",
                            Action::Seek(-10.),
                            cx,
                        )
                        .disabled(!available),
                    )
                    .child(
                        self.player_icon(
                            "audio-play",
                            "play",
                            if self.playback.paused {
                                "播放 · 空格"
                            } else {
                                "暂停 · 空格"
                            },
                            Action::Play,
                            cx,
                        )
                        .icon(
                            Icon::new(IconName::ChevronDown).path(if self.playback.paused {
                                "ui/play.svg"
                            } else {
                                "ui/pause.svg"
                            }),
                        )
                        .w(px(40.))
                        .h(px(40.))
                        .rounded_full()
                        .primary()
                        .disabled(!available),
                    )
                    .child(
                        self.player_icon(
                            "audio-forward10",
                            "forward",
                            "前进 10 秒",
                            Action::Seek(10.),
                            cx,
                        )
                        .disabled(!available),
                    )
                    .child(div().text_xs().child(if available {
                        format!(
                            "{} / {}",
                            clock(self.playback.position),
                            clock(self.playback.duration)
                        )
                    } else {
                        "00:00 / 00:00".into()
                    }))
                    .child(self.player_menu("audio-speed", format!("{}× ▾", self.rate), rates, cx))
                    .child(div().flex_1())
                    .child(self.player_menu(
                        "audio-repeat",
                        format!(
                            "{} ▾",
                            ["不循环", "单曲循环", "A–B 循环", "组循环", "单次连播"][self.repeat]
                        ),
                        repeat,
                        cx,
                    ))
                    .child(
                        self.button(
                            "audio-clip-mode",
                            if self.audio_clip_mode && !self.audio_blind.enabled {
                                "截取片段 ✓"
                            } else {
                                "截取片段"
                            },
                            Action::AudioClipMode,
                            cx,
                        )
                        .selected(self.audio_clip_mode)
                        .disabled(!available || self.audio_blind.enabled),
                    )
                    .child(
                        self.button(
                            "audio-highlights",
                            if self.key_points.is_empty() {
                                "✦ 标重点"
                            } else {
                                "清除重点"
                            },
                            if self.key_points.is_empty() {
                                Action::Highlight
                            } else {
                                Action::ClearHighlights
                            },
                            cx,
                        )
                        .ghost()
                        .disabled(
                            self.busy.is_some() || self.cues.is_empty() || self.audio_blind.enabled,
                        ),
                    )
                    .child(self.player_icon(
                        "audio-mute",
                        if self.volume == 0. { "muted" } else { "volume" },
                        "静音 / 恢复音量",
                        Action::Mute,
                        cx,
                    ))
                    .child(self.player_volume(cx)),
            );
        if self.audio_clip_mode && !self.audio_blind.enabled {
            footer = footer.child(
                div()
                    .flex()
                    .gap_2()
                    .items_center()
                    .flex_wrap()
                    .child(self.button(
                        "audio-mark-a",
                        format!(
                            "A {}",
                            self.loop_a.map(clip_clock).unwrap_or_else(|| "起点".into())
                        ),
                        Action::Mark(true),
                        cx,
                    ))
                    .child(self.button(
                        "audio-mark-b",
                        format!(
                            "B {}",
                            self.loop_b.map(clip_clock).unwrap_or_else(|| "终点".into())
                        ),
                        Action::Mark(false),
                        cx,
                    ))
                    .child(
                        self.button("audio-clear-ab", "清除", Action::VideoClearLoop, cx)
                            .ghost(),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("点击单词选片段；Shift 点击扩展，拖动波形 A/B 调整"),
                    )
                    .child(self.button(
                        "audio-clip-editor",
                        if self.audio_tools_open {
                            "收起编辑"
                        } else {
                            "编辑片段"
                        },
                        Action::AudioTools,
                        cx,
                    ))
                    .child(
                        self.button("audio-save-clip", "保存为闪卡", Action::Clip, cx)
                            .primary()
                            .disabled(!valid || self.busy.is_some()),
                    ),
            );
            if self.audio_tools_open {
                footer = footer.child(
                    div()
                        .flex()
                        .gap_3()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(self.field("front", "片段原文")),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(self.field("back", "片段翻译")),
                        ),
                );
            }
        }
        div()
            .w_full()
            .h(px(
                (f32::from(window.viewport_size().height) - 206.).max(360.)
            ))
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded_2xl()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(header)
            .child(reading)
            .child(footer)
            .into_any_element()
    }
    fn transport(&self, cx: &Context<Self>) -> AnyElement {
        if self.page == "video" {
            return self.video_controls(cx);
        }
        div()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .when(self.page != "video", |v| v.rounded_lg())
            .bg(if self.page == "video" {
                rgb(0x14241e).into()
            } else {
                cx.theme().muted
            })
            .when(self.page == "video", |v| v.text_color(rgb(0xe8f0eb)))
            .when(self.page == "video", |v| v.child(self.video_timeline(cx)))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .flex_wrap()
                    .items_center()
                    .child(self.transport_button("back10", "−10秒", Action::Seek(-10.), cx))
                    .child(
                        self.transport_button(
                            "play",
                            if self.playback.paused {
                                "播放"
                            } else {
                                "暂停"
                            },
                            Action::Play,
                            cx,
                        )
                        .primary(),
                    )
                    .child(self.transport_button("forward10", "+10秒", Action::Seek(10.), cx))
                    .child(format!(
                        "{} / {}",
                        clock(self.playback.position),
                        clock(self.playback.duration)
                    ))
                    .child(self.transport_button(
                        "repeat",
                        ["不循环", "单曲循环", "A/B循环", "组循环", "单次连播"][self.repeat],
                        Action::Repeat,
                        cx,
                    )),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .flex_wrap()
                    .children([0.6, 0.8, 1., 1.25, 1.5, 2.].into_iter().map(|r| {
                        self.transport_button(
                            format!("speed-{r}"),
                            format!("{r}×"),
                            Action::Rate(r),
                            cx,
                        )
                    }))
                    .child(self.transport_button(
                        "volume-down",
                        "音量 −",
                        Action::Volume(self.volume - 10.),
                        cx,
                    ))
                    .child(self.transport_button(
                        "volume-up",
                        "音量 +",
                        Action::Volume(self.volume + 10.),
                        cx,
                    )),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .flex_wrap()
                    .child(self.transport_button(
                        "point-a",
                        format!("A {}", self.loop_a.map(clock).unwrap_or("未设置".into())),
                        Action::Mark(true),
                        cx,
                    ))
                    .child(self.transport_button(
                        "point-b",
                        format!("B {}", self.loop_b.map(clock).unwrap_or("未设置".into())),
                        Action::Mark(false),
                        cx,
                    ))
                    .child(
                        self.transport_button(
                            "export-clip",
                            "保存 A/B 片段为闪卡",
                            Action::Clip,
                            cx,
                        )
                        .disabled(self.busy.is_some()),
                    ),
            )
            .into_any_element()
    }
}
fn clip_clock(s: f64) -> String {
    let tenths = (s.max(0.) * 10.).round() as u64;
    format!("{}.{:01}", clock(tenths as f64 / 10.), tenths % 10)
}
fn clock(s: f64) -> String {
    let n = s.max(0.) as u64;
    format!("{:02}:{:02}", n / 60, n % 60)
}
impl AppView {
    fn content(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let mut content = div().flex().flex_col().gap_5().w_full();
        match self.page.as_str() {
            "flashcard" | "discover" => {
                let discover = self.page == "discover";
                content = content.child(
                    div()
                        .p_6()
                        .rounded_2xl()
                        .bg(cx.theme().accent)
                        .flex()
                        .justify_between()
                        .items_center()
                        .gap_4()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child(div().text_xs().text_color(cx.theme().primary).child(
                                    if discover {
                                        "EXPLORE & LEARN"
                                    } else {
                                        "A LITTLE, EVERY DAY"
                                    },
                                ))
                                .child(div().text_xl().font_weight(FontWeight::SEMIBOLD).child(
                                    if discover {
                                        "让好奇心，带你学下一句。"
                                    } else {
                                        "把遇见的表达，变成熟悉。"
                                    },
                                )),
                        )
                        .child(
                            div()
                                .text_right()
                                .flex_shrink_0()
                                .child(
                                    div()
                                        .text_2xl()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(cx.theme().primary)
                                        .child(self.cards.len().to_string()),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(if self.due && !discover {
                                            "待学卡片"
                                        } else {
                                            "可学卡片"
                                        }),
                                ),
                        ),
                );
                content = content.child(
                    div()
                        .flex()
                        .gap_2()
                        .flex_wrap()
                        .child(self.button("previous", "上一条", Action::Previous, cx))
                        .child(self.button("next", "下一条", Action::Next, cx).primary())
                        .child(self.button("favorite", "收藏 / 取消", Action::Favorite, cx))
                        .child(format!(
                            "{} / {}",
                            if self.cards.is_empty() {
                                0
                            } else {
                                self.index + 1
                            },
                            self.cards.len()
                        )),
                );
                if discover {
                    content = content.child(
                        div()
                            .flex()
                            .gap_2()
                            .child(self.button(
                                "discover-all",
                                "全部内容",
                                Action::Saved(false),
                                cx,
                            ))
                            .child(self.button(
                                "discover-saved",
                                "我的收藏",
                                Action::Saved(true),
                                cx,
                            )),
                    );
                }
                if !discover {
                    content = content.child(
                        div()
                            .flex()
                            .gap_2()
                            .flex_wrap()
                            .child(self.button("due", "计划复习", Action::ReviewMode(true), cx))
                            .child(self.button("browse", "自由浏览", Action::ReviewMode(false), cx))
                            .children(
                                [
                                    ("all", "全部"),
                                    ("word", "单词"),
                                    ("sentence", "句子"),
                                    ("clip", "听力片段"),
                                    ("video", "视频片段"),
                                    ("ai", "AI 问答"),
                                ]
                                .into_iter()
                                .map(|(kind, label)| {
                                    self.button(
                                        format!("filter-{kind}"),
                                        label,
                                        Action::Filter(kind.into()),
                                        cx,
                                    )
                                }),
                            ),
                    );
                }
                if let Some(card) = self.current_card() {
                    let back = if self.revealed {
                        card.back.clone()
                    } else {
                        "点击翻面查看释义".into()
                    };
                    content = content.child(
                        div()
                            .p_8()
                            .min_h(px(310.))
                            .rounded_2xl()
                            .border_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().background)
                            .shadow_sm()
                            .flex()
                            .flex_col()
                            .gap_6()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .text_xs()
                                            .text_color(cx.theme().primary)
                                            .child(design::icon("flashcard", cx.theme().primary))
                                            .child(match card.kind.as_str() {
                                                "word" => "单词卡片",
                                                "sentence" => "句子卡片",
                                                "clip" => "听力片段",
                                                "video" => "视频片段",
                                                _ => "AI 问答",
                                            }),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(format!(
                                                "{:02}  /  {:02}",
                                                self.index + 1,
                                                self.cards.len()
                                            )),
                                    ),
                            )
                            .child(
                                div()
                                    .py_4()
                                    .text_size(px(if card.kind == "word" { 36. } else { 23. }))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(TextView::markdown(
                                        "card-front",
                                        card.front.clone(),
                                        window,
                                        cx,
                                    )),
                            )
                            .child(
                                div()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(TextView::markdown("card-back", back, window, cx)),
                            )
                            .child(
                                div()
                                    .mt_4()
                                    .pt_5()
                                    .border_t_1()
                                    .border_color(cx.theme().border)
                                    .flex()
                                    .justify_between()
                                    .items_center()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(if self.revealed {
                                                "回想一次，再选择熟悉程度"
                                            } else {
                                                "先回想，再查看答案 · Space"
                                            }),
                                    )
                                    .child(
                                        self.button(
                                            "flip",
                                            if self.revealed {
                                                "收起答案"
                                            } else {
                                                "查看答案  →"
                                            },
                                            Action::Flip,
                                            cx,
                                        )
                                        .primary(),
                                    ),
                            ),
                    );
                    if !card.media.is_empty() {
                        if card.kind == "video" {
                            if let Some(image) = &self.playback.frame {
                                content = content.child(
                                    img(image.clone())
                                        .w_full()
                                        .h(px(300.))
                                        .object_fit(ObjectFit::Contain),
                                );
                            }
                        }
                        content = content.child(self.transport(cx));
                    }
                    if !discover && self.due {
                        content = content.child(
                            div().flex().gap_3().children(
                                [
                                    ("again", "忘了 · 10 分钟"),
                                    ("hard", "模糊"),
                                    ("good", "认识"),
                                ]
                                .into_iter()
                                .map(|(grade, label)| {
                                    self.button(
                                        format!("grade-{grade}"),
                                        label,
                                        Action::Grade(grade.into()),
                                        cx,
                                    )
                                    .disabled(!self.revealed)
                                }),
                            ),
                        );
                    }
                } else {
                    content =
                        content.child(div().p_8().text_lg().child(if self.due && !discover {
                            "今日待复习内容已完成，可切换自由浏览或导入新材料。"
                        } else {
                            "还没有学习内容，请导入词条、句子或带字幕的音频。"
                        }));
                }
                content = content.child(
                    div()
                        .flex()
                        .gap_2()
                        .flex_wrap()
                        .child(self.button(
                            "import-words",
                            "导入文本",
                            Action::Pick("words".into()),
                            cx,
                        ))
                        .child(self.button(
                            "import-dir",
                            "导入目录",
                            Action::Pick("directory".into()),
                            cx,
                        ))
                        .child(self.button(
                            "import-anki",
                            "Anki 导入",
                            Action::Pick("anki".into()),
                            cx,
                        )),
                );
                if !discover {
                    content = content.child(
                        div()
                            .flex()
                            .gap_2()
                            .child(self.button("edit-card", "编辑此卡", Action::EditCard, cx))
                            .child(self.button(
                                "suspend-card",
                                "暂停此卡复习",
                                Action::Suspend(true),
                                cx,
                            ))
                            .child(self.button(
                                "resume-card",
                                "恢复此卡复习",
                                Action::Suspend(false),
                                cx,
                            )),
                    );
                }
                content = content.child(
                    self.button(
                        "toggle-composer",
                        if self.composer_open {
                            "收起句子编辑"
                        } else {
                            "＋ 添加句子卡"
                        },
                        Action::Composer,
                        cx,
                    )
                    .ghost(),
                );
                if self.composer_open || self.editing.is_some() {
                    content = content.child(
                        panel(cx)
                            .child(if self.editing.is_some() {
                                "编辑卡片"
                            } else {
                                "添加句子卡"
                            })
                            .child(self.field("front", "原文"))
                            .child(self.field("back", "翻译"))
                            .child(if self.editing.is_some() {
                                self.button(
                                    "save-edited-card",
                                    "保存修改",
                                    Action::SaveEditedCard,
                                    cx,
                                )
                            } else {
                                self.button("sentence-save", "保存句子卡", Action::SaveSentence, cx)
                            }),
                    );
                }
            }
            "video" => {
                let download_panel = panel(cx)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .p_2()
                                    .rounded_lg()
                                    .bg(cx.theme().accent)
                                    .child(design::icon("video", cx.theme().primary)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child("从 bilibili 开始一堂课"),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child("收藏喜欢的视频，慢慢听懂每一句。"),
                                    ),
                            ),
                    )
                    .child(self.field("bilibili", "完整视频链接或 BV / av 编号"))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                self.button("download", "下载并导入", Action::Download, cx)
                                    .primary()
                                    .disabled(self.busy.is_some()),
                            )
                            .child(
                                self.button("read-parts", "读取分P / 选集", Action::ReadParts, cx)
                                    .disabled(self.busy.is_some()),
                            )
                            .child(
                                self.button("cancel-download", "取消下载", Action::Cancel, cx)
                                    .disabled(self.busy.is_none()),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("自动识别分P；点击选集，未下载的集数会先下载再播放。"),
                    );
                let available = (f32::from(window.viewport_size().width)
                    - if self.navigation_open { 224. } else { 0. }
                    - 32.)
                    .max(300.);
                let stacked = available < 780.;
                let side_width = self
                    .video_panel_width
                    .clamp(300., (available * 0.48).max(300.));
                let picture_height = if stacked {
                    (available * 9. / 16.).clamp(220., 360.)
                } else {
                    ((available - side_width - 12.) * 9. / 16.).clamp(260., 410.)
                };
                let board_height = picture_height + 114.;
                let mut stage = div()
                    .flex_1()
                    .when(stacked, |v| v.w_full().flex_none())
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .rounded_2xl()
                    .overflow_hidden()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(rgb(0x0c1612))
                    .child(
                        div()
                            .flex_shrink_0()
                            .px_4()
                            .py_2()
                            .bg(cx.theme().muted)
                            .flex()
                            .items_center()
                            .flex_wrap()
                            .gap_2()
                            .child(
                                div().flex_1().min_w_0().text_sm().truncate().child(
                                    self.current_path
                                        .as_ref()
                                        .and_then(|p| p.file_name())
                                        .map(|v| v.to_string_lossy().to_string())
                                        .unwrap_or_else(|| "视频播放器".into()),
                                ),
                            )
                            .when_some(
                                self.video_series
                                    .as_ref()
                                    .and_then(|s| s.current.map(|i| (i, s.parts.len()))),
                                |v, (i, count)| {
                                    v.child(self.button(
                                        "player-parts",
                                        format!("P{} / {} ▾", i + 1, count),
                                        Action::VideoPanel(1),
                                        cx,
                                    ))
                                    .child(
                                        self.button(
                                            "player-prev-part",
                                            "‹",
                                            Action::SelectPart(i.saturating_sub(1)),
                                            cx,
                                        )
                                        .tooltip("上一 P")
                                        .disabled(self.busy.is_some() || i == 0),
                                    )
                                    .child(
                                        self.button(
                                            "player-next-part",
                                            "›",
                                            Action::SelectPart(i + 1),
                                            cx,
                                        )
                                        .tooltip("下一 P")
                                        .disabled(self.busy.is_some() || i + 1 >= count),
                                    )
                                },
                            )
                            .child(
                                self.player_menu(
                                    "video-collection-picker",
                                    format!("切换收藏 · {}", self.videos.items.len()),
                                    self.videos
                                        .items
                                        .iter()
                                        .map(|v| {
                                            (
                                                if self
                                                    .videos
                                                    .items
                                                    .iter()
                                                    .filter(|other| other.title == v.title)
                                                    .count()
                                                    > 1
                                                {
                                                    format!(
                                                        "{} · {}",
                                                        v.title,
                                                        v.path
                                                            .parent()
                                                            .and_then(|p| p.file_name())
                                                            .unwrap_or_default()
                                                            .to_string_lossy()
                                                    )
                                                } else {
                                                    v.title.clone()
                                                },
                                                Action::VideoSelect(v.id.clone()),
                                                self.videos.selected.as_ref() == Some(&v.id),
                                                self.busy.is_some() || v.playable_path().is_none(),
                                            )
                                        })
                                        .collect(),
                                    cx,
                                ),
                            )
                            .child(self.button(
                                "video-more",
                                if self.video_tools_open {
                                    "收起更多 ▴"
                                } else {
                                    "更多 ▾"
                                },
                                Action::VideoTools,
                                cx,
                            )),
                    );
                stage = stage.child(self.video_picture(Some(px(picture_height)), cx));
                stage = stage.child(self.transport(cx));
                let count = self.video_series.as_ref().map_or(0, |s| s.parts.len());
                let tabs = div()
                    .flex()
                    .items_center()
                    .flex_wrap()
                    .gap_1()
                    .p_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .children(
                        [
                            (3, format!("我的视频 · {}", self.videos.items.len())),
                            (0, format!("字幕 · {}", self.video_cues.len())),
                            (1, format!("选集 · {count}")),
                            (2, "AI 提问".into()),
                        ]
                        .into_iter()
                        .map(|(tab, label)| {
                            self.button(
                                format!("video-tab-{tab}"),
                                label,
                                Action::VideoPanel(tab),
                                cx,
                            )
                            .selected(self.video_tab == tab)
                            .flex_none()
                        }),
                    );
                let mut study = div()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .rounded_2xl()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().background)
                    .when(stacked, |v| v.w_full().h(px(480.)))
                    .when(!stacked, |v| {
                        v.w(px(side_width)).h(px(board_height)).flex_shrink_0()
                    })
                    .child(tabs);
                match self.video_tab {
                    3 => {
                        study = study.child(self.video_collection_panel(cx));
                    }
                    0 => {
                        let transcript = list(
                            self.video_scroll.clone(),
                            cx.processor(|this, i: usize, _, cx| {
                                let Some(cue) = this.video_cues.get(i) else {
                                    return div().into_any_element();
                                };
                                let active = Some(i)
                                    == video_subtitles::active(
                                        &this.video_cues,
                                        this.playback.position,
                                    );
                                div()
                                    .id(("video-cue", i))
                                    .w_full()
                                    .px_3()
                                    .py_2()
                                    .flex()
                                    .gap_2()
                                    .cursor_pointer()
                                    .border_l_2()
                                    .border_color(if active {
                                        cx.theme().primary
                                    } else {
                                        cx.theme().background
                                    })
                                    .bg(if active {
                                        cx.theme().accent
                                    } else {
                                        cx.theme().background
                                    })
                                    .child(
                                        div()
                                            .w(px(38.))
                                            .flex_shrink_0()
                                            .pt_1()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(clock(cue.start)),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .flex()
                                            .flex_col()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .whitespace_normal()
                                                    .child(cue.text.clone()),
                                            )
                                            .child(
                                                div()
                                                    .flex()
                                                    .gap_2()
                                                    .child(
                                                        this.button(
                                                            format!("video-cue-copy-{i}"),
                                                            "复制",
                                                            Action::VideoCopySubtitle(Some(i)),
                                                            cx,
                                                        )
                                                        .ghost()
                                                        .on_mouse_down(
                                                            MouseButton::Left,
                                                            |_, _, cx| cx.stop_propagation(),
                                                        ),
                                                    )
                                                    .child(
                                                        this.button(
                                                            format!("video-cue-ask-{i}"),
                                                            "问 AI",
                                                            Action::VideoAsk(video_qa::Scope::Cue(
                                                                i,
                                                            )),
                                                            cx,
                                                        )
                                                        .ghost()
                                                        .disabled(this.chat_running)
                                                        .on_mouse_down(
                                                            MouseButton::Left,
                                                            |_, _, cx| cx.stop_propagation(),
                                                        ),
                                                    ),
                                            ),
                                    )
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.handle(Action::VideoPlayCue(i), window, cx)
                                    }))
                                    .into_any_element()
                            }),
                        )
                        .flex_1()
                        .min_h_0()
                        .w_full();
                        study = study.child(div().px_3().py_2().flex().items_center().gap_1().flex_wrap()
                            .child(self.button("video-follow", if self.video_follow { "跟随 ✓" } else { "跟随" }, Action::VideoFollow, cx).ghost())
                            .child(self.button("video-import-english", "导入英文字幕", Action::Pick("video-subtitle".into()),cx).ghost().disabled(self.current_path.is_none()||self.busy.is_some()))
                            .child(self.button("video-translate-zh", "英文生成中文", Action::VideoTranslate,cx).disabled(self.video_cues.is_empty()||self.busy.is_some()||self.current_path.is_none()))
                            .child(div().flex_1())
                            .child(self.button("video-copy-all", "复制全部", Action::VideoCopySubtitle(None), cx).ghost().disabled(self.video_cues.is_empty()))
                            .child(self.button("video-transcribe", if self.video_cues.is_empty() { "生成字幕" } else { "重新生成" }, Action::VideoTranscribe, cx)
                                .disabled(self.busy.is_some() || self.current_path.is_none())))
                            .when(self.video_cues.is_empty(), |v| v.child(div().p_5().text_sm().text_color(cx.theme().muted_foreground)
                                .child("生成或导入字幕后，点击句子即可播放。复制与 AI 提问位于每句下方。")))
                            .when(!self.video_cues.is_empty(), |v| v.child(transcript));
                    }
                    1 => {
                        if let Some(series) = &self.video_series {
                            let current = series.current;
                            let parts = uniform_list(
                                "bilibili-parts",
                                count,
                                cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                                    range
                                        .map(|i| {
                                            let series = this.video_series.as_ref().unwrap();
                                            let part = &series.parts[i];
                                            let playing = series.current == Some(i)
                                                && part.path.as_ref() == this.current_path.as_ref();
                                            div()
                                                .id(("bili-part", i))
                                                .h(px(64.))
                                                .w_full()
                                                .px_3()
                                                .py_2()
                                                .flex()
                                                .items_center()
                                                .gap_2()
                                                .cursor_pointer()
                                                .bg(if playing {
                                                    cx.theme().accent
                                                } else {
                                                    cx.theme().background
                                                })
                                                .child(
                                                    div()
                                                        .w(px(32.))
                                                        .flex_shrink_0()
                                                        .text_sm()
                                                        .text_color(cx.theme().primary)
                                                        .child(format!("P{}", part.number)),
                                                )
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .min_w_0()
                                                        .flex()
                                                        .flex_col()
                                                        .gap_1()
                                                        .child(
                                                            div()
                                                                .text_sm()
                                                                .truncate()
                                                                .child(part.title.clone()),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_xs()
                                                                .text_color(
                                                                    cx.theme().muted_foreground,
                                                                )
                                                                .child(format!(
                                                                    "{} · {}",
                                                                    clock(part.duration),
                                                                    if playing {
                                                                        "正在播放"
                                                                    } else if series
                                                                        .cached(i)
                                                                        .is_some()
                                                                    {
                                                                        "已下载"
                                                                    } else {
                                                                        "点击下载并播放"
                                                                    }
                                                                )),
                                                        ),
                                                )
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        if this.busy.is_none() {
                                                            this.handle(
                                                                Action::SelectPart(i),
                                                                window,
                                                                cx,
                                                            );
                                                        }
                                                    },
                                                ))
                                                .into_any_element()
                                        })
                                        .collect::<Vec<_>>()
                                }),
                            )
                            .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::FitList)
                            .track_scroll(self.parts_scroll.clone())
                            .size_full()
                            .pr_3();
                            study = study
                                .child(
                                    div()
                                        .px_3()
                                        .py_2()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .flex_wrap()
                                        .child(
                                            self.button(
                                                "previous-part",
                                                "上一P",
                                                Action::SelectPart(
                                                    current.unwrap_or(0).saturating_sub(1),
                                                ),
                                                cx,
                                            )
                                            .disabled(
                                                self.busy.is_some()
                                                    || current.is_none_or(|i| i == 0),
                                            ),
                                        )
                                        .child(
                                            self.button(
                                                "next-part",
                                                "下一P",
                                                Action::SelectPart(current.unwrap_or(0) + 1),
                                                cx,
                                            )
                                            .disabled(
                                                self.busy.is_some()
                                                    || current.is_none_or(|i| i + 1 >= count),
                                            ),
                                        )
                                        .child(
                                            self.button(
                                                "auto-next-part",
                                                if self.auto_next_part {
                                                    "自动连播 ✓"
                                                } else {
                                                    "自动连播"
                                                },
                                                Action::AutoNextPart,
                                                cx,
                                            )
                                            .ghost(),
                                        ),
                                )
                                .child(
                                    div()
                                        .id("bilibili-parts-viewport")
                                        .relative()
                                        .flex_1()
                                        .min_h_0()
                                        .overflow_hidden()
                                        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                                        .child(parts)
                                        .child(
                                            Scrollbar::vertical(&self.parts_scroll)
                                                .scrollbar_show(ScrollbarShow::Always),
                                        ),
                                );
                        } else {
                            study = study
                                .child(
                                    div()
                                        .p_5()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("从 bilibili 导入或读取分P后，在这里选择集数。"),
                                )
                                .child(self.button(
                                    "show-import",
                                    "导入视频",
                                    Action::VideoTools,
                                    cx,
                                ));
                        }
                    }
                    _ => study = study.child(self.assistant_view(window, cx)),
                }
                let divider = div()
                    .id("video-panel-resize")
                    .w(px(12.))
                    .h(px(board_height))
                    .flex_shrink_0()
                    .cursor(CursorStyle::ResizeLeftRight)
                    .flex()
                    .justify_center()
                    .items_center()
                    .child(
                        div()
                            .w(px(3.))
                            .h(px(40.))
                            .rounded_full()
                            .bg(cx.theme().border),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, _, _| {
                            this.video_resize = Some((event.position.x, side_width));
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, _| this.video_resize = None),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this, _, _, _| this.video_resize = None),
                    );
                content = content.child(
                    div()
                        .flex()
                        .items_start()
                        .when(stacked, |v| v.flex_col().gap_3())
                        .child(stage)
                        .when(!stacked, |v| v.child(divider))
                        .child(study),
                );
                if self.video_tools_open || self.current_path.is_none() {
                    content = content.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(self.button(
                                "open-video",
                                "打开本地视频",
                                Action::Pick("video".into()),
                                cx,
                            ))
                            .child(
                                self.button(
                                    "video-export-srt",
                                    "导出 SRT",
                                    Action::VideoSubtitleExport,
                                    cx,
                                )
                                .disabled(self.video_cues.is_empty()),
                            ),
                    );
                    content = content.child(download_panel);
                    let mut tracks = div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child("字幕轨道 · 主字幕 / 第二字幕")
                        .child(self.button(
                            "video-external-sub",
                            "加载外部字幕",
                            Action::Pick("video-subtitle".into()),
                            cx,
                        ))
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(self.button(
                                    "sub-off",
                                    "关闭主字幕",
                                    Action::Subtitle(0, false),
                                    cx,
                                ))
                                .child(self.button(
                                    "secondary-off",
                                    "关闭第二字幕",
                                    Action::Subtitle(0, true),
                                    cx,
                                )),
                        );
                    for track in &self.playback.tracks {
                        if track["type"] == "sub" {
                            let key = track["id"].as_i64().unwrap_or(0);
                            let label = track["title"]
                                .as_str()
                                .or(track["lang"].as_str())
                                .unwrap_or("字幕");
                            tracks = tracks.child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(self.button(
                                        format!("sub-{key}"),
                                        format!("主：{label}"),
                                        Action::Subtitle(key, false),
                                        cx,
                                    ))
                                    .child(self.button(
                                        format!("secondary-{key}"),
                                        format!("副：{label}"),
                                        Action::Subtitle(key, true),
                                        cx,
                                    )),
                            );
                        }
                    }
                    content = content
                        .child(tracks)
                        .child(self.field("front", "片段原文"))
                        .child(self.field("back", "片段翻译"));
                }
            }
            "library" => {
                content = content.child(self.audio_library_view(window, cx));
            }
            "listening" => {
                content = content.child(self.audio_study(window, cx));
            }
            "speaking" => {
                content = content
                    .child(self.field("reference", "朗读参考原文"))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .flex_wrap()
                            .child(
                                self.button("record", "开始录音", Action::Record, cx)
                                    .primary()
                                    .disabled(self.recording.is_some()),
                            )
                            .child(
                                self.button("stop-record", "停止录音", Action::StopRecord, cx)
                                    .disabled(self.recording.is_none()),
                            )
                            .child(self.button(
                                "pick-record",
                                "选择已有录音",
                                Action::Pick("recording".into()),
                                cx,
                            ))
                            .child(
                                self.button("score-record", "转写并对齐评分", Action::Score, cx)
                                    .disabled(self.busy.is_some()),
                            ),
                    );
                if let Some(path) = &self.recorded {
                    content = content.child(
                        path.file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string(),
                    );
                }
                if !self.output.is_empty() {
                    content = content.child(TextView::markdown(
                        "speaking-output",
                        self.output.clone(),
                        window,
                        cx,
                    ));
                }
                for (i, record) in documents::records(&self.lib, "speaking")
                    .unwrap_or_default()
                    .into_iter()
                    .rev()
                    .take(20)
                    .enumerate()
                {
                    content = content.child(
                        panel(cx)
                            .child(format!(
                                "记录 {} · {:.0}%",
                                i + 1,
                                record["accuracy"].as_f64().unwrap_or(0.) * 100.
                            ))
                            .child(record["reference"].as_str().unwrap_or("").to_string())
                            .child(record["transcript"].as_str().unwrap_or("").to_string()),
                    );
                }
            }
            "pdf" => {
                let mut library = div().flex().gap_2().flex_wrap();
                if let Ok(files) = fs::read_dir(self.lib.root.join("pdf")) {
                    for (i, path) in files
                        .filter_map(Result::ok)
                        .map(|e| e.path())
                        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")))
                        .enumerate()
                    {
                        library = library.child(
                            self.button(
                                format!("pdf-library-{i}"),
                                path.file_stem()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .to_string(),
                                Action::PdfOpen(path),
                                cx,
                            ),
                        );
                    }
                }
                content = content.child(library);
                content = content.child(
                    div()
                        .flex()
                        .gap_2()
                        .flex_wrap()
                        .child(
                            self.button("import-pdf", "导入 PDF", Action::Pick("pdf".into()), cx)
                                .primary(),
                        )
                        .child(
                            self.button("pdf-prev", "上一页", Action::PdfPage(-1), cx)
                                .disabled(self.busy.is_some()),
                        )
                        .child(
                            self.button("pdf-next", "下一页", Action::PdfPage(1), cx)
                                .disabled(self.busy.is_some()),
                        )
                        .child(self.button("pdf-smaller", "缩小", Action::PdfZoom(-200), cx))
                        .child(self.button("pdf-larger", "放大", Action::PdfZoom(200), cx))
                        .child(format!("{}%", self.pdf_width / 10))
                        .child(format!("{} / {}", self.pdf_page + 1, self.pdf_pages)),
                );
                if let Some(image) = &self.pdf_image {
                    content = content.child(
                        img(image.clone())
                            .w(relative(self.pdf_width as f32 / 1000.))
                            .flex_shrink_0()
                            .object_fit(ObjectFit::Contain),
                    );
                }
                for (i, (title, page)) in self.pdf_bookmarks.iter().enumerate() {
                    content = content.child(self.button(
                        format!("bookmark-{i}"),
                        title.clone(),
                        Action::PdfPage(*page as i32 - self.pdf_page as i32),
                        cx,
                    ));
                }
            }
            "ocr" => {
                content = content.child(
                    self.button(
                        "ocr-image",
                        "导入图片并识字",
                        Action::Pick("ocr".into()),
                        cx,
                    )
                    .primary()
                    .disabled(self.busy.is_some()),
                );
                if !self.output.is_empty() {
                    content = content.child(TextView::markdown(
                        "ocr-output",
                        self.output.clone(),
                        window,
                        cx,
                    ));
                }
                for record in documents::records(&self.lib, "ocr")
                    .unwrap_or_default()
                    .into_iter()
                    .rev()
                    .take(30)
                {
                    content = content.child(
                        panel(cx)
                            .child(record["name"].as_str().unwrap_or("图片").to_string())
                            .child(record["text"].as_str().unwrap_or("").to_string()),
                    );
                }
            }
            "graph" => {
                let graph: Value =
                    serde_json::from_str(include_str!("../../clarora-app/assets/graph-data.json"))
                        .unwrap_or_default();
                let nodes = graph["nodes"].as_array().cloned().unwrap_or_default();
                content = content.child(Input::new(&self.inputs["search"]));
                let mut lists = std::collections::BTreeSet::new();
                for n in &nodes {
                    if let Some(list) = n["list_num"].as_i64() {
                        lists.insert(list);
                    }
                }
                let mut filters = div().flex().gap_2().flex_wrap().child(self.button(
                    "graph-all",
                    "全部",
                    Action::GraphList(0),
                    cx,
                ));
                for list in lists {
                    filters = filters.child(self.button(
                        format!("graph-list-{list}"),
                        format!("List {list}"),
                        Action::GraphList(list),
                        cx,
                    ));
                }
                content = content.child(filters);
                let query = self.value("search", cx).to_lowercase();
                let selected = nodes.iter().find(|n| n["id"] == self.graph_node);
                if let Some(node) = selected {
                    let mut detail = panel(cx).child(
                        div()
                            .text_3xl()
                            .child(node["label"].as_str().unwrap_or("").to_string()),
                    );
                    for key in ["pron", "category", "kaofa", "etymology", "mnemonic"] {
                        if let Some(text) = node[key].as_str().filter(|s| !s.is_empty()) {
                            detail = detail.child(text.to_string());
                        }
                    }
                    for sense in node["senses"].as_array().into_iter().flatten() {
                        detail = detail.child(format!(
                            "{}  {}",
                            sense["en"].as_str().unwrap_or(""),
                            sense["zh"].as_str().unwrap_or("")
                        ));
                    }
                    let mut related = div().flex().gap_2().flex_wrap();
                    for (i, edge) in graph["links"].as_array().into_iter().flatten().enumerate() {
                        let other = if edge["source"] == self.graph_node {
                            &edge["target"]
                        } else if edge["target"] == self.graph_node {
                            &edge["source"]
                        } else {
                            continue;
                        };
                        if let Some(target) = nodes.iter().find(|n| n["id"] == *other) {
                            related = related.child(self.button(
                                format!("graph-edge-{i}"),
                                format!(
                                    "{} · {}",
                                    edge["link_type"].as_str().unwrap_or("关联"),
                                    target["label"].as_str().unwrap_or("")
                                ),
                                Action::GraphSelect(other.as_str().unwrap_or("").into()),
                                cx,
                            ));
                        }
                    }
                    content = content.child(detail.child(related));
                }
                let mut nodes_view = div().flex().gap_2().flex_wrap();
                for node in nodes.iter().filter(|n| {
                    (self.graph_list == 0 || n["list_num"] == self.graph_list)
                        && n["label"]
                            .as_str()
                            .unwrap_or("")
                            .to_lowercase()
                            .contains(&query)
                }) {
                    nodes_view = nodes_view.child(self.button(
                        format!("graph-node-{}", node["id"]),
                        node["label"].as_str().unwrap_or("").to_string(),
                        Action::GraphSelect(node["id"].as_str().unwrap_or("").into()),
                        cx,
                    ));
                }
                content = content.child(nodes_view);
                for row in self
                    .lib
                    .list("words")
                    .unwrap_or_default()
                    .iter()
                    .filter(|r| text(r, "word").to_lowercase().contains(&query))
                    .take(300)
                {
                    content = content.child(
                        div()
                            .flex()
                            .gap_4()
                            .child(text(row, "word"))
                            .child(text(row, "meaning")),
                    );
                }
            }
            "stats" => {
                let rows = self.lib.list("study_stats").unwrap_or_default();
                let listening: f64 = rows
                    .iter()
                    .filter_map(|r| r["listen_seconds"].as_f64())
                    .sum();
                let review: f64 = rows.iter().filter_map(|r| r["review_count"].as_f64()).sum();
                let pomodoro: f64 = rows
                    .iter()
                    .filter_map(|r| r["pomodoro_count"].as_f64())
                    .sum();
                content = content.child(
                    div()
                        .flex()
                        .gap_4()
                        .flex_wrap()
                        .child(
                            panel(cx)
                                .child("累计听力")
                                .child(format!("{:.1} 分钟", listening / 60.)),
                        )
                        .child(panel(cx).child("复习卡片").child(format!("{review:.0} 张")))
                        .child(
                            panel(cx)
                                .child("完成番茄钟")
                                .child(format!("{pomodoro:.0} 轮")),
                        ),
                );
                let mut heatmap = div().flex().gap_1().flex_wrap().max_w(px(650.));
                for day in (0..119).rev() {
                    let date = (chrono::Local::now() - chrono::Duration::days(day))
                        .format("%Y-%m-%d")
                        .to_string();
                    let count = rows
                        .iter()
                        .find(|r| text(r, "day") == date)
                        .and_then(|r| r["review_count"].as_f64())
                        .unwrap_or(0.);
                    heatmap =
                        heatmap.child(div().size(px(20.)).rounded_sm().bg(rgb(if count > 30. {
                            0x17604c
                        } else if count > 10. {
                            0x459d7a
                        } else if count > 0. {
                            0x9ed6b5
                        } else {
                            0xe6e9e4
                        })));
                }
                content = content.child("最近 17 周复习热力图").child(heatmap);
                let mut listening_chart = div().flex().gap_1().items_end().h(px(160.));
                let days = (0..14)
                    .rev()
                    .map(|day| {
                        let date = (chrono::Local::now() - chrono::Duration::days(day))
                            .format("%Y-%m-%d")
                            .to_string();
                        let seconds = rows
                            .iter()
                            .find(|r| text(r, "day") == date)
                            .and_then(|r| r["listen_seconds"].as_f64())
                            .unwrap_or(0.);
                        (date, seconds)
                    })
                    .collect::<Vec<_>>();
                let maximum = days.iter().map(|(_, seconds)| *seconds).fold(60., f64::max);
                for (day, seconds) in days {
                    listening_chart = listening_chart.child(
                        div()
                            .flex()
                            .flex_col()
                            .justify_end()
                            .items_center()
                            .w(px(45.))
                            .gap_1()
                            .child(format!("{:.0}m", seconds / 60.))
                            .child(
                                div()
                                    .w(px(24.))
                                    .h(px((seconds / maximum * 100.).max(2.) as f32))
                                    .bg(cx.theme().primary),
                            )
                            .child(day[5..].to_string()),
                    );
                }
                content = content.child("最近 14 天听力时长").child(listening_chart);
                let attempts = documents::records(&self.lib, "speaking").unwrap_or_default();
                let mut trend = div().flex().gap_2().items_end().h(px(140.));
                for record in attempts.iter().rev().take(20).rev() {
                    let accuracy = record["score"]["accuracy"]
                        .as_f64()
                        .or_else(|| record["accuracy"].as_f64().map(|v| v * 100.))
                        .unwrap_or(0.);
                    trend = trend.child(
                        div()
                            .flex()
                            .flex_col()
                            .justify_end()
                            .items_center()
                            .gap_1()
                            .child(format!("{accuracy:.0}%"))
                            .child(
                                div()
                                    .w(px(20.))
                                    .h(px(accuracy.clamp(2., 100.) as f32))
                                    .bg(cx.theme().primary),
                            ),
                    );
                }
                content = content.child("最近 20 次跟读准确度").child(trend);

                for row in rows.iter().rev().take(14) {
                    content = content.child(format!(
                        "{} · 复习 {} · 听力 {:.1} 分钟",
                        text(row, "day"),
                        text(row, "review_count"),
                        row["listen_seconds"].as_f64().unwrap_or(0.) / 60.
                    ));
                }
            }
            "meditation" => {
                content = content.child(
                    panel(cx)
                        .items_center()
                        .p_10()
                        .child(div().text_3xl().child(clock(self.timer_remaining as f64)))
                        .child(if self.timer_focus {
                            "专注时间"
                        } else {
                            "休息时间"
                        })
                        .child(
                            div()
                                .flex()
                                .gap_3()
                                .child(self.button(
                                    "timer-pause",
                                    if self.timer_paused {
                                        "继续"
                                    } else {
                                        "暂停"
                                    },
                                    Action::TimerPause,
                                    cx,
                                ))
                                .child(self.button("timer-reset", "重置", Action::TimerReset, cx)),
                        )
                        .child(div().flex().gap_2().flex_wrap().children(
                            [5u64, 10, 15, 25, 45, 60].into_iter().map(|minutes| {
                                self.button(
                                    format!("timer-{minutes}"),
                                    format!("{minutes} 分钟"),
                                    Action::Timer(minutes * 60),
                                    cx,
                                )
                            }),
                        ))
                        .child(self.button(
                            "background-sound",
                            "选择本机背景音乐",
                            Action::Pick("background".into()),
                            cx,
                        )),
                );
            }
            "settings" => {
                content = content.child(
                    panel(cx)
                        .child("本机资料与外观")
                        .child(self.lib.root.to_string_lossy().to_string())
                        .child(self.button("theme", "切换深浅主题", Action::Theme, cx))
                        .child(self.field("daily_limit", "每日新卡上限")),
                );
                // Read only the saved configuration, without accessing the credential store
                // during rendering. Keep draft selection distinct from the active engine.
                let saved_ai = self.lib.setting("ai_config").ok().and_then(|raw| {
                    if raw.is_empty() {
                        Some(ai::Config {
                            engine: "local".into(),
                            local_model: "base.en".into(),
                            ..Default::default()
                        })
                    } else {
                        serde_json::from_str::<ai::Config>(&raw).ok()
                    }
                });
                let draft_model = self.value("local_model", cx);
                let draft_local = self.value("engine", cx) == "local";
                let active_model = saved_ai
                    .as_ref()
                    .filter(|c| c.engine == "local")
                    .map(|c| c.local_model.as_str());
                let selection_summary = match saved_ai.as_ref() {
                    Some(c) if c.engine == "local" => {
                        format!("当前生效：本地模型 {}", c.local_model)
                    }
                    Some(_) => "当前生效：远端转写 API".into(),
                    None => "无法读取已保存的模型设置".into(),
                };
                let pending = draft_local && active_model != Some(draft_model.as_str());
                let mut models = panel(cx)
                    .child(div().text_lg().child("本地语音识别模型 · ASR"))
                    .child("用于音频、视频字幕及跟读转写。下载后可离线识别；聊天与图片识字仍使用 API。")
                    .child("中文视频请选择多语言 Whisper 或 SenseVoice；.en 仅英语。体积为下载量，不是运行内存。")
                    .child("操作：下载 → 选用 → 点击下方「保存 AI 与学习设置」。已下载仅表示文件存在。 ")
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(selection_summary))
                    .when(pending, |v| v.child(div().text_color(cx.theme().primary).child(format!("已选择：{draft_model} · 待保存，保存后生效"))));
                if let Some(progress) = &self.model_progress {
                    models = models.child(
                        div()
                            .p_4()
                            .rounded_lg()
                            .bg(cx.theme().accent)
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("模型下载进度（当前文件）"),
                            )
                            .child(div().text_sm().child(progress.label()))
                            .when_some(progress.percent(), |v, p| {
                                v.child(Progress::new().value(p).bg(cx.theme().primary))
                            })
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!("下载源：{}", progress.source)),
                            )
                            .child(self.button("cancel-model", "取消下载", Action::Cancel, cx)),
                    );
                }
                for (model, name, details) in [
                    (
                        "tiny.en",
                        "OpenAI Whisper Tiny.en",
                        "仅英语 · 39M 参数 · 约 75 MiB · 最小的英语版本",
                    ),
                    (
                        "base.en",
                        "OpenAI Whisper Base.en",
                        "仅英语 · 74M 参数 · 约 142 MiB · 默认表单模型",
                    ),
                    (
                        "small.en",
                        "OpenAI Whisper Small.en",
                        "仅英语 · 244M 参数 · 约 466 MiB · 更大的英语版本",
                    ),
                    (
                        "base",
                        "OpenAI Whisper Base · 多语言",
                        "含中文、英语、日语等 · 74M 参数 · 约 142 MiB",
                    ),
                    (
                        "small",
                        "OpenAI Whisper Small · 多语言",
                        "含中文、英语、日语等 · 244M 参数 · 约 466 MiB",
                    ),
                    (
                        "sense-voice",
                        "FunAudioLLM SenseVoiceSmall · 2024-07-17",
                        "普通话、英语、日语、韩语、粤语 · INT8 ONNX 模型 + tokens.txt",
                    ),
                ] {
                    let present = if model == "sense-voice" {
                        self.lib
                            .root
                            .join("models/sense-voice/model.int8.onnx")
                            .is_file()
                            && self
                                .lib
                                .root
                                .join("models/sense-voice/tokens.txt")
                                .is_file()
                    } else {
                        self.lib
                            .root
                            .join("models")
                            .join(format!("ggml-{model}.bin"))
                            .is_file()
                    };
                    let selected = draft_local && draft_model == model;
                    let active = active_model == Some(model);
                    models = models.child(
                        div()
                            .p_3()
                            .rounded_lg()
                            .border_1()
                            .border_color(if selected {
                                cx.theme().primary
                            } else {
                                cx.theme().border
                            })
                            .when(selected, |v| v.bg(cx.theme().accent))
                            .flex()
                            .flex_col()
                            .gap_1()
                            .when(active, |v| {
                                v.child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().primary)
                                        .child("✓ 当前生效"),
                                )
                            })
                            .when(selected && !active, |v| {
                                v.child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().primary)
                                        .child("✓ 已选择 · 待保存"),
                                )
                            })
                            .child(
                                div()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(format!("{name}  [{model}]")),
                            )
                            .child(div().text_sm().child(details))
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(
                                        self.button(
                                            format!("model-{model}"),
                                            if present {
                                                "文件已下载"
                                            } else {
                                                "下载模型"
                                            },
                                            Action::Models(model.into()),
                                            cx,
                                        )
                                        .disabled(present || self.busy.is_some()),
                                    )
                                    .child(
                                        self.button(
                                            format!("select-model-{model}"),
                                            if selected {
                                                if active {
                                                    "✓ 已选用"
                                                } else {
                                                    "✓ 已选择 · 待保存"
                                                }
                                            } else {
                                                "选用"
                                            },
                                            Action::SelectModel(model.into()),
                                            cx,
                                        )
                                        .selected(selected)
                                        .disabled(!present || self.busy.is_some() || selected),
                                    ),
                            ),
                    );
                }
                models = models
                    .child("运行要求：FFmpeg；Whisper 需 clarora-asr 或 whisper-cli，SenseVoice 需 clarora-asr + sherpa-onnx。下载模型不会安装推理程序。")
                    .child("本机已实测 Whisper tiny.en；其余模型尚未完成真实推理验收。详细来源、文件路径和安装方法见 GPUI README。 ");
                content = content.child(models);
                content = content.child(
                    panel(cx)
                        .child(div().text_lg().child("AI 服务"))
                        .child(div().text_sm().text_color(cx.theme().muted_foreground)
                            .child("聊天与远端转写共用下面一组设置，模型需支持这两种能力。保存后统一生效。"))
                        .child(self.field("base_url", "共用 API Base URL"))
                        .child(self.field("model", "聊天／远端转写共用模型 ID"))
                        .child(self.field("chat_key", "共用 API Key · 系统凭证库"))
                        .child(self.field("vision_model", "视觉模型（可选，留空使用共用模型）"))
                        .child(
                            self.field("engine", "转写引擎：local = 本机；compatible = 远端 API"),
                        )
                        .child(self.field(
                            "local_model",
                            "本地语音模型 ID（local 引擎使用，选用后需保存）",
                        ))
                        .child(self.field("command", "本机命令导入"))
                        .child(
                            self.button("save-ai", "保存 AI 与学习设置", Action::SaveAi, cx)
                                .primary()
                                .disabled(self.busy.is_some()),
                        ),
                );

                content = content.child(
                    panel(cx)
                        .child(div().text_lg().child("同步与备份"))
                        .child(self.field("provider", "存储类型 s3 / oss"))
                        .child(self.field("endpoint", "Endpoint"))
                        .child(self.field("region", "Region"))
                        .child(self.field("bucket", "Bucket"))
                        .child(self.field("access_key", "Access Key ID"))
                        .child(self.field("secret_key", "Secret Access Key"))
                        .child(self.field("session_token", "Session Token（可选）"))
                        .child(self.field("prefix", "资料库前缀"))
                        .child(self.field("path_style", "S3 路径模式 true / false"))
                        .child(
                            self.button("save-storage", "保存存储配置", Action::SaveStorage, cx)
                                .disabled(self.busy.is_some()),
                        )
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .flex_wrap()
                                .child(
                                    self.button(
                                        "cloud-backup",
                                        "备份本机资料",
                                        Action::CloudBackup,
                                        cx,
                                    )
                                    .disabled(self.busy.is_some()),
                                )
                                .child(
                                    self.button(
                                        "cloud-list",
                                        "读取备份版本",
                                        Action::CloudList,
                                        cx,
                                    )
                                    .disabled(self.busy.is_some()),
                                ),
                        ),
                );
                for (i, key) in self.backups.iter().enumerate() {
                    content = content.child(
                        self.button(
                            format!("restore-{i}"),
                            format!("合并：{key}"),
                            Action::CloudRestore(key.clone()),
                            cx,
                        )
                        .disabled(self.busy.is_some()),
                    );
                }
                content = content.child(
                    div()
                        .flex()
                        .gap_2()
                        .child(self.button(
                            "local-backup",
                            "导出本机备份 ZIP",
                            Action::Pick("backup".into()),
                            cx,
                        ))
                        .child(self.button(
                            "local-restore",
                            "导入并合并 ZIP",
                            Action::Pick("restore".into()),
                            cx,
                        )),
                );
            }
            _ => {}
        }
        content.into_any_element()
    }
}
fn panel(cx: &App) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .p_5()
        .rounded_2xl()
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().background)
}
impl Render for AppView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (title, subtitle) = PAGES
            .iter()
            .find(|(key, _, _)| *key == self.page)
            .map(|(_, a, b)| (*a, *b))
            .unwrap_or(("Clarora", ""));
        if self.video_fullscreen {
            if window.is_fullscreen() {
                if !self.video_fullscreen_seen {
                    window.focus(&self.focus);
                }
                self.video_fullscreen_seen = true;
            } else if self.video_fullscreen_seen {
                // The user may exit through the native window controls.
                self.video_fullscreen = false;
                self.video_fullscreen_owned = false;
                self.video_fullscreen_seen = false;
            }
        }
        if self.video_fullscreen && self.page == "video" {
            return self.fullscreen_video(cx);
        }
        let navigation_progress = self.navigation_progress();
        if let Some((started, _)) = self.navigation_motion {
            if started.elapsed() < Duration::from_millis(240) {
                window.request_animation_frame();
            } else {
                self.navigation_motion = None;
            }
        }
        let content = self.content(window, cx);
        let assistant = if self.assistant_open && self.page != "video" {
            Some(self.assistant_view(window, cx))
        } else {
            None
        };
        let mut links = div()
            .id("navigation-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_1();
        for (i, (key, label, _)) in PAGES.iter().enumerate() {
            if let Some(section) = match i {
                0 => Some("学习空间"),
                5 => Some("资料库"),
                9 => Some("我的"),
                _ => None,
            } {
                links = links.child(
                    div()
                        .px_3()
                        .pt_5()
                        .pb_2()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(section),
                );
            }
            let selected = self.page == *key;
            let key = (*key).to_string();
            links = links.child(
                div()
                    .id(SharedString::from(format!("nav-{key}")))
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_2p5()
                    .rounded_lg()
                    .cursor_pointer()
                    .text_sm()
                    .font_weight(if selected {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::NORMAL
                    })
                    .bg(if selected {
                        cx.theme().accent
                    } else {
                        transparent_black()
                    })
                    .text_color(if selected {
                        cx.theme().accent_foreground
                    } else {
                        cx.theme().foreground
                    })
                    .hover(|s| s.bg(cx.theme().accent))
                    .child(design::icon(
                        &key,
                        if selected {
                            cx.theme().primary
                        } else {
                            cx.theme().muted_foreground
                        },
                    ))
                    .child(div().flex_1().child(*label))
                    .when(selected, |v| {
                        v.child(div().size(px(5.)).rounded_full().bg(cx.theme().primary))
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.handle(Action::Page(key.clone()), window, cx)
                    })),
            );
        }
        let nav = div()
            .w(px(224.))
            .flex_shrink_0()
            .h_full()
            .px_4()
            .py_6()
            .flex()
            .flex_col()
            .bg(design::rail(cx))
            .child(
                div()
                    .px_2()
                    .pb_4()
                    .flex()
                    .gap_3()
                    .items_center()
                    .child(
                        div()
                            .size(px(36.))
                            .rounded_xl()
                            .bg(cx.theme().primary)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                design::icon("meditation", cx.theme().primary_foreground)
                                    .size(px(24.)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(FontWeight::BOLD)
                                    .child("Clarora"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("把世界，听得更清楚"),
                            ),
                    ),
            )
            .child(links)
            .child(
                div()
                    .mt_4()
                    .p_4()
                    .rounded_xl()
                    .bg(cx.theme().background)
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .mb_2()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(design::icon("meditation", cx.theme().primary))
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child("留一点时间给自己"),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("每天一点，积累新的表达。"),
                    ),
            )
            .child(
                div()
                    .pt_4()
                    .px_2()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("本机学习空间"),
                    )
                    .child(
                        self.button("nav-theme", "明 / 暗", Action::Theme, cx)
                            .ghost(),
                    ),
            );
        div()
            .id("app")
            .track_focus(&self.focus)
            .key_context("Clarora")
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                if let Some((start, width)) = this.video_resize {
                    if event.dragging() {
                        let available = (f32::from(window.viewport_size().width)
                            - if this.navigation_open { 224. } else { 0. }
                            - 32.)
                            .max(300.);
                        this.video_panel_width = (width - f32::from(event.position.x - start))
                            .clamp(300., (available * 0.48).max(300.));
                        cx.notify();
                    } else {
                        this.video_resize = None;
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &OpenFile, _, cx| {
                let kind = match this.page.as_str() {
                    "pdf" => "pdf",
                    "video" => "video",
                    "ocr" => "ocr",
                    "speaking" => "recording",
                    "library" | "listening" => "audio",
                    _ => "words",
                };
                this.pick(kind, cx);
            }))
            .on_action(cx.listener(|this, _: &NextVideoPart, window, cx| {
                if this.page == "video" {
                    if let Some(index) = this
                        .video_series
                        .as_ref()
                        .and_then(|s| s.current)
                        .map(|i| i + 1)
                    {
                        this.handle(Action::SelectPart(index), window, cx);
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &PreviousVideoPart, window, cx| {
                if this.page == "video" {
                    if let Some(index) = this
                        .video_series
                        .as_ref()
                        .and_then(|s| s.current)
                        .and_then(|i| i.checked_sub(1))
                    {
                        this.handle(Action::SelectPart(index), window, cx);
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &ReadVideoParts, window, cx| {
                if this.page == "video" {
                    this.handle(Action::ReadParts, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleVideoFullscreen, window, cx| {
                this.handle(Action::VideoFullscreen, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleVideoPrecision, window, cx| {
                if this.page == "video" {
                    this.handle(Action::VideoPrecision, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &MarkVideoA, window, cx| {
                if this.page == "video" {
                    this.handle(Action::Mark(true), window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &MarkVideoB, window, cx| {
                if this.page == "video" {
                    this.handle(Action::Mark(false), window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ClearVideoLoop, window, cx| {
                if this.page == "video" {
                    this.handle(Action::VideoClearLoop, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &RepeatVideoCue, window, cx| {
                if this.page == "video" {
                    this.handle(Action::VideoRepeatCue, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &PreviousVideoSentence, window, cx| {
                if this.page == "video" {
                    this.handle(Action::VideoSentence(false), window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &NextVideoSentence, window, cx| {
                if this.page == "video" {
                    this.handle(Action::VideoSentence(true), window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &AskVideoCurrent, window, cx| {
                this.handle(Action::VideoAsk(video_qa::Scope::Current), window, cx);
            }))
            .on_action(cx.listener(|this, _: &AskVideoAll, window, cx| {
                this.handle(Action::VideoAsk(video_qa::Scope::All), window, cx);
            }))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| {
                this.handle(Action::Page("settings".into()), window, cx);
            }))
            .on_action(cx.listener(|this, _: &DownloadLocalModel, window, cx| {
                if this.page == "settings" {
                    this.handle(Action::Models(this.value("local_model", cx)), window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &CancelTask, window, cx| {
                this.handle(Action::Cancel, window, cx);
            }))
            .on_action(cx.listener(|this, _: &TranscribeVideo, window, cx| {
                this.handle(Action::VideoTranscribe, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ComposeSentence, window, cx| {
                if this.page != "flashcard" {
                    this.handle(Action::Page("flashcard".into()), window, cx);
                }
                this.composer_open = true;
                this.page_scroll.scroll_to_bottom();
                this.inputs["front"].update(cx, |state, cx| state.focus(window, cx));
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleAppearance, window, cx| {
                this.handle(Action::Theme, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleNavigation, window, cx| {
                this.handle(Action::Navigation, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleAssistant, window, cx| {
                this.handle(Action::Assistant, window, cx)
            }))
            .on_action(cx.listener(|this, _: &PdfZoomIn, window, cx| {
                if this.page == "pdf" {
                    this.handle(Action::PdfZoom(200), window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &PdfZoomOut, window, cx| {
                if this.page == "pdf" {
                    this.handle(Action::PdfZoom(-200), window, cx);
                }
            }))
            .on_action(
                cx.listener(|this, _: &AskAi, window, cx| this.handle(Action::Chat, window, cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleBlindListening, window, cx| {
                this.handle(Action::AudioBlind(!this.audio_blind.enabled), window, cx);
            }))
            .on_action(cx.listener(|this, _: &RevealBlindSentence, window, cx| {
                this.handle(Action::AudioBlindReveal, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ResumeBlindListening, window, cx| {
                this.handle(Action::AudioBlindResume(false), window, cx);
            }))
            .on_action(cx.listener(|this, _: &RestartBlindListening, window, cx| {
                this.handle(Action::AudioBlindResume(true), window, cx);
            }))
            .on_action(cx.listener(|this, _: &ViewAudioTranscript, window, cx| {
                this.handle(Action::AudioBlind(false), window, cx);
            }))
            .on_action(cx.listener(|this, _: &NewChat, window, cx| {
                if this.page == "video" {
                    this.handle(Action::VideoPanel(2), window, cx);
                } else {
                    this.assistant_open = true;
                }
                this.handle(Action::ChatSession(String::new()), window, cx);
            }))
            .on_action(cx.listener(|this, _: &Forward, window, cx| {
                this.handle(
                    if this.page == "pdf" {
                        Action::PdfPage(1)
                    } else {
                        Action::Next
                    },
                    window,
                    cx,
                )
            }))
            .on_action(cx.listener(|this, _: &Backward, window, cx| {
                this.handle(
                    if this.page == "pdf" {
                        Action::PdfPage(-1)
                    } else {
                        Action::Previous
                    },
                    window,
                    cx,
                )
            }))
            .size_full()
            .flex()
            .bg(design::canvas(cx))
            .text_color(cx.theme().foreground)
            .text_size(px(14.))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.modifiers.control
                    || (cfg!(target_os = "macos") && event.keystroke.modifiers.platform)
                {
                    if let Ok(index) = event.keystroke.key.parse::<usize>() {
                        if index >= 1 && index <= PAGES.len() {
                            this.handle(Action::Page(PAGES[index - 1].0.into()), window, cx);
                            return;
                        }
                    }
                    return;
                }
                if window.focused(cx) == Some(this.focus.clone()) {
                    if this.page == "listening"
                        && this.audio_blind.enabled
                        && !event.keystroke.modifiers.alt
                        && !event.keystroke.modifiers.shift
                    {
                        let action = match event.keystroke.key.as_str() {
                            "r" => Some(Action::AudioBlindSentence(0)),
                            "up" => Some(Action::AudioBlindSentence(-1)),
                            "down" => Some(Action::AudioBlindSentence(1)),
                            "h" => Some(if this.audio_blind.revealed.is_some() {
                                Action::AudioBlindHide
                            } else {
                                Action::AudioBlindReveal
                            }),
                            "c" => Some(Action::AudioBlindResume(false)),
                            _ => None,
                        };
                        if let Some(action) = action {
                            this.handle(action, window, cx);
                            cx.stop_propagation();
                            return;
                        }
                    }
                    match event.keystroke.key.as_str() {
                        "space" => this.handle(
                            if ["flashcard", "discover"].contains(&this.page.as_str()) {
                                Action::Flip
                            } else {
                                Action::Play
                            },
                            window,
                            cx,
                        ),
                        "right" => this.handle(
                            if ["video", "listening"].contains(&this.page.as_str()) {
                                Action::Seek(10.)
                            } else {
                                Action::Next
                            },
                            window,
                            cx,
                        ),
                        "left" => this.handle(
                            if ["video", "listening"].contains(&this.page.as_str()) {
                                Action::Seek(-10.)
                            } else {
                                Action::Previous
                            },
                            window,
                            cx,
                        ),
                        "1" => this.handle(Action::Grade("again".into()), window, cx),
                        "2" => this.handle(Action::Grade("hard".into()), window, cx),
                        "3" => this.handle(Action::Grade("good".into()), window, cx),
                        _ => {}
                    }
                }
            }))
            .when(navigation_progress > 0., |view| {
                view.child(
                    div()
                        .id("navigation-slide")
                        .relative()
                        .w(px(224. * navigation_progress))
                        .flex_shrink_0()
                        .h_full()
                        .overflow_hidden()
                        .child(
                            nav.absolute()
                                .top_0()
                                .left(px(224. * (navigation_progress - 1.))),
                        ),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .px_8()
                            .py_6()
                            .when(self.page == "video" || self.page == "listening", |v| {
                                v.py_3()
                            })
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .bg(design::canvas(cx))
                            .flex()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_3()
                                            .child(
                                                self.button(
                                                    "navigation-toggle",
                                                    if self.navigation_open {
                                                        "收起导航"
                                                    } else {
                                                        "展开导航"
                                                    },
                                                    Action::Navigation,
                                                    cx,
                                                )
                                                .icon(
                                                    Icon::new(IconName::ChevronDown)
                                                        .path(if self.navigation_open {
                                                            "ui/sidebar-close.svg"
                                                        } else {
                                                            "ui/sidebar-open.svg"
                                                        })
                                                        .size(px(22.)),
                                                )
                                                .h(px(36.))
                                                .px_3()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .cursor_pointer()
                                                .custom(
                                                    ButtonCustomVariant::new(cx)
                                                        .color(cx.theme().accent)
                                                        .foreground(cx.theme().primary)
                                                        .border(cx.theme().primary.opacity(0.45))
                                                        .hover(cx.theme().primary.opacity(0.16))
                                                        .active(cx.theme().primary.opacity(0.24)),
                                                )
                                                .tooltip(if cfg!(target_os = "macos") {
                                                    "显示或隐藏左侧导航 · ⌘B"
                                                } else {
                                                    "显示或隐藏左侧导航 · Ctrl+B"
                                                }),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child("CLARORA  /  学习空间"),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(27.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(title),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(subtitle),
                                    ),
                            )
                            .child(
                                self.button(
                                    "assistant-toggle",
                                    "✦  学习助手",
                                    Action::Assistant,
                                    cx,
                                )
                                .outline(),
                            ),
                    )
                    .child(
                        div()
                            .id("page-scroll")
                            .track_scroll(&self.page_scroll)
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .when(self.page == "pdf", |view| view.overflow_x_scroll())
                            .p_8()
                            .when(self.page == "video" || self.page == "listening", |v| {
                                v.p_4()
                            })
                            .child(content),
                    )
                    .child(
                        div()
                            .px_6()
                            .py_3()
                            .bg(cx.theme().background)
                            .border_t_1()
                            .border_color(cx.theme().border)
                            .flex()
                            .gap_3()
                            .items_center()
                            .child(
                                div()
                                    .size(px(6.))
                                    .rounded_full()
                                    .bg(if self.busy.is_some() {
                                        cx.theme().primary
                                    } else {
                                        cx.theme().muted_foreground
                                    }),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(self.status.clone()),
                            )
                            .when_some(
                                self.model_progress.as_ref().and_then(|p| p.percent()),
                                |view, p| {
                                    view.child(
                                        Progress::new().value(p).bg(cx.theme().primary).w(px(120.)),
                                    )
                                },
                            )
                            .when(self.busy.is_some(), |view| {
                                view.child(self.button(
                                    "cancel-task",
                                    "取消任务",
                                    Action::Cancel,
                                    cx,
                                ))
                            })
                            .when(!self.playback.error.is_empty(), |view| {
                                view.child(self.playback.error.clone())
                            }),
                    ),
            )
            .when_some(assistant, |view, assistant| view.child(assistant))
            .children(Root::render_dialog_layer(window, cx))
            .into_any_element()
    }
}
fn main() -> Result<()> {
    let args = std::env::args().collect::<Vec<_>>();
    let root = if let Some(index) = args.iter().position(|arg| arg == "--data-dir") {
        PathBuf::from(args.get(index + 1).context("--data-dir 缺少目录")?)
    } else {
        dirs::data_local_dir()
            .context("找不到本机数据目录")?
            .join("Clarora-GPUI")
    };
    let library = Library::open(root)?;
    let panic_path = library.root.join("last-panic.log");
    std::panic::set_hook(Box::new(move |info| {
        use std::io::Write;
        if let Ok(mut file) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&panic_path)
        {
            let _ = writeln!(
                file,
                "{info}\n{}",
                std::backtrace::Backtrace::force_capture()
            );
        }
    }));
    Application::new()
        .with_assets(design::Assets)
        .run(move |cx| {
            gpui_component::init(cx);
            design::apply(cx);
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.bind_keys([
                KeyBinding::new("f11", ToggleVideoFullscreen, None),
                KeyBinding::new("space", FullscreenPlayPause, Some("VideoFullscreen")),
                KeyBinding::new("left", FullscreenSeekBack, Some("VideoFullscreen")),
                KeyBinding::new("right", FullscreenSeekForward, Some("VideoFullscreen")),
                KeyBinding::new("escape", ExitVideoFullscreen, Some("VideoFullscreen")),
                KeyBinding::new("cmd-,", OpenSettings, None),
                KeyBinding::new("ctrl-,", OpenSettings, None),
                KeyBinding::new("cmd-q", Quit, None),
                KeyBinding::new("ctrl-q", Quit, None),
                KeyBinding::new("cmd-shift-l", ToggleAppearance, None),
                KeyBinding::new("ctrl-shift-l", ToggleAppearance, None),
                KeyBinding::new("cmd-shift-n", ComposeSentence, None),
                KeyBinding::new("ctrl-shift-n", ComposeSentence, None),
                KeyBinding::new("cmd-shift-right", NextVideoPart, None),
                KeyBinding::new("ctrl-shift-right", NextVideoPart, None),
                KeyBinding::new("cmd-shift-left", PreviousVideoPart, None),
                KeyBinding::new("ctrl-shift-left", PreviousVideoPart, None),
                KeyBinding::new("cmd-shift-t", TranscribeVideo, None),
                KeyBinding::new("ctrl-shift-t", TranscribeVideo, None),
                KeyBinding::new("cmd-o", OpenFile, None),
                KeyBinding::new("ctrl-o", OpenFile, None),
                KeyBinding::new("cmd-b", ToggleNavigation, None),
                KeyBinding::new("ctrl-b", ToggleNavigation, None),
                KeyBinding::new("cmd-j", ToggleAssistant, None),
                KeyBinding::new("ctrl-j", ToggleAssistant, None),
                KeyBinding::new("cmd-enter", AskAi, None),
                KeyBinding::new("ctrl-enter", AskAi, None),
                KeyBinding::new("cmd-n", NewChat, None),
                KeyBinding::new("ctrl-n", NewChat, None),
                KeyBinding::new("cmd-right", Forward, None),
                KeyBinding::new("ctrl-right", Forward, None),
                KeyBinding::new("cmd-left", Backward, None),
                KeyBinding::new("ctrl-left", Backward, None),
                KeyBinding::new("cmd-=", PdfZoomIn, None),
                KeyBinding::new("ctrl-=", PdfZoomIn, None),
                KeyBinding::new("cmd--", PdfZoomOut, None),
                KeyBinding::new("ctrl--", PdfZoomOut, None),
            ]);
            cx.set_menus(vec![
                Menu {
                    name: "Clarora GPUI".into(),
                    items: vec![
                        MenuItem::action("设置…", OpenSettings),
                        MenuItem::action("显示或隐藏左侧导航", ToggleNavigation),
                        MenuItem::action("切换浅色 / 深色", ToggleAppearance),
                        MenuItem::action("取消当前任务", CancelTask),
                        MenuItem::action("退出 Clarora GPUI", Quit),
                    ],
                },
                Menu {
                    name: "文件".into(),
                    items: vec![
                        MenuItem::action("打开或导入…", OpenFile),
                        MenuItem::action("新建句子卡", ComposeSentence),
                        MenuItem::action("生成视频字幕", TranscribeVideo),
                        MenuItem::action("视频全屏 / 退出全屏", ToggleVideoFullscreen),
                        MenuItem::action("展开或收起 A–B 精听", ToggleVideoPrecision),
                        MenuItem::action("设置视频 A 起点", MarkVideoA),
                        MenuItem::action("设置视频 B 终点", MarkVideoB),
                        MenuItem::action("清除视频 A/B", ClearVideoLoop),
                        MenuItem::action("重听当前视频字幕句", RepeatVideoCue),
                        MenuItem::action("上一句视频字幕", PreviousVideoSentence),
                        MenuItem::action("下一句视频字幕", NextVideoSentence),
                        MenuItem::action("读取 bilibili 分P列表", ReadVideoParts),
                        MenuItem::action("上一P", PreviousVideoPart),
                        MenuItem::action("下一P", NextVideoPart),
                        MenuItem::action("上一项", Backward),
                        MenuItem::action("下一项", Forward),
                        MenuItem::action("放大 PDF", PdfZoomIn),
                        MenuItem::action("缩小 PDF", PdfZoomOut),
                    ],
                },
                Menu {
                    name: "听力".into(),
                    items: vec![
                        MenuItem::action("切换字幕 / 盲听", ToggleBlindListening),
                        MenuItem::action("看刚才一句", RevealBlindSentence),
                        MenuItem::action("继续盲听", ResumeBlindListening),
                        MenuItem::action("从头再听", RestartBlindListening),
                        MenuItem::action("查看全文（暂停）", ViewAudioTranscript),
                    ],
                },
                Menu {
                    name: "AI".into(),
                    items: vec![
                        MenuItem::action("下载当前表单的本地语音模型", DownloadLocalModel),
                        MenuItem::action("显示或隐藏助手", ToggleAssistant),
                        MenuItem::action("向当前视频字幕提问", AskVideoCurrent),
                        MenuItem::action("向整段视频字幕提问", AskVideoAll),
                        MenuItem::action("新会话", NewChat),
                        MenuItem::action("发送问题", AskAi),
                    ],
                },
            ]);
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let bounds = Bounds::centered(None, size(px(1240.), px(860.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Clarora · GPUI".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |window, cx| {
                    let view = cx.new(|cx| AppView::new(library, window, cx));
                    view.read(cx).focus.focus(window);
                    cx.new(|cx| Root::new(view, window, cx))
                },
            )
            .expect("无法打开 Clarora GPUI 窗口");
            cx.activate(true);
        });
    Ok(())
}

impl Drop for AppView {
    fn drop(&mut self) {
        if let Some(cancel) = &self.busy {
            cancel.store(true, Ordering::SeqCst);
        }
        if let Some(cancel) = &self.waveform_cancel {
            cancel.store(true, Ordering::SeqCst);
        }
        self.save_media_position();
        let _ = self.lib.stat("listen_seconds", self.stats_ticks);
    }
}
