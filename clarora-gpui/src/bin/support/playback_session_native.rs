//! Dedicated AppView acceptance, independent of the older video-resume probe.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

pub fn run(root: &Path, results: &mut Vec<Value>) -> Result<()> {
    let workflow = "原生共享播放会话：音视频/同源闪卡移交、偏好与位置恢复、迟到请求隔离";
    if !cfg!(debug_assertions)
        || !cfg!(target_os = "macos")
        || std::env::var("CLARORA_GUI_MEMORY_CHECK").as_deref() != Ok("1")
    {
        results.push(json!({"workflow":workflow,"passed":true,"skipped":true,
            "reason":"Requires macOS graphical session and CLARORA_GUI_MEMORY_CHECK=1"}));
        return Ok(());
    }
    let out = root.join(format!(
        "native-playback-session-{}",
        clarora_gpui::data::id()
    ));
    fs::create_dir_all(&out)?;
    let exe = std::env::current_exe()?
        .parent()
        .context("executable directory")?
        .join("clarora-gpui");
    fs::write(
        out.join("invocation.json"),
        serde_json::to_vec_pretty(&json!({
            "executable":exe,"sha256":hex::encode(Sha256::digest(fs::read(&exe)?)),
            "args":["--playback-session-check",out],"environment":{"CLARORA_GUI_MEMORY_CHECK":"1"},
            "replay":"CLARORA_GUI_MEMORY_CHECK=1 cargo run --manifest-path clarora-gpui/Cargo.toml --locked --bin e2e -- <new-evidence-directory> --playback-session-native-only",
            "uses_isolated_fixture":true,"physical_audio_verified":false
        }))?,
    )?;
    let mut command = Command::new(&exe);
    command.arg("--playback-session-check").arg(&out);
    let started = Instant::now();
    let status = super::native_diagnostic::run(command, &out, Duration::from_secs(90))?;
    let path = out.join("playback-session-native-report.json");
    let report: Value = serde_json::from_slice(
        &fs::read(&path).with_context(|| format!("missing report {}", path.display()))?,
    )?;
    let cases = report["cases"]
        .as_array()
        .context("missing native playback session cases")?;
    let required = [
        "legacy SQLite audio preference loads through AppView",
        "audio triple sentence belongs only to audio owner",
        "audio paused sentence seek preserves its own position",
        "same-source audio to video releases old request and saves audio bookmark",
        "inactive audio seek and reset leave native video request intact",
        "video triple preference and paused position remain independent",
        "same-source card claims its own owner and saved range",
        "preloaded card swap claims a new session and clears old policy",
        "preloaded card native playback stops at its own clip endpoint",
        "no-media card releases shared owner native request and cached snapshot",
        "returning video restores its bookmark beyond old card endpoint",
        "same-owner same-source reopen rejects archived native snapshot",
        "video track-loop preference selects typed policy through Action",
        "leaving video preserves track-loop preference while releasing active policy",
        "returning video restores track-loop preference without an old sentence request",
        "late PartVideo result caches without replacing a newer same-source session",
        "returning audio restores its own paused position mode and first play",
        "real SQLite reopen preserves independent page preferences and bookmarks",
        "shared native audio request finishes final cue exactly three times",
        "background import owns a separate session and preserves learning preferences",
        "background playback does not overwrite audio or video bookmarks",
        "leaving background releases owner loop preloader and native playback",
    ];
    ensure!(
        status.success()
            && report["passed"] == true
            && report["actual_appview_handlers"] == true
            && report["actual_libmpv"] == true
            && report["isolated_sqlite"] == true
            && cases.len() >= required.len()
            && required
                .iter()
                .all(|name| cases.iter().any(|case| case["case"] == *name))
            && cases.iter().all(|case| case["passed"] == true),
        "native playback session failed: {}",
        path.display()
    );
    results.push(
        json!({"workflow":workflow,"passed":true,"checks":cases.len(),
        "report":path,"seconds":started.elapsed().as_secs_f64()}),
    );
    Ok(())
}
