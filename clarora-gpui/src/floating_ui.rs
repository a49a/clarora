//! A separate nonactivating quick-import window observing the main app's state.
use super::*;

const COMPACT_WIDTH: f32 = 232.;
const COMPACT_HEIGHT: f32 = 60.;
const EXPANDED_HEIGHT: f32 = 148.;
const TARGETS_HEIGHT: f32 = 228.;
const SPEAKING_TARGET_SETTING: &str = "floating_speaking_group";

#[derive(Clone, PartialEq, Eq)]
enum ImportTarget {
    Audio,
    Speaking(String),
}

impl ImportTarget {
    fn load(lib: &Library) -> Self {
        match lib.setting(SPEAKING_TARGET_SETTING) {
            Ok(id) if !id.is_empty() => Self::Speaking(id),
            _ => Self::Audio,
        }
    }

    fn page(&self) -> &'static str {
        match self {
            Self::Audio => "library",
            Self::Speaking(_) => "speaking",
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Audio => "音频",
            Self::Speaking(_) => "口语",
        }
    }

    fn action(&self) -> Action {
        match self {
            Self::Audio => Action::CommandImport,
            Self::Speaking(id) => Action::CommandImportSpeaking(id.clone()),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FloatingPanel {
    Compact,
    Details,
    Targets,
}

struct FloatingImport {
    owner: WeakEntity<AppView>,
    main: AnyWindowHandle,
    snapshot: ImportSnapshot,
    panel: FloatingPanel,
    target: ImportTarget,
    focus: FocusHandle,
    _changes: Subscription,
}
impl FloatingImport {
    fn new(
        owner: Entity<AppView>,
        main: AnyWindowHandle,
        snapshot: ImportSnapshot,
        target: ImportTarget,
        cx: &mut Context<Self>,
    ) -> Self {
        let changes = cx.observe(&owner, |this, owner, cx| {
            this.snapshot = ImportSnapshot::from_app(owner.read(cx), &this.target);
            cx.notify();
        });
        Self {
            owner: owner.downgrade(),
            main,
            snapshot,
            panel: FloatingPanel::Compact,
            target,
            focus: cx.focus_handle(),
            _changes: changes,
        }
    }
    fn toggle_details(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let panel = if self.panel == FloatingPanel::Details {
            FloatingPanel::Compact
        } else {
            FloatingPanel::Details
        };
        self.set_panel(panel, window, cx);
    }
    fn set_panel(&mut self, panel: FloatingPanel, window: &mut Window, cx: &mut Context<Self>) {
        self.panel = panel;
        let height = match panel {
            FloatingPanel::Compact => COMPACT_HEIGHT,
            FloatingPanel::Details => EXPANDED_HEIGHT,
            FloatingPanel::Targets => TARGETS_HEIGHT,
        };
        window.resize(size(px(COMPACT_WIDTH), px(height)));
        cx.notify();
    }
    fn toggle_targets(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.panel == FloatingPanel::Targets {
            self.set_panel(FloatingPanel::Compact, window, cx);
            return;
        }
        self.set_panel(FloatingPanel::Targets, window, cx);
        let owner = self.owner.clone();
        cx.defer(move |cx| {
            let _ = owner.update(cx, |app, cx| {
                if let Err(error) = app.refresh_speaking_groups() {
                    app.report_status(FeedbackKind::Error, format!("读取口语分组失败：{error:#}"));
                }
                cx.notify();
            });
        });
    }
    fn select_target(&mut self, target: ImportTarget, window: &mut Window, cx: &mut Context<Self>) {
        self.target = target;
        self.set_panel(FloatingPanel::Compact, window, cx);
        if let Some(owner) = self.owner.upgrade() {
            let app = owner.read(cx);
            let id = match &self.target {
                ImportTarget::Audio => "",
                ImportTarget::Speaking(id) => id,
            };
            let saved = app.lib.set_setting(SPEAKING_TARGET_SETTING, id);
            self.snapshot = ImportSnapshot::from_app(app, &self.target);
            if let Err(error) = saved {
                cx.defer(move |cx| {
                    owner.update(cx, |app, cx| {
                        app.report_status(
                            FeedbackKind::Error,
                            format!("保存悬浮窗导入目标失败：{error:#}"),
                        );
                        cx.notify();
                    })
                });
            }
        }
    }
    fn import(&self, cx: &mut Context<Self>) {
        if self.snapshot.busy {
            self.dispatch(Action::Cancel, false, cx);
        } else if !self.snapshot.command_ready {
            self.dispatch(Action::ConfigureCommandImport, true, cx);
        } else if self.snapshot.group.is_none() {
            self.dispatch(Action::Page(self.target.page().into()), true, cx);
        } else {
            self.dispatch(self.target.action(), false, cx);
        }
    }
    fn target_button(
        &self,
        id: SharedString,
        label: String,
        target: ImportTarget,
        cx: &Context<Self>,
    ) -> Button {
        Button::new(id)
            .small()
            .outline()
            .w_full()
            .justify_start()
            .flex_shrink_0()
            .selected(self.target == target)
            .child(div().min_w_0().w_full().truncate().child(label.clone()))
            .tooltip(label)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.select_target(target.clone(), window, cx)
            }))
    }
    fn hide(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.dispatch(Action::CloseFloatingNames, false, cx);
        if let Some(owner) = self.owner.upgrade() {
            let _ = owner
                .read(cx)
                .lib
                .set_setting("floating_import_open", "false");
        }
        window.remove_window();
    }
    fn dispatch(&self, action: Action, activate: bool, cx: &mut Context<Self>) {
        let owner = self.owner.clone();
        let main = self.main;
        // Return the floating view's lease before the owner's notification updates it.
        cx.defer(move |cx| {
            let _ = main.update(cx, |_, window, cx| {
                if activate {
                    window.activate_window();
                    cx.activate(true);
                }
                let _ = owner.update(cx, |app, cx| app.handle(action, window, cx));
            });
        });
    }
}
#[derive(Clone)]
struct ImportSnapshot {
    group: Option<String>,
    audio_group: Option<String>,
    speaking_groups: Vec<clarora_gpui::speaking::Group>,
    command_ready: bool,
    busy: bool,
    cancelling: bool,
    detail: String,
    failed: bool,
    caption: String,
    can_name: bool,
}
impl ImportSnapshot {
    fn from_app(app: &AppView, target: &ImportTarget) -> Self {
        let audio_group = app
            .audio
            .practices
            .iter()
            .find(|r| text(r, "id") == app.audio.practice_id)
            .map(|r| text(r, "name"));
        let speaking_groups = app.speaking_groups().to_vec();
        let group = match target {
            ImportTarget::Audio => audio_group.clone(),
            ImportTarget::Speaking(id) => speaking_groups
                .iter()
                .find(|group| &group.id == id)
                .map(|group| group.name.clone()),
        };
        let command_ready = !app.import_command.trim().is_empty();
        let busy = app.tasks.foreground.is_running();
        let cancelling = app
            .tasks
            .foreground
            .cancel_token()
            .is_some_and(|t| t.load(Ordering::SeqCst));
        let notice = app.tasks.notice.as_ref().filter(|n| {
            n.title
                == match target {
                    ImportTarget::Audio => "命令导入音频",
                    ImportTarget::Speaking(_) => "口语命令导入",
                }
        });
        let detail = if busy {
            if cancelling {
                "正在取消…".into()
            } else {
                app.tasks.progress_label.clone()
            }
        } else if let Some(n) = notice {
            n.detail.clone()
        } else if group.is_none() {
            "先在主窗口新建或选择练习组".into()
        } else if !command_ready {
            "先设置本机导入命令，每行输出一个音频路径".into()
        } else {
            "随时收集音频，导入当前练习组".into()
        };
        let failed = notice.is_some_and(|n| n.state == TaskState::Failed);
        let caption = if cancelling {
            "正在取消"
        } else if busy {
            "任务进行中"
        } else if failed {
            "导入失败 · 查看详情"
        } else if notice.is_some_and(|n| n.state == TaskState::Succeeded) {
            "导入完成"
        } else if notice.is_some_and(|n| n.state == TaskState::Cancelled) {
            "已取消"
        } else if group.is_none() {
            "先选择练习组"
        } else if !command_ready {
            "先设置导入命令"
        } else {
            "当前组"
        }
        .to_string();
        Self {
            group,
            audio_group,
            speaking_groups,
            command_ready,
            busy,
            cancelling,
            detail,
            failed,
            caption,
            can_name: matches!(target, ImportTarget::Audio) && app.command_imported.is_some(),
        }
    }
}

