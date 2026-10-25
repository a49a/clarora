//! Real HTTP/SQLite/file workflows specified before the batch implementation.
use anyhow::{Context, Result, ensure};
use clarora_gpui::{
    audio_batch::{Mode, Plan},
    data::{Library, id, now, text},
    process, subtitles,
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex, atomic::Ordering},
    thread,
    time::Duration,
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
                    ensure!(n > 0, "HTTP头中断");
                    bytes.extend_from_slice(&buf[..n]);
                    if let Some(i) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        break i + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&bytes[..split]).to_string();
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|v| v.trim().parse::<usize>().ok())
                    })
                    .context("缺少HTTP长度")?;
                while bytes.len() < split + length {
                    let n = socket.read(&mut buf)?;
                    ensure!(n > 0, "HTTP体中断");
                    bytes.extend_from_slice(&buf[..n]);
                }
                let asr = headers
                    .lines()
                    .next()
                    .unwrap()
                    .contains("/audio/transcriptions");
                let (body, kind) = if asr {
                    requests.lock().unwrap().push(json!({"kind":"asr"}));
                    (json!({"segments":[{"start":0.,"end":1.,"text":"Generated English first."},{"start":1.,"end":2.,"text":"Generated English second."}]}).to_string(),"application/json")
                } else {
                    let request: Value = serde_json::from_slice(&bytes[split..split + length])?;
                    let message = request["messages"][0]["content"]
                        .as_str()
                        .context("无翻译请求")?;
                    let start = message.find("[{").context("无字幕数组")?;
                    let end = message.rfind(']').context("数组未结束")?;
                    let cues: Vec<Value> = serde_json::from_str(&message[start..=end])?;
                    ensure!(!message.contains("保留译文"), "已有中文字幕被再次发送翻译");
                    let bad = request["model"] == "bad"
                        || (request["model"] == "bad-second" && message.contains("English 33"));
                    let answer=cues.iter().map(|c|json!({"id":if bad{999}else{c["id"].as_u64().unwrap()},"text":"批量中文译文"})).collect::<Vec<_>>();
                    requests
                        .lock()
                        .unwrap()
                        .push(json!({"kind":"translate","count":cues.len(),"source":cues}));
                    (
                        format!(
                            "data: {}\n\ndata: [DONE]\n\n",
                            json!({"choices":[{"delta":{"content":serde_json::to_string(&answer)?},"finish_reason":"stop"}]})
                        ),
                        "text/event-stream",
                    )
                };
                if let Some(hook) = next.lock().unwrap().take() {
                    hook()?;
                }
                let _ = write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
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
        lib.set_setting("ai_config",&json!({"engine":"compatible","base_url":self.url,"asr_base_url":self.url,"asr_model":"asr","model":model}).to_string())
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .expect("HTTP fixture panic")
                .expect("HTTP fixture failed");
        }
    }
}
fn audio(lib: &Library, group: &str, name: &str, body: Option<&str>) -> Result<String> {
    let key = id();
    let wav = lib.root.join(format!("{key}.wav"));
    let mut writer = hound::WavWriter::create(
        &wav,
        hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )?;
    for _ in 0..32000 {
        writer.write_sample(0i16)?;
    }
    writer.finalize()?;
    let subtitle = if let Some(body) = body {
        let path = lib.root.join(format!("{key}.srt"));
        fs::write(&path, body)?;
        path.to_string_lossy().to_string()
    } else {
        String::new()
    };
    lib.put("listening_audios",json!({"id":key,"practice_id":group,"name":name,"audio_uri":wav,"subtitle_uri":subtitle,"created_at":now()}))?;
    Ok(key)
}
fn row(lib: &Library, key: &str) -> Result<clarora_gpui::data::Row> {
    lib.rows(
        "SELECT * FROM listening_audios WHERE id=?",
        &[key.to_string().into()],
    )?
    .pop()
    .context("audio missing")
}
fn body(lib: &Library, key: &str) -> Result<String> {
    Ok(fs::read_to_string(text(&row(lib, key)?, "subtitle_uri"))?)
}
fn scenario(parent: &Library, name: &str, server: &Server) -> Result<(Library, String)> {
    let lib = Library::open(parent.root.join(name))?;
    server.config(&lib, "good")?;
    let group = lib.practice(name)?;
    Ok((lib, group))
}
pub fn run(parent: &Library, results: &mut Vec<Value>) -> Result<()> {
    let server = Server::start()?;
    let (lib, group) = scenario(parent, "group-batch", &server)?;
    let other = lib.practice("other")?;
    let en = "1\n00:00:00,000 --> 00:00:01,000\nEnglish first.\n\n2\n00:00:01,000 --> 00:00:02,000\nEnglish second.\n";
    let bilingual = en
        .replace("English first.", "English first.\n保留译文一")
        .replace("English second.", "English second.\n保留译文二");
    let partial = en.replace("English first.", "English first.\n保留译文一");
    let zh = en
        .replace("English first.", "保留译文一")
        .replace("English second.", "保留译文二");
    let empty = audio(&lib, &group, "empty", None)?;
    let english = audio(&lib, &group, "english", Some(en))?;
    let both = audio(&lib, &group, "bilingual", Some(&bilingual))?;
    let part = audio(&lib, &group, "partial", Some(&partial))?;
    let chinese = audio(&lib, &group, "chinese", Some(&zh))?;
    let broken = audio(&lib, &group, "missing-media", None)?;
    fs::remove_file(text(&row(&lib, &broken)?, "audio_uri"))?;
    let untouched = audio(&lib, &other, "another-group", None)?;
    let plan = Plan::new(&lib, &group, Mode::English)?;
    let later = audio(&lib, &group, "added-after-start", None)?;
    lib.set_setting("library_group", &other)?;
    let cancel = process::cancel_token();
    let mut progress = vec![];
    let first = plan.run(&lib, &cancel, |s| progress.push(s.to_string()));
    ensure!(
        first.total == 6
            && first.english == 2
            && first.skipped == 3
            && first.failed.len() == 1
            && !first.cancelled,
        "英文整组计数/失败继续不符"
    );
    ensure!(
        text(&row(&lib, &later)?, "subtitle_uri").is_empty()
            && text(&row(&lib, &untouched)?, "subtitle_uri").is_empty(),
        "快照/组隔离失败"
    );
    ensure!(
        body(&lib, &english)? == en
            && body(&lib, &both)? == bilingual
            && body(&lib, &part)? == partial,
        "已存在英文被覆盖"
    );
    ensure!(
        body(&lib, &chinese)?.contains("保留译文"),
        "补英文删除了原中文"
    );
    let second = Plan::new(&lib, &group, Mode::Chinese)?.run(&lib, &cancel, |_| {});
    ensure!(
        second.english == 1
            && second.translated == 4
            && second.skipped == 2
            && second.failed.len() == 1,
        "中文补全/跳过/预生成英文计数错误"
    );
    let partial_cues = subtitles::parse(&body(&lib, &part)?);
    ensure!(
        partial_cues[0].text == "English first.\n保留译文一"
            && partial_cues[1].text.contains("批量中文译文"),
        "部分中文被覆盖或未补齐"
    );
    ensure!(body(&lib, &both)? == bilingual, "完整双语字幕被改写");
    let before = server.seen.lock().unwrap().len();
    let repeat = Plan::new(&lib, &group, Mode::Chinese)?.run(&lib, &cancel, |_| {});
    ensure!(
        repeat.skipped == 6
            && repeat.failed.len() == 1
            && server.seen.lock().unwrap().len() == before,
        "重复执行没有跳过已有语言"
    );
    fs::write(
        lib.root.join("group-report.json"),
        serde_json::to_vec_pretty(
            &json!({"english":first,"chinese":second,"repeat":repeat,"progress":progress,"generated":body(&lib,&empty)?}),
        )?,
    )?;
    results.push(json!({"workflow":"真实HTTP/SQLite整组英文与中文：已有语言跳过、部分中文补齐、自动先生成英文、目标组/快照隔离、失败继续及重复运行","passed":true,"evidence":lib.root.join("group-report.json")}));

    let (fail, fail_group) = scenario(parent, "failures", &server)?;
    let long = (0..40)
        .map(|i| {
            format!(
                "{}\n00:00:{:02},000 --> 00:00:{:02},500\nEnglish {i}.\n\n",
                i + 1,
                i,
                i
            )
        })
        .collect::<String>();
    let key = audio(&fail, &fail_group, "long", Some(&long))?;
    server.config(&fail, "bad-second")?;
    let failed = Plan::new(&fail, &fail_group, Mode::Chinese)?.run(&fail, &cancel, |_| {});
    ensure!(
        failed.failed.len() == 1 && body(&fail, &key)? == long,
        "后批次翻译失败覆盖原文"
    );
    let (stale, stale_group) = scenario(parent, "stale", &server)?;
    let key = audio(&stale, &stale_group, "changed-during-asr", None)?;
    let manual = stale.root.join("manual.srt");
    fs::write(&manual, en)?;
    let updated = stale.clone();
    let target = key.clone();
    let expected = manual.to_string_lossy().to_string();
    let saved = expected.clone();
    *server.hook.lock().unwrap() = Some(Box::new(move || {
        updated.db()?.execute(
            "UPDATE listening_audios SET subtitle_uri=? WHERE id=?",
            [saved, target],
        )?;
        Ok(())
    }));
    let failed = Plan::new(&stale, &stale_group, Mode::English)?.run(&stale, &cancel, |_| {});
    ensure!(
        failed.failed.len() == 1
            && text(&row(&stale, &key)?, "subtitle_uri") == expected
            && fs::read_dir(stale.root.join("media"))?.count() == 0,
        "迟到结果覆盖较新字幕/未清理附件"
    );
    let (bootstrap, bootstrap_group) = scenario(parent, "bootstrap-retry", &server)?;
    let key = audio(&bootstrap, &bootstrap_group, "no-english", None)?;
    server.config(&bootstrap, "bad")?;
    let failed =
        Plan::new(&bootstrap, &bootstrap_group, Mode::Chinese)?.run(&bootstrap, &cancel, |_| {});
    ensure!(
        failed.english == 1
            && failed.failed.len() == 1
            && body(&bootstrap, &key)?.contains("Generated English"),
        "翻译失败丢失已完成英文"
    );
    server.config(&bootstrap, "good")?;
    let retried =
        Plan::new(&bootstrap, &bootstrap_group, Mode::Chinese)?.run(&bootstrap, &cancel, |_| {});
    ensure!(
        retried.english == 0 && retried.translated == 1 && retried.failed.is_empty(),
        "重试没有复用已完成英文"
    );
    let (deleted, deleted_group) = scenario(parent, "deleted-during-asr", &server)?;
    let key = audio(&deleted, &deleted_group, "deleted", None)?;
    let removed = deleted.clone();
    *server.hook.lock().unwrap() = Some(Box::new(move || removed.delete("listening_audios", &key)));
    let failed = Plan::new(&deleted, &deleted_group, Mode::English)?.run(&deleted, &cancel, |_| {});
    ensure!(
        failed.failed.len() == 1
            && deleted.list("listening_audios")?.is_empty()
            && fs::read_dir(deleted.root.join("media"))?.count() == 0,
        "迟到ASR复活删除音频或残留新附件"
    );
    results.push(json!({"workflow":"长字幕分批翻译失败保留原文；迟到ASR不覆盖新字幕/复活删除项；翻译失败重试复用英文","passed":true}));

    let (cancel_lib, cancel_group) = scenario(parent, "cancel", &server)?;
    let keys = (0..3)
        .map(|i| audio(&cancel_lib, &cancel_group, &format!("audio-{i}"), None))
        .collect::<Result<Vec<_>>>()?;
    let token = process::cancel_token();
    let signal = token.clone();
    let stopped =
        Plan::new(&cancel_lib, &cancel_group, Mode::English)?.run(&cancel_lib, &token, |s| {
            if s.starts_with("已处理 1/") {
                signal.store(true, Ordering::SeqCst);
            }
        });
    ensure!(
        stopped.cancelled
            && stopped.english == 1
            && keys
                .iter()
                .filter(|key| !text(&row(&cancel_lib, key).unwrap(), "subtitle_uri").is_empty())
                .count()
                == 1,
        "取消丢失已完成或继续处理未完成项"
    );
    let moved = Plan::new(&cancel_lib, &cancel_group, Mode::English)?;
    let destination = cancel_lib.practice("moved")?;
    for key in &keys {
        cancel_lib.db()?.execute(
            "UPDATE listening_audios SET practice_id=? WHERE id=?",
            [&destination, key],
        )?;
    }
    let stopped = moved.run(&cancel_lib, &process::cancel_token(), |_| {});
    ensure!(
        stopped.failed.len() == 3 && stopped.english == 0,
        "被移动目标仍写入或复活"
    );
    let (inflight, inflight_group) = scenario(parent, "cancel-during-asr", &server)?;
    let key = audio(&inflight, &inflight_group, "cancel-in-flight", None)?;
    let token = process::cancel_token();
    let signal = token.clone();
    *server.hook.lock().unwrap() = Some(Box::new(move || {
        signal.store(true, Ordering::SeqCst);
        Ok(())
    }));
    let stopped =
        Plan::new(&inflight, &inflight_group, Mode::English)?.run(&inflight, &token, |_| {});
    ensure!(
        stopped.cancelled
            && stopped.english == 0
            && text(&row(&inflight, &key)?, "subtitle_uri").is_empty()
            && fs::read_dir(inflight.root.join("media"))?.count() == 0,
        "进行中取消仍保存字幕"
    );
    let (invalid, invalid_group) = scenario(parent, "invalid-subtitles", &server)?;
    let malformed = audio(&invalid, &invalid_group, "malformed", Some("not timed SRT"))?;
    let missing = audio(&invalid, &invalid_group, "missing", Some(en))?;
    fs::remove_file(text(&row(&invalid, &missing)?, "subtitle_uri"))?;
    let repaired =
        Plan::new(&invalid, &invalid_group, Mode::English)?.run(&invalid, &cancel, |_| {});
    ensure!(
        repaired.english == 2
            && repaired.skipped == 0
            && repaired.failed.is_empty()
            && body(&invalid, &malformed)?.contains("Generated English"),
        "无效/丢失字幕被误跳过"
    );
    fs::write(
        parent.root.join("audio-batch-http.json"),
        serde_json::to_vec_pretty(&*server.seen.lock().unwrap())?,
    )?;
    results.push(json!({"workflow":"整组取消保留完成项、进行中取消不保存；移动目标拒绝写入；无效/缺失字幕重新生成","passed":true,"evidence":parent.root.join("audio-batch-http.json")}));
    Ok(())
}
