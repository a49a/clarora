//! Optional OS credential storage. Ordinary mode reads/writes only the local SQLite database.
use crate::data::Library;
use anyhow::{Result, ensure};
use rusqlite::{OptionalExtension, params};
use std::{
    collections::BTreeMap,
    sync::{Mutex, OnceLock},
};
const MODE: &str = "credential_secure_mode";
fn cache() -> &'static Mutex<BTreeMap<String, String>> {
    static CACHE: OnceLock<Mutex<BTreeMap<String, String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
}
pub fn secure_mode(lib: &Library) -> Result<bool> {
    Ok(lib.setting(MODE)? == "true")
}
fn keychain(name: &str, cache: &mut BTreeMap<String, String>) -> Result<Option<String>> {
    if let Some(v) = cache.get(name) {
        return Ok(Some(v.clone()));
    }
    match keyring::Entry::new("Clarora-GPUI", name)?.get_password() {
        Ok(v) => {
            cache.insert(name.into(), v.clone());
            Ok(Some(v))
        }
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}
fn local(lib: &Library, name: &str) -> Result<Option<String>> {
    Ok(lib
        .db()?
        .query_row(
            "SELECT value FROM local_credentials WHERE name=?",
            [name],
            |r| r.get(0),
        )
        .optional()?)
}
pub fn get(lib: &Library, name: &str) -> Result<String> {
    let mut cache = cache().lock().unwrap();
    if secure_mode(lib)? {
        return Ok(keychain(name, &mut cache)?.unwrap_or_default());
    }
    local(lib, name)?.ok_or_else(|| {
        anyhow::anyhow!("本机尚未保存密钥。请在设置中重新输入并保存，或主动导入旧钥匙串密钥")
    })
}
pub fn get_optional(lib: &Library, name: &str) -> Result<String> {
    let mut cache = cache().lock().unwrap();
    if secure_mode(lib)? {
        return Ok(keychain(name, &mut cache)?.unwrap_or_default());
    }
    Ok(local(lib, name)?.unwrap_or_default())
}
/// Store new credential names and their config reference atomically in ordinary mode.
pub fn store_config(
    lib: &Library,
    setting: &str,
    json: &str,
    entries: &[(String, String)],
) -> Result<()> {
    let mut cache = cache().lock().unwrap();
    if secure_mode(lib)? {
        let result = (|| -> Result<()> {
            for (name, value) in entries {
                keyring::Entry::new("Clarora-GPUI", name)?.set_password(value)?;
            }
            lib.set_setting(setting, json)
        })();
        if result.is_err() {
            for (name, _) in entries {
                if let Ok(entry) = keyring::Entry::new("Clarora-GPUI", name) {
                    let _ = entry.delete_credential();
                }
            }
        } else {
            for (name, value) in entries {
                cache.insert(name.clone(), value.clone());
            }
        }
        return result;
    }
    let mut db = lib.db()?;
    let tx = db.transaction()?;
    for (name, value) in entries {
        tx.execute("INSERT INTO local_credentials(name,value)VALUES(?,?) ON CONFLICT(name) DO UPDATE SET value=excluded.value",params![name,value])?;
    }
    tx.execute("INSERT INTO app_settings(key,value)VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![setting,json])?;
    tx.commit()?;
    Ok(())
}
pub fn set_secure_mode(lib: &Library, enabled: bool) -> Result<()> {
    let mut cache = cache().lock().unwrap();
    if enabled == secure_mode(lib)? {
        return Ok(());
    }
    if !enabled {
        return lib.set_setting(MODE, "false");
    }
    let entries: Vec<(String, String)> = {
        let db = lib.db()?;
        let mut q = db.prepare("SELECT name,value FROM local_credentials")?;
        q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?
    };
    // Preserve SQLite and the old mode if any OS write fails. Partial copies are harmless.
    for (name, value) in &entries {
        keyring::Entry::new("Clarora-GPUI", name)?.set_password(value)?;
        cache.insert(name.clone(), value.clone());
    }
    let mut db = lib.db()?;
    let tx = db.transaction()?;
    tx.execute("DELETE FROM local_credentials", [])?;
    tx.execute("INSERT INTO app_settings(key,value)VALUES(?,'true') ON CONFLICT(key) DO UPDATE SET value='true'",[MODE])?;
    tx.commit()?;
    Ok(())
}
fn referenced(lib: &Library) -> Result<Vec<String>> {
    let mut names = vec![];
    let raw = lib.setting("ai_config")?;
    if !raw.is_empty() {
        let c: serde_json::Value = serde_json::from_str(&raw)?;
        if let Some(id) = c["credential_id"].as_str().filter(|s| !s.is_empty()) {
            let roles: &[&str] = match c["shared_key_source"].as_str() {
                Some("chat") => &["chat"],
                Some("asr") => &["asr"],
                _ => &["chat", "asr"],
            };
            for role in roles {
                names.push(format!("{role}-{id}"));
            }
        }
    }
    let raw = lib.setting("storage_config")?;
    if !raw.is_empty() {
        let c: serde_json::Value = serde_json::from_str(&raw)?;
        if let Some(id) = c["credential_id"].as_str().filter(|s| !s.is_empty()) {
            names.push(format!("storage-{id}"));
            names.push(format!("token-{id}"));
        }
    }
    Ok(names)
}
pub fn needs_legacy_import(lib: &Library) -> Result<bool> {
    if secure_mode(lib)? {
        return Ok(false);
    }
    for name in referenced(lib)? {
        if !name.starts_with("token-") && local(lib, &name)?.is_none() {
            return Ok(true);
        }
    }
    Ok(false)
}
/// Explicit user action only: never called by startup, playback, or ordinary credential reads.
pub fn import_legacy(lib: &Library) -> Result<usize> {
    let mut cache = cache().lock().unwrap();
    ensure!(!secure_mode(lib)?, "请先关闭密钥安全模式");
    let mut entries = vec![];
    let mut found = 0;
    for name in referenced(lib)? {
        if local(lib, &name)?.is_some() {
            continue;
        }
        let value = keychain(&name, &mut cache)?;
        if value.is_some() {
            found += 1;
        }
        entries.push((name, value.unwrap_or_default()));
    }
    ensure!(
        found > 0,
        "没有找到需要导入的旧钥匙串密钥，可重新输入密钥保存"
    );
    let mut db = lib.db()?;
    let tx = db.transaction()?;
    for (name, value) in entries {
        tx.execute(
            "INSERT INTO local_credentials(name,value)VALUES(?,?) ON CONFLICT(name) DO NOTHING",
            params![name, value],
        )?;
    }
    tx.commit()?;
    Ok(found)
}
