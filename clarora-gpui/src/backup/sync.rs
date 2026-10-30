//! Device revision tracking and last-modified-wins cloud synchronization.
use super::cloud::{self, Attachment, Prepared, Session, Snapshot};
use super::*;
use std::collections::BTreeSet;
mod apply;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
struct Version {
    modified_ms: i64,
    device: String,
    sequence: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct Entry {
    kind: String,
    identity: Value,
    version: Version,
    value: Option<Value>,
}
#[derive(Clone, Serialize, Deserialize, Default)]
struct Envelope {
    format_version: u32,
    records: BTreeMap<String, Entry>,
    files: BTreeMap<String, Attachment>,
    sources: BTreeMap<String, String>,
}
struct Local {
    envelope: Envelope,
    paths: BTreeMap<String, PathBuf>,
    source_paths: BTreeMap<String, PathBuf>,
    fingerprint: String,
    sequence: u64,
    prepared: Prepared,
}
fn entity_key(kind: &str, identity: &Value) -> String {
    json!([kind, identity]).to_string()
}
fn raw_key(table: &str, row: &Row) -> String {
    Value::Array(
        primary(table)
            .iter()
            .map(|k| row.get(*k).cloned().unwrap_or_default())
            .collect(),
    )
    .to_string()
}
fn blob(file: &Attachment) -> String {
    format!("blob:{}.{}", file.sha256, file.extension)
}
fn fingerprint(lib: &Library) -> Result<String> {
    let (data, settings) = lib.vault_with_settings()?;
    Ok(hash(&serde_json::to_string(&(data, settings))?))
}
fn clock(db: &rusqlite::Connection) -> Result<(String, i64, u64)> {
    Ok(db.query_row(
        "SELECT device_id,last_ms,sequence FROM cloud_sync_identity WHERE singleton=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?)
}
fn mint(db: &rusqlite::Connection) -> Result<Version> {
    db.execute("UPDATE cloud_sync_identity SET last_ms=MAX(last_ms,CAST(strftime('%s','now') AS INTEGER)*1000+CAST(substr(strftime('%f','now'),4,3) AS INTEGER)),sequence=sequence+1 WHERE singleton=1",[])?;
    let (device, modified_ms, sequence) = clock(db)?;
    Ok(Version {
        modified_ms,
        device,
        sequence,
    })
}
fn word_reference(id: &str, words: &BTreeMap<String, String>) -> String {
    words
        .get(id)
        .map(|word| format!("word/{word}"))
        .unwrap_or_else(|| id.into())
}
fn stable_identity(table: &str, row: &Row, words: &BTreeMap<String, String>) -> Value {
    let mut row = row.clone();
    if table == "review_schedule" && text(&row, "card_kind") == "word" {
        row.insert(
            "card_id".into(),
            json!(word_reference(&text(&row, "card_id"), words)),
        );
    }
    if table == "discover_favorites" {
        if let Some(id) = text(&row, "key").strip_prefix("word:") {
            let value = format!("word:{}", word_reference(id, words));
            row.insert("key".into(), json!(value));
        }
    }
    Value::Array(
        primary(table)
            .iter()
            .map(|key| row.get(*key).cloned().unwrap_or_default())
            .collect(),
    )
}
fn canonical_context(binding: &ContextBinding) -> String {
    hash(&json!(["cloud-material", binding.page, binding.path, binding.cue]).to_string())
}
fn fix_chat_contexts(
    data: &mut BTreeMap<String, Vec<Row>>,
    mapping: &HashMap<String, String>,
) -> Result<()> {
    for row in data
        .get_mut("ai_records")
        .into_iter()
        .flatten()
        .filter(|r| text(r, "kind") == "chat")
    {
        let mut payload: Value = serde_json::from_str(&text(row, "payload"))?;
        if let Some(context) = payload["context"].as_str().and_then(|s| mapping.get(s)) {
            payload["context"] = json!(context);
            row.insert("payload".into(), json!(payload.to_string()));
        }
    }
    Ok(())
}
fn capture(lib: &Library, scope: &str, cancel: &Cancel) -> Result<Local> {
    let before = fingerprint(lib)?;
    let prepared = cloud::prepare(lib, cancel)?;
    ensure!(
        fingerprint(lib)? == before,
        "SYNC_LOCAL_CHANGED: 本机资料正在更新，正在重新整理"
    );
    let mut manifest = prepared.snapshot.manifest.clone();
    let originals = manifest.data.clone();
    let mut word_ids = originals["words"]
        .iter()
        .map(|r| (text(r, "id"), text(r, "word")))
        .collect::<BTreeMap<_, _>>();
    let mut files = BTreeMap::new();
    let mut paths = BTreeMap::new();
    let mut refs = HashMap::new();
    for (reference, file) in &prepared.snapshot.attachments {
        let stable = blob(file);
        refs.insert(reference.clone(), stable.clone());
        files.insert(stable.clone(), file.clone());
        paths.insert(stable, prepared.paths[reference].clone());
    }
    map_manifest(&mut manifest, |reference| {
        Ok(refs
            .get(reference)
            .cloned()
            .unwrap_or_else(|| reference.into()))
    })?;
    let mut source_paths_local = BTreeMap::new();
    let raw_collection = lib.setting("video_favorites_v1")?;
    if let Ok(videos) = serde_json::from_str::<crate::video_library::Collection>(&raw_collection) {
        for (path, url) in source_paths(&LibraryState {
            videos: Some(videos),
            ..Default::default()
        })? {
            let key = format!("source:{}", hash(&url));
            if path.is_file() || !source_paths_local.contains_key(&key) {
                source_paths_local.insert(key, path);
            }
        }
    }
    let mut library = manifest.library.take().unwrap_or_default();
    if let Some(collection) = &mut library.videos {
        for video in &mut collection.items {
            if let Some(series) = &mut video.series {
                let current = series
                    .current
                    .unwrap_or(series.requested)
                    .min(series.parts.len().saturating_sub(1));
                for index in 0..series.parts.len() {
                    let url = series.url(index)?;
                    let reference = format!("source:{}", hash(&url));
                    manifest.sources.insert(reference.clone(), url);
                    series.parts[index].path = Some(PathBuf::from(&reference));
                    if index == current {
                        video.path = PathBuf::from(reference);
                    }
                }
            }
        }
    }
    let mut contexts = HashMap::new();
    for binding in &mut library.contexts {
        let context = canonical_context(binding);
        contexts.insert(binding.context.clone(), context.clone());
        binding.context = context;
    }
    fix_chat_contexts(&mut manifest.data, &contexts)?;
    let db = lib.db()?;
    let mut revisions = HashMap::new();
    let mut statement=db.prepare("SELECT table_name,record_key,modified_ms,sequence,deleted,payload FROM cloud_sync_revisions")?;
    for row in statement.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, u64>(3)?,
            r.get::<_, bool>(4)?,
            r.get::<_, String>(5)?,
        ))
    })? {
        let (t, k, ms, seq, deleted, payload) = row?;
        revisions.insert((t, k), (ms, seq, deleted, payload));
    }
    drop(statement);
    let mut observed = BTreeMap::new();
    let mut statement = db
        .prepare("SELECT entity_key,value,local_sequence FROM cloud_sync_entities WHERE scope=?")?;
    for row in statement.query_map([scope], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, u64>(2)?,
        ))
    })? {
        let (key, value, seq) = row?;
        observed.insert(key, (serde_json::from_str::<Entry>(&value)?, seq));
    }
    drop(statement);
    for deleted in [true, false] {
        for ((table, _), (_, _, is_deleted, payload)) in &revisions {
            if table == "words" && *is_deleted == deleted {
                if let Ok(row) = serde_json::from_str::<Row>(payload) {
                    word_ids.insert(text(&row, "id"), text(&row, "word"));
                }
            }
        }
    }
    for row in &originals["words"] {
        word_ids.insert(text(row, "id"), text(row, "word"));
    }
    let (device, _, _) = clock(&db)?;
    let mut values = BTreeMap::<String, (String, Value, Value, Option<(String, String)>)>::new();
    let mut add = |kind: &str, identity: Value, value: Value, raw: Option<(String, String)>| {
        values.insert(
            entity_key(kind, &identity),
            (kind.into(), identity, value, raw),
        );
    };
    for (table, _) in TABLES {
        for (index, row) in manifest
            .data
            .get_mut(*table)
            .context("同步缺少资料表")?
            .iter_mut()
            .enumerate()
        {
            let original = &originals[*table][index];
            let raw = Some((table.to_string(), raw_key(table, original)));
            if *table == "words" {
                row.remove("id");
            }
            if *table == "review_schedule" && text(row, "card_kind") == "word" {
                row.insert(
                    "card_id".into(),
                    json!(word_reference(&text(row, "card_id"), &word_ids)),
                );
            }
            if *table == "discover_favorites" {
                let key = text(row, "key");
                if let Some(id) = key.strip_prefix("word:") {
                    let stable = word_reference(id, &word_ids);
                    row.insert("key".into(), json!(format!("word:{stable}")));
                    if let Ok(mut payload) = serde_json::from_str::<Value>(&text(row, "payload")) {
                        if payload["id"].is_string() || payload["id"].is_number() {
                            payload["id"] = json!(stable);
                        }
                        row.insert("payload".into(), json!(payload.to_string()));
                    }
                }
            }
            let identity = Value::Array(
                primary(table)
                    .iter()
                    .map(|key| row.get(*key).cloned().unwrap_or_default())
                    .collect(),
            );
            add(table, identity, Value::Object(row.clone()), raw);
        }
    }
    if let Some(collection) = library.videos {
        for video in collection.items {
            add(
                "video",
                json!(video.id),
                serde_json::to_value(video)?,
                Some((
                    "app_settings".into(),
                    json!(["video_favorites_v1"]).to_string(),
                )),
            );
        }
    }
    for mut book in library.books {
        let ext = Path::new(&book.name)
            .extension()
            .and_then(|s| s.to_str())
            .context("书籍扩展名")?;
        book.name = format!(
            "{}.{}",
            crate::ebook::display_name(Path::new(&book.name)),
            ext
        );
        add("book", json!(book.path), serde_json::to_value(book)?, None);
    }
    for subtitle in library.subtitles {
        let raw = paths
            .get(&subtitle.video)
            .or_else(|| source_paths_local.get(&subtitle.video))
            .and_then(|path| crate::video_subtitles::identity_key(path).ok())
            .map(|key| ("app_settings".into(), json!([key]).to_string()));
        add(
            "subtitle",
            json!(subtitle.video),
            serde_json::to_value(subtitle)?,
            raw,
        );
    }
    for binding in library.contexts {
        add(
            "context",
            json!(binding.context),
            serde_json::to_value(binding)?,
            None,
        );
    }
    for (key, value) in library.settings {
        if key.starts_with("chat_draft_")
            || [
                "last_page",
                "navigation_open",
                "library_group",
                "last_listening_audio",
            ]
            .contains(&key.as_str())
        {
            continue;
        }
        add(
            "setting",
            json!(key),
            json!(value),
            Some(("app_settings".into(), json!([key]).to_string())),
        );
    }
    let mut records = BTreeMap::new();
    for (key, (kind, identity, value, raw)) in values {
        let old = observed.get(&key);
        let changed = old.is_none_or(|(e, _)| e.value.as_ref() != Some(&value));
        let revision = raw.as_ref().and_then(|r| revisions.get(r));
        let individual =
            kind == "setting" || kind == "subtitle" || TABLES.iter().any(|(t, _)| *t == kind);
        let edited = individual
            && old.is_some_and(|(_, seen)| revision.is_some_and(|(_, seq, _, _)| seq > seen));
        let version = if !changed && !edited {
            old.unwrap().0.version.clone()
        } else if let Some((ms, seq, _, _)) =
            revision.filter(|(_, seq, _, _)| old.is_none_or(|(_, seen)| seq > seen))
        {
            Version {
                modified_ms: *ms,
                device: device.clone(),
                sequence: *seq,
            }
        } else if old.is_some() {
            mint(&db)?
        } else {
            Version {
                modified_ms: 0,
                device: device.clone(),
                sequence: 0,
            }
        };
        records.insert(
            key,
            Entry {
                kind,
                identity,
                version,
                value: Some(value),
            },
        );
    }
    for (key, (old, seen)) in &observed {
        if records.contains_key(key) {
            continue;
        }
        if old.value.is_none() {
            records.insert(key.clone(), old.clone());
            continue;
        }
        let deletion = revisions
            .iter()
            .filter(|((table, _), (_, seq, deleted, _))| {
                table == &old.kind && *deleted && seq > seen
            })
            .find_map(|((table, _), (ms, seq, _, payload))| {
                let row = serde_json::from_str::<Row>(payload).ok()?;
                (entity_key(table, &stable_identity(table, &row, &word_ids)) == *key).then_some(
                    Version {
                        modified_ms: *ms,
                        device: device.clone(),
                        sequence: *seq,
                    },
                )
            })
            .or_else(|| {
                let raw = match old.kind.as_str() {
                    "video" => Some(json!(["video_favorites_v1"]).to_string()),
                    "setting" => Some(json!([old.identity.as_str()?]).to_string()),
                    "subtitle" => {
                        let uri = old.identity.as_str()?;
                        let path = paths.get(uri).or_else(|| source_paths_local.get(uri))?;
                        Some(json!([crate::video_subtitles::identity_key(path).ok()?]).to_string())
                    }
                    _ => None,
                }?;
                let (ms, seq, _, _) = revisions.get(&("app_settings".into(), raw))?;
                (*seq > *seen).then_some(Version {
                    modified_ms: *ms,
                    device: device.clone(),
                    sequence: *seq,
                })
            });
        records.insert(
            key.clone(),
            Entry {
                value: None,
                version: match deletion {
                    Some(version) => version,
                    None => mint(&db)?,
                },
                ..old.clone()
            },
        );
    }
    let (_, _, sequence) = clock(&db)?;
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)?;
    ensure!(
        fingerprint(lib)? == before && clock(&tx)?.2 == sequence,
        "SYNC_LOCAL_CHANGED: 本机资料在整理期间发生变化，正在重新整理"
    );
    for (key, entry) in &records {
        check_cancel(cancel)?;
        tx.execute("INSERT INTO cloud_sync_entities(scope,entity_key,value,local_sequence) VALUES(?,?,?,?) ON CONFLICT(scope,entity_key) DO UPDATE SET value=excluded.value,local_sequence=excluded.local_sequence",rusqlite::params![scope,key,serde_json::to_string(entry)?,sequence])?;
    }
    tx.commit()?;
    Ok(Local {
        envelope: Envelope {
            format_version: 3,
            records,
            files,
            sources: manifest.sources,
        },
        paths,
        source_paths: source_paths_local,
        fingerprint: before,
        sequence,
        prepared,
    })
}

