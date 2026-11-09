//! Explicit macOS kiosk session. Native policy is process-local and RAII-restored.
use super::*;

#[cfg(target_os = "macos")]
#[path = "focus_recovery_macos.rs"]
pub(super) mod recovery;

pub(super) const EXIT_SHORTCUT: &str = "Ctrl + Shift + Esc";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Interruption {
    #[cfg(target_os = "macos")]
    Shortcut,
    Deactivated,
}

#[derive(Default)]
pub(super) struct Mode {
    native: Option<native::Guard>,
    unlock_at_ms: u64,
    maximum: Option<Instant>,
    owned_fullscreen: bool,
    restore_fullscreen_at: Option<Instant>,
    entered: Option<Instant>,
    pub escape: Option<Instant>,
    covers: Vec<AnyWindowHandle>,
    displays: String,
    checked: Option<Instant>,
}
impl Mode {
    pub fn active(&self) -> bool {
        self.native.is_some()
    }
    #[cfg(debug_assertions)]
    pub fn expire_safety_limit(&mut self) {
        self.unlock_at_ms = 0;
    }
    pub fn escape_remaining(&self) -> Option<u64> {
        self.escape.map(|at| {
            Duration::from_secs(5)
                .saturating_sub(at.elapsed())
                .as_secs()
                .saturating_add(u64::from(at.elapsed() < Duration::from_secs(5)))
        })
    }
}
impl AppView {
    fn focus_probe_active(&self) -> bool {
        #[cfg(debug_assertions)]
        {
            std::env::args().any(|a| a == "--focus-mode-check")
        }
        #[cfg(not(debug_assertions))]
        {
            false
        }
    }
    pub(super) fn start_focus_mode(
        &mut self,
        seconds: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        ensure!(!self.focus_mode.active(), "电脑专注已开启");
        ensure!(
            cfg!(target_os = "macos"),
            "电脑专注目前支持 macOS；Windows 暂不支持系统级限制"
        );
        let guard = native::Guard::enter()?;
        let now = focus_timer::now_ms();
        let starting = !self.timer.active_round();
        if let Some(seconds) = seconds {
            self.timer.start_strict(&self.lib, now, seconds)?;
        } else {
            self.timer.resume_strict(&self.lib, now)?;
        }
        // The learning round is open-ended, but system restrictions remain bounded.
        self.focus_mode.unlock_at_ms = now.saturating_add(3_600_000);
        self.focus_mode.maximum = Some(Instant::now() + Duration::from_secs(3600));
        self.focus_mode.native = Some(guard);
        self.focus_mode.entered = Some(Instant::now());
        self.focus_mode.escape = None;
        self.close_floating_names(cx);
        if let Some(handle) = self.floating_window.take() {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
        if !self.focus_probe_active() {
            let fullscreen = window.is_fullscreen();
            self.focus_mode.owned_fullscreen =
                self.focus_mode.restore_fullscreen_at.take().is_some() || !fullscreen;
            if !fullscreen {
                window.toggle_fullscreen();
            }
            if let Err(error) = self.update_focus_covers(window, cx) {
                self.stop_focus_mode(window, cx);
                return Err(error);
            }
        }
        if self.page == "video" {
            // Starting/resuming focus is an explicit return to video learning.
            // Window activation/fullscreen do not change GPUI's input focus.
            window.focus(&self.focus);
        }
        self.timer_seconds = self.timer.display_seconds(now);
        if starting {
            self.play_timer_chime(timer_chimes::Cue::Start);
        }
        self.report_status(
            FeedbackKind::Info,
            format!("电脑专注已开启，按 {EXIT_SHORTCUT} 立即退出"),
        );
        cx.notify();
        Ok(())
    }
    pub(super) fn stop_focus_mode(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Release native restrictions first, even when persistence is unavailable.
        self.focus_mode.native.take();
        for handle in self.focus_mode.covers.drain(..) {
            let _ = handle.update(cx, |_, w, _| w.remove_window());
        }
        if self.focus_mode.owned_fullscreen {
            // Wait for an in-flight fullscreen transition and the user's return
            // to Clarora. Exiting its Space in the background can steal focus.
            self.focus_mode.restore_fullscreen_at = self
                .focus_mode
                .entered
                .map(|entered| entered + Duration::from_secs(1));
        }
        self.focus_mode = Mode {
            restore_fullscreen_at: self.focus_mode.restore_fullscreen_at,
            ..Mode::default()
        };
        self.restore_focus_fullscreen(window);
        cx.notify();
    }
    fn restore_focus_fullscreen(&mut self, window: &mut Window) {
        if self
            .focus_mode
            .restore_fullscreen_at
            .is_some_and(|at| Instant::now() >= at)
            && native::is_active()
        {
            self.focus_mode.restore_fullscreen_at = None;
            if window.is_fullscreen() {
                window.toggle_fullscreen();
            }
        }
    }
    fn interrupt_focus(
        &mut self,
        message: &str,
        distracted: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.stop_focus_mode(window, cx);
        let now = focus_timer::now_ms();
        if self.timer.is_running() {
            self.timer
                .interrupt(&self.lib, now, distracted)
                .context("电脑专注已解除，但暂停计时失败")?;
        }
        self.timer_seconds = self.timer.display_seconds(now);
        self.report_status(FeedbackKind::Info, message.into());
        Ok(())
    }
    pub(super) fn emergency_focus_end(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        if !self.focus_mode.active() {
            return Ok(());
        }
        if let Some(at) = self.focus_mode.escape {
            ensure!(
                at.elapsed() >= Duration::from_secs(5),
                "请等待应急结束倒计时"
            );
            self.interrupt_focus("电脑专注已提前结束，计时已暂停", false, window, cx)?;
        } else {
            self.focus_mode.escape = Some(Instant::now());
            cx.notify();
        }
        Ok(())
    }
    pub(super) fn poll_focus_mode(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(guard) = &self.focus_mode.native else {
            self.restore_focus_fullscreen(window);
            return false;
        };
        if focus_timer::now_ms() >= self.focus_mode.unlock_at_ms
            || self
                .focus_mode
                .maximum
                .is_some_and(|at| Instant::now() >= at)
        {
            self.stop_focus_mode(window, cx);
            match self.timer.release_strict(&self.lib) {
                Ok(()) => self.report_status(
                    FeedbackKind::Info,
                    "电脑专注已解锁，本轮专注继续计时".into(),
                ),
                Err(error) => self.report_status(
                    FeedbackKind::Error,
                    format!("电脑专注已解锁，计时状态保存失败：{error:#}"),
                ),
            }
            return true;
        }
        let diagnostic = self.focus_probe_active();
        let interruption = guard
            .interruption()
            .or_else(|| (!diagnostic && !native::is_active()).then_some(Interruption::Deactivated));
        if let Some(interruption) = interruption {
            let message = match interruption {
                #[cfg(target_os = "macos")]
                Interruption::Shortcut => "已通过快捷键退出电脑专注，计时已暂停",
                Interruption::Deactivated => "Clarora 已离开前台，电脑专注已解除，计时已暂停",
            };
            if let Err(error) = self.interrupt_focus(
                message,
                interruption == Interruption::Deactivated,
                window,
                cx,
            ) {
                self.report_status(FeedbackKind::Error, format!("{error:#}"));
            }
            return true;
        }
        if !diagnostic {
            if let Err(error) = guard.refresh() {
                self.stop_focus_mode(window, cx);
                self.report_status(
                    FeedbackKind::Error,
                    format!("专注限制未保持，已解除：{error:#}"),
                );
                return true;
            }
            if !window.is_fullscreen()
                && self
                    .focus_mode
                    .entered
                    .is_some_and(|at| at.elapsed() > Duration::from_secs(3))
            {
                window.toggle_fullscreen();
                self.focus_mode.entered = Some(Instant::now());
            }
            if self
                .focus_mode
                .checked
                .is_none_or(|at| at.elapsed() > Duration::from_secs(1))
            {
                self.focus_mode.checked = Some(Instant::now());
                if let Err(error) = self.update_focus_covers(window, cx) {
                    self.stop_focus_mode(window, cx);
                    self.report_status(
                        FeedbackKind::Error,
                        format!("屏幕专注遮罩未建立，已解除：{error:#}"),
                    );
                    return true;
                }
            }
        }
        self.focus_mode.escape.is_some()
    }
    fn update_focus_covers(&mut self, window: &Window, cx: &mut Context<Self>) -> Result<()> {
        let main = window.display(cx).context("当前屏幕不可用")?.id();
        let displays = cx.displays();
        let signature = format!(
            "{main:?}:{:?}",
            displays
                .iter()
                .map(|d| (d.id(), d.bounds()))
                .collect::<Vec<_>>()
        );
        if signature == self.focus_mode.displays {
            return Ok(());
        }
        for handle in self.focus_mode.covers.drain(..) {
            let _ = handle.update(cx, |_, w, _| w.remove_window());
        }
        for display in displays.iter().filter(|d| d.id() != main) {
            let main_window = window.window_handle();
            let cover = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(display.bounds())),
                    display_id: Some(display.id()),
                    titlebar: None,
                    kind: WindowKind::PopUp,
                    is_movable: false,
                    is_resizable: false,
                    is_minimizable: false,
                    focus: false,
                    ..Default::default()
                },
                move |window, cx| {
                    window.on_window_should_close(cx, |_, _| false);
                    native::shield(window);
                    cx.new(|_| Cover { main: main_window })
                },
            )?;
            self.focus_mode.covers.push(cover.into());
        }
        self.focus_mode.displays = signature;
        Ok(())
    }
    pub(super) fn focus_mode_button(&self, cx: &Context<Self>) -> AnyElement {
        if self.focus_mode.active() {
            let mut row = div().flex().items_center().flex_wrap().gap_2();
            if let Some(remaining) = self.focus_mode.escape_remaining() {
                row = row
                    .child(
                        self.button(
                            "focus-emergency-confirm",
                            if remaining > 0 {
                                format!("{remaining} 秒后可提前结束")
                            } else {
                                "确认提前结束".into()
                            },
                            Action::FocusEmergency,
                            cx,
                        )
                        .danger()
                        .disabled(remaining > 0),
                    )
                    .child(
                        self.button("focus-emergency-cancel", "继续专注", Action::FocusKeep, cx)
                            .outline(),
                    );
            } else {
                row = row.child(
                    self.button("focus-emergency", "紧急结束", Action::FocusEmergency, cx)
                        .outline(),
                );
            }
            return row.into_any_element();
        }
        let view = cx.entity().downgrade();
        let fallback_focus = self.focus.clone();
        let active_round = self.timer.active_round();
        let goal = self.timer.goal_seconds();
        let in_rest = !self.timer.focus();
        let primary_label = self.focus_control_label().to_string();
        let primary_action = self.focus_control_primary_action();
        let label = div()
            .flex_none()
            .line_height(relative(1.))
            .font_weight(FontWeight::BOLD)
            .text_color(cx.theme().primary_foreground)
            .child(format!("{primary_label} ▾"));
        #[cfg(debug_assertions)]
        let label = label.children(if self.timer.awaiting_focus() {
            focus_mode_probe::button_color_probe(cx.theme().primary, "timer-confirmation")
        } else {
            focus_mode_probe::label_color_probe(cx.theme().primary)
        });
        Button::new("global-focus-control")
            .child(label)
            .h(px(40.))
            .px_4()
            .custom(
                ButtonCustomVariant::new(cx)
                    .color(cx.theme().primary)
                    .foreground(cx.theme().primary_foreground)
                    .border(cx.theme().primary)
                    .hover(cx.theme().primary_hover)
                    .active(cx.theme().primary_active)
                    .shadow(true),
            )
            .tooltip(if cfg!(target_os = "macos") {
                format!("管理专注计时和电脑专注。电脑专注开启后可按 {EXIT_SHORTCUT} 立即退出。")
            } else {
                "管理专注计时和时长。".into()
            })
            .dropdown_menu_with_anchor(Corner::BottomRight, move |mut menu, window, cx| {
                // The builder runs before the menu takes focus. Cancel restores
                // the original editor; a successful start restores video focus.
                menu = menu
                    .action_context(window.focused(cx).unwrap_or_else(|| fallback_focus.clone()));
                menu = menu.item(Self::focus_menu_item(
                    view.clone(),
                    primary_label.clone(),
                    primary_action.clone(),
                ));
                if active_round {
                    if cfg!(target_os = "macos") {
                        menu = menu.item(Self::focus_menu_item(
                            view.clone(),
                            "开启电脑专注",
                            Action::FocusResume,
                        ));
                    }
                    return menu;
                }
                let duration_view = view.clone();
                menu = menu.separator().submenu(
                    "最低专注目标",
                    window,
                    cx,
                    move |mut sub, _, _| {
                        for minutes in focus_timer::GOAL_PRESETS_MINUTES {
                            let label = if minutes * 60 == goal {
                                format!("✓ {minutes} 分钟")
                            } else {
                                format!("{minutes} 分钟")
                            };
                            sub = sub.item(Self::focus_menu_item(
                                duration_view.clone(),
                                label,
                                Action::Timer(minutes * 60),
                            ));
                        }
                        sub
                    },
                );
                if cfg!(target_os = "macos") && !in_rest {
                    menu = menu.item(Self::focus_menu_item(
                        view.clone(),
                        format!("开启电脑专注 · 最低 {} 分钟", goal / 60),
                        Action::FocusStart(goal),
                    ));
                }
                if in_rest {
                    menu = menu.item(Self::focus_menu_item(
                        view.clone(),
                        "结束休息",
                        Action::TimerReset,
                    ));
                }
                menu
            })
            .into_any_element()
    }

    fn focus_menu_item(
        view: WeakEntity<Self>,
        label: impl Into<SharedString>,
        action: Action,
    ) -> PopupMenuItem {
        PopupMenuItem::new(label).on_click(move |_, window, cx| {
            let view = view.clone();
            let action = action.clone();
            let handle = window.window_handle();
            // Restore the menu context first, then apply the selected action.
            cx.defer(move |cx| {
                let _ = handle.update(cx, move |_, window, cx| {
                    let _ = view.update(cx, |this, cx| this.handle(action, window, cx));
                });
            });
        })
    }
}
struct Cover {
    main: AnyWindowHandle,
}
impl Render for Cover {
    fn render(&mut self, _: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let main = self.main;
        div()
            .size_full()
            .bg(rgb(0x172d26))
            .text_color(rgb(0xf0f5f1))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_4()
            .child(div().text_2xl().child("正在电脑专注"))
            .child(format!(
                "返回学习窗口继续学习，或按 {EXIT_SHORTCUT} 退出专注"
            ))
            .child(
                Button::new("return-focus-main")
                    .label("返回学习窗口")
                    .on_click(move |_, _, cx| {
                        let _ = main.update(cx, |_, w, cx| {
                            cx.activate(true);
                            w.activate_window();
                        });
                    }),
            )
    }
}

