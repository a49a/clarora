//! Failure matrix is recorded before production changes; real library and native playback E2E.
use anyhow::{Result, ensure};
use clarora_gpui::{
    bilibili,
    data::{Library, id},
    media::Player,
    process,
    sentence_search::{self, Audition, MatchMode, Query, Scope},
    video_library, video_subtitles,
};
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    sync::atomic::Ordering,
    thread,
    time::{Duration, Instant},
};

pub fn run(root: &Path, results: &mut Vec<Value>) -> Result<()> {
    let lib = Library::open(root.join(format!("sentence-search-{}", id())))?;
    let cancel = process::cancel_token();
    let mut checks = vec![];
    let group_a = lib.practice("A 练习组")?;
    let group_b = lib.practice("B 练习组")?;
    let mut media = vec![];
    for i in 0..4 {
        if i < 2 {
            let path = lib.root.join("media").join(format!("source-{i}.wav"));
            let mut wav = hound::WavWriter::create(
                &path,
                hound::WavSpec {
                    channels: 1,
                    sample_rate: 16000,
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                },
            )?;
            for n in 0..32000 {
                wav.write_sample(((n as f32 / 12.).sin() * 1500.) as i16)?;
            }
            wav.finalize()?;
            media.push(path);
            continue;
        }
        let path = lib.root.join("media").join(format!("source-{i}.mp4"));
        process::run(
            &process::tool("ffmpeg")?,
            &[
                "-nostdin".into(),
                "-v".into(),
                "error".into(),
                "-y".into(),
                "-f".into(),
                "lavfi".into(),
                "-i".into(),
                "color=c=green:s=160x90:r=30".into(),
                "-f".into(),
                "lavfi".into(),
                "-i".into(),
                "sine=frequency=440:sample_rate=16000".into(),
                "-t".into(),
                "2".into(),
                "-c:v".into(),
                "libx264".into(),
                "-preset".into(),
                "ultrafast".into(),
                "-c:a".into(),
                "aac".into(),
                path.to_string_lossy().into_owned(),
            ],
            &cancel,
            Duration::from_secs(30),
        )?;
        media.push(path);
    }
    let srt = "1\n00:00:00,200 --> 00:00:00,650\n\"When is the train coming?\"\n火车什么时候到？\n\n2\n00:00:00,800 --> 00:00:01,250\nWhenever you like.\n\n3\n00:00:01,400 --> 00:00:01,850\nWhy did you leave?\n";
    for (i, group) in [&group_a, &group_b].iter().enumerate() {
        let sub = lib.root.join("media").join(format!("source-{i}.srt"));
        fs::write(&sub, srt)?;
        lib.put("listening_audios",json!({"id":format!("audio-{i}"),"practice_id":group,"name":format!("Audio {i}"),"audio_uri":media[i],"subtitle_uri":sub}))?;
    }
    let series = bilibili::Series {
        title: "Video collection".into(),
        base: "https://www.bilibili.com/video/BV1xx411c7mD".into(),
        parts: vec![
            bilibili::Part {
                number: 1,
                title: "P1".into(),
                duration: 2.,
                path: Some(media[2].clone()),
            },
            bilibili::Part {
                number: 2,
                title: "P2".into(),
                duration: 2.,
                path: Some(media[3].clone()),
            },
        ],
        requested: 0,
        current: Some(0),
    };
    let mut collection = video_library::Collection::default();
    collection.add(&[media[2].clone(), media[3].clone()], Some(&series))?;
    collection.save(&lib)?;
    for path in &media[2..] {
        video_subtitles::save(&lib, path, &clarora_gpui::subtitles::parse(srt), &cancel)?;
    }
    let query = |phrase: &str, mode, scope| Query {
        phrase: phrase.into(),
        mode,
        scope,
    };
    let all = sentence_search::search(
        &lib,
        &query("  wHeN   IS  ", MatchMode::Prefix, Scope::All),
        &cancel,
    )?;
    let mut worker = sentence_search::Controller::default();
    worker.request(&lib, query("Why", MatchMode::Prefix, Scope::All));
    worker.request(&lib, query("When", MatchMode::Prefix, Scope::Audio));
    worker.request(&lib, query("When is", MatchMode::Prefix, Scope::All));
    let began = Instant::now();
    loop {
        if let Some(result) = worker.poll() {
            let found = result.map_err(anyhow::Error::msg)?;
            ensure!(
                found.hits.len() == 4 && found.hits.iter().all(|h| h.english.contains("When is")),
                "coalesced worker published stale query"
            );
            break;
        }
        ensure!(
            began.elapsed() < Duration::from_secs(8),
            "search worker timeout"
        );
        thread::sleep(Duration::from_millis(10));
    }
    worker.request(&lib, query("When", MatchMode::Prefix, Scope::All));
    worker.cancel();
    ensure!(
        !worker.is_loading() && worker.poll().is_none(),
        "cancelled worker publishes result"
    );
    checks.push(json!({"case":"single-worker coalescing and cancellation","passed":true}));
    ensure!(
        all.hits.len() == 4,
        "cross-library/P search missed sources: {}",
        all.hits.len()
    );
    ensure!(
        all.hits.iter().all(|h| h.english.contains("When is")
            && !h.english.contains('火')
            && h.cue.start == 0.2),
        "language/time mapping"
    );
    checks.push(json!({"case":"case/whitespace/quotes/bilingual/cross-group/all-P","passed":true}));
    ensure!(
        sentence_search::search(&lib, &query("When", MatchMode::Prefix, Scope::All), &cancel)?
            .hits
            .len()
            == 4,
        "When must not match Whenever"
    );
    ensure!(
        sentence_search::search(
            &lib,
            &query("is the", MatchMode::Prefix, Scope::All),
            &cancel
        )?
        .hits
        .is_empty(),
        "prefix became contains"
    );
    ensure!(
        sentence_search::search(
            &lib,
            &query("is the", MatchMode::Contains, Scope::All),
            &cancel
        )?
        .hits
        .len()
            == 4,
        "contains phrase"
    );
    ensure!(
        sentence_search::search(
            &lib,
            &query("Why did", MatchMode::Prefix, Scope::Audio),
            &cancel
        )?
        .hits
        .len()
            == 2,
        "audio scope"
    );
    ensure!(
        sentence_search::search(
            &lib,
            &query("Why did", MatchMode::Prefix, Scope::Video),
            &cancel
        )?
        .hits
        .len()
            == 2,
        "video scope"
    );
    checks.push(json!({"case":"whole words/prefix/contains/audio-video scopes","passed":true}));
    ensure!(
        sentence_search::search(&lib, &query("   ", MatchMode::Prefix, Scope::All), &cancel)
            .is_err(),
        "empty query must not scan all"
    );
    cancel.store(true, Ordering::SeqCst);
    ensure!(
        sentence_search::search(&lib, &query("When", MatchMode::Prefix, Scope::All), &cancel)
            .is_err(),
        "cancel"
    );
    cancel.store(false, Ordering::SeqCst);
    let sub = lib.root.join("media/source-0.srt");
    fs::write(&sub, srt.replace("When is", "Where is"))?;
    ensure!(
        sentence_search::search(&lib, &query("When", MatchMode::Prefix, Scope::All), &cancel)?
            .hits
            .len()
            == 3,
        "stale subtitle cache"
    );
    fs::write(&sub, srt)?;
    checks.push(json!({"case":"empty/cancel/subtitle replacement","passed":true}));
    lib.set_setting("audio_repeat_mode", "6")?;
    lib.set_setting("last_listening_position", "{\"position\":1}")?;
    let mut queue = Audition::new(Player::without_audio_device());
    queue.select(&all.hits, 0)?;
    let started = Instant::now();
    let mut trace = vec![];
    let mut last = 0;
    while !queue.finished {
        queue.poll(&all.hits)?;
        if queue.completed != last {
            last = queue.completed;
            trace.push(
                json!({"completed":last,"last_index":queue.last_completed,"next":queue.index}),
            );
        }
        ensure!(
            started.elapsed() < Duration::from_secs(20),
            "playlist timeout index={:?} error={}",
            queue.index,
            queue.error
        );
        thread::sleep(Duration::from_millis(15));
    }
    ensure!(
        queue.completed == 4 && queue.last_completed == Some(3),
        "playlist skipped/repeated result"
    );
    ensure!(queue.snapshot().paused, "last result must pause");
    checks.push(json!({"case":"real native cross-file queue plays four exact ranges then pauses","passed":true}));
    queue.autoplay = false;
    queue.repeats = 3;
    queue.select(&all.hits, 1)?;
    let started = Instant::now();
    while !queue.finished {
        queue.poll(&all.hits)?;
        ensure!(
            started.elapsed() < Duration::from_secs(8),
            "three plays timed out"
        );
        thread::sleep(Duration::from_millis(15));
    }
    ensure!(
        queue
            .snapshot()
            .sentence_repeat
            .as_ref()
            .is_some_and(|s| s.current_play == 3 && s.finished),
        "three count"
    );
    queue.select(&all.hits, 0)?;
    queue.select(&all.hits, 3)?;
    queue.select(&all.hits, 1)?;
    let started = Instant::now();
    while queue.is_loading() || queue.snapshot().sentence_repeat.is_none() {
        queue.poll(&all.hits)?;
        ensure!(
            started.elapsed() < Duration::from_secs(8),
            "rapid selection"
        );
        thread::sleep(Duration::from_millis(15));
    }
    ensure!(
        queue.index == Some(1) && queue.snapshot().path == all.hits[1].path.to_string_lossy(),
        "rapid source selection used stale request"
    );
    queue.stop();
    ensure!(queue.index.is_none(), "stop leaves active queue");
    ensure!(
        lib.setting("audio_repeat_mode")? == "6"
            && lib.setting("last_listening_position")? == "{\"position\":1}",
        "audition changed normal learning state"
    );
    checks.push(json!({"case":"three plays/rapid selection/stop/modes and progress isolated","passed":true}));
    let broken = lib.root.join("media/broken.wav");
    fs::write(&broken, b"not an audio file")?;
    let mut bad = all.hits[0].clone();
    bad.path = broken;
    let recovery = vec![bad, all.hits[0].clone()];
    queue.repeats = 1;
    queue.select(&recovery, 0)?;
    let started = Instant::now();
    loop {
        if queue.poll(&recovery).is_err() {
            break;
        }
        ensure!(
            started.elapsed() < Duration::from_secs(15),
            "bad decoder was not reported"
        );
        thread::sleep(Duration::from_millis(15));
    }
    queue.select(&recovery, 1)?;
    while !queue.finished {
        queue.poll(&recovery)?;
        ensure!(
            started.elapsed() < Duration::from_secs(20),
            "could not recover from decoder error"
        );
        thread::sleep(Duration::from_millis(15));
    }
    queue.stop();
    checks.push(
        json!({"case":"failed source pauses and selecting a valid source recovers","passed":true}),
    );
    let missing = lib.root.join("media/missing.wav");
    lib.put("listening_audios",json!({"id":"missing","practice_id":group_a,"name":"Missing","audio_uri":missing,"subtitle_uri":sub}))?;
    let report =
        sentence_search::search(&lib, &query("When", MatchMode::Prefix, Scope::All), &cancel)?;
    ensure!(
        report.hits.len() == 4 && report.unavailable >= 1,
        "missing source poisons whole search"
    );
    checks.push(
        json!({"case":"missing media is reported without losing valid results","passed":true}),
    );
    fs::write(&sub, [0xff, 0xfe])?;
    let damaged =
        sentence_search::search(&lib, &query("When", MatchMode::Prefix, Scope::All), &cancel)?;
    ensure!(
        damaged.hits.len() == 3 && !damaged.problems.is_empty(),
        "bad subtitle poisons whole search"
    );
    let mut many = String::new();
    for i in 0..2005 {
        many.push_str(&format!(
            "{i}\n00:00:00,200 --> 00:00:00,650\nWhen is the train coming?\n\n"
        ));
    }
    fs::write(&sub, many)?;
    let capped =
        sentence_search::search(&lib, &query("When", MatchMode::Prefix, Scope::All), &cancel)?;
    ensure!(
        capped.hits.len() == sentence_search::MAX_RESULTS && capped.total == 2008,
        "bounded results lose total count"
    );
    fs::write(&sub, srt)?;
    lib.put("listening_audios",json!({"id":"alias","practice_id":group_b,"name":"Alias","audio_uri":media[0],"subtitle_uri":sub}))?;
    ensure!(
        sentence_search::search(&lib, &query("When", MatchMode::Prefix, Scope::All), &cancel)?
            .hits
            .len()
            == 4,
        "duplicate registered path"
    );
    checks.push(json!({"case":"bad subtitle isolation/result bound and total/duplicate paths","passed":true}));
    fs::write(
        &sub,
        format!(
            "{srt}\n99\n00:00:00,200 --> 00:00:00,650\nWhen {}\n",
            "x".repeat(20000)
        ),
    )?;
    let long =
        sentence_search::search(&lib, &query("When", MatchMode::Prefix, Scope::All), &cancel)?;
    ensure!(
        long.hits.len() == 4 && !long.problems.is_empty(),
        "long caption is not bounded"
    );
    fs::File::create(&sub)?.set_len(16 * 1024 * 1024 + 1)?;
    let huge =
        sentence_search::search(&lib, &query("When", MatchMode::Prefix, Scope::All), &cancel)?;
    ensure!(
        huge.hits.len() == 3 && !huge.problems.is_empty(),
        "oversized subtitle is not bounded"
    );
    fs::write(&sub, srt)?;
    checks.push(json!({"case":"per-file and per-caption memory bounds","passed":true}));
    let evidence = lib.root.join("sentence-search-evidence.json");
    fs::write(
        &evidence,
        serde_json::to_vec_pretty(
            &json!({"passed":true,"checks":checks,"hits":all,"playback_trace":trace}),
        )?,
    )?;
    results.push(json!({"workflow":"跨音视频全库句型检索 → 精确范围逐条试听 → 三遍/快速切换/状态隔离","passed":true,"evidence":evidence}));
    Ok(())
}
