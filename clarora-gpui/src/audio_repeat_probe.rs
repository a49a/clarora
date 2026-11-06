//! Hidden-window E2E of actual audio actions and libmpv sentence requests.
use super::*;

pub(super) fn prepare(out: &Path) -> Result<Library> {
    let lib = ui_controller_probe::prepare(out)?;
    let wav = lib.root.join("media/repeat.wav");
    let mut writer = hound::WavWriter::create(
        &wav,
        hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )?;
    for i in 0..64000 {
        writer.write_sample(((i as f32 / 10.).sin() * 1000.) as i16)?;
    }
    writer.finalize()?;
    let srt = lib.root.join("media/repeat.srt");
    fs::write(
        &srt,
        "1\n00:00:00,300 --> 00:00:00,800\nAlpha sentence.\n\n2\n00:00:01,200 --> 00:00:01,700\nBeta sentence.\n\n3\n00:00:02,000 --> 00:00:02,500\nGamma sentence.\n",
    )?;
    for id in ["repeat-audio", "repeat-other"] {
        lib.put("listening_audios",json!({"id":id,"practice_id":"probe-group-a","name":id,"audio_uri":wav,"subtitle_uri":srt}))?;
    }
    let selection = lib.root.join("media/selection.srt");
    fs::write(
        &selection,
        "1\n00:00:00,300 --> 00:00:00,800\nWhen is the train leaving?\n火车🌱什么时候出发？\n\n2\n00:00:01,200 --> 00:00:01,700\nWhy did you leave?\n你为什么离开？\n",
    )?;
    lib.put("listening_audios",json!({"id":"selection-audio","practice_id":"probe-group-a","name":"Bilingual selection fixture","audio_uri":wav,"subtitle_uri":selection}))?;
    Ok(lib)
}

