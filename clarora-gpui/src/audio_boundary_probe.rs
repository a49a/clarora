//! Hidden-window E2E: drag the A/B vertical markers inside real audio subtitle text.
use super::*;
use subtitle_text_ui::Surface;

const AUDIO: &str = "boundary-audio";
const GROUP: &str = "probe-group-a";

pub(super) fn prepare(out: &Path) -> Result<Library> {
    let lib = ui_controller_probe::prepare(out)?;
    let wav = lib.root.join("media/boundary.wav");
    let mut writer = hound::WavWriter::create(
        &wav,
        hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )?;
    for i in 0..48000 {
        writer.write_sample(((i as f32 / 10.).sin() * 1000.) as i16)?;
    }
    writer.finalize()?;
    let srt = lib.root.join("media/boundary.srt");
    fs::write(
        &srt,
        "1\n00:00:00,300 --> 00:00:00,800\nAlpha beta gamma.\n\n2\n00:00:01,200 --> 00:00:01,700\nDelta epsilon zeta.\n",
    )?;
    lib.put(
        "listening_audios",
        json!({
            "id": AUDIO, "practice_id": GROUP, "name": "A/B text marker drag fixture",
            "audio_uri": wav, "subtitle_uri": srt,
        }),
    )?;
    for (id, start, end, front) in [
        ("boundary-clip-a", 1200, 1500, "Delta epsilon"),
        ("boundary-clip-b", 1500, 1700, "zeta."),
    ] {
        lib.put(
            "clip_cards",
            json!({
                "id": id, "en_text": front, "zh_text": "",
                "audio_uri": wav.to_string_lossy(), "start_ms": start, "end_ms": end,
            }),
        )?;
    }
    Ok(lib)
}

pub(super) struct Probe {
    out: PathBuf,
    started: Instant,
    stage_started: Instant,
    stage: u8,
    cases: Vec<Value>,
    observations: Vec<Value>,
    drawn: bool,
    saw_second_review_clip: bool,
    surface: Surface,
}

