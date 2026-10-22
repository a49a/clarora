use crate::{
    data::{Library, id, now},
    process::{Cancel, run, tool},
    subtitles::{Cue, parse, srt},
};
use anyhow::{Context, Result, ensure};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    sync::{Mutex, OnceLock, atomic::Ordering},
    time::Duration,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialSource {
    Chat,
    Asr,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
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
impl Config {
    /// Keep legacy fields in sync so existing transcription consumers keep working.
    pub fn shared_api(mut self) -> Self {
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
fn secret_cache() -> &'static Mutex<BTreeMap<String, String>> {
    static CACHE: OnceLock<Mutex<BTreeMap<String, String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
}
fn secret(name: &str) -> Result<String> {
    let mut cache = secret_cache().lock().unwrap();
    if let Some(value) = cache.get(name) {
        return Ok(value.clone());
    }
    let value = match keyring::Entry::new("Clarora-GPUI", name)?.get_password() {
        Ok(v) => v,
        Err(keyring::Error::NoEntry) => String::new(),
        Err(e) => return Err(e.into()),
    };
    cache.insert(name.to_owned(), value.clone());
    Ok(value)
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
fn credential(config: &Config, role: CredentialSource) -> Result<String> {
    if config.credential_id.is_empty() {
        return Ok(String::new());
    }
    let role = match config.shared_key_source.unwrap_or(role) {
        CredentialSource::Chat => "chat",
        CredentialSource::Asr => "asr",
    };
    secret(&format!("{role}-{}", config.credential_id))
}
pub fn load(lib: &Library) -> Result<(Config, String, String)> {
    let _guard = config_lock().lock().unwrap();
    let config = load_config(lib)?;
    let chat = credential(&config, CredentialSource::Chat)?;
    let asr = credential(&config, CredentialSource::Asr)?;
    Ok((config, chat, asr))
}
pub fn load_chat(lib: &Library) -> Result<(Config, String)> {
    let _guard = config_lock().lock().unwrap();
    let config = load_config(lib)?;
    let key = credential(&config, CredentialSource::Chat)?;
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
    let chat_entry =
        keyring::Entry::new("Clarora-GPUI", &format!("chat-{}", config.credential_id))?;
    let asr_entry = keyring::Entry::new("Clarora-GPUI", &format!("asr-{}", config.credential_id))?;
    let result = (|| {
        chat_entry.set_password(chat)?;
        asr_entry.set_password(asr)?;
        lib.set_setting("ai_config", &serde_json::to_string(&config)?)
    })();
    if result.is_err() {
        let _ = chat_entry.delete_credential();
        let _ = asr_entry.delete_credential();
    }
    if result.is_ok() {
        let mut cache = secret_cache().lock().unwrap();
        cache.insert(format!("chat-{}", config.credential_id), chat.to_owned());
        cache.insert(format!("asr-{}", config.credential_id), asr.to_owned());
    }
    result
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
    let payload = json!({"model":model,"messages":[{"role":"user","content":[{"type":"text","text":"逐字识别图片中的文字，保留段落，不要解释。"},{"type":"image_url","image_url":{"url":format!("data:{mime};base64,{encoded}")}}]}]});
    let value = crate::ai_chat::remote_json(cancel, move |client| async move {
        let mut request = client
            .post(format!("{base}/chat/completions"))
            .json(&payload);
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
        let key = credential(&c, CredentialSource::Asr)?;
        let base = if c.asr_base_url.is_empty() {
            &c.base_url
        } else {
            &c.asr_base_url
        };
        ensure!(!c.asr_model.is_empty(), "请配置转写模型");
        let base = endpoint_url(base)?;
        let model = c.asr_model;
        let value = crate::ai_chat::remote_json(cancel, move |client| async move {
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
    ensure!(
        [
            "tiny.en",
            "base.en",
            "small.en",
            "base",
            "small",
            "sense-voice"
        ]
        .contains(&c.local_model.as_str()),
        "请选择已下载的 Whisper 模型"
    );
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
            } else if c.local_model.ends_with(".en") {
                "en".into()
            } else {
                "auto".into()
            };
            let output = run(
                &tool("clarora-asr")?,
                &[
                    if c.local_model == "sense-voice" {
                        "sense-voice".into()
                    } else {
                        "whisper".into()
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
                if c.local_model.ends_with(".en") {
                    "en".into()
                } else {
                    "auto".into()
                },
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
    ensure!(
        [
            "tiny.en",
            "base.en",
            "small.en",
            "base",
            "small",
            "sense-voice"
        ]
        .contains(&model),
        "模型无效"
    );
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
    let directory = if model == "sense-voice" {
        lib.root.join("models/sense-voice")
    } else {
        lib.root.join("models")
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
    crate::model_download::download(
        model,
        &directory,
        &files,
        &["https://huggingface.co", "https://hf-mirror.com"],
        cancel,
        &mut notify,
    )
}
pub fn save_record(lib: &Library, kind: &str, payload: Value) -> Result<String> {
    let key = id();
    lib.put(
        "ai_records",
        json!({"id":key,"kind":kind,"payload":payload.to_string(),"created_at":now()}),
    )?;
    Ok(key)
}
pub fn save_chat(
    lib: &Library,
    session: &str,
    context: &str,
    title: &str,
    messages: &[Value],
) -> Result<()> {
    let mut db = lib.db()?;
    let tx = db.transaction()?;
    tx.execute("INSERT INTO ai_records(id,kind,payload,created_at) VALUES(?,'chat',?,?) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload,created_at=excluded.created_at", rusqlite::params![session,json!({"id":session,"context":context,"title":title,"messages":messages}).to_string(),chrono::Utc::now().format("%Y-%m-%d %H:%M:%S%.9f").to_string()])?;
    let rows = {
        let mut stmt = tx.prepare("SELECT id,payload FROM ai_records WHERE kind='chat' ORDER BY created_at DESC,rowid DESC")?;
        stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?
    };
    let mut count = 0;
    for (key, payload) in rows {
        if serde_json::from_str::<Value>(&payload)
            .ok()
            .is_some_and(|p| p["context"] == context)
        {
            count += 1;
            if count > 12 {
                tx.execute("DELETE FROM ai_records WHERE id=?", [key])?;
            }
        }
    }
    tx.commit()?;
    Ok(())
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
    for j in 0..=b.len() {
        grid[0][j] = j
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
