//! Held loopback ASR/chat responses through actual hidden AppView actions.
use super::*;
use clarora_gpui::audio_batch::Mode;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex, atomic::AtomicBool},
    thread,
};

struct Fixture {
    host: String,
    waiting: Arc<AtomicBool>,
    release: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    error: Arc<Mutex<Option<String>>>,
}
impl Fixture {
    fn new() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let host = format!("http://{}", listener.local_addr()?);
        let waiting = Arc::new(AtomicBool::new(false));
        let release = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let (arrived, go, end) = (waiting.clone(), release.clone(), stop.clone());
        let error = Arc::new(Mutex::new(None));
        let error_out = error.clone();
        let worker = thread::spawn(move || {
            while !end.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    Ok(v) => v,
                    Err(_) => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                };
                // Accepted sockets can inherit O_NONBLOCK on macOS; read timeouts
                // only apply after returning the fixture stream to blocking I/O.
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                let result = (|| -> Result<()> {
                    let mut input = vec![];
                    let mut block = [0; 16384];
                    let split = loop {
                        let n = stream.read(&mut block)?;
                        ensure!(n > 0, "fixture request closed");
                        input.extend_from_slice(&block[..n]);
                        ensure!(input.len() < 4 * 1024 * 1024, "fixture request too large");
                        if let Some(i) = input.windows(4).position(|w| w == b"\r\n\r\n") {
                            break i + 4;
                        }
                    };
                    let header = String::from_utf8_lossy(&input[..split]).into_owned();
                    let length = header
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|n| n.trim().parse::<usize>().ok())
                        })
                        .context("request length")?;
                    while input.len() < split + length {
                        let n = stream.read(&mut block)?;
                        ensure!(n > 0, "fixture body closed");
                        input.extend_from_slice(&block[..n]);
                    }
                    let (body, kind) = if header
                        .lines()
                        .next()
                        .unwrap_or("")
                        .contains("audio/transcriptions")
                    {
                        (json!({"segments":[{"start":0.,"end":1.,"text":"Fresh background English."}]}).to_string(),"application/json")
                    } else {
                        let request: Value = serde_json::from_slice(&input[split..split + length])?;
                        let prompt = request["messages"][0]["content"]
                            .as_str()
                            .context("translation prompt")?;
                        let a = prompt.find("[{").context("source array")?;
                        let b = prompt.rfind(']').context("source array end")?;
                        let source: Vec<Value> = serde_json::from_str(&prompt[a..=b])?;
                        let translated = source
                            .iter()
                            .map(|c| json!({"id":c["id"],"text":"后台中文译文"}))
                            .collect::<Vec<_>>();
                        (
                            format!(
                                "data: {}\n\ndata: [DONE]\n\n",
                                json!({"choices":[{"delta":{"content":serde_json::to_string(&translated)?},"finish_reason":"stop"}]})
                            ),
                            "text/event-stream",
                        )
                    };
                    arrived.store(true, Ordering::SeqCst);
                    while !go.load(Ordering::SeqCst) && !end.load(Ordering::SeqCst) {
                        thread::sleep(Duration::from_millis(5));
                    }
                    if !end.load(Ordering::SeqCst) {
                        let _ = write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                    }
                    Ok(())
                })();
                if let Err(error) = result {
                    *error_out.lock().unwrap() = Some(format!("{error:#}"));
                }
            }
        });
        Ok(Self {
            host,
            waiting,
            release,
            stop,
            worker: Some(worker),
            error,
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

pub(super) struct Flow {
    stage: usize,
    key: String,
    group: String,
    fixture: Option<Fixture>,
    token: Option<Cancel>,
    before: String,
    other_before: String,
    cases: Vec<Value>,
    entered: Instant,
}
impl Flow {
    pub(super) fn new() -> Self {
        Self {
            stage: 0,
            key: String::new(),
            group: String::new(),
            fixture: None,
            token: None,
            before: String::new(),
            other_before: String::new(),
            cases: vec![],
            entered: Instant::now(),
        }
    }
    fn row(app: &AppView, key: &str) -> Result<Row> {
        app.lib
            .rows(
                "SELECT * FROM listening_audios WHERE id=?",
                &[key.to_string().into()],
            )?
            .pop()
            .context("audio fixture row")
    }
    fn source(app: &AppView, key: &str) -> Result<String> {
        Ok(fs::read_to_string(text(
            &Self::row(app, key)?,
            "subtitle_uri",
        ))?)
    }
    fn fixture(&mut self, app: &AppView) -> Result<()> {
        self.fixture = Some(Fixture::new()?);
        app.lib.set_setting("ai_config",&json!({"engine":"compatible","base_url":self.fixture.as_ref().unwrap().host,"model":"fixture","asr_base_url":self.fixture.as_ref().unwrap().host,"asr_model":"fixture"}).to_string())?;
        Ok(())
    }
    fn waiting(&self) -> bool {
        self.fixture
            .as_ref()
            .unwrap()
            .waiting
            .load(Ordering::SeqCst)
    }
    fn release(&self) {
        self.fixture
            .as_ref()
            .unwrap()
            .release
            .store(true, Ordering::SeqCst);
    }
    fn note(&mut self, label: &str, app: &AppView) {
        self.cases.push(json!({"case":label,"passed":true,"page":app.page,"audio":app.audio.current_id,"foreground":app.tasks.foreground.is_running(),"background_audio":app.tasks.audio_subtitles.lane.is_running()}));
    }
    fn foreground(app: &mut AppView) {
        app.begin_scoped(
            TaskScope::Application,
            "独立前台任务",
            |_, cancel, _| {
                let (tx, rx) = mpsc::channel::<()>();
                while !cancel.load(Ordering::SeqCst) {
                    let _ = rx.recv_timeout(Duration::from_millis(20));
                }
                drop(tx);
                anyhow::bail!("已取消")
            },
        );
    }
    pub(super) fn step(
        &mut self,
        app: &mut AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Result<Option<Vec<Value>>> {
        let detail = app
            .tasks
            .audio_subtitles
            .notice
            .as_ref()
            .map(|n| n.detail.clone())
            .unwrap_or_default();
        fs::write(
            app.lib.root.join("audio-subtitles-progress.json"),
            serde_json::to_vec_pretty(
                &json!({"stage":self.stage,"detail":detail,"audio":app.audio.current_id,"running":app.tasks.audio_subtitles.lane.is_running(),"cases":self.cases}),
            )?,
        )?;
        if let Some(error) = self
            .fixture
            .as_ref()
            .and_then(|f| f.error.lock().unwrap().clone())
        {
            anyhow::bail!("audio HTTP fixture stage {}: {error}", self.stage);
        }
        ensure!(
            self.entered.elapsed() < Duration::from_secs(15),
            "audio background flow stage {} timed out: {detail}",
            self.stage
        );
        if [1, 4, 6, 8, 10].contains(&self.stage) && !app.tasks.audio_subtitles.lane.is_running() {
            anyhow::bail!(
                "audio task ended before fixture request at stage {}: {detail}",
                self.stage
            );
        }
        match self.stage {
            0 => {
                self.key = app.audio.current_id.clone();
                self.group = app.audio.practice_id.clone();
                self.other_before = Self::source(app, "probe-audio-b")?;
                self.fixture(app)?;
                app.handle(Action::Transcribe, window, cx);
                self.token = app.tasks.audio_subtitles.lane.cancel_token().cloned();
                ensure!(
                    self.token.is_some() && !app.tasks.foreground.is_running(),
                    "audio ASR occupies foreground lane"
                );
                self.note("single ASR uses independent background lane", app);
            }
            1 => {
                if !self.waiting() {
                    return Ok(None);
                }
                app.handle(Action::AudioPractice("probe-group-b".into()), window, cx);
                ensure!(
                    app.audio.current_id == "probe-audio-b"
                        && !self.token.as_ref().unwrap().load(Ordering::SeqCst),
                    "audio/group switch blocked or cancelled ASR"
                );
                ensure!(
                    !app.tasks
                        .audio_subtitles
                        .working("probe-audio-b", Mode::English),
                    "new audio falsely marked generating"
                );
                app.handle(Action::AudioBlind(true), window, cx);
                app.handle(Action::AudioBlind(false), window, cx);
                app.handle(Action::Page("settings".into()), window, cx);
                app.handle(Action::Page("listening".into()), window, cx);
                Self::foreground(app);
                app.handle(Action::Cancel, window, cx);
                ensure!(
                    !self.token.as_ref().unwrap().load(Ordering::SeqCst),
                    "foreground cancel cancelled ASR"
                );
                self.note(
                    "selection/blind mode/page changes and foreground cancel leave ASR running",
                    app,
                );
                self.release();
            }
            2 => {
                if app.tasks.audio_subtitles.lane.is_running()
                    || app.tasks.foreground.is_running()
                    || app.loads.restore_pending
                {
                    return Ok(None);
                }
                ensure!(
                    app.tasks
                        .audio_subtitles
                        .notice
                        .as_ref()
                        .is_some_and(|n| n.state == TaskState::Succeeded),
                    "background ASR failed"
                );
                ensure!(
                    Self::source(app, &self.key)?.contains("Fresh background English.")
                        && Self::source(app, "probe-audio-b")? == self.other_before,
                    "ASR result saved to wrong audio"
                );
                ensure!(
                    app.audio.current_id == "probe-audio-b" && app.page == "listening",
                    "completion switched back to source"
                );
                self.note(
                    "ASR completion updates only captured source and preserves current selection",
                    app,
                );
                self.before = text(&Self::row(app, &self.key)?, "subtitle_uri");
                app.handle(Action::Audio(self.key.clone()), window, cx);
            }
            3 => {
                if app.audio.cues.is_empty() {
                    return Ok(None);
                }
                self.fixture(app)?;
                app.handle(Action::Translate, window, cx);
                self.token = app.tasks.audio_subtitles.lane.cancel_token().cloned();
                ensure!(self.token.is_some(), "translation did not start");
            }
            4 => {
                if !self.waiting() {
                    return Ok(None);
                }
                app.handle(Action::AudioPractice("probe-group-b".into()), window, cx);
                Self::foreground(app);
                let foreground = app
                    .tasks
                    .foreground
                    .cancel_token()
                    .cloned()
                    .context("foreground token")?;
                app.handle(Action::CancelAudioSubtitles, window, cx);
                ensure!(
                    self.token.as_ref().unwrap().load(Ordering::SeqCst)
                        && !foreground.load(Ordering::SeqCst),
                    "subtitle cancel affected unrelated task"
                );
                app.handle(Action::Cancel, window, cx);
                self.note(
                    "translation remains navigable and has independent cancellation",
                    app,
                );
            }
            5 => {
                if app.tasks.audio_subtitles.lane.is_running() || app.tasks.foreground.is_running()
                {
                    return Ok(None);
                }
                ensure!(
                    app.tasks
                        .audio_subtitles
                        .notice
                        .as_ref()
                        .is_some_and(|n| n.state == TaskState::Cancelled)
                        && text(&Self::row(app, &self.key)?, "subtitle_uri") == self.before,
                    "cancelled translation overwrote original subtitles"
                );
                self.note(
                    "cancelled translation preserves original subtitle reference",
                    app,
                );
                app.lib.db()?.execute(
                    "UPDATE listening_audios SET subtitle_uri='' WHERE id='probe-audio-b'",
                    [],
                )?;
                self.fixture(app)?;
                app.handle(Action::AudioGroupSubtitles(Mode::English), window, cx);
                self.token = app.tasks.audio_subtitles.lane.cancel_token().cloned();
                ensure!(
                    self.token.is_some(),
                    "group task did not use audio background lane"
                );
            }
            6 => {
                if !self.waiting() {
                    return Ok(None);
                }
                app.handle(Action::AudioPractice(self.group.clone()), window, cx);
                ensure!(
                    app.audio.current_id == self.key
                        && !self.token.as_ref().unwrap().load(Ordering::SeqCst),
                    "group task blocks group selection"
                );
                self.release();
                self.note(
                    "group subtitles keep captured group while another group plays",
                    app,
                );
            }
            7 => {
                if app.tasks.audio_subtitles.lane.is_running() {
                    return Ok(None);
                }
                ensure!(
                    Self::source(app, "probe-audio-b")?.contains("Fresh background English.")
                        && app.audio.current_id == self.key,
                    "group completion changed active material"
                );
                self.note(
                    "group result saved to initial group without navigation reset",
                    app,
                );
                self.fixture(app)?;
                app.handle(Action::Transcribe, window, cx);
            }
            8 => {
                if !self.waiting() {
                    return Ok(None);
                }
                fs::write(
                    text(&Self::row(app, &self.key)?, "subtitle_uri"),
                    "1\n00:00:00,000 --> 00:00:01,000\nNewer manual subtitles.\n手工新字幕。\n",
                )?;
                self.release();
            }
            9 => {
                if app.tasks.audio_subtitles.lane.is_running() {
                    return Ok(None);
                }
                ensure!(
                    app.tasks
                        .audio_subtitles
                        .notice
                        .as_ref()
                        .is_some_and(|n| n.state == TaskState::Failed)
                        && Self::source(app, &self.key)?.contains("Newer manual subtitles."),
                    "late ASR overwrote newer manual subtitles"
                );
                self.note("late ASR refuses concurrent subtitle changes", app);
                app.handle(Action::DismissAudioSubtitles, window, cx);
                ensure!(
                    app.tasks.audio_subtitles.notice.is_none(),
                    "audio notice cannot be dismissed"
                );
                self.fixture(app)?;
                app.handle(Action::Transcribe, window, cx);
            }
            10 => {
                if !self.waiting() {
                    return Ok(None);
                }
                let token = app
                    .tasks
                    .audio_subtitles
                    .lane
                    .cancel_token()
                    .cloned()
                    .context("close cancel token")?;
                drop(std::mem::take(&mut app.tasks.audio_subtitles));
                ensure!(
                    token.load(Ordering::SeqCst),
                    "audio task owner drop does not cancel"
                );
                self.note("app-owned audio worker is cancelled on owner drop", app);
                return Ok(Some(std::mem::take(&mut self.cases)));
            }
            _ => {}
        }
        self.stage += 1;
        self.entered = Instant::now();
        Ok(None)
    }
}
