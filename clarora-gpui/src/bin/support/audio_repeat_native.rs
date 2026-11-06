//! Actual AppView + libmpv + isolated SQLite/SRT; not physical keyboard input.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
pub fn run(root: &Path, results: &mut Vec<Value>) -> Result<()> {
    let workflow = "音频逐句循环/定位与切页保留/助手选区删除与侧栏隔离";
    if !cfg!(debug_assertions)
        || !cfg!(target_os = "macos")
        || std::env::var("CLARORA_GUI_MEMORY_CHECK").as_deref() != Ok("1")
    {
        results.push(json!({"workflow":workflow,"passed":true,"skipped":true,"reason":"Requires macOS graphical session and CLARORA_GUI_MEMORY_CHECK=1"}));
        return Ok(());
    }
    let out = root.join(format!("native-audio-repeat-{}", clarora_gpui::data::id()));
    fs::create_dir_all(&out)?;
    let exe = std::env::current_exe()?
        .parent()
        .context("executable directory")?
        .join("clarora-gpui");
    let mut child = Command::new(exe)
        .arg("--audio-repeat-check")
        .arg(&out)
        .stdin(Stdio::null())
        .stdout(fs::File::create(out.join("stdout.log"))?)
        .stderr(fs::File::create(out.join("stderr.log"))?)
        .spawn()?;
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() > Duration::from_secs(45) {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("audio repeat diagnostic timeout: {}", out.display());
        }
        thread::sleep(Duration::from_millis(20));
    };
    let report_path = out.join("audio-repeat-report.json");
    let report: Value = serde_json::from_slice(&fs::read(&report_path)?)?;
    let cases = report["cases"]
        .as_array()
        .context("missing audio repeat cases")?;
    ensure!(
        status.success()
            && report["passed"] == true
            && cases.len() >= 37
            && [
                "close waiting sentence card and reclaim subtitle space",
                "existing current sentence action reopens the card",
                "close active sentence card without moving playback",
                "restore sentence card after closing an active cue",
                "mouse selection and backspace edit assistant question",
                "mouse selection deletes Chinese and emoji without splitting characters",
                "mouse selection across wrapped multilingual text deletes correctly",
                "mouse selection and forward delete edit assistant question",
                "mouse selection in assistant reply can be quoted",
                "select all and delete multiline multilingual question",
                "ordinary backspace removes a complete emoji",
                "uncovered subtitle selection still offers an AI quote",
                "subtitle drag into assistant finishes with an AI quote",
                "background editor drag into assistant finishes selection",
                "narrow assistant mouse selection and backspace",
                "double click selects the entire multilingual assistant question",
                "single click still places the assistant question caret",
                "typing replaces the entire double-click selection",
            ]
            .iter()
            .all(|name| cases.iter().any(|case| case["case"] == *name)),
        "audio repeat failed: {}",
        report_path.display()
    );
    results.push(json!({"workflow":workflow,"passed":true,"report":report_path}));
    Ok(())
}
