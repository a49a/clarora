//! Object storage credentials, signed HTTP requests and streaming transport.
use crate::data::{Library, id};
use crate::process::Cancel;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex, atomic::Ordering},
    task::{Context as TaskContext, Poll},
    time::{Duration, Instant},
};

pub(crate) fn check_cancel(cancel: &Cancel) -> Result<()> {
    ensure!(!cancel.load(Ordering::SeqCst), "已取消");
    Ok(())
}
async fn interrupted(cancel: &Cancel) {
    while !cancel.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
pub(crate) async fn cancellable<T>(
    cancel: &Cancel,
    work: impl Future<Output = Result<T>>,
) -> Result<T> {
    check_cancel(cancel)?;
    tokio::select! {
        biased;
        // A completed PUT response wins over a concurrent late cancellation.
        result = work => result,
        _ = interrupted(cancel) => anyhow::bail!("已取消"),
    }
}

// Error bodies can include request signatures, object names and other private
// details. Only retain the short service error identifier, never its Message.
fn storage_error_code(bytes: &[u8]) -> Option<String> {
    use quick_xml::events::Event;
    let xml = std::str::from_utf8(bytes).ok()?;
    if xml.contains("<!DOCTYPE") || xml.contains("<!ENTITY") {
        return None;
    }
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut depth = 0_usize;
    let mut root_seen = false;
    let mut code = None;
    loop {
        match reader.read_event().ok()? {
            Event::Start(tag) => {
                if depth == 0 {
                    if root_seen || tag.local_name().as_ref() != b"Error" {
                        return None;
                    }
                    root_seen = true;
                }
                if depth == 1 && tag.local_name().as_ref() == b"Code" {
                    if code.is_some() {
                        return None;
                    }
                    let value = reader.read_text(tag.name()).ok()?;
                    if value.is_empty()
                        || value.len() > 64
                        || !value
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                    {
                        return None;
                    }
                    code = Some(value.into_owned());
                } else {
                    depth += 1;
                    if depth > 32 {
                        return None;
                    }
                }
            }
            Event::End(_) => depth = depth.checked_sub(1)?,
            Event::Empty(tag) => {
                if depth == 0 || (depth == 1 && tag.local_name().as_ref() == b"Code") {
                    return None;
                }
            }
            Event::Text(text) if depth == 0 => {
                let bytes: &[u8] = text.as_ref();
                if !bytes.iter().all(u8::is_ascii_whitespace) {
                    return None;
                }
            }
            Event::DocType(_) | Event::CData(_) | Event::GeneralRef(_) if depth == 0 => {
                return None;
            }
            Event::Eof => return (root_seen && depth == 0).then_some(code).flatten(),
            _ => {}
        }
    }
}

fn storage_error_message(status: reqwest::StatusCode, code: Option<&str>) -> String {
    let hint = match code {
        Some("AccessDenied" | "AllAccessDisabled") => {
            "访问被拒绝。请确认密钥所属账户、Bucket，以及该桶的对象读写权限。"
        }
        Some("SignatureDoesNotMatch") => {
            "签名校验失败。请核对 Secret Access Key 是否与 Access Key ID 配对，以及 Endpoint 和 Region 是否正确；复制时不要带入空格或换行。"
        }
        Some("InvalidAccessKeyId" | "InvalidAccessKeyID") => {
            "Access Key ID 无效。请确认密钥未被撤销，并使用对象存储的 S3 访问密钥。"
        }
        Some("ExpiredToken" | "TokenRefreshRequired") => {
            "临时凭证已过期。请更新访问密钥及 Session Token；使用长期密钥时无需填写 Session Token。"
        }
        Some("InvalidToken") => {
            "Session Token 无效。请核对临时凭证；使用长期密钥时请清空 Session Token 后保存。"
        }
        Some("RequestTimeTooSkewed" | "RequestExpired" | "ExpiredRequest") => {
            "请求时间校验失败。请检查本机日期、时间和时区，并开启系统自动校时后重试。"
        }
        Some("AuthorizationHeaderMalformed") => {
            "请求认证信息不符合存储服务要求。请核对 Endpoint 和 Region；Cloudflare R2 的 Region 使用 auto。"
        }
        Some("NotEntitled") => {
            "账户尚未启用当前存储功能。使用 Cloudflare R2 时，请确认账户已启用 R2 服务。"
        }
        Some("ObjectLockedByBucketPolicy") => {
            "对象受桶的对象锁定规则保护，暂时无法修改。请检查该桶的保留规则。"
        }
        Some("NoSuchBucket") => "找不到 Bucket。请核对桶名，以及 Endpoint 是否属于该桶所在账户。",
        Some("InvalidArgument") => {
            "请求参数被存储服务拒绝。阿里云 OSS 请在「存储类型」选择 oss，Region 填 cn-hangzhou（不要填 auto 或 oss-cn-hangzhou），Endpoint 填 https://oss-cn-hangzhou.aliyuncs.com，Bucket 单独填写。"
        }
        Some("SlowDown" | "Throttling") => "请求过于频繁，请稍后重试。",
        _ if status == reqwest::StatusCode::FORBIDDEN => {
            "服务端拒绝了请求，暂不能确定是密钥或权限问题。请保留此错误码继续排查具体原因。"
        }
        _ if status.is_server_error() => "存储服务暂时不可用，请稍后重试。",
        _ => "请核对存储服务配置，或稍后重试。",
    };
    format!(
        "对象存储请求失败：HTTP {status}{}。{hint}",
        code.map(|value| format!("（{value}）")).unwrap_or_default()
    )
}
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    pub provider: String,
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub access_key_id: String,
    pub prefix: String,
    pub path_style: bool,
    pub credential_id: String,
}

