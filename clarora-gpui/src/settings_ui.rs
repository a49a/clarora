//! Settings render cached configuration, model and runtime status.
use super::*;

impl AppView {
    pub(super) fn settings_view(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut content = div().flex().flex_col().gap_5().w_full();
        content = content.child(panel(cx)
            .child(div().text_lg().child("悬浮窗与命令导入"))
            .child(self.field("command", "本机导入命令"))
            .child(div().text_sm().text_color(cx.theme().muted_foreground)
                .child("仅在点击导入时执行。命令每行输出一个本机音频文件的完整路径，音频保存到点击时选择的练习组。"))
            .child(div().flex().gap_2()
                .child(self.button("save-import-command", "保存导入命令", Action::SaveCommandImport, cx)
                    .primary().disabled(self.tasks.cancel.is_some()))
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
        content=content.child(panel(cx)
                    .child(div().text_lg().child("密钥存储"))
                    .child(Switch::new("credential-security-mode")
                        .label("使用系统凭证库保护密钥")
                        .checked(self.settings.secure_credentials)
                        .disabled(self.tasks.cancel.is_some()||self.tasks.video_job.is_some())
                        .on_click(cx.listener(|this,checked:&bool,window,cx|this.handle(Action::CredentialMode(*checked),window,cx))))
                    .child(div().text_sm().text_color(cx.theme().muted_foreground).child("默认关闭：密钥保存到本机 SQLite，不访问钥匙串。开启后使用系统凭证库，系统可能要求授权。"))
                    .child(div().text_sm().text_color(cx.theme().muted_foreground).child("SQLite 中的密钥未额外加密。应用导出的资料备份不包含密钥。"))
                    .when(!self.settings.secure_credentials && self.data.needs_legacy_import,|v|v
                        .child(div().text_sm().child("旧密钥还在钥匙串中。可重新输入后保存，或主动导入一次。"))
                        .child(self.button("import-legacy-credentials","导入旧钥匙串密钥（可能需一次授权）",Action::ImportLegacyCredentials,cx)
                            .disabled(self.tasks.cancel.is_some()||self.tasks.video_job.is_some()))));
        // Read only the saved configuration, without accessing the credential store
        // during rendering. Keep draft selection distinct from the active engine.
        let saved_ai = self.data.saved_ai.as_ref();
        let draft_model = self.value("local_model", cx);
        let draft_local = self.value("engine", cx) == "local";
        let active_model = saved_ai
            .as_ref()
            .filter(|c| c.engine == "local")
            .map(|c| c.local_model.as_str());
        let selection_summary = match saved_ai.as_ref() {
            Some(c) if c.engine == "local" => {
                format!("当前生效：本地模型 {}", c.local_model)
            }
            Some(_) => "当前生效：远端转写 API".into(),
            None => "无法读取已保存的模型设置".into(),
        };
        let pending = draft_local && active_model != Some(draft_model.as_str());
        let mut models = panel(cx)
                    .child(div().text_lg().child("本地语音识别模型 · ASR"))
                    .child("用于音频、视频字幕及跟读转写。下载后可离线识别；聊天与图片识字仍使用 API。")
                    .child("中文视频请选择多语言 Whisper 或 SenseVoice；.en 仅英语。体积为下载量，不是运行内存。")
                    .child("操作：下载 → 选用 → 点击下方「保存 AI 与学习设置」。已下载仅表示文件存在。 ")
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(selection_summary))
                    .when(pending, |v| v.child(div().text_color(cx.theme().primary).child(format!("已选择：{draft_model} · 待保存，保存后生效"))));
        if let Some(progress) = &self.tasks.model_progress {
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
                    .child(self.button("cancel-model", "取消下载", Action::Cancel, cx)),
            );
        }
        for (model, name, details) in [
            (
                "tiny.en",
                "OpenAI Whisper Tiny.en",
                "仅英语 · 39M 参数 · 约 75 MiB · 最小的英语版本",
            ),
            (
                "base.en",
                "OpenAI Whisper Base.en",
                "仅英语 · 74M 参数 · 约 142 MiB · 默认表单模型",
            ),
            (
                "small.en",
                "OpenAI Whisper Small.en",
                "仅英语 · 244M 参数 · 约 466 MiB · 更大的英语版本",
            ),
            (
                "base",
                "OpenAI Whisper Base · 多语言",
                "含中文、英语、日语等 · 74M 参数 · 约 142 MiB",
            ),
            (
                "small",
                "OpenAI Whisper Small · 多语言",
                "含中文、英语、日语等 · 244M 参数 · 约 466 MiB",
            ),
            (
                "sense-voice",
                "FunAudioLLM SenseVoiceSmall · 2024-07-17",
                "普通话、英语、日语、韩语、粤语 · INT8 ONNX 模型 + tokens.txt",
            ),
        ] {
            let present = self.data.asr_models.contains(model);
            let selected = draft_local && draft_model == model;
            let active = active_model == Some(model);
            models = models.child(
                div()
                    .p_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(if selected {
                        cx.theme().primary
                    } else {
                        cx.theme().border
                    })
                    .when(selected, |v| v.bg(cx.theme().accent))
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
                    .when(selected && !active, |v| {
                        v.child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().primary)
                                .child("✓ 已选择 · 待保存"),
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
                                .disabled(present || self.tasks.cancel.is_some()),
                            )
                            .child(
                                self.button(
                                    format!("select-model-{model}"),
                                    if selected {
                                        if active {
                                            "✓ 已选用"
                                        } else {
                                            "✓ 已选择 · 待保存"
                                        }
                                    } else {
                                        "选用"
                                    },
                                    Action::SelectModel(model.into()),
                                    cx,
                                )
                                .selected(selected)
                                .disabled(!present || self.tasks.cancel.is_some() || selected),
                            ),
                    ),
            );
        }
        models = models
                    .child("运行要求：FFmpeg；Whisper 需 clarora-asr 或 whisper-cli，SenseVoice 需 clarora-asr + sherpa-onnx。下载模型不会安装推理程序。")
                    .child("本机已实测 Whisper tiny.en；其余模型尚未完成真实推理验收。详细来源、文件路径和安装方法见项目说明文档。 ");
        content = content.child(models);
        content = content.child(
                    panel(cx)
                        .child(div().text_lg().child("AI 服务"))
                        .child(div().text_sm().text_color(cx.theme().muted_foreground)
                            .child("聊天与远端转写共用下面一组设置，模型需支持这两种能力。保存后统一生效。"))
                        .child(self.field("base_url", "共用 API Base URL"))
                        .child(self.field("model", "聊天／远端转写共用模型 ID"))
                        .child(self.field("chat_key", "共用 API Key · 留空保留已保存密钥"))
                        .child(self.field("vision_model", "视觉模型（可选，留空使用共用模型）"))
                        .child(
                            self.field("engine", "转写引擎：local = 本机；compatible = 远端 API"),
                        )
                        .child(self.field(
                            "local_model",
                            "本地语音模型 ID（local 引擎使用，选用后需保存）",
                        ))
                        .child(
                            self.button("save-ai", "保存 AI 与学习设置", Action::SaveAi, cx)
                                .primary()
                                .disabled(self.tasks.cancel.is_some()),
                        ),
                );

