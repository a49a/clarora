//! One worker per search view, with only the latest query/result retained.
use crate::{
    data::Library,
    process::{self, Cancel},
    sentence_search::{self, Query, Results},
};
use std::sync::{Arc, Condvar, Mutex, atomic::Ordering};
type Response = std::result::Result<Results, String>;
struct Job {
    generation: u64,
    lib: Library,
    query: Query,
    cancel: Cancel,
}
#[derive(Default)]
struct Mailbox {
    generation: u64,
    pending: Option<Job>,
    completed: Option<(u64, Response)>,
    closed: bool,
}
#[derive(Default)]
struct Shared {
    mailbox: Mutex<Mailbox>,
    ready: Condvar,
}
#[derive(Default)]
pub struct Controller {
    shared: Arc<Shared>,
    generation: u64,
    wanted: Option<u64>,
    cancel: Option<Cancel>,
    started: bool,
}
impl Controller {
    pub fn request(&mut self, lib: &Library, query: Query) {
        self.cancel();
        let generation = self.generation;
        self.wanted = Some(generation);
        let cancel = process::cancel_token();
        self.cancel = Some(cancel.clone());
        {
            let mut mailbox = self
                .shared
                .mailbox
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            mailbox.pending = Some(Job {
                generation,
                lib: lib.clone(),
                query,
                cancel,
            });
        }
        if !self.started {
            let shared = self.shared.clone();
            match std::thread::Builder::new()
                .name("clarora-sentence-search".into())
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
                    mailbox.completed = Some((generation, Err(format!("无法启动搜索：{error}"))));
                }
            }
        }
        self.shared.ready.notify_one();
    }
    pub fn is_loading(&self) -> bool {
        self.wanted.is_some()
    }
    pub fn poll(&mut self) -> Option<Response> {
        let (generation, result) = self
            .shared
            .mailbox
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .completed
            .take()?;
        if self.wanted != Some(generation) {
            return None;
        }
        self.wanted = None;
        self.cancel = None;
        Some(result)
    }
    pub fn cancel(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel.store(true, Ordering::SeqCst);
        }
        self.generation = self.generation.wrapping_add(1);
        self.wanted = None;
        let mut mailbox = self
            .shared
            .mailbox
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        mailbox.generation = self.generation;
        mailbox.pending = None;
        mailbox.completed = None;
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        self.cancel();
        self.shared
            .mailbox
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .closed = true;
        self.shared.ready.notify_one();
    }
}
fn worker(shared: Arc<Shared>) {
    loop {
        let job = {
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
            sentence_search::search(&job.lib, &job.query, &job.cancel)
        }))
        .map_err(|_| "搜索线程异常，请重试".to_string())
        .and_then(|r| r.map_err(|e| format!("{e:#}")));
        let mut mailbox = shared.mailbox.lock().unwrap_or_else(|e| e.into_inner());
        if mailbox.closed {
            return;
        }
        if mailbox.generation == job.generation && !job.cancel.load(Ordering::SeqCst) {
            mailbox.completed = Some((job.generation, result));
        }
    }
}