fn merge(mut local: Envelope, remote: Envelope) -> Result<Envelope> {
    ensure!(remote.format_version == 3, "同步格式不支持");
    for (key, entry) in remote.records {
        ensure!(
            key == entity_key(&entry.kind, &entry.identity)
                && (TABLES.iter().any(|(table, _)| *table == entry.kind)
                    || ["video", "book", "subtitle", "context", "setting"]
                        .contains(&entry.kind.as_str()))
                && entry.version.modified_ms >= 0
                && entry.version.modified_ms < i64::MAX - 1
                && !entry.version.device.is_empty()
                && entry.version.device.len() <= 128
                && entry
                    .version
                    .device
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-'),
            "同步实体标识不一致"
        );
        let replace = local.records.get(&key).is_none_or(|old| {
            entry.version > old.version
                || (entry.version == old.version
                    && hash(&serde_json::to_string(&entry.value).unwrap_or_default())
                        > hash(&serde_json::to_string(&old.value).unwrap_or_default()))
        });
        if replace {
            local.records.insert(key, entry);
        }
    }
    for (key, file) in remote.files {
        local.files.insert(key, file);
    }
    local.sources.extend(remote.sources);
    repair_dependencies(&mut local)?;
    Ok(local)
}
fn repair_dependencies(state: &mut Envelope) -> Result<()> {
    let mut changes = vec![];
    for entry in state
        .records
        .values()
        .filter(|e| e.kind == "listening_audios" && e.value.is_some())
    {
        let row = entry
            .value
            .as_ref()
            .unwrap()
            .as_object()
            .context("音频实体无效")?;
        let parent = entity_key("listening_practices", &json!([text(row, "practice_id")]));
        let Some(group) = state.records.get(&parent) else {
            anyhow::bail!("同步音频缺少练习组");
        };
        if group.value.is_some() {
            continue;
        }
        if entry.version <= group.version {
            changes.push((
                entity_key(&entry.kind, &entry.identity),
                Entry {
                    value: None,
                    version: group.version.clone(),
                    ..entry.clone()
                },
            ));
        } else {
            let group_id = "cloud-recovered-audio";
            let version = Version {
                modified_ms: entry
                    .version
                    .modified_ms
                    .checked_add(1)
                    .context("同步时间溢出")?,
                device: "dependency-repair".into(),
                sequence: 0,
            };
            let group = Entry {
                kind: "listening_practices".into(),
                identity: json!([group_id]),
                version: version.clone(),
                value: Some(
                    json!({"id":group_id,"name":"同步恢复的音频","created_at":chrono::DateTime::from_timestamp_millis(version.modified_ms).context("同步时间无效")?.format("%Y-%m-%d %H:%M:%S").to_string()}),
                ),
            };
            changes.push((entity_key(&group.kind, &group.identity), group));
            let mut value = entry.value.clone().unwrap();
            value["practice_id"] = json!(group_id);
            changes.push((
                entity_key(&entry.kind, &entry.identity),
                Entry {
                    value: Some(value),
                    version,
                    ..entry.clone()
                },
            ));
        }
    }
    for (key, entry) in changes {
        state.records.insert(key, entry);
    }
    repair_derived(state)?;
    Ok(())
}
fn repair_derived(state: &mut Envelope) -> Result<()> {
    let mut video_paths = BTreeSet::new();
    let mut book_paths = BTreeSet::new();
    for entry in state.records.values().filter(|e| e.value.is_some()) {
        let value = entry.value.as_ref().unwrap();
        if entry.kind == "video" {
            let video: crate::video_library::Video = serde_json::from_value(value.clone())?;
            video_paths.insert(video.path.to_string_lossy().into_owned());
            if let Some(series) = video.series {
                for part in series.parts {
                    if let Some(path) = part.path {
                        video_paths.insert(path.to_string_lossy().into_owned());
                    }
                }
            }
        }
        if entry.kind == "book" {
            book_paths.insert(value["path"].as_str().context("书籍路径无效")?.to_string());
        }
    }
    let parent_key = |kind: &str, id: &str| -> Option<String> {
        let (table, id) = match kind {
            "word" => ("words", id.strip_prefix("word/")?),
            "clip" => ("clip_cards", id),
            "video" => ("video_clips", id),
            "sentence" => ("sentence_cards", id),
            "ai" => ("ai_cards", id),
            _ => return None,
        };
        Some(entity_key(table, &json!([id])))
    };
    let latest_delete = state
        .records
        .values()
        .filter(|e| e.value.is_none())
        .map(|e| e.version.modified_ms)
        .max()
        .unwrap_or(0);
    let mut changes = vec![];
    for (key, entry) in state.records.iter().filter(|(_, e)| e.value.is_some()) {
        let value = entry.value.as_ref().unwrap();
        let remove = match entry.kind.as_str() {
            "subtitle" => !video_paths.contains(value["video"].as_str().unwrap_or("")),
            "context" => {
                let path = value["path"].as_str().unwrap_or("");
                !video_paths.contains(path) && !book_paths.contains(path)
            }
            "setting" => {
                let name = entry.identity.as_str().unwrap_or("");
                if ["pdf_current", "last_video_position"].contains(&name) {
                    let parsed: Value =
                        serde_json::from_str(value.as_str().context("阅读位置无效")?)?;
                    let path = parsed["path"].as_str().unwrap_or("");
                    if name == "pdf_current" {
                        !book_paths.contains(path)
                    } else {
                        !video_paths.contains(path)
                    }
                } else {
                    false
                }
            }
            "review_schedule" => parent_key(
                value["card_kind"].as_str().unwrap_or(""),
                value["card_id"].as_str().unwrap_or(""),
            )
            .is_some_and(|k| state.records.get(&k).is_none_or(|p| p.value.is_none())),
            "discover_favorites" => value["key"]
                .as_str()
                .and_then(|s| s.split_once(':'))
                .and_then(|(kind, id)| parent_key(kind, id))
                .is_some_and(|k| state.records.get(&k).is_none_or(|p| p.value.is_none())),
            _ => false,
        };
        if remove {
            let version = Version {
                modified_ms: entry
                    .version
                    .modified_ms
                    .max(latest_delete)
                    .checked_add(1)
                    .context("同步时间溢出")?,
                device: "dependency-repair".into(),
                sequence: 0,
            };
            changes.push((
                key.clone(),
                Entry {
                    value: None,
                    version,
                    ..entry.clone()
                },
            ));
        }
    }
    for (key, entry) in changes {
        state.records.insert(key, entry);
    }
    Ok(())
}

