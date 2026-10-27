//! Actual SQLite/files and the same background controller used by the desktop UI.
use anyhow::{Context, Result, ensure};
use clarora_gpui::{
    data::Library,
    library_controller::{Controller, Key, Snapshot, load},
};
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

fn wait(controller: &mut Controller, key: &Key) -> Result<Snapshot> {
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        if let Some(result) = controller.poll(key) {
            return result.map_err(anyhow::Error::msg);
        }
        ensure!(
            Instant::now() < deadline,
            "background data request timed out"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

pub fn run(root: &Path, results: &mut Vec<Value>) -> Result<()> {
    let root = root.join("data-loader");
    fs::create_dir_all(&root)?;
    let lib = Library::open(root.join("source"))?;
    lib.put("listening_practices", json!({"id":"a","name":"A"}))?;
    lib.put("listening_practices", json!({"id":"b","name":"B"}))?;
    let subtitle = lib.root.join("media/a.srt");
    fs::write(
        &subtitle,
        "1\n00:00:00,000 --> 00:00:10,000\nFirst sentence.\n第一句。\n\n2\n00:00:10,000 --> 00:00:22,000\nSecond sentence.\n第二句。\n",
    )?;
    for (id, group, subtitle_uri) in [
        ("a1", "a", subtitle.to_string_lossy().into_owned()),
        ("b1", "b", String::new()),
    ] {
        lib.put("listening_audios", json!({"id":id,"practice_id":group,"name":id,"audio_uri":"","subtitle_uri":subtitle_uri}))?;
    }
    lib.put(
        "sentence_cards",
        json!({"id":"sentence","text":"Front","translation":"Back","notes":""}),
    )?;
    lib.put("clip_cards", json!({"id":"clip","en_text":"Saved clip","zh_text":"片段","audio_uri":"","start_ms":0,"end_ms":2000}))?;
    lib.put("discover_favorites", json!({"key":"clip:clip","payload":json!({"front":"Saved clip","back":"片段","audio":{"uri":"","startMs":0,"endMs":2000}}).to_string()}))?;
    let mut cases = vec![];
    let key = Key {
        page: "listening".into(),
        practice_id: "a".into(),
        selected_audio_id: "a1".into(),
        selected_subtitle_uri: subtitle.to_string_lossy().into_owned(),
        library_selected: "a1".into(),
        card_filter: "all".into(),
        ..Default::default()
    };
    let snapshot = load(&lib, &key, false)?;
    ensure!(
        snapshot.cards.is_none() && snapshot.audios.len() == 2 && snapshot.practices.len() == 2,
        "non-card page scanned cards or lost cross-group audios"
    );
    ensure!(
        snapshot.cues.as_ref().is_some_and(|c| c.len() == 2)
            && snapshot.audio_library.preview.len() == 2
            && snapshot.audio_library.subtitle_ids.contains("a1"),
        "selected subtitles/preview were not prepared in background snapshot"
    );
    cases.push(json!({"case":"跨组音频全集/当前字幕与管理预览/非卡片页按需加载","passed":true}));

    let discover = Key {
        page: "discover".into(),
        ..key.clone()
    };
    let all = load(&lib, &discover, false)?;
    let cards = all.cards.context("missing discover cards")?;
    ensure!(
        cards.iter().any(|c| c.id == "segment:a1:0"
            && c.start == 0.
            && c.end == 22.
            && c.front.contains("Second"))
            && !cards.iter().any(|c| c.kind == "sentence"),
        "discover segmentation changed"
    );
    ensure!(
        all.discover_favorites
            .as_ref()
            .is_some_and(|s| s.contains("clip:clip"))
            && all.discover_due.is_some(),
        "discover next-card cache missing"
    );
    let saved = load(
        &lib,
        &Key {
            saved_only: true,
            ..discover.clone()
        },
        false,
    )?;
    ensure!(
        saved
            .cards
            .as_ref()
            .is_some_and(|c| c.len() == 1 && c[0].id == "clip"),
        "saved-only filter changed"
    );
    let flashcards = load(
        &lib,
        &Key {
            page: "flashcard".into(),
            card_filter: "sentence".into(),
            due: true,
            ..key.clone()
        },
        false,
    )?;
    ensure!(
        flashcards
            .cards
            .as_ref()
            .is_some_and(|c| c.len() == 1 && c[0].id == "sentence"),
        "kind/due card queue changed"
    );
    cases.push(json!({"case":"发现片段合句/收藏与到期缓存/闪卡类型过滤","passed":true}));

    let fallback = load(
        &lib,
        &Key {
            practice_id: "deleted".into(),
            library_selected: "deleted".into(),
            ..key.clone()
        },
        false,
    )?;
    ensure!(
        fallback.practice_id == "a" && fallback.audio_library.selected == "a1",
        "deleted group/selection fallback failed"
    );
    let missing = load(
        &lib,
        &Key {
            selected_audio_id: "b1".into(),
            selected_subtitle_uri: String::new(),
            ..key.clone()
        },
        false,
    )?;
    ensure!(
        missing.cues.as_ref().is_some_and(Vec::is_empty),
        "empty subtitle did not clear previous audio cues"
    );
    cases.push(json!({"case":"练习组删除fallback/无字幕音频清空旧字幕","passed":true}));

    lib.set_setting("last_listening_audio", "a1")?;
    lib.set_setting(
        "last_listening_position",
        &json!({"id":"a1","position":17.25}).to_string(),
    )?;
    lib.set_setting(
        "key_points_a1",
        &json!([{"text":"First", "translation":"第一"}]).to_string(),
    )?;
    let video = lib.root.join("media/source.mp4");
    fs::write(&video, b"file identity fixture; no decoding in data loader")?;
    clarora_gpui::video_subtitles::save(
        &lib,
        &video,
        snapshot.cues.as_ref().unwrap(),
        &clarora_gpui::process::cancel_token(),
    )?;
    let video_key = Key {
        video_path: video.to_string_lossy().into_owned(),
        ..key.clone()
    };
    let media = load(&lib, &video_key, false)?;
    ensure!(
        media.restore_audio == Some(("a1".into(), 17.25))
            && media
                .audio_key_points
                .as_ref()
                .is_some_and(|p| p.len() == 1 && p[0].text == "First")
            && media.video_cues.as_ref().is_some_and(|c| c.len() == 2),
        "restore/keypoint/video subtitle snapshot lost source identity"
    );
    lib.set_setting(
        "last_listening_position",
        &json!({"id":"b1","position":17.25}).to_string(),
    )?;
    ensure!(
        load(&lib, &key, false)?.restore_audio == Some(("a1".into(), 0.)),
        "another audio's resume position leaked"
    );
    fs::remove_file(&video)?;
    ensure!(
        load(&lib, &video_key, false)?
            .video_cues
            .as_ref()
            .is_some_and(Vec::is_empty),
        "removed video prevented catalog recovery"
    );
    cases.push(
        json!({"case":"后台音频位置/重点与视频字幕读取/材料身份/缺失视频清空","passed":true}),
    );

    // The source slot is intentionally absent: link-only Bili restores have subtitles
    // without a local video. Keep the same stable SHA256 identity as source_path().
    use sha2::{Digest, Sha256};
    let source_url = "https://www.bilibili.com/video/BV1xx411c7mD?p=1";
    let source_slot = lib
        .root
        .join("downloads/remote-bilibili")
        .join(hex::encode(Sha256::digest(source_url.as_bytes())))
        .join("video.mp4");
    ensure!(
        !source_slot.exists(),
        "Bili source fixture unexpectedly has video bytes"
    );
    clarora_gpui::video_subtitles::save(
        &lib,
        &source_slot,
        snapshot.cues.as_ref().unwrap(),
        &clarora_gpui::process::cancel_token(),
    )?;
    let source_snapshot = load(
        &lib,
        &Key {
            video_path: source_slot.to_string_lossy().into_owned(),
            ..key.clone()
        },
        false,
    )?;
    ensure!(
        source_snapshot
            .video_cues
            .as_ref()
            .is_some_and(|c| c.len() == 2 && c[0].text.contains("First")),
        "missing restored Bili video discarded its valid source-bound subtitle cache"
    );
    cases.push(json!({"case":"Bili仅链接来源槽位无视频文件/仍读取已保存字幕缓存","passed":true,"source_slot":source_slot}));

    let mut controller = Controller::default();
    let blocker = lib.db()?;
    blocker.execute_batch("PRAGMA journal_mode=DELETE; BEGIN EXCLUSIVE;")?;
    let requested = Instant::now();
    for _ in 0..200 {
        controller.request(&lib, key.clone(), false);
    }
    let caller_ms = requested.elapsed().as_millis();
    ensure!(
        requested.elapsed() < Duration::from_millis(250) && controller.is_loading(),
        "refresh waited for SQLite on caller thread"
    );
    ensure!(
        controller.poll(&key).is_none(),
        "blocked database yielded a partial snapshot"
    );
    let latest = Key {
        page: "flashcard".into(),
        card_filter: "clip".into(),
        ..key.clone()
    };
    controller.request(&lib, latest.clone(), false);
    blocker.execute_batch("COMMIT; PRAGMA journal_mode=WAL;")?;
    drop(blocker);
    let current = wait(&mut controller, &latest)?;
    ensure!(
        current.key == latest
            && current
                .cards
                .as_ref()
                .is_some_and(|c| c.len() == 1 && c[0].kind == "clip")
            && !controller.is_loading(),
        "stale generation replaced current page/filter"
    );
    cases.push(json!({"case":"真实SQLite锁等待在后台/201次请求合并/仅最新代次接受","passed":true,"requests":201,"caller_ms":caller_ms}));

    for (label, changed) in [
        (
            "page",
            Key {
                page: "stats".into(),
                ..key.clone()
            },
        ),
        (
            "group",
            Key {
                practice_id: "b".into(),
                ..key.clone()
            },
        ),
        (
            "filter",
            Key {
                card_filter: "clip".into(),
                ..key.clone()
            },
        ),
        (
            "saved",
            Key {
                saved_only: true,
                ..key.clone()
            },
        ),
        (
            "audio",
            Key {
                selected_audio_id: "b1".into(),
                ..key.clone()
            },
        ),
        (
            "subtitle",
            Key {
                selected_subtitle_uri: "changed.srt".into(),
                ..key.clone()
            },
        ),
        (
            "video",
            Key {
                video_path: "missing-video.mp4".into(),
                ..key.clone()
            },
        ),
        (
            "management_selection",
            Key {
                library_selected: "b1".into(),
                ..key.clone()
            },
        ),
    ] {
        controller.request(&lib, key.clone(), false);
        ensure!(
            controller.poll(&changed).is_none(),
            "accepted stale {label} identity"
        );
        controller.request(&lib, changed.clone(), false);
        ensure!(
            wait(&mut controller, &changed)?.key == changed,
            "fresh {label} failed"
        );
    }
    controller.request(&lib, key.clone(), false);
    controller.invalidate();
    ensure!(
        !controller.is_loading() && controller.poll(&key).is_none(),
        "invalidate did not drop outstanding snapshot"
    );
    cases.push(json!({"case":"页面/组/筛选/收藏/音频/字幕/管理选择身份隔离与失效","passed":true,"identities":8}));

    let before = load(&lib, &key, false)?;
    lib.db()?
        .execute_batch("ALTER TABLE study_stats RENAME TO missing_study_stats")?;
    controller.request(&lib, key.clone(), false);
    let fault_deadline = Instant::now() + Duration::from_secs(12);
    let error = loop {
        ensure!(
            Instant::now() < fault_deadline,
            "database failure was not surfaced"
        );
        if let Some(result) = controller.poll(&key) {
            break result
                .err()
                .context("SQLite failure accepted as snapshot")?;
        }
        thread::sleep(Duration::from_millis(5));
    };
    ensure!(
        !error.is_empty() && before.audios.len() == 2 && before.data.revision == 1,
        "failure changed retained snapshot"
    );
    lib.db()?
        .execute_batch("ALTER TABLE missing_study_stats RENAME TO study_stats")?;
    lib.set_setting("ai_config", "{broken")?;
    ensure!(
        load(&lib, &key, false).is_err(),
        "broken config returned a partial snapshot"
    );
    lib.set_setting("ai_config", "")?;
    let pdf = lib.root.join("pdf");
    fs::rename(&pdf, lib.root.join("pdf-away"))?;
    fs::write(&pdf, b"not a directory")?;
    ensure!(
        load(&lib, &key, false).is_err(),
        "broken directory returned a partial snapshot"
    );
    fs::remove_file(&pdf)?;
    fs::rename(lib.root.join("pdf-away"), &pdf)?;
    fs::write(&subtitle, [0xff, 0xfe, 0xff])?;
    ensure!(
        load(&lib, &key, false).is_err(),
        "unreadable selected subtitles returned partial snapshot"
    );
    fs::write(&subtitle, "1\n00:00:00,000 --> 00:00:03,000\nRecovered.\n")?;
    controller.request(&lib, key.clone(), true);
    ensure!(
        wait(&mut controller, &key)?.runtime.is_some(),
        "repaired request/runtime inventory failed"
    );
    cases.push(json!({"case":"SQLite/配置/目录/字幕故障原子失败与修复重试/运行环境后台读取","passed":true,"database_error":error}));
    let report = root.join("data-loader-evidence.json");
    fs::write(
        &report,
        serde_json::to_vec_pretty(&json!({"passed":true,"cases":cases}))?,
    )?;
    lib.db()?.execute(
        "VACUUM INTO ?1",
        [root.join("data-loader.sqlite").to_string_lossy().as_ref()],
    )?;
    results.push(json!({"workflow":"后台资料快照 → SQLite锁等待/请求合并 → 代次与材料隔离 → 故障保留 → 按页卡片/发现片段","passed":true,"cases":cases.len(),"report":report}));
    Ok(())
}
