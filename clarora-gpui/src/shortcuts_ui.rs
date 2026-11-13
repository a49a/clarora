//! Keyboard shortcut reference, discoverable from the app menu and F1.
use super::*;

impl AppView {
    pub(super) fn show_shortcuts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.open_dialog(cx, move |dialog, window, cx| {
            let rows = [
                ("学习助手", "Cmd / Ctrl + J"),
                ("展开或收起导航", "Cmd / Ctrl + B"),
                ("选区问 AI", "Cmd / Ctrl + Shift + L"),
                ("发送问题", "Cmd / Ctrl + Enter"),
                ("导入 / 悬浮窗", "Cmd / Ctrl + Shift + I"),
                ("生成视频字幕", "Cmd / Ctrl + Shift + T"),
                ("视频全屏 / 退出", "F11"),
                ("视频上一句 / 下一句", "← / →"),
                ("播放 / 暂停", "Space"),
                ("闪卡评分：忘了 / 模糊 / 认识", "1 / 2 / 3"),
                ("退出电脑专注", "Ctrl + Shift + Esc"),
                ("打开本页", "F1"),
            ];
            let mut list = div().flex().flex_col().gap_2().text_sm();
            for (action, keys) in rows {
                list = list.child(
                    div()
                        .flex()
                        .justify_between()
                        .gap_4()
                        .child(action)
                        .child(div().text_color(cx.theme().muted_foreground).child(keys)),
                );
            }
            dialog
                .title("键盘快捷键")
                .confirm()
                .w((window.viewport_size().width - px(32.)).min(px(520.)))
                .button_props(DialogButtonProps::default().ok_text("知道了"))
                .child(list)
        });
    }
}