        content = content.child(
            panel(cx)
                .child(div().text_lg().child("同步与备份"))
                .child(div().text_sm().text_color(cx.theme().muted_foreground)
                    .child("使用云备份需要先填写你自己的存储服务信息。仅在本机保存备份时，直接点击「导出本机备份 ZIP」即可，无需配置以下项目。"))
                .child(div().text_sm().text_color(cx.theme().muted_foreground)
                    .child("云备份中的 bilibili 原视频仅保存来源链接，恢复后按需下载；字幕、学习记录和本地视频文件继续备份。本机 ZIP 保留可用视频原文件。"))
                .child(self.field("provider", "存储类型 s3 / oss"))
                .child(self.field("endpoint", "Endpoint"))
                .child(self.field("region", "Region"))
                .child(self.field("bucket", "Bucket"))
                .child(self.field("access_key", "Access Key ID"))
                .child(self.field("secret_key", "Secret Access Key"))
                .child(self.field("session_token", "Session Token（可选）"))
                .child(self.field("prefix", "资料库前缀"))
                .child(self.field("path_style", "S3 路径模式 true / false"))
                .child(
                    self.button("save-storage", "保存存储配置", Action::SaveStorage, cx)
                        .disabled(self.tasks.cancel.is_some()),
                )
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .flex_wrap()
                        .child(
                            self.button("cloud-backup", "备份本机资料", Action::CloudBackup, cx)
                                .disabled(self.tasks.cancel.is_some()),
                        )
                        .child(
                            self.button("cloud-list", "读取备份版本", Action::CloudList, cx)
                                .disabled(self.tasks.cancel.is_some()),
                        ),
                ),
        );
        for (i, key) in self.backups.iter().enumerate() {
            content = content.child(
                self.button(
                    format!("restore-{i}"),
                    format!("合并：{key}"),
                    Action::CloudRestore(key.clone()),
                    cx,
                )
                .disabled(self.tasks.cancel.is_some()),
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
