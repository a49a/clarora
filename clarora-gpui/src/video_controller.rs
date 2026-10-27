//! Video playback and sentence-repeat coordination.
use super::*;

impl AppView {
    pub(super) fn start_part(&mut self, index: usize) {
        if self.tasks.cancel.is_some() {
            self.report_status(FeedbackKind::Error, "请先完成或取消当前前台任务".into());
            return;
        }
        let Some(mut series) = self.video.series.clone() else {
            return;
        };
        if index >= series.parts.len() {
            return;
        }
        if let Some(path) = self.tasks.downloads.cached(&series, index) {
            series.parts[index].path = Some(path.clone());
            series.current = Some(index);
            self.video.series = Some(series);
            let panel = self.video.tab;
            if self.open(path) {
                self.video.tab = panel;
                self.media.player.set("pause", "no");
                self.report_status(FeedbackKind::Info, format!("正在播放 P{}", index + 1));
            }
            return;
        }
        let origin = self.media.path.clone();
        let downloads = self.tasks.downloads.clone();
        self.begin(
            &format!("正在准备 P{}…", index + 1),
            move |lib, cancel, _| {
                let path = downloads.obtain(&lib, &series, index, &cancel)?;
                // Completed files belong to the shared cache, even if this foreground request is cancelled.
                Ok(Outcome::PartVideo(series, index, path, false, origin))
            },
        );
    }
    pub(super) fn open(&mut self, path: PathBuf) -> bool {
        self.open_at(path, 0., false)
    }
    pub(super) fn open_at(&mut self, path: PathBuf, position: f64, paused: bool) -> bool {
        self.clear_video_sentence_repeat();
        self.video.picture_click = None;
        let result = (|| -> Result<_> {
            let mut videos = video_library::Collection::load(&self.lib)?;
            let ids = videos.add(&[path], self.video.series.as_ref())?;
            let video = videos.select(&ids[0])?;
            videos.save(&self.lib)?;
            self.video.collection = videos;
            Ok(video)
        })();
        let video = match result {
            Ok(video) => video,
            Err(e) => {
                self.report_status(FeedbackKind::Error, format!("无法选择视频：{e:#}"));
                return false;
            }
        };
        let path = video.path;
        self.video.series = video.series;
        self.video.remove = None;
        if let Some(cancel) = self.audio.waveform_cancel.take() {
            cancel.store(true, Ordering::SeqCst);
        }
        self.audio.waveform = None;
        self.audio.waveform_rx = None;
        self.audio.key_points.clear();
        if let Some(series) = &mut self.video.series {
            series.current = series
                .parts
                .iter()
                .position(|p| p.path.as_ref() == Some(&path));
            let _ = series.save(&self.lib);
            if let Some(index) = series.current {
                self.video
                    .parts_scroll
                    .scroll_to_item(index, ScrollStrategy::Center);
            }
        }
        if self.media.repeat >= 2 {
            self.media.repeat = 0;
        }
        self.video.path = Some(path.clone());
        self.media.player.open_range(&path, position, 0., paused);
        self.media.path = Some(path.clone());
        self.media.loop_a = None;
        self.media.loop_b = None;
        self.audio.cues.clear();
        self.audio.current_id.clear();
        self.load_video_subtitles(&path);
        if self.video.series.is_none() {
            let _ = self.lib.set_setting("bilibili_series", "");
        }
        true
    }
    pub(super) fn prepare_video_material(&mut self, scope: video_qa::Scope) -> Result<()> {
        let path = self.media.path.as_ref().context("请先打开视频")?;
        match video_qa::build(
            path,
            &self.video.cues,
            scope,
            self.media.snapshot.position,
            &self.media.snapshot.subtitle,
        ) {
            Ok(material) => {
                self.video.material = Some(material);
                self.video.material_error.clear();
                Ok(())
            }
            Err(error) => {
                self.video.material = None;
                self.video.material_error = error.to_string();
                Err(error)
            }
        }
    }
    pub(super) fn load_video_subtitles(&mut self, _path: &std::path::Path) {
        self.clear_video_sentence_repeat();
        self.video.caption_source = VideoCaptionSource::Transcript;
        self.video.material = None;
        self.video.material_error.clear();
        self.video.set_cues(vec![]);
        self.refresh_saved_ranges();
        self.refresh();
        self.video.tab = 0;
    }
    pub(super) fn leave_video_fullscreen(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
        self.video.picture_click = None;
        if !self.video.fullscreen {
            return;
        }
        if self.video.fullscreen_owned && window.is_fullscreen() {
            window.toggle_fullscreen();
        }
        self.video.fullscreen = false;
        self.video.fullscreen_owned = false;
        self.video.fullscreen_seen = false;
        window.focus(&self.focus);
    }
    pub(super) fn clear_video_sentence_repeat(&mut self) {
        if self.video.sentence_repeat.take().is_some() {
            self.media.player.clear_sentence_repeat();
        }
    }
    pub(super) fn follow_video_repeat(&mut self) {
        if let Some(session) = self.video.sentence_repeat {
            self.video.active = Some(session.index);
            if self.video.follow {
                self.video.center.request(session.index);
            }
            self.report_status(
                FeedbackKind::Info,
                "每句播放三遍后自动下一句；← / → 切句并从第一遍开始".into(),
            );
        }
    }
    pub(super) fn poll_video_sentence_repeat(&mut self) -> bool {
        let Some(mut session) = self.video.sentence_repeat else {
            return false;
        };
        let Some(path) = self.video.path.as_ref() else {
            return false;
        };
        let snapshot = self.media.player.snapshot();
        match session.poll(&self.media.player, path, &self.video.cues, &snapshot) {
            Ok(clarora_gpui::video_repeat::Transition::Unchanged) => false,
            Ok(transition) => {
                self.video.sentence_repeat = Some(session);
                if transition == clarora_gpui::video_repeat::Transition::Advanced {
                    self.follow_video_repeat();
                } else {
                    self.report_status(
                        FeedbackKind::Info,
                        "最后一句已播放三遍，已暂停；← 可切换上一句继续三次重复".into(),
                    );
                }
                true
            }
            Err(error) => {
                self.clear_video_sentence_repeat();
                self.report_status(FeedbackKind::Error, error.to_string());
                true
            }
        }
    }
    pub(super) fn clear_finished_video_sentence_repeat(&mut self) {
        self.poll_video_sentence_repeat();
        if self.video.sentence_repeat.is_some_and(|session| {
            self.media
                .player
                .snapshot()
                .sentence_repeat
                .as_ref()
                .is_some_and(|state| state.request_id == session.request_id && state.finished)
        }) {
            self.clear_video_sentence_repeat();
        }
    }
    pub(super) fn video_horizontal_action(&self, next: bool) -> Action {
        if self.page == "video" {
            Action::VideoSentence(next)
        } else {
            Action::Seek(if next { 10. } else { -10. })
        }
    }
    pub(super) fn can_move_video_sentence(&self, next: bool) -> bool {
        if let Some(session) = self.video.sentence_repeat.filter(|s| s.three) {
            session
                .neighbor(&self.video.cues, next, self.media.snapshot.duration)
                .is_some()
        } else {
            clarora_gpui::video_repeat::adjacent(
                &self.video.cues,
                self.media.snapshot.position,
                next,
                self.media.snapshot.duration,
            )
            .is_some()
        }
    }
    pub(super) fn video_repeat_cue(&self) -> Option<usize> {
        self.video
            .sentence_repeat
            .map(|session| session.index)
            .or_else(|| video_subtitles::active(&self.video.cues, self.media.snapshot.position))
            .filter(|index| self.video.cues.get(*index).is_some())
    }
    pub(super) fn start_video_sentence_repeat(
        &mut self,
        index: Option<usize>,
        three: bool,
    ) -> Result<()> {
        ensure!(self.page == "video", "请先进入视频学习");
        let path = self.video.path.as_ref().context("请先打开视频")?;
        let playback = self.media.player.snapshot();
        ensure!(
            self.media.path.as_ref() == Some(path)
                && playback.path == path.to_string_lossy()
                && playback.duration > 0.,
            "视频正在切换，请稍候再重复本句"
        );
        let index = index
            .or_else(|| self.video_repeat_cue())
            .context("当前没有带时间轴的字幕，请先选择一句字幕")?;
        self.video.sentence_repeat = Some(clarora_gpui::video_repeat::Session::start(
            &self.media.player,
            path,
            &self.video.cues,
            index,
            three,
            playback.duration,
        )?);
        self.media.repeat = 0;
        self.media.loop_a = None;
        self.media.loop_b = None;
        self.media.end_handled = true;
        if three {
            self.follow_video_repeat();
        } else {
            self.report_status(
                FeedbackKind::Info,
                "正在无限重复本句，可从本句重复菜单停止".into(),
            );
        }
        Ok(())
    }
}
