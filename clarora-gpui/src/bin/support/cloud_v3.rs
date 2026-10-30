//! Full storage/sync E2E scenarios written before the v3 implementation.
use super::cloud_fixture::{Cloud, Object};
use anyhow::{Context, Result, ensure};
use clarora_gpui::{
    backup::Retention,
    data::{Library, text},
    process,
};
use serde_json::{Value, json};
use std::{fs, io::Write, path::Path, sync::atomic::Ordering};

fn source(root: &Path) -> Result<Library> {
    let lib = Library::open(root.to_path_buf())?;
    let path = lib.root.join("media/source.bin");
    let mut file = fs::File::create(&path)?;
    for byte in [19u8, 41, 73] {
        file.write_all(&vec![byte; 8 * 1024 * 1024])?;
    }
    file.sync_all()?;
    lib.put("clip_cards",json!({"id":"portable-clip","en_text":"Portable attachment","zh_text":"附件","audio_uri":path.to_string_lossy(),"start_ms":0,"end_ms":1000,"created_at":"2026-10-29 00:00:00"}))?;
    lib.db()?.execute(
        "INSERT INTO words(word,meaning) VALUES('portable','initial')",
        [],
    )?;
    Ok(lib)
}
pub fn run(parent: &Library, results: &mut Vec<Value>) -> Result<()> {
    let root = parent.root.join("cloud-v3");
    fs::create_dir_all(&root)?;
    let cancel = process::cancel_token();
    let mut cases = vec![];
    let cloud = Cloud::new()?;
    let storage = cloud.storage()?;
    let lib = source(&root.join("source"))?;
    let first = storage.upload_incremental(&lib, &cancel, |_| {})?;
    let initial_puts = cloud.blob_puts();
    ensure!(
        initial_puts == 3,
        "first upload did not use bounded content chunks"
    );
    let second = storage.upload_incremental(&lib, &cancel, |_| {})?;
    ensure!(
        cloud.blob_puts() == initial_puts && first != second,
        "unchanged second backup uploaded duplicate chunks"
    );
    ensure!(
        storage.versions(&cancel)?.len() == 2,
        "committed versions missing"
    );
    let restored = Library::open(root.join("restored"))?;
    storage.restore(&restored, &second, &cancel)?;
    let row = restored.list("clip_cards")?.remove(0);
    ensure!(
        fs::read(text(&row, "audio_uri"))? == fs::read(lib.root.join("media/source.bin"))?,
        "v3 roundtrip changed content"
    );
    cases.push(json!({"case":"chunked deduplicated cross-version backup and verified restore","passed":true,"first":first,"second":second,"blob_puts":initial_puts}));

    let legacy = storage.upload(&lib, &cancel)?;
    let legacy_target = Library::open(root.join("legacy-target"))?;
    storage.restore(&legacy_target, &legacy, &cancel)?;
    ensure!(
        legacy_target.list("clip_cards")?.len() == 1,
        "legacy v2 restore regressed"
    );
    cases.push(json!({"case":"legacy v2 remains readable alongside v3","passed":true}));

    let corrupt_key = cloud
        .objects
        .lock()
        .unwrap()
        .keys()
        .find(|s| s.contains("/objects/"))
        .cloned()
        .context("blob absent")?;
    let previous = cloud.objects.lock().unwrap()[&corrupt_key].bytes.clone();
    cloud
        .objects
        .lock()
        .unwrap()
        .get_mut(&corrupt_key)
        .unwrap()
        .bytes[0] ^= 1;
    let target = Library::open(root.join("corrupt-target"))?;
    target.db()?.execute(
        "INSERT INTO words(word,meaning) VALUES('existing','preserve')",
        [],
    )?;
    let before = serde_json::to_value(target.vault()?)?;
    let error = storage
        .restore(&target, &second, &cancel)
        .expect_err("corrupt object accepted")
        .to_string();
    ensure!(
        serde_json::to_value(target.vault()?)? == before,
        "corrupt restore changed SQLite"
    );
    ensure!(
        fs::read_dir(target.root.join("media"))?.next().is_none(),
        "corrupt restore left partial media"
    );
    cloud
        .objects
        .lock()
        .unwrap()
        .get_mut(&corrupt_key)
        .unwrap()
        .bytes = previous;
    cases.push(json!({"case":"same-length corruption rejected before atomic apply","passed":true,"error":error}));

    let resume_cloud = Cloud::new()?;
    resume_cloud.fail_blob_at.store(2, Ordering::SeqCst);
    let resume = resume_cloud.storage()?;
    ensure!(
        resume.upload_incremental(&lib, &cancel, |_| {}).is_err(),
        "injected interrupted backup succeeded"
    );
    ensure!(
        resume.versions(&cancel)?.is_empty(),
        "incomplete upload became a visible snapshot"
    );
    let committed_before = resume_cloud
        .objects
        .lock()
        .unwrap()
        .keys()
        .filter(|s| s.contains("/objects/"))
        .cloned()
        .collect::<Vec<_>>();
    ensure!(
        !committed_before.is_empty(),
        "resume fixture has no confirmed chunk"
    );
    let attempts_before = resume_cloud
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r["method"] == "PUT" && r["key"] == committed_before[0])
        .count();
    let resumed = resume.upload_incremental(&lib, &cancel, |_| {})?;
    let attempts_after = resume_cloud
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r["method"] == "PUT" && r["key"] == committed_before[0])
        .count();
    ensure!(
        attempts_after == attempts_before,
        "retry uploaded a confirmed chunk again"
    );
    resume.restore(
        &Library::open(root.join("resumed-target"))?,
        &resumed,
        &cancel,
    )?;
    cases.push(json!({"case":"new upload resumes confirmed chunks after process-independent interruption","passed":true,"http":resume_cloud.evidence()}));

    cloud.age(14);
    let orphan="clarora/v3/objects/sha256/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string();
    cloud.objects.lock().unwrap().insert(
        orphan.clone(),
        Object {
            bytes: b"orphan".to_vec(),
            modified: chrono::Utc::now() - chrono::Duration::days(14),
        },
    );
    let policy = Retention {
        keep_last: 1,
        keep_days: 0,
        orphan_days: 7,
    };
    let stale = storage.preview_cleanup(policy.clone(), &cancel)?;
    let newest = storage.upload_incremental(&lib, &cancel, |_| {})?;
    ensure!(
        storage.cleanup(&stale, &cancel).is_err(),
        "stale cleanup plan was executed after new backup"
    );
    let plan = storage.preview_cleanup(policy, &cancel)?;
    let cleaned = storage.cleanup(&plan, &cancel)?;
    ensure!(
        !cloud.objects.lock().unwrap().contains_key(&orphan),
        "old unreferenced blob survived cleanup"
    );
    storage.restore(
        &Library::open(root.join("retained-target"))?,
        &newest,
        &cancel,
    )?;
    cases.push(json!({"case":"retention preview rejects stale plan and preserves referenced chunks","passed":true,"cleanup":serde_json::to_value(cleaned)?}));

    let race_cloud = Cloud::new()?;
    race_cloud.fail_blob_at.store(2, Ordering::SeqCst);
    let race = race_cloud.storage()?;
    ensure!(
        race.upload_incremental(&lib, &cancel, |_| {}).is_err(),
        "race fixture did not interrupt"
    );
    {
        let mut objects = race_cloud.objects.lock().unwrap();
        let catalog = objects
            .get_mut("clarora/v3/catalog.json")
            .context("race catalog")?;
        let mut value: Value = serde_json::from_slice(&catalog.bytes)?;
        value["pins"] = json!({});
        catalog.bytes = serde_json::to_vec(&value)?;
    }
    race_cloud.age(14);
    let race_plan = race.preview_cleanup(
        Retention {
            keep_last: 1,
            keep_days: 0,
            orphan_days: 7,
        },
        &cancel,
    )?;
    race_cloud.pause_deletes.store(true, Ordering::SeqCst);
    let cleanup_storage = race_cloud.storage()?;
    let worker =
        std::thread::spawn(move || cleanup_storage.cleanup(&race_plan, &process::cancel_token()));
    let began = std::time::Instant::now();
    while !race_cloud.delete_entered.load(Ordering::SeqCst)
        && began.elapsed() < std::time::Duration::from_secs(2)
    {
        std::thread::sleep(std::time::Duration::from_millis(3));
    }
    ensure!(
        race_cloud.delete_entered.load(Ordering::SeqCst),
        "cleanup did not reach physical deletion"
    );
    let during_cleanup = race.upload_incremental(&lib, &cancel, |_| {});
    race_cloud.pause_deletes.store(false, Ordering::SeqCst);
    worker
        .join()
        .map_err(|_| anyhow::anyhow!("cleanup worker panicked"))??;
    let during_cleanup = during_cleanup?;
    race.restore(
        &Library::open(root.join("race-restored"))?,
        &during_cleanup,
        &cancel,
    )?;
    cases.push(json!({"case":"retired physical keys are not reused by a concurrent new upload","passed":true,"http":race_cloud.evidence()}));

    fs::write(
        root.join("cloud-v3-storage-evidence.json"),
        serde_json::to_vec_pretty(&json!({"passed":true,"cases":cases,"http":cloud.evidence()}))?,
    )?;
    if std::env::args().any(|arg| arg == "--cloud-storage-only") {
        results.push(json!({"workflow":"v3云存储：分块去重/完整性/续传/保留与并发清理","passed":true,"cases":cases.len(),"evidence":root.join("cloud-v3-storage-evidence.json")}));
        return Ok(());
    }

    let sync_cloud = Cloud::new()?;
    let a = Library::open(root.join("device-a"))?;
    let b = Library::open(root.join("device-b"))?;
    a.db()?
        .execute("INSERT INTO words(word,meaning) VALUES('fresh','A')", [])?;
    let sync = sync_cloud.storage()?;
    sync.synchronize(&a, &cancel, |_| {})?;
    sync.synchronize(&b, &cancel, |_| {})?;
    ensure!(
        text(&b.list("words")?[0], "meaning") == "A",
        "new device did not pull data"
    );
    b.db()?
        .execute("UPDATE words SET meaning='B newer' WHERE word='fresh'", [])?;
    sync.synchronize(&b, &cancel, |_| {})?;
    sync.synchronize(&a, &cancel, |_| {})?;
    ensure!(
        text(&a.list("words")?[0], "meaning") == "B newer",
        "LWW update did not propagate"
    );
    let word_id = text(&a.list("words")?[0], "id");
    a.delete("words", &word_id)?;
    sync.synchronize(&a, &cancel, |_| {})?;
    sync.synchronize(&b, &cancel, |_| {})?;
    ensure!(
        b.list("words")?.is_empty(),
        "offline old copy resurrected deleted word"
    );
    cases
        .push(json!({"case":"two-device LWW update and durable deletion tombstone","passed":true}));

    a.db()?
        .execute("INSERT INTO words(word,meaning) VALUES('race','base')", [])?;
    sync.synchronize(&a, &cancel, |_| {})?;
    sync.synchronize(&b, &cancel, |_| {})?;
    let base = chrono::Utc::now().timestamp_millis() + 1000;
    a.db()?.execute(
        "UPDATE cloud_sync_identity SET last_ms=? WHERE singleton=1",
        [base],
    )?;
    a.db()?.execute(
        "UPDATE words SET meaning='older edit' WHERE word='race'",
        [],
    )?;
    b.db()?.execute(
        "UPDATE cloud_sync_identity SET last_ms=? WHERE singleton=1",
        [base + 1000],
    )?;
    b.db()?.execute(
        "UPDATE words SET meaning='newer edit' WHERE word='race'",
        [],
    )?;
    sync.synchronize(&b, &cancel, |_| {})?;
    sync.synchronize(&a, &cancel, |_| {})?;
    ensure!(
        a.list("words")?
            .iter()
            .any(|r| text(r, "word") == "race" && text(r, "meaning") == "newer edit"),
        "network arrival order overrode actual edit time"
    );
    a.db()?.execute(
        "UPDATE cloud_sync_identity SET last_ms=? WHERE singleton=1",
        [base + 2000],
    )?;
    let race_id = text(
        a.list("words")?
            .iter()
            .find(|r| text(r, "word") == "race")
            .context("race word missing")?,
        "id",
    );
    a.delete("words", &race_id)?;
    b.db()?.execute(
        "UPDATE cloud_sync_identity SET last_ms=? WHERE singleton=1",
        [base + 3000],
    )?;
    b.db()?.execute(
        "UPDATE words SET meaning='edited after deletion' WHERE word='race'",
        [],
    )?;
    sync.synchronize(&a, &cancel, |_| {})?;
    sync.synchronize(&b, &cancel, |_| {})?;
    sync.synchronize(&a, &cancel, |_| {})?;
    ensure!(
        a.list("words")?
            .iter()
            .any(|r| text(r, "word") == "race" && text(r, "meaning") == "edited after deletion"),
        "newer edit did not supersede older deletion"
    );
    a.db()?.execute(
        "UPDATE cloud_sync_identity SET last_ms=? WHERE singleton=1",
        [base + 4000],
    )?;
    a.db()?.execute(
        "UPDATE words SET meaning='older than deletion' WHERE word='race'",
        [],
    )?;
    b.db()?.execute(
        "UPDATE cloud_sync_identity SET last_ms=? WHERE singleton=1",
        [base + 5000],
    )?;
    let race_id = text(
        b.list("words")?
            .iter()
            .find(|r| text(r, "word") == "race")
            .context("race word missing on B")?,
        "id",
    );
    b.delete("words", &race_id)?;
    sync.synchronize(&b, &cancel, |_| {})?;
    sync.synchronize(&a, &cancel, |_| {})?;
    ensure!(
        !a.list("words")?.iter().any(|r| text(r, "word") == "race"),
        "older offline edit resurrected newer deletion"
    );
    cases.push(json!({"case":"edit-time LWW independent of network order and both delete/edit directions","passed":true}));

    let id_cloud = Cloud::new()?;
    let ids = id_cloud.storage()?;
    let c = Library::open(root.join("id-c"))?;
    let d = Library::open(root.join("id-d"))?;
    c.db()?.execute(
        "INSERT INTO words(word,meaning) VALUES('local-first','local')",
        [],
    )?;
    d.db()?.execute(
        "INSERT INTO words(word,meaning) VALUES('mapped','remote')",
        [],
    )?;
    d.db()?.execute(
        "INSERT INTO review_schedule(card_kind,card_id) VALUES('word','1')",
        [],
    )?;
    d.put("discover_favorites",json!({"key":"word:1","payload":json!({"kind":"word","id":"1","word":"mapped"}).to_string()}))?;
    ids.synchronize(&d, &cancel, |_| {})?;
    ids.synchronize(&c, &cancel, |_| {})?;
    let mapped_id = text(
        c.list("words")?
            .iter()
            .find(|r| text(r, "word") == "mapped")
            .context("mapped word absent")?,
        "id",
    );
    ensure!(
        mapped_id != "1" && text(&c.list("review_schedule")?[0], "card_id") == mapped_id,
        "cross-device integer ID was not remapped"
    );
    ensure!(
        text(&c.list("discover_favorites")?[0], "key") == format!("word:{mapped_id}"),
        "word favorite identity did not rebind"
    );
    c.delete("words", &mapped_id)?;
    ids.synchronize(&c, &cancel, |_| {})?;
    ids.synchronize(&d, &cancel, |_| {})?;
    ensure!(
        d.list("review_schedule")?.is_empty() && d.list("discover_favorites")?.is_empty(),
        "deleted word dependencies returned from another device"
    );
    cases.push(json!({"case":"independent integer word IDs and related review/favorite tombstones","passed":true}));

    let portable = source(&root.join("portable-a"))?;
    fs::write(
        portable.root.join("pdf/portable.txt"),
        "Portable book content",
    )?;
    let portable_b = Library::open(root.join("portable-b"))?;
    let portable_cloud = Cloud::new()?;
    let ps = portable_cloud.storage()?;
    ps.synchronize(&portable, &cancel, |_| {})?;
    ps.synchronize(&portable_b, &cancel, |_| {})?;
    let sync_puts = portable_cloud.blob_puts();
    ps.synchronize(&portable, &cancel, |_| {})?;
    ps.synchronize(&portable_b, &cancel, |_| {})?;
    ensure!(
        portable_cloud.blob_puts() == sync_puts,
        "local path rebinding caused repeated attachment uploads"
    );
    ensure!(
        portable_b.list("clip_cards")?.len() == 1,
        "attachment record missing after sync"
    );
    let bclip = portable_b.list("clip_cards")?.remove(0);
    ensure!(
        Path::new(&text(&bclip, "audio_uri")).is_file(),
        "synced attachment path does not exist"
    );
    ensure!(
        fs::read_dir(portable_b.root.join("pdf"))?
            .any(|p| p.is_ok_and(|p| p.path().extension().is_some_and(|e| e == "txt"))),
        "book missing after sync"
    );
    cases.push(json!({"case":"portable attachment/book identities converge across different local paths","passed":true}));

    let groups_cloud = Cloud::new()?;
    let gs = groups_cloud.storage()?;
    let ga = Library::open(root.join("group-a"))?;
    let gb = Library::open(root.join("group-b"))?;
    let audio = ga.root.join("media/group-audio.bin");
    fs::write(&audio, b"group fixture")?;
    ga.put(
        "listening_practices",
        json!({"id":"shared-group","name":"Shared"}),
    )?;
    ga.put("listening_audios", json!({"id":"shared-audio","practice_id":"shared-group","name":"Initial","audio_uri":audio.to_string_lossy(),"subtitle_uri":""}))?;
    ga.set_setting("chat_draft_private-test", "unsent private draft")?;
    gs.synchronize(&ga, &cancel, |_| {})?;
    gs.synchronize(&gb, &cancel, |_| {})?;
    ensure!(
        gb.setting("chat_draft_private-test")?.is_empty(),
        "sync copied another device's unsent draft"
    );
    let clock = chrono::Utc::now().timestamp_millis() + 10000;
    ga.db()?
        .execute("UPDATE cloud_sync_identity SET last_ms=?", [clock])?;
    ga.delete("listening_practices", "shared-group")?;
    gb.db()?
        .execute("UPDATE cloud_sync_identity SET last_ms=?", [clock + 1000])?;
    gb.db()?.execute(
        "UPDATE listening_audios SET name='Newer audio survives' WHERE id='shared-audio'",
        [],
    )?;
    gs.synchronize(&ga, &cancel, |_| {})?;
    gs.synchronize(&gb, &cancel, |_| {})?;
    gs.synchronize(&ga, &cancel, |_| {})?;
    ensure!(
        ga.list("listening_audios")?.len() == 1
            && text(&ga.list("listening_audios")?[0], "name") == "Newer audio survives"
            && ga.list("listening_practices")?.len() == 1,
        "newer audio was lost after its older group deletion"
    );
    let recovered_group = text(&ga.list("listening_practices")?[0], "id");
    gb.db()?
        .execute("UPDATE cloud_sync_identity SET last_ms=?", [clock + 2000])?;
    gb.db()?.execute(
        "UPDATE listening_audios SET name='Older edit' WHERE id='shared-audio'",
        [],
    )?;
    ga.db()?
        .execute("UPDATE cloud_sync_identity SET last_ms=?", [clock + 3000])?;
    ga.delete("listening_practices", &recovered_group)?;
    gs.synchronize(&gb, &cancel, |_| {})?;
    gs.synchronize(&ga, &cancel, |_| {})?;
    gs.synchronize(&gb, &cancel, |_| {})?;
    ensure!(
        ga.list("listening_audios")?.is_empty() && gb.list("listening_audios")?.is_empty(),
        "newer group deletion did not remove older audio"
    );
    ensure!(
        ga.setting("chat_draft_private-test")? == "unsent private draft",
        "local unsent draft changed during sync"
    );
    cases.push(json!({"case":"group delete/audio edit LWW dependencies converge; unsent drafts stay local","passed":true}));

    // Hold an actual immutable sync PUT before its catalog CAS, then let a
    // second independent SQLite library publish first. The losing writer must
    // merge/retry rather than silently overwrite the winner.
    let concurrent_cloud = Cloud::new()?;
    let ca = Library::open(root.join("concurrent-a"))?;
    let cb = Library::open(root.join("concurrent-b"))?;
    ca.db()?.execute(
        "INSERT INTO words(word,meaning) VALUES('first-device','A')",
        [],
    )?;
    cb.db()?.execute(
        "INSERT INTO words(word,meaning) VALUES('second-device','B')",
        [],
    )?;
    concurrent_cloud
        .pause_next_sync
        .store(true, Ordering::SeqCst);
    let worker_storage = concurrent_cloud.storage()?;
    let a_root = ca.root.clone();
    let worker = std::thread::spawn(move || {
        worker_storage.synchronize(&Library::open(a_root)?, &process::cancel_token(), |_| {})
    });
    wait_sync(&concurrent_cloud)?;
    let second_result = concurrent_cloud
        .storage()?
        .synchronize(&cb, &cancel, |_| {});
    concurrent_cloud.release_sync.store(true, Ordering::SeqCst);
    worker
        .join()
        .map_err(|_| anyhow::anyhow!("concurrent sync panicked"))??;
    second_result?;
    concurrent_cloud
        .storage()?
        .synchronize(&cb, &cancel, |_| {})?;
    ensure!(
        ca.list("words")?.len() == 2 && cb.list("words")?.len() == 2,
        "concurrent sync lost an independent edit"
    );
    cases.push(json!({"case":"concurrent independent-device catalog CAS retries without lost rows","passed":true,"http":concurrent_cloud.evidence()}));

    let editing_cloud = Cloud::new()?;
    let editor = Library::open(root.join("editing-during-sync"))?;
    editor.db()?.execute(
        "INSERT INTO words(word,meaning) VALUES('editing','captured')",
        [],
    )?;
    editing_cloud.pause_next_sync.store(true, Ordering::SeqCst);
    let worker_storage = editing_cloud.storage()?;
    let editor_root = editor.root.clone();
    let worker = std::thread::spawn(move || {
        worker_storage.synchronize(
            &Library::open(editor_root)?,
            &process::cancel_token(),
            |_| {},
        )
    });
    wait_sync(&editing_cloud)?;
    editor.db()?.execute(
        "UPDATE words SET meaning='edited during network' WHERE word='editing'",
        [],
    )?;
    editing_cloud.release_sync.store(true, Ordering::SeqCst);
    worker
        .join()
        .map_err(|_| anyhow::anyhow!("editing sync panicked"))??;
    let reader = Library::open(root.join("editing-reader"))?;
    editing_cloud
        .storage()?
        .synchronize(&reader, &cancel, |_| {})?;
    ensure!(
        text(&editor.list("words")?[0], "meaning") == "edited during network"
            && text(&reader.list("words")?[0], "meaning") == "edited during network",
        "network sync overwrote a local edit after capture"
    );
    cases.push(json!({"case":"local edit during actual network PUT survives guarded apply and retry","passed":true}));

    // Books are represented by content identity rather than absolute paths;
    // deleting a managed copy must not be undone by another device's cache.
    for entry in fs::read_dir(portable.root.join("pdf"))? {
        fs::remove_file(entry?.path())?;
    }
    ps.synchronize(&portable, &cancel, |_| {})?;
    ps.synchronize(&portable_b, &cancel, |_| {})?;
    ps.synchronize(&portable, &cancel, |_| {})?;
    ensure!(
        fs::read_dir(portable_b.root.join("pdf"))?.next().is_none(),
        "book tombstone revived from another device's local copy"
    );
    cases.push(json!({"case":"book deletion converges and removes managed copy from visible library","passed":true}));

    let videos_cloud = Cloud::new()?;
    let vs = videos_cloud.storage()?;
    let va = Library::open(root.join("video-a"))?;
    let vb = Library::open(root.join("video-b"))?;
    let path = va.root.join("media/source-video.mp4");
    fs::write(&path, b"must remain source-only, never upload original")?;
    let series = clarora_gpui::bilibili::Series {
        title: "Source-only series".into(),
        base: "https://www.bilibili.com/video/BV1xx411c7mD".into(),
        parts: vec![
            clarora_gpui::bilibili::Part {
                number: 1,
                title: "One".into(),
                duration: 2.,
                path: Some(path.clone()),
            },
            clarora_gpui::bilibili::Part {
                number: 2,
                title: "Two".into(),
                duration: 2.,
                path: None,
            },
        ],
        requested: 0,
        current: Some(0),
    };
    let mut collection = clarora_gpui::video_library::Collection::default();
    collection.add(&[path.clone()], Some(&series))?;
    collection.save(&va)?;
    let cues = clarora_gpui::subtitles::parse(
        "1\n00:00:00,000 --> 00:00:01,000\nPortable subtitle.\n跨设备字幕。\n",
    );
    clarora_gpui::video_subtitles::save(&va, &path, &cues, &cancel)?;
    let material =
        clarora_gpui::video_qa::build(&path, &cues, clarora_gpui::video_qa::Scope::All, 0., "")?;
    use sha2::Digest;
    let context = hex::encode(sha2::Sha256::digest(
        format!("video:{}", material.key).as_bytes(),
    ));
    clarora_gpui::chat_store::save(
        &va,
        "portable-chat",
        &context,
        "Question",
        &[json!({"role":"user","content":"Explain this subtitle"})],
    )?;
    va.set_setting(
        "last_video_position",
        &json!({"path":path,"position":0.5}).to_string(),
    )?;
    vs.synchronize(&va, &cancel, |_| {})?;
    vs.synchronize(&vb, &cancel, |_| {})?;
    let restored_videos = clarora_gpui::video_library::Collection::load(&vb)?;
    let video = restored_videos
        .items
        .first()
        .context("synced Bili video absent")?;
    ensure!(
        !video.path.is_file() && video.series.as_ref().is_some_and(|s| s.parts.len() == 2),
        "Bili sync copied original video or lost parts"
    );
    ensure!(
        clarora_gpui::video_subtitles::load(&vb, &video.path)?.len() == 1,
        "source-only subtitle binding missing"
    );
    let position: Value = serde_json::from_str(&vb.setting("last_video_position")?)?;
    ensure!(
        position["path"].as_str() == video.path.to_str() && position["position"] == 0.5,
        "synced video progress did not rebind"
    );
    ensure!(
        vb.list("ai_records")?
            .iter()
            .any(|r| text(r, "id") == "portable-chat"),
        "material conversation lost after sync"
    );
    let puts = videos_cloud.blob_puts();
    vs.synchronize(&va, &cancel, |_| {})?;
    vs.synchronize(&vb, &cancel, |_| {})?;
    ensure!(
        videos_cloud.blob_puts() == puts
            && videos_cloud
                .objects
                .lock()
                .unwrap()
                .values()
                .all(|o| o.bytes != b"must remain source-only, never upload original"),
        "source cache rebind caused repeated uploads"
    );
    let remote_path = video.path.clone();
    let edit_clock = chrono::Utc::now().timestamp_millis() + 30000;
    va.db()?
        .execute("UPDATE cloud_sync_identity SET last_ms=?", [edit_clock])?;
    let old_cues =
        clarora_gpui::subtitles::parse("1\n00:00:00,000 --> 00:00:01,000\nOlder subtitle edit.\n");
    clarora_gpui::video_subtitles::save(&va, &path, &old_cues, &cancel)?;
    vb.db()?.execute(
        "UPDATE cloud_sync_identity SET last_ms=?",
        [edit_clock + 1000],
    )?;
    let new_cues =
        clarora_gpui::subtitles::parse("1\n00:00:00,000 --> 00:00:01,000\nNewer subtitle edit.\n");
    clarora_gpui::video_subtitles::save(&vb, &remote_path, &new_cues, &cancel)?;
    vs.synchronize(&vb, &cancel, |_| {})?;
    vs.synchronize(&va, &cancel, |_| {})?;
    ensure!(
        clarora_gpui::video_subtitles::load(&va, &path)?[0].text == "Newer subtitle edit.",
        "subtitle sync used network arrival instead of actual save time"
    );
    let mut removed = clarora_gpui::video_library::Collection::load(&va)?;
    removed.remove(&removed.items[0].id.clone())?;
    removed.save(&va)?;
    vs.synchronize(&va, &cancel, |_| {})?;
    vs.synchronize(&vb, &cancel, |_| {})?;
    ensure!(
        clarora_gpui::video_library::Collection::load(&vb)?
            .items
            .is_empty(),
        "deleted video returned from other device"
    );
    cases.push(json!({"case":"Bili source-only parts, subtitle/context/progress rebinding and video deletion converge","passed":true,"http":videos_cloud.evidence()}));

    let catalog: Value = serde_json::from_slice(
        &portable_cloud.objects.lock().unwrap()["clarora/v3/catalog.json"].bytes,
    )?;
    let corrupt_sync_key = catalog["sync_head"]["objects"][0]
        .as_str()
        .context("live sync chunk absent")?
        .to_string();
    portable_cloud
        .objects
        .lock()
        .unwrap()
        .get_mut(&corrupt_sync_key)
        .unwrap()
        .bytes[0] ^= 1;
    let protected = Library::open(root.join("sync-corrupt-target"))?;
    protected.db()?.execute(
        "INSERT INTO words(word,meaning) VALUES('preserved','unchanged')",
        [],
    )?;
    let before = serde_json::to_value(protected.vault()?)?;
    ensure!(
        ps.synchronize(&protected, &cancel, |_| {}).is_err(),
        "corrupt sync chunk was accepted"
    );
    ensure!(
        serde_json::to_value(protected.vault()?)? == before
            && fs::read_dir(protected.root.join("media"))?.next().is_none(),
        "corrupt sync changed local records or left partial media"
    );
    cases.push(json!({"case":"sync corruption rejects before local database apply and removes staging","passed":true}));

    fs::write(
        root.join("cloud-v3-evidence.json"),
        serde_json::to_vec_pretty(
            &json!({"passed":true,"cases":cases,"objects":cloud.evidence(),"sync":sync_cloud.evidence(),"portable":portable_cloud.evidence()}),
        )?,
    )?;
    results.push(json!({"workflow":"v3云存储：跨版本分块去重/校验/续传/保留清理/跨设备LWW与删除同步","passed":true,"cases":cases.len(),"evidence":root.join("cloud-v3-evidence.json")}));
    Ok(())
}

fn wait_sync(cloud: &Cloud) -> Result<()> {
    let began = std::time::Instant::now();
    while !cloud.sync_entered.load(Ordering::SeqCst)
        && began.elapsed() < std::time::Duration::from_secs(5)
    {
        std::thread::sleep(std::time::Duration::from_millis(3));
    }
    ensure!(
        cloud.sync_entered.load(Ordering::SeqCst),
        "sync PUT did not reach concurrency barrier"
    );
    Ok(())
}
