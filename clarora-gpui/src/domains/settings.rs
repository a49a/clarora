//! Settings commands own their action effects and execution.
use super::*;
pub(super) enum Command {
    ChatGpt(chatgpt_ui::Command),
    SaveAi,
    CredentialMode(bool),
    ImportLegacyCredentials,
    SaveStorage,
    Models(String),
    SelectModel(String),
    Theme,
}
impl Command {
    pub(super) fn effects(&self, _app: &AppView) -> Effects {
        Effects {
            refresh_data: matches!(self, Command::SaveAi | Command::SaveStorage),
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
            Command::CredentialMode(enabled) => {
                ensure!(
                    !app.tasks.foreground.is_running()
                        && app.tasks.video_job.is_none()
                        && !app.tasks.audio_subtitles.lane.is_running()
                        && !app.chat.running
                        && !app.chatgpt.busy(),
                    "请先完成或取消当前任务再切换密钥存储模式"
                );
                app.begin_scoped(
                    TaskScope::Application,
                    "正在切换密钥存储模式…",
                    move |lib, _, _| {
                        credentials::set_secure_mode(&lib, enabled)?;
                        Ok(Outcome::CredentialMode(enabled))
                    },
                );
            }

            Command::ImportLegacyCredentials => {
                ensure!(
                    !app.tasks.foreground.is_running()
                        && app.tasks.video_job.is_none()
                        && !app.tasks.audio_subtitles.lane.is_running()
                        && !app.chat.running
                        && !app.chatgpt.busy(),
                    "请先完成或取消当前任务"
                );
                app.begin_scoped(
                    TaskScope::Application,
                    "正在导入旧钥匙串密钥，系统可能要求一次授权…",
                    move |lib, _, _| {
                        let count = credentials::import_legacy(&lib)?;
                        Ok(Outcome::Message(format!(
                            "已导入 {count} 项密钥，以后从本机 SQLite 读取"
                        )))
                    },
                );
            }

            Command::SaveAi => {
                let config = ai::Config {
                    base_url: app.value("base_url", cx),
                    model: app.value("model", cx),
                    vision_model: app.value("vision_model", cx),
                    local_model: app.value("local_model", cx),
                    engine: app.value("engine", cx),
                    ..Default::default()
                };
                let key = app.value("chat_key", cx);
                let limit = app.value("daily_limit", cx);
                ensure!(limit.parse::<usize>().is_ok(), "每日新卡上限无效");
                app.lib.set_setting("new_cards_per_day", &limit)?;
                app.lib.set_setting("daily_limit", &limit)?;
                app.lib.set_setting("command", &app.value("command", cx))?;
                app.import_command = app.value("command", cx);
                app.begin("正在保存 AI 设置…", move |lib, _, _| {
                    ai::save_preferences(&lib, config, &key)?;
                    let location = if credentials::secure_mode(&lib)? {
                        "系统凭证库"
                    } else {
                        "本机 SQLite"
                    };
                    Ok(Outcome::Message(format!(
                        "AI 设置已保存，聊天与远端转写共用配置，密钥保存在{location}"
                    )))
                });
            }

            Command::ChatGpt(command) => app.handle_chatgpt(command, cx)?,

            Command::SaveStorage => {
                app.cloud.plan = None;
                app.cloud.versions.clear();
                app.cloud.status.clear();
                let config = backup::StorageConfig {
                    provider: app.value("provider", cx),
                    endpoint: app.value("endpoint", cx),
                    region: app.value("region", cx),
                    bucket: app.value("bucket", cx),
                    access_key_id: app.value("access_key", cx),
                    prefix: app.value("prefix", cx),
                    path_style: app.value("path_style", cx) == "true",
                    credential_id: String::new(),
                };
                let key = app.value("secret_key", cx);
                let token = app.value("session_token", cx);
                app.begin("正在保存存储凭证…", move |lib, _, _| {
                    backup::Storage::save(&lib, config, &key, &token)?;
                    Ok(Outcome::Message(format!(
                        "对象存储设置已保存，密钥保存在{}",
                        if credentials::secure_mode(&lib)? {
                            "系统凭证库"
                        } else {
                            "本机 SQLite"
                        }
                    )))
                });
            }

            Command::SelectModel(model) => {
                ensure!(
                    !app.tasks.foreground.is_running(),
                    "请先完成或取消当前任务再切换模型"
                );
                let config =
                    ai::select_local_model(&app.lib, &model).context("无法保存模型选择")?;
                app.set("local_model", model, window, cx);
                app.set("engine", "local", window, cx);
                app.data.saved_ai = Some(config);
                app.refresh_view_data();
            }

            Command::Models(model) => {
                app.begin_model_download(model_controller::Request::Asr(model))?;
            }

            Command::Theme => {
                app.dark = !app.dark;
                Theme::change(
                    if app.dark {
                        ThemeMode::Dark
                    } else {
                        ThemeMode::Light
                    },
                    Some(window),
                    cx,
                );
                design::apply(cx);
            }
        }
        Ok(())
    }
}

// Completion policy belongs to this domain: cancellation may race a durable commit.
pub(super) enum Completed {
    CredentialMode(bool),
}
impl Completed {
    pub(super) fn apply(
        self,
        app: &mut AppView,
        _context: &mut Completion,
        _window: &mut Window,
        _cx: &mut Context<AppView>,
    ) {
        match self {
            Completed::CredentialMode(enabled) => {
                app.settings.secure_credentials = enabled;
                app.report_status(
                    FeedbackKind::Info,
                    if enabled {
                        "密钥安全模式已开启，现有本机密钥已迁入系统凭证库"
                    } else {
                        "密钥安全模式已关闭。已有钥匙串密钥可重新输入保存，或主动导入一次"
                    }
                    .into(),
                );
            }
        }
    }
}
