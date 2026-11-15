//! Video commands own their action effects and execution.
use super::*;
pub(super) enum Command {
    VideoTimelineSeek(f64),
    VideoFullscreen,
    VideoExitFullscreen,
    VideoPrecision,
    VideoSubtitleSize(u8),
    VideoRepeatTail(u16),
    VideoCaptions,
    VideoTranslate,
    VideoClearLoop,
    VideoRepeatCue,
    VideoSentenceRepeat(Option<usize>, Option<u32>),
    VideoStopSentenceRepeat,
    VideoSentence(bool),
    VideoArrow(bool, bool),
    VideoCancelPart,
    VideoPlayCue(usize),
    VideoCopySubtitle(Option<usize>),
    VideoImport,
    VideoImportRetry,
    VideoImportEdit,
    DismissVideoImport,
    ReadParts,
    SelectPart(usize),
    SelectPartAtEnd(usize),
    AutoNextPart,
    VideoTranscribe,
    VideoTranscribeAll,
    VideoTranslateAll,
    CancelVideoSubtitles,
    DismissVideoSubtitles,
    VideoAsk(video_qa::Scope),
    VideoFollow,
    VideoPanel(usize),
    VideoTools,
    VideoTranscriptMode(bool),
    VideoCopyTranscript,
    VideoAskTranscript,
    VideoSelect(String),
    VideoSelectFromStart(String),
    VideoEpisodes(String),
    VideoRemove(String),
    VideoConfirmRemove,
    VideoSubtitleExport,
    VideoGroupFilter(String),
    VideoGroupNew,
    VideoGroupRename,
    VideoGroupDelete,
    VideoSetGroup(String, String),
}
impl Command {
    pub(super) fn effects(&self, app: &AppView) -> Effects {
        Effects {
            restore_focus: (app.page == "video"
                && matches!(
                    self,
                    Command::VideoTimelineSeek(..)
                        | Command::VideoPrecision
                        | Command::VideoSubtitleSize(..)
                        | Command::VideoRepeatTail(..)
                        | Command::VideoCaptions
                        | Command::VideoClearLoop
                        | Command::VideoRepeatCue
                        | Command::VideoSentenceRepeat(..)
                        | Command::VideoStopSentenceRepeat
                        | Command::VideoSentence(..)
                        | Command::VideoPlayCue(..)
                )),
            leave_video_sentence: match self {
                Command::VideoTranscribe => false,
                Command::VideoClearLoop | Command::VideoRemove(..) => true,
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
            Command::VideoTimelineSeek(time) => {
                if app.page == "video" && app.video.sentence_repeat.is_some() {
                    app.seek_video_timeline_repeat(time)?;
                } else {
                    app.handle(Action::SeekTo(time), window, cx);
                }
            }

            Command::VideoFullscreen => {
                ensure!(
                    app.page == "video" && app.media.session.path.is_some(),
                    "请先打开视频"
                );
                if app.video.fullscreen {
                    app.leave_video_fullscreen(window, cx);
                } else {
                    app.video.fullscreen_owned = !window.is_fullscreen();
                    app.video.fullscreen_seen = window.is_fullscreen();
                    app.video.fullscreen = true;
                    window.focus(&app.focus);
                    if app.video.fullscreen_owned {
                        window.toggle_fullscreen();
                    }
                }
            }

            Command::VideoExitFullscreen => app.leave_video_fullscreen(window, cx),

            Command::VideoCaptions => {
                ensure!(app.page == "video", "请先进入视频学习");
                let enabled = !app.video.captions_enabled;
                app.lib.set_setting(
                    "video_captions_enabled",
                    if enabled { "true" } else { "false" },
                )?;
                app.video.captions_enabled = enabled;
                if enabled && app.video.caption_source == VideoCaptionSource::Hidden {
                    app.video.caption_source = if app.video.cues.is_empty() {
                        VideoCaptionSource::Track
                    } else {
                        VideoCaptionSource::Transcript
                    };
                    app.media.player.set("sid", "auto");
                }
                app.media.player.set_subtitles_visible(enabled);
                app.report_status(
                    FeedbackKind::Info,
                    if enabled {
                        "画面字幕已开启"
                    } else {
                        "画面字幕已关闭，字幕列表仍保留"
                    }
                    .into(),
                );
            }

            Command::VideoTranslate => {
                ensure!(app.tasks.video_job.is_none(), "请先完成后台字幕生成再翻译");
                ensure!(app.page == "video", "请先进入视频学习");
                ensure!(!app.tasks.foreground.is_running(), "请等待当前任务完成");
                video_subtitles::english_source(&app.video.cues)?;
                if video_subtitles::has_chinese(&app.video.cues) {
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
                                    "当前中文字幕会被新译文覆盖，英文字幕和时间轴保持不变。",
                                ),
                            )
                            .on_ok(move |_, _window, cx| {
                                let _ = view.update(cx, |this, cx| {
                                    if let Err(error) = this.begin_video_translate() {
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
                app.begin_video_translate()?;
            }

            Command::VideoSubtitleSize(size) => {
                ensure!([14, 16, 20, 24, 28, 32].contains(&size), "无效字幕字号");
                app.lib
                    .set_setting("video_subtitle_size", &size.to_string())?;
                app.video.subtitle_size = size;
            }

            Command::VideoRepeatTail(ms) => {
                ensure!(
                    [200, 350, 500, 650].contains(&ms),
                    "逐句尾音需要 0.2–0.65 秒"
                );
                app.lib
                    .set_setting("video_repeat_tail_ms", &ms.to_string())?;
                app.video.repeat_tail_ms = ms;
                if app.media.session.video_sentence().is_some() {
                    app.report_status(
                        FeedbackKind::Info,
                        "逐句尾音已保存；重新开启逐句循环后生效".into(),
                    );
                }
            }

            Command::VideoPrecision => {
                app.video.precision = !app.video.precision;
                if app.video.precision && app.media.session.is_video() {
                    let has_range = app
                        .media
                        .session
                        .loop_a
                        .zip(app.media.session.loop_b)
                        .is_some_and(|(a, b)| b > a);
                    if !has_range {
                        let position = app.media.snapshot.position;
                        let duration = app.media.snapshot.duration;
                        let index =
                            video_subtitles::active(&app.video.cues, position).or_else(|| {
                                clarora_gpui::video_repeat::at_or_next(
                                    &app.video.cues,
                                    position,
                                    duration,
                                )
                            });
                        if let Some(cue) = index.and_then(|i| app.video.cues.get(i)).cloned() {
                            let end = cue.end.min(duration);
                            if duration > 0. && end > cue.start {
                                let paused = app.media.snapshot.paused;
                                app.media.session.loop_a = Some(cue.start);
                                app.media.session.loop_b = Some(end);
                                app.media.session.mode = PlaybackMode::Ab;
                                app.media.player.command([
                                    "seek".into(),
                                    cue.start.to_string(),
                                    "absolute+exact".into(),
                                ]);
                                if paused {
                                    app.media.player.set("pause", "yes");
                                }
                                app.report_status(
                                    FeedbackKind::Info,
                                    "已按当前句设置 A–B，可拖动进度条边界调整".into(),
                                );
                            }
                        }
                    }
                }
            }

            Command::VideoClearLoop => {
                app.media.session.loop_a = None;
                app.media.session.loop_b = None;
                if app.media.session.mode == PlaybackMode::Ab {
                    app.media.session.mode = PlaybackMode::Normal;
                }
            }

            Command::VideoRepeatCue => {
                app.start_video_sentence_repeat(None, None)?;
            }

            Command::VideoSentenceRepeat(index, total) => {
                app.start_video_sentence_repeat(index, total)?;
            }

            Command::VideoStopSentenceRepeat => app.clear_video_sentence_repeat(),

            Command::VideoArrow(next, held) => app.video_arrow(next, held, window, cx),

            Command::VideoCancelPart => {}

            Command::VideoPlayCue(index) => {
                ensure!(
                    app.page == "video" && app.media.session.path.is_some(),
                    "请先打开视频"
                );
                let cue = app.video.cues.get(index).context("字幕行不存在")?;
                let start = cue.start;
                ensure!(start.is_finite() && start >= 0., "字幕时间无效");
                if app.video.sentence_repeat.is_some()
                    || app.media.session.video_sentence().is_some()
                {
                    // A subtitle click chooses a new sentence in the current
                    // repeat mode, restarting its play count from the beginning.
                    app.start_video_sentence_repeat(Some(index), app.video.sentence_repeat)?;
                    return Ok(());
                }
                app.reset_video_sentence_request();
                // A previous sentence loop must not pull playback back to its B point.
                if app.media.session.mode == PlaybackMode::Ab {
                    app.media.session.mode = PlaybackMode::Normal;
                }
                app.media.player.command([
                    "seek".into(),
                    start.to_string(),
                    "absolute+exact".into(),
                ]);
                app.media.player.set("pause", "no");
            }

            Command::VideoSentence(next) => {
                if app.video.pending_start.is_some() {
                    app.poll_pending_video_start();
                    if app.video.pending_start.is_some() {
                        app.report_status(
                            FeedbackKind::Info,
                            "逐句重复模式正在等待当前视频和字幕就绪".into(),
                        );
                        return Ok(());
                    }
                }
                if let Some(mut session) = app
                    .media
                    .session
                    .video_sentence()
                    .filter(|s| s.total.is_some())
                {
                    let path = app.video.path.as_ref().context("请先打开视频")?;
                    ensure!(
                        app.media.session.path.as_ref() == Some(path),
                        "视频正在切换"
                    );
                    if session.step(
                        &app.media.player,
                        path,
                        &app.video.cues,
                        next,
                        app.media.snapshot.duration,
                    )? {
                        app.media.session.set_video_sentence(Some(session));
                        app.follow_video_repeat();
                    } else {
                        app.report_status(
                            FeedbackKind::Info,
                            if next {
                                "已经是最后一句"
                            } else {
                                "已经是第一句"
                            }
                            .into(),
                        );
                    }
                    return Ok(());
                }
                let path = app.video.path.as_ref().context("请先打开视频")?;
                ensure!(
                    app.page == "video"
                        && app.media.session.path.as_ref() == Some(path)
                        && app.media.snapshot.path == path.to_string_lossy(),
                    "视频正在切换，请稍候再切句"
                );
                let Some(index) = clarora_gpui::video_repeat::adjacent(
                    &app.video.cues,
                    app.media.snapshot.position,
                    next,
                    app.media.snapshot.duration,
                ) else {
                    app.report_status(
                        FeedbackKind::Info,
                        if app.video.cues.is_empty() {
                            "暂无字幕，请先生成或导入字幕"
                        } else if next {
                            "已经是最后一句"
                        } else {
                            "已经是第一句"
                        }
                        .into(),
                    );
                    return Ok(());
                };
                let target = app.video.cues[index].start;
                app.clear_video_sentence_repeat();
                // Moving to another sentence exits the previous A/B loop.
                if app.media.session.mode == PlaybackMode::Ab {
                    app.media.session.mode = PlaybackMode::Normal;
                }
                app.media.player.command([
                    "seek".into(),
                    target.to_string(),
                    "absolute+exact".into(),
                ]);
            }

            Command::VideoImport => app.open_video_import_dialog(window, cx)?,

            Command::VideoImportRetry => {
                let draft = app
                    .video
                    .import_recovery
                    .clone()
                    .context("导入参数已失效，请重新打开导入弹框")?;
                if app.page != "video" {
                    app.handle(Action::Page("video".into()), window, cx);
                }
                ensure!(app.page == "video", "请先进入视频学习");
                app.clear_feedback_error();
                app.begin_video_link_import(&draft.raw, false, draft.group, draft.part)?;
                app.video.import_recovery = None;
            }

            Command::VideoImportEdit => {
                let draft = app
                    .video
                    .import_recovery
                    .clone()
                    .context("导入参数已失效，请重新打开导入弹框")?;
                if app.page != "video" {
                    app.handle(Action::Page("video".into()), window, cx);
                }
                ensure!(app.page == "video", "请先进入视频学习");
                app.clear_feedback_error();
                app.video.import_edit = Some(draft);
                app.open_video_import_dialog(window, cx)?;
            }

            Command::DismissVideoImport => {
                app.video.import_recovery = None;
            }

            Command::ReadParts => {
                if let Some(link) = app.video.series.as_ref().map(|series| series.base.clone()) {
                    app.begin_video_link_import(&link, true, app.video.group.clone(), None)?;
                } else {
                    app.open_video_import_dialog(window, cx)?;
                }
            }

            Command::SelectPart(index) => app.start_part(index),

            Command::SelectPartAtEnd(index) => app.start_part_from(index, true),

            Command::AutoNextPart => {
                let enabled = !app.video.auto_next_part;
                app.lib
                    .set_setting("video_auto_next", if enabled { "true" } else { "false" })?;
                app.video.auto_next_part = enabled;
                app.media.session.end_handled = false;
            }

            Command::VideoAsk(scope) => {
                ensure!(app.page == "video", "请先进入视频学习");
                ensure!(!app.chat.running, "请等待当前回答完成或取消后切换引用");
                app.prepare_video_material(scope)?;
                if scope != video_qa::Scope::All {
                    app.media.player.set("pause", "yes");
                }
                let material = app.video.material.as_ref().context("请先选取字幕")?;
                let text = clarora_gpui::assistant::plain_reference(&material.text);
                if scope == video_qa::Scope::All {
                    app.add_material_background(text, window, cx)?;
                } else {
                    app.add_subtitle_question(text, window, cx)?;
                }
            }

            Command::VideoGroupFilter(group) => {
                ensure!(
                    group.is_empty()
                        || group == video_library::UNGROUPED
                        || app.video.collection.has_group(&group),
                    "视频分组不存在"
                );
                app.video.group = group;
                app.lib
                    .set_setting("video_group_filter", &app.video.group)?;
            }

            Command::VideoGroupNew => {
                ensure!(!app.tasks.foreground.is_running(), "请等待当前任务完成");
                app.open_video_group_dialog(None, window, cx);
            }

            Command::VideoGroupRename => {
                ensure!(!app.tasks.foreground.is_running(), "请等待当前任务完成");
                let group = app.video.group.clone();
                ensure!(
                    !group.is_empty() && group != video_library::UNGROUPED,
                    "请先选择一个视频分组"
                );
                app.open_video_group_dialog(Some(group), window, cx);
            }

            Command::VideoGroupDelete => {
                ensure!(!app.tasks.foreground.is_running(), "请等待当前任务完成");
                let group = app.video.group.clone();
                ensure!(
                    !group.is_empty() && group != video_library::UNGROUPED,
                    "请先选择一个视频分组"
                );
                app.confirm_video_group_delete(group, window, cx);
            }

            Command::VideoSetGroup(key, group) => {
                ensure!(!app.tasks.foreground.is_running(), "请等待当前任务完成");
                let label = if group.is_empty() || group == video_library::UNGROUPED {
                    "未分组".into()
                } else {
                    group.clone()
                };
                let mut videos = video_library::Collection::load(&app.lib)?;
                videos.set_group(&key, &group)?;
                videos.save(&app.lib)?;
                app.video.collection = videos;
                app.report_status(FeedbackKind::Info, format!("已移动到“{label}”"));
            }

            Command::VideoTranscribe => app.begin_background_subtitles(false)?,

            Command::VideoTranscribeAll => app.begin_background_subtitles(true)?,

            Command::VideoTranslateAll => app.begin_background_translation_all()?,

            Command::CancelVideoSubtitles => {
                if let Some(job) = &app.tasks.video_job {
                    job.cancel.store(true, Ordering::SeqCst);
                }
                if let Some(n) = app.tasks.video_notice.as_mut().filter(|n| n.active()) {
                    n.state = TaskState::Cancelling;
                    n.detail = "正在停止后台字幕队列，已完成的字幕会保留".into();
                }
            }

            Command::DismissVideoSubtitles => {
                if app.tasks.video_job.is_none() {
                    app.tasks.video_notice = None;
                }
            }

            Command::VideoTools => app.video.tools_open = !app.video.tools_open,

            Command::VideoSelect(key) => {
                ensure!(
                    !app.tasks.foreground.is_running(),
                    "请完成或取消当前任务后再切换视频"
                );
                if app.select_source_only(&key, true, false)? {
                    return Ok(());
                }
                let mut videos = video_library::Collection::load(&app.lib)?;
                let video = videos.select(&key)?;
                videos.save(&app.lib)?;
                app.video.collection = videos;
                app.save_media_position();
                app.save_video_progress();
                let path = video.resume_path().context("视频文件不可用")?;
                if !app.open_continue(path, true) {
                    return Ok(());
                }
                app.video.tools_open = false;
                app.report_status(FeedbackKind::Info, format!("已选择：{}", video.title));
            }

            Command::VideoSelectFromStart(key) => {
                ensure!(
                    !app.tasks.foreground.is_running(),
                    "请完成或取消当前任务后再切换视频"
                );
                if app.select_source_only(&key, true, true)? {
                    return Ok(());
                }
                let mut videos = video_library::Collection::load(&app.lib)?;
                let video = videos.select(&key)?;
                videos.save(&app.lib)?;
                app.video.collection = videos;
                app.save_media_position();
                app.save_video_progress();
                let path = video.resume_path().context("视频文件不可用")?;
                if !app.open_from_start(path, true) {
                    return Ok(());
                }
                app.video.tools_open = false;
                app.report_status(FeedbackKind::Info, format!("从头播放：{}", video.title));
            }

            Command::VideoEpisodes(key) => {
                ensure!(
                    !app.tasks.foreground.is_running(),
                    "请完成或取消当前任务后再切换视频"
                );
                if app.select_source_only(&key, false, false)? {
                    return Ok(());
                }
                if app.video.collection.selected.as_ref() != Some(&key) || app.video.path.is_none()
                {
                    let mut videos = video_library::Collection::load(&app.lib)?;
                    let video = videos.select(&key)?;
                    videos.save(&app.lib)?;
                    app.save_media_position();
                    app.save_video_progress();
                    app.video.collection = videos;
                    let path = video.resume_path().context("视频文件不可用")?;
                    if !app.open_continue(path, false) {
                        return Ok(());
                    }
                    app.media.player.command(["set", "pause", "yes"]);
                }
                app.video.tab = 1;
                app.video.tools_open = false;
            }

            Command::VideoRemove(key) => {
                ensure!(!app.tasks.foreground.is_running(), "请等待当前任务完成");
                let title = app
                    .video
                    .collection
                    .items
                    .iter()
                    .find(|v| v.id == key)
                    .context("该视频已移出收藏")?
                    .title
                    .clone();
                app.video.remove = Some(key);
                let view = cx.entity().downgrade();
                window.open_dialog(cx, move |dialog, window, cx| {
                    let confirm_view = view.clone();
                    let close_view = view.clone();
                    dialog
                        .title("移除视频收藏？")
                        .confirm()
                        .w((window.viewport_size().width - px(32.)).min(px(480.)))
                        .margin_top(((window.viewport_size().height - px(280.)) / 2.).max(px(16.)))
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
                                    this.video.remove.is_none()
                                })
                                .unwrap_or(true)
                        })
                        .on_close(move |_, _, cx| {
                            let _ = close_view.update(cx, |this, cx| {
                                this.video.remove = None;
                                cx.notify();
                            });
                        })
                });
            }

            Command::VideoConfirmRemove => {
                ensure!(!app.tasks.foreground.is_running(), "请等待当前任务完成");
                let key = app.video.remove.clone().context("请重新选择要移除的视频")?;
                let mut videos = video_library::Collection::load(&app.lib)?;
                if let Some(job) = app.tasks.video_job.as_ref().filter(|j| j.source_id == key) {
                    job.cancel.store(true, Ordering::SeqCst);
                }
                let video = videos.remove(&key)?;
                videos.save(&app.lib)?;
                app.video.collection = videos;
                app.video.remove = None;
                if app
                    .video
                    .path
                    .as_ref()
                    .is_some_and(|path| video.contains(path))
                {
                    app.leave_video_fullscreen(window, cx);
                    app.media.player.command(["stop"]);
                    app.media.snapshot = Playback::default();
                    app.media.release();
                    app.video.path = None;
                    app.video.series = None;
                    app.video.set_cues(vec![]);
                    app.refresh_saved_ranges();
                    app.video.active = None;
                    app.video.material = None;
                    app.video.material_error.clear();
                    app.media.session.loop_a = None;
                    app.media.session.loop_b = None;
                    app.media.session.mode = PlaybackMode::Normal;
                    app.media.session.end_handled = true;
                    app.output.clear();
                    app.chat.reasoning.clear();
                    app.lib.set_setting("bilibili_series", "")?;
                    app.lib.set_setting("last_video_position", "")?;
                }
                app.video.tab = 3;
                app.report_status(
                    FeedbackKind::Info,
                    format!("已移除「{}」收藏，视频和字幕文件保留", video.title),
                );
            }

            Command::VideoPanel(tab) => {
                app.video.remove = None;
                ensure!(tab < 4, "无效学习面板");
                app.video.tab = tab;
                if tab == 2 {
                    app.video.tab = 0;
                    app.open_assistant(window, cx);
                    app.inputs["question"].update(cx, |state, cx| state.focus(window, cx));
                } else {
                    window.focus(&app.focus);
                }
            }

            Command::VideoFollow => {
                app.video.follow = !app.video.follow;
                if app.video.follow
                    && let Some(i) = app.video.active
                {
                    app.video.center.request(i);
                }
            }

            Command::VideoTranscriptMode(show) => {
                ensure!(app.page == "video", "请先进入视频学习");
                if app.video.transcript_open != show {
                    app.subtitle_text.borrow_mut().clear();
                }
                app.video.transcript_open = show;
                app.video.tab = 0;
                app.focus.focus(window);
            }

            Command::VideoCopyTranscript => {
                ensure!(app.page == "video", "请先进入视频学习");
                ensure!(
                    !app.video.transcript.text.is_empty(),
                    "没有可复制的英文文稿"
                );
                cx.write_to_clipboard(ClipboardItem::new_string(app.video.transcript.text.clone()));
                app.report_status(
                    FeedbackKind::Info,
                    format!(
                        "已复制英文全文 · {} 句，约 {} 词",
                        app.video.transcript.cue_count, app.video.transcript.word_count
                    ),
                );
                app.focus.focus(window);
            }

            Command::VideoAskTranscript => {
                ensure!(app.page == "video", "请先进入视频学习");
                ensure!(
                    !app.video.transcript.text.is_empty(),
                    "暂无英文全文，请先生成或导入英文字幕"
                );
                // Use the exact cached English copy text. VideoAsk(All)
                // includes bilingual cues, timestamps and legacy wrappers.
                app.add_material_background(app.video.transcript.text.clone(), window, cx)?;
                app.report_status(
                    FeedbackKind::Info,
                    "已添加英文全文引用；请在助手中检查引用并输入问题后发送".into(),
                );
            }

            Command::VideoCopySubtitle(index) => {
                let body = match index {
                    Some(index) => app
                        .video
                        .cues
                        .get(index)
                        .context("字幕已改变，请重试")?
                        .text
                        .clone(),
                    None => app
                        .video
                        .cues
                        .iter()
                        .map(|cue| cue.text.as_str())
                        .collect::<Vec<_>>()
                        .join("\n"),
                };
                ensure!(!body.trim().is_empty(), "没有可复制的字幕");
                cx.write_to_clipboard(ClipboardItem::new_string(body));
                app.report_status(
                    FeedbackKind::Info,
                    match index {
                        Some(index) => format!("已复制第 {} 句字幕", index + 1),
                        None => format!("已复制全部 {} 句字幕", app.video.cues.len()),
                    },
                );
            }

            Command::VideoSubtitleExport => {
                ensure!(!app.video.cues.is_empty(), "请先生成字幕");
                let body = subtitles::srt(&app.video.cues);
                let picker = cx.prompt_for_new_path(&app.lib.root, Some("video-subtitles.srt"));
                cx.spawn(async move |this, cx| {
                    if let Ok(Ok(Some(path))) = picker.await {
                        let result = fs::write(path, body);
                        let _ = this.update(cx, |this, cx| {
                            this.report_status(
                                if result.is_err() {
                                    FeedbackKind::Error
                                } else {
                                    FeedbackKind::Info
                                },
                                match result {
                                    Ok(()) => "SRT 字幕已导出".into(),
                                    Err(e) => format!("导出失败：{e}"),
                                },
                            );
                            cx.notify();
                        });
                    }
                })
                .detach();
            }
        }
        Ok(())
    }
}

impl AppView {
    /// Follow group renames recorded while an import task was in flight and
    /// fall back to the ungrouped bucket if the target was deleted.
    pub(crate) fn resolve_import_group(&self, group: &str) -> String {
        if self.video.import_active
            && let Some(draft) = &self.video.import_draft
        {
            return if draft.group.is_empty() {
                String::new()
            } else {
                draft.group.clone()
            };
        }
        let mut current = group.to_owned();
        let mut seen = std::collections::BTreeSet::new();
        while seen.insert(current.clone()) {
            let Some(next) = self.video.group_aliases.get(&current) else {
                break;
            };
            current.clone_from(next);
        }
        if !current.is_empty() && self.video.collection.has_group(&current) {
            current
        } else {
            String::new()
        }
    }

