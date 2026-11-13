//! Focus commands own their action effects and execution.
use super::*;

pub(super) enum Command {
    Timer(u64),
    TimerPause,
    TimerReset,
    FocusStart(u64),
    FocusResume,
    FocusEmergency,
    FocusKeep,
    FocusFinish,
    FocusDistraction,
    FocusReview(String, Option<u8>),
    FocusNoteToggle,
    FocusNoteSave,
    FocusNoteDelete(String),
}
impl Command {
    pub(super) fn effects(&self, app: &AppView) -> Effects {
        Effects {
            restore_focus: matches!(app.page.as_str(), "listening" | "video")
                && matches!(
                    self,
                    Self::Timer(..) | Self::TimerPause | Self::TimerReset | Self::FocusFinish
                ),
            ..Effects::default()
        }
    }
    pub(super) fn execute(
        self,
        app: &mut AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Result<()> {
        let now = focus_timer::now_ms();
        match self {
            Command::Timer(seconds) => app.timer.set_goal(&app.lib, seconds)?,
            Command::FocusStart(seconds) => app.start_focus_mode(Some(seconds), window, cx)?,
            Command::FocusResume => app.start_focus_mode(None, window, cx)?,
            Command::FocusEmergency => app.emergency_focus_end(window, cx)?,
            Command::FocusKeep => app.focus_mode.escape = None,
            Command::TimerPause => match app.timer.toggle(&app.lib, now)? {
                focus_timer::Toggle::Started => app.play_timer_chime(timer_chimes::Cue::Start),
                focus_timer::Toggle::Completed(event) => app.focus_phase_completed(event),
                focus_timer::Toggle::Paused | focus_timer::Toggle::Resumed => {}
            },
            Command::TimerReset => {
                app.timer.reset(&app.lib)?;
                app.timer_chimes.stop();
            }
            Command::FocusFinish => {
                if app.focus_mode.active() && !app.timer.goal_met(now) {
                    let view = cx.entity().downgrade();
                    window.open_dialog(cx, move |dialog, window, _cx| {
                        let view = view.clone();
                        dialog
                            .title("还没达到最低目标")
                            .confirm()
                            .w((window.viewport_size().width - px(32.)).min(px(480.)))
                            .button_props(
                                DialogButtonProps::default()
                                    .cancel_text("继续专注")
                                    .ok_text("结束本轮")
                                    .ok_variant(ButtonVariant::Danger),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .child("本轮会按实际专注时长记录，并解除电脑专注、退出全屏。"),
                            )
                            .on_ok(move |_, window, cx| {
                                let _ = view.update(cx, |this, cx| {
                                    if let Err(error) = this.finish_focus_round(window, cx) {
                                        this.report_status(
                                            FeedbackKind::Error,
                                            format!("结束专注失败：{error:#}"),
                                        );
                                    }
                                    cx.notify();
                                });
                                true
                            })
                    });
                    return Ok(());
                }
                app.finish_focus_round(window, cx)?;
            }
            Command::FocusDistraction => app.timer.record_distraction(&app.lib)?,
            Command::FocusReview(id, fatigue) => {
                app.timer.review(&app.lib, &id, fatigue)?;
                if fatigue.is_some() {
                    app.report_status(FeedbackKind::Info, app.focus_advice());
                }
            }
            Command::FocusNoteToggle => {
                app.focus_note_open = !app.focus_note_open;
                if app.focus_note_open {
                    app.inputs["focus_note"].update(cx, |input, cx| input.focus(window, cx));
                } else {
                    window.focus(&app.focus);
                }
            }
            Command::FocusNoteSave => {
                let body = app.value("focus_note", cx);
                let note = focus_notes::save(&app.lib, &body, now)?;
                app.focus_notes.insert(0, note);
                app.focus_notes.truncate(100);
                app.set("focus_note", "", window, cx);
                app.report_status(
                    FeedbackKind::Info,
                    "想法已记录；专注结束后可在「统计」页查看".into(),
                );
            }
            Command::FocusNoteDelete(id) => {
                let preview = app
                    .focus_notes
                    .iter()
                    .find(|note| note.id == id)
                    .map(|note| {
                        let body: String = note.body.chars().take(120).collect();
                        if body.chars().count() < note.body.chars().count() {
                            format!("{body}…")
                        } else {
                            body
                        }
                    })
                    .unwrap_or_default();
                let view = cx.entity().downgrade();
                window.open_dialog(cx, move |dialog, window, cx| {
                    let view = view.clone();
                    let id = id.clone();
                    dialog
                        .title("删除这条想法？")
                        .confirm()
                        .w((window.viewport_size().width - px(32.)).min(px(480.)))
                        .button_props(
                            DialogButtonProps::default()
                                .cancel_text("取消")
                                .ok_text("确认删除")
                                .ok_variant(ButtonVariant::Danger),
                        )
                        .child(div().text_sm().child(preview.clone()))
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("删除后不能恢复；其它想法和专注记录不受影响。"),
                        )
                        .on_ok(move |_, _, cx| {
                            view.update(cx, |this, cx| {
                                match focus_notes::delete(&this.lib, &id) {
                                    Ok(()) => {
                                        this.focus_notes.retain(|note| note.id != id);
                                        this.report_status(
                                            FeedbackKind::Info,
                                            "已删除这条想法".into(),
                                        );
                                    }
                                    Err(error) => this.report_status(
                                        FeedbackKind::Error,
                                        format!("删除想法失败：{error:#}"),
                                    ),
                                }
                                cx.notify();
                                true
                            })
                            .unwrap_or(true);
                            true
                        })
                });
            }
        }
        app.timer_seconds = app.timer.display_seconds(focus_timer::now_ms());
        app.timer_retry = None;
        Ok(())
    }
}
