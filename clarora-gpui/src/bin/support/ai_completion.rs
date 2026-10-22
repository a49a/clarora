//! Real loopback HTTP → chat completion validation → SQLite session persistence.
use anyhow::{Context, Result, ensure};
use clarora_gpui::{ai, data::Library, documents, process};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex, atomic::Ordering},
    thread,
    time::{Duration, Instant},
};

const LIMIT: usize = 4 * 1024 * 1024;

struct Fixture {
    address: String,
    stop: process::Cancel,
    cancel: Arc<Mutex<process::Cancel>>,
    requests: Arc<Mutex<Vec<Value>>>,
    worker: Option<thread::JoinHandle<Result<()>>>,
}
impl Fixture {
    fn start() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?.to_string();
        let stop = process::cancel_token();
        let stopping = stop.clone();
        let cancel = Arc::new(Mutex::new(process::cancel_token()));
        let cancellation = cancel.clone();
        let requests = Arc::new(Mutex::new(vec![]));
        let seen = requests.clone();
        let worker = thread::spawn(move || -> Result<()> {
            while !stopping.load(Ordering::SeqCst) {
                let (mut socket, _) = listener.accept()?;
                if stopping.load(Ordering::SeqCst) {
                    break;
                }
                socket.set_read_timeout(Some(Duration::from_secs(3)))?;
                socket.set_write_timeout(Some(Duration::from_secs(3)))?;
                let request = read_request(&mut socket)?;
                let model = request["model"].as_str().context("fixture model missing")?;
                let (route, case) = model.split_once('/').context("fixture case missing")?;
                let streaming = request["stream"].as_bool().unwrap_or(false);
                seen.lock()
                    .unwrap()
                    .push(json!({"model":model,"stream":streaming}));
                if route == "fallback" && streaming {
                    response(&mut socket, 405, "application/json", b"{}")?;
                    continue;
                }
                if case == "cancel-headers" || case == "cancel-body" {
                    if case == "cancel-body" {
                        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 10000\r\nConnection: close\r\n\r\n{")?;
                        socket.flush()?;
                    }
                    cancellation.lock().unwrap().store(true, Ordering::SeqCst);
                    // Hold the response open until the client drops its cancelled request.
                    let mut byte = [0];
                    let _ = socket.read(&mut byte);
                    continue;
                }
                let body = payload(route, case);
                let kind = if route == "sse" {
                    "text/event-stream"
                } else {
                    "application/json"
                };
                // Oversized responses can be rejected while the server is still sending.
                if let Err(error) = response(&mut socket, 200, kind, body.as_bytes()) {
                    if !case.starts_with("oversize-") && case != "cancel-progress" {
                        return Err(error);
                    }
                }
            }
            Ok(())
        });
        Ok(Self {
            address,
            stop,
            cancel,
            requests,
            worker: Some(worker),
        })
    }
    fn finish(&mut self) -> Result<()> {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(&self.address);
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| anyhow::anyhow!("HTTP fixture panicked"))??;
        }
        Ok(())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}
