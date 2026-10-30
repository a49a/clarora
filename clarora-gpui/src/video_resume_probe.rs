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
    assistant_before_keyboard: Option<(bool, bool, bool, String)>,
    keyboard_position: f64,
    timeline_request: u64,
}
impl Probe {
    pub(super) fn new(out: &Path) -> Self {
        Self {
            out: out.into(),
            stage: 0,
            entered: Instant::now(),
            cases: vec![],
            assistant_before_keyboard: None,
            keyboard_position: 0.,
            timeline_request: 0,
        }
    }
    fn advance(&mut self) {
        self.stage += 1;
        self.entered = Instant::now();
    }
    fn record(&mut self, app: &AppView, name: &str) -> Result<()> {
        let state = app.media.player.snapshot();
        self.cases.push(json!({"case":name,"passed":true,"path":state.path,"position":state.position,"duration":state.duration,"paused":state.paused,"error":state.error,"page":app.page,"bookmark":app.lib.setting("last_video_position")?,"part_confirmation":app.video.part_confirmation.is_some(),"series_current":app.video.series.as_ref().and_then(|s| s.current),"triple_sentence":app.video.sentence_repeat.filter(|s| s.three).map(|s| s.index),"repeat_three":app.video.repeat_three,"pending_video_start":app.video.pending_start.as_ref().map(|p|json!({"path":p.path,"paused":p.paused,"from_end":p.from_end,"selected_time":p.selected_time}))}));
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
                ensure!(
                    app.video.repeat_three
                        && app.video.sentence_repeat.is_none()
                        && app.video.pending_start.is_none(),
                    "切栏目应保留三次选择并停止当前视频请求"
                );
                app.handle(Action::Page("video".into()), window, cx);
                app.handle(Action::VideoStopSentenceRepeat, window, cx);
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
                ensure!(app.open(path.clone()), "音轨夹具打开失败");
                app.media.player.set("pause", "yes");
                self.advance();
            }
            33 => {
                if !Self::at(app, &path, 0.) || app.video.cues.len() != 3 {
                    return Ok(false);
                }
                app.handle(Action::VideoSentenceRepeat(Some(1), true), window, cx);
                self.advance();
            }
            34 => {
                let state = app.media.player.snapshot();
                if state.paused
                    || !state
                        .sentence_repeat
                        .as_ref()
                        .is_some_and(|s| s.start == 4. && s.end == 8. && s.total_plays == Some(3))
                {
                    return Ok(false);
                }
                app.video.seek_dragging = true;
                app.video.seek_drag_target = None;
                self.timeline_request = app
                    .video
                    .sentence_repeat
                    .context("当前句请求缺失")?
                    .request_id;
                app.handle(Action::VideoTimelineSeek(6.6), window, cx);
                let request = app
                    .video
                    .sentence_repeat
                    .context("音轨点击清掉三次")?
                    .request_id;
                ensure!(
                    app.video.repeat_three
                        && app
                            .video
                            .sentence_repeat
                            .is_some_and(|s| s.index == 1 && s.range == (4., 8.)),
                    "句内音轨定位改变了当前句或模式"
                );
                ensure!(request == self.timeline_request, "句内定位重建了三次请求");
                for time in [6.7, 6.9, 7.1] {
                    app.handle(Action::VideoTimelineSeek(time), window, cx);
                }
                ensure!(
                    app.video.sentence_repeat.unwrap().request_id == request,
                    "同句拖动反复重置循环"
                );
                app.video.seek_dragging = false;
                app.video.seek_drag_target = None;
                self.advance();
            }
            35 => {
                let state = app.media.player.snapshot();
                if state.paused
                    || state.position < 7.0
                    || state.position > 7.6
                    || !state.sentence_repeat.as_ref().is_some_and(|s| {
                        s.request_id == self.timeline_request
                            && s.start == 4.
                            && s.end == 8.
                            && s.total_plays == Some(3)
                            && s.current_play == 1
                    })
                {
                    return Ok(false);
                }
                self.record(
                    app,
                    "播放中句内精确定位/连续拖动使用最后点/保留同一三次请求",
                )?;
                app.handle(Action::Play, window, cx);
                self.advance();
            }
            36 => {
                if !app.media.player.snapshot().paused {
                    return Ok(false);
                }
                app.handle(Action::VideoTimelineSeek(9.1), window, cx);
                self.advance();
            }
            37 => {
                let state = app.media.player.snapshot();
                if !state.paused
                    || state.position < 7.9
                    || state.position >= 8.
                    || !state.sentence_repeat.as_ref().is_some_and(|s| {
                        s.request_id == self.timeline_request
                            && s.start == 4.
                            && s.end == 8.
                            && s.total_plays == Some(3)
                            && s.current_play == 1
                    })
                {
                    return Ok(false);
                }
                ensure!(app.video.repeat_three, "暂停定位丢模式");
                self.record(app, "暂停时点击后句吸附当前句尾/不切句不换请求")?;
                app.handle(Action::VideoTimelineSeek(0.), window, cx);
                self.advance();
            }
            38 => {
                let state = app.media.player.snapshot();
                if !state.paused
                    || (state.position - 4.).abs() > 0.18
                    || !state.sentence_repeat.as_ref().is_some_and(|s| {
                        s.request_id == self.timeline_request && s.start == 4. && s.end == 8.
                    })
                {
                    return Ok(false);
                }
                self.record(app, "三次模式点击当前句之前吸附本句句首")?;
                app.handle(Action::VideoTimelineSeek(12.), window, cx);
                self.advance();
            }
            39 => {
                let state = app.media.player.snapshot();
                if !state.paused
                    || state.position < 7.9
                    || state.position >= 8.
                    || !state.sentence_repeat.as_ref().is_some_and(|s| {
                        s.request_id == self.timeline_request && s.start == 4. && s.end == 8.
                    })
                {
                    return Ok(false);
                }
                self.record(app, "三次模式点击全视频终点仍吸附当前句尾")?;
                app.reset_video_sentence_request();
                app.video.set_cues(vec![
                    Cue {
                        start: 0.,
                        end: 1.,
                        text: "First".into(),
                    },
                    Cue {
                        start: 3.,
                        end: 4.,
                        text: "After gap".into(),
                    },
                    Cue {
                        start: 7.,
                        end: 8.,
                        text: "Last valid".into(),
                    },
                    Cue {
                        start: 13.,
                        end: 14.,
                        text: "Outside duration".into(),
                    },
                ]);
                app.handle(Action::VideoSentenceRepeat(Some(1), true), window, cx);
                self.timeline_request = app
                    .video
                    .sentence_repeat
                    .context("稀疏字幕请求缺失")?
                    .request_id;
                app.media.player.set("pause", "yes");
                app.handle(Action::VideoTimelineSeek(2.), window, cx);
                self.advance();
            }
            40 => {
                let state = app.media.player.snapshot();
                if !state.paused
                    || (state.position - 3.).abs() > 0.18
                    || !state.sentence_repeat.as_ref().is_some_and(|s| {
                        s.request_id == self.timeline_request
                            && s.start == 3.
                            && s.end == 4.
                            && s.total_plays == Some(3)
                    })
                {
                    return Ok(false);
                }
                self.record(app, "点击句间空白吸附当前句首/不改变当前句")?;
                app.handle(Action::VideoTimelineSeek(12.), window, cx);
                self.advance();
            }
            41 => {
                let state = app.media.player.snapshot();
                if !state.paused
                    || state.position < 3.9
                    || state.position >= 4.
                    || !state.sentence_repeat.as_ref().is_some_and(|s| {
                        s.request_id == self.timeline_request
                            && s.start == 3.
                            && s.end == 4.
                            && s.total_plays == Some(3)
                    })
                {
                    return Ok(false);
                }
                self.record(app, "音轨末尾点击不切入后句或越界字幕/吸附当前句尾")?;
                ensure!(
                    app.handle_video_arrow_key(&Self::key("right", false), window, cx),
                    "精确音轨定位后方向键未交给视频"
                );
                ensure!(
                    app.video.repeat_three
                        && app
                            .video
                            .sentence_repeat
                            .is_some_and(|s| s.index == 2 && s.range == (7., 8.)),
                    "音轨仅句内定位后方向键不能正常切换字幕句"
                );
                self.record(app, "音轨仅限当前句/方向键仍能切下一句并保留三次")?;
                app.handle(Action::VideoStopSentenceRepeat, window, cx);
                // Keyboard sentence stepping starts playback. The following
                // ordinary-seek case uses an explicitly paused fixture.
                app.media.player.set("pause", "yes");
                app.handle(Action::VideoTimelineSeek(5.5), window, cx);
                self.advance();
            }
            42 => {
                if !Self::at(app, &path, 5.5) {
                    return Ok(false);
                }
                ensure!(
                    !app.video.repeat_three && app.video.sentence_repeat.is_none(),
                    "普通音轨定位误启用三次"
                );
                self.record(app, "普通模式音轨仍精确定位到点击时间")?;
                app.handle(Action::VideoSentenceRepeat(Some(0), true), window, cx);
                app.reset_video_sentence_request();
                app.video.set_cues(vec![]);
                app.media.player.set("pause", "yes");
                app.media.player.command([
                    "seek".into(),
                    POSITION.to_string(),
                    "absolute+exact".into(),
                ]);
                self.advance();
            }
            43 => {
                if !Self::at(app, &path, POSITION) {
                    return Ok(false);
                }
                app.handle(Action::VideoTimelineSeek(10.), window, cx);
                ensure!(
                    app.video.repeat_three
                        && app.video.pending_start.as_ref().is_some_and(|p| p
                            .selected_time
                            .is_some_and(|t| (t - POSITION).abs() < 0.18)),
                    "未就绪音轨定位把点击点当成当前句锚点"
                );
                self.record(app, "字幕未就绪时保留原位置作为当前句锚点/单独等待点击目标")?;
                app.video.set_cues(vec![
                    Cue {
                        start: 1.,
                        end: 2.,
                        text: "First".into(),
                    },
                    Cue {
                        start: 6.,
                        end: 7.,
                        text: "Selected later".into(),
                    },
                    Cue {
                        start: 9.,
                        end: 10.,
                        text: "Last".into(),
                    },
                ]);
                self.advance();
            }
            44 => {
                let state = app.media.player.snapshot();
                if !state.paused
                    || state.position < 6.9
                    || state.position >= 7.
                    || !state.sentence_repeat.as_ref().is_some_and(|s| {
                        s.start == 6.
                            && s.end == 7.
                            && s.total_plays == Some(3)
                            && s.current_play == 1
                            && app.video.sentence_repeat.is_some_and(|session| {
                                session.index == 1 && session.request_id == s.request_id
                            })
                    })
                {
                    return Ok(false);
                }
                self.record(app, "迟到字幕仍选原句/将句外点击吸附本句尾并保留暂停")?;
                app.handle(Action::VideoStopSentenceRepeat, window, cx);
                app.video.set_cues(vec![
                    Cue {
                        start: 1.,
                        end: 2.,
                        text: "First after silence".into(),
                    },
                    Cue {
                        start: 4.,
                        end: 5.,
                        text: "After a gap".into(),
                    },
                    Cue {
                        start: 8.,
                        end: 9.,
                        text: "Last valid".into(),
                    },
                    Cue {
                        start: 13.,
                        end: 14.,
                        text: "Outside video".into(),
                    },
                ]);
                app.handle(Action::SeekTo(0.), window, cx);
                self.advance();
            }
            45 => {
                if !Self::at(app, &path, 0.) || app.video_repeat_cue().is_some() {
                    return Ok(false);
                }
                app.handle(Action::VideoSentenceRepeat(None, true), window, cx);
                ensure!(
                    app.video.repeat_three
                        && app.video.sentence_repeat.is_some_and(|s| s.index == 0),
                    "未选句时无法从首条字幕开启三次模式"
                );
                self.advance();
            }
            46 => {
                let state = app.media.player.snapshot();
                if state.paused
                    || !state
                        .sentence_repeat
                        .as_ref()
                        .is_some_and(|s| s.start == 1. && s.end == 2. && s.total_plays == Some(3))
                {
                    return Ok(false);
                }
                self.record(app, "首句前无需选中字幕即可开启三次播放")?;
                app.handle(Action::VideoStopSentenceRepeat, window, cx);
                app.media.player.set("pause", "yes");
                app.handle(Action::SeekTo(4.4), window, cx);
                self.advance();
            }
            47 => {
                if !Self::at(app, &path, 4.4) {
                    return Ok(false);
                }
                app.video.active = None;
                app.handle(Action::VideoSentenceRepeat(None, true), window, cx);
                ensure!(
                    app.video.sentence_repeat.is_some_and(|s| s.index == 1),
                    "未按真实时钟选择当前句"
                );
                self.advance();
            }
            48 => {
                let state = app.media.player.snapshot();
                if state.paused
                    || !state
                        .sentence_repeat
                        .as_ref()
                        .is_some_and(|s| s.start == 4. && s.end == 5. && s.total_plays == Some(3))
                {
                    return Ok(false);
                }
                self.record(app, "未点选字幕时从当前播放句开启三次")?;
                app.handle(Action::VideoStopSentenceRepeat, window, cx);
                app.media.player.set("pause", "yes");
                app.handle(Action::SeekTo(2.5), window, cx);
                self.advance();
            }
            49 => {
                if !Self::at(app, &path, 2.5) || app.video_repeat_cue().is_some() {
                    return Ok(false);
                }
                app.handle(Action::VideoSentenceRepeat(None, true), window, cx);
                self.advance();
            }
            50 => {
                let state = app.media.player.snapshot();
                if state.paused
                    || !state
                        .sentence_repeat
                        .as_ref()
                        .is_some_and(|s| s.start == 4. && s.end == 5. && s.total_plays == Some(3))
                {
                    return Ok(false);
                }
                self.record(app, "句间空白无需选句即可从下一有效句开启三次")?;
                app.handle(Action::VideoStopSentenceRepeat, window, cx);
                app.media.player.set("pause", "yes");
                app.handle(Action::SeekTo(11.5), window, cx);
                self.advance();
            }
            51 => {
                if !Self::at(app, &path, 11.5) {
                    return Ok(false);
                }
                app.handle(Action::VideoSentenceRepeat(None, true), window, cx);
                self.advance();
            }
            52 => {
                let state = app.media.player.snapshot();
                if state.paused
                    || !state
                        .sentence_repeat
                        .as_ref()
                        .is_some_and(|s| s.start == 8. && s.end == 9. && s.total_plays == Some(3))
                {
                    return Ok(false);
                }
                self.record(app, "字幕末尾直接开启三次取最后有效句并跳过越界字幕")?;
                app.handle(Action::VideoStopSentenceRepeat, window, cx);
                app.video.set_cues(vec![]);
                app.media.player.set("pause", "yes");
                app.handle(Action::SeekTo(6.4), window, cx);
                self.advance();
            }
            53 => {
                if !Self::at(app, &path, 6.4) {
                    return Ok(false);
                }
                app.handle(Action::VideoSentenceRepeat(None, true), window, cx);
                ensure!(
                    app.video.repeat_three
                        && app.video.sentence_repeat.is_none()
                        && app.video.pending_start.as_ref().is_some_and(|p| {
                            p.path == path
                                && p.selected_time.is_some_and(|t| (t - 6.4).abs() < 0.18)
                                && !p.paused
                        }),
                    "无字幕开启没有保留模式和当前播放位置"
                );
                self.record(app, "无字幕可先开启三次模式并等待字幕/不启动ASR")?;
                ensure!(app.tasks.cancel.is_none(), "选择模式误触发后台任务");
                app.handle(Action::VideoStopSentenceRepeat, window, cx);
                app.video.set_cues(vec![Cue {
                    start: 6.,
                    end: 7.,
                    text: "Late after stop".into(),
                }]);
                self.advance();
            }
            54 => {
                if !self.stable() || !app.media.player.snapshot().paused {
                    return Ok(false);
                }
                ensure!(
                    !app.video.repeat_three
                        && app.video.pending_start.is_none()
                        && app.video.sentence_repeat.is_none(),
                    "停止后迟到字幕重新启用了三次"
                );
                self.record(app, "等待字幕期间可停止且迟到字幕不重启")?;
                app.video.set_cues(vec![]);
                app.handle(Action::VideoSentenceRepeat(None, true), window, cx);
                app.video.set_cues(vec![
                    Cue {
                        start: 1.,
                        end: 2.,
                        text: "First".into(),
                    },
                    Cue {
                        start: 6.,
                        end: 7.,
                        text: "Current when enabled".into(),
                    },
                    Cue {
                        start: 9.,
                        end: 10.,
                        text: "Last".into(),
                    },
                ]);
                self.advance();
            }
            55 => {
                let state = app.media.player.snapshot();
                if state.paused
                    || !state
                        .sentence_repeat
                        .as_ref()
                        .is_some_and(|s| s.start == 6. && s.end == 7. && s.total_plays == Some(3))
                {
                    return Ok(false);
                }
                self.record(app, "迟到字幕按开启时的位置自动开始三次")?;
                app.handle(Action::VideoStopSentenceRepeat, window, cx);
                ensure!(app.open(other_video(&app.lib)), "未就绪视频夹具打开失败");
                app.handle(Action::VideoSentenceRepeat(None, true), window, cx);
                ensure!(
                    app.video.repeat_three
                        && app
                            .video
                            .pending_start
                            .as_ref()
                            .is_some_and(|p| p.path == other_video(&app.lib)),
                    "媒体未就绪时不能直接开启三次"
                );
                self.advance();
            }
            56 => {
                let state = app.media.player.snapshot();
                if state.path != other_video(&app.lib).to_string_lossy()
                    || !app
                        .video
                        .sentence_repeat
                        .is_some_and(|s| s.three && s.range.1 < 2.)
                    || !state
                        .sentence_repeat
                        .as_ref()
                        .is_some_and(|s| s.total_plays == Some(3))
                {
                    return Ok(false);
                }
                self.record(app, "视频未就绪时直接开启/字幕载入后只循环新视频句子")?;
                app.handle(Action::Page("settings".into()), window, cx);
                ensure!(
                    app.video.repeat_three
                        && app.video.sentence_repeat.is_none()
                        && app.video.pending_start.is_none(),
                    "离开视频没有保留三次偏好或留下运行请求"
                );
                app.handle(Action::VideoStopSentenceRepeat, window, cx);
                app.handle(Action::Page("video".into()), window, cx);
                ensure!(app.open(path.clone()), "页面保留夹具打开失败");
                app.media.player.set("pause", "yes");
                self.advance();
            }
            57 => {
                if !Self::at(app, &path, 0.) || app.video.cues.len() != 3 {
                    return Ok(false);
                }
                app.handle(Action::SeekTo(POSITION), window, cx);
                self.advance();
            }
            58 => {
                if !Self::at(app, &path, POSITION) {
                    return Ok(false);
                }
                app.handle(Action::VideoSentenceRepeat(Some(1), true), window, cx);
                app.media.player.set("pause", "yes");
                self.advance();
            }
            59 => {
                if !Self::paused_middle_repeat(app, &path) {
                    return Ok(false);
                }
                app.handle(Action::Page("settings".into()), window, cx);
                ensure!(
                    app.video.repeat_three
                        && app.video.sentence_repeat.is_none()
                        && app.video.pending_start.is_none(),
                    "三次模式离开到设置后被关闭或继续控制播放器"
                );
                app.handle(Action::Page("video".into()), window, cx);
                self.advance();
            }
            60 => {
                if !Self::paused_middle_repeat(app, &path) {
                    return Ok(false);
                }
                self.record(app, "三次模式设置往返保留原字幕句/暂停/第一遍")?;
                app.handle(Action::Page("meditation".into()), window, cx);
                ensure!(app.video.repeat_three, "进入冥想清掉三次偏好");
                app.handle(Action::Page("video".into()), window, cx);
                self.advance();
            }
            61 => {
                if !Self::paused_middle_repeat(app, &path) {
                    return Ok(false);
                }
                self.record(app, "三次模式冥想快速往返仍恢复原句并暂停")?;
                app.card_filter = "clip".into();
                app.handle(Action::Page("flashcard".into()), window, cx);
                self.advance();
            }
            62 => {
                let state = app.media.player.snapshot();
                if !app.cards_ready()
                    || app
                        .current_card()
                        .is_none_or(|c| c.id != "resume-same-video-clip")
                    || state.path != path.to_string_lossy()
                    || state.position < 0.95
                    || state.position > 2.2
                    || state.sentence_repeat.is_some()
                {
                    return Ok(false);
                }
                ensure!(
                    app.video.repeat_three
                        && app.video.sentence_repeat.is_none()
                        && app.video.pending_start.is_none(),
                    "闪卡播放丢失视频偏好或被视频循环接管"
                );
                self.record(app, "三次偏好保留/同源片段闪卡按自身范围播放无旧循环")?;
                app.handle(Action::Page("video".into()), window, cx);
                self.advance();
            }
            63 => {
                if !Self::paused_middle_repeat(app, &path) {
                    return Ok(false);
                }
                self.record(app, "同源片段闪卡返回视频恢复原句三次而非片段起点")?;
                app.handle(Action::Page("listening".into()), window, cx);
                self.advance();
            }
            64 => {
                let state = app.media.player.snapshot();
                if app.audio.current_id.is_empty()
                    || state.duration < 7.9
                    || state.duration > 8.1
                    || !state.paused
                    || state.sentence_repeat.is_some()
                    || app
                        .media
                        .path
                        .as_ref()
                        .is_none_or(|p| p.to_string_lossy() != state.path)
                {
                    return Ok(false);
                }
                app.handle(Action::SeekTo(4.25), window, cx);
                app.handle(Action::Repeat, window, cx);
                ensure!(
                    app.video.repeat_three
                        && app.video.sentence_repeat.is_none()
                        && app.video.pending_start.is_none(),
                    "音频SeekTo/Repeat改变了视频三次偏好或启动视频循环"
                );
                self.advance();
            }
            65 => {
                let state = app.media.player.snapshot();
                if (state.position - 4.25).abs() > 0.18 || !state.paused {
                    return Ok(false);
                }
                ensure!(state.sentence_repeat.is_none(), "视频循环施加到了音频");
                self.record(app, "音频定位与重复操作保持独立/不清视频三次偏好")?;
                app.handle(Action::Page("video".into()), window, cx);
                self.advance();
            }
            66 => {
                if !Self::paused_middle_repeat(app, &path) {
                    return Ok(false);
                }
                self.record(app, "音频返回视频仍恢复原句三次/保持暂停")?;
                app.handle(Action::VideoStopSentenceRepeat, window, cx);
                app.handle(Action::Page("meditation".into()), window, cx);
                app.handle(Action::Page("video".into()), window, cx);
                self.advance();
            }
            67 => {
                let state = app.media.player.snapshot();
                if !Self::at(app, &path, 4.) || !self.stable() {
                    return Ok(false);
                }
                ensure!(
                    !app.video.repeat_three
                        && app.video.sentence_repeat.is_none()
                        && app.video.pending_start.is_none()
                        && state.sentence_repeat.is_none(),
                    "明确停止后页面往返恢复了旧三次模式"
                );
                self.record(app, "明确停止三次后切页返回继续普通模式")?;
                app.handle(Action::SeekTo(POSITION), window, cx);
                let subtitle = video_subtitles::save(
                    &app.lib,
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
                fs::write(subtitle, "")?;
                app.load_video_subtitles(&path);
                self.advance();
            }
            68 => {
                if !Self::at(app, &path, POSITION) || !self.stable() || !app.video.cues.is_empty() {
                    return Ok(false);
                }
                app.handle(Action::VideoSentenceRepeat(None, true), window, cx);
                ensure!(
                    app.video.repeat_three && app.video.pending_start.is_some(),
                    "无字幕未进入三次等待"
                );
                app.handle(Action::Page("settings".into()), window, cx);
                ensure!(
                    app.video.repeat_three && app.video.pending_start.is_none(),
                    "无字幕离页未停止请求或丢失模式"
                );
                app.handle(Action::Page("video".into()), window, cx);
                self.advance();
            }
            69 => {
                if !Self::at(app, &path, POSITION) || !self.stable() || !app.video.cues.is_empty() {
                    return Ok(false);
                }
                ensure!(
                    app.video.repeat_three
                        && app.video.sentence_repeat.is_none()
                        && app
                            .video
                            .pending_start
                            .as_ref()
                            .is_some_and(|p| p.path == path && p.paused),
                    "无字幕页面往返没有保留三次等待/暂停意图"
                );
                self.record(app, "无字幕等待跨页后仍保持三次/只为当前视频暂停等待")?;
                video_subtitles::save(
                    &app.lib,
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
                app.load_video_subtitles(&path);
                self.advance();
            }
            70 => {
                if !Self::paused_middle_repeat(app, &path) {
                    return Ok(false);
                }
                self.record(app, "无字幕跨页后迟到字幕恢复原位置三次且保持暂停")?;
                app.handle(Action::VideoStopSentenceRepeat, window, cx);
                self.assistant_before_keyboard = Some((
                    app.assistant.enabled,
                    app.assistant_open,
                    app.assistant.legacy,
                    app.value("question", cx),
                ));
                app.open_assistant(window, cx);
                self.advance();
            }
            71 => {
                if app.chat.is_loading() || app.chat.context != clarora_gpui::assistant::CONTEXT {
                    return Ok(false);
                }
                app.set("question", "KeyboardProbe", window, cx);
                app.inputs["question"].update(cx, |state, cx| state.focus(window, cx));
                Self::dispatch_keys(window, cx, &["right", "left", "space"]);
                self.advance();
            }
            72 => {
                if !self.stable() {
                    return Ok(false);
                }
                ensure!(Self::at(app, &path, 4.), "输入框按键改变了视频播放");
                ensure!(
                    app.value("question", cx).contains(' '),
                    "空格没有进入真实输入框"
                );
                self.record(app, "真实键盘路由/问题输入框左右与空格不控制视频")?;
                app.inputs["question"].update(cx, |state, cx| {
                    state.replace_and_mark_text_in_range(None, "中文", None, window, cx);
                    ensure!(
                        state.marked_text_range(window, cx).is_some(),
                        "未建立IME组合文字"
                    );
                    Ok::<_, anyhow::Error>(())
                })?;
                Self::dispatch_keys(window, cx, &["right", "left", "space"]);
                self.advance();
            }
            73 => {
                if !self.stable() {
                    return Ok(false);
                }
                ensure!(Self::at(app, &path, 4.), "IME输入按键穿透到视频");
                self.record(app, "真实InputState组合文字期间方向键与空格不控制视频")?;
                app.inputs["question"].update(cx, |state, cx| state.unmark_text(window, cx));
                app.handle(Action::Assistant, window, cx);
                Self::dispatch_keys(window, cx, &["right"]);
                self.advance();
            }
            74 => {
                if !Self::at(app, &path, 8.) {
                    return Ok(false);
                }
                ensure!(!app.assistant_open, "助手未关闭");
                self.record(app, "收起聚焦问题框的助手后无需点视频/真实右键切句")?;
                Self::dispatch_keys(window, cx, &["left"]);
                self.advance();
            }
            75 => {
                if !Self::at(app, &path, 4.) {
                    return Ok(false);
                }
                self.record(app, "助手关闭后真实左键仍能切回上一句")?;
                Self::dispatch_keys(window, cx, &["space"]);
                self.advance();
            }
            76 => {
                if app.media.player.snapshot().paused {
                    return Ok(false);
                }
                Self::dispatch_keys(window, cx, &["space"]);
                self.advance();
            }
            77 => {
                if !app.media.player.snapshot().paused {
                    return Ok(false);
                }
                self.record(app, "助手关闭后真实空格可播放并再次暂停")?;
                app.open_assistant(window, cx);
                app.inputs["question"].update(cx, |state, cx| state.focus(window, cx));
                app.handle(Action::SeekTo(POSITION), window, cx);
                self.advance();
            }
            78 => {
                if !Self::at(app, &path, POSITION) {
                    return Ok(false);
                }
                Self::dispatch_keys(window, cx, &["right"]);
                self.advance();
            }
            79 => {
                if !Self::at(app, &path, 8.) {
                    return Ok(false);
                }
                self.record(app, "问题框聚焦后点击视频定位动作/真实键盘恢复切句")?;
                app.inputs["question"].update(cx, |state, cx| state.focus(window, cx));
                app.handle(Action::VideoSentenceRepeat(Some(1), true), window, cx);
                self.advance();
            }
            80 => {
                let state = app.media.player.snapshot();
                if state.paused
                    || !app
                        .video
                        .sentence_repeat
                        .is_some_and(|s| s.three && s.index == 1)
                {
                    return Ok(false);
                }
                Self::dispatch_keys(window, cx, &["right"]);
                self.advance();
            }
            81 => {
                if !app
                    .video
                    .sentence_repeat
                    .is_some_and(|s| s.three && s.index == 2)
                {
                    return Ok(false);
                }
                self.record(app, "问题框聚焦后点击逐句三次动作/真实右键切句保留模式")?;
                app.handle(Action::VideoStopSentenceRepeat, window, cx);
                app.inputs["question"].update(cx, |state, cx| state.focus(window, cx));
                app.handle(Action::VideoPlayCue(1), window, cx);
                self.advance();
            }
            82 => {
                if app.media.player.snapshot().paused {
                    return Ok(false);
                }
                Self::dispatch_keys(window, cx, &["space"]);
                self.advance();
            }
            83 => {
                if !app.media.player.snapshot().paused {
                    return Ok(false);
                }
                self.record(app, "问题框聚焦后点击字幕播放动作/真实空格可暂停")?;
                app.inputs["question"].update(cx, |state, cx| state.focus(window, cx));
                app.handle(Action::Play, window, cx);
                self.advance();
            }
            84 => {
                if app.media.player.snapshot().paused {
                    return Ok(false);
                }
                Self::dispatch_keys(window, cx, &["space"]);
                self.advance();
            }
            85 => {
                let state = app.media.player.snapshot();
                if !state.paused {
                    return Ok(false);
                }
                self.record(app, "问题框聚焦后点击播放动作/真实空格继续控制视频")?;
                self.keyboard_position = state.position;
                window.open_dialog(cx, |dialog, _, _| dialog.title("键盘焦点回归"));
                Self::dispatch_keys(window, cx, &["right", "space"]);
                self.advance();
            }
            86 => {
                if !self.stable() {
                    return Ok(false);
                }
                let state = app.media.player.snapshot();
                ensure!(window.has_active_dialog(cx), "原生对话框夹具未打开");
                ensure!(
                    state.paused && (state.position - self.keyboard_position).abs() < 0.08,
                    "对话框键盘穿透控制视频"
                );
                self.record(app, "真实窗口对话框期间方向键与空格不穿透播放")?;
                window.close_dialog(cx);
                Self::dispatch_keys(window, cx, &["left"]);
                self.advance();
            }
            87 => {
                if !Self::at(app, &path, 0.) {
                    return Ok(false);
                }
                self.record(app, "关闭对话框后真实键盘恢复视频导航")?;
                app.video.fullscreen = true;
                app.inputs["question"].update(cx, |state, cx| state.focus(window, cx));
                app.handle(Action::Assistant, window, cx);
                Self::dispatch_keys(window, cx, &["right"]);
                self.advance();
            }
            88 => {
                if !Self::at(app, &path, 4.) {
                    return Ok(false);
                }
                ensure!(!app.assistant_open, "全屏助手没有收起");
                self.record(app, "全屏渲染分支收起助手后真实方向键恢复")?;
                app.video.fullscreen = false;
                if let Some((enabled, open, legacy, question)) =
                    self.assistant_before_keyboard.take()
                {
                    app.assistant.enabled = enabled;
                    app.assistant_open = open;
                    app.assistant.legacy = legacy;
                    app.set("question", question, window, cx);
                    app.chat_requested_context.clear();
                }
                app.handle(Action::Rate(1.), window, cx);
                app.handle(Action::VideoSentenceRepeat(Some(0), true), window, cx);
                app.media.player.set("pause", "yes");
                self.advance();
            }
            89 => {
                let state = app.media.player.snapshot();
                let Some(session) = app.video.sentence_repeat else {
                    return Ok(false);
                };
                if !Self::at(app, &path, 0.)
                    || !state.sentence_repeat.as_ref().is_some_and(|repeat| {
                        repeat.request_id == session.request_id && repeat.total_plays == Some(3)
                    })
                {
                    return Ok(false);
                }
                self.timeline_request = session.request_id;
                app.handle(Action::VideoPlayCue(1), window, cx);
                ensure!(
                    app.video.repeat_three
                        && app.video.sentence_repeat.is_some_and(|s| {
                            s.three && s.index == 1 && s.request_id != self.timeline_request
                        }),
                    "点击字幕正文清除了三次模式或未切换到目标句"
                );
                self.advance();
            }
            90 => {
                if !Self::clicked_cue_playing(app, &path, 1, true) {
                    return Ok(false);
                }
                self.record(app, "暂停三次模式点击字幕/目标句首从第一遍播放且保留模式")?;
                app.handle(Action::VideoPlayCue(2), window, cx);
                app.handle(Action::VideoPlayCue(0), window, cx);
                app.handle(Action::VideoPlayCue(1), window, cx);
                app.media.player.set("pause", "yes");
                self.advance();
            }
            91 => {
                let state = app.media.player.snapshot();
                let Some(session) = app.video.sentence_repeat else {
                    return Ok(false);
                };
                if !Self::at(app, &path, 4.)
                    || !state.sentence_repeat.as_ref().is_some_and(|repeat| {
                        repeat.request_id == session.request_id
                            && repeat.total_plays == Some(3)
                            && repeat.current_play == 1
                    })
                {
                    return Ok(false);
                }
                ensure!(
                    app.video.repeat_three && session.three && session.index == 1,
                    "连续点击字幕后没有保留最终目标句的三次模式"
                );
                self.record(app, "三次模式快速连续点击/最终句生效且旧完成事件不拉回")?;
                app.handle(Action::VideoPlayCue(usize::MAX), window, cx);
                ensure!(
                    app.video.repeat_three
                        && app.video.sentence_repeat.is_some_and(|s| {
                            s.index == 1 && s.request_id == session.request_id
                        }),
                    "无效字幕行清除了有效三次模式或替换了原请求"
                );
                self.record(app, "无效字幕点击保留当前模式和请求")?;
                self.timeline_request = session.request_id;
                app.handle(Action::VideoPlayCue(1), window, cx);
                self.advance();
            }
            92 => {
                if !Self::clicked_cue_playing(app, &path, 1, true) {
                    return Ok(false);
                }
                ensure!(
                    app.video.sentence_repeat.unwrap().request_id != self.timeline_request,
                    "点击同一句没有从第一遍重新建立重复"
                );
                self.record(app, "点击当前字幕句从句首和第一遍重新开始三次")?;
                app.handle(Action::Rate(8.), window, cx);
                self.advance();
            }
            93 => {
                let state = app.media.player.snapshot();
                let Some(session) = app
                    .video
                    .sentence_repeat
                    .filter(|s| s.three && s.index == 2)
                else {
                    return Ok(false);
                };
                if !state.sentence_repeat.as_ref().is_some_and(|repeat| {
                    repeat.request_id == session.request_id && repeat.total_plays == Some(3)
                }) {
                    return Ok(false);
                }
                ensure!(app.video.repeat_three, "点击字幕后的自然下一句丢失三次模式");
                self.record(app, "点击字幕后真实完成三遍/自动下一句继续三遍")?;
                app.handle(Action::Rate(1.), window, cx);
                app.handle(Action::VideoSentenceRepeat(Some(0), false), window, cx);
                app.media.player.set("pause", "yes");
                self.advance();
            }
            94 => {
                let state = app.media.player.snapshot();
                let Some(session) = app
                    .video
                    .sentence_repeat
                    .filter(|s| !s.three && s.index == 0)
                else {
                    return Ok(false);
                };
                if !Self::at(app, &path, 0.)
                    || !state.sentence_repeat.as_ref().is_some_and(|repeat| {
                        repeat.request_id == session.request_id && repeat.total_plays.is_none()
                    })
                {
                    return Ok(false);
                }
                app.handle(Action::VideoPlayCue(1), window, cx);
                ensure!(
                    !app.video.repeat_three
                        && app
                            .video
                            .sentence_repeat
                            .is_some_and(|s| !s.three && s.index == 1),
                    "点击字幕正文清除了无限重复模式"
                );
                self.advance();
            }
            95 => {
                if !Self::clicked_cue_playing(app, &path, 1, false) {
                    return Ok(false);
                }
                self.record(app, "无限重复时点击字幕/继续无限重复目标句")?;
                app.handle(Action::Rate(8.), window, cx);
                app.handle(Action::VideoSentenceRepeat(Some(2), true), window, cx);
                self.advance();
            }
            96 => {
                let state = app.media.player.snapshot();
                let Some(session) = app
                    .video
                    .sentence_repeat
                    .filter(|s| s.three && s.index == 2)
                else {
                    return Ok(false);
                };
                if !state.paused
                    || !state.sentence_repeat.as_ref().is_some_and(|repeat| {
                        repeat.request_id == session.request_id && repeat.finished
                    })
                {
                    return Ok(false);
                }
                app.handle(Action::Rate(1.), window, cx);
                app.handle(Action::VideoPlayCue(0), window, cx);
                ensure!(app.video.repeat_three, "末句完成后点击字幕退出了三次模式");
                self.advance();
            }
            97 => {
                if !Self::clicked_cue_playing(app, &path, 0, true) {
                    return Ok(false);
                }
                self.record(app, "末句完成三次后点击首句/保留模式并重新播放三遍")?;
                app.handle(Action::VideoStopSentenceRepeat, window, cx);
                app.handle(Action::VideoPlayCue(2), window, cx);
                self.advance();
            }
            98 => {
                let state = app.media.player.snapshot();
                if state.paused || state.position < 7.95 || state.position > 9. {
                    return Ok(false);
                }
                ensure!(
                    !app.video.repeat_three
                        && app.video.sentence_repeat.is_none()
                        && state.sentence_repeat.is_none(),
                    "普通模式点击字幕错误启用了句子重复"
                );
                self.record(app, "普通模式点击字幕仍从目标句正常播放")?;
                app.handle(Action::Page("settings".into()), window, cx);
                self.report(Some(true), None)?;
                self.advance();
                return Ok(true);
            }
            _ => return Ok(true),
        }
        Ok(false)
    }
    fn clicked_cue_playing(app: &AppView, path: &Path, index: usize, three: bool) -> bool {
        let state = app.media.player.snapshot();
        let Some(session) = app.video.sentence_repeat else {
            return false;
        };
        state.path == path.to_string_lossy()
            && !state.paused
            && state.error.is_empty()
            && app.video.repeat_three == three
            && session.three == three
            && session.index == index
            && state.position >= session.range.0 - 0.05
            && state.position < session.range.0 + 1.
            && state.sentence_repeat.as_ref().is_some_and(|repeat| {
                repeat.request_id == session.request_id
                    && repeat.total_plays == if three { Some(3) } else { None }
                    && repeat.current_play == 1
                    && !repeat.finished
                    && (repeat.start - session.range.0).abs() < 0.001
                    && (repeat.end - session.range.1).abs() < 0.001
            })
    }
    fn paused_middle_repeat(app: &AppView, path: &std::path::Path) -> bool {
        let state = app.media.player.snapshot();
        state.path == path.to_string_lossy()
            && state.paused
            && (state.position - 4.).abs() < 0.18
            && app.video.repeat_three
            && app.media.repeat == 0
            && app.video.pending_start.is_none()
            && app
                .video
                .sentence_repeat
                .is_some_and(|s| s.three && s.index == 1 && s.range == (4., 8.))
            && state.sentence_repeat.as_ref().is_some_and(|s| {
                s.start == 4.
                    && s.end == 8.
                    && s.total_plays == Some(3)
                    && s.current_play == 1
                    && app
                        .video
                        .sentence_repeat
                        .is_some_and(|session| session.request_id == s.request_id)
            })
    }
    fn key(key: &str, is_held: bool) -> KeyDownEvent {
        KeyDownEvent {
            keystroke: Keystroke::parse(key).expect("probe key"),
            is_held,
        }
    }
    fn dispatch_keys(window: &Window, cx: &mut Context<AppView>, keys: &[&str]) {
        let handle = window.window_handle();
        let keys = keys
            .iter()
            .map(|key| Keystroke::parse(key).expect("probe key"))
            .collect::<Vec<_>>();
        cx.defer(move |cx| {
            let _ = handle.update(cx, move |_, window, cx| {
                window.draw(cx).clear();
                for key in keys {
                    window.dispatch_keystroke(key, cx);
                }
            });
        });
    }
}
