//! Nonblocking account work; rendering reads public snapshots only.
use super::*;
use clarora_gpui::chatgpt;
#[derive(Clone)]
pub(super) enum Command {
    Login(Option<String>),
    Cancel,
    Use(bool),
    Profile(String),
    Model(String),
    Refresh,
    SignOut,
    ImportLegacy,
    Usage,
}
enum Event {
    Open(String),
    Done(std::result::Result<(chatgpt::Snapshot, String), String>),
}
pub(super) struct State {
    pub snapshot: chatgpt::Snapshot,
    pub status: String,
    pub error: String,
    cancel: Option<Cancel>,
    events: Option<mpsc::Receiver<Event>>,
}
impl State {
    pub(super) fn busy(&self) -> bool {
        self.cancel.is_some()
    }
    pub fn new(lib: &Library) -> Self {
        let result = chatgpt::snapshot(lib);
        Self {
            snapshot: result.as_ref().cloned().unwrap_or_default(),
            status: String::new(),
            error: result.err().map(|e| e.to_string()).unwrap_or_default(),
            cancel: None,
            events: None,
        }
    }
}
impl Drop for State {
    fn drop(&mut self) {
        if let Some(c) = &self.cancel {
            c.store(true, Ordering::SeqCst);
        }
    }
}
impl AppView {
    pub(super) fn handle_chatgpt(
        &mut self,
        command: Command,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        if matches!(command, Command::Usage) {
            cx.open_url("https://chatgpt.com/#settings");
            return Ok(());
        }
        if matches!(command, Command::Cancel) {
            if let Some(c) = &self.chatgpt.cancel {
                c.store(true, Ordering::SeqCst);
                self.chatgpt.status = "正在取消 ChatGPT 操作…".into();
            }
            return Ok(());
        }
        ensure!(
            self.chatgpt.cancel.is_none(),
            "请先完成或取消当前 ChatGPT 操作"
        );
        ensure!(
            !self.chat.running
                && !self.tasks.foreground.is_running()
                && !self.tasks.audio_subtitles.lane.is_running()
                && self.tasks.video_job.is_none(),
            "请先完成或取消正在使用 AI 的任务，再更改服务或账户"
        );
        if matches!(command, Command::Login(_)) {
            ensure!(
                !self.focus_mode.active(),
                "请先退出电脑专注，再打开浏览器登录 ChatGPT"
            );
        }
        let client = chatgpt::Client::new(&self.lib);
        let lib = self.lib.clone();
        let cancel = process::cancel_token();
        let token = cancel.clone();
        let (tx, rx) = mpsc::channel();
        self.chatgpt.cancel = Some(cancel);
        self.chatgpt.events = Some(rx);
        self.chatgpt.error.clear();
        self.chatgpt.status = if matches!(command, Command::Login(_)) {
            "正在准备浏览器登录…"
        } else {
            "正在更新 ChatGPT 设置…"
        }
        .into();
        std::thread::spawn(move || {
            let result = (|| -> Result<(chatgpt::Snapshot, String)> {
                let (state, message) = match command {
                    Command::Login(profile) => {
                        let state = client.sign_in(profile.as_deref(), &token, |address| {
                            tx.send(Event::Open(address.into()))
                                .map_err(|_| anyhow::anyhow!("应用已关闭"))
                        })?;
                        let message = match state.current() {
                            Some(p) if !p.sharing => "已登录，但尚未授权会员使用；可点击重新授权",
                            Some(p) if p.models.is_empty() => {
                                "已登录，暂未读到可用模型，请点击刷新模型"
                            }
                            _ => "已连接 ChatGPT，学习助手将使用所选会员账户",
                        };
                        (state, message.into())
                    }
                    Command::Use(enabled) => (
                        chatgpt::set_enabled(&lib, enabled)?,
                        if enabled {
                            "已切换到 ChatGPT 会员"
                        } else {
                            "已切换到自定义 AI 服务，使用你配置的地址和模型"
                        }
                        .into(),
                    ),
                    Command::Profile(id) => {
                        let state = client.select_profile(&id)?;
                        if state.current().is_some_and(|p| p.connected && p.sharing) {
                            (
                                client.refresh_models(&token)?,
                                "已切换 ChatGPT 账户并更新模型".into(),
                            )
                        } else {
                            (state, "已选择账户，请重新登录或授权会员使用".into())
                        }
                    }
                    Command::Model(model) => {
                        (client.select_model(&model)?, "已更新 ChatGPT 模型".into())
                    }
                    Command::Refresh => (
                        client.refresh_models(&token)?,
                        "已更新当前账户的模型目录".into(),
                    ),
                    Command::SignOut => {
                        let (state, revoked) = client.sign_out(&token)?;
                        (
                            state,
                            if revoked {
                                "已退出 ChatGPT 并撤销本次登录；原账户注册保留"
                            } else {
                                "已清除本机登录；远端撤销未确认，请到 ChatGPT 设置断开 Clarora 授权"
                            }
                            .into(),
                        )
                    }
                    Command::ImportLegacy => (
                        chatgpt::import_legacy(&lib)?,
                        "已导入旧 ChatGPT 登录，后续按所选密钥存储方式读取".into(),
                    ),
                    Command::Cancel | Command::Usage => unreachable!(),
                };
                Ok((state, message))
            })()
            .map_err(|e| format!("{e:#}"));
            let _ = tx.send(Event::Done(result));
        });
        Ok(())
    }
    pub(super) fn poll_chatgpt(&mut self, cx: &mut Context<Self>) -> bool {
        let events = self
            .chatgpt
            .events
            .as_ref()
            .map(|r| r.try_iter().collect::<Vec<_>>())
            .unwrap_or_default();
        let changed = !events.is_empty();
        for event in events {
            match event {
                Event::Open(address) => {
                    if self
                        .chatgpt
                        .cancel
                        .as_ref()
                        .is_some_and(|c| !c.load(Ordering::SeqCst))
                    {
                        cx.open_url(&address);
                        self.chatgpt.status =
                            "请在浏览器完成 ChatGPT 登录与授权，可继续切换栏目学习。".into();
                    }
                }
                Event::Done(result) => {
                    self.chatgpt.cancel = None;
                    self.chatgpt.events = None;
                    match result {
                        Ok((state, message)) => {
                            self.chatgpt.snapshot = state;
                            self.chatgpt.status = message;
                            self.chatgpt.error.clear();
                        }
                        Err(error) => {
                            if let Ok(state) = chatgpt::snapshot(&self.lib) {
                                self.chatgpt.snapshot = state;
                            }
                            self.chatgpt.error = error;
                            self.chatgpt.status.clear();
                        }
                    }
                }
            }
        }
        changed
    }
    pub(super) fn chatgpt_settings(&self, cx: &Context<Self>) -> AnyElement {
        let state = &self.chatgpt.snapshot;
        let busy = self.chatgpt.busy();
        let locked = busy
            || self.chat.running
            || self.tasks.foreground.is_running()
            || self.tasks.audio_subtitles.lane.is_running()
            || self.tasks.video_job.is_some();
        let action = |c| Action::ChatGpt(c);
        let mut panel=panel(cx)
   .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child("AI 服务设置"))
   .child(div().flex().gap_2().flex_wrap()
    .child(self.button("chatgpt-use","ChatGPT 会员",action(Command::Use(true)),cx).outline().selected(state.enabled).disabled(locked))
    .child(self.button("chatgpt-compatible","自定义 AI 服务",action(Command::Use(false)),cx).outline().selected(!state.enabled).disabled(locked)))
   .child(div().text_sm().text_color(cx.theme().muted_foreground).child("问答、字幕翻译和写作批改使用所选服务；远端语音转写和图片识字使用下方的自定义配置。"))
   .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("ChatGPT 账户与授权"));
        if state.needs_legacy_import {
            panel = panel.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().text_sm().child(
                        "旧登录保存在钥匙串中，默认不读取。可以重新登录，或主动授权导入一次。",
                    ))
                    .child(
                        self.button(
                            "chatgpt-import-legacy",
                            "导入旧 ChatGPT 登录（需钥匙串授权）",
                            action(Command::ImportLegacy),
                            cx,
                        )
                        .outline()
                        .disabled(locked),
                    ),
            );
        }
        let accounts = state
            .profiles
            .iter()
            .map(|p| {
                (
                    format!(
                        "{}{}",
                        p.label(),
                        if p.connected { "" } else { " · 需登录" }
                    ),
                    action(Command::Profile(p.id.clone())),
                    p.id == state.active,
                    locked,
                )
            })
            .collect();
        if !state.profiles.is_empty() {
            panel = panel.child(
                self.surface_menu(
                    "chatgpt-account",
                    state
                        .current()
                        .map(|p| format!("账户：{} ▾", p.label()))
                        .unwrap_or_else(|| "选择已有的 ChatGPT 注册 ▾".into()),
                    accounts,
                    cx,
                ),
            );
        }
        if let Some(p) = state.current() {
            panel = panel.child(div().text_sm().child(if p.connected {
                if p.sharing {
                    "已连接 · 已授权会员使用"
                } else {
                    "已连接 · 尚未授权会员使用"
                }
            } else if p.subject.is_empty() {
                "已收到注册 · 登录未完成，继续登录会复用该注册"
            } else {
                "已退出 · 重新登录会复用原账户注册"
            }));
            let models = p
                .models
                .iter()
                .map(|m| {
                    (
                        m.display_name.clone(),
                        action(Command::Model(m.slug.clone())),
                        m.slug == p.model,
                        locked || !p.connected,
                    )
                })
                .collect();
            panel = panel.child(self.surface_menu(
                "chatgpt-model",
                format!(
                        "模型：{} ▾",
                        p.models
                            .iter()
                            .find(|m| m.slug == p.model)
                            .map(|m| m.display_name.as_str())
                            .unwrap_or("请先刷新模型")
                    ),
                models,
                cx,
            ));
            panel = panel.child(
                div()
                    .flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        self.button(
                            "chatgpt-reconnect",
                            "重新登录 / 授权",
                            action(Command::Login(Some(p.id.clone()))),
                            cx,
                        )
                        .outline()
                        .disabled(locked || self.focus_mode.active()),
                    )
                    .child(
                        self.button("chatgpt-refresh", "刷新模型", action(Command::Refresh), cx)
                            .outline()
                            .disabled(locked || !p.connected || !p.sharing),
                    )
                    .child(
                        self.button(
                            "chatgpt-signout",
                            "退出当前账户",
                            action(Command::SignOut),
                            cx,
                        )
                        .outline()
                        .disabled(locked || !p.connected),
                    ),
            );
        }
        panel = panel.child(
            div()
                .flex()
                .gap_2()
                .flex_wrap()
                .child(
                    self.button(
                        "chatgpt-login",
                        "Continue with ChatGPT",
                        action(Command::Login(state.login_profile().map(|p| p.id.clone()))),
                        cx,
                    )
                    .outline()
                    .tooltip("继续所选账户或上次未完成的登录，复用已有应用注册")
                    .disabled(locked || self.focus_mode.active()),
                )
                .when(!state.profiles.is_empty(), |row| {
                    row.child(
                        self.button(
                            "chatgpt-add-account",
                            "添加其他账户",
                            action(Command::Login(None)),
                            cx,
                        )
                        .outline()
                        .disabled(locked || self.focus_mode.active()),
                    )
                })
                .child(
                    self.button(
                        "chatgpt-usage",
                        "查看会员用量与授权",
                        action(Command::Usage),
                        cx,
                    )
                    .outline(),
                )
                .when(busy, |row| {
                    row.child(
                        self.button("chatgpt-cancel", "取消操作", action(Command::Cancel), cx)
                            .outline(),
                    )
                }),
        );
        if !self.chatgpt.status.is_empty() {
            panel = panel.child(div().text_sm().child(self.chatgpt.status.clone()));
        }
        if !self.chatgpt.error.is_empty() {
            panel = panel.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div().text_sm().text_color(cx.theme().danger).child(
                            self.chatgpt
                                .error
                                .split("\n诊断：")
                                .next()
                                .unwrap_or("")
                                .to_string(),
                        ),
                    )
                    .child(
                        div().child(
                            self.button(
                                "chatgpt-copy-error",
                                "复制错误详情",
                                Action::CopySelection(self.chatgpt.error.clone()),
                                cx,
                            )
                            .outline()
                            .small(),
                        ),
                    ),
            );
        }
        panel
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child("默认不访问钥匙串：登录凭证在本机加密，加密密钥保存在本机 SQLite。可在「密钥存储」主动开启系统凭证库。登录凭证及密钥均不进入资料备份；直接复制整个本机资料目录会包含它们。"))
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .pt_4()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("自定义 AI 服务"))
                    .child(div().text_sm().text_color(cx.theme().muted_foreground).child("配置服务地址、模型和 API Key。远端转写需使用支持时间轴的模型；图片识字可单独指定视觉模型。"))
                    .child({
                        let provider = self.value("ai_provider", cx);
                        let selected = match provider.as_str() {
                            ai::PROVIDER_OPENCODE_GO => ai::PROVIDER_OPENCODE_GO,
                            ai::PROVIDER_DEEPSEEK => ai::PROVIDER_DEEPSEEK,
                            _ => ai::PROVIDER_CUSTOM,
                        };
                        self.surface_menu(
                            "ai-provider",
                            format!("服务类型：{} ▾", ai::provider_label(selected)),
                            vec![
                                (
                                    ai::provider_label(ai::PROVIDER_CUSTOM).into(),
                                    Action::SelectAiProvider(ai::PROVIDER_CUSTOM.into()),
                                    selected == ai::PROVIDER_CUSTOM,
                                    locked,
                                ),
                                (
                                    ai::provider_label(ai::PROVIDER_OPENCODE_GO).into(),
                                    Action::SelectAiProvider(ai::PROVIDER_OPENCODE_GO.into()),
                                    selected == ai::PROVIDER_OPENCODE_GO,
                                    locked,
                                ),
                                (
                                    ai::provider_label(ai::PROVIDER_DEEPSEEK).into(),
                                    Action::SelectAiProvider(ai::PROVIDER_DEEPSEEK.into()),
                                    selected == ai::PROVIDER_DEEPSEEK,
                                    locked,
                                ),
                            ],
                            cx,
                        )
                    })
                    .when(self.value("ai_provider", cx) == ai::PROVIDER_OPENCODE_GO, |panel| {
                        panel
                            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(
                                "OpenCode Go 固定使用官方 Base URL，自动发送 x-opencode-session 与 Clarora User-Agent；仅支持 Chat Completions 模型，如 deepseek-v4.1-flash、glm-5.3、kimi-k3。远端转写请改用本地模型。",
                            ))
                            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(format!(
                                "当前 API Base URL：{}",
                                ai::OPENCODE_GO_BASE_URL
                            )))
                    })
                    .when(self.value("ai_provider", cx) == ai::PROVIDER_DEEPSEEK, |panel| {
                        panel
                            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(
                                "DeepSeek 固定使用官方 Base URL，仅支持 Chat Completions；模型可用 deepseek-flash 或 deepseek-v4-pro。远端转写请改用本地模型。",
                            ))
                            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(format!(
                                "当前 API Base URL：{}",
                                ai::DEEPSEEK_BASE_URL
                            )))
                    })
                    .when(!self.data.ai_profiles.is_empty(), |panel| {
                        let active = self
                            .data
                            .ai_profiles
                            .iter()
                            .find(|profile| profile.id == self.data.active_ai_profile)
                            .map(|profile| profile.name.as_str())
                            .unwrap_or("未选择");
                        let options = self
                            .data
                            .ai_profiles
                            .iter()
                            .map(|profile| {
                                (
                                    profile.name.clone(),
                                    Action::SelectAiProfile(profile.id.clone()),
                                    profile.id == self.data.active_ai_profile,
                                    locked,
                                )
                            })
                            .collect();
                        panel.child(self.surface_menu(
                            "ai-profile",
                            format!("当前配置：{active} ▾"),
                            options,
                            cx,
                        ))
                    })
                    .child(self.field("ai_profile_name", "配置名称（保存为新配置时使用）"))
                    .child(self.field("base_url", "共用 API Base URL"))
                    .child(self.field("model", "聊天／远端转写共用模型 ID"))
                    .child(self.field("chat_key", "共用 API Key · 留空保留已保存密钥"))
                    .child(self.field("vision_model", "视觉模型（可选，留空使用共用模型）"))
                    .child(div().text_xs().text_color(cx.theme().muted_foreground).child("「保存为新配置」会另存一套地址、模型与密钥并立即切换；「保存 AI 与学习设置」更新当前配置。"))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .child(
                                self.button("save-ai", "保存 AI 与学习设置", Action::SaveAi, cx)
                                    .primary()
                                    .disabled(self.tasks.foreground.is_running()),
                            )
                            .child(
                                self.button(
                                    "save-ai-profile",
                                    "保存为新配置",
                                    Action::SaveAiProfile,
                                    cx,
                                )
                                .outline()
                                .disabled(locked),
                            ),
                    )
            )
            .into_any_element()
    }
}