impl StorageConfig {
    /// Aliyun OSS V4 requires the region ID (for example `cn-hangzhou`) in the
    /// credential scope. R2's `auto` and endpoint-shaped values are invalid.
    /// Derive the region from a standard Aliyun endpoint when possible.
    pub fn normalized(mut self) -> Self {
        if let Some(region) = aliyun_region(&self.endpoint, &self.bucket) {
            if self.provider == "s3" || self.provider.is_empty() {
                self.provider = "oss".into();
            }
            if self.provider == "oss"
                && (self.region.trim().is_empty()
                    || self.region.eq_ignore_ascii_case("auto")
                    || self.region.starts_with("oss-"))
            {
                self.region = region;
            }
        }
        if self.provider == "oss" {
            if let Some(region) = self.region.strip_prefix("oss-") {
                self.region = region.trim_end_matches("-internal").to_string();
            }
            // Users often paste the bucket endpoint. Avoid bucket.bucket.host.
            let host = self
                .endpoint
                .trim_start_matches("https://")
                .trim_end_matches('/');
            if let Some(rest) = host.strip_prefix(&format!("{}.", self.bucket))
                && rest.starts_with("oss-")
            {
                self.endpoint = format!("https://{rest}");
            }
        }
        self
    }
}

fn aliyun_region(endpoint: &str, bucket: &str) -> Option<String> {
    let host = endpoint
        .trim_start_matches("https://")
        .trim_end_matches('/');
    let host = host.strip_prefix(&format!("{bucket}.")).unwrap_or(host);
    let rest = host.strip_prefix("oss-")?;
    let region = rest
        .strip_suffix(".aliyuncs.com")
        .or_else(|| rest.strip_suffix(".aliyuncs.com.cn"))?;
    Some(region.trim_end_matches("-internal").to_string())
}

#[derive(Clone, Serialize, Deserialize)]
pub struct StorageProfile {
    pub id: String,
    pub name: String,
    pub config: StorageConfig,
}

const PROFILES_KEY: &str = "storage_profiles";
const ACTIVE_PROFILE_KEY: &str = "storage_active_profile";
const DEFAULT_PROFILE_NAME: &str = "当前存储";
const MAX_PROFILE_NAME_CHARS: usize = 60;

