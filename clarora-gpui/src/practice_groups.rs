//! Persisted group records used by independent practice libraries.
use crate::data::{Library, id};
use anyhow::{Context, Result, ensure};
use rusqlite::{TransactionBehavior, params};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

pub(crate) fn list(lib: &Library, kind: &str) -> Result<Vec<Group>> {
    let db = lib.db()?;
    let mut query =
        db.prepare("SELECT id,payload FROM ai_records WHERE kind=?1 ORDER BY created_at DESC,id")?;
    let rows = query.query_map([kind], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    rows.map(|row| {
        let (id, payload) = row?;
        let group: Group = serde_json::from_str(&payload).context("无法读取练习分组")?;
        ensure!(group.id == id, "练习分组标识不一致");
        Ok(group)
    })
    .collect()
}

pub(crate) fn create(lib: &Library, kind: &str, name: &str) -> Result<Group> {
    let name = name.trim();
    ensure!(
        !name.is_empty() && name.chars().count() <= 64 && !name.chars().any(char::is_control),
        "请输入 1 到 64 个字符的分组名称"
    );
    let mut db = lib.db()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let duplicate: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM ai_records WHERE kind=?1 AND lower(json_extract(payload,'$.name'))=lower(?2))",
        [kind, name], |row| row.get(0),
    )?;
    ensure!(!duplicate, "已有同名分组");
    let group = Group {
        id: id(),
        name: name.to_owned(),
        tags: Vec::new(),
    };
    tx.execute(
        "INSERT INTO ai_records(id,kind,payload,created_at) VALUES(?,?,?,?)",
        params![
            group.id,
            kind,
            serde_json::to_string(&group)?,
            chrono::Utc::now()
                .format("%Y-%m-%d %H:%M:%S%.9f")
                .to_string()
        ],
    )?;
    tx.commit()?;
    Ok(group)
}
