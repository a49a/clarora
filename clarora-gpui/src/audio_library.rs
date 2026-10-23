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
}
impl State {
    pub fn sync(&mut self, group: &str, rows: &[Row]) {
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
        if let Some(row) = rows.iter().find(|r| text(r, "id") == self.selected) {
            if let Ok(body) = fs::read_to_string(text(row, "subtitle_uri")) {
                let cues = subtitles::parse(&body);
                self.subtitle_count = cues.len();
                self.preview = cues.into_iter().take(5).collect();
            }
        }
    }
    pub fn visible<'a>(&self, rows: &'a [Row], group: &str, query: &str) -> Vec<&'a Row> {
        let query = query.trim().to_lowercase();
        let mut rows: Vec<_> = rows
            .iter()
            .filter(|r| {
                text(r, "practice_id") == group
                    && text(r, "name").to_lowercase().contains(&query)
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
}
pub fn scoped_ids(rows: &[Row], group: &str, checked: &BTreeSet<String>) -> Vec<String> {
    rows.iter()
        .filter(|r| text(r, "practice_id") == group && checked.contains(&text(r, "id")))
        .map(|r| text(r, "id"))
        .collect()
}
pub fn rename(lib: &Library, table: &str, key: &str, name: &str) -> Result<()> {
    ensure!(
        ["listening_practices", "listening_audios"].contains(&table),
        "不支持的名称操作"
    );
    let name = name.trim();
    ensure!(
        !name.is_empty() && name.chars().count() <= 120,
        "名称需为 1–120 字"
    );
    let mut db = lib.db()?;
    let tx = db.transaction()?;
    let group: String = if table == "listening_audios" {
        tx.query_row(
            "SELECT practice_id FROM listening_audios WHERE id=?",
            [key],
            |r| r.get(0),
        )
        .context("音频不存在")?
    } else {
        String::new()
    };
    let duplicate: i64 = if table == "listening_audios" {
        tx.query_row("SELECT count(*) FROM listening_audios WHERE practice_id=? AND lower(trim(name))=lower(?) AND id<>?",[group.as_str(),name,key],|r|r.get(0))?
    } else {
        tx.query_row(
            "SELECT count(*) FROM listening_practices WHERE lower(trim(name))=lower(?) AND id<>?",
            [name, key],
            |r| r.get(0),
        )?
    };
    ensure!(duplicate == 0, "已存在同名条目，请换一个名称");
    ensure!(
        tx.execute(
            &format!("UPDATE {table} SET name=? WHERE id=?"),
            [name, key]
        )? == 1,
        "条目已不存在"
    );
    tx.commit()?;
    Ok(())
}
fn validate_targets(tx: &rusqlite::Transaction<'_>, source: &str, ids: &[String]) -> Result<()> {
    ensure!(!ids.is_empty(), "请先选择音频");
    ensure!(
        ids.iter().collect::<BTreeSet<_>>().len() == ids.len(),
        "音频选择重复"
    );
    for key in ids {
        let count: i64 = tx.query_row(
            "SELECT count(*) FROM listening_audios WHERE id=? AND practice_id=?",
            [key.as_str(), source],
            |r| r.get(0),
        )?;
        ensure!(count == 1, "选择的音频或所属组已改变，请重新选择");
    }
    Ok(())
}
pub fn move_records(lib: &Library, source: &str, destination: &str, ids: &[String]) -> Result<()> {
    ensure!(source != destination, "请选择另一个练习组");
    let mut db = lib.db()?;
    let tx = db.transaction()?;
    validate_targets(&tx, source, ids)?;
    let exists: i64 = tx.query_row(
        "SELECT count(*) FROM listening_practices WHERE id=?",
        [destination],
        |r| r.get(0),
    )?;
    ensure!(exists == 1, "目标练习组已不存在");
    for key in ids {
        let name: String =
            tx.query_row("SELECT name FROM listening_audios WHERE id=?", [key], |r| {
                r.get(0)
            })?;
        let duplicate: i64 = tx.query_row("SELECT count(*) FROM listening_audios WHERE practice_id=? AND lower(trim(name))=lower(trim(?))",[destination,name.as_str()],|r|r.get(0))?;
        ensure!(duplicate == 0, "目标组已存在「{name}」，未移动任何音频");
        tx.execute(
            "UPDATE listening_audios SET practice_id=? WHERE id=?",
            [destination, key],
        )?;
    }
    tx.commit()?;
    Ok(())
}
pub fn delete_records(lib: &Library, source: &str, ids: &[String]) -> Result<()> {
    let mut db = lib.db()?;
    let tx = db.transaction()?;
    validate_targets(&tx, source, ids)?;
    for key in ids {
        tx.execute("DELETE FROM listening_audios WHERE id=?", [key])?;
    }
    tx.commit()?;
    Ok(())
}
