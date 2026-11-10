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
    /// `None` is an infinite sentence loop; `Some(n)` stops after n plays.
    pub total: Option<u32>,
    pub range: (f64, f64),
    pub request_id: u64,
    completed: bool,
}

/// Human-readable label for finite sentence repetition counts.
pub fn label(total: u32) -> String {
    match total {
        3 => "每句三遍".into(),
        4 => "每句四遍".into(),
        5 => "每句五遍".into(),
        6 => "每句六遍".into(),
        n => format!("每句 {n} 遍"),
    }
}

/// Leading plays whose subtitle is hidden for the blind half of 4/6 repeats.
pub fn blind_plays(total: u32) -> u32 {
    match total {
        4 => 2,
        6 => 3,
        _ => 0,
    }
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

/// Move relative to the current subtitle; in a gap, choose the nearest cue in
/// the requested direction. No fixed time threshold may skip a short sentence.
pub fn adjacent(cues: &[Cue], position: f64, next: bool, duration: f64) -> Option<usize> {
    if !position.is_finite() || position < 0. {
        return None;
    }
    let current = cues.iter().rposition(|cue| {
        range(cue, duration).is_some_and(|(start, end)| position >= start && position < end)
    });
    if let Some(index) = current {
        if next {
            ((index + 1)..cues.len()).find(|&i| range(&cues[i], duration).is_some())
        } else {
            (0..index)
                .rev()
                .find(|&i| range(&cues[i], duration).is_some())
        }
    } else if next {
        cues.iter()
            .position(|cue| range(cue, duration).is_some_and(|(start, _)| start > position))
    } else {
        cues.iter()
            .rposition(|cue| range(cue, duration).is_some_and(|(start, _)| start <= position))
    }
}

pub fn first(cues: &[Cue], duration: f64) -> Option<usize> {
    cues.iter().position(|cue| range(cue, duration).is_some())
}

/// Timeline selection snaps to its cue; a gap chooses the next valid cue,
/// with the final valid cue used beyond the subtitle tail.
pub fn at_or_next(cues: &[Cue], position: f64, duration: f64) -> Option<usize> {
    if !position.is_finite() || position < 0. {
        return None;
    }
    cues.iter()
        .rposition(|cue| {
            range(cue, duration).is_some_and(|(start, end)| position >= start && position < end)
        })
        .or_else(|| adjacent(cues, position, true, duration))
        .or_else(|| adjacent(cues, position, false, duration))
}

impl Session {
    pub fn start(
        player: &Player,
        path: &Path,
        cues: &[Cue],
        index: usize,
        total: Option<u32>,
        duration: f64,
    ) -> Result<Self> {
        let range = cues
            .get(index)
            .and_then(|cue| range(cue, duration))
            .context("本句字幕时间无效")?;
        let request_id = player.repeat_sentence(path, range.0, range.1, total)?;
        Ok(Self {
            index,
            total,
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

    /// Move inside this request without changing its sentence or play count.
    pub fn seek(
        &self,
        player: &Player,
        path: &Path,
        cues: &[Cue],
        position: f64,
        duration: f64,
    ) -> Result<()> {
        ensure!(
            cues.get(self.index)
                .and_then(|cue| range(cue, duration))
                .is_some_and(|(a, b)| {
                    (a - self.range.0).abs() < 0.001 && (b - self.range.1).abs() < 0.001
                }),
            "字幕时间轴已更新，请重新选择重复句"
        );
        player.seek_sentence_repeat(path, self.request_id, position)
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
        *self = Self::start(player, path, cues, index, self.total, duration)?;
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
        let Some(total) = self.total.filter(|_| !self.completed) else {
            return Ok(Transition::Unchanged);
        };
        let finished = snapshot.path == path.to_string_lossy()
            && snapshot.sentence_repeat.as_ref().is_some_and(|r| {
                r.request_id == self.request_id && r.finished && r.total_plays == Some(total)
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
