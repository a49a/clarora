//! Non-blocking material chat repository. Only the worker thread touches SQLite.
use crate::{
    chat_store::{self, Session, State},
    data::Library,
};
use anyhow::{Result, anyhow, ensure};
use std::{
    collections::BTreeMap,
    ops::{Deref, DerefMut},
    sync::{Arc, Condvar, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

const DEBOUNCE: Duration = Duration::from_millis(300);
const RETRY: Duration = Duration::from_secs(3);

#[derive(Clone)]
struct Draft {
    context: String,
    text: String,
    revision: u64,
    ready: Instant,
    persisted: bool,
}

#[derive(Clone)]
enum Kind {
    Context,
    Select { id: String, clear: Option<Draft> },
    Refresh { id: String, running: bool },
}

#[derive(Clone)]
struct Command {
    generation: u64,
    context: String,
    before: Option<Draft>,
    kind: Kind,
}

#[derive(Default)]
struct Inbox {
    drafts: BTreeMap<String, Draft>,
    command: Option<Command>,
    barriers: Vec<mpsc::Sender<std::result::Result<(), String>>>,
    closing: bool,
}

type Shared = Arc<(Mutex<Inbox>, Condvar)>;

struct Loaded {
    sessions: Vec<Session>,
    selected: Option<Session>,
    draft: Option<String>,
}

enum Reply {
    Loaded(Command, std::result::Result<Loaded, String>),
    Saved(Draft),
    Failed(String),
}

/// Apply `draft` to the question editor only when present. Errors are already
/// deduplicated, so a failed SQLite retry does not flood the notification area.
#[derive(Default)]
pub struct Update {
    pub changed: bool,
    pub draft: Option<String>,
    pub errors: Vec<String>,
}

pub struct Controller {
    state: State,
    inbox: Shared,
    replies: mpsc::Receiver<Reply>,
    drafts: BTreeMap<String, Draft>,
    generation: u64,
    revision: u64,
    pending: Option<Command>,
    retry_after: Option<(String, Instant)>,
    reported_error: String,
}

impl Deref for Controller {
    type Target = State;
    fn deref(&self) -> &State {
        &self.state
    }
}
impl DerefMut for Controller {
    fn deref_mut(&mut self) -> &mut State {
        &mut self.state
    }
}

impl Controller {
    pub fn new(lib: Library) -> Self {
        let inbox = Shared::default();
        let (tx, replies) = mpsc::channel();
        let worker_inbox = inbox.clone();
        thread::spawn(move || worker(lib, worker_inbox, tx));
        Self {
            state: State::default(),
            inbox,
            replies,
            drafts: BTreeMap::new(),
            generation: 0,
            revision: 0,
            pending: None,
            retry_after: None,
            reported_error: String::new(),
        }
    }

    pub fn is_loading(&self) -> bool {
        self.pending.is_some()
    }

    /// Editor mutations and explicit new-session clears advance this version;
    /// storage acknowledgements and context loads do not.
    pub fn draft_revision(&self) -> u64 {
        self.revision
    }

    pub fn requested_context(&self) -> &str {
        self.pending
            .as_ref()
            .map_or(&self.state.context, |c| &c.context)
    }

    fn next_revision(&mut self) -> u64 {
        self.revision = self.revision.wrapping_add(1);
        self.revision
    }

    fn queue_draft(&self, draft: Draft) {
        let (mutex, notify) = &*self.inbox;
        let mut inbox = mutex.lock().unwrap_or_else(|e| e.into_inner());
        if inbox
            .drafts
            .get(&draft.context)
            .is_none_or(|d| d.revision <= draft.revision)
        {
            inbox.drafts.insert(draft.context.clone(), draft);
        }
        notify.notify_one();
    }

    /// Called from input/tick. This never opens SQLite or waits for the worker.
    pub fn observe_draft(&mut self, text: &str) {
        if self.state.draft == text {
            return;
        }
        let context = if self.state.context.is_empty() {
            match self.pending.as_ref() {
                Some(command) if !text.is_empty() => command.context.clone(),
                _ => return,
            }
        } else {
            self.state.context.clone()
        };
        let draft = Draft {
            context,
            text: text.into(),
            revision: self.next_revision(),
            ready: Instant::now() + DEBOUNCE,
            persisted: false,
        };
        self.state.draft = text.into();
        self.drafts.insert(draft.context.clone(), draft.clone());
        self.queue_draft(draft);
    }

    fn schedule(&mut self, context: &str, kind: Kind) -> Result<()> {
        self.generation = self.generation.wrapping_add(1);
        let before = self.drafts.get(&self.state.context).cloned();
        let command = Command {
            generation: self.generation,
            context: context.into(),
            before,
            kind,
        };
        let (mutex, notify) = &*self.inbox;
        let mut inbox = mutex.lock().unwrap_or_else(|e| e.into_inner());
        ensure!(!inbox.closing, "聊天仓储正在关闭");
        // One latest intent, rather than an unbounded sequence of stale loads.
        inbox.command = Some(command.clone());
        self.pending = Some(command);
        notify.notify_one();
        Ok(())
    }

    pub fn switch_context(&mut self, context: &str, editor: &str) -> Result<bool> {
        self.observe_draft(editor);
        if self.requested_context() == context {
            return Ok(false);
        }
        if self
            .retry_after
            .as_ref()
            .is_some_and(|(failed, after)| failed == context && Instant::now() < *after)
        {
            return Ok(false);
        }
        self.state.invalidate_request();
        self.schedule(context, Kind::Context)?;
        Ok(true)
    }

    pub fn select(&mut self, id: &str, editor: &str) -> Result<()> {
        ensure!(!self.state.context.is_empty(), "请先载入材料会话");
        ensure!(
            self.requested_context() == self.state.context,
            "材料会话正在读取"
        );
        self.observe_draft(editor);
        self.state.invalidate_request();
        let clear = if id.is_empty() {
            Some(Draft {
                context: self.state.context.clone(),
                text: String::new(),
                revision: self.next_revision(),
                ready: Instant::now(),
                persisted: true,
            })
        } else {
            None
        };
        self.schedule(
            &self.state.context.clone(),
            Kind::Select {
                id: id.into(),
                clear,
            },
        )
    }

    pub fn refresh(&mut self) -> Result<()> {
        if self.pending.is_some() || self.state.context.is_empty() {
            return Ok(());
        }
        self.schedule(
            &self.state.context.clone(),
            Kind::Refresh {
                id: self.state.id.clone(),
                running: self.state.running,
            },
        )
    }

    fn report(&mut self, error: String, update: &mut Update) {
        self.state.error = error.clone();
        if self.reported_error != error {
            self.reported_error = error.clone();
            update.errors.push(error);
            update.changed = true;
        }
    }

    fn prune_saved_drafts(&mut self) {
        let context = &self.state.context;
        let requested = self.pending.as_ref().map(|c| c.context.as_str());
        self.drafts.retain(|key, draft| {
            !draft.persisted || key == context || Some(key.as_str()) == requested
        });
    }

    pub fn poll(&mut self) -> Update {
        let mut update = Update::default();
        while let Ok(reply) = self.replies.try_recv() {
            match reply {
                Reply::Failed(error) => self.report(error, &mut update),
                Reply::Saved(draft) => {
                    if let Some(local) = self
                        .drafts
                        .get_mut(&draft.context)
                        .filter(|d| d.revision == draft.revision)
                    {
                        local.persisted = true;
                        self.state.error.clear();
                        self.reported_error.clear();
                    }
                    self.prune_saved_drafts();
                }
                Reply::Loaded(command, loaded) => {
                    if self
                        .pending
                        .as_ref()
                        .is_none_or(|p| p.generation != command.generation)
                    {
                        continue;
                    }
                    self.pending = None;
                    let loaded = match loaded {
                        Ok(loaded) => loaded,
                        Err(error) => {
                            self.retry_after =
                                Some((command.context.clone(), Instant::now() + RETRY));
                            self.report(error, &mut update);
                            continue;
                        }
                    };
                    self.state.error.clear();
                    self.reported_error.clear();
                    self.retry_after = None;
                    self.state.sessions = loaded.sessions;
                    match command.kind {
                        Kind::Context => {
                            self.state.context = command.context.clone();
                            let draft = if let Some(local) = self.drafts.get(&command.context) {
                                local.text.clone()
                            } else {
                                loaded.draft.unwrap_or_default()
                            };
                            self.state.draft = draft.clone();
                            self.state.loaded(loaded.selected);
                            update.draft = Some(draft);
                        }
                        Kind::Select { clear, .. } => {
                            self.state.loaded(loaded.selected);
                            if let Some(clear) = clear {
                                if let Some(newer) = self
                                    .drafts
                                    .get(&clear.context)
                                    .filter(|d| d.revision > clear.revision)
                                    .cloned()
                                {
                                    // A new keystroke may have been persisted before the clear
                                    // finished. Requeue it so clear can never win that race.
                                    self.queue_draft(newer);
                                } else {
                                    self.state.draft.clear();
                                    self.drafts.insert(clear.context.clone(), clear);
                                    update.draft = Some(String::new());
                                }
                            }
                        }
                        Kind::Refresh { id, running } => {
                            // Streaming may have started after this read was queued.
                            if !running
                                && self.state.id == id
                                && !id.is_empty()
                                && !self.state.running
                            {
                                if loaded.selected.is_none() {
                                    self.state.invalidate_request();
                                }
                                self.state.loaded(loaded.selected);
                            }
                        }
                    }
                    self.prune_saved_drafts();
                    update.changed = true;
                }
            }
        }
        update
    }

    /// Expedite pending drafts without waiting on a SQLite lock on the UI thread.
    pub fn flush(&self) {
        let (mutex, notify) = &*self.inbox;
        let mut inbox = mutex.lock().unwrap_or_else(|e| e.into_inner());
        for draft in inbox.drafts.values_mut() {
            draft.ready = Instant::now();
        }
        notify.notify_one();
    }

    /// Return immediately; a close hook may await the acknowledgement off-thread.
    pub fn flush_async(&self) -> mpsc::Receiver<std::result::Result<(), String>> {
        let (tx, rx) = mpsc::channel();
        let (mutex, notify) = &*self.inbox;
        let mut inbox = mutex.lock().unwrap_or_else(|e| e.into_inner());
        inbox.barriers.push(tx);
        notify.notify_one();
        rx
    }

    /// For background shutdown hooks and E2E only; never call this from a UI tick.
    pub fn flush_timeout(&self, timeout: Duration) -> Result<()> {
        let rx = self.flush_async();
        rx.recv_timeout(timeout)
            .map_err(|e| anyhow!("等待聊天草稿保存超时：{e}"))?
            .map_err(|e| anyhow!(e))
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        let (mutex, notify) = &*self.inbox;
        mutex.lock().unwrap_or_else(|e| e.into_inner()).closing = true;
        notify.notify_one();
    }
}

fn write(lib: &Library, draft: &Draft) -> Result<()> {
    lib.set_setting(&format!("chat_draft_{}", draft.context), &draft.text)
}

fn write_latest(lib: &Library, draft: &Draft, written: &mut BTreeMap<String, u64>) -> Result<()> {
    if written
        .get(&draft.context)
        .is_some_and(|revision| *revision >= draft.revision)
    {
        return Ok(());
    }
    write(lib, draft)?;
    written.insert(draft.context.clone(), draft.revision);
    Ok(())
}

fn load(lib: &Library, command: &Command) -> Result<Loaded> {
    let sessions = chat_store::list(lib, &command.context)?;
    let (selected, draft) = match &command.kind {
        Kind::Context => (
            sessions.first().cloned(),
            Some(lib.setting(&format!("chat_draft_{}", command.context))?),
        ),
        Kind::Select { id, .. } if id.is_empty() => (None, None),
        Kind::Select { id, .. } => (
            Some(
                chat_store::get(lib, &command.context, id)?
                    .ok_or_else(|| anyhow!("当前材料中没有这条会话，请刷新后重试"))?,
            ),
            None,
        ),
        Kind::Refresh { id, running } => {
            let selected = if id.is_empty() || *running {
                None
            } else if let Some(session) = sessions.iter().find(|s| &s.id == id) {
                Some(session.clone())
            } else {
                chat_store::get(lib, &command.context, id)?
            };
            (selected, None)
        }
    };
    Ok(Loaded {
        sessions,
        selected,
        draft,
    })
}

fn saved(shared: &Shared, draft: &Draft) {
    let mut inbox = shared.0.lock().unwrap_or_else(|e| e.into_inner());
    if inbox
        .drafts
        .get(&draft.context)
        .is_some_and(|d| d.revision <= draft.revision)
    {
        inbox.drafts.remove(&draft.context);
    }
}

fn retry(shared: &Shared, mut draft: Draft) {
    let mut inbox = shared.0.lock().unwrap_or_else(|e| e.into_inner());
    if !inbox.closing
        && inbox
            .drafts
            .get(&draft.context)
            .is_none_or(|d| d.revision <= draft.revision)
    {
        draft.ready = Instant::now() + RETRY;
        inbox.drafts.insert(draft.context.clone(), draft);
    }
}

fn worker(lib: Library, shared: Shared, replies: mpsc::Sender<Reply>) {
    let mut written = BTreeMap::new();
    loop {
        let (command, drafts, barriers, closing) = {
            let (mutex, notify) = &*shared;
            let mut inbox = mutex.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                let now = Instant::now();
                let urgent = inbox.closing || !inbox.barriers.is_empty();
                if inbox.command.is_some()
                    || urgent
                    || inbox.drafts.values().any(|d| d.ready <= now)
                {
                    break;
                }
                if let Some(deadline) = inbox.drafts.values().map(|d| d.ready).min() {
                    inbox = notify
                        .wait_timeout(inbox, deadline.saturating_duration_since(now))
                        .unwrap_or_else(|e| e.into_inner())
                        .0;
                } else {
                    inbox = notify.wait(inbox).unwrap_or_else(|e| e.into_inner());
                }
            }
            let command = inbox.command.take();
            let urgent = inbox.closing || !inbox.barriers.is_empty();
            let now = Instant::now();
            let keys: Vec<_> = inbox
                .drafts
                .iter()
                .filter(|(_, d)| urgent || (command.is_none() && d.ready <= now))
                .map(|(key, _)| key.clone())
                .collect();
            let drafts: Vec<_> = keys
                .into_iter()
                .filter_map(|key| inbox.drafts.remove(&key))
                .collect();
            (
                command,
                drafts,
                std::mem::take(&mut inbox.barriers),
                inbox.closing,
            )
        };
        let mut failure = None;
        if let Some(command) = command {
            let result = (|| -> Result<Loaded> {
                if let Some(before) = &command.before {
                    if let Err(error) = write_latest(&lib, before, &mut written) {
                        retry(&shared, before.clone());
                        return Err(error);
                    }
                    saved(&shared, before);
                    let _ = replies.send(Reply::Saved(before.clone()));
                }
                let loaded = load(&lib, &command)?;
                if let Kind::Select {
                    clear: Some(clear), ..
                } = &command.kind
                {
                    write_latest(&lib, clear, &mut written)?;
                    saved(&shared, clear);
                }
                Ok(loaded)
            })()
            .map_err(|e| format!("读取或保存材料会话失败，当前内容已保留：{e:#}"));
            if let Err(error) = &result {
                failure = Some(error.clone());
            }
            let _ = replies.send(Reply::Loaded(command, result));
        }
        for draft in drafts {
            match write_latest(&lib, &draft, &mut written) {
                Ok(()) => {
                    let _ = replies.send(Reply::Saved(draft));
                }
                Err(error) => {
                    let error = format!("问题草稿尚未保存，输入已保留并将重试：{error:#}");
                    failure = Some(error.clone());
                    let _ = replies.send(Reply::Failed(error));
                    retry(&shared, draft);
                }
            }
        }
        for barrier in barriers {
            let _ = barrier.send(failure.clone().map_or(Ok(()), Err));
        }
        if closing {
            return;
        }
    }
}
