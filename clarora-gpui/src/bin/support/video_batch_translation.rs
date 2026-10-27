//! Real HTTP, missing Bilibili media, SQLite caches and competing imports.
use anyhow::{Context, Result, ensure};
use clarora_gpui::{
    bilibili::{Part, Series},
    data::Library,
    process,
    subtitles::Cue,
    video_jobs::{Downloads, Event, Job, Mode, Plan, Report},
    video_subtitles,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, atomic::Ordering},
    thread,
    time::{Duration, Instant},
};
type Hook = Box<dyn FnOnce() -> Result<()> + Send>;
struct Server {
    url: String,
    stop: process::Cancel,
    seen: Arc<Mutex<Vec<Value>>>,
    hook: Arc<Mutex<Option<Hook>>>,
    worker: Option<thread::JoinHandle<Result<()>>>,
}
impl Server {
    fn start() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let url = format!("http://{}", listener.local_addr()?);
        let stop = process::cancel_token();
        let flag = stop.clone();
        let seen = Arc::new(Mutex::new(vec![]));
        let requests = seen.clone();
        let hook: Arc<Mutex<Option<Hook>>> = Arc::new(Mutex::new(None));
        let next = hook.clone();
        let worker = thread::spawn(move || -> Result<()> {
            while !flag.load(Ordering::SeqCst) {
                let (mut socket, _) = match listener.accept() {
                    Ok(v) => v,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(e) => return Err(e.into()),
                };
                socket.set_nonblocking(false)?;
                socket.set_read_timeout(Some(Duration::from_secs(5)))?;
                let mut bytes = vec![];
                let mut buf = [0; 8192];
                let split = loop {
                    let n = socket.read(&mut buf)?;
                    ensure!(n > 0, "HTTP header interrupted");
                    bytes.extend_from_slice(&buf[..n]);
                    if let Some(i) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        break i + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&bytes[..split]).to_string();
                ensure!(
                    !headers.contains("audio/transcriptions"),
                    "batch translation ran ASR"
                );
                let length = headers
                    .lines()
                    .find_map(|l| {
                        l.to_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|v| v.trim().parse::<usize>().ok())
                    })
                    .context("missing HTTP length")?;
                while bytes.len() < split + length {
                    let n = socket.read(&mut buf)?;
                    ensure!(n > 0, "HTTP body interrupted");
                    bytes.extend_from_slice(&buf[..n]);
                }
                let request: Value = serde_json::from_slice(&bytes[split..split + length])?;
                let message = request["messages"][0]["content"]
                    .as_str()
                    .context("missing translation source")?;
                let start = message.find("[{").context("missing subtitle array")?;
                let end = message.rfind(']').context("missing array end")?;
                let cues: Vec<Value> = serde_json::from_str(&message[start..=end])?;
                ensure!(
                    !message.contains("保留中文"),
                    "Chinese P was sent for translation"
                );
                let bad = request["model"] == "bad"
                    || (request["model"] == "bad-second"
                        && message.contains("English sentence 32"));
                let answer = cues.iter().map(|c| json!({"id":if bad {999} else {c["id"].as_u64().unwrap()},"text":"整集中文翻译"})).collect::<Vec<_>>();
                requests
                    .lock()
                    .unwrap()
                    .push(json!({"request":request,"source":cues}));
                if let Some(hook) = next.lock().unwrap().take() {
                    hook()?;
                }
                let body = format!(
                    "data: {}\n\ndata: [DONE]\n\n",
                    json!({"choices":[{"delta":{"content":serde_json::to_string(&answer)?},"finish_reason":"stop"}]})
                );
                let _ = write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
            Ok(())
        });
        Ok(Self {
            url,
            stop,
            seen,
            hook,
            worker: Some(worker),
        })
    }
    fn config(&self, lib: &Library, model: &str) -> Result<()> {
        lib.set_setting(
            "ai_config",
            &json!({"engine":"compatible","base_url":self.url,"model":model}).to_string(),
        )
    }
    fn count(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .expect("HTTP fixture panic")
                .expect("HTTP fixture error");
        }
    }
}
fn series(lib: &Library, count: usize) -> (Series, Vec<PathBuf>) {
    let base = "https://www.bilibili.com/video/BV1xx411c7mD";
    let paths = (0..count)
        .map(|i| {
            lib.root
                .join("downloads/remote-bilibili")
                .join(hex::encode(Sha256::digest(
                    format!("{base}?p={}", i + 1).as_bytes(),
                )))
                .join("video.mp4")
        })
        .collect::<Vec<_>>();
    (
        Series {
            title: "all P translation".into(),
            base: base.into(),
            requested: 0,
            current: None,
            parts: (0..count)
                .map(|i| Part {
                    number: (i + 1) as u32,
                    title: format!("Episode {}", i + 1),
                    duration: 35.,
                    path: None,
                })
                .collect(),
        },
        paths,
    )
}
fn english(count: usize) -> Vec<Cue> {
    (0..count)
        .map(|i| Cue {
            start: i as f64 + 0.125,
            end: i as f64 + 0.875,
            text: format!("English sentence {i}."),
        })
        .collect()
}
fn report(job: &Job) -> Result<(Report, Vec<String>, usize)> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut progress = vec![];
    let mut downloads = 0;
    loop {
        if Instant::now() > deadline {
            job.cancel.store(true, Ordering::SeqCst);
            anyhow::bail!("translation queue timed out");
        }
        match job.events.recv_timeout(Duration::from_millis(100)) {
            Ok(Event::Finished(report)) => return Ok((report, progress, downloads)),
            Ok(Event::Progress(stage)) => progress.push(stage),
            Ok(Event::Downloaded { .. }) => downloads += 1,
            Ok(Event::Ready { .. }) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(error) => return Err(error.into()),
        }
    }
}
fn run_plan(lib: &Library, series: Series) -> Result<(Report, Vec<String>, usize)> {
    report(&Job::start(
        lib.clone(),
        Plan::translate_all("video-source".into(), series),
        Downloads::default(),
    ))
}
fn record(root: &Path, evidence: &mut Vec<Value>, case: &str, details: Value) -> Result<()> {
    evidence.push(json!({"case":case,"passed":true,"details":details}));
    fs::write(
        root.join("video-batch-translation-evidence.json"),
        serde_json::to_vec_pretty(evidence)?,
    )?;
    Ok(())
}
pub fn run(parent: &Library, results: &mut Vec<Value>) -> Result<()> {
    let root = parent.root.join("video-batch-translation");
    let lib = Library::open(root.join("library"))?;
    let server = Server::start()?;
    server.config(&lib, "good")?;
    let mut evidence = vec![];
    let (all, paths) = series(&lib, 5);
    let en = english(35);
    let cancel = process::cancel_token();
    video_subtitles::save(&lib, &paths[0], &en, &cancel)?;
    let mut bilingual = english(2);
    bilingual[0].text.push_str("\n保留中文");
    video_subtitles::save(&lib, &paths[1], &bilingual, &cancel)?;
    video_subtitles::save(
        &lib,
        &paths[3],
        &[Cue {
            start: 0.,
            end: 1.,
            text: "1234".into(),
        }],
        &cancel,
    )?;
    video_subtitles::save(&lib, &paths[4], &english(1), &cancel)?;
    let (first, progress, downloaded) = run_plan(&lib, all.clone())?;
    ensure!(
        first.mode == Mode::Chinese
            && first.total == 5
            && first.translated == 2
            && first.generated == 0
            && first.skipped == 1
            && first.failed.len() == 2
            && !first.cancelled,
        "translation counts incorrect: {}",
        serde_json::to_string(&first)?
    );
    ensure!(
        downloaded == 0 && paths.iter().all(|p| !p.exists()) && server.count() == 3,
        "translation downloaded video or requested skipped P"
    );
    let translated = video_subtitles::load(&lib, &paths[0])?;
    ensure!(
        translated.len() == en.len()
            && translated.iter().zip(&en).all(|(a, b)| a.start == b.start
                && a.end == b.end
                && a.text == format!("{}\n整集中文翻译", b.text)),
        "English/timeline changed"
    );
    ensure!(
        video_subtitles::load(&lib, &paths[1])?[0].text == bilingual[0].text,
        "partially Chinese P was modified"
    );
    let (again, _, _) = run_plan(&lib, all.clone())?;
    ensure!(
        again.translated == 0
            && again.skipped == 3
            && again.failed.len() == 2
            && server.count() == 3,
        "repeat did not skip translated P"
    );
    record(
        &root,
        &mut evidence,
        "all-P-skip-partial-Chinese-no-media-no-ASR-and-retry",
        json!({"first":first,"again":again,"progress":progress,"http_requests":server.count(),"download_events":downloaded,"all_video_files_absent":true}),
    )?;

    let (single, _) = series(&lib, 1);
    video_subtitles::save(&lib, &paths[0], &en, &cancel)?;
    server.config(&lib, "bad-second")?;
    let before = lib.setting(&format!(
        "video_subtitles_source_{}",
        paths[0]
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
    ))?;
    let (bad, _, _) = run_plan(&lib, single.clone())?;
    ensure!(
        bad.failed.len() == 1
            && bad.translated == 0
            && video_subtitles::load(&lib, &paths[0])?[0].text == en[0].text
            && fs::read_to_string(&before)?.contains("English sentence 34"),
        "partial failed translation replaced source"
    );
    record(
        &root,
        &mut evidence,
        "later-AI-batch-failure-preserves-entire-cache",
        json!({"report":bad,"source_path":before}),
    )?;

    server.config(&lib, "good")?;
    let changed_lib = lib.clone();
    let changed_path = paths[0].clone();
    *server.hook.lock().unwrap() = Some(Box::new(move || {
        video_subtitles::save(
            &changed_lib,
            &changed_path,
            &[Cue {
                start: 0.,
                end: 1.,
                text: "Newer English.\n保留中文".into(),
            }],
            &process::cancel_token(),
        )?;
        Ok(())
    }));
    let (stale, _, _) = run_plan(&lib, single.clone())?;
    ensure!(
        stale.failed.len() == 1
            && video_subtitles::load(&lib, &paths[0])?[0].text == "Newer English.\n保留中文",
        "late translation replaced concurrent Chinese import"
    );
    let count = server.count();
    let (skip, _, _) = run_plan(&lib, single.clone())?;
    ensure!(
        skip.skipped == 1 && server.count() == count,
        "new Chinese import not skipped on retry"
    );
    record(
        &root,
        &mut evidence,
        "concurrent-import-and-new-Chinese-not-overwritten",
        json!({"report":stale,"retry":skip}),
    )?;

    let cache = video_subtitles::save(&lib, &paths[0], &english(1), &cancel)?;
    let changed_cache = cache.clone();
    *server.hook.lock().unwrap() = Some(Box::new(move || {
        fs::write(
            changed_cache,
            "1\n00:00:00,125 --> 00:00:00,875\nIn-place new English.\n保留中文\n",
        )?;
        Ok(())
    }));
    let (edited, _, _) = run_plan(&lib, single.clone())?;
    ensure!(
        edited.failed.len() == 1
            && video_subtitles::load(&lib, &paths[0])?[0]
                .text
                .contains("In-place new English"),
        "same-path edit was overwritten"
    );
    record(
        &root,
        &mut evidence,
        "in-place-subtitle-file-edit-not-overwritten",
        json!({"report":edited,"cache":cache}),
    )?;

    video_subtitles::save(&lib, &paths[0], &en, &cancel)?;
    let (tx, rx) = std::sync::mpsc::channel::<process::Cancel>();
    *server.hook.lock().unwrap() = Some(Box::new(move || {
        rx.recv_timeout(Duration::from_secs(2))?
            .store(true, Ordering::SeqCst);
        Ok(())
    }));
    let count = server.count();
    let job = Job::start(
        lib.clone(),
        Plan::translate_all("cancel-source".into(), all),
        Downloads::default(),
    );
    tx.send(job.cancel.clone())?;
    let (cancelled, _, _) = report(&job)?;
    ensure!(
        cancelled.cancelled
            && cancelled.translated == 0
            && server.count() == count + 1
            && video_subtitles::load(&lib, &paths[0])?[0].text == en[0].text,
        "cancel did not preserve source/stop later P"
    );
    record(
        &root,
        &mut evidence,
        "cancel-during-HTTP-preserves-cache-and-stops-later-P",
        json!({"report":cancelled}),
    )?;
    fs::write(
        root.join("http-requests.json"),
        serde_json::to_vec_pretty(&*server.seen.lock().unwrap())?,
    )?;
    lib.db()?.execute(
        "VACUUM INTO ?1",
        [root
            .join("video-batch-translation.sqlite")
            .to_string_lossy()
            .as_ref()],
    )?;
    results.push(json!({"workflow":"全部P翻译中文 → 已有中文整P跳过 → 无视频仅缓存翻译 → 缺英文失败继续 → 取消与并发字幕保护","passed":true,"cases":evidence.len(),"report":root.join("video-batch-translation-evidence.json")}));
    Ok(())
}
