//! Exercise the real executables' registration and startup boundary without opening user data.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn invoke(exe: &Path, args: &[String], out: &Path) -> Result<std::process::ExitStatus> {
    fs::create_dir_all(out)?;
    let started = Instant::now();
    let mut child = Command::new(exe)
        .args(args)
        .stdin(Stdio::null())
        .stdout(fs::File::create(out.join("stdout.log"))?)
        .stderr(fs::File::create(out.join("stderr.log"))?)
        .spawn()?;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() > Duration::from_secs(8) {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("diagnostic startup timed out: {}", out.display());
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    fs::write(
        out.join("invocation.json"),
        serde_json::to_vec_pretty(&json!({
            "executable":exe,"args":args,"success":status.success(),"seconds":started.elapsed().as_secs_f64()
        }))?,
    )?;
    Ok(status)
}

pub fn run(root: &Path, results: &mut Vec<Value>) -> Result<()> {
    let out = root.join(format!("diagnostic-entry-{}", clarora_gpui::data::id()));
    fs::create_dir_all(&out)?;
    let exe = std::env::current_exe()?;
    let app = exe
        .parent()
        .context("binary directory")?
        .join(if cfg!(windows) {
            "clarora-gpui.exe"
        } else {
            "clarora-gpui"
        });
    let mut cases = vec![];
    let result = (|| -> Result<()> {
        let catalog = out.join("native-catalog.json");
        let sentinel = out.join("never-open-user-data");
        let status = invoke(
            &app,
            &[
                "--list-diagnostics".into(),
                catalog.display().to_string(),
                "--data-dir".into(),
                sentinel.display().to_string(),
            ],
            &out.join("list-native"),
        )?;
        ensure!(
            status.success() && !sentinel.exists(),
            "listing native diagnostics opened a library"
        );
        let registry: Value = serde_json::from_slice(&fs::read(&catalog)?)?;
        let registered = registry["diagnostics"]
            .as_array()
            .context("diagnostic catalogue missing")?;
        let flags = [
            "--task-lifecycle-check",
            "--assistant-latency-check",
            "--audio-remove-check",
            "--audio-repeat-check",
            "--audio-boundary-check",
            "--playback-session-check",
            "--model-navigation-check",
            "--backup-progress-check",
            "--sentence-search-check",
            "--speaking-groups-check",
            "--subtitle-workbench-check",
            #[cfg(target_os = "macos")]
            "--writing-paste-check",
            "--focus-mode-check",
            "--ui-controller-check",
            "--ui-controller-quit-check",
        ];
        ensure!(
            registered.len() == flags.len()
                && flags
                    .iter()
                    .all(|flag| registered.iter().any(|v| v["flag"] == *flag)),
            "an existing native diagnostic or alias disappeared"
        );
        ensure!(
            registered.iter().all(|v| v["window"]["show"] == false
                && v["window"]["focus"] == false
                && v["window"]["activate"] == false),
            "diagnostic window policy is inconsistent"
        );
        cases.push(json!({"case":"all native entries and hidden window policies are registered","passed":true,"entries":registered.len()}));

        for (name, args) in [
            ("missing-output", vec!["--audio-repeat-check".into()]),
            (
                "flag-as-output",
                vec!["--audio-repeat-check".into(), "--focus-mode-check".into()],
            ),
            (
                "two-probes",
                vec![
                    "--audio-repeat-check".into(),
                    out.join("a").display().to_string(),
                    "--focus-mode-check".into(),
                    out.join("b").display().to_string(),
                ],
            ),
            (
                "duplicate-probe",
                vec![
                    "--audio-repeat-check".into(),
                    out.join("a").display().to_string(),
                    "--audio-repeat-check".into(),
                    out.join("a").display().to_string(),
                ],
            ),
            (
                "runtime-conflict",
                vec![
                    "--audio-repeat-check".into(),
                    out.join("a").display().to_string(),
                    "--runtime-check".into(),
                    out.join("runtime.json").display().to_string(),
                ],
            ),
            ("unknown-native-option", vec!["--audio-repeet-check".into()]),
        ] {
            let sentinel = out.join(format!("{name}-data"));
            let mut args = args;
            args.extend(["--data-dir".into(), sentinel.display().to_string()]);
            ensure!(
                !invoke(&app, &args, &out.join(name))?.success() && !sentinel.exists(),
                "invalid native arguments reached library startup: {name}"
            );
            cases.push(json!({"case":name,"passed":true,"user_data_opened":false}));
        }
        ensure!(
            !out.join("a").exists() && !out.join("b").exists(),
            "conflicting flags prepared fixtures before validation"
        );
        let reused = out.join("reused");
        fs::create_dir_all(reused.join("library"))?;
        fs::write(
            reused.join("library/marker.txt"),
            "preserve prior failed fixture",
        )?;
        let status = invoke(
            &app,
            &["--audio-repeat-check".into(), reused.display().to_string()],
            &out.join("reused-run"),
        )?;
        ensure!(
            !status.success()
                && fs::read_to_string(reused.join("library/marker.txt"))?
                    == "preserve prior failed fixture"
                && !reused.join("library/clarora.db").exists(),
            "reused diagnostic fixture was opened or overwritten"
        );
        cases.push(json!({"case":"reused fixture is rejected without modification","passed":true}));

        let listed = out.join("e2e-list");
        ensure!(
            invoke(
                &exe,
                &[listed.display().to_string(), "--list-scenarios".into()],
                &out.join("list-e2e")
            )?
            .success(),
            "E2E catalogue failed"
        );
        let scenarios: Value =
            serde_json::from_slice(&fs::read(listed.join("scenario-catalog.json"))?)?;
        ensure!(
            [
                "--asr-models-worker",
                "--focus-timer-worker",
                "--runtime-stability-worker",
                "--gpu-memory-worker"
            ]
            .iter()
            .all(|flag| scenarios["workers"]
                .as_array()
                .is_some_and(|workers| workers.iter().any(|w| w == flag))),
            "internal workers disappeared from catalogue"
        );
        let registered = scenarios["scenarios"]
            .as_array()
            .context("scenario catalogue missing")?;
        let old_flags = [
            "--video-batch-translation-only",
            "--ui-controller-only",
            "--flashcard-keys-only",
            "--subtitle-workbench-only",
            "--assistant-only",
            "--network-proxy-only",
            "--chat-controller-only",
            "--data-loader-only",
            "--focus-timer-only",
            "--focus-mode-only",
            "--gpu-memory-only",
            "--runtime-stability-only",
            "--http-only",
            "--model-navigation-only",
            "--asr-models-only",
            "--reliability-only",
            "--storage-config-only",
            "--storage-errors-only",
            "--backup-progress-only",
            "--cloud-v3-only",
            "--cloud-storage-only",
            "--backup-background-only",
            "--writing-only",
            "--ebook-only",
            "--transcript-only",
            "--state-only",
            "--playback-session-only",
            "--playback-session-native-only",
            "--video-repeat-only",
            "--backup-library-only",
            "--video-render-only",
            "--command-import-only",
            "--audio-merge-only",
            "--audio-repeat-only",
            "--audio-boundary-only",
            "--assistant-latency-only",
            "--chatgpt-only",
            "--audio-remove-only",
            "--sentence-search-only",
            "--audio-background-only",
            "--audio-batch-only",
            "--clip-marks-only",
            "--subtitle-selection-only",
            "--timer-chimes-only",
            "--timer-chimes-audible",
        ];
        ensure!(
            old_flags
                .iter()
                .all(|flag| registered.iter().any(|v| v["flag"] == *flag)),
            "an existing E2E entry disappeared"
        );
        cases.push(json!({"case":"all existing E2E modes remain registered","passed":true,"original_modes":old_flags.len()}));
        for (name, flags) in [
            ("unknown-e2e", vec!["--definitely-invalid-mode"]),
            ("missing-required-media", vec!["--http-only"]),
        ] {
            let dir = out.join(name);
            let mut args = vec![dir.display().to_string()];
            args.extend(flags.into_iter().map(str::to_owned));
            ensure!(
                !invoke(&exe, &args, &out.join(format!("{name}-run")))?.success(),
                "invalid E2E arguments unexpectedly succeeded"
            );
            let report: Value = serde_json::from_slice(&fs::read(dir.join("e2e-report.json"))?)?;
            ensure!(
                report["passed"] == false
                    && report["workflows"].as_array().is_some_and(Vec::is_empty),
                "invalid E2E mode fell back to a full run"
            );
            cases.push(json!({"case":name,"passed":true}));
        }
        Ok(())
    })();
    let report = out.join("diagnostic-entry-report.json");
    fs::write(
        &report,
        serde_json::to_vec_pretty(
            &json!({"passed":result.is_ok(),"cases":cases,"error":result.as_ref().err().map(|e|format!("{e:#}")),"real_subprocesses":true,"no_user_data":true}),
        )?,
    )?;
    result?;
    results.push(json!({"workflow":"诊断登记/参数冲突/隔离资料目录/无效E2E不运行全套","passed":true,"checks":cases.len(),"report":report}));
    Ok(())
}

