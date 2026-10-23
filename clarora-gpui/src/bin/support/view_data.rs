//! Real persisted records/files → explicit snapshot reload → last-known-good failure.
use anyhow::{Result, ensure};
use clarora_gpui::{
    data::{Library, Row},
    tts, view_data,
};
use serde_json::{Value, json};
use std::{fs, path::Path};

fn record(root: &Path, evidence: &mut Vec<Value>, case: &str, details: Value) -> Result<()> {
    evidence.push(json!({"case":case,"passed":true,"details":details}));
    fs::write(
        root.join("view-data-evidence.json"),
        serde_json::to_vec_pretty(evidence)?,
    )?;
    Ok(())
}

fn visible(state: &view_data::State) -> Value {
    json!({"speaking":state.speaking,"ocr":state.ocr,"stats":state.stats,"words":state.words,
        "pdf_files":state.pdf_files,"available_audio":state.available_audio,"asr_models":state.asr_models,
        "tts_ready":state.tts_ready,"saved_ai":state.saved_ai,"needs_legacy_import":state.needs_legacy_import,"revision":state.revision})
}

fn audio(id: &str, uri: String) -> Row {
    json!({"id":id,"audio_uri":uri})
        .as_object()
        .unwrap()
        .clone()
}

pub fn run(parent: &Library, results: &mut Vec<Value>) -> Result<()> {
    let root = parent.root.join("view-data");
    let lib = Library::open(root.join("source"))?;
    let mut evidence = vec![];
    let mut db = lib.db()?;
    let tx = db.transaction()?;
    for (kind, count) in [("speaking", 30), ("ocr", 35)] {
        for index in 0..count {
            tx.execute(
                "INSERT INTO ai_records VALUES(?1,?2,?3,'2020-01-01')",
                rusqlite::params![
                    format!("{kind}-{index}"),
                    kind,
                    json!({"index":index,"kind":kind}).to_string()
                ],
            )?;
        }
        tx.execute(
            "INSERT INTO ai_records VALUES(?1,?2,'{broken','2030-01-01')",
            rusqlite::params![format!("{kind}-broken"), kind],
        )?;
    }
    for n in 0..1000 {
        tx.execute(
            "INSERT INTO ai_records VALUES(?1,'chat','{broken','2040-01-01')",
            [format!("unrelated-chat-{n}")],
        )?;
    }
    tx.commit()?;
    lib.put("words", json!({"id":1,"word":"cache","meaning":"缓存","source":"fixture","created_at":"2020-01-01"}))?;
    lib.put(
        "study_stats",
        json!({"day":"2020-01-02","listen_seconds":20,"review_count":1,"pomodoro_count":0}),
    )?;
    lib.put(
        "study_stats",
        json!({"day":"2020-01-01","listen_seconds":10,"review_count":2,"pomodoro_count":1}),
    )?;
    fs::write(lib.root.join("pdf/z-last.PDF"), b"%PDF-fixture")?;
    fs::write(lib.root.join("pdf/a-first.pdf"), b"%PDF-fixture")?;
    fs::write(lib.root.join("pdf/render-cache.png"), b"fixture")?;
    fs::create_dir_all(lib.root.join("pdf/directory.pdf"))?;
    let local_audio = lib.root.join("media/local.wav");
    let uri_audio = lib.root.join("media/file-uri.wav");
    fs::write(&local_audio, b"audio fixture")?;
    fs::write(&uri_audio, b"audio fixture")?;
    let local = local_audio.to_string_lossy().to_string();
    let file_uri = url::Url::from_file_path(&uri_audio).unwrap().to_string();
    let audios = vec![
        audio("local", local.clone()),
        audio("uri", file_uri.clone()),
        audio(
            "missing",
            lib.root.join("media/missing.wav").to_string_lossy().into(),
        ),
        audio("empty", String::new()),
    ];
    fs::write(
        lib.root.join("models/ggml-tiny.en.bin"),
        b"model presence fixture",
    )?;
    fs::create_dir_all(lib.root.join("models/sense-voice"))?;
    fs::write(
        lib.root.join("models/sense-voice/model.int8.onnx"),
        b"incomplete model fixture",
    )?;
    let mut state = view_data::State::default();
    state.reload(&lib, &audios)?;
    ensure!(
        state.speaking.len() == 20
            && state.speaking[0]["index"] == 29
            && state.speaking[19]["index"] == 10,
        "speaking cap/order or malformed JSON filtering failed"
    );
    ensure!(
        state.ocr.len() == 30 && state.ocr[0]["index"] == 34 && state.ocr[29]["index"] == 5,
        "OCR cap/order or malformed JSON filtering failed"
    );
    ensure!(
        state.stats == lib.list("study_stats")? && state.words == lib.list("words")?,
        "general data ordering changed"
    );
    ensure!(
        state
            .pdf_files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect::<Vec<_>>()
            == ["a-first.pdf", "z-last.PDF"],
        "PDF sort/filter included directory or cache image"
    );
    ensure!(
        state.available_audio.len() == 2
            && state.available_audio.contains(&local)
            && state.available_audio.contains(&file_uri),
        "audio path/URI availability incorrect"
    );
    ensure!(
        state
            .asr_models
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            == ["tiny.en"]
            && !state.tts_ready,
        "incomplete models advertised as ready"
    );
    ensure!(
        state
            .saved_ai
            .as_ref()
            .is_some_and(|c| c.engine == "local" && c.local_model == "base.en")
            && state.revision == 1,
        "default saved AI snapshot/revision incorrect"
    );
    record(
        &root,
        &mut evidence,
        "bounded-records-general-order-and-file-readiness",
        json!({"speaking":20,"ocr":30,"unrelated_bad_json":1000,"pdf_files":state.pdf_files,"audio_count":state.available_audio.len(),"asr_models":state.asr_models,"revision":state.revision}),
    )?;

    let initial = visible(&state);
    lib.put("words", json!({"id":2,"word":"reload","meaning":"刷新","source":"fixture","created_at":"2020-01-01"}))?;
    fs::remove_file(&local_audio)?;
    fs::write(
        lib.root.join("models/sense-voice/tokens.txt"),
        b"fixture tokens",
    )?;
    fs::write(lib.root.join("pdf/new.pdf"), b"%PDF-fixture")?;
    for _ in 0..1000 {
        ensure!(
            visible(&state) == initial,
            "cached presentation performed implicit reload"
        );
    }
    state.reload(&lib, &audios)?;
    ensure!(
        state.revision == 2
            && state.words.len() == 2
            && !state.available_audio.contains(&local)
            && state.asr_models.contains("sense-voice")
            && state.pdf_files.len() == 3,
        "explicit reload did not replace full snapshot"
    );
    record(
        &root,
        &mut evidence,
        "cached-presentation-and-explicit-refresh",
        json!({"presentations_without_io":1000,"revision":state.revision,"word_count":state.words.len(),"audio_count":state.available_audio.len(),"pdf_count":state.pdf_files.len()}),
    )?;

    // These are deliberately synthetic secrets. Cache loading must not resolve them.
    let sentinel = "view-data-fixture-secret-do-not-cache";
    lib.set_setting("ai_config", &json!({"engine":"local","local_model":"tiny.en","base_url":"https://example.invalid","model":"fixture-model","credential_id":"view-fixture","shared_key_source":"chat","api_key":sentinel}).to_string())?;
    db.execute(
        "INSERT INTO local_credentials VALUES('chat-view-fixture',?1)",
        [sentinel],
    )?;
    state.reload(&lib, &audios)?;
    ensure!(
        !state.needs_legacy_import && !visible(&state).to_string().contains(sentinel),
        "cache exposed stored or unknown-field credential"
    );
    db.execute("DELETE FROM local_credentials", [])?;
    state.reload(&lib, &audios)?;
    ensure!(
        state.needs_legacy_import,
        "missing local referenced credential did not show legacy import"
    );
    lib.set_setting("credential_secure_mode", "true")?;
    state.reload(&lib, &audios)?;
    ensure!(
        !state.needs_legacy_import
            && state
                .saved_ai
                .as_ref()
                .is_some_and(|c| c.model == "fixture-model")
            && !visible(&state).to_string().contains(sentinel),
        "secure mode attempted secret resolution or changed config"
    );
    record(
        &root,
        &mut evidence,
        "configuration-and-legacy-status-without-secret-resolution",
        json!({"secret_cached":false,"ordinary_missing_key_detected":true,"secure_config_loaded":true}),
    )?;

    let before = visible(&state);
    db.execute_batch("ALTER TABLE study_stats RENAME TO unavailable_study_stats")?;
    ensure!(
        state.reload(&lib, &audios).is_err()
            && visible(&state) == before
            && !state.error.is_empty(),
        "failed SQLite reload discarded snapshot/revision"
    );
    db.execute_batch("ALTER TABLE unavailable_study_stats RENAME TO study_stats")?;
    let saved = lib.setting("ai_config")?;
    lib.set_setting("ai_config", "{broken")?;
    ensure!(
        state.reload(&lib, &audios).is_err()
            && visible(&state) == before
            && !state.error.is_empty(),
        "invalid saved AI replaced snapshot"
    );
    lib.set_setting("ai_config", &saved)?;
    let pdf_dir = lib.root.join("pdf");
    let unavailable_pdf = lib.root.join("unavailable-pdf");
    fs::rename(&pdf_dir, &unavailable_pdf)?;
    fs::write(&pdf_dir, b"directory failure fixture")?;
    ensure!(
        state.reload(&lib, &audios).is_err() && visible(&state) == before,
        "PDF reload failure discarded partial snapshot"
    );
    fs::remove_file(&pdf_dir)?;
    fs::rename(&unavailable_pdf, &pdf_dir)?;
    state.reload(&lib, &audios)?;
    ensure!(
        state.revision == before["revision"].as_u64().unwrap() + 1 && state.error.is_empty(),
        "repaired reload did not recover cleanly"
    );
    record(
        &root,
        &mut evidence,
        "atomic-last-known-good-on-sql-config-and-directory-failures",
        json!({"failures":3,"snapshot_retained":true,"recovered_revision":state.revision}),
    )?;

    let model = tts::model_dir(&lib);
    for (file, len) in [
        ("model.onnx", 1_000_001),
        ("voices.bin", 1001),
        ("tokens.txt", 11),
        ("lexicon-us-en.txt", 11),
        ("lexicon-zh.txt", 11),
        ("espeak-ng-data/phontab", 11),
    ] {
        let path = model.join(file);
        fs::create_dir_all(path.parent().unwrap())?;
        fs::File::create(&path)?.set_len(len)?;
    }
    state.reload(&lib, &audios)?;
    ensure!(
        state.tts_ready,
        "Kokoro readiness did not use required-file validation"
    );
    let graph = view_data::graph();
    ensure!(
        graph["nodes"].as_array().is_some_and(|n| !n.is_empty()),
        "public graph data empty"
    );
    for _ in 0..1000 {
        ensure!(
            std::ptr::eq(graph, view_data::graph()),
            "graph cache reparsed data"
        );
    }
    record(
        &root,
        &mut evidence,
        "kokoro-required-files-and-once-parsed-public-graph",
        json!({"tts_ready":true,"graph_nodes":graph["nodes"].as_array().unwrap().len(),"same_graph_reference_checks":1000}),
    )?;
    lib.db()?.execute(
        "VACUUM INTO ?1",
        [root.join("view-data.sqlite").to_string_lossy().as_ref()],
    )?;
    results.push(json!({"workflow":"界面记录与文件缓存 → 显式刷新 → SQLite/配置/目录故障保留快照 → 无密钥解析 → 公共词图常驻","passed":true,"cases":evidence.len(),"report":root.join("view-data-evidence.json"),"sqlite":root.join("view-data.sqlite")}));
    Ok(())
}
