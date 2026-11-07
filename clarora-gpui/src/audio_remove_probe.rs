//! Actual modal actions and SQLite deletion; not physical pointer input evidence.
use super::*;
const A: &str = "probe-audio-a";
const B: &str = "probe-audio-b";
const EXTRA: &str = "remove-extra";
const GROUP: &str = "probe-group-a";
pub(super) fn prepare(out: &Path) -> Result<Library> {
    let lib = ui_controller_probe::prepare(out)?;
    let mut extra = lib
        .list("listening_audios")?
        .into_iter()
        .find(|r| text(r, "id") == A)
        .context("fixture audio")?;
    extra.insert("id".into(), json!(EXTRA));
    extra.insert("name".into(), json!("额外的音频"));
    lib.put("listening_audios", Value::Object(extra))?;
    Ok(lib)
}
pub(super) struct Probe {
    out: PathBuf,
    start: Instant,
    stage: u8,
    originals: Vec<Row>,
    cases: Vec<Value>,
}
impl Probe {
    pub fn new(out: &Path) -> Self {
        Self {
            out: out.into(),
            start: Instant::now(),
            stage: 0,
            originals: vec![],
            cases: vec![],
        }
    }
    fn key(key: &str, window: &mut Window, cx: &mut Context<AppView>) {
        let stroke = Keystroke::parse(key).expect("diagnostic keystroke");
        let handle = window.window_handle();
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| {
                window.dispatch_keystroke(stroke, cx);
            });
        });
    }
    fn note(&mut self, label: &str) {
        self.cases.push(json!({"case":label,"passed":true}));
    }
    fn restore(&self, app: &mut AppView) -> Result<()> {
        let existing: std::collections::BTreeSet<_> = app
            .lib
            .list("listening_audios")?
            .iter()
            .map(|row| text(row, "id"))
            .collect();
        for row in &self.originals {
            if !existing.contains(&text(row, "id")) {
                app.lib
                    .put("listening_audios", Value::Object(row.clone()))?;
            }
        }
        app.refresh();
        Ok(())
    }
    fn report(&self, passed: bool, error: Option<String>) -> Result<()> {
        fs::write(
            self.out.join("audio-remove-report.json"),
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
            self.start.elapsed() < Duration::from_secs(30),
            "audio remove dialog timeout stage {}: {}",
            self.stage,
            app.status
        );
        app.poll_data();
        match self.stage {
            0 => {
                self.originals = app.lib.list("listening_audios")?;
                app.handle(Action::Page("library".into()), window, cx);
            }
            1 => {
                if app.audio.audios.len() != 3 || app.audio.practice_id != GROUP {
                    return Ok(false);
                }
                app.handle(Action::LibraryDelete(vec![A.into()]), window, cx);
                ensure!(
                    window.has_active_dialog(cx) && app.audio.library.pending_delete.len() == 1,
                    "single remove did not open modal"
                );
                ensure!(
                    app.lib.list("listening_audios")?.len() == 3,
                    "opening removed record"
                );
                self.note("single audio opens modal without deleting");
            }
            2 => Self::key("escape", window, cx),
            3 => {
                if window.has_active_dialog(cx) {
                    return Ok(false);
                }
                ensure!(
                    app.audio.library.pending_delete.is_empty()
                        && app.lib.list("listening_audios")?.len() == 3,
                    "cancel changed records or retained pending selection"
                );
                self.note("real dialog cancel clears pending state and preserves records");
                app.handle(Action::LibraryDelete(vec![A.into()]), window, cx);
            }
            4 => Self::key("enter", window, cx),
            5 => {
                if window.has_active_dialog(cx) {
                    return Ok(false);
                }
                let rows = app.lib.list("listening_audios")?;
                ensure!(
                    rows.len() == 2 && !rows.iter().any(|r| text(r, "id") == A),
                    "single confirmation did not remove only target"
                );
                self.note("real confirm callback passes modal guard and removes one target");
                self.restore(app)?;
            }
            6 => {
                if app.audio.audios.len() != 3 {
                    return Ok(false);
                }
                app.handle(
                    Action::LibraryDelete(vec![A.into(), EXTRA.into()]),
                    window,
                    cx,
                );
                ensure!(
                    window.has_active_dialog(cx) && app.audio.library.pending_delete.len() == 2,
                    "batch modal"
                );
            }
            7 => Self::key("enter", window, cx),
            8 => {
                if window.has_active_dialog(cx) {
                    return Ok(false);
                }
                let rows = app.lib.list("listening_audios")?;
                ensure!(
                    rows.len() == 1 && text(&rows[0], "id") == B,
                    "batch removed wrong records"
                );
                for row in &self.originals {
                    ensure!(
                        Path::new(&text(row, "audio_uri")).is_file()
                            && Path::new(&text(row, "subtitle_uri")).is_file(),
                        "attachment deleted"
                    );
                }
                self.note(
                    "batch confirm removes selected records only and preserves all attachments",
                );
                self.restore(app)?;
            }
            9 => {
                if app.audio.audios.len() != 3 {
                    return Ok(false);
                }
                app.handle(
                    Action::LibraryDelete(vec![A.into(), EXTRA.into()]),
                    window,
                    cx,
                );
                audio_library::move_records(&app.lib, GROUP, "probe-group-b", &[EXTRA.into()])?;
            }
            10 => Self::key("enter", window, cx),
            11 => {
                ensure!(
                    window.has_active_dialog(cx)
                        && app.audio.library.pending_delete.len() == 2
                        && app.lib.list("listening_audios")?.len() == 3,
                    "changed group did not reject batch atomically"
                );
                self.note("concurrent membership change rejects atomically and keeps modal open");
                Self::key("escape", window, cx);
            }
            12 => {
                if window.has_active_dialog(cx) {
                    return Ok(false);
                }
                app.handle(Action::LibraryConfirmDelete(GROUP.into()), window, cx);
                ensure!(
                    app.lib.list("listening_audios")?.len() == 3
                        && app.audio.library.pending_delete.is_empty(),
                    "unarmed confirmation removed records"
                );
                self.note("confirmation outside modal cannot remove anything");
                self.report(true, None)?;
                return Ok(true);
            }
            _ => {}
        }
        self.stage += 1;
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
}
