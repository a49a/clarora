//! Upload progress through real streamed HTTP, SQLite, files and restore.
use anyhow::{Context, Result, ensure};
use clarora_gpui::{
    backup::{Storage, UploadProgress, UploadStage},
    data::{self, Library},
    process::Cancel,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const LARGE_BYTES: u64 = 16 * 1024 * 1024;
const SMALL_BYTES: u64 = 128 * 1024;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Normal,
    RejectAttachment,
    RejectSnapshot,
}
struct Fixture {
    address: SocketAddr,
    objects: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    requests: Arc<Mutex<Vec<Value>>>,
    received: Arc<AtomicU64>,
    acknowledged_files: Arc<AtomicU64>,
    snapshot_response_started: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}
fn reply(stream: &mut TcpStream, status: &str, bytes: &[u8]) -> Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        bytes.len()
    )?;
    stream.write_all(bytes)?;
    stream.flush()?;
    Ok(())
}
impl Fixture {
    fn new(mode: Mode) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let objects = Arc::new(Mutex::new(BTreeMap::<String, Vec<u8>>::new()));
        let requests = Arc::new(Mutex::new(vec![]));
        let received = Arc::new(AtomicU64::new(0));
        let acknowledged_files = Arc::new(AtomicU64::new(0));
        let snapshot_response_started = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let (store, seen, count, confirmed, committed, stopped) = (
            objects.clone(),
            requests.clone(),
            received.clone(),
            acknowledged_files.clone(),
            snapshot_response_started.clone(),
            stop.clone(),
        );
        let handle = thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                };
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                let serve = (|| -> Result<()> {
                    let mut incoming = vec![];
                    let mut chunk = [0_u8; 32 * 1024];
                    let split = loop {
                        let n = stream.read(&mut chunk)?;
                        ensure!(n > 0, "request ended before headers");
                        incoming.extend_from_slice(&chunk[..n]);
                        if let Some(i) = incoming.windows(4).position(|w| w == b"\r\n\r\n") {
                            break i + 4;
                        }
                        ensure!(incoming.len() <= 64 * 1024, "request headers too large");
                    };
                    let header = String::from_utf8_lossy(&incoming[..split]);
                    let mut line = header
                        .lines()
                        .next()
                        .context("request line")?
                        .split_whitespace();
                    let method = line.next().context("method")?.to_string();
                    let target = line.next().context("target")?.to_string();
                    let key = target
                        .split('?')
                        .next()
                        .unwrap()
                        .strip_prefix("/fixture-bucket/")
                        .context("bucket path")?
                        .to_string();
                    let length = header
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())
                                .flatten()
                        })
                        .unwrap_or(0);
                    ensure!(
                        header.contains("AWS4-HMAC-SHA256")
                            && header.contains("Credential=fixture-access/"),
                        "unsigned storage request"
                    );
                    let attachment = method == "PUT" && key.contains("/media/");
                    let snapshot = method == "PUT" && key.contains("/snapshots/");
                    let index = {
                        let mut rows = seen.lock().unwrap();
                        let i = rows.len();
                        rows.push(json!({"method":method,"key":key,"declared_bytes":length,"received_bytes":0,"response_started":false}));
                        i
                    };
                    let mut body = incoming[split..].to_vec();
                    if attachment {
                        count.fetch_add(body.len() as u64, Ordering::SeqCst);
                    }
                    while body.len() < length {
                        let n = stream.read(&mut chunk)?;
                        ensure!(n > 0, "request body interrupted");
                        body.extend_from_slice(&chunk[..n]);
                        if attachment {
                            count.fetch_add(n as u64, Ordering::SeqCst);
                            // Slow actual TCP consumption so the sender experiences backpressure.
                            thread::sleep(Duration::from_millis(3));
                        }
                        if stopped.load(Ordering::SeqCst) {
                            return Ok(());
                        }
                    }
                    seen.lock().unwrap()[index]["received_bytes"] = json!(body.len());
                    if method == "PUT" {
                        thread::sleep(Duration::from_millis(150));
                        if (attachment && mode == Mode::RejectAttachment)
                            || (snapshot && mode == Mode::RejectSnapshot)
                        {
                            seen.lock().unwrap()[index]["response_started"] = json!(true);
                            return reply(
                                &mut stream,
                                "403 Forbidden",
                                b"<Error><Code>AccessDenied</Code></Error>",
                            );
                        }
                        store.lock().unwrap().insert(key, body);
                        if attachment {
                            confirmed.fetch_add(1, Ordering::SeqCst);
                        }
                        if snapshot {
                            committed.store(true, Ordering::SeqCst);
                        }
                        seen.lock().unwrap()[index]["response_started"] = json!(true);
                        reply(&mut stream, "200 OK", b"")?;
                    } else if target.contains("list-type=2") {
                        let contents = store
                            .lock()
                            .unwrap()
                            .keys()
                            .filter(|key| key.contains("/snapshots/"))
                            .map(|key| format!("<Contents><Key>{key}</Key></Contents>"))
                            .collect::<String>();
                        reply(&mut stream, "200 OK", format!("<ListBucketResult>{contents}<IsTruncated>false</IsTruncated></ListBucketResult>").as_bytes())?;
                    } else {
                        let bytes = store
                            .lock()
                            .unwrap()
                            .get(&key)
                            .context("missing stored object")?
                            .clone();
                        reply(&mut stream, "200 OK", &bytes)?;
                    }
                    Ok(())
                })();
                if let Err(error) = serve {
                    // An intentionally cancelled upload can close its socket mid-body.
                    seen.lock()
                        .unwrap()
                        .push(json!({"request_aborted":error.to_string()}));
                }
            }
        });
        Ok(Self {
            address,
            objects,
            requests,
            received,
            acknowledged_files,
            snapshot_response_started,
            stop,
            handle: Some(handle),
        })
    }
    fn storage(&self) -> Result<Storage> {
        Storage::loopback_fixture(self.address)
    }
    fn evidence(&self) -> Value {
        json!({"requests":self.requests.lock().unwrap().clone(),"stored_keys":self.objects.lock().unwrap().keys().cloned().collect::<Vec<_>>(),"attachment_bytes_received":self.received.load(Ordering::SeqCst)})
    }
    fn has_snapshot(&self) -> bool {
        self.objects
            .lock()
            .unwrap()
            .keys()
            .any(|key| key.contains("/snapshots/"))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
fn token() -> Cancel {
    Arc::new(AtomicBool::new(false))
}
fn source_library(root: &Path, attachments: bool) -> Result<Library> {
    let lib = Library::open(root.to_path_buf())?;
    lib.db()?.execute(
        "INSERT INTO words(word,meaning) VALUES('progress-fixture','备份进度')",
        [],
    )?;
    if attachments {
        for (name, size) in [("large.bin", LARGE_BYTES), ("small.bin", SMALL_BYTES)] {
            let path = lib.root.join("media").join(name);
            let mut file = fs::File::create(&path)?;
            file.write_all(format!("real progress attachment {name}").as_bytes())?;
            file.set_len(size)?;
            lib.put("clip_cards", json!({"id":name,"en_text":name,"zh_text":"附件","audio_uri":path.to_string_lossy(),"start_ms":0,"end_ms":1000,"created_at":data::now()}))?;
        }
    }
    Ok(lib)
}
#[derive(Clone, Copy, PartialEq)]
enum CancelAt {
    Never,
    DuringUpload,
    BeforePublishing,
}
fn upload(
    fixture: &Fixture,
    lib: &Library,
    cancel: &Cancel,
    cancel_at: CancelAt,
) -> Result<(Result<String>, Vec<Value>)> {
    let events = Arc::new(Mutex::new(vec![]));
    let (rows, flag, received, confirmed, acknowledged) = (
        events.clone(),
        cancel.clone(),
        fixture.received.clone(),
        fixture.acknowledged_files.clone(),
        fixture.snapshot_response_started.clone(),
    );
    let started = Instant::now();
    let result =
        fixture
            .storage()?
            .upload_with_progress(lib, cancel, move |progress: UploadProgress| {
                let mut row = serde_json::to_value(&progress).unwrap();
                row["label"] = json!(progress.label());
                row["percent"] = json!(progress.percent());
                row["elapsed_ms"] = json!(started.elapsed().as_millis());
                row["server_attachment_bytes"] = json!(received.load(Ordering::SeqCst));
                row["server_files_acknowledged"] = json!(confirmed.load(Ordering::SeqCst));
                row["snapshot_response_started"] = json!(acknowledged.load(Ordering::SeqCst));
                rows.lock().unwrap().push(row);
                if (cancel_at == CancelAt::DuringUpload
                    && progress.stage == UploadStage::Uploading
                    && progress.bytes_sent > 0)
                    || (cancel_at == CancelAt::BeforePublishing
                        && progress.stage == UploadStage::Uploading
                        && progress.files_total > 0
                        && progress.files_done == progress.files_total)
                {
                    flag.store(true, Ordering::SeqCst);
                }
            });
    let events = events.lock().unwrap().clone();
    Ok((result, events))
}
fn check_events(events: &[Value], completed: bool) -> Result<()> {
    ensure!(
        !events.is_empty() && events[0]["stage"] == "preparing",
        "upload omitted preparing progress"
    );
    let (mut sent, mut done) = (0, 0);
    for event in events {
        let next_sent = event["bytes_sent"].as_u64().context("bytes_sent")?;
        let total = event["bytes_total"].as_u64().context("bytes_total")?;
        let next_done = event["files_done"].as_u64().context("files_done")?;
        let files = event["files_total"].as_u64().context("files_total")?;
        ensure!(
            next_sent >= sent && next_sent <= total && next_done >= done && next_done <= files,
            "upload progress regressed or overflowed: {event}"
        );
        ensure!(
            next_done
                <= event["server_files_acknowledged"]
                    .as_u64()
                    .context("acknowledged files")?,
            "file counted complete before HTTP success: {event}"
        );
        sent = next_sent;
        done = next_done;
        if event["stage"] != "completed" {
            ensure!(
                event["percent"].as_f64().is_none_or(|p| p < 100.),
                "upload reported 100% before completion"
            );
        } else {
            ensure!(
                event["snapshot_response_started"] == true && event["percent"] == 100.0,
                "upload completed before snapshot acknowledgement"
            );
        }
        let value = serde_json::to_string(event)?;
        for private in [
            "private-workspace-marker",
            "fixture-access",
            "fixture-secret",
        ] {
            ensure!(!value.contains(private), "progress leaked private context");
        }
    }
    ensure!(
        events.iter().any(|event| event["stage"] == "completed") == completed,
        "unexpected completion stage"
    );
    if completed {
        ensure!(
            events.last().unwrap()["stage"] == "completed",
            "completion was not final progress"
        );
    }
    Ok(())
}
pub fn run(parent: &Library, results: &mut Vec<Value>) -> Result<()> {
    let root = parent.root.join("backup-progress");
    fs::create_dir_all(&root)?;
    let source = source_library(&root.join("private-workspace-marker/source"), true)?;
    let mut evidence = vec![];

    let fixture = Fixture::new(Mode::Normal)?;
    let (result, events) = upload(&fixture, &source, &token(), CancelAt::Never)?;
    let key = result?;
    check_events(&events, true)?;
    let large_start = events
        .iter()
        .find(|event| event["stage"] == "uploading" && event["current_name"] == "large.bin")
        .context("large-file progress missing")?;
    let sent_before_large = large_start["bytes_sent"]
        .as_u64()
        .context("large-file starting bytes")?;
    let large_file_index = large_start["files_done"]
        .as_u64()
        .context("large-file index")?;
    let increments = events
        .iter()
        .filter(|event| {
            event["stage"] == "uploading"
                && event["current_name"] == "large.bin"
                && event["files_done"] == large_file_index
                && event["bytes_sent"].as_u64().is_some_and(|sent| {
                    sent > sent_before_large && sent < sent_before_large + LARGE_BYTES
                })
        })
        .count();
    ensure!(
        increments >= 2,
        "large attachment had no repeated in-flight byte updates"
    );
    let final_event = events.last().unwrap();
    ensure!(
        final_event["bytes_sent"] == LARGE_BYTES + SMALL_BYTES
            && final_event["bytes_total"] == LARGE_BYTES + SMALL_BYTES
            && final_event["files_done"] == 2
            && final_event["files_total"] == 2,
        "final byte/file totals differ from source"
    );
    let storage = fixture.storage()?;
    ensure!(
        storage.list()? == vec![key.clone()],
        "completed upload missing from list"
    );
    let restored = Library::open(root.join("restored"))?;
    ensure!(
        storage.restore(&restored, &key, &token())? == 3,
        "roundtrip row count differs"
    );
    for row in restored.list("clip_cards")? {
        ensure!(
            fs::read(data::text(&row, "audio_uri"))?
                == fs::read(source.root.join("media").join(data::text(&row, "en_text")))?,
            "roundtrip attachment changed"
        );
    }
    evidence.push(json!({"case":"streamed-progress-and-roundtrip","passed":true,"intermediate_updates":increments,"events":events,"http":fixture.evidence()}));
    drop(fixture);

    for (name, cancel_at, mode) in [
        ("cancel-during-upload", CancelAt::DuringUpload, Mode::Normal),
        (
            "cancel-before-publishing",
            CancelAt::BeforePublishing,
            Mode::Normal,
        ),
        (
            "attachment-http-failure",
            CancelAt::Never,
            Mode::RejectAttachment,
        ),
    ] {
        let fixture = Fixture::new(mode)?;
        let started = Instant::now();
        let (result, events) = upload(&fixture, &source, &token(), cancel_at)?;
        let error = result
            .expect_err("failed/cancelled upload unexpectedly succeeded")
            .to_string();
        check_events(&events, false)?;
        ensure!(
            !fixture.has_snapshot(),
            "unsuccessful upload published a snapshot"
        );
        if cancel_at != CancelAt::Never {
            ensure!(
                error.contains("已取消"),
                "upload cancellation was hidden: {error}"
            );
            let last_ms = events.last().unwrap()["elapsed_ms"]
                .as_u64()
                .context("cancellation event time")?;
            ensure!(
                started
                    .elapsed()
                    .as_millis()
                    .saturating_sub(last_ms as u128)
                    < 1000,
                "cancellation did not return promptly"
            );
        } else {
            ensure!(
                error.contains("AccessDenied"),
                "attachment error classification lost: {error}"
            );
        }
        evidence.push(json!({"case":name,"passed":true,"error":error,"elapsed_ms":started.elapsed().as_millis(),"events":events,"http":fixture.evidence()}));
    }

    let empty = source_library(&root.join("zero-attachments"), false)?;
    let fixture = Fixture::new(Mode::RejectSnapshot)?;
    let (result, events) = upload(&fixture, &empty, &token(), CancelAt::Never)?;
    let error = format!(
        "{:#}",
        result.expect_err("snapshot HTTP failure was accepted")
    );
    check_events(&events, false)?;
    ensure!(
        error.contains("发布结果未确认") && error.contains("AccessDenied"),
        "snapshot failure lost commit uncertainty or cause: {error}"
    );
    ensure!(!fixture.has_snapshot(), "rejected snapshot was published");
    evidence.push(json!({"case":"snapshot-http-failure","passed":true,"error":error,"events":events,"http":fixture.evidence()}));
    drop(fixture);

    let fixture = Fixture::new(Mode::Normal)?;
    let (result, events) = upload(&fixture, &empty, &token(), CancelAt::Never)?;
    let key = result?;
    check_events(&events, true)?;
    ensure!(
        events.iter().any(|event| event["stage"] == "publishing"),
        "zero-attachment upload omitted publishing"
    );
    ensure!(
        events.last().unwrap()["bytes_total"] == 0 && events.last().unwrap()["files_total"] == 0,
        "zero-attachment totals changed"
    );
    ensure!(
        fixture.storage()?.list()? == vec![key],
        "zero-attachment snapshot missing"
    );
    evidence.push(
        json!({"case":"zero-attachments","passed":true,"events":events,"http":fixture.evidence()}),
    );

    let report = root.join("backup-progress-evidence.json");
    fs::write(&report, serde_json::to_vec_pretty(&evidence)?)?;
    results.push(json!({"workflow":"云备份真实上传进度：流式字节/文件总数、确认后完成、取消/失败、零附件及恢复","passed":true,"cases":evidence.len(),"evidence":report}));
    Ok(())
}
