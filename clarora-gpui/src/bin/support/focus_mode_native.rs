//! Hidden inactive AppView + real AppKit presentation flags and SQLite.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command, time::Duration};
pub fn run(root: &Path, results: &mut Vec<Value>) -> Result<()> {
    let workflow = "电脑专注：原生选项、全局快捷键、失活恢复、计时与关闭保护";
    if !cfg!(debug_assertions)
        || !cfg!(target_os = "macos")
        || std::env::var("CLARORA_GUI_MEMORY_CHECK").as_deref() != Ok("1")
    {
        results.push(json!({"workflow":workflow,"passed":true,"skipped":true,"reason":"需要 macOS 图形会话与 CLARORA_GUI_MEMORY_CHECK=1"}));
        return Ok(());
    }
    let exe = std::env::current_exe()?
        .parent()
        .context("executable parent")?
        .join("clarora-gpui");
    let out = root.join(format!("native-focus-mode-{}", clarora_gpui::data::id()));
    fs::create_dir_all(&out)?;
    let mut command = Command::new(exe);
    command
        .arg("--focus-mode-check")
        .arg(&out)
        .arg("--data-dir")
        .arg(out.join("library"))
        .env("CLARORA_FOCUS_DIAGNOSTIC_INACTIVE", "1");
    super::native_diagnostic::run(command, &out, Duration::from_secs(45))?;
    let report = out.join("focus-mode-report.json");
    let value: Value = serde_json::from_slice(&fs::read(&report)?)?;
    let cases = value["cases"]
        .as_array()
        .context("missing native focus cases")?;
    let recovery_cases = [
        "AppKit deactivation releases restrictions and pauses the timer",
        "occupied global shortcut leaves the desktop and ordinary timer usable",
        "late shortcut event cannot affect the next focus session",
        "Carbon shortcut bypasses the button countdown and saves the paused timer",
        "failed pause save still releases restrictions and unregisters the shortcut",
    ];
    ensure!(
        value["passed"] == true
            && cases.iter().all(|case| case["passed"] == true)
            && recovery_cases
                .iter()
                .all(|name| cases.iter().any(|case| case["case"] == *name)),
        "native focus failed: {}",
        report.display()
    );
    results.push(json!({"workflow":workflow,"passed":true,"cases":cases.len(),"report":report}));
    Ok(())
}
