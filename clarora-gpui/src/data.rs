use anyhow::{Context, Result, bail, ensure};
use chrono::{Local, Utc};
use rusqlite::{
    Connection, OptionalExtension, TransactionBehavior, params, types::Value as SqlValue,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub type Row = Map<String, Value>;
const CARD_KINDS: [&str; 5] = ["word", "sentence", "ai", "clip", "video"];
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
        query_rows(&db, query, values)
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
        self.cards_for_kind(due, "all")
    }
    pub fn cards_for_kind(&self, due: bool, kind: &str) -> Result<Vec<Card>> {
        ensure!(kind == "all" || CARD_KINDS.contains(&kind), "未知卡片类型");
        let mut cards = vec![];
        for (table, card_kind, front, back, media) in [
            ("words", "word", "word", "meaning", ""),
            ("sentence_cards", "sentence", "text", "translation", ""),
            ("ai_cards", "ai", "question", "answer", ""),
            ("clip_cards", "clip", "en_text", "zh_text", "audio_uri"),
            ("video_clips", "video", "en_text", "zh_text", "video_uri"),
        ] {
            if kind != "all" && kind != card_kind {
                continue;
            }
            for r in self.rows(
                &format!("SELECT * FROM {table} ORDER BY created_at,id"),
                &[],
            )? {
                cards.push(Card {
                    kind: card_kind.into(),
                    id: text(&r, "id"),
                    front: text(&r, front),
                    back: text(&r, back),
                    media: text(&r, media),
                    start: r.get("start_ms").and_then(Value::as_f64).unwrap_or(0.) / 1000.,
                    end: r.get("end_ms").and_then(Value::as_f64).unwrap_or(0.) / 1000.,
                });
            }
        }
        if !due {
            return Ok(cards);
        }
        let schedules = self
            .list("review_schedule")?
            .into_iter()
            .map(|schedule| {
                (
                    (text(&schedule, "card_kind"), text(&schedule, "card_id")),
                    schedule,
                )
            })
            .collect::<BTreeMap<_, _>>();
        let now = now();
        let limit = self
            .setting("new_cards_per_day")?
            .parse::<usize>()
            .unwrap_or(20);
        let introduced: usize = self.db()?.query_row(
            "SELECT COUNT(*) FROM review_schedule WHERE reps>0 AND date(created_at,'localtime')=date('now','localtime')",
            [],
            |row| row.get(0),
        )?;
        let mut remaining = limit.saturating_sub(introduced);
        let mut scheduled = vec![];
        let mut new_cards: [VecDeque<Card>; 5] = std::array::from_fn(|_| VecDeque::new());
        for card in cards {
            if let Some(schedule) = schedules.get(&(card.kind.clone(), card.id.clone())) {
                if schedule["suspended"].as_i64() != Some(0) {
                    continue;
                }
                if schedule["reps"].as_i64().unwrap_or(0) > 0 {
                    let due_at = text(schedule, "due_at");
                    if due_at <= now {
                        scheduled.push((due_at, card));
                    }
                    continue;
                }
            }
            // A schedule created only by suspend/resume is still an unintroduced card.
            let index = CARD_KINDS
                .iter()
                .position(|kind| *kind == card.kind)
                .unwrap();
            new_cards[index].push_back(card);
        }
        scheduled.sort_by(|(a_due, a), (b_due, b)| {
            (a_due, &a.kind, &a.id).cmp(&(b_due, &b.kind, &b.id))
        });
        let mut cards = scheduled
            .into_iter()
            .map(|(_, card)| card)
            .collect::<Vec<_>>();
        let next_kind = self.setting("review_new_card_next_kind")?;
        let start = CARD_KINDS
            .iter()
            .position(|kind| *kind == next_kind)
            .unwrap_or(0);
        while remaining > 0 {
            let mut added = false;
            for offset in 0..CARD_KINDS.len() {
                if remaining == 0 {
                    break;
                }
                if let Some(card) = new_cards[(start + offset) % CARD_KINDS.len()].pop_front() {
                    cards.push(card);
                    remaining -= 1;
                    added = true;
                }
            }
            if !added {
                break;
            }
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
        let mut db = self.db()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (reps, ease, interval, mut lapses) = tx
            .query_row(
                "SELECT reps,ease,interval_days,lapses FROM review_schedule WHERE card_kind=? AND card_id=?",
                params![card.kind, card.id],
                |row| Ok((row.get::<_, f64>(0)?, row.get::<_, f64>(1)?, row.get::<_, f64>(2)?, row.get::<_, f64>(3)?)),
            )
            .optional()?
            .unwrap_or((0., 2.5, 0., 0.));
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
        let graded_at = Utc::now();
        let due = (graded_at + chrono::Duration::seconds((days * 86400.) as i64))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let timestamp = graded_at.format("%Y-%m-%d %H:%M:%S").to_string();
        tx.execute("INSERT INTO review_schedule(card_kind,card_id,ease,interval_days,due_at,reps,lapses,last_graded_at,created_at) VALUES(?,?,?,?,?,?,?,?,?) ON CONFLICT(card_kind,card_id) DO UPDATE SET ease=excluded.ease,interval_days=excluded.interval_days,due_at=excluded.due_at,reps=excluded.reps,lapses=excluded.lapses,last_graded_at=excluded.last_graded_at,created_at=CASE WHEN review_schedule.reps=0 THEN excluded.created_at ELSE review_schedule.created_at END",params![card.kind,card.id,next_ease,days,due,reps+1.,lapses,timestamp,timestamp])?;
        tx.execute("INSERT INTO study_stats(day,review_count) VALUES(?,1) ON CONFLICT(day) DO UPDATE SET review_count=review_count+1",[graded_at.with_timezone(&Local).format("%Y-%m-%d").to_string()])?;
        if reps == 0. {
            if let Some(index) = CARD_KINDS.iter().position(|kind| *kind == card.kind) {
                tx.execute("INSERT INTO app_settings(key,value) VALUES('review_new_card_next_kind',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[CARD_KINDS[(index + 1) % CARD_KINDS.len()]])?;
            }
        }
        tx.commit()?;
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
        Ok(self.vault_with_settings()?.0)
    }
    /// Tables and setting metadata belong to one WAL read snapshot. Callers must
    /// explicitly whitelist any settings they publish; credentials are never tables here.
    pub(crate) fn vault_with_settings(
        &self,
    ) -> Result<(BTreeMap<String, Vec<Row>>, BTreeMap<String, String>)> {
        let mut db = self.db()?;
        let tx = db.transaction()?;
        let data = TABLES
            .iter()
            .map(|(table, _)| {
                Ok((
                    table.to_string(),
                    query_rows(&tx, &format!("SELECT * FROM {table}"), &[])?,
                ))
            })
            .collect::<Result<_>>()?;
        let settings = query_rows(&tx, "SELECT key,value FROM app_settings", &[])?
            .into_iter()
            .map(|row| (text(&row, "key"), text(&row, "value")))
            .collect();
        tx.commit()?;
        Ok((data, settings))
    }
}
fn query_rows(db: &Connection, query: &str, values: &[SqlValue]) -> Result<Vec<Row>> {
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
