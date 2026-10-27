#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod action_router;
mod app_state;
mod assistant_ui;
mod audio_controller;
mod audio_current_ui;
mod audio_player_ui;
mod audio_subtitle_ui;
mod chat_controller_ui;
mod clip_selection_ui;
mod data_controller;
mod design;
mod feedback_ui;
mod floating_ui;
mod focus_ui;
mod library_ui;
mod pages_ui;
mod playback_controller;
mod reading_ui;
mod saved_clip_ui;
mod settings_ui;
mod subtitle_follow_ui;
mod subtitle_text_ui;
mod task_controller;
mod task_ui;
#[cfg(debug_assertions)]
mod ui_controller_probe;
mod video_controller;
mod video_job_ui;
mod video_library_ui;
mod video_player_ui;
mod video_transcript_ui;
mod writing_ui;
use anyhow::{Context as _, Result, ensure};
use clarora_gpui::{
    ai, audio_library, backup, bilibili, blind_listening, clip_marks, credentials,
    data::{Card, Library, Row, id, now, text},
    documents, ebook, focus_timer,
    media::{Playback, Player},
    model_download::Progress as ModelProgress,
    process::{self, Cancel},
    recording::Recorder,
    subtitles::{self, Cue},
    timer_chimes, tts, video_jobs, video_library, video_qa, video_subtitles,
    waveform::{self, Waveform},
};
use feedback_ui::FeedbackKind;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, Root, Selectable, Sizable, Theme, ThemeMode,
    WindowExt,
    button::*,
    dialog::DialogButtonProps,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu, PopupMenuItem},
    progress::Progress,
    scroll::{Scrollbar, ScrollbarShow},
    switch::Switch,
    text::TextView,
    tooltip::Tooltip,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{atomic::Ordering, mpsc},
    time::{Duration, Instant},
};
use subtitle_follow_ui::centered_list;
use task_ui::{TaskNotice, TaskState};
actions!(
    clarora_gpui_actions,
    [
        Quit,
        CopySubtitleSelection,
        ClearSubtitleSelection,
        OpenFile,
        ToggleAssistant,
        ToggleFloatingImport,
        ImportAudioCommand,
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
    ("writing", "写作练习", "保存草稿与 AI 批改"),
    ("library", "音频管理", "整理练习组"),
    ("pdf", "文档阅读", "PDF / EPUB / TXT"),
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
    CopySelectedSubtitle,
    AudioCopySubtitle(String, Option<Cue>),
    SavedClips(Vec<clip_marks::Saved>),
    AudioRepeat(usize),
    LibrarySelect { id: String, reveal: bool },
    LibraryRenameAudio(String),
    LibraryCommitRename,
    LibraryCancelRename,
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
    VideoSentenceRepeat(Option<usize>, bool),
    VideoStopSentenceRepeat,
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
    DismissTaskNotice,
    DismissFeedback(bool),
    RetryFeedback,
    Clip,
    Chat,
    SaveAnswer,
    ChatSession(String),
    ToggleReasoning,
    PdfOpen(PathBuf),
    Favorite,
    Transcribe,
    VideoTranscribe,
    VideoTranscribeAll,
    VideoTranslateAll,
    CancelVideoSubtitles,
    DismissVideoSubtitles,
    VideoAsk(video_qa::Scope),
    VideoFollow,
    VideoPanel(usize),
    VideoTools,
    VideoTranscript,
    VideoCopyTranscript,
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
    CredentialMode(bool),
    ImportLegacyCredentials,
    SaveStorage,
    Models(String),
    SelectModel(String),
    CloudList,
    CloudBackup,
    CloudRestore(String),
    PdfPage(i32),
    PdfZoom(i32),
    DocumentCopy,
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
    AudioGroupSubtitles(clarora_gpui::audio_batch::Mode),
    ConfigureCommandImport,
    SaveCommandImport,
    FloatingImport,
    NameImportedAudio,
    CloseFloatingNames,
    Refresh,
    GraphSelect(String),
    GraphList(i64),
}
enum Outcome {
    CredentialMode(bool),
    LibraryRestored(usize, video_library::Collection),
    Message(String),
    Issue(String),
    Parts(bilibili::Series),
    PartVideo(bilibili::Series, usize, PathBuf, bool, Option<PathBuf>),
    Output(String),
    VideoTranslated(PathBuf, String, Vec<Cue>, Vec<Cue>),
    Pdf(PathBuf, i32, documents::PdfPage),
    Ebook(ebook::Reading),
    Ocr(String),
    KeyPoints(String, Vec<ai::KeyPoint>),
    Backups(Vec<String>),
    Recording(PathBuf),
    TtsReady(Row),
    CommandImported(clarora_gpui::command_import::Report),
    AudioBatch(clarora_gpui::audio_batch::Report),
}
enum Event {
    SubtitleProgress(String),
    Done(
        Option<clarora_gpui::chat_store::Request>,
        std::result::Result<Outcome, String>,
    ),
    Chunk(clarora_gpui::chat_store::Request, ai::ChatReply),
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
    #[cfg(debug_assertions)]
    probe: Option<ui_controller_probe::Probe>,
    data: clarora_gpui::view_data::State,
    loads: data_controller::Loads,
    settings: app_state::SettingsState,
    chat: clarora_gpui::chat_controller::Controller,
    chat_requested_context: String,
    chat_closing: Option<chat_controller_ui::Closing>,
    tasks: app_state::TasksState,
    media: app_state::PlaybackState,
    video: app_state::VideoState,
    audio: app_state::AudioState,
    writing: Option<Entity<writing_ui::WritingView>>,
    tts_voice: i32,
    tts_speed: f32,
    tts_last: Option<Row>,
    page: String,
    navigation_open: bool,
    navigation_motion: Option<(Instant, f32)>,
    inputs: BTreeMap<String, Entity<InputState>>,
    status: String,
    feedback: feedback_ui::FeedbackState,
    cards: Vec<Card>,
    subtitle_text: std::rc::Rc<std::cell::RefCell<subtitle_text_ui::State>>,
    clip_catalog: clip_marks::Catalog,
    audio_saved: Vec<clip_marks::Range>,
    video_saved: Vec<clip_marks::Range>,
    index: usize,
    revealed: bool,
    editing: Option<Card>,
    composer_open: bool,
    page_scroll: ScrollHandle,
    due: bool,
    card_filter: String,
    saved_only: bool,
    history: Vec<usize>,
    output: String,
    backups: Vec<String>,
    recording: Option<Recorder>,
    recorded: Option<PathBuf>,
    timer_chimes: timer_chimes::Chimes,
    rendered_images: clarora_gpui::render_images::RenderedImages,
    timer: focus_timer::Timer,
    timer_remaining: u64,
    timer_retry: Option<Instant>,
    pdf_path: Option<PathBuf>,
    pdf_image: Option<std::sync::Arc<RenderImage>>,
    pdf_page: u16,
    pdf_pages: u16,
    pdf_width: i32,
    pdf_bookmarks: Vec<(String, u16)>,
    ebook_reading: Option<ebook::Reading>,
    document_text: String,
    ocr_text: String,
    ocr_path: Option<PathBuf>,
    dark: bool,
    focus: FocusHandle,
    assistant_open: bool,
    floating_window: Option<AnyWindowHandle>,
    floating_name_window: Option<AnyWindowHandle>,
    command_imported: Option<clarora_gpui::command_import::Report>,
    import_command: String,
    graph_node: String,
    graph_list: i64,
}
impl AppView {
    fn new(lib: Library, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.on_release(|this: &mut Self, cx| this.rendered_images.clear(cx))
            .detach();
        let mut inputs = BTreeMap::new();
        for (key, placeholder, multi, masked) in [
            ("search", "搜索内容", false, false),
            ("library_search", "搜索当前组的音频", false, false),
            ("library_groups", "搜索练习组", false, false),
            ("library_name", "输入名称", false, false),
            ("library_audio_name", "音频名称", false, false),
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
        let (timer, timer_warning, timer_completed) =
            match focus_timer::Timer::load(&lib, focus_timer::now_ms()) {
                Ok(loaded) => (loaded.timer, loaded.warning, loaded.completed),
                Err(e) => (
                    focus_timer::Timer::default(),
                    Some(format!("无法读取专注计时：{e:#}")),
                    None,
                ),
            };
        let mut this = Self {
            #[cfg(debug_assertions)]
            probe: None,
            data: Default::default(),
            loads: Default::default(),
            settings: app_state::SettingsState::new(&lib),
            chat: clarora_gpui::chat_controller::Controller::new(lib.clone()),
            chat_requested_context: String::new(),
            chat_closing: None,
            tasks: app_state::TasksState::new(),
            media: app_state::PlaybackState::new(),
            video: app_state::VideoState::new(),
            audio: app_state::AudioState::new(),
            timer_chimes: timer_chimes::Chimes::new(lib.root.join("cache/timer-chimes")),
            rendered_images: Default::default(),
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
            import_command: lib.setting("command").unwrap_or_default(),
            lib,
            writing: None,
            page: "flashcard".into(),
            navigation_open: true,
            navigation_motion: None,
            inputs,
            status: String::new(),
            feedback: Default::default(),
            cards: vec![],
            subtitle_text: std::rc::Rc::new(std::cell::RefCell::new(subtitle_text_ui::State::new(
                cx,
            ))),
            clip_catalog: Default::default(),
            audio_saved: vec![],
            video_saved: vec![],
            index: 0,
            revealed: false,
            editing: None,
            composer_open: false,
            page_scroll: ScrollHandle::new(),
            due: true,
            card_filter: "all".into(),
            saved_only: false,
            history: vec![],
            output: String::new(),
            backups: vec![],
            recording: None,
            recorded: None,
            timer_remaining: timer.remaining_seconds(focus_timer::now_ms()),
            timer,
            timer_retry: None,
            pdf_path: None,
            pdf_image: None,
            pdf_page: 0,
            pdf_pages: 0,
            pdf_width: 1000,
            pdf_bookmarks: vec![],
            ebook_reading: None,
            document_text: String::new(),
            ocr_text: String::new(),
            ocr_path: None,
            dark: cx.theme().is_dark(),
            focus: cx.focus_handle(),
            assistant_open: false,
            floating_window: None,
            floating_name_window: None,
            command_imported: None,
            graph_node: "demo-1".into(),
            graph_list: 0,
        };
        if let Some(warning) = timer_warning {
            this.report_status(FeedbackKind::Error, warning);
        }
        if let Some(focused) = timer_completed {
            this.focus_phase_completed(focused);
        }
        // Video is enabled by the actual picture's measured layout, never by an audio preload.
        this.media.player.set_video_target(0, 0, false);
        this.media.preloader.set_video_target(0, 0, false);
        for player in [&this.media.player, &this.media.preloader] {
            let frames = player.frame_events();
            let source = player.state.clone();
            cx.spawn_in(window, async move |this, cx| {
                let mut revision = frames.revision();
                loop {
                    revision = frames.changed(revision).await;
                    if this
                        .update_in(cx, |this, _, cx| {
                            // Audio preloading swaps these two players; follow identity, not a slot.
                            if !std::sync::Arc::ptr_eq(&source, &this.media.player.state) {
                                return;
                            }
                            let snapshot = this.media.player.snapshot();
                            this.media.snapshot.frame = snapshot.frame;
                            this.media.snapshot.frame_revision = snapshot.frame_revision;
                            this.media.snapshot.frame_width = snapshot.frame_width;
                            this.media.snapshot.frame_height = snapshot.frame_height;
                            this.media.snapshot.render_stats = snapshot.render_stats;
                            if this.video_frame_path().is_some() {
                                cx.notify();
                            }
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .detach();
        }
        this.audio.rename_subscription = Some(cx.subscribe_in(
            &this.inputs["library_audio_name"],
            window,
            |this, input, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { secondary: false })
                    && this.page == "library"
                    && this.audio.library.renaming.is_some()
                    && !input.update(cx, |state, cx| {
                        state.marked_text_range(window, cx).is_some()
                    })
                {
                    this.handle(Action::LibraryCommitRename, window, cx);
                }
            },
        ));
        this.video.series = bilibili::Series::load(&this.lib).unwrap_or(None);
        if let Some(series) = &this.video.series {
            this.video.parts_scroll.scroll_to_item(
                series.current.unwrap_or(series.requested),
                ScrollStrategy::Center,
            );
        }
        this.video.captions_enabled =
            this.lib.setting("video_captions_enabled").ok().as_deref() != Some("false");
        this.media
            .player
            .set_subtitles_visible(this.video.captions_enabled);
        this.video.subtitle_size = this
            .lib
            .setting("video_subtitle_size")
            .ok()
            .and_then(|value| value.parse::<u8>().ok())
            .filter(|size| [14, 16, 20, 24, 28, 32].contains(size))
            .unwrap_or(16);
        this.audio.font_size = this
            .lib
            .setting("subtitle_font_size")
            .ok()
            .and_then(|v| v.parse::<u8>().ok())
            .filter(|n| [14, 16, 18, 20, 24, 28, 32].contains(n))
            .unwrap_or(18);
        this.audio.blind_chinese =
            this.lib.setting("blind_chinese_hint").ok().as_deref() == Some("true");
        this.video.auto_next_part =
            this.lib.setting("video_auto_next").ok().as_deref() != Some("false");
        if let Some(series) = &this.video.series {
            let base = series.base.clone();
            this.set("bilibili", base, window, cx);
        }
        match video_library::Collection::load(&this.lib) {
            Ok(videos) => this.video.collection = videos,
            Err(e) => this.report_status(FeedbackKind::Error, format!("{e:#}")),
        }
        let sources: Vec<_> = this
            .video
            .collection
            .items
            .iter()
            .filter_map(|v| v.series.as_ref())
            .filter(|s| s.title.trim().is_empty())
            .map(|s| s.base.clone())
            .collect();
        if !sources.is_empty() {
            let (tx, rx) = mpsc::channel();
            this.tasks.title_events = Some(rx);
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
        this.writing =
            Some(cx.new(|cx| writing_ui::WritingView::new(this.lib.clone(), window, cx)));
        this.audio.practice_id = this.lib.setting("library_group").unwrap_or_default();
        this.refresh();
        if this.lib.setting("floating_import_open").ok().as_deref() == Some("true") {
            cx.defer_in(window, |this, window, cx| {
                this.toggle_floating_import(window, cx)
            });
        }
        let ai_result = ai::load_config(&this.lib);
        match ai_result {
            Ok(c) => {
                let (c, _) = ai::shared_settings(c, String::new(), String::new());
                for (k, v) in [
                    ("base_url", c.base_url),
                    ("model", c.model),
                    ("vision_model", c.vision_model),
                    ("local_model", c.local_model),
                    ("engine", c.engine),
                ] {
                    this.set(k, v, window, cx);
                }
            }
            Err(e) => this.report_status(FeedbackKind::Error, e.to_string()),
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
        let view = cx.entity().downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            view.update(cx, |this, cx| this.prepare_close(cx))
                .unwrap_or(true)
        });
        cx.on_app_quit(|this, cx| {
            if let Err(error) = this.flush_writing(cx) {
                eprintln!("退出前写作草稿保存失败：{error:#}");
            }
            let editor = this.value("question", cx);
            this.chat.observe_draft(&editor);
            let draft_saved = this.chat.flush_async();
            let done = this.tasks.video_job.as_ref().map(|j| {
                j.cancel.store(true, Ordering::SeqCst);
                j.done.clone()
            });
            async move {
                // GPUI bounds shutdown observers to 100ms. Normal Quit actions
                // use prepare_close and finish persistence before removing the window.
                for _ in 0..18 {
                    match draft_saved.try_recv() {
                        Ok(Ok(())) => break,
                        Ok(Err(error)) => {
                            eprintln!("退出前问题草稿保存失败：{error}");
                            break;
                        }
                        Err(mpsc::TryRecvError::Disconnected) => break,
                        Err(mpsc::TryRecvError::Empty) => {
                            Timer::after(Duration::from_millis(5)).await;
                        }
                    }
                }
                if let Some(done) = done {
                    for _ in 0..30 {
                        if done.load(Ordering::SeqCst) {
                            break;
                        }
                        Timer::after(Duration::from_millis(50)).await;
                    }
                }
            }
        })
        .detach();
        cx.spawn_in(window, async move |this, cx| {
            loop {
                Timer::after(Duration::from_millis(50)).await;
                if this
                    .update_in(cx, |this, window, cx| this.tick(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        if let Some(video) = this
            .video
            .collection
            .active()
            .cloned()
            .filter(|v| v.path.is_file())
        {
            this.video.series = video.series;
            if let Some(series) = &mut this.video.series {
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
            this.media
                .player
                .open_range(&video.path, position, 0., true);
            this.media.path = Some(video.path.clone());
            this.video.path = Some(video.path.clone());
            this.load_video_subtitles(&video.path);
            this.video.tab = 3;
        } else {
            this.video.series = None;
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
    fn flush_writing(&mut self, cx: &mut Context<Self>) -> Result<()> {
        if let Some(view) = &self.writing {
            view.update(cx, |view, cx| view.flush(cx))?;
        }
        Ok(())
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
    fn focus_phase_completed(&mut self, focused: bool) {
        self.play_timer_chime(timer_chimes::Cue::End);
        if focused && self.page == "stats" {
            self.refresh_view_data();
        }
        self.report_status(
            FeedbackKind::Info,
            if focused {
                "专注完成，休息一下"
            } else {
                "休息结束，可以开始下一轮"
            }
            .into(),
        );
    }
    fn play_timer_chime(&mut self, cue: timer_chimes::Cue) {
        if let Err(error) = self.timer_chimes.play(cue) {
            self.report_status(
                FeedbackKind::Error,
                format!("计时提示音播放失败：{error:#}"),
            );
        }
    }
    fn save_media_position(&self) {
        if self.page == "listening" && !self.audio.current_id.is_empty() {
            let _ = self.lib.set_setting(
                "last_listening_position",
                &json!({"id":self.audio.current_id,"position":self.media.snapshot.position})
                    .to_string(),
            );
        } else if self.page == "video" {
            if let Some(path) = &self.video.path {
                let _ = self.lib.set_setting(
                    "last_video_position",
                    &json!({"path":path,"position":self.media.snapshot.position}).to_string(),
                );
            }
        }
    }
    fn current_card(&self) -> Option<Card> {
        self.cards.get(self.index).cloned()
    }
    fn play_card(&mut self) {
        if self.defer_card_play() {
            return;
        }
        self.media.player.set("volume", self.media.volume);
        self.media.player.set("speed", self.media.rate);
        self.media.player.set("pause", "yes");
        if let Some(card) = self.current_card() {
            if !card.media.is_empty() {
                if self.media.preloaded_index == Some(self.index) {
                    std::mem::swap(&mut self.media.player, &mut self.media.preloader);
                    self.media.preloader.set_video_target(0, 0, false);
                    self.media.player.set("pause", "no");
                } else {
                    self.media.player.open_range(
                        &PathBuf::from(card.media),
                        card.start,
                        card.end,
                        false,
                    );
                }
            }
        }
        self.media.preloaded_index = None;
        if self.page == "discover" && !self.cards.is_empty() {
            let due = &self.loads.discover_due;
            let favorites = &self.loads.discover_favorites;
            if let Some(index) = clarora_gpui::discover::next(
                &self.cards,
                &self.history,
                self.index,
                &due,
                &favorites,
            ) {
                self.media.preloaded_index = Some(index);
                let next = &self.cards[index];
                if !next.media.is_empty() {
                    self.media.preloader.open_range(
                        &PathBuf::from(&next.media),
                        next.start,
                        next.end,
                        true,
                    );
                } else {
                    self.media.preloader.set("pause", "yes");
                }
            }
        }
    }
    fn pick(&self, kind: &str, cx: &mut Context<Self>) {
        let kind = kind.to_string();
        let directory = kind == "directory";
        let audio_group = self.audio.practice_id.clone();
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: !directory,
            directories: directory,
            multiple: kind == "audio" || kind == "merge" || kind == "video",
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picker.await {
                let _ = this.update(cx, |this, cx| {
                    if kind == "audio" && this.audio.practice_id != audio_group {
                        this.report_status(
                            FeedbackKind::Error,
                            "选择文件期间练习组已切换，请在目标组重新导入".into(),
                        );
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
                    ensure!(
                        self.tasks.cancel.is_none(),
                        "请完成或取消当前任务后再导入视频"
                    );
                    let mut videos = video_library::Collection::load(&self.lib)?;
                    let ids = videos.add(&paths, None)?;
                    videos.save(&self.lib)?;
                    let first = videos.select(&ids[0])?;
                    self.video.collection = videos;
                    self.save_media_position();
                    ensure!(self.open(first.path), "{}", self.status);
                    self.video.tab = 3;
                    self.video.tools_open = false;
                    self.report_status(
                        FeedbackKind::Info,
                        format!("已收藏 {} 个视频，可在“我的视频”切换或移除", ids.len()),
                    );
                    Ok(())
                })();
                if let Err(e) = result {
                    self.report_status(FeedbackKind::Error, format!("视频导入失败：{e:#}"));
                }
            }
            "video-subtitle" => {
                let result = (|| -> Result<()> {
                    ensure!(
                        self.tasks.video_job.is_none(),
                        "请先完成或取消后台字幕任务后再导入字幕"
                    );
                    let video = self.media.path.as_ref().context("请先打开视频")?;
                    let mut cues = subtitles::parse(&fs::read_to_string(&path)?);
                    ensure!(!cues.is_empty(), "请选择带时间轴的 UTF-8 SRT / VTT 字幕");
                    cues.sort_by(|a, b| a.start.total_cmp(&b.start));
                    video_subtitles::save(&self.lib, video, &cues, &process::cancel_token())?;
                    self.media.player.command([
                        "sub-add".into(),
                        path.to_string_lossy().into_owned(),
                        "select".into(),
                    ]);
                    self.load_video_subtitles(video.clone().as_path());
                    Ok(())
                })();
                self.report_status(
                    if result.is_err() {
                        FeedbackKind::Error
                    } else {
                        FeedbackKind::Info
                    },
                    match result {
                        Ok(()) => "已加载并缓存字幕，可同步高亮及向 AI 提问".into(),
                        Err(error) => error.to_string(),
                    },
                );
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
                let group = self.audio.practice_id.clone();
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
                    if errors.is_empty() {
                        Ok(Outcome::Message(format!("已导入 {count} 条音频")))
                    } else {
                        Ok(Outcome::Issue(format!(
                            "已导入 {count} 条；其余导入失败：{}",
                            errors.join("；")
                        )))
                    }
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
                if self.tasks.cancel.is_some() {
                    self.report_status(
                        FeedbackKind::Error,
                        "已有任务正在执行，请完成或取消后重试".into(),
                    );
                    return;
                }
                if ebook::is_ebook(&path) {
                    self.begin("正在导入电子书…", move |lib, cancel, _| {
                        Ok(Outcome::Ebook(ebook::open(&lib, &path, true, &cancel)?))
                    });
                } else if ebook::supported(&path) {
                    match self.lib.copy_media(&path, "pdf") {
                        Ok(p) => {
                            self.load_pdf(p, 0, self.pdf_width);
                        }
                        Err(e) => self.report_status(FeedbackKind::Error, e.to_string()),
                    }
                } else {
                    self.report_status(
                        FeedbackKind::Error,
                        "请选择 PDF、EPUB 或 TXT 文件；MOBI/AZW3 暂不支持".into(),
                    );
                }
            }
            "rn-library" => self.begin("正在读取原版资料快照…", move |lib, _, _| {
                Ok(Outcome::Message(format!(
                    "已从快照合并 {} 条，原库保持只读",
                    backup::import_sqlite(&lib, &path)?
                )))
            }),
            "restore" => self.begin("正在合并资料备份…", move |lib, cancel, _| {
                let count = backup::import_local_cancellable(&lib, &path, &cancel)?;
                let videos = video_library::Collection::load(&lib)
                    .context("资料已恢复，但重新载入视频库失败，请刷新")?;
                Ok(Outcome::LibraryRestored(count, videos))
            }),
            "recording" => {
                self.recorded = Some(path);
                self.report_status(FeedbackKind::Info, "已选择录音".into());
            }
            "background" => {
                self.media.player.open(&path);
                self.media.player.set("loop-file", "inf");
            }
            _ => {}
        }
    }
    fn load_pdf(&mut self, path: PathBuf, page: u16, width: i32) {
        self.begin("正在读取 PDF…", move |lib, cancel, _| {
            ensure!(!cancel.load(Ordering::SeqCst), "已取消");
            let loaded = documents::load_pdf_page(&lib, &path, page, width)?;
            Ok(Outcome::Pdf(path, width, loaded))
        });
    }
    fn render_pdf(&mut self) {
        if self.tasks.cancel.is_some() {
            self.report_status(
                FeedbackKind::Error,
                "已有任务正在执行，请完成或取消后重试".into(),
            );
            return;
        }
        if let Some(path) = self.pdf_path.clone() {
            if ebook::is_ebook(&path) {
                self.begin("正在读取电子书…", move |lib, cancel, _| {
                    Ok(Outcome::Ebook(ebook::open(&lib, &path, false, &cancel)?))
                });
            } else {
                self.load_pdf(path, self.pdf_page, self.pdf_width);
            }
        }
    }
    fn reference(&self, cx: &App) -> String {
        match self.page.as_str() {
            "flashcard" | "discover" => self
                .current_card()
                .filter(|_| self.cards_ready())
                .map(|c| format!("{}\n{}", c.front, c.back))
                .unwrap_or_default(),
            "listening" if self.audio.blind.enabled => {
                self.audio.blind.reference_for(self.audio.blind_chinese)
            }
            "listening" if self.media.loop_a.is_some() && self.media.loop_b.is_some() => {
                let (front, back) = subtitles::excerpt(
                    &self.audio.cues,
                    self.media.loop_a.unwrap(),
                    self.media.loop_b.unwrap(),
                );
                format!("选中内容：{front}\n翻译：{back}")
            }
            "listening" => self
                .audio
                .cues
                .iter()
                .map(|c| c.text.clone())
                .collect::<Vec<_>>()
                .join("\n"),
            "pdf" => self.document_text.clone(),
            "ocr" => self.ocr_text.clone(),
            "speaking" => self.value("reference", cx),
            "writing" => self
                .writing
                .as_ref()
                .map(|view| view.read(cx).reference(cx))
                .unwrap_or_default(),
            "video" => self
                .video
                .material
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
                        | Action::VideoSentenceRepeat(..)
                        | Action::AudioCopySubtitle(..)
                        | Action::AudioBlindRevealCue(..)
                        | Action::AudioBlindHide
                ) {
                    cx.stop_propagation();
                }
                this.handle(action.clone(), window, cx)
            }))
    }
    fn field(&self, key: &str, label: &str) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_sm().child(label.to_string()))
            .child(Input::new(self.inputs.get(key).unwrap()))
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
        let volume = self.media.volume as f32 / 100.;
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
                        this.video.volume_dragging = true;
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
                    if this.video.volume_dragging && event.dragging() && area.size.width > px(0.) {
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
                cx.listener(|this, _, _, _| this.video.volume_dragging = false),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.video.volume_dragging = false),
            )
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
}
fn clip_clock(s: f64) -> String {
    let tenths = (s.max(0.) * 10.).round() as u64;
    format!("{}.{:01}", clock(tenths as f64 / 10.), tenths % 10)
}
fn clock(s: f64) -> String {
    let n = s.max(0.) as u64;
    format!("{:02}:{:02}", n / 60, n % 60)
}
impl AppView {}
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
        let mut images = Vec::new();
        if self.video_frame_path().is_some() {
            images.extend(self.media.snapshot.frame.clone());
        }
        if self.page == "pdf" {
            if let Some(reading) = &self.ebook_reading {
                images.extend(reading.images.values().cloned());
            } else {
                images.extend(self.pdf_image.clone());
            }
        }
        self.rendered_images.retain(images, window, cx);
        self.subtitle_text.borrow_mut().begin_frame();
        let (title, subtitle) = PAGES
            .iter()
            .find(|(key, _, _)| *key == self.page)
            .map(|(_, a, b)| (*a, *b))
            .unwrap_or(("Clarora", ""));
        if self.video.fullscreen {
            if window.is_fullscreen() {
                if !self.video.fullscreen_seen {
                    window.focus(&self.focus);
                }
                self.video.fullscreen_seen = true;
            } else if self.video.fullscreen_seen {
                // The user may exit through the native window controls.
                self.video.fullscreen = false;
                self.video.fullscreen_owned = false;
                self.video.fullscreen_seen = false;
            }
        }
        if self.video.fullscreen && self.page == "video" {
            return div()
                .relative()
                .size_full()
                .child(self.fullscreen_video(cx))
                .children(self.feedback_view(window, cx))
                .into_any_element();
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
                6 => Some("资料库"),
                10 => Some("我的"),
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
            .child(self.subtitle_copy_anchor(cx))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.subtitle_text.borrow_mut().block_click = false),
            )
            .on_action(cx.listener(|this, _: &ToggleFloatingImport, window, cx| {
                this.toggle_floating_import(window, cx);
            }))
            .on_action(cx.listener(|this, _: &ImportAudioCommand, window, cx| {
                this.handle(Action::CommandImport, window, cx);
            }))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                this.subtitle_drag_update(event, cx);
                if let Some((start, width)) = this.video.resize {
                    if event.dragging() {
                        let available = (f32::from(window.viewport_size().width)
                            - if this.navigation_open { 224. } else { 0. }
                            - 32.)
                            .max(300.);
                        this.video.panel_width = (width - f32::from(event.position.x - start))
                            .clamp(300., (available * 0.48).max(300.));
                        cx.notify();
                    } else {
                        this.video.resize = None;
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &OpenFile, _, cx| {
                if this.page == "writing" {
                    return;
                }
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
            .on_action(cx.listener(|this, _: &Quit, _, cx| {
                cx.stop_propagation();
                this.prepare_close(cx);
            }))
            .on_action(cx.listener(|this, _: &NextVideoPart, window, cx| {
                if this.page == "video" {
                    if let Some(index) = this
                        .video
                        .series
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
                        .video
                        .series
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
                this.handle(Action::AudioBlind(!this.audio.blind.enabled), window, cx);
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
                if this.page == "writing" {
                    return;
                }
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
                if this.page == "writing" {
                    return;
                }
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
                if this.page == "writing" {
                    return;
                }
                let subtitle_focus = this.subtitle_text.borrow().focus.clone();
                if window.focused(cx) == Some(this.focus.clone())
                    || window.focused(cx) == Some(subtitle_focus)
                {
                    if this.page == "listening"
                        && this.audio.blind.enabled
                        && !event.keystroke.modifiers.alt
                        && !event.keystroke.modifiers.shift
                    {
                        let action = match event.keystroke.key.as_str() {
                            "r" => Some(Action::AudioBlindSentence(0)),
                            "up" => Some(Action::AudioBlindSentence(-1)),
                            "down" => Some(Action::AudioBlindSentence(1)),
                            "h" => Some(if this.audio.blind.revealed.is_some() {
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
                                this.video_horizontal_action(true)
                            } else {
                                Action::Next
                            },
                            window,
                            cx,
                        ),
                        "left" => this.handle(
                            if ["video", "listening"].contains(&this.page.as_str()) {
                                this.video_horizontal_action(false)
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
                                div()
                                    .flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        self.button(
                                            "floating-import-toggle",
                                            "悬浮窗",
                                            Action::FloatingImport,
                                            cx,
                                        )
                                        .outline(),
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
                            ),
                    )
                    .child(self.focus_timer_bar(cx))
                    .children(self.subtitle_task_banner(cx))
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
                    ),
            )
            .when_some(assistant, |view, assistant| view.child(assistant))
            .children(self.feedback_view(window, cx))
            .children(Root::render_dialog_layer(window, cx))
            .into_any_element()
    }
}
fn main() -> Result<()> {
    let args = std::env::args().collect::<Vec<_>>();
    if let Some(index) = args.iter().position(|arg| arg == "--runtime-check") {
        let path = PathBuf::from(
            args.get(index + 1)
                .context("--runtime-check 缺少报告路径")?,
        );
        let report = clarora_gpui::runtime::check(&path)?;
        ensure!(report.passed, "运行环境检查失败，详情见 {}", path.display());
        return Ok(());
    }
    let root = if let Some(index) = args.iter().position(|arg| arg == "--data-dir") {
        PathBuf::from(args.get(index + 1).context("--data-dir 缺少目录")?)
    } else {
        dirs::data_local_dir()
            .context("找不到本机数据目录")?
            .join("Clarora-GPUI")
    };
    #[cfg(debug_assertions)]
    let probe_dir = args
        .iter()
        .position(|arg| arg == "--ui-controller-check" || arg == "--ui-controller-quit-check")
        .map(|i| {
            args.get(i + 1)
                .context("--ui-controller-check 缺少证据目录")
                .map(PathBuf::from)
        })
        .transpose()?;
    #[cfg(not(debug_assertions))]
    ensure!(
        !args
            .iter()
            .any(|arg| arg == "--ui-controller-check" || arg == "--ui-controller-quit-check"),
        "原生协调验收仅支持调试构建"
    );
    #[cfg(not(debug_assertions))]
    let probe_dir: Option<PathBuf> = None;
    #[cfg(debug_assertions)]
    let library = if let Some(out) = &probe_dir {
        ui_controller_probe::prepare(out)?
    } else {
        Library::open(root)?
    };
    #[cfg(not(debug_assertions))]
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
                KeyBinding::new("cmd-c", CopySubtitleSelection, Some("SubtitleText")),
                KeyBinding::new("ctrl-c", CopySubtitleSelection, Some("SubtitleText")),
                KeyBinding::new("escape", ClearSubtitleSelection, Some("SubtitleText")),
                KeyBinding::new("cmd-o", OpenFile, None),
                KeyBinding::new("ctrl-o", OpenFile, None),
                KeyBinding::new("cmd-b", ToggleNavigation, None),
                KeyBinding::new("ctrl-b", ToggleNavigation, None),
                KeyBinding::new("cmd-j", ToggleAssistant, None),
                KeyBinding::new("ctrl-j", ToggleAssistant, None),
                KeyBinding::new("cmd-shift-i", ToggleFloatingImport, None),
                KeyBinding::new("ctrl-shift-i", ToggleFloatingImport, None),
                KeyBinding::new("cmd-shift-enter", ImportAudioCommand, None),
                KeyBinding::new("ctrl-shift-enter", ImportAudioCommand, None),
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
                    name: "Clarora".into(),
                    items: vec![
                        MenuItem::action("设置…", OpenSettings),
                        MenuItem::action("显示或隐藏左侧导航", ToggleNavigation),
                        MenuItem::action("切换浅色 / 深色", ToggleAppearance),
                        MenuItem::action("取消当前任务", CancelTask),
                        MenuItem::action("退出 Clarora", Quit),
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
                        MenuItem::action("放大文档 / 字号", PdfZoomIn),
                        MenuItem::action("缩小文档 / 字号", PdfZoomOut),
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
                        MenuItem::action("显示或隐藏导入悬浮窗", ToggleFloatingImport),
                        MenuItem::action("命令导入音频", ImportAudioCommand),
                        MenuItem::action("向当前视频字幕提问", AskVideoCurrent),
                        MenuItem::action("向整段视频字幕提问", AskVideoAll),
                        MenuItem::action("新会话", NewChat),
                        MenuItem::action("发送问题", AskAi),
                    ],
                },
            ]);
            let bounds = Bounds::centered(None, size(px(1240.), px(860.)), cx);
            let mut quit_view = None;
            let main = cx
                .open_window(
                    WindowOptions {
                        show: probe_dir.is_none(),
                        focus: probe_dir.is_none(),
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        titlebar: Some(TitlebarOptions {
                            title: Some("Clarora".into()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    |window, cx| {
                        let view = cx.new(|cx| {
                            #[allow(unused_mut)]
                            let mut view = AppView::new(library, window, cx);
                            #[cfg(debug_assertions)]
                            if let Some(out) = &probe_dir {
                                view.probe =
                                    Some(ui_controller_probe::Probe::new(out).with_global_quit(
                                        args.iter().any(|arg| arg == "--ui-controller-quit-check"),
                                    ));
                            }
                            view
                        });
                        quit_view = Some(view.downgrade());
                        view.read(cx).focus.focus(window);
                        cx.new(|cx| Root::new(view, window, cx))
                    },
                )
                .expect("无法打开 Clarora 窗口");
            cx.on_action(move |_: &Quit, cx| {
                if let Some(view) = &quit_view {
                    let _ = view.update(cx, |this, cx| this.prepare_close(cx));
                }
            });
            cx.on_window_closed(move |cx| {
                if !cx
                    .windows()
                    .iter()
                    .any(|w| w.window_id() == main.window_id())
                {
                    cx.quit();
                }
            })
            .detach();
            if probe_dir.is_none() {
                cx.activate(true);
            }
        });
    Ok(())
}

impl Drop for AppView {
    fn drop(&mut self) {
        if let Some(cancel) = &self.tasks.cancel {
            cancel.store(true, Ordering::SeqCst);
        }
        if let Some(cancel) = &self.audio.waveform_cancel {
            cancel.store(true, Ordering::SeqCst);
        }
        self.save_media_position();
        let _ = self.lib.stat("listen_seconds", self.media.stats_ticks);
    }
}
