//! Actual hidden AppView actions with isolated media and a held loopback model response.
//! Author self-check only; not physical-input, external-download or independent review evidence.
use super::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, atomic::AtomicBool},
    thread,
};

pub(super) fn prepare(out: &Path) -> Result<Library> {
    let lib = ui_controller_probe::prepare(out)?;
    let mut third = lib
        .list("listening_audios")?
        .into_iter()
        .find(|r| text(r, "id") == "probe-audio-a")
        .context("audio fixture")?;
    third.insert("id".into(), json!("probe-audio-c"));
    third.insert("name".into(), json!("同组第二条"));
    lib.put("listening_audios", Value::Object(third))?;
    Ok(lib)
}

struct Fixture {
    host: String,
    release: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Fixture {
    fn new(fail: bool) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let host = format!("http://{}", listener.local_addr()?);
        let release = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let (go, end) = (release.clone(), stop.clone());
        let worker = thread::spawn(move || {
            while !end.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    Ok(s) => s,
                    Err(_) => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                let mut request = [0; 4096];
                let _ = stream.read(&mut request);
                if fail {
                    let _ = stream.write_all(
                        b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                    break;
                }
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2097152\r\nConnection: close\r\n\r\n",
                );
                for index in 0..128 {
                    if index == 16 {
                        while !go.load(Ordering::SeqCst) && !end.load(Ordering::SeqCst) {
                            thread::sleep(Duration::from_millis(5));
                        }
                    }
                    if end.load(Ordering::SeqCst) || stream.write_all(&[42; 16384]).is_err() {
                        break;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                break;
            }
        });
        Ok(Self {
            host,
            release,
            stop,
            worker: Some(worker),
        })
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub(super) struct Probe {
    out: PathBuf,
    started: Instant,
    stage: usize,
    fixture: Option<Fixture>,
    token: Option<Cancel>,
    foreground: Option<Cancel>,
    merged_id: String,
    cases: Vec<Value>,
}
impl Probe {
    pub(super) fn new(out: &Path) -> Self {
        Self {
            out: out.into(),
            started: Instant::now(),
            stage: 0,
            fixture: None,
            token: None,
            foreground: None,
            merged_id: String::new(),
            cases: vec![],
        }
    }
    fn report(&self, passed: bool, error: Option<String>) -> Result<()> {
        fs::write(
            self.out.join("model-navigation-report.json"),
            serde_json::to_vec_pretty(
                &json!({"passed":passed,"error":error,"stage":self.stage,"cases":self.cases,"hidden_native_window":true,"real_http_and_media":true,"independent_review":false,"physical_input_verified":false}),
            )?,
        )?;
        Ok(())
    }
    fn note(&mut self, name: &str, app: &AppView) {
        self.cases.push(json!({"case":name,"passed":true,"page":app.page,"audio":app.audio.current_id,"group":app.audio.practice_id,"video":app.video.path,"model_progress":app.tasks.models.progress,"foreground_active":app.tasks.cancel.is_some()}));
    }
    fn start(
        &mut self,
        app: &mut AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
        fail: bool,
    ) -> Result<()> {
        self.fixture = Some(Fixture::new(fail)?);
        app.tasks.models.fixture = Some(self.fixture.as_ref().unwrap().host.clone());
        app.handle(Action::Models("medium.en".into()), window, cx);
        self.token = app.tasks.models.cancel.clone();
        ensure!(
            self.token.is_some(),
            "Models action did not start background download"
        );
        Ok(())
    }
    fn active(&self, app: &AppView, cx: &Context<AppView>) -> Result<()> {
        ensure!(
            app.tasks.models.cancel.is_some()
                && !self.token.as_ref().unwrap().load(Ordering::SeqCst),
            "navigation cancelled model download"
        );
        ensure!(
            app.subtitle_task_banner(cx).is_some(),
            "global download banner missing"
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
            self.started.elapsed() < Duration::from_secs(35),
            "model navigation timed out stage {}",
            self.stage
        );
        match self.stage {
            0 => {
                app.media.player = Player::without_audio_device();
                self.start(app, window, cx, false)?;
                ensure!(
                    app.tasks.cancel.is_none(),
                    "model download occupies foreground lane"
                );
                self.note("model download has independent task lane", app);
            }
            1 => {
                if !app
                    .tasks
                    .models
                    .progress
                    .as_ref()
                    .is_some_and(|p| p.downloaded > 0)
                {
                    return Ok(false);
                }
                self.active(app, cx)?;
                let token = app.tasks.models.cancel.clone().unwrap();
                app.handle(Action::TtsDownload, window, cx);
                ensure!(
                    Arc::ptr_eq(&token, app.tasks.models.cancel.as_ref().unwrap()),
                    "duplicate model download started"
                );
                self.note(
                    "ASR and Kokoro share only the model lane; duplicate is rejected",
                    app,
                );
                app.handle(Action::Page("video".into()), window, cx);
                let a = app.lib.root.join("media/resume-video.mp4");
                let b = app.lib.root.join("media/resume-other.mp4");
                app.video.series = Some(bilibili::Series {
                    title: "Model navigation".into(),
                    base: "https://www.bilibili.com/video/BV1xx411c7mD".into(),
                    requested: 0,
                    current: Some(0),
                    parts: vec![
                        bilibili::Part {
                            number: 1,
                            title: "A".into(),
                            duration: 12.,
                            path: Some(a),
                        },
                        bilibili::Part {
                            number: 2,
                            title: "B".into(),
                            duration: 12.,
                            path: Some(b.clone()),
                        },
                    ],
                });
                app.handle(Action::SelectPart(1), window, cx);
                ensure!(
                    app.video.series.as_ref().and_then(|s| s.current) == Some(1)
                        && app.video.path.as_ref() == Some(&b),
                    "download blocks actual SelectPart handler"
                );
                self.active(app, cx)?;
                self.note(
                    "video episode selection works during real HTTP download",
                    app,
                );
                app.handle(Action::Page("listening".into()), window, cx);
            }
            2 => {
                if app.audio.audios.is_empty() || app.chat.is_loading() {
                    return Ok(false);
                }
                app.handle(Action::Audio("probe-audio-c".into()), window, cx);
                ensure!(
                    app.audio.current_id == "probe-audio-c",
                    "explicit audio selection blocked"
                );
                self.active(app, cx)?;
                self.note("explicit audio selection preserves model progress", app);
                app.handle(Action::AudioPractice("probe-group-b".into()), window, cx);
                ensure!(
                    app.audio.practice_id == "probe-group-b"
                        && app.audio.current_id == "probe-audio-b",
                    "group/audio switching blocked"
                );
                self.note("switching practice group and target audio works", app);
                app.begin_scoped(
                    TaskScope::Application,
                    "并行前台任务",
                    |_, cancel, _| {
                        let (tx, rx) = mpsc::channel::<()>();
                        while !cancel.load(Ordering::SeqCst) {
                            let _ = rx.recv_timeout(Duration::from_millis(20));
                        }
                        drop(tx);
                        anyhow::bail!("已取消")
                    },
                );
                self.foreground = app.tasks.cancel.clone();
                ensure!(
                    self.foreground.is_some(),
                    "foreground task cannot start while downloading"
                );
                ensure!(
                    app.tasks
                        .models
                        .progress
                        .as_ref()
                        .is_some_and(|p| p.downloaded > 0),
                    "foreground task cleared model progress"
                );
                app.handle(Action::Cancel, window, cx);
                ensure!(
                    self.foreground.as_ref().unwrap().load(Ordering::SeqCst),
                    "foreground cancel failed"
                );
                self.active(app, cx)?;
                self.note(
                    "foreground progress/cancel are independent from download",
                    app,
                );
            }
            3 => {
                if app.tasks.cancel.is_some() {
                    return Ok(false);
                }
                self.active(app, cx)?;
                self.fixture
                    .as_ref()
                    .unwrap()
                    .release
                    .store(true, Ordering::SeqCst);
            }
            4 => {
                if app.tasks.models.cancel.is_some() {
                    return Ok(false);
                }
                ensure!(
                    app.tasks
                        .models
                        .notice
                        .as_ref()
                        .is_some_and(|n| n.state == TaskState::Succeeded),
                    "download did not succeed"
                );
                ensure!(
                    app.lib
                        .root
                        .join("models/ggml-medium.en.bin")
                        .metadata()?
                        .len()
                        == 2097152,
                    "model install incomplete"
                );
                ensure!(
                    app.audio.current_id == "probe-audio-b"
                        && app.audio.practice_id == "probe-group-b"
                        && app.page == "listening",
                    "completion changed learning selection"
                );
                self.note("atomic model completion preserves current media", app);
                app.handle(Action::DismissModelNotice, window, cx);
                ensure!(
                    app.tasks.models.notice.is_none(),
                    "download notice cannot be dismissed"
                );
                self.start(app, window, cx, false)?;
            }
            5 => {
                if !app
                    .tasks
                    .models
                    .progress
                    .as_ref()
                    .is_some_and(|p| p.downloaded > 0)
                {
                    return Ok(false);
                }
                ensure!(
                    app.data.asr_models.contains("medium.en"),
                    "completed model availability was not refreshed"
                );
                ensure!(
                    app.audio.current_id == "probe-audio-b",
                    "asynchronous availability refresh changed current audio"
                );
                self.note(
                    "model availability refresh retains the learning selection",
                    app,
                );
                app.begin_scoped(
                    TaskScope::Application,
                    "与取消模型并行",
                    |_, cancel, _| {
                        let (tx, rx) = mpsc::channel::<()>();
                        while !cancel.load(Ordering::SeqCst) {
                            let _ = rx.recv_timeout(Duration::from_millis(20));
                        }
                        drop(tx);
                        anyhow::bail!("已取消")
                    },
                );
                self.foreground = app.tasks.cancel.clone();
                app.handle(Action::CancelModel, window, cx);
                ensure!(
                    self.token.as_ref().unwrap().load(Ordering::SeqCst)
                        && !self.foreground.as_ref().unwrap().load(Ordering::SeqCst),
                    "model cancel affected foreground"
                );
                self.note("model cancel leaves foreground task running", app);
            }
            6 => {
                if app.tasks.models.cancel.is_some() {
                    return Ok(false);
                }
                ensure!(
                    app.tasks
                        .models
                        .notice
                        .as_ref()
                        .is_some_and(|n| n.state == TaskState::Cancelled),
                    "cancel state missing"
                );
                ensure!(
                    app.lib
                        .root
                        .join("models/ggml-medium.en.bin")
                        .metadata()?
                        .len()
                        == 2097152
                        && !app.lib.root.join("models/ggml-medium.en.part").exists(),
                    "cancel overwrote model or leaked partial file"
                );
                self.note(
                    "cancel preserves installed model and removes partial download",
                    app,
                );
                app.handle(Action::Cancel, window, cx);
            }
            7 => {
                if app.tasks.cancel.is_some() {
                    return Ok(false);
                }
                self.start(app, window, cx, true)?;
            }
            8 => {
                if app.tasks.models.cancel.is_some() {
                    return Ok(false);
                }
                ensure!(
                    app.tasks
                        .models
                        .notice
                        .as_ref()
                        .is_some_and(|n| n.state == TaskState::Failed && n.detail.contains("403")),
                    "HTTP failure not visible"
                );
                ensure!(
                    app.page == "listening" && app.audio.current_id == "probe-audio-b",
                    "failure changed media"
                );
                self.note(
                    "HTTP failure releases download lane without changing media",
                    app,
                );
                app.handle(Action::DismissModelNotice, window, cx);
                self.start(app, window, cx, false)?;
            }
            9 => {
                if app.tasks.models.progress.is_none() {
                    return Ok(false);
                }
                let token = app.tasks.models.cancel.clone().unwrap();
                let models = std::mem::take(&mut app.tasks.models);
                drop(models);
                ensure!(
                    token.load(Ordering::SeqCst),
                    "model owner drop did not cancel download"
                );
                self.note("dropping app-owned model task cancels download", app);
                app.handle(Action::Page("library".into()), window, cx);
                app.handle(Action::Practice("probe-group-a".into()), window, cx);
            }
            10 => {
                if app.audio.practice_id != "probe-group-a" || app.audio.audios.len() != 3 {
                    return Ok(false);
                }
                app.audio.library.sort = 1;
                app.audio.library.batch = true;
                app.audio.checked = std::collections::BTreeSet::from([
                    "probe-audio-a".into(),
                    "probe-audio-c".into(),
                ]);
                app.handle(Action::MergeAudios, window, cx);
                ensure!(
                    app.tasks.cancel.is_some() && app.tasks.scope == TaskScope::Application,
                    "actual merge action did not start application task"
                );
                let token = app.tasks.cancel.clone().unwrap();
                app.handle(Action::Page("settings".into()), window, cx);
                ensure!(!token.load(Ordering::SeqCst), "page switch cancelled merge");
                self.note("actual selected-audio merge continues across modules", app);
            }
            11 => {
                if app.tasks.cancel.is_some() || app.audio.audios.len() != 4 {
                    return Ok(false);
                }
                self.merged_id = app.audio.library.selected.clone();
                ensure!(
                    app.audio.library.merges.source_count(&self.merged_id) == 2
                        && app.audio.checked.is_empty()
                        && !app.audio.library.batch,
                    "merge completion did not select study version or clear selection"
                );
                app.handle(Action::Page("listening".into()), window, cx);
                app.handle(Action::AudioPractice("probe-group-a".into()), window, cx);
                ensure!(
                    app.audio.current_id == self.merged_id,
                    "group study did not prefer merged version"
                );
                ensure!(
                    app.audio
                        .library
                        .merges
                        .learning(&app.audio.audios, "probe-group-a")
                        .len()
                        == 1,
                    "learning catalog still includes originals"
                );
                self.note(
                    "merge completion and group learning select merged version",
                    app,
                );
                app.handle(Action::Page("settings".into()), window, cx);
                app.handle(Action::Page("listening".into()), window, cx);
            }
            12 => {
                if app.loads.restore_pending {
                    return Ok(false);
                }
                ensure!(
                    app.audio.current_id == self.merged_id,
                    "cross-module restore returned to original"
                );
                app.handle(Action::Audio("probe-audio-a".into()), window, cx);
                ensure!(
                    app.audio.current_id == self.merged_id,
                    "explicit stale source ID bypassed merged study version"
                );
                self.note(
                    "restoring and selecting an original resolves to merged learning version",
                    app,
                );
                app.handle(Action::LibraryShowSources, window, cx);
            }
            13 => {
                ensure!(
                    app.audio.library.show_sources,
                    "show-originals action failed"
                );
                ensure!(
                    app.audio
                        .library
                        .visible(&app.audio.audios, "probe-group-a", "")
                        .len()
                        == 3
                        && app
                            .audio
                            .library
                            .merges
                            .learning(&app.audio.audios, "probe-group-a")
                            .len()
                            == 1,
                    "show-originals changed study queue"
                );
                app.handle(Action::LibraryShowSources, window, cx);
                ensure!(
                    app.audio
                        .library
                        .visible(&app.audio.audios, "probe-group-a", "")
                        .len()
                        == 1,
                    "hide-originals action failed"
                );
                self.note(
                    "originals remain manageable while learning queue stays deduplicated",
                    app,
                );
                self.report(true, None)?;
                return Ok(true);
            }
            _ => {}
        }
        self.stage += 1;
        Ok(false)
    }
}
impl AppView {
    pub(super) fn poll_model_probe(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(mut probe) = self.model_probe.take() else {
            return false;
        };
        match probe.step(self, window, cx) {
            Ok(true) => {
                window.remove_window();
                return true;
            }
            Err(e) => {
                let _ = probe.report(false, Some(format!("{e:#}")));
                window.remove_window();
                return true;
            }
            Ok(false) => {}
        }
        self.model_probe = Some(probe);
        false
    }
}
