//! Run the actual hidden native AppView against isolated loopback backup fixtures.
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
    let workflow =
        "原生后台云备份与同步：跨模块进度/确认前不满100%/取消与失败/草稿保留/同步开关及保留策略";
    if !cfg!(debug_assertions)
        || !cfg!(target_os = "macos")
        || std::env::var("CLARORA_GUI_MEMORY_CHECK").as_deref() != Ok("1")
    {
        results.push(json!({"workflow":workflow,"passed":true,"skipped":true,"reason":"需要启用 CLARORA_GUI_MEMORY_CHECK=1 的 macOS 原生图形会话"}));
        return Ok(());
    }
    let executable = std::env::current_exe()?
        .parent()
        .context("缺少可执行文件目录")?
        .join("clarora-gpui");
    ensure!(
        executable.is_file(),
        "后台备份原生验收需要先构建 clarora-gpui"
    );
    let out = root.join(format!(
        "native-backup-progress-{}",
        clarora_gpui::data::id()
    ));
    fs::create_dir_all(&out)?;
    let mut child = Command::new(&executable)
        .arg("--backup-progress-check")
        .arg(&out)
        .arg("--data-dir")
        .arg(out.join("library"))
        .stdin(Stdio::null())
        .stdout(fs::File::create(out.join("stdout.log"))?)
        .stderr(fs::File::create(out.join("stderr.log"))?)
        .spawn()?;
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("后台备份原生验收超时：{}", out.display());
        }
        thread::sleep(Duration::from_millis(20));
    };
    let report_path = out.join("backup-progress-ui-report.json");
    let report: Value = serde_json::from_slice(
        &fs::read(&report_path)
            .with_context(|| format!("后台备份原生验收缺少报告：{}", out.display()))?,
    )?;
    ensure!(
        status.success() && report["passed"] == true,
        "后台备份原生验收失败，详见 {}",
        report_path.display()
    );
    ensure!(
        report["cases"]
            .as_array()
            .is_some_and(|cases| cases.len() >= 13),
        "后台备份原生验收未覆盖全部步骤"
    );
    results.push(json!({"workflow":workflow,"passed":true,"report":report_path,"seconds":started.elapsed().as_secs_f64()}));
    Ok(())
}
