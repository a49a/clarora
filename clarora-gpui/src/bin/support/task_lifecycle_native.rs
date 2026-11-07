//! Repeatable native task acceptance, exercising the real AppView and action router.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command, time::Duration};
pub fn run(root: &Path, results: &mut Vec<Value>) -> Result<()> {
    let workflow = "原生任务边界：聊天关联/忙碌保护/切页取消/全局进度/异常恢复/迟到回答";
    if !cfg!(debug_assertions)
        || !cfg!(target_os = "macos")
        || std::env::var("CLARORA_GUI_MEMORY_CHECK").as_deref() != Ok("1")
    {
        results.push(json!({"workflow":workflow,"passed":true,"skipped":true,"reason":"Requires macOS graphical session and CLARORA_GUI_MEMORY_CHECK=1"}));
        return Ok(());
    }
    let out = root.join(format!(
        "native-task-lifecycle-{}",
        clarora_gpui::data::id()
    ));
    let exe = std::env::current_exe()?
        .parent()
        .context("binary directory")?
        .join("clarora-gpui");
    let mut command = Command::new(exe);
    command.arg("--task-lifecycle-check").arg(&out);
    let status = super::native_diagnostic::run(command, &out, Duration::from_secs(35))?;
    let path = out.join("task-lifecycle-report.json");
    let report: Value = serde_json::from_slice(&fs::read(&path)?)?;
    let cases = report["cases"].as_array().context("task cases missing")?;
    let required = [
        "ordinary task explicitly has no chat correlation",
        "ordinary completion preserves unrelated live chat and its output",
        "busy start keeps the active task identity",
        "page leave requests cancellation for material scope",
        "cancelled progress cannot overwrite terminal notice",
        "application scope and global progress survive navigation",
        "worker panic produces failed notice and releases foreground lane",
        "new task can commit and reopen SQLite after panic",
        "stale chat completion cannot paint or finish a newer request",
    ];
    ensure!(
        status.success()
            && report["passed"] == true
            && report["actual_appview_handlers"] == true
            && report["isolated_sqlite"] == true
            && cases.len() == required.len()
            && required.iter().all(|name| cases
                .iter()
                .any(|c| c["case"] == *name && c["passed"] == true)),
        "native task boundary failed: {}",
        path.display()
    );
    results.push(json!({"workflow":workflow,"passed":true,"checks":cases.len(),"report":path}));
    Ok(())
}
