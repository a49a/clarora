//! Hidden native input/layout benchmark with synthetic history and isolated SQLite.
//! Timing is diagnostic, not a physical-keyboard/visible-frame SLA.
use super::*;
use clarora_gpui::chat_store;
use std::{cell::RefCell, rc::Rc};
const SESSION: &str = "assistant-latency-session";
const PASTE: &str =
    "请解释这些句型，并保留中文、标点与 emoji 🌱。\n粘贴后继续输入，检查文字是否完整。\n";
const KEYS: usize = 24;
fn fixture() -> String {
    let mut text = String::from("# Native assistant latency fixture\n\n");
    let mut i = 0;
    while text.len() < 36_000 {
        text.push_str(&format!("## {}. 句首评注与后置修饰\n\n- **As shown in the chart**, sales increased. 正如图表所示，销量上升。\n- **As mentioned above**, the deadline is Friday. 如上所述，截止日期是周五。\n- The results **as reported by the committee** were encouraging. 委员会报告的结果令人鼓舞。\n\n| Expression | Meaning | Example |\n| --- | --- | --- |\n| as expected | 正如预料 | As expected, it was crowded. |\n| as planned | 按计划 | We will meet at noon. |\n\n",i+1));
        i += 1;
    }
    text
}
pub(super) fn prepare(out: &Path) -> Result<Library> {
    let lib = ui_controller_probe::prepare(out)?;
    let body = fixture();
    fs::write(out.join("history-fixture.md"), &body)?;
    chat_store::save(
        &lib,
        SESSION,
        clarora_gpui::assistant::CONTEXT,
        "Latency fixture",
        &[
            json!({"role":"user","content":"请解释 as 引导的表达。"}),
            json!({"role":"assistant","content":body}),
        ],
    )?;
    lib.set_setting("assistant_open", "true")?;
    lib.set_setting("floating_import_open", "false")?;
    Ok(lib)
}
#[derive(Clone)]
enum Edit {
    Key(String),
    Frame,
}
pub(super) struct Probe {
    out: PathBuf,
    start: Instant,
    stage: u8,
    cases: Vec<Value>,
    samples: Rc<RefCell<Vec<Value>>>,
    requested: usize,
    typed: usize,
    typing_builds: u64,
    replace_builds: u64,
    replacement_frames: u8,
    empty_typed: usize,
    empty_warmups: u8,
    empty_builds: u64,
    clipboard: Option<ClipboardItem>,
    clipboard_touched: bool,
    flush: Option<mpsc::Receiver<std::result::Result<(), String>>>,
}
impl Probe {
    pub(super) fn new(out: &Path) -> Self {
        Self {
            out: out.into(),
            start: Instant::now(),
            stage: 0,
            cases: vec![],
            samples: Default::default(),
            requested: 0,
            typed: 0,
            typing_builds: 0,
            replace_builds: 0,
            replacement_frames: 0,
            empty_typed: 0,
            empty_warmups: 0,
            empty_builds: 0,
            clipboard: None,
            clipboard_touched: false,
            flush: None,
        }
    }
    fn sample(
        &mut self,
        label: &str,
        edit: Edit,
        app: &AppView,
        window: &Window,
        cx: &mut Context<AppView>,
    ) {
        let handle = window.window_handle();
        let samples = self.samples.clone();
        let label = label.to_string();
        let input = app.inputs["question"].clone();
        let builds = app.assistant.debug_history_builds.clone();
        self.requested += 1;
        cx.defer(move|cx|{let _=handle.update(cx,|_,window,cx|{
            let before=builds.get();let began=Instant::now();
            if let Edit::Key(key)=edit {window.dispatch_keystroke(Keystroke::parse(&key).expect("diagnostic keystroke"),cx);}
            let edit_us=began.elapsed().as_micros() as u64;let draw=Instant::now();
            window.draw(cx).clear();
            let draw_us=draw.elapsed().as_micros() as u64;
            samples.borrow_mut().push(json!({"operation":label,"edit_us":edit_us,"draw_us":draw_us,"total_us":began.elapsed().as_micros() as u64,"history_builds_before":before,"history_builds_after":builds.get(),"editor_bytes":input.read(cx).value().len()}));
        });});
    }
    fn clipboard_restore(&mut self, cx: &mut App) {
        if self.clipboard_touched {
            cx.write_to_clipboard(
                self.clipboard
                    .clone()
                    .unwrap_or_else(|| ClipboardItem::new_string(String::new())),
            );
            self.clipboard_touched = false;
        }
    }
    fn note(&mut self, name: &str, details: Value) {
        self.cases
            .push(json!({"case":name,"passed":true,"details":details}));
    }
    fn report(&self, passed: bool, error: Option<String>) -> Result<()> {
        let samples = self.samples.borrow();
        let typing = samples
            .iter()
            .filter(|s| s["operation"] == "short_key")
            .collect::<Vec<_>>();
        let mut times = typing
            .iter()
            .filter_map(|s| s["total_us"].as_u64())
            .collect::<Vec<_>>();
        times.sort_unstable();
        let metric = |p: f64| {
            times
                .get(((times.len().saturating_sub(1) as f64) * p).round() as usize)
                .copied()
        };
        let mut empty_times = samples
            .iter()
            .filter(|s| s["operation"] == "empty_history_key")
            .filter_map(|s| s["total_us"].as_u64())
            .collect::<Vec<_>>();
        empty_times.sort_unstable();
        let empty_metric = |p: f64| {
            empty_times
                .get(((empty_times.len().saturating_sub(1) as f64) * p).round() as usize)
                .copied()
        };
        fs::write(
            self.out.join("assistant-latency-report.json"),
            serde_json::to_vec_pretty(
                &json!({"passed":passed,"stage":self.stage,"error":error,"elapsed_ms":self.start.elapsed().as_millis(),"cases":self.cases,"samples":*samples,"typing_total_us":{"p50":metric(0.5),"p95":metric(0.95),"max":times.last()},"empty_history_typing_total_us":{"p50":empty_metric(0.5),"p95":empty_metric(0.95),"max":empty_times.last()},"hidden_native_window":true,"physical_input_verified":false,"measures_window_draw_not_physical_presentation":true,"absolute_latency_threshold_enforced":false,"require_cached_history":std::env::var("CLARORA_ASSISTANT_LATENCY_REQUIRE_CACHED").as_deref()==Ok("1")}),
            )?,
        )?;
        Ok(())
    }
    fn step(
        &mut self,
        app: &mut AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Result<bool> {
        ensure!(
            self.start.elapsed() < Duration::from_secs(150),
            "assistant latency timeout stage {}: {}",
            self.stage,
            app.status
        );
        if self.samples.borrow().len() < self.requested {
            return Ok(false);
        }
        match self.stage {
            0 => {
                app.open_assistant(window, cx);
                self.stage = 1;
            }
            1 => {
                if app.chat.is_loading()
                    || app.chat.context != clarora_gpui::assistant::CONTEXT
                    || app.chat.id != SESSION
                {
                    return Ok(false);
                }
                ensure!(
                    app.chat.history.len() == 2
                        && app.chat.history[1]["content"]
                            .as_str()
                            .is_some_and(|s| s.len() >= 36_000),
                    "long history fixture not loaded"
                );
                app.set("question", "", window, cx);
                app.inputs["question"].update(cx, |input, cx| input.focus(window, cx));
                self.clipboard = cx.read_from_clipboard();
                self.clipboard_touched = true;
                cx.write_to_clipboard(ClipboardItem::new_string(PASTE.into()));
                self.sample("warm_history", Edit::Frame, app, window, cx);
                self.stage = 2;
            }
            2 => {
                self.sample(
                    "chinese_paste",
                    Edit::Key(
                        if cfg!(target_os = "macos") {
                            "cmd-v"
                        } else {
                            "ctrl-v"
                        }
                        .into(),
                    ),
                    app,
                    window,
                    cx,
                );
                self.stage = 3;
            }
            3 => {
                ensure!(
                    app.value("question", cx) == PASTE,
                    "Chinese paste changed input text"
                );
                self.typing_builds = app.assistant.debug_history_builds.get();
                self.note(
                    "Chinese clipboard paste preserves Unicode and newlines",
                    json!({"bytes":PASTE.len()}),
                );
                self.stage = 4;
            }
            4 => {
                if self.typed < KEYS {
                    self.sample("short_key", Edit::Key("s".into()), app, window, cx);
                    self.typed += 1;
                    return Ok(false);
                }
                let expected = format!("{PASTE}{}", "s".repeat(KEYS));
                ensure!(
                    app.value("question", cx) == expected,
                    "short keystrokes were lost or reordered"
                );
                let delta = app
                    .assistant
                    .debug_history_builds
                    .get()
                    .saturating_sub(self.typing_builds);
                if std::env::var("CLARORA_ASSISTANT_LATENCY_REQUIRE_CACHED").as_deref() == Ok("1") {
                    ensure!(
                        delta == 0,
                        "editing rebuilt unchanged history {delta} times"
                    );
                }
                self.note(
                    "short key sequence preserves text and records latency/history rebuilds",
                    json!({"keys":KEYS,"history_build_delta":delta}),
                );
                self.sample(
                    "select_all",
                    Edit::Key(
                        if cfg!(target_os = "macos") {
                            "cmd-a"
                        } else {
                            "ctrl-a"
                        }
                        .into(),
                    ),
                    app,
                    window,
                    cx,
                );
                self.stage = 5;
            }
            5 => {
                ensure!(
                    app.inputs["question"]
                        .update(cx, |s, cx| s.selected_text_range(false, window, cx))
                        .is_some_and(|r| !r.range.is_empty()),
                    "native select all did not select draft"
                );
                self.sample(
                    "copy_draft",
                    Edit::Key(
                        if cfg!(target_os = "macos") {
                            "cmd-c"
                        } else {
                            "ctrl-c"
                        }
                        .into(),
                    ),
                    app,
                    window,
                    cx,
                );
                self.stage = 6;
            }
            6 => {
                let expected = format!("{PASTE}{}", "s".repeat(KEYS));
                ensure!(
                    cx.read_from_clipboard().and_then(|c| c.text()).as_deref()
                        == Some(expected.as_str()),
                    "native copy lost draft text"
                );
                self.clipboard_restore(cx);
                fs::write(self.out.join("draft-final.txt"), &expected)?;
                self.note(
                    "native selection and copy preserve the complete draft",
                    json!({"bytes":expected.len(),"clipboard_restored":true}),
                );
                self.replace_builds = app.assistant.debug_history_builds.get();
                let original = app.chat.history[1]["content"]
                    .as_str()
                    .context("history")?
                    .to_string();
                let replacement = original.replacen("As shown", "As noted", 1);
                ensure!(
                    original.len() == replacement.len() && original != replacement,
                    "same-length replacement fixture"
                );
                app.chat.history[1]["content"] = json!(replacement.clone());
                cx.notify();
                self.sample(
                    "same_length_history_replacement",
                    Edit::Frame,
                    app,
                    window,
                    cx,
                );
                self.stage = 7;
            }
            7 => {
                if app.assistant.debug_history_builds.get() <= self.replace_builds
                    && self.replacement_frames < 3
                {
                    // Child sync notifications become dirty effects after the
                    // parent's draw; a hidden window needs an explicit next draw.
                    self.replacement_frames += 1;
                    self.sample("history_replacement_followup", Edit::Frame, app, window, cx);
                    return Ok(false);
                }
                ensure!(
                    app.assistant.debug_history_builds.get() > self.replace_builds,
                    "same-length history replacement did not invalidate history view"
                );
                self.note("same-length answer replacement refreshes history rendering",json!({"history_build_delta":app.assistant.debug_history_builds.get()-self.replace_builds}));
                app.chat.history.clear();
                app.chat.output.clear();
                app.chat.reasoning.clear();
                app.set("question", PASTE, window, cx);
                app.inputs["question"].update(cx, |input, cx| {
                    input.set_cursor_position(
                        gpui_component::input::Position::new(PASTE.lines().count() as u32, 0),
                        window,
                        cx,
                    )
                });
                cx.notify();
                self.sample("empty_history_warm", Edit::Frame, app, window, cx);
                self.stage = 8;
            }
            8 => {
                if self.empty_warmups == 0 {
                    self.empty_warmups = 1;
                    self.sample("empty_history_warm_followup", Edit::Frame, app, window, cx);
                    return Ok(false);
                }
                if self.empty_typed == 0 {
                    self.empty_builds = app.assistant.debug_history_builds.get();
                }
                if self.empty_typed < KEYS {
                    self.sample("empty_history_key", Edit::Key("s".into()), app, window, cx);
                    self.empty_typed += 1;
                    return Ok(false);
                }
                let expected = format!("{PASTE}{}", "s".repeat(KEYS));
                ensure!(
                    app.value("question", cx) == expected,
                    "empty-history control changed editor contents"
                );
                self.note("empty-history control uses identical editor contents and key sequence",json!({"keys":KEYS,"editor_bytes":expected.len(),"history_build_delta":app.assistant.debug_history_builds.get().saturating_sub(self.empty_builds)}));
                let editor = app.value("question", cx);
                app.chat.observe_draft(&editor);
                self.flush = Some(app.chat.flush_async());
                self.stage = 9;
            }
            9 => {
                let Some(reply) = &self.flush else {
                    anyhow::bail!("flush receiver missing")
                };
                match reply.try_recv() {
                    Ok(result) => result.map_err(anyhow::Error::msg)?,
                    Err(mpsc::TryRecvError::Empty) => return Ok(false),
                    Err(error) => anyhow::bail!("draft flush: {error}"),
                }
                let expected = format!("{PASTE}{}", "s".repeat(KEYS));
                ensure!(
                    app.lib
                        .setting(&format!("chat_draft_{}", clarora_gpui::assistant::CONTEXT))?
                        == expected,
                    "saved draft differs from editor"
                );
                self.note(
                    "asynchronous draft flush saves exact editor contents",
                    json!({"bytes":expected.len()}),
                );
                self.report(true, None)?;
                return Ok(true);
            }
            _ => {}
        }
        Ok(false)
    }
}
impl diagnostics::Probe for Probe {
    fn tick(
        &mut self,
        app: &mut AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Result<bool> {
        self.step(app, window, cx)
    }
    fn failed(&mut self, error: String) -> Result<()> {
        self.report(false, Some(error))
    }
    fn cleanup(&mut self, _app: &mut AppView, _window: &mut Window, cx: &mut Context<AppView>) {
        self.clipboard_restore(cx);
    }
}
