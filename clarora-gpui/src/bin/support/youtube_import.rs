#[cfg(unix)]
use anyhow::Context;
use anyhow::{Result, ensure};
use clarora_gpui::{data::Library, process};
use serde_json::{Value, json};
#[cfg(unix)]
use std::fs;

const CANONICAL: &str = "https://www.youtube.com/watch?v=jNQXAC9IVRw";
const DOUYIN: &str = "https://www.douyin.com/video/7607104129969849061";

pub fn run(lib: &Library, results: &mut Vec<Value>) -> Result<()> {
    social_links()?;
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
    ensure!(
        process::normalize_douyin(
            "7.43 复制打开抖音，看看【示例】 https://v.douyin.com/f-QpBB_oD9Y/ 复制此链接"
        )? == "https://v.douyin.com/f-QpBB_oD9Y/",
        "抖音分享文案规范化失败"
    );
    ensure!(
        matches!(process::normalize_video_link(DOUYIN)?, process::VideoLink::External(process::VideoSource::Douyin, link) if link == DOUYIN)
            && matches!(process::normalize_video_link(CANONICAL)?, process::VideoLink::External(process::VideoSource::YouTube, link) if link == CANONICAL)
            && matches!(process::normalize_video_link("BV1xx411c7mD")?, process::VideoLink::Bilibili(link) if link.ends_with("BV1xx411c7mD"))
            && process::normalize_video_link("https://www.douyin.com/note/7607104129969849061")
                .is_err_and(|e| e.to_string().contains("图文")),
        "导入弹窗没有分派到正确的视频平台"
    );
    for input in [
        "https://www.douyin.com/video/7607104129969849061",
        "https://douyin.com/video/7607104129969849061/?previous_page=app_code_link",
        "https://m.douyin.com/share/video/7607104129969849061/?region=CN",
        "https://www.iesdouyin.com/share/video/7607104129969849061/",
        "https://www.douyin.com/discover?modal_id=7607104129969849061",
        "7607104129969849061",
    ] {
        ensure!(
            process::normalize_douyin(input)? == DOUYIN,
            "抖音链接规范化失败：{input}"
        );
    }
    for input in [
        "https://www.douyin.com/note/7607104129969849061",
        "https://www.iesdouyin.com/share/note/7607104129969849061/",
        "https://www.douyin.com/user/MS4wLjABAAAA",
        "https://v.douyin.com/",
        "https://v.douyin.com/share/extra/",
        "https://www.douyin.com.evil.test/video/7607104129969849061",
        "http://www.douyin.com/video/7607104129969849061",
        "https://www.douyin.com/video/short",
        "https://www.youtube.com/watch?v=jNQXAC9IVRw",
    ] {
        ensure!(
            process::normalize_douyin(input).is_err(),
            "无效抖音输入未拒绝：{input}"
        );
    }
    let auth = anyhow::anyhow!(
        "ERROR: [youtube] fixture: Sign in to confirm you’re not a bot. Use --cookies-from-browser or --cookies"
    );
    ensure!(
        process::is_video_auth_error(&auth),
        "未识别 YouTube 登录验证错误"
    );
    ensure!(
        !process::is_video_auth_error(&anyhow::anyhow!("ERROR: [youtube] fixture: Private video")),
        "普通下载失败被误判为登录验证"
    );
    ensure!(
        process::YoutubeBrowser::from_id("chrome") == Some(process::YoutubeBrowser::Chrome)
            && process::YoutubeBrowser::from_id("bogus").is_none(),
        "浏览器标识解析错误"
    );

    subtitle_track(lib, results)?;
    #[cfg(unix)]
    fixture_download(lib, results)?;

    if let Ok(link) = std::env::var("CLARORA_DOUYIN_LIVE_VIDEO") {
        live_douyin(lib, &link, results)?;
    }
    for (source, env) in [
        (process::VideoSource::Weibo, "CLARORA_WEIBO_LIVE_VIDEO"),
        (process::VideoSource::X, "CLARORA_X_LIVE_VIDEO"),
    ] {
        if let Ok(link) = std::env::var(env) {
            live_social_video(lib, source, &link, results)?;
        }
    }

    results.push(json!({
        "workflow":"视频链接分派与规范化、图文和播放列表拒绝、YouTube Cookie 重试及失败清理",
        "passed":true
    }));
    Ok(())
}

