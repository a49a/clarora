use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use ring::{
    aead,
    rand::{SecureRandom, SystemRandom},
};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::Ordering,
};
const FORMAT: &[u8] = b"CLARORA-CGPT-VAULT-1\n";
const VAULT_REF: &str = "chatgpt_vault_ref";

fn managed_session(path: &Path) -> Result<bool> {
    let mut header = [0; FORMAT.len()];
    match File::open(path)?.read_exact(&mut header) {
        Ok(()) => Ok(header == FORMAT),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(e) => Err(e.into()),
    }
}

fn directory(lib: &Library) -> Result<PathBuf> {
    let p = lib.root.join("chatgpt");
    fs::create_dir_all(&p)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&p, fs::Permissions::from_mode(0o700))?;
    }
    Ok(p)
}
fn atomic(lib: &Library, name: &str, bytes: &[u8]) -> Result<()> {
    let dir = directory(lib)?;
    let mut f = tempfile::NamedTempFile::new_in(&dir)?;
    f.write_all(bytes)?;
    f.as_file().sync_all()?;
    f.persist(dir.join(name))
        .map_err(|_| anyhow::anyhow!("保存 ChatGPT 本机配置失败"))?;
    Ok(())
}
pub(super) fn load(lib: &Library) -> Result<Snapshot> {
    let path = lib.root.join("chatgpt/state.json");
    if !path.exists() {
        return Ok(Snapshot::default());
    }
    let bytes = fs::read(path)?;
    ensure!(bytes.len() < 1024 * 1024, "ChatGPT 本机配置过大");
    let mut state: Snapshot = serde_json::from_slice(&bytes).context("ChatGPT 本机配置损坏")?;
    ensure!(state.profiles.len() <= 100, "ChatGPT 账户过多");
    for p in &mut state.profiles {
        uuid::Uuid::parse_str(&p.id).context("ChatGPT 账户标识无效")?;
        let file = lib.root.join(format!("chatgpt/{}.session", p.id));
        let exists = file.is_file();
        let legacy = exists && !managed_session(&file)?;
        state.needs_legacy_import |= legacy;
        p.connected &= exists && !legacy;
    }
    Ok(state)
}
pub(super) fn save(lib: &Library, state: &Snapshot) -> Result<()> {
    atomic(lib, "state.json", &serde_json::to_vec(state)?)
}
fn lock_file(lib: &Library) -> Result<File> {
    let dir = directory(lib)?;
    let mut opts = OpenOptions::new();
    opts.create(true).read(true).write(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    Ok(opts.open(dir.join("session.lock"))?)
}
pub(super) async fn lock(lib: &Library, cancel: &Cancel) -> Result<File> {
    let file = lock_file(lib)?;
    let started = std::time::Instant::now();
    loop {
        ensure!(!cancel.load(Ordering::SeqCst), "已取消");
        match fs2::FileExt::try_lock_exclusive(&file) {
            Ok(()) => return Ok(file),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                ensure!(
                    started.elapsed() < Duration::from_secs(190),
                    "ChatGPT 登录状态正在被其他窗口使用，请稍后再试"
                );
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Err(e) => return Err(e.into()),
        }
    }
}
pub(super) fn lock_blocking(lib: &Library) -> Result<File> {
    let f = lock_file(lib)?;
    fs2::FileExt::try_lock_exclusive(&f).context("ChatGPT 登录状态正在更新，请稍后再试")?;
    Ok(f)
}
fn cipher(lib: &Library, create: bool) -> Result<aead::LessSafeKey> {
    let saved = lib.setting(VAULT_REF)?;
    let name = if saved.is_empty() {
        format!(
            "chatgpt-vault-{}",
            hex::encode(Sha256::digest(lib.root.to_string_lossy().as_bytes()))
        )
    } else {
        saved.clone()
    };
    let mut encoded = crate::credentials::get_optional(lib, &name)?;
    if encoded.is_empty() {
        // A reference without a readable key can follow an explicit opt-out from
        // keychain storage. Never generate a replacement and strand other sessions.
        ensure!(
            saved.is_empty() && create,
            "本机没有可用的 ChatGPT 加密密钥。请在设置中主动导入旧钥匙串密钥；默认不会访问钥匙串"
        );
        let mut key = [0; 32];
        SystemRandom::new()
            .fill(&mut key)
            .map_err(|_| anyhow::anyhow!("生成本机加密密钥失败"))?;
        encoded = B64.encode(key);
        crate::credentials::store_config(
            lib,
            VAULT_REF,
            &name,
            &[(name.clone(), encoded.clone())],
        )?;
    }
    decode_key(&encoded)
}
// Called exclusively by the explicit import command, never by normal reads.
fn legacy_cipher(lib: &Library) -> Result<aead::LessSafeKey> {
    let name = format!(
        "vault-{}",
        hex::encode(Sha256::digest(lib.root.to_string_lossy().as_bytes()))
    );
    let entry = keyring::Entry::new("Clarora-ChatGPT", &name)?;
    let encoded = match entry.get_password() {
        Ok(v) => v,
        Err(keyring::Error::NoEntry) => {
            anyhow::bail!("系统凭证库中没有 ChatGPT 登录密钥，请重新登录")
        }
        Err(_) => anyhow::bail!("无法读取系统凭证库，请允许 Clarora 访问凭证库"),
    };
    decode_key(&encoded)
}
fn decode_key(encoded: &str) -> Result<aead::LessSafeKey> {
    let key = B64.decode(encoded).context("本机加密密钥无效")?;
    let key = aead::UnboundKey::new(&aead::AES_256_GCM, &key)
        .map_err(|_| anyhow::anyhow!("本机加密密钥无效"))?;
    Ok(aead::LessSafeKey::new(key))
}
pub(super) fn save_session(lib: &Library, p: &Profile, s: &Session) -> Result<()> {
    let key = cipher(lib, true)?;
    let mut nonce = [0; 12];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| anyhow::anyhow!("生成加密随机数失败"))?;
    let mut bytes = serde_json::to_vec(s)?;
    key.seal_in_place_append_tag(
        aead::Nonce::assume_unique_for_key(nonce),
        aead::Aad::from(p.id.as_bytes()),
        &mut bytes,
    )
    .map_err(|_| anyhow::anyhow!("加密 ChatGPT 登录失败"))?;
    let mut blob = FORMAT.to_vec();
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&bytes);
    atomic(lib, &format!("{}.session", p.id), &blob)
}
pub(super) fn session(lib: &Library, p: &Profile) -> Result<Option<Session>> {
    let file = lib.root.join(format!("chatgpt/{}.session", p.id));
    if !file.exists() {
        return Ok(None);
    }
    let bytes = read_session(&file)?;
    ensure!(
        bytes.starts_with(FORMAT),
        "旧 ChatGPT 登录需要主动导入钥匙串，或重新登录；默认不会访问钥匙串"
    );
    let key = cipher(lib, false)?;
    Ok(Some(decrypt(&bytes[FORMAT.len()..], p, &key)?))
}
fn read_session(file: &Path) -> Result<Vec<u8>> {
    ensure!(
        fs::metadata(file)?.len() <= 128 * 1024,
        "ChatGPT 登录凭证过大"
    );
    Ok(fs::read(file)?)
}
fn decrypt(bytes: &[u8], p: &Profile, key: &aead::LessSafeKey) -> Result<Session> {
    ensure!(
        bytes.len() >= 28 && bytes.len() <= 128 * 1024,
        "ChatGPT 登录凭证损坏，请重新登录"
    );
    let nonce: [u8; 12] = bytes[..12].try_into()?;
    let mut bytes = bytes.to_vec();
    let plain = key
        .open_in_place(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(p.id.as_bytes()),
            &mut bytes[12..],
        )
        .map_err(|_| anyhow::anyhow!("ChatGPT 登录凭证无法解密，请重新登录"))?;
    serde_json::from_slice(plain).context("ChatGPT 登录凭证损坏")
}
pub(super) fn import_legacy(lib: &Library) -> Result<Snapshot> {
    let _lock = lock_blocking(lib)?;
    let mut state = load(lib)?;
    ensure!(state.needs_legacy_import, "没有需要导入的旧 ChatGPT 登录");
    let key = legacy_cipher(lib)?;
    let mut sessions = vec![];
    // Validate every old session before replacing any. A refused authorization,
    // corrupt ciphertext or mismatched account leaves the old sessions untouched.
    for p in &state.profiles {
        let file = lib.root.join(format!("chatgpt/{}.session", p.id));
        if file.is_file() && !managed_session(&file)? {
            let session = decrypt(&read_session(&file)?, p, &key)?;
            ensure!(
                session.client_id == p.client_id && session.subject == p.subject,
                "旧 ChatGPT 登录与账户不匹配，请重新登录"
            );
            sessions.push((p.clone(), session));
        }
    }
    for (p, session) in &sessions {
        save_session(lib, p, session)?;
        if let Some(saved) = state.profiles.iter_mut().find(|v| v.id == p.id) {
            saved.connected = true;
            saved.sharing = session.scopes.iter().any(|s| s == DIRECT);
        }
    }
    save(lib, &state)?;
    load(lib)
}
pub(super) fn clear_session(lib: &Library, p: &Profile) -> Result<()> {
    let file = lib.root.join(format!("chatgpt/{}.session", p.id));
    if file.exists() {
        fs::remove_file(file).context("清除 ChatGPT 本机登录失败")?;
    }
    Ok(())
}
