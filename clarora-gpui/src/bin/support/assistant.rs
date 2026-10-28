//! Real HTTP / explicit selected-text / SQLite conversation workflow.
use anyhow::{Result, ensure};
use clarora_gpui::{
    ai,
    assistant::{self, Reference},
    chat_store,
    data::Library,
    process,
};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::Duration,
};

pub fn run(parent: &Library, results: &mut Vec<Value>) -> Result<()> {
    let lib = Library::open(parent.root.join("global-assistant"))?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = format!("http://{}", listener.local_addr()?);
    let server = thread::spawn(move || -> Result<Vec<Value>> {
        let mut seen = vec![];
        for index in 0..3 {
            let (mut socket, _) = listener.accept()?;
            socket.set_read_timeout(Some(Duration::from_secs(10)))?;
            let mut bytes = vec![];
            let mut buf = [0u8; 4096];
            let header = loop {
                let count = socket.read(&mut buf)?;
                ensure!(count > 0, "HTTP header closed");
                bytes.extend_from_slice(&buf[..count]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let length = String::from_utf8_lossy(&bytes[..header])
                .lines()
                .find_map(|line| {
                    line.to_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|s| s.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            while bytes.len() < header + length {
                let count = socket.read(&mut buf)?;
                ensure!(count > 0, "HTTP body closed");
                bytes.extend_from_slice(&buf[..count]);
            }
            seen.push(serde_json::from_slice(&bytes[header..header + length])?);
            let finish = if index == 2 { "length" } else { "stop" };
            let payload = format!(
                "data: {}\n\ndata: [DONE]\n\n",
                json!({"choices":[{"delta":{"content":format!("清晰的回答 {index}")},"finish_reason":finish}]})
            );
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                payload.len(),
                payload
            )?;
        }
        Ok(seen)
    });
    ai::save_shared(
        &lib,
        ai::Config {
            base_url: address,
            model: "text-model".into(),
            vision_model: "vision-model".into(),
            ..Default::default()
        },
        "",
    )?;
    const TITLE: &str = "Fixture-Private-Video-Title.mp4";
    let old_ref = json!({"label":format!("视频学习 · {TITLE}"),"text":"The bread is baked fresh every morning."});
    let first: Reference = serde_json::from_value(old_ref.clone())?;
    let old_video_text = format!(
        "视频：{TITLE}\n引用范围：所选一句 · 00:00–00:03\n以下仅为字幕文本，不包含画面信息：\n[00:00:00.000–00:00:03.000] The bread is baked fresh every morning."
    );
    let old_wrapped = json!({"label":format!("视频学习 · {TITLE}"),"text":old_video_text});
    let restored: Reference = serde_json::from_value(old_wrapped.clone())?;
    ensure!(
        restored.text == first.text,
        "旧草稿没有去掉自动标题／时间标记"
    );
    let new_turn = assistant::user_turn("fresh 在这里是什么意思？", &[first.clone()]);
    ensure!(
        new_turn["references"][0].get("label").is_none(),
        "新消息仍保存来源名称"
    );
    let cancel = process::cancel_token();
    let reply = assistant::ask(
        &lib,
        "fresh 在这里是什么意思？",
        &[first.clone()],
        &[],
        &cancel,
        |_| {},
    )?;
    ensure!(reply.answer == "清晰的回答 0", "首次回答错误");
    let mut history = vec![
        json!({"role":"user","content":"fresh 在这里是什么意思？","references":[old_wrapped]}),
        json!({"role":"assistant","content":reply.answer}),
    ];
    let picture = Reference {
        text: "I baked fresh bread this morning.\n这是鼠标选取的作答。".into(),
    };
    let second = assistant::ask(
        &lib,
        "与这句比较一下",
        &[picture.clone()],
        &history,
        &cancel,
        |_| {},
    )?;
    history.extend([
        assistant::user_turn("与这句比较一下", &[picture.clone()]),
        json!({"role":"assistant","content":second.answer}),
    ]);
    chat_store::save(
        &lib,
        "global-session",
        assistant::CONTEXT,
        "跨模块对话",
        &history,
    )?;
    let reopened = Library::open(lib.root.clone())?;
    let loaded = chat_store::get(&reopened, assistant::CONTEXT, "global-session")?.unwrap();
    ensure!(loaded.messages == history, "重开丢失引用或会话");
    history.extend([
        json!({"role":"user","content":"旧会话追问","reference":old_video_text}),
        json!({"role":"assistant","content":"旧回答"}),
    ]);
    ensure!(
        assistant::ask(&lib, "这次会截断", &[], &history, &cancel, |_| {}).is_err(),
        "截断被误认成功"
    );
    cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    ensure!(
        assistant::ask(&lib, "取消", &[], &history, &cancel, |_| {}).is_err(),
        "取消仍发送"
    );
    let requests = server.join().unwrap()?;
    ensure!(
        requests[0]["model"] == "text-model" && requests[1]["model"] == "text-model",
        "模型选择错误"
    );
    let serialized = requests[1].to_string();
    ensure!(
        serialized.contains("fresh every morning") && serialized.contains("I baked fresh bread"),
        "多轮请求遗漏原引用／新选区"
    );
    for request in &requests {
        let body = request.to_string();
        ensure!(
            !body.contains(TITLE)
                && !body.contains("source=")
                && !body.contains("视频学习")
                && !body.contains("引用范围："),
            "HTTP 请求夹带自动标题／来源：{body}"
        );
    }
    ensure!(
        assistant::messages(
            "文件名是我选取的内容",
            &[Reference { text: TITLE.into() }],
            &[]
        )?
        .last()
        .unwrap()["content"]
            .as_str()
            .unwrap()
            .contains(TITLE),
        "用户实际选取的文字被删除"
    );
    ensure!(
        requests[2].to_string().contains("I baked fresh bread"),
        "追问丢失引用"
    );
    ensure!(
        !serde_json::to_string(&history)?.contains("base64"),
        "数据库写入图片 base64"
    );
    ensure!(
        assistant::validate(&[Reference {
            text: "中".repeat(120001),
        }])
        .is_err(),
        "过长引用静默截断"
    );
    ensure!(
        !requests[0].to_string().contains("I baked fresh bread"),
        "首轮误带其他模块材料"
    );
    ensure!(
        assistant::validate(&vec![picture; 5]).is_err(),
        "过多引用未拒绝"
    );
    std::fs::write(
        lib.root.join("request-evidence.json"),
        serde_json::to_vec_pretty(&requests)?,
    )?;
    std::fs::write(
        lib.root.join("conversation.json"),
        serde_json::to_vec_pretty(&loaded.messages)?,
    )?;
    results.push(json!({"workflow":"global_assistant_selected_references_and_history","passed":true,"requests":requests.len(),"evidence":lib.root}));
    Ok(())
}
