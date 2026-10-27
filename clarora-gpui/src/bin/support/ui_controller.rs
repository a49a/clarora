//! Launch the actual debug application against isolated SQLite fixtures.
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
    let workflow = "原生AppView资料/聊天/视频进度 → 快速切材料与栏目 → 关闭与全局退出保存";
    if !cfg!(debug_assertions)
        || !cfg!(target_os = "macos")
        || std::env::var("CLARORA_GUI_MEMORY_CHECK").as_deref() != Ok("1")
    {
        results.push(json!({"workflow":workflow,"passed":true,"skipped":true,"reason":"需要启用CLARORA_GUI_MEMORY_CHECK=1的macOS原生图形会话"}));
        return Ok(());
    }
    let executable = std::env::current_exe()?
        .parent()
        .context("缺少可执行文件目录")?
        .join("clarora-gpui");
    ensure!(
        executable.is_file(),
        "原生协调验收需要先cargo build --bin clarora-gpui：{}",
        executable.display()
    );
    let mut checks = vec![];
    for (name, flag) in [
        ("window-close", "--ui-controller-check"),
        ("global-quit", "--ui-controller-quit-check"),
    ] {
        let out = root
            .join("native-ui")
            .join(format!("{name}-{}", clarora_gpui::data::id()));
        fs::create_dir_all(&out)?;
        let report_path = out.join("ui-controller-report.json");
        if report_path.exists() {
            fs::remove_file(&report_path)?;
        }
        let mut child = Command::new(&executable)
            .arg(flag)
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
                anyhow::bail!("原生协调验收超时：{}", out.display());
            }
            thread::sleep(Duration::from_millis(20));
        };
        let report: Value = serde_json::from_slice(
            &fs::read(&report_path)
                .with_context(|| format!("原生协调验收缺少报告：{}", out.display()))?,
        )?;
        ensure!(
            status.success() && report["passed"] == true,
            "原生协调验收失败：{}，详见{}",
            name,
            report_path.display()
        );
        let video_report_path = out.join("video-resume-report.json");
        let video_report: Value = serde_json::from_slice(&fs::read(&video_report_path)?)?;
        ensure!(
            video_report["passed"] == true
                && video_report["cases"]
                    .as_array()
                    .is_some_and(|cases| cases.len() >= 8),
            "视频切页进度验收未完成：{}",
            video_report_path.display()
        );
        checks.push(json!({"case":name,"passed":true,"report":report_path,"video_resume_report":video_report_path,"seconds":started.elapsed().as_secs_f64()}));
    }
    results.push(json!({"workflow":workflow,"passed":true,"checks":checks}));
    Ok(())
}
