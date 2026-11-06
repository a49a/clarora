use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use ring::{
    aead,
    rand::{SecureRandom, SystemRandom},
};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::atomic::Ordering,
};

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
        p.connected &= lib.root.join(format!("chatgpt/{}.session", p.id)).is_file();
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
    let name = format!(
        "vault-{}",
        hex::encode(Sha256::digest(lib.root.to_string_lossy().as_bytes()))
    );
    let entry = keyring::Entry::new("Clarora-ChatGPT", &name)?;
    let encoded = match entry.get_password() {
        Ok(v) => v,
        Err(keyring::Error::NoEntry) if create => {
            let mut key = [0; 32];
            SystemRandom::new()
                .fill(&mut key)
                .map_err(|_| anyhow::anyhow!("生成本机加密密钥失败"))?;
            let v = B64.encode(key);
            entry
                .set_password(&v)
                .context("无法保存到系统凭证库，请允许 Clarora 访问凭证库")?;
            v
        }
        Err(keyring::Error::NoEntry) => {
            anyhow::bail!("系统凭证库中没有 ChatGPT 登录密钥，请重新登录")
        }
        Err(_) => anyhow::bail!("无法读取系统凭证库，请允许 Clarora 访问凭证库"),
    };
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
    let mut blob = nonce.to_vec();
    blob.extend_from_slice(&bytes);
    atomic(lib, &format!("{}.session", p.id), &blob)
}
pub(super) fn session(lib: &Library, p: &Profile) -> Result<Option<Session>> {
    let file = lib.root.join(format!("chatgpt/{}.session", p.id));
    if !file.exists() {
        return Ok(None);
    }
    let mut bytes = fs::read(file)?;
    ensure!(
        bytes.len() >= 28 && bytes.len() <= 128 * 1024,
        "ChatGPT 登录凭证损坏，请重新登录"
    );
    let nonce: [u8; 12] = bytes[..12].try_into()?;
    let key = cipher(lib, false)?;
    let plain = key
        .open_in_place(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(p.id.as_bytes()),
            &mut bytes[12..],
        )
        .map_err(|_| anyhow::anyhow!("ChatGPT 登录凭证无法解密，请重新登录"))?;
    Ok(Some(
        serde_json::from_slice(plain).context("ChatGPT 登录凭证损坏")?,
    ))
}
pub(super) fn clear_session(lib: &Library, p: &Profile) -> Result<()> {
    let file = lib.root.join(format!("chatgpt/{}.session", p.id));
    if file.exists() {
        fs::remove_file(file).context("清除 ChatGPT 本机登录失败")?;
    }
    Ok(())
}
