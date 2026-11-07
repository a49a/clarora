//! Portable manifest schema, attachment references, and snapshot capture.
use super::check_cancel;
use crate::data::{
    Library, Row, id,
    schema::{PREFERENCES, TABLES},
    text,
};
use crate::process::Cancel;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
};

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
pub(super) fn hash(value: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(value.as_bytes()))
}
pub(super) fn material_context(page: &str, material: &str) -> String {
    hash(&format!("{page}:{material}"))
}
fn hash_id(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|c| c.is_ascii_hexdigit())
}
pub(super) fn safe_setting(key: &str) -> bool {
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
    pub(super) fn map_paths(&mut self, map: &mut impl FnMut(&str) -> Result<String>) -> Result<()> {
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
pub(super) fn map_manifest(
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
pub(super) fn map_media(
    data: &mut BTreeMap<String, Vec<Row>>,
    mut map: impl FnMut(&str) -> Result<String>,
) -> Result<()> {
    for spec in TABLES.iter().filter(|spec| !spec.media.is_empty()) {
        if let Some(rows) = data.get_mut(spec.name) {
            for row in rows {
                for field in spec.media {
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
pub(super) fn source_paths(state: &LibraryState) -> Result<BTreeMap<PathBuf, String>> {
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
pub(super) fn collect_cancellable(
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
fn local_path(uri: &str) -> Result<PathBuf> {
    if uri.starts_with("file:") {
        url::Url::parse(uri)?
            .to_file_path()
            .map_err(|_| anyhow::anyhow!("附件路径无效"))
    } else {
        Ok(PathBuf::from(uri))
    }
}
pub(super) fn valid_extension(ext: &str) -> bool {
    !ext.is_empty() && ext.len() <= 15 && ext.bytes().all(|c| c.is_ascii_alphanumeric())
}
pub fn validate(manifest: &mut Manifest) -> Result<()> {
    ensure!(
        manifest.version == 2
            && manifest.id.len() <= 128
            && regex::Regex::new(r"^[a-z0-9-]+$")?.is_match(&manifest.id),
        "备份版本或标识错误"
    );
    manifest.data.entry("sentence_cards".into()).or_default();
    for spec in TABLES {
        let table = spec.name;
        let columns = spec.columns;
        let rows = manifest.data.get(table).context("缺少备份数据表")?;
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
