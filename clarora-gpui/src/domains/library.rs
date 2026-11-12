//! Library commands own their action effects and execution.
use super::*;
pub(super) enum Command {
    Practice(String),
    LibrarySelect { id: String, reveal: bool },
    LibraryRenameAudio(String),
    LibraryCommitRename,
    LibraryCancelRename,
    LibraryTool(String),
    LibrarySaveName,
    LibraryFilter(u8),
    LibrarySort(u8),
    LibraryBatch,
    LibraryShowSources,
    LibrarySelectAll,
    LibraryReset,
    LibraryMove(String),
    LibraryDelete(Vec<String>),
    LibraryConfirmDelete(String),
    LibraryStudy(bool),
    MergeAudios,
    CheckAudio(String),
    RunImport,
    ConfigureCommandImport,
    SaveCommandImport,
    NameImportedAudio,
    CloseFloatingNames,
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
            Command::Practice(key) => {
                ensure!(
                    app.audio.practices.iter().any(|r| text(r, "id") == key),
                    "练习组不存在"
                );
                app.lib.set_setting("library_group", &key)?;
                app.audio.practice_id = key;
                app.audio.library.cancel_rename();
                app.audio.checked.clear();
                app.audio.library.selected.clear();
                app.audio.library.filter = 0;
                app.audio.library.tool.clear();
                app.audio.library.pending_delete.clear();
                app.set("library_search", "", window, cx);
                app.refresh();
            }

            Command::LibrarySelect { id: key, reveal } => {
                ensure!(
                    app.audio
                        .audios
                        .iter()
                        .any(|r| text(r, "id") == key
                            && text(r, "practice_id") == app.audio.practice_id),
                    "音频已不存在"
                );
                if app
                    .audio
                    .library
                    .renaming
                    .as_ref()
                    .is_some_and(|target| target.id != key)
                {
                    app.audio.library.cancel_rename();
                }
                if app.audio.library.selected != key {
                    app.audio.library.preview.clear();
                    app.audio.library.subtitle_count = 0;
                }
                app.audio.library.selected = key;
                if reveal {
                    app.audio.library.tool = "detail".into();
                    app.page_scroll.set_offset(point(px(0.), px(0.)));
                }
                app.audio.library.pending_delete.clear();
                app.refresh();
            }

            Command::LibraryRenameAudio(key) => {
                ensure!(app.page == "library", "请先返回音频管理");
                ensure!(!app.tasks.foreground.is_running(), "请等待当前任务完成");
                let name = app.audio.library.begin_rename(
                    &app.audio.practice_id,
                    &app.audio.audios,
                    &key,
                )?;
                // Preserve the list's position when details already precede it in a narrow window.
                if app.audio.library.tool != "detail" {
                    app.audio.library.tool.clear();
                }
                app.audio.library.pending_delete.clear();
                let query = app.value("library_search", cx);
                if !app
                    .audio
                    .library
                    .visible(&app.audio.audios, &app.audio.practice_id, &query)
                    .iter()
                    .any(|r| text(r, "id") == key)
                {
                    app.audio.library.filter = 0;
                    app.set("library_search", "", window, cx);
                }
                app.refresh();
                app.set("library_audio_name", name, window, cx);
                app.inputs["library_audio_name"].update(cx, |state, cx| state.focus(window, cx));
                cx.on_next_frame(window, move |this, window, cx| {
                    if this.page == "library"
                        && this
                            .audio
                            .library
                            .renaming
                            .as_ref()
                            .is_some_and(|t| t.id == key)
                    {
                        let query = this.value("library_search", cx);
                        if let Some(index) = this
                            .audio
                            .library
                            .visible(&this.audio.audios, &this.audio.practice_id, &query)
                            .iter()
                            .position(|r| text(r, "id") == key)
                        {
                            this.audio.library_scroll.scroll_to_item(index);
                        }
                        this.inputs["library_audio_name"]
                            .update(cx, |state, cx| state.focus(window, cx));
                        window.dispatch_action(Box::new(gpui_component::input::SelectAll), cx);
                        cx.notify();
                    }
                });
            }

            Command::LibraryCommitRename => {
                ensure!(app.page == "library", "请先返回音频管理");
                ensure!(!app.tasks.foreground.is_running(), "请等待当前任务完成");
                let name = app.value("library_audio_name", cx);
                app.audio
                    .library
                    .save_rename(&app.lib, &app.audio.practice_id, &name)?;
                app.refresh();
                window.focus(&app.focus);
                app.report_status(FeedbackKind::Info, "名称已保存".into());
            }

            Command::LibraryCancelRename => {
                app.audio.library.cancel_rename();
                window.focus(&app.focus);
            }

