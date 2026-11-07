//! Shell commands own their action effects and execution.
use super::*;
pub(super) enum Command {
    Page(String),
    Pick(String),
    Navigation,
    FloatingImport,
    Refresh,
    GraphSelect(String),
    GraphList(i64),
}
impl Command {
    pub(super) fn effects(&self, app: &AppView) -> Effects {
        Effects {
            leave_video_sentence: match self {
                Command::Pick(kind) => {
                    kind != "video"
                        && !(kind == "video-subtitle" && app.video.pending_start.is_some())
                }
                _ => false,
            },
            refresh_data: matches!(self, Command::Refresh),
            refresh_chat: matches!(self, Command::Refresh),
            ..Effects::default()
        }
    }
    pub(super) fn execute(
        self,
        app: &mut AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Result<()> {
        match self {
            Command::GraphSelect(node) => app.graph_node = node,

            Command::GraphList(list) => app.graph_list = list,

            Command::Page(page) => navigate(app, page, window, cx)?,

            Command::Refresh => {
                app.refresh();
            }

            Command::Pick(kind) => {
                if kind == "backup" {
                    let picker = cx.prompt_for_new_path(&app.lib.root, Some("clarora-library.zip"));
                    cx.spawn(async move |this, cx| {
                        if let Ok(Ok(Some(path))) = picker.await {
                            let _ = this.update(cx, |this, cx| {
                                this.begin("正在备份资料…", move |lib, cancel, _| {
                                    backup::export_local_cancellable(&lib, &path, &cancel)?;
                                    Ok(Outcome::Message("已导出资料与附件".into()))
                                });
                                cx.notify();
                            });
                        }
                    })
                    .detach();
                } else {
                    app.pick(&kind, cx);
                }
            }

            Command::Navigation => {
                let open = !app.navigation_open;
                app.lib
                    .set_setting("navigation_open", if open { "true" } else { "false" })?;
                let from = app.navigation_progress();
                app.navigation_open = open;
                app.navigation_motion = Some((Instant::now(), from));
            }

            Command::FloatingImport => app.toggle_floating_import(window, cx),
        }
        Ok(())
    }
}

/// Keep fallible saves before releasing the current learning session.
fn navigate(
    app: &mut AppView,
    page: String,
    window: &mut Window,
    cx: &mut Context<AppView>,
) -> Result<()> {
    sentences::leave(app, &page);
    app.subtitle_text.borrow_mut().clear();
    if app.page == "writing" {
        app.flush_writing(cx)?;
    }
    audio::prepare_leave(app);
    let previous = app.page.clone();
    library::leave(app);
    app.save_media_position();
    audio::leave(app, &page);
    video::leave(app, &page);
    tasks::leave(app);
    media::leave(app, previous != page);
    app.page = page;
    if app.page == "writing" {
        if let Some(view) = &app.writing {
            view.update(cx, |view, cx| view.reload(cx))?;
        }
    }
    library::enter(app)?;
    app.page_scroll.set_offset(point(px(0.), px(0.)));
    if app.page == "discover" {
        app.card_filter = "all".into();
    }
    app.focus.focus(window);
    if app.page == "sentences" {
        app.inputs["sentence_query"].update(cx, |input, cx| input.focus(window, cx));
    }
    app.revealed = false;
    app.index = 0;
    app.history.clear();
    app.lib.set_setting("last_page", &app.page)?;
    app.refresh();
    cards::enter(app);
    if app.page != previous {
        audio::enter(app);
        video::enter(app);
    }
    reading::enter(app);
    Ok(())
}