fn validate_profile_name(name: &str) -> Result<&str> {
    let name = name.trim();
    ensure!(!name.is_empty(), "请输入存储配置名称");
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

fn default_profile(config: &StorageConfig) -> Option<StorageProfile> {
    let has_config = !config.endpoint.trim().is_empty()
        || !config.bucket.trim().is_empty()
        || !config.access_key_id.trim().is_empty()
        || !config.credential_id.trim().is_empty();
    has_config.then(|| StorageProfile {
        id: if config.credential_id.is_empty() {
            "default".into()
        } else {
            config.credential_id.clone()
        },
        name: DEFAULT_PROFILE_NAME.into(),
        config: config.clone(),
    })
}

/// Saved storage profiles plus the active profile ID. A single legacy config
/// appears as a synthetic profile so existing libraries keep working.
pub fn profiles(lib: &Library) -> Result<(Vec<StorageProfile>, String)> {
    let raw = lib.setting(PROFILES_KEY)?;
    let mut profiles: Vec<StorageProfile> = if raw.trim().is_empty() {
        Vec::new()
    } else {
        serde_json::from_str(&raw).context("已保存的存储配置格式无效")?
    };
    for profile in &mut profiles {
        profile.config = profile.config.clone().normalized();
    }
    if profiles.is_empty() {
        let config: StorageConfig =
            serde_json::from_str(&lib.setting("storage_config")?).unwrap_or_default();
        let config = config.normalized();
        if let Some(profile) = default_profile(&config) {
            profiles.push(profile);
        }
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

pub fn list_profiles(lib: &Library) -> Result<Vec<StorageProfile>> {
    Ok(profiles(lib)?.0)
}

/// Switch the active storage profile. The secret stays under its credential ID.
pub fn select_profile(lib: &Library, profile_id: &str) -> Result<StorageConfig> {
    let (profiles, _) = profiles(lib)?;
    let profile = profiles
        .into_iter()
        .find(|profile| profile.id == profile_id)
        .context("存储配置不存在，请刷新后重试")?;
    lib.set_setting("storage_config", &serde_json::to_string(&profile.config)?)?;
    lib.set_setting(ACTIVE_PROFILE_KEY, &profile.id)?;
    Ok(profile.config)
}

/// Keep the active profile in sync after editing the shared storage form.
pub fn sync_active_profile(lib: &Library) -> Result<()> {
    let (mut profiles, active) = profiles(lib)?;
    let Some(profile) = profiles.iter_mut().find(|profile| profile.id == active) else {
        return Ok(());
    };
    let config = serde_json::from_str::<StorageConfig>(&lib.setting("storage_config")?)
        .context("已保存的存储配置格式无效")?
        .normalized();
    if profile.config == config {
        return Ok(());
    }
    profile.config = config;
    lib.set_setting(PROFILES_KEY, &serde_json::to_string(&profiles)?)
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UploadStage {
    Preparing,
    Uploading,
    Publishing,
    Completed,
}

/// Attachment bytes handed to the HTTP transport, and files acknowledged by
/// the service. Completion also requires a successful manifest response.
#[derive(Clone, Debug, Serialize)]
pub struct UploadProgress {
    pub stage: UploadStage,
    pub bytes_sent: u64,
    pub bytes_total: u64,
    pub bytes_reused: u64,
    pub files_done: usize,
    pub files_total: usize,
    pub current_name: String,
}
impl UploadProgress {
    pub fn percent(&self) -> Option<f32> {
        match self.stage {
            UploadStage::Preparing => None,
            UploadStage::Uploading if self.bytes_total == 0 => None,
            UploadStage::Uploading => Some(
                ((self.bytes_sent.min(self.bytes_total) as f64 / self.bytes_total as f64) * 100.)
                    .min(99.) as f32,
            ),
            UploadStage::Publishing => Some(99.),
            UploadStage::Completed => Some(100.),
        }
    }
    pub fn label(&self) -> String {
        match self.stage {
            UploadStage::Preparing if !self.current_name.is_empty() => self.current_name.clone(),
            UploadStage::Preparing => "正在整理资料并计算内容校验…".into(),
            UploadStage::Uploading => format!(
                "正在处理附件 · 已完成 {}/{} 个\n本轮上传 {} / {} · 复用 {}{}",
                self.files_done,
                self.files_total,
                upload_size(self.bytes_sent),
                upload_size(self.bytes_total),
                upload_size(self.bytes_reused),
                if self.current_name.is_empty() {
                    String::new()
                } else {
                    format!(" · {}", self.current_name)
                }
            ),
            UploadStage::Publishing => "附件已处理，正在提交云端版本并等待确认…".into(),
            UploadStage::Completed => format!(
                "已完成 · {} 个附件 · 上传 {} · 复用 {}",
                self.files_total,
                upload_size(self.bytes_sent),
                upload_size(self.bytes_reused)
            ),
        }
    }
}
fn upload_size(bytes: u64) -> String {
    let (divisor, suffix) = if bytes >= 1024 * 1024 * 1024 {
        (1024. * 1024. * 1024., "GiB")
    } else if bytes >= 1024 * 1024 {
        (1024. * 1024., "MiB")
    } else if bytes >= 1024 {
        (1024., "KiB")
    } else {
        return format!("{bytes} B");
    };
    format!("{:.1} {suffix}", bytes as f64 / divisor)
}

pub(crate) struct UploadReporter {
    state: Mutex<(UploadProgress, Instant)>,
    callback: Box<dyn Fn(UploadProgress) + Send + Sync>,
}
impl UploadReporter {
    pub(crate) fn new(callback: impl Fn(UploadProgress) + Send + Sync + 'static) -> Self {
        Self {
            state: Mutex::new((
                UploadProgress {
                    stage: UploadStage::Preparing,
                    bytes_sent: 0,
                    bytes_total: 0,
                    bytes_reused: 0,
                    files_done: 0,
                    files_total: 0,
                    current_name: String::new(),
                },
                Instant::now(),
            )),
            callback: Box::new(callback),
        }
    }
    pub(crate) fn update(&self, force: bool, update: impl FnOnce(&mut UploadProgress)) {
        let progress = {
            let mut state = self.state.lock().unwrap();
            update(&mut state.0);
            if force || state.1.elapsed() >= Duration::from_millis(100) {
                state.1 = Instant::now();
                Some(state.0.clone())
            } else {
                None
            }
        };
        if let Some(progress) = progress {
            (self.callback)(progress);
        }
    }
    pub(crate) fn sent(&self) -> u64 {
        self.state.lock().unwrap().0.bytes_sent
    }
}

struct UploadStream {
    file: tokio::fs::File,
    remaining: u64,
    cancel: Cancel,
    reporter: Arc<UploadReporter>,
    first_chunk: bool,
    verification: Option<(sha2::Sha256, String)>,
}
impl futures_core::Stream for UploadStream {
    type Item = std::io::Result<Vec<u8>>;
    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
    ) -> Poll<Option<Self::Item>> {
        use tokio::io::AsyncRead;
        let this = self.get_mut();
        if this.cancel.load(Ordering::SeqCst) {
            return Poll::Ready(Some(Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "已取消",
            ))));
        }
        if this.remaining == 0 {
            return Poll::Ready(None);
        }
        // The request body is driven by transport backpressure. Never buffer a
        // whole attachment, and stop at its snapshotted Content-Length.
        let mut chunk = vec![0; this.remaining.min(64 * 1024) as usize];
        let mut buffer = tokio::io::ReadBuf::new(&mut chunk);
        match std::pin::Pin::new(&mut this.file).poll_read(cx, &mut buffer) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(error)) => Poll::Ready(Some(Err(error))),
            Poll::Ready(Ok(())) => {
                let count = buffer.filled().len();
                if count == 0 {
                    return Poll::Ready(Some(Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "备份期间附件已改变，请重试",
                    ))));
                }
                chunk.truncate(count);
                if let Some((hasher, expected)) = &mut this.verification {
                    use sha2::Digest;
                    hasher.update(&chunk);
                    if this.remaining == count as u64
                        && hex::encode(hasher.clone().finalize()) != *expected
                    {
                        return Poll::Ready(Some(Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "备份期间附件内容已改变，请重试",
                        ))));
                    }
                }
                this.remaining -= count as u64;
                this.reporter.update(this.first_chunk, |progress| {
                    progress.bytes_sent = progress
                        .bytes_sent
                        .saturating_add(count as u64)
                        .min(progress.bytes_total);
                });
                this.first_chunk = false;
                Poll::Ready(Some(Ok(chunk)))
            }
        }
    }
}
pub(crate) enum Payload {
    Bytes(Vec<u8>),
    File {
        path: PathBuf,
        length: u64,
        reporter: Arc<UploadReporter>,
    },
    FileRange {
        path: PathBuf,
        offset: u64,
        length: u64,
        sha256: String,
        reporter: Arc<UploadReporter>,
    },
}
pub struct Storage {
    config: StorageConfig,
    secret: String,
    token: String,
    fixture_endpoint: Option<SocketAddr>,
}
fn mac(key: &[u8], value: &str) -> Vec<u8> {
    use hmac::{Hmac, Mac};
    let mut h = Hmac::<sha2::Sha256>::new_from_slice(key).unwrap();
    h.update(value.as_bytes());
    h.finalize().into_bytes().to_vec()
}
fn digest(value: &str) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(value.as_bytes()))
}
fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
fn encode_path(value: &str) -> String {
    value.split('/').map(encode).collect::<Vec<_>>().join("/")
}
impl Storage {
    pub fn incremental_supported(&self) -> bool {
        self.config.provider == "s3"
    }
    pub fn load(lib: &Library) -> Result<Self> {
        let raw = lib.setting("storage_config")?;
        ensure!(
            !raw.trim().is_empty(),
            "尚未设置云备份。请在「设置 → 同步与备份」填写存储服务信息和访问密钥，然后点击「保存存储配置」。"
        );
        let c: StorageConfig = serde_json::from_str(&raw).map_err(|_| {
            anyhow::anyhow!(
                "云备份设置无法读取。请在「设置 → 同步与备份」重新填写，并点击「保存存储配置」。"
            )
        })?;
        let c = c.normalized();
        ensure!(
            [
                &c.provider,
                &c.endpoint,
                &c.region,
                &c.bucket,
                &c.access_key_id,
                &c.prefix
            ]
            .iter()
            .all(|value| !value.trim().is_empty()),
            "云备份设置尚未填完整。请在「设置 → 同步与备份」补全存储服务信息和访问密钥，然后点击「保存存储配置」。"
        );
        let secret = crate::credentials::get(lib, &format!("storage-{}", c.credential_id))?;
        let token = crate::credentials::get_optional(lib, &format!("token-{}", c.credential_id))?;
        let s = Self {
            config: c,
            secret,
            token,
            fixture_endpoint: None,
        };
        s.validate()?;
        if let Ok(normalized) = serde_json::to_string(&s.config)
            && normalized != raw
        {
            // Best-effort migration for legacy R2-style region values. Only
            // after the normalized configuration is known to be usable.
            let _ = lib.set_setting("storage_config", &normalized);
            let _ = sync_active_profile(lib);
        }
        Ok(s)
    }
    /// Persist a config with an existing or fresh credential ID.
    fn persist(
        lib: &Library,
        mut c: StorageConfig,
        secret: &str,
        token: &str,
    ) -> Result<StorageConfig> {
        c = c.normalized();
        let tmp = Self {
            config: c.clone(),
            secret: secret.into(),
            token: token.into(),
            fixture_endpoint: None,
        };
        tmp.validate()?;
        if c.credential_id.is_empty() {
            c.credential_id = id();
        }
        crate::credentials::store_config(
            lib,
            "storage_config",
            &serde_json::to_string(&c)?,
            &[
                (format!("storage-{}", c.credential_id), secret.into()),
                (format!("token-{}", c.credential_id), token.into()),
            ],
        )?;
        Ok(c)
    }

