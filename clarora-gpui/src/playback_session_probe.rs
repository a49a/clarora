//! Isolated AppView handoff acceptance for the shared native player.
//! PS-01..PS-14 failure scenarios were recorded before this probe was written.
//! Uses real actions, SQLite, WAV/SRT and libmpv; no physical input or sound device.
use super::*;

const AUDIO: &str = "session-audio";
const GROUP: &str = "session-group";
const CLIP_A: &str = "session-clip-a";
const CLIP_B: &str = "session-clip-b";
const EMPTY: &str = "session-empty";
const AUDIO_POSITION: f64 = 0.62;
const VIDEO_POSITION: f64 = 3.18;
// Existing video triple-repeat restoration bookmarks the cue start and restarts
// play one. An explicit in-session seek still uses VIDEO_POSITION above.
const VIDEO_SENTENCE_START: f64 = 3.;

fn source(lib: &Library) -> PathBuf {
    lib.root.join("media/shared-session.wav")
}

pub(super) fn prepare(out: &Path) -> Result<Library> {
    fs::create_dir_all(out)?;
    let lib = Library::open(out.join("library"))?;
    let path = source(&lib);
    let mut writer = hound::WavWriter::create(
        &path,
        hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )?;
    for sample in 0..128_000 {
        writer.write_sample(((sample as f32 / 12.).sin() * 800.) as i16)?;
    }
    writer.finalize()?;
    fs::copy(&path, lib.root.join("media/late-part.wav"))?;
    let srt = lib.root.join("media/shared-session.srt");
    fs::write(
        &srt,
        "1\n00:00:00,400 --> 00:00:00,900\nAudio owns this sentence.\n\n2\n00:00:03,000 --> 00:00:03,400\nVideo owns this sentence.\n\n3\n00:00:06,000 --> 00:00:06,400\nFinish exactly three times.\n",
    )?;
    let cues = subtitles::parse(&fs::read_to_string(&srt)?);
    video_subtitles::save(&lib, &path, &cues, &process::cancel_token())?;
    lib.put(
        "listening_practices",
        json!({"id":GROUP,"name":"Shared session fixture"}),
    )?;
    lib.put("listening_audios", json!({"id":AUDIO,"practice_id":GROUP,"name":"Shared session source","audio_uri":path,"subtitle_uri":srt}))?;
    for (id, start, end) in [(CLIP_A, 1800., 2200.), (CLIP_B, 1200., 1600.)] {
        lib.put("clip_cards", json!({"id":id,"en_text":id,"zh_text":"Isolated saved clip","audio_uri":path,"start_ms":start,"end_ms":end}))?;
    }
    lib.put("sentence_cards", json!({"id":EMPTY,"text":"No media belongs to this card.","translation":"No stale playback is allowed.","notes":""}))?;
    lib.set_setting("library_group", GROUP)?;
    lib.set_setting("last_listening_audio", AUDIO)?;
    lib.set_setting("audio_repeat_mode", "6")?;
    lib.set_setting("last_page", "listening")?;
    lib.set_setting("assistant_enabled", "false")?;
    // Network access is never needed for the fixture or any stage below.
    lib.set_setting(
        "ai_config",
        &json!({"engine":"compatible","base_url":"http://127.0.0.1:9/v1","model":"session-probe"})
            .to_string(),
    )?;
    fs::write(
        out.join("playback-session-native-fixture.json"),
        serde_json::to_vec_pretty(&json!({
            "library":lib.root,"shared_source":path,"subtitle":srt,"audio_id":AUDIO,
            "clip_ids":[CLIP_A,CLIP_B],"empty_card":EMPTY,
            "audio_position":AUDIO_POSITION,"video_position":VIDEO_POSITION,
            "video_resume_position":VIDEO_SENTENCE_START,
            "video_resume_policy":"triple repeat saves the sentence start and rebuilds from play one; an explicit in-session seek retains its requested position",
            "replay":["clarora-gpui","--playback-session-check","<new-empty-evidence-directory>"]
        }))?,
    )?;
    Ok(lib)
}

