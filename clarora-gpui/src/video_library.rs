//! Persistent video bookmarks, independent of the transient player and recent-file order.
use crate::{
    bilibili,
    data::{Library, id},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
const KEY: &str = "video_favorites_v1";
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Video {
    pub id: String,
    pub path: PathBuf,
    pub title: String,
    pub series: Option<bilibili::Series>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Collection {
    version: u8,
    pub items: Vec<Video>,
    pub selected: Option<String>,
}
impl Default for Collection {
    fn default() -> Self {
        Self {
            version: 1,
            items: vec![],
            selected: None,
        }
    }
}
fn normalized(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}
impl Collection {
    pub fn load(lib: &Library) -> Result<Self> {
        let value = lib.setting(KEY)?;
        if !value.is_empty() {
            let mut c: Self =
                serde_json::from_str(&value).context("视频收藏数据损坏，请保留资料目录后修复")?;
            c.validate()?;
            c.group_parts();
            if serde_json::to_string(&c)? != value {
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
                    let selected = self.selected.as_ref() == Some(&video.id);
                    merge_series(existing.series.as_mut().unwrap(), series);
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
    fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1 && self.items.len() <= 10000,
            "视频收藏版本或条目数量无效"
        );
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
        self.validate()?;
        lib.set_setting(KEY, &serde_json::to_string(self)?)
    }
    /// Validate the whole import before changing the collection; preserve input ordering.
    pub fn add(
        &mut self,
        paths: &[PathBuf],
        series: Option<&bilibili::Series>,
    ) -> Result<Vec<String>> {
        ensure!(!paths.is_empty(), "请选择视频文件");
        for path in paths {
            ensure!(
                path.is_file() && std::fs::metadata(path)?.len() > 0,
                "视频文件不存在或为空：{}",
                path.display()
            );
        }
        let mut next = self.clone();
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
            });
            added.push(key);
        }
        // Also absorb legacy local records that now have a known source.
        let old_ids: Vec<_> = added
            .iter()
            .map(|id| {
                (
                    id.clone(),
                    next.items
                        .iter()
                        .find(|v| &v.id == id)
                        .unwrap()
                        .path
                        .clone(),
                )
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
        next.validate()?;
        *self = next;
        Ok(added)
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
        if let Some((name, suffix)) = stem.rsplit_once(" [") {
            if suffix
                .strip_suffix(']')
                .is_some_and(|id| id == source || id.starts_with(&format!("{source}_p")))
                && !name.trim().is_empty()
            {
                return name.trim().to_owned();
            }
        }
        if !stem.trim().is_empty() {
            return stem.into_owned();
        }
        return "bilibili 视频".into();
    }
    path.file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}