/// An actual failed AppView must close cleanly before another isolated scenario starts.
pub fn native(root: &Path, results: &mut Vec<Value>) -> Result<()> {
    let workflow = "诊断生命周期：失败清理后运行独立场景";
    if !cfg!(debug_assertions)
        || !cfg!(target_os = "macos")
        || std::env::var("CLARORA_GUI_MEMORY_CHECK").as_deref() != Ok("1")
    {
        results.push(json!({"workflow":workflow,"passed":true,"skipped":true,"reason":"Requires macOS graphical session and CLARORA_GUI_MEMORY_CHECK=1"}));
        return Ok(());
    }
    let out = root.join(format!("diagnostic-lifecycle-{}", clarora_gpui::data::id()));
    fs::create_dir_all(&out)?;
    let app = std::env::current_exe()?
        .parent()
        .context("binary directory")?
        .join("clarora-gpui");
    let failed = out.join("failed");
    let mut command = Command::new(&app);
    command
        .arg("--audio-repeat-check")
        .arg(&failed)
        .env("CLARORA_DIAGNOSTIC_FAIL_AFTER_POLLS", "2");
    ensure!(
        super::native_diagnostic::run(command, &failed, Duration::from_secs(30)).is_err(),
        "injected failure was accepted as success"
    );
    let report: Value = serde_json::from_slice(&fs::read(failed.join("diagnostic-run.json"))?)?;
    ensure!(
        report["passed"] == false
            && report["cleanup"] == true
            && report["polls"] == 2
            && report["error"]
                .as_str()
                .is_some_and(|s| s.contains("injected diagnostic failure")),
        "failed probe did not record lifecycle cleanup"
    );
    let timeout = out.join("timeout");
    let mut command = Command::new(&app);
    command.arg("--playback-session-check").arg(&timeout);
    ensure!(
        super::native_diagnostic::run(command, &timeout, Duration::from_millis(1)).is_err(),
        "bounded child did not time out"
    );
    let timed_out: Value = serde_json::from_slice(&fs::read(timeout.join("native-process.json"))?)?;
    ensure!(
        timed_out["passed"] == false && timed_out["child_reaped"] == true,
        "timed out child was not reaped"
    );
    let passed = out.join("next");
    let mut command = Command::new(&app);
    command.arg("--playback-session-check").arg(&passed);
    super::native_diagnostic::run(command, &passed, Duration::from_secs(45))?;
    let probe: Value = serde_json::from_slice(&fs::read(
        passed.join("playback-session-native-report.json"),
    )?)?;
    ensure!(
        probe["passed"] == true && probe["cases"].as_array().is_some_and(|c| c.len() == 22),
        "next isolated AppView scenario did not complete"
    );
    fs::write(
        out.join("diagnostic-lifecycle-report.json"),
        serde_json::to_vec_pretty(
            &json!({"passed":true,"expected_failure":failed,"next_success":passed,"same_parent_environment":true,"scenarios_use_distinct_libraries":true}),
        )?,
    )?;
    results.push(json!({"workflow":workflow,"passed":true,"report":out.join("diagnostic-lifecycle-report.json")}));
    Ok(())
}