fn subtitle_track(lib: &Library, results: &mut Vec<Value>) -> Result<()> {
    let directory = lib.root.join("downloads/subtitle-track");
    std::fs::create_dir_all(&directory)?;
    let video = directory.join("Lesson [fixture].mp4");
    std::fs::write(&video, b"video")?;
    std::fs::write(
        directory.join("Lesson [fixture].zh-Hans.srt"),
        "1\n00:00:01,000 --> 00:00:02,500\n中文字幕\n",
    )?;
    std::fs::write(
        directory.join("Lesson [fixture].en.srt"),
        "1\n00:00:01,000 --> 00:00:02,500\nPlatform subtitle.\n",
    )?;
    std::fs::write(
        directory.join("Lesson [fixture].extra.en.srt"),
        "1\n00:00:01,000 --> 00:00:02,500\nAnother video subtitle.\n",
    )?;
    let track = process::downloaded_subtitle(&video).context("未找到视频自带字幕")?;
    ensure!(
        track.file_name().and_then(|name| name.to_str()) == Some("Lesson [fixture].en.srt"),
        "没有优先选择当前视频的英文字幕：{}",
        track.display()
    );
    let plain = directory.join("Lesson.mp4");
    std::fs::write(&plain, b"video")?;
    std::fs::write(
        directory.join("Lesson.extra.en.srt"),
        "1\n00:00:01,000 --> 00:00:02,500\nWrong video.\n",
    )?;
    std::fs::write(directory.join("Lesson.en.mkv"), b"other video")?;
    std::fs::write(
        directory.join("Lesson.en.srt"),
        "1\n00:00:01,000 --> 00:00:02,500\nSibling video.\n",
    )?;
    ensure!(
        process::downloaded_subtitle(&plain).is_none(),
        "同前缀 MKV 的字幕被误选：{:?}",
        process::downloaded_subtitle(&plain)
    );
    std::fs::remove_file(directory.join("Lesson.en.mkv"))?;
    std::fs::write(directory.join("Lesson.en.mp4"), b"other video")?;
    ensure!(
        process::downloaded_subtitle(&plain).is_none(),
        "同前缀 MP4 的字幕被误选：{:?}",
        process::downloaded_subtitle(&plain)
    );
    let course = directory.join("Course.mp4");
    std::fs::write(&course, b"video")?;
    std::fs::write(
        directory.join("Course.zh-Hans.srt"),
        "1\n00:00:01,000 --> 00:00:02,500\n中文字幕\n",
    )?;
    std::fs::write(
        directory.join("Course.en-US.srt"),
        "1\n00:00:01,000 --> 00:00:02,500\nEnglish subtitle.\n",
    )?;
    ensure!(
        process::downloaded_subtitle(&course)
            .is_some_and(|track| track.file_name().and_then(|name| name.to_str())
                == Some("Course.en-US.srt")),
        "en-US 字幕没有按主语言优先于中文"
    );
    let cues = clarora_gpui::subtitles::parse(&std::fs::read_to_string(&track)?);
    ensure!(
        cues.len() == 1 && cues[0].text == "Platform subtitle.",
        "下载字幕解析错误"
    );
    clarora_gpui::video_subtitles::save(lib, &video, &cues, &process::cancel_token())?;
    let loaded = clarora_gpui::video_subtitles::load(lib, &video)?;
    ensure!(
        loaded.len() == cues.len()
            && loaded[0].start == cues[0].start
            && loaded[0].end == cues[0].end
            && loaded[0].text == cues[0].text,
        "下载字幕没有进入视频字幕缓存"
    );
    results.push(json!({
        "workflow":"平台自带字幕：优先英文轨道、解析并写入视频字幕缓存",
        "passed":true,
        "track":track,
        "cues":cues.len()
    }));
    Ok(())
}

