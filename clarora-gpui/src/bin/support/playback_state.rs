//! Real native playback transitions consumed by the UI snapshot controller.
use anyhow::{Result, ensure};
use clarora_gpui::{
    data::Library,
    media::{Playback, Player, visible_changed},
    process,
};
use serde_json::{Value, json};
use std::{
    fs,
    time::{Duration, Instant},
};

fn wait(player: &Player, predicate: impl Fn(&Playback) -> bool) -> Result<Playback> {
    let start = Instant::now();
    loop {
        let snapshot = player.snapshot();
        if predicate(&snapshot) {
            return Ok(snapshot);
        }
        ensure!(
            start.elapsed() < Duration::from_secs(8),
            "等待播放器快照超时：{}",
            snapshot.error
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub fn run(lib: &Library, results: &mut Vec<Value>) -> Result<()> {
    let root = lib.root.join("playback-state");
    fs::create_dir_all(&root)?;
    let player = Player::without_audio_device();
    let ready = wait(&player, |state| {
        state.initialized || !state.error.is_empty()
    })?;
    ensure!(ready.error.is_empty(), "{}", ready.error);
    ensure!(
        visible_changed(&Playback::default(), &ready),
        "初始化状态未通知界面"
    );
    player.open(&root.join("missing-video.mp4"));
    let failed = wait(&player, |state| !state.error.is_empty())?;
    let original_ui_changed = ready.paused != failed.paused
        || (ready.position - failed.position).abs() > 0.01
        || failed.frame.is_some();
    ensure!(!original_ui_changed, "无效媒体没有进入原界面漏刷新场景");
    ensure!(
        visible_changed(&ready, &failed),
        "暂停状态下的媒体错误未通知界面"
    );
    let video = root.join("frame-fixture.mp4");
    process::run(
        &process::tool("ffmpeg")?,
        &[
            "-nostdin".into(),
            "-hide_banner".into(),
            "-loglevel".into(),
            "error".into(),
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            "color=green:s=64x64:r=30:d=1".into(),
            "-pix_fmt".into(),
            "yuv420p".into(),
            "-y".into(),
            video.to_string_lossy().into_owned(),
        ],
        &process::cancel_token(),
        Duration::from_secs(15),
    )?;
    player.open_range(&video, 0., 0., true);
    let frame = wait(&player, |state| {
        state.frame.is_some() && state.paused && state.error.is_empty()
    })?;
    ensure!(visible_changed(&failed, &frame), "媒体恢复/出帧未通知界面");
    player.command(["stop"]);
    let stopped = wait(&player, |state| {
        state.frame.is_none() && state.path.is_empty()
    })?;
    ensure!(visible_changed(&frame, &stopped), "清空旧画面未通知界面");
    let evidence = root.join("playback-state-evidence.json");
    fs::write(
        &evidence,
        serde_json::to_vec_pretty(&json!({
            "passed":true, "checks":4, "native_initialized":ready.initialized,
            "missing_media_error":failed.error, "paused":failed.paused,
            "original_ui_would_redraw":original_ui_changed,
            "recovered_real_frame":frame.frame.is_some(), "frame_cleared":stopped.frame.is_none(),
            "stopped_path":stopped.path,
        }))?,
    )?;
    results.push(
        json!({"workflow":"原生播放器快照：初始化/暂停报错/恢复解码/画面清空均通知界面",
        "passed":true,"checks":4,"evidence":evidence}),
    );
    Ok(())
}
