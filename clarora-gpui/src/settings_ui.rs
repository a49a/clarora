//! Settings render cached configuration, model and runtime status.
use super::*;

impl AppView {
    pub(super) fn settings_view(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut content = div().flex().flex_col().gap_5().w_full();
        content = content.child(self.chatgpt_settings(cx));
        content = content.child(panel(cx)
            .child(div().text_lg().child("悬浮窗与命令导入"))
            .child(self.field("command", "本机导入命令"))
            .child(div().text_sm().text_color(cx.theme().muted_foreground)
                .child("音频管理和口语题库共用这条命令。每行输出一个本机音频文件的完整路径，点击导入时执行。"))
            .child(div().flex().gap_2()
                .child(self.button("save-import-command", "保存导入命令", Action::SaveCommandImport, cx)
                    .primary().disabled(self.tasks.foreground.is_running()))
                .child(self.button("settings-floating-import", "显示 / 隐藏悬浮窗", Action::FloatingImport, cx))));
        let mut runtime = panel(cx)
            .child(div().text_lg().child("运行环境"))
            .child(if self.settings.runtime.packaged {
                "正在使用安装包内运行库"
            } else {
                "开发模式：使用本机运行库"
            })
            .child(
                self.button("runtime-refresh", "重新检查", Action::Refresh, cx)
                    .outline(),
            );
        for component in &self.settings.runtime.components {
            runtime = runtime.child(
                div()
                    .text_sm()
                    .text_color(if component.passed {
                        cx.theme().foreground
                    } else {
                        cx.theme().danger
                    })
                    .child(format!(
                        "{} {} · {}",
                        if component.passed { "✓" } else { "缺少" },
                        component.name,
                        component
                            .path
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| component.detail.clone())
                    )),
            );
        }
        content = content.child(runtime);
        content = content.child(
            panel(cx)
                .child("本机资料与外观")
                .child(self.lib.root.to_string_lossy().to_string())
                .child(self.button("theme", "切换深浅主题", Action::Theme, cx))
                .child(self.field("daily_limit", "每日新卡上限")),
        );
        let credential_locked = self.tasks.foreground.is_running()
            || self.tasks.video_job.is_some()
            || self.tasks.audio_subtitles.lane.is_running()
            || self.chat.running
            || self.chatgpt.busy();
        content=content.child(panel(cx)
                    .child(div().text_lg().child("密钥存储"))
                    .child(Switch::new("credential-security-mode")
                        .label("使用系统凭证库保护密钥")
                        .checked(self.settings.secure_credentials)
                        .disabled(credential_locked)
                        .on_click(cx.listener(|this,checked:&bool,window,cx|this.handle(Action::CredentialMode(*checked),window,cx))))
                    .child(div().text_sm().text_color(cx.theme().muted_foreground).child("默认关闭：API 密钥及 ChatGPT 登录加密密钥保存到本机 SQLite，不访问钥匙串。主动开启后使用系统凭证库，系统可能要求授权。"))
                    .child(div().text_sm().text_color(cx.theme().muted_foreground).child("SQLite 中的密钥未额外加密。ChatGPT 登录凭证另行加密保存。应用导出的资料备份不包含这些凭证和密钥。"))
                    .when(!self.settings.secure_credentials && self.data.needs_legacy_import,|v|v
                        .child(div().text_sm().child("旧密钥还在钥匙串中。可重新输入后保存，或主动导入一次。"))
                        .child(self.button("import-legacy-credentials","导入旧钥匙串密钥（可能需一次授权）",Action::ImportLegacyCredentials,cx)
                            .disabled(credential_locked))));
        // The catalog reflects the saved configuration, without accessing credentials.
        let saved_ai = self.data.saved_ai.as_ref();
        let active_model = saved_ai
            .filter(|c| c.engine == "local")
            .map(|c| c.local_model.as_str());
        let selection_summary = match saved_ai {
            Some(c) if c.engine == "local" => {
                format!("当前生效：本地模型 {}", c.local_model)
            }
            Some(_) => "当前生效：远端转写 API".into(),
            None => "无法读取已保存的模型设置".into(),
        };
        let mut models = panel(cx)
                    .child(div().text_lg().child("语音转写与本地模型"))
                    .child("用于音频、视频字幕及跟读转写。下载后可离线识别；聊天与图片识字仍使用 API。")
                    .child("中文视频请选择多语言 Whisper 或 SenseVoice；.en 仅英语。体积为下载量，不是运行内存。")
                    .child("已下载的模型，点击「选用」后立即生效。")
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(selection_summary))
                    .child(div().text_sm().text_color(cx.theme().muted_foreground).child("手动修改转写引擎或模型 ID 时，请在下方保存。"))
                    .child(self.field("engine", "转写引擎：local = 本机；compatible = 远端 API"))
                    .child(self.field("local_model", "本地语音模型 ID"));
        if let Some(progress) = &self.tasks.models.progress {
            models = models.child(
                div()
                    .p_4()
                    .rounded_lg()
                    .bg(cx.theme().accent)
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("模型下载进度（当前文件）"),
                    )
                    .child(div().text_sm().child(progress.label()))
                    .when_some(progress.percent(), |v, p| {
                        v.child(Progress::new().value(p).bg(cx.theme().primary))
                    })
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("下载源：{}", progress.source)),
                    )
                    .child(
                        self.button("cancel-model", "取消下载", Action::CancelModel, cx)
                            .disabled(
                                self.tasks
                                    .models
                                    .notice
                                    .as_ref()
                                    .is_some_and(|n| n.state == TaskState::Cancelling),
                            ),
                    ),
            );
        }
        for spec in ai::LOCAL_MODELS {
            let (model, name, details) = (spec.id, spec.name, spec.details);
            let present = self.data.asr_models.contains(model);
            let active = active_model == Some(model);
            models = models.child(
                div()
                    .p_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(if active {
                        cx.theme().primary
                    } else {
                        cx.theme().border
                    })
                    .when(active, |v| v.bg(cx.theme().accent))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .when(active, |v| {
                        v.child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().primary)
                                .child("✓ 当前生效"),
                        )
                    })
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(format!("{name}  [{model}]")),
                    )
                    .child(div().text_sm().child(details))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                self.button(
                                    format!("model-{model}"),
                                    if present {
                                        "文件已下载"
                                    } else {
                                        "下载模型"
                                    },
                                    Action::Models(model.into()),
                                    cx,
                                )
                                .disabled(present || self.tasks.models.lane.is_running()),
                            )
                            .child(
                                self.button(
                                    format!("select-model-{model}"),
                                    if active { "使用中" } else { "选用" },
                                    Action::SelectModel(model.into()),
                                    cx,
                                )
                                .selected(active)
                                .disabled(!present || self.tasks.foreground.is_running() || active),
                            ),
                    ),
            );
        }
        models = models
                    .child("运行要求：FFmpeg；Whisper 需 clarora-asr 或 whisper-cli，SenseVoice 需 clarora-asr + sherpa-onnx。下载模型不会安装推理程序。")
                    .child("本机已实测 Whisper tiny.en；其余模型尚未完成真实推理验收。详细来源、文件路径和安装方法见项目说明文档。 ")
                    .child(div().text_xs().text_color(cx.theme().muted_foreground).child("此按钮保存手动填写的 AI 与学习设置。"))
                    .child(
                        self.button("save-asr-settings", "保存 AI 与学习设置", Action::SaveAi, cx)
                            .primary()
                            .disabled(self.tasks.foreground.is_running()),
                    );
        content = content.child(models);

        content = content.child(
            panel(cx)
                .child(div().text_lg().child("同步与备份"))
                .child(div().text_sm().text_color(cx.theme().muted_foreground)
                    .child("使用云备份需要先填写你自己的存储服务信息。仅在本机保存备份时，直接点击「导出本机备份 ZIP」即可，无需配置以下项目。"))
                .child(div().text_sm().text_color(cx.theme().muted_foreground)
                    .child("云备份中的 bilibili 原视频仅保存来源链接，恢复后按需下载；字幕、学习记录和本地视频文件继续备份。本机 ZIP 保留可用视频原文件。"))
                .when(!self.data.storage_profiles.is_empty(), |panel| {
                    let active_profile = self
                        .data
                        .storage_profiles
                        .iter()
                        .find(|profile| profile.id == self.data.active_storage_profile);
                    let active = active_profile
                        .map(|profile| profile.name.as_str())
                        .unwrap_or("未选择");
                    let busy = self.tasks.foreground.is_running();
                    let mut options: Vec<_> = self
                        .data
                        .storage_profiles
                        .iter()
                        .map(|profile| {
                            (
                                profile.name.clone(),
                                Action::SelectStorageProfile(profile.id.clone()),
                                profile.id == self.data.active_storage_profile,
                                busy,
                            )
                        })
                        .collect();
                    if let Some(profile) = active_profile {
                        options.push((
                            format!("删除“{}”…", profile.name),
                            Action::DeleteStorageProfile(profile.id.clone()),
                            false,
                            busy,
                        ));
                    }
                    panel.child(self.surface_menu(
                        "storage-profile",
                        format!("当前存储：{active} ▾"),
                        options,
                        cx,
                    ))
                })
                .child(self.field("storage_profile_name", "配置名称（重命名或另存为时使用）"))
                .child(self.field("provider", "存储类型 s3 / oss"))
                .child(self.field("endpoint", "Endpoint"))
                .child(self.field("region", "Region"))
                .child(self.field("bucket", "Bucket"))
                .child(self.field("access_key", "Access Key ID"))
                .child(self.field("secret_key", "Secret Access Key"))
                .child(self.field("session_token", "Session Token（可选）"))
                .child(self.field("prefix", "资料库前缀"))
                .child(self.field("path_style", "S3 路径模式 true / false"))
                .child(div().text_xs().text_color(cx.theme().muted_foreground)
                    .child("阿里云 OSS：存储类型选 oss，Endpoint 填 https://oss-cn-hangzhou.aliyuncs.com，Region 填 cn-hangzhou（不是 auto 或 oss-cn-hangzhou），Bucket 单独填写。"))
                .child(div().text_xs().text_color(cx.theme().muted_foreground)
                    .child("「保存当前配置」更新正在使用的存储，密钥留空则沿用已保存的密钥；「另存为新配置」会再存一份并立即切换；「重命名当前配置」使用上方配置名称。"))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .flex_wrap()
                        .child(
                            self.button("save-storage", "保存当前配置", Action::SaveStorage, cx)
                                .primary()
                                .disabled(self.tasks.foreground.is_running()),
                        )
                        .child(
                            self.button(
                                "save-storage-profile",
                                "另存为新配置",
                                Action::SaveStorageProfile,
                                cx,
                            )
                            .outline()
                            .disabled(self.tasks.foreground.is_running()),
                        )
                        .child(
                            self.button(
                                "rename-storage-profile",
                                "重命名当前配置",
                                Action::RenameStorageProfile,
                                cx,
                            )
                            .outline()
                            .disabled(
                                self.tasks.foreground.is_running()
                                    || self.data.active_storage_profile.is_empty(),
                            ),
                        )
                        .child(
                            self.button("test-storage", "测试连接", Action::TestStorage, cx)
                                .outline()
                                .disabled(self.tasks.foreground.is_running()),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .flex_wrap()
                        .child(
                            self.button("cloud-backup", "备份本机资料", Action::CloudBackup, cx)
                                .disabled(self.tasks.foreground.is_running()),
                        )
                        .child(
                            self.button("cloud-list", "读取备份版本", Action::CloudList, cx)
                                .disabled(self.tasks.foreground.is_running()),
                        ),
                ),
        );
        content = content.child(panel(cx).child(self.cloud_sync_settings(cx)));
        for (i, version) in self.cloud.versions.iter().enumerate() {
            let created = chrono::DateTime::parse_from_rfc3339(&version.created_at)
                .map(|d| {
                    d.with_timezone(&chrono::Local)
                        .format("%Y-%m-%d %H:%M")
                        .to_string()
                })
                .unwrap_or_else(|_| "未知时间".into());
            content = content.child(
                self.button(
                    format!("restore-{i}"),
                    format!(
                        "恢复合并 · {created} · {}",
                        if version.format_version == 3 {
                            "增量备份"
                        } else {
                            "旧版备份"
                        }
                    ),
                    Action::CloudRestore(version.key.clone()),
                    cx,
                )
                .disabled(self.tasks.foreground.is_running()),
            );
        }
        content = content.child(
            div()
                .flex()
                .gap_2()
                .child(self.button(
                    "local-backup",
                    "导出本机备份 ZIP",
                    Action::Pick("backup".into()),
                    cx,
                ))
                .child(self.button(
                    "local-restore",
                    "导入并合并 ZIP",
                    Action::Pick("restore".into()),
                    cx,
                )),
        );
        content.into_any_element()
    }
}
