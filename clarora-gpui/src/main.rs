#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod action_router;
mod app_render;
mod app_state;
mod assistant_history_ui;
#[cfg(debug_assertions)]
mod assistant_latency_probe;
#[cfg(debug_assertions)]
mod assistant_selection_probe;
mod assistant_ui;
#[cfg(debug_assertions)]
mod audio_boundary_probe;
mod audio_controller;
mod audio_current_ui;
mod audio_player_ui;
#[cfg(debug_assertions)]
mod audio_remove_probe;
mod audio_remove_ui;
mod audio_repeat_controller;
#[cfg(debug_assertions)]
mod audio_repeat_probe;
#[cfg(debug_assertions)]
mod audio_selection_probe;
mod audio_subtitle_ui;
mod audio_subtitles_controller;
#[cfg(debug_assertions)]
mod audio_subtitles_probe;
#[cfg(debug_assertions)]
mod backup_progress_probe;
mod chat_controller_ui;
mod chatgpt_ui;
mod clip_selection_ui;
mod cloud_ui;
mod data_controller;
mod design;
mod diagnostics;
mod domains;
mod feedback_ui;
#[cfg(debug_assertions)]
mod flashcard_shortcut_probe;
mod floating_ui;
#[cfg(debug_assertions)]
mod focus_mode_probe;
mod focus_mode_ui;
mod focus_ui;
mod library_ui;
mod model_controller;
#[cfg(debug_assertions)]
mod model_navigation_probe;
#[cfg(debug_assertions)]
mod native_probe_input;
mod pages_ui;
mod playback_controller;
#[cfg(debug_assertions)]
mod playback_session_probe;
mod reading_ui;
mod saved_clip_ui;
#[cfg(debug_assertions)]
mod sentence_search_probe;
mod sentence_search_ui;
mod settings_ui;
mod speaking_ui;
mod stats_ui;
mod subtitle_follow_ui;
mod subtitle_text_ui;
#[cfg(debug_assertions)]
mod subtitle_workbench_probe;
mod task_controller;
#[cfg(debug_assertions)]
mod task_lifecycle_probe;
mod task_status;
mod task_ui;
#[cfg(debug_assertions)]
mod ui_controller_probe;
mod video_controller;
mod video_job_ui;
mod video_library_ui;
mod video_player_ui;
#[cfg(debug_assertions)]
mod video_resume_probe;
mod video_transcript_ui;
mod writing_clipboard;
mod writing_ui;
use anyhow::{Context as _, Result, ensure};
use clarora_gpui::playback_session::{Mode as PlaybackMode, Owner as PlaybackOwner};
use clarora_gpui::task_runtime::{self, Scope as TaskScope};
use clarora_gpui::{
    ai, audio_library, backup, bilibili, blind_listening, clip_marks, credentials,
    data::{Card, Library, Row, id, now, text},
    documents, ebook, focus_notes, focus_timer,
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
use task_status::{TaskNotice, TaskState};
actions!(
    clarora_gpui_actions,
    [
        Quit,
        CopySubtitleSelection,
        ClearSubtitleSelection,
        OpenFile,
        ToggleAssistant,
        QuoteSelectedText,
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
    ("speaking", "口语练习", "题库、草稿与录音"),
    ("writing", "写作练习", "保存草稿与 AI 批改"),
    ("library", "音频管理", "整理练习组"),
    ("pdf", "文档阅读", "PDF / EPUB / TXT"),
    ("ocr", "拍照识字", "从图片提取"),
    ("graph", "词汇表", "发现词语关系"),
    ("stats", "学习统计", "记录每一点进步"),
    ("meditation", "冥想放松", "给专注留白"),
    ("settings", "设置", "AI 与存储"),
    ("sentences", "句型搜索", "搜遍音视频，逐句对照听"),
];
/// Display order only; Cmd/Ctrl+number keeps the PAGES order above.
const NAV_ORDER: &[&str] = &[
    "listening",
    "video",
    "speaking",
    "writing",
    "sentences",
    "library",
    "pdf",
    "ocr",
    "graph",
    "stats",
    "meditation",
    "settings",
    "discover",
    "flashcard",
];
use domains::Action;

use task_controller::{Outcome, TaskUpdate};
#[derive(Clone, Copy, PartialEq, Eq)]
enum VideoCaptionSource {
    Transcript,
    Track,
    Hidden,
}
struct AppView {
    #[cfg(debug_assertions)]
    diagnostic: Option<diagnostics::Active>,
    lib: Library,
    data: clarora_gpui::view_data::State,
    loads: data_controller::Loads,
    settings: app_state::SettingsState,
    chatgpt: chatgpt_ui::State,
    cloud: cloud_ui::State,
    chat: clarora_gpui::chat_controller::Controller,
    assistant: assistant_ui::State,
    chat_requested_context: String,
    chat_closing: Option<chat_controller_ui::Closing>,
    tasks: app_state::TasksState,
    media: app_state::PlaybackState,
    video: app_state::VideoState,
    audio: app_state::AudioState,
    sentences: sentence_search_ui::State,
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
    speaking: speaking_ui::SpeakingLibrary,
    timer_chimes: timer_chimes::Chimes,
    rendered_images: clarora_gpui::render_images::RenderedImages,
    timer: focus_timer::Timer,
    timer_seconds: u64,
    timer_retry: Option<Instant>,
    focus_mode: focus_mode_ui::Mode,
    focus_notes: Vec<focus_notes::Note>,
    focus_note_open: bool,
    focus_note_close_saved: Option<String>,
    pdf_path: Option<PathBuf>,
    pdf_image: Option<std::sync::Arc<RenderImage>>,
    pdf_page: u16,
    pdf_pages: u16,
    pdf_width: i32,
    pdf_bookmarks: Vec<(String, u16)>,
    ebook_reading: Option<ebook::Reading>,
    reading_sidebar_open: bool,
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
            ("question", "输入问题或粘贴内容，Enter 换行…", true, false),
            (
                "sentence_query",
                "例如 When、when is、Why did",
                false,
                false,
            ),
            ("reference", "朗读参考原文", true, false),
            ("speaking_group", "分组名称", false, false),
            ("speaking_group_tag", "例如：托福、校园", false, false),
            ("speaking_title", "可选，方便查找题目", false, false),
            (
                "speaking_answer_draft",
                "写下或粘贴你的口语作答草稿",
                true,
                false,
            ),
            ("speaking_target_words", "不设目标", false, false),
            ("focus_note", "记下此刻的想法、疑问或待办…", true, false),
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
            ("ai_provider", "custom / opencode_go", false, false),
            ("ai_profile_name", "例如：DeepSeek、公司代理", false, false),
            ("storage_profile_name", "例如：主存储、备用桶", false, false),
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
                    if key == "question" {
                        input = input.auto_grow(4, 10).soft_wrap(true);
                    } else if multi {
                        input = input.multi_line(true).rows(3);
                    }
                    if key == "speaking_target_words" {
                        input = input.validate(|value, _| {
                            value.is_empty()
                                || (value.bytes().all(|byte| byte.is_ascii_digit())
                                    && value.parse::<usize>().is_ok_and(|count| {
                                        (1..=clarora_gpui::speaking::MAX_TARGET_WORDS)
                                            .contains(&count)
                                    }))
                        });
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
        let focus_notes = focus_notes::list(&lib, 20).unwrap_or_default();
        let mut this = Self {
            #[cfg(debug_assertions)]
            diagnostic: None,
            data: Default::default(),
            loads: Default::default(),
            settings: app_state::SettingsState::new(&lib),
            chatgpt: chatgpt_ui::State::new(&lib),
            cloud: cloud_ui::State::new(&lib),
            chat: clarora_gpui::chat_controller::Controller::new(lib.clone()),
            assistant: Default::default(),
            chat_requested_context: String::new(),
            chat_closing: None,
            tasks: app_state::TasksState::new(),
            media: app_state::PlaybackState::new(),
            video: app_state::VideoState::new(),
            audio: app_state::AudioState::new(&lib),
            sentences: Default::default(),
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
            speaking: Default::default(),
            timer_seconds: timer.display_seconds(focus_timer::now_ms()),
            timer,
            timer_retry: None,
            focus_mode: focus_mode_ui::Mode::default(),
            focus_notes,
            focus_note_open: false,
            focus_note_close_saved: None,
            pdf_path: None,
            pdf_image: None,
            pdf_page: 0,
            pdf_pages: 0,
            pdf_width: 1000,
            pdf_bookmarks: vec![],
            ebook_reading: None,
            reading_sidebar_open: true,
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
                            if !this.media.matches_snapshot(&snapshot) {
                                return;
                            }
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
        this.sentences.subscription = Some(cx.subscribe_in(
            &this.inputs["sentence_query"],
            window,
            |this, input, event, window, cx| {
                if this.page == "sentences"
                    && matches!(event, InputEvent::PressEnter { secondary: false })
                    && !input.update(cx, |state, cx| {
                        state.marked_text_range(window, cx).is_some()
                    })
                {
                    this.handle(
                        Action::Sentence(sentence_search_ui::Command::Search),
                        window,
                        cx,
                    );
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
        for key in ["reference", "speaking_title"] {
            cx.subscribe(&this.inputs[key], |_, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            })
            .detach();
        }
        cx.subscribe(&this.inputs["focus_note"], |this, _, event, _| {
            if matches!(event, InputEvent::Change) {
                this.focus_note_close_saved = None;
            }
        })
        .detach();
        for key in ["speaking_answer_draft", "speaking_target_words"] {
            cx.subscribe(&this.inputs[key], |this, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    this.schedule_speaking_draft_save(cx);
                }
            })
            .detach();
        }
        if this.page == "speaking" {
            if let Err(error) = this.reload_speaking_questions(window, cx) {
                this.report_status(FeedbackKind::Error, format!("读取口语题库失败：{error:#}"));
            }
        }
        let owner = cx.entity().downgrade();
        this.writing = Some(cx.new(|cx| {
            writing_ui::WritingView::new(
                this.lib.clone(),
                this.page_scroll.clone(),
                owner,
                window,
                cx,
            )
        }));
        this.audio.practice_id = this.lib.setting("library_group").unwrap_or_default();
        this.refresh();
        if this.lib.setting("floating_import_open").ok().as_deref() == Some("true") {
            cx.defer_in(window, |this, window, cx| {
                this.toggle_floating_import(window, cx)
            });
        }
        match this
            .lib
            .setting("assistant_reference_draft")
            .and_then(|raw| {
                if raw.is_empty() {
                    return Ok(Vec::new());
                }
                let references: Vec<clarora_gpui::assistant::Reference> =
                    serde_json::from_str(&raw)?;
                clarora_gpui::assistant::validate(&references)?;
                Ok(references)
            }) {
            Ok(references) => this.assistant.references = references,
            Err(error) => this.assistant.error = format!("未恢复引用草稿：{error:#}"),
        }
        if this.lib.setting("assistant_open").ok().as_deref() == Some("true") {
            this.assistant.enabled = true;
            this.assistant_open = true;
            this.sync_chat(window, cx);
        }
        let ai_result = ai::load_config(&this.lib);
        match ai_result {
            Ok(c) => this.apply_ai_config(&c, window, cx),
            Err(e) => this.report_status(FeedbackKind::Error, e.to_string()),
        }
        if let Ok(c) = serde_json::from_str::<backup::StorageConfig>(
            &this.lib.setting("storage_config").unwrap_or_default(),
        ) {
            this.apply_storage_config(&c.normalized(), None, window, cx);
        }
        if let Ok((profiles, active)) = backup::profiles(&this.lib)
            && let Some(profile) = profiles.iter().find(|profile| profile.id == active)
        {
            this.set("storage_profile_name", profile.name.clone(), window, cx);
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
            this.tasks.audio_subtitles.cancel();
            this.tasks.models.cancel();
            if let Err(error) = this.flush_speaking_draft(cx) {
                eprintln!("退出前口语草稿保存失败：{error:#}");
            }
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
            if this.page == "video" {
                this.media.open(
                    PlaybackOwner::Video,
                    video.path.clone(),
                    position,
                    0.,
                    true,
                    PlaybackMode::Normal,
                );
            }
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
    fn apply_ai_config(&self, config: &ai::Config, window: &mut Window, cx: &mut Context<Self>) {
        let (config, _) = ai::shared_settings(config.clone(), String::new(), String::new());
        for (k, v) in [
            (
                "ai_provider",
                if config.provider.is_empty() {
                    ai::PROVIDER_CUSTOM.to_string()
                } else {
                    config.provider
                },
            ),
            ("base_url", config.base_url),
            ("model", config.model),
            ("vision_model", config.vision_model),
            ("local_model", config.local_model),
            ("engine", config.engine),
        ] {
            self.set(k, v, window, cx);
        }
    }
    /// Populate the storage form without ever reading a secret into an input.
    fn apply_storage_config(
        &self,
        config: &backup::StorageConfig,
        profile_name: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for (k, v) in [
            (
                "provider",
                if config.provider.is_empty() {
                    "s3".to_string()
                } else {
                    config.provider.clone()
                },
            ),
            ("endpoint", config.endpoint.clone()),
            ("region", config.region.clone()),
            ("bucket", config.bucket.clone()),
            ("access_key", config.access_key_id.clone()),
            ("prefix", config.prefix.clone()),
            ("path_style", config.path_style.to_string()),
            ("secret_key", String::new()),
            ("session_token", String::new()),
        ] {
            self.set(k, v, window, cx);
        }
        if let Some(name) = profile_name {
            self.set("storage_profile_name", name.to_string(), window, cx);
        }
    }
    fn focus_phase_completed(&mut self, event: focus_timer::Event) {
        let (cue, message) = match event {
            focus_timer::Event::GoalReached => (
                timer_chimes::Cue::Start,
                "已达到最低专注目标，可以继续学习，准备好时结束本轮",
            ),
            focus_timer::Event::BreakEnded => {
                (timer_chimes::Cue::End, "休息已结束，准备好后开始下一轮")
            }
        };
        self.play_timer_chime(cue);
        self.report_status(FeedbackKind::Info, message.into());
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
        if self.page == "listening" && self.media.session.is_audio(&self.audio.current_id) {
            let _ = self.lib.set_setting(
                "last_listening_position",
                &json!({"id":self.audio.current_id,"position":self.audio_saved_position()})
                    .to_string(),
            );
        } else if self.page == "video" {
            if let Some(path) = &self.video.path {
                let playback = self.media.player.snapshot();
                // A page switch can happen before the UI's next clock poll,
                // or while the player still reports the previous audio/card.
                if self.media.session.is_video()
                    && self.media.session.path.as_ref() == Some(path)
                    && self.media.matches_snapshot(&playback)
                    && playback.path == path.to_string_lossy()
                    && playback.duration > 0.
                    && playback.position.is_finite()
                    && playback.position >= 0.
                {
                    // A page return rebuilds triple repeat from play one. Save
                    // the validated sentence start so loadfile's start cannot
                    // later overwrite its seek with a mid-sentence bookmark.
                    let position = self
                        .media
                        .session
                        .video_sentence()
                        .filter(|session| {
                            let (a, b) = session.range;
                            session.total.is_some()
                                && a.is_finite()
                                && b.is_finite()
                                && a >= 0.
                                && a < b
                                && b <= playback.duration
                                && self.video.cues.get(session.index).is_some_and(|cue| {
                                    (cue.start - a).abs() < 0.001
                                        && (cue.end.min(playback.duration) - b).abs() < 0.001
                                })
                        })
                        .map(|session| session.range.0)
                        .unwrap_or(playback.position);
                    let _ = self.lib.set_setting(
                        "last_video_position",
                        &json!({"path":path,"position":position}).to_string(),
                    );
                }
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
        if let Some(card) = self.current_card().filter(|card| !card.media.is_empty()) {
            let owner = PlaybackOwner::Card {
                kind: card.kind,
                id: card.id,
            };
            let path = PathBuf::from(card.media);
            if self.media.preloaded_index == Some(self.index) {
                self.media.play_preloaded(owner, path);
            } else {
                self.media.open(
                    owner,
                    path,
                    card.start,
                    card.end,
                    false,
                    PlaybackMode::Normal,
                );
            }
        } else {
            self.media.release();
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
                let next = &self.cards[index];
                if !next.media.is_empty() {
                    self.media.preload_card(
                        index,
                        &PathBuf::from(&next.media),
                        next.start,
                        next.end,
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
                        !self.tasks.foreground.is_running(),
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
                    let video = self.media.session.path.as_ref().context("请先打开视频")?;
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
                if self.tasks.foreground.is_running() {
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
                self.media.open(
                    PlaybackOwner::Background,
                    path,
                    0.,
                    0.,
                    false,
                    PlaybackMode::Track,
                );
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
        if self.tasks.foreground.is_running() {
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
            "listening"
                if self.media.session.loop_a.is_some() && self.media.session.loop_b.is_some() =>
            {
                let (front, back) = subtitles::excerpt(
                    &self.audio.cues,
                    self.media.session.loop_a.unwrap(),
                    self.media.session.loop_b.unwrap(),
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
        self.menu_control(key, label, options, false, cx)
    }
    fn surface_menu(
        &self,
        key: &'static str,
        label: String,
        options: Vec<(String, Action, bool, bool)>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        self.menu_control(key, label, options, true, cx)
    }
    fn menu_control(
        &self,
        key: &'static str,
        label: String,
        options: Vec<(String, Action, bool, bool)>,
        surface: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let view = cx.entity().downgrade();
        let focus = self.focus.clone();
        let theme_colors = surface || self.page == "listening" || self.page == "library";
        let variant = ButtonCustomVariant::new(cx)
            .foreground(if theme_colors {
                cx.theme().foreground
            } else {
                rgb(0xe8f0eb).into()
            })
            .hover(if theme_colors {
                cx.theme().accent
            } else {
                rgb(0x303c38).into()
            })
            .active(if theme_colors {
                cx.theme().accent
            } else {
                rgb(0x41554b).into()
            });
        let variant = if surface {
            variant
                .color(cx.theme().background)
                .border(cx.theme().border)
        } else {
            variant
        };
        // The menu is transient; the checked value is rebuilt from current playback state.
        Button::new(key)
            .label(label)
            .small()
            .when(surface, |button| button.h(px(32.)).px_3())
            .custom(variant)
            .dropdown_menu_with_anchor(Corner::BottomRight, move |mut menu, _, _| {
                menu = menu.action_context(focus.clone());
                for (label, action, checked, disabled) in options.clone() {
                    let view = view.clone();
                    menu = menu.item(
                        PopupMenuItem::new(label)
                            .checked(checked)
                            .disabled(disabled)
                            .on_click(move |_, window, cx| {
                                let view = view.clone();
                                let action = action.clone();
                                let handle = window.window_handle();
                                // Dismiss restores the menu's action context.
                                // Run afterward so actions opening an input or
                                // dialog retain their newly assigned focus.
                                cx.defer(move |cx| {
                                    let _ = handle.update(cx, move |_, window, cx| {
                                        let _ = view
                                            .update(cx, |this, cx| this.handle(action, window, cx));
                                    });
                                });
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
fn main() -> Result<()> {
    let args = std::env::args().collect::<Vec<_>>();
    let startup = diagnostics::Startup::parse(&args)?;
    if let Some(path) = &startup.catalogue {
        fs::write(
            path,
            serde_json::to_vec_pretty(&diagnostics::Startup::catalogue())?,
        )?;
        return Ok(());
    }
    if let Some(path) = &startup.runtime {
        let report = clarora_gpui::runtime::check(path)?;
        ensure!(report.passed, "运行环境检查失败，详情见 {}", path.display());
        return Ok(());
    }
    let root = match startup.data_dir {
        Some(path) => path,
        None => dirs::data_local_dir()
            .context("找不到本机数据目录")?
            .join("Clarora-GPUI"),
    };
    #[cfg(debug_assertions)]
    let library = if let Some(launch) = &startup.diagnostic {
        launch.prepare()?
    } else {
        Library::open(root)?
    };
    #[cfg(not(debug_assertions))]
    let library = Library::open(root)?;
    let interactive = startup.diagnostic.is_none();
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
                KeyBinding::new("cmd-shift-l", QuoteSelectedText, None),
                KeyBinding::new("ctrl-shift-l", QuoteSelectedText, None),
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
                        MenuItem::action("向当前视频的英文全文提问", AskVideoAll),
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
                        show: interactive,
                        focus: interactive,
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
                            if let Some(launch) = &startup.diagnostic {
                                view.diagnostic = Some(launch.start());
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
            if interactive {
                cx.activate(true);
            }
        });
    Ok(())
}

impl Drop for AppView {
    fn drop(&mut self) {
        if let Some(cancel) = self.tasks.foreground.cancel_token() {
            cancel.store(true, Ordering::SeqCst);
        }
        if let Some(cancel) = &self.audio.waveform_cancel {
            cancel.store(true, Ordering::SeqCst);
        }
        self.save_media_position();
        let _ = self.lib.stat("listen_seconds", self.media.stats_ticks);
    }
}