#[cfg(debug_assertions)]
pub(super) fn speaking_probe_action(app: &AppView, group: &str) -> Result<Action> {
    app.lib.set_setting(SPEAKING_TARGET_SETTING, group)?;
    let target = ImportTarget::load(&app.lib);
    ensure!(target == ImportTarget::Speaking(group.to_owned()));
    let snapshot = ImportSnapshot::from_app(app, &target);
    ensure!(snapshot.group.as_deref() == Some("悬浮窗目标") && snapshot.command_ready);
    ensure!(!snapshot.can_name);
    Ok(target.action())
}
impl Render for FloatingImport {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // This window has no inputs/dialogs: render directly on a transparent surface,
        // instead of Root's full rectangular background. Never read the owner here.
        window.set_rem_size(cx.theme().font_size);
        let ImportSnapshot {
            group,
            audio_group,
            speaking_groups,
            command_ready,
            busy,
            cancelling,
            detail,
            failed,
            caption,
            can_name,
        } = self.snapshot.clone();
        let has_group = group.is_some();
        let group_name = group.unwrap_or_else(|| "未选分组".into());
        let group_tip = format!(
            "{} · {} · 点击选择导入目标",
            self.target.label(),
            group_name
        );
        let (icon, tip) = if busy {
            (
                IconName::Close,
                if cancelling {
                    "正在取消"
                } else {
                    "取消当前任务"
                },
            )
        } else if !command_ready {
            (IconName::Settings2, "设置导入命令")
        } else if !has_group {
            (IconName::FolderClosed, "选择练习组")
        } else {
            (
                IconName::ArrowDown,
                if matches!(self.target, ImportTarget::Audio) {
                    "命令导入音频"
                } else {
                    "命令导入口语题目"
                },
            )
        };
        let bar = div()
            .h(px(50.))
            .flex_shrink_0()
            .px_2()
            .flex()
            .items_center()
            .gap_1()
            .child(
                div()
                    .id("floating-drag")
                    .size(px(26.))
                    .flex_shrink_0()
                    .rounded_full()
                    .bg(cx.theme().accent)
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor(CursorStyle::OpenHand)
                    .window_control_area(WindowControlArea::Drag)
                    .tooltip(|window, cx| Tooltip::new("拖动悬浮窗").build(window, cx))
                    .on_mouse_down(MouseButton::Left, |_, window, _| native::drag(window))
                    .child(design::icon("meditation", cx.theme().primary).size(px(17.))),
            )
            .child(
                div()
                    .id("floating-current-group")
                    .flex_1()
                    .min_w_0()
                    .px_1()
                    .flex()
                    .flex_col()
                    .cursor_pointer()
                    .tooltip(move |window, cx| Tooltip::new(group_tip.clone()).build(window, cx))
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(if failed {
                                cx.theme().danger
                            } else {
                                cx.theme().muted_foreground
                            })
                            .truncate()
                            .child(format!("{} · {caption}", self.target.label())),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(format!("{group_name} ▾")),
                    )
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_targets(window, cx))),
            )
            .child(
                Button::new("floating-command-import")
                    .icon(icon)
                    .primary()
                    .small()
                    .w(px(30.))
                    .h(px(30.))
                    .rounded_full()
                    .tooltip(tip)
                    .disabled(cancelling)
                    .on_click(cx.listener(|this, _, _, cx| this.import(cx))),
            )
            .child(
                Button::new("floating-more")
                    .icon(if self.panel == FloatingPanel::Details {
                        IconName::ChevronUp
                    } else {
                        IconName::Ellipsis
                    })
                    .ghost()
                    .small()
                    .w(px(22.))
                    .h(px(28.))
                    .tooltip(if self.panel == FloatingPanel::Details {
                        "收起详情"
                    } else {
                        "详情与更多操作"
                    })
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_details(window, cx))),
            );
        let details = (self.panel == FloatingPanel::Details).then(|| {
            div()
                .border_t_1()
                .border_color(cx.theme().border)
                .px_2()
                .pt_1()
                .pb_2()
                .min_h_0()
                .flex_1()
                .flex()
                .flex_col()
                .gap_1()
                .when(can_name, |this| {
                    this.child(
                        Button::new("floating-name-imported")
                            .label("命名刚导入的音频")
                            .small()
                            .h(px(24.))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.dispatch(Action::NameImportedAudio, false, cx)
                            })),
                    )
                })
                .child(
                    div()
                        .id("floating-detail")
                        .flex_1()
                        .min_h_0()
                        .text_xs()
                        .line_clamp(2)
                        .text_color(if failed {
                            cx.theme().danger
                        } else {
                            cx.theme().muted_foreground
                        })
                        .tooltip(move |window, cx| Tooltip::new(detail.clone()).build(window, cx))
                        .child(self.snapshot.detail.clone()),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_1()
                        .child(
                            Button::new("floating-open-main")
                                .label("主窗口")
                                .ghost()
                                .small()
                                .h(px(24.))
                                .px_2()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.dispatch(Action::Page(this.target.page().into()), true, cx)
                                })),
                        )
                        .child(
                            Button::new("floating-settings")
                                .label("设置")
                                .ghost()
                                .small()
                                .h(px(24.))
                                .px_2()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.dispatch(Action::ConfigureCommandImport, true, cx)
                                })),
                        )
                        .child(
                            Button::new("floating-close")
                                .label("隐藏")
                                .ghost()
                                .small()
                                .h(px(24.))
                                .px_2()
                                .on_click(cx.listener(|this, _, window, cx| this.hide(window, cx))),
                        ),
                )
        });
        let targets = (self.panel == FloatingPanel::Targets).then(|| {
            let mut list = div()
                .id("floating-targets")
                .min_h_0()
                .flex_1()
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap_1()
                .px_2()
                .pb_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("导入目标"),
                );
            list = list.child(self.target_button(
                "floating-target-audio".into(),
                format!("音频 · {}", audio_group.as_deref().unwrap_or("当前练习组")),
                ImportTarget::Audio,
                cx,
            ));
            if speaking_groups.is_empty() {
                list = list.child(
                    Button::new("floating-open-speaking")
                        .small()
                        .outline()
                        .label("去口语练习新建组")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.dispatch(Action::Page("speaking".into()), true, cx)
                        })),
                );
            }
            for group in speaking_groups {
                list = list.child(self.target_button(
                    format!("floating-target-speaking-{}", group.id).into(),
                    format!("口语 · {}", group.name),
                    ImportTarget::Speaking(group.id),
                    cx,
                ));
            }
            list
        });
        div()
            .id("floating-import")
            .track_focus(&self.focus)
            .key_context("Clarora")
            .size_full()
            .p(px(4.))
            .font_family(cx.theme().font_family.clone())
            .text_color(cx.theme().foreground)
            .on_action(
                cx.listener(|this, _: &ToggleFloatingImport, window, cx| this.hide(window, cx)),
            )
            .on_action(cx.listener(|this, _: &ImportAudioCommand, _, cx| {
                if !this.snapshot.busy {
                    this.import(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &Quit, _, cx| {
                let main = this.main;
                cx.defer(move |cx| {
                    let _ = main.update(cx, |_, window, cx| {
                        window.dispatch_action(Box::new(Quit), cx)
                    });
                });
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    if this.panel != FloatingPanel::Compact {
                        this.set_panel(FloatingPanel::Compact, window, cx);
                    } else {
                        this.hide(window, cx);
                    }
                    cx.stop_propagation();
                } else if event.keystroke.key == "tab" {
                    if event.keystroke.modifiers.shift {
                        window.focus_prev();
                    } else {
                        window.focus_next();
                    }
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .rounded(px(if self.panel == FloatingPanel::Compact {
                        26.
                    } else {
                        16.
                    }))
                    .border_1()
                    .border_color(cx.theme().primary.opacity(0.22))
                    .bg(cx.theme().background)
                    .shadow_sm()
                    .child(bar)
                    .children(details)
                    .children(targets),
            )
    }
}

impl AppView {
    pub(super) fn toggle_floating_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_floating_names(cx);
        if let Some(handle) = self.floating_window.take()
            && handle
                .update(cx, |_, window, _| window.remove_window())
                .is_ok()
        {
            let _ = self.lib.set_setting("floating_import_open", "false");
            cx.notify();
            return;
        }
        if let Err(error) = self.refresh_speaking_groups() {
            self.report_status(FeedbackKind::Error, format!("读取口语分组失败：{error:#}"));
        }
        let owner = cx.entity();
        let target = ImportTarget::load(&self.lib);
        let snapshot = ImportSnapshot::from_app(self, &target);
        let main = window.window_handle();
        let display = window.display(cx);
        let screen = display
            .as_ref()
            .map(|d| d.bounds())
            .unwrap_or_else(|| window.bounds());
        let bounds = Bounds::new(
            point(
                screen.right() - px(COMPACT_WIDTH + 24.),
                screen.top() + px(80.),
            ),
            size(px(COMPACT_WIDTH), px(COMPACT_HEIGHT)),
        );
        match cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: None,
                kind: WindowKind::PopUp,
                focus: false,
                is_movable: true,
                is_resizable: false,
                is_minimizable: false,
                window_background: WindowBackgroundAppearance::Transparent,
                display_id: display.map(|d| d.id()),
                ..Default::default()
            },
            move |window, cx| {
                window.set_window_title("Clarora · 快速导入");
                native::configure(window);
                let view = cx.new(|cx| FloatingImport::new(owner, main, snapshot, target, cx));
                view.read(cx).focus.focus(window);
                view
            },
        ) {
            Ok(handle) => {
                self.floating_window = Some(handle.into());
                let _ = self.lib.set_setting("floating_import_open", "true");
            }
            Err(error) => {
                self.report_status(FeedbackKind::Error, format!("打开悬浮窗失败：{error:#}"))
            }
        }
        cx.notify();
    }

    pub(super) fn close_floating_names(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.floating_name_window.take() {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    }

    pub(super) fn show_floating_names(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let Some(floating) = self.floating_window else {
            return Ok(());
        };
        let Ok((anchor, display)) =
            floating.update(cx, |_, window, cx| (window.bounds(), window.display(cx)))
        else {
            return Ok(());
        };
        if let Some(handle) = self.floating_name_window
            && handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        {
            return Ok(());
        }
        let Some(mut report) = self.command_imported.clone() else {
            return Ok(());
        };
        // Reopening uses current names; the captured IDs/group still guard saves.
        let rows = self.lib.list("listening_audios")?;
        for audio in &mut report.audios {
            if let Some(current) = rows.iter().find(|row| text(row, "id") == text(audio, "id")) {
                audio["name"] = current["name"].clone();
            }
        }
        let screen = display.as_ref().map(|d| d.bounds()).unwrap_or(anchor);
        let y = if anchor.bottom() + px(NAME_HEIGHT + 4.) <= screen.bottom() {
            anchor.bottom() + px(4.)
        } else {
            (anchor.top() - px(NAME_HEIGHT + 4.)).max(screen.top())
        };
        let bounds = Bounds::new(
            point(anchor.left(), y),
            size(px(COMPACT_WIDTH), px(NAME_HEIGHT)),
        );
        let lib = self.lib.clone();
        let owner = cx.weak_entity();
        let main = window.window_handle();
        let handle = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: None,
                kind: WindowKind::PopUp,
                focus: true,
                is_resizable: false,
                is_minimizable: false,
                display_id: display.map(|d| d.id()),
                ..Default::default()
            },
            move |window, cx| {
                window.set_window_title("Clarora · 音频名称");
                native::configure(window);
                let editor = cx.new(|cx| ImportNames::new(lib, report, owner, main, window, cx));
                cx.new(|cx| Root::new(editor, window, cx))
            },
        )?;
        self.floating_name_window = Some(handle.into());
        Ok(())
    }
}

