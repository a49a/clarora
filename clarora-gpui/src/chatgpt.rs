//! Official Sign in with ChatGPT for the open-source local client.
//! Tokens stay in an encrypted local vault; only public snapshots reach GPUI.
mod login_error;
mod oauth;
mod responses;
mod storage;

use crate::{ai::ChatReply, data::Library, process::Cancel};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{future::Future, time::Duration};

const ISSUER: &str = "https://auth.openai.com";
const RESOURCE: &str = "https://api.openai.com/v1";
const DIRECT: &str = "chatgpt.tokens.use.direct";
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Model {
    pub slug: String,
    pub display_name: String,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub id: String,
    pub client_id: String,
    pub subject: String,
    pub email: String,
    pub connected: bool,
    pub sharing: bool,
    pub model: String,
    pub models: Vec<Model>,
}
impl Profile {
    pub fn label(&self) -> String {
        let email = if self.email.is_empty() {
            "ChatGPT".into()
        } else if self.email.chars().count() > 20 {
            format!("{}…", self.email.chars().take(20).collect::<String>())
        } else {
            self.email.clone()
        };
        format!("{} · {}", email, self.id.get(..6).unwrap_or(&self.id))
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Snapshot {
    pub host_id: String,
    pub enabled: bool,
    pub active: String,
    pub profiles: Vec<Profile>,
    #[serde(skip)]
    pub needs_legacy_import: bool,
}
impl Snapshot {
    pub fn current(&self) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.id == self.active)
    }
    pub fn login_profile(&self) -> Option<&Profile> {
        self.current().or_else(|| {
            self.profiles
                .iter()
                .rev()
                .find(|p| !p.connected && p.subject.is_empty() && !p.client_id.is_empty())
        })
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Session {
    client_id: String,
    subject: String,
    access_token: String,
    refresh_token: String,
    id_token: String,
    scopes: Vec<String>,
    expires_at: i64,
}
#[derive(Clone)]
pub struct Client {
    lib: Library,
    auth: String,
    api: String,
}
pub fn snapshot(lib: &Library) -> Result<Snapshot> {
    storage::load(lib)
}
pub fn enabled(lib: &Library) -> Result<bool> {
    Ok(snapshot(lib)?.enabled)
}
/// Explicit user action: legacy keychain access must never be a session-read fallback.
pub fn import_legacy(lib: &Library) -> Result<Snapshot> {
    storage::import_legacy(lib)
}
pub(crate) fn storage_mode_guard(lib: &Library) -> Result<std::fs::File> {
    storage::lock_blocking(lib)
}
pub fn set_enabled(lib: &Library, enabled: bool) -> Result<Snapshot> {
    let _lock = storage::lock_blocking(lib)?;
    let mut state = snapshot(lib)?;
    if enabled {
        let p = state.current().context("请先登录 ChatGPT")?;
        ensure!(
            p.connected && p.sharing && !p.model.is_empty(),
            "请先登录、授权会员使用并选择可用模型"
        );
    }
    state.enabled = enabled;
    storage::save(lib, &state)?;
    Ok(state)
}
fn run<T>(work: impl Future<Output = Result<T>>) -> Result<T> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(work)
}
fn http(client: &Client) -> Result<reqwest::Client> {
    Ok(crate::network::client_builder(&client.auth)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(180))
        .user_agent(concat!("Clarora/", env!("CARGO_PKG_VERSION")))
        .build()?)
}
impl Client {
    pub fn new(lib: &Library) -> Self {
        #[allow(unused_mut)]
        let mut client = Self {
            lib: lib.clone(),
            auth: ISSUER.into(),
            api: RESOURCE.into(),
        };
        #[cfg(debug_assertions)]
        diagnostic::apply(&mut client);
        client
    }
    pub fn sign_in(
        &self,
        profile: Option<&str>,
        cancel: &Cancel,
        open: impl FnMut(&str) -> Result<()>,
    ) -> Result<Snapshot> {
        run(oauth::sign_in(self, profile, cancel, open))
    }
    pub fn refresh_models(&self, cancel: &Cancel) -> Result<Snapshot> {
        run(responses::models(self, cancel))
    }
    pub fn select_profile(&self, id: &str) -> Result<Snapshot> {
        let _lock = storage::lock_blocking(&self.lib)?;
        let mut state = snapshot(&self.lib)?;
        state
            .profiles
            .iter()
            .find(|p| p.id == id)
            .context("ChatGPT 账户不存在")?;
        // Signed-out registrations remain selectable for reauthorization.
        state.active = id.into();
        storage::save(&self.lib, &state)?;
        Ok(state)
    }
    pub fn select_model(&self, model: &str) -> Result<Snapshot> {
        let _lock = storage::lock_blocking(&self.lib)?;
        let mut state = snapshot(&self.lib)?;
        let id = state.active.clone();
        let p = state
            .profiles
            .iter_mut()
            .find(|p| p.id == id)
            .context("请先登录 ChatGPT")?;
        ensure!(
            p.models.iter().any(|m| m.slug == model),
            "请选择当前账户提供的模型"
        );
        p.model = model.into();
        storage::save(&self.lib, &state)?;
        Ok(state)
    }
    pub fn sign_out(&self, cancel: &Cancel) -> Result<(Snapshot, bool)> {
        run(oauth::sign_out(self, cancel))
    }
    pub(crate) fn stream(
        &self,
        messages: Vec<Value>,
        cancel: &Cancel,
        progress: impl FnMut(ChatReply),
    ) -> Result<ChatReply> {
        run(responses::stream(self, messages, cancel, progress))
    }
    async fn access(&self, cancel: &Cancel) -> Result<(Profile, String)> {
        let _lock = storage::lock(&self.lib, cancel).await?;
        let mut state = snapshot(&self.lib)?;
        let mut p = state
            .current()
            .cloned()
            .context("请先在设置中登录 ChatGPT")?;
        ensure!(
            p.connected,
            if state.needs_legacy_import {
                "旧 ChatGPT 登录需要主动导入钥匙串，或重新登录；默认不会访问钥匙串"
            } else {
                "ChatGPT 已退出，请重新登录"
            }
        );
        let mut session =
            storage::session(&self.lib, &p)?.context("ChatGPT 登录已失效，请重新登录")?;
        ensure!(
            session.client_id == p.client_id && session.subject == p.subject,
            "ChatGPT 账户凭证不匹配，请重新登录"
        );
        if session.expires_at <= chrono::Utc::now().timestamp() + 60 {
            // Refresh tokens rotate: once renewal starts, finish and persist it even
            // if the caller cancels. Never leave a consumed refresh token on disk.
            let renewal = crate::process::cancel_token();
            let refreshed = tokio::time::timeout(
                Duration::from_secs(30),
                oauth::refresh(self, &session, &renewal),
            )
            .await
            .context("ChatGPT 登录续期超时，请重试或重新登录")?;
            match refreshed {
                Ok(next) => {
                    session = next;
                    p.sharing = session.scopes.iter().any(|s| s == DIRECT);
                    storage::save_session(&self.lib, &p, &session)?;
                    if let Some(saved) = state.profiles.iter_mut().find(|v| v.id == p.id) {
                        *saved = p.clone();
                    }
                    storage::save(&self.lib, &state)?;
                }
                Err(error) => {
                    if error.downcast_ref::<oauth::Expired>().is_some() {
                        storage::clear_session(&self.lib, &p)?;
                        if let Some(saved) = state.profiles.iter_mut().find(|v| v.id == p.id) {
                            saved.connected = false;
                        }
                        storage::save(&self.lib, &state)?;
                    }
                    return Err(error);
                }
            }
        }
        ensure!(!cancel.load(std::sync::atomic::Ordering::SeqCst), "已取消");
        ensure!(
            p.sharing && session.scopes.iter().any(|s| s == DIRECT),
            "尚未授权 ChatGPT 会员使用，请点击重新授权并同意会员使用权限"
        );
        Ok((p, session.access_token))
    }
}

