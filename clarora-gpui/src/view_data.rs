//! Explicitly loaded, last-known-good data for otherwise I/O-free UI rendering.
use crate::{
    ai, backup, credentials,
    data::{Library, Row, text},
    tts,
};
use anyhow::Result;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
    sync::OnceLock,
};

#[derive(Default)]
pub struct State {
    pub speaking: Vec<Value>,
    pub ocr: Vec<Value>,
    pub stats: Vec<Row>,
    /// Actual focus-round minutes keyed by local day.
    pub focus_by_day: BTreeMap<String, f64>,
    /// All finished focus rounds, used for the distraction-interval stat.
    pub focus_minutes: f64,
    pub focus_distractions: u64,
    pub words: Vec<Row>,
    pub document_files: Vec<PathBuf>,
    pub available_audio: BTreeSet<String>,
    pub asr_models: BTreeSet<String>,
    pub tts_ready: bool,
    pub saved_ai: Option<ai::Config>,
    pub ai_profiles: Vec<ai::Profile>,
    pub active_ai_profile: String,
    pub storage_profiles: Vec<backup::StorageProfile>,
    pub active_storage_profile: String,
    pub needs_legacy_import: bool,
    pub error: String,
    pub revision: u64,
}

fn records(lib: &Library, kind: &str, limit: u32) -> Result<Vec<Value>> {
    let db = lib.db()?;
    let mut statement = db.prepare(
        "SELECT payload FROM ai_records WHERE kind=?1 AND json_valid(payload)
         ORDER BY created_at DESC,rowid DESC LIMIT ?2",
    )?;
    let rows = statement.query_map(rusqlite::params![kind, limit], |row| {
        row.get::<_, String>(0)
    })?;
    let mut records = vec![];
    for row in rows {
        if let Ok(record) = serde_json::from_str(&row?) {
            records.push(record);
        }
    }
    Ok(records)
}

fn audio_exists(uri: &str) -> bool {
    if uri.is_empty() {
        return false;
    }
    let path = if uri.starts_with("file:") {
        url::Url::parse(uri)
            .ok()
            .and_then(|uri| uri.to_file_path().ok())
    } else {
        Some(PathBuf::from(uri))
    };
    path.is_some_and(|path| path.is_file())
}

impl State {
    /// All I/O is completed into a temporary value before replacing the snapshot.
    pub fn reload(&mut self, lib: &Library, audios: &[Row]) -> Result<()> {
        let _ = graph();
        let loaded = (|| -> Result<Self> {
            let speaking = records(lib, "speaking", 20)?;
            let ocr = records(lib, "ocr", 30)?;
            let stats = lib.list("study_stats")?;
            let mut focus_by_day = BTreeMap::new();
            let mut focus_minutes = 0.;
            let mut focus_distractions = 0_u64;
            for row in lib.rows(
                "SELECT ended_ms,actual_ms,distractions FROM focus_sessions",
                &[],
            )? {
                let (Some(ended), Some(actual)) = (
                    row.get("ended_ms").and_then(Value::as_u64),
                    row.get("actual_ms").and_then(Value::as_u64),
                ) else {
                    continue;
                };
                let Some(date) = chrono::DateTime::from_timestamp_millis(ended as i64) else {
                    continue;
                };
                let day = date
                    .with_timezone(&chrono::Local)
                    .format("%Y-%m-%d")
                    .to_string();
                let minutes = actual as f64 / 60_000.;
                *focus_by_day.entry(day).or_insert(0.) += minutes;
                focus_minutes += minutes;
                focus_distractions += row
                    .get("distractions")
                    .and_then(Value::as_u64)
                    .unwrap_or_default();
            }
            let words = lib.list("words")?;
            let mut document_files = vec![];
            for entry in fs::read_dir(lib.root.join("pdf"))? {
                let entry = entry?;
                let path = entry.path();
                if crate::ebook::supported(&path) && entry.file_type()?.is_file() {
                    document_files.push(path);
                }
            }
            document_files.sort();
            let available_audio = audios
                .iter()
                .map(|r| text(r, "audio_uri"))
                .filter(|uri| audio_exists(uri))
                .collect();
            let asr_models = ai::LOCAL_MODELS
                .iter()
                .filter(|model| model.installed(lib))
                .map(|model| model.id.to_string())
                .collect();
            let (ai_profiles, active_ai_profile) = ai::profiles(lib)?;
            let (storage_profiles, active_storage_profile) = backup::profiles(lib)?;
            Ok(Self {
                speaking,
                ocr,
                stats,
                focus_by_day,
                focus_minutes,
                focus_distractions,
                words,
                document_files,
                available_audio,
                asr_models,
                tts_ready: tts::validate_model(&tts::model_dir(lib)).is_ok(),
                // These APIs read non-secret settings/local credential presence only.
                // They deliberately do not resolve OS keychain passwords.
                saved_ai: Some(ai::load_config(lib)?),
                ai_profiles,
                active_ai_profile,
                storage_profiles,
                active_storage_profile,
                needs_legacy_import: credentials::needs_legacy_import(lib)?,
                error: String::new(),
                revision: self.revision.wrapping_add(1),
            })
        })();
        match loaded {
            Ok(loaded) => {
                *self = loaded;
                Ok(())
            }
            Err(error) => {
                self.error = format!("{error:#}");
                Err(error)
            }
        }
    }
}

pub fn graph() -> &'static Value {
    static GRAPH: OnceLock<Value> = OnceLock::new();
    GRAPH.get_or_init(|| {
        serde_json::from_str(include_str!("../../clarora-app/assets/graph-data.json"))
            .expect("the bundled public graph data must be valid JSON")
    })
}
