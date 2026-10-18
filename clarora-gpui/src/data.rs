use anyhow::{Context, Result, bail, ensure};
use chrono::{Local, Utc};
use rusqlite::{Connection, params, types::Value as SqlValue};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub type Row = Map<String, Value>;
pub const TABLES: &[(&str, &[&str])] = &[
    ("words", &["id", "word", "meaning", "source", "created_at"]),
    ("listening_practices", &["id", "name", "created_at"]),
    (
        "listening_audios",
        &[
            "id",
            "practice_id",
            "name",
            "audio_uri",
            "subtitle_uri",
            "created_at",
        ],
    ),
    (
        "clip_cards",
        &[
            "id",
            "en_text",
            "zh_text",
            "audio_uri",
            "start_ms",
            "end_ms",
            "created_at",
        ],
    ),
    (
        "sentence_cards",
        &["id", "text", "translation", "notes", "created_at"],
    ),
    (
        "ai_cards",
        &["id", "question", "answer", "context_text", "created_at"],
    ),
    (
        "video_clips",
        &["id", "en_text", "zh_text", "video_uri", "created_at"],
    ),
    (
        "review_schedule",
        &[
            "card_kind",
            "card_id",
            "ease",
            "interval_days",
            "due_at",
            "reps",
            "lapses",
            "suspended",
            "last_graded_at",
            "created_at",
        ],
    ),
    (
        "study_stats",
        &["day", "listen_seconds", "review_count", "pomodoro_count"],
    ),
    ("discover_favorites", &["key", "payload"]),
    ("ai_records", &["id", "kind", "payload", "created_at"]),
];
pub fn text(row: &Row, key: &str) -> String {
    match row.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}
