use super::{Cancel, normalize_douyin};
use crate::{ai_chat::request, object_storage::check_cancel};
use anyhow::{Context, Result, bail, ensure};
use reqwest::{Client, Response, Url, header};
use serde_json::Value;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

const REFERER: &str = "https://open.douyin.com/";
const DETAIL: &str = "https://www.douyin.com/aweme/v1/web/aweme/detail/";
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";

pub(super) fn resolve(link: &str, cancel: &Cancel) -> Result<String> {
    check_cancel(cancel)?;
    if !link.starts_with("https://v.douyin.com/") {
        return Ok(link.to_owned());
    }
    runtime()?.block_on(async {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let mut url = Url::parse(link)?;
        for _ in 0..5 {
            let reply = request(cancel, client.get(url.clone()).send()).await?;
            ensure!(
                reply.status().is_redirection(),
                "此抖音分享链接无法打开，请重新复制视频链接"
            );
            let location = reply
                .headers()
                .get(header::LOCATION)
                .context("抖音短链接缺少跳转地址")?;
            let redirected = url.join(location.to_str()?)?;
            let normalized = normalize_douyin(redirected.as_str())?;
            if normalized.starts_with("https://www.douyin.com/video/") {
                return Ok(normalized);
            }
            url = Url::parse(&normalized)?;
        }
        bail!("抖音短链接跳转次数过多，请重新复制视频链接")
    })
}

pub(super) fn download(directory: &Path, link: &str, cancel: &Cancel) -> Result<PathBuf> {
    check_cancel(cancel)?;
    let id = link.rsplit('/').next().context("抖音链接缺少视频编号")?;
    runtime()?.block_on(async {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(12))
            .read_timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() < 5 && allowed_media_url(attempt.url()) {
                    attempt.follow()
                } else {
                    attempt.stop()
                }
            }))
            .build()?;
        download_from(&client, id, directory, cancel).await
    })
}

fn runtime() -> Result<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?)
}

fn allowed_media_url(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.host_str().is_some_and(|host| {
            host.ends_with(".douyinvod.com")
                || host.ends_with(".zjcdn.com")
                || (host == "www.douyin.com" && url.path() == "/aweme/v1/play/")
        })
}

async fn download_from(
    client: &Client,
    id: &str,
    directory: &Path,
    cancel: &Cancel,
) -> Result<PathBuf> {
    // The embedded Douyin player uses the web detail endpoint with aid=6383.
    let reply = request(
        cancel,
        client
            .get(DETAIL)
            .header(header::REFERER, REFERER)
            .header(header::ORIGIN, REFERER.trim_end_matches('/'))
            .query(&[("aweme_id", id), ("aid", "6383")])
            .timeout(Duration::from_secs(20))
            .send(),
    )
    .await
    .context("无法读取抖音视频")?;
    if matches!(reply.status().as_u16(), 401 | 403) {
        bail!("抖音暂时拒绝下载请求。请在抖音打开作品，确认可以保存后再试");
    }
    let mut reply = reply.error_for_status()?;
    let mut payload = Vec::new();
    while let Some(chunk) = request(cancel, reply.chunk()).await? {
        ensure!(
            payload.len() + chunk.len() <= 4 * 1024 * 1024,
            "抖音作品信息超出大小限制"
        );
        payload.extend_from_slice(&chunk);
    }
    if payload.iter().all(u8::is_ascii_whitespace) {
        bail!("抖音暂未提供下载信息，请稍后重试");
    }
    let detail: Value = serde_json::from_slice(&payload).context("抖音返回了无法读取的作品信息")?;
    let video = &detail["aweme_detail"];
    ensure!(
        video["aweme_id"].as_str() == Some(id),
        "抖音视频不可用，可能已删除、未公开或链接已失效"
    );
    ensure!(
        video["video_control"]["allow_download"].as_bool() != Some(false),
        "这条抖音视频未开放下载"
    );
    ensure!(
        video["images"].as_array().is_none_or(Vec::is_empty),
        "暂不支持抖音图文作品，请分享视频链接"
    );
    let mut title = String::new();
    for ch in video["desc"].as_str().unwrap_or_default().trim().chars() {
        if title.len() + ch.len_utf8() > 100 {
            break;
        }
        title.push(if ch.is_control() || "<>:\"/\\|?*".contains(ch) {
            ' '
        } else {
            ch
        });
    }
    let title = title.trim().trim_start_matches('.').trim();
    let title = if title.is_empty() {
        "抖音视频"
    } else {
        title
    };
    let target = directory.join(format!("{title} [{id}].mp4"));
    let address = &video["video"]["download_addr"];
    let expected_size = address["data_size"].as_u64().filter(|size| *size > 0);
    let urls = address["url_list"]
        .as_array()
        .context("这条抖音作品没有可用的视频下载地址")?;
    let mut failure = anyhow::anyhow!("抖音未返回可用的视频下载地址");
    for url in urls
        .iter()
        .filter_map(Value::as_str)
        .filter_map(|s| Url::parse(s).ok())
        .filter(allowed_media_url)
    {
        match download_media(client, url, &target, expected_size, cancel).await {
            Ok(()) => return Ok(target),
            Err(error) => failure = error,
        }
        check_cancel(cancel)?;
    }
    Err(failure).context("抖音视频下载失败")
}

async fn download_media(
    client: &Client,
    url: Url,
    path: &Path,
    expected_size: Option<u64>,
    cancel: &Cancel,
) -> Result<()> {
    let reply = request(
        cancel,
        client
            .get(url)
            .header(header::REFERER, REFERER)
            .timeout(Duration::from_secs(1800))
            .send(),
    )
    .await;
    check_cancel(cancel)?;
    let reply = reply.map_err(|_| anyhow::anyhow!("无法连接抖音视频服务器，请重试"))?;
    save_video(reply, path, expected_size, cancel).await
}

async fn save_video(
    mut reply: Response,
    path: &Path,
    expected_size: Option<u64>,
    cancel: &Cancel,
) -> Result<()> {
    ensure!(
        reply.status() == reqwest::StatusCode::OK,
        "抖音视频服务器返回 HTTP {}",
        reply.status().as_u16()
    );
    let length = reply.content_length().or(expected_size);
    let mut file = fs::File::create(path)?;
    let mut header = Vec::with_capacity(12);
    let mut bytes = 0;
    loop {
        let chunk = request(cancel, reply.chunk()).await;
        check_cancel(cancel)?;
        let Some(chunk) = chunk.map_err(|_| anyhow::anyhow!("抖音视频传输中断，请重试"))?
        else {
            break;
        };
        if header.len() < 12 {
            header.extend_from_slice(&chunk[..chunk.len().min(12 - header.len())]);
            if header.len() == 12 {
                ensure!(&header[4..8] == b"ftyp", "抖音服务器返回的内容不是视频文件");
            }
        }
        file.write_all(&chunk)?;
        bytes += chunk.len() as u64;
    }
    ensure!(
        bytes >= 12 && length.is_none_or(|length| length == bytes),
        "抖音视频下载不完整"
    );
    file.sync_all()?;
    check_cancel(cancel)
}
