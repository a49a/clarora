use anyhow::{Context, Result, bail, ensure};
mod douyin;
mod social;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
pub type Cancel = Arc<AtomicBool>;
pub fn cancel_token() -> Cancel {
    Arc::new(AtomicBool::new(false))
}
pub fn tool(name: &str) -> Result<PathBuf> {
    let binary = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    };
    if crate::runtime::packaged() {
        let parent = crate::runtime::executable_dir().context("无法读取安装目录")?;
        return [parent.join("tools").join(&binary), parent.join(&binary)]
            .into_iter()
            .find(|path| path.is_file())
            .with_context(|| format!("安装包缺少 {binary}，请重新安装完整版本"));
    }
    let mut dirs =
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect::<Vec<_>>();
    dirs.extend([
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
    ]);
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".local/bin"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        dirs.push(PathBuf::from(local).join("Microsoft/WinGet/Links"));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        dirs.insert(0, parent.join("tools"));
        dirs.insert(0, parent.into());
    }
    dirs.into_iter()
        .filter(|d| d.is_absolute())
        .map(|d| d.join(&binary))
        .find(|p| p.is_file())
        .with_context(|| format!("未找到 {binary}，请安装并加入 PATH"))
}
fn drain(mut stream: impl Read) -> Vec<u8> {
    let mut tail = vec![];
    let mut chunk = [0u8; 8192];
    while let Ok(n) = stream.read(&mut chunk) {
        if n == 0 {
            break;
        }
        tail.extend_from_slice(&chunk[..n]);
        if tail.len() > 1048576 {
            tail.drain(..tail.len() - 1048576);
        }
    }
    tail
}
#[cfg(windows)]
struct Job(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
/// Waits run on worker threads; both streams are drained and every nonzero exit fails.
pub fn run(
    executable: &Path,
    args: &[String],
    cancel: &Cancel,
    timeout: Duration,
) -> Result<String> {
    ensure!(!cancel.load(Ordering::SeqCst), "已取消");
    let mut cmd = Command::new(executable);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let mut child = cmd
        .spawn()
        .with_context(|| format!("无法启动 {}", executable.display()))?;
    #[cfg(windows)]
    let _job = unsafe {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::*;
        let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if handle.is_null()
            || SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as _,
                std::mem::size_of_val(&limits) as u32,
            ) == 0
            || AssignProcessToJobObject(handle, child.as_raw_handle() as _) == 0
        {
            let _ = child.kill();
            let _ = child.wait();
            if !handle.is_null() {
                windows_sys::Win32::Foundation::CloseHandle(handle);
            }
            bail!("无法创建下载进程组");
        }
        Job(handle)
    };
    let stdout = child.stdout.take().context("stdout")?;
    let stderr = child.stderr.take().context("stderr")?;
    let out = thread::spawn(move || drain(stdout));
    let err = thread::spawn(move || drain(stderr));
    let start = Instant::now();
    let mut aborted = false;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if cancel.load(Ordering::SeqCst) || start.elapsed() > timeout {
            aborted = true;
            #[cfg(unix)]
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.kill();
            break child.wait()?;
        }
        thread::sleep(Duration::from_millis(40));
    };
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    #[cfg(windows)]
    drop(_job);
    let out = out.join().unwrap_or_default();
    let err = err.join().unwrap_or_default();
    if aborted {
        bail!(if cancel.load(Ordering::SeqCst) {
            "已取消"
        } else {
            "任务超时，已停止"
        })
    }
    ensure!(
        status.success(),
        "{} 执行失败：{}",
        executable.file_name().unwrap_or_default().to_string_lossy(),
        String::from_utf8_lossy(&err[err.len().saturating_sub(4000)..])
    );
    String::from_utf8(out).context("命令输出不是 UTF-8")
}
pub fn normalize_bilibili(input: &str) -> Result<String> {
    let text = input.trim();
    let id = regex::Regex::new(r"^(BV[0-9A-Za-z]{10}|av[1-9][0-9]*)$")?;
    if id.is_match(text) {
        return Ok(format!("https://www.bilibili.com/video/{text}"));
    }
    let link = share_link(text).context("请输入完整 bilibili 视频链接或 BV 号")?;
    let url = url::Url::parse(link)?;
    ensure!(
        url.scheme() == "https"
            && [
                Some("www.bilibili.com"),
                Some("m.bilibili.com"),
                Some("bilibili.com")
            ]
            .contains(&url.host_str())
            && url.username().is_empty()
            && url.password().is_none()
            && url.port().is_none(),
        "仅支持 bilibili 完整视频链接，短链接请先在浏览器展开"
    );
    let video = url
        .path()
        .trim_end_matches('/')
        .strip_prefix("/video/")
        .context("请输入普通视频链接")?;
    ensure!(id.is_match(video), "BV/av 编号无效");
    let pages = url
        .query_pairs()
        .filter(|(k, _)| k == "p")
        .map(|(_, v)| v.into_owned())
        .collect::<Vec<_>>();
    ensure!(pages.len() <= 1, "重复的分 P 参数");
    let suffix = if let Some(p) = pages.first() {
        ensure!(
            regex::Regex::new(r"^[1-9][0-9]{0,4}$")?.is_match(p),
            "分 P 参数无效"
        );
        format!("?p={p}")
    } else {
        String::new()
    };
    Ok(format!("https://www.bilibili.com/video/{video}{suffix}"))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoSource {
    Douyin,
    Weibo,
    X,
    YouTube,
}

pub enum VideoLink {
    Bilibili(String),
    External(VideoSource, String),
}

pub fn normalize_video_link(input: &str) -> Result<VideoLink> {
    let text = input.trim();
    let url = share_link(text).map(url::Url::parse).transpose()?;
    if let Some(url) = &url
        && let Some(source) = url.host_str().and_then(social::source_for_host)
    {
        return social::normalize(source, url).map(|url| VideoLink::External(source, url));
    }
    match url.as_ref().and_then(url::Url::host_str) {
        Some(host) if youtube_host(host) => {
            normalize_youtube(text).map(|url| VideoLink::External(VideoSource::YouTube, url))
        }
        Some(host) if douyin_host(host) => {
            normalize_douyin(text).map(|url| VideoLink::External(VideoSource::Douyin, url))
        }
        Some("www.bilibili.com" | "m.bilibili.com" | "bilibili.com") => {
            normalize_bilibili(text).map(VideoLink::Bilibili)
        }
        None if douyin_id(text) => {
            normalize_douyin(text).map(|url| VideoLink::External(VideoSource::Douyin, url))
        }
        None if text.starts_with("BV") || text.starts_with("av") => {
            normalize_bilibili(text).map(VideoLink::Bilibili)
        }
        _ => bail!("请粘贴 B 站、YouTube、抖音、微博或 X 的视频链接，或输入有效的 BV / av 编号"),
    }
}

fn share_link(text: &str) -> Option<&str> {
    static LINKS: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r#"https?://[^\s<>"「」『』【】，。；！？、）]+"#)
            .expect("valid video link pattern")
    });
    LINKS
        .find(text)
        .map(|link| link.as_str().trim_end_matches([')', ',', '.']))
}

