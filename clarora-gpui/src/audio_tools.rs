//! Audio merging and the persistent relationship between study versions and originals.
use crate::{
    data::{Library, Row, id, insert, now, text},
    process::{self, Cancel},
    subtitles,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    path::PathBuf,
    sync::atomic::Ordering,
    time::{Duration, Instant, SystemTime},
};

const MERGE: &str = "audio_merge";
#[derive(Serialize, Deserialize)]
struct Relationship {
    target: String,
    sources: Vec<String>,
}
#[derive(Clone, Default)]
pub struct Catalog {
    next: BTreeMap<String, String>,
    targets: BTreeMap<String, usize>,
}
impl Catalog {
    pub fn load(lib: &Library, audios: &[Row]) -> Result<Self> {
        let rows = audios
            .iter()
            .map(|r| (text(r, "id"), r))
            .collect::<BTreeMap<_, _>>();
        let mut catalog = Self::default();
        for record in lib.rows(
            "SELECT payload FROM ai_records WHERE kind='audio_merge' ORDER BY created_at,id",
            &[],
        )? {
            let Ok(relation) = serde_json::from_str::<Relationship>(&text(&record, "payload"))
            else {
                continue;
            };
            if !(2..=500).contains(&relation.sources.len())
                || relation.sources.iter().collect::<BTreeSet<_>>().len() != relation.sources.len()
            {
                continue;
            }
            let Some(target) = rows.get(&relation.target) else {
                continue;
            };
            // A lost merged attachment must never make its preserved originals disappear.
            if !fs::metadata(text(target, "audio_uri")).is_ok_and(|m| m.is_file() && m.len() > 0) {
                continue;
            }
            catalog
                .targets
                .insert(relation.target.clone(), relation.sources.len());
            for source in relation.sources {
                let Some(row) = rows.get(&source) else {
                    continue;
                };
                if source == relation.target
                    || text(row, "practice_id") != text(target, "practice_id")
                {
                    continue;
                }
                let mut cursor = relation.target.as_str();
                let mut visited = BTreeSet::new();
                while cursor != source && visited.insert(cursor) {
                    let Some(next) = catalog.next.get(cursor) else {
                        break;
                    };
                    cursor = next;
                }
                if cursor != source {
                    catalog.next.insert(source, relation.target.clone());
                }
            }
        }
        Ok(catalog)
    }
    pub fn hidden(&self, id: &str) -> bool {
        self.next.contains_key(id)
    }
    pub fn source_count(&self, id: &str) -> usize {
        self.targets.get(id).copied().unwrap_or(0)
    }
    pub fn replacement<'a>(&self, id: &str, audios: &'a [Row]) -> Option<&'a Row> {
        let mut current = id;
        let mut seen = BTreeSet::new();
        while seen.insert(current) {
            let Some(next) = self.next.get(current) else {
                break;
            };
            current = next;
        }
        audios.iter().find(|r| text(r, "id") == current)
    }
    pub fn learning<'a>(&self, audios: &'a [Row], group: &str) -> Vec<&'a Row> {
        let mut rows = audios
            .iter()
            .filter(|r| text(r, "practice_id") == group && !self.hidden(&text(r, "id")))
            .collect::<Vec<_>>();
        // Stable sorting keeps the existing order within merged and ordinary items.
        rows.sort_by_key(|r| self.source_count(&text(r, "id")) == 0);
        rows
    }
}
/// Backup merge may deduplicate same-name audios/groups to different IDs.
pub fn map_record(row: &mut Row, mapping: &HashMap<(String, String), String>) -> Result<()> {
    if text(row, "kind") != MERGE {
        return Ok(());
    }
    let mut relationship: Relationship = serde_json::from_str(&text(row, "payload"))?;
    for reference in
        std::iter::once(&mut relationship.target).chain(relationship.sources.iter_mut())
    {
        if let Some(mapped) = mapping.get(&("audio".into(), reference.clone())) {
            *reference = mapped.clone();
        }
    }
    row.insert(
        "payload".into(),
        json!(serde_json::to_string(&relationship)?),
    );
    Ok(())
}

