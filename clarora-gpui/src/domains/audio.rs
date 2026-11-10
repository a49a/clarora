//! Audio commands own their action effects and execution.
use super::*;
pub(super) enum Command {
    Audio(String),
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
    AudioReview,
    AudioReviewStep(i8),
    AudioReviewStop,
    AudioTools,
    AudioFont(u8),
    AudioCurrent,
    Highlight,
    ClearHighlights,
    AudioFollow,
    AudioCopySubtitle(String, Option<Cue>),
    AudioRepeat(usize),
    AudioSentence(i8),
    Transcribe,
    Translate,
    AudioGroupSubtitles(clarora_gpui::audio_batch::Mode),
}
impl Command {
    pub(super) fn effects(&self, app: &AppView) -> Effects {
        Effects {
            restore_focus: (app.page == "listening"
                && matches!(
                    self,
                    Command::Audio(..)
                        | Command::AudioPractice(..)
                        | Command::AudioBlind(..)
                        | Command::AudioRepeat(..)
                        | Command::AudioSentence(..)
                )),
            sync_audio_mode: app.page == "listening"
                && matches!(
                    self,
                    Command::AudioPractice(..)
                        | Command::AudioBlind(..)
                        | Command::AudioBlindRow(..)
                        | Command::AudioClipMode
                ),
            leave_video_sentence: match self {
                Command::Transcribe => true,
                _ => false,
            },
            ..Effects::default()
        }
    }
    pub(super) fn execute(
        self,
        app: &mut AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Result<()> {
        match self {
            Command::ClearHighlights => {
                app.audio.key_points.clear();
                app.lib
                    .set_setting(&format!("key_points_{}", app.audio.current_id), "[]")?;
                app.refresh();
            }

            Command::Highlight => {
                ensure!(
                    !(app.page == "listening" && app.audio.blind.enabled),
                    "请先切回字幕模式"
                );
                let audio = app.audio.current_id.clone();
                let cues = app.audio.cues.clone();
                ensure!(
                    !audio.is_empty() && !cues.is_empty(),
                    "请先选择音频并加载字幕"
                );
                app.begin("正在标注学习重点…", move |lib, cancel, _| {
                    Ok(Outcome::KeyPoints(
                        audio,
                        ai::key_points(&lib, &cues, &cancel)?,
                    ))
                });
            }

            Command::Audio(key) => {
                let row = app
                    .audio
                    .audios
                    .iter()
                    .find(|r| text(r, "id") == key)
                    .cloned()
                    .context("音频不存在")?;
                if app.page == "video" {
                    app.save_media_position();
                }
                app.reset_video_sentence_request();
                app.open_audio(&row);
                app.page = "listening".into();
                app.lib.set_setting("last_page", "listening")?;
            }

            Command::AudioPractice(key) => {
                app.reset_audio_sentence_request();
                app.invalidate_blind();
                ensure!(
                    app.audio.practices.iter().any(|r| text(r, "id") == key),
                    "练习组不存在"
                );
                app.lib.set_setting("library_group", &key)?;
                app.audio.practice_id = key;
                if let Some(row) = app
                    .audio
                    .library
                    .merges
                    .learning(&app.audio.audios, &app.audio.practice_id)
                    .first()
                    .map(|row| (*row).clone())
                {
                    app.open_audio(&row);
                } else {
                    app.media.player.set("pause", "yes");
                    app.audio.current_id.clear();
                    app.media.release();
                    app.audio.cues.clear();
                    app.audio.scroll.reset(0);
                    app.audio.active = None;
                    app.audio.waveform = None;
                    app.audio.key_points.clear();
                    if let Some(cancel) = app.audio.waveform_cancel.take() {
                        cancel.store(true, Ordering::SeqCst);
                    }
                    app.audio.waveform_rx = None;
                    app.lib.set_setting("last_listening_audio", "")?;
                    if app.media.session.mode == PlaybackMode::Ab {
                        app.media.session.mode = PlaybackMode::Normal;
                    }
                    app.media.session.loop_a = None;
                    app.media.session.loop_b = None;
                }
            }

            Command::AudioBlind(enabled) => {
                if enabled {
                    app.stop_audio_review(true);
                }
                app.subtitle_text.borrow_mut().clear();
                ensure!(app.page == "listening", "请先进入音频学习");
                app.audio.blind.enabled = enabled;
                app.invalidate_blind();
                app.assistant_open = false;
                app.output.clear();
                app.chat.reasoning.clear();
                if enabled {
                    if app.media.session.loop_a.is_none() || app.media.session.loop_b.is_none() {
                        app.media.session.loop_a = None;
                        app.media.session.loop_b = None;
                        if app.media.session.mode == PlaybackMode::Ab {
                            app.media.session.mode = PlaybackMode::Normal;
                        }
                    }
                    app.audio.tools_open = false;
                } else {
                    app.media.player.set("pause", "yes");
                }
                app.sync_chat(window, cx);
                window.focus(&app.focus);
            }

            Command::AudioBlindChinese(enabled) => {
                ensure!(
                    app.page == "listening" && app.audio.blind.enabled,
                    "请先开启盲听"
                );
                app.lib
                    .set_setting("blind_chinese_hint", if enabled { "true" } else { "false" })?;
                app.audio.blind_chinese = enabled;
                app.invalidate_blind();
                app.assistant_open = false;
                app.sync_chat(window, cx);
                window.focus(&app.focus);
            }

            action @ (Command::AudioBlindReveal | Command::AudioBlindRevealCue(..)) => {
                ensure!(
                    app.page == "listening" && app.audio.blind.enabled,
                    "请先开启盲听"
                );
                ensure!(
                    app.audio.blind_pending.is_none(),
                    "正在确认上一条字幕，请稍候"
                );
                ensure!(
                    app.audio
                        .blind_settle_until
                        .is_none_or(|t| Instant::now() >= t),
                    "正在定位，请稍候重试"
                );
                let playback = app.media.player.snapshot();
                ensure!(
                    !app.audio.current_id.is_empty()
                        && app
                            .media
                            .session
                            .path
                            .as_ref()
                            .is_some_and(|p| p.to_string_lossy() == playback.path),
                    "音频正在切换，请稍候"
                );
                let target = if let Command::AudioBlindRevealCue(audio_id, cue) = action {
                    ensure!(audio_id == app.audio.current_id, "音频已切换，请重新选择");
                    blind_listening::row_target(&app.audio.cues, &cue, playback.duration)
                        .context("字幕已变化或时间无效，请重新选择")?;
                    blind_listening::Revealed { cue, sparse: false }
                } else {
                    blind_listening::resolve(&app.audio.cues, playback.position, playback.duration)
                        .context("播放到有字幕的部分后再查看")?
                };
                app.invalidate_blind();
                app.assistant_open = false;
                app.sync_chat(window, cx);
                app.audio.blind_pending =
                    Some((Instant::now(), app.audio.current_id.clone(), target));
                app.media.player.set("pause", "yes");
                app.report_status(
                    FeedbackKind::Progress,
                    "正在暂停，确认后显示所选字幕…".into(),
                );
            }

            Command::AudioBlindHide => {
                app.invalidate_blind();
                app.sync_chat(window, cx);
                window.focus(&app.focus);
            }

            Command::AudioBlindResume(restart) => {
                ensure!(
                    app.page == "listening"
                        && app.audio.blind.enabled
                        && !app.audio.current_id.is_empty(),
                    "请先选择音频并开启盲听"
                );
                app.invalidate_blind();
                app.assistant_open = false;
                if restart {
                    app.media.player.command(["seek", "0", "absolute+exact"]);
                }
                app.media.player.set("pause", "no");
                app.audio.blind_settle_until = Some(Instant::now() + Duration::from_millis(200));
                app.sync_chat(window, cx);
                window.focus(&app.focus);
            }

            Command::AudioBlindRow(audio_id, cue, word_index) => {
                ensure!(
                    app.page == "listening" && app.audio.blind.enabled,
                    "请先开启盲听"
                );
                let playback = app.media.player.snapshot();
                ensure!(
                    audio_id == app.audio.current_id
                        && !audio_id.is_empty()
                        && app
                            .media
                            .session
                            .path
                            .as_ref()
                            .is_some_and(|p| p.to_string_lossy() == playback.path),
                    "音频正在切换，请重试"
                );
                let start = match word_index {
                    Some(index) => blind_listening::word_target(
                        &app.audio.cues,
                        &cue,
                        index,
                        playback.duration,
                    ),
                    None => blind_listening::row_target(&app.audio.cues, &cue, playback.duration),
                }
                .context("字幕已变化或时间无效，请重新选择")?;
                app.invalidate_blind();
                app.assistant_open = false;
                if app.seek_audio_repeat(start, true)? {
                    app.sync_chat(window, cx);
                    window.focus(&app.focus);
                    return Ok(());
                }
                let outside_loop = app.media.session.mode == PlaybackMode::Ab
                    && app
                        .media
                        .session
                        .loop_a
                        .zip(app.media.session.loop_b)
                        .is_some_and(|(a, b)| start < a || start >= b);
                if outside_loop {
                    app.media.session.mode = PlaybackMode::Normal;
                    app.media.session.loop_a = None;
                    app.media.session.loop_b = None;
                }
                app.media.player.command([
                    "seek".into(),
                    start.to_string(),
                    "absolute+exact".into(),
                ]);
                app.media.player.set("pause", "no");
                app.audio.blind_settle_until = Some(Instant::now() + Duration::from_millis(250));
                if outside_loop {
                    app.report_status(FeedbackKind::Info, "已退出 A–B 循环".into());
                }
                app.sync_chat(window, cx);
                window.focus(&app.focus);
            }

            Command::AudioBlindSentence(direction) => {
                if app.audio.repeat_mode.is_sentence() {
                    app.audio_sentence(direction)?;
                    return Ok(());
                }
                ensure!(
                    app.page == "listening" && app.audio.blind.enabled,
                    "请先开启盲听"
                );
                ensure!(
                    app.audio.blind_pending.is_none(),
                    "正在确认上一条字幕，请稍候"
                );
                ensure!(
                    app.audio
                        .blind_settle_until
                        .is_none_or(|t| Instant::now() >= t),
                    "正在定位，请稍候重试"
                );
                let playback = app.media.player.snapshot();
                ensure!(
                    !app.audio.current_id.is_empty()
                        && app
                            .media
                            .session
                            .path
                            .as_ref()
                            .is_some_and(|p| p.to_string_lossy() == playback.path),
                    "音频正在切换，请稍候"
                );
                let target = blind_listening::sentence_target(
                    &app.audio.cues,
                    playback.position,
                    playback.duration,
                    app.audio.blind.revealed.as_ref(),
                    direction,
                    app.media
                        .session
                        .loop_a
                        .zip(app.media.session.loop_b)
                        .filter(|(a, b)| app.media.session.mode == PlaybackMode::Ab && b > a),
                )
                .context("当前范围内没有可播放的字幕句")?;
                app.invalidate_blind();
                app.assistant_open = false;
                app.media.player.command([
                    "seek".into(),
                    target.cue.start.to_string(),
                    "absolute+exact".into(),
                ]);
                app.media.player.set("pause", "no");
                app.audio.blind_settle_until = Some(Instant::now() + Duration::from_millis(250));
                app.sync_chat(window, cx);
                window.focus(&app.focus);
            }

            Command::AudioClipMode => {
                ensure!(!app.audio.blind.enabled, "请先切回字幕模式");
                app.stop_audio_review(true);
                if app.audio.repeat_mode.is_sentence() {
                    app.set_audio_repeat_mode(0)?;
                }
                app.audio.clip_mode = !app.audio.clip_mode;
                if !app.audio.clip_mode {
                    app.media.session.loop_a = None;
                    app.media.session.loop_b = None;
                    if app.media.session.mode == PlaybackMode::Ab {
                        app.media.session.mode = PlaybackMode::Normal;
                    }
                }
            }

            Command::AudioReview => {
                ensure!(app.page == "listening", "请先进入音频学习");
                app.start_audio_review()?;
            }

            Command::AudioReviewStep(direction) => {
                ensure!(app.page == "listening", "请先进入音频学习");
                app.step_audio_review(direction)?;
            }

            Command::AudioReviewStop => app.stop_audio_review(true),

            Command::AudioTools => {
                ensure!(!app.audio.blind.enabled, "请先切回字幕模式");
                app.audio.tools_open = !app.audio.tools_open;
            }

            Command::AudioCurrent => app.audio.show_current = !app.audio.show_current,

            Command::AudioFollow => {
                app.audio.follow = !app.audio.follow;
                if app.audio.follow {
                    if let Some(i) = app.audio.active {
                        app.audio.center.request(i);
                    }
                }
            }

            Command::AudioFont(size) => {
                ensure!([14, 16, 18, 20, 24, 28, 32].contains(&size), "无效字幕字号");
                app.audio.font_size = size;
                app.lib
                    .set_setting("subtitle_font_size", &size.to_string())?;
                app.invalidate_blind();
                app.audio.scroll.reset(app.audio.cues.len());
                if let Some(index) = app.audio.active {
                    app.audio.center.request(index);
                }
            }

            Command::AudioRepeat(mode) => app.set_audio_repeat_mode(mode)?,

            Command::AudioSentence(direction) => app.audio_sentence(direction)?,

            Command::AudioCopySubtitle(audio_id, cue) => {
                ensure!(
                    app.page == "listening"
                        && !audio_id.is_empty()
                        && audio_id == app.audio.current_id,
                    "音频已切换，请重新复制"
                );
                let body = blind_listening::copy_text(
                    &app.audio.cues,
                    cue.as_ref(),
                    &app.audio.blind,
                    app.audio.blind_chinese,
                )
                .context("暂无可复制的字幕；盲听时请先翻开对应句子")?;
                cx.write_to_clipboard(ClipboardItem::new_string(body));
                app.report_status(
                    FeedbackKind::Info,
                    if cue.is_some() {
                        "已复制这句字幕"
                    } else {
                        "已复制全部字幕（不含时间戳）"
                    }
                    .into(),
                );
            }

            action @ (Command::Transcribe | Command::Translate) => {
                let mode = if matches!(action, Command::Transcribe) {
                    clarora_gpui::audio_batch::Mode::English
                } else {
                    clarora_gpui::audio_batch::Mode::Chinese
                };
                let row = app.selected().context("请先选择音频")?;
                if mode == clarora_gpui::audio_batch::Mode::Chinese {
                    ensure!(!app.audio.cues.is_empty(), "请先导入或生成原文字幕");
                }
                let target = audio_subtitles_controller::Target {
                    audio: Some(text(&row, "id")),
                    name: text(&row, "name"),
                    mode,
                };
                app.begin_audio_subtitles(
                    clarora_gpui::audio_batch::Plan::single(row, mode)?,
                    target,
                )?;
            }

            Command::AudioGroupSubtitles(mode) => {
                ensure!(
                    !app.tasks.audio_subtitles.lane.is_running(),
                    "已有音频字幕任务正在处理，请完成或取消后重试"
                );
                let plan =
                    clarora_gpui::audio_batch::Plan::new(&app.lib, &app.audio.practice_id, mode)?;
                let target = audio_subtitles_controller::Target {
                    audio: None,
                    name: plan.group_name.clone(),
                    mode,
                };
                app.begin_audio_subtitles(plan, target)?;
            }
        }
        Ok(())
    }
}

// Completion policy belongs to this domain: cancellation may race a durable commit.
pub(super) enum Completed {
    KeyPoints(String, Vec<ai::KeyPoint>),
    TtsReady(Row),
    AudioMerged(clarora_gpui::audio_tools::Merged),
}
impl Completed {
    pub(super) fn apply(
        self,
        app: &mut AppView,
        context: &mut Completion,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) {
        match self {
            Completed::AudioMerged(merged) => {
                context.cancelled = false;
                if app.audio.practice_id == text(&merged.row, "practice_id") {
                    app.audio.library.selected = text(&merged.row, "id");
                    app.audio.checked.clear();
                    app.audio.library.batch = false;
                    app.audio.library.filter = 0;
                    app.audio.library.show_sources = false;
                    app.set("library_search", "", window, cx);
                    if app.page == "listening" {
                        app.open_audio(&merged.row);
                        app.media.player.set("pause", "yes");
                    }
                }
                app.report_status(
                    FeedbackKind::Info,
                    format!(
                        "已合并 {} 条音频。原音频和字幕保留，学习时使用合并版本。{}",
                        merged.sources,
                        if merged.subtitles_complete {
                            "字幕已按合并顺序衔接。"
                        } else {
                            "部分原音频缺少字幕，请为合并版本生成完整字幕。"
                        }
                    ),
                );
            }

            Completed::TtsReady(row) => {
                app.report_status(
                    FeedbackKind::Info,
                    "音频及逐句字幕已保存到生成时选择的练习组，点击试听查看".into(),
                );
                if text(&row, "practice_id") == app.audio.practice_id {
                    app.audio.library.selected = text(&row, "id");
                }
                app.tts_last = Some(row);
            }

            Completed::KeyPoints(audio, points) if !context.cancelled => {
                let _ = app.lib.set_setting(
                    &format!("key_points_{audio}"),
                    &serde_json::to_string(&points).unwrap_or_default(),
                );
                if app.audio.current_id == audio {
                    app.audio.key_points = points;
                }
                app.report_status(FeedbackKind::Info, "学习重点已标注，可悬停查看释义".into());
            }

            _ => app.report_status(FeedbackKind::Info, "已取消".into()),
        }
    }
}

pub(super) fn prepare_leave(app: &mut AppView) {
    app.audio.blind.enabled = false;
    app.invalidate_blind();
}
pub(super) fn leave(app: &mut AppView, next: &str) {
    if app.page == "listening" && next != "listening" {
        app.reset_audio_sentence_request();
        if app.media.session.mode.is_sentence() {
            app.media.session.mode = PlaybackMode::Normal;
        }
    }
}
pub(super) fn enter(app: &mut AppView) {
    if app.page == "listening" {
        app.restore_listening();
    }
}
