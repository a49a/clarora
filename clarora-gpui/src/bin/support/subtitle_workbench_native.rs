//! Run the actual hidden AppView against isolated subtitle and loopback-AI fixtures.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

pub fn run(root: &Path, results: &mut Vec<Value>) -> Result<()> {
    let workflow =
        "原生字幕工作台：窄栏视图/纯英文复制/固定引用/空与超限保护/显式AI发送及SQLite重开";
    if !cfg!(debug_assertions)
        || !cfg!(target_os = "macos")
        || std::env::var("CLARORA_GUI_MEMORY_CHECK").as_deref() != Ok("1")
    {
        results.push(json!({"workflow":workflow,"passed":true,"skipped":true,"reason":"需要启用CLARORA_GUI_MEMORY_CHECK=1的macOS原生图形会话"}));
        return Ok(());
    }
    let executable = std::env::current_exe()?
        .parent()
        .context("缺少可执行目录")?
        .join("clarora-gpui");
    ensure!(executable.is_file(), "请先构建clarora-gpui");
    let out = root.join(format!(
        "native-subtitle-workbench-{}",
        clarora_gpui::data::id()
    ));
    fs::create_dir_all(&out)?;
    let mut command = Command::new(executable);
    command
        .arg("--subtitle-workbench-check")
        .arg(&out)
        .arg("--data-dir")
        .arg(out.join("library"));
    let started = Instant::now();
    let status = super::native_diagnostic::run(command, &out, Duration::from_secs(60))?;
    let report_path = out.join("subtitle-workbench-report.json");
    let report: Value = serde_json::from_slice(
        &fs::read(&report_path).with_context(|| format!("缺少原生验收报告：{}", out.display()))?,
    )?;
    ensure!(
        status.success() && report["passed"] == true,
        "字幕工作台原生验收失败，详见{}",
        report_path.display()
    );
    ensure!(
        report["cases"]
            .as_array()
            .is_some_and(|cases| cases.len() >= 12),
        "原生字幕工作台检查未覆盖全部流程"
    );
    results.push(json!({"workflow":workflow,"passed":true,"report":report_path,"seconds":started.elapsed().as_secs_f64()}));
    Ok(())
}
