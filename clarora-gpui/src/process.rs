use anyhow::{Context, Result, bail, ensure};
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
    let link = regex::Regex::new(r#"https://[^\s<>「」【】]+"#)?
        .find(text)
        .context("请输入完整 bilibili 视频链接或 BV 号")?
        .as_str()
        .trim_end_matches(['，', '。', '；', '！', '、', '）', ')']);
    let url = url::Url::parse(link)?;
    ensure!(
        [
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
pub fn is_youtube_auth_error(error: &anyhow::Error) -> bool {
    let text = format!("{error:#}");
    text.contains("Sign in to confirm") || text.contains("--cookies-from-browser")
}
pub fn normalize_youtube(input: &str) -> Result<String> {
    let text = input.trim();
    let link = regex::Regex::new(r#"https://[^\s<>「」【】]+"#)?
        .find(text)
        .context("请输入完整 YouTube 视频链接")?
        .as_str()
        .trim_end_matches(['，', '。', '；', '！', '、', '）', ')']);
    let url = url::Url::parse(link)?;
    ensure!(
        [
            Some("www.youtube.com"),
            Some("youtube.com"),
            Some("m.youtube.com"),
            Some("music.youtube.com"),
            Some("youtu.be"),
            Some("www.youtube-nocookie.com"),
        ]
        .contains(&url.host_str())
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
fn download_url(root: &Path, url: &str, cancel: &Cancel) -> Result<PathBuf> {
    download_into(
        &root.join("downloads").join(crate::data::id()),
        url,
        None,
        cancel,
    )
}
fn download_into(
    directory: &Path,
    url: &str,
    browser: Option<YoutubeBrowser>,
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
            "--socket-timeout",
            "30",
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
pub fn download_youtube(root: &Path, input: &str, cancel: &Cancel) -> Result<PathBuf> {
    download_youtube_with(root, input, None, cancel)
}
pub fn download_youtube_with(
    root: &Path,
    input: &str,
    browser: Option<YoutubeBrowser>,
    cancel: &Cancel,
) -> Result<PathBuf> {
    use sha2::{Digest, Sha256};
    let url = normalize_youtube(input)?;
    let directory = root
        .join("downloads")
        .join("remote-youtube")
        .join(hex::encode(Sha256::digest(url.as_bytes())));
    if let Some(path) = fs::read_dir(&directory)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.is_file()
                && path.extension().and_then(|ext| ext.to_str()) == Some("mp4")
                && fs::metadata(path).is_ok_and(|meta| meta.len() > 0)
        })
    {
        return path.canonicalize().map_err(Into::into);
    }
    download_into(&directory, &url, browser, cancel)
}
pub fn download(root: &Path, input: &str, cancel: &Cancel) -> Result<PathBuf> {
    let url = normalize_bilibili(input)?;
    download_url(root, &url, cancel)
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
