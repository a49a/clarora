//! Explicitly loaded, last-known-good data for otherwise I/O-free UI rendering.
use crate::{
    ai, credentials,
    data::{Library, Row, text},
    tts,
};
use anyhow::Result;
use serde_json::Value;
use std::{collections::BTreeSet, fs, path::PathBuf, sync::OnceLock};

#[derive(Default)]
pub struct State {
    pub speaking: Vec<Value>,
    pub ocr: Vec<Value>,
    pub stats: Vec<Row>,
    pub words: Vec<Row>,
    pub document_files: Vec<PathBuf>,
    pub available_audio: BTreeSet<String>,
    pub asr_models: BTreeSet<String>,
    pub tts_ready: bool,
    pub saved_ai: Option<ai::Config>,
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
            let asr_models = [
                "tiny.en",
                "base.en",
                "small.en",
                "base",
                "small",
                "sense-voice",
            ]
            .into_iter()
            .filter(|model| {
                if *model == "sense-voice" {
                    lib.root
                        .join("models/sense-voice/model.int8.onnx")
                        .is_file()
                        && lib.root.join("models/sense-voice/tokens.txt").is_file()
                } else {
                    lib.root
                        .join("models")
                        .join(format!("ggml-{model}.bin"))
                        .is_file()
                }
            })
            .map(str::to_string)
            .collect();
            Ok(Self {
                speaking,
                ocr,
                stats,
                words,
                document_files,
                available_audio,
                asr_models,
                tts_ready: tts::validate_model(&tts::model_dir(lib)).is_ok(),
                // These APIs read non-secret settings/local credential presence only.
                // They deliberately do not resolve OS keychain passwords.
                saved_ai: Some(ai::load_config(lib)?),
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
