//! Real libmpv sentence repeat workflow, with the failure scenarios recorded first.
use anyhow::{Context, Result, ensure};
use clarora_gpui::{
    data::Library,
    media::{Playback, Player},
    process,
};
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

fn observation(stage: &str, start: Instant, state: &Playback) -> Value {
    json!({
        "stage": stage,
        "elapsed_ms": start.elapsed().as_millis(),
        "path": state.path,
        "position": state.position,
        "duration": state.duration,
        "paused": state.paused,
        "eof": state.eof,
        "frame": state.frame.is_some(),
        "error": state.error,
        "repeat": state.sentence_repeat.as_ref().map(|r| json!({
            "request_id": r.request_id, "path": r.path, "start": r.start, "end": r.end,
            "total_plays": r.total_plays, "current_play": r.current_play,
            "finished": r.finished,
        })),
    })
}

fn wait(
    player: &Player,
    stage: &str,
    timeout: Duration,
    traces: &mut Vec<Value>,
    predicate: impl Fn(&Playback) -> bool,
) -> Result<Playback> {
    let started = Instant::now();
    let mut previous = Value::Null;
    loop {
        let state = player.snapshot();
        let entry = observation(stage, started, &state);
        // The worker publishes asynchronously; retain each distinct observation.
        let mut signature = entry.clone();
        signature.as_object_mut().unwrap().remove("elapsed_ms");
        if signature != previous {
            previous = signature;
            traces.push(entry.clone());
        }
        ensure!(state.error.is_empty(), "{stage}: {}", state.error);
        if predicate(&state) {
            return Ok(state);
        }
        ensure!(
            started.elapsed() < timeout,
            "{stage}: 超时，末次快照 {entry}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn observe_for(
    player: &Player,
    stage: &str,
    duration: Duration,
    traces: &mut Vec<Value>,
    predicate: impl Fn(&Playback) -> bool,
) -> Result<()> {
    let started = Instant::now();
    let mut previous = Value::Null;
    loop {
        let state = player.snapshot();
        let entry = observation(stage, started, &state);
        let mut signature = entry.clone();
        signature.as_object_mut().unwrap().remove("elapsed_ms");
        if signature != previous {
            previous = signature;
            traces.push(entry.clone());
        }
        ensure!(state.error.is_empty(), "{stage}: {}", state.error);
        ensure!(predicate(&state), "{stage}: 不满足持续状态 {entry}");
        if started.elapsed() >= duration {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn ready(player: &Player, path: &Path, traces: &mut Vec<Value>) -> Result<Playback> {
    let expected = path.to_string_lossy();
    player.open_range(path, 0., 0., true);
    wait(
        player,
        "媒体解码就绪",
        Duration::from_secs(8),
        traces,
        |s| s.path == expected && s.duration > 2.9 && s.frame.is_some() && s.paused,
    )
}

fn finished(
    player: &Player,
    stage: &str,
    request_id: u64,
    start: f64,
    end: f64,
    traces: &mut Vec<Value>,
) -> Result<Playback> {
    let state = wait(player, stage, Duration::from_secs(8), traces, |s| {
        s.sentence_repeat.as_ref().is_some_and(|r| {
            r.request_id == request_id
                && r.finished
                && r.total_plays == Some(3)
                && (r.start - start).abs() < 0.001
                && (r.end - end).abs() < 0.001
        })
    })?;
    let repeat = state
        .sentence_repeat
        .as_ref()
        .context("完成时丢失本句状态")?;
    ensure!(state.paused, "{stage}: 完成态没有在同一快照发布暂停");
    ensure!(repeat.current_play == 3, "{stage}: 三遍计数错误");
    ensure!(
        (state.position - repeat.end).abs() < 0.09,
        "{stage}: 未停在句尾，position={} end={}",
        state.position,
        repeat.end
    );
    ensure!(!state.eof, "{stage}: 本句完成不应发布自然 EOF");
    Ok(state)
}

fn save_frame(path: &Path, state: &Playback) -> Result<()> {
    let frame = state.frame.as_ref().context("libmpv 没有实际解码帧")?;
    let mut bytes = frame.as_bytes(0).context("解码帧像素不存在")?.to_vec();
    for pixel in bytes.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    image::save_buffer(path, &bytes, 960, 540, image::ColorType::Rgba8)?;
    Ok(())
}

pub fn run(lib: &Library, results: &mut Vec<Value>) -> Result<()> {
    let root = lib.root.join("video-repeat");
    fs::create_dir_all(&root)?;
    let video = root.join("sentence-fixture.mp4");
    let second = root.join("another-video.mp4");
    let ffmpeg = process::tool("ffmpeg")?;
    let mut args = [
        "-nostdin",
        "-v",
        "error",
        "-y",
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=320x180:rate=120",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:sample_rate=48000",
        "-t",
        "3",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-g",
        "1",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
    ]
    .map(String::from)
    .to_vec();
    args.push(video.to_string_lossy().into_owned());
    let mut traces = vec![];
    let mut checks = vec![];
    let outcome = (|| -> Result<()> {
        process::run(
            &ffmpeg,
            &args,
            &process::cancel_token(),
            Duration::from_secs(30),
        )?;
        fs::copy(&video, &second)?;
        fs::write(
            video.with_extension("srt"),
            "1\n00:00:00,300 --> 00:00:00,650\nRepeat this sentence.\n\n2\n00:00:00,900 --> 00:00:00,940\nShort sentence.\n\n3\n00:00:02,550 --> 00:00:03,000\nLast sentence.\n",
        )?;
        let player = Player::without_audio_device();
        let initial = ready(&player, &video, &mut traces)?;
        save_frame(&root.join("decoded-first-frame.png"), &initial)?;

        let infinite_request = player.repeat_sentence(&video, 0.3, 0.65, None)?;
        wait(
            &player,
            "无限模式至少四遍",
            Duration::from_secs(5),
            &mut traces,
            |s| {
                s.sentence_repeat.as_ref().is_some_and(|r| {
                    r.request_id == infinite_request
                        && r.total_plays.is_none()
                        && r.current_play >= 4
                        && !r.finished
                }) && !s.paused
                    && !s.eof
            },
        )?;
        observe_for(
            &player,
            "无限模式保持同句",
            Duration::from_millis(500),
            &mut traces,
            |s| {
                s.path == video.to_string_lossy()
                    && !s.eof
                    && !s.paused
                    && s.position >= 0.25
                    && s.position <= 0.72
                    && s.sentence_repeat
                        .as_ref()
                        .is_some_and(|r| r.request_id == infinite_request && !r.finished)
            },
        )?;
        checks.push(json!({"scenario":"无限重复超过三遍且不越句/不EOF","passed":true}));

        let triple_request = player.repeat_sentence(&video, 0.3, 0.65, Some(3))?;
        let complete = finished(
            &player,
            "三遍后暂停",
            triple_request,
            0.3,
            0.65,
            &mut traces,
        )?;
        observe_for(
            &player,
            "三遍完成后保持暂停",
            Duration::from_millis(250),
            &mut traces,
            |s| {
                s.paused
                    && !s.eof
                    && (s.position - complete.position).abs() < 0.03
                    && s.sentence_repeat.as_ref().is_some_and(|r| {
                        r.request_id == triple_request && r.current_play == 3 && r.finished
                    })
            },
        )?;
        save_frame(&root.join("three-plays-paused.png"), &complete)?;
        checks.push(json!({"scenario":"切换三遍从句首重计并精确暂停句尾","passed":true}));

        player.set("pause", "no");
        let continued = wait(
            &player,
            "三遍完成后直接继续",
            Duration::from_secs(2),
            &mut traces,
            |s| !s.paused && s.sentence_repeat.is_none(),
        )?;
        ensure!(
            continued.position >= complete.position - 0.03,
            "完成后继续错误跳回句首或媒体开头"
        );
        wait(
            &player,
            "完成后继续越过原句尾",
            Duration::from_secs(2),
            &mut traces,
            |s| !s.paused && s.sentence_repeat.is_none() && s.position > 0.9,
        )?;
        checks.push(json!({"scenario":"三遍完成后直接继续从句尾向后播放","passed":true}));

        let before_reselect_request = player.repeat_sentence(&video, 0.3, 1.4, Some(3))?;
        wait(
            &player,
            "同模式重选前第二遍",
            Duration::from_secs(3),
            &mut traces,
            |s| {
                s.sentence_repeat.as_ref().is_some_and(|r| {
                    r.request_id == before_reselect_request
                        && r.start == 0.3
                        && r.end == 1.4
                        && r.current_play == 2
                        && !r.finished
                })
            },
        )?;
        let reselect_request = player.repeat_sentence(&video, 0.3, 1.4, Some(3))?;
        wait(
            &player,
            "同模式重选归第一遍",
            Duration::from_secs(2),
            &mut traces,
            |s| {
                !s.paused
                    && s.position < 0.6
                    && s.sentence_repeat.as_ref().is_some_and(|r| {
                        r.request_id == reselect_request
                            && r.start == 0.3
                            && r.end == 1.4
                            && r.current_play == 1
                            && !r.finished
                    })
            },
        )?;
        checks.push(json!({"scenario":"同一句同模式重选从句首重计第一遍","passed":true}));
        wait(
            &player,
            "新句第一遍中段",
            Duration::from_secs(3),
            &mut traces,
            |s| {
                !s.paused
                    && s.position > 0.65
                    && s.position < 1.1
                    && s.sentence_repeat.as_ref().is_some_and(|r| {
                        r.request_id == reselect_request && r.current_play == 1 && !r.finished
                    })
            },
        )?;
        player.set("pause", "yes");
        let paused = wait(
            &player,
            "本句中途暂停",
            Duration::from_secs(2),
            &mut traces,
            |s| s.paused,
        )?;
        let paused_count = paused
            .sentence_repeat
            .as_ref()
            .context("暂停丢失重复状态")?
            .current_play;
        observe_for(
            &player,
            "暂停不计遍数",
            Duration::from_millis(350),
            &mut traces,
            |s| {
                s.paused
                    && (s.position - paused.position).abs() < 0.03
                    && s.sentence_repeat.as_ref().is_some_and(|r| {
                        r.request_id == reselect_request
                            && r.current_play == paused_count
                            && !r.finished
                    })
            },
        )?;
        player.set("pause", "no");
        finished(
            &player,
            "恢复并完成剩余三遍",
            reselect_request,
            0.3,
            1.4,
            &mut traces,
        )?;
        checks.push(json!({"scenario":"切句重计/暂停不计数/恢复完成","passed":true}));

        let manual_seek_request = player.repeat_sentence(&video, 0.3, 1.4, Some(3))?;
        wait(
            &player,
            "手动后退前第一遍",
            Duration::from_secs(3),
            &mut traces,
            |s| {
                !s.paused
                    && s.position > 0.65
                    && s.position < 1.1
                    && s.sentence_repeat
                        .as_ref()
                        .is_some_and(|r| r.request_id == manual_seek_request && r.current_play == 1)
            },
        )?;
        player.set("pause", "yes");
        wait(
            &player,
            "手动后退前暂停",
            Duration::from_secs(2),
            &mut traces,
            |s| s.paused,
        )?;
        player.command(["seek", "0.4", "absolute+exact"]);
        wait(
            &player,
            "手动后退退出重复",
            Duration::from_secs(2),
            &mut traces,
            |s| s.paused && (s.position - 0.4).abs() < 0.05 && s.sentence_repeat.is_none(),
        )?;
        checks.push(json!({"scenario":"手动负向定位退出重复且不误计成完成","passed":true}));

        let short_request = player.repeat_sentence(&video, 0.9, 0.94, Some(3))?;
        finished(
            &player,
            "40ms短句总三遍",
            short_request,
            0.9,
            0.94,
            &mut traces,
        )?;
        checks.push(json!({"scenario":"短于UI采样的40ms句子准确三遍","passed":true}));

        let eof_request = player.repeat_sentence(&video, 2.55, initial.duration, Some(3))?;
        finished(
            &player,
            "媒体物理末句总三遍",
            eof_request,
            2.55,
            initial.duration,
            &mut traces,
        )?;
        checks.push(json!({"scenario":"物理EOF末句三遍后暂停且不产生分集EOF","passed":true}));
        // This clamps to the same target. Match the returned request token,
        // never accept the previous request's finished snapshot.
        let clamped_request =
            player.repeat_sentence(&video, 2.55, initial.duration + 0.08, Some(3))?;
        let clamped = finished(
            &player,
            "字幕末句轻微超过媒体长度",
            clamped_request,
            2.55,
            initial.duration,
            &mut traces,
        )?;
        ensure!(
            clamped.sentence_repeat.as_ref().unwrap().end <= initial.duration + 0.001,
            "字幕超长边界没有收敛到媒体长度"
        );
        checks.push(json!({"scenario":"字幕结束略超媒体时长仍可三遍","passed":true}));

        let clear_request = player.repeat_sentence(&video, 0.3, 0.65, None)?;
        wait(
            &player,
            "清除前无限重复已生效",
            Duration::from_secs(3),
            &mut traces,
            |s| {
                s.sentence_repeat.as_ref().is_some_and(|r| {
                    r.request_id == clear_request && r.total_plays.is_none() && !r.finished
                }) && !s.paused
            },
        )?;
        player.clear_sentence_repeat();
        wait(
            &player,
            "清除本句状态",
            Duration::from_secs(2),
            &mut traces,
            |s| s.sentence_repeat.is_none(),
        )?;
        player.command(["seek", "0.4", "absolute+exact"]);
        player.set("pause", "no");
        wait(
            &player,
            "清除后越过原句尾",
            Duration::from_secs(3),
            &mut traces,
            |s| s.position > 0.95 && s.sentence_repeat.is_none() && !s.eof,
        )?;
        checks.push(json!({"scenario":"清除同时移除底层AB边界并向外播放","passed":true}));

        let media_switch_request = player.repeat_sentence(&video, 0.3, 0.65, None)?;
        wait(
            &player,
            "跨媒体前本句状态",
            Duration::from_secs(3),
            &mut traces,
            |s| {
                s.sentence_repeat.as_ref().is_some_and(|r| {
                    r.request_id == media_switch_request && r.path == video.to_string_lossy()
                })
            },
        )?;
        ready(&player, &second, &mut traces)?;
        observe_for(
            &player,
            "切媒体复位不串路径",
            Duration::from_millis(250),
            &mut traces,
            |s| s.path == second.to_string_lossy() && s.sentence_repeat.is_none() && s.paused,
        )?;
        player.set("pause", "no");
        wait(
            &player,
            "新媒体不受旧句边界限制",
            Duration::from_secs(3),
            &mut traces,
            |s| {
                s.path == second.to_string_lossy()
                    && s.position > 0.95
                    && s.sentence_repeat.is_none()
            },
        )?;
        checks.push(json!({"scenario":"切媒体清除旧状态和底层循环且路径一致","passed":true}));
        drop(player);

        // These are public worker commands against libmpv, not controller-only tests.
        let invalid = Player::without_audio_device();
        ready(&invalid, &video, &mut traces)?;
        let invalid_preserved_request = invalid.repeat_sentence(&video, 0.3, 1.4, None)?;
        wait(
            &invalid,
            "非法输入前有效重复",
            Duration::from_secs(2),
            &mut traces,
            |s| {
                !s.paused
                    && s.position >= 0.3
                    && s.position < 1.4
                    && s.sentence_repeat
                        .as_ref()
                        .is_some_and(|r| r.request_id == invalid_preserved_request)
            },
        )?;
        invalid.set("pause", "yes");
        let preserved = wait(
            &invalid,
            "非法输入前暂停",
            Duration::from_secs(2),
            &mut traces,
            |s| s.paused,
        )?;
        for (label, start, end, total) in [
            ("NaN", f64::NAN, 1., Some(3)),
            ("Infinity", 0.2, f64::INFINITY, Some(3)),
            ("负数", -0.1, 1., Some(3)),
            ("倒序", 1., 0.5, Some(3)),
            ("零长", 0.5, 0.5, Some(3)),
            ("零遍", 0.2, 0.5, Some(0)),
            ("过大遍数", 0.2, 0.5, Some(u32::MAX)),
        ] {
            let error = invalid
                .repeat_sentence(&video, start, end, total)
                .err()
                .with_context(|| format!("非法范围 {label} 未拒绝"))?;
            traces.push(
                json!({"stage": "非法范围同步拒绝", "case": label, "error": error.to_string()}),
            );
        }
        observe_for(
            &invalid,
            "非法参数保持原有效配置",
            Duration::from_millis(200),
            &mut traces,
            |s| {
                s.paused
                    && (s.position - preserved.position).abs() < 0.03
                    && s.sentence_repeat.as_ref().is_some_and(|r| {
                        r.request_id == invalid_preserved_request
                            && r.start == 0.3
                            && r.end == 1.4
                            && r.total_plays.is_none()
                            && !r.finished
                    })
            },
        )?;
        ensure!(
            invalid
                .repeat_sentence(Path::new(""), 0.2, 0.5, Some(3))
                .is_err(),
            "空媒体路径没有同步拒绝"
        );
        invalid.repeat_sentence(&second, 0.2, 0.5, Some(3))?;
        let rejected_path_started = Instant::now();
        loop {
            let state = invalid.snapshot();
            if !state.error.is_empty() {
                ensure!(
                    state.path == video.to_string_lossy()
                        && state
                            .sentence_repeat
                            .as_ref()
                            .is_some_and(|r| r.request_id == invalid_preserved_request
                                && r.path == video.to_string_lossy()),
                    "迟到的异媒体请求覆盖了当前循环"
                );
                traces.push(observation(
                    "异媒体重复请求拒绝并保持当前配置",
                    rejected_path_started,
                    &state,
                ));
                break;
            }
            ensure!(
                rejected_path_started.elapsed() < Duration::from_secs(2),
                "异媒体重复请求没有失败反馈"
            );
            thread::sleep(Duration::from_millis(10));
        }
        let no_media = Player::without_audio_device();
        no_media.repeat_sentence(&video, 0.2, 0.5, Some(3))?;
        let started = Instant::now();
        loop {
            let state = no_media.snapshot();
            if !state.error.is_empty() {
                ensure!(state.sentence_repeat.is_none(), "无媒体留下重复状态");
                traces.push(observation("无媒体拒绝", started, &state));
                break;
            }
            ensure!(
                started.elapsed() < Duration::from_secs(3),
                "无媒体重复没有失败反馈"
            );
            thread::sleep(Duration::from_millis(10));
        }
        checks
            .push(json!({"scenario":"无媒体/异媒体失败反馈与非法时间/遍数原子拒绝","passed":true}));
        Ok(())
    })();
    fs::write(
        root.join("repeat-evidence.json"),
        serde_json::to_vec_pretty(&json!({
            "passed": outcome.is_ok(),
            "error": outcome.as_ref().err().map(|e| format!("{e:#}")),
            "clock": "real libmpv with null audio output", "fixture_fps": 120,
            "fixture": video, "ffmpeg": ffmpeg, "ffmpeg_args": args,
            "checks": checks, "observations": traces,
            "frames": ["decoded-first-frame.png", "three-plays-paused.png"],
        }))?,
    )?;
    outcome?;
    results.push(json!({
        "workflow": "真实视频逐句无限/三遍重复、短句与EOF、暂停恢复与清除、切媒体复位",
        "passed": true,
        "checks": checks.len(),
        "evidence": root.join("repeat-evidence.json"),
    }));
    Ok(())
}
