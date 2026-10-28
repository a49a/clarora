//! Foreground work dispatch, separate from page rendering.
use super::*;

impl AppView {
    pub(super) fn begin(
        &mut self,
        label: &str,
        task: impl FnOnce(Library, Cancel, mpsc::Sender<Event>) -> Result<Outcome> + Send + 'static,
    ) {
        self.begin_scoped(TaskScope::Material, label, task);
    }
    pub(super) fn begin_scoped(
        &mut self,
        scope: TaskScope,
        label: &str,
        task: impl FnOnce(Library, Cancel, mpsc::Sender<Event>) -> Result<Outcome> + Send + 'static,
    ) {
        if self.tasks.cancel.is_some() {
            self.report_status(
                FeedbackKind::Error,
                "已有任务正在执行，请完成或取消后重试".into(),
            );
            return;
        }
        self.tasks.notice = None;
        self.tasks.model_progress = None;
        self.tasks.backup_progress = None;
        self.tasks.scope = scope;
        self.tasks.started = Some(Instant::now());
        self.tasks.progress_elapsed = 0;
        self.tasks.retry = self.feedback.retry_candidate.clone();
        let cancel = process::cancel_token();
        let token = cancel.clone();
        let lib = self.lib.clone();
        let (tx, rx) = mpsc::channel();
        self.tasks.events = Some(rx);
        self.tasks.cancel = Some(cancel);
        self.report_status(FeedbackKind::Progress, label.into());
        let chat_request = self.chat.current_request();
        std::thread::spawn(move || {
            let result = task(lib, token, tx.clone()).map_err(|e| format!("{e:#}"));
            let _ = tx.send(Event::Done(chat_request, result));
        });
    }
    pub(super) fn begin_subtitle(
        &mut self,
        title: &str,
        detail: &str,
        task: impl FnOnce(Library, Cancel, mpsc::Sender<Event>) -> Result<Outcome> + Send + 'static,
    ) {
        if self.tasks.cancel.is_some() {
            self.report_status(
                FeedbackKind::Error,
                "已有任务正在执行，请完成或取消后重试".into(),
            );
            return;
        }
        self.begin(&format!("正在{title}…"), task);
        self.tasks.notice = Some(TaskNotice::new(title, detail));
    }
    pub(super) fn poll_tasks(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let mut changed = false;
        if let Some(notice) = &mut self.tasks.notice {
            if notice.state == TaskState::Running
                && self
                    .tasks
                    .cancel
                    .as_ref()
                    .is_some_and(|t| t.load(Ordering::SeqCst))
            {
                notice.state = TaskState::Cancelling;
                notice.detail = "正在停止当前处理，请稍候…".into();
                changed = true;
            }
            if notice.active() && notice.elapsed != notice.started.elapsed().as_secs() {
                notice.elapsed = notice.started.elapsed().as_secs();
                changed = true;
            }
        }
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
        if let Some(rx) = &self.tasks.events {
            let events = rx.try_iter().collect::<Vec<_>>();
            for event in events {
                changed = true;
                match event {
                    Event::SubtitleProgress(stage) => {
                        if let Some(notice) = self
                            .tasks
                            .notice
                            .as_mut()
                            .filter(|n| n.state == TaskState::Running)
                        {
                            notice.detail = stage;
                        }
                    }
                    Event::ModelProgress(progress) => {
                        if self
                            .tasks
                            .cancel
                            .as_ref()
                            .is_some_and(|t| !t.load(Ordering::SeqCst))
                        {
                            self.report_status(FeedbackKind::Progress, progress.label());
                            self.tasks.model_progress = Some(progress);
                        }
                    }
                    Event::BackupProgress(progress) => {
                        if self.tasks.cancel.as_ref().is_some_and(|t| {
                            !t.load(Ordering::SeqCst)
                                || progress.stage == backup::UploadStage::Completed
                        }) {
                            let label = progress.label();
                            self.report_status(FeedbackKind::Progress, label.clone());
                            if let Some(notice) = self
                                .tasks
                                .notice
                                .as_mut()
                                .filter(|n| n.state == TaskState::Running)
                            {
                                notice.detail = label;
                            }
                            self.tasks.backup_progress = Some(progress);
                        }
                    }
                    Event::Chunk(request, chunk) => {
                        if !self.chat.accepts(&request) {
                            continue;
                        }
                        if self
                            .tasks
                            .cancel
                            .as_ref()
                            .is_some_and(|cancel| !cancel.load(Ordering::SeqCst))
                        {
                            self.chat.output = chunk.answer;
                            self.chat.reasoning = chunk.reasoning;
                        }
                    }
                    Event::Done(request, result) => {
                        let task_retry = self.tasks.retry.take();
                        let failed = result.is_err()
                            || matches!(&result, Ok(Outcome::Issue(_)))
                            || matches!(&result, Ok(Outcome::BackupFailed(_)))
                            || matches!(&result, Ok(Outcome::AudioBatch(report)) if !report.failed.is_empty());
                        let completed_backup = self.tasks.backup_progress.is_some();
                        let completed_chat = request.is_some();
                        let accepted = request.as_ref().is_none_or(|r| self.chat.finish_request(r));
                        let mut cancelled = self
                            .tasks
                            .cancel
                            .as_ref()
                            .is_some_and(|t| t.load(Ordering::SeqCst));
                        self.tasks.cancel = None;
                        self.tasks.scope = TaskScope::Material;
                        self.tasks.model_progress = None;
                        if !accepted {
                            // The old request may have committed before its UI callback
                            // was invalidated. Refresh the current index without painting
                            // the old request's output or selecting its conversation.
                            if let Err(error) = self.chat.refresh() {
                                self.report_status(
                                    FeedbackKind::Error,
                                    format!("刷新会话失败：{error:#}"),
                                );
                            }
                            changed = true;
                            continue;
                        }
                        if completed_chat && self.assistant.enabled {
                            if cancelled || result.is_err() {
                                self.assistant.failed_output = self.chat.output.clone();
                            }
                            self.assistant.error = if cancelled {
                                "已停止回答。问题和引用已保留，可再次发送。".into()
                            } else if let Err(error) = &result {
                                format!("回答未完成：{error:#}。问题和引用已保留。")
                            } else {
                                String::new()
                            };
                        }
                        match result {
                            Ok(Outcome::AudioBatch(report)) => {
                                cancelled = report.cancelled;
                                self.report_status(
                                    if report.failed.is_empty() {
                                        FeedbackKind::Info
                                    } else {
                                        FeedbackKind::Error
                                    },
                                    report.message(),
                                );
                            }
                            Ok(Outcome::CommandImported(report)) if !cancelled => {
                                self.report_status(FeedbackKind::Info, report.message());
                                self.command_imported = Some(report);
                                if let Err(error) = self.show_floating_names(window, cx) {
                                    self.report_status(
                                        FeedbackKind::Error,
                                        format!("音频已导入，打开命名面板失败：{error:#}"),
                                    );
                                }
                            }
                            Ok(Outcome::TtsReady(row)) => {
                                self.report_status(
                                    FeedbackKind::Info,
                                    "音频及逐句字幕已保存到生成时选择的练习组，点击试听查看".into(),
                                );
                                if text(&row, "practice_id") == self.audio.practice_id {
                                    self.audio.library.selected = text(&row, "id");
                                }
                                self.tts_last = Some(row);
                            }
                            Ok(Outcome::Parts(series)) if !cancelled => {
                                self.report_status(
                                    FeedbackKind::Info,
                                    format!("已识别 {} P，点击列表选集播放", series.parts.len()),
                                );
                                if let Err(e) = series.save(&self.lib) {
                                    self.report_status(
                                        FeedbackKind::Error,
                                        format!("保存分P列表失败：{e}"),
                                    );
                                }
                                self.video.parts_scroll.scroll_to_item(
                                    series.current.unwrap_or(series.requested),
                                    ScrollStrategy::Center,
                                );
                                self.video.series = Some(series);
                                if self.page == "video" {
                                    self.video.tab = 1;
                                }
                            }
                            Ok(Outcome::PartVideo(
                                mut series,
                                index,
                                path,
                                new_file,
                                origin,
                                from_end,
                            )) => {
                                if cancelled {
                                    if new_file {
                                        if let Some(parent) = path.parent().filter(|p| {
                                            p.starts_with(self.lib.root.join("downloads"))
                                        }) {
                                            let _ = fs::remove_dir_all(parent);
                                        }
                                    }
                                    self.report_status(FeedbackKind::Info, "已取消分P下载".into());
                                } else {
                                    series.parts[index].path = Some(path.clone());
                                    let activate =
                                        self.page == "video" && self.media.path == origin;
                                    if activate {
                                        series.current = Some(index);
                                    }
                                    let saved = (|| -> Result<()> {
                                        let mut videos =
                                            video_library::Collection::load(&self.lib)?;
                                        videos.add(&[path.clone()], Some(&series))?;
                                        videos.save(&self.lib)?;
                                        self.video.collection = videos;
                                        Ok(())
                                    })();
                                    if activate && saved.is_ok() {
                                        self.video.series = Some(series);
                                        // Opening media reloads subtitles; keep the panel the user is viewing.
                                        let panel = self.video.tab;
                                        if self.open_part_video(path, from_end) {
                                            self.video.tab = panel;
                                            if !self.video.repeat_three && !from_end {
                                                self.media.player.set("pause", "no");
                                            }
                                            self.report_status(
                                                FeedbackKind::Info,
                                                if from_end {
                                                    format!("正在定位 P{} 的最后一句", index + 1)
                                                } else if self.video.repeat_three {
                                                    format!(
                                                        "正在准备 P{} 的每句三次播放",
                                                        index + 1
                                                    )
                                                } else {
                                                    format!("正在播放 P{}", index + 1)
                                                },
                                            );
                                        }
                                    } else {
                                        self.report_status(
                                            FeedbackKind::Info,
                                            "分P已下载，返回视频页选集播放".into(),
                                        );
                                    }
                                    if let Err(e) = saved {
                                        self.report_status(
                                            FeedbackKind::Error,
                                            format!("分P已导入，但保存列表失败：{e}"),
                                        );
                                    }
                                }
                            }
                            Ok(Outcome::VideoTranslated(video, path, source, cues))
                                if !cancelled =>
                            {
                                let same_source = self.video.cues.len() == source.len()
                                    && self.video.cues.iter().zip(&source).all(|(a, b)| {
                                        a.start == b.start && a.end == b.end && a.text == b.text
                                    });
                                if self.page == "video"
                                    && self.media.path.as_ref() == Some(&video)
                                    && same_source
                                {
                                    self.video.set_cues(cues);
                                    self.refresh_saved_ranges();
                                    self.video.active = None;
                                    self.video.caption_source = VideoCaptionSource::Transcript;
                                    self.video.material = None;
                                    self.video.material_error.clear();
                                    self.media.player.command([
                                        "sub-add".into(),
                                        path,
                                        "select".into(),
                                    ]);
                                    self.report_status(
                                        FeedbackKind::Info,
                                        "中文字幕已生成并保存，英文和时间轴已保留".into(),
                                    );
                                } else {
                                    self.report_status(
                                        FeedbackKind::Info,
                                        "翻译已完成，当前材料已改变，未切换显示".into(),
                                    );
                                }
                            }
                            Ok(Outcome::Output(s)) if !cancelled => {
                                if completed_chat {
                                    self.chat.output = s;
                                } else {
                                    self.output = s;
                                }
                                self.report_status(FeedbackKind::Info, "已完成".into());
                            }
                            Ok(Outcome::KeyPoints(audio, points)) if !cancelled => {
                                let _ = self.lib.set_setting(
                                    &format!("key_points_{audio}"),
                                    &serde_json::to_string(&points).unwrap_or_default(),
                                );
                                if self.audio.current_id == audio {
                                    self.audio.key_points = points;
                                }
                                self.report_status(
                                    FeedbackKind::Info,
                                    "学习重点已标注，可悬停查看释义".into(),
                                );
                            }
                            Ok(Outcome::Ocr(text)) if !cancelled => {
                                self.ocr_text = text.clone();
                                self.output = text;
                                self.report_status(FeedbackKind::Info, "图片文字已识别".into());
                            }
                            Ok(Outcome::Ebook(mut reading)) => {
                                if cancelled {
                                    drop(reading);
                                    self.report_status(
                                        FeedbackKind::Info,
                                        "已取消，保留原阅读内容".into(),
                                    );
                                } else {
                                    reading.accept_import();
                                    self.document_text = reading.text();
                                    self.pdf_path = Some(reading.book.path.clone());
                                    self.pdf_image = None;
                                    self.pdf_bookmarks.clear();
                                    if let Err(error) = ebook::save_progress(&self.lib, &reading) {
                                        self.report_status(
                                            FeedbackKind::Error,
                                            format!("已载入，但进度保存失败：{error:#}"),
                                        );
                                    } else {
                                        self.report_status(
                                            FeedbackKind::Info,
                                            format!("已载入《{}》", reading.book.title),
                                        );
                                    }
                                    let _ = self.lib.set_setting("pdf_current", &json!({"path":reading.book.path,"page":reading.page,"width":self.pdf_width}).to_string());
                                    self.ebook_reading = Some(reading);
                                    self.page_scroll.set_offset(point(px(0.), px(0.)));
                                }
                            }
                            Ok(Outcome::Pdf(path, width, page)) if !cancelled => {
                                self.pdf_path = Some(path);
                                self.pdf_width = width;
                                self.ebook_reading = None;
                                self.document_text = page.text;
                                self.pdf_image = Some(page.image);
                                self.pdf_page = page.page;
                                self.pdf_pages = page.count;
                                self.pdf_bookmarks = page.bookmarks;
                                let _ = self.lib.set_setting(
                                    "pdf_current",
                                    &json!({"path":self.pdf_path,"page":self.pdf_page,"width":self.pdf_width}).to_string(),
                                );
                                self.report_status(FeedbackKind::Info, "PDF 已载入".into());
                            }
                            Ok(Outcome::Backups(v)) => {
                                let message = if v.is_empty() {
                                    "连接成功，当前资料库前缀下暂无备份。点击「备份本机资料」可创建备份。".into()
                                } else {
                                    format!(
                                        "已读取 {} 个备份版本，可在「同步与备份」中查看",
                                        v.len()
                                    )
                                };
                                self.backups = v;
                                self.report_status(FeedbackKind::Info, message);
                            }
                            Ok(Outcome::BackupCreated(key)) => {
                                // A confirmed remote commit wins over a simultaneous cancel.
                                cancelled = false;
                                if !self.backups.contains(&key) {
                                    self.backups.insert(0, key);
                                }
                                if let Some(progress) = &mut self.tasks.backup_progress {
                                    progress.stage = backup::UploadStage::Completed;
                                    progress.bytes_sent = progress.bytes_total;
                                    progress.files_done = progress.files_total;
                                    progress.current_name.clear();
                                }
                                let detail = self
                                    .tasks
                                    .backup_progress
                                    .as_ref()
                                    .map(backup::UploadProgress::label)
                                    .unwrap_or_else(|| "云备份完成".into());
                                self.report_status(FeedbackKind::Info, detail);
                            }
                            Ok(Outcome::BackupFailed(message)) => {
                                // Preserve an uncertain manifest publication or HTTP error,
                                // even if cancellation was requested at the same time.
                                cancelled = message == "已取消";
                                self.report_status(
                                    if cancelled {
                                        FeedbackKind::Info
                                    } else {
                                        FeedbackKind::Error
                                    },
                                    if cancelled {
                                        "云备份已取消".into()
                                    } else {
                                        message
                                    },
                                );
                            }
                            Ok(Outcome::Recording(path)) => {
                                self.recorded = Some(path);
                                self.report_status(
                                    FeedbackKind::Info,
                                    "录音已保存，可转写评分".into(),
                                );
                            }
                            Ok(Outcome::CredentialMode(enabled)) => {
                                self.settings.secure_credentials = enabled;
                                self.report_status(FeedbackKind::Info, if enabled {"密钥安全模式已开启，现有本机密钥已迁入系统凭证库"} else {"密钥安全模式已关闭。已有钥匙串密钥可重新输入保存，或主动导入一次"}.into());
                            }
                            Ok(Outcome::LibraryRestored(_count, videos)) => {
                                if self.video.path.as_ref().is_none_or(|path| !path.is_file()) {
                                    if let Some(video) =
                                        videos.active().filter(|v| v.path.is_file())
                                    {
                                        self.video.path = Some(video.path.clone());
                                        self.video.series = video.series.clone();
                                    }
                                }
                                self.video.collection = videos;
                                if self.pdf_path.as_ref().is_none_or(|path| !path.is_file()) {
                                    if let Ok(saved) = self.lib.setting("pdf_current") {
                                        if let Ok(value) = serde_json::from_str::<Value>(&saved) {
                                            let path =
                                                PathBuf::from(value["path"].as_str().unwrap_or(""));
                                            if path.is_file() && ebook::supported(&path) {
                                                self.pdf_path = Some(path);
                                                self.pdf_page = value["page"]
                                                    .as_u64()
                                                    .unwrap_or(0)
                                                    .min(u16::MAX as u64)
                                                    as u16;
                                                self.pdf_width = value["width"]
                                                    .as_i64()
                                                    .unwrap_or(1000)
                                                    .clamp(300, 3000)
                                                    as i32;
                                                self.pdf_image = None;
                                                self.ebook_reading = None;
                                                self.document_text.clear();
                                            }
                                        }
                                    }
                                }
                                self.report_status(
                                    FeedbackKind::Info,
                                    "资料、附件与学习进度已恢复".into(),
                                );
                            }
                            Ok(Outcome::Message(s)) => self.report_status(FeedbackKind::Info, s),
                            Ok(Outcome::Issue(s)) => self.report_status(FeedbackKind::Error, s),
                            Err(_) if cancelled => {
                                self.report_status(FeedbackKind::Info, "已取消当前任务".into());
                            }
                            Err(e) => {
                                self.report_status(FeedbackKind::Error, e);
                                self.feedback.attach_retry(task_retry);
                            }
                            _ => self.report_status(FeedbackKind::Info, "已取消".into()),
                        }
                        if let Some(notice) = self.tasks.notice.as_mut().filter(|n| n.active()) {
                            notice.state = if cancelled {
                                TaskState::Cancelled
                            } else if failed {
                                TaskState::Failed
                            } else {
                                TaskState::Succeeded
                            };
                            notice.elapsed = notice.started.elapsed().as_secs();
                            notice.detail = self.status.clone();
                        }
                        // Chat only updates its persisted conversation. Refreshing materials
                        // here would invalidate the revealed cue and switch its chat context.
                        if !completed_chat && !completed_backup {
                            self.refresh();
                        }
                        if let Err(error) = if completed_backup {
                            Ok(())
                        } else {
                            self.chat.refresh()
                        } {
                            self.report_status(
                                FeedbackKind::Error,
                                format!("刷新会话失败：{error:#}"),
                            );
                        }
                    }
                }
            }
        }
        changed
    }
}
