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
                app.handle(Action::Page("video".into()), window, cx);
                app.handle(Action::VideoImport, window, cx);
                ensure!(
                    window.has_active_dialog(cx),
                    "video import did not open a dialog"
                );
                window.close_dialog(cx);

                let mut videos = video_library::Collection::load(&app.lib)?;
                videos.create_group("Download target")?;
                videos.create_group("Other group")?;
                videos.save(&app.lib)?;
                app.video.collection = videos;
                app.handle(Action::VideoGroupFilter("Other group".into()), window, cx);
                let origin = app.media.session.revision();
                let downloaded = app.lib.root.join("downloads/youtube-fixture.mp4");
                fs::write(&downloaded, b"fixture")?;
                let task = app.tasks.foreground.begin(TaskScope::Application)?;
                task.complete(Ok(Outcome::VideoDownloaded(
                    downloaded.clone(),
                    origin,
                    "Download target".into(),
                    process::VideoSource::YouTube,
                )));
                app.poll_tasks(window, cx);
                ensure!(
                    app.video.group == "Other group"
                        && video_library::Collection::load(&app.lib)?
                            .active()
                            .is_some_and(|video| video.path == downloaded
                                && video.group == "Download target"),
                    "YouTube result was saved in the current filter instead of the import group"
                );
                self.record(
                    "YouTube completion selects the video and retains the import group",
                    app,
                )?;
                // An import confirmed before a group rename/deletion resolves
                // through the stored alias at completion time.
                app.video
                    .group_aliases
                    .insert("Download target".into(), "Renamed target".into());
                let mut renamed = video_library::Collection::load(&app.lib)?;
                renamed.rename_group("Download target", "Renamed target")?;
                renamed.save(&app.lib)?;
                app.video.collection = renamed;
                let renamed_file = app.lib.root.join("downloads/renamed-fixture.mp4");
                fs::write(&renamed_file, b"renamed fixture")?;
                let task = app.tasks.foreground.begin(TaskScope::Application)?;
                task.complete(Ok(Outcome::VideoDownloaded(
                    renamed_file.clone(),
                    origin,
                    "Download target".into(),
                    process::VideoSource::YouTube,
                )));
                app.poll_tasks(window, cx);
                ensure!(
                    video_library::Collection::load(&app.lib)?
                        .active()
                        .is_some_and(|video| video.group == "Renamed target"),
                    "renamed import target was not followed"
                );
                self.record("in-flight import follows the renamed target group", app)?;
                app.video
                    .group_aliases
                    .insert("Renamed target".into(), String::new());
                let mut deleted = video_library::Collection::load(&app.lib)?;
                deleted.remove_group("Renamed target")?;
                deleted.save(&app.lib)?;
                app.video.collection = deleted;
                let deleted_file = app.lib.root.join("downloads/deleted-target-fixture.mp4");
                fs::write(&deleted_file, b"deleted target fixture")?;
                let task = app.tasks.foreground.begin(TaskScope::Application)?;
                task.complete(Ok(Outcome::VideoDownloaded(
                    deleted_file,
                    origin,
                    "Download target".into(),
                    process::VideoSource::YouTube,
                )));
                app.poll_tasks(window, cx);
                ensure!(
                    video_library::Collection::load(&app.lib)?
                        .active()
                        .is_some_and(|video| video.group.is_empty()),
                    "deleted import target did not fall back to ungrouped"
                );
                self.record(
                    "in-flight import falls back to ungrouped after target deletion",
                    app,
                )?;
                app.video.import_draft = Some(app_state::VideoImportDraft {
                    raw: "https://www.bilibili.com/video/BV1xx411c7mD?p=2".into(),
                    normalized: Some("https://www.bilibili.com/video/BV1xx411c7mD?p=2".into()),
                    part: Some(1),
                    group: "Download target".into(),
                });
                app.video.import_active = true;
                let task = app.tasks.foreground.begin(TaskScope::Application)?;
                task.complete(Err("fixture import failure".into()));
                app.poll_tasks(window, cx);
                ensure!(
                    app.video.import_recovery.as_ref().is_some_and(
                        |draft| draft.part == Some(1) && draft.group == "Download target"
                    ),
                    "failed import lost its part or target group"
                );
                ensure!(
                    app.subtitle_task_banner(cx).is_some(),
                    "failed import recovery card is missing"
                );
                app.handle(Action::Page("listening".into()), window, cx);
                app.handle(Action::VideoImportEdit, window, cx);
                ensure!(
                    app.page == "video" && window.has_active_dialog(cx),
                    "editing a failed import did not switch to video and open the dialog"
                );
                window.close_dialog(cx);
                app.handle(Action::VideoImport, window, cx);
                ensure!(
                    app.page == "video" && window.has_active_dialog(cx),
                    "normal import entry did not reopen the failed draft"
                );
                window.close_dialog(cx);
                self.record(
                    "failed import keeps link, selected part and group for editing",
                    app,
                )?;
                app.handle(Action::DismissVideoImport, window, cx);
                ensure!(
                    app.video.import_recovery.is_none(),
                    "dismissing import recovery retained its parameters"
                );
                let mut stable_groups = video_library::Collection::load(&app.lib)?;
                stable_groups.create_group("甲")?;
                stable_groups.save(&app.lib)?;
                app.video.collection = stable_groups;
                let group_draft = app_state::VideoImportDraft {
                    raw: "https://www.bilibili.com/video/BV1xx411c7mD".into(),
                    normalized: Some("https://www.bilibili.com/video/BV1xx411c7mD".into()),
                    part: None,
                    group: "甲".into(),
                };
                app.video.import_draft = Some(group_draft.clone());
                app.video.import_recovery = Some(group_draft);
                app.rename_video_group("甲", "乙")?;
                ensure!(
                    app.video
                        .import_recovery
                        .as_ref()
                        .is_some_and(|draft| draft.group == "乙"),
                    "renaming a group did not update the failed import draft"
                );
                let mut recreated = video_library::Collection::load(&app.lib)?;
                recreated.create_group("甲")?;
                recreated.save(&app.lib)?;
                app.video.collection = recreated;
                app.video.import_active = true;
                ensure!(
                    app.resolve_import_group("甲") == "乙",
                    "a recreated same-name group captured an older import"
                );
                app.video.import_active = false;
                app.delete_video_group("乙")?;
                ensure!(
                    app.video
                        .import_recovery
                        .as_ref()
                        .is_some_and(|draft| draft.group.is_empty()),
                    "deleting a group did not clear the failed import draft"
                );
                let mut recreated_again = video_library::Collection::load(&app.lib)?;
                recreated_again.create_group("乙")?;
                recreated_again.save(&app.lib)?;
                app.video.collection = recreated_again;
                app.video.import_active = true;
                ensure!(
                    app.resolve_import_group("乙").is_empty(),
                    "recreating a deleted group captured an older import"
                );
                app.video.import_active = false;
                app.video.import_draft = None;
                app.video.import_recovery = None;

                let first = app.lib.root.join("media/first-part.mp4");
                let second = app.lib.root.join("media/second-part.mp4");
                fs::write(&first, b"first part")?;
                fs::write(&second, b"second part")?;
                let series = bilibili::Series {
                    title: "Resume fixture".into(),
                    base: "https://www.bilibili.com/video/BV1xx411c7mD".into(),
                    requested: 0,
                    current: Some(0),
                    parts: vec![
                        bilibili::Part {
                            number: 1,
                            title: "First".into(),
                            duration: 6.,
                            path: Some(first.clone()),
                        },
                        bilibili::Part {
                            number: 2,
                            title: "Second".into(),
                            duration: 6.,
                            path: None,
                        },
                    ],
                };
                let mut videos = video_library::Collection::load(&app.lib)?;
                let ids = videos.add_in_group(
                    std::slice::from_ref(&first),
                    Some(&series),
                    "Download target",
                )?;
                videos.select(&ids[0])?;
                videos.save(&app.lib)?;
                app.video.collection = videos;
                app.video.series = Some(series.clone());
                let origin = app.media.session.revision();
                app.handle(Action::Page("stats".into()), window, cx);
                let bookmark = json!({"path":first,"position":3.}).to_string();
                app.lib.set_setting("last_video_position", &bookmark)?;
                let task = app.tasks.foreground.begin(TaskScope::Application)?;
                task.complete(Ok(Outcome::PartVideo {
                    series,
                    index: 1,
                    path: second.clone(),
                    origin,
                    from_end: false,
                    resume: false,
                    group: "Download target".into(),
                }));
                app.poll_tasks(window, cx);
                let reopened = Library::open(app.lib.root.clone())?;
                let videos = video_library::Collection::load(&reopened)?;
                let active = videos
                    .active()
                    .context("selected series missing after completion")?;
                ensure!(
                    active.path == first
                        && active.series.as_ref().is_some_and(|series| {
                            series.current == Some(0) && series.cached(1) == Some(second)
                        })
                        && reopened.setting("last_video_position")? == bookmark,
                    "late Bilibili completion overwrote the saved P or its position"
                );
                self.record(
                    "background Bilibili completion caches P2 and preserves the P1 bookmark",
                    app,
                )?;

                app.handle(Action::Page("video".into()), window, cx);
                app.video.fullscreen = true;
                let task = app.tasks.foreground.begin(TaskScope::Application)?;
                task.complete(Ok(Outcome::VideoAuthRequired(
                    "https://youtu.be/AUTHFIXTURE".into(),
                    app.media.session.revision(),
                    "Download target".into(),
                    process::VideoSource::YouTube,
                )));
                app.poll_tasks(window, cx);
                ensure!(
                    window.has_active_dialog(cx),
                    "video verification dialog did not open"
                );
                let handle = window.window_handle();
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, cx| {
                        window.draw(cx).clear();
                        window.dispatch_keystroke(Keystroke::parse("escape").unwrap(), cx);
                    });
                });
            }
            7 => {
                ensure!(
                    !window.has_active_dialog(cx) && app.video.fullscreen,
                    "escape did not dismiss the YouTube verification dialog in fullscreen"
                );
                self.record(
                    "fullscreen YouTube verification dialog renders and accepts Escape",
                    app,
                )?;
                app.video.fullscreen = false;
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