impl Probe {
    pub fn new(out: &Path) -> Self {
        Self {
            out: out.into(),
            started: Instant::now(),
            stage_started: Instant::now(),
            stage: 0,
            cases: vec![],
            observations: vec![],
            drawn: false,
            saw_second_review_clip: false,
            surface: Surface::Focus,
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
    fn point(app: &AppView, surface: Surface, index: usize) -> Result<Point<Pixels>> {
        let visible = if surface == Surface::Focus {
            app.audio.current_scroll.bounds()
        } else {
            app.audio.scroll.viewport_bounds()
        };
        app.subtitle_text
            .borrow()
            .probe_point(surface, index, visible)
            .with_context(|| {
                format!(
                    "A/B subtitle point {index} was not painted on {surface:?}; visible {visible:?}; fragments {:?}",
                    app.subtitle_text.borrow().probe_fragments(surface)
                )
            })
    }
    fn expected_time(app: &AppView, target: usize) -> Result<f64> {
        let doc = Self::document(app);
        let cue_text = doc.cues.get(1).context("missing second cue")?;
        let cue = app.audio.cues.get(1).context("missing second audio cue")?;
        let local = doc
            .text
            .get(cue_text.original.start..target)
            .context("A/B target outside the original cue")?
            .chars()
            .count() as f64;
        let total = doc
            .text
            .get(cue_text.original.clone())
            .map(|text| text.chars().count())
            .unwrap_or(1)
            .max(1) as f64;
        Ok(cue.start + (cue.end - cue.start) * local / total)
    }
    fn drag(
        window: &Window,
        cx: &mut Context<AppView>,
        from: Point<Pixels>,
        to: Point<Pixels>,
    ) -> Result<()> {
        native_probe_input::send(
            window,
            cx,
            vec![
                native_probe_input::MouseEvent::Down(from),
                native_probe_input::MouseEvent::Drag(to),
                native_probe_input::MouseEvent::Up(to),
            ],
        )
    }
    fn report(&self, passed: bool, error: Option<String>) -> Result<()> {
        fs::write(
            self.out.join("audio-boundary-report.json"),
            serde_json::to_vec_pretty(&json!({
                "passed": passed,
                "stage": self.stage,
                "error": error,
                "cases": self.cases,
                "observations": self.observations,
                "hidden_native_window": true,
                "physical_input_verified": false,
            }))?,
        )?;
        Ok(())
    }
    fn step(
        &mut self,
        app: &mut AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Result<bool> {
        ensure!(
            self.started.elapsed() < Duration::from_secs(30),
            "audio A/B marker drag timeout at stage {}: {}",
            self.stage,
            app.status
        );
        app.poll_data();
        match self.stage {
            0 => {
                app.handle(Action::Page("listening".into()), window, cx);
                app.handle(Action::AudioPractice(GROUP.into()), window, cx);
            }
            1 => {
                if app.audio.practice_id != GROUP
                    || !app.audio.audios.iter().any(|row| text(row, "id") == AUDIO)
                {
                    return Ok(false);
                }
                app.handle(Action::Audio(AUDIO.into()), window, cx);
            }
            2 => {
                if app.audio.current_id != AUDIO
                    || app.loads.restore_pending
                    || app.audio.cues.len() != 2
                {
                    return Ok(false);
                }
                app.media.player.set("pause", "yes");
                app.media.player.command(["seek", "1.4", "absolute+exact"]);
                // The player publishes its position asynchronously. Wait for the
                // seek before opening clip mode, which defaults to the active cue.
                if !app.media.session.is_audio(&app.audio.current_id)
                    || clarora_gpui::video_subtitles::active(
                        &app.audio.cues,
                        app.media.snapshot.position,
                    ) != Some(1)
                {
                    return Ok(false);
                }
                let cue = app.audio.cues[1].clone();
                if !app.audio.clip_mode {
                    app.handle(Action::AudioClipMode, window, cx);
                }
                ensure!(
                    app.audio.clip_mode
                        && app.media.session.loop_a == Some(cue.start)
                        && app.media.session.loop_b == Some(cue.end),
                    "A-B default did not use the active sentence boundaries: clip={} a={:?} b={:?} cue={:?}..{:?} position={} duration={} mode={:?}",
                    app.audio.clip_mode,
                    app.media.session.loop_a,
                    app.media.session.loop_b,
                    cue.start,
                    cue.end,
                    app.media.snapshot.position,
                    app.media.snapshot.duration,
                    app.media.session.mode
                );
                app.handle(Action::Boundary(cue.start, true), window, cx);
                app.handle(Action::Boundary(cue.end, false), window, cx);
                ensure!(
                    app.audio.clip_mode
                        && app.media.session.loop_a == Some(cue.start)
                        && app.media.session.loop_b == Some(cue.end),
                    "could not prepare A/B text markers"
                );
                Self::draw(window, cx);
            }
            3 => {
                if !app.media.snapshot.paused {
                    return Ok(false);
                }
                if !self.drawn {
                    self.drawn = true;
                    Self::draw(window, cx);
                    return Ok(false);
                }
                let focus_painted = !app
                    .subtitle_text
                    .borrow()
                    .probe_fragments(Surface::Focus)
                    .is_empty();
                if !focus_painted {
                    if !app
                        .subtitle_text
                        .borrow()
                        .probe_fragments(Surface::Audio)
                        .is_empty()
                    {
                        self.surface = Surface::Audio;
                    } else {
                        ensure!(
                            self.stage_started.elapsed() < Duration::from_secs(5),
                            "A/B subtitle text was not painted after enabling clip mode"
                        );
                        Self::draw(window, cx);
                        return Ok(false);
                    }
                }
                self.observations.push(json!({
                    "event": "A/B marker surface",
                    "surface": format!("{:?}", self.surface),
                }));
                let doc = Self::document(app);
                let cue_text = doc.cues[1].clone();
                let from = Self::point(app, self.surface, cue_text.original.start)?;
                let target = doc.text.find("epsilon").context("missing A drag target")?;
                let to = Self::point(app, self.surface, target)?;
                Self::drag(window, cx, from, to)?;
            }
            4 => {
                let doc = Self::document(app);
                let cue = app.audio.cues[1].clone();
                let target = doc.text.find("epsilon").context("missing A drag target")?;
                let expected = Self::expected_time(app, target)?;
                if app.media.session.loop_a == Some(cue.start) {
                    ensure!(
                        self.stage_started.elapsed() < Duration::from_secs(2),
                        "dragging the A text marker did not update the boundary"
                    );
                    return Ok(false);
                }
                ensure!(
                    app.media
                        .session
                        .loop_a
                        .is_some_and(|a| (a - expected).abs() < 0.02)
                        && app.media.session.loop_b == Some(cue.end),
                    "A text marker updated the wrong boundary: a={:?} b={:?} expected={expected}",
                    app.media.session.loop_a,
                    app.media.session.loop_b
                );
                ensure!(
                    !app.has_subtitle_selection(),
                    "A text marker drag started a subtitle selection"
                );
                self.observations.push(json!({
                    "event": "A text marker drag", "a": app.media.session.loop_a,
                    "b": app.media.session.loop_b, "expected_a": expected,
                }));
                let from = Self::point(app, self.surface, doc.cues[1].original.end)?;
                let target = doc.text.find("zeta").context("missing B drag target")?;
                let to = Self::point(app, self.surface, target)?;
                Self::drag(window, cx, from, to)?;
            }
            5 => {
                let doc = Self::document(app);
                let target = doc.text.find("zeta").context("missing B drag target")?;
                let expected = Self::expected_time(app, target)?;
                if app.media.session.loop_b == Some(app.audio.cues[1].end)
                    || app.audio.text_edge_drag.is_some()
                {
                    ensure!(
                        self.stage_started.elapsed() < Duration::from_secs(2),
                        "dragging the B text marker did not update the boundary"
                    );
                    return Ok(false);
                }
                ensure!(
                    app.media
                        .session
                        .loop_b
                        .is_some_and(|b| (b - expected).abs() < 0.02)
                        && app.media.session.mode == PlaybackMode::Ab
                        && !app.has_subtitle_selection(),
                    "B text marker updated the wrong boundary or left drag state: b={:?} expected={expected} mode={:?} selected={} drag={:?}",
                    app.media.session.loop_b,
                    app.media.session.mode,
                    app.has_subtitle_selection(),
                    app.audio.text_edge_drag
                );
                self.observations.push(json!({
                    "event": "B text marker drag", "a": app.media.session.loop_a,
                    "b": app.media.session.loop_b, "expected_b": expected,
                    "clip_mode": app.audio.clip_mode,
                }));
                self.cases.push(json!({"case":"audio subtitle A/B vertical markers drag both boundaries without selecting text","passed":true}));
                ensure!(
                    app.audio_saved.len() >= 2,
                    "review fixture clips missing: {:?}",
                    app.audio_saved
                );
                app.handle(Action::AudioReview, window, cx);
                ensure!(
                    app.audio
                        .review
                        .as_ref()
                        .is_some_and(|review| review.index == 0),
                    "audio review did not start at the first saved clip"
                );
            }
            6 => {
                if let Some(review) = app.audio.review.as_ref() {
                    if review.index == 1 {
                        self.saw_second_review_clip = true;
                    }
                    ensure!(
                        self.stage_started.elapsed() < Duration::from_secs(3),
                        "audio review did not reach the second clip: index={} saved={}",
                        review.index,
                        app.audio_saved.len()
                    );
                    return Ok(false);
                }
                ensure!(
                    self.saw_second_review_clip,
                    "audio review finished without playing the second clip"
                );
                self.observations.push(json!({
                    "event": "audio saved-clip review",
                    "saved": app.audio_saved.len(),
                    "saw_second": self.saw_second_review_clip,
                    "paused": app.media.snapshot.paused,
                }));
                self.cases.push(json!({"case":"audio learning plays saved A/B clips in order and returns to the full track","passed":true}));
                self.report(true, None)?;
                return Ok(true);
            }
            _ => unreachable!(),
        }
        self.stage += 1;
        self.stage_started = Instant::now();
        Ok(false)
    }
}

impl diagnostics::Probe for Probe {
    fn tick(
        &mut self,
        app: &mut AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Result<bool> {
        self.step(app, window, cx)
    }
    fn failed(&mut self, error: String) -> Result<()> {
        self.report(false, Some(error))
    }
}
