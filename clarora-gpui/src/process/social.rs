use super::{Cancel, DownloadCache, VideoSource, download_cached, download_into, share_link};
use crate::{ai_chat::request, object_storage::check_cancel};
use anyhow::{Context, Result, bail, ensure};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use url::Url;

pub(super) fn source_for_host(host: &str) -> Option<VideoSource> {
    match host {
        "weibo.com" | "www.weibo.com" | "m.weibo.cn" | "video.weibo.com" | "t.cn" => {
            Some(VideoSource::Weibo)
        }
        "x.com" | "www.x.com" | "m.x.com" | "mobile.x.com" | "twitter.com" | "www.twitter.com"
        | "m.twitter.com" | "mobile.twitter.com" | "t.co" => Some(VideoSource::X),
        _ => None,
    }
}

fn short_host(source: VideoSource) -> Option<&'static str> {
    match source {
        VideoSource::Weibo => Some("t.cn"),
        VideoSource::X => Some("t.co"),
        _ => None,
    }
}

pub(super) fn normalize(source: VideoSource, url: &Url) -> Result<String> {
    let host = url.host_str().unwrap_or_default();
    ensure!(
        source_for_host(host) == Some(source)
            && matches!(url.scheme(), "http" | "https")
            && url.username().is_empty()
            && url.password().is_none()
            && url.port().is_none(),
        "请粘贴完整的{}视频作品链接",
        source.label()
    );

    let path: Vec<&str> = url
        .path()
        .trim_end_matches('/')
        .split('/')
        .skip(1)
        .collect();
    if short_host(source) == Some(host) {
        if let [code] = path.as_slice()
            && alphanumeric(code, 64)
        {
            return Ok(format!("https://{host}/{code}"));
        }
        bail!("{}分享短链接无效", source.label());
    }
    match source {
        VideoSource::Weibo => normalize_weibo(host, &path, url),
        VideoSource::X => normalize_x(&path),
        _ => bail!("不支持此视频来源"),
    }
}

fn alphanumeric(value: &str, max_len: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn positive_number(value: &str) -> Option<u64> {
    if value.bytes().all(|byte| byte.is_ascii_digit()) {
        value.parse::<u64>().ok().filter(|&number| number > 0)
    } else {
        None
    }
}

fn normalize_weibo(host: &str, path: &[&str], url: &Url) -> Result<String> {
    let object = match path {
        ["tv", "show", object] if matches!(host, "weibo.com" | "www.weibo.com") => {
            Some((*object).into())
        }
        ["show"] if host == "video.weibo.com" => Some(
            url.query_pairs()
                .find_map(|(key, value)| (key == "fid").then_some(value))
                .context("微博视频链接缺少作品编号")?,
        ),
        _ => None,
    };
    if let Some(object) = object {
        let (namespace, id) = object
            .split_once(':')
            .or_else(|| object.split_once("%3A"))
            .or_else(|| object.split_once("%3a"))
            .filter(|(namespace, id)| {
                positive_number(namespace).is_some()
                    && ((16..=25).contains(&id.len())
                        && id.bytes().all(|byte| byte.is_ascii_digit())
                        || id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit()))
            })
            .context("微博视频编号无效")?;
        return Ok(format!("https://weibo.com/tv/show/{namespace}:{id}"));
    }

    let id = match (host, path) {
        ("m.weibo.cn", ["status" | "detail", id]) => Some(*id),
        ("weibo.com" | "www.weibo.com", ["detail", id]) => Some(*id),
        ("weibo.com" | "www.weibo.com", [account, id] | [account, "detail", id])
            if *account == "0" || positive_number(account).is_some() =>
        {
            Some(*id)
        }
        _ => None,
    }
    .filter(|id| alphanumeric(id, 25))
    .context("请复制包含视频的微博正文或视频分享链接")?;
    Ok(format!("https://weibo.com/0/{id}"))
}