    pub(crate) fn begin_video_link_import(
        &mut self,
        link: &str,
        parts_only: bool,
        group: String,
        selected_part: Option<usize>,
    ) -> Result<()> {
        ensure!(self.page == "video", "请先进入视频学习");
        ensure!(
            !self.tasks.foreground.is_running(),
            "请完成或取消当前任务后再导入视频"
        );
        ensure!(
            !link.trim().is_empty(),
            "请先粘贴视频链接或输入 BV / av 编号"
        );
        let parsed = process::normalize_video_link(link)?;
        if selected_part.is_some() && !matches!(parsed, process::VideoLink::Bilibili(_)) {
            anyhow::bail!("该来源没有可选分P，请直接下载");
        }
        let normalized = match &parsed {
            process::VideoLink::External(_, url) | process::VideoLink::Bilibili(url) => {
                Some(url.clone())
            }
        };
        self.video.import_draft = Some(app_state::VideoImportDraft {
            raw: link.to_owned(),
            normalized,
            part: selected_part,
            group: group.clone(),
        });
        self.video.import_recovery = None;
        self.video.import_active = true;
        match parsed {
            process::VideoLink::External(source, url) => {
                ensure!(!parts_only, "{} 视频请点击「下载并导入」", source.label());
                self.begin_external_download(
                    source,
                    url,
                    if source == process::VideoSource::YouTube {
                        self.video.youtube_browser
                    } else {
                        None
                    },
                    self.media.session.revision(),
                    group,
                );
            }
            process::VideoLink::Bilibili(url) => {
                let origin = self.media.session.revision();
                let previous = self.video.series.clone();
                let downloads = self.tasks.downloads.clone();
                let (scope, label) = if parts_only {
                    (TaskScope::Material, "正在读取 bilibili 分P列表…")
                } else {
                    (TaskScope::Application, "正在识别分P并导入所选集…")
                };
                self.begin_scoped(scope, label, move |lib, cancel, _| {
                    let mut series = bilibili::fetch(&url, &cancel)?;
                    if let Some(previous) = previous.filter(|p| p.base == series.base) {
                        for part in &mut series.parts {
                            part.path = previous
                                .parts
                                .iter()
                                .find(|p| p.number == part.number)
                                .and_then(|p| p.path.clone());
                        }
                        if parts_only {
                            series.current = previous.current.filter(|i| *i < series.parts.len());
                        }
                    }
                    if parts_only {
                        return Ok(Outcome::Parts(series));
                    }
                    if let Some(index) = selected_part {
                        ensure!(index < series.parts.len(), "所选分P不存在，请重新读取选集");
                        series.requested = index;
                    }
                    let index = series.requested;
                    let path = downloads.obtain(&lib, &series, index, &cancel)?;
                    Ok(Outcome::PartVideo {
                        series,
                        index,
                        path,
                        origin,
                        from_end: false,
                        resume: false,
                        group,
                    })
                });
            }
        }
        if !self.tasks.foreground.is_running() {
            self.video.import_active = false;
            anyhow::bail!("{}", self.status);
        }
        Ok(())
    }

