//! Transactional local import and merge of portable library data.
use super::attachments::{StagedAttachment, file_hash};
use super::check_cancel;
use super::manifest::{
    LibraryState, Manifest, map_manifest, map_media, material_context, safe_setting,
};
use crate::data::{Library, Row, id, insert, schema::TABLES, text};
use crate::process::Cancel;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub(super) fn finish_restore(
    lib: &Library,
    manifest: &mut Manifest,
    stage: &Path,
    cancel: &Cancel,
) -> Result<usize> {
    let mut created = vec![];
    let result = (|| {
        let mut relocated: HashMap<String, String> = HashMap::new();
        let mut existing = BTreeMap::new();
        if let Some(library) = &mut manifest.library {
            for entry in fs::read_dir(lib.root.join("pdf"))? {
                let path = entry?.path();
                if path.is_file() && crate::ebook::supported(&path) {
                    existing.insert(
                        (
                            file_hash(&path, cancel)?,
                            path.extension()
                                .map(|s| s.to_string_lossy().to_ascii_lowercase())
                                .unwrap_or_default(),
                        ),
                        path,
                    );
                }
            }
            for book in &mut library.books {
                check_cancel(cancel)?;
                if let Some(path) = relocated.get(&book.path) {
                    book.path = path.clone();
                    continue;
                }
                let original = PathBuf::from(&book.path);
                let hash = file_hash(&original, cancel)?;
                let ext = Path::new(&book.name)
                    .extension()
                    .context("书籍扩展名无效")?
                    .to_string_lossy()
                    .to_ascii_lowercase();
                ensure!(
                    original
                        .extension()
                        .is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case(&ext)),
                    "书籍附件格式不一致"
                );
                let target = if let Some(path) = existing.get(&(hash.clone(), ext.clone())) {
                    path.clone()
                } else {
                    let candidate = lib.root.join("pdf").join(&book.name);
                    let target = if candidate.exists() {
                        let mut stem = Path::new(&book.name)
                            .file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned();
                        while stem.len() > 180 {
                            stem.pop();
                        }
                        lib.root
                            .join("pdf")
                            .join(format!("{}-{}.{}", stem, &hash[..32], ext))
                    } else {
                        candidate
                    };
                    if target.exists() {
                        ensure!(file_hash(&target, cancel)? == hash, "书库恢复目标名称冲突");
                    } else {
                        let input = fs::File::open(&original)?;
                        let length = input.metadata()?.len();
                        let mut output = StagedAttachment::new(&target, Some(&hash))?;
                        output.copy(input, cancel)?;
                        output.finish(&target, Some(length), cancel)?;
                        created.push(target.clone());
                    }
                    existing.insert((hash, ext), target.clone());
                    target
                };
                relocated.insert(book.path.clone(), target.to_string_lossy().into_owned());
            }
        }
        map_manifest(manifest, |path| {
            Ok(relocated.get(path).cloned().unwrap_or_else(|| path.into()))
        })?;
        merge_snapshot(lib, &manifest.data, manifest.library.as_ref(), cancel)
    })();
    if result.is_err() {
        for path in created {
            let _ = fs::remove_file(path);
        }
    }
    if result.is_ok() {
        prune_stage(lib, stage);
    }
    result
}
fn references(value: &Value, path: &str, depth: u8) -> bool {
    if depth > 8 {
        return false;
    }
    match value {
        Value::String(value) => {
            value == path
                || serde_json::from_str::<Value>(value)
                    .is_ok_and(|v| references(&v, path, depth + 1))
        }
        Value::Array(values) => values.iter().any(|v| references(v, path, depth + 1)),
        Value::Object(values) => values.values().any(|v| references(v, path, depth + 1)),
        _ => false,
    }
}
fn prune_stage(lib: &Library, stage: &Path) {
    // Cleanup after the atomic DB commit is conservative: a failed read keeps files.
    let Ok((data, settings)) = lib.vault_with_settings() else {
        return;
    };
    let Ok(data) = serde_json::to_value(data) else {
        return;
    };
    let Ok(settings) = serde_json::to_value(settings) else {
        return;
    };
    if let Ok(entries) = fs::read_dir(stage) {
        for entry in entries.flatten() {
            let path = entry.path();
            let value = path.to_string_lossy();
            if path.is_file() && !references(&data, &value, 0) && !references(&settings, &value, 0)
            {
                let _ = fs::remove_file(path);
            }
        }
    }
    let _ = fs::remove_dir(stage);
}
pub fn merge(lib: &Library, data: &BTreeMap<String, Vec<Row>>) -> Result<usize> {
    merge_cancellable(lib, data, &crate::process::cancel_token())
}
fn merge_cancellable(
    lib: &Library,
    data: &BTreeMap<String, Vec<Row>>,
    cancel: &Cancel,
) -> Result<usize> {
    merge_snapshot(lib, data, None, cancel)
}
fn merge_snapshot(
    lib: &Library,
    original_data: &BTreeMap<String, Vec<Row>>,
    library: Option<&LibraryState>,
    cancel: &Cancel,
) -> Result<usize> {
    check_cancel(cancel)?;
    let db = lib.db()?;
    // Acquire the writer before reading: a deferred WAL transaction cannot safely
    // retry an upgrade from a read snapshot when another writer owns the database.
    db.busy_timeout(Duration::from_millis(50))?;
    let waiting = Instant::now();
    let tx = loop {
        check_cancel(cancel)?;
        // This is a fresh connection with no outer transaction; runtime checking
        // allows the failed BEGIN attempts to share a borrow across this loop.
        match rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate) {
            Ok(tx) => break tx,
            Err(error) => {
                check_cancel(cancel)?;
                if error.sqlite_error_code() != Some(rusqlite::ErrorCode::DatabaseBusy)
                    || waiting.elapsed() >= Duration::from_secs(10)
                {
                    return Err(error.into());
                }
            }
        }
    };
    check_cancel(cancel)?;
    tx.busy_timeout(Duration::from_secs(10))?;
    let mut data = original_data.clone();
    let mut library = library.cloned();
    if let Some(library) = &mut library {
        prepare_library(&tx, &mut data, library, cancel)?;
    }
    let data = &data;
    let mut mapping: HashMap<(String, String), String> = HashMap::new();
    let mut imported = 0;
    for spec in TABLES {
        let table = spec.name;
        if ["review_schedule", "study_stats", "discover_favorites"].contains(&table) {
            continue;
        }
        let kind = spec.card.map_or_else(
            || match table {
                "listening_practices" => "practice",
                "listening_audios" => "audio",
                _ => "record",
            },
            crate::data::CardKind::as_str,
        );
        for original in data.get(table).context("缺少资料表")? {
            check_cancel(cancel)?;
            let mut row = original.clone();
            let old = text(&row, "id");
            if table == "ai_records" {
                crate::clip_marks::map_audio(&mut row, &mapping)?;
                crate::audio_tools::map_record(&mut row, &mapping)?;
            }
            let mut existing = None;
            if table == "words" {
                existing = tx
                    .query_row(
                        "SELECT CAST(id AS TEXT) FROM words WHERE word=?",
                        [text(&row, "word")],
                        |r| r.get::<_, String>(0),
                    )
                    .ok();
            }
            if table == "listening_practices" {
                existing = tx
                    .query_row(
                        "SELECT id FROM listening_practices WHERE lower(name)=lower(?)",
                        [text(&row, "name")],
                        |r| r.get::<_, String>(0),
                    )
                    .ok();
            }
            if table == "listening_audios" {
                let practice = mapping
                    .get(&("practice".into(), text(&row, "practice_id")))
                    .context("备份音频的练习组缺失")?;
                row.insert("practice_id".into(), json!(practice));
                existing = tx
                    .query_row(
                        "SELECT id FROM listening_audios WHERE practice_id=? AND name=?",
                        rusqlite::params![practice, text(&row, "name")],
                        |r| r.get::<_, String>(0),
                    )
                    .ok();
            }
            if existing.is_none() && table != "words" {
                existing = tx
                    .query_row(&format!("SELECT id FROM {table} WHERE id=?"), [&old], |r| {
                        r.get::<_, String>(0)
                    })
                    .ok();
            }
            let key = if let Some(existing) = existing {
                if table == "ai_records" && text(&row, "kind") == "chat" {
                    if let Some(library) = &library {
                        repair_chat_context(&tx, &existing, &text(&row, "payload"), library)?;
                    }
                }
                existing
            } else {
                if table == "words" {
                    row.remove("id");
                }
                insert(&tx, table, &row, false)?;
                imported += 1;
                if table == "words" {
                    tx.last_insert_rowid().to_string()
                } else {
                    old.clone()
                }
            };
            mapping.insert((kind.into(), old), key);
        }
    }
    for row in data.get("review_schedule").context("缺少复习记录")? {
        check_cancel(cancel)?;
        let mut row = row.clone();
        let kind = text(&row, "card_kind");
        let Some(key) = mapping.get(&(kind.clone(), text(&row, "card_id"))) else {
            continue;
        };
        row.insert("card_id".into(), json!(key));
        let previous=tx.query_row("SELECT coalesce(last_graded_at,'') FROM review_schedule WHERE card_kind=? AND card_id=?",rusqlite::params![kind,key],|r|r.get::<_,String>(0)).ok();
        if previous.is_some_and(|old| old >= text(&row, "last_graded_at")) {
            continue;
        }
        tx.execute(
            "DELETE FROM review_schedule WHERE card_kind=? AND card_id=?",
            rusqlite::params![kind, key],
        )?;
        insert(&tx, "review_schedule", &row, false)?;
    }
    for row in data.get("study_stats").context("缺少统计记录")? {
        check_cancel(cancel)?;
        tx.execute("INSERT INTO study_stats(day,listen_seconds,review_count,pomodoro_count) VALUES(?,?,?,?) ON CONFLICT(day) DO UPDATE SET listen_seconds=max(listen_seconds,excluded.listen_seconds),review_count=max(review_count,excluded.review_count),pomodoro_count=max(pomodoro_count,excluded.pomodoro_count)",rusqlite::params![text(row,"day"),row["listen_seconds"].as_f64().unwrap_or(0.),row["review_count"].as_f64().unwrap_or(0.),row["pomodoro_count"].as_f64().unwrap_or(0.)])?;
    }
    for row in data.get("discover_favorites").context("缺少收藏")? {
        check_cancel(cancel)?;
        let mut row = row.clone();
        let key = text(&row, "key");
        let mut payload: Value = serde_json::from_str(&text(&row, "payload"))?;
        if let Some((kind, old)) = key.split_once(':') {
            if let Some(mapped) = mapping.get(&(kind.into(), old.into())) {
                let key = format!("{kind}:{mapped}");
                payload["key"] = json!(key);
                row.insert("key".into(), json!(key));
                row.insert("payload".into(), json!(payload.to_string()));
            }
        }
        insert(&tx, "discover_favorites", &row, true)?;
    }
    if let Some(library) = &mut library {
        merge_library(&tx, library, &mapping, cancel)?;
    }
    // Once commit starts, return its actual result; a late cancel cannot undo it.
    check_cancel(cancel)?;
    tx.commit()?;
    Ok(imported)
}
fn repair_chat_context(
    db: &rusqlite::Connection,
    id: &str,
    incoming: &str,
    library: &LibraryState,
) -> Result<()> {
    use rusqlite::OptionalExtension;
    let Ok(imported) = serde_json::from_str::<Value>(incoming) else {
        return Ok(());
    };
    let Some(context) = imported["context"].as_str().filter(|context| {
        library
            .contexts
            .iter()
            .any(|binding| binding.context == *context)
    }) else {
        return Ok(());
    };
    let previous: Option<String> = db
        .query_row(
            "SELECT payload FROM ai_records WHERE id=? AND kind='chat'",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    let Some(mut previous) = previous.and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
    else {
        return Ok(());
    };
    if previous["context"] != context {
        if let Some(old) = previous["context"].as_str() {
            let draft = setting_in(db, &format!("chat_draft_{old}"))?;
            if !draft.is_empty() {
                preserve_setting(db, &format!("chat_draft_{context}"), &draft)?;
            }
        }
        previous["context"] = json!(context);
        db.execute(
            "UPDATE ai_records SET payload=? WHERE id=?",
            rusqlite::params![previous.to_string(), id],
        )?;
    }
    Ok(())
}
pub(super) fn setting_in(db: &rusqlite::Connection, key: &str) -> Result<String> {
    use rusqlite::OptionalExtension;
    Ok(db
        .query_row("SELECT value FROM app_settings WHERE key=?", [key], |row| {
            row.get(0)
        })
        .optional()?
        .unwrap_or_default())
}
fn usable_file(path: &Path) -> bool {
    path.is_file() && fs::metadata(path).is_ok_and(|meta| meta.len() > 0)
}
fn preserve_setting(db: &rusqlite::Connection, key: &str, value: &str) -> Result<()> {
    let existing = setting_in(db, key)?;
    let usable = if existing.is_empty() {
        false
    } else if ["pdf_current", "last_video_position"].contains(&key) {
        serde_json::from_str::<Value>(&existing)
            .ok()
            .and_then(|value| value["path"].as_str().map(PathBuf::from))
            .is_some_and(|path| usable_file(&path))
    } else if key.starts_with("video_subtitles_") {
        usable_file(Path::new(&existing))
    } else {
        true
    };
    if !usable {
        db.execute("INSERT INTO app_settings(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",rusqlite::params![key,value])?;
    }
    Ok(())
}
fn prepare_library(
    db: &rusqlite::Connection,
    data: &mut BTreeMap<String, Vec<Row>>,
    library: &mut LibraryState,
    cancel: &Cancel,
) -> Result<()> {
    let mut mapped = HashMap::<String, String>::new();
    if let Some(imported) = &library.videos {
        let current = setting_in(db, "video_favorites_v1")?;
        let mut current: crate::video_library::Collection = if current.is_empty() {
            crate::video_library::Collection::default()
        } else {
            serde_json::from_str(&current).context("当前视频收藏损坏，未恢复")?
        };
        current.validate()?;
        let initially_empty = current.items.is_empty();
        let mut repaired = HashMap::<String, String>::new();
        for video in &imported.items {
            check_cancel(cancel)?;
            let old = current.items.iter_mut().find(|old| {
                old.id == video.id
                    || video.series.as_ref().is_some_and(|series| {
                        old.series.as_ref().is_some_and(|s| s.base == series.base)
                    })
            });
            if let Some(old) = old {
                if let (Some(existing), Some(source)) = (&mut old.series, &video.series) {
                    for part in &source.parts {
                        if let Some(path) = &part.path {
                            if let Some(target) =
                                existing.parts.iter_mut().find(|p| p.number == part.number)
                            {
                                if let Some(target_path) = &target.path {
                                    if usable_file(target_path) {
                                        mapped.insert(
                                            path.to_string_lossy().into_owned(),
                                            target_path.to_string_lossy().into_owned(),
                                        );
                                    } else {
                                        repaired.insert(
                                            target_path.to_string_lossy().into_owned(),
                                            path.to_string_lossy().into_owned(),
                                        );
                                        target.path = Some(path.clone());
                                    }
                                } else {
                                    target.path = Some(path.clone());
                                }
                            } else {
                                existing.parts.push(part.clone());
                            }
                        } else if !existing
                            .parts
                            .iter()
                            .any(|existing| existing.number == part.number)
                        {
                            existing.parts.push(part.clone());
                        }
                    }
                    if !usable_file(&old.path) {
                        old.path = PathBuf::from(
                            repaired
                                .get(&old.path.to_string_lossy().into_owned())
                                .cloned()
                                .unwrap_or_else(|| video.path.to_string_lossy().into_owned()),
                        );
                    }
                    existing.parts.sort_by_key(|part| part.number);
                    if let Some(target) = source
                        .parts
                        .iter()
                        .find(|part| part.path.as_ref() == Some(&video.path))
                        .and_then(|source| {
                            existing
                                .parts
                                .iter()
                                .find(|part| part.number == source.number)
                        })
                        .and_then(|part| part.path.as_ref())
                    {
                        mapped.insert(
                            video.path.to_string_lossy().into_owned(),
                            target.to_string_lossy().into_owned(),
                        );
                    }
                } else if usable_file(&old.path) {
                    mapped.insert(
                        video.path.to_string_lossy().into_owned(),
                        old.path.to_string_lossy().into_owned(),
                    );
                } else {
                    repaired.insert(
                        old.path.to_string_lossy().into_owned(),
                        video.path.to_string_lossy().into_owned(),
                    );
                    old.path = video.path.clone();
                }
            } else {
                current.items.push(video.clone());
            }
        }
        // A deliberate deselection in an existing library stays deselected.
        if initially_empty {
            current.selected = imported.selected.clone();
        }
        let value = setting_in(db, "last_video_position")?;
        if let Ok(mut position) = serde_json::from_str::<Value>(&value) {
            if let Some(path) = position["path"].as_str().and_then(|old| repaired.get(old)) {
                position["path"] = json!(path);
                db.execute(
                    "UPDATE app_settings SET value=? WHERE key='last_video_position'",
                    [position.to_string()],
                )?;
            }
        }
        current.validate()?;
        library.videos = Some(current);
    }
    library.map_paths(&mut |path| Ok(mapped.get(path).cloned().unwrap_or_else(|| path.into())))?;
    map_media(data, |path| {
        Ok(mapped.get(path).cloned().unwrap_or_else(|| path.into()))
    })?;
    crate::clip_marks::map_sources(data, &mapped)?;
    let mut contexts = HashMap::new();
    for binding in &mut library.contexts {
        check_cancel(cancel)?;
        let material = if let Some(cue) = binding.cue {
            let subtitle = library
                .subtitles
                .iter()
                .find(|s| s.video == binding.path)
                .context("备份材料缺少视频字幕")?;
            let cues = crate::subtitles::parse(&fs::read_to_string(&subtitle.subtitle)?);
            crate::video_qa::build(
                Path::new(&binding.path),
                &cues,
                if cue < 0 {
                    crate::video_qa::Scope::All
                } else {
                    crate::video_qa::Scope::Cue(cue as usize)
                },
                0.,
                "",
            )?
            .key
        } else {
            binding.path.clone()
        };
        let context = material_context(&binding.page, &material);
        contexts.insert(binding.context.clone(), context.clone());
        binding.context = context;
    }
    for row in data
        .get_mut("ai_records")
        .into_iter()
        .flatten()
        .filter(|row| text(row, "kind") == "chat")
    {
        if let Ok(mut payload) = serde_json::from_str::<Value>(&text(row, "payload")) {
            if let Some(context) = payload["context"]
                .as_str()
                .and_then(|old| contexts.get(old))
            {
                payload["context"] = json!(context);
                row.insert("payload".into(), json!(payload.to_string()));
            }
        }
    }
    let mut drafts = vec![];
    library.settings.retain(|key, value| {
        if let Some(context) = key
            .strip_prefix("chat_draft_")
            .and_then(|old| contexts.get(old))
        {
            drafts.push((format!("chat_draft_{context}"), value.clone()));
            false
        } else {
            true
        }
    });
    library.settings.extend(drafts);
    Ok(())
}
fn merge_library(
    db: &rusqlite::Connection,
    library: &mut LibraryState,
    mapping: &HashMap<(String, String), String>,
    cancel: &Cancel,
) -> Result<()> {
    if let Some(videos) = &library.videos {
        videos.validate()?;
        db.execute("INSERT INTO app_settings(key,value) VALUES('video_favorites_v1',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[serde_json::to_string(videos)?])?;
        if let Some(series) = videos.active().and_then(|v| v.series.as_ref()) {
            preserve_setting(db, "bilibili_series", &serde_json::to_string(series)?)?;
        }
    }
    for subtitle in &library.subtitles {
        check_cancel(cancel)?;
        let key = crate::video_subtitles::identity_key(Path::new(&subtitle.video))?;
        preserve_setting(db, &key, &subtitle.subtitle)?;
    }
    for (key, value) in &library.settings {
        check_cancel(cancel)?;
        ensure!(safe_setting(key), "备份包含不允许的设置");
        let (mut key, mut value) = (key.clone(), value.clone());
        if let Some(id) = key.strip_prefix("key_points_") {
            if let Some(mapped) = mapping.get(&("audio".into(), id.into())) {
                key = format!("key_points_{mapped}");
            }
        }
        if ["library_group", "last_listening_audio"].contains(&key.as_str()) {
            if let Some(mapped) = mapping.get(&(
                if key == "library_group" {
                    "practice"
                } else {
                    "audio"
                }
                .into(),
                value.clone(),
            )) {
                value = mapped.clone();
            }
        }
        if key == "last_listening_position" {
            let mut position: Value = serde_json::from_str(&value)?;
            if let Some(id) = position["id"]
                .as_str()
                .and_then(|id| mapping.get(&("audio".into(), id.into())))
            {
                position["id"] = json!(id);
                value = position.to_string();
            }
        }
        preserve_setting(db, &key, &value)?;
    }
    Ok(())
}
/// Snapshot an RN SQLite library read-only, copy attachments and merge into GPUI's own DB.
pub fn import_sqlite(lib: &Library, path: &Path) -> Result<usize> {
    let source =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    source.busy_timeout(std::time::Duration::from_secs(10))?;
    source.execute_batch("BEGIN")?;
    let mut data = BTreeMap::new();
    for spec in TABLES {
        let table = spec.name;
        let columns = spec.columns;
        let exists: i64 = source.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?",
            [table],
            |r| r.get(0),
        )?;
        if exists == 0 && table == "sentence_cards" {
            data.insert(table.to_string(), vec![]);
            continue;
        }
        ensure!(exists > 0, "所选数据库不是兼容的 Clarora 资料库");
        let rows = crate::data::query_rows(
            &source,
            &format!("SELECT {} FROM {table}", columns.join(",")),
            &[],
        )?;
        data.insert(table.to_string(), rows);
    }
    source.execute_batch("ROLLBACK")?;
    let stage = lib.root.join("media").join(format!("imported-{}", id()));
    fs::create_dir_all(&stage)?;
    let mut paths: HashMap<String, String> = HashMap::new();
    let result = (|| {
        map_media(&mut data, |uri| {
            if let Some(path) = paths.get(uri) {
                return Ok(path.clone());
            }
            let original = if uri.starts_with("file:") {
                url::Url::parse(uri)?
                    .to_file_path()
                    .map_err(|_| anyhow::anyhow!("附件路径无效"))?
            } else {
                PathBuf::from(uri)
            };
            ensure!(
                original.is_absolute() && original.is_file(),
                "原库附件不存在，未导入"
            );
            let ext = original
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("bin");
            ensure!(
                ext.chars().all(|c| c.is_ascii_alphanumeric()) && ext.len() <= 15,
                "附件扩展名无效"
            );
            let target = stage.join(format!("{}.{}", id(), ext));
            fs::copy(&original, &target)?;
            let path = target.to_string_lossy().to_string();
            paths.insert(uri.into(), path.clone());
            Ok(path)
        })?;
        merge(lib, &data)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&stage);
    } else {
        prune_stage(lib, &stage);
    }
    result
}
