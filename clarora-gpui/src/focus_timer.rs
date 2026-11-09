//! Local focus sessions. A goal is a milestone; only an explicit finish records a round.
use crate::data::{Library, id};
use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Local};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

pub const KEY: &str = "focus_timer_v1";
pub const GOAL_PRESETS_MINUTES: [u64; 12] = [5, 10, 15, 20, 25, 30, 35, 40, 45, 50, 55, 60];
const DEFAULT_GOAL_MS: u64 = 25 * 60_000;
const TRAINING_ROUNDS: usize = 3;
const BREAK_MS: u64 = 5 * 60_000;
const CHECKPOINT_MS: u64 = 15_000;
const RECENT_ROUNDS: usize = 8;

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
enum Phase {
    #[default]
    Ready,
    Awaiting,
    Focus {
        elapsed_ms: u64,
        running_since: Option<u64>,
        reminded: bool,
        strict: bool,
        distractions: u32,
    },
    Rest {
        remaining_ms: u64,
        deadline_ms: Option<u64>,
    },
}

#[derive(Clone, Serialize, Deserialize)]
struct State {
    version: u8,
    goal_ms: u64,
    #[serde(flatten)]
    phase: Phase,
}
impl Default for State {
    fn default() -> Self {
        Self {
            version: 2,
            goal_ms: DEFAULT_GOAL_MS,
            phase: Phase::Ready,
        }
    }
}
impl State {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 2 && (1000..=3_600_000).contains(&self.goal_ms),
            "专注计时记录格式无效或版本不支持"
        );
        match self.phase {
            Phase::Focus {
                running_since,
                strict,
                ..
            } => ensure!(!strict || running_since.is_some(), "电脑专注状态无效"),
            Phase::Rest {
                remaining_ms,
                deadline_ms,
            } => ensure!(
                (1..=BREAK_MS).contains(&remaining_ms) && deadline_ms != Some(0),
                "休息状态无效"
            ),
            _ => {}
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct Legacy {
    #[serde(default)]
    strict: bool,
    focus: bool,
    paused: bool,
    duration_ms: u64,
    remaining_ms: u64,
    deadline_ms: Option<u64>,
    #[serde(default)]
    awaiting_focus: bool,
}
impl Legacy {
    fn migrate(self, now: u64) -> Result<State> {
        ensure!(
            (1000..=3_600_000).contains(&self.duration_ms)
                && (1..=self.duration_ms).contains(&self.remaining_ms)
                && !(self.paused && self.deadline_ms.is_some())
                && self.deadline_ms != Some(0)
                && (!self.strict || self.focus && self.deadline_ms.is_some())
                && (!self.awaiting_focus
                    || self.focus && self.paused && self.remaining_ms == self.duration_ms),
            "旧专注计时记录无效"
        );
        let remaining = self.deadline_ms.map_or(self.remaining_ms, |deadline| {
            deadline.saturating_sub(now).min(self.duration_ms)
        });
        let phase = if self.awaiting_focus {
            Phase::Awaiting
        } else if !self.focus {
            Phase::Rest {
                remaining_ms: self.remaining_ms,
                deadline_ms: self.deadline_ms,
            }
        } else if self.paused || self.deadline_ms.is_some() {
            Phase::Focus {
                elapsed_ms: self.duration_ms - remaining,
                running_since: None,
                reminded: remaining == 0,
                strict: false,
                distractions: 0,
            }
        } else {
            Phase::Ready
        };
        Ok(State {
            version: 2,
            goal_ms: if matches!(phase, Phase::Focus { .. }) {
                self.duration_ms
            } else {
                DEFAULT_GOAL_MS
            },
            phase,
        })
    }
}

#[derive(Clone, Serialize)]
pub struct Round {
    pub id: String,
    pub ended_ms: u64,
    pub goal_ms: u64,
    pub actual_ms: u64,
    pub distractions: u32,
    pub fatigue: Option<u8>,
    pub review_pending: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Recommendation {
    NeedMore,
    Stay,
    IncreaseTo(u64),
    AtMaximum,
}

#[derive(Default)]
pub struct Timer {
    state: State,
    saved: String,
    recent: Vec<Round>,
    comparable: Vec<Round>,
}

pub struct Loaded {
    pub timer: Timer,
    pub warning: Option<String>,
    pub completed: Option<Event>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    GoalReached,
    BreakEnded,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Toggle {
    Started,
    Paused,
    Resumed,
    Completed(Event),
}

impl Timer {
    pub fn load(lib: &Library, now: u64) -> Result<Loaded> {
        let saved = lib.setting(KEY)?;
        let parsed = (|| -> Result<State> {
            if saved.is_empty() {
                return Ok(State::default());
            }
            ensure!(saved.len() <= 4096, "专注计时记录过大");
            let value: serde_json::Value = serde_json::from_str(&saved)?;
            let mut state: State = if value["version"] == 1 {
                serde_json::from_value::<Legacy>(value)?.migrate(now)?
            } else {
                serde_json::from_value(value)?
            };
            state.validate()?;
            if let Phase::Focus {
                running_since,
                strict,
                ..
            } = &mut state.phase
            {
                // Resume from the last checkpoint. Time while the app was closed is not study time.
                *running_since = None;
                *strict = false;
            }
            Ok(state)
        })();
        let (state, mut warning) = match parsed {
            Ok(state) => (state, None),
            Err(error) => (
                State::default(),
                Some(format!("无法恢复专注计时：{error}；可以开始新一轮")),
            ),
        };
        let db = lib.db()?;
        let recent = read_rounds(&db, None, RECENT_ROUNDS)?;
        let comparable = read_rounds(&db, Some(state.goal_ms), TRAINING_ROUNDS)?;
        let mut timer = Self {
            state,
            saved,
            recent,
            comparable,
        };
        let completed = match timer.tick(lib, now) {
            Ok(event) => event,
            Err(error) => {
                warning = Some(format!("恢复休息状态失败，保留原记录并稍后重试：{error:#}"));
                None
            }
        };
        Ok(Loaded {
            timer,
            warning,
            completed,
        })
    }

    pub fn focus(&self) -> bool {
        !matches!(self.state.phase, Phase::Rest { .. })
    }
    pub fn active_round(&self) -> bool {
        matches!(self.state.phase, Phase::Focus { .. })
    }
    pub fn paused(&self) -> bool {
        matches!(
            self.state.phase,
            Phase::Focus {
                running_since: None,
                ..
            } | Phase::Rest {
                deadline_ms: None,
                ..
            }
        )
    }
    pub fn is_running(&self) -> bool {
        matches!(
            self.state.phase,
            Phase::Focus {
                running_since: Some(_),
                ..
            } | Phase::Rest {
                deadline_ms: Some(_),
                ..
            }
        )
    }
    pub fn strict(&self) -> bool {
        matches!(self.state.phase, Phase::Focus { strict: true, .. })
    }
    pub fn awaiting_focus(&self) -> bool {
        matches!(self.state.phase, Phase::Awaiting)
    }
    pub fn goal_seconds(&self) -> u64 {
        self.state.goal_ms / 1000
    }
    pub fn goal_met(&self, now: u64) -> bool {
        self.active_round() && self.elapsed_ms(now) >= self.state.goal_ms
    }
    pub fn distractions(&self) -> u32 {
        match self.state.phase {
            Phase::Focus { distractions, .. } => distractions,
            _ => 0,
        }
    }
    pub fn recent(&self) -> &[Round] {
        &self.recent
    }
    pub fn recommendation(&self) -> Recommendation {
        let current = self.goal_seconds();
        let next = if current == 45 * 60 {
            Some(60 * 60)
        } else {
            GOAL_PRESETS_MINUTES
                .iter()
                .map(|minutes| minutes * 60)
                .find(|&seconds| seconds > current)
        };
        let Some(next) = next else {
            return Recommendation::AtMaximum;
        };
        let rounds = &self.comparable;
        if rounds.len() < TRAINING_ROUNDS || rounds.iter().any(|r| r.fatigue.is_none()) {
            Recommendation::NeedMore
        } else if rounds
            .iter()
            .all(|r| r.actual_ms >= r.goal_ms && r.distractions <= 1 && r.fatigue == Some(1))
        {
            Recommendation::IncreaseTo(next)
        } else {
            Recommendation::Stay
        }
    }
    fn elapsed_ms(&self, now: u64) -> u64 {
        match self.state.phase {
            Phase::Focus {
                elapsed_ms,
                running_since,
                ..
            } => elapsed_ms
                .saturating_add(running_since.map_or(0, |start| now.saturating_sub(start))),
            _ => 0,
        }
    }
    pub fn display_seconds(&self, now: u64) -> u64 {
        if let Phase::Rest {
            remaining_ms,
            deadline_ms,
        } = self.state.phase
        {
            deadline_ms
                .map_or(remaining_ms, |end| {
                    end.saturating_sub(now).min(remaining_ms)
                })
                .div_ceil(1000)
        } else {
            self.elapsed_ms(now) / 1000
        }
    }

    pub fn set_goal(&mut self, lib: &Library, seconds: u64) -> Result<()> {
        ensure!(!self.active_round(), "请结束本轮后再调整最低目标");
        ensure!((1..=3600).contains(&seconds), "最低目标必须为 1 至 3600 秒");
        let mut next = self.state.clone();
        next.goal_ms = seconds * 1000;
        self.commit(lib, next, None)
    }

    pub fn start_strict(&mut self, lib: &Library, now: u64, seconds: u64) -> Result<()> {
        ensure!(!self.active_round(), "请继续当前专注，或结束本轮后再开始");
        ensure!((1..=3600).contains(&seconds), "最低目标必须为 1 至 3600 秒");
        let mut next = self.state.clone();
        next.goal_ms = seconds * 1000;
        next.phase = Self::new_focus(now, true);
        self.commit(lib, next, None)
    }
    pub fn resume_strict(&mut self, lib: &Library, now: u64) -> Result<()> {
        ensure!(self.active_round(), "请先开始一轮专注");
        let mut next = self.state.clone();
        if let Phase::Focus {
            running_since,
            strict,
            ..
        } = &mut next.phase
        {
            *running_since = Some(running_since.unwrap_or(now));
            *strict = true;
        }
        self.commit(lib, next, None)
    }
    fn new_focus(now: u64, strict: bool) -> Phase {
        Phase::Focus {
            elapsed_ms: 0,
            running_since: Some(now),
            reminded: false,
            strict,
            distractions: 0,
        }
    }
    pub fn release_strict(&mut self, lib: &Library) -> Result<()> {
        let mut next = self.state.clone();
        if let Phase::Focus { strict, .. } = &mut next.phase {
            *strict = false;
            self.commit(lib, next, None)?;
        }
        Ok(())
    }
    pub fn reset(&mut self, lib: &Library) -> Result<()> {
        self.commit(
            lib,
            State {
                phase: Phase::Ready,
                ..self.state.clone()
            },
            None,
        )
    }
    pub fn toggle(&mut self, lib: &Library, now: u64) -> Result<Toggle> {
        let mut next = self.state.clone();
        let event = match &mut next.phase {
            Phase::Ready | Phase::Awaiting => {
                next.phase = Self::new_focus(now, false);
                Toggle::Started
            }
            Phase::Focus {
                elapsed_ms,
                running_since,
                strict,
                ..
            } => {
                *strict = false;
                if running_since.take().is_some() {
                    *elapsed_ms = self.elapsed_ms(now);
                    Toggle::Paused
                } else {
                    *running_since = Some(now);
                    Toggle::Resumed
                }
            }
            Phase::Rest {
                remaining_ms,
                deadline_ms,
            } => {
                if let Some(end) = deadline_ms.take() {
                    if now >= end {
                        next.phase = Phase::Awaiting;
                        Toggle::Completed(Event::BreakEnded)
                    } else {
                        *remaining_ms = end.saturating_sub(now).min(*remaining_ms);
                        Toggle::Paused
                    }
                } else {
                    *deadline_ms =
                        Some(now.checked_add(*remaining_ms).context("休息时间超出范围")?);
                    Toggle::Resumed
                }
            }
        };
        self.commit(lib, next, None)?;
        Ok(event)
    }
    pub fn interrupt(&mut self, lib: &Library, now: u64, distracted: bool) -> Result<()> {
        let mut next = self.state.clone();
        if let Phase::Focus {
            elapsed_ms,
            running_since,
            strict,
            distractions,
            ..
        } = &mut next.phase
        {
            *elapsed_ms = self.elapsed_ms(now);
            *running_since = None;
            *strict = false;
            *distractions = distractions.saturating_add(u32::from(distracted));
            self.commit(lib, next, None)?;
        }
        Ok(())
    }
    pub fn record_distraction(&mut self, lib: &Library) -> Result<()> {
        let mut next = self.state.clone();
        let Phase::Focus { distractions, .. } = &mut next.phase else {
            anyhow::bail!("请先开始专注");
        };
        *distractions = distractions.saturating_add(1);
        self.commit(lib, next, None)
    }
    pub fn tick(&mut self, lib: &Library, now: u64) -> Result<Option<Event>> {
        match self.state.phase {
            Phase::Rest {
                deadline_ms: Some(end),
                ..
            } if now >= end => {
                self.commit(
                    lib,
                    State {
                        phase: Phase::Awaiting,
                        ..self.state.clone()
                    },
                    None,
                )?;
                Ok(Some(Event::BreakEnded))
            }
            Phase::Focus {
                running_since: Some(start),
                reminded,
                ..
            } => {
                let elapsed = self.elapsed_ms(now);
                let reached = !reminded && elapsed >= self.state.goal_ms;
                if reached || now.saturating_sub(start) >= CHECKPOINT_MS {
                    let mut next = self.state.clone();
                    if let Phase::Focus {
                        elapsed_ms,
                        running_since,
                        reminded,
                        ..
                    } = &mut next.phase
                    {
                        *elapsed_ms = elapsed;
                        *running_since = Some(now);
                        *reminded |= reached;
                    }
                    self.commit(lib, next, None)?;
                }
                Ok(reached.then_some(Event::GoalReached))
            }
            _ => Ok(None),
        }
    }
    pub fn finish(&mut self, lib: &Library, now: u64) -> Result<()> {
        ensure!(self.active_round(), "当前没有可结束的专注");
        let round = Round {
            id: id(),
            ended_ms: now,
            goal_ms: self.state.goal_ms,
            actual_ms: self.elapsed_ms(now),
            distractions: self.distractions(),
            fatigue: None,
            review_pending: true,
        };
        let next = State {
            phase: Phase::Rest {
                remaining_ms: BREAK_MS,
                deadline_ms: Some(now.checked_add(BREAK_MS).context("休息时间超出范围")?),
            },
            ..self.state.clone()
        };
        self.commit(lib, next, Some(round))
    }
    pub fn review(&mut self, lib: &Library, id: &str, fatigue: Option<u8>) -> Result<()> {
        ensure!(fatigue.is_none_or(|n| (1..=3).contains(&n)), "疲劳等级无效");
        let round = self
            .recent
            .iter_mut()
            .find(|round| round.id == id)
            .context("专注记录已更新")?;
        ensure!(lib.db()?.execute(
            "UPDATE focus_sessions SET fatigue=COALESCE(?,fatigue), review_pending=0 WHERE id=?",
            params![fatigue, id],
        )? == 1, "专注记录已更新");
        round.review_pending = false;
        if fatigue.is_some() {
            round.fatigue = fatigue;
        }
        if let Some(sample) = self.comparable.iter_mut().find(|sample| sample.id == id) {
            *sample = round.clone();
        }
        Ok(())
    }
    fn commit(&mut self, lib: &Library, next: State, round: Option<Round>) -> Result<()> {
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
        tx.execute(
            "INSERT INTO app_settings(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![KEY, serialized],
        )?;
        if let Some(round) = &round {
            tx.execute(
                "INSERT INTO focus_sessions(id,ended_ms,goal_ms,actual_ms,distractions) VALUES(?,?,?,?,?)",
                params![round.id, round.ended_ms, round.goal_ms, round.actual_ms, round.distractions],
            )?;
            if round.actual_ms >= round.goal_ms {
                let day = DateTime::from_timestamp_millis(i64::try_from(round.ended_ms)?)
                    .context("计时日期无效")?
                    .with_timezone(&Local)
                    .format("%Y-%m-%d")
                    .to_string();
                tx.execute("INSERT INTO study_stats(day,pomodoro_count) VALUES(?,1) ON CONFLICT(day) DO UPDATE SET pomodoro_count=pomodoro_count+1",[day])?;
            }
        }
        let comparable = if next.goal_ms != self.state.goal_ms {
            Some(read_rounds(&tx, Some(next.goal_ms), TRAINING_ROUNDS)?)
        } else {
            None
        };
        tx.commit()?;
        if let Some(comparable) = comparable {
            self.comparable = comparable;
        }
        self.state = next;
        self.saved = serialized;
        if let Some(round) = round {
            self.comparable.insert(0, round.clone());
            self.comparable.truncate(TRAINING_ROUNDS);
            self.recent.insert(0, round);
            self.recent.truncate(RECENT_ROUNDS);
        }
        Ok(())
    }
}

fn read_rounds(db: &Connection, goal_ms: Option<u64>, limit: usize) -> Result<Vec<Round>> {
    let mut query = db.prepare(
        "SELECT id,ended_ms,goal_ms,actual_ms,distractions,fatigue,review_pending
         FROM focus_sessions WHERE (?1 IS NULL OR goal_ms=?1) ORDER BY rowid DESC LIMIT ?2",
    )?;
    let rows = query.query_map(params![goal_ms, limit as u32], |r| {
        Ok(Round {
            id: r.get(0)?,
            ended_ms: r.get(1)?,
            goal_ms: r.get(2)?,
            actual_ms: r.get(3)?,
            distractions: r.get(4)?,
            fatigue: r.get(5)?,
            review_pending: r.get(6)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}
