//! Synthetic long conversation, actual native editor/layout and isolated draft persistence.
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
    let workflow = "AI助手长对话输入/绘制延迟与中文粘贴/复制/草稿保存";
    if !cfg!(debug_assertions)
        || !cfg!(target_os = "macos")
        || std::env::var("CLARORA_GUI_MEMORY_CHECK").as_deref() != Ok("1")
    {
        results.push(json!({"workflow":workflow,"passed":true,"skipped":true,"reason":"Requires macOS graphical session and CLARORA_GUI_MEMORY_CHECK=1"}));
        return Ok(());
    }
    let out = root.join(format!(
        "native-assistant-latency-{}",
        clarora_gpui::data::id()
    ));
    fs::create_dir_all(&out)?;
    let exe = std::env::current_exe()?
        .parent()
        .context("executable directory")?
        .join("clarora-gpui");
    let mut child = Command::new(exe)
        .arg("--assistant-latency-check")
        .arg(&out)
        .env(
            "CLARORA_ASSISTANT_LATENCY_REQUIRE_CACHED",
            std::env::var("CLARORA_ASSISTANT_LATENCY_REQUIRE_CACHED")
                .unwrap_or_else(|_| "1".into()),
        )
        .stdin(Stdio::null())
        .stdout(fs::File::create(out.join("stdout.log"))?)
        .stderr(fs::File::create(out.join("stderr.log"))?)
        .spawn()?;
    let began = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if began.elapsed() > Duration::from_secs(180) {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("assistant latency diagnostic timeout: {}", out.display());
        }
        thread::sleep(Duration::from_millis(20));
    };
    let path = out.join("assistant-latency-report.json");
    let report: Value = serde_json::from_slice(&fs::read(&path)?)?;
    ensure!(
        status.success()
            && report["passed"] == true
            && report["cases"].as_array().is_some_and(|c| c.len() >= 6),
        "assistant latency diagnostic failed: {}",
        path.display()
    );
    results.push(json!({"workflow":workflow,"passed":true,"report":path,"typing_total_us":report["typing_total_us"]}));
    Ok(())
}
