//! Hidden-window author self-check of cloud backup across real AppView page actions.
//! Uses isolated files and loopback HTTP; this is not a physical-input or visual review.
use super::*;
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64},
    },
    thread,
};

pub(super) fn prepare(out: &Path) -> Result<Library> {
    fs::create_dir_all(out)?;
    let lib = Library::open(out.join("library"))?;
    let path = lib.root.join("media/native-backup-fixture.bin");
    let mut file = fs::File::create(&path)?;
    file.write_all(b"Clarora isolated native backup progress fixture")?;
    file.set_len(32 * 1024 * 1024)?;
    lib.put("clip_cards", json!({"id":"native-backup-fixture","en_text":"Native backup fixture.","zh_text":"后台备份探针。","audio_uri":path.to_string_lossy(),"start_ms":0,"end_ms":1000,"created_at":clarora_gpui::data::now()}))?;
    lib.set_setting("last_page", "settings")?;
    Ok(lib)
}

#[derive(Default)]
struct Control {
    stop: AtomicBool,
    manifest_waiting: AtomicBool,
    release_manifest: AtomicBool,
    reject_manifest: AtomicBool,
    bytes_received: AtomicU64,
    manifests_accepted: AtomicU64,
    requests: Mutex<Vec<Value>>,
    objects: Mutex<std::collections::BTreeMap<String, Vec<u8>>>,
    error: Mutex<Option<String>>,
}
struct Fixture {
    address: SocketAddr,
    control: Arc<Control>,
    handle: Option<thread::JoinHandle<()>>,
}
impl Fixture {
    fn new() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let control = Arc::new(Control::default());
        let worker = control.clone();
        let handle = thread::spawn(move || {
            while !worker.stop.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => {
                        *worker.error.lock().unwrap() = Some(error.to_string());
                        break;
                    }
                };
                if let Err(error) = Self::serve(&mut stream, &worker) {
                    if !worker.stop.load(Ordering::SeqCst) {
                        *worker.error.lock().unwrap() = Some(format!("{error:#}"));
                    }
                }
            }
        });
        Ok(Self {
            address,
            control,
            handle: Some(handle),
        })
    }
    fn serve(stream: &mut TcpStream, control: &Control) -> Result<()> {
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(Duration::from_millis(100)))?;
        stream.set_write_timeout(Some(Duration::from_secs(3)))?;
        let mut input = vec![];
        let mut chunk = [0u8; 32768];
        let split = loop {
            match stream.read(&mut chunk) {
                Ok(0) => return Ok(()),
                Ok(n) => input.extend_from_slice(&chunk[..n]),
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    if control.stop.load(Ordering::SeqCst) {
                        return Ok(());
                    }
                    continue;
                }
                Err(e) => return Err(e.into()),
            }
            if let Some(i) = input.windows(4).position(|s| s == b"\r\n\r\n") {
                break i + 4;
            }
            ensure!(input.len() < 65536, "native fixture header too large");
        };
        let header = std::str::from_utf8(&input[..split])?;
        let mut line = header
            .lines()
            .next()
            .context("request line")?
            .split_whitespace();
        let method = line.next().context("method")?.to_string();
        let target = line.next().context("target")?;
        let url = url::Url::parse(&format!("http://fixture{target}"))?;
        let key = url
            .path()
            .strip_prefix("/fixture-bucket/")
            .context("bucket")?
            .to_string();
        ensure!(
            header.contains("Credential=fixture-access/"),
            "fixture signature missing"
        );
        let headers = header
            .lines()
            .skip(1)
            .filter_map(|s| s.split_once(':'))
            .map(|(k, v)| (k.to_ascii_lowercase(), v.trim().to_string()))
            .collect::<std::collections::BTreeMap<_, _>>();
        let length = headers
            .get("content-length")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(0);
        ensure!(length <= 64 * 1024 * 1024, "fixture request too large");
        let attachment = method == "PUT" && (key.contains("/objects/") || key.contains("/media/"));
        let manifest = method == "PUT" && (key.contains("/snapshots/") || key.contains("/sync/"));
        let mut body = input[split..].to_vec();
        if attachment {
            control
                .bytes_received
                .fetch_add(body.len() as u64, Ordering::SeqCst);
        }
        while body.len() < length && !control.stop.load(Ordering::SeqCst) {
            let count = (length - body.len()).min(chunk.len());
            match stream.read(&mut chunk[..count]) {
                Ok(0) => break,
                Ok(n) => {
                    body.extend_from_slice(&chunk[..n]);
                    if attachment {
                        control.bytes_received.fetch_add(n as u64, Ordering::SeqCst);
                        thread::sleep(Duration::from_millis(4));
                    }
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    continue;
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
                    ) =>
                {
                    break;
                }
                Err(e) => return Err(e.into()),
            }
        }
        control.requests.lock().unwrap().push(json!({"method":method,"key":key,"manifest":manifest,"bytes_received":body.len(),"declared_bytes":length,"complete":body.len()==length}));
        if body.len() != length || control.stop.load(Ordering::SeqCst) {
            return Ok(());
        }
        if manifest {
            control.manifest_waiting.store(true, Ordering::SeqCst);
            while !control.release_manifest.load(Ordering::SeqCst)
                && !control.stop.load(Ordering::SeqCst)
            {
                thread::sleep(Duration::from_millis(5));
            }
            control.manifest_waiting.store(false, Ordering::SeqCst);
            if control.stop.load(Ordering::SeqCst) {
                return Ok(());
            }
        }
        let mut store = control.objects.lock().unwrap();
        let old = store.get(&key);
        let etag = old.map(|b| format!("\"{}\"", backup::cloud::checksum(b)));
        let precondition = headers.get("if-none-match").is_some_and(|s| s == "*") && old.is_some()
            || headers
                .get("if-match")
                .is_some_and(|s| Some(s) != etag.as_ref());
        let (status, output, extra) = if precondition {
            ("412 Precondition Failed", vec![], String::new())
        } else if manifest && control.reject_manifest.load(Ordering::SeqCst) {
            (
                "403 Forbidden",
                b"<Error><Code>AccessDenied</Code><Message>native-private-marker</Message></Error>"
                    .to_vec(),
                String::new(),
            )
        } else if method == "PUT" {
            let etag = backup::cloud::checksum(&body);
            store.insert(key, body);
            if manifest {
                control.manifests_accepted.fetch_add(1, Ordering::SeqCst);
            }
            ("200 OK", vec![], format!("ETag: \"{etag}\"\r\n"))
        } else if method == "DELETE" {
            store.remove(&key);
            ("204 No Content", vec![], String::new())
        } else if url.query_pairs().any(|(k, _)| k == "list-type") {
            let prefix = url
                .query_pairs()
                .find(|(k, _)| k == "prefix")
                .map(|(_, v)| v.into_owned())
                .unwrap_or_default();
            let items=store.iter().filter(|(k,_)|k.starts_with(&prefix)).map(|(k,v)|format!("<Contents><Key>{k}</Key><Size>{}</Size><LastModified>{}</LastModified></Contents>",v.len(),chrono::Utc::now().to_rfc3339())).collect::<String>();
            (
                "200 OK",
                format!(
                    "<ListBucketResult>{items}<IsTruncated>false</IsTruncated></ListBucketResult>"
                )
                .into_bytes(),
                String::new(),
            )
        } else if let Some(bytes) = old {
            let sha = backup::cloud::checksum(bytes);
            (
                "200 OK",
                bytes.clone(),
                format!("ETag: \"{sha}\"\r\nx-amz-meta-sha256: {sha}\r\n"),
            )
        } else {
            ("404 Not Found", vec![], String::new())
        };
        write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n",
            output.len()
        )?;
        if method != "HEAD" {
            stream.write_all(&output)?;
        }
        stream.flush()?;
        Ok(())
    }
    fn reset(&self, reject: bool) {
        self.control.bytes_received.store(0, Ordering::SeqCst);
        self.control.release_manifest.store(false, Ordering::SeqCst);
        self.control.reject_manifest.store(reject, Ordering::SeqCst);
        self.control.manifest_waiting.store(false, Ordering::SeqCst);
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.control.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

pub(super) struct Probe {
    out: PathBuf,
    started: Instant,
    entered: Instant,
    stage: usize,
    cases: Vec<Value>,
    fixture: Option<Fixture>,
    active_token: Option<Cancel>,
    first_bytes: u64,
    max_bytes: u64,
    material_guard: Option<Value>,
}
impl Probe {
    pub(super) fn new(out: &Path) -> Self {
        Self {
            out: out.to_path_buf(),
            started: Instant::now(),
            entered: Instant::now(),
            stage: 0,
            cases: vec![],
            fixture: None,
            active_token: None,
            first_bytes: 0,
            max_bytes: 0,
            material_guard: None,
        }
    }
    fn report(&self, passed: Option<bool>, error: Option<String>) -> Result<()> {
        let requests = self
            .fixture
            .as_ref()
            .map(|fixture| fixture.control.requests.lock().unwrap().clone())
            .unwrap_or_default();
        fs::write(
            self.out.join("backup-progress-ui-report.json"),
            serde_json::to_vec_pretty(
                &json!({"passed":passed,"error":error,"stage":self.stage,"elapsed_ms":self.started.elapsed().as_millis(),"cases":self.cases,"http_requests":requests,"hidden_native_window":true,"actual_appview_handlers":true,"actual_signed_http":true,"physical_input_visual_verified":false,"independent_review":false}),
            )?,
        )?;
        Ok(())
    }
    fn note(&mut self, name: &str, app: &AppView, cx: &Context<AppView>) -> Result<()> {
        self.cases.push(json!({"case":name,"passed":true,"page":app.page,"status":app.status,"scope_application":app.tasks.scope==TaskScope::Application,"banner_present":app.subtitle_task_banner(cx).is_some(),"progress":app.tasks.backup_progress,"elapsed_ms":self.started.elapsed().as_millis()}));
        self.report(None, None)
    }
    fn advance(&mut self) {
        self.stage += 1;
        self.entered = Instant::now();
    }
    fn request_draw(window: &Window, cx: &mut Context<AppView>) {
        let handle = window.window_handle();
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| {
                window.draw(cx).clear();
            });
        });
    }
    fn material_state(app: &AppView, cx: &App) -> Value {
        json!({"page":app.page,"audio":app.audio.current_id,"video":app.video.path,"chat_context":app.chat.context,"chat_session":app.chat.id,"draft":app.value("question",cx),"data_revision":app.data.revision})
    }
    fn running(&mut self, app: &AppView, cx: &Context<AppView>) -> Result<()> {
        let cancel = app.tasks.cancel.as_ref().with_context(|| {
            format!(
                "cloud operation stopped during page navigation: {}",
                app.status
            )
        })?;
        ensure!(
            !cancel.load(Ordering::SeqCst),
            "page navigation cancelled cloud backup"
        );
        ensure!(
            self.active_token
                .as_ref()
                .is_some_and(|original| Arc::ptr_eq(original, cancel)),
            "page navigation replaced backup task"
        );
        ensure!(
            app.tasks.scope == TaskScope::Application,
            "cloud backup did not use application scope"
        );
        ensure!(
            app.subtitle_task_banner(cx).is_some(),
            "cloud backup banner missing on current page"
        );
        ensure!(
            app.tasks
                .notice
                .as_ref()
                .is_some_and(
                    |notice| ["云备份", "云端同步"].contains(&notice.title.as_str())
                        && notice.state == TaskState::Running
                ),
            "cloud backup notice stopped or disappeared"
        );
        if let Some(progress) = &app.tasks.backup_progress {
            let library_path = app.lib.root.to_string_lossy();
            let library_path: &str = library_path.as_ref();
            ensure!(
                !progress.label().contains(library_path),
                "progress leaked library path"
            );
            self.max_bytes = self.max_bytes.max(progress.bytes_sent);
        }
        Ok(())
    }
    fn start(
        &mut self,
        app: &mut AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
        reject: bool,
    ) -> Result<()> {
        let fixture = self
            .fixture
            .as_ref()
            .context("native HTTP fixture missing")?;
        fixture.reset(reject);
        // Each run edits the isolated source so cancellation still has a real
        // in-flight chunk even after a prior successful deduplicated backup.
        let path = app.lib.root.join("media/native-backup-fixture.bin");
        let mut file = fs::OpenOptions::new().write(true).open(path)?;
        file.write_all(id().as_bytes())?;
        file.sync_all()?;
        app.tasks.backup_fixture = Some(fixture.address);
        app.handle(Action::CloudBackup, window, cx);
        self.active_token = app.tasks.cancel.clone();
        ensure!(
            self.active_token.is_some(),
            "real CloudBackup action did not start a task"
        );
        ensure!(
            app.tasks
                .backup_progress
                .as_ref()
                .is_none_or(|progress| progress.bytes_sent == 0),
            "new backup retained previous progress"
        );
        Ok(())
    }
    fn step(
        &mut self,
        app: &mut AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Result<bool> {
        ensure!(
            self.started.elapsed() < Duration::from_secs(50),
            "native backup progress check timed out at stage {}",
            self.stage
        );
        if let Some(error) = self
            .fixture
            .as_ref()
            .and_then(|fixture| fixture.control.error.lock().unwrap().clone())
        {
            anyhow::bail!("native HTTP fixture: {error}");
        }
        match self.stage {
            0 => {
                app.media.player = Player::without_audio_device();
                app.media.preloader = Player::without_audio_device();
                app.handle(Action::Page("settings".into()), window, cx);
                self.fixture = Some(Fixture::new()?);
                self.start(app, window, cx, false)?;
                self.note("actual CloudBackup starts application-scoped task", app, cx)?;
                self.advance();
            }
            1 => {
                self.running(app, cx)?;
                let Some(progress) = app
                    .tasks
                    .backup_progress
                    .as_ref()
                    .filter(|progress| progress.bytes_sent > 0)
                else {
                    return Ok(false);
                };
                self.first_bytes = progress.bytes_sent;
                app.handle(Action::Page("video".into()), window, cx);
                self.running(app, cx)?;
                self.note(
                    "switch to video preserves live task and global progress",
                    app,
                    cx,
                )?;
                Self::request_draw(window, cx);
                self.advance();
            }
            2..=4 => {
                self.running(app, cx)?;
                if self.entered.elapsed() < Duration::from_millis(150) {
                    return Ok(false);
                }
                let page = match self.stage {
                    2 => "library",
                    3 => "writing",
                    _ => "meditation",
                };
                app.handle(Action::Page(page.into()), window, cx);
                self.running(app, cx)?;
                self.note(
                    &format!("switch to {page} preserves live task and global progress"),
                    app,
                    cx,
                )?;
                Self::request_draw(window, cx);
                self.advance();
            }
            5 => {
                self.running(app, cx)?;
                if !self
                    .fixture
                    .as_ref()
                    .unwrap()
                    .control
                    .manifest_waiting
                    .load(Ordering::SeqCst)
                {
                    return Ok(false);
                }
                let progress = app
                    .tasks
                    .backup_progress
                    .as_ref()
                    .context("missing publishing progress")?;
                if progress.stage != backup::UploadStage::Publishing {
                    return Ok(false);
                }
                ensure!(
                    self.max_bytes > self.first_bytes,
                    "backup progress never advanced after switching modules"
                );
                ensure!(
                    progress.percent().is_none_or(|percent| percent < 100.),
                    "backup reached 100 before manifest acknowledgement"
                );
                if !app.cards_ready() || app.chat.is_loading() {
                    return Ok(false);
                }
                app.set("question", "后台备份完成时保留的当前问题草稿", window, cx);
                self.material_guard = Some(Self::material_state(app, cx));
                self.note(
                    "pending manifest acknowledgement remains running below 100",
                    app,
                    cx,
                )?;
                self.fixture
                    .as_ref()
                    .unwrap()
                    .control
                    .release_manifest
                    .store(true, Ordering::SeqCst);
                self.advance();
            }
            6 => {
                if app.tasks.cancel.is_some() {
                    return Ok(false);
                }
                ensure!(
                    app.tasks
                        .notice
                        .as_ref()
                        .is_some_and(|notice| notice.state == TaskState::Succeeded),
                    "successful backup did not retain success notice"
                );
                ensure!(
                    app.tasks
                        .backup_progress
                        .as_ref()
                        .is_some_and(|progress| progress.stage == backup::UploadStage::Completed
                            && progress.percent() == Some(100.)),
                    "success did not retain completed progress"
                );
                ensure!(
                    app.subtitle_task_banner(cx).is_some(),
                    "success notice missing outside settings"
                );
                ensure!(
                    self.material_guard.as_ref() == Some(&Self::material_state(app, cx)),
                    "backup completion refreshed current material/session or changed its draft"
                );
                self.note(
                    "server-confirmed success preserves current material/session and is visible outside settings",
                    app,
                    cx,
                )?;
                let key = app
                    .backups
                    .first()
                    .context("completed backup was not added to version list")?
                    .clone();
                app.handle(Action::Page("settings".into()), window, cx);
                ensure!(
                    app.backups.contains(&key),
                    "returning to settings lost completed backup version"
                );
                self.note(
                    "returning to settings displays completed backup version",
                    app,
                    cx,
                )?;
                self.start(app, window, cx, false)?;
                self.advance();
            }
            7 => {
                self.running(app, cx)?;
                if !app
                    .tasks
                    .backup_progress
                    .as_ref()
                    .is_some_and(|progress| progress.bytes_sent > 0)
                {
                    return Ok(false);
                }
                app.handle(Action::Page("library".into()), window, cx);
                self.running(app, cx)?;
                app.handle(Action::Cancel, window, cx);
                ensure!(
                    self.active_token.as_ref().unwrap().load(Ordering::SeqCst),
                    "cancel action did not reach global backup"
                );
                self.note(
                    "cancel from another module reaches same backup task",
                    app,
                    cx,
                )?;
                self.advance();
            }
            8 => {
                if app.tasks.cancel.is_some() {
                    return Ok(false);
                }
                ensure!(
                    app.tasks
                        .notice
                        .as_ref()
                        .is_some_and(|notice| notice.state == TaskState::Cancelled),
                    "cancelled backup was marked failed or completed"
                );
                ensure!(
                    app.tasks
                        .backup_progress
                        .as_ref()
                        .is_none_or(|progress| progress.stage != backup::UploadStage::Completed),
                    "cancelled backup retained completed progress"
                );
                ensure!(
                    app.subtitle_task_banner(cx).is_some(),
                    "cancel notice missing outside settings"
                );
                self.note("cancelled backup stays cancelled and visible", app, cx)?;
                self.start(app, window, cx, true)?;
                self.advance();
            }
            9 => {
                self.running(app, cx)?;
                if !self
                    .fixture
                    .as_ref()
                    .unwrap()
                    .control
                    .manifest_waiting
                    .load(Ordering::SeqCst)
                {
                    return Ok(false);
                }
                app.handle(Action::Page("video".into()), window, cx);
                self.running(app, cx)?;
                self.fixture
                    .as_ref()
                    .unwrap()
                    .control
                    .release_manifest
                    .store(true, Ordering::SeqCst);
                self.advance();
            }
            10 => {
                if app.tasks.cancel.is_some() {
                    return Ok(false);
                }
                ensure!(
                    app.tasks
                        .notice
                        .as_ref()
                        .is_some_and(|notice| notice.state == TaskState::Failed),
                    "HTTP failure was marked cancelled or completed"
                );
                ensure!(
                    app.status.contains("AccessDenied"),
                    "service error was lost after changing modules"
                );
                ensure!(
                    !app.status.contains("native-private-marker"),
                    "service private message leaked"
                );
                ensure!(
                    app.tasks
                        .backup_progress
                        .as_ref()
                        .is_none_or(|progress| progress.stage != backup::UploadStage::Completed),
                    "failed backup retained completed progress"
                );
                ensure!(
                    app.subtitle_task_banner(cx).is_some(),
                    "failure notice missing outside settings"
                );
                self.note(
                    "HTTP failure remains visible and incomplete outside settings",
                    app,
                    cx,
                )?;
                self.fixture.as_ref().unwrap().reset(false);
                app.handle(Action::CloudSync, window, cx);
                self.active_token = app.tasks.cancel.clone();
                ensure!(
                    self.active_token.is_some(),
                    "real CloudSync action did not start"
                );
                self.note(
                    "actual CloudSync starts application-scoped signed S3 operation",
                    app,
                    cx,
                )?;
                self.advance();
            }
            11 => {
                self.running(app, cx)?;
                if !self
                    .fixture
                    .as_ref()
                    .unwrap()
                    .control
                    .manifest_waiting
                    .load(Ordering::SeqCst)
                    || !app
                        .tasks
                        .backup_progress
                        .as_ref()
                        .is_some_and(|p| p.stage == backup::UploadStage::Publishing)
                {
                    return Ok(false);
                }
                if app.page != "writing" {
                    app.handle(Action::Page("writing".into()), window, cx);
                    Self::request_draw(window, cx);
                    return Ok(false);
                }
                if app.chat.is_loading() {
                    return Ok(false);
                }
                app.set("question", "同步跨页后仍保留的问题草稿", window, cx);
                app.chat.observe_draft("同步跨页后仍保留的问题草稿");
                self.running(app, cx)?;
                self.note(
                    "sync keeps global progress across modules before server confirmation",
                    app,
                    cx,
                )?;
                self.fixture
                    .as_ref()
                    .unwrap()
                    .control
                    .release_manifest
                    .store(true, Ordering::SeqCst);
                self.advance();
            }
            12 => {
                if app.tasks.cancel.is_some() {
                    return Ok(false);
                }
                ensure!(
                    app.tasks
                        .notice
                        .as_ref()
                        .is_some_and(|n| n.state == TaskState::Succeeded),
                    "CloudSync did not complete successfully: {}",
                    app.status
                );
                ensure!(
                    app.cloud.status.contains("同步完成")
                        && app
                            .tasks
                            .backup_progress
                            .as_ref()
                            .is_some_and(|p| p.stage == backup::UploadStage::Completed),
                    "sync result or completed progress missing"
                );
                ensure!(
                    app.value("question", cx) == "同步跨页后仍保留的问题草稿",
                    "sync refreshed away active question draft"
                );
                self.note(
                    "successful guarded sync is visible outside settings and preserves draft",
                    app,
                    cx,
                )?;
                app.handle(Action::CloudRetention(0, 5), window, cx);
                ensure!(
                    app.cloud.retention.keep_last == 5
                        && app.lib.setting("cloud_retention_v3")?.contains("5"),
                    "retention choice did not persist"
                );
                app.handle(Action::CloudAutoSync, window, cx);
                ensure!(
                    app.cloud.automatic && app.lib.setting("cloud_auto_sync_v3")? == "true",
                    "automatic sync opt-in did not persist"
                );
                app.handle(Action::CloudAutoSync, window, cx);
                ensure!(!app.cloud.automatic, "automatic sync opt-out failed");
                self.note(
                    "retention and explicit automatic-sync opt-in persist through real actions",
                    app,
                    cx,
                )?;
                app.tasks.backup_fixture = None;
                self.report(Some(true), None)?;
                return Ok(true);
            }
            _ => {}
        }
        Ok(false)
    }
}

impl AppView {
    pub(super) fn poll_backup_probe(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(mut probe) = self.backup_probe.take() else {
            return false;
        };
        match probe.step(self, window, cx) {
            Ok(true) => {
                window.remove_window();
                return true;
            }
            Err(error) => {
                let _ = probe.report(Some(false), Some(format!("{error:#}")));
                window.remove_window();
                return true;
            }
            Ok(false) => {}
        }
        self.backup_probe = Some(probe);
        false
    }
}
