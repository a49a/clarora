use crate::{
    data::{Library, id, now},
    process::{Cancel, run, tool},
    subtitles::{Cue, parse, srt},
};
use anyhow::{Context, Result, ensure};
use base64::Engine;
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    sync::{Mutex, OnceLock, atomic::Ordering},
    time::Duration,
};
/// Supported local ASR models. UI availability and process routing share these IDs.
pub struct LocalModel {
    pub id: &'static str,
    pub name: &'static str,
    pub details: &'static str,
    pub language: &'static str,
}
impl LocalModel {
    pub(crate) fn installed(&self, lib: &Library) -> bool {
        let models = lib.root.join("models");
        if self.id == "sense-voice" {
            models.join("sense-voice/model.int8.onnx").is_file()
                && models.join("sense-voice/tokens.txt").is_file()
        } else {
            models.join(format!("ggml-{}.bin", self.id)).is_file()
        }
    }
}
pub const LOCAL_MODELS: &[LocalModel] = &[
    LocalModel {
        id: "tiny.en",
        name: "OpenAI Whisper Tiny.en",
        details: "仅英语 · 39M 参数 · 约 75 MiB · 最小的英语版本",
        language: "en",
    },
    LocalModel {
        id: "base.en",
        name: "OpenAI Whisper Base.en",
        details: "仅英语 · 74M 参数 · 约 142 MiB · 默认表单模型",
        language: "en",
    },
    LocalModel {
        id: "small.en",
        name: "OpenAI Whisper Small.en",
        details: "仅英语 · 244M 参数 · 约 466 MiB · 更大的英语版本",
        language: "en",
    },
    LocalModel {
        id: "medium.en",
        name: "OpenAI Whisper Medium.en",
        details: "仅英语 · 769M 参数 · 约 1.5 GiB · 占用和耗时更高，可对比识别效果",
        language: "en",
    },
    LocalModel {
        id: "large-v3-turbo",
        name: "OpenAI Whisper Large-v3-turbo",
        details: "多语言（含英语）· 809M 参数 · 约 1.5 GiB · Large-v3 的加速版本",
        language: "auto",
    },
    LocalModel {
        id: "base",
        name: "OpenAI Whisper Base · 多语言",
        details: "含中文、英语、日语等 · 74M 参数 · 约 142 MiB",
        language: "auto",
    },
    LocalModel {
        id: "small",
        name: "OpenAI Whisper Small · 多语言",
        details: "含中文、英语、日语等 · 244M 参数 · 约 466 MiB",
        language: "auto",
    },
    LocalModel {
        id: "sense-voice",
        name: "FunAudioLLM SenseVoiceSmall · 2024-07-17",
        details: "普通话、英语、日语、韩语、粤语 · INT8 ONNX 模型 + tokens.txt",
        language: "auto",
    },
];
pub fn local_model(id: &str) -> Option<&'static LocalModel> {
    LOCAL_MODELS.iter().find(|model| model.id == id)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialSource {
    Chat,
    Asr,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub provider: String,
    pub base_url: String,
    pub model: String,
    pub vision_model: String,
    pub asr_base_url: String,
    pub asr_model: String,
    pub local_model: String,
    pub engine: String,
    pub credential_id: String,
    pub shared_key_source: Option<CredentialSource>,
}

pub const PROVIDER_CUSTOM: &str = "custom";
pub const PROVIDER_OPENCODE_GO: &str = "opencode_go";
pub const PROVIDER_DEEPSEEK: &str = "deepseek";
pub const OPENCODE_GO_BASE_URL: &str = "https://opencode.ai/inference/go/openai/v1";
pub const DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com";

pub fn provider_label(provider: &str) -> &'static str {
    match provider {
        PROVIDER_OPENCODE_GO => "OpenCode Go",
        PROVIDER_DEEPSEEK => "DeepSeek",
        _ => "自定义 OpenAI 兼容",
    }
}

/// A named, reusable custom AI configuration. Secrets stay in credential storage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub config: Config,
}

const PROFILES_KEY: &str = "ai_profiles";
const ACTIVE_PROFILE_KEY: &str = "ai_active_profile";
const DEFAULT_PROFILE_NAME: &str = "当前配置";
const MAX_PROFILE_NAME_CHARS: usize = 60;

