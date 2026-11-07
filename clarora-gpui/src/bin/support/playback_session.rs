//! Real SQLite/WAV/libmpv integration for the shared session ownership contract.
//! Page handlers remain covered by the existing native audio/video/card probes.
use anyhow::{Context, Result, ensure};
use clarora_gpui::{
    data::{self, Library, text},
    media::{Playback, Player},
    playback_session::{Mode, Owner, Session},
    subtitles::Cue,
    video_repeat::{Session as Sentence, Transition},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

fn snapshot(state: &Playback) -> Value {
    json!({"path":state.path,"load_revision":state.load_revision,
        "position":state.position,"duration":state.duration,"paused":state.paused,
        "eof":state.eof,"error":state.error,
        "sentence":state.sentence_repeat.as_ref().map(|s| json!({
            "request_id":s.request_id,"start":s.start,"end":s.end,
            "total_plays":s.total_plays,"current_play":s.current_play,"finished":s.finished}))})
}

fn wait(
    player: &Player,
    stage: &str,
    traces: &mut Vec<Value>,
    predicate: impl Fn(&Playback) -> bool,
) -> Result<Playback> {
    let started = Instant::now();
    let mut previous = Value::Null;
    loop {
        let state = player.snapshot();
        let entry = snapshot(&state);
        if entry != previous {
            previous = entry.clone();
            traces.push(
                json!({"stage":stage,"elapsed_ms":started.elapsed().as_millis(),"playback":entry}),
            );
        }
        ensure!(state.error.is_empty(), "{stage}: {}", state.error);
        if predicate(&state) {
            return Ok(state);
        }
        ensure!(
            started.elapsed() < Duration::from_secs(8),
            "{stage}: timeout: {entry}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn loaded(
    player: &Player,
    path: &Path,
    revision: u64,
    stage: &str,
    traces: &mut Vec<Value>,
) -> Result<Playback> {
    wait(player, stage, traces, |s| {
        s.path == path.to_string_lossy() && s.load_revision > revision && s.duration > 2.9
    })
}

fn record(checks: &mut Vec<Value>, case: &str, session: &Session, state: &Playback) {
    checks.push(json!({"case":case,"passed":true,"owner":format!("{:?}",session.owner()),
        "material":session.path,"mode":session.mode.index(),
        "range":[session.loop_a,session.loop_b],"end_handled":session.end_handled,
        "sentence":session.sentence().map(|s| json!({"index":s.index,"request_id":s.request_id,"three":s.three})),
        "playback":snapshot(state)}));
}

fn null_audio_clocks(root: &Path, traces: &mut Vec<Value>) -> Result<Vec<Value>> {
    let mut rates = vec![];
    for sample_rate in [8_000, 16_000, 44_100, 48_000] {
        let path = root.join(format!("null-clock-{sample_rate}.wav"));
        let mut wav = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 1,
                sample_rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )?;
        for sample in 0..sample_rate * 2 {
            wav.write_sample(
                ((sample as f64 * std::f64::consts::TAU * 220. / sample_rate as f64).sin() * 2400.)
                    as i16,
            )?;
        }
        wav.finalize()?;
        let player = Player::without_audio_device();
        wait(
            &player,
            &format!("null clock {sample_rate} initialization"),
            traces,
            |s| s.initialized,
        )?;
        player.open_range(&path, 0., 0., true);
        let ready = wait(
            &player,
            &format!("null clock {sample_rate} paused WAV ready"),
            traces,
            |s| s.path == path.to_string_lossy() && s.duration > 1.9 && s.paused,
        )?;
        player.command(["seek", "0.62", "absolute+exact"]);
        let sought = wait(
            &player,
            &format!("null clock {sample_rate} paused exact seek to 0.62"),
            traces,
            |s| s.path == path.to_string_lossy() && s.paused && (s.position - 0.62).abs() <= 0.03,
        )?;
        player.set("pause", "no");
        let resumed = wait(
            &player,
            &format!("null clock {sample_rate} resumes beyond 0.72"),
            traces,
            |s| s.path == path.to_string_lossy() && !s.paused && !s.eof && s.position > 0.62 + 0.1,
        )?;
        rates.push(json!({"sample_rate":sample_rate,"fixture":path,
            "wav_sha256":hex::encode(Sha256::digest(fs::read(&path)?)),
            "target":0.62,"maximum_seek_error":0.03,"ready":snapshot(&ready),
            "paused_seek":snapshot(&sought),"resumed":snapshot(&resumed)}));
    }
    Ok(rates)
}

fn sequence(root: &Path, traces: &mut Vec<Value>, checks: &mut Vec<Value>) -> Result<()> {
    let rates = null_audio_clocks(root, traces)?;
    checks.push(
        json!({"case":"null audio clocks seek and resume across sample rates",
        "passed":true,"sample_rates":rates}),
    );
    let lib = Library::open(root.join("library"))?;
    let source = root.join("shared-source.wav");
    let mut wav = hound::WavWriter::create(
        &source,
        hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )?;
    for sample in 0..48_000 {
        wav.write_sample(
            ((sample as f64 * std::f64::consts::TAU * 220. / 16_000.).sin() * 2400.) as i16,
        )?;
    }
    wav.finalize()?;
    let group = lib.practice("Shared playback session")?;
    lib.import_audio(&group, &source)?;
    let rows = lib.list("listening_audios")?;
    let audio = rows.first().context("imported audio missing")?;
    let audio_id = text(audio, "id");
    let path = std::path::PathBuf::from(text(audio, "audio_uri"));
    lib.put(
        "clip_cards",
        json!({"id":"shared-session-card","en_text":"Shared source clip",
        "zh_text":"同源片段","audio_uri":path,"start_ms":1200,"end_ms":1650}),
    )?;
    let card = lib
        .cards(false)?
        .into_iter()
        .find(|c| c.id == "shared-session-card")
        .context("same-source clip card missing")?;
    ensure!(
        Path::new(&card.media) == path && card.start == 1.2 && card.end == 1.65,
        "SQLite card did not retain the shared path and clip range"
    );

    for (index, expected) in [
        Mode::Normal,
        Mode::Track,
        Mode::Ab,
        Mode::Group,
        Mode::Continuous,
        Mode::Sentence,
        Mode::SentenceThree,
    ]
    .into_iter()
    .enumerate()
    {
        ensure!(
            Mode::from_index(index) == expected && expected.index() == index,
            "legacy repeat index {index} changed"
        );
        ensure!(
            expected.is_sentence() == (index >= 5),
            "sentence mode classification changed"
        );
    }
    ensure!(
        Mode::from_index(usize::MAX) == Mode::Normal,
        "unknown legacy mode has no stable fallback"
    );
    lib.set_setting(
        "audio_repeat_mode",
        &Mode::SentenceThree.index().to_string(),
    )?;
    lib.set_setting(
        "last_listening_position",
        &json!({"id":audio_id,"position":2.1}).to_string(),
    )?;
    lib.set_setting(
        "last_video_position",
        &json!({"path":path,"position":0.7}).to_string(),
    )?;
    let original_audio_preference = lib.setting("audio_repeat_mode")?;
    let original_audio_position = lib.setting("last_listening_position")?;
    let original_video_position = lib.setting("last_video_position")?;
    checks.push(
        json!({"case":"legacy mode indices and actual SQLite audio/card fixture","passed":true,
        "modes":7,"unknown_mode_fallback":0,"audio_id":audio_id,"card_id":card.id,
        "wav_sha256":hex::encode(Sha256::digest(fs::read(&path)?))}),
    );

    let player = Player::without_audio_device();
    wait(&player, "initialize libmpv", traces, |s| s.initialized)?;
    let mut session = Session::default();
    let audio_lease = session.claim(
        Owner::Audio(audio_id.clone()),
        path.clone(),
        Mode::SentenceThree,
    );
    let revision = player.snapshot().load_revision;
    player.open_range(&path, 0., 0., true);
    let state = loaded(&player, &path, revision, "audio claim", traces)?;
    ensure!(
        session.is_current(audio_lease),
        "new audio lease was rejected"
    );
    let cues = vec![
        Cue {
            start: 0.2,
            end: 0.42,
            text: "First sentence".into(),
        },
        Cue {
            start: 0.7,
            end: 0.94,
            text: "Second sentence".into(),
        },
    ];
    let audio_sentence = Sentence::start(&player, &path, &cues, 0, true, state.duration)?;
    ensure!(
        session.set_audio_sentence(Some(audio_sentence)),
        "audio owner cannot set audio sentence"
    );
    ensure!(
        !session.set_video_sentence(None),
        "video setter can clear an audio owner"
    );
    ensure!(
        session.audio_sentence().is_some() && session.video_sentence().is_none(),
        "one active sentence was exposed as both audio and video"
    );
    wait(&player, "audio actual sentence request", traces, |s| {
        s.sentence_repeat
            .as_ref()
            .is_some_and(|r| r.request_id == audio_sentence.request_id)
    })?;
    session.loop_a = Some(0.2);
    session.loop_b = Some(0.42);
    session.end_handled = true;
    record(
        checks,
        "audio owns the sole active sentence",
        &session,
        &player.snapshot(),
    );

    // The same physical path is deliberately claimed by a different page.
    let video_lease = session.claim(Owner::Video, path.clone(), Mode::SentenceThree);
    ensure!(
        !session.is_current(audio_lease) && session.is_current(video_lease),
        "owner handoff reused the audio lease"
    );
    ensure!(
        session.sentence().is_none()
            && session.loop_a.is_none()
            && session.loop_b.is_none()
            && !session.end_handled,
        "claim inherited the former sentence/range/EOF state"
    );
    ensure!(
        !session.set_sentence(audio_lease, Some(audio_sentence))
            && !session.clear_sentence(audio_lease),
        "stale audio lease can mutate a same-path video claim"
    );
    let revision = player.snapshot().load_revision;
    player.open_range(&path, 0., 0., true);
    let state = loaded(&player, &path, revision, "same-path video claim", traces)?;
    ensure!(
        state.sentence_repeat.is_none(),
        "open_range inherited the former native audio sentence"
    );
    ensure!(
        !session.set_audio_sentence(None),
        "audio setter can mutate a video owner"
    );
    let first = Sentence::start(&player, &path, &cues, 0, true, state.duration)?;
    ensure!(
        session.set_sentence(video_lease, Some(first)),
        "current video lease rejected sentence"
    );
    record(
        checks,
        "same-path audio to video expires former owner and range",
        &session,
        &player.snapshot(),
    );

    let first_finished = wait(&player, "video first three plays", traces, |s| {
        s.sentence_repeat
            .as_ref()
            .is_some_and(|r| r.request_id == first.request_id && r.finished)
    })?;
    let repeat = first_finished.sentence_repeat.as_ref().unwrap();
    ensure!(
        repeat.current_play == 3
            && repeat.total_plays == Some(3)
            && first_finished.paused
            && !first_finished.eof
            && (first_finished.position - 0.42).abs() < 0.09,
        "first sentence did not finish exactly three plays at its end"
    );
    let mut active = session
        .video_sentence()
        .context("video session disappeared")?;
    ensure!(
        active.poll(&player, &path, &cues, &first_finished)? == Transition::Advanced,
        "finished sentence did not advance to the next valid cue"
    );
    ensure!(
        active.index == 1 && active.request_id != first.request_id,
        "next sentence reused the old request"
    );
    ensure!(
        session.set_sentence(video_lease, Some(active)),
        "current video lease rejected advancement"
    );
    ensure!(
        active.poll(&player, &path, &cues, &first_finished)? == Transition::Unchanged,
        "old finished snapshot advanced the new request again"
    );
    record(
        checks,
        "actual three-play completion advances once and ignores old completion",
        &session,
        &first_finished,
    );
    let final_finished = wait(&player, "video last three plays", traces, |s| {
        s.sentence_repeat
            .as_ref()
            .is_some_and(|r| r.request_id == active.request_id && r.finished)
    })?;
    ensure!(
        final_finished.paused
            && final_finished
                .sentence_repeat
                .as_ref()
                .unwrap()
                .current_play
                == 3,
        "last sentence did not pause after three plays"
    );
    ensure!(
        active.poll(&player, &path, &cues, &final_finished)? == Transition::Completed,
        "last valid sentence did not complete"
    );
    ensure!(
        active.poll(&player, &path, &cues, &final_finished)? == Transition::Unchanged,
        "last sentence completion was consumed twice"
    );
    ensure!(
        session.set_video_sentence(Some(active)),
        "video owner cannot store completion"
    );
    record(
        checks,
        "last sentence completes once without crossing the boundary",
        &session,
        &final_finished,
    );

    let card_lease = session.claim(
        Owner::Card {
            kind: card.kind.clone(),
            id: card.id.clone(),
        },
        path.clone(),
        Mode::Normal,
    );
    ensure!(
        !session.is_current(video_lease)
            && session.is_current(card_lease)
            && session.sentence().is_none(),
        "same-source card retained the video sentence"
    );
    ensure!(
        !session.set_sentence(video_lease, Some(active))
            && !session.clear_sentence(video_lease)
            && !session.set_audio_sentence(None)
            && !session.set_video_sentence(None),
        "a former page can mutate card session metadata"
    );
    let revision = player.snapshot().load_revision;
    player.open_range(Path::new(&card.media), card.start, card.end, false);
    loaded(&player, &path, revision, "same-source card claim", traces)?;
    let clip_end = wait(&player, "actual saved clip end", traces, |s| {
        s.eof && s.paused
    })?;
    ensure!(
        clip_end.sentence_repeat.is_none()
            && clip_end.position >= card.start
            && clip_end.position <= card.end + 0.09,
        "card playback escaped its saved range or inherited a repeat"
    );
    record(
        checks,
        "same-source card obeys its SQLite range with no page repeat",
        &session,
        &clip_end,
    );

    let returned_video = session.claim(Owner::Video, path.clone(), Mode::SentenceThree);
    ensure!(
        !session.is_current(card_lease) && !session.clear_sentence(card_lease),
        "former card lease remained live on video return"
    );
    let revision = player.snapshot().load_revision;
    player.open_range(&path, 0.7, 0., true);
    loaded(&player, &path, revision, "return complete video", traces)?;
    let state = wait(&player, "video restored bookmark", traces, |s| {
        s.paused && (s.position - 0.7).abs() < 0.09
    })?;
    ensure!(
        state.sentence_repeat.is_none(),
        "video restore inherited card/native sentence state"
    );
    // Real playback must be able to cross the former clip end after full-video restore.
    player.set("pause", "no");
    wait(
        &player,
        "complete video crosses old card endpoint",
        traces,
        |s| s.position > card.end + 0.1 && !s.eof,
    )?;
    player.set("pause", "yes");
    wait(&player, "pause returned video", traces, |s| s.paused)?;
    record(
        checks,
        "video return restores own position and clears former card endpoint",
        &session,
        &player.snapshot(),
    );

    let next_video = session.claim(Owner::Video, path.clone(), Mode::Normal);
    ensure!(
        !session.is_current(returned_video)
            && session.is_current(next_video)
            && !session.set_sentence(returned_video, Some(active))
            && !session.clear_sentence(returned_video),
        "same-owner same-path reopen accepted a previous generation"
    );
    record(
        checks,
        "same-owner same-path reopen expires the old generation",
        &session,
        &player.snapshot(),
    );
    let audio_lease = session.claim(
        Owner::Audio(audio_id.clone()),
        path.clone(),
        Mode::from_index(original_audio_preference.parse()?),
    );
    let revision = player.snapshot().load_revision;
    player.open_range(&path, 2.1, 0., true);
    loaded(&player, &path, revision, "return audio", traces)?;
    wait(&player, "audio restored own bookmark", traces, |s| {
        s.paused && (s.position - 2.1).abs() < 0.09
    })?;
    ensure!(
        session.mode == Mode::SentenceThree
            && session.is_current(audio_lease)
            && session.video_sentence().is_none(),
        "audio return inherited video mode/session"
    );
    record(
        checks,
        "audio return retains its own selected mode and position",
        &session,
        &player.snapshot(),
    );

    session.claim(Owner::Background, path.clone(), Mode::Track);
    ensure!(
        !session.set_audio_sentence(None)
            && !session.set_video_sentence(None)
            && !session.is_current(audio_lease),
        "background owner can be controlled by a learning page"
    );
    record(
        checks,
        "background playback has its own owner",
        &session,
        &player.snapshot(),
    );
    session.release();
    player.command(["stop"]);
    let stopped = wait(&player, "release and stop", traces, |s| s.path.is_empty())?;
    ensure!(
        session.lease().is_none()
            && session.owner().is_none()
            && session.path.is_none()
            && session.sentence().is_none()
            && session.loop_a.is_none()
            && session.loop_b.is_none()
            && !session.end_handled,
        "release retained an active owner/material/policy"
    );
    record(
        checks,
        "release clears ownership and active policy",
        &session,
        &stopped,
    );
    drop(lib);
    let reopened = Library::open(root.join("library"))?;
    ensure!(
        reopened.setting("audio_repeat_mode")? == original_audio_preference
            && reopened.setting("last_listening_position")? == original_audio_position
            && reopened.setting("last_video_position")? == original_video_position,
        "session handoff overwrote persisted learning preference/bookmarks"
    );
    ensure!(
        reopened
            .cards(false)?
            .iter()
            .any(|c| c.id == card.id && c.media == card.media),
        "same-source clip reference did not survive actual SQLite reopen"
    );
    checks.push(json!({"case":"session transitions preserve SQLite learning bookmarks and audio preference on reopen",
        "passed":true,"audio_repeat_mode":original_audio_preference,
        "audio_bookmark":serde_json::from_str::<Value>(&original_audio_position)?,
        "video_bookmark":serde_json::from_str::<Value>(&original_video_position)?}));
    Ok(())
}

pub fn run(output: &Path, results: &mut Vec<Value>) -> Result<()> {
    let root = output.join(format!("playback-session-{}", data::id()));
    fs::create_dir_all(&root)?;
    let started = Instant::now();
    let mut traces = vec![];
    let mut checks = vec![];
    let result = sequence(&root, &mut traces, &mut checks);
    let report = root.join("playback-session-report.json");
    fs::write(
        &report,
        serde_json::to_vec_pretty(&json!({
            "passed":result.is_ok(),"error":result.as_ref().err().map(|e| format!("{e:#}")),
            "elapsed_ms":started.elapsed().as_millis(),"checks":checks,"observations":traces,
            "actual_sqlite":true,"actual_wav":true,"actual_libmpv":true,"audio_device":"null",
            "scope":"public shared-session contract and native-player integration; AppView handoff is covered separately by native probes",
            "physical_audio_verified":false,"independent_review":false,
            "replay":{"bin":"e2e","args":["<new-evidence-directory>","--playback-session-only"],
                "fixture":"shared-source.wav","sqlite":"library/clarora.db"}
        }))?,
    )?;
    results.push(
        json!({"workflow":"共享播放会话owner/代际隔离 → 同源音视频闪卡移交 → 三次完成 → SQLite重开",
        "passed":result.is_ok(),"checks":checks.len(),"report":report}),
    );
    result
}
