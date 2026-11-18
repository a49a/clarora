//! Hidden-window native UI smoke. Windows CI runs this without macOS-only pointer probes.
use super::*;
use std::process::Command;

pub fn run(root: &Path, results: &mut Vec<Value>) -> Result<()> {
    let workflow = "原生窗口冒烟 → 核心页面导航 / 助手开关";
    if !cfg!(debug_assertions) || !cfg!(any(target_os = "macos", target_os = "windows")) {
        results.push(json!({"workflow":workflow,"passed":true,"skipped":true,"reason":"Requires a debug desktop graphical session"}));
        return Ok(());
    }
    let out = root.join(format!("native-ui-smoke-{}", clarora_gpui::data::id()));
    fs::create_dir_all(&out)?;
    let executable = std::env::current_exe()?
        .parent()
        .context("缺少可执行文件目录")?
        .join(if cfg!(windows) {
            "clarora-gpui.exe"
        } else {
            "clarora-gpui"
        });
    ensure!(
        executable.is_file(),
        "原生窗口冒烟需要先 cargo build --bin clarora-gpui：{}",
        executable.display()
    );
    let mut command = Command::new(&executable);
    command
        .arg("--ui-smoke-check")
        .arg(&out)
        .arg("--data-dir")
        .arg(out.join("library"));
    let started = Instant::now();
    let status = native_diagnostic::run(command, &out, Duration::from_secs(90))?;
    let report_path = out.join("ui-smoke-report.json");
    let report: Value = serde_json::from_slice(
        &fs::read(&report_path)
            .with_context(|| format!("原生窗口冒烟缺少报告：{}", report_path.display()))?,
    )?;
    ensure!(
        status.success()
            && report["passed"] == true
            && report["actual_window"] == true
            && report["cases"]
                .as_array()
                .is_some_and(|cases| cases.len() >= 9),
        "原生窗口冒烟失败：{}",
        report_path.display()
    );
    results.push(json!({
        "workflow": workflow,
        "passed": true,
        "report": report_path,
        "seconds": started.elapsed().as_secs_f64()
    }));
    Ok(())
}