fn validate_profile_name(name: &str) -> Result<&str> {
    let name = name.trim();
    ensure!(!name.is_empty(), "请输入配置名称");
    ensure!(
        name.chars().count() <= MAX_PROFILE_NAME_CHARS,
        "配置名称最多 {MAX_PROFILE_NAME_CHARS} 个字符"
    );
    ensure!(
        !name.chars().any(char::is_control),
        "配置名称不能包含控制字符"
    );
    Ok(name)
}

fn default_profile(config: &Config) -> Option<Profile> {
    let has_config = !config.base_url.trim().is_empty()
        || !config.model.trim().is_empty()
        || !config.asr_base_url.trim().is_empty()
        || !config.credential_id.is_empty();
    has_config.then(|| Profile {
        id: if config.credential_id.is_empty() {
            "default".into()
        } else {
            config.credential_id.clone()
        },
        name: DEFAULT_PROFILE_NAME.into(),
        config: config.clone(),
    })
}

/// Saved profiles plus the active profile ID. An unmodified single config appears
/// as a synthetic profile so existing libraries keep working without migration.
pub fn profiles(lib: &Library) -> Result<(Vec<Profile>, String)> {
    let raw = lib.setting(PROFILES_KEY)?;
    let mut profiles: Vec<Profile> = if raw.trim().is_empty() {
        Vec::new()
    } else {
        serde_json::from_str(&raw).context("已保存的 AI 配置格式无效")?
    };
    if profiles.is_empty()
        && let Some(profile) = default_profile(&load_config(lib)?)
    {
        profiles.push(profile);
    }
    let stored = lib.setting(ACTIVE_PROFILE_KEY)?;
    let active = if profiles.iter().any(|profile| profile.id == stored) {
        stored
    } else {
        profiles
            .first()
            .map(|profile| profile.id.clone())
            .unwrap_or_default()
    };
    Ok((profiles, active))
}

pub fn list_profiles(lib: &Library) -> Result<Vec<Profile>> {
    Ok(profiles(lib)?.0)
}

/// Switch the active configuration without reading any secret.
pub fn select_profile(lib: &Library, profile_id: &str) -> Result<Config> {
    let _guard = config_lock().lock().unwrap();
    let (profiles, _) = profiles(lib)?;
    let profile = profiles
        .into_iter()
        .find(|profile| profile.id == profile_id)
        .context("AI 配置不存在，请刷新后重试")?;
    for endpoint in [&profile.config.base_url, &profile.config.asr_base_url] {
        if !endpoint.is_empty() {
            endpoint_url(endpoint)?;
        }
    }
    lib.set_setting("ai_config", &serde_json::to_string(&profile.config)?)?;
    lib.set_setting(ACTIVE_PROFILE_KEY, &profile.id)?;
    Ok(profile.config)
}

/// Keep the active profile in sync after editing the shared settings form.
pub fn sync_active_profile(lib: &Library) -> Result<()> {
    let _guard = config_lock().lock().unwrap();
    let (mut profiles, active) = profiles(lib)?;
    if active.is_empty() {
        return Ok(());
    }
    let config = load_config(lib)?;
    let Some(profile) = profiles.iter_mut().find(|profile| profile.id == active) else {
        return Ok(());
    };
    if profile.config == config {
        return Ok(());
    }
    profile.config = config;
    lib.set_setting(PROFILES_KEY, &serde_json::to_string(&profiles)?)
}

