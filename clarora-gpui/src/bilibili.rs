//! Bilibili multipart metadata and a persisted, file-bound playback queue.
use crate::{
    data::Library,
    process::{self, Cancel},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::atomic::Ordering,
    time::Duration,
};

/// A restored source has a stable local cache slot, without a fake video file.
pub(crate) fn source_path(lib: &Library, url: &str) -> Result<PathBuf> {
    use sha2::{Digest, Sha256};
    ensure!(process::normalize_bilibili(url)? == url, "视频来源链接无效");
    Ok(lib
        .root
        .join("downloads")
        .join("remote-bilibili")
        .join(hex::encode(Sha256::digest(url.as_bytes())))
        .join("video.mp4"))
}
pub(crate) fn source_identity(path: &Path) -> Option<String> {
    if path.file_name()?.to_str()? != "video.mp4" {
        return None;
    }
    let parent = path.parent()?;
    let key = parent.file_name()?.to_str()?;
    if key.len() != 64
        || !key.bytes().all(|b| b.is_ascii_hexdigit())
        || parent.parent()?.file_name()?.to_str()? != "remote-bilibili"
        || parent.parent()?.parent()?.file_name()?.to_str()? != "downloads"
    {
        return None;
    }
    Some(key.to_string())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Part {
    pub number: u32,
    pub title: String,
    pub duration: f64,
    pub path: Option<PathBuf>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Series {
    #[serde(default)]
    pub title: String,
    pub base: String,
    pub parts: Vec<Part>,
    pub requested: usize,
    pub current: Option<usize>,
}
impl Series {
    pub fn url(&self, index: usize) -> Result<String> {
        let part = self.parts.get(index).context("分P不存在")?;
        Ok(format!("{}?p={}", self.base, part.number))
    }
    pub fn cached(&self, index: usize) -> Option<PathBuf> {
        self.parts
            .get(index)?
            .path
            .clone()
            .filter(|p| p.is_file() && std::fs::metadata(p).is_ok_and(|m| m.len() > 0))
    }
    pub fn next_for(&self, path: &Path) -> Option<usize> {
        let current = self.current?;
        if self.parts.get(current)?.path.as_deref() != Some(path) {
            return None;
        }
        (current + 1 < self.parts.len()).then_some(current + 1)
    }
    pub fn save(&self, lib: &Library) -> Result<()> {
        lib.set_setting("bilibili_series", &serde_json::to_string(self)?)
    }
    pub fn load(lib: &Library) -> Result<Option<Self>> {
        let value = lib.setting("bilibili_series")?;
        if value.is_empty() {
            return Ok(None);
        }
        let series: Self = serde_json::from_str(&value)?;
        ensure!(
            process::normalize_bilibili(&series.base)? == series.base,
            "分P来源无效"
        );
        ensure!(
            !series.parts.is_empty() && series.parts.len() <= 10000,
            "分P列表无效"
        );
        for (i, part) in series.parts.iter().enumerate() {
            ensure!(part.number as usize == i + 1, "分P顺序无效");
        }
        ensure!(
            series.requested < series.parts.len()
                && series.current.is_none_or(|i| i < series.parts.len()),
            "分P索引无效"
        );
        Ok(Some(series))
    }
}
pub fn fetch(input: &str, cancel: &Cancel) -> Result<Series> {
    fetch_from(
        input,
        "https://api.bilibili.com/x/web-interface/view",
        cancel,
    )
}
/// Endpoint injection is used by the native HTTP workflow E2E, never by app settings.
pub fn fetch_from(input: &str, endpoint: &str, cancel: &Cancel) -> Result<Series> {
    ensure!(!cancel.load(Ordering::SeqCst), "已取消");
    let normalized = process::normalize_bilibili(input)?;
    let mut url = url::Url::parse(&normalized)?;
    let requested = url
        .query_pairs()
        .find(|(k, _)| k == "p")
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(1);
    url.set_query(None);
    let base = url.to_string();
    let id = url
        .path()
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .context("缺少视频ID")?;
    let query = if let Some(aid) = id.strip_prefix("av") {
        ("aid", aid)
    } else {
        ("bvid", id)
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let value: Value = runtime.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("Mozilla/5.0 Clarora-GPUI")
            .build()?;
        let request = async {
            let response = client
                .get(endpoint)
                .header("Referer", "https://www.bilibili.com/")
                .query(&[query])
                .send()
                .await?
                .error_for_status()?;
            ensure!(
                response.content_length().unwrap_or(0) <= 8 * 1024 * 1024,
                "分P响应过大"
            );
            let bytes = response.bytes().await?;
            ensure!(bytes.len() <= 8 * 1024 * 1024, "分P响应过大");
            Ok::<Value, anyhow::Error>(serde_json::from_slice(&bytes)?)
        };
        tokio::pin!(request);
        loop {
            if cancel.load(Ordering::SeqCst) {
                anyhow::bail!("已取消");
            }
            tokio::select! {
                result=&mut request=>return result,
                _=tokio::time::sleep(Duration::from_millis(50))=>{}
            }
        }
    })?;
    ensure!(!cancel.load(Ordering::SeqCst), "已取消");
    ensure!(
        value["code"].as_i64() == Some(0),
        "读取分P失败：{}",
        value["message"].as_str().unwrap_or("来源不可用")
    );
    let title = value["data"]["title"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .context("视频缺少标题")?
        .to_owned();
    let pages = value["data"]["pages"]
        .as_array()
        .context("分P返回格式无效")?;
    ensure!(
        !pages.is_empty() && pages.len() <= 10000,
        "分P列表为空或过大"
    );
    let mut parts = vec![];
    for (i, page) in pages.iter().enumerate() {
        ensure!(
            page["page"].as_u64() == Some((i + 1) as u64),
            "分P编号重复或不连续"
        );
        let title = page["part"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .context("分P缺少标题")?
            .to_owned();
        parts.push(Part {
            number: (i + 1) as u32,
            title,
            duration: page["duration"].as_f64().unwrap_or(0.),
            path: None,
        });
    }
    ensure!(requested > 0 && requested <= parts.len(), "指定的分P不存在");
    Ok(Series {
        title,
        base,
        parts,
        requested: requested - 1,
        current: None,
    })
}
