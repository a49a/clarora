//! Task identity, cancellation and delivery. Domains decide what a committed result means.
use crate::process::{self, Cancel};
use anyhow::{Result, ensure};
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Id(u64);
impl Id {
    pub fn value(self) -> u64 {
        self.0
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Scope {
    #[default]
    Material,
    Application,
}
static NEXT: AtomicU64 = AtomicU64::new(1);
enum Packet<U, C> {
    Progress(Id, U),
    Completed(Id, std::result::Result<C, String>),
}
pub struct Reporter<U, C> {
    id: Id,
    tx: mpsc::Sender<Packet<U, C>>,
}
impl<U, C> Clone for Reporter<U, C> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            tx: self.tx.clone(),
        }
    }
}
impl<U, C> Reporter<U, C> {
    pub fn send(&self, update: U) -> std::result::Result<(), ()> {
        self.tx
            .send(Packet::Progress(self.id, update))
            .map_err(|_| ())
    }
}
pub struct Handle<U, C> {
    report: Reporter<U, C>,
    cancel: Cancel,
    terminal: bool,
}
impl<U, C> Handle<U, C> {
    pub fn id(&self) -> Id {
        self.report.id
    }
    pub fn cancel_token(&self) -> Cancel {
        self.cancel.clone()
    }
    pub fn reporter(&self) -> Reporter<U, C> {
        self.report.clone()
    }
    pub fn complete(mut self, result: std::result::Result<C, String>) {
        self.terminal = true;
        let _ = self.report.tx.send(Packet::Completed(self.id(), result));
    }
}
impl<U, C> Drop for Handle<U, C> {
    fn drop(&mut self) {
        if !self.terminal {
            let _ = self.report.tx.send(Packet::Completed(
                self.id(),
                Err("任务未返回完成结果".into()),
            ));
        }
    }
}
pub struct Completion<C> {
    pub id: Id,
    pub scope: Scope,
    pub cancelled: bool,
    pub elapsed: Duration,
    pub result: std::result::Result<C, String>,
}
pub enum Event<U, C> {
    Progress { id: Id, value: U },
    Completed(Completion<C>),
}
struct Active<U, C> {
    id: Id,
    scope: Scope,
    started: Instant,
    cancel: Cancel,
    rx: mpsc::Receiver<Packet<U, C>>,
}
pub struct Lane<U, C> {
    active: Option<Active<U, C>>,
}
impl<U, C> Default for Lane<U, C> {
    fn default() -> Self {
        Self { active: None }
    }
}
impl<U, C> Lane<U, C> {
    pub fn is_running(&self) -> bool {
        self.active.is_some()
    }
    pub fn current_id(&self) -> Option<Id> {
        self.active.as_ref().map(|a| a.id)
    }
    pub fn cancel_token(&self) -> Option<&Cancel> {
        self.active.as_ref().map(|a| &a.cancel)
    }
    pub fn scope(&self) -> Scope {
        self.active.as_ref().map(|a| a.scope).unwrap_or_default()
    }
    pub fn started(&self) -> Option<Instant> {
        self.active.as_ref().map(|a| a.started)
    }
    pub fn cancel(&self) {
        if let Some(a) = &self.active {
            a.cancel.store(true, Ordering::SeqCst);
        }
    }
    pub fn cancel_material(&self) {
        if self.scope() == Scope::Material {
            self.cancel();
        }
    }
    pub fn abort(&mut self) {
        self.cancel();
        self.active = None;
    }
    pub fn begin(&mut self, scope: Scope) -> Result<Handle<U, C>> {
        ensure!(!self.is_running(), "已有任务正在执行");
        let id = Id(NEXT.fetch_add(1, Ordering::Relaxed));
        let cancel = process::cancel_token();
        let (tx, rx) = mpsc::channel();
        self.active = Some(Active {
            id,
            scope,
            started: Instant::now(),
            cancel: cancel.clone(),
            rx,
        });
        Ok(Handle {
            report: Reporter { id, tx },
            cancel,
            terminal: false,
        })
    }
    pub fn spawn<F>(&mut self, scope: Scope, work: F) -> Result<Id>
    where
        U: Send + 'static,
        C: Send + 'static,
        F: FnOnce(Cancel, Reporter<U, C>) -> std::result::Result<C, String> + Send + 'static,
    {
        let handle = self.begin(scope)?;
        let id = handle.id();
        if let Err(error) = std::thread::Builder::new()
            .name(format!("task-{}", id.value()))
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    work(handle.cancel_token(), handle.reporter())
                }))
                .unwrap_or_else(|_| Err("后台任务异常结束，请重试".into()));
                handle.complete(result);
            })
        {
            self.abort();
            return Err(error.into());
        }
        Ok(id)
    }
    pub fn poll(&mut self) -> Option<Event<U, C>> {
        loop {
            let active = self.active.as_ref()?;
            match active.rx.try_recv() {
                Ok(Packet::Progress(id, value)) if id == active.id => {
                    return Some(Event::Progress { id, value });
                }
                Ok(Packet::Completed(id, result)) if id == active.id => {
                    let a = self.active.take().unwrap();
                    return Some(Event::Completed(Completion {
                        id,
                        scope: a.scope,
                        cancelled: a.cancel.load(Ordering::SeqCst),
                        elapsed: a.started.elapsed(),
                        result,
                    }));
                }
                Err(mpsc::TryRecvError::Empty) => return None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    let a = self.active.take().unwrap();
                    return Some(Event::Completed(Completion {
                        id: a.id,
                        scope: a.scope,
                        cancelled: a.cancel.load(Ordering::SeqCst),
                        elapsed: a.started.elapsed(),
                        result: Err("任务连接提前结束".into()),
                    }));
                }
                _ => {}
            }
        }
    }
}
impl<U, C> Drop for Lane<U, C> {
    fn drop(&mut self) {
        self.abort();
    }
}
