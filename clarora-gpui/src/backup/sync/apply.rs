//! Verified materialization and guarded, atomic application of a merged sync view.
use super::*;
use sha2::{Digest, Sha256};

fn download_file(session: &Session<'_>, file: &Attachment, target: &Path) -> Result<()> {
    ensure!(file.length < 8 * 1024 * 1024 * 1024, "同步附件描述无效");
    let mut output = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(target)?;
    let mut digest = Sha256::new();
    let mut size = 0u64;
    for chunk in &file.chunks {
        check_cancel(session.cancel)?;
        ensure!(
            chunk
                .key
                .starts_with(&format!("{}/objects/", session.prefix()))
                && chunk.key.ends_with(&chunk.sha256)
                && chunk.length > 0
                && chunk.length <= cloud::CHUNK_BYTES
                && !chunk.key.split('/').any(|s| s == ".." || s == "."),
            "同步附件路径无效"
        );
        let (bytes, _) = session.get(&chunk.key)?.context("同步附件分块缺失")?;
        ensure!(
            bytes.len() as u64 == chunk.length && cloud::checksum(&bytes) == chunk.sha256,
            "同步附件分块校验失败"
        );
        output.write_all(&bytes)?;
        digest.update(&bytes);
        size += bytes.len() as u64;
    }
    ensure!(
        size == file.length && hex::encode(digest.finalize()) == file.sha256,
        "同步附件文件校验失败"
    );
    output.sync_all()?;
    Ok(())
}

