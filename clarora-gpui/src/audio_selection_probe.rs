//! Real audio screen → native pointer selection → OS clipboard / explicit AI quote.
use super::*;
use std::ops::Range;
use subtitle_text_ui::Surface;

pub(super) struct Flow {
    stage: u8,
    started: Instant,
    clipboard: Option<Option<ClipboardItem>>,
    show_current: bool,
    position: f64,
    expected: String,
    last_observation: Value,
    focus_drawn: bool,
    word_request: u64,
}
impl Flow {
    pub fn new() -> Self {
        Self {
            stage: 0,
            started: Instant::now(),
            clipboard: None,
            show_current: false,
            position: 0.,
            expected: String::new(),
            last_observation: Value::Null,
            focus_drawn: false,
            word_request: 0,
        }
    }
    pub(super) fn cleanup(&mut self, cx: &mut App) {
        if let Some(previous) = self.clipboard.take() {
            cx.write_to_clipboard(
                previous.unwrap_or_else(|| ClipboardItem::new_string(String::new())),
            );
        }
    }
    fn draw(window: &Window, cx: &mut Context<AppView>) {
        let handle = window.window_handle();
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| window.draw(cx).clear());
        });
    }
    fn document(app: &AppView) -> clarora_gpui::subtitle_selection::Document {
        clarora_gpui::subtitle_selection::Document::new(
            app.audio.current_id.clone(),
            &app.audio.cues,
        )
    }
    fn points(
        app: &AppView,
        surface: Surface,
        range: Range<usize>,
    ) -> Result<(Point<Pixels>, Point<Pixels>)> {
        let visible = if surface == Surface::Focus {
            app.audio.current_scroll.bounds()
        } else {
            app.audio.scroll.viewport_bounds()
        };
        let state = app.subtitle_text.borrow();
        Ok((
            state
                .probe_point(surface, range.start, visible)
                .with_context(|| {
                    format!(
                        "audio selection start {} not painted; visible {visible:?}; fragments {:?}",
                        range.start,
                        state.probe_fragments(surface)
                    )
                })?,
            state
                .probe_point(surface, range.end, visible)
                .context("audio selection end not painted")?,
        ))
    }
    fn drag(
        &mut self,
        app: &AppView,
        surface: Surface,
        range: Range<usize>,
        window: &Window,
        cx: &mut Context<AppView>,
    ) -> Result<()> {
        let doc = Self::document(app);
        self.expected = doc.text[range.clone()].into();
        let (start, end) = Self::points(app, surface, range)?;
        native_probe_input::send(
            window,
            cx,
            vec![
                native_probe_input::MouseEvent::Down(start),
                native_probe_input::MouseEvent::Drag(end),
                native_probe_input::MouseEvent::Up(end),
            ],
        )?;
        Ok(())
    }
    fn double_click_word(
        &mut self,
        app: &AppView,
        surface: Surface,
        window: &Window,
        cx: &mut Context<AppView>,
        observations: &mut Vec<Value>,
    ) -> Result<()> {
        let doc = Self::document(app);
        let start = doc
            .text
            .find("illustrates,")
            .context("missing punctuated word fixture")?;
        let offset = start + 4;
        let (position, _) = Self::points(app, surface, offset..offset + 1)?;
        self.expected = "illustrates".into();
        self.position = app.media.snapshot.position;
        observations.push(json!({
            "event": "native audio subtitle double click",
            "surface": if surface == Surface::Focus { "current-card" } else { "transcript" },
            "input": if surface == Surface::Focus {
                "AppKit full double click: Down1/Up1/Down2/Up2"
            } else {
                "AppKit second click: Down2/Up2"
            },
            "click_counts": if surface == Surface::Focus { vec![1, 1, 2, 2] } else { vec![2, 2] },
            "click_count": 2, "offset": offset,
            "click": [f32::from(position.x), f32::from(position.y)],
            "expected": self.expected, "fixture": doc.text,
            "position": self.position, "paused": app.media.snapshot.paused,
            "mode": app.audio.repeat_mode.index(),
        }));
        let mut events = vec![];
        if surface == Surface::Focus {
            events.push(native_probe_input::MouseEvent::DownCount(position, 1));
            events.push(native_probe_input::MouseEvent::UpCount(position, 1));
        }
        events.push(native_probe_input::MouseEvent::DownCount(position, 2));
        events.push(native_probe_input::MouseEvent::UpCount(position, 2));
        native_probe_input::send(window, cx, events)
    }
    fn copy_shortcut(window: &Window, cx: &mut Context<AppView>) {
        let handle = window.window_handle();
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| {
                window.dispatch_keystroke(
                    Keystroke::parse(if cfg!(target_os = "macos") {
                        "cmd-c"
                    } else {
                        "ctrl-c"
                    })
                    .unwrap(),
                    cx,
                );
            });
        });
    }
    fn click_offer(
        app: &AppView,
        ai: bool,
        window: &Window,
        cx: &mut Context<AppView>,
    ) -> Result<()> {
        let (_, position) = app
            .assistant
            .offer
            .as_ref()
            .context("no selection action bar")?;
        // Public selection toolbar contract: two equal buttons in a 232 × 44 card.
        let left = position
            .x
            .min(window.viewport_size().width - px(240.))
            .max(px(8.));
        let top = (position.y + px(12.))
            .min(window.viewport_size().height - px(52.))
            .max(px(8.));
        let p = point(left + px(if ai { 174. } else { 58. }), top + px(22.));
        Self::draw(window, cx);
        native_probe_input::send(
            window,
            cx,
            vec![
                native_probe_input::MouseEvent::Down(p),
                native_probe_input::MouseEvent::Up(p),
            ],
        )
    }
    fn selected(&self, app: &AppView) -> Result<()> {
        ensure!(
            app.subtitle_text.borrow().selection_text() == self.expected,
            "audio drag did not select exact text: expected {:?}, got {:?}",
            self.expected,
            app.subtitle_text.borrow().selection_text()
        );
        ensure!(
            app.assistant
                .offer
                .as_ref()
                .is_some_and(|(r, _)| r.text == self.expected),
            "audio selection did not offer the same text to AI"
        );
        ensure!(
            app.audio.repeat_mode == PlaybackMode::SentenceThree
                && app.media.snapshot.paused
                && (app.media.snapshot.position - self.position).abs() < 0.15,
            "drag changed playback position or three-repeat mode"
        );
        Ok(())
    }
    fn copy_full(app: &AppView, window: &Window, cx: &mut Context<AppView>) -> Result<()> {
        let bounds = app
            .subtitle_text
            .borrow()
            .probe_audio_copy
            .context("no visible full-transcript copy button in subtitle tools")?;
        let p = bounds.center();
        native_probe_input::send(
            window,
            cx,
            vec![
                native_probe_input::MouseEvent::Down(p),
                native_probe_input::MouseEvent::Up(p),
            ],
        )
    }
    fn assert_quote(&self, app: &AppView) -> Result<()> {
        ensure!(
            app.assistant_open
                && app
                    .assistant
                    .references
                    .last()
                    .is_some_and(|r| r.text == self.expected),
            "audio selection not attached to assistant exactly"
        );
        ensure!(
            !app.tasks.foreground.is_running() && !app.chat.running,
            "quoting audio sent an unsolicited AI request"
        );
        Ok(())
    }
    pub fn step(
        &mut self,
        app: &mut AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
        cases: &mut Vec<Value>,
        observations: &mut Vec<Value>,
    ) -> Result<bool> {
        let observed = json!({"audio_selection_stage":self.stage,"audio":app.audio.current_id,"cues":app.audio.cues.iter().map(|c|c.text.clone()).collect::<Vec<_>>(),"restore_pending":app.loads.restore_pending,"position":app.media.snapshot.position,"paused":app.media.snapshot.paused,"mode":app.audio.repeat_mode.index()});
        if observed != self.last_observation {
            observations.push(observed.clone());
            self.last_observation = observed;
        }
        let result = self.advance(app, window, cx, cases, observations);
        if result.is_err() || matches!(result, Ok(true)) {
            self.cleanup(cx);
        }
        result
    }
    fn advance(
        &mut self,
        app: &mut AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
        cases: &mut Vec<Value>,
        observations: &mut Vec<Value>,
    ) -> Result<bool> {
        if self.stage > 1 && self.started.elapsed() < Duration::from_millis(250) {
            return Ok(false);
        }
        match self.stage {
            0 => {
                self.clipboard = Some(cx.read_from_clipboard());
                self.show_current = app.audio.show_current;
                if app.assistant_open {
                    app.handle(Action::Assistant, window, cx);
                }
                app.handle(Action::Page("listening".into()), window, cx);
                app.handle(Action::Audio("selection-audio".into()), window, cx);
                ensure!(
                    app.audio.current_id == "selection-audio",
                    "could not select fixture audio; status={}, page={}, available={:?}",
                    app.status,
                    app.page,
                    app.audio
                        .audios
                        .iter()
                        .map(|r| text(r, "id"))
                        .collect::<Vec<_>>()
                );
            }
            1 => {
                if app.audio.current_id != "selection-audio"
                    || app.loads.restore_pending
                    || app.audio.cues.len() != 2
                    || !app.audio.cues[0].text.starts_with("When is")
                {
                    return Ok(false);
                }
                if !app.audio.show_current {
                    app.handle(Action::AudioCurrent, window, cx);
                }
                app.handle(Action::AudioRepeat(6), window, cx);
                app.media.player.set("pause", "yes");
                app.media.player.command(["seek", "0.4", "absolute+exact"]);
                Self::draw(window, cx);
            }
            2 => {
                if !app.media.snapshot.paused
                    || !(0.29..0.81).contains(&app.media.snapshot.position)
                {
                    return Ok(false);
                }
                if !self.focus_drawn {
                    self.focus_drawn = true;
                    Self::draw(window, cx);
                    self.started = Instant::now();
                    return Ok(false);
                }
                self.position = app.media.snapshot.position;
                self.drag(app, Surface::Focus, 5..17, window, cx)?;
            }
            3 => {
                observations.push(json!({"audio_selection_surface":"current-card","expected":self.expected,"selected":app.subtitle_text.borrow().selection_text(),"offer":app.assistant.offer.as_ref().map(|(r,_)|r.text.clone())}));
                self.selected(app)?;
                cases.push(json!({"case":"audio current card drag selects words without seeking or leaving triple repeat","passed":true}));
                Self::click_offer(app, false, window, cx)?;
            }
            4 => {
                ensure!(
                    cx.read_from_clipboard().and_then(|v| v.text()).as_deref()
                        == Some(self.expected.as_str()),
                    "audio selection popup did not copy selected text"
                );
                cases.push(json!({"case":"audio selection floating copy button writes exact OS clipboard text","passed":true}));
                self.drag(app, Surface::Focus, 5..17, window, cx)?;
            }
            5 => {
                self.selected(app)?;
                Self::click_offer(app, true, window, cx)?;
            }
            6 => {
                self.assert_quote(app)?;
                cases.push(json!({"case":"audio current card ask AI attaches only selected words without sending","passed":true}));
                app.handle(Action::AssistantRemove(0), window, cx);
                app.handle(Action::Assistant, window, cx);
                if app.audio.show_current {
                    app.handle(Action::AudioCurrent, window, cx);
                }
                Self::draw(window, cx);
            }
            7 => {
                let doc = Self::document(app);
                self.drag(
                    app,
                    Surface::Audio,
                    12..doc.cues[1].translation.end,
                    window,
                    cx,
                )?;
            }
            8 => {
                self.selected(app)?;
                observations
                    .push(json!({"audio_selection_surface":"transcript","selected":self.expected}));
                cases.push(json!({"case":"audio transcript selects across cues Chinese and emoji exactly","passed":true}));
                Self::copy_shortcut(window, cx);
            }
            9 => {
                ensure!(
                    cx.read_from_clipboard().and_then(|v| v.text()).as_deref()
                        == Some(self.expected.as_str()),
                    "audio transcript shortcut copied wrong text"
                );
                cases.push(json!({"case":"audio subtitle native keyboard copy preserves bilingual cross-cue selection","passed":true}));
                Self::click_offer(app, true, window, cx)?;
            }
            10 => {
                self.assert_quote(app)?;
                cases.push(json!({"case":"audio transcript ask AI preserves exact multilingual selection","passed":true}));
                app.handle(Action::AssistantRemove(0), window, cx);
                app.handle(Action::Assistant, window, cx);
                app.handle(Action::AudioBlind(true), window, cx);
                Self::draw(window, cx);
            }
            11 => {
                ensure!(
                    !app.has_subtitle_selection()
                        && app.assistant.offer.is_none()
                        && app.subtitle_text.borrow().probe_audio_bounds().is_empty(),
                    "blind listening leaked prior subtitle selection"
                );
                cases.push(json!({"case":"audio blind mode clears selection and offers no hidden subtitle quote","passed":true}));
                app.handle(Action::AudioBlind(false), window, cx);
                app.handle(Action::Audio("repeat-other".into()), window, cx);
                self.focus_drawn = false;
            }
            12 => {
                if app.audio.current_id != "repeat-other"
                    || app.loads.restore_pending
                    || !app
                        .audio
                        .cues
                        .first()
                        .is_some_and(|c| c.text.starts_with("Alpha"))
                {
                    return Ok(false);
                }
                if !self.focus_drawn {
                    self.focus_drawn = true;
                    app.media.player.set("pause", "yes");
                    Self::draw(window, cx);
                    self.started = Instant::now();
                    return Ok(false);
                }
                ensure!(
                    !app.has_subtitle_selection() && app.assistant.offer.is_none(),
                    "switch audio retained selection"
                );
                let doc = Self::document(app);
                let (start, _) = Self::points(
                    app,
                    Surface::Audio,
                    doc.cues[1].original.start..doc.cues[1].original.start + 1,
                )?;
                native_probe_input::send(
                    window,
                    cx,
                    vec![
                        native_probe_input::MouseEvent::Down(start),
                        native_probe_input::MouseEvent::Up(start),
                    ],
                )?;
            }
            13 => {
                if !app
                    .media
                    .session
                    .audio_sentence()
                    .is_some_and(|s| s.index == 1)
                {
                    return Ok(false);
                }
                ensure!(
                    app.audio.repeat_mode == PlaybackMode::SentenceThree
                        && !app.has_subtitle_selection(),
                    "single click stopped selecting a sentence or changed repeat mode"
                );
                cases.push(json!({"case":"audio subtitle single click still seeks and retains triple repeat","passed":true}));
                app.handle(Action::Audio("repeat-audio".into()), window, cx);
                app.media.player.set("pause", "yes");
            }
            14 => {
                if app.audio.current_id != "repeat-audio"
                    || app.loads.restore_pending
                    || !app.media.snapshot.paused
                {
                    return Ok(false);
                }
                if app.audio.show_current != self.show_current {
                    app.handle(Action::AudioCurrent, window, cx);
                }
                app.subtitle_text.borrow_mut().clear();
                app.handle(Action::Audio("selection-audio".into()), window, cx);
                self.focus_drawn = false;
            }
            15 => {
                if app.audio.current_id != "selection-audio"
                    || app.loads.restore_pending
                    || app.audio.cues.len() != 2
                {
                    return Ok(false);
                }
                app.media.player.set("pause", "yes");
                Self::draw(window, cx);
            }
            16 => {
                if !app.media.snapshot.paused {
                    return Ok(false);
                }
                self.position = app.media.snapshot.position;
                Self::copy_full(app, window, cx)?;
            }
            17 => {
                let expected = "When is the train leaving?\n火车🌱什么时候出发？\nWhy did you leave?\n你为什么离开？";
                let actual = cx
                    .read_from_clipboard()
                    .and_then(|v| v.text())
                    .unwrap_or_default();
                ensure!(
                    actual == expected,
                    "full audio copy omitted or added text: {actual:?}"
                );
                ensure!(
                    app.media.snapshot.paused
                        && app.audio.repeat_mode == PlaybackMode::SentenceThree
                        && (app.media.snapshot.position - self.position).abs() < 0.15,
                    "full copy changed audio playback"
                );
                fs::write(app.lib.root.join("audio-transcript-copied.txt"), &actual)?;
                observations.push(json!({"full_audio_copy":actual,"button_bounds":format!("{:?}",app.subtitle_text.borrow().probe_audio_copy),"paused":app.media.snapshot.paused,"mode":app.audio.repeat_mode.index()}));
                cases.push(json!({"case":"visible audio copy full button copies complete bilingual transcript without changing playback","passed":true}));
                app.handle(Action::Audio("repeat-other".into()), window, cx);
                self.focus_drawn = false;
            }
            18 => {
                if app.audio.current_id != "repeat-other"
                    || app.loads.restore_pending
                    || app.audio.cues.len() != 3
                {
                    return Ok(false);
                }
                if !self.focus_drawn {
                    self.focus_drawn = true;
                    app.media.player.set("pause", "yes");
                    Self::draw(window, cx);
                    self.started = Instant::now();
                    return Ok(false);
                }
                Self::copy_full(app, window, cx)?;
            }
            19 => {
                let expected = "Alpha sentence.\nBeta sentence.\nGamma sentence.";
                ensure!(
                    cx.read_from_clipboard().and_then(|v| v.text()).as_deref() == Some(expected),
                    "copy full retained the previous audio"
                );
                cases.push(json!({"case":"audio copy full follows the newly selected audio","passed":true}));
                app.handle(
                    Action::AudioCopySubtitle("selection-audio".into(), None),
                    window,
                    cx,
                );
                ensure!(
                    cx.read_from_clipboard().and_then(|v| v.text()).as_deref() == Some(expected),
                    "stale audio copy replaced the clipboard"
                );
                cases.push(json!({"case":"stale audio full copy action cannot copy old material","passed":true}));
                app.handle(Action::AudioBlind(true), window, cx);
                Self::draw(window, cx);
            }
            20 => {
                ensure!(
                    app.subtitle_text.borrow().probe_audio_copy.is_none(),
                    "full copy button exposed during blind listening"
                );
                app.handle(
                    Action::AudioCopySubtitle(app.audio.current_id.clone(), None),
                    window,
                    cx,
                );
                ensure!(
                    cx.read_from_clipboard().and_then(|v| v.text()).as_deref()
                        == Some("Alpha sentence.\nBeta sentence.\nGamma sentence."),
                    "blind mode exposed the full transcript"
                );
                cases.push(json!({"case":"blind listening hides and blocks full audio copy","passed":true}));
                app.handle(Action::AudioBlind(false), window, cx);
                app.handle(Action::Audio("repeat-audio".into()), window, cx);
                app.media.player.set("pause", "yes");
            }
            21 => {
                if app.audio.current_id != "repeat-audio"
                    || app.loads.restore_pending
                    || !app.media.snapshot.paused
                {
                    return Ok(false);
                }
                let subtitle = app.lib.root.join("media/word-double-click.srt");
                fs::write(
                    &subtitle,
                    "1\n00:00:00,300 --> 00:00:00,800\nThis illustrates, punctuation.\n",
                )?;
                app.lib.put("listening_audios", json!({
                    "id": "word-double-click-audio", "practice_id": "probe-group-a",
                    "name": "Punctuated word double click fixture",
                    "audio_uri": app.lib.root.join("media/repeat.wav"), "subtitle_uri": subtitle,
                }))?;
                app.refresh();
            }
            22 => {
                if !app
                    .audio
                    .audios
                    .iter()
                    .any(|row| text(row, "id") == "word-double-click-audio")
                {
                    return Ok(false);
                }
                app.handle(Action::Audio("word-double-click-audio".into()), window, cx);
                if !app.audio.show_current {
                    app.handle(Action::AudioCurrent, window, cx);
                }
            }
            23 => {
                if app.audio.current_id != "word-double-click-audio"
                    || app.loads.restore_pending
                    || app.audio.cues.len() != 1
                    || app.audio.cues[0].text != "This illustrates, punctuation."
                {
                    return Ok(false);
                }
                app.media.player.set("pause", "yes");
                app.media.player.command(["seek", "0.4", "absolute+exact"]);
                Self::draw(window, cx);
            }
            24 => {
                if !app.media.snapshot.paused || (app.media.snapshot.position - 0.4).abs() > 0.05 {
                    return Ok(false);
                }
                self.double_click_word(app, Surface::Focus, window, cx, observations)?;
            }
            25 | 28 => {
                observations.push(json!({
                    "event": "native word double click result", "audio_selection_stage": self.stage,
                    "expected": self.expected, "selected": app.subtitle_text.borrow().selection_text(),
                    "offer": app.assistant.offer.as_ref().map(|(r, _)| r.text.clone()),
                    "position": app.media.snapshot.position, "paused": app.media.snapshot.paused,
                    "mode": app.audio.repeat_mode.index(),
                }));
                self.selected(app)?;
                if self.stage == 25 {
                    cases.push(json!({"case":"audio current card native double click selects a word without punctuation","passed":true}));
                    Self::click_offer(app, false, window, cx)?;
                } else {
                    cases.push(json!({"case":"audio transcript native double click selects a word without punctuation","passed":true}));
                    Self::copy_shortcut(window, cx);
                }
            }
            26 | 29 => {
                let copied = cx
                    .read_from_clipboard()
                    .and_then(|v| v.text())
                    .unwrap_or_default();
                ensure!(
                    copied == "illustrates",
                    "double-clicked word copied wrong text: {copied:?}"
                );
                let name = if self.stage == 26 {
                    "current-card"
                } else {
                    "transcript"
                };
                fs::write(
                    app.lib.root.join(format!("double-click-{name}-copied.txt")),
                    &copied,
                )?;
                observations.push(json!({"event":"double clicked word OS clipboard", "surface":name,"clipboard":copied}));
                if self.stage == 26 {
                    cases.push(json!({"case":"audio current card double clicked word floating copy writes exact OS clipboard text","passed":true}));
                    app.subtitle_text.borrow_mut().clear();
                    app.assistant.offer = None;
                    app.handle(Action::AudioCurrent, window, cx);
                    Self::draw(window, cx);
                } else {
                    cases.push(json!({"case":"audio transcript double clicked word keyboard copy writes exact OS clipboard text","passed":true}));
                    self.word_request = app
                        .media
                        .session
                        .audio_sentence()
                        .context("word double-click sentence request")?
                        .request_id;
                    let doc = Self::document(app);
                    let index = doc
                        .text
                        .find("punctuation.")
                        .context("missing single-click word fixture")?
                        + 4;
                    let (point, _) = Self::points(app, Surface::Audio, index..index + 1)?;
                    observations.push(json!({"event":"native ordinary single click after double-click copy",
                        "click_count":1,"click":[f32::from(point.x),f32::from(point.y)],
                        "previous_request":self.word_request,"position":app.media.snapshot.position}));
                    native_probe_input::send(
                        window,
                        cx,
                        vec![
                            native_probe_input::MouseEvent::DownCount(point, 1),
                            native_probe_input::MouseEvent::UpCount(point, 1),
                        ],
                    )?;
                }
            }
            27 => self.double_click_word(app, Surface::Audio, window, cx, observations)?,
            30 => {
                if app.media.snapshot.paused
                    || !app
                        .media
                        .session
                        .audio_sentence()
                        .is_some_and(|session| session.request_id != self.word_request)
                {
                    ensure!(
                        self.started.elapsed() < Duration::from_secs(2),
                        "ordinary subtitle single click after double click did not resume sentence playback"
                    );
                    return Ok(false);
                }
                ensure!(
                    app.audio.repeat_mode == PlaybackMode::SentenceThree
                        && !app.has_subtitle_selection(),
                    "ordinary subtitle single click after double click retained selection or changed mode"
                );
                cases.push(json!({"case":"audio ordinary subtitle single click still seeks after double-click copy","passed":true}));
                observations.push(json!({"event":"native ordinary single click after double-click result",
                    "request":app.media.session.audio_sentence().map(|session|session.request_id),
                    "previous_request":self.word_request,"position":app.media.snapshot.position,
                    "paused":app.media.snapshot.paused,"selected":app.subtitle_text.borrow().selection_text(),
                    "mode":app.audio.repeat_mode.index()}));
                app.handle(Action::Audio("repeat-audio".into()), window, cx);
                app.media.player.set("pause", "yes");
                if app.audio.show_current != self.show_current {
                    app.handle(Action::AudioCurrent, window, cx);
                }
            }
            31 => {
                if app.audio.current_id != "repeat-audio"
                    || app.loads.restore_pending
                    || !app.media.snapshot.paused
                {
                    return Ok(false);
                }
                return Ok(true);
            }
            _ => unreachable!(),
        }
        self.stage += 1;
        self.started = Instant::now();
        Ok(false)
    }
}
