//! Audio sentence requests share the native, precisely counted player primitive.
use super::*;
use clarora_gpui::video_repeat::{self, Session, Transition};

pub(super) const MODES: [&str; 8] = [
    "不循环",
    "单曲循环",
    "A–B 循环",
    "组循环",
    "单次连播",
    "单句循环",
    "每句三遍",
    "每句五遍",
];
impl AppView {
    pub(super) fn reset_audio_sentence_request(&mut self) {
        if self.media.session.audio_sentence().is_some() {
            self.media.player.clear_sentence_repeat();
            self.media.session.set_audio_sentence(None);
        }
    }
    fn audio_repeat_ready(&self) -> Result<(PathBuf, Playback)> {
        ensure!(!self.loads.restore_pending, "正在恢复音频进度，请稍候");
        ensure!(
            self.page == "listening" && !self.audio.current_id.is_empty(),
            "请先选择音频"
        );
        ensure!(
            self.media.session.is_audio(&self.audio.current_id),
            "当前播放器不是所选音频"
        );
        let path = self.media.session.path.clone().context("请先选择音频")?;
        ensure!(
            self.selected()
                .is_some_and(|row| Path::new(&text(&row, "audio_uri")) == path),
            "当前播放器不是所选音频"
        );
        let state = self.media.player.snapshot();
        ensure!(
            self.media.matches_snapshot(&state)
                && state.path == path.to_string_lossy()
                && state.duration.is_finite()
                && state.duration > 0.,
            "音频正在切换，请稍候"
        );
        Ok((path, state))
    }
    pub(super) fn start_audio_sentence(&mut self, index: usize, paused: bool) -> Result<()> {
        let (path, state) = self.audio_repeat_ready()?;
        let session = Session::start(
            &self.media.player,
            &path,
            &self.audio.cues,
            index,
            match self.audio.repeat_mode {
                PlaybackMode::SentenceThree => Some(3),
                PlaybackMode::SentenceFive => Some(5),
                _ => None,
            },
            state.duration,
        )?;
        ensure!(
            self.media.session.set_audio_sentence(Some(session)),
            "当前播放器不是所选音频"
        );
        self.audio.pending_repeat_pause = None;
        self.audio.pending_repeat_position = None;
        self.media.session.loop_a = None;
        self.media.session.loop_b = None;
        self.media.session.mode = self.audio.repeat_mode;
        self.media.session.end_handled = true;
        self.audio.active = Some(index);
        self.invalidate_blind();
        if self.audio.follow {
            self.audio.center.request(index);
        }
        if paused {
            self.media.player.set("pause", "yes");
        }
        Ok(())
    }
    pub(super) fn set_audio_repeat_mode(&mut self, mode: usize) -> Result<()> {
        ensure!(mode < MODES.len(), "无效循环方式");
        let mode = PlaybackMode::from_index(mode);
        let index = if mode.is_sentence() {
            let (_, state) = self.audio_repeat_ready()?;
            Some(
                video_repeat::at_or_next(&self.audio.cues, state.position, state.duration)
                    .context("请先生成或导入带时间轴的字幕")?,
            )
        } else {
            None
        };
        if mode == PlaybackMode::Ab {
            ensure!(
                self.media.session.is_audio(&self.audio.current_id),
                "当前播放器不是所选音频"
            );
            ensure!(
                self.media
                    .session
                    .loop_a
                    .zip(self.media.session.loop_b)
                    .is_some_and(|(a, b)| b > a),
                "先截取有效片段"
            );
        }
        // A/B bounds are per file and intentionally are not restored as a preference.
        self.lib.set_setting(
            "audio_repeat_mode",
            &if mode == PlaybackMode::Ab {
                PlaybackMode::Normal
            } else {
                mode
            }
            .index()
            .to_string(),
        )?;
        self.reset_audio_sentence_request();
        self.audio.repeat_mode = mode;
        if self.media.session.is_audio(&self.audio.current_id) {
            self.media.session.mode = mode;
        }
        self.audio.pending_repeat_pause = None;
        self.audio.pending_repeat_position = None;
        if let Some(index) = index {
            self.audio.clip_mode = false;
            self.start_audio_sentence(index, false)?;
        }
        Ok(())
    }
    pub(super) fn audio_sentence(&mut self, direction: i8) -> Result<()> {
        let (_, state) = self.audio_repeat_ready()?;
        let index = if let Some(session) = self.media.session.audio_sentence() {
            if direction == 0 {
                Some(session.index)
            } else {
                session.neighbor(&self.audio.cues, direction > 0, state.duration)
            }
        } else if direction == 0 {
            video_repeat::at_or_next(&self.audio.cues, state.position, state.duration)
        } else {
            video_repeat::adjacent(
                &self.audio.cues,
                state.position,
                direction > 0,
                state.duration,
            )
        };
        let Some(index) = index else {
            ensure!(
                video_repeat::first(&self.audio.cues, state.duration).is_some(),
                "请先生成或导入带时间轴的字幕"
            );
            return Ok(());
        };
        if self.audio.repeat_mode.is_sentence() {
            self.start_audio_sentence(index, false)?;
        } else {
            self.invalidate_blind();
            let time = self.audio.cues[index].start;
            if self.media.session.mode == PlaybackMode::Ab
                && !self
                    .media
                    .session
                    .loop_a
                    .zip(self.media.session.loop_b)
                    .is_some_and(|(a, b)| time >= a && time < b)
            {
                self.set_audio_repeat_mode(0)?;
                self.media.session.loop_a = None;
                self.media.session.loop_b = None;
            }
            self.media
                .player
                .command(["seek".into(), time.to_string(), "absolute+exact".into()]);
            self.media.player.set("pause", "no");
            self.audio.active = Some(index);
            if self.audio.follow {
                self.audio.center.request(index);
            }
        }
        Ok(())
    }
    pub(super) fn seek_audio_repeat(&mut self, time: f64, subtitle: bool) -> Result<bool> {
        if self.page != "listening" || !self.audio.repeat_mode.is_sentence() {
            return Ok(false);
        }
        ensure!(time.is_finite(), "播放位置无效");
        let (path, state) = self.audio_repeat_ready()?;
        if subtitle
            || self.media.session.audio_sentence().is_none()
            || state.sentence_repeat.as_ref().is_some_and(|r| r.finished)
        {
            let index = if !subtitle {
                self.media
                    .session
                    .audio_sentence()
                    .map(|s| s.index)
                    .or_else(|| {
                        video_repeat::at_or_next(&self.audio.cues, time.max(0.), state.duration)
                    })
            } else {
                video_repeat::at_or_next(&self.audio.cues, time.max(0.), state.duration)
            }
            .context("请先生成或导入带时间轴的字幕")?;
            self.start_audio_sentence(index, !subtitle && state.paused)?;
        }
        let session = self
            .media
            .session
            .audio_sentence()
            .context("请先选择字幕句")?;
        session.seek(
            &self.media.player,
            &path,
            &self.audio.cues,
            time.max(0.),
            state.duration,
        )?;
        self.invalidate_blind();
        if subtitle {
            self.media.player.set("pause", "no");
        }
        Ok(true)
    }
    pub(super) fn restart_finished_audio_track(&mut self) -> Result<bool> {
        if self.page != "listening"
            || self.audio.repeat_mode.is_sentence()
            || !self.media.player.snapshot().eof
        {
            return Ok(false);
        }
        let (_, state) = self.audio_repeat_ready()?;
        if !state.eof {
            return Ok(false);
        }
        let start = if self.media.session.mode == PlaybackMode::Ab {
            self.media
                .session
                .loop_a
                .zip(self.media.session.loop_b)
                .filter(|(a, b)| {
                    a.is_finite() && b.is_finite() && *a >= 0. && *a < state.duration && b > a
                })
                .map(|(a, _)| a)
                .unwrap_or(0.)
        } else {
            0.
        };
        self.invalidate_blind();
        self.media
            .player
            .command(["seek".into(), start.to_string(), "absolute+exact".into()]);
        self.media.player.set("pause", "no");
        // Keep the old EOF handled until polling observes the completed seek,
        // so continuous playback cannot advance using a stale end snapshot.
        Ok(true)
    }
    pub(super) fn restart_finished_audio_sentence(&mut self) -> Result<bool> {
        if self.page != "listening" || !self.audio.repeat_mode.is_sentence() {
            return Ok(false);
        }
        if let Some(session) = self.media.session.audio_sentence()
            && self
                .media
                .player
                .snapshot()
                .sentence_repeat
                .as_ref()
                .is_some_and(|r| r.request_id == session.request_id && r.finished)
        {
            self.start_audio_sentence(session.index, false)?;
            return Ok(true);
        }
        Ok(false)
    }
    pub(super) fn poll_audio_sentence_repeat(&mut self) -> bool {
        if self.page != "listening" || !self.audio.repeat_mode.is_sentence() {
            return false;
        }
        let Ok((path, state)) = self.audio_repeat_ready() else {
            return false;
        };
        let position = self.audio.pending_repeat_position.unwrap_or(state.position);
        let result = (|| -> Result<bool> {
            if let Some(mut session) = self.media.session.audio_sentence() {
                let transition =
                    session.poll(&self.media.player, &path, &self.audio.cues, &state)?;
                ensure!(
                    self.media.session.set_audio_sentence(Some(session)),
                    "当前播放器不是所选音频"
                );
                if transition == Transition::Advanced {
                    self.audio.active = Some(session.index);
                    self.invalidate_blind();
                    if self.audio.follow {
                        self.audio.center.request(session.index);
                    }
                }
                Ok(transition != Transition::Unchanged)
            } else if let Some(index) =
                video_repeat::at_or_next(&self.audio.cues, position, state.duration)
            {
                let paused = self.audio.pending_repeat_pause.unwrap_or(state.paused);
                self.start_audio_sentence(index, paused)?;
                if paused
                    && let Some(session) = self
                        .media
                        .session
                        .audio_sentence()
                        .filter(|s| position >= s.range.0 && position < s.range.1)
                {
                    session.seek(
                        &self.media.player,
                        &path,
                        &self.audio.cues,
                        position,
                        state.duration,
                    )?;
                }
                Ok(true)
            } else {
                self.media.player.set("pause", "yes");
                Ok(false)
            }
        })();
        match result {
            Ok(changed) => changed,
            Err(error) => {
                self.reset_audio_sentence_request();
                self.audio.pending_repeat_pause = Some(true);
                self.media.player.set("pause", "yes");
                self.report_status(FeedbackKind::Error, format!("逐句重复已暂停：{error:#}"));
                true
            }
        }
    }
    pub(super) fn audio_saved_position(&self) -> f64 {
        let state = self.media.player.snapshot();
        let ready = self.media.matches_snapshot(&state);
        let position = if ready && state.position.is_finite() && state.position >= 0. {
            state.position
        } else {
            self.audio
                .pending_repeat_position
                .filter(|p| p.is_finite() && *p >= 0.)
                .unwrap_or(self.media.snapshot.position)
        };
        self.media
            .session
            .audio_sentence()
            .map(|s| {
                if ready
                    && state
                        .sentence_repeat
                        .as_ref()
                        .is_some_and(|r| r.request_id == s.request_id)
                {
                    state
                        .position
                        .clamp(s.range.0, (s.range.1 - 0.001).max(s.range.0))
                } else {
                    s.range.0
                }
            })
            .unwrap_or(position)
    }
    pub(super) fn audio_repeat_label(&self) -> String {
        if matches!(
            self.audio.repeat_mode,
            PlaybackMode::SentenceThree | PlaybackMode::SentenceFive
        ) {
            let total = if self.audio.repeat_mode == PlaybackMode::SentenceFive {
                5
            } else {
                3
            };
            let play = self
                .media
                .session
                .audio_sentence()
                .and_then(|s| {
                    self.media
                        .snapshot
                        .sentence_repeat
                        .as_ref()
                        .filter(|r| r.request_id == s.request_id)
                })
                .map(|r| r.current_play)
                .unwrap_or(1);
            format!(
                "{} · {}/{}",
                clarora_gpui::video_repeat::label(total),
                play.clamp(1, total),
                total
            )
        } else {
            MODES
                .get(self.audio.repeat_mode.index())
                .unwrap_or(&MODES[0])
                .to_string()
        }
    }
    pub(super) fn handle_audio_arrow_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let m = event.keystroke.modifiers;
        if self.page != "listening"
            || m.control
            || m.platform
            || m.alt
            || m.shift
            || window.has_active_dialog(cx)
            || !window.focused(cx).is_some_and(|focus| {
                focus == self.focus || focus == self.subtitle_text.borrow().focus
            })
        {
            return false;
        }
        let direction = match event.keystroke.key.as_str() {
            "left" => -1,
            "right" => 1,
            _ => return false,
        };
        self.handle(Action::AudioSentence(direction), window, cx);
        true
    }
}