pub fn id() -> String {
    Uuid::new_v4().simple().to_string()
}
pub fn now() -> String {
    Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
}
fn sql(v: &Value) -> SqlValue {
    match v {
        Value::Null => SqlValue::Null,
        Value::Number(n) => n
            .as_i64()
            .map(SqlValue::Integer)
            .unwrap_or_else(|| SqlValue::Real(n.as_f64().unwrap_or(0.))),
        Value::String(s) => SqlValue::Text(s.clone()),
        _ => SqlValue::Text(v.to_string()),
    }
}
#[derive(Clone)]
pub struct Library {
    pub root: PathBuf,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Card {
    pub kind: String,
    pub id: String,
    pub front: String,
    pub back: String,
    pub media: String,
    pub start: f64,
    pub end: f64,
}
impl Library {
    pub fn open(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(&root)?;
        let this = Self {
            root: root.canonicalize()?,
        };
        let db = this.db()?;
        db.execute_batch(include_str!("schema.sql"))?;
        for dir in ["media", "pdf", "models", "downloads", "recordings"] {
            fs::create_dir_all(this.root.join(dir))?;
        }
        Ok(this)
    }
    pub fn db(&self) -> Result<Connection> {
        let db = Connection::open(self.root.join("clarora.db"))?;
        db.busy_timeout(std::time::Duration::from_secs(10))?;
        db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL;")?;
        Ok(db)
    }
    pub fn rows(&self, query: &str, values: &[SqlValue]) -> Result<Vec<Row>> {
        let db = self.db()?;
        let mut stmt = db.prepare(query)?;
        let cols = stmt
            .column_names()
            .iter()
            .map(|x| x.to_string())
            .collect::<Vec<_>>();
        let rows = stmt
            .query_map(rusqlite::params_from_iter(values), |r| {
                let mut row = Row::new();
                for (i, col) in cols.iter().enumerate() {
                    let v: SqlValue = r.get(i)?;
                    row.insert(
                        col.clone(),
                        match v {
                            SqlValue::Null => Value::Null,
                            SqlValue::Integer(n) => json!(n),
                            SqlValue::Real(n) => json!(n),
                            SqlValue::Text(s) => json!(s),
                            SqlValue::Blob(_) => Value::Null,
                        },
                    );
                }
                Ok(row)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
    pub fn list(&self, table: &str) -> Result<Vec<Row>> {
        ensure!(TABLES.iter().any(|(t, _)| *t == table), "无效资料表");
        self.rows(&format!("SELECT * FROM {table}"), &[])
    }
    pub fn setting(&self, key: &str) -> Result<String> {
        Ok(self
            .rows(
                "SELECT value FROM app_settings WHERE key=?",
                &[key.to_string().into()],
            )?
            .first()
            .map(|r| text(r, "value"))
            .unwrap_or_default())
    }
    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.db()?.execute("INSERT INTO app_settings(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,value])?;
        Ok(())
    }
    pub fn put(&self, table: &str, row: Value) -> Result<()> {
        insert(
            &self.db()?,
            table,
            row.as_object().context("记录格式错误")?,
            false,
        )
    }
    pub fn delete(&self, table: &str, key: &str) -> Result<()> {
        ensure!(TABLES.iter().any(|(t, _)| *t == table), "无效资料表");
        let mut db = self.db()?;
        let tx = db.transaction()?;
        tx.execute(
            &format!(
                "DELETE FROM {table} WHERE {}=?",
                if table == "discover_favorites" {
                    "key"
                } else {
                    "id"
                }
            ),
            [key],
        )?;
        let kind = match table {
            "words" => "word",
            "clip_cards" => "clip",
            "sentence_cards" => "sentence",
            "video_clips" => "video",
            "ai_cards" => "ai",
            _ => "",
        };
        tx.execute(
            "DELETE FROM review_schedule WHERE card_kind=? AND card_id=?",
            params![kind, key],
        )?;
        if !kind.is_empty() {
            tx.execute(
                "DELETE FROM discover_favorites WHERE key=?",
                [format!("{kind}:{key}")],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn copy_media(&self, path: &Path, folder: &str) -> Result<PathBuf> {
        ensure!(path.is_file(), "文件不存在");
        ensure!(["media", "pdf", "recordings"].contains(&folder), "无效目录");
        let ext = path.extension().and_then(|x| x.to_str()).unwrap_or("bin");
        ensure!(
            ext.len() < 16 && ext.chars().all(|c| c.is_ascii_alphanumeric()),
            "文件扩展名无效"
        );
        let dst = self.root.join(folder).join(format!("{}.{}", id(), ext));
        fs::copy(path, &dst)?;
        Ok(dst)
    }
    pub fn import_words(
        &self,
        content: &str,
        source: &str,
        replace_directory: bool,
    ) -> Result<usize> {
        let pairs = parse_words(content);
        ensure!(!pairs.is_empty(), "没有可导入的词条");
        let mut db = self.db()?;
        let tx = db.transaction()?;
        if replace_directory {
            tx.execute("DELETE FROM review_schedule WHERE card_kind='word' AND card_id IN (SELECT CAST(id AS TEXT) FROM words WHERE source='directory')",[])?;
            tx.execute("DELETE FROM words WHERE source='directory'", [])?;
        }
        let mut count = 0;
        for (front, back) in pairs {
            count+=tx.execute("INSERT INTO words(word,meaning,source) VALUES(?,?,?) ON CONFLICT(word) DO UPDATE SET meaning=excluded.meaning",params![front,back,source])?;
        }
        tx.commit()?;
        Ok(count)
    }
    pub fn import_directory(&self, path: &Path) -> Result<usize> {
        fn walk(p: &Path, out: &mut Vec<(String, String)>) -> Result<()> {
            for file in fs::read_dir(p)? {
                let file = file?;
                let path = file.path();
                if file.file_name().to_string_lossy().starts_with('.')
                    || file.file_type()?.is_symlink()
                {
                    continue;
                }
                if path.is_dir() {
                    walk(&path, out)?
                } else if let Ok(body) = fs::read_to_string(&path) {
                    if let Some(stem) = path.file_stem() {
                        if !body.trim().is_empty() {
                            out.push((stem.to_string_lossy().into(), body));
                        }
                    }
                }
            }
            Ok(())
        }
        let mut pairs = vec![];
        walk(path, &mut pairs)?;
        ensure!(!pairs.is_empty(), "目录没有可导入的文本");
        let mut db = self.db()?;
        let tx = db.transaction()?;
        let mut kept = vec![];
        for (front, back) in &pairs {
            tx.execute("INSERT INTO words(word,meaning,source) VALUES(?,?,'directory') ON CONFLICT(word) DO UPDATE SET meaning=excluded.meaning,source='directory'",params![front,back])?;
            kept.push(front.clone());
        }
        let existing: Vec<(i64, String)> = {
            let mut s = tx.prepare("SELECT id,word FROM words WHERE source='directory'")?;
            s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?
        };
        for (key, word) in existing {
            if !kept.contains(&word) {
                tx.execute(
                    "DELETE FROM review_schedule WHERE card_kind='word' AND card_id=?",
                    [key.to_string()],
                )?;
                tx.execute("DELETE FROM words WHERE id=?", [key])?;
            }
        }
        tx.commit()?;
        Ok(pairs.len())
    }
    pub fn cards(&self, due: bool) -> Result<Vec<Card>> {
        let mut cards = vec![];
        for (table, kind, front, back, media) in [
            ("words", "word", "word", "meaning", ""),
            ("sentence_cards", "sentence", "text", "translation", ""),
            ("ai_cards", "ai", "question", "answer", ""),
            ("clip_cards", "clip", "en_text", "zh_text", "audio_uri"),
            ("video_clips", "video", "en_text", "zh_text", "video_uri"),
        ] {
            for r in self.list(table)? {
                cards.push(Card {
                    kind: kind.into(),
                    id: text(&r, "id"),
                    front: text(&r, front),
                    back: text(&r, back),
                    media: text(&r, media),
                    start: r.get("start_ms").and_then(Value::as_f64).unwrap_or(0.) / 1000.,
                    end: r.get("end_ms").and_then(Value::as_f64).unwrap_or(0.) / 1000.,
                });
            }
        }
        if due {
            let schedules = self.list("review_schedule")?;
            let now = now();
            let mut new = 0;
            let limit = self
                .setting("new_cards_per_day")?
                .parse::<usize>()
                .unwrap_or(20);
            let introduced=self.rows("SELECT COUNT(*) AS n FROM review_schedule WHERE date(created_at,'localtime')=date('now','localtime')",&[])?.first().and_then(|r|r["n"].as_u64()).unwrap_or(0) as usize;
            cards.retain(|c| {
                if let Some(s) = schedules
                    .iter()
                    .find(|s| text(s, "card_kind") == c.kind && text(s, "card_id") == c.id)
                {
                    s["suspended"].as_i64() == Some(0) && text(s, "due_at") <= now
                } else {
                    new += 1;
                    introduced + new <= limit
                }
            });
        }
        Ok(cards)
    }
    pub fn update_card(&self, card: &Card, front: &str, back: &str) -> Result<()> {
        ensure!(!front.trim().is_empty(), "原文不能为空");
        let (table, a, b) = match card.kind.as_str() {
            "word" => ("words", "word", "meaning"),
            "sentence" => ("sentence_cards", "text", "translation"),
            "ai" => ("ai_cards", "question", "answer"),
            "clip" => ("clip_cards", "en_text", "zh_text"),
            "video" => ("video_clips", "en_text", "zh_text"),
            _ => bail!("未知卡片类型"),
        };
        let count = self.db()?.execute(
            &format!("UPDATE {table} SET {a}=?,{b}=? WHERE id=?"),
            params![front.trim(), back.trim(), card.id],
        )?;
        ensure!(count == 1, "卡片尚未保存，请先收藏后编辑");
        Ok(())
    }
    pub fn grade(&self, card: &Card, grade: &str) -> Result<String> {
        ensure!(["again", "hard", "good"].contains(&grade), "评分无效");
        let db = self.db()?;
        let old = self.rows(
            "SELECT * FROM review_schedule WHERE card_kind=? AND card_id=?",
            &[card.kind.clone().into(), card.id.clone().into()],
        )?;
        let number = |name: &str, default: f64| {
            old.first()
                .and_then(|r| r.get(name))
                .and_then(Value::as_f64)
                .unwrap_or(default)
        };
        let reps = number("reps", 0.);
        let ease = number("ease", 2.5);
        let interval = number("interval_days", 0.);
        let mut lapses = number("lapses", 0.);
        let (next_ease, days) = if grade == "again" {
            if reps > 0. {
                lapses += 1.;
            }
            (
                if reps == 0. {
                    2.5
                } else {
                    (ease - 0.2).max(1.3)
                },
                10. / 1440.,
            )
        } else if reps == 0. {
            (2.5, if grade == "good" { 4. } else { 1. })
        } else if grade == "hard" {
            ((ease - 0.15).max(1.3), (interval * 1.2).clamp(1., 730.))
        } else {
            ((ease + 0.1).min(3.2), (interval * ease).clamp(1., 730.))
        };
        let due = (Utc::now() + chrono::Duration::seconds((days * 86400.) as i64))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        db.execute("INSERT INTO review_schedule(card_kind,card_id,ease,interval_days,due_at,reps,lapses,last_graded_at) VALUES(?,?,?,?,?,?,?,?) ON CONFLICT(card_kind,card_id) DO UPDATE SET ease=excluded.ease,interval_days=excluded.interval_days,due_at=excluded.due_at,reps=excluded.reps,lapses=excluded.lapses,last_graded_at=excluded.last_graded_at",params![card.kind,card.id,next_ease,days,due,reps+1.,lapses,now()])?;
        self.stat("review_count", 1.)?;
        Ok(due)
    }
    pub fn stat(&self, field: &str, amount: f64) -> Result<()> {
        ensure!(
            ["listen_seconds", "review_count", "pomodoro_count"].contains(&field)
                && amount.is_finite()
                && amount >= 0.,
            "统计无效"
        );
        self.db()?.execute(&format!("INSERT INTO study_stats(day,{field}) VALUES(?,?) ON CONFLICT(day) DO UPDATE SET {field}={field}+excluded.{field}"),params![Local::now().format("%Y-%m-%d").to_string(),amount])?;
        Ok(())
    }
    pub fn practice(&self, name: &str) -> Result<String> {
        ensure!(!name.trim().is_empty(), "请输入练习组名称");
        let rows = self.rows(
            "SELECT id FROM listening_practices WHERE lower(name)=lower(?)",
            &[name.to_string().into()],
        )?;
        if let Some(row) = rows.first() {
            return Ok(text(row, "id"));
        }
        let key = id();
        self.put(
            "listening_practices",
            json!({"id":key,"name":name.trim(),"created_at":now()}),
        )?;
        Ok(key)
    }
    pub fn import_audio(&self, practice: &str, path: &Path) -> Result<()> {
        let name = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if !self
            .rows(
                "SELECT id FROM listening_audios WHERE practice_id=? AND name=?",
                &[practice.to_string().into(), name.clone().into()],
            )?
            .is_empty()
        {
            bail!("同组已存在同名音频")
        }
        let media = self.copy_media(path, "media")?;
        let sub = ["srt", "vtt", "txt"]
            .iter()
            .map(|ext| path.with_extension(ext))
            .find(|p| p.is_file());
        let subtitle = if let Some(p) = sub {
            self.copy_media(&p, "media")?.to_string_lossy().to_string()
        } else {
            String::new()
        };
        if let Err(e)=self.put("listening_audios",json!({"id":id(),"practice_id":practice,"name":name,"audio_uri":media.to_string_lossy(),"subtitle_uri":subtitle,"created_at":now()})){let _=fs::remove_file(media);return Err(e)}
        Ok(())
    }
    pub fn vault(&self) -> Result<BTreeMap<String, Vec<Row>>> {
        TABLES
            .iter()
            .map(|(t, _)| Ok((t.to_string(), self.list(t)?)))
            .collect()
    }
}
pub fn insert(db: &Connection, table: &str, row: &Row, ignore: bool) -> Result<()> {
    let allowed = TABLES
        .iter()
        .find(|(t, _)| *t == table)
        .context("资料表无效")?
        .1;
    ensure!(
        row.keys().all(|k| allowed.contains(&k.as_str())),
        "包含未知字段"
    );
    let cols = row.keys().map(String::as_str).collect::<Vec<_>>();
    let vars = vec!["?"; cols.len()].join(",");
    db.execute(
        &format!(
            "INSERT {} INTO {table} ({}) VALUES ({vars})",
            if ignore { "OR IGNORE" } else { "" },
            cols.join(",")
        ),
        rusqlite::params_from_iter(row.values().map(sql)),
    )?;
    Ok(())
}
pub fn parse_words(content: &str) -> Vec<(String, String)> {
    content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            for sep in ["\t", ",", "，", " - ", " : ", "："] {
                if let Some((a, b)) = line.split_once(sep) {
                    if !a.trim().is_empty() && !b.trim().is_empty() {
                        return Some((a.trim().into(), b.trim().into()));
                    }
                }
            }
            None
        })
        .collect()
}
