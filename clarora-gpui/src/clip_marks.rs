//! Saved clip provenance and subtitle ranges. Rendering consumes this in-memory catalog.
use crate::{
    data::{self, Library, Row, id, now, text},
    process::{self, Cancel},
    subtitles::{self, Cue},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::Path,
    sync::atomic::Ordering,
};
const ORIGIN: &str = "clip_origin";
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub page: String,
    pub key: String,
}
impl Source {
    pub fn audio(id: &str) -> Self {
        Self {
            page: "listening".into(),
            key: id.into(),
        }
    }
    pub fn video(path: &Path) -> Self {
        Self {
            page: "video".into(),
            key: path.to_string_lossy().into_owned(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Origin {
    source: Source,
    start: f64,
    end: f64,
}
#[derive(Clone, Debug, Serialize)]
pub struct Saved {
    pub kind: String,
    pub id: String,
    pub front: String,
    pub media: String,
    pub start: f64,
    pub end: f64,
}
#[derive(Clone, Debug, Serialize)]
pub struct Range {
    pub saved: Saved,
    pub start: f64,
    pub end: f64,
    pub inferred: bool,
}
impl Range {
    pub fn overlaps(&self, start: f64, end: f64) -> bool {
        self.start < end - 0.001 && self.end > start + 0.001
    }
}
#[derive(Default)]
pub struct Catalog {
    cards: Vec<(Saved, Option<Origin>)>,
}
fn origin_id(kind: &str, id: &str) -> String {
    format!("clip-origin:{kind}:{id}")
}
fn table(kind: &str) -> Result<&'static str> {
    match kind {
        "clip" => Ok("clip_cards"),
        "video" => Ok("video_clips"),
        _ => anyhow::bail!("不是片段闪卡"),
    }
}
impl Catalog {
    pub fn load(lib: &Library) -> Result<Self> {
        let origins = lib
            .rows(
                "SELECT id,payload FROM ai_records WHERE kind='clip_origin'",
                &[],
            )?
            .into_iter()
            .map(|r| {
                Ok((
                    text(&r, "id"),
                    serde_json::from_str::<Origin>(&text(&r, "payload"))
                        .context("片段来源数据损坏")?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        let mut cards = vec![];
        for (kind, table, media) in [
            ("clip", "clip_cards", "audio_uri"),
            ("video", "video_clips", "video_uri"),
        ] {
            for r in lib.list(table)? {
                let id = text(&r, "id");
                let origin = origins.get(&origin_id(kind, &id)).cloned();
                cards.push((
                    Saved {
                        kind: kind.into(),
                        id,
                        front: text(&r, "en_text"),
                        media: text(&r, media),
                        start: r.get("start_ms").and_then(Value::as_f64).unwrap_or(0.) / 1000.,
                        end: r.get("end_ms").and_then(Value::as_f64).unwrap_or(0.) / 1000.,
                    },
                    origin,
                ));
            }
        }
        Ok(Self { cards })
    }
    pub fn ranges(&self, source: &Source, cues: &[Cue], media: &str) -> Vec<Range> {
        let words = cues.iter().flat_map(subtitles::words).collect::<Vec<_>>();
        let normalized = words.iter().map(|w| normalize(&w.text)).collect::<Vec<_>>();
        let mut ranges = vec![];
        for (saved, origin) in &self.cards {
            if (source.page == "listening") != (saved.kind == "clip") {
                continue;
            }
            let found = if let Some(origin) = origin {
                if origin.source != *source
                    || !origin.start.is_finite()
                    || !origin.end.is_finite()
                    || origin.start < 0.
                    || origin.end <= origin.start
                {
                    continue;
                }
                Some((origin.start, origin.end, false))
            } else if source.page == "listening" && saved.media == media && saved.end > saved.start
            {
                Some((saved.start, saved.end, false))
            } else {
                let tokens = saved
                    .front
                    .split_whitespace()
                    .map(normalize)
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>();
                if tokens.is_empty() || tokens.len() > normalized.len() {
                    continue;
                }
                let hits = normalized
                    .windows(tokens.len())
                    .enumerate()
                    .filter(|(_, s)| *s == tokens.as_slice())
                    .map(|(i, _)| i)
                    .take(2)
                    .collect::<Vec<_>>();
                if hits.len() == 1 {
                    let i = hits[0];
                    Some((words[i].start, words[i + tokens.len() - 1].end, true))
                } else {
                    None
                }
            };
            if let Some((start, end, inferred)) = found {
                ranges.push(Range {
                    saved: saved.clone(),
                    start,
                    end,
                    inferred,
                })
            }
        }
        ranges
    }
}
fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}
#[allow(clippy::too_many_arguments)]
pub fn save(
    lib: &Library,
    source: Source,
    path: &Path,
    a: f64,
    b: f64,
    front: &str,
    back: &str,
    cancel: &Cancel,
) -> Result<Saved> {
    ensure!(
        a.is_finite() && b.is_finite() && a >= 0. && b > a,
        "请设置有效片段范围"
    );
    ensure!(!front.trim().is_empty(), "片段原文不能为空");
    ensure!(
        ["listening", "video"].contains(&source.page.as_str()) && !source.key.is_empty(),
        "片段来源无效"
    );
    let kind = if source.page == "listening" {
        "clip"
    } else {
        "video"
    };
    let key = id();
    let output = lib.root.join("media").join(format!("{key}.mp4"));
    let result = (|| {
        process::clip(path, &output, a, b, cancel)?;
        ensure!(!cancel.load(Ordering::SeqCst), "已取消");
        let mut db = lib.db()?;
        let tx = db.transaction()?;
        let row = if kind == "clip" {
            json!({"id":key,"en_text":front,"zh_text":back,"audio_uri":output.to_string_lossy(),"start_ms":0,"end_ms":((b-a)*1000.) as u64,"created_at":now()})
        } else {
            json!({"id":key,"en_text":front,"zh_text":back,"video_uri":output.to_string_lossy(),"created_at":now()})
        };
        data::insert(&tx, table(kind)?, row.as_object().unwrap(), false)?;
        let payload = serde_json::to_string(&Origin {
            source,
            start: a,
            end: b,
        })?;
        data::insert(
            &tx,
            "ai_records",
            json!({"id":origin_id(kind,&key),"kind":ORIGIN,"payload":payload,"created_at":now()})
                .as_object()
                .unwrap(),
            false,
        )?;
        ensure!(!cancel.load(Ordering::SeqCst), "已取消");
        tx.commit()?;
        Ok(Saved {
            kind: kind.into(),
            id: key,
            front: front.into(),
            media: output.to_string_lossy().into_owned(),
            start: 0.,
            end: b - a,
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(output);
    }
    result
}
/// Remove the card, schedule, favorite and provenance atomically; retain all media.
pub fn remove(lib: &Library, saved: &Saved) -> Result<()> {
    let mut db = lib.db()?;
    let tx = db.transaction()?;
    tx.execute(
        &format!("DELETE FROM {} WHERE id=?", table(&saved.kind)?),
        [&saved.id],
    )?;
    tx.execute(
        "DELETE FROM review_schedule WHERE card_kind=? AND card_id=?",
        rusqlite::params![saved.kind, saved.id],
    )?;
    tx.execute(
        "DELETE FROM discover_favorites WHERE key=?",
        [format!("{}:{}", saved.kind, saved.id)],
    )?;
    tx.execute(
        "DELETE FROM ai_records WHERE id=? AND kind='clip_origin'",
        [origin_id(&saved.kind, &saved.id)],
    )?;
    tx.commit()?;
    Ok(())
}
/// Map only known media references. Provenance must never add/upload a source video.
pub fn map_sources(
    data: &mut BTreeMap<String, Vec<Row>>,
    mapped: &HashMap<String, String>,
) -> Result<()> {
    for row in data
        .get_mut("ai_records")
        .into_iter()
        .flatten()
        .filter(|r| text(r, "kind") == ORIGIN)
    {
        let mut origin: Origin = serde_json::from_str(&text(row, "payload"))?;
        if origin.source.page == "video" {
            if let Some(key) = mapped.get(&origin.source.key) {
                origin.source.key = key.clone();
                row.insert("payload".into(), json!(serde_json::to_string(&origin)?));
            }
        }
    }
    Ok(())
}
pub fn map_audio(row: &mut Row, mapping: &HashMap<(String, String), String>) -> Result<()> {
    if text(row, "kind") != ORIGIN {
        return Ok(());
    }
    let mut origin: Origin = serde_json::from_str(&text(row, "payload"))?;
    if origin.source.page == "listening" {
        if let Some(key) = mapping.get(&("audio".into(), origin.source.key.clone())) {
            origin.source.key = key.clone();
            row.insert("payload".into(), json!(serde_json::to_string(&origin)?));
        }
    }
    Ok(())
}