    fn begin_external_download(
        &mut self,
        source: process::VideoSource,
        url: String,
        browser: Option<process::YoutubeBrowser>,
        origin: u64,
        group: String,
    ) {
        self.begin_scoped(
            TaskScope::Application,
            &format!("正在下载 {} 视频…", source.label()),
            move |lib, cancel, _| match process::download_external_with(
                source, &lib.root, &url, browser, &cancel,
            ) {
                Ok(path) => Ok(Outcome::VideoDownloaded(path, origin, group, source)),
                Err(error)
                    if source == process::VideoSource::YouTube
                        && process::is_video_auth_error(&error) =>
                {
                    Ok(Outcome::VideoAuthRequired(url, origin, group, source))
                }
                Err(error) => Err(error),
            },
        );
    }
    fn open_video_group_dialog(
        &mut self,
        group: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let title = if group.is_some() {
            "重命名视频分组"
        } else {
            "新建视频分组"
        };
        let current = group.clone().unwrap_or_default();
        self.set("video_group_name", current, window, cx);
        let input = self.inputs["video_group_name"].clone();
        let view = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, window, _cx| {
            let view = view.clone();
            let group = group.clone();
            let input = input.clone();
            let ok_input = input.clone();
            let ok_view = view.clone();
            let ok_group = group.clone();
            dialog
                .title(title)
                .confirm()
                .w((window.viewport_size().width - px(32.)).min(px(440.)))
                .button_props(
                    DialogButtonProps::default()
                        .cancel_text("取消")
                        .ok_text("保存"),
                )
                .child(div().w_full().child(Input::new(&input)))
                .on_ok(move |_, _, cx| {
                    let name = ok_input.read(cx).value().to_string();
                    ok_view
                        .update(cx, |this, cx| {
                            let result = if let Some(group) = &ok_group {
                                this.rename_video_group(group, &name)
                            } else {
                                this.create_video_group(&name)
                            };
                            match result {
                                Ok(()) => {
                                    cx.notify();
                                    true
                                }
                                Err(error) => {
                                    this.report_status(
                                        FeedbackKind::Error,
                                        format!("保存视频分组失败：{error:#}"),
                                    );
                                    cx.notify();
                                    false
                                }
                            }
                        })
                        .unwrap_or(true)
                })
        });
        self.inputs["video_group_name"].update(cx, |input, cx| input.focus(window, cx));
    }
    fn create_video_group(&mut self, name: &str) -> Result<()> {
        let mut collection = video_library::Collection::load(&self.lib)?;
        let group = collection.create_group(name)?;
        collection.save(&self.lib)?;
        self.video.collection = collection;
        self.video.group = group.clone();
        self.video.group_aliases.remove(&group);
        self.lib.set_setting("video_group_filter", &group)?;
        self.report_status(FeedbackKind::Info, format!("已创建视频分组“{group}”"));
        Ok(())
    }
    pub(crate) fn rename_video_group(&mut self, old: &str, name: &str) -> Result<()> {
        let mut collection = video_library::Collection::load(&self.lib)?;
        let new = collection.rename_group(old, name)?;
        collection.save(&self.lib)?;
        self.video.collection = collection;
        if old != new {
            self.video.group_aliases.insert(old.to_owned(), new.clone());
            for draft in [
                self.video.import_draft.as_mut(),
                self.video.import_recovery.as_mut(),
            ]
            .into_iter()
            .flatten()
            {
                if draft.group == old {
                    draft.group = new.clone();
                }
            }
        }
        if self.video.group == old {
            self.video.group = new.clone();
            self.lib.set_setting("video_group_filter", &new)?;
        }
        self.report_status(FeedbackKind::Info, format!("已重命名为“{new}”"));
        Ok(())
    }
    pub(crate) fn delete_video_group(&mut self, group: &str) -> Result<()> {
        let mut collection = video_library::Collection::load(&self.lib)?;
        let moved = collection.remove_group(group)?;
        collection.save(&self.lib)?;
        self.video.collection = collection;
        self.video
            .group_aliases
            .insert(group.to_owned(), String::new());
        for draft in [
            self.video.import_draft.as_mut(),
            self.video.import_recovery.as_mut(),
        ]
        .into_iter()
        .flatten()
        {
            if draft.group == group {
                draft.group.clear();
            }
        }
        if self.video.group == group {
            self.video.group.clear();
            self.lib.set_setting("video_group_filter", "")?;
        }
        self.report_status(
            FeedbackKind::Info,
            format!("已删除视频分组；{moved} 个视频变为未分组"),
        );
        Ok(())
    }
    fn confirm_video_group_delete(
        &mut self,
        group: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let count = self.video.collection.group_count(&group);
        let view = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, window, cx| {
            let view = view.clone();
            let group = group.clone();
            dialog
                .title("删除这个视频分组？")
                .confirm()
                .w((window.viewport_size().width - px(32.)).min(px(440.)))
                .button_props(
                    DialogButtonProps::default()
                        .cancel_text("取消")
                        .ok_text("删除分组")
                        .ok_variant(ButtonVariant::Danger),
                )
                .child(div().text_sm().child(format!("分组：{group}")))
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("组内 {count} 个视频不会被删除，只会变为未分组。")),
                )
                .on_ok(move |_, _, cx| {
                    view.update(cx, |this, cx| match this.delete_video_group(&group) {
                        Ok(()) => {
                            cx.notify();
                            true
                        }
                        Err(error) => {
                            this.report_status(
                                FeedbackKind::Error,
                                format!("删除视频分组失败：{error:#}"),
                            );
                            cx.notify();
                            false
                        }
                    })
                    .unwrap_or(true)
                })
        });
    }
    fn show_video_auth(
        &mut self,
        url: String,
        origin: u64,
        group: String,
        source: process::VideoSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, window, cx| {
            let mut browsers = div().flex().flex_wrap().gap_2();
            for browser in process::YOUTUBE_BROWSERS {
                let weak = view.clone();
                let retry_url = url.clone();
                let retry_group = group.clone();
                let browser = *browser;
                browsers = browsers.child(
                    Button::new(SharedString::from(format!(
                        "video-auth-{}",
                        browser.id()
                    )))
                    .label(browser.label())
                    .outline()
                    .on_click(move |_, window, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            let _ = this.lib.set_setting("youtube_cookie_browser", browser.id());
                            this.video.youtube_browser = Some(browser);
                            let group = this.resolve_import_group(&retry_group);
                            this.begin_external_download(
                                source,
                                retry_url.clone(),
                                Some(browser),
                                origin,
                                group,
                            );
                            cx.notify();
                        });
                        window.close_dialog(cx);
                    }),
                );
            }
            dialog
                .title(format!("{} 需要浏览器验证", source.label()))
                .alert()
                .w((window.viewport_size().width - px(32.)).min(px(520.)))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("关闭")
                        .ok_variant(ButtonVariant::Secondary),
                )
                .child(
                    div()
                        .text_sm()
                        .child(format!(
                            "{} 拒绝了匿名下载。请先在浏览器中打开 {} 并登录或刷新页面，再选择该浏览器重试。",
                            source.label(),
                            source.label()
                        )),
                )
                .child(div().text_sm().child("选择已登录对应网站的浏览器："))
                .child(browsers)
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child(
                    "Cookie 由本机 yt-dlp 使用。应用会记住浏览器，你可在「导入视频」弹窗中清除。",
                ))
        });
    }
}

