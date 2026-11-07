//! Actual AppView task delivery and navigation with an isolated SQLite library.
//! Failure cases A-01..T-10 were recorded before implementation.
use super::*;

pub(super) fn prepare(out: &Path) -> Result<Library> {
    let lib = Library::open(out.join("library"))?;
    lib.set_setting("last_page", "settings")?;
    lib.set_setting("assistant_enabled", "true")?;
    lib.set_setting(
        "ai_config",
        &json!({"engine":"compatible","base_url":"http://127.0.0.1:9/v1","model":"task-fixture"})
            .to_string(),
    )?;
    Ok(lib)
}
pub(super) struct Probe {
    out: PathBuf,
    stage: u8,
    started: Instant,
    cases: Vec<Value>,
    request: Option<clarora_gpui::chat_store::Request>,
    token: Option<Cancel>,
}
impl Probe {
    pub(super) fn new(out: &Path) -> Self {
        Self {
            out: out.into(),
            stage: 0,
            started: Instant::now(),
            cases: vec![],
            request: None,
            token: None,
        }
    }
    fn report(&self, passed: Option<bool>, error: Option<String>) -> Result<()> {
        fs::write(
            self.out.join("task-lifecycle-report.json"),
            serde_json::to_vec_pretty(&json!({
                "passed":passed,"error":error,"stage":self.stage,"cases":self.cases,
                "actual_appview_handlers":true,"isolated_sqlite":true,"external_ai_called":false,
                "physical_input_verified":false,"elapsed_ms":self.started.elapsed().as_millis(),
                "replay":["clarora-gpui","--task-lifecycle-check","<new-empty-evidence-directory>"]
            }))?,
        )?;
        Ok(())
    }
    fn record(&mut self, name: &str, app: &AppView) -> Result<()> {
        self.cases
            .push(json!({"case":name,"passed":true,"page":app.page,
            "foreground":app.tasks.foreground.current_id().map(|id|id.value()),
            "chat_running":app.chat.running,"status":app.status}));
        self.report(None, None)
    }
    fn wait_for_cancel(
        _lib: Library,
        token: Cancel,
        report: task_runtime::Reporter<TaskUpdate, Outcome>,
    ) -> Result<Outcome> {
        while !token.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
        }
        let _ = report.send(TaskUpdate::SubtitleProgress(
            "late cancelled progress".into(),
        ));
        anyhow::bail!("fixture cancellation")
    }
    fn check(
        &mut self,
        app: &mut AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Result<bool> {
        ensure!(
            self.started.elapsed() < Duration::from_secs(20),
            "task lifecycle timeout at stage {}",
            self.stage
        );
        match self.stage {
            0 => {
                if app.chat.is_loading() || app.chat.context != app.chat_context_key(cx) {
                    return Ok(false);
                }
                app.chat.output = "keep live chat output".into();
                self.request = Some(app.chat.begin_request());
                app.begin("ordinary task", |lib, _, _| {
                    lib.set_setting("ordinary_task_committed", "yes")?;
                    Ok(Outcome::Output("ordinary task result".into()))
                });
                ensure!(
                    app.tasks.chat_request.is_none(),
                    "ordinary task captured an unrelated chat request"
                );
                self.record("ordinary task explicitly has no chat correlation", app)?;
            }
            1 => {
                if app.tasks.foreground.is_running() || app.chat.is_loading() {
                    return Ok(false);
                }
                ensure!(
                    app.chat.accepts(self.request.as_ref().unwrap())
                        && app.chat.output == "keep live chat output"
                        && app.output == "ordinary task result"
                        && app.lib.setting("ordinary_task_committed")? == "yes",
                    "ordinary completion finished or overwrote live chat"
                );
                self.record(
                    "ordinary completion preserves unrelated live chat and its output",
                    app,
                )?;
                app.chat.finish_request(self.request.as_ref().unwrap());
                app.begin_subtitle("material fixture", "waiting", Self::wait_for_cancel);
                self.token = app.tasks.foreground.cancel_token().cloned();
                let id = app.tasks.foreground.current_id();
                app.begin("must be refused", |_, _, _| {
                    Ok(Outcome::Message("wrong task".into()))
                });
                ensure!(
                    app.tasks.foreground.current_id() == id,
                    "busy start replaced active material task"
                );
                self.record("busy start keeps the active task identity", app)?;
                app.handle(Action::Page("stats".into()), window, cx);
                ensure!(
                    self.token.as_ref().unwrap().load(Ordering::SeqCst),
                    "material task survived page leave"
                );
                self.record("page leave requests cancellation for material scope", app)?;
            }
            2 => {
                if app.tasks.foreground.is_running() {
                    return Ok(false);
                }
                ensure!(
                    app.tasks
                        .notice
                        .as_ref()
                        .is_some_and(|n| n.state == TaskState::Cancelled
                            && n.detail != "late cancelled progress"),
                    "late progress replaced terminal cancellation"
                );
                self.record("cancelled progress cannot overwrite terminal notice", app)?;
                app.begin_scoped(
                    TaskScope::Application,
                    "application fixture",
                    Self::wait_for_cancel,
                );
                self.token = app.tasks.foreground.cancel_token().cloned();
                let id = app.tasks.foreground.current_id();
                app.handle(Action::Page("pdf".into()), window, cx);
                ensure!(
                    app.page == "pdf"
                        && app.tasks.foreground.current_id() == id
                        && !self.token.as_ref().unwrap().load(Ordering::SeqCst)
                        && app.subtitle_task_banner(cx).is_some(),
                    "application task or progress did not survive navigation"
                );
                self.record(
                    "application scope and global progress survive navigation",
                    app,
                )?;
                app.handle(Action::Cancel, window, cx);
                ensure!(
                    self.token.as_ref().unwrap().load(Ordering::SeqCst),
                    "cancel action missed application lane"
                );
            }
            3 => {
                if app.tasks.foreground.is_running() {
                    return Ok(false);
                }
                app.begin_subtitle("panic fixture", "waiting", |_, _, _| -> Result<Outcome> {
                    panic!("isolated foreground panic")
                });
            }
            4 => {
                if app.tasks.foreground.is_running() {
                    return Ok(false);
                }
                ensure!(
                    app.tasks
                        .notice
                        .as_ref()
                        .is_some_and(|n| n.state == TaskState::Failed),
                    "panic left task active or hid failure"
                );
                self.record(
                    "worker panic produces failed notice and releases foreground lane",
                    app,
                )?;
                app.begin("after panic", |lib, _, _| {
                    lib.set_setting("task_after_panic", "yes")?;
                    Ok(Outcome::Message("recovered foreground".into()))
                });
            }
            5 => {
                if app.tasks.foreground.is_running() || app.chat.is_loading() {
                    return Ok(false);
                }
                ensure!(
                    Library::open(app.lib.root.clone())?.setting("task_after_panic")? == "yes",
                    "foreground could not commit after panic"
                );
                self.record("new task can commit and reopen SQLite after panic", app)?;
                let old = app.chat.begin_request();
                app.begin_chat_task(
                    TaskScope::Application,
                    "old chat fixture",
                    old.clone(),
                    |_, _, _| Ok(Outcome::Output("stale chat answer".into())),
                );
                self.request = Some(app.chat.begin_request());
                app.chat.output = "new request output".into();
            }
            6 => {
                if app.tasks.foreground.is_running() || app.chat.is_loading() {
                    return Ok(false);
                }
                ensure!(
                    app.chat.output == "new request output"
                        && app.chat.accepts(self.request.as_ref().unwrap()),
                    "old completed chat painted or finished a new request"
                );
                self.record(
                    "stale chat completion cannot paint or finish a newer request",
                    app,
                )?;
                app.chat.finish_request(self.request.as_ref().unwrap());
                self.report(Some(true), None)?;
                return Ok(true);
            }
            _ => anyhow::bail!("unexpected task stage"),
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
        self.check(app, window, cx)
    }
    fn failed(&mut self, error: String) -> Result<()> {
        self.report(Some(false), Some(error))
    }
    fn cleanup(&mut self, app: &mut AppView, _window: &mut Window, _cx: &mut Context<AppView>) {
        app.tasks.foreground.cancel();
        if let Some(request) = &self.request {
            app.chat.finish_request(request);
        }
    }
}
