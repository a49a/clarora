use crate::data::{Library, Row, TABLES, id, insert, text};
use crate::process::Cancel;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    future::Future,
    io::{Read, Write},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

fn check_cancel(cancel: &Cancel) -> Result<()> {
    ensure!(!cancel.load(Ordering::SeqCst), "已取消");
    Ok(())
}
async fn interrupted(cancel: &Cancel) {
    while !cancel.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
async fn cancellable<T>(cancel: &Cancel, work: impl Future<Output = Result<T>>) -> Result<T> {
    check_cancel(cancel)?;
    tokio::select! {
        biased;
        // A completed PUT response wins over a concurrent late cancellation.
        result = work => result,
        _ = interrupted(cancel) => anyhow::bail!("已取消"),
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub id: String,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    pub data: BTreeMap<String, Vec<Row>>,
    pub files: BTreeMap<String, String>,
}
fn map_media(
    data: &mut BTreeMap<String, Vec<Row>>,
    mut map: impl FnMut(&str) -> Result<String>,
) -> Result<()> {
    for (table, fields) in [
        ("listening_audios", vec!["audio_uri", "subtitle_uri"]),
        ("clip_cards", vec!["audio_uri"]),
        ("video_clips", vec!["video_uri"]),
    ] {
        if let Some(rows) = data.get_mut(table) {
            for row in rows {
                for field in &fields {
                    let uri = text(row, field);
                    if !uri.is_empty() {
                        row.insert((*field).into(), json!(map(&uri)?));
                    }
                }
            }
        }
    }
    if let Some(rows) = data.get_mut("discover_favorites") {
        for row in rows {
            let mut payload: Value = serde_json::from_str(&text(row, "payload"))?;
            if let Some(uri) = payload["audio"]["uri"].as_str() {
                let mapped = map(uri)?;
                payload["audio"]["uri"] = json!(mapped);
                row.insert("payload".into(), json!(payload.to_string()));
            }
        }
    }
    Ok(())
}
pub fn collect(lib: &Library) -> Result<(Manifest, BTreeMap<String, PathBuf>)> {
    let mut data = lib.vault()?;
    let mut paths = BTreeMap::new();
    let mut refs: HashMap<String, String> = HashMap::new();
    map_media(&mut data, |uri| {
        if let Some(r) = refs.get(uri) {
            return Ok(r.clone());
        }
        let path = if uri.starts_with("file:") {
            url::Url::parse(uri)?
                .to_file_path()
                .map_err(|_| anyhow::anyhow!("附件路径无效"))?
        } else {
            PathBuf::from(uri)
        };
        ensure!(
            path.is_file(),
            "备份附件缺失：{}",
            path.file_name().unwrap_or_default().to_string_lossy()
        );
        let reference = format!("media:{}", paths.len());
        paths.insert(reference.clone(), path);
        refs.insert(uri.to_string(), reference.clone());
        Ok(reference)
    })?;
    Ok((
        Manifest {
            version: 2,
            id: id(),
            created_at: chrono::Utc::now().to_rfc3339(),
            data,
            files: BTreeMap::new(),
        },
        paths,
    ))
}
pub fn export_local(lib: &Library, path: &Path) -> Result<()> {
    let (mut manifest, files) = collect(lib)?;
    let temp = path.with_extension("part");
    let result = (|| {
        let mut zip = zip::ZipWriter::new(fs::File::create(&temp)?);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (reference, file) in files {
            let ext = file.extension().and_then(|s| s.to_str()).unwrap_or("bin");
            let name = format!(
                "media/{}.{}",
                reference.strip_prefix("media:").unwrap(),
                ext
            );
            manifest.files.insert(reference, name.clone());
            zip.start_file(name, options)?;
            std::io::copy(&mut fs::File::open(file)?, &mut zip)?;
        }
        zip.start_file("manifest.json", options)?;
        zip.write_all(serde_json::to_string(&manifest)?.as_bytes())?;
        zip.finish()?.sync_all()?;
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
pub fn validate(manifest: &mut Manifest) -> Result<()> {
    ensure!(
        manifest.version == 2 && regex::Regex::new(r"^[a-z0-9-]+$")?.is_match(&manifest.id),
        "备份版本或标识错误"
    );
    manifest.data.entry("sentence_cards".into()).or_default();
    for (table, columns) in TABLES {
        let rows = manifest.data.get(*table).context("缺少备份数据表")?;
        ensure!(rows.len() <= 500000, "数据表过大");
        for row in rows {
            ensure!(
                columns.iter().all(|col| row
                    .get(*col)
                    .is_some_and(|v| v.is_string() || v.is_number() || v.is_null())),
                "备份字段无效：{table}"
            );
        }
    }
    Ok(())
}
pub fn import_local(lib: &Library, path: &Path) -> Result<usize> {
    let mut zip = zip::ZipArchive::new(fs::File::open(path)?)?;
    let mut raw = String::new();
    zip.by_name("manifest.json")?
        .take(64 * 1024 * 1024)
        .read_to_string(&mut raw)?;
    let mut manifest: Manifest = serde_json::from_str(&raw)?;
    validate(&mut manifest)?;
    let stage = lib.root.join("media").join(format!("restored-{}", id()));
    fs::create_dir_all(&stage)?;
    let result = (|| {
        let mut mapped: HashMap<String, String> = HashMap::new();
        map_media(&mut manifest.data, |reference| {
            if let Some(path) = mapped.get(reference) {
                return Ok(path.clone());
            }
            let key = manifest.files.get(reference).context("备份缺少附件引用")?;
            ensure!(
                regex::Regex::new(r"^media/[0-9]+\.[A-Za-z0-9]{1,15}$")?.is_match(key),
                "备份附件路径无效"
            );
            let target = stage.join(Path::new(key).file_name().context("附件名称无效")?);
            let mut entry = zip.by_name(key)?;
            ensure!(entry.size() < 8 * 1024 * 1024 * 1024, "附件过大");
            let mut output = fs::File::create(&target)?;
            std::io::copy(&mut entry, &mut output)?;
            output.sync_all()?;
            let path = target.to_string_lossy().to_string();
            mapped.insert(reference.to_string(), path.clone());
            Ok(path)
        })?;
        merge(lib, &manifest.data)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(stage);
    }
    result
}
pub fn merge(lib: &Library, data: &BTreeMap<String, Vec<Row>>) -> Result<usize> {
    merge_cancellable(lib, data, &crate::process::cancel_token())
}
fn merge_cancellable(
    lib: &Library,
    data: &BTreeMap<String, Vec<Row>>,
    cancel: &Cancel,
) -> Result<usize> {
    check_cancel(cancel)?;
    let db = lib.db()?;
    // Acquire the writer before reading: a deferred WAL transaction cannot safely
    // retry an upgrade from a read snapshot when another writer owns the database.
    db.busy_timeout(Duration::from_millis(50))?;
    let waiting = Instant::now();
    let tx = loop {
        check_cancel(cancel)?;
        // This is a fresh connection with no outer transaction; runtime checking
        // allows the failed BEGIN attempts to share a borrow across this loop.
        match rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate) {
            Ok(tx) => break tx,
            Err(error) => {
                check_cancel(cancel)?;
                if error.sqlite_error_code() != Some(rusqlite::ErrorCode::DatabaseBusy)
                    || waiting.elapsed() >= Duration::from_secs(10)
                {
                    return Err(error.into());
                }
            }
        }
    };
    check_cancel(cancel)?;
    tx.busy_timeout(Duration::from_secs(10))?;
    let mut mapping: HashMap<(String, String), String> = HashMap::new();
    let mut imported = 0;
    for (table, _) in TABLES {
        if ["review_schedule", "study_stats", "discover_favorites"].contains(table) {
            continue;
        }
        let kind = match *table {
            "words" => "word",
            "listening_practices" => "practice",
            "listening_audios" => "audio",
            "clip_cards" => "clip",
            "sentence_cards" => "sentence",
            "ai_cards" => "ai",
            "video_clips" => "video",
            _ => "record",
        };
        for original in data.get(*table).context("缺少资料表")? {
            check_cancel(cancel)?;
            let mut row = original.clone();
            let old = text(&row, "id");
            let mut existing = None;
            if *table == "words" {
                existing = tx
                    .query_row(
                        "SELECT CAST(id AS TEXT) FROM words WHERE word=?",
                        [text(&row, "word")],
                        |r| r.get::<_, String>(0),
                    )
                    .ok();
            }
            if *table == "listening_practices" {
                existing = tx
                    .query_row(
                        "SELECT id FROM listening_practices WHERE lower(name)=lower(?)",
                        [text(&row, "name")],
                        |r| r.get::<_, String>(0),
                    )
                    .ok();
            }
            if *table == "listening_audios" {
                let practice = mapping
                    .get(&("practice".into(), text(&row, "practice_id")))
                    .context("备份音频的练习组缺失")?;
                row.insert("practice_id".into(), json!(practice));
                existing = tx
                    .query_row(
                        "SELECT id FROM listening_audios WHERE practice_id=? AND name=?",
                        rusqlite::params![practice, text(&row, "name")],
                        |r| r.get::<_, String>(0),
                    )
                    .ok();
            }
            if existing.is_none() && *table != "words" {
                existing = tx
                    .query_row(&format!("SELECT id FROM {table} WHERE id=?"), [&old], |r| {
                        r.get::<_, String>(0)
                    })
                    .ok();
            }
            let key = if let Some(existing) = existing {
                existing
            } else {
                if *table == "words" {
                    row.remove("id");
                }
                insert(&tx, table, &row, false)?;
                imported += 1;
                if *table == "words" {
                    tx.last_insert_rowid().to_string()
                } else {
                    old.clone()
                }
            };
            mapping.insert((kind.into(), old), key);
        }
    }
    for row in data.get("review_schedule").context("缺少复习记录")? {
        check_cancel(cancel)?;
        let mut row = row.clone();
        let kind = text(&row, "card_kind");
        let Some(key) = mapping.get(&(kind.clone(), text(&row, "card_id"))) else {
            continue;
        };
        row.insert("card_id".into(), json!(key));
        let previous=tx.query_row("SELECT coalesce(last_graded_at,'') FROM review_schedule WHERE card_kind=? AND card_id=?",rusqlite::params![kind,key],|r|r.get::<_,String>(0)).ok();
        if previous.is_some_and(|old| old >= text(&row, "last_graded_at")) {
            continue;
        }
        tx.execute(
            "DELETE FROM review_schedule WHERE card_kind=? AND card_id=?",
            rusqlite::params![kind, key],
        )?;
        insert(&tx, "review_schedule", &row, false)?;
    }
    for row in data.get("study_stats").context("缺少统计记录")? {
        check_cancel(cancel)?;
        tx.execute("INSERT INTO study_stats(day,listen_seconds,review_count,pomodoro_count) VALUES(?,?,?,?) ON CONFLICT(day) DO UPDATE SET listen_seconds=max(listen_seconds,excluded.listen_seconds),review_count=max(review_count,excluded.review_count),pomodoro_count=max(pomodoro_count,excluded.pomodoro_count)",rusqlite::params![text(row,"day"),row["listen_seconds"].as_f64().unwrap_or(0.),row["review_count"].as_f64().unwrap_or(0.),row["pomodoro_count"].as_f64().unwrap_or(0.)])?;
    }
    for row in data.get("discover_favorites").context("缺少收藏")? {
        check_cancel(cancel)?;
        let mut row = row.clone();
        let key = text(&row, "key");
        let mut payload: Value = serde_json::from_str(&text(&row, "payload"))?;
        if let Some((kind, old)) = key.split_once(':') {
            if let Some(mapped) = mapping.get(&(kind.into(), old.into())) {
                let key = format!("{kind}:{mapped}");
                payload["key"] = json!(key);
                row.insert("key".into(), json!(key));
                row.insert("payload".into(), json!(payload.to_string()));
            }
        }
        insert(&tx, "discover_favorites", &row, true)?;
    }
    // Once commit starts, return its actual result; a late cancel cannot undo it.
    check_cancel(cancel)?;
    tx.commit()?;
    Ok(imported)
}
#[derive(Clone, Default, Serialize, Deserialize)]
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
enum Payload {
    Bytes(Vec<u8>),
    File(PathBuf),
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
    pub fn load(lib: &Library) -> Result<Self> {
        let c: StorageConfig =
            serde_json::from_str(&lib.setting("storage_config")?).context("请先配置对象存储")?;
        let secret = crate::credentials::get(lib, &format!("storage-{}", c.credential_id))?;
        let token = crate::credentials::get_optional(lib, &format!("token-{}", c.credential_id))?;
        let s = Self {
            config: c,
            secret,
            token,
            fixture_endpoint: None,
        };
        s.validate()?;
        Ok(s)
    }
    pub fn save(lib: &Library, mut c: StorageConfig, secret: &str, token: &str) -> Result<()> {
        let tmp = Self {
            config: c.clone(),
            secret: secret.into(),
            token: token.into(),
            fixture_endpoint: None,
        };
        tmp.validate()?;
        c.credential_id = id();
        crate::credentials::store_config(
            lib,
            "storage_config",
            &serde_json::to_string(&c)?,
            &[
                (format!("storage-{}", c.credential_id), secret.into()),
                (format!("token-{}", c.credential_id), token.into()),
            ],
        )
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
    async fn response(
        &self,
        client: &reqwest::Client,
        method: &str,
        key: &str,
        query: &BTreeMap<String, String>,
        body: Option<Payload>,
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
        if let Some(body) = body {
            request = match body {
                Payload::Bytes(bytes) => request.body(bytes),
                Payload::File(path) => {
                    let file = tokio::fs::File::open(path).await?;
                    request
                        .header("content-length", file.metadata().await?.len())
                        .body(file)
                }
            };
        }
        let response = cancellable(cancel, async { Ok(request.send().await?) }).await?;
        ensure!(
            response.status().is_success(),
            "对象存储请求失败：HTTP {}",
            response.status()
        );
        Ok(response)
    }
    async fn request(
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
    fn client(&self) -> Result<reqwest::Client> {
        let mut builder = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(20))
            .timeout(Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::none());
        if self.fixture_endpoint.is_some() {
            builder = builder.no_proxy();
        }
        Ok(builder.build()?)
    }
    pub fn list(&self) -> Result<Vec<String>> {
        self.list_cancellable(&crate::process::cancel_token())
    }
    pub fn list_cancellable(&self, cancel: &Cancel) -> Result<Vec<String>> {
        check_cancel(cancel)?;
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(async {
                let client = self.client()?;
                let prefix = format!("{}/snapshots/", self.config.prefix);
                let mut keys = vec![];
                let mut token = String::new();
                let mut seen = std::collections::HashSet::new();
                loop {
                    check_cancel(cancel)?;
                    let mut query = BTreeMap::from([
                        ("list-type".into(), "2".into()),
                        ("prefix".into(), prefix.clone()),
                    ]);
                    if !token.is_empty() {
                        query.insert("continuation-token".into(), token);
                    }
                    let xml = String::from_utf8(
                        self.request(&client, "GET", "", &query, None, cancel)
                            .await?,
                    )?;
                    ensure!(
                        !xml.contains("<!DOCTYPE") && !xml.contains("<!ENTITY"),
                        "不支持的 XML"
                    );
                    #[derive(Deserialize)]
                    #[serde(rename_all = "PascalCase")]
                    struct Listing {
                        #[serde(default)]
                        contents: Vec<Content>,
                        #[serde(default)]
                        is_truncated: bool,
                        next_continuation_token: Option<String>,
                    }
                    #[derive(Deserialize)]
                    #[serde(rename_all = "PascalCase")]
                    struct Content {
                        key: String,
                    }
                    let listing: Listing = quick_xml::de::from_str(&xml)?;
                    keys.extend(
                        listing
                            .contents
                            .into_iter()
                            .map(|c| c.key)
                            .filter(|k| k.starts_with(&prefix) && k.ends_with(".json")),
                    );
                    if !listing.is_truncated {
                        break;
                    }
                    token = listing
                        .next_continuation_token
                        .context("存储分页缺少令牌")?;
                    ensure!(
                        !token.is_empty() && seen.insert(token.clone()),
                        "存储分页循环"
                    );
                }
                keys.sort();
                keys.reverse();
                Ok(keys)
            })
    }
    pub fn upload(&self, lib: &Library, cancel: &crate::process::Cancel) -> Result<String> {
        check_cancel(cancel)?;
        let (mut manifest, files) = collect(lib)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let client = self.client()?;
        for (reference, path) in files {
            check_cancel(cancel)?;
            let key = format!(
                "{}/media/{}/{}.{}",
                self.config.prefix,
                manifest.id,
                reference.trim_start_matches("media:"),
                path.extension().unwrap_or_default().to_string_lossy()
            );
            runtime.block_on(self.response(
                &client,
                "PUT",
                &key,
                &BTreeMap::new(),
                Some(Payload::File(path)),
                cancel,
            ))?;
            manifest.files.insert(reference, key);
        }
        let key = format!("{}/snapshots/{}.json", self.config.prefix, manifest.id);
        let bytes = serde_json::to_vec(&manifest)?;
        check_cancel(cancel)?;
        // Publishing the manifest is the remote commit. A lost response cannot prove rollback.
        runtime
            .block_on(self.response(
                &client,
                "PUT",
                &key,
                &BTreeMap::new(),
                Some(Payload::Bytes(bytes)),
                cancel,
            ))
            .context("备份清单发布结果未确认，请刷新云端列表核实；附件可能已上传")?;
        Ok(key)
    }
    pub fn restore(
        &self,
        lib: &Library,
        key: &str,
        cancel: &crate::process::Cancel,
    ) -> Result<usize> {
        check_cancel(cancel)?;
        ensure!(
            key.starts_with(&format!("{}/snapshots/", self.config.prefix)),
            "备份路径无效"
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let client = self.client()?;
        let mut manifest: Manifest = serde_json::from_slice(&runtime.block_on(self.request(
            &client,
            "GET",
            key,
            &BTreeMap::new(),
            None,
            cancel,
        ))?)?;
        check_cancel(cancel)?;
        validate(&mut manifest)?;
        ensure!(
            key == format!("{}/snapshots/{}.json", self.config.prefix, manifest.id),
            "备份清单标识不一致"
        );
        let root = lib.root.join("media").join(format!("restored-{}", id()));
        fs::create_dir_all(&root)?;
        let mut paths: HashMap<String, String> = HashMap::new();
        let result = (|| {
            map_media(&mut manifest.data, |reference| {
                check_cancel(cancel)?;
                if let Some(p) = paths.get(reference) {
                    return Ok(p.clone());
                }
                let key = manifest.files.get(reference).context("备份缺少附件")?;
                let prefix = format!("{}/media/{}/", self.config.prefix, manifest.id);
                ensure!(
                    key.starts_with(&prefix)
                        && regex::Regex::new(r"^[0-9]+\.[A-Za-z0-9]{1,15}$")?
                            .is_match(&key[prefix.len()..]),
                    "远端附件路径无效"
                );
                let path = root.join(&key[prefix.len()..]);
                runtime.block_on(async {
                    let mut response = self
                        .response(&client, "GET", key, &BTreeMap::new(), None, cancel)
                        .await?;
                    cancellable(cancel, async {
                        let mut output = fs::File::create(&path)?;
                        let mut length = 0_u64;
                        while let Some(chunk) = response.chunk().await? {
                            check_cancel(cancel)?;
                            length += chunk.len() as u64;
                            ensure!(length < 8 * 1024 * 1024 * 1024, "附件过大");
                            output.write_all(&chunk)?;
                        }
                        check_cancel(cancel)?;
                        output.sync_all()?;
                        Ok(())
                    })
                    .await
                })?;
                let path = path.to_string_lossy().to_string();
                paths.insert(reference.into(), path.clone());
                Ok(path)
            })?;
            merge_cancellable(lib, &manifest.data, cancel)
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(root);
        }
        result
    }
}

/// Snapshot an RN SQLite library read-only, copy attachments and merge into GPUI's own DB.
pub fn import_sqlite(lib: &Library, path: &Path) -> Result<usize> {
    let source =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    source.busy_timeout(std::time::Duration::from_secs(10))?;
    source.execute_batch("BEGIN")?;
    let mut data = BTreeMap::new();
    for (table, columns) in TABLES {
        let exists: i64 = source.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?",
            [table],
            |r| r.get(0),
        )?;
        if exists == 0 && *table == "sentence_cards" {
            data.insert(table.to_string(), vec![]);
            continue;
        }
        ensure!(exists > 0, "所选数据库不是兼容的 Clarora 资料库");
        let mut statement =
            source.prepare(&format!("SELECT {} FROM {table}", columns.join(",")))?;
        let rows = statement
            .query_map([], |r| {
                let mut row = Row::new();
                for (i, col) in columns.iter().enumerate() {
                    let value: rusqlite::types::Value = r.get(i)?;
                    row.insert(
                        col.to_string(),
                        match value {
                            rusqlite::types::Value::Null => Value::Null,
                            rusqlite::types::Value::Integer(n) => json!(n),
                            rusqlite::types::Value::Real(n) => json!(n),
                            rusqlite::types::Value::Text(s) => json!(s),
                            _ => Value::Null,
                        },
                    );
                }
                Ok(row)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        data.insert(table.to_string(), rows);
    }
    source.execute_batch("ROLLBACK")?;
    let stage = lib.root.join("media").join(format!("imported-{}", id()));
    fs::create_dir_all(&stage)?;
    let mut paths: HashMap<String, String> = HashMap::new();
    let result = (|| {
        map_media(&mut data, |uri| {
            if let Some(path) = paths.get(uri) {
                return Ok(path.clone());
            }
            let original = if uri.starts_with("file:") {
                url::Url::parse(uri)?
                    .to_file_path()
                    .map_err(|_| anyhow::anyhow!("附件路径无效"))?
            } else {
                PathBuf::from(uri)
            };
            ensure!(
                original.is_absolute() && original.is_file(),
                "原库附件不存在，未导入"
            );
            let ext = original
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("bin");
            ensure!(
                ext.chars().all(|c| c.is_ascii_alphanumeric()) && ext.len() <= 15,
                "附件扩展名无效"
            );
            let target = stage.join(format!("{}.{}", id(), ext));
            fs::copy(&original, &target)?;
            let path = target.to_string_lossy().to_string();
            paths.insert(uri.into(), path.clone());
            Ok(path)
        })?;
        merge(lib, &data)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(stage);
    }
    result
}