// Completion policy belongs to this domain: cancellation may race a durable commit.
pub(super) enum Completed {
    Parts(bilibili::Series),
    PartVideo {
        series: bilibili::Series,
        index: usize,
        path: PathBuf,
        origin: u64,
        from_end: bool,
        resume: bool,
        group: String,
    },
    VideoTranslated(PathBuf, String, Vec<Cue>, Vec<Cue>),
    VideoDownloaded(PathBuf, u64, String, process::VideoSource),
    VideoAuthRequired(String, u64, String, process::VideoSource),
    ClipSaved,
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
            Completed::Parts(series) if !context.cancelled => {
                app.report_status(
                    FeedbackKind::Info,
                    format!("已识别 {} P，点击列表选集播放", series.parts.len()),
                );
                if let Err(e) = series.save(&app.lib) {
                    app.report_status(FeedbackKind::Error, format!("保存分P列表失败：{e}"));
                }
                app.video.parts_scroll.scroll_to_item(
                    series.current.unwrap_or(series.requested),
                    ScrollStrategy::Center,
                );
                app.video.series = Some(series);
                if app.page == "video" {
                    app.video.tab = 1;
                }
            }

            Completed::VideoAuthRequired(url, origin, group, source) => {
                if context.cancelled {
                    app.report_status(
                        FeedbackKind::Info,
                        format!("已取消 {} 视频导入", source.label()),
                    );
                } else {
                    app.report_status(
                        FeedbackKind::Error,
                        format!("{} 需要浏览器验证，请选择 Cookie 重试", source.label()),
                    );
                    let group = app.resolve_import_group(&group);
                    app.show_video_auth(url, origin, group, source, window, cx);
                }
            }

