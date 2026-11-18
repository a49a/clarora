//! Hidden-window Windows UI smoke: real GPUI window, AppView navigation and assistant.
//! This intentionally avoids macOS-only native pointer events; it is a startup and
//! handler smoke, not mouse/keyboard/visual acceptance.
use super::*;

const PAGES: &[&str] = &[
    "flashcard",
    "listening",
    "video",
    "pdf",
    "settings",
    "stats",
];

pub(super) fn prepare(out: &Path) -> Result<Library> {
    fs::create_dir_all(out)?;
    let lib = Library::open(out.join("library"))?;
    lib.put(
        "sentence_cards",
        json!({
            "id": "ui-smoke-card",
            "text": "A short smoke-test sentence.",
            "translation": "一句冒烟测试文本。",
            "notes": ""
        }),
    )?;
    lib.set_setting("last_page", "flashcard")?;
    Ok(lib)
}

pub(super) struct Probe {
    out: PathBuf,
    started: Instant,
    stage: usize,
    cases: Vec<Value>,
    page_index: usize,
    viewport: Option<(f32, f32)>,
    finished: bool,
}
impl Probe {
    pub(super) fn new(out: &Path) -> Self {
        Self {
            out: out.to_path_buf(),
            started: Instant::now(),
            stage: 0,
            cases: vec![],
            page_index: 0,
            viewport: None,
            finished: false,
        }
    }
    fn report(&self, passed: Option<bool>, error: Option<String>) -> Result<()> {
        fs::write(
            self.out.join("ui-smoke-report.json"),
            serde_json::to_vec_pretty(&json!({
                "passed": passed,
                "error": error,
                "stage": self.stage,
                "elapsed_ms": self.started.elapsed().as_millis(),
                "cases": self.cases,
                "hidden_window": true,
                "actual_window": true,
                "viewport": self.viewport.map(|(width, height)| json!({"width": width, "height": height})),
                "mouse_keyboard_visual_verified": false,
                "independent_review": false
            }))?,
        )?;
        Ok(())
    }
    fn note(&mut self, name: &str, app: &AppView) -> Result<()> {
        self.cases.push(json!({
            "case": name,
            "passed": true,
            "elapsed_ms": self.started.elapsed().as_millis(),
            "page": app.page
        }));
        self.report(None, None)
    }
    fn advance(&mut self) {
        self.stage += 1;
    }
    fn step(
        &mut self,
        app: &mut AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Result<bool> {
        ensure!(
            self.started.elapsed() < Duration::from_secs(60),
            "UI smoke timeout at stage {}",
            self.stage
        );
        match self.stage {
            0 => {
                if !app.cards_ready() {
                    return Ok(false);
                }
                let size = window.viewport_size();
                ensure!(
                    size.width > px(0.) && size.height > px(0.),
                    "UI smoke window has no viewport"
                );
                self.viewport = Some((f32::from(size.width), f32::from(size.height)));
                self.note(
                    "hidden GPUI window created and first AppView frame rendered",
                    app,
                )?;
                self.advance();
            }
            1 => {
                if self.page_index >= PAGES.len() {
                    self.advance();
                    return Ok(false);
                }
                let target = PAGES[self.page_index];
                app.handle(Action::Page(target.into()), window, cx);
                ensure!(app.page == target, "page navigation failed: {target}");
                self.note(&format!("navigate to {target}"), app)?;
                self.page_index += 1;
                if self.page_index == PAGES.len() {
                    self.advance();
                }
            }
            2 => {
                app.open_assistant(window, cx);
                ensure!(app.assistant_open, "assistant did not open");
                self.note("assistant opens", app)?;
                app.assistant_open = false;
                app.lib.set_setting("assistant_open", "false")?;
                cx.notify();
                self.note("assistant closes", app)?;
                self.advance();
            }
            3 => {
                app.handle(Action::Page("flashcard".into()), window, cx);
                ensure!(app.page == "flashcard", "return to flashcard failed");
                self.note("return to flashcard", app)?;
                self.advance();
            }
            _ => {
                self.report(Some(true), None)?;
                self.finished = true;
                return Ok(true);
            }
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
        self.report(Some(false), Some(error))
    }
    fn completed(&mut self) -> Result<()> {
        ensure!(self.finished, "UI smoke finished without a passing report");
        Ok(())
    }
}