pub(super) struct Probe {
    out: PathBuf,
    started: Instant,
    stage: usize,
    cases: Vec<Value>,
    request: u64,
    plays: u32,
    infinite_seek: bool,
    finite_resume: bool,
    observations: Vec<Value>,
    current_card_stage: u8,
    current_card_click: Option<Instant>,
    transcript_top: Pixels,
    assistant_selection: assistant_selection_probe::Probe,
    audio_selection: audio_selection_probe::Flow,
}
impl Probe {
    pub(super) fn new(out: &Path) -> Self {
        Self {
            out: out.into(),
            started: Instant::now(),
            stage: 0,
            cases: vec![],
            request: 0,
            plays: 1,
            infinite_seek: false,
            finite_resume: false,
            observations: vec![],
            current_card_stage: 0,
            current_card_click: None,
            transcript_top: px(0.),
            assistant_selection: assistant_selection_probe::Probe::new(),
            audio_selection: audio_selection_probe::Flow::new(),
        }
    }
    fn note(&mut self, name: &str, app: &AppView) {
        self.cases.push(json!({"case":name,"passed":true,"mode":app.audio.repeat_mode,"cue":app.audio.sentence_repeat.map(|s|s.index),"playback":app.media.snapshot.sentence_repeat.as_ref().map(|r|json!({"request":r.request_id,"current":r.current_play,"total":r.total_plays,"finished":r.finished}))}));
    }
    fn report(&self, passed: bool, error: Option<String>) -> Result<()> {
        fs::write(
            self.out.join("audio-repeat-report.json"),
            serde_json::to_vec_pretty(
                &json!({"passed":passed,"stage":self.stage,"error":error,"cases":self.cases,"observations":self.observations,"real_libmpv":true,"hidden_native_window":true,"physical_input_verified":false}),
            )?,
        )?;
        Ok(())
    }
    fn draw(window: &Window, cx: &mut Context<AppView>) {
        let handle = window.window_handle();
        cx.defer(move |cx| {
            handle
                .update(cx, |_, window, cx| window.draw(cx).clear())
                .expect("draw current sentence diagnostic");
        });
    }
    fn click_current_close(
        app: &AppView,
        window: &Window,
        cx: &mut Context<AppView>,
    ) -> Result<()> {
        let bounds = app.audio.current_scroll.bounds();
        ensure!(
            bounds.size.width > px(100.) && bounds.size.height > px(50.),
            "current sentence card has no rendered bounds"
        );
        let position = point(bounds.right() - px(32.), bounds.top() - px(16.));
        native_probe_input::send(
            window,
            cx,
            vec![
                native_probe_input::MouseEvent::Down(position),
                native_probe_input::MouseEvent::Up(position),
            ],
        )
    }
    fn current_card_step(
        &mut self,
        app: &mut AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Result<bool> {
        match self.current_card_stage {
            0 => {
                if !app.media.snapshot.paused || app.media.snapshot.position > 0.1 {
                    return Ok(false);
                }
                ensure!(app.audio.show_current, "current sentence card not enabled");
                Self::draw(window, cx);
            }
            1 => {
                ensure!(app.audio.active.is_none(), "expected waiting sentence card");
                self.transcript_top = app.audio.scroll.viewport_bounds().top();
                ensure!(self.transcript_top > px(0.), "subtitle list not rendered");
                let bounds = app.audio.current_scroll.bounds();
                self.observations.push(json!({
                    "current_card_body": [
                        f32::from(bounds.left()),
                        f32::from(bounds.top()),
                        f32::from(bounds.right()),
                        f32::from(bounds.bottom())
                    ],
                    "click": [
                        f32::from(bounds.right() - px(32.)),
                        f32::from(bounds.top() - px(16.))
                    ],
                    "transcript_top": f32::from(self.transcript_top)
                }));
                Self::click_current_close(app, window, cx)?;
                self.current_card_click = Some(Instant::now());
            }
            2 => {
                if app.audio.show_current {
                    if self
                        .current_card_click
                        .is_none_or(|t| t.elapsed() >= Duration::from_secs(2))
                    {
                        let mouse = window.mouse_position();
                        self.observations.push(json!({
                            "mouse_after_click": [f32::from(mouse.x), f32::from(mouse.y)],
                            "card_visible": app.audio.show_current
                        }));
                        anyhow::bail!(
                            "clicking the waiting sentence close button did not hide the card"
                        );
                    }
                    return Ok(false);
                }
                Self::draw(window, cx);
            }
            3 => {
                ensure!(
                    !app.audio.show_current,
                    "clicking the waiting sentence close button did not hide the card"
                );
                ensure!(
                    app.audio.scroll.viewport_bounds().top() + px(100.) < self.transcript_top,
                    "hidden sentence card still occupies space"
                );
                ensure!(
                    app.audio.current_id == "repeat-audio"
                        && app.audio.cues.len() == 3
                        && app.media.snapshot.paused
                        && app.audio.repeat_mode == 0,
                    "closing the sentence card changed audio playback or subtitles"
                );
                self.note(
                    "close waiting sentence card and reclaim subtitle space",
                    app,
                );
                app.handle(Action::AudioCurrent, window, cx);
                Self::draw(window, cx);
            }
            4 => {
                ensure!(
                    app.audio.show_current
                        && app.audio.scroll.viewport_bounds().top() > self.transcript_top - px(2.),
                    "subtitle options could not reopen the sentence card"
                );
                self.note("existing current sentence action reopens the card", app);
                app.media.player.command(["seek", "0.5", "absolute+exact"]);
            }
            5 => {
                if (app.media.snapshot.position - 0.5).abs() > 0.04 || app.audio.active != Some(0) {
                    return Ok(false);
                }
                Self::draw(window, cx);
            }
            6 => {
                Self::click_current_close(app, window, cx)?;
                self.current_card_click = Some(Instant::now());
            }
            7 => {
                if app.audio.show_current {
                    ensure!(
                        self.current_card_click
                            .is_some_and(|t| t.elapsed() < Duration::from_secs(2)),
                        "clicking the active sentence close button did not hide the card"
                    );
                    return Ok(false);
                }
                ensure!(
                    !app.audio.show_current
                        && app.media.snapshot.paused
                        && (app.media.snapshot.position - 0.5).abs() < 0.04
                        && app.audio.current_id == "repeat-audio"
                        && app.audio.cues.len() == 3,
                    "closing the active sentence card changed playback or failed to hide"
                );
                self.note("close active sentence card without moving playback", app);
                app.handle(Action::AudioCurrent, window, cx);
                app.media.player.command(["seek", "0", "absolute+exact"]);
                Self::draw(window, cx);
            }
            8 => {
                if app.media.snapshot.position > 0.1 {
                    return Ok(false);
                }
                ensure!(
                    app.audio.show_current && app.media.snapshot.paused,
                    "sentence card could not be restored after closing an active cue"
                );
                self.note("restore sentence card after closing an active cue", app);
                self.current_card_stage = 9;
                return Ok(true);
            }
            9 => return Ok(true),
            _ => unreachable!(),
        }
        self.current_card_stage += 1;
        Ok(false)
    }
    fn step(
        &mut self,
        app: &mut AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Result<bool> {
        ensure!(
            self.started.elapsed() < Duration::from_secs(35),
            "audio repeat timeout stage {}, current card stage {}",
            self.stage,
            self.current_card_stage
        );
        app.poll_data();
        app.media.poll();
        app.poll_audio_sentence_repeat();
        let state = app.media.player.snapshot();
        let observation = json!({"stage":self.stage,"position":state.position,"paused":state.paused,"expected_play":self.plays,"expected_request":self.request,"repeat":state.sentence_repeat.as_ref().map(|r|json!({"id":r.request_id,"play":r.current_play,"finished":r.finished})),"error":state.error});
        if self.observations.last() != Some(&observation) && self.observations.len() < 300 {
            self.observations.push(observation);
        }
        match self.stage {
            0 => {
                app.handle(Action::Page("listening".into()), window, cx);
            }
            1 => {
                if app.audio.current_id != "repeat-audio" {
                    if !app
                        .audio
                        .audios
                        .iter()
                        .any(|r| text(r, "id") == "repeat-audio")
                    {
                        return Ok(false);
                    }
                    app.handle(Action::Audio("repeat-audio".into()), window, cx);
                    app.media.player.set("pause", "yes");
                    app.media.player.command(["seek", "0", "absolute+exact"]);
                    return Ok(false);
                }
                if app.audio.cues.len() != 3
                    || app.media.snapshot.duration <= 0.
                    || app.media.snapshot.path
                        != app
                            .media
                            .path
                            .as_ref()
                            .context("audio path")?
                            .to_string_lossy()
                {
                    return Ok(false);
                }
                if !self.current_card_step(app, window, cx)? {
                    return Ok(false);
                }
                if !self.assistant_selection.step(
                    app,
                    window,
                    cx,
                    &mut self.cases,
                    &mut self.observations,
                )? {
                    return Ok(false);
                }
                app.handle(Action::AudioRepeat(5), window, cx);
                ensure!(
                    app.audio.sentence_repeat.is_some(),
                    "single cue mode did not start"
                );
                self.note("single sentence selected without selecting subtitle", app);
            }
            2 => {
                if !app
                    .media
                    .snapshot
                    .sentence_repeat
                    .as_ref()
                    .is_some_and(|s| s.total_plays.is_none() && s.current_play >= 2)
                {
                    return Ok(false);
                }
                if !self.infinite_seek {
                    app.media.player.set("pause", "yes");
                    app.seek_audio_repeat(0.55, false)?;
                    self.infinite_seek = true;
                    return Ok(false);
                }
                if !app.media.snapshot.paused || (app.media.snapshot.position - 0.55).abs() > 0.04 {
                    return Ok(false);
                }
                self.note("infinite sentence loop supports precise paused seek", app);
                ensure!(
                    app.audio.sentence_repeat.context("loop")?.index == 0,
                    "single loop advanced"
                );
                self.note("real infinite loop repeats only one cue", app);
                app.handle(Action::AudioRepeat(6), window, cx);
                let reopened = Library::open(app.lib.root.clone())?;
                ensure!(
                    app_state::AudioState::new(&reopened).repeat_mode == 6,
                    "repeat preference not persisted"
                );
                self.note("repeat preference survives SQLite reopen", app);
            }
            3 => {
                if !app.audio.sentence_repeat.is_some_and(|s| s.index == 1) {
                    return Ok(false);
                }
                self.note("three real plays advance to next cue", app);
                app.handle(Action::AudioSentence(-1), window, cx);
                ensure!(
                    app.audio.sentence_repeat.context("previous")?.index == 0,
                    "previous cue failed"
                );
                app.handle(Action::AudioSentence(1), window, cx);
                app.handle(Action::AudioSentence(1), window, cx);
                ensure!(
                    app.audio.sentence_repeat.context("rapid next")?.index == 2
                        && app.audio.repeat_mode == 6,
                    "rapid arrows used stale playback"
                );
                self.note(
                    "rapid previous and next retain mode and selected cursor",
                    app,
                );
                app.media.player.set("pause", "yes");
            }
            4 => {
                if !app.media.snapshot.paused {
                    return Ok(false);
                }
                self.request = app.audio.sentence_repeat.context("seek loop")?.request_id;
                if !app
                    .media
                    .snapshot
                    .sentence_repeat
                    .as_ref()
                    .is_some_and(|r| r.request_id == self.request)
                {
                    return Ok(false);
                }
                self.plays = app
                    .media
                    .snapshot
                    .sentence_repeat
                    .as_ref()
                    .unwrap()
                    .current_play;
                app.handle(Action::SeekTo(99.), window, cx);
                ensure!(
                    app.audio
                        .sentence_repeat
                        .context("seek preserves")?
                        .request_id
                        == self.request
                        && app.audio.repeat_mode == 6,
                    "timeline changed request/mode"
                );
            }
            5 => {
                if !self.finite_resume {
                    if app.media.snapshot.position < 2.4
                        || app.media.snapshot.position > 2.5
                        || !app.media.snapshot.paused
                    {
                        return Ok(false);
                    }
                    ensure!(
                        app.media
                            .snapshot
                            .sentence_repeat
                            .as_ref()
                            .is_some_and(
                                |r| r.request_id == self.request && r.current_play == self.plays
                            ),
                        "timeline reset native play counter: expected request={} play={}, actual={:?}",
                        self.request,
                        self.plays,
                        app.media.snapshot.sentence_repeat
                    );
                    self.note(
                        "timeline clamps to selected sentence and preserves pause/request",
                        app,
                    );
                    app.handle(Action::SeekTo(2.2), window, cx);
                    app.handle(Action::Play, window, cx);
                    self.finite_resume = true;
                    return Ok(false);
                }
                if app.media.snapshot.paused
                    || !app
                        .media
                        .snapshot
                        .sentence_repeat
                        .as_ref()
                        .is_some_and(|r| {
                            r.request_id == self.request
                                && r.current_play >= self.plays
                                && !r.finished
                        })
                {
                    return Ok(false);
                }
                self.note(
                    "resume after paused positioning rearms the same sentence request",
                    app,
                );
                app.inputs["question"].update(cx, |s, cx| s.focus(window, cx));
                let key = KeyDownEvent {
                    keystroke: Keystroke::parse("left").expect("left key"),
                    is_held: false,
                };
                ensure!(
                    !app.handle_audio_arrow_key(&key, window, cx),
                    "editor arrows were stolen"
                );
                window.focus(&app.focus);
                ensure!(
                    app.handle_audio_arrow_key(&key, window, cx),
                    "audio arrow not handled"
                );
                ensure!(
                    app.audio.sentence_repeat.context("keyboard")?.index == 1,
                    "left did not select previous cue"
                );
                self.note("actual key handler respects editor focus", app);
                app.handle(Action::Word(0.4, 0.5, "Alpha".into(), false), window, cx);
                ensure!(
                    app.audio.sentence_repeat.context("word")?.index == 0
                        && app.audio.repeat_mode == 6,
                    "subtitle click exited repetition"
                );
                self.note("subtitle selection retains three-play mode", app);
                app.media.player.set("pause", "yes");
                app.seek_audio_repeat(0.55, false)?;
            }
            6 => {
                if !app.media.snapshot.paused || (app.media.snapshot.position - 0.55).abs() > 0.04 {
                    return Ok(false);
                }
                self.note("paused in-sentence position observed before leaving", app);
                app.handle(Action::Page("settings".into()), window, cx);
                ensure!(
                    app.audio.repeat_mode == 6 && app.audio.sentence_repeat.is_none(),
                    "page lost preference or left native repeat active"
                );
                app.handle(Action::Page("listening".into()), window, cx);
            }
            7 => {
                if !app.audio.sentence_repeat.is_some_and(|s| {
                    app.media.snapshot.paused
                        && app
                            .media
                            .snapshot
                            .sentence_repeat
                            .as_ref()
                            .is_some_and(|r| r.request_id == s.request_id)
                }) {
                    return Ok(false);
                }
                ensure!(
                    app.audio.sentence_repeat.unwrap().index == 0,
                    "return restored stale player position instead of selected sentence"
                );
                if (app.media.snapshot.position - 0.55).abs() > 0.04 {
                    return Ok(false);
                }
                self.note("returning to audio retains repeat preference", app);
                app.handle(Action::Audio("repeat-other".into()), window, cx);
            }
            8 => {
                if app.audio.current_id != "repeat-other" || app.audio.sentence_repeat.is_none() {
                    return Ok(false);
                }
                ensure!(
                    app.audio.repeat_mode == 6
                        && app.audio.sentence_repeat.is_some_and(|s| s.index == 0),
                    "audio switch lost mode or started on old cue"
                );
                self.note("audio switch starts a new sentence request", app);
                app.handle(Action::AudioSentence(1), window, cx);
                app.handle(Action::AudioSentence(1), window, cx);
            }
            9 => {
                if !app
                    .media
                    .snapshot
                    .sentence_repeat
                    .as_ref()
                    .is_some_and(|s| {
                        s.finished
                            && app
                                .audio
                                .sentence_repeat
                                .is_some_and(|session| session.request_id == s.request_id)
                    })
                {
                    return Ok(false);
                }
                ensure!(
                    app.audio.sentence_repeat.context("last")?.index == 2
                        && app.audio.current_id == "repeat-other"
                        && app.media.snapshot.paused,
                    "last cue did not stop"
                );
                self.note(
                    "last cue stops after three and does not auto-switch audio",
                    app,
                );
                app.handle(Action::Play, window, cx);
            }
            10 => {
                if app.media.snapshot.paused
                    || app
                        .media
                        .snapshot
                        .sentence_repeat
                        .as_ref()
                        .is_none_or(|s| s.finished)
                {
                    return Ok(false);
                }
                self.note("play restarts finished sentence from first repetition", app);
                app.handle(Action::AudioRepeat(0), window, cx);
                ensure!(
                    app.audio.sentence_repeat.is_none() && app.audio.repeat_mode == 0,
                    "exit repetition failed"
                );
                self.note("ordinary playback clears only audio sentence repeat", app);
                let cues = std::mem::take(&mut app.audio.cues);
                app.handle(Action::AudioRepeat(6), window, cx);
                ensure!(
                    app.audio.repeat_mode == 0 && app.audio.sentence_repeat.is_none(),
                    "invalid subtitle mode was accepted"
                );
                app.audio.cues = cues;
                self.note(
                    "missing subtitles reject repetition without changing mode",
                    app,
                );
                app.handle(Action::AudioRepeat(5), window, cx);
                app.handle(Action::AudioClipMode, window, cx);
                app.handle(Action::Boundary(0.3, true), window, cx);
                app.handle(Action::Boundary(0.7, false), window, cx);
                ensure!(
                    app.audio.repeat_mode == 2
                        && app.media.repeat == 2
                        && app.audio.sentence_repeat.is_none(),
                    "A/B conflicts with sentence request"
                );
                self.note("A/B capture replaces sentence loop explicitly", app);
                app.handle(Action::AudioClipMode, window, cx);
                app.handle(Action::AudioRepeat(5), window, cx);
                ensure!(
                    app.open(app.lib.root.join("media/resume-video.mp4")),
                    "direct video open failed"
                );
                ensure!(
                    app.audio.sentence_repeat.is_none()
                        && app.audio.repeat_mode == 5
                        && app.media.repeat == 0
                        && !app.video.repeat_three,
                    "audio repetition leaked into video"
                );
                self.note(
                    "direct video open releases audio request without changing video mode",
                    app,
                );
            }
            11 => {
                if !self.audio_selection.step(
                    app,
                    window,
                    cx,
                    &mut self.cases,
                    &mut self.observations,
                )? {
                    return Ok(false);
                }
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
    pub(super) fn poll_audio_repeat_probe(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(mut probe) = self.audio_repeat_probe.take() else {
            return false;
        };
        match probe.step(self, window, cx) {
            Ok(true) => {
                window.remove_window();
                return true;
            }
            Err(error) => {
                let _ = probe.report(false, Some(format!("{error:#}")));
                window.remove_window();
                return true;
            }
            Ok(false) => {}
        }
        self.audio_repeat_probe = Some(probe);
        false
    }
}
