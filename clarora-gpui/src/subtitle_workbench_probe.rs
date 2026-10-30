//! Actual hidden AppView / clipboard / loopback-AI workflow for subtitle tools.
//! Uses isolated data and fake credentials; not a physical-input or visual review.
use super::*;
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{Arc, Mutex, atomic::AtomicBool},
    thread,
};

const ENGLISH: &str =
    "The bread is baked fresh every morning.\n\nWe open at eight and close at five.";
const OTHER: &str = "This is another video with a different transcript.";
const DRAFT: &str = "Keep this unsent question draft.";
const QUESTION: &str = "What is the speaker's main point?";
const ANSWER: &str = "A concise English transcript summary.";
const PRIVATE: &str = "PRIVATE-video-title";
fn path(lib: &Library, other: bool) -> PathBuf {
    lib.root.join("media").join(if other {
        "PRIVATE-video-title-B.mp4"
    } else {
        "PRIVATE-video-title-A.mp4"
    })
}
fn cues(other: bool) -> Vec<Cue> {
    if other {
        vec![Cue {
            start: 0.,
            end: 8.,
            text: format!("{OTHER}\n另一个视频的中文字幕。"),
        }]
    } else {
        vec![
            Cue {
                start: 0.,
                end: 4.,
                text: "The bread is baked fresh every morning.\n面包每天早晨新鲜烘焙。".into(),
            },
            Cue {
                start: 4.,
                end: 8.,
                text: "We open at eight and close at five.\n我们八点开门，五点关门。".into(),
            },
        ]
    }
}
pub(super) fn prepare(out: &Path) -> Result<Library> {
    fs::create_dir_all(out)?;
    let lib = Library::open(out.join("library"))?;
    process::run(
        &process::tool("ffmpeg")?,
        &[
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=15",
            "-t",
            "12",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
            "-an",
            "-y",
        ]
        .into_iter()
        .map(String::from)
        .chain([path(&lib, false).to_string_lossy().into_owned()])
        .collect::<Vec<_>>(),
        &process::cancel_token(),
        Duration::from_secs(30),
    )?;
    fs::copy(path(&lib, false), path(&lib, true))?;
    let mut collection = video_library::Collection::load(&lib)?;
    let ids = collection.add(&[path(&lib, false), path(&lib, true)], None)?;
    collection.select(&ids[0])?;
    collection.save(&lib)?;
    for other in [false, true] {
        video_subtitles::save(
            &lib,
            &path(&lib, other),
            &cues(other),
            &process::cancel_token(),
        )?;
    }
    lib.set_setting("last_page", "video")?;
    Ok(lib)
}

struct Server {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<Value>>>,
    error: Arc<Mutex<Option<String>>>,
    handle: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(vec![]));
        let error = Arc::new(Mutex::new(None));
        let (stopped, seen, failure) = (stop.clone(), requests.clone(), error.clone());
        let handle = thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                let (mut socket, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => {
                        *failure.lock().unwrap() = Some(error.to_string());
                        break;
                    }
                };
                if stopped.load(Ordering::SeqCst) {
                    break;
                }
                let result = (|| -> Result<()> {
                    socket.set_read_timeout(Some(Duration::from_secs(3)))?;
                    socket.set_write_timeout(Some(Duration::from_secs(3)))?;
                    let mut input = vec![];
                    let mut chunk = [0_u8; 4096];
                    let split = loop {
                        let count = socket.read(&mut chunk)?;
                        ensure!(count > 0, "fixture request ended before header");
                        input.extend_from_slice(&chunk[..count]);
                        ensure!(input.len() <= 64 * 1024, "fixture header exceeded bound");
                        if let Some(index) = input.windows(4).position(|part| part == b"\r\n\r\n") {
                            break index + 4;
                        }
                    };
                    let header = std::str::from_utf8(&input[..split])?;
                    ensure!(
                        header
                            .lines()
                            .next()
                            .is_some_and(|line| line.starts_with("POST /v1/chat/completions ")),
                        "not a chat completion request"
                    );
                    let length = header
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())
                                .flatten()
                        })
                        .context("fixture missing request length")?;
                    ensure!(length < 1024 * 1024, "fixture request exceeded bound");
                    while input.len() < split + length {
                        let count = socket.read(&mut chunk)?;
                        ensure!(count > 0, "fixture request body ended early");
                        input.extend_from_slice(&chunk[..count]);
                    }
                    seen.lock()
                        .unwrap()
                        .push(serde_json::from_slice(&input[split..split + length])?);
                    let payload = format!(
                        "data: {}\n\ndata: [DONE]\n\n",
                        json!({"choices":[{"delta":{"content":ANSWER},"finish_reason":"stop"}]})
                    );
                    write!(
                        socket,
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        payload.len(),
                        payload
                    )?;
                    socket.flush()?;
                    Ok(())
                })();
                if let Err(error) = result {
                    *failure.lock().unwrap() = Some(format!("{error:#}"));
                }
            }
        });
        Ok(Self {
            address,
            stop,
            requests,
            error,
            handle: Some(handle),
        })
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect_timeout(&self.address, Duration::from_millis(100));
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

