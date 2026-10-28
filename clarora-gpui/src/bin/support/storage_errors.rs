//! R2/S3 error diagnostics through real signed HTTP requests and bounded bodies.
//! Covers service codes, untrusted XML, bounded response reads, cancellation and successful lists.
use anyhow::{Context, Result, ensure};
use clarora_gpui::{backup::Storage, data::Library, process::Cancel};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

const PRIVATE_MARKERS: [&str; 5] = [
    "server-private-message-marker",
    "server-private-request-marker",
    "server-private-host-marker",
    "fixture-access",
    "fixture-secret",
];

#[derive(Clone, Copy)]
enum BodyMode {
    Complete,
    HoldAfterPrefix,
    HoldBeyondReadTimeout,
    CancelAfterPrefix,
}

struct Fixture {
    address: SocketAddr,
    stop: mpsc::Sender<()>,
    handle: Option<thread::JoinHandle<Result<Value>>>,
}
impl Fixture {
    fn new(status: &'static str, body: Vec<u8>, mode: BodyMode, cancel: Cancel) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let (stop, stopped) = mpsc::channel();
        let handle = thread::spawn(move || -> Result<Value> {
            let (mut stream, _) = listener.accept()?;
            stream.set_read_timeout(Some(Duration::from_secs(2)))?;
            stream.set_write_timeout(Some(Duration::from_secs(2)))?;
            let mut request = vec![];
            let mut chunk = [0; 2048];
            loop {
                let size = stream.read(&mut chunk)?;
                ensure!(
                    size > 0,
                    "storage error fixture request ended before headers"
                );
                request.extend_from_slice(&chunk[..size]);
                ensure!(
                    request.len() <= 64 * 1024,
                    "fixture request headers too large"
                );
                if request.windows(4).any(|part| part == b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8(request)?;
            let line = request.lines().next().context("fixture request line")?;
            ensure!(line.starts_with("GET /fixture-bucket/"), "not a bucket GET");
            ensure!(line.contains("list-type=2"), "not ListObjectsV2");
            ensure!(
                request.contains("Credential=fixture-access/"),
                "missing SigV4 access id"
            );
            ensure!(
                request.contains("AWS4-HMAC-SHA256"),
                "missing SigV4 algorithm"
            );
            let length = match mode {
                BodyMode::Complete => body.len(),
                _ => body.len() + 1024 * 1024,
            };
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/xml\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n"
            )?;
            // A bounded reader may close while this oversized prefix is in flight.
            let written = stream.write_all(&body).and_then(|_| stream.flush());
            if matches!(mode, BodyMode::Complete) {
                written?;
            }
            if matches!(mode, BodyMode::CancelAfterPrefix) {
                let _ = stopped.recv_timeout(Duration::from_millis(100));
                cancel.store(true, Ordering::SeqCst);
            }
            if !matches!(mode, BodyMode::Complete) {
                let hold = if matches!(mode, BodyMode::HoldBeyondReadTimeout) {
                    8
                } else {
                    2
                };
                let _ = stopped.recv_timeout(Duration::from_secs(hold));
            }
            // Persist only a method/path summary; never Authorization or other headers.
            Ok(
                json!({"method":"GET","list_objects_v2":true,"signed_request":true,
                "response_status":status,"prefix_bytes":body.len(),"declared_bytes":length}),
            )
        });
        Ok(Self {
            address,
            stop,
            handle: Some(handle),
        })
    }
    fn finish(mut self) -> Result<Value> {
        let _ = self.stop.send(());
        self.handle
            .take()
            .unwrap()
            .join()
            .map_err(|_| anyhow::anyhow!("storage error fixture panicked"))?
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        // Release accept if client construction failed before connecting.
        let _ = TcpStream::connect_timeout(&self.address, Duration::from_millis(100));
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn token() -> Cancel {
    Arc::new(AtomicBool::new(false))
}
fn error_xml(code: &str) -> Vec<u8> {
    format!("<?xml version=\"1.0\"?><Error><Code>{code}</Code><Message>{}</Message><RequestId>{}</RequestId><HostId>{}</HostId></Error>",
        PRIVATE_MARKERS[0], PRIVATE_MARKERS[1], PRIVATE_MARKERS[2]).into_bytes()
}
fn request(body: Vec<u8>, mode: BodyMode) -> Result<(String, Value, Duration)> {
    let cancel = token();
    let fixture = Fixture::new("403 Forbidden", body, mode, cancel.clone())?;
    let storage = Storage::loopback_fixture(fixture.address)?;
    let started = Instant::now();
    let message = storage
        .list_cancellable(&cancel)
        .expect_err("403 was accepted")
        .to_string();
    let elapsed = started.elapsed();
    let seen = fixture.finish()?;
    for marker in PRIVATE_MARKERS {
        ensure!(
            !message.contains(marker),
            "storage response leaked {marker}: {message}"
        );
    }
    ensure!(
        !message.contains("<Error") && !message.contains("<Message"),
        "raw XML leaked"
    );
    Ok((message, seen, elapsed))
}

pub fn run(parent: &Library, results: &mut Vec<Value>) -> Result<()> {
    let root = parent.root.join("storage-errors");
    fs::create_dir_all(&root)?;
    let mut evidence = vec![];
    for (code, expected) in [
        ("AccessDenied", vec!["访问被拒绝", "Bucket", "对象读写"]),
        (
            "SignatureDoesNotMatch",
            vec!["签名校验失败", "Secret Access Key", "Endpoint"],
        ),
        ("InvalidAccessKeyId", vec!["Access Key ID 无效"]),
        ("ExpiredToken", vec!["临时凭证已过期"]),
        ("InvalidToken", vec!["Session Token 无效"]),
        ("RequestTimeTooSkewed", vec!["本机日期、时间和时区"]),
        ("RequestExpired", vec!["本机日期、时间和时区"]),
        ("ExpiredRequest", vec!["本机日期、时间和时区"]),
        ("NotEntitled", vec!["账户尚未启用", "R2"]),
        ("ObjectLockedByBucketPolicy", vec!["对象锁定规则", "修改"]),
        ("R2ExampleError", vec![]),
    ] {
        let (message, seen, elapsed) = request(error_xml(code), BodyMode::Complete)?;
        ensure!(
            message.contains("HTTP 403") && message.contains(code),
            "missing status/code for {code}: {message}"
        );
        for hint in expected {
            ensure!(
                message.contains(hint),
                "missing hint {hint} for {code}: {message}"
            );
        }
        evidence.push(json!({"case":code,"passed":true,"message":message,"elapsed_ms":elapsed.as_millis(),"request":seen}));
    }

    let valid_64 = "R".repeat(64);
    let (message, seen, _) = request(error_xml(&valid_64), BodyMode::Complete)?;
    ensure!(
        message.contains(&valid_64),
        "64-character safe Code was lost"
    );
    evidence.push(
        json!({"case":"safe-code-64-characters","passed":true,"message":message,"request":seen}),
    );

    let mut invalid = vec![
        ("empty", vec![]),
        (
            "html",
            b"<html><body><Code>AccessDenied</Code>server-private-message-marker</body></html>"
                .to_vec(),
        ),
        (
            "non-error-root",
            b"<Response><Code>AccessDenied</Code></Response>".to_vec(),
        ),
        (
            "missing-code",
            b"<Error><Message>server-private-message-marker</Message></Error>".to_vec(),
        ),
        (
            "unclosed-error",
            b"<Error><Code>AccessDenied</Code>".to_vec(),
        ),
        (
            "mismatched-end-tag",
            b"<Error><Code>AccessDenied</Wrong></Error>".to_vec(),
        ),
        (
            "dtd-entity",
            b"<!DOCTYPE Error [<!ENTITY code 'AccessDenied'>]><Error><Code>&code;</Code></Error>"
                .to_vec(),
        ),
        (
            "non-utf8",
            b"<Error><Code>AccessDenied</Code><Message>\xff</Message></Error>".to_vec(),
        ),
        ("long-code", error_xml(&"R".repeat(65))),
        ("control-code", error_xml("Access\u{1b}Denied")),
        (
            "code-punctuation",
            error_xml("AccessDenied?secret=server-private-message-marker"),
        ),
        ("credential-code", error_xml("fixture-secret")),
        ("access-key-code", error_xml("fixture-access")),
    ];
    // A namespace is ordinary S3 XML and should not prevent reading a safe Code.
    let namespaced = b"<Error xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Code>AccessDenied</Code></Error>".to_vec();
    let (message, seen, _) = request(namespaced, BodyMode::Complete)?;
    ensure!(
        message.contains("AccessDenied") && message.contains("对象读写"),
        "namespace lost safe Code: {message}"
    );
    evidence
        .push(json!({"case":"namespaced-error","passed":true,"message":message,"request":seen}));

    for (name, body) in invalid.drain(..) {
        let (message, seen, elapsed) = request(body, BodyMode::Complete)?;
        ensure!(
            message.contains("HTTP 403"),
            "lost HTTP status for {name}: {message}"
        );
        ensure!(
            !message.contains("AccessDenied") && !message.contains(&"R".repeat(64)),
            "untrusted Code shown for {name}: {message}"
        );
        evidence.push(json!({"case":name,"passed":true,"message":message,"elapsed_ms":elapsed.as_millis(),"request":seen}));
    }

    let mut oversized = b"<Error>".to_vec();
    oversized.extend(std::iter::repeat_n(b' ', 32 * 1024));
    oversized.extend_from_slice(b"<Code>AccessDenied</Code></Error>");
    let (message, seen, elapsed) = request(oversized, BodyMode::HoldAfterPrefix)?;
    ensure!(
        elapsed < Duration::from_secs(1),
        "error body reader waited for the whole oversized response: {elapsed:?}"
    );
    ensure!(
        message.contains("HTTP 403") && !message.contains("AccessDenied"),
        "oversized response escaped the 16KiB window: {message}"
    );
    evidence.push(json!({"case":"bounded-error-body","passed":true,"message":message,"elapsed_ms":elapsed.as_millis(),"request":seen}));

    let (message, seen, elapsed) = request(
        b"<Error><Code>Access".to_vec(),
        BodyMode::HoldBeyondReadTimeout,
    )?;
    ensure!(
        elapsed >= Duration::from_secs(2) && elapsed < Duration::from_secs(6),
        "error-body read deadline was not bounded: {elapsed:?}"
    );
    ensure!(
        message.contains("HTTP 403")
            && !message.contains("AccessDenied")
            && !message.contains("已取消"),
        "stalled error body lost HTTP fallback: {message}"
    );
    evidence.push(json!({"case":"stalled-error-body-deadline","passed":true,"message":message,"elapsed_ms":elapsed.as_millis(),"request":seen}));

    let (message, seen, elapsed) =
        request(b"<Error><Code>Access".to_vec(), BodyMode::CancelAfterPrefix)?;
    ensure!(
        message.contains("已取消"),
        "error-body cancellation was swallowed: {message}"
    );
    ensure!(
        elapsed < Duration::from_secs(1),
        "error-body cancellation was slow: {elapsed:?}"
    );
    evidence.push(json!({"case":"cancel-error-body-read","passed":true,"message":message,"elapsed_ms":elapsed.as_millis(),"request":seen}));

    let cancel = token();
    let fixture = Fixture::new(
        "200 OK",
        b"<ListBucketResult><IsTruncated>false</IsTruncated></ListBucketResult>".to_vec(),
        BodyMode::Complete,
        cancel.clone(),
    )?;
    let keys = Storage::loopback_fixture(fixture.address)?.list_cancellable(&cancel)?;
    ensure!(keys.is_empty(), "successful empty list changed");
    evidence
        .push(json!({"case":"successful-empty-list","passed":true,"request":fixture.finish()?}));

    let report = root.join("storage-error-evidence.json");
    fs::write(&report, serde_json::to_vec_pretty(&evidence)?)?;
    results.push(json!({"workflow":"对象存储错误：R2/S3 中文分类、安全 Code、响应体隐藏、有界读取与取消","passed":true,"cases":evidence.len(),"evidence":report}));
    Ok(())
}
