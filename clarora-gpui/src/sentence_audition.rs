//! Independent exact-range playback; never changes ordinary learning preferences.
use crate::{
    media::{Playback, Player},
    sentence_search::Hit,
};
use anyhow::{Context, Result, ensure};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
struct Loading {
    path: PathBuf,
    revision: u64,
    since: Instant,
    previous_error: String,
}
pub struct Audition {
    player: Player,
    loading: Option<Loading>,
    active_path: Option<PathBuf>,
    request: Option<u64>,
    pub index: Option<usize>,
    pub autoplay: bool,
    /// 0 repeats indefinitely, 1 once, 3 three times.
    pub repeats: u8,
    pub finished: bool,
    pub completed: u64,
    pub last_completed: Option<usize>,
    pub error: String,
}
impl Audition {
    pub fn new(player: Player) -> Self {
        Self {
            player,
            loading: None,
            active_path: None,
            request: None,
            index: None,
            autoplay: true,
            repeats: 1,
            finished: false,
            completed: 0,
            last_completed: None,
            error: String::new(),
        }
    }
    pub fn snapshot(&self) -> Playback {
        self.player.snapshot()
    }
    pub fn is_loading(&self) -> bool {
        self.loading.is_some()
    }
    pub fn select(&mut self, hits: &[Hit], index: usize) -> Result<()> {
        ensure!(index < hits.len(), "搜索结果已改变，请重新选择");
        self.index = Some(index);
        self.finished = false;
        self.error.clear();
        self.request = None;
        self.player.set("pause", "yes");
        if self.loading.is_none() {
            if let Err(error) = self.install(hits) {
                self.error = format!("{error:#}");
                self.finished = true;
                return Err(error);
            }
        }
        Ok(())
    }
    fn install(&mut self, hits: &[Hit]) -> Result<()> {
        let hit = hits
            .get(self.index.context("请先选择句子")?)
            .context("搜索结果已改变")?;
        ensure!(
            hit.path.is_file(),
            "来源文件已不存在，请重新搜索或恢复该文件"
        );
        let state = self.snapshot();
        if self.active_path.as_ref() != Some(&hit.path) || state.path != hit.path.to_string_lossy()
        {
            self.loading = Some(Loading {
                path: hit.path.clone(),
                revision: state.load_revision,
                since: Instant::now(),
                previous_error: state.error,
            });
            self.player.open_range(&hit.path, 0., 0., true);
            return Ok(());
        }
        ensure!(
            hit.cue.start < state.duration,
            "字幕时间超出来源媒体，请重新生成字幕"
        );
        self.request = Some(self.player.repeat_sentence(
            &hit.path,
            hit.cue.start,
            hit.cue.end.min(state.duration),
            if self.repeats == 0 {
                None
            } else {
                Some(u32::from(self.repeats))
            },
        )?);
        Ok(())
    }
    pub fn poll(&mut self, hits: &[Hit]) -> Result<bool> {
        if self.index.is_none() || (self.finished && !self.error.is_empty()) {
            return Ok(false);
        }
        let state = self.snapshot();
        let result = (|| -> Result<bool> {
            ensure!(
                state.error.is_empty()
                    || (state.initialized
                        && self
                            .loading
                            .as_ref()
                            .is_some_and(|load| load.previous_error == state.error)),
                "试听失败：{}",
                state.error
            );
            if let Some(load) = &self.loading {
                ensure!(
                    load.since.elapsed() < Duration::from_secs(12),
                    "载入来源文件超时，请重试或选择下一条"
                );
                if state.load_revision > load.revision
                    && state.path == load.path.to_string_lossy()
                    && state.duration > 0.
                {
                    self.active_path = Some(load.path.clone());
                    self.loading = None;
                    self.install(hits)?;
                    return Ok(true);
                }
                return Ok(false);
            }
            if !self.finished
                && state
                    .sentence_repeat
                    .as_ref()
                    .is_some_and(|r| Some(r.request_id) == self.request && r.finished)
            {
                let index = self.index.unwrap();
                self.last_completed = Some(index);
                self.completed = self.completed.saturating_add(1);
                if self.autoplay && index + 1 < hits.len() {
                    self.select(hits, index + 1)?;
                } else {
                    self.finished = true;
                }
                return Ok(true);
            }
            Ok(false)
        })();
        if let Err(error) = &result {
            self.error = format!("{error:#}");
            self.player.set("pause", "yes");
            self.player.clear_sentence_repeat();
            self.loading = None;
            self.request = None;
            self.finished = true;
        }
        result
    }
    pub fn toggle(&mut self, hits: &[Hit]) -> Result<()> {
        if self.is_loading() {
            return Ok(());
        }
        let index = self.index.unwrap_or(0);
        if self.finished || self.index.is_none() {
            return self.select(hits, index);
        }
        let paused = self.snapshot().paused;
        self.player.set("pause", if paused { "no" } else { "yes" });
        Ok(())
    }
    pub fn step(&mut self, hits: &[Hit], next: bool) -> Result<()> {
        if hits.is_empty() {
            return Ok(());
        }
        let index = self.index.unwrap_or(0);
        let target = if next {
            (index + 1).min(hits.len() - 1)
        } else {
            index.saturating_sub(1)
        };
        self.select(hits, target)
    }
    pub fn stop(&mut self) {
        self.player.set("pause", "yes");
        self.player.clear_sentence_repeat();
        self.player.command(["stop"]);
        self.loading = None;
        self.active_path = None;
        self.request = None;
        self.index = None;
        self.finished = false;
    }
}
