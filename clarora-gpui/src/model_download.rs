//! Stream model files with cancellable HTTP and throttled per-file progress.
use crate::process::Cancel;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::Path,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
#[derive(Clone)]
pub struct FileSpec {
    pub name: String,
    pub remote: String,
    pub min_bytes: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Progress {
    pub model: String,
    pub file: String,
    pub file_index: usize,
    pub file_count: usize,
    pub source: String,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub bytes_per_second: f64,
    pub phase: String,
}
impl Progress {
    pub fn percent(&self) -> Option<f32> {
        self.total.filter(|t| *t > 0).map(|total| {
            let percent = (self.downloaded as f64 / total as f64 * 100.).clamp(0., 100.);
            if self.phase == "文件完成" {
                100.
            } else {
                percent.min(99.9) as f32
            }
        })
    }
    pub fn label(&self) -> String {
        let amount = if let Some(total) = self.total {
            format!(
                "{:.1} / {:.1} MiB",
                self.downloaded as f64 / 1048576.,
                total as f64 / 1048576.
            )
        } else {
            format!(
                "已下载 {:.1} MiB · 总大小未知",
                self.downloaded as f64 / 1048576.
            )
        };
        format!(
            "{} · {} ({}/{}) · {} · {}{} · {:.2} MiB/s",
            self.model,
            self.file,
            self.file_index,
            self.file_count,
            self.phase,
            amount,
            self.percent()
                .map(|p| format!(" · {}%", p.floor() as u32))
                .unwrap_or_default(),
            self.bytes_per_second / 1048576.
        )
    }
}
pub fn download(
    model: &str,
    directory: &Path,
    files: &[FileSpec],
    hosts: &[&str],
    cancel: &Cancel,
    notify: &mut impl FnMut(Progress),
) -> Result<()> {
    ensure!(!files.is_empty() && !hosts.is_empty(), "下载来源为空");
    ensure!(!cancel.load(Ordering::SeqCst), "已取消");
    fs::create_dir_all(directory)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3600))
            .connect_timeout(Duration::from_secs(20))
            .build()?;
        for (index, spec) in files.iter().enumerate() {
            ensure!(
                Path::new(&spec.name).file_name().and_then(|p| p.to_str())
                    == Some(spec.name.as_str()),
                "模型文件名无效"
            );
            let path = directory.join(&spec.name);
            let temp = path.with_extension("part");
            let mut last_error = None;
            for (attempt, host) in hosts.iter().enumerate() {
                let mut progress = Progress {
                    model: model.into(),
                    file: spec.name.clone(),
                    file_index: index + 1,
                    file_count: files.len(),
                    source: (*host).into(),
                    downloaded: 0,
                    total: None,
                    bytes_per_second: 0.,
                    phase: "连接中".into(),
                };
                notify(progress.clone());
                let transfer = async {
                    let mut response = client
                        .get(format!(
                            "{}/{}/{}",
                            host.trim_end_matches('/'),
                            spec.remote.trim_matches('/'),
                            spec.name
                        ))
                        .send()
                        .await?
                        .error_for_status()?;
                    progress.total = response.content_length().filter(|n| *n > 0);
                    progress.phase = "下载中".into();
                    notify(progress.clone());
                    let mut file = fs::File::create(&temp)?;
                    let started = Instant::now();
                    let mut last_emit = Instant::now();
                    while let Some(chunk) = response.chunk().await? {
                        ensure!(!cancel.load(Ordering::SeqCst), "已取消");
                        file.write_all(&chunk)?;
                        progress.downloaded += chunk.len() as u64;
                        progress.bytes_per_second =
                            progress.downloaded as f64 / started.elapsed().as_secs_f64().max(0.001);
                        if last_emit.elapsed() >= Duration::from_millis(100) {
                            notify(progress.clone());
                            last_emit = Instant::now();
                        }
                    }
                    progress.phase = "校验中".into();
                    notify(progress.clone());
                    ensure!(progress.downloaded > spec.min_bytes, "模型下载不完整");
                    ensure!(
                        progress.total.is_none_or(|n| n == progress.downloaded),
                        "模型长度不匹配"
                    );
                    file.sync_all()?;
                    drop(file);
                    ensure!(!cancel.load(Ordering::SeqCst), "已取消");
                    fs::rename(&temp, &path)?;
                    progress.phase = "文件完成".into();
                    notify(progress.clone());
                    Ok::<(), anyhow::Error>(())
                };
                let result = {
                    tokio::pin!(transfer);
                    loop {
                        if cancel.load(Ordering::SeqCst) {
                            break Err(anyhow::anyhow!("已取消"));
                        }
                        tokio::select! {
                            result=&mut transfer=>break result,
                            _=tokio::time::sleep(Duration::from_millis(50))=>{}
                        }
                    }
                };
                if result.is_ok() {
                    last_error = None;
                    break;
                }
                let _ = fs::remove_file(&temp);
                ensure!(!cancel.load(Ordering::SeqCst), "已取消");
                last_error = result.err();
                if attempt + 1 < hosts.len() {
                    progress.phase = "换源重试".into();
                    notify(progress.clone());
                }
            }
            if let Some(error) = last_error {
                return Err(error.context(format!("{} 下载失败", spec.name)));
            }
        }
        Ok(())
    })
}