fn youtube_host(host: &str) -> bool {
    matches!(
        host,
        "www.youtube.com"
            | "youtube.com"
            | "m.youtube.com"
            | "music.youtube.com"
            | "youtu.be"
            | "www.youtube-nocookie.com"
    )
}

fn douyin_host(host: &str) -> bool {
    matches!(
        host,
        "www.douyin.com"
            | "douyin.com"
            | "m.douyin.com"
            | "v.douyin.com"
            | "www.iesdouyin.com"
            | "iesdouyin.com"
    )
}

fn douyin_id(value: &str) -> bool {
    (15..=25).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_digit())
}

impl VideoSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Douyin => "抖音",
            Self::Weibo => "微博",
            Self::X => "X",
            Self::YouTube => "YouTube",
        }
    }

    fn cache_folder(self) -> &'static str {
        match self {
            Self::Douyin => "remote-douyin",
            Self::Weibo => "remote-weibo",
            Self::X => "remote-x",
            Self::YouTube => "remote-youtube",
        }
    }

    pub fn from_cache_folder(folder: &str) -> Option<Self> {
        [Self::Douyin, Self::Weibo, Self::X, Self::YouTube]
            .into_iter()
            .find(|source| source.cache_folder() == folder)
    }
}
/// Browser cookie source for YouTube imports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YoutubeBrowser {
    Brave,
    Chrome,
    Edge,
    Firefox,
    Safari,
    Vivaldi,
}
impl YoutubeBrowser {
    pub fn id(self) -> &'static str {
        match self {
            Self::Brave => "brave",
            Self::Chrome => "chrome",
            Self::Edge => "edge",
            Self::Firefox => "firefox",
            Self::Safari => "safari",
            Self::Vivaldi => "vivaldi",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Brave => "Brave",
            Self::Chrome => "Chrome",
            Self::Edge => "Edge",
            Self::Firefox => "Firefox",
            Self::Safari => "Safari",
            Self::Vivaldi => "Vivaldi",
        }
    }
    pub fn from_id(id: &str) -> Option<Self> {
        let browser = [
            Self::Brave,
            Self::Chrome,
            Self::Edge,
            Self::Firefox,
            Self::Safari,
            Self::Vivaldi,
        ]
        .into_iter()
        .find(|browser| browser.id() == id)?;
        YOUTUBE_BROWSERS.contains(&browser).then_some(browser)
    }
}
#[cfg(target_os = "macos")]
pub const YOUTUBE_BROWSERS: &[YoutubeBrowser] = &[
    YoutubeBrowser::Chrome,
    YoutubeBrowser::Edge,
    YoutubeBrowser::Firefox,
    YoutubeBrowser::Safari,
    YoutubeBrowser::Brave,
    YoutubeBrowser::Vivaldi,
];
#[cfg(target_os = "windows")]
pub const YOUTUBE_BROWSERS: &[YoutubeBrowser] = &[
    YoutubeBrowser::Chrome,
    YoutubeBrowser::Edge,
    YoutubeBrowser::Firefox,
    YoutubeBrowser::Brave,
    YoutubeBrowser::Vivaldi,
];
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub const YOUTUBE_BROWSERS: &[YoutubeBrowser] = &[YoutubeBrowser::Chrome, YoutubeBrowser::Firefox];
pub fn is_video_auth_error(error: &anyhow::Error) -> bool {
    let text = format!("{error:#}");
    text.contains("Sign in to confirm") || text.contains("--cookies-from-browser")
}
pub fn normalize_youtube(input: &str) -> Result<String> {
    let link = share_link(input.trim()).context("请输入完整 YouTube 视频链接")?;
    let url = url::Url::parse(link)?;
    ensure!(
        url.scheme() == "https"
            && url.host_str().is_some_and(youtube_host)
            && url.username().is_empty()
            && url.password().is_none()
            && url.port().is_none(),
        "仅支持 YouTube 单视频链接"
    );
    let path = url.path().trim_end_matches('/');
    let id = if url.host_str() == Some("youtu.be") {
        path.trim_start_matches('/')
            .split('/')
            .next()
            .map(str::to_owned)
    } else if path == "/watch" {
        url.query_pairs()
            .find(|(key, _)| key == "v")
            .map(|(_, value)| value.into_owned())
    } else {
        path.strip_prefix("/shorts/")
            .or_else(|| path.strip_prefix("/live/"))
            .or_else(|| path.strip_prefix("/embed/"))
            .and_then(|value| value.split('/').next())
            .map(str::to_owned)
    }
    .context("YouTube 链接缺少视频 ID")?;
    ensure!(
        regex::Regex::new(r"^[A-Za-z0-9_-]{11}$")?.is_match(&id),
        "YouTube 视频 ID 无效"
    );
    Ok(format!("https://www.youtube.com/watch?v={id}"))
}
pub fn normalize_douyin(input: &str) -> Result<String> {
    let text = input.trim();
    if douyin_id(text) {
        return Ok(format!("https://www.douyin.com/video/{text}"));
    }
    let link = share_link(text).context("请输入完整抖音视频链接")?;
    let url = url::Url::parse(link)?;
    ensure!(
        url.scheme() == "https"
            && url.host_str().is_some_and(douyin_host)
            && url.username().is_empty()
            && url.password().is_none()
            && url.port().is_none(),
        "仅支持抖音视频链接"
    );
    let host = url.host_str().unwrap_or_default();
    let path = url.path().trim_end_matches('/');
    if host == "v.douyin.com" {
        let code = path.trim_start_matches('/');
        ensure!(
            !code.is_empty()
                && code.len() <= 64
                && code
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'),
            "抖音短链接无效"
        );
        return Ok(format!("https://v.douyin.com/{code}/"));
    }
    ensure!(
        !path.starts_with("/note/") && !path.starts_with("/share/note/"),
        "暂不支持抖音图文作品，请分享视频链接"
    );
    let id = if let Some(value) = path
        .strip_prefix("/video/")
        .or_else(|| path.strip_prefix("/share/video/"))
    {
        value.split('/').next().map(str::to_owned)
    } else {
        url.query_pairs()
            .find(|(key, _)| key == "modal_id")
            .map(|(_, value)| value.into_owned())
    }
    .context("抖音链接缺少视频 ID")?;
    ensure!(douyin_id(&id), "抖音视频 ID 无效");
    Ok(format!("https://www.douyin.com/video/{id}"))
}
fn download_into(
    directory: &Path,
    url: &str,
    browser: Option<YoutubeBrowser>,
    options: &[&str],
    cancel: &Cancel,
) -> Result<PathBuf> {
    let yt = tool("yt-dlp")?;
    let ffmpeg = tool("ffmpeg")?;
    fs::create_dir_all(directory)?;
    let result = (|| {
        let mut args = [
            "--ignore-config",
            "--no-playlist",
            "--no-simulate",
            "--no-progress",
            "--no-color",
            "--encoding",
            "utf-8",
            "--retries",
            "2",
            "--fragment-retries",
            "2",
            "--format",
            "bv*[vcodec^=avc1]+ba[acodec^=mp4a]/b[ext=mp4]",
            "--merge-output-format",
            "mp4",
            "--remux-video",
            "mp4",
            "--write-subs",
            "--sub-langs",
            "en.*,zh.*,ai-en.*,ai-zh.*",
            "--embed-subs",
            "--windows-filenames",
            "--print",
            "after_move:%(filepath)j",
            "--output",
            "%(title).100B [%(id)s].%(ext)s",
        ]
        .map(String::from)
        .to_vec();
        if let Some(browser) = browser {
            args.extend(["--cookies-from-browser".into(), browser.id().into()]);
        }
        if let Ok(deno) = tool("deno") {
            args.extend([
                "--js-runtimes".into(),
                format!("deno:{}", deno.to_string_lossy()),
            ]);
        }
        args.extend(options.iter().map(|arg| (*arg).to_owned()));
        args.extend([
            "--ffmpeg-location".into(),
            ffmpeg
                .parent()
                .context("ffmpeg路径")?
                .to_string_lossy()
                .into_owned(),
            "--paths".into(),
            directory.to_string_lossy().into_owned(),
            "--".into(),
            url.to_owned(),
        ]);
        let output = run(&yt, &args, cancel, Duration::from_secs(1800))?;
        ensure!(!cancel.load(Ordering::SeqCst), "已取消");
        let file = output
            .lines()
            .filter_map(|line| serde_json::from_str::<String>(line).ok())
            .next_back()
            .context("下载器未返回完整视频")?;
        let path = PathBuf::from(file).canonicalize()?;
        ensure!(
            path.is_file()
                && path.starts_with(directory.canonicalize()?)
                && path.extension().and_then(|s| s.to_str()) == Some("mp4")
                && fs::metadata(&path)?.len() > 0,
            "下载结果无效"
        );
        Ok(path)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(directory);
    }
    result
}
/// A subtitle file written next to a downloaded video by yt-dlp. Prefer an
/// English track, then Chinese, then any remaining SRT/VTT track.
fn subtitle_language_tag(tag: &str) -> bool {
    let lower = tag.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "orig" | "default" | "forced" | "sdh" | "cc" | "und"
    ) {
        return true;
    }
    if let Some(rest) = lower.strip_prefix("ai-") {
        return subtitle_language_tag(rest);
    }
    let parts = lower.split('-').collect::<Vec<_>>();
    !parts.is_empty()
        && parts.len() <= 3
        && parts[0].len() >= 2
        && parts[0].len() <= 3
        && parts[0].bytes().all(|byte| byte.is_ascii_alphabetic())
        && parts[1..].iter().all(|part| {
            (1..=8).contains(&part.len()) && part.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
}

const MEDIA_EXTENSIONS: [&str; 15] = [
    "mp4", "mkv", "webm", "mov", "m4v", "avi", "flv", "ts", "m2ts", "wmv", "mpg", "mpeg", "3gp",
    "ogv", "rmvb",
];

fn sibling_media(directory: &std::path::Path, stem: &str) -> bool {
    let stem = stem.to_ascii_lowercase();
    fs::read_dir(directory)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_file())
        .any(|entry| {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            name.rsplit_once('.')
                .is_some_and(|(base, ext)| base == stem && MEDIA_EXTENSIONS.contains(&ext))
        })
}

fn subtitle_tag(video_stem: &str, file_name: &str) -> Option<String> {
    let lower = file_name.to_ascii_lowercase();
    let stem = video_stem.to_ascii_lowercase();
    let extension = lower.rsplit('.').next()?;
    if !matches!(extension, "srt" | "vtt") {
        return None;
    }
    let base = lower.strip_suffix(&format!(".{extension}"))?;
    if base == stem {
        return Some(String::new());
    }
    let tag = base.strip_prefix(&format!("{stem}."))?;
    (!tag.contains('.') && subtitle_language_tag(tag)).then(|| tag.to_string())
}

fn language_priority(tag: &str) -> u8 {
    let tag = tag.strip_prefix("ai-").unwrap_or(tag);
    let primary = tag.split('-').next().unwrap_or_default();
    if tag.is_empty() {
        0
    } else if primary == "en" {
        1
    } else if primary == "zh" {
        2
    } else {
        3
    }
}

pub fn downloaded_subtitle(video: &Path) -> Option<PathBuf> {
    let directory = video.parent()?;
    let stem = video.file_stem()?.to_str()?;
    let mut tracks = fs::read_dir(directory)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter_map(|path| {
            let name = path.file_name()?.to_str()?;
            let tag = subtitle_tag(stem, name)?;
            // A file like Lesson.en.srt belongs to a sibling media file with
            // the same stem, whichever supported container it uses.
            if !tag.is_empty()
                && let Some(candidate) = path.file_stem()
                && sibling_media(directory, &candidate.to_string_lossy())
            {
                return None;
            }
            Some((path, language_priority(&tag)))
        })
        .collect::<Vec<_>>();
    tracks.sort_by_key(|(_, priority)| *priority);
    tracks.into_iter().map(|(path, _)| path).next()
}

pub fn download_youtube(root: &Path, input: &str, cancel: &Cancel) -> Result<PathBuf> {
    download_youtube_with(root, input, None, cancel)
}
pub fn download_youtube_with(
    root: &Path,
    input: &str,
    browser: Option<YoutubeBrowser>,
    cancel: &Cancel,
) -> Result<PathBuf> {
    let url = normalize_youtube(input)?;
    download_cached(
        root,
        DownloadCache {
            folder: VideoSource::YouTube.cache_folder(),
            stage_prefix: ".youtube-",
            noun: "YouTube",
        },
        &url,
        cancel,
        |directory| {
            if cfg!(windows) && crate::runtime::packaged() {
                ensure!(
                    tool("deno").is_ok(),
                    "YouTube 下载需要外部 JavaScript runtime；安装包缺少 tools/deno.exe"
                );
            }
            download_into(
                directory,
                &url,
                browser,
                &["--socket-timeout", "30"],
                cancel,
            )
        },
    )
}
pub fn download_douyin(root: &Path, input: &str, cancel: &Cancel) -> Result<PathBuf> {
    let url = douyin::resolve(&normalize_douyin(input)?, cancel)?;
    download_cached(
        root,
        DownloadCache {
            folder: VideoSource::Douyin.cache_folder(),
            stage_prefix: ".douyin-",
            noun: "抖音",
        },
        &url,
        cancel,
        |directory| douyin::download(directory, &url, cancel),
    )
}
pub fn download_external_with(
    source: VideoSource,
    root: &Path,
    input: &str,
    browser: Option<YoutubeBrowser>,
    cancel: &Cancel,
) -> Result<PathBuf> {
    match source {
        VideoSource::YouTube => download_youtube_with(root, input, browser, cancel),
        VideoSource::Douyin => download_douyin(root, input, cancel),
        VideoSource::Weibo | VideoSource::X => social::download(source, root, input, cancel),
    }
}
struct DownloadCache<'a> {
    folder: &'a str,
    stage_prefix: &'a str,
    noun: &'a str,
}
fn download_cached(
    root: &Path,
    cache: DownloadCache<'_>,
    url: &str,
    cancel: &Cancel,
    fetch: impl FnOnce(&Path) -> Result<PathBuf>,
) -> Result<PathBuf> {
    let DownloadCache {
        folder,
        stage_prefix,
        noun,
    } = cache;
    use sha2::{Digest, Sha256};
    ensure!(!cancel.load(Ordering::SeqCst), "已取消");
    let cache = root.join("downloads").join(folder);
    let key = hex::encode(Sha256::digest(url.as_bytes()));
    let directory = cache.join(format!("v2-{key}"));
    if let Some(path) = published_video(&directory).or_else(|| legacy_video(&cache.join(key))) {
        return path.canonicalize().map_err(Into::into);
    }
    fs::create_dir_all(&cache)?;
    let stage = tempfile::Builder::new()
        .prefix(stage_prefix)
        .tempdir_in(&cache)?;
    let path = fetch(stage.path())?;
    ensure!(!cancel.load(Ordering::SeqCst), "已取消");
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("视频文件名无效")?;
    fs::write(stage.path().join("completed"), name)?;
    // Coordinate publication and repair across app instances, after all network work.
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(cache.join(".publish.lock"))?;
    loop {
        ensure!(!cancel.load(Ordering::SeqCst), "已取消");
        match fs2::FileExt::try_lock_exclusive(&lock) {
            Ok(()) => break,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) => {
                return Err(error).with_context(|| format!("无法锁定{noun}视频缓存"));
            }
        }
    }
    if let Some(path) = published_video(&directory) {
        return path.canonicalize().map_err(Into::into);
    }
    // A stale cache can remain after the user removes its media file.
    let retired = if directory.try_exists()? {
        let retired = tempfile::tempdir_in(&cache)?;
        fs::rename(&directory, retired.path().join("cache"))?;
        Some(retired)
    } else {
        None
    };
    // A directory rename publishes the completed video and its subtitles together.
    if let Err(error) = fs::rename(stage.path(), &directory) {
        if let Some(retired) = retired {
            let _ = fs::rename(retired.path().join("cache"), &directory);
        }
        return Err(error).with_context(|| format!("无法保存{noun}视频缓存"));
    }
    drop(lock);
    drop(retired);
    directory.join(name).canonicalize().map_err(Into::into)
}

