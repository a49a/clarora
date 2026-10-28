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
    let second_srt = video_subtitles::save(
        lib,
        &other_video(lib),
        &[
            Cue {
                start: 0.3,
                end: 0.34,
                text: "First valid cue".into(),
            },
            Cue {
                start: 1.2,
                end: 1.24,
                text: "Last cue".into(),
            },
        ],
        &process::cancel_token(),
    )?;
    lib.set_setting("inherit_fixture_subtitle", &second_srt)?;
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
        self.cases.push(json!({"case":name,"passed":true,"path":state.path,"position":state.position,"duration":state.duration,"paused":state.paused,"error":state.error,"page":app.page,"bookmark":app.lib.setting("last_video_position")?,"part_confirmation":app.video.part_confirmation.is_some(),"series_current":app.video.series.as_ref().and_then(|s| s.current),"triple_sentence":app.video.sentence_repeat.filter(|s| s.three).map(|s| s.index)}));
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
                app.video.auto_next_part = false;
                app.video.series = Some(bilibili::Series {
                    title: "Keyboard confirmation fixture".into(),
                    base: "https://www.bilibili.com/video/BV1xx411c7mD".into(),
                    requested: 0,
                    current: Some(0),
                    parts: vec![
                        bilibili::Part {
                            number: 1,
                            title: "First fixture".into(),
                            duration: 12.,
                            path: Some(path.clone()),
                        },
                        bilibili::Part {
                            number: 2,
                            title: "Next fixture".into(),
                            duration: 12.,
                            path: Some(other_video(&app.lib)),
                        },
                    ],
                });
                ensure!(app.open(path.clone()), "无法打开键盘确认夹具");
                app.media.player.set("pause", "yes");
                self.advance();
            }
            12 => {
                if !Self::at(app, &path, 0.) || app.video.cues.len() != 3 {
                    return Ok(false);
                }
                window.focus(&app.focus);
                ensure!(
                    app.handle_video_arrow_key(&Self::key("right", false), window, cx),
                    "方向键未路由"
                );
                ensure!(app.video.part_confirmation.is_none(), "普通句子误弹切P确认");
                app.handle(Action::VideoSentenceRepeat(Some(2), true), window, cx);
                app.handle_video_arrow_key(&Self::key("right", false), window, cx);
                ensure!(
                    app.video.part_confirmation.is_some() && app.video.path.as_ref() == Some(&path),
                    "首次右键未只提示"
                );
                ensure!(
                    app.video
                        .sentence_repeat
                        .is_some_and(|s| s.three && s.index == 2),
                    "确认前清掉三遍模式"
                );
                self.record(app, "末句首次右方向键只弹确认/原P与三遍模式保留")?;
                for _ in 0..4 {
                    app.handle_video_arrow_key(&Self::key("right", true), window, cx);
                }
                ensure!(
                    app.video.part_confirmation.is_some() && app.video.path.as_ref() == Some(&path),
                    "长按重复事件误确认"
                );
                app.handle_video_arrow_key(&Self::key("escape", false), window, cx);
                ensure!(app.video.part_confirmation.is_none(), "Esc未取消");
                app.handle_video_arrow_key(&Self::key("right", false), window, cx);
                app.handle_video_arrow_key(&Self::key("left", false), window, cx);
                ensure!(
                    app.video.part_confirmation.is_none()
                        && app.video.sentence_repeat.is_some_and(|s| s.index == 1),
                    "左键没有取消并继续三遍切句"
                );
                self.record(app, "普通右键切句/末句首次确认/长按不确认/Esc与左键取消")?;
                app.handle(Action::SeekTo(POSITION), window, cx);
                app.media.player.set("pause", "yes");
                self.advance();
            }
            13 => {
                if !Self::at(app, &path, POSITION) {
                    return Ok(false);
                }
                window.focus(&app.focus);
                let cues = app.video.cues.clone();
                app.video.set_cues(vec![]);
                app.handle_video_arrow_key(&Self::key("right", false), window, cx);
                ensure!(app.video.part_confirmation.is_none(), "无字幕误确认");
                app.video.set_cues(cues);
                app.handle(Action::VideoSentenceRepeat(Some(2), true), window, cx);
                app.tasks.cancel = Some(process::cancel_token());
                app.handle_video_arrow_key(&Self::key("right", false), window, cx);
                ensure!(app.video.part_confirmation.is_none(), "任务忙仍弹切P确认");
                app.tasks.cancel = None;
                app.handle_video_arrow_key(&Self::key("right", false), window, cx);
                ensure!(app.video.part_confirmation.is_some(), "忙碌结束后未可确认");
                app.handle(Action::Page("settings".into()), window, cx);
                ensure!(app.video.part_confirmation.is_none(), "切栏目保留旧确认");
                app.handle(Action::Page("video".into()), window, cx);
                self.advance();
            }
            14 => {
                if !self.stable()
                    || app.media.player.snapshot().path != path.to_string_lossy()
                    || app.video.cues.len() != 3
                {
                    return Ok(false);
                }
                window.focus(&app.focus);
                app.handle(Action::VideoSentenceRepeat(Some(2), true), window, cx);
                app.handle_video_arrow_key(&Self::key("right", false), window, cx);
                let original = app.video.series.as_ref().unwrap().base.clone();
                app.video.series.as_mut().unwrap().base =
                    "https://www.bilibili.com/video/BV1yy411c7mD".into();
                app.handle_video_arrow_key(&Self::key("right", false), window, cx);
                ensure!(
                    app.video.path.as_ref() == Some(&path),
                    "选集变化后确认了旧目标"
                );
                app.video.series.as_mut().unwrap().base = original;
                app.handle(Action::VideoCancelPart, window, cx);
                app.handle_video_arrow_key(&Self::key("right", false), window, cx);
                app.inputs["question"].update(cx, |state, cx| state.focus(window, cx));
                ensure!(
                    !app.handle_video_arrow_key(&Self::key("right", false), window, cx),
                    "编辑输入时触发了切P"
                );
                window.focus(&app.focus);
                app.handle(Action::VideoCancelPart, window, cx);
                self.record(app, "无字幕/任务忙/切页/换选集/输入焦点不会误切下一P")?;
                let mut cues = app.video.cues.clone();
                cues[2].end = cues[2].start + 0.04;
                app.video.set_cues(cues);
                app.handle(Action::VideoSentenceRepeat(Some(2), true), window, cx);
                self.advance();
            }
            15 => {
                let state = app.media.player.snapshot();
                if !state.paused || !state.sentence_repeat.as_ref().is_some_and(|s| s.finished) {
                    return Ok(false);
                }
                window.focus(&app.focus);
                app.handle_video_arrow_key(&Self::key("right", false), window, cx);
                ensure!(
                    app.video.part_confirmation.is_some(),
                    "三遍结束后无法弹确认"
                );
                app.handle_video_arrow_key(&Self::key("right", false), window, cx);
                ensure!(
                    app.video.part_confirmation.is_none()
                        && app.video.path.as_ref() == Some(&other_video(&app.lib)),
                    "第二次右键未切缓存下一P"
                );
                ensure!(
                    app.video.sentence_repeat.is_none() && app.video.repeat_three,
                    "切P未区分旧循环与三次模式"
                );
                self.advance();
            }
            16 => {
                let state = app.media.player.snapshot();
                if state.path != other_video(&app.lib).to_string_lossy()
                    || state.duration < 11.9
                    || !app
                        .video
                        .sentence_repeat
                        .is_some_and(|s| s.three && s.index == 0)
                    || !state
                        .sentence_repeat
                        .as_ref()
                        .is_some_and(|s| s.total_plays == Some(3))
                {
                    return Ok(false);
                }
                ensure!(
                    app.video
                        .sentence_repeat
                        .is_some_and(|s| s.range == (0.3, 0.34)),
                    "新P套用了上一P的范围或没有跳过无效首句"
                );
                ensure!(
                    app.video.series.as_ref().and_then(|s| s.current) == Some(1),
                    "实际播放器与选集索引不一致"
                );
                self.record(app, "末句播完三遍/再次独立右键实际打开并播放缓存下一P")?;
                self.record(app, "新P异步载入后继承三次/第一条有效字幕/新请求范围")?;
                app.handle(Action::VideoStopSentenceRepeat, window, cx);
                app.video.set_cues(vec![Cue {
                    start: 0.,
                    end: 12.,
                    text: "Last part.".into(),
                }]);
                window.focus(&app.focus);
                app.handle_video_arrow_key(&Self::key("right", false), window, cx);
                app.handle_video_arrow_key(&Self::key("right", false), window, cx);
                ensure!(
                    app.video.part_confirmation.is_none()
                        && app.video.path.as_ref() == Some(&other_video(&app.lib)),
                    "末P误弹下一P或越界"
                );
                self.record(app, "整个选集最后P不弹无效确认/不越界")?;
                app.handle(Action::VideoSentenceRepeat(Some(0), true), window, cx);
                ensure!(app.open(path.clone()), "快速切视频失败");
                ensure!(app.open(other_video(&app.lib)), "快速切回下一视频失败");
                app.handle(Action::Play, window, cx);
                self.advance();
            }
            17 => {
                let state = app.media.player.snapshot();
                if state.path != other_video(&app.lib).to_string_lossy()
                    || !app
                        .video
                        .sentence_repeat
                        .is_some_and(|s| s.three && s.index == 0)
                    || !state
                        .sentence_repeat
                        .as_ref()
                        .is_some_and(|s| s.total_plays == Some(3))
                {
                    return Ok(false);
                }
                ensure!(
                    state.paused && app.video.repeat_three,
                    "准备期间暂停被自动播放覆盖"
                );
                self.record(app, "快速换片只采用当前字幕/暂停意图保留三次")?;
                app.handle(Action::VideoStopSentenceRepeat, window, cx);
                ensure!(!app.video.repeat_three, "停止后模式仍选中");
                app.handle(Action::SelectPart(0), window, cx);
                self.advance();
            }
            18 => {
                if app.media.player.snapshot().path != path.to_string_lossy()
                    || app.video.cues.len() != 3
                    || !self.stable()
                {
                    return Ok(false);
                }
                ensure!(
                    !app.video.repeat_three && app.video.sentence_repeat.is_none(),
                    "关闭后切P重新打开三次"
                );
                self.record(app, "主动停止后切P不恢复三次")?;
                app.handle(Action::VideoSentenceRepeat(Some(0), true), window, cx);
                fs::write(app.lib.setting("inherit_fixture_subtitle")?, "")?;
                ensure!(app.open(other_video(&app.lib)), "无字幕夹具无法打开");
                self.advance();
            }
            19 => {
                let state = app.media.player.snapshot();
                if state.path != other_video(&app.lib).to_string_lossy() || !self.stable() {
                    return Ok(false);
                }
                ensure!(
                    app.video.repeat_three && app.video.sentence_repeat.is_none() && state.paused,
                    "无字幕没有保留等待模式/错误播放旧循环"
                );
                self.record(app, "无字幕下一视频暂停等待/模式保留")?;
                video_subtitles::save(
                    &app.lib,
                    &other_video(&app.lib),
                    &[
                        Cue {
                            start: 0.4,
                            end: 0.44,
                            text: "Arrived later".into(),
                        },
                        Cue {
                            start: 1.2,
                            end: 1.24,
                            text: "Later last".into(),
                        },
                    ],
                    &process::cancel_token(),
                )?;
                app.load_video_subtitles(&other_video(&app.lib));
                self.advance();
            }
            20 => {
                let state = app.media.player.snapshot();
                if !app
                    .video
                    .sentence_repeat
                    .is_some_and(|s| s.three && s.range == (0.4, 0.44))
                    || !state
                        .sentence_repeat
                        .as_ref()
                        .is_some_and(|s| s.total_plays == Some(3))
                {
                    return Ok(false);
                }
                self.record(app, "字幕稍后载入自动从首个有效句恢复三次")?;
                let series = app.video.series.clone().context("异步夹具无选集")?;
                let origin = app.media.path.clone();
                let target = path.clone();
                app.begin("本机异步切P夹具", move |_, _, _| {
                    Ok(Outcome::PartVideo(series, 0, target, false, origin, false))
                });
                self.advance();
            }
            21 => {
                let state = app.media.player.snapshot();
                if app.tasks.cancel.is_some()
                    || state.path != path.to_string_lossy()
                    || !app
                        .video
                        .sentence_repeat
                        .is_some_and(|s| s.three && s.index == 0)
                    || !state
                        .sentence_repeat
                        .as_ref()
                        .is_some_and(|s| s.total_plays == Some(3))
                {
                    return Ok(false);
                }
                self.record(app, "异步PartVideo结果到达后仍建立新视频三次循环")?;
                let series = app.video.series.clone().unwrap();
                let origin = app.media.path.clone();
                let target = other_video(&app.lib);
                app.begin("本机取消结果夹具", move |_, _, _| {
                    Ok(Outcome::PartVideo(series, 1, target, false, origin, false))
                });
                app.tasks
                    .cancel
                    .as_ref()
                    .unwrap()
                    .store(true, Ordering::SeqCst);
                self.advance();
            }
            22 => {
                if app.tasks.cancel.is_some() {
                    return Ok(false);
                }
                ensure!(
                    app.video.repeat_three && app.video.path.as_ref() == Some(&path),
                    "取消异步切P清掉原模式或切错视频"
                );
                self.record(app, "取消异步切P保留原视频和三次模式")?;
                let series = app.video.series.clone().unwrap();
                let origin = app.media.path.clone();
                let target = other_video(&app.lib);
                app.begin("本机迟到结果夹具", move |_, _, _| {
                    Ok(Outcome::PartVideo(series, 1, target, false, origin, false))
                });
                app.handle(Action::VideoStopSentenceRepeat, window, cx);
                self.advance();
            }
            23 => {
                let state = app.media.player.snapshot();
                if app.tasks.cancel.is_some()
                    || state.path != other_video(&app.lib).to_string_lossy()
                    || !self.stable()
                {
                    return Ok(false);
                }
                ensure!(
                    !app.video.repeat_three && app.video.sentence_repeat.is_none(),
                    "异步迟到结果恢复了用户关闭的模式"
                );
                self.record(app, "准备期间主动停止三次不被异步结果重新打开")?;
                window.focus(&app.focus);
                app.handle(Action::VideoSentenceRepeat(Some(0), true), window, cx);
                app.handle_video_arrow_key(&Self::key("left", false), window, cx);
                ensure!(
                    app.video.part_confirmation.is_some()
                        && app.video.path.as_ref() == Some(&other_video(&app.lib)),
                    "首句第一次左键未只提示上一P"
                );
                for _ in 0..4 {
                    app.handle_video_arrow_key(&Self::key("left", true), window, cx);
                }
                ensure!(
                    app.video.part_confirmation.is_some()
                        && app.video.path.as_ref() == Some(&other_video(&app.lib)),
                    "左键长按误切上一P"
                );
                self.record(app, "首句左键提示上一P/长按不确认/三次模式保留")?;
                app.handle_video_arrow_key(&Self::key("escape", false), window, cx);
                ensure!(app.video.part_confirmation.is_none(), "左侧确认Esc未取消");
                app.handle_video_arrow_key(&Self::key("left", false), window, cx);
                app.handle_video_arrow_key(&Self::key("right", false), window, cx);
                ensure!(
                    app.video.part_confirmation.is_none()
                        && app.video.sentence_repeat.is_some_and(|s| s.index == 1),
                    "右键未取消上一P确认并切下一句"
                );
                app.handle_video_arrow_key(&Self::key("left", false), window, cx);
                ensure!(
                    app.video.sentence_repeat.is_some_and(|s| s.index == 0)
                        && app.video.part_confirmation.is_none(),
                    "普通左键误弹跨P确认"
                );
                self.record(app, "上一P确认反向取消/非首句左键仍切上一句")?;
                app.tasks.cancel = Some(process::cancel_token());
                app.handle_video_arrow_key(&Self::key("left", false), window, cx);
                ensure!(app.video.part_confirmation.is_none(), "任务忙仍弹上一P确认");
                app.tasks.cancel = None;
                app.handle(Action::SeekTo(0.), window, cx);
                app.media.player.set("pause", "yes");
                self.advance();
            }
            24 => {
                if !Self::at(app, &other_video(&app.lib), 0.) {
                    return Ok(false);
                }
                window.focus(&app.focus);
                app.handle_video_arrow_key(&Self::key("left", false), window, cx);
                ensure!(
                    app.video.part_confirmation.is_some(),
                    "首句之前静音没有上一P入口"
                );
                app.handle_video_arrow_key(&Self::key("left", false), window, cx);
                ensure!(
                    app.video.path.as_ref() == Some(&path) && app.video.part_confirmation.is_none(),
                    "第二次左键未回上一P"
                );
                self.advance();
            }
            25 => {
                if app.media.player.snapshot().path != path.to_string_lossy()
                    || app.video.cues.len() != 3
                    || !self.stable()
                {
                    return Ok(false);
                }
                ensure!(!app.video.repeat_three, "普通模式回上一P误启用三次");
                ensure!(
                    app.media.player.snapshot().position >= 7.95
                        && app.media.player.snapshot().position < 10.,
                    "普通模式回上一P没有从末句8秒开头播放"
                );
                self.record(app, "普通模式两次左键回上一P从最后一句播放")?;
                app.handle(Action::VideoSentenceRepeat(Some(0), true), window, cx);
                window.focus(&app.focus);
                app.handle_video_arrow_key(&Self::key("left", false), window, cx);
                app.handle_video_arrow_key(&Self::key("left", false), window, cx);
                ensure!(
                    app.video.part_confirmation.is_none() && app.video.path.as_ref() == Some(&path),
                    "整个选集首P左键越界"
                );
                self.record(app, "整个选集第一P左键不越界")?;
                app.handle(Action::VideoStopSentenceRepeat, window, cx);
                app.handle(Action::SelectPart(1), window, cx);
                self.advance();
            }
            26 => {
                if app.media.player.snapshot().path != other_video(&app.lib).to_string_lossy()
                    || app.video.cues.len() != 2
                {
                    return Ok(false);
                }
                app.handle(Action::VideoSentenceRepeat(Some(0), true), window, cx);
                window.focus(&app.focus);
                app.handle_video_arrow_key(&Self::key("left", false), window, cx);
                app.handle_video_arrow_key(&Self::key("left", false), window, cx);
                self.advance();
            }
            27 => {
                let state = app.media.player.snapshot();
                if state.path != path.to_string_lossy()
                    || !app
                        .video
                        .sentence_repeat
                        .is_some_and(|s| s.three && s.index == 2 && s.range == (8., 12.))
                    || !state
                        .sentence_repeat
                        .as_ref()
                        .is_some_and(|s| s.total_plays == Some(3))
                {
                    return Ok(false);
                }
                ensure!(
                    app.video.series.as_ref().and_then(|s| s.current) == Some(0),
                    "上一P媒体与序号不一致"
                );
                self.record(app, "三次模式左键回上一P从末句建立三次循环")?;
                let series = app.video.series.clone().unwrap();
                let origin = app.media.path.clone();
                let target = other_video(&app.lib);
                app.begin("异步末句夹具", move |_, _, _| {
                    Ok(Outcome::PartVideo(series, 1, target, false, origin, true))
                });
                self.advance();
            }
            28 => {
                let state = app.media.player.snapshot();
                if app.tasks.cancel.is_some()
                    || state.path != other_video(&app.lib).to_string_lossy()
                    || !app
                        .video
                        .sentence_repeat
                        .is_some_and(|s| s.three && s.index == 1 && s.range == (1.2, 1.24))
                    || !state
                        .sentence_repeat
                        .as_ref()
                        .is_some_and(|s| s.total_plays == Some(3))
                {
                    return Ok(false);
                }
                self.record(app, "异步PartVideo结果携带末句意图/三次重新计数")?;
                app.handle(Action::VideoStopSentenceRepeat, window, cx);
                let series = app.video.series.clone().unwrap();
                let origin = app.media.path.clone();
                let target = path.clone();
                app.begin("普通异步末句夹具", move |_, _, _| {
                    Ok(Outcome::PartVideo(series, 0, target, false, origin, true))
                });
                self.advance();
            }
            29 => {
                let state = app.media.player.snapshot();
                if app.tasks.cancel.is_some()
                    || state.path != path.to_string_lossy()
                    || !self.stable()
                {
                    return Ok(false);
                }
                ensure!(
                    !app.video.repeat_three && state.position >= 7.95 && state.position < 10.,
                    "普通异步末句定位错误"
                );
                self.record(app, "普通模式异步切P也从末句开头播放")?;
                let srt = video_subtitles::save(
                    &app.lib,
                    &other_video(&app.lib),
                    &[Cue {
                        start: 0.,
                        end: 1.,
                        text: "Temporary".into(),
                    }],
                    &process::cancel_token(),
                )?;
                fs::write(srt, "")?;
                app.handle(Action::SelectPartAtEnd(1), window, cx);
                self.advance();
            }
            30 => {
                let state = app.media.player.snapshot();
                if state.path != other_video(&app.lib).to_string_lossy() || !self.stable() {
                    return Ok(false);
                }
                ensure!(
                    state.paused
                        && !app.video.repeat_three
                        && app.video.pending_start.as_ref().is_some_and(|p| p.from_end),
                    "无字幕时未暂停等待末句"
                );
                self.record(app, "普通模式无字幕上一P等待末句定位")?;
                app.handle(Action::Play, window, cx);
                video_subtitles::save(
                    &app.lib,
                    &other_video(&app.lib),
                    &[
                        Cue {
                            start: 0.2,
                            end: 0.3,
                            text: "First".into(),
                        },
                        Cue {
                            start: 2.,
                            end: 2.1,
                            text: "Last valid".into(),
                        },
                        Cue {
                            start: 13.,
                            end: 14.,
                            text: "Outside duration".into(),
                        },
                    ],
                    &process::cancel_token(),
                )?;
                app.load_video_subtitles(&other_video(&app.lib));
                self.advance();
            }
            31 => {
                if !Self::at(app, &other_video(&app.lib), 2.) || app.video.pending_start.is_some() {
                    return Ok(false);
                }
                ensure!(!app.video.repeat_three, "普通末句定位变成三次");
                self.record(app, "字幕迟到后定位最后有效句/跳过越界尾句/暂停意图保留")?;
                ensure!(app.open_part_video(path.clone(), true), "末句夹具打开失败");
                ensure!(app.open(other_video(&app.lib)), "快速换片失败");
                self.advance();
            }
            32 => {
                let state = app.media.player.snapshot();
                if state.path != other_video(&app.lib).to_string_lossy() || !self.stable() {
                    return Ok(false);
                }
                ensure!(
                    app.video.pending_start.is_none() && state.position < 2.,
                    "快速换片被旧末句定位覆盖"
                );
                self.record(app, "快速换片取消旧末句定位/普通切集仍从头播放")?;
                app.handle(Action::Page("settings".into()), window, cx);
                ensure!(
                    !app.video.repeat_three && app.video.sentence_repeat.is_none(),
                    "离开视频后延迟恢复仍在运行"
                );
                self.report(Some(true), None)?;
                self.advance();
                return Ok(true);
            }
            _ => return Ok(true),
        }
        Ok(false)
    }
    fn key(key: &str, is_held: bool) -> KeyDownEvent {
        KeyDownEvent {
            keystroke: Keystroke::parse(key).expect("probe key"),
            is_held,
        }
    }
}