fn social_links() -> Result<()> {
    use process::VideoSource::{Weibo, X};
    for (input, source, expected) in [
        (
            "微博 https://m.weibo.cn/status/FBqgOmDxO?source=share，转发",
            Weibo,
            "https://weibo.com/0/FBqgOmDxO",
        ),
        (
            "https://weibo.com/5926682210/FBqgOmDxO",
            Weibo,
            "https://weibo.com/0/FBqgOmDxO",
        ),
        (
            "https://www.weibo.com/2/detail/5037296838246572",
            Weibo,
            "https://weibo.com/0/5037296838246572",
        ),
        (
            "https://video.weibo.com/show?from=share&fid=1034%3A5037296697933885",
            Weibo,
            "https://weibo.com/tv/show/1034:5037296697933885",
        ),
        (
            "https://weibo.com/tv/show/1034%3A5037296697933885/",
            Weibo,
            "https://weibo.com/tv/show/1034:5037296697933885",
        ),
        (
            "分享 http://t.cn/A6aerGsM。",
            Weibo,
            "https://t.cn/A6aerGsM",
        ),
        (
            "https://x.com/jcarlos2001/status/2103382268051964103?s=20",
            X,
            "https://x.com/i/web/status/2103382268051964103",
        ),
        (
            "https://mobile.twitter.com/jcarlos2001/status/2103382268051964103",
            X,
            "https://x.com/i/web/status/2103382268051964103",
        ),
        (
            "https://x.com/i/status/2103382268051964103/video/2?s=20",
            X,
            "https://x.com/i/web/status/2103382268051964103/video/2",
        ),
        ("https://t.co/vZ4mR6x7Iv", X, "https://t.co/vZ4mR6x7Iv"),
    ] {
        ensure!(
            matches!(process::normalize_video_link(input)?, process::VideoLink::External(actual_source, link)
                if actual_source == source && link == expected),
            "微博或 X 的分享链接未正确分派与规范化：{input}"
        );
    }
    for input in [
        "https://x.com/jcarlos2001",
        "https://x.com/i/spaces/123",
        "https://x.com/user/status/20/photo/1",
        "https://x.com/user/status/20/video/0",
        "https://x.com@other.invalid/user/status/20",
        "https://member@x.com/user/status/20",
        "https://weibo.com.evil.test/tv/show/1034:5037296697933885",
        "https://weibo.com/u/5926682210",
        "https://video.weibo.com/show?fid=not-a-video",
        "https://t.cn/A6aerGsM/extra",
    ] {
        ensure!(
            process::normalize_video_link(input).is_err(),
            "输入不是受支持的单条作品链接：{input}"
        );
    }
    Ok(())
}

