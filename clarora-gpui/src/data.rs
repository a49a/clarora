use anyhow::{Context, Result, ensure};
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
mod audio;
mod cards;
pub(crate) mod schema;
mod tracking;
pub use schema::CardKind;
use schema::{TABLES, table_spec};

pub fn text(row: &Row, key: &str) -> String {
    match row.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}
// SQLite text columns in older libraries can contain numeric values or NULL.
fn column_text(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<String> {
    use rusqlite::types::ValueRef;
    Ok(match row.get_ref(index)? {
        ValueRef::Text(bytes) => std::str::from_utf8(bytes)
            .map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    index,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?
            .to_owned(),
        ValueRef::Integer(value) => value.to_string(),
        ValueRef::Real(value) => serde_json::Number::from_f64(value)
            .map(|value| value.to_string())
            .unwrap_or_default(),
        _ => String::new(),
    })
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
        tracking::install(&db)?;
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
        ensure!(table_spec(table).is_some(), "无效资料表");
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
        let spec = table_spec(table).context("无效资料表")?;
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
        let kind = spec.card.map_or("", CardKind::as_str);
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
        if let Err(error) = fs::copy(path, &dst) {
            let _ = fs::remove_file(&dst);
            return Err(error.into());
        }
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
            .map(|spec| {
                let table = spec.name;
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
pub(crate) fn query_rows(db: &Connection, query: &str, values: &[SqlValue]) -> Result<Vec<Row>> {
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
    let allowed = table_spec(table).context("资料表无效")?.columns;
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
