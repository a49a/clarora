//! Full-library backup E2E planned before implementation: real files, SQLite, ZIP and HTTP.
use anyhow::{Context, Result, ensure};
use clarora_gpui::{
    backup, bilibili, chat_store, data::Library, documents, ebook, media::Player, process,
    subtitles, video_library, video_qa,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
fn hash(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}
fn context(page: &str, material: &str) -> String {
    hash(&format!("{page}:{material}"))
}
fn zip_file(path: &Path, entries: &[(String, Vec<u8>)]) -> Result<()> {
    let mut zip = zip::ZipWriter::new(fs::File::create(path)?);
    for (name, bytes) in entries {
        zip.start_file(name, zip::write::SimpleFileOptions::default())?;
        zip.write_all(bytes)?;
    }
    zip.finish()?.sync_all()?;
    Ok(())
}
fn epub(path: &Path) -> Result<()> {
    zip_file(path, &[
        ("mimetype".into(), b"application/epub+zip".to_vec()),
        ("META-INF/container.xml".into(), b"<container><rootfiles><rootfile full-path='book.opf'/></rootfiles></container>".to_vec()),
        ("book.opf".into(), b"<package xmlns:dc='http://purl.org/dc/elements/1.1/' version='3.0'><metadata><dc:title>Backup EPUB</dc:title></metadata><manifest><item id='a' href='a.xhtml' media-type='application/xhtml+xml'/></manifest><spine><itemref idref='a'/></spine></package>".to_vec()),
        ("a.xhtml".into(), format!("<html><body><h1>Backup EPUB</h1>{}</body></html>", "<p>Portable reading progress and preserved content.</p>".repeat(400)).into_bytes()),
    ])
}
struct Cloud {
    address: std::net::SocketAddr,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    objects: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    mutate: Arc<Mutex<Option<PathBuf>>>,
}
impl Cloud {
    fn start() -> Result<Self> {
        let server = TcpListener::bind("127.0.0.1:0")?;
        let address = server.local_addr()?;
        server.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let objects = Arc::new(Mutex::new(BTreeMap::<String, Vec<u8>>::new()));
        let mutate = Arc::new(Mutex::new(None::<PathBuf>));
        let mutation = mutate.clone();
        let (done, store) = (stop.clone(), objects.clone());
        let worker = thread::spawn(move || {
            while !done.load(Ordering::SeqCst) {
                let Ok((mut socket, _)) = server.accept() else {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                };
                let result = (|| -> Result<()> {
                    socket.set_nonblocking(false)?;
                    socket.set_read_timeout(Some(Duration::from_secs(10)))?;
                    let mut raw = vec![];
                    let mut chunk = [0; 65536];
                    let split = loop {
                        let n = socket.read(&mut chunk)?;
                        ensure!(n > 0, "request ended");
                        raw.extend_from_slice(&chunk[..n]);
                        if let Some(i) = raw.windows(4).position(|s| s == b"\r\n\r\n") {
                            break i + 4;
                        }
                        ensure!(raw.len() < 65536, "headers too large");
                    };
                    let headers = String::from_utf8(raw[..split].to_vec())?;
                    let first = headers.lines().next().context("no request")?;
                    let mut first = first.split_whitespace();
                    let method = first.next().context("no method")?;
                    let target = first.next().context("no target")?;
                    let key = target.trim_start_matches("/fixture-bucket/").to_string();
                    let length = headers
                        .lines()
                        .filter_map(|s| s.split_once(':'))
                        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                        .and_then(|(_, v)| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    let mut body = raw[split..].to_vec();
                    while body.len() < length {
                        let n = socket
                            .read(&mut chunk)
                            .with_context(|| format!("body read {} / {}", body.len(), length))?;
                        ensure!(n > 0, "body ended");
                        body.extend_from_slice(&chunk[..n]);
                    }
                    let body = if method == "PUT" {
                        if key.contains("/media/") {
                            if let Some(path) = mutation.lock().unwrap().take() {
                                fs::OpenOptions::new()
                                    .append(true)
                                    .open(path)?
                                    .write_all(b"changed during upload")?;
                            }
                        }
                        store.lock().unwrap().insert(key, body);
                        vec![]
                    } else if target.contains("list-type=2") {
                        format!("<ListBucketResult>{}<IsTruncated>false</IsTruncated></ListBucketResult>",store.lock().unwrap().keys().filter(|s|s.contains("/snapshots/")).map(|s|format!("<Contents><Key>{s}</Key></Contents>")).collect::<String>()).into_bytes()
                    } else {
                        store
                            .lock()
                            .unwrap()
                            .get(&key)
                            .context("no object")?
                            .clone()
                    };
                    write!(
                        socket,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )?;
                    socket.write_all(&body)?;
                    Ok(())
                })();
                if let Err(error) = result {
                    if !done.load(Ordering::SeqCst) {
                        eprintln!("Full-backup HTTP fixture error: {error:#}");
                    }
                    break;
                }
            }
        });
        Ok(Self {
            address,
            stop,
            worker: Some(worker),
            objects,
            mutate,
        })
    }
    fn storage(&self) -> Result<backup::Storage> {
        backup::Storage::loopback_fixture(self.address)
    }
}
impl Drop for Cloud {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = vec![];
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        if path.is_dir() {
            files.extend(self::files(&path)?);
        } else {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}
fn assert_restored(
    lib: &Library,
    expected_epub: &str,
    original_doc_context: &str,
    original_video_context: &str,
    decode: bool,
) -> Result<Value> {
    let collection = video_library::Collection::load(lib)?;
    ensure!(collection.items.len() == 1, "video collection missing");
    let video = collection.active().context("selected video missing")?;
    ensure!(
        video.path.starts_with(&lib.root) && video.path.is_file(),
        "video path not relocated"
    );
    let series = video.series.as_ref().context("parts missing")?;
    ensure!(
        series.current == Some(1) && series.parts.len() == 3 && series.parts[2].path.is_none(),
        "part selection/uncached part changed"
    );
    for part in &series.parts[..2] {
        let path = part.path.as_ref().context("cached part lost")?;
        ensure!(
            path.starts_with(&lib.root) && path.is_file(),
            "cached part not relocated"
        );
        let cues = clarora_gpui::video_subtitles::load(lib, path)?;
        ensure!(
            cues.len() == 1 && cues[0].text.contains("Restored subtitle"),
            "subtitle identity not rebound"
        );
    }
    let position: Value = serde_json::from_str(&lib.setting("last_video_position")?)?;
    ensure!(
        position["path"].as_str() == video.path.to_str() && position["position"] == 0.25,
        "video position not relocated"
    );
    let current: Value = serde_json::from_str(&lib.setting("pdf_current")?)?;
    let pdf = PathBuf::from(current["path"].as_str().context("current doc path lost")?);
    ensure!(
        pdf.starts_with(lib.root.join("pdf")) && current["width"] == 1200,
        "PDF current path/zoom lost"
    );
    let rendered = documents::load_pdf_page(lib, &pdf, 0, 1200)?;
    ensure!(
        rendered.text.contains("Clarora GPUI"),
        "restored PDF cannot render"
    );
    let doc_context = context("pdf", &pdf.to_string_lossy());
    ensure!(
        doc_context != original_doc_context && chat_store::list(lib, &doc_context)?.len() == 1,
        "document material chat context not rebound"
    );
    let doc_files = files(&lib.root.join("pdf"))?;
    let book = doc_files
        .iter()
        .find(|p| p.extension().is_some_and(|e| e == "epub"))
        .context("EPUB missing")?;
    let read = ebook::open(lib, book, false, &process::cancel_token())?;
    ensure!(
        read.book.identity == expected_epub && read.page == 2 && read.font_size == 26,
        "EPUB identity/progress/font lost"
    );
    let txt = doc_files
        .iter()
        .find(|p| p.extension().is_some_and(|e| e == "txt"))
        .context("TXT missing")?;
    let txt_read = ebook::open(lib, txt, false, &process::cancel_token())?;
    ensure!(
        txt_read.page == 1 && txt_read.font_size == 22,
        "TXT progress/font lost"
    );
    let material = video_qa::build(
        &video.path,
        &clarora_gpui::video_subtitles::load(lib, &video.path)?,
        video_qa::Scope::All,
        0.,
        "",
    )?;
    let video_context = context("video", &material.key);
    ensure!(
        lib.setting(&format!("chat_draft_{doc_context}"))? == "Unsaved question",
        "document draft context not rebound"
    );
    ensure!(
        video_context != original_video_context
            && chat_store::list(lib, &video_context)?.len() == 1,
        "video material chat context not rebound"
    );
    ensure!(
        lib.setting("ai_config")?.is_empty()
            && lib.setting("storage_config")?.is_empty()
            && lib.setting("command")?.is_empty(),
        "sensitive settings restored"
    );
    if decode {
        let player = Player::without_audio_device();
        player.open_range(&video.path, 0.25, 0., true);
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let state = player.snapshot();
            ensure!(
                state.error.is_empty(),
                "restored video decode failed: {}",
                state.error
            );
            if state.frame.is_some() && state.duration > 0. {
                break;
            }
            ensure!(Instant::now() < deadline, "restored video did not decode");
            thread::sleep(Duration::from_millis(20));
        }
    }
    Ok(
        json!({"video":video,"current_pdf":current,"epub_identity":read.book.identity,"epub_page":read.page,"epub_font":read.font_size,"txt_page":txt_read.page,"material_contexts":[doc_context,video_context],"rendered_pdf_text":rendered.text}),
    )
}
pub fn run(parent: &Library, fixture: &Path, results: &mut Vec<Value>) -> Result<()> {
    let root = parent.root.join("backup-library");
    fs::create_dir_all(&root)?;
    let source = Library::open(root.join("source"))?;
    let cancel = process::cancel_token();
    let mut cases = vec![];
    let a = source.root.join("media/part-one.mp4");
    let b = source.root.join("media/part-two.mp4");
    fs::copy(fixture, &a)?;
    fs::copy(fixture, &b)?;
    let series = bilibili::Series {
        title: "Portable source".into(),
        base: "https://www.bilibili.com/video/BV1xx411c7mD".into(),
        parts: vec![
            bilibili::Part {
                number: 1,
                title: "P1".into(),
                duration: 4.,
                path: Some(a.clone()),
            },
            bilibili::Part {
                number: 2,
                title: "P2".into(),
                duration: 4.,
                path: Some(b.clone()),
            },
            bilibili::Part {
                number: 3,
                title: "P3".into(),
                duration: 4.,
                path: None,
            },
        ],
        requested: 1,
        current: Some(1),
    };
    let mut collection = video_library::Collection::default();
    let ids = collection.add(&[a.clone(), b.clone()], Some(&series))?;
    collection.select(&ids[0])?;
    collection.save(&source)?;
    series.save(&source)?;
    let cues = subtitles::parse("1\n00:00:00,000 --> 00:00:01,000\nRestored subtitle\n恢复字幕\n");
    for path in [&a, &b] {
        clarora_gpui::video_subtitles::save(&source, path, &cues, &cancel)?;
    }
    source.set_setting(
        "last_video_position",
        &json!({"path":b,"position":0.25}).to_string(),
    )?;
    let pdf = source.root.join("pdf/portable.pdf");
    super::report_pdf(&pdf)?;
    source.set_setting(
        "pdf_current",
        &json!({"path":pdf,"page":0,"width":1200}).to_string(),
    )?;
    let input = root.join("portable.epub");
    epub(&input)?;
    let mut reading = ebook::open(&source, &input, true, &cancel)?;
    reading.accept_import();
    let epub_identity = reading.book.identity.clone();
    reading.page = 2;
    reading.font_size = 26;
    ebook::save_progress(&source, &reading)?;
    let text = root.join("portable.txt");
    fs::write(&text, "Portable text reading.\n".repeat(1000))?;
    let mut txt = ebook::open(&source, &text, true, &cancel)?;
    txt.accept_import();
    txt.page = 1;
    txt.font_size = 22;
    ebook::save_progress(&source, &txt)?;
    let doc_context = context("pdf", &pdf.to_string_lossy());
    chat_store::save(
        &source,
        "document-chat",
        &doc_context,
        "Document",
        &[json!({"role":"user","content":"Explain this"})],
    )?;
    let material = video_qa::build(&b, &cues, video_qa::Scope::All, 0., "")?;
    let video_context = context("video", &material.key);
    chat_store::save(
        &source,
        "video-chat",
        &video_context,
        "Video",
        &[json!({"role":"user","content":"Explain video"})],
    )?;
    source.set_setting(&format!("chat_draft_{doc_context}"), "Unsaved question")?;
    for key in [
        "ai_config",
        "storage_config",
        "command",
        "unknown_private_setting",
    ] {
        source.set_setting(key, "PRIVATE_SENTINEL_SHOULD_NOT_EXPORT")?;
    }
    source.db()?.execute("INSERT INTO local_credentials(name,value) VALUES('sentinel','PRIVATE_SENTINEL_SHOULD_NOT_EXPORT')",[])?;
    source.set_setting("video_subtitle_size", "28")?;
    let archive = root.join("full-library.zip");
    backup::export_local_cancellable(&source, &archive, &cancel)?;
    let mut zip = zip::ZipArchive::new(fs::File::open(&archive)?)?;
    let mut raw = String::new();
    zip.by_name("manifest.json")?.read_to_string(&mut raw)?;
    ensure!(
        !raw.contains("PRIVATE_SENTINEL") && !raw.contains("unknown_private_setting"),
        "private metadata leaked"
    );
    fs::write(root.join("exported-manifest.json"), &raw)?;
    let cloud = Cloud::start()?;
    let storage = cloud.storage()?;
    let remote = storage.upload(&source, &cancel)?;
    let changing = Library::open(root.join("changing-source"))?;
    let changing_file = changing.root.join("media/changing.bin");
    fs::write(&changing_file, vec![42u8; 65536])?;
    changing.put("clip_cards",json!({"id":"changing-clip","en_text":"test","zh_text":"","audio_uri":changing_file.to_string_lossy(),"start_ms":0,"end_ms":1000,"created_at":"2026-10-23 00:00:00"}))?;
    let mutation = Cloud::start()?;
    *mutation.mutate.lock().unwrap() = Some(changing_file);
    let error = mutation
        .storage()?
        .upload(&changing, &cancel)
        .err()
        .context("changed source attachment was published")?;
    ensure!(
        error.to_string().contains("附件已改变")
            && !mutation
                .objects
                .lock()
                .unwrap()
                .keys()
                .any(|key| key.contains("/snapshots/")),
        "changed attachment not stopped before manifest publication"
    );
    cases.push(json!({"case":"source attachment mutation prevents cloud snapshot publication","passed":true,"error":error.to_string()}));
    fs::rename(&source.root, root.join("source-unavailable"))?;
    let local = Library::open(root.join("local-restored"))?;
    backup::import_local_cancellable(&local, &archive, &cancel)?;
    let evidence = assert_restored(&local, &epub_identity, &doc_context, &video_context, true)?;
    cases
        .push(json!({"case":"ZIP full library after source moved","passed":true,"state":evidence}));
    let paths_before = files(&local.root.join("media"))?;
    let books_before = files(&local.root.join("pdf"))?;
    local.set_setting("video_subtitle_size", "34")?;
    backup::import_local(&local, &archive)?;
    ensure!(
        files(&local.root.join("media"))? == paths_before
            && files(&local.root.join("pdf"))? == books_before
            && local.setting("video_subtitle_size")? == "34",
        "repeat restore created orphans/overwrote preferences"
    );
    cases.push(
        json!({"case":"repeat restore file dedup and destination preferences","passed":true}),
    );
    let mut deselected = video_library::Collection::load(&local)?;
    let selected = deselected.selected.clone();
    deselected.selected = None;
    deselected.save(&local)?;
    backup::import_local(&local, &archive)?;
    ensure!(
        video_library::Collection::load(&local)?.selected.is_none(),
        "repeat restore changed deliberate video deselection"
    );
    deselected.selected = selected;
    deselected.save(&local)?;
    cases.push(json!({"case":"repeat restore preserves deliberate deselection","passed":true}));
    let disaster = Library::open(root.join("lost-local-attachments"))?;
    backup::import_local(&disaster, &archive)?;
    let before_video = video_library::Collection::load(&disaster)?
        .active()
        .context("missing disaster video")?
        .path
        .clone();
    let old_chat = chat_store::list(
        &disaster,
        &context(
            "video",
            &video_qa::build(
                &before_video,
                &clarora_gpui::video_subtitles::load(&disaster, &before_video)?,
                video_qa::Scope::All,
                0.,
                "",
            )?
            .key,
        ),
    )?
    .remove(0);
    let mut messages = old_chat.messages.clone();
    messages.push(json!({"role":"assistant","content":"Newer local reply must survive restore"}));
    chat_store::save(
        &disaster,
        &old_chat.id,
        &old_chat.context,
        &old_chat.title,
        &messages,
    )?;
    for path in files(&disaster.root.join("media"))?
        .into_iter()
        .chain(files(&disaster.root.join("pdf"))?)
    {
        fs::remove_file(path)?;
    }
    backup::import_local(&disaster, &archive)?;
    let repaired = assert_restored(
        &disaster,
        &epub_identity,
        &doc_context,
        &video_context,
        false,
    )?;
    let repaired_video = video_library::Collection::load(&disaster)?
        .active()
        .context("missing repaired video")?
        .path
        .clone();
    ensure!(
        repaired_video != before_video,
        "lost video path not repaired"
    );
    let repaired_context = context(
        "video",
        &video_qa::build(
            &repaired_video,
            &clarora_gpui::video_subtitles::load(&disaster, &repaired_video)?,
            video_qa::Scope::All,
            0.,
            "",
        )?
        .key,
    );
    ensure!(
        chat_store::list(&disaster, &repaired_context)?[0].messages == messages,
        "repair lost newer local chat history"
    );
    cases.push(json!({"case":"existing SQLite library recovers missing media/books/cache and rebinds existing chat without losing local replies","passed":true,"state":repaired}));
    let reopened = Library::open(local.root.clone())?;
    assert_restored(
        &reopened,
        &epub_identity,
        &doc_context,
        &video_context,
        false,
    )?;
    let partial = Library::open(root.join("partial-series"))?;
    let cached = partial.root.join("media/already-downloaded.mp4");
    fs::copy(fixture, &cached)?;
    let mut old_series = series.clone();
    old_series.parts.truncate(1);
    old_series.parts[0].path = Some(cached.clone());
    old_series.current = Some(0);
    old_series.requested = 0;
    let mut previous = video_library::Collection::default();
    let keys = previous.add(std::slice::from_ref(&cached), Some(&old_series))?;
    previous.select(&keys[0])?;
    previous.save(&partial)?;
    partial.set_setting(
        "last_video_position",
        &json!({"path":cached,"position":0.75}).to_string(),
    )?;
    backup::import_local(&partial, &archive)?;
    let merged = video_library::Collection::load(&partial)?;
    let active = merged.active().context("existing selected source lost")?;
    let parts = &active
        .series
        .as_ref()
        .context("merged source lost parts")?
        .parts;
    ensure!(
        active.path == cached
            && parts.len() == 3
            && parts[1].path.as_ref().is_some_and(|p| p.is_file())
            && parts[2].path.is_none(),
        "restore dropped newly available P or changed existing current P"
    );
    let old_position: Value = serde_json::from_str(&partial.setting("last_video_position")?)?;
    ensure!(
        old_position["position"] == 0.75,
        "restore overwrote valid local video position"
    );
    cases.push(json!({"case":"same source merges new P metadata/cache while preserving valid current P and position","passed":true}));
    let remote_lib = Library::open(root.join("cloud-restored"))?;
    storage.restore(&remote_lib, &remote, &cancel)?;
    let state = assert_restored(
        &remote_lib,
        &epub_identity,
        &doc_context,
        &video_context,
        false,
    )?;
    cases.push(json!({"case":"loopback cloud full-library roundtrip","passed":true,"state":state,"objects":cloud.objects.lock().unwrap().keys().cloned().collect::<Vec<_>>()}));
    let before = local.vault()?;
    let before_media = files(&local.root.join("media"))?;
    let broken = root.join("missing-attachment.zip");
    let mut entries = vec![];
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        if entry.name()? == "manifest.json" {
            continue;
        }
        let mut bytes = vec![];
        entry.read_to_end(&mut bytes)?;
        entries.push((entry.name()?.to_string(), bytes));
    }
    let complete_entries = entries.clone();
    let p3 = Library::open(root.join("only-third-part-cached"))?;
    let p3_cache = p3.root.join("media/previous-third-part.mp4");
    fs::copy(fixture, &p3_cache)?;
    let mut third = series.clone();
    third.parts[0].path = None;
    third.parts[1].path = None;
    third.parts[2].path = Some(p3_cache.clone());
    third.current = Some(2);
    third.requested = 2;
    let mut third_collection = video_library::Collection::default();
    let keys = third_collection.add(std::slice::from_ref(&p3_cache), Some(&third))?;
    third_collection.select(&keys[0])?;
    third_collection.save(&p3)?;
    let mut first_manifest: Value = serde_json::from_str(&raw)?;
    let first_path =
        first_manifest["library"]["videos"]["items"][0]["series"]["parts"][0]["path"].clone();
    first_manifest["library"]["videos"]["items"][0]["path"] = first_path.clone();
    first_manifest["library"]["videos"]["items"][0]["series"]["current"] = json!(0);
    first_manifest["library"]["videos"]["items"][0]["series"]["requested"] = json!(0);
    first_manifest["library"]["settings"]["last_video_position"] =
        json!(json!({"path":first_path,"position":0.25}).to_string());
    let mut first_entries = complete_entries.clone();
    first_entries.push(("manifest.json".into(), serde_json::to_vec(&first_manifest)?));
    let first_zip = root.join("selected-first-part.zip");
    zip_file(&first_zip, &first_entries)?;
    backup::import_local(&p3, &first_zip)?;
    let restored = video_library::Collection::load(&p3)?;
    let active = restored.active().context("P3 selected source lost")?;
    let restored_parts = &active
        .series
        .as_ref()
        .context("P3 source metadata lost")?
        .parts;
    ensure!(
        active.path == p3_cache
            && active.series.as_ref().unwrap().current == Some(2)
            && restored_parts
                .iter()
                .enumerate()
                .all(|(i, part)| part.number == i as u32 + 1
                    && part.path.as_ref().is_some_and(|p| p.is_file())),
        "P3-only restore changed order/current selection or lost cache"
    );
    let paths = restored_parts
        .iter()
        .map(|part| part.path.clone().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    ensure!(paths.len() == 3, "P1/P2 incorrectly remapped to P3 cache");
    for part in &restored_parts[..2] {
        ensure!(
            clarora_gpui::video_subtitles::load(&p3, part.path.as_ref().unwrap())?.len() == 1,
            "new P subtitle bound to wrong part"
        );
    }
    cases.push(json!({"case":"only P3 cached locally plus imported selected P1 retains independent P1/P2/P3 paths and P3 selection","passed":true}));

    if !entries.is_empty() {
        entries.remove(0);
    }
    entries.push(("manifest.json".into(), raw.as_bytes().to_vec()));
    zip_file(&broken, &entries)?;
    ensure!(
        backup::import_local(&local, &broken).is_err(),
        "missing attachment accepted"
    );
    ensure!(
        local.vault()? == before && files(&local.root.join("media"))? == before_media,
        "failed restore changed database/files"
    );
    cases.push(json!({"case":"missing attachment atomic rollback","passed":true}));
    let mut malicious: Value = serde_json::from_str(&raw)?;
    malicious["library"]["settings"]["ai_config"] = json!("injected-secret");
    entries = complete_entries;
    entries.retain(|(key, _)| key != "manifest.json");
    entries.push(("manifest.json".into(), serde_json::to_vec(&malicious)?));
    let attack = root.join("injected-settings.zip");
    zip_file(&attack, &entries)?;
    ensure!(
        backup::import_local(&local, &attack).is_err() && local.setting("ai_config")?.is_empty(),
        "untrusted setting injection accepted"
    );
    cases.push(json!({"case":"restore rejects non-whitelisted setting","passed":true}));
    for name in [
        "../outside.pdf",
        "..\\outside.pdf",
        "C:outside.pdf",
        "C:\\outside.pdf",
        "/outside.pdf",
        "CON.pdf",
        "book.pdf ",
    ] {
        let mut attack: Value = serde_json::from_str(&raw)?;
        attack["library"]["books"][0]["name"] = json!(name);
        let mut attack: backup::Manifest = serde_json::from_value(attack)?;
        let error = backup::validate(&mut attack)
            .err()
            .context("unsafe book name accepted")?;
        ensure!(
            error.to_string().contains("书籍名称"),
            "bad filename failed for wrong reason"
        );
    }
    cases.push(json!({"case":"cross-platform absolute/traversal/reserved book paths rejected","passed":true}));
    let collision = Library::open(root.join("book-name-collision"))?;
    fs::write(
        collision.root.join("pdf/portable.pdf"),
        b"existing PDF bytes remain untouched",
    )?;
    backup::import_local(&collision, &archive)?;
    ensure!(
        fs::read(collision.root.join("pdf/portable.pdf"))?
            == b"existing PDF bytes remain untouched",
        "same-name book overwritten"
    );
    assert_restored(
        &collision,
        &epub_identity,
        &doc_context,
        &video_context,
        false,
    )?;
    cases.push(json!({"case":"same filename different book bytes preserves both","passed":true}));
    let rejected = Library::open(root.join("database-failure"))?;
    fs::write(
        rejected.root.join("pdf/portable.pdf"),
        b"prior file must survive rollback",
    )?;
    video_library::Collection::default().save(&rejected)?;
    rejected.set_setting("video_subtitle_size", "30")?;
    let before = rejected.vault()?;
    let before_collection = rejected.setting("video_favorites_v1")?;
    let before_books = files(&rejected.root.join("pdf"))?;
    rejected.db()?.execute_batch("CREATE TRIGGER fixture_reject_restore BEFORE INSERT ON ai_records BEGIN SELECT RAISE(ABORT,'fixture restore transaction rejected'); END;")?;
    let error = backup::import_local(&rejected, &archive)
        .err()
        .context("DB trigger failed to reject restore")?;
    ensure!(
        error
            .to_string()
            .contains("fixture restore transaction rejected"),
        "merge failed before intended commit boundary: {error:#}"
    );
    ensure!(
        rejected.vault()? == before
            && rejected.setting("video_favorites_v1")? == before_collection
            && rejected.setting("video_subtitle_size")? == "30"
            && rejected.setting("pdf_current")?.is_empty(),
        "failed DB merge changed data/settings"
    );
    ensure!(
        files(&rejected.root.join("media"))?.is_empty()
            && files(&rejected.root.join("pdf"))? == before_books
            && fs::read(rejected.root.join("pdf/portable.pdf"))?
                == b"prior file must survive rollback",
        "DB failure left stage or books or destroyed original"
    );
    cases.push(json!({"case":"downloaded media and installed books rolled back on SQLite trigger failure","passed":true,"error":error.to_string()}));
    let waiting = Library::open(root.join("cancel-during-merge"))?;
    let writer = waiting.db()?;
    writer.execute_batch("BEGIN IMMEDIATE")?;
    let cancellation = process::cancel_token();
    let (target, zip_path, flag) = (waiting.clone(), archive.clone(), cancellation.clone());
    let (send, receive) = std::sync::mpsc::channel();
    let worker = thread::spawn(move || {
        let _ = send.send(backup::import_local_cancellable(&target, &zip_path, &flag));
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    while files(&waiting.root.join("pdf"))?.len() < 3 {
        ensure!(
            Instant::now() < deadline,
            "local restore never installed book before SQLite writer wait"
        );
        thread::sleep(Duration::from_millis(10));
    }
    thread::sleep(Duration::from_millis(100));
    ensure!(
        receive.try_recv().is_err(),
        "local restore completed despite held SQLite writer"
    );
    let started = Instant::now();
    cancellation.store(true, Ordering::SeqCst);
    let outcome = receive.recv_timeout(Duration::from_secs(1));
    let latency = started.elapsed().as_millis();
    writer.execute_batch("ROLLBACK")?;
    worker
        .join()
        .map_err(|_| anyhow::anyhow!("local restore thread panicked"))?;
    let error = outcome
        .context("local merge cancellation stalled")?
        .err()
        .context("cancelled merge committed")?;
    ensure!(
        error.to_string().contains("已取消")
            && waiting.vault()?.values().all(Vec::is_empty)
            && files(&waiting.root.join("media"))?.is_empty()
            && files(&waiting.root.join("pdf"))?.is_empty(),
        "local cancellation failed to roll back books/media/SQLite"
    );
    cases.push(json!({"case":"local cancellation after book install while SQLite writer locked","passed":true,"cancel_ms":latency}));
    let legacy_source = Library::open(root.join("legacy-source"))?;
    legacy_source.practice("Legacy")?;
    let (manifest, _) = backup::collect(&legacy_source)?;
    let mut legacy = serde_json::to_value(manifest)?;
    legacy.as_object_mut().unwrap().remove("library");
    let legacy_zip = root.join("legacy-v2.zip");
    zip_file(
        &legacy_zip,
        &[("manifest.json".into(), serde_json::to_vec(&legacy)?)],
    )?;
    let legacy_target = Library::open(root.join("legacy-target"))?;
    backup::import_local(&legacy_target, &legacy_zip)?;
    ensure!(
        legacy_target.list("listening_practices")?.len() == 1,
        "old v2 no longer supported"
    );
    let legacy_attachment = legacy_source.root.join("media/legacy.bin");
    fs::write(&legacy_attachment, b"legacy local attachment")?;
    legacy_source.put("clip_cards",json!({"id":"legacy-clip","en_text":"legacy","zh_text":"","audio_uri":legacy_attachment.to_string_lossy(),"start_ms":0,"end_ms":1000,"created_at":"2026-10-23 00:00:00"}))?;
    let rn = Library::open(root.join("rn-target"))?;
    backup::import_sqlite(&rn, &legacy_source.root.join("clarora.db"))?;
    let first_import = files(&rn.root.join("media"))?;
    backup::import_sqlite(&rn, &legacy_source.root.join("clarora.db"))?;
    ensure!(
        files(&rn.root.join("media"))? == first_import,
        "repeated SQLite migration left orphan attachments"
    );
    ensure!(
        rn.list("listening_practices")?.len() == 1,
        "RN import regressed"
    );
    cases.push(json!({"case":"legacy v2 and SQLite import compatibility","passed":true}));
    let snapshot = Library::open(root.join("consistent-snapshot"))?;
    let shared = snapshot.root.join("media/stable.bin");
    fs::write(&shared, b"immutable attachment for concurrent snapshot")?;
    let write_snapshot = move |db: &mut rusqlite::Connection, generation: usize| -> Result<()> {
        let tx = db.transaction()?;
        let key = format!("generation-{generation}");
        tx.execute("DELETE FROM listening_practices", [])?;
        tx.execute(
            "INSERT INTO listening_practices(id,name) VALUES(?,?)",
            rusqlite::params![key, key],
        )?;
        tx.execute("INSERT INTO listening_audios(id,practice_id,name,audio_uri,subtitle_uri) VALUES(?,?,?,?,'')",rusqlite::params![key,key,key,shared.to_string_lossy()])?;
        for setting in ["library_group", "last_listening_audio"] {
            tx.execute("INSERT INTO app_settings(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",rusqlite::params![setting,key])?;
        }
        tx.commit()?;
        Ok(())
    };
    write_snapshot(&mut snapshot.db()?, 0)?;
    let copy = snapshot.clone();
    let writer = thread::spawn(move || -> Result<()> {
        let mut db = copy.db()?;
        for i in 1..100 {
            write_snapshot(&mut db, i)?;
            thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    });
    for _ in 0..20 {
        let (manifest, _) = backup::collect(&snapshot)?;
        let practice = manifest.data["listening_practices"][0]["id"]
            .as_str()
            .context("snapshot group ID missing")?;
        ensure!(
            manifest.data["listening_audios"][0]["practice_id"] == practice
                && manifest
                    .library
                    .as_ref()
                    .context("snapshot metadata missing")?
                    .settings["library_group"]
                    == practice,
            "tables/settings came from different SQLite snapshots"
        );
    }
    writer
        .join()
        .map_err(|_| anyhow::anyhow!("snapshot writer panicked"))??;
    cases.push(json!({"case":"tables and whitelisted settings share WAL snapshot during 99 concurrent commits","passed":true,"snapshots":20}));
    let cancelled = process::cancel_token();
    cancelled.store(true, Ordering::SeqCst);
    let preserved = root.join("cancel-preserve.zip");
    fs::write(&preserved, b"preserve existing")?;
    ensure!(
        backup::export_local_cancellable(&local, &preserved, &cancelled).is_err()
            && fs::read(&preserved)? == b"preserve existing",
        "cancelled export overwrote existing backup"
    );
    ensure!(
        backup::import_local_cancellable(&local, &archive, &cancelled).is_err()
            && files(&local.root.join("media"))? == before_media,
        "pre-cancelled restore changed files"
    );
    cases.push(
        json!({"case":"local pre-cancellation preserves prior archive and library","passed":true}),
    );
    fs::write(
        root.join("full-library-evidence.json"),
        serde_json::to_vec_pretty(&cases)?,
    )?;
    results.push(json!({"workflow":"完整资料备份：视频分P/字幕重绑定、PDF/EPUB/TXT/进度、材料会话、ZIP/云端/旧包/失败及取消","passed":true,"cases":cases.len(),"evidence":"backup-library/full-library-evidence.json"}));
    Ok(())
}