/// Save the current form as a new named profile and make it active.
pub fn save_profile(lib: &Library, name: &str, config: Config, key: &str) -> Result<Profile> {
    let name = validate_profile_name(name)?;
    let (profiles, _) = profiles(lib)?;
    ensure!(
        !profiles
            .iter()
            .any(|profile| profile.name.eq_ignore_ascii_case(name)),
        "已有同名配置，请换一个名称"
    );
    if key.is_empty() {
        save_preferences(lib, config, "")?;
    } else {
        save_shared(lib, config, key)?;
    }
    let profile = Profile {
        id: id(),
        name: name.to_owned(),
        config: load_config(lib)?,
    };
    let mut saved = profiles;
    saved.push(profile.clone());
    lib.set_setting(PROFILES_KEY, &serde_json::to_string(&saved)?)?;
    lib.set_setting(ACTIVE_PROFILE_KEY, &profile.id)?;
    Ok(profile)
}
impl Config {
    pub fn is_opencode_go(&self) -> bool {
        self.provider == PROVIDER_OPENCODE_GO
    }
    pub fn is_deepseek(&self) -> bool {
        self.provider == PROVIDER_DEEPSEEK
    }
    /// Built-in presets only expose chat completions, not audio transcription.
    pub fn is_chat_preset(&self) -> bool {
        self.is_opencode_go() || self.is_deepseek()
    }
    /// Keep legacy fields in sync so existing transcription consumers keep working.
    pub fn shared_api(mut self) -> Self {
        self.provider = match self.provider.as_str() {
            PROVIDER_OPENCODE_GO => PROVIDER_OPENCODE_GO.into(),
            PROVIDER_DEEPSEEK => PROVIDER_DEEPSEEK.into(),
            _ => PROVIDER_CUSTOM.into(),
        };
        if self.is_opencode_go() {
            self.base_url = OPENCODE_GO_BASE_URL.into();
        } else if self.is_deepseek() {
            self.base_url = DEEPSEEK_BASE_URL.into();
        }
        self.asr_base_url = self.base_url.clone();
        self.asr_model = self.model.clone();
        self
    }
}
/// Select an existing complete group for the simplified settings form.
/// Never pair one provider's URL with another provider's credential.
pub fn shared_settings(mut config: Config, chat_key: String, asr_key: String) -> (Config, String) {
    if config.base_url.trim().is_empty() && config.model.trim().is_empty() && chat_key.is_empty() {
        config.base_url = config.asr_base_url.clone();
        config.model = config.asr_model.clone();
        (config, asr_key)
    } else {
        (config, chat_key)
    }
}
pub fn save_shared(lib: &Library, config: Config, key: &str) -> Result<()> {
    let mut config = config.shared_api();
    config.shared_key_source = Some(CredentialSource::Chat);
    save(lib, config, key, key)
}
fn config_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}
/// Non-secret settings can be read at startup and for local inference without OS authorization.
pub fn load_config(lib: &Library) -> Result<Config> {
    let raw = lib.setting("ai_config")?;
    if raw.is_empty() {
        return Ok(Config {
            local_model: "base.en".into(),
            engine: "local".into(),
            ..Default::default()
        });
    }
    Ok(serde_json::from_str(&raw)?)
}
/// Change only the transcription choice in one SQLite transaction, without resolving secrets.
pub fn select_local_model(lib: &Library, model: &str) -> Result<Config> {
    let model = local_model(model).context("请选择支持的本地语音模型")?;
    ensure!(model.installed(lib), "模型文件尚未下载或不完整");
    let mut db = lib.db()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let raw: Option<String> = tx
        .query_row(
            "SELECT value FROM app_settings WHERE key='ai_config'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let mut stored = match raw.as_deref().filter(|raw| !raw.is_empty()) {
        Some(raw) => serde_json::from_str(raw)?,
        None => json!({}),
    };
    let fields = stored.as_object_mut().context("转写配置格式无效")?;
    fields.insert("engine".into(), json!("local"));
    fields.insert("local_model".into(), json!(model.id));
    let config = serde_json::from_value(stored.clone())?;
    tx.execute(
        "INSERT INTO app_settings(key,value) VALUES('ai_config',?1)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![serde_json::to_string(&stored)?],
    )?;
    tx.commit()?;
    Ok(config)
}
fn credential(lib: &Library, config: &Config, role: CredentialSource) -> Result<String> {
    if config.credential_id.is_empty() {
        return Ok(String::new());
    }
    let role = match config.shared_key_source.unwrap_or(role) {
        CredentialSource::Chat => "chat",
        CredentialSource::Asr => "asr",
    };
    crate::credentials::get(lib, &format!("{role}-{}", config.credential_id))
}
pub fn load(lib: &Library) -> Result<(Config, String, String)> {
    let _guard = config_lock().lock().unwrap();
    let config = load_config(lib)?;
    let chat = credential(lib, &config, CredentialSource::Chat)?;
    let asr = credential(lib, &config, CredentialSource::Asr)?;
    Ok((config, chat, asr))
}
pub fn load_chat(lib: &Library) -> Result<(Config, String)> {
    let _guard = config_lock().lock().unwrap();
    let config = load_config(lib)?;
    let key = credential(lib, &config, CredentialSource::Chat)?;
    Ok((config, key))
}
/// An untouched password field retains the credential reference without reading its secret.
pub fn save_preferences(lib: &Library, config: Config, key: &str) -> Result<()> {
    if !key.is_empty() {
        return save_shared(lib, config, key);
    }
    let _guard = config_lock().lock().unwrap();
    let old = load_config(lib)?;
    let source = old.shared_key_source.unwrap_or_else(|| {
        if old.base_url.trim().is_empty()
            && old.model.trim().is_empty()
            && (!old.asr_base_url.trim().is_empty() || !old.asr_model.trim().is_empty())
        {
            CredentialSource::Asr
        } else {
            CredentialSource::Chat
        }
    });
    let mut config = config.shared_api();
    for endpoint in [&config.base_url, &config.asr_base_url] {
        if !endpoint.is_empty() {
            endpoint_url(endpoint)?;
        }
    }
    config.credential_id = old.credential_id;
    config.shared_key_source = Some(source);
    lib.set_setting("ai_config", &serde_json::to_string(&config)?)
}
pub fn save(lib: &Library, mut config: Config, chat: &str, asr: &str) -> Result<()> {
    let _guard = config_lock().lock().unwrap();
    for endpoint in [&config.base_url, &config.asr_base_url] {
        if !endpoint.is_empty() {
            endpoint_url(endpoint)?;
        }
    }
    config.credential_id = id();
    crate::credentials::store_config(
        lib,
        "ai_config",
        &serde_json::to_string(&config)?,
        &[
            (format!("chat-{}", config.credential_id), chat.into()),
            (format!("asr-{}", config.credential_id), asr.into()),
        ],
    )
}

