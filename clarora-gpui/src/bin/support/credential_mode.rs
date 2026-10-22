//! SQLite, export and credential-provider integration; never touches the user's keychain.
use anyhow::{Result, ensure};
use clarora_gpui::{ai, backup, credentials, data::Library};
use keyring::credential::{Credential, CredentialApi, CredentialBuilderApi};
use serde_json::{Value, json};
use std::{
    any::Any,
    collections::BTreeMap,
    fs,
    io::Read,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
#[derive(Clone, Default)]
struct Provider {
    values: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    calls: Arc<AtomicUsize>,
    fail: Arc<AtomicBool>,
}
struct Entry {
    provider: Provider,
    name: String,
}
impl CredentialApi for Entry {
    fn set_secret(&self, v: &[u8]) -> keyring::Result<()> {
        self.provider.calls.fetch_add(1, Ordering::SeqCst);
        if self.provider.fail.load(Ordering::SeqCst) {
            return Err(keyring::Error::NoStorageAccess(Box::new(
                std::io::Error::new(std::io::ErrorKind::PermissionDenied, "fixture denied"),
            )));
        };
        self.provider
            .values
            .lock()
            .unwrap()
            .insert(self.name.clone(), v.to_vec());
        Ok(())
    }
    fn get_secret(&self) -> keyring::Result<Vec<u8>> {
        self.provider.calls.fetch_add(1, Ordering::SeqCst);
        self.provider
            .values
            .lock()
            .unwrap()
            .get(&self.name)
            .cloned()
            .ok_or(keyring::Error::NoEntry)
    }
    fn delete_credential(&self) -> keyring::Result<()> {
        self.provider.values.lock().unwrap().remove(&self.name);
        Ok(())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
impl CredentialBuilderApi for Provider {
    fn build(
        &self,
        _: Option<&str>,
        service: &str,
        user: &str,
    ) -> keyring::Result<Box<Credential>> {
        ensure_service(service)?;
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(Entry {
            provider: self.clone(),
            name: user.into(),
        }))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
fn ensure_service(service: &str) -> keyring::Result<()> {
    if service == "Clarora-GPUI" {
        Ok(())
    } else {
        Err(keyring::Error::NoEntry)
    }
}
pub fn run(lib: &Library, results: &mut Vec<Value>) -> Result<()> {
    let lib = Library::open(lib.root.join("credential-mode"))?;
    let provider = Provider::default();
    keyring::set_default_credential_builder(Box::new(provider.clone()));
    ensure!(!credentials::secure_mode(&lib)?, "安全模式默认开启");
    let cfg = ai::Config {
        base_url: "http://127.0.0.1:1".into(),
        model: "fixture".into(),
        ..Default::default()
    };
    ai::save_shared(&lib, cfg.clone(), "sqlite-fixture-key")?;
    ensure!(
        ai::load_chat(&lib)?.1 == "sqlite-fixture-key",
        "默认密钥未存SQLite"
    );
    let reopened = Library::open(lib.root.clone())?;
    ensure!(
        ai::load_chat(&reopened)?.1 == "sqlite-fixture-key",
        "SQLite重开丢密钥"
    );
    let mut draft = ai::load_config(&lib)?;
    draft.local_model = "tiny.en".into();
    ai::save_preferences(&lib, draft, "")?;
    ensure!(
        ai::load_chat(&lib)?.1 == "sqlite-fixture-key",
        "空值保存清空SQLite密钥"
    );
    let original_id = ai::load_config(&lib)?.credential_id;
    let original_rows: i64 =
        lib.db()?
            .query_row("SELECT count(*) FROM local_credentials", [], |r| r.get(0))?;
    lib.db()?.execute_batch("CREATE TRIGGER fixture_fail_config BEFORE INSERT ON app_settings WHEN NEW.key='ai_config' BEGIN SELECT RAISE(ABORT,'fixture write failure'); END;")?;
    ensure!(
        ai::save_shared(&lib, cfg.clone(), "failed-replacement").is_err()
            && ai::load_config(&lib)?.credential_id == original_id,
        "SQLite保存失败替换了配置"
    );
    let after_rows: i64 =
        lib.db()?
            .query_row("SELECT count(*) FROM local_credentials", [], |r| r.get(0))?;
    ensure!(
        after_rows == original_rows && ai::load_chat(&lib)?.1 == "sqlite-fixture-key",
        "SQLite保存失败残留凭证或丢原值"
    );
    lib.db()?
        .execute_batch("DROP TRIGGER fixture_fail_config;")?;
    let storage = backup::StorageConfig {
        provider: "s3".into(),
        endpoint: "https://storage.example.invalid".into(),
        region: "us-east-1".into(),
        bucket: "fixture-bucket".into(),
        access_key_id: "fixture-access".into(),
        prefix: "clarora".into(),
        path_style: true,
        ..Default::default()
    };
    backup::Storage::save(&lib, storage, "sqlite-storage-key", "")?;
    backup::Storage::load(&lib)?;
    ensure!(
        provider.calls.load(Ordering::SeqCst) == 0,
        "SQLite模式仍访问钥匙串"
    );
    let zip = lib.root.join("without-secrets.zip");
    backup::export_local(&lib, &zip)?;
    let mut archive = zip::ZipArchive::new(fs::File::open(zip)?)?;
    let mut manifest = String::new();
    archive
        .by_name("manifest.json")?
        .read_to_string(&mut manifest)?;
    ensure!(
        !manifest.contains("sqlite-fixture-key")
            && !manifest.contains("sqlite-storage-key")
            && !manifest.contains("local_credentials"),
        "备份泄漏SQLite密钥"
    );
    provider.fail.store(true, Ordering::SeqCst);
    ensure!(
        credentials::set_secure_mode(&lib, true).is_err()
            && !credentials::secure_mode(&lib)?
            && ai::load_chat(&lib)?.1 == "sqlite-fixture-key",
        "启用失败破坏原模式或密钥"
    );
    provider.fail.store(false, Ordering::SeqCst);
    lib.db()?.execute_batch("CREATE TRIGGER fixture_fail_mode BEFORE INSERT ON app_settings WHEN NEW.key='credential_secure_mode' BEGIN SELECT RAISE(ABORT,'fixture mode failure'); END;")?;
    ensure!(
        credentials::set_secure_mode(&lib, true).is_err()
            && !credentials::secure_mode(&lib)?
            && ai::load_chat(&lib)?.1 == "sqlite-fixture-key",
        "模式事务失败丢失本机密钥"
    );
    lib.db()?.execute_batch("DROP TRIGGER fixture_fail_mode;")?;
    credentials::set_secure_mode(&lib, true)?;
    ensure!(
        credentials::secure_mode(&lib)? && ai::load_chat(&lib)?.1 == "sqlite-fixture-key",
        "迁移到安全模式失败"
    );
    let count: i64 = lib
        .db()?
        .query_row("SELECT count(*) FROM local_credentials", [], |r| r.get(0))?;
    ensure!(count == 0, "开启安全模式后SQLite仍保留密钥");
    let old = ai::load_config(&lib)?.credential_id;
    provider.fail.store(true, Ordering::SeqCst);
    ensure!(
        ai::save_shared(&lib, cfg, "replace-failure").is_err()
            && ai::load_config(&lib)?.credential_id == old,
        "安全模式保存失败破坏配置"
    );
    provider.fail.store(false, Ordering::SeqCst);
    let before = provider.calls.load(Ordering::SeqCst);
    credentials::set_secure_mode(&lib, false)?;
    ensure!(
        ai::load_chat(&lib).is_err() && provider.calls.load(Ordering::SeqCst) == before,
        "关闭模式偷偷回读钥匙串"
    );
    ensure!(credentials::import_legacy(&lib)? > 0, "显式迁移旧凭证失败");
    ensure!(
        ai::load_chat(&lib)?.1 == "sqlite-fixture-key",
        "迁移后本机密钥丢失"
    );
    backup::Storage::load(&lib)?;
    let before = provider.calls.load(Ordering::SeqCst);
    ai::load_chat(&lib)?;
    backup::Storage::load(&lib)?;
    ensure!(
        provider.calls.load(Ordering::SeqCst) == before,
        "迁移后普通读取仍访问钥匙串"
    );
    let legacy = Library::open(lib.root.join("legacy-entry"))?;
    legacy.set_setting(
        "ai_config",
        &json!({"credential_id":"e2e-explicit-legacy","shared_key_source":"chat"}).to_string(),
    )?;
    keyring::Entry::new("Clarora-GPUI", "chat-e2e-explicit-legacy")?
        .set_password("legacy-fixture-key")?;
    let before = provider.calls.load(Ordering::SeqCst);
    ensure!(
        ai::load_chat(&legacy).is_err() && provider.calls.load(Ordering::SeqCst) == before,
        "旧钥匙串引用触发隐式访问"
    );
    credentials::import_legacy(&legacy)?;
    ensure!(
        ai::load_chat(&legacy)?.1 == "legacy-fixture-key",
        "旧钥匙串实际读取未迁移到SQLite"
    );
    ensure!(
        provider
            .values
            .lock()
            .unwrap()
            .contains_key("chat-e2e-explicit-legacy"),
        "迁移误删旧钥匙串项"
    );
    fs::write(
        lib.root.join("credential-mode-evidence.json"),
        serde_json::to_vec_pretty(
            &json!({"sqlite_default":true,"no_keychain_calls_in_sqlite_mode":true,"restart_preserves_key":true,"blank_preserves_key":true,"export_excludes_keys":true,"secure_migration_clears_sqlite":true,"migration_failure_preserves_data":true,"explicit_legacy_import":true,"controlled_provider_not_system_keychain":true}),
        )?,
    )?;
    results.push(json!({"workflow":"密钥存储模式：SQLite默认/重开/空值保留 → 备份排除 → 安全迁移/失败保留 → 关闭不回读/主动导入","passed":true,"evidence":"credential-mode/credential-mode-evidence.json"}));
    Ok(())
}
