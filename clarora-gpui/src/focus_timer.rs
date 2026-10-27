//! Durable local timer. A committed deadline survives even an ungraceful exit.
use crate::data::Library;
use anyhow::{Result, ensure};
use chrono::{DateTime, Local};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
pub const KEY: &str = "focus_timer_v1";

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
#[derive(Clone, Serialize, Deserialize)]
struct State {
    version: u8,
    focus: bool,
    paused: bool,
    duration_ms: u64,
    remaining_ms: u64,
    deadline_ms: Option<u64>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            version: 1,
            focus: true,
            paused: false,
            duration_ms: 1_500_000,
            remaining_ms: 1_500_000,
            deadline_ms: None,
        }
    }
}
impl State {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1
                && (1..=3_600_000).contains(&self.duration_ms)
                && (1..=self.duration_ms).contains(&self.remaining_ms)
                && !(self.paused && self.deadline_ms.is_some())
                && self.deadline_ms != Some(0),
            "专注计时记录格式无效或版本不支持"
        );
        Ok(())
    }
}
#[derive(Default)]
pub struct Timer {
    state: State,
    saved: String,
    recovery_pending: bool,
}
pub struct Loaded {
    pub timer: Timer,
    pub warning: Option<String>,
    pub completed: Option<bool>,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Toggle {
    Started,
    Paused,
    Resumed,
    Completed(bool),
}
impl Timer {
    pub fn load(lib: &Library, now: u64) -> Result<Loaded> {
        let saved = lib.setting(KEY)?;
        let parsed = (|| -> Result<State> {
            if saved.is_empty() {
                return Ok(State::default());
            }
            ensure!(saved.len() <= 4096, "专注计时记录过大");
            let state: State = serde_json::from_str(&saved)?;
            state.validate()?;
            Ok(state)
        })();
        let (state, warning) = match parsed {
            Ok(s) => (s, None),
            Err(e) => (
                State::default(),
                Some(format!(
                    "无法恢复专注计时：{e}；原记录已保留，可重置后开始新一轮"
                )),
            ),
        };
        let mut timer = Self {
            state,
            saved,
            recovery_pending: true,
        };
        // Never fabricate unattended cycles while the app was closed.
        let (completed, warning) = match timer.tick(lib, now, false) {
            Ok(completed) => {
                timer.recovery_pending = false;
                (completed, warning)
            }
            Err(error) => (
                None,
                Some(format!(
                    "专注计时恢复结算失败，保留原状态并稍后重试：{error:#}"
                )),
            ),
        };
        Ok(Loaded {
            timer,
            warning,
            completed,
        })
    }
    pub fn focus(&self) -> bool {
        self.state.focus
    }
    pub fn paused(&self) -> bool {
        self.state.paused
    }
    pub fn is_running(&self) -> bool {
        self.state.deadline_ms.is_some()
    }
    fn remaining_ms(&self, now: u64) -> u64 {
        self.state
            .deadline_ms
            .map(|end| end.saturating_sub(now).min(self.state.duration_ms))
            .unwrap_or(self.state.remaining_ms)
    }
    pub fn remaining_seconds(&self, now: u64) -> u64 {
        self.remaining_ms(now).div_ceil(1000)
    }
    pub fn start(&mut self, lib: &Library, now: u64, seconds: u64) -> Result<()> {
        ensure!((1..=3600).contains(&seconds), "专注时长必须为1至3600秒");
        let duration_ms = seconds * 1000;
        let deadline_ms = now
            .checked_add(duration_ms)
            .ok_or_else(|| anyhow::anyhow!("计时时间超出范围"))?;
        self.commit(
            lib,
            State {
                version: 1,
                focus: true,
                paused: false,
                duration_ms,
                remaining_ms: duration_ms,
                deadline_ms: Some(deadline_ms),
            },
            None,
        )
    }
    pub fn reset(&mut self, lib: &Library) -> Result<()> {
        self.commit(lib, State::default(), None)
    }
    pub fn toggle(&mut self, lib: &Library, now: u64) -> Result<Toggle> {
        if let Some(focus) = self.tick(lib, now, false)? {
            return Ok(Toggle::Completed(focus));
        }
        let mut next = self.state.clone();
        let event = if self.is_running() {
            next.remaining_ms = self.remaining_ms(now);
            next.deadline_ms = None;
            next.paused = true;
            Toggle::Paused
        } else {
            let event = if next.paused {
                Toggle::Resumed
            } else {
                Toggle::Started
            };
            next.deadline_ms = Some(
                now.checked_add(next.remaining_ms)
                    .ok_or_else(|| anyhow::anyhow!("计时时间超出范围"))?,
            );
            next.paused = false;
            event
        };
        self.commit(lib, next, None)?;
        Ok(event)
    }
    pub fn tick(&mut self, lib: &Library, now: u64, auto_next: bool) -> Result<Option<bool>> {
        let Some(end) = self.state.deadline_ms.filter(|&end| now >= end) else {
            return Ok(None);
        };
        let auto_next = auto_next && !self.recovery_pending;
        let focus = self.state.focus;
        let duration_ms = if focus { 300_000 } else { 1_500_000 };
        let deadline_ms = if auto_next {
            Some(
                now.checked_add(duration_ms)
                    .ok_or_else(|| anyhow::anyhow!("计时时间超出范围"))?,
            )
        } else {
            None
        };
        let next = State {
            version: 1,
            focus: !focus,
            paused: !auto_next,
            duration_ms,
            remaining_ms: duration_ms,
            deadline_ms,
        };
        self.commit(lib, next, focus.then_some(end))?;
        Ok(Some(focus))
    }
    fn commit(&mut self, lib: &Library, next: State, completed: Option<u64>) -> Result<()> {
        next.validate()?;
        let serialized = serde_json::to_string(&next)?;
        let mut db = lib.db()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let saved: Option<String> = tx
            .query_row("SELECT value FROM app_settings WHERE key=?", [KEY], |r| {
                r.get(0)
            })
            .optional()?;
        ensure!(
            saved.as_deref().unwrap_or_default() == self.saved,
            "计时已被另一个实例更新，请重新打开应用后继续"
        );
        tx.execute("INSERT INTO app_settings(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![KEY,serialized])?;
        if let Some(end) = completed {
            let day = DateTime::from_timestamp_millis(end.min(i64::MAX as u64) as i64)
                .ok_or_else(|| anyhow::anyhow!("计时日期无效"))?
                .with_timezone(&Local)
                .format("%Y-%m-%d")
                .to_string();
            tx.execute("INSERT INTO study_stats(day,pomodoro_count) VALUES(?,1) ON CONFLICT(day) DO UPDATE SET pomodoro_count=pomodoro_count+1",[day])?;
        }
        tx.commit()?;
        self.state = next;
        self.saved = serialized;
        self.recovery_pending = false;
        Ok(())
    }
}
