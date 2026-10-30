//! Explicit macOS kiosk session. Native policy is process-local and RAII-restored.
use super::*;

#[derive(Default)]
pub(super) struct Mode {
    native: Option<native::Guard>,
    pub deadline: u64,
    maximum: Option<Instant>,
    owned_fullscreen: bool,
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
        if let Some(seconds) = seconds {
            self.timer.start_strict(&self.lib, now, seconds)?;
        } else {
            ensure!(
                self.timer.strict() && self.timer.deadline_ms().is_some_and(|end| end > now),
                "上次电脑专注已结束，请重新选择时长"
            );
        }
        self.focus_mode.deadline = self.timer.deadline_ms().context("专注计时未开始")?;
        self.focus_mode.maximum =
            Some(Instant::now() + Duration::from_secs(self.timer.remaining_seconds(now).min(3600)));
        self.focus_mode.native = Some(guard);
        self.focus_mode.entered = Some(Instant::now());
        self.focus_mode.escape = None;
        self.close_floating_names(cx);
        if let Some(handle) = self.floating_window.take() {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
        if !self.focus_probe_active() {
            self.focus_mode.owned_fullscreen = !window.is_fullscreen();
            if self.focus_mode.owned_fullscreen {
                window.toggle_fullscreen();
            }
            window.activate_window();
            cx.activate(true);
            if let Err(error) = self.update_focus_covers(window, cx) {
                self.stop_focus_mode(window, cx);
                return Err(error);
            }
        }
        self.timer_remaining = self.timer.remaining_seconds(now);
        self.play_timer_chime(timer_chimes::Cue::Start);
        self.report_status(
            FeedbackKind::Info,
            "电脑专注已开启，计时结束后自动恢复应用切换".into(),
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
        if self.focus_mode.owned_fullscreen && window.is_fullscreen() {
            window.toggle_fullscreen();
        }
        self.focus_mode = Mode::default();
        cx.notify();
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
            self.stop_focus_mode(window, cx);
            if self.timer.is_running() {
                let _ = self.timer.toggle(&self.lib, focus_timer::now_ms())?;
            }
            self.timer_remaining = self.timer.remaining_seconds(focus_timer::now_ms());
            self.report_status(
                FeedbackKind::Info,
                "电脑专注已提前结束，应用切换已恢复".into(),
            );
        } else {
            self.focus_mode.escape = Some(Instant::now());
            cx.notify();
        }
        Ok(())
    }
    pub(super) fn poll_focus_mode(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.focus_mode.active() {
            return false;
        }
        if focus_timer::now_ms() >= self.focus_mode.deadline
            || self
                .focus_mode
                .maximum
                .is_some_and(|at| Instant::now() >= at)
        {
            self.stop_focus_mode(window, cx);
            return true;
        }
        if !self.focus_probe_active() {
            if let Err(error) = self.focus_mode.native.as_ref().unwrap().refresh() {
                self.stop_focus_mode(window, cx);
                self.report_status(
                    FeedbackKind::Error,
                    format!("专注限制未保持，已解除：{error:#}"),
                );
                return true;
            }
            if !native::is_active() {
                cx.activate(true);
                window.activate_window();
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
            let mut row = div().flex().items_center().gap_2().child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("电脑专注中"),
            );
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
        let resumable = self.timer.strict() && self.timer.is_running();
        Button::new("focus-computer").label(if resumable{"继续电脑专注"}else{"电脑专注"}).outline().small()
            .disabled(!cfg!(target_os="macos"))
            .tooltip("选择时长后全屏专注，限制切换其他应用；App 内学习操作正常。计时结束自动解除。系统强制退出保留用于异常恢复。")
            .dropdown_menu_with_anchor(Corner::BottomRight,move |mut menu,_,_|{
                if resumable { let weak=view.clone();menu=menu.item(PopupMenuItem::new("继续上次专注（沿用剩余时间）").on_click(move|_,w,cx|{let _=weak.update(cx,|v,cx|v.handle(Action::FocusResume,w,cx));})); }
                for minutes in [5,15,25,45,60] { let weak=view.clone();menu=menu.item(PopupMenuItem::new(format!("专注 {minutes} 分钟 · 屏蔽其他应用")).on_click(move|_,w,cx|{let _=weak.update(cx,|v,cx|v.handle(Action::FocusStart(minutes*60),w,cx));})); } menu
            }).into_any_element()
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
            .child("在 Clarora 主窗口继续学习，计时结束后恢复。")
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
    use anyhow::{Result, ensure};
    pub const REQUIRED: usize = (1 << 1) | (1 << 3) | (1 << 4) | (1 << 5) | (1 << 8);
    pub struct Guard {
        previous: usize,
        _main_thread: std::marker::PhantomData<std::rc::Rc<()>>,
    }
    #[cfg(target_os = "macos")]
    fn application() -> *mut objc2::runtime::AnyObject {
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
    fn set(value: usize) {
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
            #[cfg(debug_assertions)]
            if std::env::args().any(|a| a == "--focus-mode-check") {
                ensure!(
                    std::env::var("CLARORA_FOCUS_DIAGNOSTIC_INACTIVE").as_deref() == Ok("1")
                        && !is_active(),
                    "专注诊断只允许在显式标记的非激活隐藏进程内运行"
                );
            }
            ensure!(cfg!(target_os = "macos"), "系统专注目前仅支持 macOS");
            let guard = Self {
                previous: options(),
                _main_thread: Default::default(),
            };
            guard.refresh()?;
            Ok(guard)
        }
        pub fn refresh(&self) -> Result<()> {
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
            set(self.previous);
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
                        let _: () = msg_send![panel,setHidesOnDeactivate:false];
                    }
                }
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = window;
    }
}
