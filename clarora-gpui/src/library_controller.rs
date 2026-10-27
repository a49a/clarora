//! Bounded background reads. UI code only requests and accepts complete, current snapshots.
use crate::{
    ai, audio_library, clip_marks,
    data::{Card, Library, Row, text},
    runtime,
    subtitles::{self, Cue},
    view_data,
};
use anyhow::{Context, Result};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs,
    path::Path,
    sync::{Arc, Condvar, Mutex},
};

/// Every choice that changes the meaning of a refresh participates in identity.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Key {
    pub page: String,
    pub practice_id: String,
    pub card_filter: String,
    pub due: bool,
    pub saved_only: bool,
    pub selected_audio_id: String,
    pub selected_subtitle_uri: String,
    pub library_selected: String,
    pub library_filter: u8,
    pub library_sort: u8,
    pub video_path: String,
}

pub struct Snapshot {
    pub key: Key,
    pub practice_id: String,
    pub practices: Vec<Row>,
    /// Cross-group management and group switching still consume the same full audio catalog.
    pub audios: Vec<Row>,
    /// Other pages do not scan and allocate the entire card collection.
    pub cards: Option<Vec<Card>>,
    pub cues: Option<Vec<Cue>>,
    pub audio_key_points: Option<Vec<ai::KeyPoint>>,
    pub restore_audio: Option<(String, f64)>,
    pub video_cues: Option<Vec<Cue>>,
    /// Only selected/preview/subtitle_count/subtitle_ids are presentation data;
    /// rename state, tools, and checked selection remain owned by the view.
    pub audio_library: audio_library::State,
    pub clip_catalog: clip_marks::Catalog,
    pub data: view_data::State,
    pub runtime: Option<runtime::Report>,
    pub discover_due: Option<BTreeSet<String>>,
    pub discover_favorites: Option<BTreeSet<String>>,
}

