//! HTTP workflow verification against a real loopback server, including failed connections.
use anyhow::{Result, ensure};
use clarora_gpui::{ai, data::Library, process};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub fn run(
    lib: &Library,
    media: &std::path::Path,
    image: &std::path::Path,
    results: &mut Vec<Value>,
) -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = format!("http://{}", listener.local_addr()?);
    listener.set_nonblocking(true)?;
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let seen = Arc::new(Mutex::new(vec![]));
    let requests = seen.clone();
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(120);
        let mut children = vec![];
        while !flag.load(Ordering::SeqCst) && Instant::now() < deadline {
            let (mut socket, _) = match listener.accept() {
                Ok(s) => s,
                Err(_) => {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }
            };
            let requests = requests.clone();
            children.push(thread::spawn(move || {
              let result=(|| -> Result<()> {
                socket.set_nonblocking(false)?;
                socket.set_read_timeout(Some(Duration::from_secs(5)))?;
                let mut bytes = vec![];
                let mut block = [0u8;4096];
                let split = loop {
                    let n=socket.read(&mut block)?; ensure!(n>0,"请求头中断"); bytes.extend_from_slice(&block[..n]);
                    if let Some(i)=bytes.windows(4).position(|w|w==b"\r\n\r\n") {break i+4;}
                };
                let header=String::from_utf8_lossy(&bytes[..split]);
                let length=header.lines().find_map(|s|s.to_lowercase().strip_prefix("content-length:").and_then(|v|v.trim().parse::<usize>().ok())).unwrap_or(0);
                while bytes.len()<split+length {let n=socket.read(&mut block)?;ensure!(n>0,"请求体中断");bytes.extend_from_slice(&block[..n]);}
                let raw=String::from_utf8_lossy(&bytes[split..split+length]);
                let body:Value=serde_json::from_slice(&bytes[split..split+length]).unwrap_or_else(|_| {
                    let model=raw.split("name=\"model\"").nth(1).and_then(|s|s.split("\r\n\r\n").nth(1)).and_then(|s|s.split("\r\n").next()).unwrap_or("");
                    json!({"model":model,"stream":false,"messages":[]})
                });
                let model=body["model"].as_str().unwrap_or("");
                let streaming=body["stream"].as_bool().unwrap_or(false);
                let history_clean=body["messages"].as_array().unwrap().iter().all(|m|m.as_object().is_some_and(|m|m.keys().all(|k|["role","content"].contains(&k.as_str()))));
                requests.lock().unwrap().push(json!({"model":model,"stream":streaming,"history_clean":history_clean}));
                let (status,kind,payload)=match model {
                    "fallback" if streaming => (405,"application/json","{}".to_string()),
                    "fallback" => (200,"application/json",json!({"choices":[{"message":{"content":"plain fallback"},"finish_reason":"stop"}]}).to_string()),
                    "denied" => (401,"application/json","{}".to_string()),
                    "keypoints" => (200,"application/json",json!({"choices":[{"message":{"content":json!([{"text":"Hello","translation":"你好"},{"text":"absent fixture","translation":"应过滤"}]).to_string()},"finish_reason":"stop"}]}).to_string()),
                    "truncated" => (200,"text/event-stream","data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n".to_string()),
                    "error" => (200,"text/event-stream","data: {\"error\":{\"message\":\"fixture\"}}\n\ndata: [DONE]\n\n".to_string()),
                    "length" => (200,"text/event-stream","data: {\"choices\":[{\"delta\":{\"content\":\"partial\"},\"finish_reason\":\"length\"}]}\n\ndata: [DONE]\n\n".to_string()),
                    "reasoning-only" => (200,"text/event-stream","data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"thinking\"}}]}\n\ndata: [DONE]\n\n".to_string()),
                    _ => (200,"text/event-stream",format!("data: {{\r\ndata: \"choices\":[{{\"delta\":{{\"reasoning_content\":\"分析材料\"}}}}]}}\r\n\r\ndata: {}\r\n\r\ndata: [DONE]\r\n\r\n",json!({"choices":[{"delta":{"content":"你好世界"},"finish_reason":"stop"}]}))),
                };
                if model=="stall-headers" {thread::sleep(Duration::from_secs(2));}
                let length=if model=="stall-body" {10000}else{payload.len()};
                if write!(socket,"HTTP/1.1 {status} Fixture\r\nContent-Type: {kind}\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n").is_err(){return Ok(());}
                if model=="stall-body" {socket.flush()?;thread::sleep(Duration::from_secs(2));return Ok(());}
                for chunk in payload.as_bytes().chunks(3) {
                    if socket.write_all(chunk).is_err(){break;}
                    socket.flush()?;
                    thread::sleep(Duration::from_millis(1));
                }
                Ok(())
              })();
              if let Err(error)=&result {eprintln!("HTTP fixture failed: {error:#}");}
              result
            }));
        }
        for child in children {
            let _ = child.join();
        }
    });
    let configure = |model: &str| {
        lib.set_setting(
            "ai_config",
            &json!({"base_url":address,"model":model,"vision_model":model,"asr_base_url":address,"asr_model":model,"engine":"compatible"}).to_string(),
        )
    };
    let cancel = process::cancel_token();
    configure("reasoning")?;
    let reply = ai::chat_stream(
        lib,
        "说明",
        "材料",
        &[json!({"role":"assistant","content":"前文","reasoning_content":"不应传回"})],
        &cancel,
        |_| {},
    )?;
    ensure!(
        reply.answer == "你好世界" && reply.reasoning == "分析材料",
        "中文分块/CRLF/多行data/推理丢失"
    );
    configure("fallback")?;
    ensure!(
        ai::chat(lib, "说明", "材料", &[], &cancel, |_| {})? == "plain fallback",
        "非流式回退失败"
    );
    configure("keypoints")?;
    let points = ai::key_points(
        lib,
        &[clarora_gpui::subtitles::Cue {
            start: 0.,
            end: 2.,
            text: "Hello world".into(),
        }],
        &cancel,
    )?;
    ensure!(
        points.len() == 1 && points[0].text == "Hello",
        "原文重点定位/过滤失败"
    );
    for model in ["denied", "truncated", "error", "length", "reasoning-only"] {
        configure(model)?;
        ensure!(
            ai::chat(lib, "说明", "材料", &[], &cancel, |_| {}).is_err(),
            "错误响应被当作成功：{model}"
        );
    }
    let mut timings = vec![];
    let before = lib.list("ai_records")?.len();
    for kind in ["chat", "ocr", "asr"] {
        for model in ["stall-headers", "stall-body"] {
            configure(model)?;
            let token = process::cancel_token();
            let flag = token.clone();
            let worker = thread::spawn(move || {
                thread::sleep(Duration::from_millis(150));
                flag.store(true, Ordering::SeqCst);
            });
            let start = Instant::now();
            let result = match kind {
                "ocr" => ai::ocr(lib, image, &token).map(|_| ()),
                "asr" => ai::transcribe(lib, media, &token).map(|_| ()),
                _ => ai::chat(lib, "说明", "材料", &[], &token, |_| {}).map(|_| ()),
            };
            let elapsed = start.elapsed();
            worker.join().unwrap();
            let error = result.err().map(|e| format!("{e:#}")).unwrap_or_default();
            ensure!(
                error.contains("已取消")
                    && elapsed >= Duration::from_millis(100)
                    && elapsed < Duration::from_millis(1500),
                "HTTP 等待无法及时取消：{kind}-{model} {elapsed:?} {error}"
            );
            timings.push(
                json!({"phase":format!("{kind}-{model}"),"milliseconds":elapsed.as_millis()}),
            );
        }
    }
    ensure!(
        lib.list("ai_records")?.len() == before,
        "取消后写入了成功记录"
    );
    stop.store(true, Ordering::SeqCst);
    server.join().unwrap();
    let requests = seen.lock().unwrap();
    ensure!(
        requests.iter().all(|r| r["history_clean"] == true),
        "推理字段泄漏到请求 history"
    );
    ensure!(
        requests.iter().filter(|r| r["model"] == "fallback").count() == 2
            && requests.iter().filter(|r| r["model"] == "denied").count() == 1,
        "错误的回退或重试次数"
    );
    results.push(json!({"workflow":"真实 HTTP 中文分块推理 → 405回退 → 401/断流/错误/截断拒绝 → 聊天/OCR/ASR响应等待取消","passed":true,"cancellation":timings,"requests":*requests}));
    Ok(())
}
