//! Run the hidden-window A/B text marker drag diagnostic and verify its report.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command, time::Duration};

pub fn run(root: &Path, results: &mut Vec<Value>) -> Result<()> {
    let workflow = "音频字幕 A/B 竖线拖动边界且不触发文字选区";
    if !cfg!(debug_assertions)
        || !cfg!(target_os = "macos")
        || std::env::var("CLARORA_GUI_MEMORY_CHECK").as_deref() != Ok("1")
    {
        results.push(json!({"workflow":workflow,"passed":true,"skipped":true,"reason":"Requires macOS graphical session and CLARORA_GUI_MEMORY_CHECK=1"}));
        return Ok(());
    }
    let out = root.join(format!(
        "native-audio-boundary-{}",
        clarora_gpui::data::id()
    ));
    fs::create_dir_all(&out)?;
    let exe = std::env::current_exe()?
        .parent()
        .context("executable directory")?
        .join("clarora-gpui");
    let mut command = Command::new(exe);
    command.arg("--audio-boundary-check").arg(&out);
    let status = super::native_diagnostic::run(command, &out, Duration::from_secs(60))?;
    let report_path = out.join("audio-boundary-report.json");
    let report: Value = serde_json::from_slice(&fs::read(&report_path)?)?;
    ensure!(
        status.success()
            && report["passed"] == true
            && report["cases"].as_array().is_some_and(|cases| cases.iter().any(
                |case| case["case"]
                    == "audio subtitle A/B vertical markers drag both boundaries without selecting text"
            )),
        "audio A/B text marker drag failed: {}",
        report_path.display()
    );
    results.push(json!({"workflow":workflow,"passed":true,"report":report_path}));
    Ok(())
}
