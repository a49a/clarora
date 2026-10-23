//! Subtitle sequence coordination; the player owns precise per-sentence counting.
use crate::{
    media::{Playback, Player},
    subtitles::Cue,
};
use anyhow::{Context, Result, ensure};
use std::path::Path;

#[derive(Clone, Copy, Debug)]
pub struct Session {
    pub index: usize,
    pub three: bool,
    pub range: (f64, f64),
    pub request_id: u64,
    completed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    Unchanged,
    Advanced,
    Completed,
}

fn range(cue: &Cue, duration: f64) -> Option<(f64, f64)> {
    let end = cue.end.min(duration);
    (duration.is_finite()
        && duration > 0.
        && cue.start.is_finite()
        && cue.end.is_finite()
        && cue.start >= 0.
        && end > cue.start)
        .then_some((cue.start, end))
}

impl Session {
    pub fn start(
        player: &Player,
        path: &Path,
        cues: &[Cue],
        index: usize,
        three: bool,
        duration: f64,
    ) -> Result<Self> {
        let range = cues
            .get(index)
            .and_then(|cue| range(cue, duration))
            .context("本句字幕时间无效")?;
        let request_id =
            player.repeat_sentence(path, range.0, range.1, if three { Some(3) } else { None })?;
        Ok(Self {
            index,
            three,
            range,
            request_id,
            completed: false,
        })
    }

    pub fn neighbor(&self, cues: &[Cue], next: bool, duration: f64) -> Option<usize> {
        if next {
            ((self.index + 1)..cues.len()).find(|&i| range(&cues[i], duration).is_some())
        } else {
            (0..self.index.min(cues.len()))
                .rev()
                .find(|&i| range(&cues[i], duration).is_some())
        }
    }

    /// Relative to the selected subtitle, even while a new player request is pending.
    pub fn step(
        &mut self,
        player: &Player,
        path: &Path,
        cues: &[Cue],
        next: bool,
        duration: f64,
    ) -> Result<bool> {
        let Some(index) = self.neighbor(cues, next, duration) else {
            return Ok(false);
        };
        *self = Self::start(player, path, cues, index, self.three, duration)?;
        Ok(true)
    }

    pub fn poll(
        &mut self,
        player: &Player,
        path: &Path,
        cues: &[Cue],
        snapshot: &Playback,
    ) -> Result<Transition> {
        let current = cues
            .get(self.index)
            .and_then(|cue| range(cue, snapshot.duration));
        ensure!(
            current.is_some_and(
                |(a, b)| (a - self.range.0).abs() <= 0.001 && (b - self.range.1).abs() <= 0.001
            ),
            "字幕时间轴已更新，已停止旧句重复"
        );
        if !self.three || self.completed {
            return Ok(Transition::Unchanged);
        }
        let finished = snapshot.path == path.to_string_lossy()
            && snapshot.sentence_repeat.as_ref().is_some_and(|r| {
                r.request_id == self.request_id && r.finished && r.total_plays == Some(3)
            });
        if !finished {
            return Ok(Transition::Unchanged);
        }
        if self.step(player, path, cues, true, snapshot.duration)? {
            Ok(Transition::Advanced)
        } else {
            self.completed = true;
            Ok(Transition::Completed)
        }
    }
}
