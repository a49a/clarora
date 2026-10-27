//! Actual AppView navigation with isolated SQLite and real libmpv decoding.
//! Failure scenarios were recorded before implementation in the task artifacts.
use super::*;

const POSITION: f64 = 6.4;
fn video(lib: &Library) -> PathBuf {
    lib.root.join("media").join("resume-video.mp4")
}
fn other_video(lib: &Library) -> PathBuf {
    lib.root.join("media").join("resume-other.mp4")
}

pub(super) fn prepare(lib: &Library) -> Result<()> {
    let path = video(lib);
    process::run(
        &process::tool("ffmpeg")?,
        &[
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=15",
            "-t",
            "12",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
            "-an",
            "-y",
        ]
        .into_iter()
        .map(String::from)
        .chain([path.to_string_lossy().into_owned()])
        .collect::<Vec<_>>(),
        &process::cancel_token(),
        Duration::from_secs(30),
    )?;
    fs::copy(&path, other_video(lib))?;
    let mut collection = video_library::Collection::load(lib)?;
    let ids = collection.add(std::slice::from_ref(&path), None)?;
    collection.select(&ids[0])?;
    collection.save(lib)?;
    video_subtitles::save(
        lib,
        &path,
        &[
            Cue {
                start: 0.,
                end: 4.,
                text: "Before resume.".into(),
            },
            Cue {
                start: 4.,
                end: 8.,
                text: "Resume this sentence.".into(),
            },
            Cue {
                start: 8.,
                end: 12.,
                text: "After resume.".into(),
            },
        ],
        &process::cancel_token(),
    )?;
    Ok(())
}

