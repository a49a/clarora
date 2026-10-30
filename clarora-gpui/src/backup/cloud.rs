//! Content-addressed cloud snapshots, verified chunks and CAS-coordinated cleanup.
use super::*;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub const CHUNK_BYTES: u64 = 8 * 1024 * 1024;
const LIMIT: usize = 64 * 1024 * 1024;
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Chunk {
    pub sha256: String,
    pub length: u64,
    pub key: String,
}
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Attachment {
    pub sha256: String,
    pub length: u64,
    pub extension: String,
    pub chunks: Vec<Chunk>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub format_version: u32,
    pub manifest: Manifest,
    pub attachments: BTreeMap<String, Attachment>,
}
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct Remote {
    pub key: String,
    pub sha256: String,
    pub length: u64,
    pub created_at: String,
    #[serde(default)]
    pub objects: BTreeSet<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Pin {
    pub expires_at: String,
    pub objects: BTreeSet<String>,
    pub snapshot: Option<String>,
}
#[derive(Clone, Serialize, Deserialize, Default)]
pub struct Catalog {
    pub format_version: u32,
    pub vault_id: String,
    pub epoch: u64,
    pub snapshots: BTreeMap<String, Remote>,
    pub blobs: BTreeMap<String, Chunk>,
    pub pins: BTreeMap<String, Pin>,
    pub sync_head: Option<Remote>,
    pub retired_legacy: BTreeSet<String>,
    pub garbage: BTreeMap<String, Option<String>>,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct BackupVersion {
    pub key: String,
    pub created_at: String,
    pub format_version: u32,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Retention {
    pub keep_last: usize,
    pub keep_days: u64,
    pub orphan_days: u64,
}
impl Default for Retention {
    fn default() -> Self {
        Self {
            keep_last: 10,
            keep_days: 30,
            orphan_days: 7,
        }
    }
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct CleanupPlan {
    pub retention: Retention,
    pub snapshots: Vec<String>,
    pub objects: Vec<String>,
    pub bytes: u64,
    pub prepared_at: String,
    pub(crate) catalog_etag: Option<String>,
    pub(crate) scope: String,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct CleanupResult {
    pub snapshots_deleted: usize,
    pub objects_deleted: usize,
    pub bytes_freed: u64,
}
pub struct Prepared {
    pub snapshot: Snapshot,
    pub paths: BTreeMap<String, PathBuf>,
    pub stamps: BTreeMap<PathBuf, (u64, std::time::SystemTime)>,
}
#[derive(Clone)]
pub struct Listed {
    pub key: String,
    pub length: u64,
    pub modified: chrono::DateTime<chrono::Utc>,
}
pub struct Session<'a> {
    pub storage: &'a Storage,
    pub cancel: &'a Cancel,
    client: reqwest::Client,
    runtime: tokio::runtime::Runtime,
}
pub fn checksum(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn time() -> String {
    chrono::Utc::now().to_rfc3339()
}
fn valid_hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}
fn utc(s: &str) -> Result<chrono::DateTime<chrono::Utc>> {
    Ok(chrono::DateTime::parse_from_rfc3339(s)?.with_timezone(&chrono::Utc))
}
impl Catalog {
    fn empty() -> Self {
        Self {
            format_version: 3,
            vault_id: id(),
            ..Default::default()
        }
    }
    fn expire_pins(&mut self) {
        let now = chrono::Utc::now();
        self.pins
            .retain(|_, p| utc(&p.expires_at).is_ok_and(|until| until > now));
    }
}
impl<'a> Session<'a> {
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
    pub fn prefix(&self) -> String {
        format!("{}/v3", self.storage.config.prefix)
    }
    pub fn scope(&self) -> String {
        hash(&format!(
            "{}|{}|{}|{}",
            self.storage.config.provider,
            self.storage.config.endpoint,
            self.storage.config.bucket,
            self.storage.config.prefix
        ))
    }
    pub fn key(&self, suffix: &str) -> String {
        format!("{}/{suffix}", self.prefix())
    }
    fn raw(
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
    fn checked(&self, response: reqwest::Response) -> Result<reqwest::Response> {
        if response.status().is_success() {
            return Ok(response);
        }
        let status = response.status();
        let bytes = self.read(response).unwrap_or_default();
        check_cancel(self.cancel)?;
        let code = storage_error_code(&bytes).filter(|code| {
            [
                &self.storage.config.access_key_id,
                &self.storage.secret,
                &self.storage.token,
            ]
            .iter()
            .all(|s| s.is_empty() || !code.contains(s.as_str()))
        });
        anyhow::bail!(storage_error_message(status, code.as_deref()))
    }
    pub fn read(&self, mut response: reqwest::Response) -> Result<Vec<u8>> {
        self.runtime.block_on(cancellable(self.cancel, async {
            let mut bytes = vec![];
            while let Some(chunk) = response.chunk().await? {
                check_cancel(self.cancel)?;
                ensure!(bytes.len() + chunk.len() <= LIMIT, "云清单或分块过大");
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
    pub fn verify_cas(&self) -> Result<()> {
        ensure!(
            self.storage.config.provider == "s3",
            "新版增量备份与同步需要支持条件写入的 S3 接口；旧 OSS 备份仍可读取"
        );
        let key = self.key(&format!("probes/{}", id()));
        ensure!(
            self.put(
                &key,
                b"conditional-write-probe".to_vec(),
                Some(("if-none-match", "*"))
            )?,
            "存储条件写入探测失败"
        );
        let create = self.put(
            &key,
            b"probe-must-not-overwrite".to_vec(),
            Some(("if-none-match", "*")),
        )?;
        let replace = self.put(
            &key,
            b"probe-must-not-overwrite".to_vec(),
            Some(("if-match", "\"clarora-mismatched-etag\"")),
        )?;
        let _ = self.delete(&key, None);
        ensure!(
            !create && !replace,
            "存储服务未执行安全条件写入，已停止新版备份／同步，避免覆盖其他设备"
        );
        Ok(())
    }
    pub fn catalog(&self) -> Result<(Catalog, Option<String>)> {
        let Some((bytes, etag)) = self.get(&self.key("catalog.json"))? else {
            return Ok((Catalog::empty(), None));
        };
        ensure!(
            etag.is_some(),
            "存储服务未返回目录 ETag，无法安全协调多设备"
        );
        let catalog: Catalog =
            serde_json::from_slice(&bytes).context("云端 v3 目录损坏，已停止修改")?;
        self.validate_catalog(&catalog)?;
        Ok((catalog, etag))
    }
    pub fn save_catalog(&self, catalog: &Catalog, etag: Option<&str>) -> Result<bool> {
        self.validate_catalog(catalog)?;
        let bytes = serde_json::to_vec(catalog)?;
        ensure!(bytes.len() <= LIMIT, "云端目录过大，请先清理历史版本");
        self.put(
            &self.key("catalog.json"),
            bytes,
            Some(
                etag.map(|e| ("if-match", e))
                    .unwrap_or(("if-none-match", "*")),
            ),
        )
    }
    pub fn mutate<T>(&self, mut edit: impl FnMut(&mut Catalog) -> Result<T>) -> Result<T> {
        for _ in 0..16 {
            check_cancel(self.cancel)?;
            let (mut catalog, etag) = self.catalog()?;
            catalog.expire_pins();
            let result = edit(&mut catalog)?;
            if self.save_catalog(&catalog, etag.as_deref())? {
                return Ok(result);
            }
        }
        anyhow::bail!("其他设备正在更新云端目录，请稍后重试")
    }
    fn object_key(&self, key: &str) -> bool {
        let Some(tail) = key.strip_prefix(&format!("{}/objects/", self.prefix())) else {
            return false;
        };
        tail.split_once('/')
            .is_some_and(|(epoch, sha)| epoch.parse::<u64>().is_ok() && valid_hash(sha))
    }
    pub fn validate_remote(&self, remote: &Remote) -> Result<()> {
        ensure!(
            remote.key.starts_with(&format!("{}/", self.prefix()))
                && !remote.key.split('/').any(|s| s == ".." || s == ".")
                && valid_hash(&remote.sha256)
                && remote.length <= LIMIT as u64,
            "云清单指针无效"
        );
        utc(&remote.created_at)?;
        ensure!(
            remote.objects.iter().all(|s| self.object_key(s)),
            "同步对象指针无效"
        );
        Ok(())
    }
    fn validate_catalog(&self, c: &Catalog) -> Result<()> {
        ensure!(
            c.format_version == 3
                && valid_id(&c.vault_id)
                && c.blobs.len() <= 100000
                && c.snapshots.len() <= 100000
                && c.pins.len() <= 10000,
            "云目录版本或大小无效"
        );
        for (id, s) in &c.snapshots {
            ensure!(
                valid_id(id) && s.key == self.key(&format!("snapshots/{id}.json")),
                "云快照路径无效"
            );
            self.validate_remote(s)?;
        }
        for (sha, b) in &c.blobs {
            ensure!(
                sha == &b.sha256
                    && valid_hash(sha)
                    && b.length > 0
                    && b.length <= CHUNK_BYTES
                    && self.object_key(&b.key),
                "云附件索引无效"
            );
        }
        if let Some(head) = &c.sync_head {
            self.validate_remote(head)?;
            ensure!(head.key.starts_with(&self.key("sync/")), "同步指针路径无效");
        }
        for p in c.pins.values() {
            utc(&p.expires_at)?;
            if let Some(key) = &p.snapshot {
                ensure!(
                    self.owned_key(key)
                        && (key.starts_with(&self.key("sync/"))
                            || key.starts_with(&self.key("snapshots/"))
                            || key.starts_with(&format!(
                                "{}/snapshots/",
                                self.storage.config.prefix
                            ))),
                    "云任务清单路径无效"
                );
            }
            ensure!(
                p.objects.len() <= 100000
                    && p.objects.iter().all(|key| self.object_key(key)
                        || key.starts_with(&format!("{}/media/", self.storage.config.prefix))),
                "云任务引用无效"
            );
        }
        for key in c.garbage.keys() {
            ensure!(self.owned_key(key), "清理对象越过资料库前缀");
        }
        Ok(())
    }
    fn owned_key(&self, key: &str) -> bool {
        (key.starts_with(&format!("{}/v3/", self.storage.config.prefix))
            || key.starts_with(&format!("{}/media/", self.storage.config.prefix))
            || key.starts_with(&format!("{}/snapshots/", self.storage.config.prefix)))
            && !key.split('/').any(|s| s == ".." || s == ".")
            && !key.contains('\\')
    }
    pub fn remote_bytes(&self, remote: &Remote) -> Result<Vec<u8>> {
        self.validate_remote(remote)?;
        let (bytes, _) = self.get(&remote.key)?.context("云清单不存在")?;
        ensure!(
            bytes.len() as u64 == remote.length && checksum(&bytes) == remote.sha256,
            "云清单完整性校验失败"
        );
        Ok(bytes)
    }
    pub fn put_remote(
        &self,
        folder: &str,
        name: &str,
        bytes: Vec<u8>,
        objects: BTreeSet<String>,
    ) -> Result<Remote> {
        ensure!(
            valid_id(name) && ["snapshots", "sync"].contains(&folder),
            "云清单名称无效"
        );
        let remote = Remote {
            key: self.key(&format!("{folder}/{name}.json")),
            sha256: checksum(&bytes),
            length: bytes.len() as u64,
            created_at: time(),
            objects,
        };
        ensure!(remote.length <= LIMIT as u64, "云清单过大");
        ensure!(
            self.put(&remote.key, bytes, Some(("if-none-match", "*")))?,
            "云清单标识已存在，请重试"
        );
        Ok(remote)
    }
    pub fn pin(&self, objects: BTreeSet<String>, snapshot: Option<String>) -> Result<String> {
        let pin = id();
        self.mutate(|c| {
            if let Some(key) = &snapshot {
                if key.starts_with(&self.key("snapshots/")) {
                    ensure!(
                        c.snapshots.values().any(|r| &r.key == key),
                        "该备份版本已清理，请刷新列表"
                    );
                } else {
                    ensure!(
                        !c.retired_legacy.contains(key),
                        "该备份版本已清理，请刷新列表"
                    );
                }
            }
            c.pins.insert(
                pin.clone(),
                Pin {
                    expires_at: (chrono::Utc::now() + chrono::Duration::hours(24)).to_rfc3339(),
                    objects: objects.clone(),
                    snapshot: snapshot.clone(),
                },
            );
            Ok(())
        })?;
        Ok(pin)
    }
    pub fn release(&self, pin: &str) {
        if !self.cancel.load(Ordering::SeqCst) {
            let _ = self.mutate(|c| {
                c.pins.remove(pin);
                Ok(())
            });
        }
    }
    pub fn allocate(&self, snapshot: &mut Snapshot) -> Result<String> {
        let pin = id();
        self.mutate(|c| {
            let mut objects = BTreeSet::new();
            for file in snapshot.attachments.values_mut() {
                for chunk in &mut file.chunks {
                    let entry = c
                        .blobs
                        .entry(chunk.sha256.clone())
                        .or_insert_with(|| Chunk {
                            sha256: chunk.sha256.clone(),
                            length: chunk.length,
                            key: self.key(&format!("objects/{}/{}", c.epoch, chunk.sha256)),
                        });
                    ensure!(entry.length == chunk.length, "内容摘要与块大小不一致");
                    *chunk = entry.clone();
                    objects.insert(chunk.key.clone());
                }
            }
            c.pins.insert(
                pin.clone(),
                Pin {
                    expires_at: (chrono::Utc::now() + chrono::Duration::hours(24)).to_rfc3339(),
                    objects,
                    snapshot: None,
                },
            );
            Ok(())
        })?;
        Ok(pin)
    }
    pub(super) fn send_prepared(
        &self,
        prepared: &Prepared,
        reporter: &Arc<UploadReporter>,
    ) -> Result<()> {
        let mut missing = BTreeMap::new();
        let mut present = BTreeSet::new();
        for (reference, file) in &prepared.snapshot.attachments {
            let mut offset = 0;
            for chunk in &file.chunks {
                if present.contains(&chunk.key) || missing.contains_key(&chunk.key) {
                    offset += chunk.length;
                    continue;
                }
                let remote = self.head(&chunk.key)?;
                if remote.as_ref().is_some_and(|(n, _, sha)| {
                    *n == chunk.length && sha.as_deref() == Some(&chunk.sha256)
                }) {
                    present.insert(chunk.key.clone());
                } else {
                    missing.insert(
                        chunk.key.clone(),
                        (
                            prepared.paths[reference].clone(),
                            offset,
                            chunk.clone(),
                            remote.and_then(|(_, etag, _)| etag),
                        ),
                    );
                }
                offset += chunk.length;
            }
        }
        let total = missing.values().map(|(_, _, c, _)| c.length).sum();
        reporter.update(true, |p| {
            p.stage = UploadStage::Uploading;
            p.bytes_total = total;
            p.bytes_sent = 0;
            p.bytes_reused = prepared
                .snapshot
                .attachments
                .values()
                .map(|f| f.length)
                .sum::<u64>()
                .saturating_sub(total);
            p.files_done = 0;
            p.files_total = prepared.snapshot.attachments.len();
        });
        for (reference, file) in &prepared.snapshot.attachments {
            let name = prepared.paths[reference]
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .chars()
                .filter(|c| !c.is_control())
                .take(80)
                .collect::<String>();
            reporter.update(true, |p| p.current_name = name);
            for chunk in &file.chunks {
                check_cancel(self.cancel)?;
                let Some((path, offset, chunk, etag)) = missing.remove(&chunk.key) else {
                    continue;
                };
                let mut headers =
                    BTreeMap::from([("x-amz-meta-sha256".into(), chunk.sha256.clone())]);
                if let Some(etag) = etag {
                    headers.insert("if-match".into(), etag);
                } else {
                    headers.insert("if-none-match".into(), "*".into());
                }
                let response = self.raw(
                    "PUT",
                    &chunk.key,
                    Some(Payload::FileRange {
                        path,
                        offset,
                        length: chunk.length,
                        sha256: chunk.sha256.clone(),
                        reporter: reporter.clone(),
                    }),
                    &headers,
                )?;
                if response.status() == reqwest::StatusCode::PRECONDITION_FAILED {
                    let Some((length, _, sha)) = self.head(&chunk.key)? else {
                        anyhow::bail!("其他设备附件上传尚未完成，请重试");
                    };
                    ensure!(
                        length == chunk.length && sha.as_deref() == Some(&chunk.sha256),
                        "远端附件与摘要不一致"
                    );
                } else {
                    self.checked(response)?;
                }
            }
            reporter.update(true, |p| {
                p.files_done += 1;
                p.current_name.clear();
            });
        }
        check_attachments(&prepared.stamps, self.cancel)?;
        Ok(())
    }
    pub fn validate_snapshot(&self, snapshot: &mut Snapshot) -> Result<()> {
        ensure!(snapshot.format_version == 3, "不支持的云备份版本");
        validate(&mut snapshot.manifest)?;
        ensure!(
            snapshot.attachments.len() == snapshot.manifest.files.len()
                && snapshot
                    .attachments
                    .keys()
                    .eq(snapshot.manifest.files.keys()),
            "附件清单不一致"
        );
        for (reference, file) in &snapshot.attachments {
            ensure!(
                valid_hash(&file.sha256)
                    && file.length < 8 * 1024 * 1024 * 1024
                    && valid_extension(&file.extension),
                "附件摘要或格式无效"
            );
            ensure!(
                snapshot.manifest.files[reference].ends_with(&format!(".{}", file.extension)),
                "附件扩展名不一致"
            );
            let mut sum = 0u64;
            for chunk in &file.chunks {
                ensure!(
                    valid_hash(&chunk.sha256)
                        && chunk.length > 0
                        && chunk.length <= CHUNK_BYTES
                        && self.object_key(&chunk.key)
                        && chunk.key.ends_with(&chunk.sha256),
                    "附件分块无效"
                );
                sum = sum.checked_add(chunk.length).context("附件大小溢出")?;
            }
            ensure!(
                sum == file.length && (file.length > 0 || file.sha256 == checksum(b"")),
                "附件长度不一致"
            );
        }
        Ok(())
    }
    pub fn download(&self, snapshot: &mut Snapshot, lib: &Library, root: &Path) -> Result<()> {
        self.validate_snapshot(snapshot)?;
        let mut mapped = HashMap::new();
        for (reference, file) in &snapshot.attachments {
            check_cancel(self.cancel)?;
            let path = root.join(&snapshot.manifest.files[reference]);
            ensure!(path.parent() == Some(root), "附件落盘路径无效");
            let mut output = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&path)?;
            let mut hasher = Sha256::new();
            let mut length = 0;
            for chunk in &file.chunks {
                let (bytes, _) = self.get(&chunk.key)?.context("备份附件分块缺失")?;
                ensure!(
                    bytes.len() as u64 == chunk.length && checksum(&bytes) == chunk.sha256,
                    "附件分块完整性校验失败"
                );
                output.write_all(&bytes)?;
                hasher.update(&bytes);
                length += bytes.len() as u64;
            }
            ensure!(
                length == file.length && hex::encode(hasher.finalize()) == file.sha256,
                "附件文件完整性校验失败"
            );
            output.sync_all()?;
            mapped.insert(reference.clone(), path.to_string_lossy().into_owned());
        }
        let sources = snapshot.manifest.sources.clone();
        map_manifest(&mut snapshot.manifest, |reference| {
            if let Some(url) = sources.get(reference) {
                return Ok(crate::bilibili::source_path(lib, url)?
                    .to_string_lossy()
                    .into_owned());
            }
            mapped.get(reference).cloned().context("备份缺少附件")
        })?;
        Ok(())
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
                    modified: utc(&entry.last_modified)?,
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
pub fn prepare(lib: &Library, cancel: &Cancel) -> Result<Prepared> {
    let (mut manifest, paths) = collect_cancellable(lib, cancel, true)?;
    let stamps = attachment_stamps(&paths)?;
    let mut attachments = BTreeMap::new();
    for (reference, path) in &paths {
        check_cancel(cancel)?;
        let length = stamps[path].0;
        ensure!(length < 8 * 1024 * 1024 * 1024, "备份附件过大");
        let extension = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("bin")
            .to_ascii_lowercase();
        let mut source = fs::File::open(path)?;
        let mut whole = Sha256::new();
        let mut chunks = vec![];
        let mut count = 0u64;
        let mut buffer = [0u8; 65536];
        while count < length {
            let mut chunk = Sha256::new();
            let mut taken = 0;
            let size = (length - count).min(CHUNK_BYTES);
            while taken < size {
                check_cancel(cancel)?;
                let n = source.read(&mut buffer[..(size - taken).min(65536) as usize])?;
                ensure!(n > 0, "备份期间附件已改变");
                whole.update(&buffer[..n]);
                chunk.update(&buffer[..n]);
                taken += n as u64;
            }
            count += taken;
            chunks.push(Chunk {
                sha256: hex::encode(chunk.finalize()),
                length: taken,
                key: String::new(),
            });
        }
        ensure!(source.read(&mut buffer[..1])? == 0, "备份期间附件已改变");
        manifest.files.insert(
            reference.clone(),
            format!("{}.{}", reference.trim_start_matches("media:"), extension),
        );
        attachments.insert(
            reference.clone(),
            Attachment {
                sha256: hex::encode(whole.finalize()),
                length,
                extension,
                chunks,
            },
        );
    }
    check_attachments(&stamps, cancel)?;
    Ok(Prepared {
        snapshot: Snapshot {
            format_version: 3,
            manifest,
            attachments,
        },
        paths,
        stamps,
    })
}
pub fn object_keys(snapshot: &Snapshot) -> BTreeSet<String> {
    snapshot
        .attachments
        .values()
        .flat_map(|f| f.chunks.iter().map(|c| c.key.clone()))
        .collect()
}
impl Storage {
    pub fn restore_checked(&self, lib: &Library, key: &str, cancel: &Cancel) -> Result<usize> {
        if key.starts_with(&format!("{}/v3/snapshots/", self.config.prefix)) {
            return self.restore_incremental(lib, key, cancel);
        }
        ensure!(
            key.starts_with(&format!("{}/snapshots/", self.config.prefix)),
            "备份路径无效"
        );
        let session = Session::new(self, cancel)?;
        let (_, etag) = session.catalog()?;
        if etag.is_none() {
            return self.restore(lib, key, cancel);
        }
        session.verify_cas()?;
        let (bytes, _) = session.get(key)?.context("旧备份清单缺失")?;
        let mut manifest: Manifest = serde_json::from_slice(&bytes)?;
        validate(&mut manifest)?;
        let pin = session.pin(manifest.files.values().cloned().collect(), Some(key.into()))?;
        let result = self.restore(lib, key, cancel);
        session.release(&pin);
        result
    }
    pub fn upload_incremental(
        &self,
        lib: &Library,
        cancel: &Cancel,
        progress: impl Fn(UploadProgress) + Send + Sync + 'static,
    ) -> Result<String> {
        let reporter = Arc::new(UploadReporter::new(progress));
        reporter.update(true, |_| {});
        let session = Session::new(self, cancel)?;
        session.verify_cas()?;
        let mut prepared = prepare(lib, cancel)?;
        let pin = session.allocate(&mut prepared.snapshot)?;
        let result = (|| {
            session.send_prepared(&prepared, &reporter)?;
            session.validate_snapshot(&mut prepared.snapshot)?;
            check_cancel(cancel)?;
            reporter.update(true, |p| p.stage = UploadStage::Publishing);
            let snapshot_id = prepared.snapshot.manifest.id.clone();
            let remote = session.put_remote(
                "snapshots",
                &snapshot_id,
                serde_json::to_vec(&prepared.snapshot)?,
                BTreeSet::new(),
            )?;
            let commit = session.mutate(|c| {
                ensure!(
                    c.pins.contains_key(&pin),
                    "备份任务已过期，请重试以续传已完成分块"
                );
                c.snapshots.insert(snapshot_id.clone(), remote.clone());
                c.pins.remove(&pin);
                Ok(())
            });
            if commit.is_err() {
                if session
                    .catalog()
                    .is_ok_and(|(c, _)| c.snapshots.get(&snapshot_id).is_some_and(|s| s == &remote))
                {
                    reporter.update(true, |p| p.stage = UploadStage::Completed);
                    return Ok(remote.key);
                }
                commit.context("备份目录发布结果未确认，请刷新版本列表核实；已确认的分块可续传")?;
            }
            reporter.update(true, |p| p.stage = UploadStage::Completed);
            Ok(remote.key)
        })();
        if result.is_ok() {
            session.release(&pin);
        }
        result
    }
    pub fn versions(&self, cancel: &Cancel) -> Result<Vec<BackupVersion>> {
        let session = Session::new(self, cancel)?;
        let (catalog, _) = session.catalog()?;
        let mut versions = catalog
            .snapshots
            .values()
            .map(|r| BackupVersion {
                key: r.key.clone(),
                created_at: r.created_at.clone(),
                format_version: 3,
            })
            .collect::<Vec<_>>();
        for key in self.list_cancellable(cancel)? {
            if catalog.retired_legacy.contains(&key) {
                continue;
            }
            let Some((bytes, _)) = session.get(&key)? else {
                continue;
            };
            let mut manifest: Manifest = serde_json::from_slice(&bytes)?;
            validate(&mut manifest)?;
            versions.push(BackupVersion {
                key,
                created_at: manifest.created_at,
                format_version: 2,
            });
        }
        versions.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| b.key.cmp(&a.key))
        });
        Ok(versions)
    }
    pub fn restore_incremental(&self, lib: &Library, key: &str, cancel: &Cancel) -> Result<usize> {
        let session = Session::new(self, cancel)?;
        session.verify_cas()?;
        let (catalog, _) = session.catalog()?;
        let remote = catalog
            .snapshots
            .values()
            .find(|r| r.key == key)
            .context("备份版本不存在或已清理")?;
        let mut snapshot: Snapshot = serde_json::from_slice(&session.remote_bytes(remote)?)?;
        session.validate_snapshot(&mut snapshot)?;
        let pin = session.pin(object_keys(&snapshot), Some(key.into()))?;
        let root = lib.root.join("media").join(format!("restored-{}", id()));
        fs::create_dir_all(&root)?;
        let result = (|| {
            session.download(&mut snapshot, lib, &root)?;
            finish_restore(lib, &mut snapshot.manifest, &root, cancel)
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(&root);
        }
        session.release(&pin);
        result
    }
    pub fn preview_cleanup(&self, retention: Retention, cancel: &Cancel) -> Result<CleanupPlan> {
        ensure!(
            retention.keep_last >= 1
                && retention.keep_last <= 10000
                && retention.keep_days <= 36500
                && retention.orphan_days >= 1
                && retention.orphan_days <= 36500,
            "保留策略无效：至少保留一份，孤立对象宽限至少一天"
        );
        let session = Session::new(self, cancel)?;
        let (mut catalog, etag) = session.catalog()?;
        catalog.expire_pins();
        let versions = self.versions(cancel)?;
        let cutoff = chrono::Utc::now() - chrono::Duration::days(retention.keep_days as i64);
        let grace = chrono::Utc::now() - chrono::Duration::days(retention.orphan_days as i64);
        let mut snapshots = vec![];
        let mut live = BTreeSet::new();
        let mut all = BTreeMap::new();
        let protected = catalog
            .pins
            .values()
            .filter_map(|p| p.snapshot.clone())
            .collect::<BTreeSet<_>>();
        for (index, version) in versions.iter().enumerate() {
            let keep = index < retention.keep_last
                || utc(&version.created_at)? >= cutoff
                || protected.contains(&version.key);
            let mut keys = BTreeSet::new();
            if version.format_version == 3 {
                let remote = catalog
                    .snapshots
                    .values()
                    .find(|r| r.key == version.key)
                    .context("快照目录变化")?;
                let mut snapshot: Snapshot =
                    serde_json::from_slice(&session.remote_bytes(remote)?)?;
                session.validate_snapshot(&mut snapshot)?;
                keys = object_keys(&snapshot);
            } else {
                let (bytes, _) = session
                    .get(&version.key)?
                    .context("旧备份清单缺失，已停止清理")?;
                let mut manifest: Manifest = serde_json::from_slice(&bytes)?;
                validate(&mut manifest)?;
                for key in manifest.files.values() {
                    ensure!(
                        key.starts_with(&format!("{}/media/{}/", self.config.prefix, manifest.id)),
                        "旧附件越过备份前缀"
                    );
                    keys.insert(key.clone());
                }
            }
            if keep {
                live.extend(keys);
            } else {
                snapshots.push(version.key.clone());
            }
        }
        if let Some(head) = &catalog.sync_head {
            super::sync::validate_cloud_head(&session, head)?;
            live.extend(head.objects.clone());
        }
        for pin in catalog.pins.values() {
            live.extend(pin.objects.clone());
            live.extend(pin.snapshot.clone());
        }
        for prefix in [
            session.key("objects/"),
            format!("{}/media/", self.config.prefix),
            session.key("sync/"),
            session.key("snapshots/"),
        ] {
            for object in session.list_prefix(&prefix)? {
                all.insert(object.key.clone(), object);
            }
        }
        let retained_manifests = catalog
            .snapshots
            .values()
            .map(|r| r.key.clone())
            .filter(|k| !snapshots.contains(k))
            .chain(catalog.sync_head.iter().map(|r| r.key.clone()))
            .collect::<BTreeSet<_>>();
        let mut objects = vec![];
        let mut bytes = 0u64;
        for object in all.values() {
            if !live.contains(&object.key)
                && !retained_manifests.contains(&object.key)
                && !snapshots.contains(&object.key)
                && object.modified < grace
            {
                objects.push(object.key.clone());
                bytes = bytes.checked_add(object.length).context("清理大小溢出")?;
            }
        }
        for key in catalog.garbage.keys() {
            if !objects.contains(key) && !snapshots.contains(key) {
                objects.push(key.clone());
            }
        }
        Ok(CleanupPlan {
            retention,
            snapshots,
            objects,
            bytes,
            prepared_at: time(),
            catalog_etag: etag,
            scope: session.scope(),
        })
    }
    pub fn cleanup(&self, plan: &CleanupPlan, cancel: &Cancel) -> Result<CleanupResult> {
        let session = Session::new(self, cancel)?;
        session.verify_cas()?;
        ensure!(
            plan.scope == session.scope()
                && chrono::Utc::now() - utc(&plan.prepared_at)? < chrono::Duration::minutes(10),
            "清理预览已过期或存储配置已改变，请重新预览"
        );
        let current = self.preview_cleanup(plan.retention.clone(), cancel)?;
        ensure!(
            current.catalog_etag == plan.catalog_etag
                && current.snapshots == plan.snapshots
                && current.objects == plan.objects,
            "云端资料已变化，请重新预览后清理"
        );
        let (mut catalog, etag) = session.catalog()?;
        ensure!(etag == plan.catalog_etag, "云端目录已变化，请重新预览");
        let retiring = plan
            .snapshots
            .iter()
            .cloned()
            .chain(plan.objects.iter().cloned())
            .collect::<BTreeSet<_>>();
        // Detach before deleting, and advance the physical-key epoch. A later writer
        // can never reuse a retired key, even if this process pauses or crashes.
        catalog.epoch = catalog.epoch.checked_add(1).context("云目录代次溢出")?;
        catalog.snapshots.retain(|_, r| !retiring.contains(&r.key));
        catalog.blobs.retain(|_, b| !retiring.contains(&b.key));
        for key in &plan.snapshots {
            if key.starts_with(&format!("{}/snapshots/", self.config.prefix)) {
                catalog.retired_legacy.insert(key.clone());
            }
        }
        for key in &retiring {
            ensure!(session.owned_key(key), "清理对象越过资料库前缀");
            catalog.garbage.insert(key.clone(), None);
        }
        ensure!(
            session.save_catalog(&catalog, etag.as_deref())?,
            "云端目录已变化，请重新预览"
        );
        let mut deleted = 0;
        for key in &retiring {
            check_cancel(cancel)?;
            let etag = session.head(key)?.and_then(|(_, e, _)| e);
            session.delete(key, etag.as_deref())?;
            session.mutate(|c| {
                c.garbage.remove(key);
                Ok(())
            })?;
            deleted += 1;
        }
        Ok(CleanupResult {
            snapshots_deleted: plan.snapshots.len(),
            objects_deleted: deleted,
            bytes_freed: plan.bytes,
        })
    }
}