struct MediaGuard {
    position: f64,
    path: String,
    paused: bool,
    repeat_three: bool,
    request_id: u64,
    current_play: u32,
}

#[derive(Default)]
struct LayoutEvidence {
    result: Option<std::result::Result<Vec<Value>, String>>,
    draw_completed: bool,
}

/// The probe tick is outside GPUI's layout/prepaint phase. Keep all test-only
/// element construction and measurement inside a real view's render callback.
struct LayoutView {
    app: Entity<AppView>,
    evidence: Arc<Mutex<LayoutEvidence>>,
}
impl Render for LayoutView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let needs_measurement = self.evidence.lock().unwrap().result.is_none();
        if needs_measurement {
            let result = self
                .app
                .update(cx, |app, app_cx| {
                    Probe::measure_toolbar(app, window, app_cx)
                })
                .map_err(|error| format!("{error:#}"));
            self.evidence.lock().unwrap().result = Some(result);
        }
        div().size_full().child(self.app.clone())
    }
}

pub(super) struct Probe {
    out: PathBuf,
    started: Instant,
    stage: usize,
    cases: Vec<Value>,
    server: Option<Server>,
    media: Option<MediaGuard>,
    reference: Option<clarora_gpui::assistant::Reference>,
    theme_before: Option<(ThemeMode, bool)>,
    layouts: Vec<Value>,
    layout_evidence: Arc<Mutex<LayoutEvidence>>,
}
impl Probe {
    pub(super) fn new(out: &Path) -> Self {
        Self {
            out: out.into(),
            started: Instant::now(),
            stage: 0,
            cases: vec![],
            server: None,
            media: None,
            reference: None,
            theme_before: None,
            layouts: vec![],
            layout_evidence: Arc::new(Mutex::new(LayoutEvidence::default())),
        }
    }
    fn report(&self, passed: Option<bool>, error: Option<String>) -> Result<()> {
        fs::write(
            self.out.join("subtitle-workbench-report.json"),
            serde_json::to_vec_pretty(&json!({
                "passed":passed,"error":error,"stage":self.stage,"cases":self.cases,
                "hidden_native_window":true,"actual_appview_handlers":true,"actual_clipboard_restored":true,
                "actual_libmpv":true,"physical_input_visual_verified":false,"independent_review":false,
            "requests":self.server.as_ref().map(|s|s.requests.lock().unwrap().clone()).unwrap_or_default(),
            "toolbar_layouts":self.layouts,
                "elapsed_ms":self.started.elapsed().as_millis()
            }))?,
        )?;
        Ok(())
    }
    fn note(&mut self, name: &str, app: &AppView) -> Result<()> {
        self.cases.push(json!({"case":name,"passed":true,"page":app.page,"english_view":app.video.transcript_open,
            "reference_count":app.assistant.references.len(),"elapsed_ms":self.started.elapsed().as_millis()}));
        self.report(None, None)
    }
    fn advance(&mut self) {
        self.stage += 1;
    }
    fn measure_toolbar(
        app: &AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Result<Vec<Value>> {
        let original = cx.theme().mode;
        let result = (|| -> Result<Vec<Value>> {
            let mut measurements = vec![];
            for mode in [ThemeMode::Light, ThemeMode::Dark] {
                Theme::change(mode, None, cx);
                design::apply(cx);
                for width in [300., 420.] {
                    let mut toolbar = app.video_subtitle_toolbar(cx);
                    let measured = toolbar.layout_as_root(
                        size(
                            AvailableSpace::Definite(px(width)),
                            AvailableSpace::MaxContent,
                        ),
                        window,
                        cx,
                    );
                    let measured_width = f32::from(measured.width);
                    let measured_height = f32::from(measured.height);
                    ensure!(
                        measured_width <= width + 1.
                            && measured_height > 50.
                            && measured_height <= 180.,
                        "toolbar layout overflow: {mode:?}, width={width}, measured={measured_width}x{measured_height}"
                    );
                    measurements.push(json!({"theme":format!("{mode:?}"),"available_width_bound":width,"intrinsic_width":measured_width,"height":measured_height}));
                }
            }
            Ok(measurements)
        })();
        Theme::change(original, None, cx);
        design::apply(cx);
        result
    }
    fn draw(window: &Window, cx: &mut Context<AppView>) {
        let handle = window.window_handle();
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| {
                window.draw(cx).clear();
            });
        });
    }
    fn draw_workbench(&self, window: &Window, cx: &mut Context<AppView>) {
        let handle = window.window_handle();
        let app = cx.entity();
        let evidence = self.layout_evidence.clone();
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| {
                window.replace_root(cx, |window, cx| {
                    let wrapper = cx.new(|_| LayoutView {
                        app,
                        evidence: evidence.clone(),
                    });
                    Root::new(wrapper, window, cx)
                });
                window.draw(cx).clear();
                evidence.lock().unwrap().draw_completed = true;
            });
        });
    }
    fn no_request(&self, app: &AppView) -> Result<()> {
        ensure!(
            app.tasks.cancel.is_none() && !app.chat.running,
            "preparing a transcript started an app task"
        );
        ensure!(
            self.server
                .as_ref()
                .unwrap()
                .requests
                .lock()
                .unwrap()
                .is_empty(),
            "preparing a transcript sent a network request"
        );
        Ok(())
    }
    fn stable(&self, app: &AppView) -> Result<()> {
        let guard = self.media.as_ref().context("missing media guard")?;
        let state = app.media.player.snapshot();
        ensure!(
            state.path == guard.path
                && (state.position - guard.position).abs() < 0.03
                && state.paused == guard.paused,
            "transcript action changed playback path/position/pause"
        );
        ensure!(
            app.video.repeat_three == guard.repeat_three,
            "transcript action changed triple preference"
        );
        ensure!(
            state
                .sentence_repeat
                .as_ref()
                .is_some_and(|repeat| repeat.request_id == guard.request_id
                    && repeat.current_play == guard.current_play),
            "transcript action changed repeat request/count"
        );
        Ok(())
    }
    fn copy_and_restore(
        app: &mut AppView,
        expected: &str,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Result<()> {
        let original = cx.read_from_clipboard();
        app.handle(Action::VideoCopyTranscript, window, cx);
        let actual = cx.read_from_clipboard().and_then(|item| item.text());
        if let Some(original) = original.clone() {
            cx.write_to_clipboard(original);
        } else {
            cx.write_to_clipboard(ClipboardItem::new_string(String::new()));
        }
        ensure!(
            actual.as_deref() == Some(expected),
            "clipboard was not the complete plain English transcript"
        );
        if let Some(original) = original {
            ensure!(
                cx.read_from_clipboard() == Some(original),
                "original clipboard was not restored"
            );
        } else {
            ensure!(
                cx.read_from_clipboard()
                    .and_then(|item| item.text())
                    .as_deref()
                    == Some(""),
                "empty clipboard was not restored"
            );
        }
        Ok(())
    }
    fn step(
        &mut self,
        app: &mut AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Result<bool> {
        ensure!(
            self.started.elapsed() < Duration::from_secs(45),
            "subtitle workbench probe timed out at stage {}",
            self.stage
        );
        if let Some(error) = self
            .server
            .as_ref()
            .and_then(|s| s.error.lock().unwrap().clone())
        {
            anyhow::bail!("fixture: {error}");
        }
        match self.stage {
            0 => {
                let server = Server::new()?;
                ai::save_shared(
                    &app.lib,
                    ai::Config {
                        base_url: format!("http://{}/v1", server.address),
                        model: "subtitle-fixture".into(),
                        engine: "compatible".into(),
                        ..Default::default()
                    },
                    "fixture-subtitle-key",
                )?;
                self.server = Some(server);
                app.media.player = Player::without_audio_device();
                app.media.preloader = Player::without_audio_device();
                app.handle(Action::Page("video".into()), window, cx);
                app.video.panel_width = 300.;
                app.open_at(path(&app.lib, false), 4., true);
                // A user can type in the global composer only after opening it
                // and completing its asynchronous context load.
                app.open_assistant(window, cx);
                self.advance();
            }
            1 => {
                if app.chat.is_loading()
                    || app.chat.context != clarora_gpui::assistant::CONTEXT
                    || app.chat_context_key(cx) != clarora_gpui::assistant::CONTEXT
                {
                    return Ok(false);
                }
                ensure!(
                    app.assistant_open && app.assistant.enabled,
                    "global composer was not open before preparing its draft"
                );
                let state = app.media.player.snapshot();
                if state.path != path(&app.lib, false).to_string_lossy()
                    || state.duration < 11.9
                    || !state.paused
                {
                    return Ok(false);
                }
                app.video.set_cues(cues(false));
                app.start_video_sentence_repeat(Some(1), true)?;
                app.media.player.set("pause", "yes");
                self.advance();
            }
            2 => {
                let state = app.media.player.snapshot();
                let Some(repeat) = state.sentence_repeat.as_ref() else {
                    return Ok(false);
                };
                if !state.paused
                    || (state.position - 4.).abs() > 0.03
                    || app
                        .video
                        .sentence_repeat
                        .as_ref()
                        .is_none_or(|session| session.request_id != repeat.request_id)
                {
                    return Ok(false);
                }
                self.media = Some(MediaGuard {
                    position: state.position,
                    path: state.path.clone(),
                    paused: state.paused,
                    repeat_three: app.video.repeat_three,
                    request_id: repeat.request_id,
                    current_play: repeat.current_play,
                });
                app.handle(Action::VideoTranscriptMode(true), window, cx);
                ensure!(
                    app.video.transcript_open && app.video.tab == 0,
                    "English mode did not select the subtitle workbench"
                );
                ensure!(
                    app.video.transcript.text == ENGLISH
                        && app.video.transcript.cue_count == 2
                        && app.video.transcript.word_count == 15
                        && app.video.transcript.character_count == ENGLISH.chars().count(),
                    "cached English text/statistics changed"
                );
                self.theme_before = Some((cx.theme().mode, app.dark));
                Theme::change(ThemeMode::Dark, Some(window), cx);
                app.dark = true;
                design::apply(cx);
                self.stable(app)?;
                self.draw_workbench(window, cx);
                self.advance();
            }
            3 => {
                {
                    let evidence = self.layout_evidence.lock().unwrap();
                    if !evidence.draw_completed {
                        return Ok(false);
                    }
                    self.layouts = evidence
                        .result
                        .as_ref()
                        .context("workbench draw did not measure its toolbar")?
                        .clone()
                        .map_err(anyhow::Error::msg)?;
                }
                self.note(
                    "300px workbench selects and draws the in-place English view",
                    app,
                )?;
                self.stable(app)?;
                Self::copy_and_restore(app, ENGLISH, window, cx)?;
                self.note("copy uses full English text and restores clipboard", app)?;
                if let Some((mode, dark)) = self.theme_before.take() {
                    Theme::change(mode, Some(window), cx);
                    app.dark = dark;
                    design::apply(cx);
                }
                app.handle(Action::VideoTranscriptMode(false), window, cx);
                ensure!(
                    !app.video.transcript_open && app.video.tab == 0,
                    "cannot return to sentence view"
                );
                self.stable(app)?;
                Self::draw(window, cx);
                self.note("return to sentence view preserves repeat and pause", app)?;
                self.advance();
            }
            4 => {
                ensure!(
                    app.assistant_open
                        && !app.chat.is_loading()
                        && app.chat.context == clarora_gpui::assistant::CONTEXT,
                    "composer is not ready for user input"
                );
                app.set("question", DRAFT, window, cx);
                // Match a real sequence: enter a draft in the visible composer,
                // close the sidebar, then quote the complete English transcript.
                app.handle(Action::Assistant, window, cx);
                ensure!(
                    !app.assistant_open,
                    "assistant did not close before quoting"
                );
                app.handle(Action::VideoAskTranscript, window, cx);
                ensure!(
                    app.assistant_open && app.assistant.enabled && app.assistant.expanded,
                    "whole-English quote did not open assistant preview"
                );
                ensure!(
                    app.assistant.references.len() == 1
                        && app.assistant.references[0].text == ENGLISH,
                    "quote was not the same complete English as clipboard"
                );
                ensure!(
                    app.value("question", cx) == DRAFT,
                    "quote replaced unsent question draft"
                );
                self.reference = app.assistant.references.first().cloned();
                self.no_request(app)?;
                self.stable(app)?;
                self.note(
                    "whole-English quote opens preview without sending or replacing the question",
                    app,
                )?;
                self.advance();
            }
            5 => {
                app.handle(Action::VideoAskTranscript, window, cx);
                ensure!(
                    app.assistant.references.len() == 1,
                    "repeat quote was not deduplicated"
                );
                self.no_request(app)?;
                self.stable(app)?;
                self.note(
                    "repeated quote is deduplicated and does not change playback",
                    app,
                )?;
                let other = app
                    .video
                    .collection
                    .items
                    .iter()
                    .find(|item| item.path == path(&app.lib, true))
                    .context("second fixture video missing")?
                    .id
                    .clone();
                app.handle(Action::VideoSelect(other), window, cx);
                self.advance();
            }
            6 => {
                let state = app.media.player.snapshot();
                if state.path != path(&app.lib, true).to_string_lossy()
                    || app.video.transcript.text != OTHER
                {
                    return Ok(false);
                }
                ensure!(
                    app.assistant.references == vec![self.reference.clone().unwrap()]
                        && app.value("question", cx) == DRAFT,
                    "changing videos replaced pending English reference or draft"
                );
                self.no_request(app)?;
                self.note(
                    "switching to another real video keeps the fixed pending reference",
                    app,
                )?;
                self.advance();
            }
            7 => {
                app.video.set_cues(vec![Cue {
                    start: 0.,
                    end: 4.,
                    text: "只有中文的字幕。".into(),
                }]);
                let refs = app.assistant.references.clone();
                app.handle(Action::VideoAskTranscript, window, cx);
                ensure!(
                    app.assistant.references == refs && app.value("question", cx) == DRAFT,
                    "empty English changed references or draft"
                );
                let clipboard = cx.read_from_clipboard();
                app.handle(Action::VideoCopyTranscript, window, cx);
                ensure!(
                    cx.read_from_clipboard() == clipboard,
                    "empty English overwrote clipboard"
                );
                self.no_request(app)?;
                self.note(
                    "empty-English quote/copy preserve existing references, draft and clipboard",
                    app,
                )?;
                self.advance();
            }
            8 => {
                app.video.set_cues(vec![Cue {
                    start: 0.,
                    end: 4.,
                    text: "Longword ".repeat(15010),
                }]);
                ensure!(
                    app.video.transcript.text.chars().count() > 120000,
                    "oversize fixture too small"
                );
                let refs = app.assistant.references.clone();
                app.handle(Action::VideoAskTranscript, window, cx);
                ensure!(
                    app.assistant.references == refs && app.value("question", cx) == DRAFT,
                    "oversized quote lost references or draft"
                );
                self.no_request(app)?;
                self.note(
                    "oversized transcript is rejected without truncating or changing the draft",
                    app,
                )?;
                self.advance();
            }
            9 => {
                app.video.set_cues(cues(true));
                app.assistant.references = vec![
                    self.reference.clone().unwrap(),
                    clarora_gpui::assistant::Reference {
                        text: "Reference two.".into(),
                    },
                    clarora_gpui::assistant::Reference {
                        text: "Reference three.".into(),
                    },
                    clarora_gpui::assistant::Reference {
                        text: "Reference four.".into(),
                    },
                ];
                app.persist_assistant_references()?;
                let refs = app.assistant.references.clone();
                app.handle(Action::VideoAskTranscript, window, cx);
                ensure!(
                    app.assistant.references == refs && app.value("question", cx) == DRAFT,
                    "fifth quote changed existing references or draft"
                );
                self.no_request(app)?;
                self.note(
                    "full reference slots are preserved when adding a fifth quote fails",
                    app,
                )?;
                app.assistant.references = vec![self.reference.clone().unwrap()];
                app.persist_assistant_references()?;
                self.advance();
            }
            10 => {
                if app.chat.is_loading() || app.chat.context != app.chat_context_key(cx) {
                    return Ok(false);
                }
                Self::copy_and_restore(app, OTHER, window, cx)?;
                ensure!(
                    app.assistant.references[0].text == ENGLISH,
                    "copying current video replaced fixed older quote"
                );
                app.set("question", QUESTION, window, cx);
                app.handle(Action::Chat, window, cx);
                ensure!(
                    app.tasks.cancel.is_some() && app.chat.running,
                    "actual chat action did not start"
                );
                self.note(
                    "explicit send starts one chat request using the pending English quote",
                    app,
                )?;
                self.advance();
            }
            11 => {
                if app.tasks.cancel.is_some() || app.chat.running || app.chat.is_loading() {
                    return Ok(false);
                }
                ensure!(
                    app.chat.output == ANSWER,
                    "native chat did not retain complete fixture answer: {}",
                    app.status
                );
                let requests = self
                    .server
                    .as_ref()
                    .unwrap()
                    .requests
                    .lock()
                    .unwrap()
                    .clone();
                ensure!(
                    requests.len() == 1,
                    "unexpected automatic or duplicate AI requests"
                );
                let content = requests[0]["messages"]
                    .as_array()
                    .context("missing chat messages")?
                    .iter()
                    .rev()
                    .find(|value| value["role"] == "user")
                    .and_then(|value| value["content"].as_str())
                    .context("missing user content")?;
                let expected = format!("{QUESTION}\n\n<reference>\n{ENGLISH}\n</reference>");
                ensure!(
                    content == expected,
                    "AI request was not exactly question plus the fixed plain English transcript"
                );
                ensure!(
                    !content.contains(PRIVATE)
                        && !content.contains("00:00")
                        && !content.contains("面包")
                        && !content.contains(OTHER),
                    "AI reference included source metadata or current replacement subtitles"
                );
                fs::write(
                    self.out.join("actual-ai-request.json"),
                    serde_json::to_vec_pretty(&requests[0])?,
                )?;
                self.note(
                    "actual loopback AI receives only question and fixed full-English content",
                    app,
                )?;
                self.advance();
            }
            12 => {
                let reopened = Library::open(app.lib.root.clone())?;
                let session = clarora_gpui::chat_store::get(
                    &reopened,
                    clarora_gpui::assistant::CONTEXT,
                    &app.chat.id,
                )?
                .context("saved global conversation missing")?;
                let user = session
                    .messages
                    .iter()
                    .find(|value| value["role"] == "user")
                    .context("saved question missing")?;
                ensure!(
                    user["content"] == QUESTION
                        && clarora_gpui::assistant::references(user)
                            == vec![self.reference.clone().unwrap()],
                    "reopened conversation lost exact question or English reference"
                );
                fs::write(
                    self.out.join("saved-conversation.json"),
                    serde_json::to_vec_pretty(&session.messages)?,
                )?;
                self.note(
                    "SQLite reopen retains exact explicit question and English reference",
                    app,
                )?;
                self.report(Some(true), None)?;
                return Ok(true);
            }
            _ => {}
        }
        Ok(false)
    }
}
impl AppView {
    pub(super) fn poll_subtitle_probe(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(mut probe) = self.subtitle_probe.take() else {
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
        self.subtitle_probe = Some(probe);
        false
    }
}
