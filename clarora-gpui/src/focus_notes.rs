//! Quick notes captured during focus so thoughts survive the session.
use crate::data::{Library, id};
use anyhow::{Result, ensure};
use rusqlite::params;
use serde::Serialize;

pub const MAX_CHARS: usize = 20_000;

#[derive(Clone, Serialize)]
pub struct Note {
    pub id: String,
    pub created_ms: u64,
    pub body: String,
}

/// Save one note verbatim apart from outer whitespace. Notes are independent of
/// the timer transaction so a pause, interruption or app restart cannot lose them.
pub fn save(lib: &Library, body: &str, now: u64) -> Result<Note> {
    let body = body.trim();
    ensure!(!body.is_empty(), "先写下一点内容再保存");
    ensure!(
        body.chars().count() <= MAX_CHARS,
        "单条想法最多 {MAX_CHARS} 个字符"
    );
    let note = Note {
        id: id(),
        created_ms: now,
        body: body.to_owned(),
    };
    lib.db()?.execute(
        "INSERT INTO focus_notes(id,created_ms,body) VALUES(?,?,?)",
        params![note.id, note.created_ms, note.body],
    )?;
    Ok(note)
}

pub fn list(lib: &Library, limit: u32) -> Result<Vec<Note>> {
    let db = lib.db()?;
    let mut query = db.prepare(
        "SELECT id,created_ms,body FROM focus_notes ORDER BY created_ms DESC,rowid DESC LIMIT ?1",
    )?;
    let rows = query.query_map([limit], |row| {
        Ok(Note {
            id: row.get(0)?,
            created_ms: row.get(1)?,
            body: row.get(2)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn delete(lib: &Library, note: &str) -> Result<()> {
    ensure!(
        lib.db()?
            .execute("DELETE FROM focus_notes WHERE id=?", [note])?
            == 1,
        "想法已删除"
    );
    Ok(())
}
