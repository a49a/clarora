//! Speech commands own their action effects and execution.
use super::*;
pub(super) enum Command {
    Score,
    ImportSpeakingGroup(String),
    Record,
    StopRecord,
    TtsDownload,
    TtsGenerate,
    TtsVoice(i32),
    TtsSpeed(f32),
    TtsPlay,
}
impl Command {
    pub(super) fn effects(&self, _app: &AppView) -> Effects {
        Effects::default()
    }
    pub(super) fn execute(
        self,
        app: &mut AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Result<()> {
        match self {
            Command::ImportSpeakingGroup(group) => {
                app.start_speaking_command_import_to(group, None, window, cx);
            }
            Command::Score => {
                let path = app.recorded.clone().context("请先录音或选择录音文件")?;
                let reference = app.value("reference", cx);
                ensure!(!reference.trim().is_empty(), "请先转写题目音频");
                app.begin("正在转写并对齐录音…", move |lib, cancel, _| {
                    let cues = ai::transcribe(&lib, &path, &cancel)?;
                    let actual = cues
                        .iter()
                        .map(|c| c.text.clone())
                        .collect::<Vec<_>>()
                        .join(" ");
                    let mut score = ai::score(&reference, &actual)?;
                    let duration = cues.last().map(|c| c.end).unwrap_or(0.);
                    let wpm = if duration > 0. { (score["words"].as_f64().unwrap_or(0.) * 60. / duration).round() } else { 0. };
                    let feedback = ai::chat(&lib, "用简短中文给出跟读建议。只能依据识别文本评价，不推断发音音质。", &score.to_string(), &[], &cancel, |_| {}).unwrap_or_else(|_| "已完成识别文本对齐；未取得 AI 点评。".into());
                    ensure!(!cancel.load(Ordering::SeqCst), "已取消");
                    score["attemptId"] = json!(id());
                    score["kind"] = json!("shadow");
                    score["createdAt"] = json!(now());
                    score["status"] = json!("completed");
                    score["feedback"] = json!(feedback);
                    score["score"] = json!({"accuracy":(score["accuracy"].as_f64().unwrap_or(0.)*100.).round(),"completeness":(score["completeness"].as_f64().unwrap_or(0.)*100.).round(),"wpm":wpm,"missed":score["missed"],"wrong":score["wrong"],"extra":score["extra"]});
                    ai::save_record(&lib, "speaking", score.clone())?;
                    Ok(Outcome::Output(format!(
                        "文本对齐准确度：{:.0}% · 完整度：{:.0}% · 语速：{wpm:.0} WPM\n\n识别：{actual}\n\n漏读：{}\n\n读错：{}\n\n多读：{}\n\n{feedback}",
                        score["accuracy"].as_f64().unwrap_or(0.) * 100.,
                        score["completeness"].as_f64().unwrap_or(0.) * 100.,
                        score["missed"], score["wrong"], score["extra"]
                    )))
                });
            }

            Command::Record => {
                ensure!(app.recording.is_none(), "正在录音");
                app.media.player.set("pause", "yes");
                let path = app
                    .lib
                    .root
                    .join("recordings")
                    .join(format!("{}.wav", id()));
                app.recording = Some(Recorder::start(path));
                app.report_status(FeedbackKind::Info, "正在录音，完成后点击停止".into());
            }

            Command::StopRecord => {
                let recorder = app.recording.take().context("没有正在进行的录音")?;
                app.begin("正在保存录音…", move |_, _, _| {
                    Ok(Outcome::Recording(recorder.finish()?))
                });
            }

            Command::TtsVoice(voice) => {
                ensure!((0..54).contains(&voice), "音色编号无效");
                app.tts_voice = voice;
                app.lib.set_setting("tts_voice", &voice.to_string())?;
            }

            Command::TtsSpeed(speed) => {
                ensure!(speed.is_finite() && (0.5..=2.).contains(&speed), "语速无效");
                app.tts_speed = speed;
                app.lib.set_setting("tts_speed", &speed.to_string())?;
            }

            Command::TtsDownload => {
                app.begin_model_download(model_controller::Request::Kokoro)?;
            }

            Command::TtsGenerate => {
                let body = app.value("tts_text", cx);
                app.audio.library.filter = 0;
                app.audio.library.sort = 0;
                app.set("library_search", "", window, cx);
                ensure!(!body.trim().is_empty(), "请先输入要合成的文字");
                ensure!(!app.audio.practice_id.is_empty(), "请先新建或选择练习组");
                let mut name = app.value("tts_name", cx);
                if name.trim().is_empty() {
                    name = format!("Kokoro · {}", chrono::Local::now().format("%m-%d %H:%M:%S"));
                }
                let group = app.audio.practice_id.clone();
                let voice = app.tts_voice;
                let speed = app.tts_speed;
                app.begin_scoped(
                    TaskScope::Application,
                    "正在本机生成语音（首次加载模型需稍候）…",
                    move |lib, cancel, _| {
                        let row = tts::generate(
                            &lib,
                            &tts::model_dir(&lib),
                            &group,
                            &name,
                            &body,
                            voice,
                            speed,
                            &cancel,
                        )?;
                        Ok(Outcome::TtsReady(row))
                    },
                );
            }

            Command::TtsPlay => {
                let row = app.tts_last.clone().context("请先生成音频")?;
                ensure!(
                    PathBuf::from(text(&row, "audio_uri")).is_file(),
                    "生成的音频已被移除"
                );
                app.audio.blind.enabled = false;
                app.page_scroll.set_offset(point(px(0.), px(0.)));
                app.handle(Action::Audio(text(&row, "id")), window, cx);
            }
        }
        Ok(())
    }
}

// Completion policy belongs to this domain: cancellation may race a durable commit.
pub(super) enum Completed {
    Ocr(String),
    Recording(PathBuf),
}
impl Completed {
    pub(super) fn apply(
        self,
        app: &mut AppView,
        context: &mut Completion,
        _window: &mut Window,
        _cx: &mut Context<AppView>,
    ) {
        match self {
            Completed::Ocr(text) if !context.cancelled => {
                app.ocr_text = text.clone();
                app.output = text;
                app.report_status(FeedbackKind::Info, "图片文字已识别".into());
            }

            Completed::Recording(path) => {
                app.recorded = Some(path);
                app.report_status(FeedbackKind::Info, "录音已保存，可转写评分".into());
            }

            _ => app.report_status(FeedbackKind::Info, "已取消".into()),
        }
    }
}
