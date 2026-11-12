//! Media commands own their action effects and execution.
use super::*;
pub(super) enum Command {
    CopySelectedSubtitle,
    CopySelection(String),
    SavedClips(Vec<clip_marks::Saved>),
    Play,
    Seek(f64),
    SeekTo(f64),
    Boundary(f64, bool),
    Word(f64, f64, String, bool),
    Rate(f64),
    Volume(f64),
    Mark(bool),
    Repeat,
    LoopMode(usize),
    Mute,
    Subtitle(i64, bool),
    SubtitleImport(bool),
    Clip,
}
impl Command {
    pub(super) fn effects(&self, app: &AppView) -> Effects {
        Effects {
            restore_focus: (app.page == "listening"
                && matches!(
                    self,
                    Command::Play
                        | Command::Seek(..)
                        | Command::SeekTo(..)
                        | Command::Word(..)
                        | Command::Rate(..)
                ))
                || (matches!(app.page.as_str(), "discover" | "flashcard")
                    && matches!(self, Command::Play))
                || (app.page == "video"
                    && matches!(
                        self,
                        Command::Play
                            | Command::Seek(..)
                            | Command::SeekTo(..)
                            | Command::Boundary(..)
                            | Command::Rate(..)
                            | Command::Volume(..)
                            | Command::Mark(..)
                            | Command::Repeat
                            | Command::LoopMode(..)
                            | Command::Mute
                    )),
            sync_audio_mode: app.page == "listening"
                && matches!(
                    self,
                    Command::Boundary(..) | Command::Word(..) | Command::Mark(..)
                ),
            leave_video_sentence: matches!(
                self,
                Command::Seek(..)
                    | Command::SeekTo(..)
                    | Command::Boundary(..)
                    | Command::Mark(..)
                    | Command::Repeat
                    | Command::LoopMode(..)
                    | Command::Subtitle(..)
                    | Command::SubtitleImport(..)
            ),
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
            Command::Play => {
                ensure!(app.media.session.owner().is_some(), "请先选择要播放的材料");
                if app.restart_finished_audio_sentence()? {
                    return Ok(());
                }
                if app.restart_finished_audio_track()? {
                    return Ok(());
                }
                if app.toggle_pending_video_start() {
                    return Ok(());
                }
                let paused = app.media.player.snapshot().paused;
                app.clear_finished_video_sentence_repeat();
                if app.page == "listening" {
                    app.invalidate_blind();
                }
                app.media
                    .player
                    .set("pause", if paused { "no" } else { "yes" });
            }

            Command::Seek(delta) => {
                if app.seek_audio_repeat(app.media.player.snapshot().position + delta, false)? {
                    return Ok(());
                }
                if app.page == "listening" {
                    app.invalidate_blind();
                    app.audio.blind_settle_until =
                        Some(Instant::now() + Duration::from_millis(200));
                }
                app.media.player.command([
                    "seek".into(),
                    delta.to_string(),
                    "relative+exact".into(),
                ])
            }

            Command::Word(start, end, word, extend) => {
                ensure!(
                    !(app.page == "listening" && app.audio.blind.enabled),
                    "请先切回字幕模式"
                );
                if !app.audio.clip_mode {
                    if app.seek_audio_repeat(start, true)? {
                        return Ok(());
                    }
                    app.media.player.command([
                        "seek".into(),
                        start.to_string(),
                        "absolute+exact".into(),
                    ]);
                    app.media.player.set("pause", "no");
                } else {
                    let start = if extend {
                        app.media.session.loop_a.unwrap_or(start).min(start)
                    } else {
                        start
                    };
                    let end = if extend {
                        app.media.session.loop_b.unwrap_or(end).max(end)
                    } else {
                        end
                    };
                    app.media.session.loop_a = Some(start);
                    app.media.session.loop_b = Some(end);
                    app.media.session.mode = PlaybackMode::Ab;
                    let (front, back) = subtitles::excerpt(&app.audio.cues, start, end);
                    app.set(
                        "front",
                        if front.is_empty() { word } else { front },
                        window,
                        cx,
                    );
                    app.set("back", back, window, cx);
                    app.media.player.command([
                        "seek".into(),
                        start.to_string(),
                        "absolute+exact".into(),
                    ]);
                    app.media.player.set("pause", "no");
                    app.report_status(
                        FeedbackKind::Info,
                        "已选中词语，可调整 A/B 边界、保存片段或询问 AI；时间按字幕长度估算".into(),
                    );
                }
            }

            Command::Boundary(time, start) => {
                if app.page == "listening" && app.audio.repeat_mode.is_sentence() {
                    app.set_audio_repeat_mode(0)?;
                }
                let time = time.clamp(0., app.media.snapshot.duration.max(time));
                if start {
                    app.media.session.loop_a = Some(
                        time.min(app.media.session.loop_b.unwrap_or(f64::MAX) - 0.02)
                            .max(0.),
                    );
                } else {
                    app.media.session.loop_b =
                        Some(time.max(app.media.session.loop_a.unwrap_or(0.) + 0.02));
                }
                if let (Some(a), Some(b)) = (app.media.session.loop_a, app.media.session.loop_b) {
                    app.media.session.mode = PlaybackMode::Ab;
                    if app.page == "listening" {
                        let (front, back) = subtitles::excerpt(&app.audio.cues, a, b);
                        app.set("front", front, window, cx);
                        app.set("back", back, window, cx);
                    }
                }
            }

            Command::SeekTo(time) => {
                if app.seek_audio_repeat(time, false)? {
                    return Ok(());
                }
                if app.page == "listening" {
                    app.invalidate_blind();
                    app.audio.blind_settle_until =
                        Some(Instant::now() + Duration::from_millis(200));
                }
                app.media
                    .player
                    .command(["seek".into(), time.to_string(), "absolute+exact".into()])
            }

            Command::Rate(rate) => {
                app.media.rate = rate;
                app.media.player.set("speed", rate);
            }

            Command::Volume(volume) => {
                app.media.volume = volume.clamp(0., 100.);
                app.media.player.set("volume", app.media.volume);
            }

            Command::LoopMode(mode) => {
                if app.page == "listening" {
                    app.set_audio_repeat_mode(mode)?;
                    return Ok(());
                }
                ensure!(mode <= 2, "循环模式无效");
                if mode == 2 {
                    ensure!(
                        app.media
                            .session
                            .loop_a
                            .zip(app.media.session.loop_b)
                            .is_some_and(|(a, b)| b > a),
                        "请先设置有效的 A/B 范围"
                    );
                }
                app.media.session.mode = PlaybackMode::from_index(mode);
            }

            Command::Mute => {
                if app.media.volume > 0. {
                    app.media.volume_before_mute = app.media.volume;
                    app.media.volume = 0.;
                } else {
                    app.media.volume = app.media.volume_before_mute.max(1.);
                }
                app.media.player.set("volume", app.media.volume);
            }

            Command::Mark(start) => {
                if app.page == "listening" && app.audio.repeat_mode.is_sentence() {
                    app.set_audio_repeat_mode(0)?;
                }
                if app.page == "video" || app.page == "listening" {
                    ensure!(
                        app.media.session.path.is_some() && app.media.snapshot.duration > 0.,
                        "请先打开可播放的媒体"
                    );
                    if start {
                        app.media.session.loop_a = Some(app.media.snapshot.position);
                        if app
                            .media
                            .session
                            .loop_b
                            .is_some_and(|b| b <= app.media.snapshot.position)
                        {
                            app.media.session.loop_b = None;
                            if app.media.session.mode == PlaybackMode::Ab {
                                app.media.session.mode = PlaybackMode::Normal;
                            }
                        }
                    } else {
                        let a = app.media.session.loop_a.context("请先设置 A 点")?;
                        ensure!(app.media.snapshot.position > a, "B 点必须晚于 A 点");
                        app.media.session.loop_b = Some(app.media.snapshot.position);
                    }
                    if app
                        .media
                        .session
                        .loop_a
                        .zip(app.media.session.loop_b)
                        .is_some_and(|(a, b)| b > a)
                    {
                        app.media.session.mode = PlaybackMode::Ab;
                    }
                } else if start {
                    app.media.session.loop_a = Some(app.media.snapshot.position);
                } else {
                    app.media.session.loop_b = Some(app.media.snapshot.position);
                }
                if let (Some(a), Some(b)) = (app.media.session.loop_a, app.media.session.loop_b) {
                    ensure!(b > a, "B 点必须晚于 A 点");
                    app.media.session.mode = PlaybackMode::Ab;
                    if app.page == "listening" {
                        let (front, back) = subtitles::excerpt(&app.audio.cues, a, b);
                        app.set("front", front, window, cx);
                        app.set("back", back, window, cx);
                    }
                }
            }

            Command::Repeat => {
                if app.page == "listening" {
                    for offset in 1..=7 {
                        let mode = (app.audio.repeat_mode.index() + offset) % 7;
                        if mode == 2
                            && !app
                                .media
                                .session
                                .loop_a
                                .zip(app.media.session.loop_b)
                                .is_some_and(|(a, b)| b > a)
                        {
                            continue;
                        }
                        if mode >= 5 && app.audio.cues.is_empty() {
                            continue;
                        }
                        app.set_audio_repeat_mode(mode)?;
                        break;
                    }
                } else {
                    app.media.session.mode = PlaybackMode::from_index(
                        (app.media.session.mode.index() + 1)
                            % if app.page == "video" { 3 } else { 5 },
                    );
                }
            }

            Command::Subtitle(key, secondary) => {
                app.media.player.set(
                    if secondary { "secondary-sid" } else { "sid" },
                    if key == 0 {
                        "no".into()
                    } else {
                        key.to_string()
                    },
                );
                if !secondary {
                    app.video.caption_source = if key == 0 {
                        VideoCaptionSource::Hidden
                    } else {
                        VideoCaptionSource::Track
                    };
                }
            }

            Command::SubtitleImport(zh) => {
                app.pick(if zh { "subtitle-zh" } else { "subtitle-en" }, cx)
            }

            Command::CopySelectedSubtitle => {
                let selected = app.subtitle_text.borrow().selection_text();
                ensure!(!selected.is_empty(), "请先拖选字幕文字");
                cx.write_to_clipboard(ClipboardItem::new_string(selected));
                app.report_status(FeedbackKind::Info, "已复制选中文字".into());
            }

            Command::CopySelection(selected) => {
                ensure!(!selected.trim().is_empty(), "请先拖选文字");
                cx.write_to_clipboard(ClipboardItem::new_string(selected));
                app.report_status(FeedbackKind::Info, "已复制选中文字".into());
            }

            Command::SavedClips(saved) => app.show_saved_clips(saved, window, cx),

            Command::Clip => {
                ensure!(
                    !(app.page == "listening" && app.audio.blind.enabled),
                    "请先切回字幕模式再保存片段"
                );
                let path = app.media.session.path.clone().context("请先打开媒体")?;
                let a = app.media.session.loop_a.context("请先设置 A 点")?;
                let b = app.media.session.loop_b.context("请先设置 B 点")?;
                let front = app.value("front", cx);
                let back = app.value("back", cx);
                let is_audio = app.page == "listening";
                let source = if is_audio {
                    clip_marks::Source::audio(&app.audio.current_id)
                } else {
                    clip_marks::Source::video(&path)
                };
                app.begin_scoped(
                    TaskScope::Application,
                    "正在导出片段…",
                    move |lib, cancel, _| {
                        clip_marks::save(&lib, source, &path, a, b, &front, &back, &cancel)?;
                        Ok(Outcome::Message("片段已存为闪卡，字幕已加下划线".into()))
                    },
                );
            }
        }
        Ok(())
    }
}

pub(super) fn leave(app: &mut AppView, changed: bool) {
    app.media.player.set("pause", "yes");
    app.media.preloader.set("pause", "yes");
    app.media.preloaded_index = None;
    if changed {
        if app.media.session.owner() == Some(&PlaybackOwner::Speaking) {
            app.stop_speaking_prompt();
        } else {
            app.media.release();
        }
    }
}