            Command::LibraryTool(tool) => {
                app.audio.library.cancel_rename();
                app.audio.library.pending_delete.clear();
                let name = match tool.as_str() {
                    "group-new" => audio_library::default_practice_name(
                        &app.audio.practices,
                        chrono::Local::now().date_naive(),
                    ),
                    "group-rename" => app
                        .audio
                        .practices
                        .iter()
                        .find(|r| text(r, "id") == app.audio.practice_id)
                        .map(|r| text(r, "name"))
                        .unwrap_or_default(),
                    _ => String::new(),
                };
                app.set("library_name", name, window, cx);
                app.audio.library.tool = tool;
                app.page_scroll.set_offset(point(px(0.), px(0.)));
                let field = if app.audio.library.tool == "tts" {
                    Some("tts_text")
                } else if ["group-new", "group-rename"].contains(&app.audio.library.tool.as_str()) {
                    Some("library_name")
                } else {
                    None
                };
                if let Some(field) = field {
                    app.inputs[field].update(cx, |state, cx| state.focus(window, cx));
                }
            }

            Command::LibrarySaveName => {
                ensure!(!app.tasks.foreground.is_running(), "请等待当前任务完成");
                let name = app.value("library_name", cx);
                match app.audio.library.tool.as_str() {
                    "group-new" => {
                        ensure!(
                            !name.trim().is_empty() && name.chars().count() <= 120,
                            "名称需为 1–120 字"
                        );
                        ensure!(
                            !app.audio
                                .practices
                                .iter()
                                .any(|r| text(r, "name").trim().eq_ignore_ascii_case(name.trim())),
                            "已有同名练习组，请直接选择或换一个名称"
                        );
                        app.audio.practice_id = app.lib.practice(name.trim())?;
                        app.audio.checked.clear();
                        app.audio.library.selected.clear();
                        app.audio.library.filter = 0;
                        app.set("library_search", "", window, cx);
                    }
                    "group-rename" => app.lib.rename_practice(&app.audio.practice_id, &name)?,
                    _ => anyhow::bail!("请重新打开名称编辑"),
                }
                app.lib
                    .set_setting("library_group", &app.audio.practice_id)?;
                app.audio.library.tool.clear();
                app.refresh();
                app.report_status(FeedbackKind::Info, "名称已保存".into());
            }

            Command::LibraryFilter(filter) => {
                app.audio.library.filter = filter;
            }

            Command::LibrarySort(sort) => {
                app.audio.library.sort = sort;
            }

            Command::LibraryBatch => {
                app.audio.library.batch = !app.audio.library.batch;
                app.audio.checked.clear();
                app.audio.library.pending_delete.clear();
            }

            Command::LibraryShowSources => {
                app.audio.library.show_sources = !app.audio.library.show_sources;
                app.audio.library.cancel_rename();
                app.audio.checked.clear();
                app.refresh();
            }

            Command::LibraryReset => {
                app.audio.library.filter = 0;
                app.set("library_search", "", window, cx);
            }

            Command::LibrarySelectAll => {
                let ids = app
                    .audio
                    .library
                    .visible(
                        &app.audio.audios,
                        &app.audio.practice_id,
                        &app.value("library_search", cx),
                    )
                    .into_iter()
                    .map(|r| text(r, "id"))
                    .collect::<Vec<_>>();
                if !ids.is_empty() && ids.iter().all(|id| app.audio.checked.contains(id)) {
                    for id in ids {
                        app.audio.checked.remove(&id);
                    }
                } else {
                    app.audio.checked.extend(ids);
                }
                app.audio.library.pending_delete.clear();
            }

            Command::LibraryMove(destination) => {
                ensure!(!app.tasks.foreground.is_running(), "请等待当前任务完成");
                let ids = audio_library::scoped_ids(
                    &app.audio.audios,
                    &app.audio.practice_id,
                    &app.audio.checked,
                );
                app.lib
                    .move_audio_records(&app.audio.practice_id, &destination, &ids)?;
                app.audio.checked.clear();
                app.audio.library.tool.clear();
                app.refresh();
                app.report_status(FeedbackKind::Info, format!("已移动 {} 条音频", ids.len()));
            }

            Command::LibraryDelete(ids) => {
                app.open_audio_remove_dialog(ids, window, cx)?;
            }

            Command::LibraryConfirmDelete(group) => {
                ensure!(window.has_active_dialog(cx), "请先打开音频移除确认弹框");
                ensure!(
                    app.page == "library" && app.audio.practice_id == group,
                    "练习组已切换，请取消后重新选择音频"
                );
                ensure!(!app.tasks.foreground.is_running(), "请等待当前任务完成");
                let ids = app.audio.library.pending_delete.clone();
                app.lib.delete_audio_records(&group, &ids)?;
                app.audio.checked.clear();
                app.audio.library.pending_delete.clear();
                app.audio.library.tool.clear();
                app.refresh();
                app.report_status(
                    FeedbackKind::Info,
                    format!("已移除 {} 条记录，附件文件保留", ids.len()),
                );
            }

