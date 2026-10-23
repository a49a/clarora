use super::*;
impl AppView {
    pub(super) fn begin_background_subtitles(&mut self, all: bool) -> Result<()> {
        ensure!(
            self.tasks.video_job.is_none(),
            "已有后台视频字幕任务，可先取消或等待完成"
        );
        ensure!(self.tasks.cancel.is_none(), "请先完成当前前台任务");
        ensure!(self.page == "video", "请先进入视频学习");
        let path = self.video.path.clone().context("请先选择视频")?;
        let videos = video_library::Collection::load(&self.lib)?;
        let source = videos
            .items
            .iter()
            .find(|v| v.contains(&path))
            .context("请先导入视频")?;
        let (plan, title) = if all {
            let stored = source.series.as_ref().context("当前视频没有分P选集")?;
            let series = self
                .video
                .series
                .as_ref()
                .filter(|s| s.base == stored.base)
                .unwrap_or(stored)
                .clone();
            let title = format!("生成全部 {} P 字幕 · {}", series.parts.len(), source.title);
            (video_jobs::Plan::all(source.id.clone(), series), title)
        } else {
            (
                video_jobs::Plan::current(source.id.clone(), path),
                "生成当前视频字幕".into(),
            )
        };
        self.tasks.video_error.clear();
        self.tasks.video_notice = Some(TaskNotice::new(
            &title,
            "后台队列已开始，可继续切换分P和播放",
        ));
        self.tasks.video_job = Some(video_jobs::Job::start(
            self.lib.clone(),
            plan,
            self.tasks.downloads.clone(),
        ));
        Ok(())
    }
    pub(super) fn poll_background_subtitles(&mut self) -> bool {
        let Some(job) = &self.tasks.video_job else {
            return false;
        };
        let cancelling = job.cancel.load(Ordering::SeqCst);
        let mut events = vec![];
        let disconnected = loop {
            match job.events.try_recv() {
                Ok(event) => events.push(event),
                Err(mpsc::TryRecvError::Empty) => break false,
                Err(mpsc::TryRecvError::Disconnected) => break true,
            }
        };
        let mut changed = !events.is_empty();
        if let Some(n) = &mut self.tasks.video_notice {
            if cancelling && n.state == TaskState::Running {
                n.state = TaskState::Cancelling;
                n.detail = "正在停止后台字幕队列，已完成的字幕会保留".into();
                changed = true;
            }
            if n.active() && n.elapsed != n.started.elapsed().as_secs() {
                n.elapsed = n.started.elapsed().as_secs();
                changed = true;
            }
        }
        for event in events {
            match event {
                video_jobs::Event::Progress(detail) => {
                    if let Some(n) = self
                        .tasks
                        .video_notice
                        .as_mut()
                        .filter(|n| n.state == TaskState::Running)
                    {
                        n.detail = detail;
                    }
                }
                video_jobs::Event::Downloaded {
                    source_id,
                    index,
                    path,
                } => {
                    let saved = (|| -> Result<()> {
                        let mut videos = video_library::Collection::load(&self.lib)?;
                        let Some(base) = videos.cache_part(&source_id, index, &path)? else {
                            return Ok(());
                        };
                        videos.save(&self.lib)?;
                        self.video.collection = videos;
                        if let Some(active) = self.video.series.as_mut().filter(|s| s.base == base)
                        {
                            if let Some(part) = active.parts.get_mut(index) {
                                part.path = Some(path);
                            }
                            active.save(&self.lib)?;
                        }
                        Ok(())
                    })();
                    if let Err(e) = saved {
                        self.tasks.video_error = format!("保存分P下载记录失败：{e:#}");
                    }
                }
                video_jobs::Event::Ready {
                    source_id,
                    path,
                    subtitle,
                    cues,
                } => {
                    if self.page == "video"
                        && self.video.path.as_ref() == Some(&path)
                        && self.media.path.as_ref() == Some(&path)
                        && self
                            .video
                            .collection
                            .items
                            .iter()
                            .any(|v| v.id == source_id)
                    {
                        self.video.set_cues(cues);
                        self.video.active = None;
                        self.video.caption_source = VideoCaptionSource::Transcript;
                        self.video.material = None;
                        self.video.material_error.clear();
                        self.media
                            .player
                            .command(["sub-add".into(), subtitle, "select".into()]);
                    }
                }
                video_jobs::Event::Finished(report) => {
                    if let Some(n) = &mut self.tasks.video_notice {
                        n.elapsed = n.started.elapsed().as_secs();
                        n.state = if report.cancelled {
                            TaskState::Cancelled
                        } else if !report.failed.is_empty() || !self.tasks.video_error.is_empty() {
                            TaskState::Failed
                        } else {
                            TaskState::Succeeded
                        };
                        n.detail = format!(
                            "共 {} 项 · 已生成 {} · 已有字幕跳过 {} · 失败 {}。已完成字幕已保存，可再次生成以处理未完成的P。",
                            report.total,
                            report.generated,
                            report.skipped,
                            report.failed.len()
                        );
                        for (label, error) in report.failed {
                            n.detail.push_str(&format!("\n{label}：{error}"));
                        }
                        if !self.tasks.video_error.is_empty() {
                            n.detail.push_str(&format!("\n{}", self.tasks.video_error));
                        }
                    }
                    self.tasks.video_job = None;
                }
            }
        }
        if disconnected && self.tasks.video_job.is_some() {
            self.tasks.video_job = None;
            if let Some(n) = &mut self.tasks.video_notice {
                n.state = TaskState::Failed;
                n.detail = "后台字幕任务意外结束，已保存的字幕保留，请重试".into();
            }
            changed = true;
        }
        changed
    }
}