fn read_cues(path: &str) -> Result<Vec<Cue>> {
    if path.is_empty() {
        return Ok(vec![]);
    }
    match fs::read_to_string(path) {
        Ok(body) => Ok(subtitles::parse(&body)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(error) => Err(error).with_context(|| format!("读取字幕失败：{path}")),
    }
}

fn segment(cards: &mut Vec<Card>, row: &Row, cues: &[Cue]) {
    if let (Some(a), Some(b)) = (cues.first(), cues.last()) {
        if b.end - a.start >= 3. {
            let key = format!("segment:{}:{}", text(row, "id"), a.start);
            if cards.iter().any(|c| c.kind == "clip" && c.id == key) {
                return;
            }
            let (front, back) = subtitles::languages(cues);
            cards.push(Card {
                kind: "clip".into(),
                id: key,
                front,
                back,
                media: text(row, "audio_uri"),
                start: a.start,
                end: b.end,
            });
        }
    }
}

/// Blocking work, intentionally separate from the controller's nonblocking UI API.
/// A failure returns no replacement snapshot. Missing subtitle files mean no subtitles.
pub fn load(lib: &Library, key: &Key, include_runtime: bool) -> Result<Snapshot> {
    let practices = lib.list("listening_practices")?;
    let practice_id = if practices.iter().any(|r| text(r, "id") == key.practice_id) {
        key.practice_id.clone()
    } else {
        practices.first().map(|r| text(r, "id")).unwrap_or_default()
    };
    let audios = lib.list("listening_audios")?;
    let mut audio_library = audio_library::State {
        selected: key.library_selected.clone(),
        filter: key.library_filter,
        sort: key.library_sort,
        ..Default::default()
    };
    // This legacy helper performs file probes and reads; it is safe here, on the worker.
    audio_library.sync(&practice_id, &audios);
    // Unlike its optional preview fallback, report a real read failure atomically.
    if let Some(row) = audios
        .iter()
        .find(|r| text(r, "id") == audio_library.selected)
    {
        let preview = read_cues(&text(row, "subtitle_uri"))?;
        audio_library.subtitle_count = preview.len();
        audio_library.preview = preview.into_iter().take(5).collect();
    }
    let selected = audios
        .iter()
        .find(|r| text(r, "id") == key.selected_audio_id);
    let cues = if key.selected_audio_id.is_empty() {
        None
    } else {
        Some(read_cues(
            &selected
                .map(|r| text(r, "subtitle_uri"))
                .unwrap_or_default(),
        )?)
    };
    let audio_key_points = if key.selected_audio_id.is_empty() {
        None
    } else {
        Some(
            serde_json::from_str(&lib.setting(&format!("key_points_{}", key.selected_audio_id))?)
                .unwrap_or_default(),
        )
    };
    let last = lib.setting("last_listening_audio")?;
    let restore_audio = if audios.iter().any(|r| text(r, "id") == last) {
        let saved = serde_json::from_str::<Value>(&lib.setting("last_listening_position")?)
            .unwrap_or(Value::Null);
        let position = if saved["id"].as_str() == Some(last.as_str()) {
            saved["position"]
                .as_f64()
                .filter(|p| p.is_finite() && *p >= 0.)
                .unwrap_or(0.)
        } else {
            0.
        };
        Some((last, position))
    } else {
        None
    };
    let video_cues = if key.video_path.is_empty() {
        None
    } else {
        let path = Path::new(&key.video_path);
        match fs::metadata(path) {
            Ok(_) => Some(crate::video_subtitles::load(lib, path)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if crate::bilibili::source_identity(path).is_some() {
                    Some(crate::video_subtitles::load(lib, path)?)
                } else {
                    Some(vec![])
                }
            }
            Err(error) => return Err(error).context("读取视频信息失败"),
        }
    };
    let mut discover_due = None;
    let mut discover_favorites = None;
    let filter = if key.card_filter.is_empty() {
        "all"
    } else {
        &key.card_filter
    };
    let cards = if key.page == "flashcard" {
        Some(lib.cards_for_kind(key.due, filter)?)
    } else if key.page == "discover" {
        let favorites = lib.list("discover_favorites")?;
        discover_favorites = Some(favorites.iter().map(|r| text(r, "key")).collect());
        discover_due = Some(
            lib.cards(true)?
                .into_iter()
                .map(|c| format!("{}:{}", c.kind, c.id))
                .collect(),
        );
        let mut cards = if key.saved_only {
            favorites
                .into_iter()
                .filter_map(|r| {
                    let key = text(&r, "key");
                    let (kind, id) = key.split_once(':')?;
                    let payload: Value = serde_json::from_str(&text(&r, "payload")).ok()?;
                    Some(Card {
                        kind: kind.into(),
                        id: id.into(),
                        front: payload["front"].as_str().unwrap_or("").into(),
                        back: payload["back"].as_str().unwrap_or("").into(),
                        media: payload["audio"]["uri"].as_str().unwrap_or("").into(),
                        start: payload["audio"]["startMs"].as_f64().unwrap_or(0.) / 1000.,
                        end: payload["audio"]["endMs"].as_f64().unwrap_or(0.) / 1000.,
                    })
                })
                .collect::<Vec<_>>()
        } else {
            let mut cards = lib.cards(false)?;
            cards.retain(|c| c.kind != "video" && c.kind != "sentence");
            if filter == "all" || filter == "clip" {
                for row in &audios {
                    let mut group: Vec<Cue> = vec![];
                    for cue in read_cues(&text(row, "subtitle_uri"))? {
                        if !group.is_empty() && cue.end - group[0].start > 60. {
                            segment(&mut cards, row, &group);
                            group.clear();
                        }
                        group.push(cue.clone());
                        if cue.end - group[0].start >= 20.
                            && cue.text.ends_with(['.', '!', '?', '。', '！', '？'])
                        {
                            segment(&mut cards, row, &group);
                            group.clear();
                        }
                    }
                    segment(&mut cards, row, &group);
                }
            }
            cards
        };
        if filter != "all" {
            cards.retain(|c| c.kind == filter);
        }
        Some(cards)
    } else {
        None
    };
    let clip_catalog = clip_marks::Catalog::load(lib)?;
    let mut data = view_data::State::default();
    data.reload(lib, &audios)?;
    let runtime = include_runtime.then(runtime::inspect);
    Ok(Snapshot {
        key: key.clone(),
        practice_id,
        practices,
        audios,
        cards,
        cues,
        audio_key_points,
        restore_audio,
        video_cues,
        audio_library,
        clip_catalog,
        data,
        runtime,
        discover_due,
        discover_favorites,
    })
}

struct Request {
    generation: u64,
    lib: Library,
    key: Key,
    include_runtime: bool,
}
struct Completed {
    generation: u64,
    key: Key,
    result: std::result::Result<Snapshot, String>,
}
#[derive(Default)]
struct Mailbox {
    closed: bool,
    pending: Option<Request>,
    completed: Option<Completed>,
}
#[derive(Default)]
struct Shared {
    mailbox: Mutex<Mailbox>,
    ready: Condvar,
}

/// One worker, one replaceable pending request, and one result slot. Refreshes
/// never wait for file/SQLite work and cannot accumulate threads or snapshots.
pub struct Controller {
    shared: Arc<Shared>,
    generation: u64,
    wanted: Option<Key>,
    started: bool,
}
impl Default for Controller {
    fn default() -> Self {
        Self {
            shared: Arc::default(),
            generation: 0,
            wanted: None,
            started: false,
        }
    }
}
impl Controller {
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn is_loading(&self) -> bool {
        self.wanted.is_some()
    }
    pub fn request(&mut self, lib: &Library, key: Key, include_runtime: bool) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.wanted = Some(key.clone());
        let generation = self.generation;
        {
            let mut mailbox = self
                .shared
                .mailbox
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            mailbox.completed = None;
            mailbox.pending = Some(Request {
                generation,
                lib: lib.clone(),
                key: key.clone(),
                include_runtime,
            });
        }
        if !self.started {
            let shared = self.shared.clone();
            match std::thread::Builder::new()
                .name("clarora-library-data".into())
                .spawn(move || worker(shared))
            {
                Ok(_) => self.started = true,
                Err(error) => {
                    let mut mailbox = self
                        .shared
                        .mailbox
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    mailbox.pending = None;
                    mailbox.completed = Some(Completed {
                        generation,
                        key,
                        result: Err(format!("启动资料读取失败：{error}")),
                    });
                }
            }
        }
        self.shared.ready.notify_one();
        generation
    }
    /// A changed page/material invalidates the old response even without a new request.
    pub fn poll(&mut self, current: &Key) -> Option<std::result::Result<Snapshot, String>> {
        if self.wanted.as_ref().is_some_and(|wanted| wanted != current) {
            self.invalidate();
            return None;
        }
        let result = self
            .shared
            .mailbox
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .completed
            .take()?;
        if result.generation != self.generation
            || result.key != *current
            || self.wanted.as_ref() != Some(current)
        {
            return None;
        }
        self.wanted = None;
        Some(result.result)
    }
    pub fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.wanted = None;
        let mut mailbox = self
            .shared
            .mailbox
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        mailbox.pending = None;
        mailbox.completed = None;
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        let mut mailbox = self
            .shared
            .mailbox
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        mailbox.closed = true;
        mailbox.pending = None;
        mailbox.completed = None;
        self.shared.ready.notify_one();
        // In-flight OS I/O may still be finishing; never join it on the UI thread.
    }
}
fn worker(shared: Arc<Shared>) {
    loop {
        let request = {
            let mut mailbox = shared.mailbox.lock().unwrap_or_else(|e| e.into_inner());
            while mailbox.pending.is_none() && !mailbox.closed {
                mailbox = shared
                    .ready
                    .wait(mailbox)
                    .unwrap_or_else(|e| e.into_inner());
            }
            if mailbox.closed {
                return;
            }
            mailbox.pending.take().unwrap()
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            load(&request.lib, &request.key, request.include_runtime)
        }))
        .map_err(|_| "资料读取线程异常，保留上次内容".to_string())
        .and_then(|result| result.map_err(|e| format!("{e:#}")));
        let mut mailbox = shared.mailbox.lock().unwrap_or_else(|e| e.into_inner());
        if mailbox.closed {
            return;
        }
        // A later queued request supersedes this result before it reaches the UI.
        if mailbox
            .pending
            .as_ref()
            .is_some_and(|pending| pending.generation != request.generation)
        {
            continue;
        }
        mailbox.completed = Some(Completed {
            generation: request.generation,
            key: request.key,
            result,
        });
    }
}