pub(super) mod native {
    use super::Interruption;
    use anyhow::{Result, ensure};
    pub const REQUIRED: usize = (1 << 1) | (1 << 3) | (1 << 4) | (1 << 5) | (1 << 8);
    pub struct Guard {
        previous: usize,
        #[cfg(target_os = "macos")]
        recovery: super::recovery::Recovery,
    }
    #[cfg(target_os = "macos")]
    pub(super) fn application() -> *mut objc2::runtime::AnyObject {
        use objc2::{msg_send, runtime::AnyClass};
        unsafe {
            AnyClass::get(c"NSApplication")
                .map(|c| msg_send![c, sharedApplication])
                .unwrap_or(std::ptr::null_mut())
        }
    }
    #[cfg(target_os = "macos")]
    pub fn options() -> usize {
        use objc2::msg_send;
        unsafe {
            let app = application();
            if app.is_null() {
                0
            } else {
                msg_send![app, presentationOptions]
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    pub fn options() -> usize {
        0
    }
    #[cfg(target_os = "macos")]
    pub(super) fn set(value: usize) {
        use objc2::msg_send;
        unsafe {
            let app = application();
            if !app.is_null() {
                let _: () = msg_send![app,setPresentationOptions:value];
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    fn set(_: usize) {}
    #[cfg(target_os = "macos")]
    pub fn is_active() -> bool {
        use objc2::msg_send;
        unsafe {
            let app = application();
            !app.is_null() && msg_send![app, isActive]
        }
    }
    #[cfg(not(target_os = "macos"))]
    pub fn is_active() -> bool {
        false
    }
    impl Guard {
        pub fn enter() -> Result<Self> {
            #[cfg(not(target_os = "macos"))]
            anyhow::bail!("系统专注目前仅支持 macOS");
            #[cfg(target_os = "macos")]
            {
                let diagnostic =
                    cfg!(debug_assertions) && std::env::args().any(|a| a == "--focus-mode-check");
                if diagnostic {
                    ensure!(
                        std::env::var("CLARORA_FOCUS_DIAGNOSTIC_INACTIVE").as_deref() == Ok("1")
                            && !is_active(),
                        "专注诊断只允许在显式标记的非激活隐藏进程内运行"
                    );
                } else {
                    ensure!(is_active(), "请先回到 Clarora，再开启电脑专注");
                }
                let previous = options();
                let guard = Self {
                    previous,
                    recovery: super::recovery::Recovery::install(previous)?,
                };
                guard.refresh()?;
                Ok(guard)
            }
        }
        pub fn interruption(&self) -> Option<Interruption> {
            #[cfg(target_os = "macos")]
            return self.recovery.interruption();
            #[cfg(not(target_os = "macos"))]
            None
        }
        pub fn refresh(&self) -> Result<()> {
            ensure!(self.interruption().is_none(), "电脑专注已解除");
            // Hide/auto-hide pairs are mutually exclusive. Preserve the OS full-screen
            // flags while replacing its default dock/menu policy with kiosk options.
            let desired = (options() & !((1 << 0) | (1 << 2))) | REQUIRED;
            if options() != desired {
                set(desired);
            }
            ensure!(options() & REQUIRED == REQUIRED, "系统未应用专注限制");
            Ok(())
        }
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            if self.interruption().is_none() {
                set(self.previous);
            }
        }
    }
    pub fn shield(window: &gpui::Window) {
        #[cfg(target_os = "macos")]
        {
            use objc2::{msg_send, runtime::AnyObject};
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(handle) = HasWindowHandle::window_handle(window) {
                if let RawWindowHandle::AppKit(h) = handle.as_raw() {
                    unsafe {
                        let view = h.ns_view.as_ptr() as *mut AnyObject;
                        let panel: *mut AnyObject = msg_send![view, window];
                        let _: () = msg_send![panel,setLevel:1000isize];
                        let _: () = msg_send![panel,setCollectionBehavior:(1usize|(1<<8))];
                        let _: () = msg_send![panel,setHidesOnDeactivate:true];
                    }
                }
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = window;
    }
}