    fn active_credentials(lib: &Library, credential_id: &str) -> Result<(String, String)> {
        ensure!(!credential_id.is_empty(), "请填写 Secret Access Key");
        Ok((
            crate::credentials::get(lib, &format!("storage-{credential_id}"))?,
            crate::credentials::get_optional(lib, &format!("token-{credential_id}"))?,
        ))
    }

    /// Update the active profile. A blank secret keeps the stored credential so
    /// editing an endpoint or bucket does not require retyping the key.
    pub fn save(lib: &Library, mut c: StorageConfig, secret: &str, token: &str) -> Result<()> {
        let (secret, token) = if secret.trim().is_empty() {
            if c.credential_id.is_empty() {
                let current: StorageConfig =
                    serde_json::from_str(&lib.setting("storage_config")?).unwrap_or_default();
                c.credential_id = current.credential_id;
            }
            let (secret, stored_token) = Self::active_credentials(lib, &c.credential_id)?;
            let token = if token.trim().is_empty() {
                stored_token
            } else {
                token.to_owned()
            };
            (secret, token)
        } else {
            // Rotate the credential ID so a failed keychain write cannot delete
            // the old working secret before the new config is committed.
            c.credential_id.clear();
            (secret.to_owned(), token.to_owned())
        };
        Self::persist(lib, c, &secret, &token)?;
        sync_active_profile(lib)
    }

