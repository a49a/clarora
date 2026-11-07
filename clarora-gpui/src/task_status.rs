//! Presentation-independent task notices read by task cards.
use std::time::Instant;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum TaskState {
    Running,
    Cancelling,
    Succeeded,
    Failed,
    Cancelled,
}
pub(super) struct TaskNotice {
    pub title: String,
    pub detail: String,
    pub state: TaskState,
    pub started: Instant,
    pub elapsed: u64,
}
impl TaskNotice {
    pub fn new(title: &str, detail: &str) -> Self {
        Self {
            title: title.into(),
            detail: detail.into(),
            state: TaskState::Running,
            started: Instant::now(),
            elapsed: 0,
        }
    }
    pub fn tick(&mut self) -> bool {
        let elapsed = self.started.elapsed().as_secs();
        if self.active() && self.elapsed != elapsed {
            self.elapsed = elapsed;
            true
        } else {
            false
        }
    }
    pub fn active(&self) -> bool {
        matches!(self.state, TaskState::Running | TaskState::Cancelling)
    }
}
