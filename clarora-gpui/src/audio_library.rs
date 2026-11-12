//! Audio management state is separate from the currently playing material.
use crate::{
    data::{Library, Row, text},
    subtitles::{self, Cue},
};
use anyhow::{Context, Result, ensure};
use chrono::NaiveDate;
use std::{collections::BTreeSet, fs, path::Path};

/// Suggest an unused local-date name without creating a practice group.
pub fn default_practice_name(practices: &[Row], date: NaiveDate) -> String {
    let names: BTreeSet<_> = practices
        .iter()
        .map(|row| text(row, "name").trim().to_string())
        .collect();
    let base = date.format("%Y-%m-%d").to_string();
    let mut name = base.clone();
    let mut suffix = 2;
    while names.contains(&name) {
        name = format!("{base} ({suffix})");
        suffix += 1;
    }
    name
}

#[derive(Default)]
pub struct State {
    pub selected: String,
    pub renaming: Option<RenameTarget>,
    /// all / attached subtitle / missing subtitle
    pub filter: u8,
    /// newest / name / oldest
    pub sort: u8,
    pub tool: String,
    pub batch: bool,
    pub pending_delete: Vec<String>,
    pub preview: Vec<Cue>,
    pub subtitle_count: usize,
    pub subtitle_ids: BTreeSet<String>,
    pub merges: crate::audio_tools::Catalog,
    pub show_sources: bool,
}

pub struct RenameTarget {
    pub id: String,
    pub group: String,
}

impl State {
    pub fn begin_rename(&mut self, group: &str, rows: &[Row], key: &str) -> Result<String> {
        let row = rows
            .iter()
            .find(|r| text(r, "id") == key && text(r, "practice_id") == group)
            .context("音频已不存在")?;
        self.selected = key.to_string();
        self.renaming = Some(RenameTarget {
            id: key.to_string(),
            group: group.to_string(),
        });
        Ok(text(row, "name"))
    }

    pub fn cancel_rename(&mut self) {
        self.renaming = None;
    }

    pub fn save_rename(&mut self, lib: &Library, group: &str, name: &str) -> Result<()> {
        let target = self.renaming.as_ref().context("请先双击音频名称")?;
        ensure!(target.group == group, "练习组已切换，请重新选择音频");
        lib.rename_audio(&target.id, name, Some(group))?;
        self.renaming = None;
        Ok(())
    }

    pub fn sync(&mut self, group: &str, rows: &[Row]) {
        if self.renaming.as_ref().is_some_and(|target| {
            target.group != group
                || !rows
                    .iter()
                    .any(|r| text(r, "id") == target.id && text(r, "practice_id") == target.group)
        }) {
            self.cancel_rename();
        }
        self.subtitle_ids = rows
            .iter()
            .filter(|r| Path::new(&text(r, "subtitle_uri")).is_file())
            .map(|r| text(r, "id"))
            .collect();
        if !rows
            .iter()
            .any(|r| text(r, "id") == self.selected && text(r, "practice_id") == group)
        {
            self.selected = self
                .visible(rows, group, "")
                .first()
                .map(|r| text(r, "id"))
                .unwrap_or_default();
        }
        self.preview.clear();
        self.subtitle_count = 0;
        if let Some(row) = rows.iter().find(|r| text(r, "id") == self.selected)
            && let Ok(body) = fs::read_to_string(text(row, "subtitle_uri"))
        {
            let cues = subtitles::parse(&body);
            self.subtitle_count = cues.len();
            self.preview = cues.into_iter().take(5).collect();
        }
    }
    pub fn visible<'a>(&self, rows: &'a [Row], group: &str, query: &str) -> Vec<&'a Row> {
        let query = query.trim().to_lowercase();
        let mut rows: Vec<_> = rows
            .iter()
            .filter(|r| {
                text(r, "practice_id") == group
                    && text(r, "name").to_lowercase().contains(&query)
                    && (self.show_sources || !self.merges.hidden(&text(r, "id")))
                    && match self.filter {
                        1 => self.subtitle_ids.contains(&text(r, "id")),
                        2 => !self.subtitle_ids.contains(&text(r, "id")),
                        _ => true,
                    }
            })
            .collect();
        rows.sort_by(|a, b| {
            let order = match self.sort {
                1 => text(a, "name")
                    .to_lowercase()
                    .cmp(&text(b, "name").to_lowercase()),
                2 => text(a, "created_at").cmp(&text(b, "created_at")),
                _ => text(b, "created_at").cmp(&text(a, "created_at")),
            };
            order.then_with(|| text(a, "id").cmp(&text(b, "id")))
        });
        rows
    }
    /// Checked rows keep the current sort even when search/filter hides a checked item.
    pub fn ordered_checked(
        &self,
        rows: &[Row],
        group: &str,
        checked: &BTreeSet<String>,
    ) -> Vec<Row> {
        let selection = Self {
            sort: self.sort,
            show_sources: true,
            ..Default::default()
        };
        selection
            .visible(rows, group, "")
            .into_iter()
            .filter(|r| checked.contains(&text(r, "id")))
            .cloned()
            .collect()
    }
}
pub fn scoped_ids(rows: &[Row], group: &str, checked: &BTreeSet<String>) -> Vec<String> {
    rows.iter()
        .filter(|r| text(r, "practice_id") == group && checked.contains(&text(r, "id")))
        .map(|r| text(r, "id"))
        .collect()
}
pub fn rename(lib: &Library, table: &str, key: &str, name: &str) -> Result<()> {
    match table {
        "listening_practices" => lib.rename_practice(key, name),
        "listening_audios" => lib.rename_audio(key, name, None),
        _ => anyhow::bail!("不支持的名称操作"),
    }
}

pub fn move_records(lib: &Library, source: &str, destination: &str, ids: &[String]) -> Result<()> {
    lib.move_audio_records(source, destination, ids)
}

pub fn delete_records(lib: &Library, source: &str, ids: &[String]) -> Result<()> {
    lib.delete_audio_records(source, ids)
}