pub(super) fn prepare(
    session: &Session<'_>,
    state: &Envelope,
    local: &Local,
    root: &Path,
) -> Result<(Prepared, BTreeMap<String, String>)> {
    let (snapshot, refs) = snapshot(state)?;
    let reverse = refs
        .iter()
        .map(|(stable, reference)| (reference.clone(), stable.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut paths = BTreeMap::new();
    for (reference, file) in &snapshot.attachments {
        let stable = &reverse[reference];
        let path = if let Some(path) = local.paths.get(stable) {
            path.clone()
        } else {
            let path = root.join(format!(
                "{}.{}",
                reference.trim_start_matches("media:"),
                file.extension
            ));
            download_file(session, file, &path)?;
            path
        };
        paths.insert(reference.clone(), path);
    }
    let stamps = attachment_stamps(&paths)?;
    Ok((
        Prepared {
            snapshot,
            paths,
            stamps,
        },
        refs,
    ))
}

fn rebind(manifest: &mut Manifest, cancel: &Cancel) -> Result<()> {
    let Some(library) = &mut manifest.library else {
        return Ok(());
    };
    let mut contexts = HashMap::new();
    for binding in &mut library.contexts {
        check_cancel(cancel)?;
        let material = if let Some(cue) = binding.cue {
            let subtitle = library
                .subtitles
                .iter()
                .find(|s| s.video == binding.path)
                .context("同步材料缺少字幕")?;
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
    fix_chat_contexts(&mut manifest.data, &contexts)?;
    Ok(())
}

fn copy_verified(
    source: &Path,
    target: &Path,
    file: &Attachment,
    cancel: &Cancel,
    created: &mut Vec<PathBuf>,
) -> Result<()> {
    if target.is_file() {
        ensure!(
            file_hash(target, cancel)? == file.sha256,
            "本机同步附件校验失败，未覆盖现有文件"
        );
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut output = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(target)?;
    created.push(target.into());
    copy_cancellable(fs::File::open(source)?, &mut output, cancel)?;
    output.sync_all()?;
    ensure!(
        file_hash(target, cancel)? == file.sha256,
        "同步附件落盘校验失败"
    );
    Ok(())
}

pub(super) fn run(
    lib: &Library,
    scope: &str,
    local: &Local,
    state: &Envelope,
    prepared: &Prepared,
    refs: &BTreeMap<String, String>,
    cancel: &Cancel,
) -> Result<(usize, usize)> {
    let mut db = lib.db()?;
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    ensure!(
        clock(&tx)?.2 == local.sequence && fingerprint(lib)? == local.fingerprint,
        "SYNC_LOCAL_CHANGED: 本机资料在同步期间已更新，正在重新整理"
    );
    check_attachments(&local.prepared.stamps, cancel)?;
    tx.execute(
        "UPDATE cloud_sync_identity SET applying=1 WHERE singleton=1",
        [],
    )?;
    tx.execute_batch("PRAGMA defer_foreign_keys=ON;")?;
    let mut created = vec![];
    let mut moved = vec![];
    let outcome = (|| -> Result<(usize, usize)> {
        let mut manifest = prepared.snapshot.manifest.clone();
        let mut mapping = HashMap::new();
        let reverse = refs
            .iter()
            .map(|(s, r)| (r.clone(), s.clone()))
            .collect::<BTreeMap<_, _>>();
        let live_books = manifest
            .library
            .as_ref()
            .map(|l| {
                l.books
                    .iter()
                    .map(|b| b.path.clone())
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default();
        for (reference, file) in &prepared.snapshot.attachments {
            let stable = &reverse[reference];
            let source = &prepared.paths[reference];
            let target = if live_books.contains(reference) {
                let book = manifest
                    .library
                    .as_ref()
                    .unwrap()
                    .books
                    .iter()
                    .find(|b| &b.path == reference)
                    .context("书籍映射无效")?;
                let name = format!(
                    "{}-{}.{}",
                    crate::ebook::display_name(Path::new(&book.name)),
                    &file.sha256[..16],
                    file.extension
                );
                let candidate = lib.root.join("pdf").join(name);
                if let Some(existing) = local.paths.get(stable).filter(|p| {
                    p.parent() == Some(lib.root.join("pdf").as_path())
                        && crate::ebook::display_name(p)
                            == crate::ebook::display_name(Path::new(&book.name))
                }) {
                    existing.clone()
                } else {
                    candidate
                }
            } else if let Some(existing) = local
                .paths
                .get(stable)
                .filter(|p| p.parent() != Some(lib.root.join("pdf").as_path()))
            {
                existing.clone()
            } else {
                lib.root
                    .join("media/cloud")
                    .join(format!("{}.{}", file.sha256, file.extension))
            };
            copy_verified(source, &target, file, cancel, &mut created)?;
            mapping.insert(reference.clone(), target.to_string_lossy().into_owned());
        }
        let desired_book_paths = live_books
            .iter()
            .filter_map(|r| mapping.get(r))
            .cloned()
            .collect::<BTreeSet<_>>();
        for (reference, file) in &local.prepared.snapshot.attachments {
            let stable = blob(file);
            let source = &local.prepared.paths[reference];
            if source.parent() == Some(lib.root.join("pdf").as_path())
                && !desired_book_paths.contains(&source.to_string_lossy().into_owned())
            {
                let key = entity_key("book", &json!(stable));
                if state.records.contains_key(&key) {
                    let trash = lib
                        .root
                        .join("sync-trash")
                        .join(id())
                        .join(source.file_name().context("书籍文件名无效")?);
                    fs::create_dir_all(trash.parent().unwrap())?;
                    fs::rename(source, &trash)?;
                    moved.push((source.clone(), trash));
                }
            }
        }
        let sources = manifest.sources.clone();
        map_manifest(&mut manifest, |reference| {
            if let Some(url) = sources.get(reference) {
                return Ok(local
                    .source_paths
                    .get(reference)
                    .cloned()
                    .unwrap_or(crate::bilibili::source_path(lib, url)?)
                    .to_string_lossy()
                    .into_owned());
            }
            mapping.get(reference).cloned().context("同步附件缺失")
        })?;
        rebind(&mut manifest, cancel)?;
        let existing_words = local.prepared.snapshot.manifest.data["words"]
            .iter()
            .map(|r| (text(r, "word"), r.get("id").cloned().unwrap_or(Value::Null)))
            .collect::<BTreeMap<_, _>>();
        for (table, _) in TABLES.iter().rev() {
            check_cancel(cancel)?;
            tx.execute(&format!("DELETE FROM {table}"), [])?;
        }
        let mut word_ids = BTreeMap::new();
        for row in manifest.data.get_mut("words").unwrap() {
            let temporary = text(row, "id");
            let word = text(row, "word");
            if let Some(id) = existing_words.get(&word) {
                row.insert("id".into(), id.clone());
            } else {
                row.remove("id");
            }
            insert(&tx, "words", row, false)?;
            let id: String = tx.query_row(
                "SELECT CAST(id AS TEXT) FROM words WHERE word=?",
                [word],
                |r| r.get(0),
            )?;
            word_ids.insert(temporary, id);
        }
        map_word_links(&mut manifest.data, &word_ids)?;
        for (table, _) in TABLES {
            if *table == "words" {
                continue;
            }
            for row in &manifest.data[*table] {
                check_cancel(cancel)?;
                insert(&tx, table, row, false)?;
            }
        }
        let library = manifest.library.as_mut().unwrap();
        let old_collection = setting_in(&tx, "video_favorites_v1")?;
        let old_selected =
            serde_json::from_str::<crate::video_library::Collection>(&old_collection)
                .ok()
                .and_then(|c| c.selected);
        let collection = library.videos.get_or_insert_with(Default::default);
        collection.selected =
            old_selected.filter(|id| collection.items.iter().any(|v| &v.id == id));
        collection.validate()?;
        tx.execute("INSERT INTO app_settings(key,value) VALUES('video_favorites_v1',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[serde_json::to_string(collection)?])?;
        if let Some(series) = collection.active().and_then(|v| v.series.as_ref()) {
            tx.execute("INSERT INTO app_settings(key,value) VALUES('bilibili_series',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[serde_json::to_string(series)?])?;
        } else {
            tx.execute("DELETE FROM app_settings WHERE key='bilibili_series'", [])?;
        }
        let previous_settings = lib.vault_with_settings()?.1;
        for key in previous_settings.keys() {
            if safe_setting(key)
                && ![
                    "last_page",
                    "navigation_open",
                    "library_group",
                    "last_listening_audio",
                ]
                .contains(&key.as_str())
                && !key.starts_with("chat_draft_")
                && !library.settings.contains_key(key)
            {
                tx.execute("DELETE FROM app_settings WHERE key=?", [key])?;
            }
        }
        for (key, value) in &library.settings {
            ensure!(safe_setting(key), "同步包含私有设置");
            tx.execute("INSERT INTO app_settings(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",rusqlite::params![key,value])?;
        }
        tx.execute(
            "DELETE FROM app_settings WHERE key LIKE 'video_subtitles_%'",
            [],
        )?;
        for subtitle in &library.subtitles {
            let key = crate::video_subtitles::identity_key(Path::new(&subtitle.video))?;
            tx.execute("INSERT INTO app_settings(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",rusqlite::params![key,subtitle.subtitle])?;
        }
        let max_time = state
            .records
            .values()
            .map(|e| e.version.modified_ms)
            .max()
            .unwrap_or(0);
        tx.execute(
            "UPDATE cloud_sync_identity SET last_ms=MAX(last_ms,?),applying=0 WHERE singleton=1",
            [max_time.checked_add(1).context("同步时间溢出")?],
        )?;
        tx.execute("DELETE FROM cloud_sync_entities WHERE scope=?", [scope])?;
        for (key, entry) in &state.records {
            check_cancel(cancel)?;
            tx.execute("INSERT INTO cloud_sync_entities(scope,entity_key,value,local_sequence) VALUES(?,?,?,?)",rusqlite::params![scope,key,serde_json::to_string(entry)?,local.sequence])?;
        }
        tx.execute("INSERT INTO cloud_sync_status(scope,last_success,last_error) VALUES(?,?,'') ON CONFLICT(scope) DO UPDATE SET last_success=excluded.last_success,last_error=''",rusqlite::params![scope,chrono::Utc::now().to_rfc3339()])?;
        tx.execute("DELETE FROM cloud_sync_journal WHERE id < (SELECT COALESCE(MAX(id),0)-1000 FROM cloud_sync_journal)",[])?;
        check_cancel(cancel)?;
        let live = state.records.values().filter(|e| e.value.is_some()).count();
        Ok((live, state.records.len() - live))
    })();
    let result = match outcome {
        Ok(count) => tx.commit().map(|_| count).map_err(anyhow::Error::from),
        Err(error) => {
            drop(tx);
            Err(error)
        }
    };
    if result.is_err() {
        for path in created {
            let _ = fs::remove_file(path);
        }
        for (source, trash) in moved {
            let _ = fs::rename(trash, source);
        }
    }
    result
}