#[cfg(unix)]
fn fixture_download(lib: &Library, results: &mut Vec<Value>) -> Result<()> {
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
name="Example video [jNQXAC9IVRw].mp4"
case "$*" in
    *'/0/1000001'|*'/status/1000001'*)
        case "$*" in
            *'--cookies-from-browser '*) echo "unexpected browser cookies" >&2; exit 9 ;;
        esac
        case "$*" in
            *'--playlist-items 1 '*'--use-extractors Weibo,WeiboVideo '*) name="Weibo fixture [1000001].mp4" ;;
            *'--playlist-items 1 '*'--use-extractors twitter,twitter:amplify '*'twitter:api=syndication '*) name="X fixture [1000001].mp4" ;;
            *'--playlist-items 1 '*'--use-extractors twitter,twitter:amplify '*)
                echo "ERROR: [twitter] 1000001: Failed to parse JSON" >&2
                exit 8 ;;
            *) echo "expected a single platform video" >&2; exit 6 ;;
        esac ;;
    *'/status/1000002')
        echo "ERROR: [twitter] 1000002: No video could be found in this tweet" >&2
        exit 8 ;;
    *CONCURRENT1*)
        if mkdir "$marker.race" 2>/dev/null; then
            file="$dir/Example video [CONCURRENT1].mp4"
            printf 'unfinished video' > "$file"
            touch "$marker.race/started"
            while [ ! -f "$marker.race/release" ]; do sleep 0.02; done
            exit 8
        fi ;;
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
file="$dir/$name"
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
        ensure!(cached == 2, "YouTube 缓存缺少已完成目录或锁文件");
        let auth_error = process::download_youtube(
            &lib.root,
            "https://youtu.be/AUTHFIXTURE",
            &process::cancel_token(),
        )
        .unwrap_err();
        ensure!(
            process::is_video_auth_error(&auth_error),
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
            !process::is_video_auth_error(&failed),
            "普通下载失败被误判为登录验证"
        );
        ensure!(
            fs::read_dir(&downloads)?.count() == after,
            "失败下载未清理临时目录"
        );
        let race = tools.join(".clarora-yt-dlp-fixture.race");
        let other_root = lib.root.clone();
        let first_cancel = process::cancel_token();
        let worker_cancel = first_cancel.clone();
        let url = "https://youtu.be/CONCURRENT1";
        let first =
            std::thread::spawn(move || process::download_youtube(&other_root, url, &worker_cancel));
        let second = (|| -> Result<_> {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(4);
            while !race.join("started").exists() {
                ensure!(
                    std::time::Instant::now() < until,
                    "并发下载未到达写入测试点"
                );
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            process::download_youtube(&lib.root, url, &process::cancel_token())
        })();
        let released = fs::write(race.join("release"), b"");
        if released.is_err() {
            first_cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        let failed = first
            .join()
            .map_err(|_| anyhow::anyhow!("并发下载异常结束"))?;
        if race.is_dir() {
            fs::remove_dir_all(&race)?;
        }
        let second = second?;
        ensure!(
            failed.is_err()
                && fs::read(&second)? == b"fixture-video-bytes"
                && process::download_youtube(&lib.root, url, &process::cancel_token())? == second
                && fs::read_dir(&cache)?.count() == cached + 2,
            "并发下载读取了未完成文件、失败删除了成功文件，或留下暂存目录"
        );
        fs::remove_file(&path)?;
        ensure!(
            process::download_youtube(&lib.root, CANONICAL, &process::cancel_token())? == path
                && fs::read(&path)? == b"fixture-video-bytes"
                && fs::read_dir(&cache)?.count() == cached + 2,
            "媒体被移走后无法修复缓存"
        );
        results.push(json!({"workflow":"YouTube 同链接并发导入及失败清理","passed":true,"cached_file":second}));
        fixture_social_video(lib, results)?;
        Ok(())
    })();
    let _ = fs::remove_file(&marker);
    let _ = fs::remove_file(&yt);
    if !tools_existed {
        let _ = fs::remove_dir(&tools);
    }
    outcome
}

#[cfg(unix)]
fn fixture_social_video(lib: &Library, results: &mut Vec<Value>) -> Result<()> {
    use process::VideoSource::{Weibo, X};
    let cancel = process::cancel_token();
    for (source, first, duplicate, expected) in [
        (
            Weibo,
            "https://m.weibo.cn/detail/1000001",
            "https://weibo.com/20/1000001",
            "Weibo fixture [1000001].mp4",
        ),
        (
            X,
            "https://twitter.com/user/status/1000001",
            "https://x.com/user/status/1000001?s=20",
            "X fixture [1000001].mp4",
        ),
    ] {
        let path = process::download_external_with(
            source,
            &lib.root,
            first,
            Some(process::YoutubeBrowser::Chrome),
            &cancel,
        )?;
        ensure!(
            path.file_name().and_then(|name| name.to_str()) == Some(expected)
                && path
                    == process::download_external_with(
                        source, &lib.root, duplicate, None, &cancel
                    )?,
            "下载未选择指定平台，或等价链接未复用文件"
        );
    }
    let cache = lib.root.join("downloads/remote-x");
    let files = fs::read_dir(&cache)?.count();
    let error = process::download_external_with(
        X,
        &lib.root,
        "https://x.com/i/status/1000002",
        None,
        &cancel,
    );
    ensure!(
        error.is_err_and(|error| error.to_string().contains("没有可下载的视频"))
            && fs::read_dir(&cache)?.count() == files,
        "无视频的帖子应解释原因并清理下载目录"
    );
    results.push(json!({"workflow":"微博 / X 单视频下载、等价链接缓存、X 公开接口重试、失败清理","passed":true}));
    Ok(())
}

