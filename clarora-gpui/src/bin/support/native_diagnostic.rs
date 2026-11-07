//! Every native child is reaped, has independent logs, and must publish a completed lifecycle.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};
struct Running(Option<Child>);
impl Drop for Running {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
pub fn run(mut command: Command, out: &Path, timeout: Duration) -> Result<ExitStatus> {
    fs::create_dir_all(out)?;
    let started = Instant::now();
    // Selector variables belong to one child, even when the parent environment
    // was used to run a focused scenario earlier.
    for key in [
        "CLARORA_FLASHCARD_KEYS_CHECK",
        "CLARORA_CHATGPT_SETTINGS_CHECK",
        "CLARORA_DIAGNOSTIC_FAIL_AFTER_POLLS",
    ] {
        if !command.get_envs().any(|(k, _)| k == key) {
            command.env_remove(key);
        }
    }

    let invocation = json!({"executable":command.get_program().to_string_lossy(),"args":command.get_args().map(|v|v.to_string_lossy().into_owned()).collect::<Vec<_>>(),"timeout_seconds":timeout.as_secs_f64()});
    fs::write(
        out.join("native-invocation.json"),
        serde_json::to_vec_pretty(&invocation)?,
    )?;
    let mut pid = None;
    let mut reaped = false;
    let result = (|| -> Result<ExitStatus> {
        command
            .stdin(Stdio::null())
            .stdout(fs::File::create(out.join("stdout.log"))?)
            .stderr(fs::File::create(out.join("stderr.log"))?);
        let mut running = Running(Some(command.spawn()?));
        pid = running.0.as_ref().map(Child::id);
        let status = loop {
            if let Some(status) = running.0.as_mut().unwrap().try_wait()? {
                running.0 = None;
                reaped = true;
                break status;
            }
            if started.elapsed() >= timeout {
                let mut child = running.0.take().unwrap();
                let _ = child.kill();
                reaped = child.wait().is_ok();
                anyhow::bail!("原生诊断超时：{}", out.display());
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let path = out.join("diagnostic-run.json");
        let lifecycle: Value = serde_json::from_slice(
            &fs::read(&path)
                .with_context(|| format!("缺少诊断生命周期报告：{}", path.display()))?,
        )?;
        ensure!(
            status.success() && lifecycle["passed"] == true && lifecycle["cleanup"] == true,
            "原生诊断失败：{}",
            path.display()
        );
        Ok(status)
    })();
    fs::write(
        out.join("native-process.json"),
        serde_json::to_vec_pretty(
            &json!({"passed":result.is_ok(),"pid":pid,"child_reaped":reaped,"seconds":started.elapsed().as_secs_f64(),"error":result.as_ref().err().map(|e|format!("{e:#}")),"invocation":invocation}),
        )?,
    )?;
    result
}