    /// Save the form as a new named profile and make it active. A blank secret
    /// copies the active credential so duplicating a working setup stays simple.
    pub fn save_profile(
        lib: &Library,
        name: &str,
        mut c: StorageConfig,
        secret: &str,
        token: &str,
    ) -> Result<StorageProfile> {
        let name = validate_profile_name(name)?;
        let (profiles, _) = profiles(lib)?;
        ensure!(
            !profiles
                .iter()
                .any(|profile| profile.name.eq_ignore_ascii_case(name)),
            "已有同名存储配置，请换一个名称"
        );
        let (secret, token) = if secret.trim().is_empty() {
            if c.credential_id.is_empty() {
                let current: StorageConfig =
                    serde_json::from_str(&lib.setting("storage_config")?).unwrap_or_default();
                c.credential_id = current.credential_id;
            }
            Self::active_credentials(lib, &c.credential_id)?
        } else {
            (secret.to_owned(), token.to_owned())
        };
        c.credential_id.clear();
        let config = Self::persist(lib, c, &secret, &token)?;
        let profile = StorageProfile {
            id: id(),
            name: name.to_owned(),
            config,
        };
        let mut saved = profiles;
        saved.push(profile.clone());
        lib.set_setting(PROFILES_KEY, &serde_json::to_string(&saved)?)?;
        lib.set_setting(ACTIVE_PROFILE_KEY, &profile.id)?;
        Ok(profile)
    }
    /// Local protocol fixture only: numeric loopback address and fixed fake credentials.
    /// Saved application configurations still require HTTPS and never use this transport.
    #[doc(hidden)]
    pub fn loopback_fixture(address: SocketAddr) -> Result<Self> {
        ensure!(
            address.ip().is_loopback() && address.port() != 0,
            "夹具只允许本机回环地址"
        );
        Ok(Self {
            config: StorageConfig {
                provider: "s3".into(),
                endpoint: "https://fixture.invalid".into(),
                region: "us-east-1".into(),
                bucket: "fixture-bucket".into(),
                access_key_id: "fixture-access".into(),
                prefix: "clarora".into(),
                path_style: true,
                ..Default::default()
            },
            secret: "fixture-secret".into(),
            token: String::new(),
            fixture_endpoint: Some(address),
        })
    }
    fn validate(&self) -> Result<()> {
        let c = &self.config;
        ensure!(
            ["s3", "oss"].contains(&c.provider.as_str()),
            "存储类型需要 s3 或 oss"
        );
        ensure!(
            regex::Regex::new(r"^https://[a-zA-Z0-9.-]+(:[0-9]+)?/?$")?.is_match(&c.endpoint),
            "Endpoint 需要不含路径的 HTTPS 域名"
        );
        ensure!(
            regex::Regex::new(r"^[a-z0-9][a-z0-9.-]{1,61}[a-z0-9]$")?.is_match(&c.bucket)
                && regex::Regex::new(r"^[a-z0-9-]+$")?.is_match(&c.region),
            "Bucket 或 Region 无效"
        );
        ensure!(
            c.provider != "oss" || c.region != "auto",
            "阿里云 OSS 的 Region 需要填写地域 ID，例如 cn-hangzhou，不能使用 auto；Endpoint 示例 https://oss-cn-hangzhou.aliyuncs.com，Bucket 单独填写"
        );
        ensure!(
            !c.prefix.is_empty()
                && !c
                    .prefix
                    .split('/')
                    .any(|x| x.is_empty() || x == ".." || x == ".")
                && !c.prefix.contains('\\'),
            "资料库前缀无效"
        );
        ensure!(
            !c.access_key_id.is_empty() && !self.secret.is_empty(),
            "访问密钥不能为空"
        );
        Ok(())
    }
    pub(crate) async fn response(
        &self,
        client: &reqwest::Client,
        method: &str,
        key: &str,
        query: &BTreeMap<String, String>,
        body: Option<Payload>,
        cancel: &Cancel,
    ) -> Result<reqwest::Response> {
        self.response_with_headers(
            client,
            method,
            key,
            query,
            body,
            &BTreeMap::new(),
            false,
            cancel,
        )
        .await
    }
    async fn response_with_headers(
        &self,
        client: &reqwest::Client,
        method: &str,
        key: &str,
        query: &BTreeMap<String, String>,
        body: Option<Payload>,
        extra: &BTreeMap<String, String>,
        raw_status: bool,
        cancel: &Cancel,
    ) -> Result<reqwest::Response> {
        check_cancel(cancel)?;
        let c = &self.config;
        let oss = c.provider == "oss";
        let host = c
            .endpoint
            .trim_start_matches("https://")
            .trim_end_matches('/');
        let virtual_host = oss || !c.path_style;
        let request_host = if let Some(address) = self.fixture_endpoint {
            address.to_string()
        } else if virtual_host {
            format!("{}.{}", c.bucket, host)
        } else {
            host.into()
        };
        let path = encode_path(&format!(
            "{}{key}",
            if virtual_host {
                "/".into()
            } else {
                format!("/{}/", c.bucket)
            }
        ));
        let canonical_path = if oss {
            encode_path(&format!("/{}/{key}", c.bucket))
        } else {
            path.clone()
        };
        let query = query
            .iter()
            .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
            .collect::<Vec<_>>()
            .join("&");
        let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
        let date = &timestamp[..8];
        let prefix = if oss { "x-oss" } else { "x-amz" };
        let mut headers = BTreeMap::new();
        headers.insert(format!("{prefix}-date"), timestamp.clone());
        headers.insert(
            format!("{prefix}-content-sha256"),
            "UNSIGNED-PAYLOAD".into(),
        );
        if !self.token.is_empty() {
            headers.insert(format!("{prefix}-security-token"), self.token.clone());
        }
        if body.is_some() {
            headers.insert("content-type".into(), "application/octet-stream".into());
        }
        headers.extend(extra.clone());
        let mut canonical_headers = headers.clone();
        if !oss {
            canonical_headers.insert("host".into(), request_host.clone());
        }
        let names = canonical_headers
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .join(";");
        let signed = if oss { "" } else { names.as_str() };
        let canonical = format!(
            "{method}\n{canonical_path}\n{query}\n{}\n{signed}\nUNSIGNED-PAYLOAD",
            canonical_headers
                .iter()
                .map(|(k, v)| format!("{k}:{}\n", v.trim()))
                .collect::<String>()
        );
        let service = if oss { "oss" } else { "s3" };
        let terminator = if oss {
            "aliyun_v4_request"
        } else {
            "aws4_request"
        };
        let algorithm = if oss {
            "OSS4-HMAC-SHA256"
        } else {
            "AWS4-HMAC-SHA256"
        };
        let scope = format!("{date}/{}/{service}/{terminator}", c.region);
        let initial = format!("{}{}", if oss { "aliyun_v4" } else { "AWS4" }, self.secret);
        let signing = mac(
            &mac(&mac(&mac(initial.as_bytes(), date), &c.region), service),
            terminator,
        );
        let signature = hex::encode(mac(
            &signing,
            &format!("{algorithm}\n{timestamp}\n{scope}\n{}", digest(&canonical)),
        ));
        headers.insert(
            "Authorization".into(),
            format!(
                "{algorithm} Credential={}/{scope}, {}Signature={signature}",
                c.access_key_id,
                if oss {
                    String::new()
                } else {
                    format!("SignedHeaders={signed}, ")
                }
            ),
        );
        let url = format!(
            "{}://{request_host}{path}{}",
            if self.fixture_endpoint.is_some() {
                "http"
            } else {
                "https"
            },
            if query.is_empty() {
                String::new()
            } else {
                format!("?{query}")
            }
        );
        let mut request = client.request(method.parse()?, url);
        for (k, v) in headers {
            request = request.header(k, v);
        }
        if oss {
            // Aliyun's SDK sends an unsigned HTTP Date alongside x-oss-date.
            request = request.header(
                "date",
                chrono::Utc::now()
                    .format("%a, %d %b %Y %H:%M:%S GMT")
                    .to_string(),
            );
        }
        if let Some(body) = body {
            request = match body {
                Payload::Bytes(bytes) => request.body(bytes),
                Payload::File {
                    path,
                    length,
                    reporter,
                } => {
                    let file = tokio::fs::File::open(path).await?;
                    ensure!(
                        file.metadata().await?.len() == length,
                        "备份期间附件已改变，请重试"
                    );
                    request
                        .header("content-length", length)
                        .body(reqwest::Body::wrap_stream(UploadStream {
                            file,
                            remaining: length,
                            cancel: cancel.clone(),
                            reporter,
                            first_chunk: true,
                            verification: None,
                        }))
                }
                Payload::FileRange {
                    path,
                    offset,
                    length,
                    sha256,
                    reporter,
                } => {
                    use sha2::Digest;
                    use tokio::io::AsyncSeekExt;
                    let mut file = tokio::fs::File::open(path).await?;
                    ensure!(
                        file.metadata().await?.len()
                            >= offset.checked_add(length).context("附件范围无效")?,
                        "备份期间附件已改变，请重试"
                    );
                    file.seek(std::io::SeekFrom::Start(offset)).await?;
                    request
                        .header("content-length", length)
                        .body(reqwest::Body::wrap_stream(UploadStream {
                            file,
                            remaining: length,
                            cancel: cancel.clone(),
                            reporter,
                            first_chunk: true,
                            verification: Some((sha2::Sha256::new(), sha256)),
                        }))
                }
            };
        }
        let mut response = cancellable(cancel, async {
            let result = request.send().await;
            if result.is_err() {
                check_cancel(cancel)?;
            }
            Ok(result?)
        })
        .await?;
        if !raw_status && !response.status().is_success() {
            let status = response.status();
            let bytes = cancellable(cancel, async {
                Ok(tokio::time::timeout(Duration::from_secs(3), async {
                    const LIMIT: usize = 16 * 1024;
                    let mut bytes = Vec::new();
                    while let Some(chunk) = response.chunk().await? {
                        let count = chunk.len().min(LIMIT - bytes.len());
                        bytes.extend_from_slice(&chunk[..count]);
                        if bytes.len() == LIMIT {
                            break;
                        }
                    }
                    Ok::<_, reqwest::Error>(bytes)
                })
                .await??)
            })
            .await;
            check_cancel(cancel)?;
            anyhow::bail!(self.error_message(status, &bytes.unwrap_or_default()));
        }
        Ok(response)
    }
    pub(crate) async fn request(
        &self,
        client: &reqwest::Client,
        method: &str,
        key: &str,
        query: &BTreeMap<String, String>,
        body: Option<Payload>,
        cancel: &Cancel,
    ) -> Result<Vec<u8>> {
        let mut response = self
            .response(client, method, key, query, body, cancel)
            .await?;
        cancellable(cancel, async {
            let mut bytes = vec![];
            while let Some(chunk) = response.chunk().await? {
                check_cancel(cancel)?;
                ensure!(
                    bytes.len() + chunk.len() <= 64 * 1024 * 1024,
                    "备份清单或列表过大"
                );
                bytes.extend_from_slice(&chunk);
            }
            Ok(bytes)
        })
        .await
    }
    pub(crate) fn client(&self) -> Result<reqwest::Client> {
        let mut builder = crate::network::client_builder(&self.config.endpoint)
            .connect_timeout(Duration::from_secs(20))
            .timeout(Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::none());
        if self.fixture_endpoint.is_some() {
            builder = builder.no_proxy();
        }
        Ok(builder.build()?)
    }
    fn error_message(&self, status: reqwest::StatusCode, bytes: &[u8]) -> String {
        let code = storage_error_code(bytes).filter(|code| {
            [&self.config.access_key_id, &self.secret, &self.token]
                .iter()
                .all(|value| value.is_empty() || !code.contains(value.as_str()))
        });
        storage_error_message(status, code.as_deref())
    }
    pub(crate) fn config(&self) -> &StorageConfig {
        &self.config
    }
}

