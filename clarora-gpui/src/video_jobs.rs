//! A serial video subtitle queue independent of the foreground player and task slot.
use crate::{
    bilibili::Series,
    data::Library,
    process::{self, Cancel},
    subtitles::Cue,
    video_subtitles,
};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::sync::atomic::AtomicBool;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, TryLockError, atomic::Ordering, mpsc},
    time::Duration,
};

type Cell = Arc<Mutex<Option<PathBuf>>>;
#[derive(Clone, Default)]
pub struct Downloads(Arc<Mutex<BTreeMap<(String, usize), Cell>>>);
fn playable(path: &Path) -> bool {
    path.is_file() && std::fs::metadata(path).is_ok_and(|m| m.len() > 0)
}
impl Downloads {
    fn cell(&self, series: &Series, index: usize) -> Cell {
        self.0
            .lock()
            .unwrap()
            .entry((series.base.clone(), index))
            .or_default()
            .clone()
    }
    /// Nonblocking: called on the UI thread before starting any download work.
    pub fn cached(&self, series: &Series, index: usize) -> Option<PathBuf> {
        if let Some(path) = series.cached(index) {
            return Some(path);
        }
        self.cell(series, index)
            .try_lock()
            .ok()
            .and_then(|g| g.clone())
            .filter(|p| playable(p))
    }
    pub fn obtain(
        &self,
        lib: &Library,
        series: &Series,
        index: usize,
        cancel: &Cancel,
    ) -> Result<PathBuf> {
        self.obtain_with(series, index, cancel, || {
            process::download(&lib.root, &series.url(index)?, cancel)
        })
    }
    /// The fetch callback supports real filesystem/concurrency E2E without external downloads.
    pub fn obtain_with(
        &self,
        series: &Series,
        index: usize,
        cancel: &Cancel,
        fetch: impl FnOnce() -> Result<PathBuf>,
    ) -> Result<PathBuf> {
        ensure!(index < series.parts.len(), "分P不存在");
        let cell = self.cell(series, index);
        let mut cached = loop {
            ensure!(!cancel.load(Ordering::SeqCst), "已取消");
            match cell.try_lock() {
                Ok(g) => break g,
                Err(TryLockError::WouldBlock) => std::thread::sleep(Duration::from_millis(40)),
                Err(e) => anyhow::bail!("分P下载任务异常：{e}"),
            }
        };
        if let Some(path) = cached
            .as_ref()
            .filter(|p| playable(p))
            .cloned()
            .or_else(|| series.cached(index))
        {
            *cached = Some(path.clone());
            return Ok(path);
        }
        let path = fetch()?;
        ensure!(playable(&path), "下载结果为空或不存在");
        *cached = Some(path.clone());
        Ok(path)
    }
}
#[derive(Clone)]
pub struct Plan {
    pub source_id: String,
    pub series: Option<Series>,
    pub paths: Vec<(usize, String, Option<PathBuf>)>,
    pub skip_existing: bool,
}
impl Plan {
    pub fn all(source_id: String, series: Series) -> Self {
        let paths = series
            .parts
            .iter()
            .enumerate()
            .map(|(i, p)| (i, p.title.clone(), p.path.clone()))
            .collect();
        Self {
            source_id,
            series: Some(series),
            paths,
            skip_existing: true,
        }
    }
    pub fn current(source_id: String, path: PathBuf) -> Self {
        let label = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        Self {
            source_id,
            series: None,
            paths: vec![(0, label, Some(path))],
            skip_existing: false,
        }
    }
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub total: usize,
    pub generated: usize,
    pub skipped: usize,
    pub failed: Vec<(String, String)>,
    pub cancelled: bool,
}
pub enum Event {
    Progress(String),
    Downloaded {
        source_id: String,
        index: usize,
        path: PathBuf,
    },
    Ready {
        source_id: String,
        path: PathBuf,
        subtitle: String,
        cues: Vec<Cue>,
    },
    Finished(Report),
}
pub struct Job {
    pub done: Arc<AtomicBool>,
    pub source_id: String,
    pub cancel: Cancel,
    pub events: mpsc::Receiver<Event>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}
impl Job {
    pub fn start(lib: Library, plan: Plan, downloads: Downloads) -> Self {
        let (tx, events) = mpsc::channel();
        let cancel = process::cancel_token();
        let token = cancel.clone();
        let source_id = plan.source_id.clone();
        let done = Arc::new(AtomicBool::new(false));
        let completion = done.clone();
        std::thread::spawn(move || {
            struct Finished(Arc<AtomicBool>);
            impl Drop for Finished {
                fn drop(&mut self) {
                    self.0.store(true, Ordering::SeqCst);
                }
            }
            let _finished = Finished(completion);
            let mut report = Report {
                total: plan.paths.len(),
                generated: 0,
                skipped: 0,
                failed: vec![],
                cancelled: false,
            };
            for (done, (index, title, input)) in plan.paths.iter().enumerate() {
                if token.load(Ordering::SeqCst) {
                    break;
                }
                let label = if plan.series.is_some() {
                    format!("P{} · {title}", index + 1)
                } else {
                    title.clone()
                };
                let progress = |stage: &str| {
                    let _ = tx.send(Event::Progress(format!(
                        "已处理 {done}/{} · {label}\n{stage}",
                        report.total
                    )));
                };
                progress("正在准备视频，未下载的分P会先下载");
                let result = (|| -> Result<bool> {
                    let path = if let Some(series) = &plan.series {
                        let p = downloads.obtain(&lib, series, *index, &token)?;
                        let _ = tx.send(Event::Downloaded {
                            source_id: plan.source_id.clone(),
                            index: *index,
                            path: p.clone(),
                        });
                        p
                    } else {
                        input.clone().context("视频路径不存在")?
                    };
                    ensure!(!token.load(Ordering::SeqCst), "已取消");
                    if plan.skip_existing {
                        let existing = video_subtitles::load(&lib, &path)?;
                        if !existing.is_empty() {
                            progress("已有字幕，跳过");
                            return Ok(false);
                        }
                    }
                    let (subtitle, cues) =
                        video_subtitles::generate_with_progress(&lib, &path, &token, progress)?;
                    let _ = tx.send(Event::Ready {
                        source_id: plan.source_id.clone(),
                        path,
                        subtitle,
                        cues,
                    });
                    Ok(true)
                })();
                match result {
                    Ok(true) => report.generated += 1,
                    Ok(false) => report.skipped += 1,
                    Err(_) if token.load(Ordering::SeqCst) => break,
                    Err(e) => report.failed.push((label, format!("{e:#}"))),
                }
            }
            report.cancelled = token.load(Ordering::SeqCst);
            let _ = tx.send(Event::Finished(report));
        });
        Self {
            source_id,
            cancel,
            events,
            done,
        }
    }
}