            Command::LibraryStudy(paused) => {
                let key = app.audio.library.selected.clone();
                ensure!(
                    app.audio
                        .audios
                        .iter()
                        .any(|r| text(r, "id") == key
                            && text(r, "practice_id") == app.audio.practice_id),
                    "请先选择音频"
                );
                app.audio.blind.enabled = false;
                app.handle(Action::Audio(key), window, cx);
                if paused {
                    app.media.player.set("pause", "yes");
                }
            }

            Command::CheckAudio(key) => {
                ensure!(
                    app.audio
                        .audios
                        .iter()
                        .any(|r| text(r, "id") == key
                            && text(r, "practice_id") == app.audio.practice_id),
                    "音频不属于当前组"
                );
                app.audio.library.pending_delete.clear();
                if !app.audio.checked.insert(key.clone()) {
                    app.audio.checked.remove(&key);
                }
            }

            Command::MergeAudios => {
                ensure!(
                    !app.tasks.foreground.is_running(),
                    "请完成或取消当前任务后再合并"
                );
                let rows = app.audio.library.ordered_checked(
                    &app.audio.audios,
                    &app.audio.practice_id,
                    &app.audio.checked,
                );
                ensure!(rows.len() >= 2, "请至少勾选两条音频");
                let group = app.audio.practice_id.clone();
                app.begin_scoped(
                    TaskScope::Application,
                    "正在合并音频与字幕…",
                    move |lib, cancel, _| {
                        Ok(Outcome::AudioMerged(clarora_gpui::audio_tools::merge(
                            &lib, &rows, &group, &cancel,
                        )?))
                    },
                );
            }

            Command::RunImport => {
                let command = app.import_command.clone();
                ensure!(
                    !app.tasks.foreground.is_running(),
                    "已有任务正在执行，请完成或取消后重试"
                );
                ensure!(
                    !app.floating_name_window
                        .is_some_and(|handle| cx.windows().contains(&handle)),
                    "请先保存音频名称，或关闭命名面板后继续导入"
                );
                let request = clarora_gpui::command_import::Request::new(
                    &app.lib,
                    &app.audio.practice_id,
                    &command,
                )?;
                let label = format!("正在导入至「{}」…", request.group_name());
                app.command_imported = None;
                app.begin_scoped(TaskScope::Application, &label, move |lib, cancel, _| {
                    Ok(Outcome::CommandImported(request.run(&lib, &cancel)?))
                });
                app.tasks.notice = Some(TaskNotice::new("命令导入音频", &label));
            }

            Command::NameImportedAudio => app.show_floating_names(window, cx)?,

            Command::CloseFloatingNames => app.close_floating_names(cx),

            Command::ConfigureCommandImport => {
                app.handle(Action::Page("settings".into()), window, cx);
                if app.page == "settings" {
                    app.inputs["command"].update(cx, |state, cx| state.focus(window, cx));
                }
            }

            Command::SaveCommandImport => {
                let command = app.value("command", cx);
                app.lib.set_setting("command", &command)?;
                app.import_command = command;
                app.report_status(FeedbackKind::Info, "本机导入命令已保存".into());
            }
        }
        Ok(())
    }
}

// Completion policy belongs to this domain: cancellation may race a durable commit.
pub(super) enum Completed {
    CommandImported(clarora_gpui::command_import::Report),
}
impl Completed {
    pub(super) fn apply(
        self,
        app: &mut AppView,
        context: &mut Completion,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) {
        match self {
            Completed::CommandImported(report) if !context.cancelled => {
                app.report_status(FeedbackKind::Info, report.message());
                app.command_imported = Some(report);
                if let Err(error) = app.show_floating_names(window, cx) {
                    app.report_status(
                        FeedbackKind::Error,
                        format!("音频已导入，打开命名面板失败：{error:#}"),
                    );
                }
            }

            _ => app.report_status(FeedbackKind::Info, "已取消".into()),
        }
    }
}

pub(super) fn leave(app: &mut AppView) {
    app.audio.library.cancel_rename();
    app.audio.library.pending_delete.clear();
    if app.audio.library.tool == "delete" {
        app.audio.library.tool.clear();
    }
}
pub(super) fn enter(app: &mut AppView) -> Result<()> {
    if app.page == "library" {
        let group = app.lib.setting("library_group")?;
        if app.audio.practices.iter().any(|r| text(r, "id") == group) {
            app.audio.practice_id = group;
        }
    }
    Ok(())
}