#[derive(Clone)]
pub(crate) struct Listed {
    pub key: String,
    pub length: u64,
    pub modified: chrono::DateTime<chrono::Utc>,
}
pub(crate) struct Objects<'a> {
    storage: &'a Storage,
    cancel: &'a Cancel,
    client: reqwest::Client,
    runtime: tokio::runtime::Runtime,
}
impl<'a> Objects<'a> {
    pub fn new(storage: &'a Storage, cancel: &'a Cancel) -> Result<Self> {
        Ok(Self {
            storage,
            cancel,
            client: storage.client()?,
            runtime: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?,
        })
    }
    pub(crate) fn raw(
        &self,
        method: &str,
        key: &str,
        body: Option<Payload>,
        headers: &BTreeMap<String, String>,
    ) -> Result<reqwest::Response> {
        self.runtime.block_on(self.storage.response_with_headers(
            &self.client,
            method,
            key,
            &BTreeMap::new(),
            body,
            headers,
            true,
            self.cancel,
        ))
    }
    pub(crate) fn checked(&self, response: reqwest::Response) -> Result<reqwest::Response> {
        if response.status().is_success() {
            return Ok(response);
        }
        let status = response.status();
        let bytes = self.read(response).unwrap_or_default();
        check_cancel(self.cancel)?;
        anyhow::bail!(self.storage.error_message(status, &bytes))
    }
    fn read(&self, mut response: reqwest::Response) -> Result<Vec<u8>> {
        self.runtime.block_on(cancellable(self.cancel, async {
            let mut bytes = vec![];
            while let Some(chunk) = response.chunk().await? {
                check_cancel(self.cancel)?;
                ensure!(
                    bytes.len() + chunk.len() <= 64 * 1024 * 1024,
                    "云清单或分块过大"
                );
                bytes.extend_from_slice(&chunk);
            }
            Ok(bytes)
        }))
    }
    pub fn get(&self, key: &str) -> Result<Option<(Vec<u8>, Option<String>)>> {
        let response = self.raw("GET", key, None, &BTreeMap::new())?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let response = self.checked(response)?;
        let etag = response
            .headers()
            .get("etag")
            .and_then(|s| s.to_str().ok())
            .map(String::from);
        Ok(Some((self.read(response)?, etag)))
    }
    pub fn put(&self, key: &str, bytes: Vec<u8>, condition: Option<(&str, &str)>) -> Result<bool> {
        let headers = condition
            .map(|(k, v)| BTreeMap::from([(k.into(), v.into())]))
            .unwrap_or_default();
        let response = self.raw("PUT", key, Some(Payload::Bytes(bytes)), &headers)?;
        if response.status() == reqwest::StatusCode::PRECONDITION_FAILED
            || response.status() == reqwest::StatusCode::CONFLICT
        {
            return Ok(false);
        }
        self.checked(response)?;
        Ok(true)
    }
    pub fn delete(&self, key: &str, etag: Option<&str>) -> Result<()> {
        let headers = etag
            .map(|s| BTreeMap::from([("if-match".into(), s.into())]))
            .unwrap_or_default();
        let response = self.raw("DELETE", key, None, &headers)?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(());
        }
        self.checked(response)?;
        Ok(())
    }
    pub fn head(&self, key: &str) -> Result<Option<(u64, Option<String>, Option<String>)>> {
        let response = self.raw("HEAD", key, None, &BTreeMap::new())?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let response = self.checked(response)?;
        let len = response
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .context("存储服务没有返回对象长度")?;
        let field = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|s| s.to_str().ok())
                .map(String::from)
        };
        Ok(Some((len, field("etag"), field("x-amz-meta-sha256"))))
    }
    pub fn list_prefix(&self, prefix: &str) -> Result<Vec<Listed>> {
        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct Listing {
            #[serde(default)]
            contents: Vec<Entry>,
            #[serde(default)]
            is_truncated: bool,
            next_continuation_token: Option<String>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct Entry {
            key: String,
            size: u64,
            last_modified: String,
        }
        let mut result = vec![];
        let mut token = String::new();
        let mut seen = BTreeSet::new();
        loop {
            let mut query = BTreeMap::from([
                ("list-type".into(), "2".into()),
                ("prefix".into(), prefix.into()),
            ]);
            if !token.is_empty() {
                query.insert("continuation-token".into(), token);
            }
            let bytes = self.runtime.block_on(self.storage.request(
                &self.client,
                "GET",
                "",
                &query,
                None,
                self.cancel,
            ))?;
            let xml = std::str::from_utf8(&bytes)?;
            ensure!(
                !xml.contains("<!DOCTYPE") && !xml.contains("<!ENTITY"),
                "不支持的存储列表"
            );
            let list: Listing = quick_xml::de::from_str(xml)?;
            for entry in list.contents {
                ensure!(entry.key.starts_with(prefix), "存储列表越过前缀");
                result.push(Listed {
                    key: entry.key,
                    length: entry.size,
                    modified: chrono::DateTime::parse_from_rfc3339(&entry.last_modified)?
                        .with_timezone(&chrono::Utc),
                });
            }
            ensure!(result.len() <= 200000, "云对象过多，请缩小资料库前缀");
            if !list.is_truncated {
                break;
            }
            token = list.next_continuation_token.context("存储分页缺少令牌")?;
            ensure!(
                !token.is_empty() && seen.insert(token.clone()),
                "存储分页循环"
            );
        }
        Ok(result)
    }
}
