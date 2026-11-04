//! Shared single/batch removal confirmation; selection is fixed until dismissal.
use super::*;
impl AppView {
    pub(super) fn open_audio_remove_dialog(
        &mut self,
        ids: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        ensure!(self.page == "library", "请在音频管理中选择要移除的音频");
        ensure!(self.tasks.cancel.is_none(), "请等待当前任务完成");
        ensure!(!ids.is_empty(), "请先选择音频");
        ensure!(
            ids.iter().collect::<std::collections::BTreeSet<_>>().len() == ids.len(),
            "音频选择重复，请重新选择"
        );
        let group = self.audio.practice_id.clone();
        let names = ids
            .iter()
            .map(|id| {
                self.audio
                    .audios
                    .iter()
                    .find(|row| text(row, "id") == *id && text(row, "practice_id") == group)
                    .map(|row| text(row, "name"))
            })
            .collect::<Option<Vec<_>>>()
            .context("所选音频或练习组已改变，请重新选择")?;
        let group_name = self
            .audio
            .practices
            .iter()
            .find(|row| text(row, "id") == group)
            .map(|row| text(row, "name"))
            .unwrap_or_default();
        self.audio.library.cancel_rename();
        self.audio.library.pending_delete = ids;
        self.audio.library.tool.clear();
        let view = cx.entity().downgrade();
        let error = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
        window.open_dialog(cx, move |dialog, window, cx| {
            let confirm_view = view.clone();
            let close_view = view.clone();
            let expected_group = group.clone();
            let confirm_error = error.clone();
            let current_error = error.borrow().clone();
            let mut list = div()
                .id("audio-remove-names")
                .flex()
                .flex_col()
                .gap_2()
                .w_full()
                .min_w_0()
                .max_h((window.viewport_size().height - px(280.)).clamp(px(64.), px(240.)))
                .overflow_y_scroll();
            for name in names.iter().take(30) {
                list = list.child(div().text_sm().child(name.clone()));
            }
            if names.len() > 30 {
                list = list.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("另有 {} 条已选音频", names.len() - 30)),
                );
            }
            dialog
                .title(format!("移除 {} 条音频记录？", names.len()))
                .confirm()
                .w((window.viewport_size().width - px(32.)).min(px(520.)))
                .margin_top(((window.viewport_size().height - px(460.)) / 2.).max(px(16.)))
                .button_props(
                    DialogButtonProps::default()
                        .cancel_text("取消")
                        .ok_text("确认移除")
                        .ok_variant(ButtonVariant::Danger),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("练习组：{group_name}")),
                )
                .child(list)
                .child(
                    div()
                        .text_sm()
                        .child("只移除音频库中的记录，音频文件和字幕附件会保留。"),
                )
                .when(!current_error.is_empty(), |dialog| {
                    dialog.child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().danger)
                            .child(current_error),
                    )
                })
                .on_ok(move |_, window, cx| {
                    confirm_view
                        .update(cx, |this, cx| {
                            this.handle(
                                Action::LibraryConfirmDelete(expected_group.clone()),
                                window,
                                cx,
                            );
                            if this.audio.library.pending_delete.is_empty() {
                                true
                            } else {
                                *confirm_error.borrow_mut() = this.status.clone();
                                cx.notify();
                                false
                            }
                        })
                        .unwrap_or(true)
                })
                .on_close(move |_, _, cx| {
                    let _ = close_view.update(cx, |this, cx| {
                        this.audio.library.pending_delete.clear();
                        cx.notify();
                    });
                })
        });
        Ok(())
    }
}
