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
    Download,
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
    VideoEpisodes(String),
    VideoRemove(String),
    VideoConfirmRemove,
    VideoSubtitleExport,
    ClearYoutubeAuth,
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

            Command::ReadParts => {
                let link = app.value("bilibili", cx);
                if process::normalize_youtube(&link).is_ok() {
                    app.report_status(
                        FeedbackKind::Info,
                        "YouTube 是单视频，直接点“下载并导入”即可".into(),
                    );
                    return Ok(());
                }
                let previous = app.video.series.clone();
                app.begin(
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
                            series.current = previous.current.filter(|i| *i < series.parts.len());
                        }
                        Ok(Outcome::Parts(series))
                    },
                );
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

            Command::Download => {
                ensure!(app.page == "video", "请先进入视频学习");
                let link = app.value("bilibili", cx);
                let youtube = process::normalize_youtube(&link).ok();
                ensure!(
                    youtube.is_some() || process::normalize_bilibili(&link).is_ok(),
                    "请输入 bilibili 或 YouTube 单视频链接"
                );
                if let Some(url) = youtube {
                    app.begin_youtube_download(url, app.video.youtube_browser);
                    return Ok(());
                }
                let origin = app.media.session.revision();
                let downloads = app.tasks.downloads.clone();
                let previous = app.video.series.clone();
                app.begin(
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
                        let path = downloads.obtain(&lib, &series, index, &cancel)?;
                        let new_file = false;
                        Ok(Outcome::PartVideo(
                            series, index, path, new_file, origin, false,
                        ))
                    },
                );
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
                    app.add_assistant_reference(
                        clarora_gpui::assistant::Reference {
                            text,
                            ..Default::default()
                        },
                        window,
                        cx,
                    )?;
                } else {
                    app.add_subtitle_question(text, window, cx)?;
                }
            }

            Command::ClearYoutubeAuth => {
                app.lib.set_setting("youtube_cookie_browser", "")?;
                app.video.youtube_browser = None;
                app.report_status(
                    FeedbackKind::Info,
                    "已清除 YouTube 登录 Cookie，之后将尝试匿名下载".into(),
                );
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
                if app.select_source_only(&key, true)? {
                    return Ok(());
                }
                let mut videos = video_library::Collection::load(&app.lib)?;
                let video = videos.select(&key)?;
                videos.save(&app.lib)?;
                app.video.collection = videos;
                app.save_media_position();
                if !app.open(video.path) {
                    return Ok(());
                }
                app.video.tab = 3;
                app.video.tools_open = false;
                app.report_status(FeedbackKind::Info, format!("已选择：{}", video.title));
            }

            Command::VideoEpisodes(key) => {
                ensure!(
                    !app.tasks.foreground.is_running(),
                    "请完成或取消当前任务后再切换视频"
                );
                if app.select_source_only(&key, false)? {
                    return Ok(());
                }
                if app.video.collection.selected.as_ref() != Some(&key) || app.video.path.is_none()
                {
                    let mut videos = video_library::Collection::load(&app.lib)?;
                    let video = videos.select(&key)?;
                    videos.save(&app.lib)?;
                    app.save_media_position();
                    app.video.collection = videos;
                    if !app.open_at(video.path, 0., true) {
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
                app.add_assistant_reference(
                    clarora_gpui::assistant::Reference {
                        text: app.video.transcript.text.clone(),
                        ..Default::default()
                    },
                    window,
                    cx,
                )?;
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
    pub(super) fn begin_youtube_download(
        &mut self,
        url: String,
        browser: Option<process::YoutubeBrowser>,
    ) {
        let origin = self.media.session.revision();
        self.begin(
            "正在下载 YouTube 视频…",
            move |lib, cancel, _| match process::download_youtube_with(
                &lib.root, &url, browser, &cancel,
            ) {
                Ok(path) => Ok(Outcome::VideoDownloaded(path, origin)),
                Err(error) if process::is_youtube_auth_error(&error) => {
                    Ok(Outcome::YoutubeAuthRequired(url.clone()))
                }
                Err(error) => Err(error),
            },
        );
    }
    fn show_youtube_auth(&mut self, url: String, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, window, cx| {
            let mut browsers = div().flex().flex_wrap().gap_2();
            for browser in process::YOUTUBE_BROWSERS {
                let weak = view.clone();
                let retry_url = url.clone();
                let browser = *browser;
                browsers = browsers.child(
                    Button::new(SharedString::from(format!(
                        "youtube-auth-{}",
                        browser.id()
                    )))
                    .label(browser.label())
                    .outline()
                    .on_click(move |_, window, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            let _ = this
                                .lib
                                .set_setting("youtube_cookie_browser", browser.id());
                            this.video.youtube_browser = Some(browser);
                            this.begin_youtube_download(retry_url.clone(), Some(browser));
                            cx.notify();
                        });
                        window.close_dialog(cx);
                    }),
                );
            }
            dialog
                .title("YouTube 要求登录验证")
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
                        .child("YouTube 拒绝了匿名下载。请先在浏览器中登录 YouTube，再选择该浏览器重试。"),
                )
                .child(div().text_sm().child("选择已登录 YouTube 的浏览器："))
                .child(browsers)
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child(
                    "Cookie 只在本机交给 yt-dlp，不会上传到 Clarora；选择会记住，可在视频导入区清除。",
                ))
        });
    }
}

