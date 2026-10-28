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
#[derive(Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub id: String,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    pub data: BTreeMap<String, Vec<Row>>,
    pub files: BTreeMap<String, String>,
    /// Re-downloadable Bilibili originals; never object-storage file keys.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sources: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library: Option<LibraryState>,
}
/// Optional v2 extension. Old GPUI/RN backups omit this section entirely.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LibraryState {
    pub videos: Option<crate::video_library::Collection>,
    pub books: Vec<BookAttachment>,
    pub subtitles: Vec<SubtitleAttachment>,
    pub contexts: Vec<ContextBinding>,
    pub settings: BTreeMap<String, String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BookAttachment {
    pub path: String,
    pub name: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubtitleAttachment {
    pub video: String,
    pub subtitle: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextBinding {
    pub context: String,
    pub page: String,
    pub path: String,
    /// None = ordinary material path, -1 = all video subtitles, >=0 = subtitle index.
    pub cue: Option<i64>,
}
const PREFERENCES: &[&str] = &[
    "video_captions_enabled",
    "video_subtitle_size",
    "subtitle_font_size",
    "blind_chinese_hint",
    "video_auto_next",
    "navigation_open",
    "last_page",
    "new_cards_per_day",
    "daily_limit",
    "review_new_card_next_kind",
    "tts_voice",
    "tts_speed",
    "library_group",
    "last_listening_audio",
    "last_listening_position",
    "last_video_position",
    "pdf_current",
];
fn hash(value: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(value.as_bytes()))
}
fn material_context(page: &str, material: &str) -> String {
    hash(&format!("{page}:{material}"))
}
fn hash_id(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|c| c.is_ascii_hexdigit())
}
fn safe_setting(key: &str) -> bool {
    PREFERENCES.contains(&key)
        || key.strip_prefix("reader_progress_").is_some_and(hash_id)
        || key.strip_prefix("chat_draft_").is_some_and(hash_id)
        || key.strip_prefix("key_points_").is_some_and(|id| {
            !id.is_empty()
                && id.len() <= 128
                && id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c))
        })
}
fn valid_book_name(name: &str) -> bool {
    let path = Path::new(name);
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || stem
            .strip_prefix("COM")
            .or_else(|| stem.strip_prefix("LPT"))
            .is_some_and(|s| matches!(s, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"));
    !name.is_empty()
        && name.len() <= 255
        && !reserved
        && !name.starts_with('.')
        && !name.ends_with([' ', '.'])
        && !name
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
        && path.file_name().and_then(|s| s.to_str()) == Some(name)
        && crate::ebook::supported(path)
}
impl LibraryState {
    fn map_paths(&mut self, map: &mut impl FnMut(&str) -> Result<String>) -> Result<()> {
        if let Some(collection) = &mut self.videos {
            for video in &mut collection.items {
                video.path = PathBuf::from(map(&video.path.to_string_lossy())?);
                if let Some(series) = &mut video.series {
                    for part in &mut series.parts {
                        if let Some(path) = &mut part.path {
                            *path = PathBuf::from(map(&path.to_string_lossy())?);
                        }
                    }
                }
            }
        }
        for book in &mut self.books {
            book.path = map(&book.path)?;
        }
        for subtitle in &mut self.subtitles {
            subtitle.video = map(&subtitle.video)?;
            subtitle.subtitle = map(&subtitle.subtitle)?;
        }
        for binding in &mut self.contexts {
            binding.path = map(&binding.path)?;
        }
        for key in ["last_video_position", "pdf_current"] {
            if let Some(value) = self.settings.get_mut(key) {
                let mut object: Value =
                    serde_json::from_str(value).context("备份阅读/播放位置损坏")?;
                if let Some(path) = object["path"].as_str().filter(|s| !s.is_empty()) {
                    object["path"] = json!(map(path)?);
                }
                *value = object.to_string();
            }
        }
        Ok(())
    }
}
fn library_state(
    lib: &Library,
    data: &BTreeMap<String, Vec<Row>>,
    settings: &BTreeMap<String, String>,
    cancel: &Cancel,
) -> Result<LibraryState> {
    let mut state = LibraryState::default();
    let value = settings
        .get("video_favorites_v1")
        .map(String::as_str)
        .unwrap_or("");
    if !value.is_empty() {
        let videos: crate::video_library::Collection =
            serde_json::from_str(value).context("视频收藏数据损坏，未备份")?;
        videos.validate()?;
        state.videos = Some(videos);
    } else if let Some(value) = settings.get("recent_videos").filter(|s| !s.is_empty()) {
        let paths: Vec<PathBuf> = serde_json::from_str(value)?;
        if !paths.is_empty() {
            let series = settings
                .get("bilibili_series")
                .filter(|s| !s.is_empty())
                .map(|s| serde_json::from_str::<crate::bilibili::Series>(s))
                .transpose()?;
            let mut videos = crate::video_library::Collection::default();
            let ids = videos.add(&paths, series.as_ref())?;
            videos.selected = ids.first().cloned();
            state.videos = Some(videos);
        }
    }
    let mut paths = std::collections::BTreeSet::new();
    for entry in fs::read_dir(lib.root.join("pdf"))? {
        check_cancel(cancel)?;
        let path = entry?.path();
        if path.is_file() && crate::ebook::supported(&path) {
            paths.insert(path);
        }
    }
    if let Some(current) = settings.get("pdf_current").filter(|s| !s.is_empty()) {
        let current: Value = serde_json::from_str(current).context("当前阅读位置损坏，未备份")?;
        if let Some(path) = current["path"].as_str().filter(|s| !s.is_empty()) {
            let path = PathBuf::from(path);
            ensure!(crate::ebook::supported(&path), "当前阅读文件格式无效");
            paths.insert(path);
        }
    }
    for path in paths {
        ensure!(
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(valid_book_name),
            "书库文件名不兼容，请重命名后备份：{}",
            path.display()
        );
        state.books.push(BookAttachment {
            name: path
                .file_name()
                .context("书籍名称无效")?
                .to_string_lossy()
                .into_owned(),
            path: path.to_string_lossy().into_owned(),
        });
    }
    // Only explicitly named UI/learning settings cross the backup boundary.
    state.settings = settings
        .iter()
        .filter(|(key, value)| {
            safe_setting(key) && !key.starts_with("chat_draft_") && !value.is_empty()
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let mut contexts = std::collections::BTreeSet::new();
    for row in data
        .get("ai_records")
        .into_iter()
        .flatten()
        .filter(|row| text(row, "kind") == "chat")
    {
        if let Ok(payload) = serde_json::from_str::<Value>(&text(row, "payload")) {
            if let Some(context) = payload["context"].as_str() {
                contexts.insert(context.to_string());
            }
        }
    }
    for key in settings.keys() {
        if let Some(context) = key.strip_prefix("chat_draft_").filter(|s| hash_id(s)) {
            contexts.insert(context.to_string());
        }
    }
    let known_sources = source_paths(&state)?;
    let mut add = |page: &str, path: &Path, cue: Option<i64>, material: &str| {
        let context = material_context(page, material);
        if contexts.contains(&context) {
            state.contexts.push(ContextBinding {
                context,
                page: page.into(),
                path: path.to_string_lossy().into_owned(),
                cue,
            });
        }
    };
    for book in &state.books {
        add("pdf", Path::new(&book.path), None, &book.path);
    }
    let mut video_paths = std::collections::BTreeSet::new();
    if let Some(videos) = &state.videos {
        for video in &videos.items {
            video_paths.insert(video.path.clone());
            if let Some(series) = &video.series {
                for part in &series.parts {
                    if let Some(path) = &part.path {
                        video_paths.insert(path.clone());
                    }
                }
            }
        }
    }
    for video in video_paths {
        check_cancel(cancel)?;
        add("video", &video, None, &video.to_string_lossy());
        let key = match crate::video_subtitles::identity_key(&video) {
            Ok(key) => key,
            Err(_) if known_sources.contains_key(&video) && !video.is_file() => continue,
            Err(error) => return Err(error),
        };
        if let Some(subtitle) = settings.get(&key).filter(|s| !s.is_empty()) {
            let path = PathBuf::from(subtitle);
            ensure!(path.starts_with(lib.root.join("media")), "字幕缓存路径无效");
            let cues = crate::subtitles::parse(
                &fs::read_to_string(&path).context("视频字幕缓存缺失，未备份")?,
            );
            state.subtitles.push(SubtitleAttachment {
                video: video.to_string_lossy().into_owned(),
                subtitle: subtitle.clone(),
            });
            if !contexts.is_empty() {
                if let Ok(material) =
                    crate::video_qa::build(&video, &cues, crate::video_qa::Scope::All, 0., "")
                {
                    add("video", &video, Some(-1), &material.key);
                }
            }
            for index in 0..if contexts.is_empty() { 0 } else { cues.len() } {
                check_cancel(cancel)?;
                if let Ok(material) = crate::video_qa::build(
                    &video,
                    &cues,
                    crate::video_qa::Scope::Cue(index),
                    0.,
                    "",
                ) {
                    add("video", &video, Some(index as i64), &material.key);
                }
            }
        }
    }
    // Preserve drafts for known stable contexts and reconstructible path contexts.
    for (key, value) in settings {
        if key.starts_with("chat_draft_") && safe_setting(key) && !value.is_empty() {
            state.settings.insert(key.clone(), value.clone());
        }
    }
    Ok(state)
}
fn map_manifest(
    manifest: &mut Manifest,
    mut map: impl FnMut(&str) -> Result<String>,
) -> Result<()> {
    let mut mapped = HashMap::new();
    {
        let mut remember = |path: &str| -> Result<String> {
            let target = map(path)?;
            mapped.insert(path.to_string(), target.clone());
            Ok(target)
        };
        map_media(&mut manifest.data, &mut remember)?;
        if let Some(library) = &mut manifest.library {
            library.map_paths(&mut remember)?;
        }
    }
    crate::clip_marks::map_sources(&mut manifest.data, &mapped)?;
    Ok(())
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
    collect_cancellable(lib, &crate::process::cancel_token(), false)
}
fn source_paths(state: &LibraryState) -> Result<BTreeMap<PathBuf, String>> {
    let mut sources = BTreeMap::new();
    if let Some(videos) = &state.videos {
        for video in &videos.items {
            if let Some(series) = &video.series {
                for (index, part) in series.parts.iter().enumerate() {
                    if let Some(path) = &part.path {
                        sources.insert(path.clone(), series.url(index)?);
                    }
                }
                let index = series
                    .parts
                    .iter()
                    .position(|p| p.path.as_ref() == Some(&video.path))
                    .or(series.current)
                    .unwrap_or(series.requested);
                sources.insert(video.path.clone(), series.url(index)?);
            }
        }
    }
    Ok(sources)
}
fn collect_cancellable(
    lib: &Library,
    cancel: &Cancel,
    links_only: bool,
) -> Result<(Manifest, BTreeMap<String, PathBuf>)> {
    check_cancel(cancel)?;
    let (data, settings) = lib.vault_with_settings()?;
    let library = library_state(lib, &data, &settings, cancel)?;
    let mut source_paths = source_paths(&library)?;
    for (path, url) in source_paths.clone() {
        if let Ok(path) = path.canonicalize() {
            source_paths.insert(path, url);
        }
    }
    let mut manifest = Manifest {
        version: 2,
        id: id(),
        created_at: chrono::Utc::now().to_rfc3339(),
        data,
        files: BTreeMap::new(),
        sources: BTreeMap::new(),
        library: Some(library),
    };
    let mut paths = BTreeMap::new();
    let mut refs: HashMap<PathBuf, String> = HashMap::new();
    let mut sources = BTreeMap::new();
    map_manifest(&mut manifest, |uri| {
        check_cancel(cancel)?;
        let original = local_path(uri)?;
        let source = source_paths.get(&original).or_else(|| {
            original
                .canonicalize()
                .ok()
                .as_ref()
                .and_then(|p| source_paths.get(p))
        });
        if let Some(url) = source.filter(|_| links_only || !original.is_file()) {
            let reference = format!("source:{}", hash(url));
            sources.insert(reference.clone(), url.clone());
            return Ok(reference);
        }
        let path = original.canonicalize().with_context(|| {
            format!(
                "备份附件缺失：{}",
                Path::new(uri)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            )
        })?;
        if let Some(reference) = refs.get(&path) {
            return Ok(reference.clone());
        }
        ensure!(path.is_file(), "备份附件不是文件");
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("bin");
        ensure!(valid_extension(ext), "备份附件扩展名无效");
        let reference = format!("media:{}", paths.len());
        paths.insert(reference.clone(), path.clone());
        refs.insert(path, reference.clone());
        Ok(reference)
    })?;
    manifest.sources = sources;
    Ok((manifest, paths))
}
fn attachment_stamps(
    files: &BTreeMap<String, PathBuf>,
) -> Result<BTreeMap<PathBuf, (u64, std::time::SystemTime)>> {
    files
        .values()
        .map(|path| {
            let meta = fs::metadata(path)?;
            Ok((path.clone(), (meta.len(), meta.modified()?)))
        })
        .collect()
}
fn check_attachments(
    stamps: &BTreeMap<PathBuf, (u64, std::time::SystemTime)>,
    cancel: &Cancel,
) -> Result<()> {
    for (path, stamp) in stamps {
        check_cancel(cancel)?;
        let meta = fs::metadata(path)?;
        ensure!(
            (meta.len(), meta.modified()?) == *stamp,
            "备份期间附件已改变，请重试：{}",
            path.file_name().unwrap_or_default().to_string_lossy()
        );
    }
    Ok(())
}
fn local_path(uri: &str) -> Result<PathBuf> {
    if uri.starts_with("file:") {
        url::Url::parse(uri)?
            .to_file_path()
            .map_err(|_| anyhow::anyhow!("附件路径无效"))
    } else {
        Ok(PathBuf::from(uri))
    }
}
fn valid_extension(ext: &str) -> bool {
    !ext.is_empty() && ext.len() <= 15 && ext.bytes().all(|c| c.is_ascii_alphanumeric())
}
fn copy_cancellable(mut input: impl Read, output: &mut impl Write, cancel: &Cancel) -> Result<u64> {
    let mut bytes = [0; 128 * 1024];
    let mut length = 0;
    loop {
        check_cancel(cancel)?;
        let count = input.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        length += count as u64;
        ensure!(length < 8 * 1024 * 1024 * 1024, "附件过大");
        output.write_all(&bytes[..count])?;
    }
    check_cancel(cancel)?;
    Ok(length)
}
pub fn export_local(lib: &Library, path: &Path) -> Result<()> {
    export_local_cancellable(lib, path, &crate::process::cancel_token())
}
pub fn export_local_cancellable(lib: &Library, path: &Path, cancel: &Cancel) -> Result<()> {
    let (mut manifest, files) = collect_cancellable(lib, cancel, false)?;
    let stamps = attachment_stamps(&files)?;
    let temp = path.with_file_name(format!(
        ".{}-{}.part",
        path.file_name()
            .context("备份文件名无效")?
            .to_string_lossy(),
        id()
    ));
    let result = (|| {
        let mut zip = zip::ZipWriter::new(
            fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temp)?,
        );
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (reference, file) in files {
            check_cancel(cancel)?;
            let name = format!(
                "media/{}.{}",
                reference.trim_start_matches("media:"),
                file.extension().and_then(|s| s.to_str()).unwrap_or("bin")
            );
            manifest.files.insert(reference, name.clone());
            zip.start_file(name, options)?;
            copy_cancellable(fs::File::open(file)?, &mut zip, cancel)?;
        }
        check_cancel(cancel)?;
        zip.start_file("manifest.json", options)?;
        zip.write_all(serde_json::to_string(&manifest)?.as_bytes())?;
        zip.finish()?.sync_all()?;
        check_attachments(&stamps, cancel)?;
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
        manifest.version == 2
            && manifest.id.len() <= 128
            && regex::Regex::new(r"^[a-z0-9-]+$")?.is_match(&manifest.id),
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
    if let Some(library) = &manifest.library {
        ensure!(
            library.books.len() <= 10000
                && library.subtitles.len() <= 10000
                && library.contexts.len() <= 100000
                && library.settings.len() <= 500000,
            "备份书库或设置过大"
        );
        for (key, value) in &library.settings {
            ensure!(
                safe_setting(key) && value.len() <= 4 * 1024 * 1024,
                "备份包含不允许的设置：{key}"
            );
        }
        for book in &library.books {
            ensure!(valid_book_name(&book.name), "备份书籍名称无效");
        }
        for binding in &library.contexts {
            ensure!(
                hash_id(&binding.context)
                    && ["pdf", "video"].contains(&binding.page.as_str())
                    && (binding.page == "video" || binding.cue.is_none())
                    && binding.cue.is_none_or(|i| (-1..10000).contains(&i)),
                "备份材料会话无效"
            );
        }
    }
    ensure!(manifest.sources.len() <= 100000, "备份视频来源过多");
    let mut used = std::collections::BTreeSet::new();
    for (reference, url) in &manifest.sources {
        ensure!(
            reference == &format!("source:{}", hash(url))
                && crate::process::normalize_bilibili(url)? == *url,
            "备份视频来源无效"
        );
    }
    if let Some(videos) = manifest.library.as_ref().and_then(|s| s.videos.as_ref()) {
        for video in &videos.items {
            let mut check_source = |path: &Path, expected: String| -> Result<()> {
                let reference = path.to_string_lossy();
                if reference.starts_with("source:") {
                    ensure!(
                        manifest.sources.get(reference.as_ref()) == Some(&expected),
                        "视频来源与分P不匹配"
                    );
                    used.insert(reference.into_owned());
                }
                Ok(())
            };
            if let Some(series) = &video.series {
                for (index, part) in series.parts.iter().enumerate() {
                    if let Some(path) = &part.path {
                        check_source(path, series.url(index)?)?;
                    }
                }
                let index = series
                    .parts
                    .iter()
                    .position(|p| p.path.as_ref() == Some(&video.path))
                    .or(series.current)
                    .unwrap_or(series.requested);
                check_source(&video.path, series.url(index)?)?;
            } else {
                ensure!(
                    !video.path.to_string_lossy().starts_with("source:"),
                    "本地视频不能引用网络来源"
                );
            }
        }
    }
    ensure!(
        used.len() == manifest.sources.len(),
        "备份包含未绑定视频的来源链接"
    );
    let references = regex::Regex::new(r"^media:[0-9]+$")?;
    let files = &manifest.files;
    let mut check = manifest.clone();
    map_manifest(&mut check, |reference| {
        if let Some(url) = manifest.sources.get(reference) {
            return Ok(std::env::temp_dir()
                .join("downloads/remote-bilibili")
                .join(hash(url))
                .join("video.mp4")
                .to_string_lossy()
                .into_owned());
        }
        ensure!(
            references.is_match(reference) && files.contains_key(reference),
            "备份缺少或包含无效附件引用"
        );
        Ok(std::env::temp_dir()
            .join(reference.replace(':', "-"))
            .to_string_lossy()
            .into_owned())
    })?;
    if let Some(videos) = check.library.as_ref().and_then(|s| s.videos.as_ref()) {
        videos.validate()?;
    }
    Ok(())
}
pub fn import_local(lib: &Library, path: &Path) -> Result<usize> {
    import_local_cancellable(lib, path, &crate::process::cancel_token())
}
pub fn import_local_cancellable(lib: &Library, path: &Path, cancel: &Cancel) -> Result<usize> {
    check_cancel(cancel)?;
    let mut zip = zip::ZipArchive::new(fs::File::open(path)?)?;
    let mut raw = String::new();
    let entry = zip.by_name("manifest.json")?;
    ensure!(entry.size() <= 64 * 1024 * 1024, "备份清单过大");
    entry.take(64 * 1024 * 1024 + 1).read_to_string(&mut raw)?;
    check_cancel(cancel)?;
    let mut manifest: Manifest = serde_json::from_str(&raw)?;
    validate(&mut manifest)?;
    let stage = lib.root.join("media").join(format!("restored-{}", id()));
    fs::create_dir_all(&stage)?;
    let result = (|| {
        let mut mapped: HashMap<String, String> = HashMap::new();
        let files = manifest.files.clone();
        let sources = manifest.sources.clone();
        map_manifest(&mut manifest, |reference| {
            check_cancel(cancel)?;
            if let Some(url) = sources.get(reference) {
                return Ok(crate::bilibili::source_path(lib, url)?
                    .to_string_lossy()
                    .into_owned());
            }
            if let Some(path) = mapped.get(reference) {
                return Ok(path.clone());
            }
            let key = files.get(reference).context("备份缺少附件引用")?;
            ensure!(
                regex::Regex::new(r"^media/[0-9]+\.[A-Za-z0-9]{1,15}$")?.is_match(key),
                "备份附件路径无效"
            );
            let target = stage.join(Path::new(key).file_name().context("附件名称无效")?);
            let entry = zip.by_name(key)?;
            let size = entry.size();
            ensure!(size < 8 * 1024 * 1024 * 1024, "附件过大");
            let mut output = fs::File::create(&target)?;
            ensure!(
                copy_cancellable(entry, &mut output, cancel)? == size,
                "备份附件不完整"
            );
            output.sync_all()?;
            let path = target.to_string_lossy().into_owned();
            mapped.insert(reference.into(), path.clone());
            Ok(path)
        })?;
        finish_restore(lib, &mut manifest, &stage, cancel)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&stage);
    }
    result
}
fn file_hash(path: &Path, cancel: &Cancel) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut bytes = [0; 128 * 1024];
    loop {
        check_cancel(cancel)?;
        let n = file.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        hash.update(&bytes[..n]);
    }
    Ok(hex::encode(hash.finalize()))
}
fn finish_restore(
    lib: &Library,
    manifest: &mut Manifest,
    stage: &Path,
    cancel: &Cancel,
) -> Result<usize> {
    let mut created = vec![];
    let result = (|| {
        let mut relocated: HashMap<String, String> = HashMap::new();
        let mut existing = BTreeMap::new();
        if let Some(library) = &mut manifest.library {
            for entry in fs::read_dir(lib.root.join("pdf"))? {
                let path = entry?.path();
                if path.is_file() && crate::ebook::supported(&path) {
                    existing.insert(
                        (
                            file_hash(&path, cancel)?,
                            path.extension()
                                .map(|s| s.to_string_lossy().to_ascii_lowercase())
                                .unwrap_or_default(),
                        ),
                        path,
                    );
                }
            }
            for book in &mut library.books {
                check_cancel(cancel)?;
                if let Some(path) = relocated.get(&book.path) {
                    book.path = path.clone();
                    continue;
                }
                let original = PathBuf::from(&book.path);
                let hash = file_hash(&original, cancel)?;
                let ext = Path::new(&book.name)
                    .extension()
                    .context("书籍扩展名无效")?
                    .to_string_lossy()
                    .to_ascii_lowercase();
                ensure!(
                    original
                        .extension()
                        .is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case(&ext)),
                    "书籍附件格式不一致"
                );
                let target = if let Some(path) = existing.get(&(hash.clone(), ext.clone())) {
                    path.clone()
                } else {
                    let candidate = lib.root.join("pdf").join(&book.name);
                    let target = if candidate.exists() {
                        let mut stem = Path::new(&book.name)
                            .file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned();
                        while stem.len() > 180 {
                            stem.pop();
                        }
                        lib.root
                            .join("pdf")
                            .join(format!("{}-{}.{}", stem, &hash[..32], ext))
                    } else {
                        candidate
                    };
                    if target.exists() {
                        ensure!(file_hash(&target, cancel)? == hash, "书库恢复目标名称冲突");
                    } else {
                        let mut output = fs::OpenOptions::new()
                            .create_new(true)
                            .write(true)
                            .open(&target)?;
                        created.push(target.clone());
                        copy_cancellable(fs::File::open(&original)?, &mut output, cancel)?;
                        output.sync_all()?;
                    }
                    existing.insert((hash, ext), target.clone());
                    target
                };
                relocated.insert(book.path.clone(), target.to_string_lossy().into_owned());
            }
        }
        map_manifest(manifest, |path| {
            Ok(relocated.get(path).cloned().unwrap_or_else(|| path.into()))
        })?;
        merge_snapshot(lib, &manifest.data, manifest.library.as_ref(), cancel)
    })();
    if result.is_err() {
        for path in created {
            let _ = fs::remove_file(path);
        }
    }
    if result.is_ok() {
        prune_stage(lib, stage);
    }
    result
}
fn references(value: &Value, path: &str, depth: u8) -> bool {
    if depth > 8 {
        return false;
    }
    match value {
        Value::String(value) => {
            value == path
                || serde_json::from_str::<Value>(value)
                    .is_ok_and(|v| references(&v, path, depth + 1))
        }
        Value::Array(values) => values.iter().any(|v| references(v, path, depth + 1)),
        Value::Object(values) => values.values().any(|v| references(v, path, depth + 1)),
        _ => false,
    }
}
fn prune_stage(lib: &Library, stage: &Path) {
    // Cleanup after the atomic DB commit is conservative: a failed read keeps files.
    let Ok((data, settings)) = lib.vault_with_settings() else {
        return;
    };
    let Ok(data) = serde_json::to_value(data) else {
        return;
    };
    let Ok(settings) = serde_json::to_value(settings) else {
        return;
    };
    if let Ok(entries) = fs::read_dir(stage) {
        for entry in entries.flatten() {
            let path = entry.path();
            let value = path.to_string_lossy();
            if path.is_file() && !references(&data, &value, 0) && !references(&settings, &value, 0)
            {
                let _ = fs::remove_file(path);
            }
        }
    }
    let _ = fs::remove_dir(stage);
}
pub fn merge(lib: &Library, data: &BTreeMap<String, Vec<Row>>) -> Result<usize> {
    merge_cancellable(lib, data, &crate::process::cancel_token())
}
fn merge_cancellable(
    lib: &Library,
    data: &BTreeMap<String, Vec<Row>>,
    cancel: &Cancel,
) -> Result<usize> {
    merge_snapshot(lib, data, None, cancel)
}
fn merge_snapshot(
    lib: &Library,
    original_data: &BTreeMap<String, Vec<Row>>,
    library: Option<&LibraryState>,
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
    let mut data = original_data.clone();
    let mut library = library.cloned();
    if let Some(library) = &mut library {
        prepare_library(&tx, &mut data, library, cancel)?;
    }
    let data = &data;
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
            if *table == "ai_records" {
                crate::clip_marks::map_audio(&mut row, &mapping)?;
            }
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
                if *table == "ai_records" && text(&row, "kind") == "chat" {
                    if let Some(library) = &library {
                        repair_chat_context(&tx, &existing, &text(&row, "payload"), library)?;
                    }
                }
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
    if let Some(library) = &mut library {
        merge_library(&tx, library, &mapping, cancel)?;
    }
    // Once commit starts, return its actual result; a late cancel cannot undo it.
    check_cancel(cancel)?;
    tx.commit()?;
    Ok(imported)
}
fn repair_chat_context(
    db: &rusqlite::Connection,
    id: &str,
    incoming: &str,
    library: &LibraryState,
) -> Result<()> {
    use rusqlite::OptionalExtension;
    let Ok(imported) = serde_json::from_str::<Value>(incoming) else {
        return Ok(());
    };
    let Some(context) = imported["context"].as_str().filter(|context| {
        library
            .contexts
            .iter()
            .any(|binding| binding.context == *context)
    }) else {
        return Ok(());
    };
    let previous: Option<String> = db
        .query_row(
            "SELECT payload FROM ai_records WHERE id=? AND kind='chat'",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    let Some(mut previous) = previous.and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
    else {
        return Ok(());
    };
    if previous["context"] != context {
        if let Some(old) = previous["context"].as_str() {
            let draft = setting_in(db, &format!("chat_draft_{old}"))?;
            if !draft.is_empty() {
                preserve_setting(db, &format!("chat_draft_{context}"), &draft)?;
            }
        }
        previous["context"] = json!(context);
        db.execute(
            "UPDATE ai_records SET payload=? WHERE id=?",
            rusqlite::params![previous.to_string(), id],
        )?;
    }
    Ok(())
}
fn setting_in(db: &rusqlite::Connection, key: &str) -> Result<String> {
    use rusqlite::OptionalExtension;
    Ok(db
        .query_row("SELECT value FROM app_settings WHERE key=?", [key], |row| {
            row.get(0)
        })
        .optional()?
        .unwrap_or_default())
}
fn usable_file(path: &Path) -> bool {
    path.is_file() && fs::metadata(path).is_ok_and(|meta| meta.len() > 0)
}
fn preserve_setting(db: &rusqlite::Connection, key: &str, value: &str) -> Result<()> {
    let existing = setting_in(db, key)?;
    let usable = if existing.is_empty() {
        false
    } else if ["pdf_current", "last_video_position"].contains(&key) {
        serde_json::from_str::<Value>(&existing)
            .ok()
            .and_then(|value| value["path"].as_str().map(PathBuf::from))
            .is_some_and(|path| usable_file(&path))
    } else if key.starts_with("video_subtitles_") {
        usable_file(Path::new(&existing))
    } else {
        true
    };
    if !usable {
        db.execute("INSERT INTO app_settings(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",rusqlite::params![key,value])?;
    }
    Ok(())
}
fn prepare_library(
    db: &rusqlite::Connection,
    data: &mut BTreeMap<String, Vec<Row>>,
    library: &mut LibraryState,
    cancel: &Cancel,
) -> Result<()> {
    let mut mapped = HashMap::<String, String>::new();
    if let Some(imported) = &library.videos {
        let current = setting_in(db, "video_favorites_v1")?;
        let mut current: crate::video_library::Collection = if current.is_empty() {
            crate::video_library::Collection::default()
        } else {
            serde_json::from_str(&current).context("当前视频收藏损坏，未恢复")?
        };
        current.validate()?;
        let initially_empty = current.items.is_empty();
        let mut repaired = HashMap::<String, String>::new();
        for video in &imported.items {
            check_cancel(cancel)?;
            let old = current.items.iter_mut().find(|old| {
                old.id == video.id
                    || video.series.as_ref().is_some_and(|series| {
                        old.series.as_ref().is_some_and(|s| s.base == series.base)
                    })
            });
            if let Some(old) = old {
                if let (Some(existing), Some(source)) = (&mut old.series, &video.series) {
                    for part in &source.parts {
                        if let Some(path) = &part.path {
                            if let Some(target) =
                                existing.parts.iter_mut().find(|p| p.number == part.number)
                            {
                                if let Some(target_path) = &target.path {
                                    if usable_file(target_path) {
                                        mapped.insert(
                                            path.to_string_lossy().into_owned(),
                                            target_path.to_string_lossy().into_owned(),
                                        );
                                    } else {
                                        repaired.insert(
                                            target_path.to_string_lossy().into_owned(),
                                            path.to_string_lossy().into_owned(),
                                        );
                                        target.path = Some(path.clone());
                                    }
                                } else {
                                    target.path = Some(path.clone());
                                }
                            } else {
                                existing.parts.push(part.clone());
                            }
                        } else if !existing
                            .parts
                            .iter()
                            .any(|existing| existing.number == part.number)
                        {
                            existing.parts.push(part.clone());
                        }
                    }
                    if !usable_file(&old.path) {
                        old.path = PathBuf::from(
                            repaired
                                .get(&old.path.to_string_lossy().into_owned())
                                .cloned()
                                .unwrap_or_else(|| video.path.to_string_lossy().into_owned()),
                        );
                    }
                    existing.parts.sort_by_key(|part| part.number);
                    if let Some(target) = source
                        .parts
                        .iter()
                        .find(|part| part.path.as_ref() == Some(&video.path))
                        .and_then(|source| {
                            existing
                                .parts
                                .iter()
                                .find(|part| part.number == source.number)
                        })
                        .and_then(|part| part.path.as_ref())
                    {
                        mapped.insert(
                            video.path.to_string_lossy().into_owned(),
                            target.to_string_lossy().into_owned(),
                        );
                    }
                } else if usable_file(&old.path) {
                    mapped.insert(
                        video.path.to_string_lossy().into_owned(),
                        old.path.to_string_lossy().into_owned(),
                    );
                } else {
                    repaired.insert(
                        old.path.to_string_lossy().into_owned(),
                        video.path.to_string_lossy().into_owned(),
                    );
                    old.path = video.path.clone();
                }
            } else {
                current.items.push(video.clone());
            }
        }
        // A deliberate deselection in an existing library stays deselected.
        if initially_empty {
            current.selected = imported.selected.clone();
        }
        let value = setting_in(db, "last_video_position")?;
        if let Ok(mut position) = serde_json::from_str::<Value>(&value) {
            if let Some(path) = position["path"].as_str().and_then(|old| repaired.get(old)) {
                position["path"] = json!(path);
                db.execute(
                    "UPDATE app_settings SET value=? WHERE key='last_video_position'",
                    [position.to_string()],
                )?;
            }
        }
        current.validate()?;
        library.videos = Some(current);
    }
    library.map_paths(&mut |path| Ok(mapped.get(path).cloned().unwrap_or_else(|| path.into())))?;
    map_media(data, |path| {
        Ok(mapped.get(path).cloned().unwrap_or_else(|| path.into()))
    })?;
    crate::clip_marks::map_sources(data, &mapped)?;
    let mut contexts = HashMap::new();
    for binding in &mut library.contexts {
        check_cancel(cancel)?;
        let material = if let Some(cue) = binding.cue {
            let subtitle = library
                .subtitles
                .iter()
                .find(|s| s.video == binding.path)
                .context("备份材料缺少视频字幕")?;
            let cues = crate::subtitles::parse(&fs::read_to_string(&subtitle.subtitle)?);
            crate::video_qa::build(
                Path::new(&binding.path),
                &cues,
                if cue < 0 {
                    crate::video_qa::Scope::All
                } else {
                    crate::video_qa::Scope::Cue(cue as usize)
                },
                0.,
                "",
            )?
            .key
        } else {
            binding.path.clone()
        };
        let context = material_context(&binding.page, &material);
        contexts.insert(binding.context.clone(), context.clone());
        binding.context = context;
    }
    for row in data
        .get_mut("ai_records")
        .into_iter()
        .flatten()
        .filter(|row| text(row, "kind") == "chat")
    {
        if let Ok(mut payload) = serde_json::from_str::<Value>(&text(row, "payload")) {
            if let Some(context) = payload["context"]
                .as_str()
                .and_then(|old| contexts.get(old))
            {
                payload["context"] = json!(context);
                row.insert("payload".into(), json!(payload.to_string()));
            }
        }
    }
    let mut drafts = vec![];
    library.settings.retain(|key, value| {
        if let Some(context) = key
            .strip_prefix("chat_draft_")
            .and_then(|old| contexts.get(old))
        {
            drafts.push((format!("chat_draft_{context}"), value.clone()));
            false
        } else {
            true
        }
    });
    library.settings.extend(drafts);
    Ok(())
}
fn merge_library(
    db: &rusqlite::Connection,
    library: &mut LibraryState,
    mapping: &HashMap<(String, String), String>,
    cancel: &Cancel,
) -> Result<()> {
    if let Some(videos) = &library.videos {
        videos.validate()?;
        db.execute("INSERT INTO app_settings(key,value) VALUES('video_favorites_v1',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[serde_json::to_string(videos)?])?;
        if let Some(series) = videos.active().and_then(|v| v.series.as_ref()) {
            preserve_setting(db, "bilibili_series", &serde_json::to_string(series)?)?;
        }
    }
    for subtitle in &library.subtitles {
        check_cancel(cancel)?;
        let key = crate::video_subtitles::identity_key(Path::new(&subtitle.video))?;
        preserve_setting(db, &key, &subtitle.subtitle)?;
    }
    for (key, value) in &library.settings {
        check_cancel(cancel)?;
        ensure!(safe_setting(key), "备份包含不允许的设置");
        let (mut key, mut value) = (key.clone(), value.clone());
        if let Some(id) = key.strip_prefix("key_points_") {
            if let Some(mapped) = mapping.get(&("audio".into(), id.into())) {
                key = format!("key_points_{mapped}");
            }
        }
        if ["library_group", "last_listening_audio"].contains(&key.as_str()) {
            if let Some(mapped) = mapping.get(&(
                if key == "library_group" {
                    "practice"
                } else {
                    "audio"
                }
                .into(),
                value.clone(),
            )) {
                value = mapped.clone();
            }
        }
        if key == "last_listening_position" {
            let mut position: Value = serde_json::from_str(&value)?;
            if let Some(id) = position["id"]
                .as_str()
                .and_then(|id| mapping.get(&("audio".into(), id.into())))
            {
                position["id"] = json!(id);
                value = position.to_string();
            }
        }
        preserve_setting(db, &key, &value)?;
    }
    Ok(())
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
        let mut response = cancellable(cancel, async { Ok(request.send().await?) }).await?;
        if !response.status().is_success() {
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
            let code = bytes
                .ok()
                .and_then(|bytes| storage_error_code(&bytes))
                .filter(|code| {
                    [&c.access_key_id, &self.secret, &self.token]
                        .iter()
                        .all(|value| value.is_empty() || !code.contains(value.as_str()))
                });
            anyhow::bail!(storage_error_message(status, code.as_deref()));
        }
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
        let (mut manifest, files) = collect_cancellable(lib, cancel, true)?;
        let stamps = attachment_stamps(&files)?;
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
        check_attachments(&stamps, cancel)?;
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
        let files = manifest.files.clone();
        let sources = manifest.sources.clone();
        let remote_prefix = format!("{}/media/{}/", self.config.prefix, manifest.id);
        let result = (|| {
            map_manifest(&mut manifest, |reference| {
                check_cancel(cancel)?;
                if let Some(url) = sources.get(reference) {
                    return Ok(crate::bilibili::source_path(lib, url)?
                        .to_string_lossy()
                        .into_owned());
                }
                if let Some(p) = paths.get(reference) {
                    return Ok(p.clone());
                }
                let key = files.get(reference).context("备份缺少附件")?;
                let prefix = remote_prefix.as_str();
                ensure!(
                    key.starts_with(prefix)
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
            finish_restore(lib, &mut manifest, &root, cancel)
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
        let _ = fs::remove_dir_all(&stage);
    } else {
        prune_stage(lib, &stage);
    }
    result
}
