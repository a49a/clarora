use super::{Library, Row, column_text, id, now};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::{collections::BTreeSet, fs, path::Path};

pub(crate) struct Practice {
    pub id: String,
    pub name: String,
    created_at: String,
}

pub(crate) struct AudioRecord {
    pub id: String,
    pub practice_id: String,
    pub name: String,
    pub audio_uri: String,
    pub subtitle_uri: String,
    created_at: String,
}

impl Practice {
    fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: column_text(row, 0)?,
            name: column_text(row, 1)?,
            created_at: column_text(row, 2)?,
        })
    }

    pub fn into_row(self) -> Row {
        Row::from_iter([
            ("id".into(), self.id.into()),
            ("name".into(), self.name.into()),
            ("created_at".into(), self.created_at.into()),
        ])
    }
}

impl AudioRecord {
    fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: column_text(row, 0)?,
            practice_id: column_text(row, 1)?,
            name: column_text(row, 2)?,
            audio_uri: column_text(row, 3)?,
            subtitle_uri: column_text(row, 4)?,
            created_at: column_text(row, 5)?,
        })
    }

    pub fn into_row(self) -> Row {
        Row::from_iter([
            ("id".into(), self.id.into()),
            ("practice_id".into(), self.practice_id.into()),
            ("name".into(), self.name.into()),
            ("audio_uri".into(), self.audio_uri.into()),
            ("subtitle_uri".into(), self.subtitle_uri.into()),
            ("created_at".into(), self.created_at.into()),
        ])
    }
}

fn ensure_available(db: &Connection, group: &str, name: &str) -> Result<()> {
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM listening_audios WHERE practice_id=? AND name=?)",
        [group, name],
        |row| row.get(0),
    )?;
    ensure!(!exists, "同组已存在同名音频");
    Ok(())
}

impl Library {
    pub(crate) fn audio_catalog(&self) -> Result<(Vec<Practice>, Vec<AudioRecord>)> {
        let mut db = self.db()?;
        let tx = db.transaction()?;
        let practices = tx
            .prepare("SELECT id,name,created_at FROM listening_practices")?
            .query_map([], Practice::read)?
            .collect::<rusqlite::Result<_>>()?;
        let audios = tx.prepare("SELECT id,practice_id,name,audio_uri,subtitle_uri,created_at FROM listening_audios")?
            .query_map([], AudioRecord::read)?
            .collect::<rusqlite::Result<_>>()?;
        tx.commit()?;
        Ok((practices, audios))
    }

    pub(crate) fn audio_practice(&self, key: &str) -> Result<Option<Practice>> {
        Ok(self
            .db()?
            .query_row(
                "SELECT id,name,created_at FROM listening_practices WHERE id=?",
                [key],
                Practice::read,
            )
            .optional()?)
    }

    pub fn practice(&self, name: &str) -> Result<String> {
        ensure!(!name.trim().is_empty(), "请输入练习组名称");
        let mut db = self.db()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing = tx
            .query_row(
                "SELECT id FROM listening_practices WHERE lower(name)=lower(?)",
                [name],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(key) = existing {
            return Ok(key);
        }
        let key = id();
        tx.execute(
            "INSERT INTO listening_practices (id,name,created_at) VALUES (?,?,?)",
            params![key, name.trim(), now()],
        )?;
        tx.commit()?;
        Ok(key)
    }

    pub fn import_audio(&self, practice: &str, path: &Path) -> Result<()> {
        self.import_audio_record(practice, path).map(|_| ())
    }

    pub(crate) fn import_audio_record(&self, practice: &str, path: &Path) -> Result<AudioRecord> {
        let name = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let mut db = self.db()?;
        ensure_available(&db, practice, &name)?;
        let media = self.copy_media(path, "media")?;
        let mut subtitle = None;
        let result = (|| {
            if let Some(path) = ["srt", "vtt", "txt"]
                .iter()
                .map(|ext| path.with_extension(ext))
                .find(|path| path.is_file())
            {
                subtitle = Some(self.copy_media(&path, "media")?);
            }
            let audio = AudioRecord {
                id: id(),
                practice_id: practice.into(),
                name,
                audio_uri: media.to_string_lossy().into_owned(),
                subtitle_uri: subtitle
                    .as_ref()
                    .map(|path| path.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                created_at: now(),
            };
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
            ensure_available(&tx, practice, &audio.name)?;
            tx.execute(
                "INSERT INTO listening_audios (id,practice_id,name,audio_uri,subtitle_uri,created_at) VALUES (?,?,?,?,?,?)",
                params![audio.id, audio.practice_id, audio.name, audio.audio_uri, audio.subtitle_uri, audio.created_at],
            )?;
            tx.commit()?;
            Ok(audio)
        })();
        if result.is_err() {
            let _ = fs::remove_file(media);
            if let Some(path) = subtitle {
                let _ = fs::remove_file(path);
            }
        }
        result
    }

    pub fn rename_practice(&self, key: &str, name: &str) -> Result<()> {
        self.rename("listening_practices", key, name, None)
    }

    pub fn rename_audio(&self, key: &str, name: &str, group: Option<&str>) -> Result<()> {
        self.rename("listening_audios", key, name, group)
    }

    fn rename(
        &self,
        table: &str,
        key: &str,
        name: &str,
        expected_group: Option<&str>,
    ) -> Result<()> {
        let name = name.trim();
        ensure!(
            !name.is_empty() && name.chars().count() <= 120,
            "名称需为 1–120 字"
        );
        let mut db = self.db()?;
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
        if let Some(expected) = expected_group {
            ensure!(group == expected, "音频已移动，请重新选择音频");
        }
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
    fn validate_targets(
        tx: &rusqlite::Transaction<'_>,
        source: &str,
        ids: &[String],
    ) -> Result<()> {
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
    pub fn move_audio_records(
        &self,
        source: &str,
        destination: &str,
        ids: &[String],
    ) -> Result<()> {
        ensure!(source != destination, "请选择另一个练习组");
        let mut db = self.db()?;
        let tx = db.transaction()?;
        Self::validate_targets(&tx, source, ids)?;
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
    pub fn delete_audio_records(&self, source: &str, ids: &[String]) -> Result<()> {
        let mut db = self.db()?;
        let tx = db.transaction()?;
        Self::validate_targets(&tx, source, ids)?;
        for key in ids {
            tx.execute("DELETE FROM listening_audios WHERE id=?", [key])?;
        }
        tx.commit()?;
        Ok(())
    }
}