// Completion policy belongs to this domain: cancellation may race a durable commit.
pub(super) enum Completed {
    Parts(bilibili::Series),
    PartVideo(bilibili::Series, usize, PathBuf, bool, u64, bool),
    VideoTranslated(PathBuf, String, Vec<Cue>, Vec<Cue>),
    VideoDownloaded(PathBuf, u64),
    YoutubeAuthRequired(String),
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

            Completed::YoutubeAuthRequired(url) => {
                if context.cancelled {
                    app.report_status(FeedbackKind::Info, "已取消 YouTube 视频导入".into());
                } else {
                    app.report_status(
                        FeedbackKind::Error,
                        "YouTube 要求登录验证，请选择浏览器 Cookie 重试".into(),
                    );
                    app.show_youtube_auth(url, window, cx);
                }
            }

            Completed::VideoDownloaded(path, origin) => {
                if context.cancelled {
                    // A finished download stays in the reusable cache; a real cancel
                    // during yt-dlp is cleaned up by download_into before it returns.
                    app.report_status(FeedbackKind::Info, "已取消 YouTube 视频导入".into());
                } else {
                    let activate = app.page == "video" && app.media.session.revision() == origin;
                    let result = if activate {
                        app.save_media_position();
                        app.open(path).then_some(()).context(app.status.clone())
                    } else {
                        (|| -> Result<()> {
                            let mut videos = video_library::Collection::load(&app.lib)?;
                            videos.add(std::slice::from_ref(&path), None)?;
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
                                "已导入 YouTube 视频，可在“我的视频”切换".into(),
                            );
                        }
                        Ok(()) => app.report_status(
                            FeedbackKind::Info,
                            "YouTube 视频已导入，返回视频页可在“我的视频”播放".into(),
                        ),
                        Err(error) => app.report_status(
                            FeedbackKind::Error,
                            format!("YouTube 视频导入失败：{error:#}"),
                        ),
                    }
                }
            }

            Completed::PartVideo(mut series, index, path, new_file, origin, from_end) => {
                if context.cancelled {
                    if new_file
                        && let Some(parent) = path
                            .parent()
                            .filter(|p| p.starts_with(app.lib.root.join("downloads")))
                    {
                        let _ = fs::remove_dir_all(parent);
                    }
                    app.report_status(FeedbackKind::Info, "已取消分P下载".into());
                } else {
                    series.parts[index].path = Some(path.clone());
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
                        videos.add(std::slice::from_ref(&path), Some(&series))?;
                        videos.save(&app.lib)?;
                        app.video.collection = videos;
                        Ok(())
                    })();
                    if activate && saved.is_ok() {
                        app.video.series = Some(series);
                        // Opening media reloads subtitles; keep the panel the user is viewing.
                        let panel = app.video.tab;
                        if app.open_part_video(path, from_end) {
                            app.video.tab = panel;
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
        let position = app
            .lib
            .setting("last_video_position")
            .ok()
            .and_then(|value| serde_json::from_str::<Value>(&value).ok())
            .filter(|value| value["path"].as_str() == path.to_str())
            .and_then(|value| value["position"].as_f64())
            .filter(|position| position.is_finite() && *position >= 0.)
            .unwrap_or(0.);
        app.open_at(path, position, true);
        app.video.tab = 3;
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
