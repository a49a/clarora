//! Native pointer and keyboard coverage for the assistant question editor.
use super::*;

#[derive(Clone, Copy)]
struct Scenario {
    name: &'static str,
    question: &'static str,
    selection: &'static str,
    start: usize,
    end: usize,
    result: &'static str,
    key: &'static str,
}

const SCENARIOS: [Scenario; 4] = [
    Scenario {
        name: "mouse selection and backspace edit assistant question",
        question: "before REMOVE after",
        selection: "REMOVE",
        start: 7,
        end: 13,
        result: "before  after",
        key: "backspace",
    },
    Scenario {
        name: "mouse selection deletes Chinese and emoji without splitting characters",
        question: "prefix 中文🌱 tail",
        selection: "中文🌱",
        start: 7,
        end: 11,
        result: "prefix  tail",
        key: "backspace",
    },
    Scenario {
        name: "mouse selection across wrapped multilingual text deletes correctly",
        question: "before REMOVE REMOVE REMOVE REMOVE REMOVE REMOVE REMOVE REMOVE REMOVE REMOVE REMOVE REMOVE 中文🌱 after",
        selection: "REMOVE REMOVE REMOVE REMOVE REMOVE REMOVE REMOVE REMOVE REMOVE REMOVE REMOVE REMOVE 中文🌱",
        start: 7,
        end: 95,
        result: "before  after",
        key: "backspace",
    },
    Scenario {
        name: "mouse selection and forward delete edit assistant question",
        question: "before REMOVE after",
        selection: "REMOVE",
        start: 7,
        end: 13,
        result: "before  after",
        key: "delete",
    },
];
const REPLY: &str =
    "A selectable assistant response should remain available for follow-up questions.";

pub(super) struct Probe {
    stage: u8,
    started: Instant,
    clipboard: Option<ClipboardItem>,
    cues: Vec<Cue>,
    selection_valid: bool,
    question_focused: bool,
    offer_valid: bool,
    scenario: usize,
    original_viewport: Size<Pixels>,
    background_selection: Option<std::ops::Range<usize>>,
    background_move: Point<Pixels>,
}

impl Probe {
    pub(super) fn new() -> Self {
        Self {
            stage: 0,
            started: Instant::now(),
            clipboard: None,
            cues: Vec::new(),
            selection_valid: false,
            question_focused: false,
            offer_valid: false,
            scenario: 0,
            original_viewport: size(px(0.), px(0.)),
            background_selection: None,
            background_move: point(px(0.), px(0.)),
        }
    }

    fn draw(window: &Window, cx: &mut Context<AppView>) {
        let handle = window.window_handle();
        cx.defer(move |cx| {
            handle
                .update(cx, |_, window, cx| window.draw(cx).clear())
                .expect("draw assistant selection diagnostic");
        });
    }

    pub(super) fn positions(
        app: &AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
        start_index: usize,
        end_index: usize,
        require_audio_underlay: bool,
    ) -> Result<(Point<Pixels>, Point<Pixels>)> {
        let viewport = window.viewport_size();
        let left = (f32::from(viewport.width) - 480.).max(0.) as usize;
        let right = f32::from(viewport.width) as usize;
        let bottom = f32::from(viewport.height) as usize;
        let underlay = app.subtitle_text.borrow().probe_audio_bounds();
        let list = app.audio.scroll.viewport_bounds();
        app.inputs["question"].update(cx, |state, cx| {
            let mut start = None;
            let mut end = None;
            for y in (0..bottom).step_by(2) {
                for x in (left..right).step_by(2) {
                    let position = point(px(x as f32), px(y as f32));
                    match state.character_index_for_point(position, window, cx) {
                        Some(index)
                            if start.is_none()
                                && index == start_index
                                && (!require_audio_underlay
                                    || (list.contains(&position)
                                        && underlay
                                            .iter()
                                            .any(|bounds| bounds.contains(&position)))) =>
                        {
                            start = Some(position)
                        }
                        Some(index)
                            if end.is_none()
                                && index == end_index
                                && start.is_some_and(|p: Point<Pixels>| p.y <= position.y) =>
                        {
                            end = Some(position)
                        }
                        _ => {}
                    }
                    if let (Some(start), Some(end)) = (start, end) {
                        return Ok((start, end));
                    }
                }
            }
            anyhow::bail!(
                "could not locate question selection: audio underlay required={require_audio_underlay}, {} fragments, list {list:?}",
                underlay.len(),
            )
        })
    }