pub struct Merged {
    pub row: Row,
    pub sources: usize,
    pub subtitles_complete: bool,
}
struct Work(PathBuf);
impl Drop for Work {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn check(cancel: &Cancel) -> Result<()> {
    ensure!(!cancel.load(Ordering::SeqCst), "已取消合并");
    Ok(())
}
fn stamp(path: &str) -> Result<(u64, SystemTime)> {
    let m = fs::metadata(path).context("原音频文件缺失或不可读")?;
    ensure!(m.is_file() && m.len() > 0, "原音频为空或不是文件");
    Ok((m.len(), m.modified()?))
}
fn subtitle_bytes(path: &str) -> Result<Option<Vec<u8>>> {
    if path.is_empty() {
        return Ok(None);
    }
    match fs::metadata(path) {
        Ok(m) => {
            ensure!(
                m.is_file() && m.len() <= 16 * 1024 * 1024,
                "原字幕不可读或超过16 MiB"
            );
            Ok(Some(fs::read(path)?))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
struct Source {
    row: Row,
    stamp: (u64, SystemTime),
    subtitle: Option<Vec<u8>>,
}
fn validate(db: &rusqlite::Connection, sources: &[Source], practice: &str) -> Result<()> {
    for source in sources {
        let current = db.query_row("SELECT audio_uri,subtitle_uri,name,created_at FROM listening_audios WHERE id=? AND practice_id=?",[text(&source.row,"id"),practice.to_string()],|r| (0..4).map(|i|r.get::<_,String>(i)).collect::<rusqlite::Result<Vec<_>>>()).context("原音频已移动或删除，请重新选择")?;
        for (key, value) in ["audio_uri", "subtitle_uri", "name", "created_at"]
            .into_iter()
            .zip(current)
        {
            ensure!(
                value == text(&source.row, key),
                "原音频或字幕已改变，请重新合并"
            );
        }
        ensure!(
            stamp(&text(&source.row, "audio_uri"))? == source.stamp
                && subtitle_bytes(&text(&source.row, "subtitle_uri"))? == source.subtitle,
            "原音频或字幕文件已改变，请重新合并"
        );
    }
    Ok(())
}
pub fn merge(lib: &Library, rows: &[Row], practice: &str, cancel: &Cancel) -> Result<Merged> {
    check(cancel)?;
    ensure!((2..=500).contains(&rows.len()), "请勾选 2–500 条音频");
    ensure!(
        rows.iter()
            .map(|r| text(r, "id"))
            .collect::<BTreeSet<_>>()
            .len()
            == rows.len(),
        "合并选择中有重复音频"
    );
    ensure!(
        rows.iter().all(|r| text(r, "practice_id") == practice),
        "只能合并同一个组的音频"
    );
    let sources = rows
        .iter()
        .map(|row| {
            Ok(Source {
                row: row.clone(),
                stamp: stamp(&text(row, "audio_uri"))?,
                subtitle: subtitle_bytes(&text(row, "subtitle_uri"))?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    validate(&lib.db()?, &sources, practice)?;
    let key = id();
    let media = lib.root.join("media");
    let work = Work(media.join(format!(".merge-{key}")));
    fs::create_dir(&work.0)?;
    let output = media.join(format!("merged-{key}.m4a"));
    let subtitle = output.with_extension("srt");
    let result = (|| -> Result<Merged> {
        let mut args = vec!["-nostdin".into(), "-v".into(), "error".into(), "-y".into()];
        let mut cues = vec![];
        let mut offset = 0.;
        let mut complete = true;
        for (i, source) in sources.iter().enumerate() {
            check(cancel)?;
            let normalized = work.0.join(format!("{i}.wav"));
            process::run(
                &process::tool("ffmpeg")?,
                &[
                    "-nostdin".into(),
                    "-v".into(),
                    "error".into(),
                    "-y".into(),
                    "-i".into(),
                    text(&source.row, "audio_uri"),
                    "-vn".into(),
                    "-ac".into(),
                    "2".into(),
                    "-ar".into(),
                    "48000".into(),
                    "-c:a".into(),
                    "pcm_s16le".into(),
                    normalized.to_string_lossy().into_owned(),
                ],
                cancel,
                Duration::from_secs(1800),
            )?;
            let decoded = hound::WavReader::open(&normalized)?;
            let duration = decoded.duration() as f64 / decoded.spec().sample_rate as f64;
            ensure!(
                duration.is_finite() && duration > 0.,
                "无法读取合并音频的真实解码时长"
            );
            let mut segment = source
                .subtitle
                .as_ref()
                .and_then(|b| std::str::from_utf8(b).ok())
                .map(subtitles::parse)
                .unwrap_or_default();
            segment.retain_mut(|cue| {
                cue.start = cue.start.clamp(0., duration);
                cue.end = cue.end.clamp(0., duration);
                cue.end > cue.start
            });
            complete &= !segment.is_empty();
            for mut cue in segment {
                cue.start += offset;
                cue.end += offset;
                cues.push(cue);
            }
            offset += duration;
            args.extend(["-i".into(), normalized.to_string_lossy().into_owned()]);
        }
        let filter = format!(
            "{}concat=n={}:v=0:a=1[out]",
            (0..rows.len())
                .map(|i| format!("[{i}:a]"))
                .collect::<String>(),
            rows.len()
        );
        args.extend([
            "-filter_complex".into(),
            filter,
            "-map".into(),
            "[out]".into(),
            "-c:a".into(),
            "aac".into(),
            output.to_string_lossy().into_owned(),
        ]);
        process::run(
            &process::tool("ffmpeg")?,
            &args,
            cancel,
            Duration::from_secs(1800),
        )?;
        check(cancel)?;
        if complete {
            fs::write(&subtitle, subtitles::srt(&cues))?;
        }
        let row=json!({"id":key,"practice_id":practice,"name":format!("合并 {} 条 · {}",rows.len(),&key[..6]),"audio_uri":output,"subtitle_uri":if complete {subtitle.to_string_lossy().into_owned()}else{String::new()},"created_at":now()}).as_object().unwrap().clone();
        let db = lib.db()?;
        db.busy_timeout(Duration::from_millis(50))?;
        let waiting = Instant::now();
        let tx = loop {
            check(cancel)?;
            match rusqlite::Transaction::new_unchecked(
                &db,
                rusqlite::TransactionBehavior::Immediate,
            ) {
                Ok(tx) => break tx,
                Err(e)
                    if e.sqlite_error_code() == Some(rusqlite::ErrorCode::DatabaseBusy)
                        && waiting.elapsed() < Duration::from_secs(10) => {}
                Err(e) => return Err(e.into()),
            }
        };
        validate(&tx, &sources, practice)?;
        check(cancel)?;
        insert(&tx, "listening_audios", &row, false)?;
        let relationship = Relationship {
            target: key.clone(),
            sources: rows.iter().map(|r| text(r, "id")).collect(),
        };
        let record = json!({"id":id(),"kind":MERGE,"payload":serde_json::to_string(&relationship)?,"created_at":now()});
        insert(&tx, "ai_records", record.as_object().unwrap(), false)?;
        check(cancel)?;
        tx.commit()?;
        Ok(Merged {
            row,
            sources: rows.len(),
            subtitles_complete: complete,
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&output);
        let _ = fs::remove_file(&subtitle);
    }
    result
}
