#[cfg(unix)]
use anyhow::Context;
use anyhow::{Result, ensure};
use clarora_gpui::{data::Library, process};
use serde_json::{Value, json};
#[cfg(unix)]
use std::fs;

const CANONICAL: &str = "https://www.youtube.com/watch?v=jNQXAC9IVRw";

pub fn run(lib: &Library, results: &mut Vec<Value>) -> Result<()> {
    let text = "分享 https://youtu.be/jNQXAC9IVRw?t=42";
    ensure!(
        process::normalize_youtube(text)? == CANONICAL,
        "短链接规范化失败"
    );
    for input in [
        "https://www.youtube.com/watch?v=jNQXAC9IVRw",
        "https://m.youtube.com/watch?list=RD123&v=jNQXAC9IVRw",
        "https://music.youtube.com/watch?v=jNQXAC9IVRw&list=RD123",
        "https://www.youtube.com/shorts/jNQXAC9IVRw/",
        "https://www.youtube.com/live/jNQXAC9IVRw?si=abc",
        "https://www.youtube.com/embed/jNQXAC9IVRw",
        "https://www.youtube-nocookie.com/embed/jNQXAC9IVRw",
        "https://www.youtube.com/watch/?v=jNQXAC9IVRw#t=10",
        "https://youtu.be/jNQXAC9IVRw，",
    ] {
        ensure!(
            process::normalize_youtube(input)? == CANONICAL,
            "YouTube 链接规范化失败：{input}"
        );
    }
    for input in [
        "https://www.youtube.com/playlist?list=PL123",
        "https://www.youtube.com/watch?v=short",
        "https://www.youtube.com/watch?list=PL123",
        "https://www.youtube.com/channel/UC123",
        "https://www.youtube.com.evil.test/watch?v=jNQXAC9IVRw",
        "http://www.youtube.com/watch?v=jNQXAC9IVRw",
        "https://youtu.be/",
        "https://www.bilibili.com/video/BV1xx411c7mD",
    ] {
        ensure!(
            process::normalize_youtube(input).is_err(),
            "无效 YouTube 输入未拒绝：{input}"
        );
    }
    ensure!(
        process::normalize_bilibili("https://youtu.be/jNQXAC9IVRw").is_err(),
        "bilibili 校验接受了 YouTube 链接"
    );
    let auth = anyhow::anyhow!(
        "ERROR: [youtube] fixture: Sign in to confirm you’re not a bot. Use --cookies-from-browser or --cookies"
    );
    ensure!(
        process::is_youtube_auth_error(&auth),
        "未识别 YouTube 登录验证错误"
    );
    ensure!(
        !process::is_youtube_auth_error(&anyhow::anyhow!(
            "ERROR: [youtube] fixture: Private video"
        )),
        "普通下载失败被误判为登录验证"
    );
    ensure!(
        process::YoutubeBrowser::from_id("chrome") == Some(process::YoutubeBrowser::Chrome)
            && process::YoutubeBrowser::from_id("bogus").is_none(),
        "浏览器标识解析错误"
    );

    #[cfg(unix)]
    fixture_download(lib)?;

    results.push(json!({
        "workflow":"YouTube短链/Shorts/Live/Embed/no-cookie/播放列表参数规范化、伪造域名与裸播放列表拒绝、登录验证错误识别与浏览器Cookie重试、共享yt-dlp下载结果导入与失败清理",
        "passed":true
    }));
    Ok(())
}

#[cfg(unix)]
fn fixture_download(lib: &Library) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let tools = std::env::current_exe()?
        .parent()
        .context("E2E 可执行文件缺少目录")?
        .join("tools");
    let tools_existed = tools.exists();
    fs::create_dir_all(&tools)?;
    let yt = tools.join("yt-dlp");
    let marker = tools.join(".clarora-yt-dlp-fixture");
    ensure!(
        !yt.exists(),
        "测试工具目录已有 yt-dlp，拒绝覆盖：{}",
        yt.display()
    );
    let script = r#"#!/bin/sh
set -eu
marker="$(dirname "$0")/.clarora-yt-dlp-fixture"
[ -f "$marker" ] || exit 3
dir=""
prev=""
for arg in "$@"; do
    if [ "$prev" = "--paths" ]; then
        dir="$arg"
    fi
    prev="$arg"