fn read_request(socket: &mut TcpStream) -> Result<Value> {
    let mut bytes = vec![];
    let mut block = [0; 8192];
    let split = loop {
        let count = socket.read(&mut block)?;
        ensure!(count > 0, "HTTP fixture request header interrupted");
        bytes.extend_from_slice(&block[..count]);
        ensure!(
            bytes.len() <= 1024 * 1024,
            "HTTP fixture request header too large"
        );
        if let Some(index) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers = String::from_utf8_lossy(&bytes[..split]);
    let length = headers
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")?
                .trim()
                .parse::<usize>()
                .ok()
        })
        .context("HTTP fixture missing Content-Length")?;
    ensure!(length <= 1024 * 1024, "HTTP fixture request too large");
    while bytes.len() < split + length {
        let count = socket.read(&mut block)?;
        ensure!(count > 0, "HTTP fixture request body interrupted");
        bytes.extend_from_slice(&block[..count]);
    }
    Ok(serde_json::from_slice(&bytes[split..split + length])?)
}
fn response(socket: &mut TcpStream, status: u16, kind: &str, body: &[u8]) -> Result<()> {
    write!(
        socket,
        "HTTP/1.1 {status} Fixture\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    socket.write_all(body)?;
    socket.flush()?;
    Ok(())
}
fn payload(route: &str, case: &str) -> String {
    let answer = match case {
        "empty" => " \r\n\t".into(),
        "oversize-answer" => "a".repeat(LIMIT + 1),
        "limit-answer" => "a".repeat(LIMIT),
        _ => "完整合成回答".into(),
    };
    let reasoning = match case {
        "oversize-reasoning" => "r".repeat(LIMIT + 1),
        "limit-reasoning" => "r".repeat(LIMIT),
        _ => "合成推理".into(),
    };
    let mut choice = json!({"finish_reason":"stop"});
    match case {
        "length" | "content_filter" => choice["finish_reason"] = json!(case),
        "unknown" => choice["finish_reason"] = json!("other-finish"),
        "missing" => {
            choice.as_object_mut().unwrap().remove("finish_reason");
        }
        "null" => choice["finish_reason"] = Value::Null,
        "numeric" => choice["finish_reason"] = json!(7),
        _ => {}
    }
    choice[if route == "sse" { "delta" } else { "message" }] =
        json!({"content":answer,"reasoning_content":reasoning});
    let mut value = json!({"choices":[choice]});
    if case == "error-with-content" {
        value["error"] = json!({"message":"synthetic provider error"});
    }
    if route == "sse" {
        let done = if case == "missing-done" {
            ""
        } else {
            "data: [DONE]\n\n"
        };
        format!("data: {value}\n\n{done}")
    } else {
        value.to_string()
    }
}
fn digest(value: &Value) -> String {
    hex::encode(Sha256::digest(value.to_string().as_bytes()))
}

pub fn run(lib: &Library, results: &mut Vec<Value>) -> Result<()> {
    let lib = Library::open(lib.root.join("ai-completion"))?;
    ai::save_chat(
        &lib,
        "existing-session",
        "existing-material",
        "seed",
        &[
            json!({"role":"user","content":"原有问题"}),
            json!({"role":"assistant","content":"原有完整回答"}),
        ],
    )?;
    let mut fixture = Fixture::start()?;
    let mut evidence = vec![];
    let cases = [
        "stop",
        "length",
        "content_filter",
        "unknown",
        "missing",
        "null",
        "numeric",
        "error-with-content",
        "empty",
        "oversize-answer",
        "oversize-reasoning",
        "cancel-progress",
    ];
    for route in ["direct", "fallback", "sse"] {
        let mut cases = cases.to_vec();
        if route == "sse" {
            cases.push("missing-done");
        } else {
            cases.extend(["cancel-headers", "cancel-body"]);
        }
        if route == "direct" {
            cases.extend(["limit-answer", "limit-reasoning"]);
        }
        for case in cases {
            let model = format!("{route}/{case}");
            lib.set_setting(
                "ai_config",
                &json!({"base_url":format!("http://{}",fixture.address),"model":model}).to_string(),
            )?;
            let before = json!(documents::records(&lib, "chat")?);
            let token = process::cancel_token();
            *fixture.cancel.lock().unwrap() = token.clone();
            let session = format!("session-{route}-{case}");
            let start = Instant::now();
            let mut progress_count = 0;
            // Same persistence gate as Action::Chat: propagate failure, recheck cancellation,
            // then append the completed answer and save the session in SQLite.
            let outcome = (|| -> Result<ai::ChatReply> {
                let reply =
                    ai::chat_stream(&lib, "合成问题", "合成材料", &[], &token, |_| {
                        progress_count += 1;
                        if case == "cancel-progress" {
                            token.store(true, Ordering::SeqCst);
                        }
                    })?;
                ensure!(!token.load(Ordering::SeqCst), "已取消");
                ai::save_chat(
                    &lib,
                    &session,
                    &model,
                    "合成问题",
                    &[
                        json!({"role":"user","content":"合成问题"}),
                        json!({"role":"assistant","content":reply.answer,"reasoning_content":reply.reasoning}),
                    ],
                )?;
                Ok(reply)
            })();
            let elapsed = start.elapsed();
            let reopened = Library::open(lib.root.clone())?;
            let after = json!(documents::records(&reopened, "chat")?);
            let expected_success = ["stop", "limit-answer", "limit-reasoning"].contains(&case);
            let error = outcome.as_ref().err().map(|e| format!("{e:#}"));
            let requests = fixture
                .requests
                .lock()
                .unwrap()
                .iter()
                .filter(|r| r["model"] == model)
                .cloned()
                .collect::<Vec<_>>();
            ensure!(
                requests.len() == if route == "fallback" { 2 } else { 1 },
                "{model}: unexpected request count"
            );
            ensure!(
                requests[0]["stream"] == true
                    && (requests.len() == 1 || requests[1]["stream"] == false),
                "{model}: incorrect fallback stream flags"
            );
            let row = json!({"case":model,"expected_success":expected_success,"returned_success":outcome.is_ok(),"error":error,"milliseconds":elapsed.as_millis(),"progress_events":progress_count,"records_before":before.as_array().unwrap().len(),"records_after":after.as_array().unwrap().len(),"before_sha256":digest(&before),"after_sha256":digest(&after),"requests":requests});
            evidence.push(row);
            fs::write(
                lib.root.join("ai-completion-report.json"),
                serde_json::to_vec_pretty(&evidence)?,
            )?;
            if expected_success {
                let reply =
                    outcome.with_context(|| format!("{model}: valid completion rejected"))?;
                let stored = after
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|r| r["id"] == session)
                    .context("completed session was not persisted")?;
                ensure!(
                    stored["messages"][1]["content"] == reply.answer
                        && stored["messages"][1]["reasoning_content"] == reply.reasoning,
                    "{model}: saved reply differs after reopening SQLite"
                );
            } else {
                ensure!(
                    outcome.is_err(),
                    "{model}: incomplete/error response returned success"
                );
                ensure!(before == after, "{model}: failure changed saved sessions");
                if case.starts_with("cancel-") {
                    ensure!(
                        error.as_deref().unwrap_or("").contains("已取消"),
                        "{model}: cancellation not reported: {error:?}"
                    );
                    ensure!(
                        elapsed < Duration::from_millis(1500),
                        "{model}: cancellation took {elapsed:?}"
                    );
                }
            }
        }
    }
    let before_requests = fixture.requests.lock().unwrap().len();
    let before = documents::records(&lib, "chat")?;
    let token = process::cancel_token();
    token.store(true, Ordering::SeqCst);
    let cancelled = ai::chat_stream(&lib, "合成问题", "合成材料", &[], &token, |_| {});
    ensure!(
        cancelled.is_err()
            && fixture.requests.lock().unwrap().len() == before_requests
            && before == documents::records(&lib, "chat")?,
        "pre-cancelled request sent HTTP or changed SQLite"
    );
    evidence.push(json!({"case":"pre-cancelled","returned_success":false,"requests":0,"database_unchanged":true}));
    fixture.finish()?;
    fs::write(
        lib.root.join("ai-completion-report.json"),
        serde_json::to_vec_pretty(&evidence)?,
    )?;
    results.push(json!({"workflow":"AI JSON/405/SSE 完成校验 → 错误/截断/超长/取消不写会话 → SQLite重开", "passed":true,"cases":evidence.len(),"report":lib.root.join("ai-completion-report.json")}));
    Ok(())
}
