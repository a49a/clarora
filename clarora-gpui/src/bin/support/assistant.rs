//! Real HTTP / explicit selected-text / SQLite conversation workflow.
use anyhow::{Result, ensure};
use clarora_gpui::{
    ai,
    assistant::{self, Reference, ReferenceKind},
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
    let new_turn = assistant::user_turn("fresh 在这里是什么意思？", std::slice::from_ref(&first));
    ensure!(
        new_turn["references"][0].get("label").is_none(),
        "新消息仍保存来源名称"
    );
    let cancel = process::cancel_token();
    let subtitle_refs: Vec<Reference> = serde_json::from_value(json!([
        {"kind":"selected_subtitle","text":first.text},
        {"kind":"transcript_background","text":"The bread is baked fresh every morning.\nWe open at eight."}
    ]))?;
    let reply = assistant::ask(
        &lib,
        "fresh 在这里是什么意思？",
        &subtitle_refs,
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
        ..Default::default()
    };
    let second = assistant::ask(
        &lib,
        "与这句比较一下",
        std::slice::from_ref(&picture),
        &history,
        &cancel,
        |_| {},
    )?;
    history.extend([
        assistant::user_turn("与这句比较一下", std::slice::from_ref(&picture)),
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
    let content = requests[0]["messages"][1]["content"]
        .as_str()
        .unwrap_or_default();
    ensure!(
        content
            == format!(
                "fresh 在这里是什么意思？\n\n<reference group=\"1\" kind=\"selected_subtitle\">\n{}\n</reference>\n\n<reference group=\"2\" kind=\"transcript_background\">\nThe bread is baked fresh every morning.\nWe open at eight.\n</reference>",
                first.text
            ),
        "发送内容没有区分所选字幕与完整背景"
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
            &[Reference {
                text: TITLE.into(),
                ..Default::default()
            }],
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
            ..Default::default()
        }])
        .is_err(),
        "过长引用静默截断"
    );
    ensure!(
        !requests[0].to_string().contains("I baked fresh bread"),
        "首轮误带其他模块材料"
    );
    ensure!(
        assistant::validate(
            &(0..5)
                .map(|index| Reference {
                    text: format!("material {index}"),
                    ..Default::default()
                })
                .collect::<Vec<_>>(),
        )
        .is_err(),
        "过多引用未拒绝"
    );
    let selected = Reference {
        text: "selected sentence".into(),
        kind: ReferenceKind::SelectedSubtitle,
        source: Some("video:A:1".into()),
        revision: Some("rev-a".into()),
    };
    let old_background = Reference {
        text: "A full material".into(),
        kind: ReferenceKind::TranscriptBackground,
        source: Some("video:A:1".into()),
        revision: Some("rev-a".into()),
    };
    let current = assistant::with_transcript_background(
        vec![selected.clone(), old_background],
        "B full material".into(),
        Some("video:B:1"),
        Some("rev-b"),
    )?;
    ensure!(
        current.len() == 3
            && current[0] == selected
            && current[1].text == "A full material"
            && current[2].text == "B full material"
            && current[2].source.as_deref() == Some("video:B:1"),
        "opening B replaced A's background or selection"
    );
    let outbound = assistant::messages("compare", &current, &[])?;
    let outbound = outbound.last().unwrap()["content"].as_str().unwrap();
    ensure!(
        outbound.contains("selected sentence")
            && outbound.contains("A full material")
            && outbound.contains("B full material")
            && !outbound.contains("video:A")
            && !outbound.contains("video:B"),
        "A/B content was not preserved without local source IDs"
    );
    ensure!(
        assistant::with_transcript_background(
            current.clone(),
            "B full material".into(),
            Some("video:B:1"),
            Some("rev-b"),
        )? == current,
        "reopening appended the same full background twice"
    );
    let cue_a = clarora_gpui::subtitles::Cue {
        start: 0.,
        end: 1.,
        text: "same timing".into(),
    };
    let mut cue_b = cue_a.clone();
    cue_b.text = "regenerated text".into();
    ensure!(
        assistant::subtitle_revision(std::slice::from_ref(&cue_a))
            != assistant::subtitle_revision(std::slice::from_ref(&cue_b)),
        "字幕文本变化未改变引用修订"
    );
    let regenerated = assistant::with_transcript_background(
        current.clone(),
        "A regenerated material".into(),
        Some("video:A:1"),
        Some("rev-a2"),
    )?;
    ensure!(
        regenerated.len() == 4
            && regenerated
                .iter()
                .any(|reference| reference.text == "A full material")
            && regenerated
                .iter()
                .any(|reference| reference.text == "A regenerated material"),
        "字幕重新生成后旧快照未保留"
    );
    ensure!(
        assistant::with_transcript_background(current, "x".repeat(120_001), None, None).is_err(),
        "oversized current material was accepted"
    );
    for (viewport, nav, page_min, wide, expected_width, expected_side) in [
        (900., 224., 760., false, 360., false),
        (1280., 224., 760., false, 360., false),
        (1440., 224., 760., false, 400., true),
        (1920., 224., 760., true, 600., true),
        (1280., 224., 620., false, 388., true),
        (1440., 224., 620., false, 400., true),
    ] {
        let available = viewport - nav - 32.;
        let width = assistant::column_width(available, page_min, wide);
        ensure!(
            (width - expected_width).abs() < 0.01
                && assistant::side_by_side(available, page_min, width) == expected_side,
            "助手布局阈值错误：{viewport}/{page_min}/{wide} -> {width}/{expected_side}"
        );
    }
    ensure!(
        assistant::column_width(600., 620., false) == assistant::MIN_COLUMN,
        "过窄窗口的助手列低于最低宽度"
    );
    let same_a = Reference {
        text: "same sentence".into(),
        kind: ReferenceKind::SelectedSubtitle,
        source: Some("video:A:1".into()),
        revision: Some("rev-a".into()),
    };
    let same_b = Reference {
        text: "same sentence".into(),
        kind: ReferenceKind::SelectedSubtitle,
        source: Some("video:B:1".into()),
        revision: Some("rev-b".into()),
    };
    let grouped = vec![
        same_a,
        Reference {
            text: "A background".into(),
            kind: ReferenceKind::TranscriptBackground,
            source: Some("video:A:1".into()),
            revision: Some("rev-a".into()),
        },
        same_b,
        Reference {
            text: "B background".into(),
            kind: ReferenceKind::TranscriptBackground,
            source: Some("video:B:1".into()),
            revision: Some("rev-b".into()),
        },
    ];
    let grouped_content = assistant::messages("compare", &grouped, &[])?;
    let grouped_content = grouped_content.last().unwrap()["content"].as_str().unwrap();
    ensure!(
        grouped_content.contains("<reference group=\"1\" kind=\"selected_subtitle\">")
            && grouped_content.contains("<reference group=\"1\" kind=\"transcript_background\">")
            && grouped_content.contains("<reference group=\"2\" kind=\"selected_subtitle\">")
            && grouped_content.contains("<reference group=\"2\" kind=\"transcript_background\">"),
        "同一句来自不同材料时没有按匿名组配对"
    );
    ensure!(
        assistant::group_indices(&grouped) == vec![0, 0, 1, 1],
        "预览分组与出站引用分组不一致"
    );
    let revisions = (0..5)
        .map(|index| Reference {
            text: format!("revision {index}"),
            kind: ReferenceKind::TranscriptBackground,
            source: Some("video:A:1".into()),
            revision: Some(format!("rev-{index}")),
        })
        .collect::<Vec<_>>();
    ensure!(
        assistant::validate(&revisions).is_err()
            && assistant::messages("compare revisions", &revisions, &[]).is_err(),
        "同一来源的五个修订没有按五组拒绝"
    );
    let four = revisions[..4].to_vec();
    ensure!(assistant::validate(&four).is_ok(), "四个来源修订被误拒绝");
    let old_turn = json!({
        "role": "user",
        "content": "旧会话",
        "references": revisions.clone(),
    });
    ensure!(
        assistant::messages("继续旧会话", &[], &[old_turn]).is_ok(),
        "旧会话被新的引用上限阻断"
    );
    results.push(json!({"workflow":"同一来源五个修订仅限制新草稿、旧会话仍可发送","passed":true}));
    results.push(json!({"workflow":"同一句 A/B 选句与全文按匿名组配对","passed":true}));
    results.push(
        json!({"workflow":"助手宽窄布局：900/1280/1440/1920与视频/其他模块阈值","passed":true}),
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