pub(crate) fn endpoint_url(base: &str) -> Result<String> {
    let url = url::Url::parse(base).context("请先设置 AI API 地址")?;
    ensure!(
        url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "API 地址不能包含凭证或查询参数"
    );
    ensure!(
        url.scheme() == "https"
            || (url.scheme() == "http"
                && [Some("localhost"), Some("127.0.0.1"), Some("[::1]")].contains(&url.host_str())),
        "远端 API 需要 HTTPS；本机服务可用 HTTP"
    );
    Ok(base.trim_end_matches('/').to_string())
}
pub use crate::ai_chat::{ChatReply, chat_stream};
pub fn chat(
    lib: &Library,
    question: &str,
    passage: &str,
    history: &[Value],
    cancel: &Cancel,
    mut progress: impl FnMut(String),
) -> Result<String> {
    Ok(
        chat_stream(lib, question, passage, history, cancel, |reply| {
            progress(reply.answer)
        })?
        .answer,
    )
}
pub fn ocr(lib: &Library, path: &Path, cancel: &Cancel) -> Result<String> {
    ensure!(!cancel.load(Ordering::SeqCst), "已取消");
    let (c, key) = load_chat(lib)?;
    let model = if c.vision_model.is_empty() {
        c.model
    } else {
        c.vision_model
    };
    ensure!(!model.is_empty(), "请配置视觉模型");
    let bytes = fs::read(path)?;
    ensure!(bytes.len() <= 30 * 1024 * 1024, "图片超过 30 MB");
    let mime = match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "png" => "image/png",
        _ => anyhow::bail!("OCR 请使用 JPG、PNG 或 WebP 图片"),
    };
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    let base = endpoint_url(&c.base_url)?;
    let provider = c.provider.clone();
    let session = crate::ai_chat::session_for(&format!("ocr:{model}:{}", path.display()));
    let payload = json!({"model":model,"messages":[{"role":"user","content":[{"type":"text","text":"逐字识别图片中的文字，保留段落，不要解释。"},{"type":"image_url","image_url":{"url":format!("data:{mime};base64,{encoded}")}}]}]});
    let connection_base = base.clone();
    let value = crate::ai_chat::remote_json(cancel, &connection_base, move |client| async move {
        let mut request = client
            .post(format!("{base}/chat/completions"))
            .json(&payload);
        request = crate::ai_chat::provider_headers(request, &provider, &session);
        if !key.is_empty() {
            request = request.bearer_auth(key);
        }
        Ok(request)
    })?;
    let text = value["choices"][0]["message"]["content"]
        .as_str()
        .context("视觉模型没有返回文字")?
        .to_string();
    ensure!(!cancel.load(Ordering::SeqCst), "已取消");
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    save_record(
        lib,
        "ocr",
        json!({"pageId":id(),"text":text,"name":name,"sourceName":name,"status":"completed","error":null,"createdAt":now()}),
    )?;
    Ok(text)
}
pub fn transcribe(lib: &Library, path: &Path, cancel: &Cancel) -> Result<Vec<Cue>> {
    ensure!(!cancel.load(Ordering::SeqCst), "已取消");
    let c = load_config(lib)?;
    if c.engine == "compatible" {
        ensure!(
            !c.is_chat_preset(),
            "{}仅提供聊天接口，请改用本地转写模型",
            provider_label(&c.provider)
        );
        let key = credential(lib, &c, CredentialSource::Asr)?;
        let base = if c.asr_base_url.is_empty() {
            &c.base_url
        } else {
            &c.asr_base_url
        };
        ensure!(!c.asr_model.is_empty(), "请配置转写模型");
        let base = endpoint_url(base)?;
        let model = c.asr_model;
        let connection_base = base.clone();
        let value =
            crate::ai_chat::remote_json(cancel, &connection_base, move |client| async move {
                let form = reqwest::multipart::Form::new()
                    .text("model", model)
                    .text("response_format", "verbose_json")
                    .file("file", path)
                    .await?;
                let mut request = client
                    .post(format!("{base}/audio/transcriptions"))
                    .multipart(form);
                if !key.is_empty() {
                    request = request.bearer_auth(key);
                }
                Ok(request)
            })?;
        ensure!(!cancel.load(Ordering::SeqCst), "已取消");
        let segments = value["segments"]
            .as_array()
            .context("转写模型必须支持 segments 时间轴")?;
        let cues = segments
            .iter()
            .filter_map(|s| {
                Some(Cue {
                    start: s["start"].as_f64()?,
                    end: s["end"].as_f64()?,
                    text: s["text"].as_str()?.trim().into(),
                })
            })
            .collect::<Vec<_>>();
        ensure!(
            !cues.is_empty()
                && cues.iter().all(|c| c.start.is_finite()
                    && c.end.is_finite()
                    && c.start >= 0.
                    && c.end > c.start
                    && !c.text.is_empty()),
            "转写模型返回无效字幕时间轴"
        );
        return Ok(cues);
    }
    let selected_model = local_model(&c.local_model).context("请选择已支持的本地转写模型")?;
    let model = if c.local_model == "sense-voice" {
        lib.root.join("models/sense-voice/model.int8.onnx")
    } else {
        lib.root
            .join("models")
            .join(format!("ggml-{}.bin", c.local_model))
    };
    ensure!(model.is_file(), "请先下载本地转写模型");
    let stem = lib.root.join("recordings").join(id());
    let wav = stem.with_extension("wav");
    let result = (|| {
        run(
            &tool("ffmpeg")?,
            &[
                "-nostdin".into(),
                "-v".into(),
                "error".into(),
                "-y".into(),
                "-i".into(),
                path.to_string_lossy().into(),
                "-ar".into(),
                "16000".into(),
                "-ac".into(),
                "1".into(),
                wav.to_string_lossy().into(),
            ],
            cancel,
            Duration::from_secs(600),
        )?;
        if c.local_model == "sense-voice" || tool("clarora-asr").is_ok() {
            let last = if c.local_model == "sense-voice" {
                lib.root
                    .join("models/sense-voice/tokens.txt")
                    .to_string_lossy()
                    .into_owned()
            } else {
                selected_model.language.into()
            };
            let output = run(
                &tool("clarora-asr")?,
                &[
                    if c.local_model == "sense-voice" {
                        "sense-voice".into()
                    } else {
                        "whisper-segmented".into()
                    },
                    wav.to_string_lossy().into_owned(),
                    model.to_string_lossy().into_owned(),
                    last,
                ],
                cancel,
                Duration::from_secs(1800),
            )?;
            let value: Value = output
                .lines()
                .rev()
                .find_map(|line| serde_json::from_str::<Value>(line).ok())
                .context("端侧引擎没有返回结果")?;
            let duration = value["duration"]
                .as_f64()
                .filter(|v| v.is_finite() && *v > 0.)
                .context("端侧引擎缺少有效时长")?;
            let cues = value["segments"]
                .as_array()
                .context("端侧引擎缺少时间轴")?
                .iter()
                .filter_map(|v| {
                    Some(Cue {
                        start: v["start"].as_f64()?.max(0.),
                        end: v["end"].as_f64()?.min(duration),
                        text: v["text"].as_str()?.trim().into(),
                    })
                })
                .filter(|c| {
                    c.start.is_finite()
                        && c.end.is_finite()
                        && c.end > c.start
                        && !c.text.is_empty()
                })
                .collect::<Vec<_>>();
            ensure!(!cues.is_empty(), "端侧引擎没有识别出文字");
            return Ok(cues);
        }
        run(
            &tool("whisper-cli")?,
            &[
                "-m".into(),
                model.to_string_lossy().into(),
                "-f".into(),
                wav.to_string_lossy().into(),
                "-l".into(),
                selected_model.language.into(),
                "-osrt".into(),
                "-of".into(),
                stem.to_string_lossy().into(),
            ],
            cancel,
            Duration::from_secs(1800),
        )?;
        let body = fs::read_to_string(stem.with_extension("srt"))?;
        let cues = parse(&body);
        ensure!(!cues.is_empty(), "端侧转写未生成字幕");
        Ok(cues)
    })();
    let _ = fs::remove_file(wav);
    let _ = fs::remove_file(stem.with_extension("srt"));
    result
}
pub fn translate(lib: &Library, cues: &[Cue], cancel: &Cancel) -> Result<Vec<Cue>> {
    let payload = cues
        .iter()
        .enumerate()
        .map(|(i, c)| json!({"id":i,"text":c.text}))
        .collect::<Vec<_>>();
    let answer = chat(
        lib,
        "把每条字幕翻译为中文。只返回 JSON 数组，每个对象包含 id 和 text，保持条数和 id。",
        &serde_json::to_string(&payload)?,
        &[],
        cancel,
        |_| {},
    )?;
    let cleaned = answer
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let values: Vec<Value> = serde_json::from_str(cleaned)?;
    ensure!(values.len() == cues.len(), "翻译条数不一致，未覆盖原字幕");
    cues.iter()
        .enumerate()
        .map(|(i, c)| {
            ensure!(values[i]["id"].as_u64() == Some(i as u64), "翻译编号不一致");
            let translated = values[i]["text"].as_str().context("翻译文字缺失")?;
            Ok(Cue {
                start: c.start,
                end: c.end,
                text: format!(
                    "{}\n{}",
                    c.text
                        .lines()
                        .filter(|s| !s.chars().any(|c| ('\u{3400}'..='\u{9fff}').contains(&c)))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    translated
                ),
            })
        })
        .collect()
}
#[derive(Clone, Serialize, Deserialize)]
pub struct KeyPoint {
    pub text: String,
    pub translation: String,
}
pub fn key_points(lib: &Library, cues: &[Cue], cancel: &Cancel) -> Result<Vec<KeyPoint>> {
    let (passage, _) = crate::subtitles::languages(cues);
    ensure!(!passage.is_empty(), "请先加载原文字幕");
    let answer = chat(
        lib,
        "选择最多30个值得学习的原文词汇或短语。只返回JSON数组，每项严格为{\"text\":\"原文中逐字出现的词汇或短语\",\"translation\":\"简短中文解释\"}，不要附加说明或改写原文。",
        &passage,
        &[],
        cancel,
        |_| {},
    )?;
    let answer = answer
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let mut points: Vec<KeyPoint> =
        serde_json::from_str(answer).context("AI 重点没有返回有效JSON数组")?;
    let source = passage.to_lowercase();
    ensure!(points.len() <= 30, "AI 返回过多重点");
    points.retain(|p| {
        !p.text.trim().is_empty()
            && p.text.len() <= 200
            && !p.translation.trim().is_empty()
            && source.contains(&p.text.to_lowercase())
    });
    ensure!(!points.is_empty(), "AI 没有返回可在原文定位的重点");
    Ok(points)
}
pub fn download_model(
    lib: &Library,
    model: &str,
    cancel: &Cancel,
    mut notify: impl FnMut(crate::model_download::Progress),
) -> Result<()> {
    let files = local_model_files(model)?;
    let directory = if model == "sense-voice" {
        lib.root.join("models/sense-voice")
    } else {
        lib.root.join("models")
    };
    crate::model_download::download(
        model,
        &directory,
        &files,
        &["https://huggingface.co", "https://hf-mirror.com"],
        cancel,
        &mut notify,
    )
}
pub fn local_model_files(model: &str) -> Result<Vec<crate::model_download::FileSpec>> {
    ensure!(local_model(model).is_some(), "模型无效");
    let files = if model == "sense-voice" {
        vec![
            (
                "model.int8.onnx".to_string(),
                "csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17",
            ),
            (
                "tokens.txt".to_string(),
                "csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17",
            ),
        ]
    } else {
        vec![(format!("ggml-{model}.bin"), "ggerganov/whisper.cpp")]
    };
    let files = files
        .into_iter()
        .map(|(name, repo)| {
            let min_bytes = if name.ends_with(".txt") {
                1024
            } else {
                1024 * 1024
            };
            crate::model_download::FileSpec {
                name,
                remote: format!("{repo}/resolve/main"),
                min_bytes,
            }
        })
        .collect::<Vec<_>>();
    Ok(files)
}
pub fn save_record(lib: &Library, kind: &str, payload: Value) -> Result<String> {
    let key = id();
    lib.put(
        "ai_records",
        json!({"id":key,"kind":kind,"payload":payload.to_string(),"created_at":now()}),
    )?;
    Ok(key)
}
/// Delete one locally stored AI record by its payload page id; no raw image is kept.
pub fn delete_record(lib: &Library, kind: &str, page_id: &str) -> Result<()> {
    ensure!(!page_id.is_empty(), "记录 ID 无效");
    let db = lib.db()?;
    let changed = db.execute(
        "DELETE FROM ai_records WHERE kind=? AND CASE WHEN json_valid(payload) THEN json_extract(payload,'$.pageId') END=?",
        params![kind, page_id],
    )?;
    ensure!(changed > 0, "记录不存在或已删除");
    Ok(())
}
pub fn save_chat(
    lib: &Library,
    session: &str,
    context: &str,
    title: &str,
    messages: &[Value],
) -> Result<()> {
    crate::chat_store::save(lib, session, context, title, messages)
}
pub fn score(reference: &str, actual: &str) -> Result<Value> {
    let word = regex::Regex::new(r"[a-z]+(?:'[a-z]+)?").unwrap();
    let a = word
        .find_iter(&reference.to_lowercase())
        .map(|m| m.as_str().to_lowercase())
        .collect::<Vec<_>>();
    let b = word
        .find_iter(&actual.to_lowercase())
        .map(|m| m.as_str().to_lowercase())
        .collect::<Vec<_>>();
    ensure!(
        a.len() <= 2000 && b.len() <= 2000,
        "跟读材料过长，请按短句练习"
    );
    let mut grid = vec![vec![0; b.len() + 1]; a.len() + 1];
    for (i, row) in grid.iter_mut().enumerate() {
        row[0] = i
    }
    for (j, cell) in grid[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            grid[i][j] = (grid[i - 1][j] + 1)
                .min(grid[i][j - 1] + 1)
                .min(grid[i - 1][j - 1] + usize::from(a[i - 1] != b[j - 1]));
        }
    }
    let (mut i, mut j) = (a.len(), b.len());
    let (mut missed, mut wrong, mut extra) = (vec![], vec![], vec![]);
    let mut matched = 0;
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && grid[i][j] == grid[i - 1][j - 1] + usize::from(a[i - 1] != b[j - 1]) {
            if a[i - 1] == b[j - 1] {
                matched += 1;
            } else {
                wrong.push(format!("{} → {}", a[i - 1], b[j - 1]));
            }
            i -= 1;
            j -= 1;
        } else if i > 0 && grid[i][j] == grid[i - 1][j] + 1 {
            i -= 1;
            missed.push(a[i].clone());
        } else {
            j -= 1;
            extra.push(b[j].clone());
        }
    }
    missed.reverse();
    wrong.reverse();
    extra.reverse();
    Ok(
        json!({"reference":reference,"transcript":actual,"accuracy":matched as f64/a.len().max(b.len()).max(1) as f64,"completeness":if a.is_empty(){0.}else{(a.len()-missed.len()) as f64/a.len() as f64},"missed":missed,"wrong":wrong,"extra":extra,"words":b.len(),"edits":grid[a.len()][b.len()]}),
    )
}
pub fn write_transcription(lib: &Library, path: &Path, cancel: &Cancel) -> Result<String> {
    let cues = transcribe(lib, path, cancel)?;
    let target = lib.root.join("media").join(format!("{}.srt", id()));
    fs::write(&target, srt(&cues))?;
    Ok(target.to_string_lossy().into())
}