fn published_video(directory: &Path) -> Option<PathBuf> {
    let name = fs::read_to_string(directory.join("completed")).ok()?;
    let file = Path::new(&name);
    if file.file_name()?.to_str()? != name {
        return None;
    }
    let path = directory.join(file);
    cached_video_file(&path).then_some(path)
}

fn legacy_video(directory: &Path) -> Option<PathBuf> {
    fs::read_dir(directory)
        .into_iter()
        .flatten()
        .flatten()
        .find_map(|entry| {
            let path = entry.path();
            let stem = path.file_stem()?.to_str()?;
            // yt-dlp's intermediate video has a format suffix after the video ID.
            (stem.ends_with(']') && cached_video_file(&path)).then_some(path)
        })
}

fn cached_video_file(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()) == Some("mp4")
        && fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.len() > 0)
}

fn interrupted_video_download(error: &anyhow::Error) -> bool {
    error.to_string().split(['\r', '\n']).any(|line| {
        let line = line.to_ascii_lowercase();
        if line.contains("error: content too short (expected ") {
            return true;
        }
        (line.contains("[download] got error:") || line.contains("unable to download video data:"))
            && [
                "unexpected_eof_while_reading",
                "eof occurred in violation of protocol",
                "timed out",
                "connection reset",
                "remote end closed connection",
                "incompleteread",
                " bytes read",
                " bytes, expected ",
            ]
            .iter()
            .any(|reason| line.contains(reason))
    })
}

