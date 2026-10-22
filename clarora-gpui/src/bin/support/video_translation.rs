//! Real HTTP, subtitle cache and libmpv caption visibility workflow.
use anyhow::{Result, ensure};
use clarora_gpui::{data::Library, media::Player, process, subtitles::Cue, video_subtitles};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::{Duration, Instant},
};
pub fn run(lib: &Library, video: &std::path::Path, results: &mut Vec<Value>) -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let address = format!("http://{}", listener.local_addr()?);
    let stop = process::cancel_token();
    let flag = stop.clone();
    let fixture_root = lib.root.clone();
    let fixture_video = video.to_path_buf();
    let server = thread::spawn(move || -> Result<Vec<Value>> {
        let mut seen = vec![];
        while !flag.load(std::sync::atomic::Ordering::SeqCst) {
            let (mut socket, _) = match listener.accept() {
                Ok(s) => s,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
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
            let headers = String::from_utf8_lossy(&bytes[..split]);
            let length = headers
                .lines()
                .find_map(|l| {
                    l.to_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|v| v.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            while bytes.len() < split + length {
                let n = socket.read(&mut buf)?;
                ensure!(n > 0, "HTTP体中断");
                bytes.extend_from_slice(&buf[..n]);
            }
            let request: Value = serde_json::from_slice(&bytes[split..split + length])?;
            let messages = request["messages"].as_array().unwrap();
            let all = serde_json::to_string(messages)?;
            ensure!(!all.contains("旧译文"), "重复翻译发出了旧中文");
            let source = messages.first().unwrap()["content"].as_str().unwrap();
            // chat_stream appends material after the user question; locate the JSON source array.
            let start = source.find("[{\"").unwrap();
            let end = source.rfind(']').unwrap();
            let cues: Vec<Value> = serde_json::from_str(&source[start..=end])?;
            let bad = request["model"] == "bad-translation"
                || (request["model"] == "partial-fail"
                    && cues[0]["text"].as_str().unwrap().contains("sentence 32"));
            if request["model"] == "stale-subtitles" {
                let changed = Library::open(fixture_root.clone())?;
                video_subtitles::save(
                    &changed,
                    &fixture_video,
                    &[Cue {
                        start: 0.,
                        end: 1.,
                        text: "Newer English source".into(),
                    }],
                    &process::cancel_token(),
                )?;
            }
            let translated=cues.iter().map(|c|json!({"id":if bad {999}else{c["id"].as_u64().unwrap()},"text":"这是中文翻译。"})).collect::<Vec<_>>();
            seen.push(json!({"model":request["model"],"cues":cues.len(),"english_only":true}));
            let answer = serde_json::to_string(&translated)?;
            let body = format!(
                "data: {}\n\ndata: [DONE]\n\n",
                json!({"choices":[{"delta":{"content":answer},"finish_reason":"stop"}]})
            );
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )?;
        }
        Ok(seen)
    });
    let run = (|| -> Result<()> {
        lib.set_setting(
            "ai_config",
            &json!({"base_url":address,"model":"video-translate"}).to_string(),
        )?;
        let source = (0..35)
            .map(|i| Cue {
                start: i as f64,
                end: i as f64 + 0.8,
                text: format!("English sentence {i}.\n旧译文"),
            })
            .collect::<Vec<_>>();
        let cancel = process::cancel_token();
        video_subtitles::save(lib, video, &source, &cancel)?;
        let mut progress = vec![];
        let (path, translated) =
            video_subtitles::translate_with_progress(lib, video, &source, &cancel, |stage| {
                progress.push(stage.to_owned())
            })?;
        ensure!(
            progress.first().is_some_and(|s| s.contains("0/"))
                && progress.last().is_some_and(|s| s.contains("保存")),
            "翻译阶段进度未报告"
        );
        fs::write(
            lib.root.join("subtitle-translation-stages.json"),
            serde_json::to_vec_pretty(&progress)?,
        )?;
        ensure!(translated.len() == source.len(), "翻译丢句");
        ensure!(
            translated
                .iter()
                .zip(&source)
                .all(|(a, b)| a.start == b.start
                    && a.end == b.end
                    && a.text.contains("English sentence")
                    && a.text.contains("这是中文翻译")
                    && !a.text.contains("旧译文")),
            "原文／时间轴／中文错误"
        );
        ensure!(
            video_subtitles::load(lib, video)?.len() == 35
                && fs::read_to_string(path)?.contains("这是中文翻译"),
            "双语缓存未保存"
        );
        lib.set_setting(
            "ai_config",
            &json!({"base_url":address,"model":"bad-translation"}).to_string(),
        )?;
        ensure!(
            video_subtitles::translate(lib, video, &translated, &cancel).is_err(),
            "错误编号被保存"
        );
        let after = video_subtitles::load(lib, video)?;
        ensure!(
            after.iter().zip(&translated).all(|(a, b)| a.text == b.text),
            "失败覆盖了原缓存"
        );
        lib.set_setting(
            "ai_config",
            &json!({"base_url":address,"model":"partial-fail"}).to_string(),
        )?;
        ensure!(
            video_subtitles::translate(lib, video, &translated, &cancel).is_err(),
            "后续批次失败仍保存"
        );
        ensure!(
            video_subtitles::load(lib, video)?.len() == 35,
            "部分翻译覆盖原缓存"
        );
        lib.set_setting(
            "ai_config",
            &json!({"base_url":address,"model":"stale-subtitles"}).to_string(),
        )?;
        ensure!(
            video_subtitles::translate(lib, video, &translated, &cancel).is_err(),
            "源字幕更换后仍提交旧翻译"
        );
        ensure!(
            video_subtitles::load(lib, video)?[0].text == "Newer English source",
            "迟到翻译覆盖新字幕"
        );
        let cancelled = process::cancel_token();
        cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
        ensure!(
            video_subtitles::translate(lib, video, &source, &cancelled).is_err(),
            "取消仍翻译"
        );
        let player = Player::without_audio_device();
        player.open(video);
        player.set_subtitles_visible(false);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let p = player.snapshot();
            if p.duration > 0. && !p.primary_subtitles_visible && !p.secondary_subtitles_visible {
                break;
            }
            ensure!(Instant::now() < deadline, "关闭字幕未生效：{}", p.error);
            thread::sleep(Duration::from_millis(20));
        }
        player.set_subtitles_visible(true);
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let p = player.snapshot();
            if p.secondary_subtitles_visible {
                break;
            }
            ensure!(Instant::now() < deadline, "恢复字幕未生效");
            thread::sleep(Duration::from_millis(20));
        }
        Ok(())
    })();
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let requests = server.join().unwrap()?;
    run?;
    fs::write(
        lib.root.join("video-translation-evidence.json"),
        serde_json::to_vec_pretty(
            &json!({"requests":requests,"timeline_preserved":true,"failure_preserved_cache":true,"cancelled":true,"native_subtitle_visibility_toggled":true}),
        )?,
    )?;
    results.push(json!({"workflow":"视频英文分批翻译 → 双语原子缓存 → 错误/取消保留 → libmpv字幕开关","passed":true,"evidence":"video-translation-evidence.json"}));
    Ok(())
}
