//! Launch the actual debug application against isolated SQLite fixtures.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

/// Flashcard keys run independently from video timing and external services.
pub fn flashcard(root: &Path, results: &mut Vec<Value>) -> Result<()> {
    let workflow = "闪卡空格翻面/R 重播范围/文字焦点/输入隔离与随便学学";
    if !cfg!(debug_assertions)
        || !cfg!(target_os = "macos")
        || std::env::var("CLARORA_GUI_MEMORY_CHECK").as_deref() != Ok("1")
    {
        results.push(json!({"workflow":workflow,"passed":true,"skipped":true,"reason":"Requires macOS graphical session and CLARORA_GUI_MEMORY_CHECK=1"}));
        return Ok(());
    }
    let out = root.join(format!(
        "native-flashcard-keys-{}",
        clarora_gpui::data::id()
    ));
    fs::create_dir_all(&out)?;
    let exe = std::env::current_exe()?
        .parent()
        .context("executable directory")?
        .join("clarora-gpui");
    let mut command = Command::new(exe);
    command
        .arg("--ui-controller-check")
        .arg(&out)
        .env("CLARORA_FLASHCARD_KEYS_CHECK", "1");
    let status = super::native_diagnostic::run(command, &out, Duration::from_secs(45))?;
    let report: Value =
        serde_json::from_slice(&fs::read(out.join("flashcard-shortcut-report.json"))?)?;
    let controller: Value =
        serde_json::from_slice(&fs::read(out.join("ui-controller-report.json"))?)?;
    ensure!(
        status.success()
            && controller["passed"] == true
            && report["passed"] == true
            && report["cases"].as_array().is_some_and(|c| c.len() >= 15),
        "flashcard key diagnostic failed: {}",
        out.display()
    );
    results.push(json!({"workflow":workflow,"passed":true,"report":out.join("flashcard-shortcut-report.json"),"controller":out.join("ui-controller-report.json")}));
    Ok(())
}

/// Dedicated settings acceptance avoids depending on unrelated video seek timing.
pub fn chatgpt(root: &Path, results: &mut Vec<Value>) -> Result<()> {
    let workflow = "ChatGPT会员设置原生布局/登录取消/切页";
    if !cfg!(debug_assertions)
        || !cfg!(target_os = "macos")
        || std::env::var("CLARORA_GUI_MEMORY_CHECK").as_deref() != Ok("1")
    {
        results.push(json!({"workflow":workflow,"passed":true,"skipped":true,"reason":"Requires macOS graphical session and CLARORA_GUI_MEMORY_CHECK=1"}));
        return Ok(());
    }
    let out = root.join(format!(
        "native-chatgpt-settings-{}",
        clarora_gpui::data::id()
    ));
    fs::create_dir_all(&out)?;
    let exe = std::env::current_exe()?
        .parent()
        .context("executable directory")?
        .join("clarora-gpui");
    let mut command = Command::new(exe);
    command
        .arg("--ui-controller-check")
        .arg(&out)
        .env("CLARORA_CHATGPT_SETTINGS_CHECK", "1");
    let status = super::native_diagnostic::run(command, &out, Duration::from_secs(30))?;
    let report: Value = serde_json::from_slice(&fs::read(out.join("ui-controller-report.json"))?)?;
    let layouts: Value =
        serde_json::from_slice(&fs::read(out.join("chatgpt-settings-layouts.json"))?)?;
    ensure!(
        status.success()
            && report["passed"] == true
            && layouts["passed"] == true
            && layouts["layouts"].as_array().is_some_and(|v| v.len() == 42
                && [
                    "pending-registration",
                    "exchange-forbidden",
                    "legacy-keychain"
                ]
                .iter()
                .all(|name| v.iter().any(|r| r["case"] == *name))),
        "ChatGPT settings diagnostic failed: {}",
        out.display()
    );
    results.push(json!({"workflow":workflow,"passed":true,"report":out.join("ui-controller-report.json"),"layouts":out.join("chatgpt-settings-layouts.json")}));
    Ok(())
}

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
        let mut command = Command::new(&executable);
        command
            .arg(flag)
            .arg(&out)
            .arg("--data-dir")
            .arg(out.join("library"));
        let started = Instant::now();
        let status = super::native_diagnostic::run(command, &out, Duration::from_secs(60))?;
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
        let flashcard: Value =
            serde_json::from_slice(&fs::read(out.join("flashcard-shortcut-report.json"))?)?;
        ensure!(
            flashcard["passed"] == true
                && flashcard["cases"].as_array().is_some_and(|c| c.len() >= 15),
            "flashcard keys missing from native UI controller acceptance"
        );
        let chatgpt_layouts: Value =
            serde_json::from_slice(&fs::read(out.join("chatgpt-settings-layouts.json"))?)?;
        ensure!(
            chatgpt_layouts["passed"] == true
                && chatgpt_layouts["layouts"]
                    .as_array()
                    .is_some_and(|v| v.len() == 42
                        && [
                            "pending-registration",
                            "exchange-forbidden",
                            "legacy-keychain"
                        ]
                        .iter()
                        .all(|name| v.iter().any(|r| r["case"] == *name))),
            "ChatGPT settings layouts missing or failed"
        );
        let video_report_path = out.join("video-resume-report.json");
        let video_report: Value = serde_json::from_slice(&fs::read(&video_report_path)?)?;
        ensure!(
            video_report["passed"] == true
                && video_report["cases"].as_array().is_some_and(|cases| {
                    cases.len() >= 8
                        && cases
                            .iter()
                            .any(|case| case["case"] == "全屏助手鼠标选区退格删除且视频保持暂停")
                }),
            "视频切页进度验收未完成：{}",
            video_report_path.display()
        );
        checks.push(json!({"case":name,"passed":true,"report":report_path,"video_resume_report":video_report_path,"seconds":started.elapsed().as_secs_f64()}));
    }
    results.push(json!({"workflow":workflow,"passed":true,"checks":checks}));
    Ok(())
}