            Completed::VideoDownloaded(path, origin, group, source) => {
                if context.cancelled {
                    // A finished download stays in the reusable cache; a real cancel
                    // during yt-dlp is cleaned up before it returns.
                    app.report_status(
                        FeedbackKind::Info,
                        format!("已取消 {} 视频导入", source.label()),
                    );
                } else {
                    let group = app.resolve_import_group(&group);
                    let activate = app.page == "video" && app.media.session.revision() == origin;
                    let result = if activate {
                        app.save_media_position();
                        app.save_video_progress();
                        app.open_downloaded_video(path, &group)
                            .then_some(())
                            .context(app.status.clone())
                    } else {
                        (|| -> Result<()> {
                            let mut videos = video_library::Collection::load(&app.lib)?;
                            videos.add_in_group(std::slice::from_ref(&path), None, &group)?;
                            videos.save(&app.lib)?;
                            app.video.collection = videos;
                            Ok(())
                        })()
                    };
                    match result {
                        Ok(()) if activate => {
                            app.video.tab = 3;
                            app.video.tools_open = false;
                            app.report_status(
                                FeedbackKind::Info,
                                format!("已导入 {} 视频，可在“我的视频”切换", source.label()),
                            );
                        }
                        Ok(()) => app.report_status(
                            FeedbackKind::Info,
                            format!(
                                "{} 视频已导入，返回视频页可在“我的视频”播放",
                                source.label()
                            ),
                        ),
                        Err(error) => app.report_status(
                            FeedbackKind::Error,
                            format!("{} 视频导入失败：{error:#}", source.label()),
                        ),
                    }
                }
            }