fn snapshot(state: &Envelope) -> Result<(Snapshot, BTreeMap<String, String>)> {
    let mut manifest = Manifest {
        version: 2,
        id: id(),
        created_at: chrono::Utc::now().to_rfc3339(),
        data: TABLES
            .iter()
            .map(|(t, _)| (t.to_string(), vec![]))
            .collect(),
        files: BTreeMap::new(),
        sources: BTreeMap::new(),
        library: Some(LibraryState::default()),
    };
    for entry in state.records.values().filter(|e| e.value.is_some()) {
        let value = entry.value.clone().unwrap();
        if let Some(rows) = manifest.data.get_mut(&entry.kind) {
            let row = value.as_object().context("同步资料行无效")?;
            let columns = TABLES
                .iter()
                .find(|(t, _)| *t == entry.kind)
                .context("同步资料类型无效")?
                .1;
            ensure!(
                row.keys().all(|k| columns.contains(&k.as_str()))
                    && Value::Array(
                        primary(&entry.kind)
                            .iter()
                            .map(|k| row.get(*k).cloned().unwrap_or_default())
                            .collect()
                    ) == entry.identity,
                "同步资料字段或主键不一致"
            );
            rows.push(row.clone());
            continue;
        }
        let library = manifest.library.as_mut().unwrap();
        match entry.kind.as_str() {
            "video" => library
                .videos
                .get_or_insert_with(Default::default)
                .items
                .push(serde_json::from_value(value)?),
            "book" => library.books.push(serde_json::from_value(value)?),
            "subtitle" => library.subtitles.push(serde_json::from_value(value)?),
            "context" => library.contexts.push(serde_json::from_value(value)?),
            "setting" => {
                let key = entry.identity.as_str().context("同步设置标识无效")?;
                ensure!(
                    safe_setting(key)
                        && !key.starts_with("chat_draft_")
                        && ![
                            "last_page",
                            "navigation_open",
                            "library_group",
                            "last_listening_audio"
                        ]
                        .contains(&key),
                    "同步包含不允许的设置"
                );
                library
                    .settings
                    .insert(key.into(), value.as_str().context("同步设置值无效")?.into());
            }
            _ => anyhow::bail!("同步包含未知资料类型"),
        }
    }
    let mut words = BTreeMap::new();
    for (index, row) in manifest
        .data
        .get_mut("words")
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        let id = (index + 1).to_string();
        words.insert(format!("word/{}", text(row, "word")), id.clone());
        row.insert("id".into(), json!(index + 1));
    }
    map_word_links(&mut manifest.data, &words)?;
    let mut references = BTreeMap::<String, String>::new();
    let mut sources = BTreeMap::new();
    map_manifest(&mut manifest, |uri| {
        if let Some(url) = state.sources.get(uri) {
            sources.insert(uri.into(), url.clone());
            return Ok(uri.into());
        }
        ensure!(state.files.contains_key(uri), "同步缺少附件描述");
        let next = format!("media:{}", references.len());
        Ok(references.entry(uri.into()).or_insert(next).clone())
    })?;
    manifest.sources = sources;
    let mut attachments = BTreeMap::new();
    for (stable, reference) in &references {
        let file = state.files.get(stable).context("同步附件无效")?.clone();
        ensure!(blob(&file) == *stable, "同步附件摘要标识不一致");
        manifest.files.insert(
            reference.clone(),
            format!(
                "{}.{}",
                reference.trim_start_matches("media:"),
                file.extension
            ),
        );
        attachments.insert(reference.clone(), file);
    }
    Ok((
        Snapshot {
            format_version: 3,
            manifest,
            attachments,
        },
        references,
    ))
}
fn map_word_links(
    data: &mut BTreeMap<String, Vec<Row>>,
    words: &BTreeMap<String, String>,
) -> Result<()> {
    if let Some(rows) = data.get_mut("review_schedule") {
        rows.retain_mut(|r| {
            if text(r, "card_kind") != "word" {
                return true;
            }
            if let Some(id) = words.get(&text(r, "card_id")) {
                r.insert("card_id".into(), json!(id));
                true
            } else {
                false
            }
        });
    }
    if let Some(rows) = data.get_mut("discover_favorites") {
        rows.retain_mut(|r| {
            let key = text(r, "key");
            let Some(word) = key.strip_prefix("word:") else {
                return true;
            };
            let Some(id) = words.get(word) else {
                return false;
            };
            r.insert("key".into(), json!(format!("word:{id}")));
            if let Ok(mut payload) = serde_json::from_str::<Value>(&text(r, "payload")) {
                payload["id"] = json!(id);
                r.insert("payload".into(), json!(payload.to_string()));
            }
            true
        });
    }
    Ok(())
}

