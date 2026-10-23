//! Real FFmpeg/libmpv render acceptance. Failure cases precede production changes
//! in artifacts/clarora/gpui-backup-video-render/video-render/failure-cases.md.
use anyhow::{Context, Result, ensure};
use clarora_gpui::{
    data::Library,
    media::{Playback, Player},
    process,
};
use serde_json::{Value, json};
use std::{
    fs,
    future::Future,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context as TaskContext, Poll, Wake, Waker},
    time::{Duration, Instant},
};

fn wait(
    player: &Player,
    name: &str,
    traces: &mut Vec<Value>,
    predicate: impl Fn(&Playback) -> bool,
) -> Result<Playback> {
    let start = Instant::now();
    loop {
        let state = player.snapshot();
        ensure!(state.error.is_empty(), "{name}: {}", state.error);
        if predicate(&state) {
            traces.push(json!({"stage":name,"elapsed_ms":start.elapsed().as_millis(),"width":state.frame_width,"height":state.frame_height,"revision":state.frame_revision,"position":state.position,"stats":state.render_stats}));
            return Ok(state);
        }
        ensure!(
            start.elapsed() < Duration::from_secs(10),
            "{name}: timeout, frame={}x{}, path={}, paused={}",
            state.frame_width,
            state.frame_height,
            state.path,
            state.paused
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn save(path: &Path, state: &Playback) -> Result<()> {
    let image = state.frame.as_ref().context("missing decoded frame")?;
    let mut bytes = image.as_bytes(0).context("missing frame bytes")?.to_vec();
    ensure!(
        bytes.chunks_exact(4).all(|pixel| pixel[3] == 255),
        "libmpv packed video pixels are not opaque"
    );
    for pixel in bytes.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    image::save_buffer(
        path,
        &bytes,
        state.frame_width,
        state.frame_height,
        image::ColorType::Rgba8,
    )?;
    Ok(())
}
struct Counter(AtomicUsize);
impl Wake for Counter {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

pub fn run(lib: &Library, results: &mut Vec<Value>) -> Result<()> {
    let root = lib.root.join("video-render");
    fs::create_dir_all(&root)?;
    let fixture = root.join("1080p60.mp4");
    let args = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        "testsrc2=size=1920x1080:rate=60".into(),
        "-t".into(),
        "3".into(),
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "ultrafast".into(),
        "-crf".into(),
        "24".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-an".into(),
        "-y".into(),
        fixture.to_string_lossy().into_owned(),
    ];
    let ffmpeg = process::tool("ffmpeg")?;
    process::run(
        &ffmpeg,
        &args,
        &process::cancel_token(),
        Duration::from_secs(30),
    )?;
    let mut traces = vec![];
    let player = Player::without_audio_device();
    player.set_video_target(1920, 1080, true);
    player.open_range(&fixture, 0., 0., true);
    let full = wait(
        &player,
        "1080p paused source resolution",
        &mut traces,
        |s| s.paused && s.frame_width == 1920 && s.frame_height == 1080,
    )?;
    save(&root.join("1080p.png"), &full)?;
    let held = full.frame.clone().unwrap();
    let held_bytes = held.as_bytes(0).unwrap().to_vec();
    let events = player.frame_events();
    let revision = events.revision();
    let counter1 = Arc::new(Counter(AtomicUsize::new(0)));
    let counter2 = Arc::new(Counter(AtomicUsize::new(0)));
    let wake1 = Waker::from(counter1.clone());
    let wake2 = Waker::from(counter2.clone());
    let mut first = Box::pin(events.changed(revision));
    let mut second = Box::pin(events.changed(revision));
    ensure!(
        first
            .as_mut()
            .poll(&mut TaskContext::from_waker(&wake1))
            .is_pending(),
        "first waiter did not wait"
    );
    ensure!(
        second
            .as_mut()
            .poll(&mut TaskContext::from_waker(&wake2))
            .is_pending(),
        "second waiter did not wait"
    );
    player.set_video_target(800, 800, true);
    let square = wait(
        &player,
        "paused square viewport preserves 16:9",
        &mut traces,
        |s| s.frame_width == 800 && s.frame_height == 450 && s.frame_revision > full.frame_revision,
    )?;
    save(&root.join("square-viewport.png"), &square)?;
    ensure!(
        counter1.0.load(Ordering::SeqCst) > 0 && counter2.0.load(Ordering::SeqCst) > 0,
        "frame notification did not broadcast"
    );
    ensure!(
        matches!(first.as_mut().poll(&mut TaskContext::from_waker(&wake1)),Poll::Ready(value) if value>revision),
        "first waiter lost revision"
    );
    ensure!(
        second
            .as_mut()
            .poll(&mut TaskContext::from_waker(&wake2))
            .is_ready(),
        "second waiter lost revision"
    );
    ensure!(
        held.as_bytes(0).unwrap() == held_bytes,
        "published image mutated after resize"
    );
    player.set_video_target(u32::MAX, u32::MAX, true);
    let capped = wait(
        &player,
        "oversized viewport capped to source and memory budget",
        &mut traces,
        |s| {
            s.frame_revision > square.frame_revision
                && s.frame_width == 1920
                && s.frame_height == 1080
        },
    )?;
    ensure!(
        u64::from(capped.frame_width) * u64::from(capped.frame_height) <= 2560 * 1440,
        "unbounded frame allocation"
    );
    player.set_video_target(0, 0, false);
    let hidden = wait(&player, "hidden surface clears image", &mut traces, |s| {
        s.frame.is_none() && s.frame_revision > capped.frame_revision
    })?;
    player.set("pause", "no");
    let hidden_running = wait(&player, "hidden video clock continues", &mut traces, |s| {
        s.position > 0.3
    })?;
    ensure!(
        hidden_running.frame.is_none()
            && hidden_running.render_stats.rendered_frames == hidden.render_stats.rendered_frames,
        "hidden surface rendered frames"
    );
    player.set_video_target(960, 540, true);
    let visible = wait(
        &player,
        "reshow video publishes new frame",
        &mut traces,
        |s| s.frame_width == 960 && s.frame_height == 540 && s.frame.is_some(),
    )?;
    save(&root.join("reshown.png"), &visible)?;
    player.command(["stop"]);
    wait(
        &player,
        "stop clears image and metadata",
        &mut traces,
        |s| s.path.is_empty() && s.frame.is_none() && s.frame_width == 0 && s.frame_height == 0,
    )?;
    player.set("hwdec", "no");
    player.open_range(&fixture, 0., 0., true);
    wait(
        &player,
        "explicit software decode fallback",
        &mut traces,
        |s| s.frame.is_some() && s.paused && s.render_stats.decoder == "no",
    )?;
    player.set("pause", "no");
    wait(
        &player,
        "natural EOF clears stale image",
        &mut traces,
        |s| s.eof && s.frame.is_none(),
    )?;
    let portrait = root.join("portrait.mp4");
    process::run(
        &ffmpeg,
        &[
            "-hide_banner".into(),
            "-loglevel".into(),
            "error".into(),
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            "testsrc2=size=360x640:rate=12".into(),
            "-t".into(),
            "0.5".into(),
            "-c:v".into(),
            "libx264".into(),
            "-preset".into(),
            "ultrafast".into(),
            "-y".into(),
            portrait.to_string_lossy().into_owned(),
        ],
        &process::cancel_token(),
        Duration::from_secs(20),
    )?;
    player.set_video_target(800, 800, true);
    player.open_range(&portrait, 0., 0., true);
    let portrait_frame = wait(
        &player,
        "portrait source keeps orientation and never upscales",
        &mut traces,
        |s| {
            s.path == portrait.to_string_lossy()
                && s.frame_width == 360
                && s.frame_height == 640
                && s.paused
        },
    )?;
    save(&root.join("portrait.png"), &portrait_frame)?;
    let large = root.join("4k.mp4");
    process::run(
        &ffmpeg,
        &[
            "-hide_banner".into(),
            "-loglevel".into(),
            "error".into(),
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            "testsrc2=size=3840x2160:rate=1".into(),
            "-frames:v".into(),
            "1".into(),
            "-c:v".into(),
            "libx264".into(),
            "-preset".into(),
            "ultrafast".into(),
            "-y".into(),
            large.to_string_lossy().into_owned(),
        ],
        &process::cancel_token(),
        Duration::from_secs(20),
    )?;
    player.set_video_target(u32::MAX, u32::MAX, true);
    player.open_range(&large, 0., 0., true);
    let bounded = wait(
        &player,
        "4k source obeys pixel allocation ceiling",
        &mut traces,
        |s| {
            s.path == large.to_string_lossy()
                && s.frame_width == 2560
                && s.frame_height == 1440
                && s.paused
        },
    )?;
    save(&root.join("4k-capped.png"), &bounded)?;
    let audio = root.join("audio-only.wav");
    process::run(
        &ffmpeg,
        &[
            "-hide_banner".into(),
            "-loglevel".into(),
            "error".into(),
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            "sine=frequency=440:duration=1".into(),
            "-y".into(),
            audio.to_string_lossy().into_owned(),
        ],
        &process::cancel_token(),
        Duration::from_secs(15),
    )?;
    player.open_range(&audio, 0., 0., true);
    wait(
        &player,
        "audio-only media produces no video image",
        &mut traces,
        |s| s.path == audio.to_string_lossy() && s.duration > 0.9 && s.frame.is_none(),
    )?;
    let software = root.join("software-codec.mkv");
    process::run(
        &ffmpeg,
        &[
            "-hide_banner".into(),
            "-loglevel".into(),
            "error".into(),
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            "testsrc2=size=320x180:rate=1".into(),
            "-frames:v".into(),
            "1".into(),
            "-c:v".into(),
            "ffv1".into(),
            "-y".into(),
            software.to_string_lossy().into_owned(),
        ],
        &process::cancel_token(),
        Duration::from_secs(15),
    )?;
    player.set("hwdec", "auto-copy");
    player.open_range(&software, 0., 0., true);
    wait(
        &player,
        "auto-copy automatically falls back for FFV1",
        &mut traces,
        |s| {
            s.path == software.to_string_lossy()
                && s.frame_width == 320
                && s.render_stats.decoder == "no"
        },
    )?;
    let preloader = Player::without_audio_device();
    preloader.set_video_target(0, 0, false);
    preloader.open_range(&fixture, 0., 0., true);
    let preload = wait(
        &preloader,
        "hidden preloader publishes no images",
        &mut traces,
        |s| s.duration > 2.9 && s.paused,
    )?;
    ensure!(
        preload.frame.is_none() && preload.render_stats.rendered_frames == 0,
        "preloader converted video pixels"
    );
    drop(preloader);
    player.set_video_target(960, 540, true);
    player.open(&fixture);
    wait(
        &player,
        "active video before renderer shutdown",
        &mut traces,
        |s| s.path == fixture.to_string_lossy() && s.frame.is_some() && s.position > 0.1,
    )?;
    let closing = Instant::now();
    drop(player);
    ensure!(
        closing.elapsed() < Duration::from_secs(5),
        "renderer shutdown blocked the owning player"
    );
    traces.push(
        json!({"stage":"active renderer shutdown","elapsed_ms":closing.elapsed().as_millis()}),
    );
    let evidence = json!({"fixture":fixture,"ffmpeg":ffmpeg,"ffmpeg_args":args,"traces":traces,"images":["1080p.png","square-viewport.png","reshown.png","portrait.png","4k-capped.png"],"render_backend":"libmpv software frame API; hardware decoder is separately reported","checks":["dynamic viewport","paused resize","aspect ratio","portrait source","source and memory cap","broadcast notifications","immutable published pixels","hidden surface no frames","show resumes","stop clears","manual and automatic software fallback","EOF clears","audio-only no frames","preloader no frames","active render shutdown"]});
    fs::write(
        root.join("evidence.json"),
        serde_json::to_vec_pretty(&evidence)?,
    )?;
    results.push(json!({"workflow":"真实1080p60视频按视口渲染、暂停缩放、帧唤醒、隐藏/EOF清帧与软件回退","passed":true,"evidence":root.join("evidence.json")}));
    Ok(())
}
