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
        stream.set_read_timeout(Some(Duration::from_millis(100)))?;
        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
        let mut input = Vec::new();
        let mut chunk = [0_u8; 32 * 1024];
        let split = loop {
            match stream.read(&mut chunk) {
                Ok(0) => return Ok(()),
                Ok(count) => input.extend_from_slice(&chunk[..count]),
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    if control.stop.load(Ordering::SeqCst) {
                        return Ok(());
                    }
                    continue;
                }
                Err(error) => return Err(error.into()),
            }
            if let Some(index) = input.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                break index + 4;
            }
            ensure!(
                input.len() < 64 * 1024,
                "native fixture header exceeded bound"
            );
        };
        let headers = std::str::from_utf8(&input[..split])?;
        let line = headers
            .lines()
            .next()
            .context("native fixture request line")?;
        ensure!(
            line.starts_with("PUT /fixture-bucket/"),
            "native fixture expected signed PUT"
        );
        ensure!(
            headers.contains("Credential=fixture-access/"),
            "native fixture missing fake signature"
        );
        let manifest = line.contains("/snapshots/");
        let length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<u64>().ok())
                    .flatten()
            })
            .context("native fixture missing content length")?;
        let mut received = (input.len() - split) as u64;
        drop(input);
        if !manifest {
            control.bytes_received.fetch_add(received, Ordering::SeqCst);
        }
        while received < length && !control.stop.load(Ordering::SeqCst) {
            let needed = (length - received).min(chunk.len() as u64) as usize;
            match stream.read(&mut chunk[..needed]) {
                Ok(0) => break,
                Ok(count) => {
                    received += count as u64;
                    if !manifest {
                        control
                            .bytes_received
                            .fetch_add(count as u64, Ordering::SeqCst);
                        thread::sleep(Duration::from_millis(4));
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    continue;
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
                    ) =>
                {
                    break;
                }
                Err(error) => return Err(error.into()),
            }
        }
        control.requests.lock().unwrap().push(json!({"method":"PUT","manifest":manifest,"bytes_received":received,"declared_bytes":length,"complete":received==length}));
        if received != length || control.stop.load(Ordering::SeqCst) {
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
        let rejected = manifest && control.reject_manifest.load(Ordering::SeqCst);
        let (status, body) = if rejected {
            (
                "403 Forbidden",
                "<Error><Code>AccessDenied</Code><Message>native-private-marker</Message></Error>",
            )
        } else {
            ("200 OK", "")
        };
        write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )?;
        stream.flush()?;
        if manifest && !rejected {
            control.manifests_accepted.fetch_add(1, Ordering::SeqCst);
        }
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
        let cancel = app
            .tasks
            .cancel
            .as_ref()
            .context("cloud backup stopped during page navigation")?;
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
            app.tasks.notice.as_ref().is_some_and(
                |notice| notice.title == "云备份" && notice.state == TaskState::Running
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