pub(super) struct Probe {
    out: PathBuf,
    stage: usize,
    started: Instant,
    entered: Instant,
    cases: Vec<Value>,
    observations: Vec<Value>,
    old_snapshot: Option<Playback>,
    old_revision: u64,
    video_request: u64,
    listening_bookmark: String,
    video_bookmark: String,
}

impl Probe {
    pub(super) fn new(out: &Path) -> Self {
        Self {
            out: out.into(),
            stage: 0,
            started: Instant::now(),
            entered: Instant::now(),
            cases: vec![],
            observations: vec![],
            old_snapshot: None,
            old_revision: 0,
            video_request: 0,
            listening_bookmark: String::new(),
            video_bookmark: String::new(),
        }
    }
    fn report(&self, passed: Option<bool>, error: Option<String>) -> Result<()> {
        fs::write(
            self.out.join("playback-session-native-report.json"),
            serde_json::to_vec_pretty(&json!({
                "passed":passed,"stage":self.stage,"error":error,"cases":self.cases,
                "observations":self.observations,"elapsed_ms":self.started.elapsed().as_millis(),
                "actual_appview_handlers":true,"actual_libmpv":true,"isolated_sqlite":true,
                "audio_device":"null","hidden_native_window":true,"physical_input_verified":false,
            "independent_review":false,"shared_video_fixture_is_audio_only":true,
            "native_pending_same_path_loads_verified":false,
            "generation_check":"reject an archived snapshot from the completed previous load; multiple concurrent same-path loads are not covered",
            "video_manual_seek_position":VIDEO_POSITION,
            "video_resume_position":VIDEO_SENTENCE_START,
            "video_resume_policy":"existing triple-repeat behavior saves the sentence start and restores play one, while audio preserves its own within-sentence bookmark"
            }))?,
        )?;
        Ok(())
    }
    fn observe(&mut self, app: &AppView) {
        let state = app.media.player.snapshot();
        let observation = json!({"stage":self.stage,"page":app.page,
            "owner":format!("{:?}",app.media.session.owner()),"revision":app.media.session.revision(),
            "session_path":app.media.session.path,"mode":app.media.session.mode.index(),
            "audio_preference":app.audio.repeat_mode.index(),"video_preference":app.video.sentence_repeat.is_some(),
            "loop_a":app.media.session.loop_a,"loop_b":app.media.session.loop_b,
            "active_sentence":app.media.session.sentence().map(|s|json!({"index":s.index,"request":s.request_id,"range":s.range,"three":s.total.is_some()})),
            "native":{"path":state.path,"position":state.position,"duration":state.duration,
                "paused":state.paused,"eof":state.eof,"error":state.error,"load_revision":state.load_revision,
                "repeat":state.sentence_repeat.as_ref().map(|s|json!({"request":s.request_id,"current":s.current_play,"total":s.total_plays,"finished":s.finished}))}});
        if self.observations.last() != Some(&observation) && self.observations.len() < 1200 {
            self.observations.push(observation);
        }
    }
    fn record(&mut self, name: &str, app: &AppView) -> Result<()> {
        self.cases.push(json!({"case":name,"passed":true,"stage":self.stage,
            "owner":format!("{:?}",app.media.session.owner()),"revision":app.media.session.revision()}));
        self.observe(app);
        self.report(None, None)
    }
    fn advance(&mut self) {
        self.stage += 1;
        self.entered = Instant::now();
    }
    fn loaded(app: &AppView, path: &Path) -> bool {
        let state = app.media.player.snapshot();
        state.path == path.to_string_lossy()
            && state.duration >= 7.9
            && state.error.is_empty()
            && app.media.matches_snapshot(&state)
    }
    fn paused_at(app: &AppView, position: f64) -> bool {
        let state = app.media.player.snapshot();
        state.paused && (state.position - position).abs() <= 0.08
    }
    fn no_range(app: &AppView) -> bool {
        app.media.session.loop_a.is_none() && app.media.session.loop_b.is_none()
    }
    fn card_owner(app: &AppView, id: &str) -> bool {
        matches!(app.media.session.owner(), Some(PlaybackOwner::Card { kind, id: current })
            if kind == "clip" && current == id)
    }
    fn bookmark(lib: &Library, setting: &str, position: f64) -> Result<bool> {
        let value: Value = serde_json::from_str(&lib.setting(setting)?)?;
        Ok(value["position"]
            .as_f64()
            .is_some_and(|at| (at - position).abs() <= 0.08))
    }
    fn check(
        &mut self,
        app: &mut AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Result<bool> {
        ensure!(
            self.started.elapsed() < Duration::from_secs(70),
            "shared session diagnostic overall timeout"
        );
        ensure!(
            self.entered.elapsed() < Duration::from_secs(10),
            "shared session stage {} timed out; owner={:?}, status={}",
            self.stage,
            app.media.session.owner(),
            app.status
        );
        app.poll_data();
        app.media.poll();
        app.poll_tasks(window, cx);
        app.poll_pending_video_start();
        app.poll_video_sentence_repeat();
        app.poll_audio_sentence_repeat();
        self.observe(app);
        let path = source(&app.lib);
        match self.stage {
            0 => {
                ensure!(
                    app.lib.root == self.out.join("library").canonicalize()?,
                    "probe opened a nonfixture library"
                );
                app.media.player = Player::without_audio_device();
                app.media.preloader = Player::without_audio_device();
                app.media.release();
                app.handle(Action::Page("listening".into()), window, cx);
            }
            1 => {
                if !app.audio.audios.iter().any(|row| text(row, "id") == AUDIO) {
                    return Ok(false);
                }
                app.handle(Action::Audio(AUDIO.into()), window, cx);
            }
            2 => {
                if !Self::loaded(app, &path)
                    || app.loads.restore_pending
                    || app.audio.cues.len() != 3
                {
                    return Ok(false);
                }
                ensure!(
                    app.audio.repeat_mode == PlaybackMode::SentenceThree,
                    "legacy SQLite mode 6 was not read as triple preference"
                );
                self.record("legacy SQLite audio preference loads through AppView", app)?;
                app.handle(Action::AudioRepeat(6), window, cx);
            }
            3 => {
                let state = app.media.player.snapshot();
                let Some(sentence) = app.media.session.audio_sentence() else {
                    return Ok(false);
                };
                if !state.sentence_repeat.as_ref().is_some_and(|r| {
                    r.request_id == sentence.request_id && r.total_plays == Some(3)
                }) {
                    return Ok(false);
                }
                ensure!(
                    app.media.session.is_audio(AUDIO)
                        && sentence.index == 0
                        && app.media.session.video_sentence().is_none(),
                    "audio request has incorrect owner"
                );
                self.record("audio triple sentence belongs only to audio owner", app)?;
                app.media.player.set("pause", "yes");
            }
            4 => {
                if !app.media.player.snapshot().paused {
                    return Ok(false);
                }
                app.handle(Action::SeekTo(AUDIO_POSITION), window, cx);
            }
            5 => {
                if !Self::paused_at(app, AUDIO_POSITION) {
                    return Ok(false);
                }
                self.record("audio paused sentence seek preserves its own position", app)?;
                app.handle(Action::Page("video".into()), window, cx);
                app.picked("video", vec![path.clone()]);
            }
            6 => {
                if !Self::loaded(app, &path)
                    || !app.media.session.is_video()
                    || app.video.cues.len() != 3
                {
                    return Ok(false);
                }
                ensure!(
                    app.media.session.audio_sentence().is_none()
                        && app.media.session.sentence().is_none()
                        && Self::no_range(app)
                        && app.media.session.mode == PlaybackMode::Normal,
                    "audio policy leaked into same-source video"
                );
                ensure!(
                    app.audio.repeat_mode == PlaybackMode::SentenceThree
                        && app.video.sentence_repeat.is_none(),
                    "opening video changed an independent preference"
                );
                ensure!(
                    Self::bookmark(&app.lib, "last_listening_position", AUDIO_POSITION)?,
                    "audio bookmark was not saved before handoff"
                );
                self.record(
                    "same-source audio to video releases old request and saves audio bookmark",
                    app,
                )?;
                app.handle(Action::VideoSentenceRepeat(Some(1), Some(3)), window, cx);
            }
            7 => {
                let state = app.media.player.snapshot();
                let Some(sentence) = app.media.session.video_sentence() else {
                    return Ok(false);
                };
                if !state.sentence_repeat.as_ref().is_some_and(|r| {
                    r.request_id == sentence.request_id && r.total_plays == Some(3)
                }) {
                    return Ok(false);
                }
                ensure!(
                    sentence.index == 1 && app.video.sentence_repeat.is_some(),
                    "video triple did not select its own cue"
                );
                self.video_request = sentence.request_id;
                ensure!(
                    !app.seek_audio_repeat(AUDIO_POSITION, false)?,
                    "inactive audio accepted a seek"
                );
                app.reset_audio_sentence_request();
                ensure!(
                    app.media
                        .session
                        .video_sentence()
                        .is_some_and(|s| s.request_id == self.video_request),
                    "inactive audio reset removed video request"
                );
                app.media.player.set("pause", "yes");
            }
            8 => {
                let state = app.media.player.snapshot();
                if !state.paused
                    || !state
                        .sentence_repeat
                        .as_ref()
                        .is_some_and(|r| r.request_id == self.video_request)
                {
                    return Ok(false);
                }
                self.record(
                    "inactive audio seek and reset leave native video request intact",
                    app,
                )?;
                // Use the actual video timeline action, which preserves sentence mode.
                app.handle(Action::VideoTimelineSeek(VIDEO_POSITION), window, cx);
            }
            9 => {
                if !Self::paused_at(app, VIDEO_POSITION) {
                    return Ok(false);
                }
                ensure!(
                    app.audio.repeat_mode == PlaybackMode::SentenceThree
                        && app.video.sentence_repeat.is_some(),
                    "audio and video preferences are not independent"
                );
                self.record(
                    "video triple preference and paused position remain independent",
                    app,
                )?;
                app.handle(Action::Page("flashcard".into()), window, cx);
                app.handle(Action::ReviewMode(false), window, cx);
                app.handle(Action::Filter("clip".into()), window, cx);
            }
            10 => {
                if !app.cards_ready() {
                    return Ok(false);
                }
                let Some(index) = app.cards.iter().position(|card| card.id == CLIP_A) else {
                    return Ok(false);
                };
                app.index = index;
                app.handle(Action::ReplayCard, window, cx);
            }
            11 => {
                if !Self::loaded(app, &path) || !Self::card_owner(app, CLIP_A) {
                    return Ok(false);
                }
                ensure!(
                    app.media.session.sentence().is_none()
                        && Self::no_range(app)
                        && app.media.session.mode == PlaybackMode::Normal
                        && app.video_frame_path().is_none(),
                    "same-source card inherited video/audio policy"
                );
                ensure!(
                    app.audio.repeat_mode == PlaybackMode::SentenceThree
                        && app.video.sentence_repeat.is_some(),
                    "card changed page preferences"
                );
                self.record("same-source card claims its own owner and saved range", app)?;
                let next = app
                    .cards
                    .iter()
                    .position(|card| card.id == CLIP_B)
                    .context("missing preloaded card")?;
                ensure!(
                    next == (app.index + 1) % app.cards.len(),
                    "fixture card ordering changed"
                );
                app.media.preload_card(next, &path, 1.2, 1.6);
            }
            12 => {
                let state = app.media.preloader.snapshot();
                if state.path != path.to_string_lossy()
                    || state.duration < 7.9
                    || !state.paused
                    || state.position < 1.1
                {
                    return Ok(false);
                }
                let next = app
                    .cards
                    .iter()
                    .position(|card| card.id == CLIP_B)
                    .context("missing preloaded card")?;
                app.media.preloaded_index = Some(next);
                self.old_revision = app.media.session.revision();
                app.handle(Action::Next, window, cx);
            }
            13 => {
                if !Self::loaded(app, &path) || !Self::card_owner(app, CLIP_B) {
                    return Ok(false);
                }
                ensure!(
                    app.media.session.revision() != self.old_revision
                        && app.media.session.sentence().is_none()
                        && Self::no_range(app)
                        && app.media.session.mode == PlaybackMode::Normal,
                    "preloaded player swap retained old session"
                );
                self.record(
                    "preloaded card swap claims a new session and clears old policy",
                    app,
                )?;
            }
            14 => {
                let state = app.media.player.snapshot();
                if !state.eof || !state.paused {
                    return Ok(false);
                }
                ensure!(
                    state.position <= 1.7,
                    "preloaded clip overran its own endpoint"
                );
                self.record(
                    "preloaded card native playback stops at its own clip endpoint",
                    app,
                )?;
                app.handle(Action::Filter("sentence".into()), window, cx);
            }
            15 => {
                if !app.cards_ready() {
                    return Ok(false);
                }
                let Some(index) = app.cards.iter().position(|card| card.id == EMPTY) else {
                    return Ok(false);
                };
                app.index = index;
                app.play_card();
            }
            16 => {
                if !app.media.player.snapshot().paused {
                    return Ok(false);
                }
                ensure!(
                    app.media.session.owner().is_none()
                        && app.media.session.path.is_none()
                        && app.media.session.sentence().is_none()
                        && Self::no_range(app)
                        && app.media.snapshot.path.is_empty(),
                    "no-media card retained old shared playback"
                );
                self.record(
                    "no-media card releases shared owner native request and cached snapshot",
                    app,
                )?;
                app.handle(Action::Page("video".into()), window, cx);
            }
            17 => {
                if !Self::loaded(app, &path)
                    || !Self::paused_at(app, VIDEO_SENTENCE_START)
                    || app.video.pending_start.is_some()
                {
                    return Ok(false);
                }
                let Some(sentence) = app.media.session.video_sentence() else {
                    return Ok(false);
                };
                ensure!(
                    sentence.index == 1 && sentence.total.is_some() && Self::no_range(app),
                    "video did not restore its own triple session"
                );
                ensure!(
                    app.media.player.snapshot().position > 2.2,
                    "video remained bounded by an old card endpoint"
                );
                self.record(
                    "returning video restores its bookmark beyond old card endpoint",
                    app,
                )?;
                self.old_revision = app.media.session.revision();
                self.old_snapshot = Some(app.media.player.snapshot());
                ensure!(
                    app.open_at(path.clone(), VIDEO_POSITION, true),
                    "same-source video reopen failed"
                );
            }
            18 => {
                if !Self::loaded(app, &path)
                    || !Self::paused_at(app, VIDEO_SENTENCE_START)
                    || app.video.pending_start.is_some()
                {
                    return Ok(false);
                }
                let Some(sentence) = app.media.session.video_sentence() else {
                    return Ok(false);
                };
                let old = self
                    .old_snapshot
                    .as_ref()
                    .context("missing old generation snapshot")?;
                ensure!(
                    app.media.session.revision() != self.old_revision
                        && !app.media.matches_snapshot(old)
                        && old
                            .sentence_repeat
                            .as_ref()
                            .is_some_and(|r| r.request_id != sentence.request_id),
                    "same-source reopen accepted an old native generation"
                );
                self.record(
                    "same-owner same-source reopen rejects archived native snapshot",
                    app,
                )?;
                let late = app.lib.root.join("media/late-part.wav");
                let series = bilibili::Series {
                    title: "Late local result fixture".into(),
                    base: "https://www.bilibili.com/video/BV1xx411c7mD".into(),
                    requested: 0,
                    current: Some(0),
                    parts: vec![bilibili::Part {
                        number: 1,
                        title: "Late part".into(),
                        duration: 8.,
                        path: Some(late.clone()),
                    }],
                };
                let revision = app.media.session.revision();
                let request = sentence.request_id;
                let task = app.tasks.foreground.begin(TaskScope::Material)?;
                task.complete(Ok(Outcome::PartVideo(
                    series,
                    0,
                    late,
                    false,
                    self.old_revision,
                    false,
                )));
                app.poll_tasks(window, cx);
                ensure!(
                    app.media.session.revision() == revision
                        && app.media.session.path.as_ref() == Some(&path)
                        && app
                            .media
                            .session
                            .video_sentence()
                            .is_some_and(|s| s.request_id == request),
                    "late PartVideo result hijacked a new playback generation"
                );
                self.record(
                    "late PartVideo result caches without replacing a newer same-source session",
                    app,
                )?;
                app.handle(Action::Page("listening".into()), window, cx);
            }
            19 => {
                if !Self::loaded(app, &path)
                    || !Self::paused_at(app, AUDIO_POSITION)
                    || app.loads.restore_pending
                {
                    return Ok(false);
                }
                let Some(sentence) = app.media.session.audio_sentence() else {
                    return Ok(false);
                };
                let state = app.media.player.snapshot();
                if !state
                    .sentence_repeat
                    .as_ref()
                    .is_some_and(|r| r.request_id == sentence.request_id && r.current_play == 1)
                {
                    return Ok(false);
                }
                ensure!(
                    app.media.session.is_audio(AUDIO)
                        && sentence.index == 0
                        && sentence.total.is_some()
                        && app.video.sentence_repeat.is_some(),
                    "audio restored another page's position or preference"
                );
                let reopened = Library::open(app.lib.root.clone())?;
                ensure!(
                    app_state::AudioState::new(&reopened).repeat_mode
                        == PlaybackMode::SentenceThree
                        && Self::bookmark(&reopened, "last_video_position", VIDEO_SENTENCE_START)?,
                    "page preferences or bookmarks did not survive SQLite reopen"
                );
                self.record(
                    "returning audio restores its own paused position mode and first play",
                    app,
                )?;
                self.record(
                    "real SQLite reopen preserves independent page preferences and bookmarks",
                    app,
                )?;
                app.handle(Action::AudioSentence(1), window, cx);
                app.handle(Action::AudioSentence(1), window, cx);
            }
            20 => {
                let state = app.media.player.snapshot();
                let Some(sentence) = app.media.session.audio_sentence() else {
                    return Ok(false);
                };
                if !state.paused
                    || !state
                        .sentence_repeat
                        .as_ref()
                        .is_some_and(|r| r.request_id == sentence.request_id && r.finished)
                {
                    return Ok(false);
                }
                ensure!(
                    sentence.index == 2
                        && state
                            .sentence_repeat
                            .as_ref()
                            .is_some_and(|r| r.total_plays == Some(3) && r.current_play == 3),
                    "last audio cue did not finish exactly three times"
                );
                self.record(
                    "shared native audio request finishes final cue exactly three times",
                    app,
                )?;
                app.handle(Action::Page("meditation".into()), window, cx);
                self.listening_bookmark = app.lib.setting("last_listening_position")?;
                self.video_bookmark = app.lib.setting("last_video_position")?;
                app.picked("background", vec![path.clone()]);
            }
            21 => {
                if !Self::loaded(app, &path) {
                    return Ok(false);
                }
                ensure!(
                    app.media.session.owner() == Some(&PlaybackOwner::Background)
                        && app.media.session.sentence().is_none()
                        && Self::no_range(app)
                        && app.media.session.mode == PlaybackMode::Track
                        && app.audio.repeat_mode == PlaybackMode::SentenceThree
                        && app.video.sentence_repeat.is_some(),
                    "background inherited a learning request or changed preferences"
                );
                self.record(
                    "background import owns a separate session and preserves learning preferences",
                    app,
                )?;
            }
            22 => {
                if self.entered.elapsed() < Duration::from_millis(250) {
                    return Ok(false);
                }
                ensure!(
                    app.lib.setting("last_listening_position")? == self.listening_bookmark
                        && app.lib.setting("last_video_position")? == self.video_bookmark,
                    "background playback overwrote a learning bookmark"
                );
                self.record(
                    "background playback does not overwrite audio or video bookmarks",
                    app,
                )?;
                app.handle(Action::Page("settings".into()), window, cx);
            }
            23 => {
                if !app.media.player.snapshot().paused || !app.media.preloader.snapshot().paused {
                    return Ok(false);
                }
                ensure!(
                    app.media.session.owner().is_none()
                        && app.media.session.path.is_none()
                        && app.media.session.sentence().is_none()
                        && Self::no_range(app)
                        && app.media.session.mode == PlaybackMode::Normal
                        && app.media.preloaded_index.is_none(),
                    "leaving background retained shared playback policy"
                );
                self.record(
                    "leaving background releases owner loop preloader and native playback",
                    app,
                )?;
                app.handle(Action::Page("video".into()), window, cx);
            }
            24 => {
                if !Self::loaded(app, &path)
                    || !Self::paused_at(app, VIDEO_SENTENCE_START)
                    || app.video.pending_start.is_some()
                {
                    return Ok(false);
                }
                app.handle(Action::VideoStopSentenceRepeat, window, cx);
                app.handle(Action::LoopMode(1), window, cx);
            }
            25 => {
                ensure!(
                    app.media.session.is_video()
                        && app.media.session.mode == PlaybackMode::Track
                        && app.video.repeat_mode == PlaybackMode::Track
                        && app.video.sentence_repeat.is_none()
                        && app.media.session.sentence().is_none(),
                    "video track-loop action did not replace the sentence policy"
                );
                self.record(
                    "video track-loop preference selects typed policy through Action",
                    app,
                )?;
                app.handle(Action::Page("settings".into()), window, cx);
            }
            26 => {
                ensure!(
                    app.media.session.owner().is_none()
                        && app.media.session.mode == PlaybackMode::Normal
                        && app.video.repeat_mode == PlaybackMode::Track
                        && app.audio.repeat_mode == PlaybackMode::SentenceThree,
                    "leaving video dropped track preference or retained active policy"
                );
                self.record(
                    "leaving video preserves track-loop preference while releasing active policy",
                    app,
                )?;
                app.handle(Action::Page("video".into()), window, cx);
            }
            27 => {
                if !Self::loaded(app, &path) || !Self::paused_at(app, VIDEO_SENTENCE_START) {
                    return Ok(false);
                }
                ensure!(
                    app.media.session.is_video()
                        && app.media.session.mode == PlaybackMode::Track
                        && app.video.repeat_mode == PlaybackMode::Track
                        && app.media.session.sentence().is_none()
                        && app.media.player.snapshot().sentence_repeat.is_none(),
                    "returning video did not restore its track-loop preference"
                );
                self.record("returning video restores track-loop preference without an old sentence request", app)?;
                self.report(Some(true), None)?;
                return Ok(true);
            }
            _ => anyhow::bail!("unexpected shared-session stage {}", self.stage),
        }
        self.advance();
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
        self.check(app, window, cx)
    }
    fn failed(&mut self, error: String) -> Result<()> {
        self.report(Some(false), Some(error))
    }
}
