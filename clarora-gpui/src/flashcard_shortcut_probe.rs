//! Native flashcard shortcut acceptance; isolated SQLite, no external AI service.
// Failure scenarios specified before production changes:
// - Space does not flip from the page, or flips twice because handlers bubble.
// - Clicking selectable card text changes focus and silently disables Space.
// - Space/IME typing in the assistant, modified keys, held-repeat or a dialog flips a card.
// - Explicit review controls retain stale editor focus; Discover has different behavior.
// Replay failure scenarios specified before adding replay acceptance:
// - R toggles pause or replays a stale shared file instead of this card's saved range.
// - Replay starts at zero, overruns the clip end, changes the card, or flips its answer.
// - A paused/ended clip cannot restart; input/modified/held R unexpectedly restarts it.
// - A card without media restarts the old player; temporary fixture survives restoration.
use super::*;
use std::sync::{Mutex, OnceLock};

static FRONT: OnceLock<Mutex<Option<Bounds<Pixels>>>> = OnceLock::new();
pub(super) fn front_probe() -> Option<AnyElement> {
    if !std::env::args()
        .any(|arg| arg == "--ui-controller-check" || arg == "--ui-controller-quit-check")
    {
        return None;
    }
    Some(
        canvas(
            |_, _, _| (),
            |bounds, _, _, _| {
                *FRONT.get_or_init(Default::default).lock().unwrap() = Some(bounds);
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .into_any_element(),
    )
}

pub(super) struct Flow {
    out: PathBuf,
    started: Instant,
    stage_started: Instant,
    stage: u8,
    cases: Vec<Value>,
    question: String,
    assistant_open: bool,
    assistant_enabled: bool,
    assistant_legacy: bool,
    due: bool,
    filter: String,
    pointer_diagnostics: Vec<Value>,
    replay_path: PathBuf,
    replay_card: String,
    replay_index: usize,
    replay_revealed: bool,
    replay_revision: u64,
    paused_position: f64,
    paused_replay_started: Option<Instant>,
    original_rate: f64,
    original_repeat: PlaybackMode,
}
impl Flow {
    pub(super) fn new(out: &Path) -> Self {
        Self {
            out: out.into(),
            started: Instant::now(),
            stage_started: Instant::now(),
            stage: 0,
            cases: vec![],
            question: String::new(),
            assistant_open: false,
            assistant_enabled: false,
            assistant_legacy: false,
            due: false,
            filter: String::new(),
            pointer_diagnostics: vec![],
            replay_path: PathBuf::new(),
            replay_card: String::new(),
            replay_index: 0,
            replay_revealed: false,
            replay_revision: 0,
            paused_position: 0.,
            paused_replay_started: None,
            original_rate: 1.,
            original_repeat: PlaybackMode::Normal,
        }
    }
    pub(super) fn cases(&self) -> Vec<Value> {
        self.cases.clone()
    }
    fn advance(&mut self) {
        self.stage += 1;
        self.stage_started = Instant::now();
        let _ = fs::write(self.out.join("flashcard-shortcut-progress.json"), serde_json::to_vec_pretty(&json!({"stage":self.stage,"cases":self.cases,"pointer_diagnostics":self.pointer_diagnostics,"elapsed_ms":self.started.elapsed().as_millis()})).unwrap());
    }
    fn settled(&self) -> bool {
        self.stage_started.elapsed() >= Duration::from_millis(120)
    }
    fn note(&mut self, case: &str) {
        self.cases.push(json!({"case":case,"passed":true}));
    }
    fn replay_note(&mut self, case: &str, app: &AppView) {
        let state = app.media.player.snapshot();
        let end_gap = app
            .current_card()
            .map(|card| (card.end - state.position).max(0.));
        self.cases.push(json!({"case":case,"passed":true,"card":app.current_card(),"index":app.index,"revealed":app.revealed,
            "player":{"path":state.path,"position":state.position,"duration":state.duration,"paused":state.paused,"eof":state.eof,"load_revision":state.load_revision},
            "paused_replay_elapsed_ms":self.paused_replay_started.map(|at|at.elapsed().as_millis()),
            "null_audio_eof_time_pos_may_lag":state.eof,"reported_end_gap_seconds":if state.eof {end_gap} else {None}}));
    }
    fn same_replay_card(&self, app: &AppView) -> Result<()> {
        ensure!(
            app.current_card()
                .is_some_and(|card| card.id == self.replay_card)
                && app.index == self.replay_index
                && app.revealed == self.replay_revealed,
            "R changed the selected card or revealed face"
        );
        Ok(())
    }
    fn at_replay_start(&self, app: &AppView) -> bool {
        let state = app.media.player.snapshot();
        state.path == self.replay_path.to_string_lossy()
            && state.load_revision > self.replay_revision
            && !state.paused
            && !state.eof
            && (0.95..=1.6).contains(&state.position)
    }
    fn restore(
        &self,
        app: &mut AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Result<()> {
        app.media.player.set("pause", "yes");
        app.lib.delete("clip_cards", "native-replay-shortcut")?;
        app.media.rate = self.original_rate;
        app.media.session.mode = self.original_repeat;
        app.handle(Action::Page("flashcard".into()), window, cx);
        app.handle(Action::ReviewMode(self.due), window, cx);
        app.handle(Action::Filter(self.filter.clone()), window, cx);
        app.set("question", self.question.clone(), window, cx);
        if app.assistant_open != self.assistant_open {
            app.handle(Action::Assistant, window, cx);
        }
        let context_changed = app.assistant.enabled != self.assistant_enabled
            || app.assistant.legacy != self.assistant_legacy;
        app.assistant.enabled = self.assistant_enabled;
        app.assistant.legacy = self.assistant_legacy;
        if context_changed {
            app.chat_requested_context.clear();
            app.sync_chat(window, cx);
        }
        window.focus(&app.focus);
        Ok(())
    }
    fn report(&self, passed: bool, error: Option<String>) -> Result<()> {
        fs::write(
            self.out.join("flashcard-shortcut-report.json"),
            serde_json::to_vec_pretty(&json!({
                "passed":passed,"error":error,"stage":self.stage,"cases":self.cases,
                "native_pointer_and_dispatched_keystrokes":true,"hidden_window":true,
                "physical_keyboard_verified":false,"external_ai_used":false,"independent_review":false,
                "pointer_diagnostics":self.pointer_diagnostics,"paused_position":self.paused_position
            }))?,
        )?;
        Ok(())
    }
    fn draw(window: &Window, cx: &mut Context<AppView>) {
        let handle = window.window_handle();
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| {
                window.draw(cx).clear();
            });
        });
    }
    fn keys(window: &Window, cx: &mut Context<AppView>, keys: &[&str]) {
        let handle = window.window_handle();
        let keys = keys
            .iter()
            .map(|k| Keystroke::parse(k).expect("flashcard diagnostic key"))
            .collect::<Vec<_>>();
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| {
                window.draw(cx).clear();
                for key in keys {
                    window.dispatch_keystroke(key, cx);
                }
                window.draw(cx).clear();
            });
        });
    }
    pub(super) fn step(
        &mut self,
        app: &mut AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Result<bool> {
        let result = self.inner(app, window, cx);
        if let Err(error) = &result {
            let _ = self.report(false, Some(format!("{error:#}")));
        }
        result
    }
    fn inner(
        &mut self,
        app: &mut AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Result<bool> {
        let observed = app.media.player.snapshot();
        ensure!(
            self.started.elapsed() < Duration::from_secs(30),
            "flashcard key timeout stage {}, position={}, paused={}, eof={}, revision={}, path={}",
            self.stage,
            observed.position,
            observed.paused,
            observed.eof,
            observed.load_revision,
            observed.path
        );
        match self.stage {
            0 => {
                app.media.player = Player::without_audio_device();
                app.media.preloader = Player::without_audio_device();
                self.question = app.value("question", cx);
                self.assistant_open = app.assistant_open;
                self.assistant_enabled = app.assistant.enabled;
                self.assistant_legacy = app.assistant.legacy;
                self.due = app.due;
                self.filter = app.card_filter.clone();
                self.original_rate = app.media.rate;
                self.original_repeat = app.media.session.mode;
                app.handle(Action::Page("flashcard".into()), window, cx);
                app.handle(Action::ReviewMode(false), window, cx);
                app.handle(Action::Filter("sentence".into()), window, cx);
                Self::draw(window, cx);
            }
            1 => {
                if !app.cards_ready() || app.current_card().is_none() {
                    return Ok(false);
                }
                ensure!(!app.revealed, "initial flashcard already revealed");
                Self::keys(window, cx, &["space"]);
            }
            2 => {
                if !self.settled() {
                    return Ok(false);
                }
                ensure!(app.revealed, "page Space did not reveal answer");
                self.note("Space reveals answer directly from flashcard page");
                Self::keys(window, cx, &["space"]);
            }
            3 => {
                if !self.settled() {
                    return Ok(false);
                }
                ensure!(
                    !app.revealed,
                    "second Space did not hide answer or flipped twice"
                );
                self.note("second Space hides answer with exactly one flip per key");
                let bounds = (*FRONT.get_or_init(Default::default).lock().unwrap())
                    .context("card front probe was not painted")?;
                ensure!(
                    bounds.size.width > px(80.) && bounds.size.height > px(20.),
                    "invalid painted front bounds"
                );
                let position = point(bounds.left() + px(10.), bounds.top() + px(25.));
                self.pointer_diagnostics.push(json!({"phase":"before native front click",
                    "bounds":{"left":f32::from(bounds.left()),"top":f32::from(bounds.top()),"width":f32::from(bounds.size.width),"height":f32::from(bounds.size.height)},
                    "point":[f32::from(position.x),f32::from(position.y)],
                    "viewport":[f32::from(window.viewport_size().width),f32::from(window.viewport_size().height)],
                    "page_scroll":[f32::from(app.page_scroll.offset().x),f32::from(app.page_scroll.offset().y)],
                    "assistant_open":app.assistant_open,
                    "context":format!("{:?}",window.context_stack())}));
                native_probe_input::send(
                    window,
                    cx,
                    vec![
                        native_probe_input::MouseEvent::Down(position),
                        native_probe_input::MouseEvent::Up(position),
                    ],
                )?;
            }
            4 => {
                if self.stage_started.elapsed() < Duration::from_millis(350) {
                    return Ok(false);
                }
                self.pointer_diagnostics
                    .push(json!({"phase":"after native front click",
                    "focus_is_app":window.focused(cx)==Some(app.focus.clone()),
                    "context":format!("{:?}",window.context_stack()),
                    "revealed":app.revealed}));
                ensure!(
                    window.focused(cx) != Some(app.focus.clone())
                        && window
                            .context_stack()
                            .iter()
                            .any(|c| c.contains("TextView"))
                        && window
                            .context_stack()
                            .iter()
                            .any(|c| c.contains("StudyCard")),
                    "native card-text click did not take read-only StudyCard/TextView focus"
                );
                ensure!(!app.revealed, "clicking text unexpectedly flipped card");
                Self::keys(window, cx, &["space"]);
            }
            5 => {
                if !self.settled() {
                    return Ok(false);
                }
                ensure!(
                    app.revealed,
                    "Space ignored after native click on selectable card text"
                );
                self.note("Space works after native pointer focuses selectable card front");
                Self::keys(
                    window,
                    cx,
                    &["shift-space", "alt-space", "ctrl-space", "cmd-space"],
                );
            }
            6 => {
                if !self.settled() {
                    return Ok(false);
                }
                ensure!(app.revealed, "modified Space flipped a card");
                let held = KeyDownEvent {
                    keystroke: Keystroke::parse("space").expect("probe held Space"),
                    is_held: true,
                };
                let _ = app.handle_card_study_key(&held, window, cx);
                ensure!(app.revealed, "held Space repeatedly flipped a card");
                self.note("modified Space and held repeat do not flip cards");
                if !app.assistant_open {
                    app.handle(Action::Assistant, window, cx);
                }
                app.set("question", "", window, cx);
                app.inputs["question"].update(cx, |input, cx| input.focus(window, cx));
                Self::draw(window, cx);
            }
            7 => Self::keys(window, cx, &["space"]),
            8 => {
                if !self.settled() {
                    return Ok(false);
                }
                ensure!(
                    app.value("question", cx) == " "
                        && app.revealed
                        && app.inputs["question"]
                            .read(cx)
                            .focus_handle(cx)
                            .is_focused(window),
                    "assistant Space did not type normally or flipped background card"
                );
                self.note("Space types in assistant question without flipping background card");
                app.handle(Action::ReviewMode(false), window, cx);
                ensure!(
                    window.focused(cx) == Some(app.focus.clone()) && !app.revealed,
                    "review mode action retained stale input focus"
                );
            }
            9 => {
                if !app.cards_ready() || app.current_card().is_none() {
                    return Ok(false);
                }
                Self::keys(window, cx, &["space"]);
            }
            10 => {
                if !self.settled() {
                    return Ok(false);
                }
                ensure!(app.revealed, "Space failed after review mode control");
                self.note("review-mode selection returns keyboard to the flashcard");
                app.inputs["question"].update(cx, |input, cx| input.focus(window, cx));
                app.handle(Action::Flip, window, cx);
                ensure!(
                    !app.revealed && window.focused(cx) == Some(app.focus.clone()),
                    "flip control retained stale input focus"
                );
                Self::keys(window, cx, &["space"]);
            }
            11 => {
                if !self.settled() {
                    return Ok(false);
                }
                ensure!(app.revealed, "Space failed after explicit flip control");
                self.note("explicit flip control returns keyboard to the flashcard");
                app.inputs["question"].update(cx, |input, cx| input.focus(window, cx));
                app.handle(Action::Filter("sentence".into()), window, cx);
                ensure!(
                    window.focused(cx) == Some(app.focus.clone()),
                    "filter control retained stale input focus"
                );
                self.note("card filter selection returns keyboard to the flashcard");
                app.handle(Action::Page("discover".into()), window, cx);
            }
            12 => {
                if !app.cards_ready() || app.current_card().is_none() {
                    return Ok(false);
                }
                ensure!(!app.revealed, "discover opened revealed");
                Self::keys(window, cx, &["space"]);
            }
            13 => {
                if !self.settled() {
                    return Ok(false);
                }
                ensure!(app.revealed, "Discover Space did not flip");
                self.note("Discover shares the same flip shortcut");
                self.replay_path = app.lib.root.join("media/replay-shortcut.wav");
                let mut writer = hound::WavWriter::create(
                    &self.replay_path,
                    hound::WavSpec {
                        channels: 1,
                        sample_rate: 16000,
                        bits_per_sample: 16,
                        sample_format: hound::SampleFormat::Int,
                    },
                )?;
                for _ in 0..64000 {
                    writer.write_sample(0i16)?;
                }
                writer.finalize()?;
                app.lib.put("clip_cards", json!({"id":"native-replay-shortcut","en_text":"R replays only this saved clip.","zh_text":"R 只重播当前保存的片段。","audio_uri":self.replay_path,"start_ms":1000,"end_ms":2200}))?;
                app.handle(Action::Page("flashcard".into()), window, cx);
                app.handle(Action::ReviewMode(false), window, cx);
                app.handle(Action::Filter("clip".into()), window, cx);
            }
            14 => {
                if !app.cards_ready() || app.current_card().is_none() {
                    return Ok(false);
                }
                app.index = app
                    .cards
                    .iter()
                    .position(|card| card.id == "native-replay-shortcut")
                    .context("temporary replay clip missing from SQLite snapshot")?;
                app.revealed = true;
                self.replay_card = app.current_card().unwrap().id;
                self.replay_index = app.index;
                self.replay_revealed = app.revealed;
                app.media.rate = 1.;
                app.media.session.mode = PlaybackMode::Normal;
                app.media.preloaded_index = None;
                app.play_card();
                window.focus(&app.focus);
            }
            15 => {
                let state = app.media.player.snapshot();
                if state.path != self.replay_path.to_string_lossy()
                    || state.paused
                    || state.position < 1.65
                {
                    return Ok(false);
                }
                ensure!(
                    state.position < 2.2 && state.duration > 3.9,
                    "replay fixture did not start inside bounded clip"
                );
                self.replay_revision = state.load_revision;
                Self::keys(window, cx, &["r"]);
            }
            16 => {
                if !self.at_replay_start(app) {
                    return Ok(false);
                }
                self.same_replay_card(app)?;
                self.replay_note("R replays the current bounded clip from its saved start without changing card or face", app);
                app.media.player.set("pause", "yes");
            }
            17 => {
                if !app.media.player.snapshot().paused {
                    return Ok(false);
                }
            }
            18 => {
                let state = app.media.player.snapshot();
                if !state.paused {
                    return Ok(false);
                }
                self.replay_revision = state.load_revision;
                self.paused_replay_started = Some(Instant::now());
                Self::keys(window, cx, &["r"]);
            }
            19 => {
                if !self.at_replay_start(app) {
                    return Ok(false);
                }
                self.same_replay_card(app)?;
                self.replay_note(
                    "R restarts a paused card instead of merely toggling pause",
                    app,
                );
            }
            20 => {
                let state = app.media.player.snapshot();
                if !state.eof {
                    return Ok(false);
                }
                ensure!(
                    (1.0..=2.35).contains(&state.position)
                        && state.duration > 3.9
                        && state.error.is_empty()
                        && app
                            .current_card()
                            .is_some_and(|card| (card.start - 1.).abs() < 0.0001
                                && (card.end - 2.2).abs() < 0.0001),
                    "replayed clip overran saved end or used whole file: position={},duration={}",
                    state.position,
                    state.duration
                );
                self.same_replay_card(app)?;
                self.replay_note(
                    "replayed clip reaches EOF within its saved range before whole-file EOF",
                    app,
                );
                self.replay_revision = state.load_revision;
                Self::keys(window, cx, &["r"]);
            }
            21 => {
                if !self.at_replay_start(app) {
                    return Ok(false);
                }
                self.same_replay_card(app)?;
                self.replay_note("R restarts an ended clip from its saved start", app);
                app.media.player.set("pause", "yes");
            }
            22 => {
                let state = app.media.player.snapshot();
                if !state.paused {
                    return Ok(false);
                }
                self.paused_position = state.position;
            }
            23 => {
                if !self.settled() {
                    return Ok(false);
                }
                let state = app.media.player.snapshot();
                if !state.paused {
                    return Ok(false);
                }
                // Paused audio may report the last decoded PTS rather than the
                // exact seek target. Preserve its actual settled position; replay
                // correctness is independently proved by a fresh file load.
                self.paused_position = state.position;
                self.replay_revision = state.load_revision;
                Self::keys(window, cx, &["shift-r", "alt-r", "ctrl-r", "cmd-r"]);
            }
            24 => {
                if !self.settled() {
                    return Ok(false);
                }
                let state = app.media.player.snapshot();
                ensure!(
                    state.load_revision == self.replay_revision
                        && state.paused
                        && (state.position - self.paused_position).abs() < 0.1,
                    "modified R restarted paused media"
                );
                let held = KeyDownEvent {
                    keystroke: Keystroke::parse("r").expect("probe held R"),
                    is_held: true,
                };
                let _ = app.handle_card_study_key(&held, window, cx);
                if !app.assistant_open {
                    app.handle(Action::Assistant, window, cx);
                }
                app.set("question", "", window, cx);
                app.inputs["question"].update(cx, |input, cx| input.focus(window, cx));
                Self::draw(window, cx);
            }
            25 => Self::keys(window, cx, &["r"]),
            26 => {
                if !self.settled() {
                    return Ok(false);
                }
                let state = app.media.player.snapshot();
                ensure!(
                    app.value("question", cx) == "r"
                        && state.load_revision == self.replay_revision
                        && state.paused
                        && (state.position - self.paused_position).abs() < 0.1,
                    "typed or held R restarted background media instead of typing"
                );
                self.same_replay_card(app)?;
                self.replay_note(
                    "R typing, modifiers and held repeat leave paused media unchanged",
                    app,
                );
                app.handle(Action::Filter("sentence".into()), window, cx);
            }
            27 => {
                if !app.cards_ready() || app.current_card().is_none() {
                    return Ok(false);
                }
                let card = app.current_card().unwrap();
                ensure!(
                    card.media.is_empty(),
                    "no-audio fixture unexpectedly has media"
                );
                self.replay_card = card.id;
                self.replay_index = app.index;
                self.replay_revealed = app.revealed;
                let state = app.media.player.snapshot();
                ensure!(
                    state.path == self.replay_path.to_string_lossy()
                        && state.paused
                        && (state.position - self.paused_position).abs() < 0.1,
                    "stale shared media fixture changed before no-audio R"
                );
                self.replay_revision = state.load_revision;
                Self::keys(window, cx, &["r"]);
            }
            28 => {
                if !self.settled() {
                    return Ok(false);
                }
                let state = app.media.player.snapshot();
                ensure!(
                    state.load_revision == self.replay_revision
                        && state.paused
                        && (state.position - self.paused_position).abs() < 0.1,
                    "R on card without audio restarted stale media"
                );
                self.same_replay_card(app)?;
                self.replay_note(
                    "R on a card without audio does not restart stale shared media",
                    app,
                );
                self.restore(app, window, cx)?;
            }
            29 => {
                if !app.cards_ready() || app.current_card().is_none() {
                    return Ok(false);
                }
                ensure!(!app.revealed, "restored fixture card remained flipped");
                ensure!(
                    !app.lib
                        .list("clip_cards")?
                        .iter()
                        .any(|row| text(row, "id") == "native-replay-shortcut"),
                    "temporary replay clip was not removed"
                );
                ensure!(
                    app.assistant.enabled == self.assistant_enabled
                        && app.assistant.legacy == self.assistant_legacy,
                    "flashcard fixture changed the following controller's chat mode"
                );
                self.cases.push(json!({
                    "case":"flashcard fixture restores captured assistant mode before controller continues",
                    "passed":true,
                    "expected_enabled":self.assistant_enabled,
                    "expected_legacy":self.assistant_legacy,
                    "restored_enabled":app.assistant.enabled,
                    "restored_legacy":app.assistant.legacy,
                    "chat_context":app.chat.context,
                    "requested_context":app.chat.requested_context()
                }));
                self.report(true, None)?;
                self.stage = 30;
                return Ok(true);
            }
            _ => return Ok(true),
        }
        self.advance();
        Ok(false)
    }
}
