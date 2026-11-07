//! Install the existing revision journal and triggers when opening a library.
use super::schema::{PREFERENCES, SETTINGS, TABLES};
use anyhow::Result;

pub(super) fn install(db: &rusqlite::Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS cloud_sync_identity(singleton INTEGER PRIMARY KEY CHECK(singleton=1),device_id TEXT NOT NULL,last_ms INTEGER NOT NULL DEFAULT 0,sequence INTEGER NOT NULL DEFAULT 0,applying INTEGER NOT NULL DEFAULT 0,tracking INTEGER NOT NULL DEFAULT 0);
         INSERT OR IGNORE INTO cloud_sync_identity(singleton,device_id) VALUES(1,lower(hex(randomblob(16))));
         CREATE TABLE IF NOT EXISTS cloud_sync_journal(id INTEGER PRIMARY KEY AUTOINCREMENT,table_name TEXT NOT NULL,record_key TEXT NOT NULL,modified_ms INTEGER NOT NULL,sequence INTEGER NOT NULL,deleted INTEGER NOT NULL,payload TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS cloud_sync_revisions(table_name TEXT NOT NULL,record_key TEXT NOT NULL,modified_ms INTEGER NOT NULL,sequence INTEGER NOT NULL,deleted INTEGER NOT NULL,payload TEXT NOT NULL,PRIMARY KEY(table_name,record_key));
         CREATE TABLE IF NOT EXISTS cloud_sync_entities(scope TEXT NOT NULL,entity_key TEXT NOT NULL,value TEXT NOT NULL,local_sequence INTEGER NOT NULL,PRIMARY KEY(scope,entity_key));
         CREATE TABLE IF NOT EXISTS cloud_sync_status(scope TEXT PRIMARY KEY,last_success TEXT NOT NULL,last_error TEXT NOT NULL DEFAULT '');"
    )?;
    for spec in TABLES.iter().chain([&SETTINGS]) {
        let table = spec.name;
        let columns = spec.columns;
        for (name, operation, row) in [
            ("insert", "INSERT", "NEW"),
            ("update", "UPDATE", "NEW"),
            ("delete", "DELETE", "OLD"),
        ] {
            let predicate = if table == "app_settings" {
                let exact = PREFERENCES
                    .iter()
                    .copied()
                    .chain(["video_favorites_v1"])
                    .map(|key| format!("'{key}'"))
                    .collect::<Vec<_>>()
                    .join(",");
                format!(
                    "AND ({row}.key IN ({exact}) OR {row}.key LIKE 'reader_progress_%' OR {row}.key LIKE 'key_points_%' OR {row}.key LIKE 'video_subtitles_%')"
                )
            } else {
                String::new()
            };
            let payload = |r: &str| {
                format!(
                    "json_object({})",
                    columns
                        .iter()
                        .map(|c| format!("'{c}',{r}.\"{c}\""))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            };
            let key = |r: &str| {
                format!(
                    "json_array({})",
                    spec.sync_key
                        .iter()
                        .map(|c| format!("{r}.\"{c}\""))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            };
            let statement = |r: &str, deleted: bool, condition: &str| {
                format!(
                "INSERT INTO cloud_sync_journal(table_name,record_key,modified_ms,sequence,deleted,payload) SELECT '{table}',{},last_ms,sequence,{},{} FROM cloud_sync_identity WHERE singleton=1 {condition};
                 INSERT INTO cloud_sync_revisions(table_name,record_key,modified_ms,sequence,deleted,payload) SELECT '{table}',{},last_ms,sequence,{},{} FROM cloud_sync_identity WHERE singleton=1 {condition} ON CONFLICT(table_name,record_key) DO UPDATE SET modified_ms=excluded.modified_ms,sequence=excluded.sequence,deleted=excluded.deleted,payload=excluded.payload;",
                key(r),u8::from(deleted),payload(r),key(r),u8::from(deleted),payload(r))
            };
            let mut body = String::from(
                "UPDATE cloud_sync_identity SET last_ms=MAX(last_ms,CAST(strftime('%s','now') AS INTEGER)*1000+CAST(substr(strftime('%f','now'),4,3) AS INTEGER)),sequence=sequence+1 WHERE singleton=1;",
            );
            if name == "update" {
                body.push_str(&statement(
                    "OLD",
                    true,
                    &format!("AND {} != {}", key("OLD"), key("NEW")),
                ));
            }
            body.push_str(&statement(row, name == "delete", ""));
            db.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS cloud_sync_{table}_{name} AFTER {operation} ON {table} WHEN (SELECT applying=0 AND tracking=1 FROM cloud_sync_identity WHERE singleton=1) {predicate} BEGIN {body} END;"))?;
        }
    }
    Ok(())
}