fn normalize_x(path: &[&str]) -> Result<String> {
    let (id, media) = match path {
        [_, "status", id, rest @ ..]
        | ["i", "web", "status", id, rest @ ..]
        | ["statuses", id, rest @ ..] => (*id, rest),
        ["i", "videos", id] | ["i", "videos", "tweet", id] => (*id, &[][..]),
        _ => bail!("请复制包含视频的 X 帖子链接"),
    };
    let id = positive_number(id).context("X 帖子编号无效")?;
    match media {
        [] => Ok(format!("https://x.com/i/web/status/{id}")),
        ["video", index] => {
            let index = positive_number(index).context("X 视频序号无效")?;
            Ok(format!("https://x.com/i/web/status/{id}/video/{index}"))
        }
        _ => bail!("请复制 X 帖子链接或选择帖子中的视频"),
    }
}

fn resolve(source: VideoSource, mut url: Url, cancel: &Cancel) -> Result<String> {
    check_cancel(cancel)?;
    if short_host(source) != url.host_str() {
        return Ok(url.into());
    }

    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(12))
                .redirect(reqwest::redirect::Policy::none())
                .user_agent("curl")
                .build()?;
            for _ in 0..5 {
                let reply = request(cancel, client.get(url.clone()).send())
                    .await
                    .with_context(|| {
                        format!("无法打开{}短链接，请尝试复制作品的完整链接", source.label())
                    })?;
                let location = reply
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .filter(|_| reply.status().is_redirection())
                    .with_context(|| {
                        format!("{}短链接未跳转到作品，请复制完整链接", source.label())
                    })?;
                let redirect = url.join(location.to_str()?)?;
                let canonical = normalize(source, &redirect)
                    .with_context(|| format!("{}短链接未指向本站的视频作品", source.label()))?;
                url = Url::parse(&canonical)?;
                if short_host(source) != url.host_str() {
                    return Ok(canonical);
                }
            }
            bail!("{}短链接跳转次数过多，请复制完整链接", source.label())
        })
}

pub(super) fn download(
    source: VideoSource,
    root: &Path,
    input: &str,
    cancel: &Cancel,
) -> Result<PathBuf> {
    let link = share_link(input.trim()).context("请粘贴要下载的视频链接")?;
    let url = normalize(source, &Url::parse(link)?)?;
    let url = resolve(source, Url::parse(&url)?, cancel)?;
    let (prefix, extractors) = match source {
        VideoSource::Weibo => (".weibo-", "Weibo,WeiboVideo"),
        VideoSource::X => (".x-", "twitter,twitter:amplify"),
        _ => bail!("不支持此视频来源"),
    };
    let options = [
        "--socket-timeout",
        "20",
        "--playlist-items",
        "1",
        "--use-extractors",
        extractors,
    ];
    download_cached(
        root,
        DownloadCache {
            folder: source.cache_folder(),
            stage_prefix: prefix,
            noun: source.label(),
        },
        &url,
        cancel,
        |directory| {
            let result = download_into(directory, &url, None, &options, cancel);
            let result = match result {
                Err(error) if source == VideoSource::X && x_metadata_error(&error) => {
                    check_cancel(cancel)?;
                    let mut fallback = options.to_vec();
                    fallback.extend(["--extractor-args", "twitter:api=syndication"]);
                    download_into(directory, &url, None, &fallback, cancel)
                }
                result => result,
            };
            result.map_err(|error| explain_failure(source, error))
        },
    )
}

fn x_metadata_error(error: &anyhow::Error) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    [
        "failed to parse json",
        "error(s) while querying api",
        "unable to download json metadata",
        "could not retrieve guest token",
    ]
    .iter()
    .any(|reason| message.contains(reason))
}

fn explain_failure(source: VideoSource, error: anyhow::Error) -> anyhow::Error {
    let message = error.to_string().to_ascii_lowercase();
    if [
        "no video could be found",
        "no video formats found",
        "no suitable extractor",
        "media #",
    ]
    .iter()
    .any(|reason| message.contains(reason))
    {
        error.context(format!("这条{}作品没有可下载的视频", source.label()))
    } else if ["http error 401", "http error 403", "login required"]
        .iter()
        .any(|reason| message.contains(reason))
    {
        error.context(format!(
            "{}暂时拒绝访问，请确认作品可公开播放后重试",
            source.label()
        ))
    } else if [
        "unexpected_eof_while_reading",
        "ssl",
        "timed out",
        "syndication",
        "api.x.com",
    ]
    .iter()
    .any(|reason| message.contains(reason))
    {
        error.context(format!(
            "{}的下载接口当前不可达，请检查网络或代理后重试",
            source.label()
        ))
    } else {
        error
    }
}
