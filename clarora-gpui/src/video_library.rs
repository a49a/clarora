//! Persistent video bookmarks, independent of the transient player and recent-file order.
use crate::{
    bilibili,
    data::{Library, id},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
pub(crate) const FAVORITES_KEY: &str = "video_favorites_v1";
pub(crate) const GROUPS_KEY: &str = "video_groups_v1";
/// Pseudo group used only by the collection filter; never stored on a video.
pub const UNGROUPED: &str = "__ungrouped__";
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Video {
    pub id: String,
    pub path: PathBuf,
    pub title: String,
    pub series: Option<bilibili::Series>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub group: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Collection {
    version: u8,
    pub items: Vec<Video>,
    pub selected: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<String>,
}
#[derive(Serialize, Deserialize)]
struct Groups {
    names: Vec<String>,
    assignments: BTreeMap<String, String>,
}
impl Default for Collection {
    fn default() -> Self {
        Self {
            version: 1,
            items: vec![],
            selected: None,
            groups: vec![],
        }
    }
}
fn normalized(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}
impl Collection {
    pub(crate) fn saved_settings(db: &rusqlite::Connection) -> Result<BTreeMap<String, String>> {
        let mut query = db.prepare("SELECT key,value FROM app_settings WHERE key IN (?,?)")?;
        Ok(query
            .query_map([FAVORITES_KEY, GROUPS_KEY], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub(crate) fn from_settings(settings: &BTreeMap<String, String>) -> Result<Option<Self>> {
        let Some(value) = settings
            .get(FAVORITES_KEY)
            .filter(|value| !value.is_empty())
        else {
            return Ok(None);
        };
        let mut collection: Self =
            serde_json::from_str(value).context("视频收藏数据损坏，请保留资料目录后修复")?;
        // Old versions rewrite the favorites JSON. Keep group membership in its own setting.
        if let Some(value) = settings.get(GROUPS_KEY) {
            let mut groups: Groups = serde_json::from_str(value).context("视频分组数据损坏")?;
            collection.groups = groups.names;
            for video in &mut collection.items {
                video.group = groups.assignments.remove(&video.id).unwrap_or_default();
            }
        }
        collection.sync_groups();
        collection.validate()?;
        Ok(Some(collection))
    }

    pub fn load(lib: &Library) -> Result<Self> {
        let settings = Self::saved_settings(&lib.db()?)?;
        if let Some(mut c) = Self::from_settings(&settings)? {
            let before = serde_json::to_string(&c)?;
            c.group_parts();
            c.sync_groups();
            if !settings.contains_key(GROUPS_KEY) || serde_json::to_string(&c)? != before {
                c.save(lib)?;
            }
            return Ok(c);
        }
        let legacy = lib.setting("recent_videos")?;
        let paths: Vec<PathBuf> = if legacy.is_empty() {
            vec![]
        } else {
            serde_json::from_str(&legacy).context("旧视频列表损坏，未覆盖")?
        };
        let series = bilibili::Series::load(lib).ok().flatten();
        let mut c = Self::default();
        // Old recents may refer to missing files. Keep them visible and removable.
        for path in paths {
            ensure!(path.is_absolute(), "旧视频路径不是绝对路径");
            if c.items
                .iter()
                .any(|v| normalized(&v.path) == normalized(&path))
            {
                continue;
            }
            let associated = series
                .as_ref()
                .filter(|s| {
                    s.parts.iter().any(|p| {
                        p.path
                            .as_ref()
                            .is_some_and(|p| normalized(p) == normalized(&path))
                    })
                })
                .cloned();
            c.items.push(Video {
                id: id(),
                title: title(&path, associated.as_ref()),
                path,
                series: associated,
                group: String::new(),
            });
        }
        c.selected = c
            .items
            .iter()
            .find(|v| v.path.is_file())
            .map(|v| v.id.clone());
        c.group_parts();
        c.save(lib)?;
        Ok(c)
    }
    /// Collapse legacy per-part bookmarks, preserving the selected part and every cache path.
    fn group_parts(&mut self) {
        let sources: Vec<_> = self.items.iter().filter_map(|v| v.series.clone()).collect();
        for video in &mut self.items {
            if video.series.is_none() {
                video.series = sources
                    .iter()
                    .find(|s| {
                        s.parts.iter().any(|p| {
                            p.path
                                .as_ref()
                                .is_some_and(|p| normalized(p) == normalized(&video.path))
                        })
                    })
                    .cloned();
            }
        }
        let mut grouped: Vec<Video> = Vec::new();
        for mut video in std::mem::take(&mut self.items) {
            if let Some(series) = &video.series {
                video.title = title(&video.path, Some(series));
                if let Some(existing) = grouped
                    .iter_mut()
                    .find(|v| v.series.as_ref().is_some_and(|old| old.base == series.base))
                {
                    if existing.group.is_empty() {
                        existing.group = video.group.clone();
                    }
                    let selected = self.selected.as_ref() == Some(&video.id);
                    if let Some(existing_series) = existing.series.as_mut() {
                        merge_series(existing_series, series);
                    }
                    if selected {
                        existing.path = video.path;
                        self.selected = Some(existing.id.clone());
                    }
                    existing.title = title(&existing.path, existing.series.as_ref());
                    existing.sync_part();
                    continue;
                }
            }
            video.sync_part();
            grouped.push(video);
        }
        self.items = grouped;
    }
    /// Apply background metadata only to an existing source; never recreate removed favorites.
    pub fn update_title(&mut self, base: &str, title: &str) -> Result<bool> {
        let title = title.trim();
        ensure!(!title.is_empty(), "视频标题为空");
        let mut updated = false;
        for video in &mut self.items {
            if let Some(series) = video.series.as_mut().filter(|s| s.base == base) {
                series.title = title.to_owned();
                video.title = title.to_owned();
                updated = true;
            }
        }
        Ok(updated)
    }
    /// Merge a completed background download without selecting a source or changing its current P.
    pub fn cache_part(&mut self, key: &str, index: usize, path: &Path) -> Result<Option<String>> {
        let Some(video) = self.items.iter_mut().find(|v| v.id == key) else {
            return Ok(None);
        };
        ensure!(
            path.is_absolute() && path.is_file() && std::fs::metadata(path)?.len() > 0,
            "后台下载文件不存在或为空"
        );
        let series = video.series.as_mut().context("后台选集信息已改变")?;
        series.parts.get_mut(index).context("后台分P已不存在")?.path = Some(path.to_path_buf());
        Ok(Some(series.base.clone()))
    }
    pub fn cache_series_part(
        &mut self,
        incoming: &bilibili::Series,
        index: usize,
        path: &Path,
    ) -> Result<bool> {
        let Some(video) = self.items.iter_mut().find(|video| {
            video
                .series
                .as_ref()
                .is_some_and(|series| series.base == incoming.base)
        }) else {
            return Ok(false);
        };
        ensure!(
            path.is_absolute() && path.is_file() && std::fs::metadata(path)?.len() > 0,
            "后台下载文件不存在或为空"
        );
        let series = video.series.as_mut().context("后台选集信息已改变")?;
        let mut part = incoming
            .parts
            .get(index)
            .context("后台分P已不存在")?
            .clone();
        part.path = Some(path.to_path_buf());
        let cached = series.cached(index).is_some();
        if let Some(existing) = series.parts.get_mut(index) {
            // Keep the path used by the saved playback position if this P already plays locally.
            if !cached {
                existing.path = part.path;
            }
        } else {
            series.parts.extend(
                incoming.parts[series.parts.len()..index]
                    .iter()
                    .cloned()
                    .map(|mut p| {
                        p.path = None;
                        p
                    }),
            );
            series.parts.push(part);
        }
        Ok(true)
    }
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1 && self.items.len() <= 10000,
            "视频收藏版本或条目数量无效"
        );
        ensure!(self.groups.len() <= 1000, "视频分组数量无效");
        let mut groups = BTreeSet::new();
        for group in &self.groups {
            ensure!(
                valid_group(group) && groups.insert(group.to_lowercase()),
                "视频分组名称无效或重复"
            );
        }
        let mut ids = BTreeSet::new();
        let mut paths = BTreeSet::new();
        for v in &self.items {
            ensure!(
                !v.id.is_empty()
                    && v.path.is_absolute()
                    && !v.title.trim().is_empty()
                    && ids.insert(&v.id)
                    && paths.insert(&v.path),
                "视频收藏包含无效或重复条目"
            );
            ensure!(
                v.group.trim() == v.group
                    && v.group.chars().count() <= 40
                    && (v.group.is_empty() || groups.contains(&v.group.to_lowercase())),
                "收藏引用了无效的视频分组"
            );
            if let Some(s) = &v.series {
                ensure!(
                    !s.parts.is_empty()
                        && s.parts.len() <= 10000
                        && s.requested < s.parts.len()
                        && s.current.is_none_or(|i| i < s.parts.len())
                        && s.parts
                            .iter()
                            .enumerate()
                            .all(|(i, p)| p.number as usize == i + 1),
                    "收藏中的分P信息无效"
                );
                ensure!(
                    crate::process::normalize_bilibili(&s.base)? == s.base,
                    "收藏中的视频来源无效"
                );
            }
        }
        ensure!(
            self.selected.as_ref().is_none_or(|id| ids.contains(id)),
            "收藏中的当前选择不存在"
        );
        Ok(())
    }
    pub fn save(&self, lib: &Library) -> Result<()> {
        let mut db = lib.db()?;
        let tx = db.transaction()?;
        self.save_in(&tx)?;
        tx.commit()?;
        Ok(())
    }

    // The caller saves both settings in the same transaction, including during restore and sync.
    pub(crate) fn save_in(&self, db: &rusqlite::Connection) -> Result<()> {
        self.validate()?;
        let mut favorites = self.clone();
        let groups = Groups {
            names: std::mem::take(&mut favorites.groups),
            assignments: favorites
                .items
                .iter_mut()
                .filter(|video| !video.group.is_empty())
                .map(|video| (video.id.clone(), std::mem::take(&mut video.group)))
                .collect(),
        };
        for (key, value) in [
            (FAVORITES_KEY, serde_json::to_string(&favorites)?),
            (GROUPS_KEY, serde_json::to_string(&groups)?),
        ] {
            db.execute(
                "INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value WHERE value<>excluded.value",
                rusqlite::params![key, value],
            )?;
        }
        Ok(())
    }
    /// Validate the whole import before changing the collection; preserve input ordering.
    pub fn add(
        &mut self,
        paths: &[PathBuf],
        series: Option<&bilibili::Series>,
    ) -> Result<Vec<String>> {
        self.add_in_group(paths, series, "")
    }
    /// Import new videos into one group; existing favorites keep their current group.
    pub fn add_in_group(
        &mut self,
        paths: &[PathBuf],
        series: Option<&bilibili::Series>,
        group: &str,
    ) -> Result<Vec<String>> {
        let group = if group == UNGROUPED { "" } else { group.trim() };
        ensure!(
            group.is_empty() || (group != UNGROUPED && group.chars().count() <= 40),
            "视频分组名称无效"
        );
        ensure!(!paths.is_empty(), "请选择视频文件");
        for path in paths {
            ensure!(
                path.is_file() && std::fs::metadata(path)?.len() > 0,
                "视频文件不存在或为空：{}",
                path.display()
            );
        }
        let mut next = self.clone();
        if !group.is_empty() && !next.groups.iter().any(|name| name == group) {
            next.groups.push(group.to_owned());
        }
        let mut added = Vec::new();
        for input in paths {
            let path = normalized(input);
            ensure!(path.is_absolute(), "视频路径无效");
            let associated = series
                .filter(|s| {
                    s.parts.iter().any(|p| {
                        p.path
                            .as_ref()
                            .is_some_and(|p| normalized(p) == normalized(&path))
                    })
                })
                .cloned();
            if let Some(existing) = next.items.iter_mut().find(|v| {
                normalized(&v.path) == path
                    || associated
                        .as_ref()
                        .is_some_and(|s| v.series.as_ref().is_some_and(|old| old.base == s.base))
            }) {
                if let Some(series) = associated {
                    if let Some(old) = &mut existing.series {
                        merge_series(old, &series);
                    } else {
                        existing.series = Some(series);
                    }
                    existing.title = title(&path, existing.series.as_ref());
                }
                if normalized(&existing.path) != path {
                    existing.path = path;
                }
                existing.sync_part();
                if !added.contains(&existing.id) {
                    added.push(existing.id.clone());
                }
                continue;
            }
            let key = id();
            next.items.push(Video {
                id: key.clone(),
                title: title(&path, associated.as_ref()),
                path,
                series: associated,
                group: group.to_owned(),
            });
            added.push(key);
        }
        // Also absorb legacy local records that now have a known source.
        let old_ids: Vec<_> = added
            .iter()
            .filter_map(|id| {
                next.items
                    .iter()
                    .find(|v| &v.id == id)
                    .map(|video| (id.clone(), video.path.clone()))
            })
            .collect();
        next.group_parts();
        added = old_ids
            .iter()
            .filter_map(|(_, path)| {
                next.items
                    .iter()
                    .find(|v| v.contains(path))
                    .map(|v| v.id.clone())
            })
            .collect();
        added.dedup();
        for (_, path) in old_ids {
            if let Some(video) = next.items.iter_mut().find(|v| v.contains(&path)) {
                video.path = path;
                video.sync_part();
            }
        }
        next.sync_groups();
        next.validate()?;
        *self = next;
        Ok(added)
    }
    pub fn has_group(&self, group: &str) -> bool {
        !group.is_empty()
            && group != UNGROUPED
            && (self.groups.iter().any(|name| name == group)
                || self.items.iter().any(|video| video.group == group))
    }
    fn has_group_ci(&self, group: &str) -> bool {
        self.groups
            .iter()
            .any(|name| name.eq_ignore_ascii_case(group))
            || self
                .items
                .iter()
                .any(|video| video.group.eq_ignore_ascii_case(group))
    }
    pub fn group_count(&self, group: &str) -> usize {
        self.items
            .iter()
            .filter(|video| matches_group(video, group))
            .count()
    }
    pub fn create_group(&mut self, name: &str) -> Result<String> {
        let name = group_name(name)?;
        ensure!(!self.has_group_ci(&name), "视频分组已存在");
        self.groups.push(name.clone());
        Ok(name)
    }
    pub fn rename_group(&mut self, old: &str, name: &str) -> Result<String> {
        ensure!(self.has_group(old), "视频分组不存在");
        let name = group_name(name)?;
        ensure!(
            name.eq_ignore_ascii_case(old) || !self.has_group_ci(&name),
            "视频分组已存在"
        );
        if let Some(group) = self.groups.iter_mut().find(|group| *group == old) {
            *group = name.clone();
        } else {
            self.groups.push(name.clone());
        }
        for video in &mut self.items {
            if video.group == old {
                video.group = name.clone();
            }
        }
        Ok(name)
    }
    pub fn remove_group(&mut self, group: &str) -> Result<usize> {
        ensure!(self.has_group(group), "视频分组不存在");
        self.groups.retain(|name| name != group);
        let mut moved = 0;
        for video in &mut self.items {
            if video.group == group {
                video.group.clear();
                moved += 1;
            }
        }
        Ok(moved)
    }
    pub fn set_group(&mut self, key: &str, group: &str) -> Result<()> {
        let group = if group == UNGROUPED { "" } else { group.trim() };
        ensure!(group.is_empty() || self.has_group(group), "视频分组不存在");
        if !group.is_empty() && !self.groups.iter().any(|name| name == group) {
            self.groups.push(group.to_owned());
        }
        let video = self
            .items
            .iter_mut()
            .find(|video| video.id == key)
            .context("该视频已移出收藏")?;
        video.group = group.to_owned();
        Ok(())
    }
    /// Fill missing group names from videos and normalize duplicate spellings.
    pub(crate) fn sync_groups(&mut self) {
        let mut groups = Vec::new();
        let mut canonical = BTreeMap::new();
        for group in std::mem::take(&mut self.groups) {
            let group = group.trim().to_owned();
            if group.is_empty() || group == UNGROUPED {
                continue;
            }
            let key = group.to_lowercase();
            if let std::collections::btree_map::Entry::Vacant(entry) = canonical.entry(key) {
                entry.insert(group.clone());
                groups.push(group);
            }
        }
        for video in &mut self.items {
            video.group = video.group.trim().to_owned();
            if video.group == UNGROUPED {
                video.group.clear();
            }
            if video.group.is_empty() {
                continue;
            }
            let key = video.group.to_lowercase();
            if let Some(existing) = canonical.get(&key) {
                video.group = existing.clone();
            } else {
                canonical.insert(key, video.group.clone());
                groups.push(video.group.clone());
            }
        }
        self.groups = groups;
    }
    pub fn select(&mut self, key: &str) -> Result<Video> {
        let video = self
            .items
            .iter_mut()
            .find(|v| v.id == key)
            .context("该视频已移出收藏")?;
        let path = video
            .playable_path()
            .context("视频文件已移动或删除，请重新导入；可直接移除此收藏")?;
        video.path = path;
        video.sync_part();
        ensure!(
            video.path.is_file() && std::fs::metadata(&video.path)?.len() > 0,
            "视频文件已移动或删除，请重新导入；可直接移除此收藏"
        );
        let video = video.clone();
        self.selected = Some(key.into());
        Ok(video)
    }
    pub fn active(&self) -> Option<&Video> {
        self.selected
            .as_ref()
            .and_then(|id| self.items.iter().find(|v| &v.id == id))
    }
    pub fn remove(&mut self, key: &str) -> Result<Video> {
        let index = self
            .items
            .iter()
            .position(|v| v.id == key)
            .context("该视频已移出收藏")?;
        let video = self.items.remove(index);
        if self.selected.as_deref() == Some(key) {
            self.selected = None;
        }
        Ok(video)
    }
}
pub(crate) fn valid_group(name: &str) -> bool {
    name.trim() == name && !name.is_empty() && name.chars().count() <= 40 && name != UNGROUPED
}

fn group_name(name: &str) -> Result<String> {
    let name = name.trim();
    ensure!(!name.is_empty(), "请输入视频分组名称");
    ensure!(name.chars().count() <= 40, "视频分组名称不能超过 40 个字符");
    ensure!(
        name != UNGROUPED && name != "全部" && name != "未分组",
        "视频分组名称无效"
    );
    Ok(name.to_owned())
}
pub fn matches_group(video: &Video, filter: &str) -> bool {
    if filter.is_empty() {
        true
    } else if filter == UNGROUPED {
        video.group.is_empty()
    } else {
        video.group == filter
    }
}
impl Video {
    pub fn contains(&self, path: &Path) -> bool {
        normalized(&self.path) == normalized(path)
            || self.series.as_ref().is_some_and(|s| {
                s.parts.iter().any(|p| {
                    p.path
                        .as_ref()
                        .is_some_and(|p| normalized(p) == normalized(path))
                })
            })
    }
    /// Path for reopening the favorite: its current part wins, then the
    /// favorite's own file, then any cached part for an incomplete download.
    pub fn resume_path(&self) -> Option<PathBuf> {
        self.series
            .as_ref()
            .and_then(|series| series.current)
            .and_then(|index| self.series.as_ref()?.cached(index))
            .or_else(|| {
                (self.path.is_file() && std::fs::metadata(&self.path).is_ok_and(|m| m.len() > 0))
                    .then(|| self.path.clone())
            })
            .or_else(|| self.playable_path())
    }
    pub fn playable_path(&self) -> Option<PathBuf> {
        if self.path.is_file() && std::fs::metadata(&self.path).is_ok_and(|m| m.len() > 0) {
            return Some(self.path.clone());
        }
        self.series
            .as_ref()
            .and_then(|s| (0..s.parts.len()).find_map(|i| s.cached(i)))
    }
    fn sync_part(&mut self) {
        if let Some(s) = &mut self.series {
            s.current = s.parts.iter().position(|p| {
                p.path
                    .as_ref()
                    .is_some_and(|p| normalized(p) == normalized(&self.path))
            });
        }
    }
}
fn merge_series(old: &mut bilibili::Series, incoming: &bilibili::Series) {
    // Keep cached paths when metadata fetched again has no downloads yet.
    if incoming.parts.len() > old.parts.len() {
        let mut merged = incoming.clone();
        if merged.title.trim().is_empty() {
            merged.title = old.title.clone();
        }
        for (i, part) in old.parts.iter().enumerate() {
            if merged.parts[i].path.is_none() {
                merged.parts[i].path = part.path.clone();
            }
        }
        *old = merged;
    } else {
        if !incoming.title.trim().is_empty() {
            old.title = incoming.title.clone();
        }
        for (i, part) in incoming.parts.iter().enumerate() {
            if part.path.is_some() {
                old.parts[i].path = part.path.clone();
            }
        }
    }
}
fn title(path: &Path, series: Option<&bilibili::Series>) -> String {
    if let Some(s) = series {
        if !s.title.trim().is_empty() {
            return s.title.trim().to_owned();
        }
        // Offline migration: show a readable downloaded name while source metadata loads.
        let first = s
            .parts
            .iter()
            .find_map(|p| p.path.as_deref())
            .unwrap_or(path);
        let stem = first.file_stem().unwrap_or_default().to_string_lossy();
        let source = s.base.rsplit('/').next().unwrap_or_default();
        if let Some((name, suffix)) = stem.rsplit_once(" [")
            && suffix
                .strip_suffix(']')
                .is_some_and(|id| id == source || id.starts_with(&format!("{source}_p")))
            && !name.trim().is_empty()
        {
            return name.trim().to_owned();
        }
        if !stem.trim().is_empty() {
            return stem.into_owned();
        }
        return "bilibili 视频".into();
    }
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    if let Some(source) = path
        .parent()
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        && let Some((name, suffix)) = stem.rsplit_once(" [")
        && suffix.strip_suffix(']').is_some_and(|id| match source {
            "remote-youtube" => {
                id.len() == 11
                    && id
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
            }
            "remote-douyin" => {
                (15..=25).contains(&id.len()) && id.bytes().all(|byte| byte.is_ascii_digit())
            }
            "remote-x" => !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()),
            "remote-weibo" => {
                !id.is_empty()
                    && id
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            }
            _ => false,
        })
        && !name.trim().is_empty()
    {
        return name.trim().to_owned();
    }
    stem.into_owned()
}