pub(crate) fn install_tracking(db: &rusqlite::Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS cloud_sync_identity(singleton INTEGER PRIMARY KEY CHECK(singleton=1),device_id TEXT NOT NULL,last_ms INTEGER NOT NULL DEFAULT 0,sequence INTEGER NOT NULL DEFAULT 0,applying INTEGER NOT NULL DEFAULT 0,tracking INTEGER NOT NULL DEFAULT 0);
         INSERT OR IGNORE INTO cloud_sync_identity(singleton,device_id) VALUES(1,lower(hex(randomblob(16))));
         CREATE TABLE IF NOT EXISTS cloud_sync_journal(id INTEGER PRIMARY KEY AUTOINCREMENT,table_name TEXT NOT NULL,record_key TEXT NOT NULL,modified_ms INTEGER NOT NULL,sequence INTEGER NOT NULL,deleted INTEGER NOT NULL,payload TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS cloud_sync_revisions(table_name TEXT NOT NULL,record_key TEXT NOT NULL,modified_ms INTEGER NOT NULL,sequence INTEGER NOT NULL,deleted INTEGER NOT NULL,payload TEXT NOT NULL,PRIMARY KEY(table_name,record_key));
         CREATE TABLE IF NOT EXISTS cloud_sync_entities(scope TEXT NOT NULL,entity_key TEXT NOT NULL,value TEXT NOT NULL,local_sequence INTEGER NOT NULL,PRIMARY KEY(scope,entity_key));
         CREATE TABLE IF NOT EXISTS cloud_sync_status(scope TEXT PRIMARY KEY,last_success TEXT NOT NULL,last_error TEXT NOT NULL DEFAULT '');"
    )?;
    let mut tables = TABLES
        .iter()
        .map(|(t, c)| (*t, c.to_vec()))
        .collect::<Vec<_>>();
    tables.push(("app_settings", vec!["key", "value"]));
    for (table, columns) in tables {
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
                    primary(table)
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

fn primary(table: &str) -> &'static [&'static str] {
    match table {
        "words" => &["word"],
        "review_schedule" => &["card_kind", "card_id"],
        "study_stats" => &["day"],
        "discover_favorites" | "app_settings" => &["key"],
        _ => &["id"],
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SyncResult {
    pub applied: usize,
    pub deleted: usize,
    pub uploaded_bytes: u64,
    pub summary: String,
}

pub(super) fn validate_cloud_head(session: &Session<'_>, head: &cloud::Remote) -> Result<()> {
    let envelope: Envelope = serde_json::from_slice(&session.remote_bytes(head)?)?;
    ensure!(envelope.format_version == 3, "同步格式不支持");
    let (mut view, _) = snapshot(&envelope)?;
    session.validate_snapshot(&mut view)?;
    ensure!(
        cloud::object_keys(&view) == head.objects,
        "同步附件索引与清单不一致，已停止清理"
    );
    Ok(())
}

impl Storage {
    pub fn synchronize(
        &self,
        lib: &Library,
        cancel: &Cancel,
        progress: impl Fn(UploadProgress) + Send + Sync + 'static,
    ) -> Result<SyncResult> {
        let reporter = Arc::new(UploadReporter::new(progress));
        reporter.update(true, |_| {});
        let session = Session::new(self, cancel)?;
        session.verify_cas()?;
        let vault = session.mutate(|c| Ok(c.vault_id.clone()))?;
        let scope = format!("{}:{vault}", session.scope());
        lib.db()?.execute(
            "UPDATE cloud_sync_identity SET tracking=1 WHERE singleton=1",
            [],
        )?;
        let mut uploaded_bytes = 0;
        for attempt in 0..8 {
            check_cancel(cancel)?;
            reporter.update(true, |p| {
                p.stage = UploadStage::Preparing;
                p.current_name = if attempt == 0 {
                    "正在比较变更并校验云端资料…".into()
                } else {
                    "检测到同时修改，正在合并并重试…".into()
                };
                p.bytes_sent = 0;
                p.bytes_total = 0;
                p.bytes_reused = 0;
                p.files_done = 0;
                p.files_total = 0;
            });
            let (catalog, _) = session.catalog()?;
            ensure!(catalog.vault_id == vault, "云资料库已重建，请重新同步");
            let head = catalog.sync_head.clone();
            let local = match capture(lib, &scope, cancel) {
                Ok(local) => local,
                Err(error) if error.to_string().contains("SYNC_LOCAL_CHANGED") => continue,
                Err(error) => return Err(error),
            };
            let read_pin = if let Some(head) = &head {
                let pin = id();
                let pinned = session.mutate(|c| {
                    if c.sync_head.as_ref() != Some(head) {
                        return Ok(false);
                    }
                    c.pins.insert(
                        pin.clone(),
                        cloud::Pin {
                            expires_at: (chrono::Utc::now() + chrono::Duration::hours(24))
                                .to_rfc3339(),
                            objects: head.objects.clone(),
                            snapshot: Some(head.key.clone()),
                        },
                    );
                    Ok(true)
                })?;
                if !pinned {
                    continue;
                }
                Some(pin)
            } else {
                None
            };
            let remote_result = (|| -> Result<Envelope> {
                let remote = if let Some(head) = &head {
                    serde_json::from_slice::<Envelope>(&session.remote_bytes(head)?)?
                } else {
                    Envelope {
                        format_version: 3,
                        ..Default::default()
                    }
                };
                let (mut remote_snapshot, _) = snapshot(&remote)?;
                session.validate_snapshot(&mut remote_snapshot)?;
                if let Some(head) = &head {
                    ensure!(
                        cloud::object_keys(&remote_snapshot) == head.objects,
                        "同步附件索引与清单不一致"
                    );
                }
                Ok(remote)
            })();
            let remote = match remote_result {
                Ok(remote) => remote,
                Err(error) => {
                    if let Some(pin) = &read_pin {
                        session.release(pin);
                    }
                    return Err(error);
                }
            };
            let mut state = merge(local.envelope.clone(), remote.clone())?;
            let root = lib
                .root
                .join("media")
                .join(format!("cloud-sync-stage-{}", id()));
            fs::create_dir_all(&root)?;
            let operation = (|| -> Result<Option<SyncResult>> {
                reporter.update(true, |p| p.current_name = "正在整理并校验所需附件…".into());
                let (mut prepared, refs) = apply::prepare(&session, &state, &local, &root)?;
                let changed = head.is_none() || state.records != remote.records;
                if changed {
                    let pin = session.allocate(&mut prepared.snapshot)?;
                    session.send_prepared(&prepared, &reporter)?;
                    uploaded_bytes += reporter.sent();
                    session.validate_snapshot(&mut prepared.snapshot)?;
                    state.files = refs
                        .iter()
                        .map(|(stable, reference)| {
                            (
                                stable.clone(),
                                prepared.snapshot.attachments[reference].clone(),
                            )
                        })
                        .collect();
                    state.sources = prepared.snapshot.manifest.sources.clone();
                    reporter.update(true, |p| p.stage = UploadStage::Publishing);
                    let new = session.put_remote(
                        "sync",
                        &id(),
                        serde_json::to_vec(&state)?,
                        cloud::object_keys(&prepared.snapshot),
                    )?;
                    let committed = session.mutate(|c| {
                        if c.sync_head != head {
                            return Ok(false);
                        }
                        ensure!(c.pins.contains_key(&pin), "同步上传任务已过期，请重新同步");
                        c.sync_head = Some(new.clone());
                        c.pins.remove(&pin);
                        if let Some(pin) = &read_pin {
                            c.pins.remove(pin);
                        }
                        Ok(true)
                    })?;
                    if !committed {
                        session.release(&pin);
                        return Ok(None);
                    }
                }
                match apply::run(lib, &scope, &local, &state, &prepared, &refs, cancel) {
                    Ok((applied, deleted)) => {
                        reporter.update(true, |p| p.stage = UploadStage::Completed);
                        Ok(Some(SyncResult {
                            applied,
                            deleted,
                            uploaded_bytes,
                            summary: format!(
                                "同步完成 · {} 项资料 · {} 条删除记录，冲突按最后修改时间处理",
                                applied, deleted
                            ),
                        }))
                    }
                    Err(error) if error.to_string().contains("SYNC_LOCAL_CHANGED") => Ok(None),
                    Err(error) => Err(error),
                }
            })();
            let _ = fs::remove_dir_all(&root);
            if let Some(pin) = &read_pin {
                session.release(pin);
            }
            if let Some(result) = operation? {
                return Ok(result);
            }
        }
        anyhow::bail!("同步期间资料持续变化，已保留本机内容；稍后重试可继续")
    }
}
