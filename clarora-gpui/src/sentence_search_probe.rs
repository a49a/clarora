//! Actual hidden AppView/InputState/background scan/player flow; not physical input evidence.
use super::*;
use sentence_search_ui::Command;
pub(super) fn prepare(out: &Path) -> Result<Library> {
    let lib = ui_controller_probe::prepare(out)?;
    for row in lib.list("listening_audios")? {
        fs::write(
            text(&row, "subtitle_uri"),
            "1\n00:00:00,200 --> 00:00:00,650\nWhen is the next train?\n下一班车什么时候到？\n\n2\n00:00:00,800 --> 00:00:01,250\nWhy did you leave?\n",
        )?;
    }
    lib.set_setting("audio_repeat_mode", "6")?;
    Ok(lib)
}
pub(super) struct Probe {
    out: PathBuf,
    start: Instant,
    stage: u8,
    cases: Vec<Value>,
}
impl Probe {
    pub fn new(out: &Path) -> Self {
        Self {
            out: out.into(),
            start: Instant::now(),
            stage: 0,
            cases: vec![],
        }
    }
    fn note(&mut self, name: &str) {
        self.cases.push(json!({"case":name,"passed":true}));
    }
    fn report(&self, passed: bool, error: Option<String>) -> Result<()> {
        fs::write(
            self.out.join("sentence-search-native.json"),
            serde_json::to_vec_pretty(
                &json!({"passed":passed,"stage":self.stage,"error":error,"cases":self.cases,"hidden_native_window":true,"physical_input_verified":false}),
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
            self.start.elapsed() < Duration::from_secs(35),
            "native search timeout stage {}: {}",
            self.stage,
            app.sentences.error
        );
        app.poll_sentence_search();
        match self.stage {
            0 => {
                app.handle(Action::Page("sentences".into()), window, cx);
                app.inputs["sentence_query"].update(cx, |input, cx| {
                    input.set_value("When", window, cx);
                    cx.emit(InputEvent::PressEnter { secondary: false });
                });
            }
            1 => {
                let Some(results) = &app.sentences.results else {
                    return Ok(false);
                };
                ensure!(
                    results.hits.len() == 2
                        && results
                            .hits
                            .iter()
                            .all(|h| h.english.starts_with("When is")),
                    "Enter search did not use all groups"
                );
                let _ = app.sentence_search_view(window, cx);
                self.note("native InputState Enter invokes cross-group search and renders results");
                app.handle(Action::Sentence(Command::Play(0)), window, cx);
            }
            2 => {
                let queue = app.sentences.audition.as_ref().context("queue")?;
                if queue.is_loading()
                    || queue.snapshot().paused
                    || queue.snapshot().sentence_repeat.is_none()
                {
                    return Ok(false);
                }
                self.note("native Play action loads and plays exact result");
                app.handle(Action::Next, window, cx);
                ensure!(
                    app.sentences.audition.as_ref().unwrap().index == Some(1),
                    "global next escaped search queue"
                );
                app.handle(Action::Previous, window, cx);
                ensure!(
                    app.sentences.audition.as_ref().unwrap().index == Some(0),
                    "global previous escaped search queue"
                );
                self.note("global next/previous commands remain in the search queue");
                let key = KeyDownEvent {
                    keystroke: Keystroke::parse("right")?,
                    is_held: false,
                };
                ensure!(
                    app.handle_sentence_search_key(&key, window, cx),
                    "right key not handled"
                );
                ensure!(
                    app.sentences.audition.as_ref().unwrap().index == Some(1),
                    "right key did not select next result"
                );
                app.inputs["sentence_query"].update(cx, |input, cx| input.focus(window, cx));
                ensure!(
                    !app.handle_sentence_search_key(&key, window, cx),
                    "query input arrows stolen"
                );
                self.note("keyboard next works and query editing retains arrow keys");
            }
            3 => {
                if !app
                    .sentences
                    .audition
                    .as_ref()
                    .is_some_and(|q| q.finished && q.snapshot().paused)
                {
                    return Ok(false);
                }
                self.note("last native result ends paused");
                app.handle(Action::Page("settings".into()), window, cx);
                ensure!(
                    app.sentences
                        .audition
                        .as_ref()
                        .is_some_and(|q| q.finished && q.index == Some(1)),
                    "leaving search left queue active"
                );
                ensure!(
                    app.lib.setting("audio_repeat_mode")? == "6",
                    "search changed ordinary audio preference"
                );
                self.note("page departure stops audition and preserves normal mode");
                app.handle(Action::Page("sentences".into()), window, cx);
                app.handle(
                    Action::Sentence(Command::Preset("Why did".into())),
                    window,
                    cx,
                );
                app.handle(
                    Action::Sentence(Command::Preset("When is".into())),
                    window,
                    cx,
                );
            }
            4 => {
                let Some(results) = &app.sentences.results else {
                    return Ok(false);
                };
                ensure!(
                    app.sentences.phrase == "When is"
                        && results.hits.len() == 2
                        && results
                            .hits
                            .iter()
                            .all(|h| h.english.starts_with("When is")),
                    "stale query overwrote latest results"
                );
                self.note("superseded background query cannot overwrite latest results");
                app.handle(
                    Action::Sentence(Command::Mode(
                        clarora_gpui::sentence_search::MatchMode::Contains,
                    )),
                    window,
                    cx,
                );
                app.handle(
                    Action::Sentence(Command::Preset("next train".into())),
                    window,
                    cx,
                );
            }
            5 => {
                let Some(results) = &app.sentences.results else {
                    return Ok(false);
                };
                ensure!(
                    results.hits.len() == 2,
                    "contains filter did not use new query"
                );
                let _ = app.sentence_search_view(window, cx);
                self.note("native contains filter and presets render the latest query");
                app.handle(Action::Sentence(Command::Preset("Why".into())), window, cx);
                app.handle(Action::Sentence(Command::Cancel), window, cx);
                self.note("cancel detaches pending results");
                self.report(true, None)?;
                return Ok(true);
            }
            _ => {}
        }
        self.stage += 1;
        Ok(false)
    }
}
impl AppView {
    pub(super) fn poll_sentence_search_probe(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(mut probe) = self.sentence_probe.take() else {
            return false;
        };
        match probe.step(self, window, cx) {
            Ok(true) => {
                window.remove_window();
                return true;
            }
            Err(error) => {
                let _ = probe.report(false, Some(format!("{error:#}")));
                window.remove_window();
                return true;
            }
            Ok(false) => {}
        }
        self.sentence_probe = Some(probe);
        false
    }
}