const NAME_HEIGHT: f32 = 190.;

/// A small attached form gets its own component Root for input/IME bookkeeping.
/// The capsule retains its transparent rendering root and 232×60 bounds.
struct ImportNames {
    lib: Library,
    report: clarora_gpui::command_import::Report,
    owner: WeakEntity<AppView>,
    main: AnyWindowHandle,
    input: Entity<InputState>,
    drafts: Vec<String>,
    saved: Vec<bool>,
    index: usize,
    error: String,
    _enter: Subscription,
}
impl ImportNames {
    fn new(
        lib: Library,
        report: clarora_gpui::command_import::Report,
        owner: WeakEntity<AppView>,
        main: AnyWindowHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let drafts: Vec<_> = report.audios.iter().map(|r| text(r, "name")).collect();
        let input = cx.new(|cx| {
            let mut input = InputState::new(window, cx).placeholder("音频名称");
            input.set_value(drafts.first().cloned().unwrap_or_default(), window, cx);
            input.focus(window, cx);
            input
        });
        let enter = cx.subscribe_in(&input, window, |this, input, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { secondary: false })
                && !input.update(cx, |input, cx| {
                    input.marked_text_range(window, cx).is_some()
                })
            {
                this.save(window, cx);
            }
        });
        Self {
            lib,
            report,
            owner,
            main,
            input,
            saved: vec![false; drafts.len()],
            drafts,
            index: 0,
            error: String::new(),
            _enter: enter,
        }
    }
    fn select(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.drafts.len() {
            return;
        }
        self.drafts[self.index] = self.input.read(cx).value().to_string();
        self.saved[self.index] &=
            self.drafts[self.index].trim() == text(&self.report.audios[self.index], "name");
        self.index = index;
        self.error.clear();
        self.input.update(cx, |input, cx| {
            input.set_value(self.drafts[index].clone(), window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .owner
            .upgrade()
            .is_none_or(|owner| owner.read(cx).tasks.foreground.is_running())
        {
            self.error = "请等待当前任务完成后保存".into();
        } else {
            let name = self.input.read(cx).value().to_string();
            match self.report.rename(&self.lib, self.index, &name) {
                Ok(()) => {
                    self.saved[self.index] = true;
                    let owner = self.owner.clone();
                    let main = self.main;
                    cx.defer(move |cx| {
                        let _ = main.update(cx, |_, _, cx| {
                            let _ = owner.update(cx, |app, cx| {
                                app.refresh();
                                cx.notify();
                            });
                        });
                    });
                    if let Some(next) = (1..=self.saved.len())
                        .map(|step| (self.index + step) % self.saved.len())
                        .find(|&index| !self.saved[index])
                    {
                        self.select(next, window, cx);
                    } else {
                        window.remove_window();
                    }
                }
                Err(error) => self.error = format!("{error:#}"),
            }
        }
        cx.notify();
    }
}
impl Render for ImportNames {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.drafts.len();
        div()
            .id("import-audio-name")
            .size_full()
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .key_context("Clarora")
            .on_action(cx.listener(|this, _: &ToggleFloatingImport, _, cx| {
                let main = this.main;
                cx.defer(move |cx| {
                    let _ = main.update(cx, |_, window, cx| {
                        window.dispatch_action(Box::new(ToggleFloatingImport), cx)
                    });
                });
            }))
            .capture_action(
                cx.listener(|this, _: &gpui_component::input::Escape, window, cx| {
                    let composing = this.input.update(cx, |input, cx| {
                        let composing = input.marked_text_range(window, cx).is_some();
                        if composing {
                            input.unmark_text(window, cx);
                            cx.notify();
                        }
                        composing
                    });
                    if !composing {
                        window.remove_window();
                    }
                    cx.stop_propagation();
                }),
            )
            .on_action(cx.listener(|this, _: &Quit, _, cx| {
                let main = this.main;
                cx.defer(move |cx| {
                    let _ = main.update(cx, |_, window, cx| {
                        window.dispatch_action(Box::new(Quit), cx)
                    });
                });
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("音频名称  {}/{}", self.index + 1, count)),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(
                                Button::new("import-name-prev")
                                    .icon(IconName::ChevronLeft)
                                    .ghost()
                                    .small()
                                    .disabled(self.index == 0)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.select(this.index.saturating_sub(1), window, cx)
                                    })),
                            )
                            .child(
                                Button::new("import-name-next")
                                    .icon(IconName::ChevronRight)
                                    .ghost()
                                    .small()
                                    .disabled(self.index + 1 >= count)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.select(this.index + 1, window, cx)
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .truncate()
                    .child(format!("导入至「{}」", self.report.group_name)),
            )
            .child(Input::new(&self.input).small())
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .text_size(px(10.))
                    .line_clamp(2)
                    .text_color(if self.error.is_empty() {
                        cx.theme().muted_foreground
                    } else {
                        cx.theme().danger
                    })
                    .child(if self.error.is_empty() {
                        "名称 1–120 字，修改后保存".to_string()
                    } else {
                        self.error.clone()
                    }),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("import-name-save")
                            .label(if count > 1 {
                                "保存并下一条"
                            } else {
                                "保存名称"
                            })
                            .primary()
                            .small()
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    )
                    .child(
                        Button::new("import-name-later")
                            .label("稍后")
                            .ghost()
                            .small()
                            .on_click(|_, window, _| window.remove_window()),
                    ),
            )
    }
}

