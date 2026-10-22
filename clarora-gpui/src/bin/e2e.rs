//! End-to-end library workflows using real SQLite, ZIP, filesystem, processes and HTTP.
#[path = "support/ai_completion.rs"]
mod ai_completion;
#[path = "support/asr_pauses.rs"]
mod asr_pauses;
#[path = "support/audio_library.rs"]
mod audio_library;
#[path = "support/backup_cancel.rs"]
mod backup_cancel;
#[path = "support/bilibili_parts.rs"]
mod bilibili_parts;
#[path = "support/blind_listening.rs"]
mod blind_listening;
#[path = "support/credential_mode.rs"]
mod credential_mode;
#[path = "support/kokoro.rs"]
mod kokoro;
#[path = "support/model_progress.rs"]
mod model_progress;
#[path = "support/protocol.rs"]
mod protocol;
#[path = "support/review_queue.rs"]
mod review_queue;
#[path = "support/video_jobs.rs"]
mod video_jobs;
#[path = "support/video_library.rs"]
mod video_library;
#[path = "support/video_qa.rs"]
mod video_qa;
#[path = "support/video_repeat.rs"]
mod video_repeat;
#[path = "support/video_translation.rs"]
mod video_translation;
#[path = "support/writing.rs"]
mod writing;
use anyhow::{Context, Result, ensure};
use clarora_gpui::{
    ai, audio_tools, backup,
    data::{self, Library, text},
    documents,
    media::Player,
    process, subtitles, waveform,
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
fn report_pdf(path: &Path) -> Result<()> {
    let bodies = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>",
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        "<< /Length 43 >>\nstream\nBT /F1 18 Tf 30 100 Td (Clarora GPUI) Tj ET\nendstream",
    ];
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![0];
    for (i, body) in bodies.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, body).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes());
    for offset in &offsets[1..] {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF",
            offsets.len()
        )
        .as_bytes(),
    );
    fs::write(path, pdf)?;
    Ok(())
}
fn ensure_fixture_authorization(header: &str) {
    assert!(
        header
            .lines()
            .any(|line| line.eq_ignore_ascii_case("authorization: Bearer sqlite-http-fixture")),
        "SQLite fixture credential was not used for the remote request"
    );
}
fn mock_ai() -> Result<(String, Arc<AtomicBool>, thread::JoinHandle<Vec<String>>)> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let address = format!("http://{}", listener.local_addr()?);
    listener.set_nonblocking(true)?;
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let handle = thread::spawn(move || {
        let mut seen = vec![];
        let deadline = Instant::now() + Duration::from_secs(30);
        while !flag.load(Ordering::SeqCst) && Instant::now() < deadline {
            let (mut stream, _) = match listener.accept() {
                Ok(s) => s,
                Err(_) => {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
            };
            // Accepted sockets can inherit nonblocking mode on macOS.
            // Header/body arrival is asynchronous; WouldBlock is not EOF.
            let _ = stream.set_nonblocking(false);
            let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
            let mut bytes = vec![];
            let mut chunk = [0; 8192];
            let split = loop {
                let n = stream.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break None;
                }
                bytes.extend_from_slice(&chunk[..n]);
                if let Some(i) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                    break Some(i + 4);
                }
            };
            let Some(split) = split else { continue };
            let header = String::from_utf8_lossy(&bytes[..split]);
            let first = header.lines().next().unwrap_or("").to_string();
            ensure_fixture_authorization(&header);
            let len = header
                .lines()
                .find_map(|line| {
                    line.to_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|s| s.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            while bytes.len() < split + len {
                let n = stream.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&chunk[..n]);
            }
            let body = String::from_utf8_lossy(&bytes[split..]).to_string();
            seen.push(first.clone());
            let (kind, payload) = if first.contains("/audio/transcriptions") {
                (
                    "application/json",
                    if body.contains("shared-fixture") && !body.contains("legacy-asr") {
                        json!({"segments":[{"start":0.0,"end":1.5,"text":"Hello world"}]})
                            .to_string()
                    } else {
                        json!({"error":"wrong shared ASR model"}).to_string()
                    },
                )
            } else if body.contains("image_url") {
                (
                    "application/json",
                    json!({"choices":[{"message":{"content":"Clarora OCR"}}]}).to_string(),
                )
            } else {
                ("text/event-stream","data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\" world\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n".to_string())
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                payload.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
        seen
    });
    Ok((address, stop, handle))
}
fn reliability(lib: &Library, results: &mut Vec<Value>) -> Result<()> {
    review_queue::run(lib, results)?;
    ai_completion::run(lib, results)?;
    backup_cancel::run(lib, results)?;
    Ok(())
}
fn run(output: &Path, results: &mut Vec<Value>) -> Result<()> {
    let root = output.join(format!("run-{}", data::id()));
    fs::create_dir_all(&root)?;
    let lib = Library::open(root.join("source"))?;
    video_repeat::run(&lib, results)?;
    writing::run(&lib, results)?;
    reliability(&lib, results)?;
    lib.import_words("bright\t明亮的\ncalm - 平静的", "manual", false)?;
    ensure!(lib.list("words")?.len() == 2, "词条导入失败");
    let card = lib
        .cards(false)?
        .into_iter()
        .find(|c| c.front == "bright")
        .context("词卡不存在")?;
    lib.grade(&card, "good")?;
    ensure!(
        !lib.cards(true)?.iter().any(|c| c.id == card.id),
        "已评分卡片仍在到期队列"
    );
    lib.import_words("bright\t明亮；聪明", "manual", false)?;
    ensure!(
        lib.list("review_schedule")?.len() == 1,
        "重复导入重置复习记录"
    );
    results.push(json!({"workflow":"词条导入 → 评分 → 重导入保留调度","passed":true}));
    let previous = lib.list("review_schedule")?;
    lib.update_card(&card, "brighter", "更明亮")?;
    ensure!(
        lib.list("review_schedule")? == previous
            && lib
                .cards(false)?
                .iter()
                .any(|c| c.id == card.id && c.front == "brighter"),
        "编辑卡片丢失调度或内容"
    );
    ensure!(
        lib.update_card(&card, "calm", "冲突").is_err() && lib.list("review_schedule")? == previous,
        "重复词条编辑没有原子失败"
    );
    results.push(json!({"workflow":"卡片编辑保留复习进度与唯一词冲突回滚","passed":true}));

    let dir = root.join("word-directory");
    fs::create_dir(&dir)?;
    fs::write(dir.join("gentle.txt"), "温和的")?;
    lib.import_directory(&dir)?;
    let gentle = lib
        .cards(false)?
        .into_iter()
        .find(|c| c.front == "gentle")
        .unwrap();
    lib.grade(&gentle, "hard")?;
    fs::write(dir.join("gentle.txt"), "温柔的")?;
    lib.import_directory(&dir)?;
    ensure!(
        lib.list("review_schedule")?.len() == 2,
        "目录重导入丢失进度"
    );
    fs::remove_file(dir.join("gentle.txt"))?;
    fs::write(dir.join("quiet.txt"), "安静的")?;
    lib.import_directory(&dir)?;
    ensure!(
        !lib.cards(false)?.iter().any(|c| c.front == "gentle"),
        "目录替换未删除旧卡"
    );
    results.push(json!({"workflow":"目录整体替换与保留已存在词条进度","passed":true}));
    let anki = root.join("anki.tsv");
    fs::write(
        &anki,
        "#separator:Tab\n#html:true\n#columns:Back\tFront\tTags\n#tags column:3\n\"<b>你好</b>\"\t\"Hello\nworld\"\tdemo\n",
    )?;
    ensure!(
        documents::import_anki(&lib, &anki)? == 1,
        "Anki 文本导入失败"
    );
    let before = lib.list("words")?.len();
    fs::write(&anki, "\"unterminated\tbroken")?;
    ensure!(
        documents::import_anki(&lib, &anki).is_err() && lib.list("words")?.len() == before,
        "Anki 失败没有原子回滚"
    );
    results.push(json!({"workflow":"Anki 多行/HTML/命名字段导入及格式错误不写库","passed":true}));
    let video = root.join("sample.mp4");
    let cancel = process::cancel_token();
    let args = [
        "-nostdin",
        "-v",
        "error",
        "-y",
        "-f",
        "lavfi",
        "-i",
        "testsrc=size=320x180:rate=10",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:sample_rate=44100",
        "-t",
        "4",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
    ]
    .map(String::from)
    .to_vec();
    let mut args = args;
    args.push(video.to_string_lossy().into());
    process::run(
        &process::tool("ffmpeg")?,
        &args,
        &cancel,
        Duration::from_secs(30),
    )?;
    fs::write(
        video.with_extension("srt"),
        "1\n00:00:00,000 --> 00:00:02,000\nHello world\n\n2\n00:00:02,000 --> 00:00:04,000\nStay calm\n",
    )?;
    let waveform = waveform::extract(&lib, &video, &cancel)?;
    ensure!(
        (waveform.duration - 4.).abs() < 0.1
            && !waveform.peaks.is_empty()
            && waveform.peaks.len() <= 512
            && waveform
                .peaks
                .iter()
                .all(|p| p.is_finite() && *p >= 0. && *p <= 1.),
        "真实音频波形无效"
    );
    let cached = waveform::extract(&lib, &video, &cancel)?;
    ensure!(cached.peaks == waveform.peaks, "波形缓存不一致");
    fs::write(
        output.join("waveform.json"),
        serde_json::to_vec_pretty(&waveform)?,
    )?;
    results.push(json!({"workflow":"真实 FFmpeg PCM 解码 → 有界波形扫描 → 缓存恢复","passed":true,"evidence":"waveform.json"}));
    let group = lib.practice("E2E")?;
    lib.import_audio(&group, &video)?;
    let audio = lib.list("listening_audios")?.remove(0);
    let en = subtitles::parse(&fs::read_to_string(text(&audio, "subtitle_uri"))?);
    let zh = subtitles::parse("WEBVTT\n\n00:00.500 --> 00:02.000\n你好世界\n");
    let merged = subtitles::merge(&en, &zh, true)?;
    ensure!(
        merged
            .iter()
            .any(|c| c.text.contains("Hello world\n你好世界")),
        "双语合并未保留原文"
    );
    let merged_path = lib.root.join("media/merged.srt");
    fs::write(&merged_path, subtitles::srt(&merged))?;
    lib.db()?.execute(
        "UPDATE listening_audios SET subtitle_uri=? WHERE id=?",
        rusqlite::params![merged_path.to_string_lossy(), text(&audio, "id")],
    )?;
    let clip = lib.root.join("media/clip.mp4");
    process::clip(&video, &clip, 1., 2.5, &cancel)?;
    lib.put("video_clips",json!({"id":data::id(),"en_text":"Hello","zh_text":"你好","video_uri":clip.to_string_lossy(),"created_at":data::now()}))?;
    results.push(
        json!({"workflow":"真实 FFmpeg 媒体 → 导入组 → 双语字幕 → 片段导出入卡","passed":true}),
    );
    let player = Player::without_audio_device();
    player.set("volume", 0);
    player.open(&video);
    let deadline = Instant::now() + Duration::from_secs(12);
    let state = loop {
        let state = player.snapshot();
        ensure!(state.error.is_empty(), "播放器失败：{}", state.error);
        if state.duration > 0. && state.position > 0.1 && state.frame.is_some() {
            break state;
        }
        ensure!(
            Instant::now() < deadline,
            "libmpv 未返回播放帧：duration={} position={} paused={} frame={}",
            state.duration,
            state.position,
            state.paused,
            state.frame.is_some()
        );
        thread::sleep(Duration::from_millis(40));
    };
    let frame = state.frame.context("没有帧")?;
    let mut bytes = frame.as_bytes(0).context("帧内容不存在")?.to_vec();
    for pixel in bytes.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    image::save_buffer(
        output.join("playback-frame.png"),
        &bytes,
        960,
        540,
        image::ColorType::Rgba8,
    )?;
    let eof_deadline = Instant::now() + Duration::from_secs(12);
    while !player.snapshot().eof {
        ensure!(Instant::now() < eof_deadline, "真实媒体未报告EOF");
        thread::sleep(Duration::from_millis(40));
    }
    player.open_range(&video, 0., 0., true);
    let reset_deadline = Instant::now() + Duration::from_secs(4);
    while player.snapshot().eof {
        ensure!(Instant::now() < reset_deadline, "重新打开未清除EOF");
        thread::sleep(Duration::from_millis(40));
    }
    player.set("pause", "yes");
    drop(player);
    results
        .push(json!({"workflow":"真实mpv自然播完EOF → 重开清除EOF，供分P续播判断","passed":true}));
    results.push(json!({"workflow":"libmpv 解码、null音频时钟、软件帧到 GPUI 图像","passed":true,"duration":state.duration,"evidence":"playback-frame.png"}));
    let second = root.join("second.mp4");
    fs::copy(&video, &second)?;
    fs::copy(video.with_extension("srt"), second.with_extension("srt"))?;
    lib.import_audio(&group, &second)?;
    let selected = lib.list("listening_audios")?;
    audio_tools::merge(&lib, &selected, &group, &cancel)?;
    let joined = lib
        .list("listening_audios")?
        .into_iter()
        .find(|r| text(r, "name").starts_with("合并"))
        .context("合并记录未保存")?;
    let joined_cues = subtitles::parse(&fs::read_to_string(text(&joined, "subtitle_uri"))?);
    ensure!(
        joined_cues.iter().any(|c| c.start >= 4.) && joined_cues.iter().all(|c| c.end > c.start),
        "合并字幕没有移动到第二段音频时间轴"
    );
    results.push(json!({"workflow":"勾选音频 → FFmpeg 采样归一合并 → 双语字幕时间轴偏移 → 保留原记录","passed":true}));
    let broken = Player::without_audio_device();
    broken.open(&root.join("missing.mp4"));
    let deadline = Instant::now() + Duration::from_secs(8);
    while broken.snapshot().error.is_empty() {
        ensure!(Instant::now() < deadline, "异步加载失败没有反馈");
        thread::sleep(Duration::from_millis(40));
    }
    drop(broken);
    results.push(json!({"workflow":"libmpv 异步加载错误反馈","passed":true}));
    let archive = root.join("library.zip");
    backup::export_local(&lib, &archive)?;
    let restored = Library::open(root.join("restored"))?;
    backup::import_local(&restored, &archive)?;
    let count = restored.cards(false)?.len();
    backup::import_local(&restored, &archive)?;
    ensure!(
        restored.cards(false)?.len() == count,
        "重复恢复产生重复卡片"
    );
    ensure!(
        restored.list("review_schedule")?.len() == lib.list("review_schedule")?.len(),
        "恢复缺少复习进度"
    );
    for row in restored.list("listening_audios")? {
        ensure!(
            PathBuf::from(text(&row, "audio_uri")).is_file()
                && PathBuf::from(text(&row, "subtitle_uri")).is_file(),
            "恢复附件引用失效"
        );
    }
    ensure!(
        restored.setting("ai_config")?.is_empty(),
        "备份包含设备凭证设置"
    );
    results.push(json!({"workflow":"含附件 ZIP 备份 → 新库恢复 → 重复恢复去重","passed":true}));
    let imported = Library::open(root.join("rn-snapshot"))?;
    backup::import_sqlite(&imported, &lib.root.join("clarora.db"))?;
    ensure!(
        imported.cards(false)?.len() == lib.cards(false)?.len(),
        "只读快照导入数量不一致"
    );
    let data_before = restored.vault()?;
    let mut malformed = data_before.clone();
    malformed.get_mut("listening_audios").unwrap()[0]
        .insert("practice_id".into(), json!("does-not-exist"));
    malformed.get_mut("words").unwrap().push(serde_json::from_value(json!({"id":999999,"word":"rollback-only","meaning":"应回滚","source":"manual","created_at":data::now()}))?);
    ensure!(
        backup::merge(&restored, &malformed).is_err(),
        "损坏引用未被拒绝"
    );
    ensure!(
        restored.vault()? == data_before,
        "恢复失败留下了部分数据库写入"
    );
    results.push(json!({"workflow":"只读 RN 快照导入与跨表恢复失败事务回滚","passed":true}));
    video_library::run(&lib, &video, results)?;
    credential_mode::run(&lib, results)?;
    audio_library::run(&lib, &video, results)?;
    blind_listening::run(&lib, &video, results)?;
    bilibili_parts::run(&lib, results)?;
    model_progress::run(&lib, results)?;
    kokoro::run(&lib, results)?;
    video_qa::run(&lib, results)?;
    video_translation::run(&lib, &video, results)?;
    ensure!(
        process::normalize_bilibili(
            "【分享】 https://www.bilibili.com/video/BV1xx411c7mD?p=2&share_source=x"
        )? == "https://www.bilibili.com/video/BV1xx411c7mD?p=2",
        "分 P 规范化失败"
    );
    ensure!(
        process::normalize_bilibili("https://www.bilibili.com.evil.test/video/BV1xx411c7mD")
            .is_err(),
        "伪造域名被允许"
    );
    #[cfg(unix)]
    {
        let cancelled = process::cancel_token();
        let token = cancelled.clone();
        let worker = thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            token.store(true, Ordering::SeqCst);
        });
        ensure!(
            process::run(
                Path::new("/bin/sh"),
                &["-c".into(), "sleep 30 & wait".into()],
                &cancelled,
                Duration::from_secs(5)
            )
            .is_err(),
            "未取消进程组"
        );
        worker.join().unwrap();
        ensure!(
            process::run(
                Path::new("/bin/sh"),
                &["-c".into(), "printf complete; exit 3".into()],
                &cancel,
                Duration::from_secs(2)
            )
            .is_err(),
            "stdout 掩盖了非零退出码"
        );
    }
    results.push(json!({"workflow":"链接校验、进程组取消、非零退出判错","passed":true}));
    let (address, stop, server) = mock_ai()?;
    let config = ai::Config {
        base_url: address.clone(),
        model: "shared-fixture".into(),
        vision_model: "mock".into(),
        asr_base_url: "http://127.0.0.1:1/legacy".into(),
        asr_model: "legacy-asr".into(),
        engine: "compatible".into(),
        ..Default::default()
    };
    let (shared, shared_key) = ai::shared_settings(
        config.clone(),
        "shared-fixture-key".into(),
        "legacy-asr-key".into(),
    );
    ensure!(
        shared.base_url == config.base_url && shared_key == "shared-fixture-key",
        "共用设置混用了旧转写密钥"
    );
    let legacy = ai::Config {
        asr_base_url: config.base_url.clone(),
        asr_model: "legacy-only".into(),
        ..Default::default()
    };
    let (legacy, legacy_key) = ai::shared_settings(legacy, String::new(), "asr-only-key".into());
    ensure!(
        legacy.base_url == config.base_url
            && legacy.model == "legacy-only"
            && legacy_key == "asr-only-key",
        "只有旧转写配置时未沿用完整一组"
    );
    let preferences = Library::open(lib.root.join("ai-preferences-without-keychain"))?;
    let mut metadata = config.clone();
    metadata.credential_id = "e2e-unread-credential".into();
    preferences.set_setting("ai_config", &serde_json::to_string(&metadata)?)?;
    ensure!(
        ai::load_config(&preferences)?.credential_id == metadata.credential_id,
        "仅配置加载丢失凭证引用"
    );
    let mut draft = metadata.clone();
    draft.local_model = "tiny.en".into();
    ai::save_preferences(&preferences, draft, "")?;
    let retained = ai::load_config(&preferences)?;
    ensure!(
        retained.credential_id == metadata.credential_id && retained.local_model == "tiny.en",
        "空密钥保存丢失原凭证或本地模型"
    );
    let mut legacy_metadata = ai::Config {
        asr_base_url: metadata.base_url.clone(),
        asr_model: "asr-only".into(),
        credential_id: metadata.credential_id.clone(),
        ..Default::default()
    };
    preferences.set_setting("ai_config", &serde_json::to_string(&legacy_metadata)?)?;
    legacy_metadata.base_url = legacy_metadata.asr_base_url.clone();
    legacy_metadata.model = legacy_metadata.asr_model.clone();
    ai::save_preferences(&preferences, legacy_metadata, "")?;
    let retained_asr = ai::load_config(&preferences)?;
    ensure!(
        retained_asr.shared_key_source == Some(ai::CredentialSource::Asr)
            && retained_asr.credential_id == metadata.credential_id,
        "旧转写密钥引用未保留"
    );
    fs::write(
        output.join("keychain-lazy-evidence.json"),
        serde_json::to_vec_pretty(
            &json!({"metadata_load":true,"blank_key_retains_reference":true,"local_preferences_without_secret_read":true,"legacy_asr_reference_retained":true,"uses_disposable_metadata_only":true}),
        )?,
    )?;
    let config = config.shared_api();
    ensure!(
        config.asr_base_url == config.base_url && config.asr_model == config.model,
        "URL或模型未共用"
    );
    ai::save_shared(&lib, config.clone(), "sqlite-http-fixture")?;
    let reopened = Library::open(lib.root.clone())?;
    let loaded = ai::load(&reopened)?.0;
    ensure!(
        loaded.asr_base_url == loaded.base_url && loaded.asr_model == loaded.model,
        "重开配置不一致"
    );
    let answer = ai::chat(&lib, "说明", "Hello", &[], &cancel, |_| {})?;
    ensure!(answer == "Hello world", "SSE 输出不完整");
    let transcript = ai::transcribe(&lib, &video, &cancel)?;
    ensure!(transcript.len() == 1, "multipart 转写没有时间轴");
    let text = ai::ocr(&lib, &output.join("playback-frame.png"), &cancel)?;
    ensure!(text == "Clarora OCR", "视觉结果错误");
    stop.store(true, Ordering::SeqCst);
    let requests = server.join().unwrap();
    ensure!(requests.len() == 3, "AI HTTP 链路缺少请求");
    fs::write(
        output.join("shared-ai-settings.json"),
        serde_json::to_vec_pretty(
            &json!({"shared_model":loaded.model,"same_endpoint":loaded.base_url==loaded.asr_base_url,"chat_reply":answer,"transcript_cues":transcript.len(),"requests":requests,"legacy_asr_only_loaded":true,"credentials_in_config":false}),
        )?,
    )?;
    let session = data::id();
    let first = vec![
        json!({"role":"user","content":"hello"}),
        json!({"role":"assistant","content":answer}),
    ];
    ai::save_chat(&lib, &session, "material-1", "hello", &first)?;
    let mut continued = first;
    continued.push(json!({"role":"user","content":"continue"}));
    ai::save_chat(&lib, &session, "material-1", "continue", &continued)?;
    let reopened = Library::open(lib.root.clone())?;
    let stored = documents::records(&reopened, "chat")?;
    ensure!(
        stored.len() == 1
            && stored[0]["messages"]
                .as_array()
                .is_some_and(|m| m.len() == 3),
        "多轮会话更新或重启丢失"
    );
    for _ in 0..13 {
        ai::save_chat(&lib, &data::id(), "material-1", "new", &[])?;
    }
    ai::save_chat(&lib, &data::id(), "material-2", "other", &[])?;
    let stored = documents::records(&lib, "chat")?;
    ensure!(
        stored
            .iter()
            .filter(|r| r["context"] == "material-1")
            .count()
            == 12
            && stored
                .iter()
                .filter(|r| r["context"] == "material-2")
                .count()
                == 1,
        "材料会话隔离或12条上限错误"
    );
    results.push(json!({"workflow":"聊天连续更新 → 重开 SQLite 恢复 → 按材料隔离与12会话裁剪","passed":true}));
    let score = ai::score("Hello world", "Hello world")?;
    ensure!(score["accuracy"] == 1., "文本对齐结果错误");
    results.push(json!({"workflow":"真实本机 HTTP → SSE 聊天 / multipart ASR / 图片 OCR → SQLite 记录","passed":true,"requests":requests}));
    protocol::run(&lib, &video, &output.join("playback-frame.png"), results)?;
    {
        use clarora_gpui::video_subtitles as vs;
        let cues = vec![
            subtitles::Cue {
                start: 0.5,
                end: 1.5,
                text: "First sentence".into(),
            },
            subtitles::Cue {
                start: 2.,
                end: 3.,
                text: "Second sentence".into(),
            },
        ];
        let path = vs::save(&lib, &video, &cues, &cancel)?;
        let restored = vs::load(&Library::open(lib.root.clone())?, &video)?;
        ensure!(
            restored.len() == 2 && Path::new(&path).is_file(),
            "视频字幕重开失败"
        );
        for (time, expected) in [
            (0., None),
            (0.5, Some(0)),
            (1.5, None),
            (2., Some(1)),
            (3., None),
            (0.6, Some(0)),
        ] {
            ensure!(
                vs::active(&restored, time) == expected,
                "播放/倒退/空白段高亮错误"
            );
        }
        let other = lib.root.join("other-video.mp4");
        fs::copy(&video, &other)?;
        ensure!(vs::load(&lib, &other)?.is_empty(), "视频之间串字幕");
        let stopped = process::cancel_token();
        stopped.store(true, Ordering::SeqCst);
        ensure!(
            vs::save(&lib, &video, &cues, &stopped).is_err(),
            "取消仍保存字幕"
        );
        ensure!(
            vs::save(&lib, &video, &[], &cancel).is_err(),
            "空字幕被保存"
        );
        fs::copy(&path, output.join("video-timeline.srt"))?;
        results.push(json!({"workflow":"视频字幕持久化重开 → 文件隔离 → 播放/倒退/句间空白高亮 → 取消不覆盖","passed":true,"evidence":"video-timeline.srt"}));
    }
    if std::env::var_os("PDFIUM_LIB_PATH").is_some() {
        let pdf = root.join("sample.pdf");
        report_pdf(&pdf)?;
        let (path, count, _) = documents::render_pdf(&lib, &pdf, 0, 700)?;
        ensure!(count == 1 && Path::new(&path).is_file(), "PDF 渲染失败");
        let (zoom, _, _) = documents::render_pdf(&lib, &pdf, 0, 900)?;
        ensure!(zoom != path, "PDF 缩放复用了旧图缓存 URI");
        let page = documents::load_pdf_page(&lib, &pdf, u16::MAX, 700)?;
        ensure!(page.page == 0 && page.count == 1, "PDF 越界页码没有校正");
        ensure!(
            page.image.size(0).width.0 == 700 && page.image.size(0).height.0 > 0,
            "PDF 没有生成可直接绘制的完整图像"
        );
        ensure!(!page.text.trim().is_empty(), "PDF 页面文字未同步载入");
        fs::copy(path, output.join("pdf-page.png"))?;
        results.push(
            json!({"workflow":"PDF 文件解析和页面渲染","passed":true,"evidence":"pdf-page.png"}),
        );
    } else {
        results.push(json!({"workflow":"PDF 文件解析和页面渲染","passed":false,"skipped":"未设置 PDFIUM_LIB_PATH"}));
    }
    if let (Some(model), Some(speech)) = (
        std::env::var_os("CLARORA_ASR_MODEL"),
        std::env::var_os("CLARORA_ASR_SPEECH"),
    ) {
        fs::copy(model, lib.root.join("models/ggml-tiny.en.bin"))?;
        lib.set_setting(
            "ai_config",
            &json!({"engine":"local","local_model":"tiny.en","credential_id":"e2e-unread-local-credential"}).to_string(),
        )?;
        let speech = PathBuf::from(speech);
        asr_pauses::run(&lib, &speech, results)?;
        let spoken_video = root.join("spoken-video.mp4");
        process::run(
            &process::tool("ffmpeg")?,
            &[
                "-y".into(),
                "-f".into(),
                "lavfi".into(),
                "-i".into(),
                "color=c=0x183c31:s=640x360:r=24".into(),
                "-i".into(),
                speech.to_string_lossy().into(),
                "-shortest".into(),
                "-c:v".into(),
                "libx264".into(),
                "-c:a".into(),
                "aac".into(),
                spoken_video.to_string_lossy().into(),
            ],
            &cancel,
            Duration::from_secs(120),
        )?;
        let mut stages = vec![];
        let (_, cues) = clarora_gpui::video_subtitles::generate_with_progress(
            &lib,
            &spoken_video,
            &cancel,
            |stage| stages.push(stage.to_owned()),
        )?;
        ensure!(
            stages.len() == 3
                && stages[0].contains("提取")
                && stages[1].contains("识别")
                && stages[2].contains("保存"),
            "字幕处理阶段缺失或顺序错误"
        );
        fs::write(
            output.join("subtitle-task-stages.json"),
            serde_json::to_vec_pretty(&stages)?,
        )?;
        let stopped = process::cancel_token();
        let stop_at_asr = stopped.clone();
        let mut cancelled_stages = vec![];
        let cancelled_run = clarora_gpui::video_subtitles::generate_with_progress(
            &lib,
            &spoken_video,
            &stopped,
            |stage| {
                cancelled_stages.push(stage.to_owned());
                if stage.starts_with("2/3") {
                    stop_at_asr.store(true, Ordering::SeqCst);
                }
            },
        );
        ensure!(
            cancelled_run.is_err() && cancelled_stages.len() == 2,
            "取消后仍继续保存或未报告识别阶段"
        );
        ensure!(
            clarora_gpui::video_subtitles::load(&lib, &spoken_video)?
                .iter()
                .zip(&cues)
                .all(|(a, b)| a.text == b.text),
            "取消覆盖已有字幕"
        );
        ensure!(!fs::read_dir(lib.root.join("media"))?.any(|p| p.is_ok_and(|p| p.file_name().to_string_lossy().starts_with("video-asr-"))), "取消后临时音轨未清理");
        fs::write(
            output.join("subtitle-task-cancel.json"),
            serde_json::to_vec_pretty(
                &json!({"stages":cancelled_stages,"cancelled_before_save":true,"original_subtitles_retained":true,"temporary_audio_cleaned":true}),
            )?,
        )?;
        fs::copy(&spoken_video, output.join("spoken-video.mp4"))?;
        let restored = clarora_gpui::video_subtitles::load(&lib, &spoken_video)?;
        ensure!(restored.len() == cues.len(), "真实视频转写未持久化");
        results.push(json!({"workflow":"合成语音MP4 → 提取音轨 → Whisper转写 → 自动恢复视频字幕","passed":true,"evidence":"spoken-video.mp4"}));
        let transcript = cues
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        ensure!(
            transcript.to_lowercase().contains("hello")
                && transcript.to_lowercase().contains("learning"),
            "真实 Whisper 端侧识别未识别出合成语音：{transcript}"
        );
        fs::write(
            output.join("offline-transcription.srt"),
            subtitles::srt(&cues),
        )?;
        results.push(json!({"workflow":"合成英文语音 → FFmpeg 转换 → 原生 Whisper tiny.en 推理 → 字幕","passed":true,"transcript":transcript,"evidence":"offline-transcription.srt"}));
        video_jobs::run(&lib, &spoken_video, results)?;
    }
    fs::write(
        output.join("data-location.txt"),
        root.to_string_lossy().as_bytes(),
    )?;
    Ok(())
}
fn main() -> Result<()> {
    let args = std::env::args_os().collect::<Vec<_>>();
    let output = args
        .get(1)
        .map(PathBuf::from)
        .context("用法：cargo run --bin e2e -- <证据目录>")?;
    fs::create_dir_all(&output)?;
    let output = output.canonicalize()?;
    let mut results = vec![];
    let mode = match args.get(2).and_then(|arg| arg.to_str()) {
        Some("--http-only") => "http-only",
        Some("--reliability-only") => "reliability-only",
        Some("--writing-only") => "writing-only",
        Some("--video-repeat-only") => "video-repeat-only",
        _ => "full",
    };
    let result = if mode == "http-only" {
        let lib = Library::open(output.join(format!("http-{}", data::id())))?;
        protocol::run(
            &lib,
            &PathBuf::from(args.get(3).context("缺少媒体夹具")?),
            &PathBuf::from(args.get(4).context("缺少图片夹具")?),
            &mut results,
        )
    } else if mode == "video-repeat-only" {
        let lib = Library::open(output.join(format!("video-repeat-{}", data::id())))?;
        video_repeat::run(&lib, &mut results)
    } else if mode == "writing-only" {
        let lib = Library::open(output.join(format!("writing-{}", data::id())))?;
        writing::run(&lib, &mut results)
    } else if mode == "reliability-only" {
        let lib = Library::open(output.join(format!("reliability-{}", data::id())))?;
        reliability(&lib, &mut results)
    } else {
        run(&output, &mut results)
    };
    let report = json!({"mode":mode,"date":chrono::Local::now().to_rfc3339(),"passed":result.is_ok(),"workflows":results,"error":result.as_ref().err().map(|e|format!("{e:#}"))});
    fs::write(
        output.join("e2e-report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    result
}