fn live_douyin(lib: &Library, link: &str, results: &mut Vec<Value>) -> Result<()> {
    use std::time::Duration;
    let cancel = process::cancel_token();
    let path = process::download_external_with(
        process::VideoSource::Douyin,
        &lib.root,
        link,
        None,
        &cancel,
    )?;
    let id = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .and_then(|stem| stem.rsplit_once(" ["))
        .and_then(|(_, suffix)| suffix.strip_suffix(']'))
        .ok_or_else(|| anyhow::anyhow!("抖音下载结果缺少作品编号"))?;
    let canonical = format!("https://www.douyin.com/video/{id}");
    ensure!(
        process::download_douyin(&lib.root, &canonical, &cancel)? == path,
        "抖音短链与完整链接未复用文件"
    );
    let mut videos = clarora_gpui::video_library::Collection::load(lib)?;
    let group = videos.create_group("抖音测试")?;
    let ids = videos.add_in_group(std::slice::from_ref(&path), None, &group)?;
    videos.select(&ids[0])?;
    videos.save(lib)?;
    let reopened = clarora_gpui::video_library::Collection::load(lib)?;
    ensure!(
        reopened.active().is_some_and(|video| video.path == path
            && video.group == group
            && video.series.is_none()),
        "抖音视频归组或重开失败"
    );
    let output = process::run(
        &process::tool("ffprobe")?,
        &[
            "-v".into(),
            "error".into(),
            "-show_entries".into(),
            "stream=codec_type:format=duration".into(),
            "-of".into(),
            "json".into(),
            path.to_string_lossy().into_owned(),
        ],
        &cancel,
        Duration::from_secs(20),
    )?;
    let probe: Value = serde_json::from_str(&output)?;
    ensure!(
        probe["streams"]
            .as_array()
            .is_some_and(|streams| streams.iter().any(|stream| stream["codec_type"] == "video")),
        "抖音下载没有可读取的视频轨"
    );
    results.push(json!({"workflow":"抖音公开作品真实下载、短链与长链复用、归组并重开","passed":true,"video":path,"media":probe}));
    Ok(())
}

fn live_social_video(
    lib: &Library,
    source: process::VideoSource,
    input: &str,
    results: &mut Vec<Value>,
) -> Result<()> {
    use clarora_gpui::video_library::Collection;
    use std::time::Duration;

    let cancel = process::cancel_token();
    let path = process::download_external_with(source, &lib.root, input, None, &cancel)?;
    let duplicate = process::download_external_with(source, &lib.root, input, None, &cancel)?;
    ensure!(duplicate == path, "再次导入没有复用已下载的视频");

    let mut videos = Collection::load(lib)?;
    let group = videos.create_group(source.label())?;
    let ids = videos.add_in_group(std::slice::from_ref(&path), None, &group)?;
    let saved = videos.select(&ids[0])?;
    ensure!(
        saved.group == group && saved.path == path && saved.series.is_none(),
        "视频未写入原分组"
    );
    ensure!(
        path.file_stem().and_then(|stem| stem.to_str()) != Some(saved.title.as_str()),
        "收藏标题仍含内部视频编号"
    );
    videos.save(lib)?;
    ensure!(
        Collection::load(lib)?
            .active()
            .is_some_and(|video| video.path == path),
        "重开应用后没有找到视频"
    );

    let report = process::run(
        &process::tool("ffprobe")?,
        &[
            "-v".into(),
            "error".into(),
            "-show_entries".into(),
            "stream=codec_type,codec_name:format=duration".into(),
            "-of".into(),
            "json".into(),
            path.to_string_lossy().into_owned(),
        ],
        &cancel,
        Duration::from_secs(20),
    )?;
    let media: Value = serde_json::from_str(&report)?;
    ensure!(
        media["streams"].as_array().is_some_and(|streams| {
            ["video", "audio"]
                .iter()
                .all(|kind| streams.iter().any(|stream| stream["codec_type"] == *kind))
        }),
        "公开视频样本缺少可读取的画面或音轨"
    );
    results.push(json!({
        "workflow": format!("{} 真实下载、缓存复用与收藏重开", source.label()),
        "passed": true,
        "video": path,
        "media": media
    }));
    Ok(())
}