done
[ -n "$dir" ] || exit 4
case "$*" in
    *AUTHFIXTURE*)
        echo "ERROR: [youtube] AUTHFIXTURE: Sign in to confirm you’re not a bot. Use --cookies-from-browser or --cookies for the authentication." >&2
        exit 7 ;;
    *COOKIEFIXTU*)
        case "$*" in
            *"--cookies-from-browser chrome"*) ;;
            *) echo "missing --cookies-from-browser chrome" >&2; exit 6 ;;
        esac ;;
    *FAILFIXTURE*) exit 5 ;;
esac
file="$dir/Example video [jNQXAC9IVRw].mp4"
printf 'fixture-video-bytes' > "$file"
printf '"%s"\n' "$file"
"#;
    let outcome = (|| -> Result<()> {
        fs::write(&yt, script)?;
        fs::set_permissions(&yt, fs::Permissions::from_mode(0o755))?;
        fs::write(&marker, b"fixture")?;
        let downloads = lib.root.join("downloads");
        let before = fs::read_dir(&downloads)
            .map(|entries| entries.count())
            .unwrap_or(0);
        let path = process::download_youtube(
            &lib.root,
            "https://youtu.be/jNQXAC9IVRw",
            &process::cancel_token(),
        )?;
        ensure!(
            path.starts_with(&downloads)
                && path.file_name().and_then(|name| name.to_str())
                    == Some("Example video [jNQXAC9IVRw].mp4"),
            "下载结果路径错误：{}",
            path.display()
        );
        ensure!(
            fs::read(&path)? == b"fixture-video-bytes",
            "下载结果内容错误"
        );
        let mut videos = clarora_gpui::video_library::Collection::load(lib)?;
        let ids = videos.add(std::slice::from_ref(&path), None)?;
        ensure!(
            videos.select(&ids[0])?.title == "Example video",
            "YouTube 收藏标题错误"
        );
        let after = fs::read_dir(&downloads)?.count();
        ensure!(after == before + 1, "下载目录未新增");
        let cache = downloads.join("remote-youtube");
        let cached = fs::read_dir(&cache)?.count();
        ensure!(cached == 1, "YouTube 缓存目录数量错误");
        let auth_error = process::download_youtube(
            &lib.root,
            "https://youtu.be/AUTHFIXTURE",
            &process::cancel_token(),
        )
        .unwrap_err();
        ensure!(
            process::is_youtube_auth_error(&auth_error),
            "未识别 YouTube 登录验证错误"
        );
        ensure!(
            fs::read_dir(&cache)?.count() == cached,
            "登录验证失败未清理临时目录"
        );
        let cookie_path = process::download_youtube_with(
            &lib.root,
            "https://youtu.be/COOKIEFIXTU",
            Some(process::YoutubeBrowser::Chrome),
            &process::cancel_token(),
        )?;
        ensure!(
            cookie_path.is_file() && cookie_path.starts_with(downloads.join("remote-youtube")),
            "浏览器 Cookie 重试结果错误"
        );
        ensure!(
            fs::read_dir(&cache)?.count() == cached + 1,
            "Cookie 重试未使用独立缓存"
        );
        ensure!(
            process::YoutubeBrowser::from_id("chrome") == Some(process::YoutubeBrowser::Chrome)
                && process::YoutubeBrowser::from_id("bogus").is_none(),
            "浏览器标识解析错误"
        );
        ensure!(
            path.starts_with(downloads.join("remote-youtube")),
            "YouTube 缓存目录错误"
        );
        fs::remove_file(&marker)?;
        ensure!(
            process::download_youtube(
                &lib.root,
                "https://www.youtube.com/watch?v=jNQXAC9IVRw",
                &process::cancel_token(),
            )? == path,
            "重复导入未复用已下载视频"
        );
        fs::write(&marker, b"fixture")?;
        let failed = process::download_youtube(
            &lib.root,
            "https://youtu.be/FAILFIXTURE",
            &process::cancel_token(),
        )
        .unwrap_err();
        ensure!(
            !process::is_youtube_auth_error(&failed),
            "普通下载失败被误判为登录验证"
        );
        ensure!(
            fs::read_dir(&downloads)?.count() == after,
            "失败下载未清理临时目录"
        );
        Ok(())
    })();
    let _ = fs::remove_file(&marker);
    let _ = fs::remove_file(&yt);
    if !tools_existed {
        let _ = fs::remove_dir(&tools);
    }
    outcome
}