            Completed::PartVideo {
                mut series,
                index,
                path,
                origin,
                from_end,
                resume,
                group,
            } => {
                if context.cancelled {
                    app.report_status(FeedbackKind::Info, "已取消分P下载".into());
                } else {
                    let group = app.resolve_import_group(&group);
                    series.parts[index].path = Some(path.clone());
                    let position = if resume {
                        app.part_resume_position(&series, index).unwrap_or(0.)
                    } else {
                        0.
                    };
                    let reading = if resume {
                        app.part_reading(&series, index)
                    } else {
                        None
                    };
                    // A downloaded part may complete after leaving and returning
                    // to the same file, or after selecting another empty source.
                    // Only the initiating playback generation may activate it;
                    // the shared cache and collection still keep completed files.
                    let activate = app.page == "video" && app.media.session.revision() == origin;
                    if activate {
                        series.current = Some(index);
                    }
                    let saved = (|| -> Result<()> {
                        let mut videos = video_library::Collection::load(&app.lib)?;
                        if activate || !videos.cache_series_part(&series, index, &path)? {
                            videos.add_in_group(
                                std::slice::from_ref(&path),
                                Some(&series),
                                &group,
                            )?;
                        }
                        videos.save(&app.lib)?;
                        if !activate
                            && let Some(active) =
                                app.video.series.as_mut().filter(|s| s.base == series.base)
                            && let Some(saved) = videos
                                .items
                                .iter()
                                .filter_map(|video| video.series.as_ref())
                                .find(|s| s.base == active.base)
                        {
                            active.parts.clone_from(&saved.parts);
                        }
                        app.video.collection = videos;
                        Ok(())
                    })();
                    if activate && saved.is_ok() {
                        app.save_video_progress();
                        app.video.series = Some(series);
                        // Opening media reloads subtitles; keep the panel the user is viewing.
                        let panel = app.video.tab;
                        let restore_tab = reading.is_some();
                        if app.open_part_video_at(path, position, from_end, reading, true) {
                            if !restore_tab {
                                app.video.tab = panel;
                            }
                            if app.video.sentence_repeat.is_none() && !from_end {
                                app.media.player.set("pause", "no");
                            }
                            app.report_status(
                                FeedbackKind::Info,
                                if from_end {
                                    format!("正在定位 P{} 的最后一句", index + 1)
                                } else if let Some(total) = app.video.sentence_repeat {
                                    format!(
                                        "正在准备 P{} 的{}播放",
                                        index + 1,
                                        clarora_gpui::video_repeat::label(total)
                                    )
                                } else {
                                    format!("正在播放 P{}", index + 1)
                                },
                            );
                        }
                    } else {
                        app.report_status(
                            FeedbackKind::Info,
                            "分P已下载，返回视频页选集播放".into(),
                        );
                    }
                    if let Err(e) = saved {
                        app.report_status(
                            FeedbackKind::Error,
                            format!("分P已导入，但保存列表失败：{e}"),
                        );
                    }
                }
            }

