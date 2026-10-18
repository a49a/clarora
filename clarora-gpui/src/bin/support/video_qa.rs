use anyhow::{Result, ensure};
use clarora_gpui::{
    ai,
    data::Library,
    process,
    subtitles::Cue,
    video_qa::{self, Scope},
};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};
pub fn run(lib: &Library, results: &mut Vec<Value>) -> Result<()> {
    let lib = &Library::open(lib.root.join("video-qa"))?;
    let video = lib.root.join("qa-fixture.mp4");
    std::fs::write(&video, b"identity fixture")?;
    let cues = vec![
        Cue {
            start: 0.,
            end: 1.5,
            text: "First sentence about retrieval practice.".into(),
        },
        Cue {
            start: 3.,
            end: 5.,
            text: "Second sentence says spaced review helps memory.".into(),
        },
    ];
    let full = video_qa::build(&video, &cues, Scope::All, 0., "")?;
    let current = video_qa::build(&video, &cues, Scope::Current, 0.5, "")?;
    let selected = video_qa::build(&video, &cues, Scope::Cue(1), 0.5, "")?;
    ensure!(
        full.text.contains(&cues[0].text)
            && full.text.contains(&cues[1].text)
            && full.text.contains("00:00:03.000"),
        "完整字幕或时间戳缺失"
    );
    ensure!(
        current.text.contains(&cues[0].text) && !current.text.contains(&cues[1].text),
        "当前句范围错误"
    );
    ensure!(
        full.key != current.key && selected.key != current.key,
        "不同范围串会话"
    );
    ensure!(
        current.key == video_qa::build(&video, &cues, Scope::Current, 1., "")?.key,
        "同一句随播放漂移"
    );
    ensure!(
        video_qa::build(&video, &cues, Scope::Current, 2., "").is_err(),
        "空白段误选当前句"
    );
    ensure!(
        video_qa::build(&video, &[], Scope::All, 0., "").is_err(),
        "无字幕未拒绝"
    );
    let fallback = video_qa::build(&video, &[], Scope::Current, 2., "Embedded subtitle")?;
    ensure!(
        fallback.text.contains("Embedded subtitle"),
        "内嵌当前字幕失效"
    );
    let other = lib.root.join("qa-other.mp4");
    std::fs::write(&other, b"identity fixture")?;
    ensure!(
        video_qa::build(&other, &cues, Scope::All, 0., "")?.key != full.key,
        "其他视频串会话"
    );
    let mut revised = cues.clone();
    revised[1].text = "Updated subtitle".into();
    ensure!(
        video_qa::build(&video, &revised, Scope::All, 0., "")?.key != full.key,
        "修改字幕串旧会话"
    );
    let huge = vec![Cue {
        start: 0.,
        end: 1.,
        text: "中".repeat(120001),
    }];
    ensure!(
        video_qa::build(&video, &huge, Scope::All, 0., "").is_err(),
        "过长字幕静默截断"
    );
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let base = format!("http://{}", listener.local_addr()?);
    let server = thread::spawn(move || {
        let mut requests = vec![];
        for _ in 0..2 {
            let (mut socket, _) = listener.accept().unwrap();
            let mut bytes = vec![];
            let mut buf = [0u8; 4096];
            let header = loop {
                let n = socket.read(&mut buf).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let length = String::from_utf8_lossy(&bytes[..header])
                .lines()
                .find_map(|l| {
                    l.to_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|v| v.trim().parse::<usize>().ok())
                })
                .unwrap();
            while bytes.len() < header + length {
                let n = socket.read(&mut buf).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
            }
            requests
                .push(serde_json::from_slice::<Value>(&bytes[header..header + length]).unwrap());
            let payload = format!(
                "data: {}\n\ndata: [DONE]\n\n",
                json!({"choices":[{"delta":{"content":"根据字幕 [00:00:03.000]，间隔复习有助记忆。"},"finish_reason":"stop"}]})
            );
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",payload.len(),payload).unwrap();
        }
        requests
    });
    let original = lib.setting("ai_config")?;
    lib.set_setting(
        "ai_config",
        &json!({"base_url":base,"model":"fixture","credential_id":""}).to_string(),
    )?;
    let cancel = process::cancel_token();
    let first = video_qa::ask(lib, "如何复习？", &full, &[], &cancel, |_| {})?;
    let history = vec![
        json!({"role":"user","content":"如何复习？","reference":full.text}),
        json!({"role":"assistant","content":first.answer}),
    ];
    let second = video_qa::ask(lib, "为什么？", &full, &history, &cancel, |_| {})?;
    ai::save_chat(lib, "qa-fixture-session", &full.key, "字幕提问", &history)?;
    let requests = server.join().unwrap();
    lib.set_setting("ai_config", &original)?;
    ensure!(
        requests[0]["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains(&cues[1].text),
        "未向AI发送完整字幕"
    );
    ensure!(
        requests[1]["messages"].as_array().unwrap().len() == 4,
        "多轮历史丢失"
    );
    ensure!(
        requests[1]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m.get("reference").is_none()),
        "本机引用元数据泄入协议字段"
    );
    ensure!(second.answer.contains("00:00:03.000"), "回答丢失");
    std::fs::write(
        lib.root.join("video-qa-requests.json"),
        serde_json::to_vec_pretty(
            &json!({"full":full,"current":current,"selected":selected,"requests":requests,"answer":second.answer}),
        )?,
    )?;
    results.push(json!({"workflow":"视频字幕当前句/选中行/整段与稳定身份 → 本机AI两轮带字幕/历史 → 空白/超长/改字幕/分P隔离","passed":true}));
    Ok(())
}
