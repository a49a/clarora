//! Bounded, cancellable library scans. No network requests or synthetic timings.
pub use crate::sentence_audition::Audition;
pub use crate::sentence_search_worker::Controller;
use crate::{
    audio_tools,
    data::{Library, text},
    process::Cancel,
    subtitles::{self, Cue},
    video_library, video_subtitles, video_transcript,
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::atomic::Ordering,
};
pub const MAX_RESULTS: usize = 2000;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatchMode {
    Prefix,
    Contains,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scope {
    All,
    Audio,
    Video,
}
#[derive(Clone, Debug)]
pub struct Query {
    pub phrase: String,
    pub mode: MatchMode,
    pub scope: Scope,
}
#[derive(Clone, Debug, Serialize)]
pub struct Hit {
    pub kind: Scope,
    pub label: String,
    pub group: String,
    pub path: PathBuf,
    pub cue: Cue,
    pub english: String,
    pub translation: String,
}
#[derive(Default, Serialize)]
pub struct Results {
    pub hits: Vec<Hit>,
    pub total: usize,
    pub scanned: usize,
    pub without_subtitles: usize,
    pub unavailable: usize,
    pub problems: Vec<String>,
}
struct Source {
    kind: Scope,
    part: u32,
    label: String,
    group: String,
    path: PathBuf,
    subtitle: Option<PathBuf>,
}
fn tokens(text: &str) -> Vec<String> {
    text.replace('’', "'")
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '\'')
        .map(|s| s.trim_matches('\'').to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}
fn check(cancel: &Cancel) -> Result<()> {
    ensure!(!cancel.load(Ordering::SeqCst), "已取消搜索");
    Ok(())
}
fn problem(results: &mut Results, message: String) {
    if results.problems.len() < 20 {
        results.problems.push(message.chars().take(400).collect());
    }
}
fn registered(path: &Path, kind: Scope, seen: &mut BTreeSet<(u8, PathBuf)>) -> bool {
    seen.insert((
        if kind == Scope::Audio { 0 } else { 1 },
        path.canonicalize().unwrap_or_else(|_| path.to_path_buf()),
    ))
}
pub fn search(lib: &Library, query: &Query, cancel: &Cancel) -> Result<Results> {
    let words = tokens(&query.phrase);
    ensure!(!words.is_empty(), "请输入英文单词或短语，例如 When is");
    ensure!(query.phrase.chars().count() <= 120, "句型最多 120 个字符");
    check(cancel)?;
    let mut results = Results::default();
    let mut sources = vec![];
    let mut seen = BTreeSet::new();
    if query.scope != Scope::Video {
        let groups: BTreeMap<_, _> = lib
            .list("listening_practices")?
            .iter()
            .map(|r| (text(r, "id"), text(r, "name")))
            .collect();
        let audios = lib.list("listening_audios")?;
        let merges = audio_tools::Catalog::load(lib, &audios)?;
        for row in &audios {
            check(cancel)?;
            if merges.hidden(&text(row, "id")) {
                continue;
            }
            let path = PathBuf::from(text(row, "audio_uri"));
            if !registered(&path, Scope::Audio, &mut seen) {
                continue;
            }
            sources.push(Source {
                kind: Scope::Audio,
                part: 0,
                label: text(row, "name"),
                group: groups
                    .get(&text(row, "practice_id"))
                    .cloned()
                    .unwrap_or_default(),
                path,
                subtitle: Some(PathBuf::from(text(row, "subtitle_uri"))),
            });
        }
    }
    if query.scope != Scope::Audio {
        match video_library::Collection::load(lib) {
            Ok(collection) => {
                for video in collection.items {
                    check(cancel)?;
                    if let Some(series) = video.series {
                        for part in series.parts {
                            check(cancel)?;
                            if let Some(path) = part.path {
                                if registered(&path, Scope::Video, &mut seen) {
                                    sources.push(Source {
                                        kind: Scope::Video,
                                        part: part.number,
                                        label: format!("P{} · {}", part.number, part.title),
                                        group: video.title.clone(),
                                        path,
                                        subtitle: None,
                                    });
                                }
                            } else {
                                results.unavailable += 1;
                            }
                        }
                    } else if registered(&video.path, Scope::Video, &mut seen) {
                        sources.push(Source {
                            kind: Scope::Video,
                            part: 0,
                            label: video.title,
                            group: String::new(),
                            path: video.path,
                            subtitle: None,
                        });
                    }
                }
            }
            Err(error) => problem(&mut results, format!("视频库读取失败：{error:#}")),
        }
    }
    sources.sort_by(|a, b| {
        (a.kind as u8, &a.group, a.part, &a.label, &a.path).cmp(&(
            b.kind as u8,
            &b.group,
            b.part,
            &b.label,
            &b.path,
        ))
    });
    for source in sources {
        check(cancel)?;
        if !fs::metadata(&source.path).is_ok_and(|meta| meta.is_file() && meta.len() > 0) {
            results.unavailable += 1;
            continue;
        }
        results.scanned += 1;
        let loaded = (|| -> Result<Vec<Cue>> {
            if let Some(path) = &source.subtitle {
                if path.as_os_str().is_empty() || !path.is_file() {
                    return Ok(vec![]);
                }
                ensure!(
                    fs::metadata(path)?.len() <= 16 * 1024 * 1024,
                    "字幕文件超过 16 MiB"
                );
                let body = fs::read_to_string(path)?;
                // A plain text transcript has no reliable listening boundaries.
                Ok(if body.contains("-->") {
                    subtitles::parse(&body)
                } else {
                    vec![]
                })
            } else {
                video_subtitles::load(lib, &source.path)
            }
        })();
        let cues = match loaded {
            Ok(cues) => cues,
            Err(error) => {
                problem(&mut results, format!("{}：{error:#}", source.label));
                continue;
            }
        };
        if cues.is_empty() {
            results.without_subtitles += 1;
            continue;
        }
        let mut reported_long_cue = false;
        for cue in cues {
            check(cancel)?;
            if cue.text.len() > 16 * 1024 {
                if !reported_long_cue {
                    problem(
                        &mut results,
                        format!("{}：已跳过超过 16 KiB 的单条字幕，请修正切句", source.label),
                    );
                    reported_long_cue = true;
                }
                continue;
            }
            if !cue.start.is_finite()
                || !cue.end.is_finite()
                || cue.start < 0.
                || cue.end <= cue.start
            {
                continue;
            }
            let english = video_transcript::Transcript::from_cues(std::slice::from_ref(&cue)).text;
            let candidate = tokens(&english);
            let matched = match query.mode {
                MatchMode::Prefix => candidate.starts_with(&words),
                MatchMode::Contains => candidate.windows(words.len()).any(|w| w == words),
            };
            if matched {
                results.total += 1;
                if results.hits.len() < MAX_RESULTS {
                    let (_, translation) = subtitles::languages(std::slice::from_ref(&cue));
                    results.hits.push(Hit {
                        kind: source.kind,
                        label: source.label.clone(),
                        group: source.group.clone(),
                        path: source.path.clone(),
                        cue,
                        english,
                        translation,
                    });
                }
            }
        }
    }
    Ok(results)
}
