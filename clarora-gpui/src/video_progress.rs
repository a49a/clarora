//! Local-only per-video and per-part playback/reading progress.
//!
//! This intentionally stays out of the backup/sync allowlist: it identifies local
//! files and reading view state, while the legacy `last_video_position` remains
//! the portable fallback used by older backups.
use crate::{
    data::Library,
    video_library::{self, Collection},
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub const KEY: &str = "video_progress_v1";
const VERSION: u8 = 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Resume {
    #[default]
    None,
    Continue,
    Replay,
}

pub fn resume(item: Option<&Item>) -> Resume {
    match item {
        Some(item) if item.completed => Resume::Replay,
        Some(item) if item.position > 0.5 => Resume::Continue,
        _ => Resume::None,
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Reading {
    #[serde(default)]
    pub tab: usize,
    #[serde(default)]
    pub transcript_open: bool,
    #[serde(default = "follow_default")]
    pub follow: bool,
    #[serde(default)]
    pub cue_start: Option<f64>,
    /// Playback cue anchor; may differ from `cue_start` when the user manually
    /// scrolled the subtitle list while following is off.
    #[serde(default)]
    pub active_start: Option<f64>,
    #[serde(default)]
    pub cue_offset: f32,
    /// Top visible transcript paragraph. Older snapshots only have a pixel
    /// offset and fall back to the playback cue.
    #[serde(default)]
    pub transcript_item: Option<usize>,
    /// Cue start of the manually read paragraph, used to re-anchor after the
    /// English transcript inserts or removes paragraphs.
    #[serde(default)]
    pub transcript_cue_start: Option<f64>,
    #[serde(default)]
    pub transcript_offset: f32,
    #[serde(default)]
    pub transcript_revision: String,
}

/// Manual scroll anchor of the part list for one series.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct PartsAnchor {
    #[serde(default)]
    pub index: usize,
    /// Fraction of the row above the viewport top, `0.0..1.0`.
    #[serde(default)]
    pub offset: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    #[serde(default)]
    pub position: f64,
    #[serde(default)]
    pub completed: bool,
    #[serde(default)]
    pub sentence_repeat: Option<u32>,
    #[serde(default)]
    pub repeat_mode: usize,
    #[serde(default)]
    pub reading: Reading,
    #[serde(default)]
    pub updated_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct State {
    version: u8,
    recent: Option<String>,
    items: BTreeMap<String, Item>,
    /// Manual part-list positions, keyed by source URL. Local-only like the
    /// rest of this file.
    #[serde(default)]
    parts: BTreeMap<String, PartsAnchor>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            version: VERSION,
            recent: None,
            items: BTreeMap::new(),
            parts: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Progress {
    state: State,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

fn normalized(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn part_number(video: &video_library::Video, path: &Path) -> Option<u32> {
    let path = normalized(path);
    let series = video.series.as_ref()?;
    series
        .parts
        .iter()
        .find(|part| {
            part.path
                .as_ref()
                .is_some_and(|part| normalized(part) == path)
        })
        .or_else(|| series.current.and_then(|index| series.parts.get(index)))
        .map(|part| part.number)
}

/// Stable playback identity: favorite ID plus part number, or a normalized local path.
pub fn item_key(collection: &Collection, path: &Path) -> String {
    if let Some(video) = collection.items.iter().find(|video| video.contains(path)) {
        let part = part_number(video, path).unwrap_or(0);
        format!("v:{}:{part}", video.id)
    } else {
        format!("f:{}", normalized(path).to_string_lossy())
    }
}

pub fn series_item_key(
    collection: &Collection,
    series: &crate::bilibili::Series,
    index: usize,
) -> Option<String> {
    let video = collection.items.iter().find(|video| {
        video
            .series
            .as_ref()
            .is_some_and(|old| old.base == series.base)
    })?;
    let part = series.parts.get(index)?.number;
    Some(format!("v:{}:{part}", video.id))
}

/// Stable identity of a series independent of the favorite that currently
/// owns it, so a manual part-list position survives collection changes.
pub fn series_key(series: &crate::bilibili::Series) -> String {
    format!("s:{}", series.base)
}

impl Progress {
    pub fn load(lib: &Library) -> Self {
        let saved = lib.setting(KEY).unwrap_or_default();
        if saved.is_empty() {
            return Self::default();
        }
        let state = serde_json::from_str::<State>(&saved)
            .ok()
            .filter(|state| state.version == VERSION);
        let Some(mut state) = state else {
            return Self::default();
        };
        state.items.retain(|key, item| {
            !key.is_empty()
                && item.position.is_finite()
                && item.position >= 0.
                && item.reading.tab < 4
                && item.reading.cue_start.is_none_or(f64::is_finite)
                && item.reading.active_start.is_none_or(f64::is_finite)
                && item.reading.transcript_cue_start.is_none_or(f64::is_finite)
                && item.reading.cue_offset.is_finite()
                && item.reading.transcript_offset.is_finite()
        });
        state.parts.retain(|key, anchor| {
            !key.is_empty() && anchor.offset.is_finite() && (0. ..1.).contains(&anchor.offset)
        });
        Self { state }
    }

    pub fn save(&self, lib: &Library) -> Result<()> {
        lib.set_setting(KEY, &serde_json::to_string(&self.state)?)
    }

    pub fn get(&self, key: &str) -> Option<&Item> {
        self.state.items.get(key)
    }

    pub fn parts_anchor(&self, key: &str) -> Option<PartsAnchor> {
        self.state.parts.get(key).copied()
    }

    pub fn record_parts_anchor(&mut self, key: String, anchor: PartsAnchor) -> Result<()> {
        ensure!(
            !key.is_empty() && anchor.offset.is_finite() && (0. ..1.).contains(&anchor.offset),
            "选集阅读位置无效"
        );
        self.state.parts.insert(key, anchor);
        Ok(())
    }

    pub fn record(&mut self, key: String, mut item: Item) -> Result<()> {
        ensure!(
            !key.is_empty() && item.position.is_finite() && item.position >= 0.,
            "视频进度无效"
        );
        item.updated_ms = now_ms();
        self.state.recent = Some(key.clone());
        self.state.items.insert(key, item);
        Ok(())
    }

    /// Seed an item from the legacy single bookmark when the paths still match.
    pub fn seed_legacy(&mut self, lib: &Library, key: &str, path: &Path) {
        if self.state.items.contains_key(key) {
            return;
        }
        let Some(position) = legacy_position(lib, path) else {
            return;
        };
        let _ = self.record(
            key.to_owned(),
            Item {
                position,
                completed: false,
                sentence_repeat: None,
                repeat_mode: 0,
                reading: Reading::default(),
                updated_ms: 0,
            },
        );
    }
}

pub fn legacy_position(lib: &Library, path: &Path) -> Option<f64> {
    let value: serde_json::Value =
        serde_json::from_str(&lib.setting("last_video_position").ok()?).ok()?;
    (value["path"].as_str() == path.to_str())
        .then(|| value["position"].as_f64())
        .flatten()
        .filter(|position| position.is_finite() && *position >= 0.)
}

impl Default for Reading {
    fn default() -> Self {
        Self {
            tab: 0,
            transcript_open: false,
            follow: true,
            cue_start: None,
            active_start: None,
            cue_offset: 0.,
            transcript_item: None,
            transcript_cue_start: None,
            transcript_offset: 0.,
            transcript_revision: String::new(),
        }
    }
}

fn follow_default() -> bool {
    true
}

impl Reading {
    pub fn for_cues(
        tab: usize,
        transcript_open: bool,
        cues: &[crate::subtitles::Cue],
        position: f64,
        transcript_offset: f32,
    ) -> Self {
        let cue_start = cues
            .iter()
            .find(|cue| position < cue.end)
            .map(|cue| cue.start);
        Self {
            tab,
            transcript_open,
            follow: true,
            cue_start,
            active_start: cue_start,
            cue_offset: 0.,
            transcript_item: None,
            transcript_cue_start: None,
            transcript_offset,
            transcript_revision: String::new(),
        }
    }
}

pub fn validate_item(item: &Item) -> Result<()> {
    ensure!(
        item.position.is_finite() && item.position >= 0.,
        "视频进度无效"
    );
    ensure!(item.reading.tab < 4, "视频阅读页签无效");
    Ok(())
}
