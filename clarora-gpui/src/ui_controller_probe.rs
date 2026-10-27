//! Hidden-window author self-check of the actual AppView handlers and InputState.
//! This is not mouse/keyboard, visual, sound-device, or independent review evidence.
use super::*;
use clarora_gpui::chat_store;
use sha2::{Digest, Sha256};

const AUDIO_A: &str = "probe-audio-a";
const AUDIO_B: &str = "probe-audio-b";
const GROUP_A: &str = "probe-group-a";
const GROUP_B: &str = "probe-group-b";
const DRAFT_A: &str = "A 的问题草稿 — material A";
const DRAFT_B: &str = "B 的问题草稿 — material B";
const LAST_DRAFT: &str = "关闭前最后输入 — 不等待 debounce";
fn context(audio: &str) -> String {
    hex::encode(Sha256::digest(format!("listening:{audio}").as_bytes()))
}

pub(super) fn prepare(out: &Path) -> Result<Library> {
    fs::create_dir_all(out)?;
    let lib = Library::open(out.join("library"))?;
    video_resume_probe::prepare(&lib)?;
    lib.put(
        "listening_practices",
        json!({"id":GROUP_A,"name":"原生探针 A"}),
    )?;
    lib.put(
        "listening_practices",
        json!({"id":GROUP_B,"name":"原生探针 B"}),
    )?;
    for (audio, group, marker) in [(AUDIO_A, GROUP_A, "A"), (AUDIO_B, GROUP_B, "B")] {
        let wav = lib.root.join("media").join(format!("{audio}.wav"));
        let mut writer = hound::WavWriter::create(
            &wav,
            hound::WavSpec {
                channels: 1,
                sample_rate: 16000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )?;
        for _ in 0..128_000 {
            writer.write_sample(0i16)?;
        }
        writer.finalize()?;
        let srt = lib.root.join("media").join(format!("{audio}.srt"));
        fs::write(
            &srt,
            format!(
                "1\n00:00:00,000 --> 00:00:03,000\n{marker} first sentence.\n{marker} 的第一句。\n\n2\n00:00:03,000 --> 00:00:07,000\n{marker} second sentence.\n{marker} 的第二句。\n"
            ),
        )?;
        lib.put("listening_audios",json!({"id":audio,"practice_id":group,"name":format!("探针 {marker}"),"audio_uri":wav.to_string_lossy(),"subtitle_uri":srt.to_string_lossy()}))?;
        chat_store::save(
            &lib,
            &format!("session-{marker}"),
            &context(audio),
            &format!("会话 {marker}"),
            &[
                json!({"role":"user","content":format!("question {marker}")}),
                json!({"role":"assistant","content":format!("answer {marker}")}),
            ],
        )?;
    }
    lib.put("sentence_cards",json!({"id":"native-probe-card","text":"Native controller card.","translation":"原生控制器卡片。","notes":""}))?;
    lib.set_setting(
        &format!("key_points_{AUDIO_A}"),
        &json!([{"text":"A first","translation":"A的第一句"}]).to_string(),
    )?;
    lib.set_setting("library_group", GROUP_A)?;
    lib.set_setting("last_page", "flashcard")?;
    // No network service is part of this probe; the send-during-load guard must prevent a task.
    lib.set_setting("ai_config",&json!({"engine":"compatible","base_url":"http://127.0.0.1:9/v1","model":"native-probe","local_model":"base.en"}).to_string())?;
    Ok(lib)
}

pub(super) struct Probe {
    out: PathBuf,
    started: Instant,
    stage: usize,
    stage_started: Instant,
    cases: Vec<Value>,
    closing: bool,
    global_quit: bool,
    fault_observed: Option<Instant>,
    highlights_revision: Option<u64>,
    video_resume: video_resume_probe::Probe,
}
impl Probe {
    pub(super) fn new(out: &Path) -> Self {
        Self {
            out: out.to_path_buf(),
            started: Instant::now(),
            stage: 0,
            stage_started: Instant::now(),
            cases: vec![],
            closing: false,
            global_quit: false,
            fault_observed: None,
            highlights_revision: None,
            video_resume: video_resume_probe::Probe::new(out),
        }
    }
    pub(super) fn with_global_quit(mut self, enabled: bool) -> Self {
        self.global_quit = enabled;
        self
    }
    fn report(&self, passed: Option<bool>, error: Option<String>) -> Result<()> {
        fs::write(
            self.out.join("ui-controller-report.json"),
            serde_json::to_vec_pretty(&json!({
                "passed":passed,"error":error,"stage":self.stage,"elapsed_ms":self.started.elapsed().as_millis(),
                "cases":self.cases,"library":self.out.join("library"),"hidden_native_window":true,
                "actual_handlers":true,"actual_input_state":true,"mouse_keyboard_visual_verified":false,
                "independent_review":false,"global_quit_action":self.global_quit
            }))?,
        )?;
        Ok(())
    }
    fn note(&mut self, name: &str, app: &AppView) -> Result<()> {
        self.cases.push(json!({"case":name,"passed":true,"elapsed_ms":self.started.elapsed().as_millis(),"page":app.page,"practice":app.audio.practice_id,"current_audio":app.audio.current_id,"management_selected":app.audio.library.selected,"cue_count":app.audio.cues.len(),"chat_context":app.chat.context,"chat_session":app.chat.id}));
        self.report(None, None)
    }
    fn advance(&mut self) {
        self.stage += 1;
        self.stage_started = Instant::now();
    }
    fn chat_ready(app: &AppView, cx: &App) -> bool {
        !app.chat.is_loading() && app.chat.context == app.chat_context_key(cx)
    }
    fn cues_are(app: &AppView, marker: &str) -> bool {
        app.audio.cues.len() == 2
            && app
                .audio
                .cues
                .iter()
                .all(|cue| cue.text.starts_with(marker))
    }
    fn step(
        &mut self,
        app: &mut AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Result<()> {
        ensure!(
            self.started.elapsed() < Duration::from_secs(60),
            "原生控制器探针超时，stage={}，status={}",
            self.stage,
            app.status
        );
        match self.stage {
            0 => {
                if !app.cards_ready() || !app.cards.iter().any(|c| c.id == "native-probe-card") {
                    return Ok(());
                }
                // The normal player API and AppView's real clock are exercised without a sound device.
                app.media.player = Player::without_audio_device();
                app.media.preloader = Player::without_audio_device();
                self.note("初始闪卡异步快照接受/cards_ready", app)?;
                let card = app.current_card().context("初始闪卡不存在")?;
                let favorite_key = format!("{}:{}", card.kind, card.id);
                app.handle(Action::Favorite, window, cx);
                ensure!(
                    app.loads.discover_favorites.contains(&favorite_key),
                    "收藏成功未更新推荐缓存"
                );
                ensure!(
                    app.lib
                        .list("discover_favorites")?
                        .iter()
                        .any(|row| text(row, "key") == favorite_key),
                    "收藏未保存至SQLite"
                );
                app.handle(Action::Favorite, window, cx);
                ensure!(
                    !app.loads.discover_favorites.contains(&favorite_key),
                    "取消收藏未更新推荐缓存"
                );
                ensure!(
                    !app.lib
                        .list("discover_favorites")?
                        .iter()
                        .any(|row| text(row, "key") == favorite_key),
                    "取消收藏未更新SQLite"
                );
                self.note("真实Favorite收藏/取消同步更新SQLite与推荐缓存", app)?;
                app.handle(Action::Page("library".into()), window, cx);
                self.advance();
            }
            1 => {
                if app.page != "library" || app.audio.audios.len() != 2 {
                    return Ok(());
                }
                app.handle(Action::Practice(GROUP_A.into()), window, cx);
                app.handle(
                    Action::LibrarySelect {
                        id: AUDIO_A.into(),
                        reveal: true,
                    },
                    window,
                    cx,
                );
                self.advance();
            }
            2 => {
                if app.audio.practice_id != GROUP_A
                    || app.audio.library.selected != AUDIO_A
                    || app.audio.library.preview.len() != 2
                {
                    return Ok(());
                }
                ensure!(
                    app.audio.library.preview[0].text.starts_with('A'),
                    "管理预览串材料"
                );
                self.note("原生Page/Practice/LibrarySelect与后台预览", app)?;
                app.handle(Action::Audio(AUDIO_A.into()), window, cx);
                ensure!(app.chat.is_loading(), "材料切换未提交异步会话请求");
                app.set("question", "读取中不应发送", window, cx);
                app.handle(Action::Chat, window, cx);
                ensure!(
                    !app.chat.running && app.tasks.cancel.is_none(),
                    "读取中发送handler启动了旧材料请求"
                );
                self.note("会话读取中发送handler保护", app)?;
                app.set("question", "", window, cx);
                self.advance();
            }
            3 => {
                if app.audio.current_id != AUDIO_A
                    || !Self::cues_are(app, "A")
                    || app.media.snapshot.duration < 7.
                    || app.audio.key_points.len() != 1
                {
                    return Ok(());
                }
                ensure!(
                    app.audio.key_points[0].text == "A first",
                    "A音频重点缓存错材料"
                );
                self.highlights_revision = Some(app.data.revision);
                app.handle(Action::Refresh, window, cx);
                app.handle(Action::ClearHighlights, window, cx);
                if !app.media.snapshot.paused {
                    app.handle(Action::Play, window, cx);
                }
                self.advance();
            }
            4 => {
                if !app.media.snapshot.paused
                    || self.stage_started.elapsed() < Duration::from_millis(300)
                    || self
                        .highlights_revision
                        .is_none_or(|revision| app.data.revision <= revision)
                {
                    return Ok(());
                }
                ensure!(
                    app.media.snapshot.error.is_empty(),
                    "有效PCM播放失败：{}",
                    app.media.snapshot.error
                );
                ensure!(app.audio.key_points.is_empty(), "在途旧快照复活已清空重点");
                ensure!(
                    app.lib.setting(&format!("key_points_{AUDIO_A}"))? == "[]",
                    "清空重点未写入SQLite"
                );
                self.note(
                    "Refresh在途后ClearHighlights/新快照接受仍为空且SQLite=[]",
                    app,
                )?;
                self.note("音频字幕接受与真实播放器暂停确认", app)?;
                app.handle(Action::AudioBlind(true), window, cx);
                let cue = app.audio.cues[0].clone();
                app.handle(Action::AudioBlindRevealCue(AUDIO_A.into(), cue), window, cx);
                self.advance();
            }
            5 => {
                if app.audio.blind.revealed.is_none() {
                    return Ok(());
                }
                ensure!(
                    app.audio
                        .blind
                        .revealed
                        .as_ref()
                        .is_some_and(|r| r.cue.text.starts_with('A')),
                    "盲听揭示错材料"
                );
                self.note("盲听handler确认暂停后揭示", app)?;
                for id in [AUDIO_B, AUDIO_A, AUDIO_B] {
                    app.handle(Action::Audio(id.into()), window, cx);
                }
                ensure!(
                    app.audio.blind.revealed.is_none() && app.audio.blind_pending.is_none(),
                    "快速切音频残留旧盲听揭示/请求"
                );
                self.advance();
            }
            6 => {
                if app.audio.current_id != AUDIO_B
                    || !Self::cues_are(app, "B")
                    || app.audio.library.selected != AUDIO_B
                {
                    return Ok(());
                }
                ensure!(
                    app.audio.practice_id == GROUP_B
                        && app.audio.blind.revealed.is_none()
                        && app.audio.blind_pending.is_none(),
                    "旧响应覆盖了B音频/练习组/盲听状态"
                );
                self.note("A/B/A/B快速切换与字幕/音频选择/盲听隔离", app)?;
                app.handle(Action::AudioBlind(false), window, cx);
                self.advance();
            }
            7 => {
                if !Self::chat_ready(app, cx) || app.chat.context != context(AUDIO_B) {
                    return Ok(());
                }
                app.set("question", DRAFT_B, window, cx);
                app.handle(Action::Audio(AUDIO_A.into()), window, cx);
                self.advance();
            }
            8 => {
                if !Self::chat_ready(app, cx) || app.chat.context != context(AUDIO_A) {
                    return Ok(());
                }
                app.set("question", DRAFT_A, window, cx);
                app.handle(Action::ChatSession(String::new()), window, cx);
                app.handle(Action::SaveAnswer, window, cx);
                ensure!(
                    app.lib.list("ai_cards")?.is_empty(),
                    "会话切换中把旧回答存为问答卡"
                );
                self.note("会话切换中禁止保存旧回答", app)?;
                self.advance();
            }
            9 => {
                if !Self::chat_ready(app, cx)
                    || !app.chat.id.is_empty()
                    || !app.value("question", cx).is_empty()
                {
                    return Ok(());
                }
                self.note("InputState输入后新建会话异步清空", app)?;
                app.handle(Action::ChatSession("session-A".into()), window, cx);
                self.advance();
            }
            10 => {
                if !Self::chat_ready(app, cx) || app.chat.id != "session-A" {
                    return Ok(());
                }
                ensure!(
                    app.chat.output == "answer A",
                    "选择已有会话未恢复原材料回答"
                );
                self.note("选择已有材料会话异步恢复", app)?;
                app.set("question", DRAFT_A, window, cx);
                app.handle(Action::Audio(AUDIO_B.into()), window, cx);
                self.advance();
            }
            11 => {
                if !Self::chat_ready(app, cx) || app.chat.context != context(AUDIO_B) {
                    return Ok(());
                }
                ensure!(
                    app.value("question", cx) == DRAFT_B,
                    "InputState B草稿切回未恢复/串材料：{}",
                    app.value("question", cx)
                );
                self.note("真实InputState材料B草稿恢复", app)?;
                app.lib.db()?.execute_batch(
                    "ALTER TABLE ai_records RENAME TO probe_ai_records_unavailable",
                )?;
                app.handle(Action::Audio(AUDIO_A.into()), window, cx);
                self.advance();
            }
            12 => {
                if app.chat.is_loading() || app.chat.error.is_empty() {
                    return Ok(());
                }
                ensure!(
                    app.chat.context == context(AUDIO_B),
                    "会话读取失败覆盖了原材料"
                );
                self.fault_observed = Some(Instant::now());
                app.lib.db()?.execute_batch(
                    "ALTER TABLE probe_ai_records_unavailable RENAME TO ai_records",
                )?;
                app.handle(Action::Refresh, window, cx);
                ensure!(
                    self.fault_observed.unwrap().elapsed() < Duration::from_secs(3),
                    "手动刷新已错过backoff窗口"
                );
                self.advance();
            }
            13 => {
                if !Self::chat_ready(app, cx) || app.chat.context != context(AUDIO_A) {
                    return Ok(());
                }
                ensure!(
                    app.value("question", cx) == DRAFT_A,
                    "InputState A草稿切回未恢复/串材料：{}",
                    app.value("question", cx)
                );
                ensure!(self.fault_observed.is_some(), "未触发故障/backoff验证");
                self.note(
                    "SQLite故障后backoff内Refresh/无再次点击自动恢复材料A草稿",
                    app,
                )?;
                self.advance();
            }
            14 => {
                if !self.video_resume.step(app, window, cx)? {
                    return Ok(());
                }
                self.note("真实视频切页进度/快速切换/同源片段恢复", app)?;
                app.handle(Action::Audio(AUDIO_A.into()), window, cx);
                self.advance();
            }
            15 => {
                if !Self::chat_ready(app, cx) || app.chat.context != context(AUDIO_A) {
                    return Ok(());
                }
                app.set("question", LAST_DRAFT, window, cx);
                self.closing = true;
                self.advance();
                if self.global_quit {
                    // Menu/global actions originate outside an entity update. Dispatch
                    // after this lease ends so the global WeakEntity handler can update it.
                    cx.defer(|cx| cx.dispatch_action(&Quit));
                } else {
                    app.prepare_close(cx);
                    ensure!(app.chat_closing.is_some(), "原生关闭未等待异步flush");
                }
            }
            _ => {}
        }
        Ok(())
    }
}

impl AppView {
    /// The normal 50ms UI tick drives the probe even while its window is hidden.
    pub(super) fn poll_ui_probe(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(mut probe) = self.probe.take() else {
            return false;
        };
        if let Err(error) = probe.step(self, window, cx) {
            let _ = probe.report(Some(false), Some(format!("{error:#}")));
            window.remove_window();
            return true;
        }
        self.probe = Some(probe);
        false
    }
}

/// Called by the real close acknowledgement immediately before remove_window.
/// Read the actual SQLite rows after flush, without forcing a separate test flush.
pub(super) fn close_verified(
    app: &mut AppView,
    _window: &mut Window,
    _cx: &mut Context<AppView>,
) -> bool {
    let Some(mut probe) = app.probe.take() else {
        return true;
    };
    let checked = (|| -> Result<()> {
        ensure!(
            probe.closing && probe.stage == 16,
            "探针未完成所有检查即关闭"
        );
        ensure!(
            app.lib
                .setting(&format!("chat_draft_{}", context(AUDIO_A)))?
                == LAST_DRAFT,
            "立即关闭丢失最后一笔InputState输入"
        );
        ensure!(
            app.lib
                .setting(&format!("chat_draft_{}", context(AUDIO_B)))?
                == DRAFT_B,
            "关闭写入覆盖了材料B草稿"
        );
        probe.note(
            if probe.global_quit {
                "真实全局Quit action异步flush确认/SQLite保留最后输入"
            } else {
                "真实原生关闭异步flush确认/SQLite保留最后输入"
            },
            app,
        )?;
        probe.report(Some(true), None)
    })();
    if let Err(error) = checked {
        let _ = probe.report(Some(false), Some(format!("{error:#}")));
    }
    true
}