    fn dispatch_keys(window: &Window, cx: &mut Context<AppView>, keys: Vec<&'static str>) {
        let handle = window.window_handle();
        cx.defer(move |cx| {
            handle
                .update(cx, |_, window, cx| {
                    for key in keys {
                        window.dispatch_keystroke(Keystroke::parse(key).unwrap(), cx);
                    }
                    window.draw(cx).clear();
                })
                .expect("dispatch assistant editing keys");
        });
    }

    fn background_positions(
        app: &AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Result<(Point<Pixels>, Point<Pixels>, Point<Pixels>)> {
        let viewport = window.viewport_size();
        let edge = viewport.width - px(480.);
        app.inputs["reference"].update(cx, |state, cx| {
            for y in (0..f32::from(viewport.height) as usize).step_by(2) {
                let start = point(edge - px(100.), px(y as f32));
                let end = point(edge + px(40.), px(y as f32));
                let later = point(edge + px(110.), px(y as f32));
                let indices = [start, end, later]
                    .map(|position| state.character_index_for_point(position, window, cx));
                if matches!(indices, [Some(a), Some(b), Some(c)] if a < b && b < c) {
                    return Ok((start, end, later));
                }
            }
            anyhow::bail!("could not locate the background editor across the sidebar")
        })
    }

    pub(super) fn step(
        &mut self,
        app: &mut AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
        cases: &mut Vec<Value>,
        observations: &mut Vec<Value>,
    ) -> Result<bool> {
        match self.stage {
            0 => {
                app.open_assistant(window, cx);
            }
            1 => {
                if app.chat.is_loading() || app.chat.context != clarora_gpui::assistant::CONTEXT {
                    return Ok(false);
                }
                self.clipboard = cx.read_from_clipboard();
                self.cues = app.audio.cues.clone();
                for cue in &mut app.audio.cues {
                    cue.text = "background subtitle ".repeat(18);
                }
                app.audio.scroll.reset(app.audio.cues.len());
                app.refresh_saved_ranges();
            }
            2 => {
                app.set("question", SCENARIOS[self.scenario].question, window, cx);
                app.assistant.offer = None;
                window.focus(&app.focus);
                Self::draw(window, cx);
            }
            3 => {
                let scenario = SCENARIOS[self.scenario];
                let (start, end) =
                    Self::positions(app, window, cx, scenario.start, scenario.end, true)?;
                observations.push(json!({
                    "scenario":scenario.name,
                    "assistant_drag_start":[f32::from(start.x), f32::from(start.y)],
                    "assistant_drag_end":[f32::from(end.x), f32::from(end.y)],
                    "audio_list":[
                        f32::from(app.audio.scroll.viewport_bounds().left()),
                        f32::from(app.audio.scroll.viewport_bounds().top()),
                        f32::from(app.audio.scroll.viewport_bounds().right()),
                        f32::from(app.audio.scroll.viewport_bounds().bottom())
                    ]
                }));
                native_probe_input::send(
                    window,
                    cx,
                    vec![
                        native_probe_input::MouseEvent::Down(start),
                        native_probe_input::MouseEvent::Drag(end),
                        native_probe_input::MouseEvent::Up(end),
                    ],
                )?;
                self.started = Instant::now();
            }
            4 => {
                if self.started.elapsed() < Duration::from_millis(400) {
                    return Ok(false);
                }
                let scenario = SCENARIOS[self.scenario];
                let selection = app.inputs["question"]
                    .update(cx, |state, cx| state.selected_text_range(false, window, cx));
                let focused = app.inputs["question"].read(cx).focus_handle(cx);
                self.offer_valid = app
                    .assistant
                    .offer
                    .as_ref()
                    .is_some_and(|(r, _)| r.text == scenario.selection);
                observations.push(json!({
                    "scenario":scenario.name,
                    "assistant_selection":selection.as_ref().map(|s| [s.range.start,s.range.end]),
                    "question_focused":focused.is_focused(window),
                    "subtitle_focused":app.subtitle_text.borrow().focus.is_focused(window),
                    "offer_matches":self.offer_valid
                }));
                self.selection_valid = selection
                    .as_ref()
                    .is_some_and(|s| s.range == (scenario.start..scenario.end));
                self.question_focused = focused.is_focused(window);
                Self::dispatch_keys(window, cx, vec![scenario.key]);
                self.started = Instant::now();
            }
            5 => {
                if self.started.elapsed() < Duration::from_millis(200) {
                    return Ok(false);
                }
                let scenario = SCENARIOS[self.scenario];
                observations.push(json!({
                    "scenario":scenario.name,
                    "offer_after_edit":app.assistant.offer.is_some()
                }));
                ensure!(
                    app.value("question", cx) == scenario.result,
                    "{} did not remove selected question text; selection={}, focus={}",
                    scenario.key,
                    self.selection_valid,
                    self.question_focused
                );
                ensure!(
                    self.selection_valid && self.question_focused && self.offer_valid,
                    "question editor lost its selection, focus or AI quote offer"
                );
                ensure!(
                    app.audio.current_id == "repeat-audio"
                        && app.media.snapshot.paused
                        && !app.subtitle_text.borrow().has_selection(),
                    "editing the question changed audio playback or selected background subtitles"
                );
                let after = cx.read_from_clipboard();
                ensure!(
                    self.clipboard == after
                        || (self.clipboard.is_none()
                            && after
                                .as_ref()
                                .is_none_or(|item| item.text().as_deref() == Some(""))),
                    "assistant selection changed the clipboard"
                );
                cases.push(json!({"case":scenario.name,"passed":true}));
                self.scenario += 1;
                if self.scenario < SCENARIOS.len() {
                    self.stage = 2;
                    return Ok(false);
                }
            }
            6 => {
                app.assistant.offer = None;
                app.chat.id = "assistant-selection-probe".into();
                app.chat.history = vec![json!({"role":"assistant","content":REPLY})];
                cx.notify();
                Self::draw(window, cx);
            }
            7 => {
                let bounds = app.assistant.scroll.bounds();
                ensure!(
                    bounds.size.width > px(100.) && bounds.size.height > px(100.),
                    "assistant reply history is not visible"
                );
                let start = point(bounds.left() + px(42.), bounds.top() + px(68.));
                let end = point(bounds.left() + px(180.), bounds.top() + px(68.));
                observations.push(json!({
                    "assistant_reply_start":[f32::from(start.x), f32::from(start.y)],
                    "assistant_reply_end":[f32::from(end.x), f32::from(end.y)],
                    "assistant_history_top":f32::from(bounds.top())
                }));
                native_probe_input::send(
                    window,
                    cx,
                    vec![native_probe_input::MouseEvent::Down(start)],
                )?;
                self.started = Instant::now();
            }
            8 => {
                if self.started.elapsed() < Duration::from_millis(200) {
                    return Ok(false);
                }
                Self::draw(window, cx);
            }
            9 => {
                let bounds = app.assistant.scroll.bounds();
                let end = point(bounds.left() + px(180.), bounds.top() + px(68.));
                native_probe_input::send(
                    window,
                    cx,
                    vec![
                        native_probe_input::MouseEvent::Drag(end),
                        native_probe_input::MouseEvent::Up(end),
                    ],
                )?;
                self.started = Instant::now();
            }
            10 => {
                if self.started.elapsed() < Duration::from_millis(400) {
                    return Ok(false);
                }
                let reference = app
                    .assistant
                    .offer
                    .as_ref()
                    .map(|(reference, _)| reference.clone())
                    .context("mouse selection in assistant reply did not offer an AI quote")?;
                ensure!(
                    !reference.text.is_empty() && REPLY.contains(&reference.text),
                    "assistant reply quote did not contain the selected text"
                );
                app.handle(Action::AssistantQuote(reference.clone()), window, cx);
                ensure!(
                    app.assistant.references.last() == Some(&reference),
                    "assistant reply selection did not become an AI reference"
                );
                app.handle(Action::AssistantRemove(0), window, cx);
                ensure!(
                    app.assistant.references.is_empty(),
                    "assistant reply reference could not be removed"
                );
                app.chat.history.clear();
                app.assistant.offer = None;
                cases.push(json!({"case":"mouse selection in assistant reply can be quoted","passed":true}));
            }
            11 => {
                app.set("question", "Hello 🌱\n第二行", window, cx);
                app.inputs["question"].update(cx, |state, cx| state.focus(window, cx));
                Self::draw(window, cx);
            }
            12 => {
                Self::dispatch_keys(
                    window,
                    cx,
                    vec![
                        if cfg!(target_os = "macos") {
                            "cmd-a"
                        } else {
                            "ctrl-a"
                        },
                        "backspace",
                    ],
                );
                self.started = Instant::now();
            }
            13 => {
                if self.started.elapsed() < Duration::from_millis(200) {
                    return Ok(false);
                }
                ensure!(
                    app.value("question", cx).is_empty(),
                    "multiline select-all and backspace failed"
                );
                cases.push(json!({"case":"select all and delete multiline multilingual question","passed":true}));
                app.inputs["question"].update(cx, |state, cx| {
                    state.insert("A🌱", window, cx);
                });
                Self::draw(window, cx);
            }
            14 => {
                Self::dispatch_keys(window, cx, vec!["backspace"]);
                self.started = Instant::now();
            }
            15 => {
                if self.started.elapsed() < Duration::from_millis(200) {
                    return Ok(false);
                }
                ensure!(
                    app.value("question", cx) == "A",
                    "ordinary backspace split or retained emoji"
                );
                cases.push(
                    json!({"case":"ordinary backspace removes a complete emoji","passed":true}),
                );
                app.set("question", "", window, cx);
                app.assistant.offer = None;
                Self::draw(window, cx);
            }
            16 => {
                let viewport = window.viewport_size();
                let sidebar_left = viewport.width - px(480.);
                let list = app.audio.scroll.viewport_bounds();
                let bounds = app
                    .subtitle_text
                    .borrow()
                    .probe_audio_bounds()
                    .into_iter()
                    .find(|bounds| {
                        bounds.size.width > px(20.)
                            && bounds.left() >= list.left()
                            && bounds.right() < sidebar_left
                            && bounds.top() >= list.top()
                            && bounds.bottom() <= list.bottom()
                    })
                    .context("no uncovered subtitle fragment available for mouse selection")?;
                let y = bounds.top() + bounds.size.height / 2.;
                let start = point(bounds.left() + px(2.), y);
                let end = point(bounds.right() - px(2.), y);
                native_probe_input::send(
                    window,
                    cx,
                    vec![
                        native_probe_input::MouseEvent::Down(start),
                        native_probe_input::MouseEvent::Drag(end),
                        native_probe_input::MouseEvent::Up(end),
                    ],
                )?;
                self.started = Instant::now();
            }
            17 => {
                if self.started.elapsed() < Duration::from_millis(400) {
                    return Ok(false);
                }
                let reference = app
                    .assistant
                    .offer
                    .as_ref()
                    .map(|(reference, _)| reference)
                    .context("uncovered subtitle selection did not offer an AI quote")?;
                ensure!(
                    !reference.text.is_empty()
                        && "background subtitle ".contains(&reference.text)
                        && app.subtitle_text.borrow().has_selection(),
                    "uncovered subtitle selection no longer produces the correct quote"
                );
                cases.push(json!({"case":"uncovered subtitle selection still offers an AI quote","passed":true}));
                app.assistant.offer = None;
                app.subtitle_text.borrow_mut().clear();
                Self::draw(window, cx);
            }
            18 => {
                let viewport = window.viewport_size();
                let sidebar_left = viewport.width - px(480.);
                let list = app.audio.scroll.viewport_bounds();
                let bounds = app
                    .subtitle_text
                    .borrow()
                    .probe_audio_bounds()
                    .into_iter()
                    .find(|bounds| {
                        bounds.size.width > px(20.)
                            && bounds.left() >= list.left()
                            && bounds.right() < sidebar_left - px(50.)
                            && bounds.top() >= list.top()
                            && bounds.bottom() <= list.bottom()
                    })
                    .context("no uncovered subtitle fragment available for cross-sidebar drag")?;
                let y = bounds.top() + bounds.size.height / 2.;
                let start = point(bounds.left() + px(2.), y);
                let end = point(sidebar_left + px(30.), y);
                native_probe_input::send(
                    window,
                    cx,
                    vec![
                        native_probe_input::MouseEvent::Down(start),
                        native_probe_input::MouseEvent::Drag(end),
                        native_probe_input::MouseEvent::Up(end),
                    ],
                )?;
                self.started = Instant::now();
            }
            19 => {
                if self.started.elapsed() < Duration::from_millis(400) {
                    return Ok(false);
                }
                ensure!(
                    app.assistant.offer.is_some()
                        && app.subtitle_text.borrow().has_selection()
                        && !app.assistant.dragging
                        && app.assistant.press.is_none(),
                    "subtitle drag into the sidebar did not finish and offer an AI quote"
                );
                cases.push(json!({"case":"subtitle drag into assistant finishes with an AI quote","passed":true}));
                app.assistant.offer = None;
                app.subtitle_text.borrow_mut().clear();
                self.original_viewport = window.viewport_size();
                window.resize(size(px(760.), px(700.)));
                self.started = Instant::now();
            }
            20 => {
                if (f32::from(window.viewport_size().width) - 760.).abs() > 2. {
                    ensure!(
                        self.started.elapsed() < Duration::from_secs(3),
                        "narrow window did not resize"
                    );
                    return Ok(false);
                }
                app.set("question", SCENARIOS[0].question, window, cx);
                window.focus(&app.focus);
                Self::draw(window, cx);
            }
            21 => {
                let scenario = SCENARIOS[0];
                let (start, end) =
                    Self::positions(app, window, cx, scenario.start, scenario.end, false)?;
                native_probe_input::send(
                    window,
                    cx,
                    vec![
                        native_probe_input::MouseEvent::Down(start),
                        native_probe_input::MouseEvent::Drag(end),
                        native_probe_input::MouseEvent::Up(end),
                    ],
                )?;
                self.started = Instant::now();
            }
            22 => {
                if self.started.elapsed() < Duration::from_millis(400) {
                    return Ok(false);
                }
                let selection = app.inputs["question"]
                    .update(cx, |state, cx| state.selected_text_range(false, window, cx));
                ensure!(
                    selection.as_ref().is_some_and(|s| s.range == (7..13))
                        && app.inputs["question"]
                            .read(cx)
                            .focus_handle(cx)
                            .is_focused(window),
                    "narrow assistant lost question selection or focus: {selection:?}"
                );
                Self::dispatch_keys(window, cx, vec!["backspace"]);
                self.started = Instant::now();
            }
            23 => {
                if self.started.elapsed() < Duration::from_millis(200) {
                    return Ok(false);
                }
                ensure!(
                    app.value("question", cx) == SCENARIOS[0].result
                        && !app.subtitle_text.borrow().has_selection(),
                    "narrow assistant deletion failed or affected background subtitles"
                );
                cases.push(
                    json!({"case":"narrow assistant mouse selection and backspace","passed":true}),
                );
                window.resize(self.original_viewport);
                self.started = Instant::now();
            }
            24 => {
                if (f32::from(window.viewport_size().width - self.original_viewport.width)).abs()
                    > 2.
                {
                    ensure!(
                        self.started.elapsed() < Duration::from_secs(3),
                        "original window size was not restored"
                    );
                    return Ok(false);
                }
                app.set("question", "", window, cx);
                app.assistant.offer = None;
                app.handle(Action::Page("speaking".into()), window, cx);
                app.set(
                    "reference",
                    "background selection words ".repeat(40),
                    window,
                    cx,
                );
                window.focus(&app.focus);
                Self::draw(window, cx);
            }
            25 => {
                let (start, end, later) = Self::background_positions(app, window, cx)?;
                observations.push(json!({
                    "background_start":[f32::from(start.x), f32::from(start.y)],
                    "background_end":[f32::from(end.x), f32::from(end.y)],
                    "background_later":[f32::from(later.x), f32::from(later.y)]
                }));
                self.background_move = later;
                native_probe_input::send(
                    window,
                    cx,
                    vec![
                        native_probe_input::MouseEvent::Down(start),
                        native_probe_input::MouseEvent::Drag(end),
                        native_probe_input::MouseEvent::Up(end),
                    ],
                )?;
                self.started = Instant::now();
            }
            26 => {
                if self.started.elapsed() < Duration::from_millis(400) {
                    return Ok(false);
                }
                let selection = app.inputs["reference"]
                    .update(cx, |state, cx| state.selected_text_range(false, window, cx));
                let range = selection
                    .filter(|s| !s.range.is_empty())
                    .context("background editor drag did not select text")?
                    .range;
                observations.push(json!({
                    "background_selection_before":[range.start, range.end],
                    "press_remaining":app.assistant.press.is_some(),
                    "dragging":app.assistant.dragging
                }));
                ensure!(
                    app.inputs["reference"]
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window)
                        && app.assistant.offer.is_some(),
                    "background editor drag lost focus or did not offer an AI quote"
                );
                self.background_selection = Some(range);
                native_probe_input::send(
                    window,
                    cx,
                    vec![
                        native_probe_input::MouseEvent::Drag(self.background_move),
                        native_probe_input::MouseEvent::Up(self.background_move),
                    ],
                )?;
                self.started = Instant::now();
            }
            27 => {
                if self.started.elapsed() < Duration::from_millis(250) {
                    return Ok(false);
                }
                let selection = app.inputs["reference"]
                    .update(cx, |state, cx| state.selected_text_range(false, window, cx));
                observations.push(json!({
                    "background_selection_after":selection.as_ref().map(|s| [s.range.start, s.range.end])
                }));
                ensure!(
                    selection.as_ref().map(|s| &s.range) == self.background_selection.as_ref(),
                    "background editor remained in drag-selection mode after mouse-up"
                );
                cases.push(json!({"case":"background editor drag into assistant finishes selection","passed":true}));
                app.assistant.offer = None;
                app.handle(Action::Page("listening".into()), window, cx);
            }
            28 => {
                if app.audio.current_id != "repeat-audio"
                    || app.loads.restore_pending
                    || app.media.snapshot.path
                        != app
                            .media
                            .path
                            .as_ref()
                            .context("audio path")?
                            .to_string_lossy()
                    || !app.media.snapshot.paused
                {
                    return Ok(false);
                }
                app.audio.cues = std::mem::take(&mut self.cues);
                app.audio.scroll.reset(app.audio.cues.len());
                app.refresh_saved_ranges();
                app.handle(Action::Assistant, window, cx);
                return Ok(true);
            }
            _ => unreachable!(),
        }
        self.stage += 1;
        Ok(false)
    }
}