pub fn download(root: &Path, input: &str, cancel: &Cancel) -> Result<PathBuf> {
    let url = normalize_bilibili(input)?;
    let download = |options: &[&str]| {
        download_into(
            &root.join("downloads").join(crate::data::id()),
            &url,
            None,
            options,
            cancel,
        )
    };
    let mut options = vec![
        "--socket-timeout",
        "15",
        "--retry-sleep",
        "http:linear=1::1",
        "--retry-sleep",
        "fragment:linear=1::1",
    ];
    match download(&options) {
        Err(error) if interrupted_video_download(&error) => {
            ensure!(!cancel.load(Ordering::SeqCst), "已取消");
            options.extend(["--format-sort", "res:720"]);
            download(&options).context("B 站视频流连接中断，切换画质后仍未完成下载")
        }
        result => result,
    }
}
pub fn clip(
    source: &Path,
    destination: &Path,
    start: f64,
    end: f64,
    cancel: &Cancel,
) -> Result<()> {
    ensure!(
        start.is_finite() && end.is_finite() && start >= 0. && end > start,
        "请先设置有效 A/B 范围"
    );
    let args = [
        "-nostdin".into(),
        "-v".into(),
        "error".into(),
        "-y".into(),
        "-ss".into(),
        start.to_string(),
        "-i".into(),
        source.to_string_lossy().into(),
        "-t".into(),
        (end - start).to_string(),
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "veryfast".into(),
        "-c:a".into(),
        "aac".into(),
        destination.to_string_lossy().into(),
    ];
    if let Err(e) = run(&tool("ffmpeg")?, &args, cancel, Duration::from_secs(600)) {
        let _ = fs::remove_file(destination);
        return Err(e);
    }
    ensure!(fs::metadata(destination)?.len() > 0, "导出片段为空");
    Ok(())
}
