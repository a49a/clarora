//! Hidden inactive AppView + real AppKit presentation flags and SQLite.
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
    let workflow = "电脑专注：原生选项/计时与关闭保护/应急解除/到期恢复/不影响桌面";
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
    let mut child = Command::new(exe)
        .arg("--focus-mode-check")
        .arg(&out)
        .arg("--data-dir")
        .arg(out.join("library"))
        .env("CLARORA_FOCUS_DIAGNOSTIC_INACTIVE", "1")
        .stdin(Stdio::null())
        .stdout(fs::File::create(out.join("stdout.log"))?)
        .stderr(fs::File::create(out.join("stderr.log"))?)
        .spawn()?;
    let began = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            ensure!(status.success(), "native focus process failed");
            break;
        }
        if began.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("native focus timeout");
        }
        thread::sleep(Duration::from_millis(20));
    }
    let report = out.join("focus-mode-report.json");
    let value: Value = serde_json::from_slice(&fs::read(&report)?)?;
    ensure!(
        value["passed"] == true && value["cases"].as_array().is_some_and(|c| c.len() >= 6),
        "native focus failed: {}",
        report.display()
    );
    results.push(json!({"workflow":workflow,"passed":true,"report":report}));
    Ok(())
}
