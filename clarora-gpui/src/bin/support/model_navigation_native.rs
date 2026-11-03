//! Repeatable native navigation E2E, with its SQLite/media/HTTP/report artifacts retained.
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
    let workflow = "原生后台任务与音频学习：下载/字幕生成翻译/切组切音频/独立取消/合并与恢复";
    if !cfg!(debug_assertions)
        || !cfg!(target_os = "macos")
        || std::env::var("CLARORA_GUI_MEMORY_CHECK").as_deref() != Ok("1")
    {
        results.push(json!({"workflow":workflow,"passed":true,"skipped":true,"reason":"需要 CLARORA_GUI_MEMORY_CHECK=1 与 macOS 图形会话"}));
        return Ok(());
    }
    let exe = std::env::current_exe()?
        .parent()
        .context("executable directory")?
        .join("clarora-gpui");
    let out = root.join(format!(
        "native-model-navigation-{}",
        clarora_gpui::data::id()
    ));
    fs::create_dir_all(&out)?;
    let mut child = Command::new(exe)
        .arg("--model-navigation-check")
        .arg(&out)
        .arg("--data-dir")
        .arg(out.join("library"))
        .stdin(Stdio::null())
        .stdout(fs::File::create(out.join("stdout.log"))?)
        .stderr(fs::File::create(out.join("stderr.log"))?)
        .spawn()?;
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() > Duration::from_secs(80) {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("model navigation timed out: {}", out.display());
        }
        thread::sleep(Duration::from_millis(20));
    };
    let path = out.join("model-navigation-report.json");
    let report: Value = serde_json::from_slice(
        &fs::read(&path).with_context(|| format!("missing report {}", path.display()))?,
    )?;
    ensure!(
        status.success()
            && report["passed"] == true
            && report["cases"].as_array().is_some_and(|v| v.len() >= 24),
        "native model navigation failed: {}",
        path.display()
    );
    results.push(json!({"workflow":workflow,"passed":true,"report":path,"seconds":start.elapsed().as_secs_f64()}));
    Ok(())
}
