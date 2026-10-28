//! Cloud-backup cancellation through real loopback HTTP, SQLite transactions and files.
use anyhow::{Context, Result, ensure};
use clarora_gpui::{
    backup::{self, Storage},
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
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Normal,
    HeaderStall,
    ManifestStall,
    AttachmentStall,
    UploadStall,
    LastUpload,
    LastDownload,
    EmptyManifest,
    SnapshotUncertain,
    SnapshotSuccess,
}
struct Fixture {
    address: SocketAddr,
    objects: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    requests: Arc<Mutex<Vec<Value>>>,
    cancelled_at: Arc<Mutex<Option<Instant>>>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}
fn trigger(cancel: &Cancel, at: &Mutex<Option<Instant>>) {
    *at.lock().unwrap() = Some(Instant::now());
    cancel.store(true, Ordering::SeqCst);
}
fn pause(stop: &AtomicBool, millis: u64) {
    let end = Instant::now() + Duration::from_millis(millis);
    while !stop.load(Ordering::SeqCst) && Instant::now() < end {
        thread::sleep(Duration::from_millis(5));
    }
}
fn reply(stream: &mut TcpStream, status: &str, bytes: &[u8], length: usize) -> Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n"
    )?;
    stream.write_all(bytes)?;
    stream.flush()?;
    Ok(())
}
impl Fixture {
    fn new(mode: Mode, cancel: Cancel, initial: BTreeMap<String, Vec<u8>>) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let objects = Arc::new(Mutex::new(initial));
        let requests = Arc::new(Mutex::new(vec![]));
        let cancelled_at = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let (store, seen, at, stopped) = (
            objects.clone(),
            requests.clone(),
            cancelled_at.clone(),
            stop.clone(),
        );
        let handle = thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                let Ok((mut stream, _)) = listener.accept() else {
                    pause(&stopped, 5);
                    continue;
                };
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));
                let serve = (|| -> Result<()> {
                    let mut incoming = vec![];
                    let mut chunk = [0_u8; 8192];
                    let split = loop {
                        let n = stream.read(&mut chunk)?;
                        ensure!(n > 0, "fixture request ended before headers");
                        incoming.extend_from_slice(&chunk[..n]);
                        if let Some(i) = incoming.windows(4).position(|w| w == b"\r\n\r\n") {
                            break i + 4;
                        }
                    };
                    let header = String::from_utf8_lossy(&incoming[..split]);
                    let line = header.lines().next().context("request line")?;
                    let mut parts = line.split_whitespace();
                    let method = parts.next().context("method")?.to_string();
                    let target = parts.next().context("target")?.to_string();
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
                        header.contains("Credential=fixture-access/"),
                        "signature missing"
                    );
                    let attachment = key.contains("/media/");
                    let snapshot = key.contains("/snapshots/");
                    let index = {
                        let mut rows = seen.lock().unwrap();
                        let i = rows.len();
                        rows.push(json!({"method":method,"key":key,"content_length":length,"received":incoming.len()-split}));
                        i
                    };
                    if mode == Mode::HeaderStall
                        || (mode == Mode::UploadStall && method == "PUT" && attachment)
                    {
                        pause(&stopped, 120);
                        trigger(&cancel, &at);
                        pause(&stopped, 2000);
                        return Ok(());
                    }
                    let mut body = incoming[split..].to_vec();
                    while body.len() < length {
                        let n = stream.read(&mut chunk)?;
                        ensure!(n > 0, "request body interrupted");
                        body.extend_from_slice(&chunk[..n]);
                    }
                    seen.lock().unwrap()[index]["received"] = json!(body.len());
                    if method == "PUT" {
                        store.lock().unwrap().insert(key, body);
                        if mode == Mode::LastUpload && attachment {
                            trigger(&cancel, &at);
                        }
                        if mode == Mode::SnapshotUncertain && snapshot {
                            pause(&stopped, 120);
                            trigger(&cancel, &at);
                            pause(&stopped, 2000);
                            return Ok(());
                        }
                        if mode == Mode::SnapshotSuccess && snapshot {
                            // A successful PUT status commits the snapshot; its body may stall.
                            reply(&mut stream, "200 OK", b"", 32)?;
                            pause(&stopped, 120);
                            trigger(&cancel, &at);
                            pause(&stopped, 2000);
                            return Ok(());
                        }
                        reply(&mut stream, "200 OK", b"", 0)?;
                    } else if target.contains("list-type=2") {
                        let contents = store
                            .lock()
                            .unwrap()
                            .keys()
                            .filter(|k| k.contains("/snapshots/"))
                            .map(|k| format!("<Contents><Key>{k}</Key></Contents>"))
                            .collect::<String>();
                        let body = format!(
                            "<ListBucketResult>{contents}<IsTruncated>false</IsTruncated></ListBucketResult>"
                        );
                        reply(&mut stream, "200 OK", body.as_bytes(), body.len())?;
                    } else {
                        let bytes = store
                            .lock()
                            .unwrap()
                            .get(&key)
                            .context("fixture object missing")?
                            .clone();
                        if (mode == Mode::ManifestStall && snapshot)
                            || (mode == Mode::AttachmentStall && attachment)
                        {
                            reply(
                                &mut stream,
                                "200 OK",
                                &bytes[..bytes.len().min(128)],
                                bytes.len(),
                            )?;
                            pause(&stopped, 120);
                            trigger(&cancel, &at);
                            pause(&stopped, 2000);
                            return Ok(());
                        }
                        if (mode == Mode::LastDownload && attachment)
                            || (mode == Mode::EmptyManifest && snapshot)
                        {
                            trigger(&cancel, &at);
                        }
                        reply(&mut stream, "200 OK", &bytes, bytes.len())?;
                    }
                    Ok(())
                })();
                if let Err(error) = serve {
                    seen.lock()
                        .unwrap()
                        .push(json!({"fixture_error":error.to_string()}));
                }
            }
        });
        Ok(Self {
            address,
            objects,
            requests,
            cancelled_at,
            stop,
            handle: Some(handle),
        })
    }
    fn storage(&self) -> Result<Storage> {
        Storage::loopback_fixture(self.address)
    }
    fn evidence(&self) -> Value {
        json!({"requests":self.requests.lock().unwrap().clone(),"stored_keys":self.objects.lock().unwrap().keys().cloned().collect::<Vec<_>>()})
    }
    fn latency(&self) -> Result<u128> {
        let elapsed = self
            .cancelled_at
            .lock()
            .unwrap()
            .context("fixture did not cancel")?
            .elapsed();
        ensure!(
            elapsed < Duration::from_secs(1),
            "取消未及时返回：{elapsed:?}"
        );
        Ok(elapsed.as_millis())
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
fn source(root: &Path, attachment: bool) -> Result<Library> {
    let lib = Library::open(root.to_path_buf())?;
    lib.db()?.execute(
        "INSERT INTO words(word,meaning) VALUES('backup-fixture','测试资料')",
        [],
    )?;
    if attachment {
        let path = lib.root.join("media/fixture.bin");
        // Sparse file exercises a streamed upload without a large retained fixture.
        let mut file = fs::File::create(&path)?;
        file.write_all(b"real backup attachment fixture")?;
        file.set_len(32 * 1024 * 1024)?;
        lib.put("clip_cards", json!({"id":"fixture-clip","en_text":"fixture","zh_text":"附件","audio_uri":path.to_string_lossy(),"start_ms":0,"end_ms":1000,"created_at":data::now()}))?;
    }
    Ok(lib)
}
fn remote_objects(lib: &Library) -> Result<(String, BTreeMap<String, Vec<u8>>)> {
    let (mut manifest, files) = backup::collect(lib)?;
    let mut objects = BTreeMap::new();
    for (reference, file) in files {
        let key = format!(
            "clarora/media/{}/{}.bin",
            manifest.id,
            reference.trim_start_matches("media:")
        );
        manifest.files.insert(reference, key.clone());
        objects.insert(key, fs::read(file)?);
    }
    let key = format!("clarora/snapshots/{}.json", manifest.id);
    objects.insert(key.clone(), serde_json::to_vec(&manifest)?);
    Ok((key, objects))
}
fn unchanged(lib: &Library, before: &Value) -> Result<()> {
    ensure!(
        serde_json::to_value(lib.vault()?)? == *before,
        "取消恢复仍改了资料"
    );
    ensure!(
        fs::read_dir(lib.root.join("media"))?.next().is_none(),
        "取消恢复留下了附件目录"
    );
    Ok(())
}
pub fn config_messages(parent: &Library, results: &mut Vec<Value>) -> Result<()> {
    let lib = Library::open(parent.root.join("storage-config-messages"))?;
    let mut evidence = vec![];
    for (name, raw) in [
        ("not configured", ""),
        ("whitespace", " \n "),
        ("malformed", "{\"private\":\"CONFIG_PRIVATE_SENTINEL\""),
        ("incomplete", "{}"),
    ] {
        lib.set_setting("storage_config", raw)?;
        let error = match Storage::load(&lib) {
            Ok(_) => anyhow::bail!("{name}: invalid config was accepted"),
            Err(error) => format!("{error:#}"),
        };
        ensure!(
            error.contains("云备份") && error.contains("同步与备份") && error.contains("保存"),
            "提示没有明确配置位置或修复步骤：{error}"
        );
        ensure!(
            !error.contains("EOF")
                && !error.contains("line 1")
                && !error.contains("CONFIG_PRIVATE_SENTINEL"),
            "界面提示泄漏解析细节／配置"
        );
        ensure!(lib.setting("storage_config")? == raw, "失败时覆盖了配置");
        evidence.push(json!({"case":name,"passed":true,"message":error}));
    }
    Storage::save(
        &lib,
        backup::StorageConfig {
            provider: "s3".into(),
            endpoint: "https://fixture.invalid".into(),
            region: "us-east-1".into(),
            bucket: "fixture-bucket".into(),
            access_key_id: "fixture-access".into(),
            prefix: "clarora".into(),
            path_style: true,
            ..Default::default()
        },
        "fixture-secret",
        "",
    )?;
    Storage::load(&Library::open(lib.root.clone())?)?;
    evidence.push(json!({"case":"save and reopen valid configuration","passed":true}));
    let report = lib.root.join("config-messages.json");
    fs::write(&report, serde_json::to_vec_pretty(&evidence)?)?;
    results.push(json!({"workflow":"云备份配置：未配置／空白／损坏／缺字段中文提示与原配置保护／有效配置重开","passed":true,"cases":evidence.len(),"evidence":report}));
    Ok(())
}
pub fn run(lib: &Library, results: &mut Vec<Value>) -> Result<()> {
    config_messages(lib, results)?;
    let root = lib.root.join("backup-cancel");
    fs::create_dir_all(&root)?;
    let source = source(&root.join("source"), true)?;
    let empty = source_without_attachment(&root)?;
    let mut evidence = vec![];
    for (name, mode, source) in [
        ("headers-wait", Mode::HeaderStall, &source),
        ("manifest-body-stall", Mode::ManifestStall, &source),
        ("last-attachment-body-stall", Mode::AttachmentStall, &source),
        ("last-attachment-boundary", Mode::LastDownload, &source),
        ("zero-attachment-boundary", Mode::EmptyManifest, &empty),
    ] {
        let target = Library::open(root.join(name))?;
        let before = serde_json::to_value(target.vault()?)?;
        let cancel = token();
        let (key, objects) = remote_objects(source)?;
        let fixture = Fixture::new(mode, cancel.clone(), objects)?;
        let error = fixture
            .storage()?
            .restore(&target, &key, &cancel)
            .err()
            .context("取消恢复却成功")?;
        ensure!(
            error.to_string().contains("已取消"),
            "取消返回错误不明确：{error:#}"
        );
        let latency = fixture.latency()?;
        unchanged(&target, &before)?;
        evidence.push(json!({"case":name,"passed":true,"cancel_ms":latency,"before":before,"after":target.vault()?,"http":fixture.evidence()}));
    }
    for (name, mode) in [
        ("upload-body-stall", Mode::UploadStall),
        ("last-upload-boundary", Mode::LastUpload),
    ] {
        let cancel = token();
        let fixture = Fixture::new(mode, cancel.clone(), BTreeMap::new())?;
        let error = fixture
            .storage()?
            .upload(&source, &cancel)
            .err()
            .context("取消上传却成功")?;
        ensure!(
            error.to_string().contains("已取消"),
            "上传取消错误：{error:#}"
        );
        let latency = fixture.latency()?;
        ensure!(
            !fixture
                .objects
                .lock()
                .unwrap()
                .keys()
                .any(|k| k.contains("/snapshots/")),
            "取消仍发布了清单"
        );
        evidence
            .push(json!({"case":name,"passed":true,"cancel_ms":latency,"http":fixture.evidence()}));
    }
    for (name, mode) in [
        ("snapshot-outcome-unknown", Mode::SnapshotUncertain),
        ("snapshot-success-body-stall", Mode::SnapshotSuccess),
    ] {
        let cancel = token();
        let fixture = Fixture::new(mode, cancel.clone(), BTreeMap::new())?;
        let result = fixture.storage()?.upload(&empty, &cancel);
        if mode == Mode::SnapshotUncertain {
            let error = result.err().context("未知提交状态被当作成功")?.to_string();
            ensure!(
                error.contains("未确认") && error.contains("云端列表"),
                "未明确远端提交不确定：{error}"
            );
            evidence.push(json!({"case":name,"passed":true,"cancel_ms":fixture.latency()?,"error":error,"http":fixture.evidence()}));
        } else {
            let key = result?;
            ensure!(
                fixture.objects.lock().unwrap().contains_key(&key),
                "成功未发布清单"
            );
            evidence.push(json!({"case":name,"passed":true,"key":key,"http":fixture.evidence()}));
        }
    }
    let cancel = token();
    cancel.store(true, Ordering::SeqCst);
    let fixture = Fixture::new(Mode::Normal, cancel.clone(), BTreeMap::new())?;
    let target = Library::open(root.join("pre-cancelled"))?;
    ensure!(
        fixture.storage()?.upload(&empty, &cancel).is_err(),
        "预取消上传成功"
    );
    ensure!(
        fixture
            .storage()?
            .restore(&target, "clarora/snapshots/fixture.json", &cancel)
            .is_err(),
        "预取消恢复成功"
    );
    ensure!(
        fixture.requests.lock().unwrap().is_empty(),
        "预取消仍发网络请求"
    );
    evidence.push(
        json!({"case":"pre-cancelled-zero-attachments","passed":true,"http":fixture.evidence()}),
    );
    drop(fixture);

    // Hold a real SQLite write lock so cancellation occurs after download, during merge.
    let target = Library::open(root.join("merge-transaction-rollback"))?;
    let before = serde_json::to_value(target.vault()?)?;
    let cancel = token();
    let (key, objects) = remote_objects(&empty)?;
    let fixture = Fixture::new(Mode::Normal, cancel.clone(), objects)?;
    let db = target.db()?;
    db.execute_batch("BEGIN IMMEDIATE")?;
    let copied = target.clone();
    let storage = fixture.storage()?;
    let flag = cancel.clone();
    let (done, completed) = std::sync::mpsc::channel();
    let task = thread::spawn(move || {
        let _ = done.send(storage.restore(&copied, &key, &flag));
    });
    let deadline = Instant::now() + Duration::from_secs(3);
    while fixture.requests.lock().unwrap().is_empty() {
        ensure!(Instant::now() < deadline, "恢复请求未启动");
        thread::sleep(Duration::from_millis(5));
    }
    thread::sleep(Duration::from_millis(120));
    let started = Instant::now();
    cancel.store(true, Ordering::SeqCst);
    // Cancellation must finish even while the competing writer still owns its lock.
    let outcome = completed.recv_timeout(Duration::from_secs(1));
    let latency = started.elapsed().as_millis();
    db.execute_batch("ROLLBACK")?;
    task.join()
        .map_err(|_| anyhow::anyhow!("restore thread panicked"))?;
    let error = outcome
        .context("写锁等待取消没有在1秒内结束")?
        .err()
        .context("merge取消未回滚")?;
    ensure!(
        error.to_string().contains("已取消"),
        "merge取消错误：{error:#}"
    );
    unchanged(&target, &before)?;
    evidence.push(json!({"case":"merge-transaction-rollback","passed":true,"cancel_ms":latency,"cancelled_while_other_writer_held_lock":true,"before":before,"after":target.vault()?,"http":fixture.evidence()}));
    drop(fixture);

    let cancel = token();
    let fixture = Fixture::new(Mode::Normal, cancel.clone(), BTreeMap::new())?;
    let storage = fixture.storage()?;
    let key = storage.upload(&source, &cancel)?;
    ensure!(storage.list()? == vec![key.clone()], "上传后列表不一致");
    let target = Library::open(root.join("roundtrip"))?;
    ensure!(
        storage.restore(&target, &key, &cancel)? == 2,
        "恢复条目数不一致"
    );
    ensure!(
        storage.restore(&target, &key, &cancel)? == 0,
        "重复恢复产生记录"
    );
    let row = target.list("clip_cards")?.remove(0);
    ensure!(
        fs::read(data::text(&row, "audio_uri"))?
            == fs::read(source.root.join("media/fixture.bin"))?,
        "恢复附件字节不同"
    );
    evidence.push(
        json!({"case":"normal-upload-list-restore-repeat","passed":true,"http":fixture.evidence()}),
    );
    fs::write(
        root.join("backup-cancel-evidence.json"),
        serde_json::to_vec_pretty(&evidence)?,
    )?;
    results.push(json!({"workflow":"云备份取消：头部/正文等待、大附件、末附件/零附件、清单提交不确定、事务回滚、正常恢复","passed":true,"cases":evidence.len(),"evidence":"backup-cancel/backup-cancel-evidence.json"}));
    Ok(())
}
fn source_without_attachment(root: &Path) -> Result<Library> {
    source(&root.join("source-zero-attachments"), false)
}
