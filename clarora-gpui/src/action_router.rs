//! Application actions coordinate domain controllers.
use super::*;

impl AppView {
    pub(super) fn handle(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        self.video.picture_click = None;
        // Modal confirmation owns interaction until it is confirmed or dismissed.
        if window.has_active_dialog(cx) && !matches!(action, Action::VideoConfirmRemove) {
            return;
        }
        let restore_video_focus = self.page == "video"
            && matches!(
                &action,
                Action::Play
                    | Action::Seek(_)
                    | Action::SeekTo(_)
                    | Action::VideoTimelineSeek(_)
                    | Action::VideoPlayCue(_)
                    | Action::VideoSentenceRepeat(..)
                    | Action::VideoStopSentenceRepeat
                    | Action::VideoRepeatCue
                    | Action::VideoSentence(_)
                    | Action::LoopMode(_)
                    | Action::Repeat
                    | Action::Rate(_)
                    | Action::Volume(_)
                    | Action::Mute
                    | Action::Mark(_)
                    | Action::Boundary(..)
                    | Action::VideoClearLoop
                    | Action::VideoPrecision
                    | Action::VideoCaptions
                    | Action::VideoSubtitleSize(_)
            );
        if !matches!(action, Action::VideoArrow(..)) {
            if matches!(action, Action::SelectPart(_) | Action::SelectPartAtEnd(_)) {
                self.video.part_confirmation = None;
            } else {
                self.dismiss_video_part_confirmation();
            }
        }
        if matches!(
            &action,
            Action::Flip
                | Action::Next
                | Action::Previous
                | Action::Grade(_)
                | Action::EditCard
                | Action::Favorite
                | Action::Play
        ) && matches!(self.page.as_str(), "discover" | "flashcard")
            && !self.cards_ready()
        {
            self.report_status(FeedbackKind::Info, "正在读取学习卡片，请稍候".into());
            cx.notify();
            return;
        }
        if self.begin_feedback_action(&action, window, cx) {
            return;
        }
        let action_retry = self.feedback.retry_candidate.clone();
        let refresh_chat = matches!(&action, Action::Refresh);
        let updates_settings = matches!(
            &action,
            Action::SaveAi | Action::SaveStorage | Action::Refresh
        );
        if matches!(&action, Action::Refresh) {
            self.chat_requested_context.clear();
        }
        let leaves_sentence = match &action {
            Action::Pick(kind) => {
                kind != "video" && !(kind == "video-subtitle" && self.video.pending_start.is_some())
            }
            Action::VideoTranscribe => self.video.pending_start.is_none(),
            Action::Seek(_)
            | Action::SeekTo(_)
            | Action::VideoPlayCue(_)
            | Action::VideoClearLoop
            | Action::LoopMode(_)
            | Action::Repeat
            | Action::Mark(_)
            | Action::Boundary(..)
            | Action::Subtitle(..)
            | Action::SubtitleImport(_)
            | Action::VideoRemove(_)
            | Action::Transcribe => true,
            _ => false,
        };
        if self.page == "video"
            && leaves_sentence
            && (self.video.sentence_repeat.is_some()
                || self.video.repeat_three
                || self.video.pending_start.is_some())
        {
            self.clear_video_sentence_repeat();
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
                    self.subtitle_text.borrow_mut().clear();
                    if self.page == "writing" {
                        self.flush_writing(cx)?;
                    }
                    self.audio.blind.enabled = false;
                    self.invalidate_blind();
                    let previous = self.page.clone();
                    self.audio.library.cancel_rename();
                    self.video.remove = None;
                    self.audio.library.pending_delete.clear();
                    if self.audio.library.tool == "delete" {
                        self.audio.library.tool.clear();
                    }
                    self.save_media_position();
                    if previous == "video" && page != "video" {
                        // Suspend the old file's loop without changing the user's
                        // mode choice. Other modules share this player.
                        self.reset_video_sentence_request();
                    }
                    if self.tasks.scope == TaskScope::Material {
                        if let Some(cancel) = &self.tasks.cancel {
                            cancel.store(true, Ordering::SeqCst);
                        }
                    }
                    self.media.player.set("pause", "yes");
                    self.media.preloader.set("pause", "yes");
                    self.media.preloaded_index = None;
                    self.page = page;
                    if self.page == "writing" {
                        if let Some(view) = &self.writing {
                            view.update(cx, |view, cx| view.reload(cx))?;
                        }
                    }
                    if self.page == "library" {
                        let group = self.lib.setting("library_group")?;
                        if self.audio.practices.iter().any(|r| text(r, "id") == group) {
                            self.audio.practice_id = group;
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
                            if let Some(path) = self.video.path.clone() {
                                let position = self
                                    .lib
                                    .setting("last_video_position")
                                    .ok()
                                    .and_then(|value| serde_json::from_str::<Value>(&value).ok())
                                    .filter(|value| value["path"].as_str() == path.to_str())
                                    .and_then(|value| value["position"].as_f64())
                                    .filter(|position| position.is_finite() && *position >= 0.)
                                    .unwrap_or(0.);
                                self.open_at(path, position, true);
                                self.video.tab = 3;
                            }
                        }
                    }
                    if self.page == "pdf"
                        && self.pdf_image.is_none()
                        && self.ebook_reading.is_none()
                        && self.pdf_path.is_some()
                        && self.tasks.cancel.is_none()
                    {
                        self.render_pdf();
                    }
                }
                Action::Refresh => {
                    self.refresh();
                }
                Action::Pick(kind) => {
                    if kind == "backup" {
                        let picker =
                            cx.prompt_for_new_path(&self.lib.root, Some("clarora-library.zip"));
                        cx.spawn(async move |this, cx| {
                            if let Ok(Ok(Some(path))) = picker.await {
                                let _ = this.update(cx, |this, cx| {
                                    this.begin("正在备份资料…", move |lib, cancel, _| {
                                        backup::export_local_cancellable(&lib, &path, &cancel)?;
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
                        .audio
                        .audios
                        .iter()
                        .find(|r| text(r, "id") == key)
                        .cloned()
                        .context("音频不存在")?;
                    if self.page == "video" {
                        self.save_media_position();
                    }
                    self.reset_video_sentence_request();
                    self.open_audio(&row);
                    self.page = "listening".into();
                    self.lib.set_setting("last_page", "listening")?;
                }
                Action::Practice(key) => {
                    ensure!(
                        self.audio.practices.iter().any(|r| text(r, "id") == key),
                        "练习组不存在"
                    );
                    self.lib.set_setting("library_group", &key)?;
                    self.audio.practice_id = key;
                    self.audio.library.cancel_rename();
                    self.audio.checked.clear();
                    self.audio.library.selected.clear();
                    self.audio.library.filter = 0;
                    self.audio.library.tool.clear();
                    self.audio.library.pending_delete.clear();
                    self.set("library_search", "", window, cx);
                    self.refresh();
                }
                Action::AudioPractice(key) => {
                    self.invalidate_blind();
                    ensure!(self.tasks.cancel.is_none(), "请等待当前任务完成");
                    ensure!(
                        self.audio.practices.iter().any(|r| text(r, "id") == key),
                        "练习组不存在"
                    );
                    self.lib.set_setting("library_group", &key)?;
                    self.audio.practice_id = key;
                    if let Some(row) = self
                        .audio
                        .audios
                        .iter()
                        .find(|r| text(r, "practice_id") == self.audio.practice_id)
                        .cloned()
                    {
                        self.open_audio(&row);
                    } else {
                        self.media.player.set("pause", "yes");
                        self.audio.current_id.clear();
                        self.media.path = None;
                        self.audio.cues.clear();
                        self.audio.scroll.reset(0);
                        self.audio.active = None;
                        self.audio.waveform = None;
                        self.audio.key_points.clear();
                        if let Some(cancel) = self.audio.waveform_cancel.take() {
                            cancel.store(true, Ordering::SeqCst);
                        }
                        self.audio.waveform_rx = None;
                        self.lib.set_setting("last_listening_audio", "")?;
                        if self.media.repeat == 2 {
                            self.media.repeat = 0;
                        }
                        self.media.loop_a = None;
                        self.media.loop_b = None;
                    }
                }
                Action::AudioBlind(enabled) => {
                    self.subtitle_text.borrow_mut().clear();
                    ensure!(self.page == "listening", "请先进入音频学习");
                    ensure!(self.tasks.cancel.is_none(), "请等待当前任务完成");
                    self.audio.blind.enabled = enabled;
                    self.invalidate_blind();
                    self.assistant_open = false;
                    self.output.clear();
                    self.chat.reasoning.clear();
                    if enabled {
                        if self.media.loop_a.is_none() || self.media.loop_b.is_none() {
                            self.media.loop_a = None;
                            self.media.loop_b = None;
                            if self.media.repeat == 2 {
                                self.media.repeat = 0;
                            }
                        }
                        self.audio.tools_open = false;
                        self.report_status(FeedbackKind::Info, "盲听已开启，字幕已隐藏".into());
                    } else {
                        self.media.player.set("pause", "yes");
                        self.report_status(FeedbackKind::Info, "已切回字幕模式，保持暂停".into());
                    }
                    self.sync_chat(window, cx);
                    window.focus(&self.focus);
                }
                Action::AudioBlindChinese(enabled) => {
                    ensure!(
                        self.page == "listening" && self.audio.blind.enabled,
                        "请先开启盲听"
                    );
                    self.lib.set_setting(
                        "blind_chinese_hint",
                        if enabled { "true" } else { "false" },
                    )?;
                    self.audio.blind_chinese = enabled;
                    self.invalidate_blind();
                    self.assistant_open = false;
                    self.sync_chat(window, cx);
                    window.focus(&self.focus);
                    self.report_status(
                        FeedbackKind::Info,
                        if enabled {
                            "查看时仅显示中文；平时中英文均遮挡"
                        } else {
                            "查看时显示中英字幕；平时均遮挡"
                        }
                        .into(),
                    );
                }
                action @ (Action::AudioBlindReveal | Action::AudioBlindRevealCue(..)) => {
                    ensure!(
                        self.page == "listening" && self.audio.blind.enabled,
                        "请先开启盲听"
                    );
                    ensure!(
                        self.tasks.cancel.is_none() && self.audio.blind_pending.is_none(),
                        "请等待当前操作完成"
                    );
                    ensure!(
                        self.audio
                            .blind_settle_until
                            .is_none_or(|t| Instant::now() >= t),
                        "正在定位，请稍候重试"
                    );
                    let playback = self.media.player.snapshot();
                    ensure!(
                        !self.audio.current_id.is_empty()
                            && self
                                .media
                                .path
                                .as_ref()
                                .is_some_and(|p| p.to_string_lossy() == playback.path),
                        "音频正在切换，请稍候"
                    );
                    let target = if let Action::AudioBlindRevealCue(audio_id, cue) = action {
                        ensure!(audio_id == self.audio.current_id, "音频已切换，请重新选择");
                        blind_listening::row_target(&self.audio.cues, &cue, playback.duration)
                            .context("字幕已变化或时间无效，请重新选择")?;
                        blind_listening::Revealed { cue, sparse: false }
                    } else {
                        blind_listening::resolve(
                            &self.audio.cues,
                            playback.position,
                            playback.duration,
                        )
                        .context("播放到有字幕的部分后再查看")?
                    };
                    self.invalidate_blind();
                    self.assistant_open = false;
                    self.sync_chat(window, cx);
                    self.audio.blind_pending =
                        Some((Instant::now(), self.audio.current_id.clone(), target));
                    self.media.player.set("pause", "yes");
                    self.report_status(
                        FeedbackKind::Progress,
                        "正在暂停，确认后显示所选字幕…".into(),
                    );
                }
                Action::AudioBlindHide => {
                    self.invalidate_blind();
                    self.sync_chat(window, cx);
                    window.focus(&self.focus);
                }
                Action::AudioBlindResume(restart) => {
                    ensure!(
                        self.page == "listening"
                            && self.audio.blind.enabled
                            && !self.audio.current_id.is_empty(),
                        "请先选择音频并开启盲听"
                    );
                    self.invalidate_blind();
                    self.assistant_open = false;
                    if restart {
                        self.media.player.command(["seek", "0", "absolute+exact"]);
                    }
                    self.media.player.set("pause", "no");
                    self.report_status(FeedbackKind::Info, "正在盲听，字幕已隐藏".into());
                    self.audio.blind_settle_until =
                        Some(Instant::now() + Duration::from_millis(200));
                    self.sync_chat(window, cx);
                    window.focus(&self.focus);
                }
                Action::AudioBlindRow(audio_id, cue, word_index) => {
                    ensure!(
                        self.page == "listening" && self.audio.blind.enabled,
                        "请先开启盲听"
                    );
                    ensure!(self.tasks.cancel.is_none(), "请等待当前任务完成");
                    let playback = self.media.player.snapshot();
                    ensure!(
                        audio_id == self.audio.current_id
                            && !audio_id.is_empty()
                            && self
                                .media
                                .path
                                .as_ref()
                                .is_some_and(|p| p.to_string_lossy() == playback.path),
                        "音频正在切换，请重试"
                    );
                    let start = match word_index {
                        Some(index) => blind_listening::word_target(
                            &self.audio.cues,
                            &cue,
                            index,
                            playback.duration,
                        ),
                        None => {
                            blind_listening::row_target(&self.audio.cues, &cue, playback.duration)
                        }
                    }
                    .context("字幕已变化或时间无效，请重新选择")?;
                    self.invalidate_blind();
                    self.assistant_open = false;
                    let outside_loop = self.media.repeat == 2
                        && self
                            .media
                            .loop_a
                            .zip(self.media.loop_b)
                            .is_some_and(|(a, b)| start < a || start >= b);
                    if outside_loop {
                        self.media.repeat = 0;
                        self.media.loop_a = None;
                        self.media.loop_b = None;
                    }
                    self.media.player.command([
                        "seek".into(),
                        start.to_string(),
                        "absolute+exact".into(),
                    ]);
                    self.media.player.set("pause", "no");
                    self.audio.blind_settle_until =
                        Some(Instant::now() + Duration::from_millis(250));
                    self.report_status(
                        FeedbackKind::Info,
                        if outside_loop {
                            "已退出 A–B 循环，从所选句开始盲听"
                        } else {
                            "已从所选位置开始盲听，字幕保持遮挡"
                        }
                        .into(),
                    );
                    self.sync_chat(window, cx);
                    window.focus(&self.focus);
                }
                Action::AudioBlindSentence(direction) => {
                    ensure!(
                        self.page == "listening" && self.audio.blind.enabled,
                        "请先开启盲听"
                    );
                    ensure!(
                        self.tasks.cancel.is_none() && self.audio.blind_pending.is_none(),
                        "请等待当前操作完成"
                    );
                    ensure!(
                        self.audio
                            .blind_settle_until
                            .is_none_or(|t| Instant::now() >= t),
                        "正在定位，请稍候重试"
                    );
                    let playback = self.media.player.snapshot();
                    ensure!(
                        !self.audio.current_id.is_empty()
                            && self
                                .media
                                .path
                                .as_ref()
                                .is_some_and(|p| p.to_string_lossy() == playback.path),
                        "音频正在切换，请稍候"
                    );
                    let target = blind_listening::sentence_target(
                        &self.audio.cues,
                        playback.position,
                        playback.duration,
                        self.audio.blind.revealed.as_ref(),
                        direction,
                        self.media
                            .loop_a
                            .zip(self.media.loop_b)
                            .filter(|(a, b)| self.media.repeat == 2 && b > a),
                    )
                    .context("当前范围内没有可播放的字幕句")?;
                    self.invalidate_blind();
                    self.assistant_open = false;
                    self.media.player.command([
                        "seek".into(),
                        target.cue.start.to_string(),
                        "absolute+exact".into(),
                    ]);
                    self.media.player.set("pause", "no");
                    self.audio.blind_settle_until =
                        Some(Instant::now() + Duration::from_millis(250));
                    self.report_status(
                        FeedbackKind::Info,
                        match direction {
                            -1 => "已从上一句开始盲听",
                            1 => "已从下一句开始盲听",
                            _ => "已隐藏文字，从句首重听",
                        }
                        .into(),
                    );
                    self.sync_chat(window, cx);
                    window.focus(&self.focus);
                }
                Action::AudioClipMode => {
                    ensure!(!self.audio.blind.enabled, "请先切回字幕模式");
                    self.audio.clip_mode = !self.audio.clip_mode;
                    if !self.audio.clip_mode {
                        self.media.loop_a = None;
                        self.media.loop_b = None;
                        if self.media.repeat == 2 {
                            self.media.repeat = 0;
                        }
                    }
                }
                Action::AudioTools => {
                    ensure!(!self.audio.blind.enabled, "请先切回字幕模式");
                    self.audio.tools_open = !self.audio.tools_open;
                }
                Action::DismissFeedback(_) | Action::RetryFeedback => {}
                Action::AudioCurrent => self.audio.show_current = !self.audio.show_current,
                Action::AudioFollow => {
                    self.audio.follow = !self.audio.follow;
                    if self.audio.follow {
                        if let Some(i) = self.audio.active {
                            self.audio.center.request(i);
                        }
                    }
                }
                Action::AudioFont(size) => {
                    ensure!([14, 16, 18, 20, 24, 28, 32].contains(&size), "无效字幕字号");
                    self.audio.font_size = size;
                    self.lib
                        .set_setting("subtitle_font_size", &size.to_string())?;
                    self.invalidate_blind();
                    self.audio.scroll.reset(self.audio.cues.len());
                    if let Some(index) = self.audio.active {
                        self.audio.center.request(index);
                    }
                }
                Action::AudioRepeat(mode) => {
                    ensure!(mode <= 4, "无效循环方式");
                    if mode == 2 {
                        ensure!(
                            self.media
                                .loop_a
                                .zip(self.media.loop_b)
                                .is_some_and(|(a, b)| b > a),
                            "先截取有效片段"
                        );
                    }
                    self.media.repeat = mode;
                }
                Action::LibrarySelect { id: key, reveal } => {
                    ensure!(
                        self.audio.audios.iter().any(|r| text(r, "id") == key
                            && text(r, "practice_id") == self.audio.practice_id),
                        "音频已不存在"
                    );
                    if self
                        .audio
                        .library
                        .renaming
                        .as_ref()
                        .is_some_and(|target| target.id != key)
                    {
                        self.audio.library.cancel_rename();
                    }
                    if self.audio.library.selected != key {
                        self.audio.library.preview.clear();
                        self.audio.library.subtitle_count = 0;
                    }
                    self.audio.library.selected = key;
                    if reveal {
                        self.audio.library.tool = "detail".into();
                        self.page_scroll.set_offset(point(px(0.), px(0.)));
                    }
                    self.audio.library.pending_delete.clear();
                    self.refresh();
                }
                Action::LibraryRenameAudio(key) => {
                    ensure!(self.page == "library", "请先返回音频管理");
                    ensure!(self.tasks.cancel.is_none(), "请等待当前任务完成");
                    let name = self.audio.library.begin_rename(
                        &self.audio.practice_id,
                        &self.audio.audios,
                        &key,
                    )?;
                    // Preserve the list's position when details already precede it in a narrow window.
                    if self.audio.library.tool != "detail" {
                        self.audio.library.tool.clear();
                    }
                    self.audio.library.pending_delete.clear();
                    let query = self.value("library_search", cx);
                    if !self
                        .audio
                        .library
                        .visible(&self.audio.audios, &self.audio.practice_id, &query)
                        .iter()
                        .any(|r| text(r, "id") == key)
                    {
                        self.audio.library.filter = 0;
                        self.set("library_search", "", window, cx);
                    }
                    self.refresh();
                    self.set("library_audio_name", name, window, cx);
                    self.inputs["library_audio_name"]
                        .update(cx, |state, cx| state.focus(window, cx));
                    cx.on_next_frame(window, move |this, window, cx| {
                        if this.page == "library"
                            && this
                                .audio
                                .library
                                .renaming
                                .as_ref()
                                .is_some_and(|t| t.id == key)
                        {
                            let query = this.value("library_search", cx);
                            if let Some(index) = this
                                .audio
                                .library
                                .visible(&this.audio.audios, &this.audio.practice_id, &query)
                                .iter()
                                .position(|r| text(r, "id") == key)
                            {
                                this.audio.library_scroll.scroll_to_item(index);
                            }
                            this.inputs["library_audio_name"]
                                .update(cx, |state, cx| state.focus(window, cx));
                            window.dispatch_action(Box::new(gpui_component::input::SelectAll), cx);
                            cx.notify();
                        }
                    });
                }
                Action::LibraryCommitRename => {
                    ensure!(self.page == "library", "请先返回音频管理");
                    ensure!(self.tasks.cancel.is_none(), "请等待当前任务完成");
                    let name = self.value("library_audio_name", cx);
                    self.audio
                        .library
                        .save_rename(&self.lib, &self.audio.practice_id, &name)?;
                    self.refresh();
                    window.focus(&self.focus);
                    self.report_status(FeedbackKind::Info, "名称已保存".into());
                }
                Action::LibraryCancelRename => {
                    self.audio.library.cancel_rename();
                    window.focus(&self.focus);
                }
                Action::LibraryTool(tool) => {
                    self.audio.library.cancel_rename();
                    self.audio.library.pending_delete.clear();
                    let name = match tool.as_str() {
                        "group-new" => audio_library::default_practice_name(
                            &self.audio.practices,
                            chrono::Local::now().date_naive(),
                        ),
                        "group-rename" => self
                            .audio
                            .practices
                            .iter()
                            .find(|r| text(r, "id") == self.audio.practice_id)
                            .map(|r| text(r, "name"))
                            .unwrap_or_default(),
                        _ => String::new(),
                    };
                    self.set("library_name", name, window, cx);
                    self.audio.library.tool = tool;
                    self.page_scroll.set_offset(point(px(0.), px(0.)));
                    let field = if self.audio.library.tool == "tts" {
                        Some("tts_text")
                    } else if ["group-new", "group-rename"]
                        .contains(&self.audio.library.tool.as_str())
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
                    ensure!(self.tasks.cancel.is_none(), "请等待当前任务完成");
                    let name = self.value("library_name", cx);
                    match self.audio.library.tool.as_str() {
                        "group-new" => {
                            ensure!(
                                !name.trim().is_empty() && name.chars().count() <= 120,
                                "名称需为 1–120 字"
                            );
                            ensure!(
                                !self.audio.practices.iter().any(|r| text(r, "name")
                                    .trim()
                                    .eq_ignore_ascii_case(name.trim())),
                                "已有同名练习组，请直接选择或换一个名称"
                            );
                            self.audio.practice_id = self.lib.practice(name.trim())?;
                            self.audio.checked.clear();
                            self.audio.library.selected.clear();
                            self.audio.library.filter = 0;
                            self.set("library_search", "", window, cx);
                        }
                        "group-rename" => audio_library::rename(
                            &self.lib,
                            "listening_practices",
                            &self.audio.practice_id,
                            &name,
                        )?,
                        _ => anyhow::bail!("请重新打开名称编辑"),
                    }
                    self.lib
                        .set_setting("library_group", &self.audio.practice_id)?;
                    self.audio.library.tool.clear();
                    self.refresh();
                    self.report_status(FeedbackKind::Info, "名称已保存".into());
                }
                Action::LibraryFilter(filter) => {
                    self.audio.library.filter = filter;
                }
                Action::LibrarySort(sort) => {
                    self.audio.library.sort = sort;
                }
                Action::LibraryBatch => {
                    self.audio.library.batch = !self.audio.library.batch;
                    self.audio.checked.clear();
                    self.audio.library.pending_delete.clear();
                }
                Action::LibraryReset => {
                    self.audio.library.filter = 0;
                    self.set("library_search", "", window, cx);
                }
                Action::LibrarySelectAll => {
                    let ids = self
                        .audio
                        .library
                        .visible(
                            &self.audio.audios,
                            &self.audio.practice_id,
                            &self.value("library_search", cx),
                        )
                        .into_iter()
                        .map(|r| text(r, "id"))
                        .collect::<Vec<_>>();
                    if !ids.is_empty() && ids.iter().all(|id| self.audio.checked.contains(id)) {
                        for id in ids {
                            self.audio.checked.remove(&id);
                        }
                    } else {
                        self.audio.checked.extend(ids);
                    }
                    self.audio.library.pending_delete.clear();
                }
                Action::LibraryMove(destination) => {
                    ensure!(self.tasks.cancel.is_none(), "请等待当前任务完成");
                    let ids = audio_library::scoped_ids(
                        &self.audio.audios,
                        &self.audio.practice_id,
                        &self.audio.checked,
                    );
                    audio_library::move_records(
                        &self.lib,
                        &self.audio.practice_id,
                        &destination,
                        &ids,
                    )?;
                    self.audio.checked.clear();
                    self.audio.library.tool.clear();
                    self.refresh();
                    self.report_status(FeedbackKind::Info, format!("已移动 {} 条音频", ids.len()));
                }
                Action::LibraryDelete(ids) => {
                    ensure!(self.tasks.cancel.is_none(), "请等待当前任务完成");
                    self.audio.library.cancel_rename();
                    self.audio.library.pending_delete = ids;
                    self.audio.library.tool = "delete".into();
                }
                Action::LibraryConfirmDelete => {
                    ensure!(self.tasks.cancel.is_none(), "请等待当前任务完成");
                    let ids = self.audio.library.pending_delete.clone();
                    audio_library::delete_records(&self.lib, &self.audio.practice_id, &ids)?;
                    self.audio.checked.clear();
                    self.audio.library.pending_delete.clear();
                    self.audio.library.tool.clear();
                    self.refresh();
                    self.report_status(
                        FeedbackKind::Info,
                        format!("已移除 {} 条记录，附件文件保留", ids.len()),
                    );
                }
                Action::LibraryStudy(paused) => {
                    let key = self.audio.library.selected.clone();
                    ensure!(
                        self.audio.audios.iter().any(|r| text(r, "id") == key
                            && text(r, "practice_id") == self.audio.practice_id),
                        "请先选择音频"
                    );
                    self.audio.blind.enabled = false;
                    self.handle(Action::Audio(key), window, cx);
                    if paused {
                        self.media.player.set("pause", "yes");
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
                            self.index = if let Some(index) = self.media.preloaded_index {
                                index
                            } else {
                                let due = &self.loads.discover_due;
                                let favorites = &self.loads.discover_favorites;
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
                    self.report_status(FeedbackKind::Info, format!("下次复习：{due}"));
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
                    self.report_status(
                        FeedbackKind::Info,
                        "在下方编辑原文与翻译，然后保存修改".into(),
                    );
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
                    self.report_status(FeedbackKind::Info, "卡片已更新，复习进度保留".into());
                }
                Action::SaveSentence => {
                    documents::save_sentence(
                        &self.lib,
                        &self.value("front", cx),
                        &self.value("back", cx),
                    )?;
                    self.refresh();
                    self.report_status(FeedbackKind::Info, "句子卡已保存".into());
                }
                Action::Play => {
                    if self.toggle_pending_video_start() {
                        return Ok(());
                    }
                    let paused = self.media.player.snapshot().paused;
                    self.clear_finished_video_sentence_repeat();
                    if self.page == "listening" {
                        self.invalidate_blind();
                    }
                    self.media
                        .player
                        .set("pause", if paused { "no" } else { "yes" });
                }
                Action::Seek(delta) => {
                    if self.page == "listening" {
                        self.invalidate_blind();
                        self.audio.blind_settle_until =
                            Some(Instant::now() + Duration::from_millis(200));
                    }
                    self.media.player.command([
                        "seek".into(),
                        delta.to_string(),
                        "relative+exact".into(),
                    ])
                }
                Action::Word(start, end, word, extend) => {
                    ensure!(
                        !(self.page == "listening" && self.audio.blind.enabled),
                        "请先切回字幕模式"
                    );
                    if !self.audio.clip_mode {
                        self.media.player.command([
                            "seek".into(),
                            start.to_string(),
                            "absolute+exact".into(),
                        ]);
                        self.media.player.set("pause", "no");
                    } else {
                        let start = if extend {
                            self.media.loop_a.unwrap_or(start).min(start)
                        } else {
                            start
                        };
                        let end = if extend {
                            self.media.loop_b.unwrap_or(end).max(end)
                        } else {
                            end
                        };
                        self.media.loop_a = Some(start);
                        self.media.loop_b = Some(end);
                        self.media.repeat = 2;
                        let (front, back) = subtitles::excerpt(&self.audio.cues, start, end);
                        self.set(
                            "front",
                            if front.is_empty() { word } else { front },
                            window,
                            cx,
                        );
                        self.set("back", back, window, cx);
                        self.media.player.command([
                            "seek".into(),
                            start.to_string(),
                            "absolute+exact".into(),
                        ]);
                        self.media.player.set("pause", "no");
                        self.report_status(
                            FeedbackKind::Info,
                            "已选中词语，可调整 A/B 边界、保存片段或询问 AI；时间按字幕长度估算"
                                .into(),
                        );
                    }
                }
                Action::Boundary(time, start) => {
                    let time = time.clamp(0., self.media.snapshot.duration.max(time));
                    if start {
                        self.media.loop_a = Some(
                            time.min(self.media.loop_b.unwrap_or(f64::MAX) - 0.02)
                                .max(0.),
                        );
                    } else {
                        self.media.loop_b = Some(time.max(self.media.loop_a.unwrap_or(0.) + 0.02));
                    }
                    if let (Some(a), Some(b)) = (self.media.loop_a, self.media.loop_b) {
                        self.media.repeat = 2;
                        if self.page == "listening" {
                            let (front, back) = subtitles::excerpt(&self.audio.cues, a, b);
                            self.set("front", front, window, cx);
                            self.set("back", back, window, cx);
                        }
                    }
                }
                Action::VideoTimelineSeek(time) => {
                    if self.page == "video" && self.video.repeat_three {
                        self.seek_video_timeline_three(time)?;
                    } else {
                        self.handle(Action::SeekTo(time), window, cx);
                    }
                }
                Action::SeekTo(time) => {
                    if self.page == "listening" {
                        self.invalidate_blind();
                        self.audio.blind_settle_until =
                            Some(Instant::now() + Duration::from_millis(200));
                    }
                    self.media.player.command([
                        "seek".into(),
                        time.to_string(),
                        "absolute+exact".into(),
                    ])
                }
                Action::Navigation => {
                    let open = !self.navigation_open;
                    self.lib
                        .set_setting("navigation_open", if open { "true" } else { "false" })?;
                    let from = self.navigation_progress();
                    self.navigation_open = open;
                    self.navigation_motion = Some((Instant::now(), from));
                }
                Action::Assistant => {
                    if self.assistant_open {
                        self.assistant_open = false;
                        window.focus(&self.focus);
                        self.lib.set_setting("assistant_open", "false")?;
                    } else {
                        self.open_assistant(window, cx);
                        self.inputs["question"].update(cx, |state, cx| state.focus(window, cx));
                    }
                }
                Action::AssistantQuote(reference) => {
                    self.add_assistant_reference(reference, window, cx)?;
                }
                Action::AssistantCurrent => {
                    if self.page == "video" {
                        self.prepare_video_material(video_qa::Scope::Current)?;
                    }
                    let reference = clarora_gpui::assistant::Reference {
                        text: clarora_gpui::assistant::plain_reference(&self.reference(cx)),
                    };
                    self.add_assistant_reference(reference, window, cx)?;
                }
                Action::AssistantRemove(index) => {
                    if index < self.assistant.references.len() {
                        self.assistant.references.remove(index);
                        self.persist_assistant_references()?;
                    }
                }
                Action::AssistantReferenceDetails => {
                    self.assistant.expanded = !self.assistant.expanded
                }
                Action::AssistantTurnReference(index) => {
                    self.assistant.expanded_turn = if self.assistant.expanded_turn == Some(index) {
                        None
                    } else {
                        Some(index)
                    };
                }
                Action::AssistantSessions => {
                    self.assistant.sessions_open = !self.assistant.sessions_open
                }
                Action::AssistantWide => self.assistant.wide = !self.assistant.wide,
                Action::AssistantLegacy => {
                    ensure!(!self.chat.running, "请先停止当前回答");
                    self.assistant.legacy = !self.assistant.legacy;
                    self.assistant.pending = None;
                    self.assistant.error.clear();
                    self.chat_requested_context.clear();
                    self.sync_chat(window, cx);
                }
                Action::AssistantCopy(answer) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(answer));
                }
                Action::AssistantPrompt(question) => {
                    self.set("question", question, window, cx);
                    self.inputs["question"].update(cx, |state, cx| state.focus(window, cx));
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
                    self.media.rate = rate;
                    self.media.player.set("speed", rate);
                }
                Action::Volume(volume) => {
                    self.media.volume = volume.clamp(0., 100.);
                    self.media.player.set("volume", self.media.volume);
                }
                Action::VideoFullscreen => {
                    ensure!(
                        self.page == "video" && self.media.path.is_some(),
                        "请先打开视频"
                    );
                    if self.video.fullscreen {
                        self.leave_video_fullscreen(window, cx);
                    } else {
                        self.video.fullscreen_owned = !window.is_fullscreen();
                        self.video.fullscreen_seen = window.is_fullscreen();
                        self.video.fullscreen = true;
                        window.focus(&self.focus);
                        if self.video.fullscreen_owned {
                            window.toggle_fullscreen();
                        }
                    }
                }
                Action::VideoExitFullscreen => self.leave_video_fullscreen(window, cx),
                Action::VideoCaptions => {
                    ensure!(self.page == "video", "请先进入视频学习");
                    let enabled = !self.video.captions_enabled;
                    self.lib.set_setting(
                        "video_captions_enabled",
                        if enabled { "true" } else { "false" },
                    )?;
                    self.video.captions_enabled = enabled;
                    if enabled && self.video.caption_source == VideoCaptionSource::Hidden {
                        self.video.caption_source = if self.video.cues.is_empty() {
                            VideoCaptionSource::Track
                        } else {
                            VideoCaptionSource::Transcript
                        };
                        self.media.player.set("sid", "auto");
                    }
                    self.media.player.set_subtitles_visible(enabled);
                    self.report_status(
                        FeedbackKind::Info,
                        if enabled {
                            "画面字幕已开启"
                        } else {
                            "画面字幕已关闭，字幕列表仍保留"
                        }
                        .into(),
                    );
                }
                Action::VideoTranslate => {
                    ensure!(self.tasks.video_job.is_none(), "请先完成后台字幕生成再翻译");
                    ensure!(self.page == "video", "请先进入视频学习");
                    ensure!(self.tasks.cancel.is_none(), "请等待当前任务完成");
                    let video = self.media.path.clone().context("请先打开视频")?;
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
                                    let _ = tx.send(Event::SubtitleProgress(stage.into()));
                                },
                            )?;
                            Ok(Outcome::VideoTranslated(video, path, source, cues))
                        },
                    );
                }
                Action::VideoSubtitleSize(size) => {
                    ensure!([14, 16, 20, 24, 28, 32].contains(&size), "无效字幕字号");
                    self.lib
                        .set_setting("video_subtitle_size", &size.to_string())?;
                    self.video.subtitle_size = size;
                }
                Action::VideoPrecision => self.video.precision = !self.video.precision,
                Action::VideoClearLoop => {
                    self.media.loop_a = None;
                    self.media.loop_b = None;
                    if self.media.repeat == 2 {
                        self.media.repeat = 0;
                    }
                }
                Action::LoopMode(mode) => {
                    ensure!(mode <= 2, "循环模式无效");
                    if mode == 2 {
                        ensure!(
                            self.media
                                .loop_a
                                .zip(self.media.loop_b)
                                .is_some_and(|(a, b)| b > a),
                            "请先设置有效的 A/B 范围"
                        );
                    }
                    self.media.repeat = mode;
                }
                Action::VideoRepeatCue => {
                    self.start_video_sentence_repeat(None, false)?;
                }
                Action::VideoSentenceRepeat(index, three) => {
                    self.start_video_sentence_repeat(index, three)?;
                }
                Action::VideoStopSentenceRepeat => self.clear_video_sentence_repeat(),
                Action::VideoArrow(next, held) => self.video_arrow(next, held, window, cx),
                Action::VideoCancelPart => {}
                Action::VideoPlayCue(index) => {
                    ensure!(
                        self.page == "video" && self.media.path.is_some(),
                        "请先打开视频"
                    );
                    let cue = self.video.cues.get(index).context("字幕行不存在")?;
                    let start = cue.start;
                    ensure!(start.is_finite() && start >= 0., "字幕时间无效");
                    // A previous sentence loop must not pull playback back to its B point.
                    if self.media.repeat == 2 {
                        self.media.repeat = 0;
                    }
                    self.media.player.command([
                        "seek".into(),
                        start.to_string(),
                        "absolute+exact".into(),
                    ]);
                    self.media.player.set("pause", "no");
                }
                Action::VideoSentence(next) => {
                    if self.video.pending_start.is_some() {
                        self.poll_pending_video_start();
                        if self.video.pending_start.is_some() {
                            self.report_status(
                                FeedbackKind::Info,
                                "每句三次模式正在等待当前视频和字幕就绪".into(),
                            );
                            return Ok(());
                        }
                    }
                    if let Some(mut session) = self.video.sentence_repeat.filter(|s| s.three) {
                        let path = self.video.path.as_ref().context("请先打开视频")?;
                        ensure!(self.media.path.as_ref() == Some(path), "视频正在切换");
                        if session.step(
                            &self.media.player,
                            path,
                            &self.video.cues,
                            next,
                            self.media.snapshot.duration,
                        )? {
                            self.video.sentence_repeat = Some(session);
                            self.follow_video_repeat();
                        } else {
                            self.report_status(
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
                    let path = self.video.path.as_ref().context("请先打开视频")?;
                    ensure!(
                        self.page == "video"
                            && self.media.path.as_ref() == Some(path)
                            && self.media.snapshot.path == path.to_string_lossy(),
                        "视频正在切换，请稍候再切句"
                    );
                    let Some(index) = clarora_gpui::video_repeat::adjacent(
                        &self.video.cues,
                        self.media.snapshot.position,
                        next,
                        self.media.snapshot.duration,
                    ) else {
                        self.report_status(
                            FeedbackKind::Info,
                            if self.video.cues.is_empty() {
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
                    let target = self.video.cues[index].start;
                    self.clear_video_sentence_repeat();
                    // Moving to another sentence exits the previous A/B loop.
                    if self.media.repeat == 2 {
                        self.media.repeat = 0;
                    }
                    self.media.player.command([
                        "seek".into(),
                        target.to_string(),
                        "absolute+exact".into(),
                    ]);
                }
                Action::Mute => {
                    if self.media.volume > 0. {
                        self.media.volume_before_mute = self.media.volume;
                        self.media.volume = 0.;
                    } else {
                        self.media.volume = self.media.volume_before_mute.max(1.);
                    }
                    self.media.player.set("volume", self.media.volume);
                }
                Action::Mark(start) => {
                    if self.page == "video" || self.page == "listening" {
                        ensure!(
                            self.media.path.is_some() && self.media.snapshot.duration > 0.,
                            "请先打开可播放的媒体"
                        );
                        if start {
                            self.media.loop_a = Some(self.media.snapshot.position);
                            if self
                                .media
                                .loop_b
                                .is_some_and(|b| b <= self.media.snapshot.position)
                            {
                                self.media.loop_b = None;
                                if self.media.repeat == 2 {
                                    self.media.repeat = 0;
                                }
                            }
                        } else {
                            let a = self.media.loop_a.context("请先设置 A 点")?;
                            ensure!(self.media.snapshot.position > a, "B 点必须晚于 A 点");
                            self.media.loop_b = Some(self.media.snapshot.position);
                        }
                        if self
                            .media
                            .loop_a
                            .zip(self.media.loop_b)
                            .is_some_and(|(a, b)| b > a)
                        {
                            self.media.repeat = 2;
                        }
                    } else if start {
                        self.media.loop_a = Some(self.media.snapshot.position);
                    } else {
                        self.media.loop_b = Some(self.media.snapshot.position);
                    }
                    if let (Some(a), Some(b)) = (self.media.loop_a, self.media.loop_b) {
                        ensure!(b > a, "B 点必须晚于 A 点");
                        self.media.repeat = 2;
                        if self.page == "listening" {
                            let (front, back) = subtitles::excerpt(&self.audio.cues, a, b);
                            self.set("front", front, window, cx);
                            self.set("back", back, window, cx);
                        }
                    }
                }
                Action::Repeat => {
                    self.media.repeat =
                        (self.media.repeat + 1) % if self.page == "video" { 3 } else { 5 };
                }
                Action::Subtitle(key, secondary) => {
                    self.media.player.set(
                        if secondary { "secondary-sid" } else { "sid" },
                        if key == 0 {
                            "no".into()
                        } else {
                            key.to_string()
                        },
                    );
                    if !secondary {
                        self.video.caption_source = if key == 0 {
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
                    let previous = self.video.series.clone();
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
                Action::SelectPartAtEnd(index) => self.start_part_from(index, true),
                Action::AutoNextPart => {
                    let enabled = !self.video.auto_next_part;
                    self.lib
                        .set_setting("video_auto_next", if enabled { "true" } else { "false" })?;
                    self.video.auto_next_part = enabled;
                    self.media.end_handled = false;
                }
                Action::Download => {
                    let downloads = self.tasks.downloads.clone();
                    let link = self.value("bilibili", cx);
                    let previous = self.video.series.clone();
                    let origin = self.media.path.clone();
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
                            let path = downloads.obtain(&lib, &series, index, &cancel)?;
                            let new_file = false;
                            Ok(Outcome::PartVideo(
                                series, index, path, new_file, origin, false,
                            ))
                        },
                    );
                }
                Action::DismissTaskNotice => {
                    if self.tasks.notice.as_ref().is_some_and(|n| !n.active()) {
                        self.tasks.notice = None;
                    }
                }
                Action::Cancel => {
                    if let Some(cancel) = &self.tasks.cancel {
                        cancel.store(true, Ordering::SeqCst);
                        self.report_status(FeedbackKind::Progress, "正在取消…".into());
                        if let Some(notice) = self.tasks.notice.as_mut().filter(|n| n.active()) {
                            notice.state = TaskState::Cancelling;
                            notice.detail = "正在停止当前处理，请稍候…".into();
                        }
                    }
                }
                Action::CopySelectedSubtitle => {
                    let selected = self.subtitle_text.borrow().selection_text();
                    ensure!(!selected.is_empty(), "请先拖选字幕文字");
                    cx.write_to_clipboard(ClipboardItem::new_string(selected));
                    self.report_status(FeedbackKind::Info, "已复制选中文字".into());
                }
                Action::SavedClips(saved) => self.show_saved_clips(saved, window, cx),
                Action::Clip => {
                    ensure!(
                        !(self.page == "listening" && self.audio.blind.enabled),
                        "请先切回字幕模式再保存片段"
                    );
                    let path = self.media.path.clone().context("请先打开媒体")?;
                    let a = self.media.loop_a.context("请先设置 A 点")?;
                    let b = self.media.loop_b.context("请先设置 B 点")?;
                    let front = self.value("front", cx);
                    let back = self.value("back", cx);
                    let is_audio = self.page == "listening";
                    let source = if is_audio {
                        clip_marks::Source::audio(&self.audio.current_id)
                    } else {
                        clip_marks::Source::video(&path)
                    };
                    self.begin_scoped(
                        TaskScope::Application,
                        "正在导出片段…",
                        move |lib, cancel, _| {
                            clip_marks::save(&lib, source, &path, a, b, &front, &back, &cancel)?;
                            Ok(Outcome::Message("片段已存为闪卡，字幕已加下划线".into()))
                        },
                    );
                }
                Action::ToggleReasoning => self.chat.reasoning_open = !self.chat.reasoning_open,
                Action::ChatSession(session) => {
                    if let Some(cancel) = self.tasks.cancel.as_ref().filter(|_| self.chat.running) {
                        cancel.store(true, Ordering::SeqCst);
                    }
                    self.chat_requested_context.clear();
                    self.sync_chat(window, cx);
                    ensure!(
                        !self.chat.is_loading() && self.chat.context == self.chat_context_key(cx),
                        "请先恢复材料会话读取"
                    );
                    ensure!(!self.chat.is_loading(), "正在读取材料会话，请稍候");
                    let draft = self.value("question", cx);
                    self.chat.select(&session, &draft)?;
                    self.assistant.pending = None;
                    self.assistant.error.clear();
                    self.assistant.expanded_turn = None;
                    self.assistant.sessions_open = false;
                    if session.is_empty() {
                        self.assistant.references.clear();
                        self.persist_assistant_references()?;
                    }
                    self.inputs["question"].update(cx, |state, cx| state.focus(window, cx));
                }
                Action::PdfOpen(path) => {
                    ensure!(
                        self.tasks.cancel.is_none(),
                        "已有任务正在执行，请完成或取消后重试"
                    );
                    if ebook::is_ebook(&path) {
                        self.begin("正在读取电子书…", move |lib, cancel, _| {
                            Ok(Outcome::Ebook(ebook::open(&lib, &path, false, &cancel)?))
                        });
                    } else {
                        self.load_pdf(path, 0, self.pdf_width);
                    }
                }
                Action::ClearHighlights => {
                    self.audio.key_points.clear();
                    self.lib
                        .set_setting(&format!("key_points_{}", self.audio.current_id), "[]")?;
                    self.refresh();
                }
                Action::Highlight => {
                    ensure!(
                        !(self.page == "listening" && self.audio.blind.enabled),
                        "请先切回字幕模式"
                    );
                    let audio = self.audio.current_id.clone();
                    let cues = self.audio.cues.clone();
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
                    ensure!(
                        self.assistant.enabled
                            || !matches!(self.page.as_str(), "flashcard" | "discover")
                            || self.cards_ready(),
                        "正在读取学习卡片，请稍候"
                    );
                    ensure!(
                        self.tasks.cancel.is_none(),
                        "已有任务正在执行，请完成或取消后重试"
                    );
                    self.chat_requested_context.clear();
                    self.sync_chat(window, cx);
                    ensure!(
                        !self.chat.is_loading() && self.chat.context == self.chat_context_key(cx),
                        "请先恢复材料会话读取"
                    );
                    if self.chat.id.is_empty() {
                        self.chat.id = id();
                    }
                    let session_id = self.chat.id.clone();
                    let context = self.chat.context.clone();
                    let question = self.value("question", cx);
                    let references = self.assistant.references.clone();
                    clarora_gpui::assistant::validate(&references)?;
                    let global = self.assistant.enabled;
                    let passage = if global {
                        references
                            .iter()
                            .map(|r| r.text.clone())
                            .collect::<Vec<_>>()
                            .join("\n")
                    } else {
                        clarora_gpui::assistant::plain_reference(&self.reference(cx))
                    };
                    let video_material = if !global && self.page == "video" {
                        Some(self.video.material.clone().context(
                            if self.video.material_error.is_empty() {
                                "请先生成字幕并选择提问范围".to_string()
                            } else {
                                self.video.material_error.clone()
                            },
                        )?)
                    } else {
                        None
                    };
                    let history = self.chat.history.clone();
                    ensure!(!question.trim().is_empty(), "请输入问题");
                    self.chat.output.clear();
                    self.chat.reasoning.clear();
                    if global {
                        self.assistant.pending = Some((question.clone(), references.clone()));
                        self.assistant.error.clear();
                        self.assistant.scroll.scroll_to_bottom();
                    }
                    let request = self.chat.begin_request();
                    self.begin_scoped(if global {TaskScope::Application}else{TaskScope::Material},"AI 正在回答…", move |lib, cancel, tx| {
                        let mut notify=|chunk|{let _=tx.send(Event::Chunk(request.clone(), chunk));};
                        let video_material=video_material.map(|mut material|{material.text=clarora_gpui::assistant::plain_reference(&material.text);material});
                        let reply=if global {clarora_gpui::assistant::ask(&lib,&question,&references,&history,&cancel,&mut notify)?}else if let Some(material)=video_material {
                            video_qa::ask(&lib,&question,&material,&history,&cancel,&mut notify)?
                        }else{ai::chat_stream(&lib,&question,&passage,&history,&cancel,&mut notify)?};
                        ensure!(!cancel.load(Ordering::SeqCst), "已取消");
                        let mut next = history;
                        next.push(if global {clarora_gpui::assistant::user_turn(&question,&references)}else{json!({"role":"user","content":question,"reference":passage})});
                        next.push(json!({"role":"assistant","content":reply.answer,"reasoning_content":reply.reasoning}));
                        if next.len() > 60 {
                            next.drain(..next.len() - 60);
                        }
                        ai::save_chat(&lib, &session_id, &context, &question, &next)?;
                        Ok(Outcome::Output(reply.answer))
                    });
                }
                Action::SaveAnswer => {
                    ensure!(
                        !self.chat.is_loading() && self.chat.context == self.chat_context_key(cx),
                        "请先等待当前材料会话读取完成"
                    );
                    ensure!(self.tasks.cancel.is_none(), "请等待 AI 回答完成");
                    let answer = self
                        .chat
                        .history
                        .last()
                        .filter(|m| m["role"] == "assistant")
                        .and_then(|m| m["content"].as_str())
                        .context("请先完成一次 AI 回答")?;
                    ensure!(
                        !answer.trim().is_empty() && answer == self.chat.output,
                        "当前回答未完整保存，请完成回答后再存卡"
                    );
                    let question = self
                        .chat
                        .history
                        .iter()
                        .rev()
                        .find(|m| m["role"] == "user")
                        .and_then(|m| m["content"].as_str())
                        .context("会话缺少问题")?;
                    let reference = self
                        .chat
                        .history
                        .iter()
                        .rev()
                        .find(|m| m["role"] == "user")
                        .and_then(|m| m["reference"].as_str())
                        .map(str::to_owned)
                        .unwrap_or_else(|| {
                            self.chat
                                .history
                                .iter()
                                .rev()
                                .find(|m| m["role"] == "user")
                                .map(clarora_gpui::assistant::references)
                                .unwrap_or_default()
                                .iter()
                                .map(|r| r.text.clone())
                                .collect::<Vec<_>>()
                                .join("\n\n")
                        });
                    self.lib.put("ai_cards",json!({"id":id(),"question":question,"answer":answer,"context_text":reference,"created_at":now()}))?;
                    self.report_status(FeedbackKind::Info, "已存为 AI 问答卡".into());
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
                        self.loads.discover_favorites.insert(key.clone());
                        self.media.preloaded_index = None;
                        if card.id.starts_with("segment:") {
                            let count=self.lib.rows("SELECT id FROM clip_cards WHERE audio_uri=? AND start_ms=? AND end_ms=?",&[card.media.clone().into(),(card.start*1000.).into(),(card.end*1000.).into()])?;
                            if count.is_empty() {
                                self.lib.put("clip_cards",json!({"id":card.id,"en_text":card.front,"zh_text":card.back,"audio_uri":card.media,"start_ms":card.start*1000.,"end_ms":card.end*1000.,"created_at":now()}))?;
                            }
                        }
                        self.report_status(FeedbackKind::Info, "已收藏".into());
                    } else {
                        self.lib.delete("discover_favorites", &key)?;
                        self.loads.discover_favorites.remove(&key);
                        self.media.preloaded_index = None;
                        self.report_status(FeedbackKind::Info, "已取消收藏".into());
                    }
                }
                Action::VideoAsk(scope) => {
                    ensure!(self.page == "video", "请先进入视频学习");
                    ensure!(!self.chat.running, "请等待当前回答完成或取消后切换引用");
                    self.prepare_video_material(scope)?;
                    if scope != video_qa::Scope::All {
                        self.media.player.set("pause", "yes");
                    }
                    let material = self.video.material.as_ref().context("请先选取字幕")?;
                    let reference = clarora_gpui::assistant::Reference {
                        text: clarora_gpui::assistant::plain_reference(&material.text),
                    };
                    self.add_assistant_reference(reference, window, cx)?;
                    self.inputs["question"].update(cx, |state, cx| state.focus(window, cx));
                    self.report_status(
                        FeedbackKind::Info,
                        "已引用字幕；输入问题后发送给设置中的聊天模型".into(),
                    );
                }
                Action::VideoTranscribe => self.begin_background_subtitles(false)?,
                Action::VideoTranscribeAll => self.begin_background_subtitles(true)?,
                Action::VideoTranslateAll => self.begin_background_translation_all()?,
                Action::CancelVideoSubtitles => {
                    if let Some(job) = &self.tasks.video_job {
                        job.cancel.store(true, Ordering::SeqCst);
                    }
                    if let Some(n) = self.tasks.video_notice.as_mut().filter(|n| n.active()) {
                        n.state = TaskState::Cancelling;
                        n.detail = "正在停止后台字幕队列，已完成的字幕会保留".into();
                    }
                }
                Action::DismissVideoSubtitles => {
                    if self.tasks.video_job.is_none() {
                        self.tasks.video_notice = None;
                    }
                }
                Action::VideoTools => self.video.tools_open = !self.video.tools_open,
                Action::VideoSelect(key) => {
                    ensure!(
                        self.tasks.cancel.is_none(),
                        "请完成或取消当前任务后再切换视频"
                    );
                    if self.select_source_only(&key, true)? {
                        return Ok(());
                    }
                    let mut videos = video_library::Collection::load(&self.lib)?;
                    let video = videos.select(&key)?;
                    videos.save(&self.lib)?;
                    self.video.collection = videos;
                    self.save_media_position();
                    if !self.open(video.path) {
                        return Ok(());
                    }
                    self.video.tab = 3;
                    self.video.tools_open = false;
                    self.report_status(FeedbackKind::Info, format!("已选择：{}", video.title));
                }
                Action::VideoEpisodes(key) => {
                    ensure!(
                        self.tasks.cancel.is_none(),
                        "请完成或取消当前任务后再切换视频"
                    );
                    if self.select_source_only(&key, false)? {
                        return Ok(());
                    }
                    if self.video.collection.selected.as_ref() != Some(&key)
                        || self.video.path.is_none()
                    {
                        let mut videos = video_library::Collection::load(&self.lib)?;
                        let video = videos.select(&key)?;
                        videos.save(&self.lib)?;
                        self.save_media_position();
                        self.video.collection = videos;
                        if !self.open_at(video.path, 0., true) {
                            return Ok(());
                        }
                        self.media.player.command(["set", "pause", "yes"]);
                    }
                    self.video.tab = 1;
                    self.video.tools_open = false;
                }
                Action::VideoRemove(key) => {
                    ensure!(self.tasks.cancel.is_none(), "请等待当前任务完成");
                    let title = self
                        .video
                        .collection
                        .items
                        .iter()
                        .find(|v| v.id == key)
                        .context("该视频已移出收藏")?
                        .title
                        .clone();
                    self.video.remove = Some(key);
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
                Action::VideoConfirmRemove => {
                    ensure!(self.tasks.cancel.is_none(), "请等待当前任务完成");
                    let key = self
                        .video
                        .remove
                        .clone()
                        .context("请重新选择要移除的视频")?;
                    let mut videos = video_library::Collection::load(&self.lib)?;
                    if let Some(job) = self.tasks.video_job.as_ref().filter(|j| j.source_id == key)
                    {
                        job.cancel.store(true, Ordering::SeqCst);
                    }
                    let video = videos.remove(&key)?;
                    videos.save(&self.lib)?;
                    self.video.collection = videos;
                    self.video.remove = None;
                    if self
                        .video
                        .path
                        .as_ref()
                        .is_some_and(|path| video.contains(path))
                    {
                        self.leave_video_fullscreen(window, cx);
                        self.media.player.command(["stop"]);
                        self.media.snapshot = Playback::default();
                        self.media.path = None;
                        self.video.path = None;
                        self.video.series = None;
                        self.video.set_cues(vec![]);
                        self.refresh_saved_ranges();
                        self.video.active = None;
                        self.video.material = None;
                        self.video.material_error.clear();
                        self.media.loop_a = None;
                        self.media.loop_b = None;
                        self.media.repeat = 0;
                        self.media.end_handled = true;
                        self.output.clear();
                        self.chat.reasoning.clear();
                        self.lib.set_setting("bilibili_series", "")?;
                        self.lib.set_setting("last_video_position", "")?;
                    }
                    self.video.tab = 3;
                    self.report_status(
                        FeedbackKind::Info,
                        format!("已移除「{}」收藏，视频和字幕文件保留", video.title),
                    );
                }
                Action::VideoPanel(tab) => {
                    self.video.remove = None;
                    ensure!(tab < 4, "无效学习面板");
                    self.video.tab = tab;
                    if tab == 2 {
                        self.video.tab = 0;
                        self.open_assistant(window, cx);
                        self.inputs["question"].update(cx, |state, cx| state.focus(window, cx));
                    } else {
                        window.focus(&self.focus);
                    }
                }
                Action::VideoFollow => {
                    self.video.follow = !self.video.follow;
                    if self.video.follow {
                        if let Some(i) = self.video.active {
                            self.video.center.request(i);
                        }
                    }
                }
                Action::VideoTranscript => {
                    self.video.transcript_open = !self.video.transcript_open;
                }
                Action::VideoCopyTranscript => {
                    ensure!(
                        !self.video.transcript.text.is_empty(),
                        "没有可复制的英文文稿"
                    );
                    cx.write_to_clipboard(ClipboardItem::new_string(
                        self.video.transcript.text.clone(),
                    ));
                    self.report_status(
                        FeedbackKind::Info,
                        "已复制整篇英文文稿（不含时间戳与中文翻译）".into(),
                    );
                }
                Action::AudioCopySubtitle(audio_id, cue) => {
                    ensure!(
                        self.page == "listening"
                            && !audio_id.is_empty()
                            && audio_id == self.audio.current_id,
                        "音频已切换，请重新复制"
                    );
                    let body = blind_listening::copy_text(
                        &self.audio.cues,
                        cue.as_ref(),
                        &self.audio.blind,
                        self.audio.blind_chinese,
                    )
                    .context("暂无可复制的字幕；盲听时请先翻开对应句子")?;
                    cx.write_to_clipboard(ClipboardItem::new_string(body));
                    self.report_status(
                        FeedbackKind::Info,
                        if cue.is_some() {
                            "已复制这句字幕"
                        } else {
                            "已复制全部字幕（不含时间戳）"
                        }
                        .into(),
                    );
                }
                Action::VideoCopySubtitle(index) => {
                    let body = match index {
                        Some(index) => self
                            .video
                            .cues
                            .get(index)
                            .context("字幕已改变，请重试")?
                            .text
                            .clone(),
                        None => self
                            .video
                            .cues
                            .iter()
                            .map(|cue| cue.text.as_str())
                            .collect::<Vec<_>>()
                            .join("\n"),
                    };
                    ensure!(!body.trim().is_empty(), "没有可复制的字幕");
                    cx.write_to_clipboard(ClipboardItem::new_string(body));
                    self.report_status(
                        FeedbackKind::Info,
                        match index {
                            Some(index) => format!("已复制第 {} 句字幕", index + 1),
                            None => format!("已复制全部 {} 句字幕", self.video.cues.len()),
                        },
                    );
                }
                Action::VideoSubtitleExport => {
                    ensure!(!self.video.cues.is_empty(), "请先生成字幕");
                    let body = subtitles::srt(&self.video.cues);
                    let picker =
                        cx.prompt_for_new_path(&self.lib.root, Some("video-subtitles.srt"));
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
                Action::Transcribe => {
                    let row = self.selected().context("请先选择音频")?;
                    self.begin_subtitle(
                        "生成音频字幕",
                        "正在识别音频并保存字幕，长音频可能需要几分钟",
                        move |lib, cancel, _| {
                            let path = ai::write_transcription(
                                &lib,
                                &PathBuf::from(text(&row, "audio_uri")),
                                &cancel,
                            )?;
                            lib.db()?.execute(
                                "UPDATE listening_audios SET subtitle_uri=? WHERE id=?",
                                rusqlite::params![path, text(&row, "id")],
                            )?;
                            Ok(Outcome::Message("字幕已生成并载入".into()))
                        },
                    );
                }
                Action::Translate => {
                    let row = self.selected().context("请先选择音频")?;
                    let cues = self.audio.cues.clone();
                    ensure!(!cues.is_empty(), "请先导入或生成原文字幕");
                    self.begin_subtitle(
                        "翻译音频字幕",
                        "正在生成双语字幕…",
                        move |lib, cancel, _| {
                            let translated = ai::translate(&lib, &cues, &cancel)?;
                            let path = lib.root.join("media").join(format!("{}.srt", id()));
                            fs::write(&path, subtitles::srt(&translated))?;
                            lib.db()?.execute(
                                "UPDATE listening_audios SET subtitle_uri=? WHERE id=?",
                                rusqlite::params![path.to_string_lossy(), text(&row, "id")],
                            )?;
                            Ok(Outcome::Message("双语字幕已生成并载入".into()))
                        },
                    );
                }
                Action::Record => {
                    ensure!(self.recording.is_none(), "正在录音");
                    self.media.player.set("pause", "yes");
                    let path = self
                        .lib
                        .root
                        .join("recordings")
                        .join(format!("{}.wav", id()));
                    self.recording = Some(Recorder::start(path));
                    self.report_status(FeedbackKind::Info, "正在录音，完成后点击停止".into());
                }
                Action::StopRecord => {
                    let recorder = self.recording.take().context("没有正在进行的录音")?;
                    self.begin("正在保存录音…", move |_, _, _| {
                        Ok(Outcome::Recording(recorder.finish()?))
                    });
                }
                Action::Timer(seconds) => {
                    self.timer
                        .start(&self.lib, focus_timer::now_ms(), seconds)?;
                    self.timer_remaining = self.timer.remaining_seconds(focus_timer::now_ms());
                    self.timer_retry = None;
                    self.play_timer_chime(timer_chimes::Cue::Start);
                }
                Action::TimerPause => {
                    match self.timer.toggle(&self.lib, focus_timer::now_ms())? {
                        focus_timer::Toggle::Started => {
                            self.play_timer_chime(timer_chimes::Cue::Start)
                        }
                        focus_timer::Toggle::Completed(focused) => {
                            self.focus_phase_completed(focused)
                        }
                        focus_timer::Toggle::Paused | focus_timer::Toggle::Resumed => {}
                    }
                    self.timer_remaining = self.timer.remaining_seconds(focus_timer::now_ms());
                    self.timer_retry = None;
                }
                Action::TimerReset => {
                    self.timer.reset(&self.lib)?;
                    self.timer_chimes.stop();
                    self.timer_remaining = self.timer.remaining_seconds(focus_timer::now_ms());
                    self.timer_retry = None;
                }
                Action::CredentialMode(enabled) => {
                    ensure!(
                        self.tasks.cancel.is_none() && self.tasks.video_job.is_none(),
                        "请先完成或取消当前任务再切换密钥存储模式"
                    );
                    self.begin_scoped(
                        TaskScope::Application,
                        "正在切换密钥存储模式…",
                        move |lib, _, _| {
                            credentials::set_secure_mode(&lib, enabled)?;
                            Ok(Outcome::CredentialMode(enabled))
                        },
                    );
                }
                Action::ImportLegacyCredentials => {
                    ensure!(
                        self.tasks.cancel.is_none() && self.tasks.video_job.is_none(),
                        "请先完成或取消当前任务"
                    );
                    self.begin_scoped(
                        TaskScope::Application,
                        "正在导入旧钥匙串密钥，系统可能要求一次授权…",
                        move |lib, _, _| {
                            let count = credentials::import_legacy(&lib)?;
                            Ok(Outcome::Message(format!(
                                "已导入 {count} 项密钥，以后从本机 SQLite 读取"
                            )))
                        },
                    );
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
                    self.import_command = self.value("command", cx);
                    self.begin("正在保存 AI 设置…", move |lib, _, _| {
                        ai::save_preferences(&lib, config, &key)?;
                        let location = if credentials::secure_mode(&lib)? {
                            "系统凭证库"
                        } else {
                            "本机 SQLite"
                        };
                        Ok(Outcome::Message(format!(
                            "AI 设置已保存，聊天与远端转写共用配置，密钥保存在{location}"
                        )))
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
                        Ok(Outcome::Message(format!(
                            "对象存储设置已保存，密钥保存在{}",
                            if credentials::secure_mode(&lib)? {
                                "系统凭证库"
                            } else {
                                "本机 SQLite"
                            }
                        )))
                    });
                }
                Action::SelectModel(model) => {
                    let name = model.clone();
                    self.set("local_model", model, window, cx);
                    self.set("engine", "local", window, cx);
                    self.report_status(
                        FeedbackKind::Info,
                        format!("已选择 {name} · 待保存；点击「保存 AI 与学习设置」后生效"),
                    );
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
                Action::CloudList => {
                    self.begin("正在读取备份版本…", move |lib, cancel, _| {
                        Ok(Outcome::Backups(
                            backup::Storage::load(&lib)?.list_cancellable(&cancel)?,
                        ))
                    })
                }
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
                        let count = backup::Storage::load(&lib)?.restore(&lib, &key, &cancel)?;
                        let videos = video_library::Collection::load(&lib)
                            .context("资料已恢复，但重新载入视频库失败，请刷新")?;
                        Ok(Outcome::LibraryRestored(count, videos))
                    })
                }
                Action::DocumentCopy => {
                    cx.write_to_clipboard(ClipboardItem::new_string(self.document_text.clone()));
                    self.report_status(FeedbackKind::Info, "已复制当前阅读文字".into());
                }
                Action::PdfPage(delta) => {
                    ensure!(self.tasks.cancel.is_none(), "文档仍在载入，请稍后翻页");
                    if let Some(reading) = &self.ebook_reading {
                        let page = (reading.page as i64 + i64::from(delta))
                            .clamp(0, reading.book.pages.len() as i64 - 1)
                            as usize;
                        let book = reading.book.clone();
                        let font = reading.font_size;
                        self.begin("正在翻页…", move |_, cancel, _| {
                            let mut reading = ebook::Reading::load(book, page, &cancel)?;
                            reading.font_size = font;
                            Ok(Outcome::Ebook(reading))
                        });
                    } else {
                        let page = (self.pdf_page as i32 + delta)
                            .clamp(0, self.pdf_pages.saturating_sub(1) as i32)
                            as u16;
                        self.load_pdf(
                            self.pdf_path.clone().context("请先导入文档")?,
                            page,
                            self.pdf_width,
                        );
                    }
                }
                Action::PdfZoom(delta) => {
                    ensure!(self.tasks.cancel.is_none(), "文档仍在载入，请稍后缩放");
                    if let Some(reading) = self.ebook_reading.as_mut() {
                        reading.font_size =
                            (i32::from(reading.font_size) + delta.signum() * 2).clamp(14, 32) as u8;
                        ebook::save_progress(&self.lib, reading)?;
                    } else {
                        let width = (self.pdf_width + delta).clamp(300, 2400);
                        self.load_pdf(
                            self.pdf_path.clone().context("请先导入文档")?,
                            self.pdf_page,
                            width,
                        );
                    }
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
                        self.audio.audios.iter().any(|r| text(r, "id") == key
                            && text(r, "practice_id") == self.audio.practice_id),
                        "音频不属于当前组"
                    );
                    self.audio.library.pending_delete.clear();
                    if !self.audio.checked.insert(key.clone()) {
                        self.audio.checked.remove(&key);
                    }
                }
                Action::Suspend(suspend) => {
                    let card = self.current_card().context("当前没有卡片")?;
                    self.lib.db()?.execute("INSERT INTO review_schedule(card_kind,card_id,suspended) VALUES(?,?,?) ON CONFLICT(card_kind,card_id) DO UPDATE SET suspended=excluded.suspended",rusqlite::params![card.kind,card.id,i32::from(suspend)])?;
                    self.refresh();
                    self.report_status(
                        FeedbackKind::Info,
                        if suspend {
                            "已暂停此卡复习"
                        } else {
                            "已恢复此卡复习"
                        }
                        .into(),
                    );
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
                    self.audio.library.filter = 0;
                    self.audio.library.sort = 0;
                    self.set("library_search", "", window, cx);
                    ensure!(!body.trim().is_empty(), "请先输入要合成的文字");
                    ensure!(!self.audio.practice_id.is_empty(), "请先新建或选择练习组");
                    let mut name = self.value("tts_name", cx);
                    if name.trim().is_empty() {
                        name =
                            format!("Kokoro · {}", chrono::Local::now().format("%m-%d %H:%M:%S"));
                    }
                    let group = self.audio.practice_id.clone();
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
                    self.audio.blind.enabled = false;
                    self.page_scroll.set_offset(point(px(0.), px(0.)));
                    self.handle(Action::Audio(text(&row, "id")), window, cx);
                }
                Action::MergeAudios => {
                    let rows = self
                        .audio
                        .audios
                        .iter()
                        .filter(|r| {
                            text(r, "practice_id") == self.audio.practice_id
                                && self.audio.checked.contains(&text(r, "id"))
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    ensure!(rows.len() >= 2, "请至少勾选两条音频");
                    let group = self.audio.practice_id.clone();
                    self.begin("正在合并音频与字幕…", move |lib, cancel, _| {
                        clarora_gpui::audio_tools::merge(&lib, &rows, &group, &cancel)?;
                        Ok(Outcome::Message("已合并音频与字幕，原文件保留".into()))
                    });
                }
                Action::AudioGroupSubtitles(mode) => {
                    ensure!(self.tasks.cancel.is_none(), "请完成或取消当前任务后重试");
                    let plan = clarora_gpui::audio_batch::Plan::new(
                        &self.lib,
                        &self.audio.practice_id,
                        mode,
                    )?;
                    let title = format!("{} · {}", mode.label(), plan.group_name);
                    self.begin_scoped(TaskScope::Application, &title, move |lib, cancel, tx| {
                        Ok(Outcome::AudioBatch(plan.run(&lib, &cancel, |stage| {
                            let _ = tx.send(Event::SubtitleProgress(stage.into()));
                        })))
                    });
                    self.tasks.notice = Some(TaskNotice::new(
                        &title,
                        "逐条处理全组；已有字幕自动跳过，可随时取消",
                    ));
                }
                Action::CommandImport => {
                    let command = self.import_command.clone();
                    ensure!(
                        self.tasks.cancel.is_none(),
                        "已有任务正在执行，请完成或取消后重试"
                    );
                    ensure!(
                        !self
                            .floating_name_window
                            .is_some_and(|handle| cx.windows().contains(&handle)),
                        "请先保存音频名称，或关闭命名面板后继续导入"
                    );
                    let request = clarora_gpui::command_import::Request::new(
                        &self.lib,
                        &self.audio.practice_id,
                        &command,
                    )?;
                    let label = format!("正在导入至「{}」…", request.group_name());
                    self.command_imported = None;
                    self.begin_scoped(TaskScope::Application, &label, move |lib, cancel, _| {
                        Ok(Outcome::CommandImported(request.run(&lib, &cancel)?))
                    });
                    self.tasks.notice = Some(TaskNotice::new("命令导入音频", &label));
                }
                Action::FloatingImport => self.toggle_floating_import(window, cx),
                Action::NameImportedAudio => self.show_floating_names(window, cx)?,
                Action::CloseFloatingNames => self.close_floating_names(cx),
                Action::ConfigureCommandImport => {
                    self.handle(Action::Page("settings".into()), window, cx);
                    if self.page == "settings" {
                        self.inputs["command"].update(cx, |state, cx| state.focus(window, cx));
                    }
                }
                Action::SaveCommandImport => {
                    let command = self.value("command", cx);
                    self.lib.set_setting("command", &command)?;
                    self.import_command = command;
                    self.report_status(FeedbackKind::Info, "本机导入命令已保存".into());
                }
            }
            Ok(())
        })();
        if let Err(e) = result {
            self.report_status(FeedbackKind::Error, format!("{e:#}"));
            self.feedback.attach_retry(action_retry);
        }
        if updates_settings {
            self.refresh_view_data();
        }
        if restore_video_focus && !self.subtitle_text.borrow().focus.is_focused(window) {
            // Buttons deliberately keep the old focus on mouse-down. Explicit
            // video controls hand it back without disturbing subtitle copying.
            window.focus(&self.focus);
        }
        if refresh_chat {
            if let Err(error) = self.chat.refresh() {
                self.report_status(FeedbackKind::Error, format!("刷新会话失败：{error:#}"));
            }
        }
        self.sync_assistant(window, cx);
        cx.notify();
    }
}