            Completed::ClipSaved => {
                let start = app.media.session.loop_a;
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
                app.report_status(FeedbackKind::Info, "片段已存为闪卡，字幕已加下划线".into());
            }

            Completed::VideoTranslated(video, path, source, cues) if !context.cancelled => {
                let same_source = app.video.cues.len() == source.len()
                    && app
                        .video
                        .cues
                        .iter()
                        .zip(&source)
                        .all(|(a, b)| a.start == b.start && a.end == b.end && a.text == b.text);
                if app.page == "video"
                    && app.media.session.path.as_ref() == Some(&video)
                    && same_source
                {
                    app.video.set_cues(cues);
                    app.refresh_saved_ranges();
                    app.video.active = None;
                    app.video.caption_source = VideoCaptionSource::Transcript;
                    app.video.material = None;
                    app.video.material_error.clear();
                    app.media
                        .player
                        .command(["sub-add".into(), path, "select".into()]);
                    app.report_status(
                        FeedbackKind::Info,
                        "中文字幕已生成并保存，英文和时间轴已保留".into(),
                    );
                } else {
                    app.report_status(
                        FeedbackKind::Info,
                        "翻译已完成，当前材料已改变，未切换显示".into(),
                    );
                }
            }

            _ => app.report_status(FeedbackKind::Info, "已取消".into()),
        }
    }
}