pub(super) struct Probe {
    out: PathBuf,
    stage: usize,
    entered: Instant,
    cases: Vec<Value>,
}
impl Probe {
    pub(super) fn new(out: &Path) -> Self {
        Self {
            out: out.into(),
            stage: 0,
            entered: Instant::now(),
            cases: vec![],
        }
    }
    fn advance(&mut self) {
        self.stage += 1;
        self.entered = Instant::now();
    }
    fn record(&mut self, app: &AppView, name: &str) -> Result<()> {
        let state = app.media.player.snapshot();
        self.cases.push(json!({"case":name,"passed":true,"path":state.path,"position":state.position,"duration":state.duration,"paused":state.paused,"error":state.error,"page":app.page,"bookmark":app.lib.setting("last_video_position")?}));
        self.report(None, None)
    }
    fn report(&self, passed: Option<bool>, error: Option<String>) -> Result<()> {
        fs::write(
            self.out.join("video-resume-report.json"),
            serde_json::to_vec_pretty(
                &json!({"passed":passed,"error":error,"stage":self.stage,"cases":self.cases,"actual_appview_handlers":true,"actual_libmpv":true,"audio_device":"null","mouse_keyboard_visual_verified":false,"independent_review":false}),
            )?,
        )?;
        Ok(())
    }
    fn at(app: &AppView, path: &Path, position: f64) -> bool {
        let state = app.media.player.snapshot();
        state.path == path.to_string_lossy()
            && state.duration > 11.9
            && state.paused
            && state.error.is_empty()
            && state.frame.is_some()
            && (state.position - position).abs() < 0.18
    }
    fn stable(&self) -> bool {
        self.entered.elapsed() >= Duration::from_millis(500)
    }
    pub(super) fn step(
        &mut self,
        app: &mut AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Result<bool> {
        let result = self.check(app, window, cx);
        if let Err(error) = &result {
            let _ = self.report(Some(false), Some(format!("{error:#}")));
        }
        result
    }
    fn check(
        &mut self,
        app: &mut AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Result<bool> {
        ensure!(
            self.stage == 0 || self.entered.elapsed() < Duration::from_secs(8),
            "视频恢复阶段{}超时，实际位置={}，错误={}",
            self.stage,
            app.media.player.snapshot().position,
            app.media.player.snapshot().error
        );
        let path = video(&app.lib);
        if app.page == "video" {
            // Hidden windows do not receive normal layout paints. Explicitly
            // request real decoded frames while checking navigation handlers.
            app.media.player.set_video_target(320, 180, true);
        }
        match self.stage {
            0 => {
                app.handle(Action::Page("video".into()), window, cx);
                self.advance();
            }
            1 => {
                let state = app.media.player.snapshot();
                if state.path != path.to_string_lossy() || state.duration < 11.9 || !state.paused {
                    return Ok(false);
                }
                app.handle(Action::SeekTo(POSITION), window, cx);
                self.advance();
            }
            2 => {
                if !Self::at(app, &path, POSITION) {
                    return Ok(false);
                }
                self.record(app, "视频停在6.4秒")?;
                app.handle(Action::Page("settings".into()), window, cx);
                let saved: Value = serde_json::from_str(&app.lib.setting("last_video_position")?)?;
                ensure!(
                    (saved["position"].as_f64().unwrap_or(-1.) - POSITION).abs() < 0.18,
                    "切页保存的进度不正确"
                );
                app.handle(Action::Page("video".into()), window, cx);
                self.advance();
            }
            3 => {
                if !self.stable() || !Self::at(app, &path, POSITION) || app.video.active != Some(1)
                {
                    return Ok(false);
                }
                self.record(app, "视频→设置→视频保留进度/暂停/字幕句")?;
                app.handle(Action::Page("listening".into()), window, cx);
                self.advance();
            }
            4 => {
                let state = app.media.player.snapshot();
                if app.audio.current_id.is_empty()
                    || state.duration < 7.9
                    || state.duration > 8.1
                    || !state.paused
                    || app
                        .media
                        .path
                        .as_ref()
                        .is_none_or(|p| p.to_string_lossy() != state.path)
                {
                    return Ok(false);
                }
                app.handle(Action::SeekTo(4.25), window, cx);
                self.advance();
            }
            5 => {
                if (app.media.player.snapshot().position - 4.25).abs() > 0.18 {
                    return Ok(false);
                }
                app.handle(Action::Page("video".into()), window, cx);
                app.handle(Action::Page("meditation".into()), window, cx);
                app.handle(Action::Page("video".into()), window, cx);
                let saved: Value = serde_json::from_str(&app.lib.setting("last_video_position")?)?;
                ensure!(
                    (saved["position"].as_f64().unwrap_or(-1.) - POSITION).abs() < 0.18,
                    "快速切页把其他媒体位置写入视频书签：{saved}"
                );
                self.advance();
            }
            6 => {
                if !self.stable() || !Self::at(app, &path, POSITION) {
                    return Ok(false);
                }
                self.record(app, "音频4.25秒→视频→冥想→视频仍为6.4秒")?;
                app.lib.put("clip_cards",json!({"id":"resume-same-video-clip","audio_uri":path.to_string_lossy(),"start_ms":1000,"end_ms":2000,"en_text":"Same video clip.","zh_text":"同源片段"}))?;
                app.card_filter = "clip".into();
                app.handle(Action::Page("flashcard".into()), window, cx);
                self.advance();
            }
            7 => {
                let state = app.media.player.snapshot();
                if !app.cards_ready()
                    || app
                        .current_card()
                        .is_none_or(|c| c.id != "resume-same-video-clip")
                    || state.path != path.to_string_lossy()
                    || state.position < 0.95
                    || state.position > 2.2
                {
                    return Ok(false);
                }
                self.record(app, "闪卡实际加载同一视频的1–2秒片段")?;
                app.handle(Action::Page("video".into()), window, cx);
                self.advance();
            }
            8 => {
                if !self.stable() || !Self::at(app, &path, POSITION) {
                    return Ok(false);
                }
                self.record(app, "同源片段→视频恢复6.4秒并退出片段区间")?;
                app.handle(Action::Page("settings".into()), window, cx);
                app.lib.set_setting("last_video_position", "{broken")?;
                app.handle(Action::Page("video".into()), window, cx);
                self.advance();
            }
            9 => {
                if !self.stable() || !Self::at(app, &path, 0.) {
                    return Ok(false);
                }
                self.record(app, "损坏进度安全回退到零")?;
                app.handle(Action::Page("settings".into()), window, cx);
                app.lib.set_setting(
                    "last_video_position",
                    &json!({"path":path,"position":-10.}).to_string(),
                )?;
                app.handle(Action::Page("video".into()), window, cx);
                self.advance();
            }
            10 => {
                if !self.stable() || !Self::at(app, &path, 0.) {
                    return Ok(false);
                }
                self.record(app, "负数进度安全回退到零")?;
                ensure!(app.open(other_video(&app.lib)), "无法选择第二个视频");
                app.handle(Action::Page("settings".into()), window, cx);
                app.lib.set_setting(
                    "last_video_position",
                    &json!({"path":path,"position":POSITION}).to_string(),
                )?;
                app.handle(Action::Page("video".into()), window, cx);
                self.advance();
            }
            11 => {
                if !self.stable() || !Self::at(app, &other_video(&app.lib), 0.) {
                    return Ok(false);
                }
                self.record(app, "第二个视频不套用第一视频的进度")?;
                self.report(Some(true), None)?;
                self.advance();
                return Ok(true);
            }
            _ => return Ok(true),
        }
        Ok(false)
    }
}