mod native {
    use gpui::Window;
    #[cfg(windows)]
    pub fn configure(window: &Window) {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos,
        };
        if let Ok(handle) = HasWindowHandle::window_handle(window) {
            if let RawWindowHandle::Win32(handle) = handle.as_raw() {
                // GPUI popup styling alone is not topmost on Windows.
                unsafe {
                    SetWindowPos(
                        handle.hwnd.get() as _,
                        HWND_TOPMOST,
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                    );
                }
            }
        }
    }
    #[cfg(target_os = "macos")]
    pub fn configure(window: &Window) {
        use objc2::{msg_send, runtime::AnyObject};
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(handle) = HasWindowHandle::window_handle(window)
            && let RawWindowHandle::AppKit(handle) = handle.as_raw()
        {
            unsafe {
                let view = handle.ns_view.as_ptr() as *mut AnyObject;
                let panel: *mut AnyObject = msg_send![view, window];
                let _: () = msg_send![panel, setHidesOnDeactivate: false];
            }
        }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    pub fn configure(_: &Window) {}
    #[cfg(target_os = "macos")]
    pub fn drag(window: &Window) {
        use objc2::{
            msg_send,
            runtime::{AnyClass, AnyObject},
        };
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(handle) = HasWindowHandle::window_handle(window)
            && let RawWindowHandle::AppKit(handle) = handle.as_raw()
        {
            // GPUI 0.2.2's start_window_move is a no-op on macOS.
            unsafe {
                let view = handle.ns_view.as_ptr() as *mut AnyObject;
                let panel: *mut AnyObject = msg_send![view, window];
                if let Some(class) = AnyClass::get(c"NSApplication") {
                    let app: *mut AnyObject = msg_send![class, sharedApplication];
                    let event: *mut AnyObject = msg_send![app, currentEvent];
                    if !panel.is_null() && !event.is_null() {
                        let _: () = msg_send![panel, performWindowDragWithEvent: event];
                    }
                }
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    pub fn drag(_: &Window) {}
}