pub(super) fn leave(app: &mut AppView, next: &str) {
    app.video.remove = None;
    if app.page == "video" && next != "video" {
        // Suspend the file-bound request; preserve the page's repeat preference.
        app.reset_video_sentence_request();
    }
}
pub(super) fn enter(app: &mut AppView) {
    if app.page == "video"
        && let Some(path) = app.video.path.clone()
    {
        app.open_continue(path, false);
    }
}

impl AppView {
    fn begin_video_translate(&mut self) -> Result<()> {
        let video = self.media.session.path.clone().context("请先打开视频")?;
        video_subtitles::english_source(&self.video.cues)?;
        let source = self.video.cues.clone();
        self.begin_subtitle(
            "生成中文字幕",
            "正在准备英文字幕…",
            move |lib, cancel, tx| {
                let (path, cues) = video_subtitles::translate_with_progress(
                    &lib,
                    &video,
                    &source,
                    &cancel,
                    |stage| {
                        let _ = tx.send(TaskUpdate::SubtitleProgress(stage.into()));
                    },
                )?;
                Ok(Outcome::VideoTranslated(video, path, source, cues))
            },
        );
        Ok(())
    }

    pub(crate) fn poll_video_titles(&mut self) -> bool {
        let mut changed = false;
        if let Some(rx) = &self.tasks.title_events {
            let titles = rx.try_iter().collect::<Vec<_>>();
            for (base, title) in titles {
                let result = (|| -> Result<()> {
                    let mut videos = video_library::Collection::load(&self.lib)?;
                    if videos.update_title(&base, &title)? {
                        videos.save(&self.lib)?;
                        self.video.collection = videos;
                        if let Some(series) = self.video.series.as_mut().filter(|s| s.base == base)
                        {
                            series.title = title;
                            // Only persist when the active queue belongs to this source.
                            series.save(&self.lib)?;
                        }
                    }
                    Ok(())
                })();
                if let Err(error) = result {
                    self.report_status(FeedbackKind::Error, format!("更新视频标题失败：{error:#}"));
                }
                changed = true;
            }
        }
        changed
    }
}