/// Only isolated numeric-loopback fixtures can override production endpoints, in debug builds.
#[cfg(debug_assertions)]
pub mod diagnostic {
    use super::*;
    use std::{
        collections::BTreeMap,
        sync::{Mutex, OnceLock},
    };
    fn fixtures() -> &'static Mutex<BTreeMap<std::path::PathBuf, String>> {
        static V: OnceLock<Mutex<BTreeMap<std::path::PathBuf, String>>> = OnceLock::new();
        V.get_or_init(Default::default)
    }
    pub fn configure(lib: &Library, base: &str) -> Result<()> {
        let url = url::Url::parse(base)?;
        ensure!(
            url.scheme() == "http"
                && url.host_str() == Some("127.0.0.1")
                && url.port().is_some()
                && url.path() == "/",
            "fixture must be numeric loopback"
        );
        fixtures()
            .lock()
            .unwrap()
            .insert(lib.root.clone(), base.trim_end_matches('/').into());
        Ok(())
    }
    pub(super) fn apply(client: &mut Client) {
        if let Some(base) = fixtures().lock().unwrap().get(&client.lib.root) {
            client.auth = base.clone();
            client.api = format!("{base}/v1");
        }
    }
    pub fn expire(lib: &Library) -> Result<()> {
        ensure!(
            fixtures().lock().unwrap().contains_key(&lib.root),
            "not a fixture"
        );
        let _lock = storage::lock_blocking(lib)?;
        let state = snapshot(lib)?;
        let p = state.current().context("profile")?;
        let mut s = storage::session(lib, p)?.context("session")?;
        s.expires_at = 0;
        storage::save_session(lib, p, &s)
    }
}
