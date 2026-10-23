//! Runtime discovery and diagnostics shared by installed apps and package smoke checks.
use crate::{documents, media::Player, process};
use anyhow::{Context, Result};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub fn executable_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(Path::to_path_buf)
}

/// A release manifest makes discovery strict: an incomplete install cannot silently
/// borrow a developer's Homebrew libraries or tools from PATH.
pub fn packaged() -> bool {
    executable_dir().is_some_and(|dir| {
        dir.join("runtime-manifest.json").is_file()
            || (cfg!(target_os = "macos")
                && dir.parent().is_some_and(|contents| {
                    contents.join("Resources/runtime-manifest.json").is_file()
                }))
    })
}

pub fn library_candidates(names: &[&str]) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(dir) = executable_dir() {
        for name in names {
            paths.push(dir.join(name));
        }
    }
    if !packaged() {
        for dir in ["/opt/homebrew/lib", "/usr/local/lib"] {
            for name in names {
                paths.push(Path::new(dir).join(name));
            }
        }
        paths.extend(names.iter().map(PathBuf::from));
    }
    paths
}

#[derive(Clone, Debug, Serialize)]
pub struct Component {
    pub name: String,
    pub path: Option<PathBuf>,
    pub passed: bool,
    pub detail: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Report {
    pub passed: bool,
    pub packaged: bool,
    pub components: Vec<Component>,
}

/// A cheap inventory for the settings snapshot; execution probes are CLI-only.
pub fn inspect() -> Report {
    let mut report = Report {
        packaged: packaged(),
        ..Default::default()
    };
    for name in ["ffmpeg", "ffprobe", "yt-dlp", "clarora-asr", "clarora-tts"] {
        let result = process::tool(name);
        report.components.push(Component {
            name: name.into(),
            path: result.as_ref().ok().cloned(),
            passed: result.is_ok(),
            detail: result.err().map(|e| e.to_string()).unwrap_or_default(),
        });
    }
    for (name, candidates) in [
        (
            "libmpv",
            if cfg!(windows) {
                vec!["mpv-2.dll", "libmpv-2.dll"]
            } else {
                vec!["libmpv.2.dylib", "libmpv.dylib"]
            },
        ),
        (
            "PDFium",
            if cfg!(windows) {
                vec!["pdfium.dll"]
            } else {
                vec!["libpdfium.dylib"]
            },
        ),
    ] {
        let mut path = library_candidates(&candidates)
            .into_iter()
            .find(|path| path.is_file());
        if name == "PDFium" && !packaged() {
            if let Some(dir) = std::env::var_os("PDFIUM_LIB_PATH") {
                path = Some(PathBuf::from(dir).join(candidates[0]))
                    .filter(|p| p.is_file())
                    .or(path);
            }
        }
        report.components.push(Component {
            name: name.into(),
            passed: path.is_some(),
            path,
            detail: if packaged() {
                "检查安装包内运行库"
            } else {
                "检查本机运行库"
            }
            .into(),
        });
    }
    report.passed = report.components.iter().all(|component| component.passed);
    report
}

/// Runs before GPUI initialization and never opens the user's data directory.
pub fn check(output: &Path) -> Result<Report> {
    let mut report = inspect();
    for component in &mut report.components {
        if !component.passed {
            continue;
        }
        let result: Result<String> = match component.name.as_str() {
            "libmpv" => {
                let player = Player::without_audio_device();
                let started = Instant::now();
                loop {
                    let state = player.snapshot();
                    if !state.error.is_empty() {
                        break Err(anyhow::anyhow!(state.error));
                    }
                    if state.initialized {
                        break Ok("libmpv 初始化及软件渲染接口正常".into());
                    }
                    if started.elapsed() > Duration::from_secs(5) {
                        break Err(anyhow::anyhow!("libmpv 初始化超时"));
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            "PDFium" => documents::check_pdf_runtime().map(|_| "PDFium 绑定正常".into()),
            "clarora-asr" | "clarora-tts" => {
                // Both helpers intentionally return 2 and a usage message without arguments.
                let result = std::process::Command::new(component.path.as_ref().unwrap()).output();
                result.context("无法启动原生语音助手").and_then(|result| {
                    anyhow::ensure!(
                        result.status.code() == Some(2),
                        "原生助手未正常进入用法分支：{}",
                        result.status
                    );
                    Ok(String::from_utf8_lossy(&result.stderr)
                        .trim()
                        .chars()
                        .take(512)
                        .collect())
                })
            }
            name => process::run(
                component.path.as_ref().unwrap(),
                &[if name == "yt-dlp" {
                    "--version"
                } else {
                    "-version"
                }
                .into()],
                &process::cancel_token(),
                Duration::from_secs(15),
            )
            .map(|text| text.lines().next().unwrap_or_default().into()),
        };
        match result {
            Ok(detail) => component.detail = detail,
            Err(error) => {
                component.passed = false;
                component.detail = format!("{error:#}");
            }
        }
    }
    report.passed = report.components.iter().all(|component| component.passed);
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    fs::write(output, serde_json::to_vec_pretty(&report)?)?;
    Ok(report)
}
