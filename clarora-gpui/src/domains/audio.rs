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
    AudioNote,
    AudioNoteSave,
    AudioFont(u8),
    AudioCurrent,
    AudioStar(String),
    AudioStarredReview,
    Highlight,
    ClearHighlights,
    AudioFollow,
    AudioCopySubtitle(String, Option<Cue>),
    AudioRepeat(usize),
    AudioSentence(i8),
    Transcribe,
    TranscribeStronger,
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
            leave_video_sentence: matches!(self, Command::Transcribe | Command::TranscribeStronger),
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
                if app.audio.starred_review && !app.audio.library.starred.contains(&key) {
                    app.audio.starred_review = false;
                }
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
                app.audio.note_open = false;
                app.audio.starred_review = false;
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

            Command::AudioStar(key) => {
                ensure!(
                    app.audio
                        .audios
                        .iter()
                        .any(|r| text(r, "id") == key
                            && text(r, "practice_id") == app.audio.practice_id),
                    "音频不属于当前练习组"
                );
                let starred = app.lib.toggle_audio_star(&key)?;
                if app.audio.starred_review && !starred {
                    app.audio.starred_review = false;
                }
                app.refresh();
                app.report_status(
                    FeedbackKind::Info,
                    if starred {
                        "已加星标，可在“星标”筛选中重点复习".into()
                    } else {
                        "已取消星标".into()
                    },
                );
            }

            Command::AudioStarredReview => {
                if app.audio.starred_review {
                    app.audio.starred_review = false;
                    app.report_status(FeedbackKind::Info, "已退出星标复习".into());
                    return Ok(());
                }
                let first = app
                    .audio
                    .library
                    .merges
                    .learning(&app.audio.audios, &app.audio.practice_id)
                    .into_iter()
                    .find(|r| app.audio.library.starred.contains(&text(r, "id")))
                    .map(|r| text(r, "id"));
                ensure!(
                    first.is_some(),
                    "当前练习组还没有星标音频；先在音频管理里加星标"
                );
                app.audio.starred_review = true;
                app.handle(Action::Audio(first.unwrap()), window, cx);
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
                if app.audio.clip_mode {
                    if let Some((a, b)) = app.clip_range() {
                        let has_range = app
                            .media
                            .session
                            .loop_a
                            .zip(app.media.session.loop_b)
                            .is_some_and(|(a, b)| b > a);
                        if !has_range {
                            let paused = app.media.snapshot.paused;
                            app.media.session.loop_a = Some(a);
                            app.media.session.loop_b = Some(b);
                            app.media.session.mode = PlaybackMode::Ab;
                            let (front, back) = subtitles::excerpt(&app.audio.cues, a, b);
                            app.set("front", front, window, cx);
                            app.set("back", back, window, cx);
                            app.media.player.command([
                                "seek".into(),
                                a.to_string(),
                                "absolute+exact".into(),
                            ]);
                            if paused {
                                app.media.player.set("pause", "yes");
                            }
                            app.report_status(
                                FeedbackKind::Info,
                                "已按当前句设置 A–B，可拖动波形边界调整".into(),
                            );
                        }
                    }
                } else {
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

            Command::AudioNote => {
                ensure!(!app.audio.practice_id.is_empty(), "请先选择练习组");
                if app.audio.note_open {
                    app.audio.note_open = false;
                } else {
                    let note = app
                        .audio
                        .practices
                        .iter()
                        .find(|row| text(row, "id") == app.audio.practice_id)
                        .map(|row| text(row, "note"))
                        .unwrap_or_default();
                    app.set("group_note", note, window, cx);
                    app.audio.note_open = true;
                }
            }

            Command::AudioNoteSave => {
                ensure!(!app.audio.practice_id.is_empty(), "请先选择练习组");
                let note = app.value("group_note", cx);
                app.lib.set_practice_note(&app.audio.practice_id, &note)?;
                app.audio.note_open = false;
                app.refresh();
                app.report_status(FeedbackKind::Info, "组笔记已保存".into());
            }

            Command::AudioCurrent => app.audio.show_current = !app.audio.show_current,

            Command::AudioFollow => {
                app.audio.follow = !app.audio.follow;
                if app.audio.follow
                    && let Some(i) = app.audio.active
                {
                    app.audio.center.request(i);
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

            Command::Transcribe => {
                let mode = clarora_gpui::audio_batch::Mode::English;
                let row = app.selected().context("请先选择音频")?;
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

            Command::TranscribeStronger => {
                let mode = clarora_gpui::audio_batch::Mode::English;
                let row = app.selected().context("请先选择音频")?;
                let (model, _) = app
                    .stronger_local_model()
                    .context("没有已下载的更高精度同语言本机转写模型；请先在设置 → 语音模型下载")?;
                let target = audio_subtitles_controller::Target {
                    audio: Some(text(&row, "id")),
                    name: text(&row, "name"),
                    mode,
                };
                let mut plan = clarora_gpui::audio_batch::Plan::single(row, mode)?;
                plan.model = Some(model.id.into());
                app.begin_audio_subtitles(plan, target)?;
            }

            Command::Translate => {
                if video_subtitles::has_chinese(&app.audio.cues) {
                    let view = cx.entity().downgrade();
                    window.open_dialog(cx, move |dialog, window, _cx| {
                        let view = view.clone();
                        dialog
                            .title("重新翻译中文字幕？")
                            .confirm()
                            .w((window.viewport_size().width - px(32.)).min(px(480.)))
                            .button_props(
                                DialogButtonProps::default()
                                    .cancel_text("取消")
                                    .ok_text("重新翻译"),
                            )
                            .child(
                                div().text_sm().child(
                                    "已有中文字幕会被新译文覆盖，英文字幕和时间轴保持不变。",
                                ),
                            )
                            .on_ok(move |_, _window, cx| {
                                let _ = view.update(cx, |this, cx| {
                                    if let Err(error) = this.begin_audio_translate() {
                                        this.report_status(
                                            FeedbackKind::Error,
                                            format!("重新翻译失败：{error:#}"),
                                        );
                                    }
                                    cx.notify();
                                });
                                true
                            })
                    });
                    return Ok(());
                }
                app.begin_audio_translate()?;
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

impl AppView {
    fn begin_audio_translate(&mut self) -> Result<()> {
        let row = self.selected().context("请先选择音频")?;
        ensure!(!self.audio.cues.is_empty(), "请先导入或生成原文字幕");
        let mode = clarora_gpui::audio_batch::Mode::Chinese;
        let target = audio_subtitles_controller::Target {
            audio: Some(text(&row, "id")),
            name: text(&row, "name"),
            mode,
        };
        let plan = if video_subtitles::has_chinese(&self.audio.cues) {
            clarora_gpui::audio_batch::Plan::single_retranslate(row)?
        } else {
            clarora_gpui::audio_batch::Plan::single(row, mode)?
        };
        self.begin_audio_subtitles(plan, target)
    }
}

// Completion policy belongs to this domain: cancellation may race a durable commit.
pub(super) enum Completed {
    ClipSaved,
    KeyPoints(String, Vec<ai::KeyPoint>),
    TtsReady(Row),
    AudioMerged(clarora_gpui::audio_tools::Merged),
    AudioDirectoryImported(clarora_gpui::data::AudioDirectoryImport),
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
            Completed::AudioDirectoryImported(report) => {
                app.audio.practice_id = report.group;
                let _ = app.lib.set_setting("library_group", &app.audio.practice_id);
                app.audio.library.cancel_rename();
                app.audio.checked.clear();
                app.audio.library.selected.clear();
                app.audio.library.filter = 0;
                app.audio.library.tool.clear();
                app.audio.library.pending_delete.clear();
                app.set("library_search", "", window, cx);
                app.refresh_view_data();
                let mut message = format!(
                    "已从「{}」导入 {} 条音频，文件夹名已作为练习组",
                    report.name, report.imported
                );
                if report.errors.is_empty() {
                    app.report_status(FeedbackKind::Info, message);
                } else {
                    let shown = report
                        .errors
                        .iter()
                        .take(4)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("；");
                    message.push_str(&format!("；{} 条失败：{shown}", report.errors.len()));
                    if report.errors.len() > 4 {
                        message.push('…');
                    }
                    app.report_status(FeedbackKind::Error, message);
                }
            }
            Completed::ClipSaved => {
                let start = app.media.session.loop_a;
                app.audio.clip_mode = false;
                app.media.session.loop_a = None;
                app.media.session.loop_b = None;
                if app.media.session.mode == PlaybackMode::Ab {
                    app.media.session.mode = PlaybackMode::Normal;
                }
                if let Some(start) = start {
                    app.media.player.command([
                        "seek".into(),
                        start.to_string(),
                        "absolute+exact".into(),
                    ]);
                }
                app.report_status(FeedbackKind::Info, "片段已存为闪卡，已退出 A-B 精听".into());
            }
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
        app.audio.starred_review = false;
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
