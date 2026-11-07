//! Actual AppView + libmpv + isolated SQLite/SRT; not physical keyboard input.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command, time::Duration};
pub fn run(root: &Path, results: &mut Vec<Value>) -> Result<()> {
    let workflow = "音频移除确认弹框/取消/单条与批量/变更保护/附件保留";
    if !cfg!(debug_assertions)
        || !cfg!(target_os = "macos")
        || std::env::var("CLARORA_GUI_MEMORY_CHECK").as_deref() != Ok("1")
    {
        results.push(json!({"workflow":workflow,"passed":true,"skipped":true,"reason":"Requires macOS graphical session and CLARORA_GUI_MEMORY_CHECK=1"}));
        return Ok(());
    }
    let out = root.join(format!("native-audio-remove-{}", clarora_gpui::data::id()));
    fs::create_dir_all(&out)?;
    let exe = std::env::current_exe()?
        .parent()
        .context("executable directory")?
        .join("clarora-gpui");
    let mut command = Command::new(exe);
    command.arg("--audio-remove-check").arg(&out);
    let status = super::native_diagnostic::run(command, &out, Duration::from_secs(45))?;
    let report_path = out.join("audio-remove-report.json");
    let report: Value = serde_json::from_slice(&fs::read(&report_path)?)?;
    ensure!(
        status.success()
            && report["passed"] == true
            && report["cases"].as_array().is_some_and(|c| c.len() >= 6),
        "audio remove failed: {}",
        report_path.display()
    );
    results.push(json!({"workflow":workflow,"passed":true,"report":report_path}));
    Ok(())
}
