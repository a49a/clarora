use anyhow::{Result, ensure};
use clarora_gpui::{
    bilibili::{Part, Series},
    data::Library,
    media::Player,
    process,
    video_jobs::{Downloads, Event, Job, Plan},
    video_subtitles,
};
use serde_json::{Value, json};
use std::{
    fs,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

pub fn run(lib: &Library, fixture: &std::path::Path, results: &mut Vec<Value>) -> Result<()> {
    let paths =
        ["queue-first.mp4", "queue-broken.mp4", "queue-last.mp4"].map(|name| lib.root.join(name));
    fs::copy(fixture, &paths[0])?;
    fs::write(&paths[1], b"invalid video")?;
    fs::copy(fixture, &paths[2])?;
    let cancel = process::cancel_token();
    let existing = video_subtitles::load(lib, fixture)?;
    video_subtitles::save(lib, &paths[0], &existing, &cancel)?;
    let series = Series {
        title: "后台选集".into(),
        base: "https://www.bilibili.com/video/BV1xx411c7mD".into(),
        requested: 0,
        current: Some(0),
        parts: paths
            .iter()
            .enumerate()
            .map(|(i, p)| Part {
                number: (i + 1) as u32,
                title: format!("Episode {}", i + 1),
                duration: 4.,
                path: Some(p.clone()),
            })
            .collect(),
    };
    let downloads = Downloads::default();
    let job = Job::start(
        lib.clone(),
        Plan::all("source-fixture".into(), series.clone()),
        downloads.clone(),
    );
    let player = Player::without_audio_device();
    player.open(&paths[2]); // Changing playback is independent of the subtitle queue.
    let deadline = Instant::now() + Duration::from_secs(90);
    let mut progress = vec![];
    let report = loop {
        ensure!(Instant::now() < deadline, "后台字幕队列没有结束");
        match job.events.recv_timeout(Duration::from_millis(100)) {
            Ok(Event::Progress(s)) => progress.push(s),
            Ok(Event::Finished(r)) => break r,
            Ok(_) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(e) => return Err(e.into()),
        }
    };
    ensure!(
        report.total == 3
            && report.generated == 1
            && report.skipped == 1
            && report.failed.len() == 1
            && !report.cancelled,
        "跳过/失败继续/生成计数错误"
    );
    ensure!(
        !video_subtitles::load(lib, &paths[2])?.is_empty(),
        "失败后未继续生成下一P"
    );
    ensure!(
        player.snapshot().path == paths[2].to_string_lossy(),
        "后台处理改变前台播放选择"
    );
    // Real shared cache: concurrent requesters must reuse one completed file.
    let mut uncached = series.clone();
    uncached.parts[0].path = None;
    uncached.base = "https://www.bilibili.com/video/av123".into();
    let count = Arc::new(AtomicUsize::new(0));
    let mut workers = vec![];
    for _ in 0..2 {
        let cache = downloads.clone();
        let s = uncached.clone();
        let count = count.clone();
        let path = paths[0].clone();
        workers.push(std::thread::spawn(move || {
            cache.obtain_with(&s, 0, &process::cancel_token(), || {
                count.fetch_add(1, Ordering::SeqCst);
                Ok(path)
            })
        }));
    }
    for w in workers {
        ensure!(w.join().unwrap()? == paths[0], "共享分P下载路径不一致");
    }
    ensure!(count.load(Ordering::SeqCst) == 1, "前后台重复下载同一分P");
    let bookmark_lib = Library::open(lib.root.join("queue-bookmarks"))?;
    let mut collection = clarora_gpui::video_library::Collection::load(&bookmark_lib)?;
    let mut initial = series.clone();
    initial.parts[2].path = None;
    let id = collection
        .add(&[paths[0].clone()], Some(&initial))?
        .remove(0);
    collection.select(&id)?;
    ensure!(
        collection.cache_part(&id, 2, &paths[2])?.is_some(),
        "后台下载路径未保存"
    );
    ensure!(
        collection.active().unwrap().path == paths[0]
            && collection
                .active()
                .unwrap()
                .series
                .as_ref()
                .unwrap()
                .current
                == Some(0),
        "后台缓存更新改变当前P"
    );
    collection.remove(&id)?;
    ensure!(
        collection.cache_part(&id, 2, &paths[2])?.is_none() && collection.items.is_empty(),
        "后台下载复活已移除收藏"
    );
    collection.save(&bookmark_lib)?;
    let second = Job::start(
        lib.clone(),
        Plan::current("source-fixture".into(), paths[2].clone()),
        downloads,
    );
    second.cancel.store(true, Ordering::SeqCst);
    let cancelled = loop {
        match second.events.recv_timeout(Duration::from_secs(15))? {
            Event::Finished(r) => break r,
            _ => {}
        }
    };
    ensure!(
        cancelled.cancelled && !video_subtitles::load(lib, &paths[2])?.is_empty(),
        "取消破坏已生成字幕"
    );
    fs::write(
        lib.root.join("background-subtitles-evidence.json"),
        serde_json::to_vec_pretty(
            &json!({"report":report,"progress":progress,"independent_playback":true,"single_download_for_concurrent_requests":true,"cancelled":cancelled.cancelled}),
        )?,
    )?;
    results.push(json!({"workflow":"后台分P队列：已有字幕跳过 → 损坏P失败继续 → 真实ASR/独立播放 → 共享下载去重/取消保留", "passed":true,"evidence":"background-subtitles-evidence.json"}));
    Ok(())
}
