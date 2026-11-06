//! Real child processes prove local services stay local even with proxy variables.
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
    let out = root.join(format!("proxy-loopback-{}", clarora_gpui::data::id()));
    fs::create_dir_all(&out)?;
    let mut modes = vec!["--assistant-only", "--model-navigation-only"];
    if cfg!(debug_assertions) {
        modes.push("--chatgpt-only");
    }
    let mut cases = vec![];
    for mode in modes {
        let dir = out.join(mode.trim_start_matches('-'));
        fs::create_dir_all(&dir)?;
        let mut command = Command::new(std::env::current_exe()?);
        command
            .arg(&dir)
            .arg(mode)
            .env("CLARORA_GUI_MEMORY_CHECK", "0");
        for name in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            command.env(name, "http://127.0.0.1:1");
        }
        command.env("NO_PROXY", "").env("no_proxy", "");
        let mut child = command
            .stdin(Stdio::null())
            .stdout(fs::File::create(dir.join("stdout.log"))?)
            .stderr(fs::File::create(dir.join("stderr.log"))?)
            .spawn()?;
        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if started.elapsed() > Duration::from_secs(45) {
                let _ = child.kill();
                let _ = child.wait();
                anyhow::bail!("loopback proxy E2E timeout: {}", dir.display());
            }
            thread::sleep(Duration::from_millis(20));
        };
        let path = dir.join("e2e-report.json");
        let report: Value =
            serde_json::from_slice(&fs::read(&path).context("missing local proxy E2E report")?)?;
        ensure!(
            status.success() && report["passed"] == true,
            "local services were sent to the proxy: {}",
            path.display()
        );
        cases.push(json!({"mode":mode,"passed":true,"report":path,"seconds":started.elapsed().as_secs_f64()}));
    }
    let path = out.join("proxy-loopback-report.json");
    fs::write(
        &path,
        serde_json::to_vec_pretty(
            &json!({"passed":true,"cases":cases,"proxy":"unreachable loopback port","no_proxy_env":"empty","changed_system_settings":false}),
        )?,
    )?;
    results.push(json!({"workflow":"启用代理后本机AI/OAuth回调/模型下载保持直连","passed":true,"report":path}));
    Ok(())
}
