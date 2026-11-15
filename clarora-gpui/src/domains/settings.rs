//! Settings commands own their action effects and execution.
use super::*;
pub(super) enum Command {
    ChatGpt(chatgpt_ui::Command),
    SaveAi,
    SaveAiProfile,
    SaveTranscription,
    SaveDailyLimit,
    SelectAiProfile(String),
    SelectAiProvider(String),
    CredentialMode(bool),
    ImportLegacyCredentials,
    SaveStorage,
    SaveStorageProfile,
    RenameStorageProfile,
    DeleteStorageProfile(String),
    SelectStorageProfile(String),
    Models(String),
    SelectModel(String),
    Theme,
}
impl Command {
    pub(super) fn effects(&self, _app: &AppView) -> Effects {
        Effects {
            refresh_data: matches!(
                self,
                Command::SaveAi
                    | Command::SaveAiProfile
                    | Command::SaveTranscription
                    | Command::SaveDailyLimit
                    | Command::SelectAiProfile(_)
                    | Command::SaveStorage
                    | Command::SaveStorageProfile
                    | Command::RenameStorageProfile
                    | Command::DeleteStorageProfile(_)
                    | Command::SelectStorageProfile(_)
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
        match self {
            Command::SelectAiProvider(provider) => {
                let previous = app.value("ai_provider", cx);
                let provider = match provider.as_str() {
                    ai::PROVIDER_OPENCODE_GO => ai::PROVIDER_OPENCODE_GO,
                    ai::PROVIDER_DEEPSEEK => ai::PROVIDER_DEEPSEEK,
                    _ => ai::PROVIDER_CUSTOM,
                };
                app.set("ai_provider", provider, window, cx);
                match provider {
                    ai::PROVIDER_OPENCODE_GO => {
                        app.set("base_url", ai::OPENCODE_GO_BASE_URL, window, cx);
                        if previous != provider {
                            app.set("model", "deepseek-v4.1-flash", window, cx);
                        }
                        app.report_status(
                            FeedbackKind::Info,
                            "已选择 OpenCode Go：保存时会使用官方 Base URL，并自动发送会话标识与 Clarora User-Agent".into(),
                        );
                    }
                    ai::PROVIDER_DEEPSEEK => {
                        app.set("base_url", ai::DEEPSEEK_BASE_URL, window, cx);
                        if previous != provider {
                            app.set("model", "deepseek-flash", window, cx);
                        }
                        app.report_status(
                            FeedbackKind::Info,
                            "已选择 DeepSeek：保存时会使用官方 Base URL，模型可用 deepseek-flash 或 deepseek-v4-pro".into(),
                        );
                    }
                    _ => {
                        app.report_status(FeedbackKind::Info, "已选择自定义 OpenAI 兼容服务".into())
                    }
                }
            }

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
                let provider = app.value("ai_provider", cx);
                let old = ai::load_config(&app.lib).unwrap_or_default();
                let base_url = app.value("base_url", cx);
                let old_provider = if old.provider.trim().is_empty() {
                    ai::PROVIDER_CUSTOM
                } else {
                    old.provider.as_str()
                };
                let provider_key = if provider.trim().is_empty() {
                    ai::PROVIDER_CUSTOM
                } else {
                    provider.as_str()
                };
                let same_service = old_provider == provider_key && old.base_url == base_url;
                let config = ai::Config {
                    provider: provider.clone(),
                    base_url,
                    model: app.value("model", cx),
                    vision_model: app.value("vision_model", cx),
                    local_model: old.local_model,
                    engine: old.engine,
                    asr_base_url: old.asr_base_url,
                    asr_model: old.asr_model,
                    ..Default::default()
                };
                let key = app.value("chat_key", cx);
                require_preset_key(app, &provider, &key)?;
                ensure!(
                    !key.trim().is_empty() || same_service,
                    "更换服务类型或地址后请重新输入 API Key"
                );
                app.begin("正在保存 AI 服务…", move |lib, _, _| {
                    ai::save_chat_preferences(&lib, config, &key)?;
                    ai::sync_active_profile(&lib)?;
                    let location = if credentials::secure_mode(&lib)? {
                        "系统凭证库"
                    } else {
                        "本机 SQLite"
                    };
                    Ok(Outcome::Message(format!(
                        "AI 服务设置已保存，聊天与远端转写共用配置，密钥保存在{location}"
                    )))
                });
            }

            Command::SaveTranscription => {
                let engine = app.value("engine", cx);
                ensure!(
                    matches!(engine.as_str(), "local" | "compatible"),
                    "请选择本机或远端转写"
                );
                let mut config = ai::load_config(&app.lib).unwrap_or_default();
                config.engine = engine.clone();
                config.local_model = app.value("local_model", cx);
                if engine == "local" {
                    ensure!(!config.local_model.trim().is_empty(), "请选择本地转写模型");
                } else {
                    ensure!(
                        !config.base_url.trim().is_empty() && !config.model.trim().is_empty(),
                        "请先保存可用的 AI 服务地址和模型"
                    );
                    config.asr_base_url = config.base_url.clone();
                    config.asr_model = config.model.clone();
                    // Choosing shared remote transcription must not keep an
                    // independent legacy credential pointed at another service.
                    config.asr_credential_id.clear();
                    config.shared_key_source = Some(ai::CredentialSource::Chat);
                }
                app.lib
                    .set_setting("ai_config", &serde_json::to_string(&config)?)?;
                app.data.saved_ai = Some(config.clone());
                app.refresh_view_data();
                app.report_status(
                    FeedbackKind::Info,
                    if engine == "local" {
                        format!("本机转写已保存：{}", config.local_model)
                    } else {
                        format!(
                            "远端转写已保存：{} · {}",
                            config.asr_base_url, config.asr_model
                        )
                    },
                );
            }

            Command::SaveDailyLimit => {
                let limit = app.value("daily_limit", cx);
                let limit: usize = limit.parse().context("每日新卡上限无效")?;
                ensure!((1..=1000).contains(&limit), "每日新卡上限需为 1 至 1000");
                let mut db = app.lib.db()?;
                let tx = db.transaction()?;
                for key in ["new_cards_per_day", "daily_limit"] {
                    tx.execute(
                        "INSERT INTO app_settings(key,value) VALUES(?1,?2)
                         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                        rusqlite::params![key, limit.to_string()],
                    )?;
                }
                tx.commit()?;
                app.report_status(FeedbackKind::Info, format!("每日新卡上限已保存为 {limit}"));
            }

            Command::SaveAiProfile => {
                ensure_ai_idle(app)?;
                let name = app.value("ai_profile_name", cx);
                ensure!(!name.trim().is_empty(), "请先填写配置名称");
                let provider = app.value("ai_provider", cx);
                let old = ai::load_config(&app.lib).unwrap_or_default();
                let config = ai::Config {
                    provider: provider.clone(),
                    base_url: app.value("base_url", cx),
                    model: app.value("model", cx),
                    vision_model: app.value("vision_model", cx),
                    local_model: old.local_model,
                    engine: old.engine,
                    asr_base_url: old.asr_base_url,
                    asr_model: old.asr_model,
                    ..Default::default()
                };
                let key = app.value("chat_key", cx);
                require_preset_key(app, &provider, &key)?;
                app.begin("正在保存并切换 AI 配置…", move |lib, _, _| {
                    let profile = ai::save_profile(&lib, &name, config, &key)?;
                    let location = if credentials::secure_mode(&lib)? {
                        "系统凭证库"
                    } else {
                        "本机 SQLite"
                    };
                    Ok(Outcome::Message(format!(
                        "已保存并切换到“{}”，密钥保存在{location}",
                        profile.name
                    )))
                });
            }

            Command::SelectAiProfile(id) => {
                ensure_ai_idle(app)?;
                let name = ai::list_profiles(&app.lib)?
                    .into_iter()
                    .find(|profile| profile.id == id)
                    .map(|profile| profile.name)
                    .unwrap_or_default();
                let config = ai::select_profile(&app.lib, &id)?;
                app.apply_ai_config(&config, window, cx);
                app.set("ai_profile_name", name.clone(), window, cx);
                app.report_status(
                    FeedbackKind::Info,
                    if name.is_empty() {
                        "已切换 AI 配置".into()
                    } else {
                        format!("已切换到“{name}”")
                    },
                );
            }

            Command::ChatGpt(command) => app.handle_chatgpt(command, cx)?,

            Command::SaveStorage => {
                ensure!(
                    !app.tasks.cloud.is_running(),
                    "云任务正在使用当前存储，请完成或取消后再修改存储配置"
                );
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

            Command::SaveStorageProfile => {
                ensure!(
                    !app.tasks.cloud.is_running(),
                    "云任务正在使用当前存储，请完成或取消后再修改存储配置"
                );
                ensure!(
                    !app.tasks.foreground.is_running(),
                    "请先完成当前任务再保存存储配置"
                );
                let name = app.value("storage_profile_name", cx);
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
                app.cloud.plan = None;
                app.cloud.versions.clear();
                app.cloud.status.clear();
                app.begin("正在保存并切换存储配置…", move |lib, _, _| {
                    let profile = backup::Storage::save_profile(&lib, &name, config, &key, &token)?;
                    let location = if credentials::secure_mode(&lib)? {
                        "系统凭证库"
                    } else {
                        "本机 SQLite"
                    };
                    Ok(Outcome::Message(format!(
                        "已保存并切换到“{}”，密钥保存在{location}",
                        profile.name
                    )))
                });
            }

            Command::SelectStorageProfile(id) => {
                ensure!(
                    !app.tasks.cloud.is_running(),
                    "云任务正在使用当前存储，请完成或取消后再切换存储配置"
                );
                ensure!(
                    !app.tasks.foreground.is_running(),
                    "请先完成当前任务再切换存储配置"
                );
                let (profiles, _) = backup::profiles(&app.lib)?;
                let profile = profiles
                    .into_iter()
                    .find(|profile| profile.id == id)
                    .context("存储配置不存在，请刷新后重试")?;
                backup::select_profile(&app.lib, &id)?;
                app.apply_storage_config(&profile.config, Some(&profile.name), window, cx);
                app.data.active_storage_profile = profile.id.clone();
                app.cloud.plan = None;
                app.cloud.versions.clear();
                app.cloud.status.clear();
                app.report_status(
                    FeedbackKind::Info,
                    format!(
                        "已切换存储配置为“{}”；新同步、备份与恢复会使用这份配置",
                        profile.name
                    ),
                );
            }

            Command::RenameStorageProfile => {
                ensure!(
                    !app.tasks.foreground.is_running(),
                    "请先完成当前任务再重命名存储配置"
                );
                let id = app.data.active_storage_profile.clone();
                ensure!(!id.is_empty(), "请先选择一个存储配置");
                let name = app.value("storage_profile_name", cx);
                let profile = backup::Storage::rename_profile(&app.lib, &id, &name)?;
                app.report_status(
                    FeedbackKind::Info,
                    format!("存储配置已重命名为“{}”", profile.name),
                );
            }

            Command::DeleteStorageProfile(id) => {
                ensure!(
                    !app.tasks.cloud.is_running(),
                    "云任务正在使用当前存储，请完成或取消后再删除存储配置"
                );
                ensure!(
                    !app.tasks.foreground.is_running(),
                    "请先完成当前任务再删除存储配置"
                );
                let profile = app
                    .data
                    .storage_profiles
                    .iter()
                    .find(|profile| profile.id == id)
                    .cloned()
                    .context("存储配置不存在，请刷新后重试")?;
                let view = cx.entity().downgrade();
                window.open_dialog(cx, move |dialog, window, cx| {
                    let view = view.clone();
                    let id = id.clone();
                    dialog
                        .title("删除这个存储配置？")
                        .confirm()
                        .w((window.viewport_size().width - px(32.)).min(px(480.)))
                        .button_props(
                            DialogButtonProps::default()
                                .cancel_text("取消")
                                .ok_text("确认删除")
                                .ok_variant(ButtonVariant::Danger),
                        )
                        .child(div().text_sm().child(format!("配置：{}", profile.name)))
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(
                                    "只会删除本机配置和未共享的密钥；云端的备份文件不会被删除。",
                                ),
                        )
                        .on_ok(move |_, _, cx| {
                            let _ = view.update(cx, |this, cx| {
                                match backup::Storage::delete_profile(&this.lib, &id) {
                                    Ok(()) => {
                                        this.cloud.plan = None;
                                        this.cloud.versions.clear();
                                        this.cloud.status.clear();
                                        this.refresh_view_data();
                                        this.report_status(
                                            FeedbackKind::Info,
                                            "已删除存储配置并切换到剩余配置".into(),
                                        );
                                    }
                                    Err(error) => this.report_status(
                                        FeedbackKind::Error,
                                        format!("删除存储配置失败：{error:#}"),
                                    ),
                                }
                                cx.notify();
                            });
                            true
                        })
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

fn ensure_ai_idle(app: &AppView) -> Result<()> {
    ensure!(
        !app.chat.running
            && !app.tasks.foreground.is_running()
            && !app.tasks.audio_subtitles.lane.is_running()
            && app.tasks.video_job.is_none()
            && !app.chatgpt.busy(),
        "请先完成或取消正在使用 AI 的任务，再保存或切换配置"
    );
    Ok(())
}

fn require_preset_key(app: &AppView, provider: &str, key: &str) -> Result<()> {
    if matches!(provider, ai::PROVIDER_OPENCODE_GO | ai::PROVIDER_DEEPSEEK) && key.is_empty() {
        let saved = ai::load_config(&app.lib)?;
        ensure!(
            saved.provider == provider,
            "首次使用{}请填写 API Key",
            ai::provider_label(provider)
        );
    }
    Ok(())
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
