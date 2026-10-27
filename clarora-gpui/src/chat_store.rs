//! Material-scoped chat persistence and UI state. Rendering consumes `State` only.
use crate::data::Library;
use anyhow::{Context, Result, ensure};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use serde_json::{Value, json};

pub const SESSION_LIMIT: usize = 12;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Session {
    pub id: String,
    pub context: String,
    pub title: String,
    pub messages: Vec<Value>,
}

fn session(id: String, payload: String) -> Result<Session> {
    let value: Value = serde_json::from_str(&payload).context("会话内容无法读取")?;
    Ok(Session {
        // SQLite's primary key is authoritative even for older imported payloads.
        id,
        context: value["context"].as_str().unwrap_or_default().into(),
        title: value["title"].as_str().unwrap_or("会话").into(),
        messages: value["messages"].as_array().cloned().unwrap_or_default(),
    })
}

/// Only the newest 12 matching payloads cross the SQLite/Rust boundary.
pub fn list(lib: &Library, context: &str) -> Result<Vec<Session>> {
    let db = lib.db()?;
    let mut statement = db.prepare(
        "SELECT id,payload FROM ai_records WHERE kind='chat' AND
         CASE WHEN json_valid(payload) THEN json_extract(payload,'$.context') END=?1
         ORDER BY created_at DESC,rowid DESC LIMIT 12",
    )?;
    statement
        .query_map([context], |row| Ok((row.get(0)?, row.get(1)?)))?
        .map(|row| {
            let (id, payload) = row?;
            session(id, payload)
        })
        .collect()
}

pub fn get(lib: &Library, context: &str, id: &str) -> Result<Option<Session>> {
    lib.db()?
        .query_row(
            "SELECT id,payload FROM ai_records WHERE id=?1 AND kind='chat' AND
             CASE WHEN json_valid(payload) THEN json_extract(payload,'$.context') END=?2",
            params![id, context],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .map(|(id, payload)| session(id, payload))
        .transpose()
}

pub fn save(lib: &Library, id: &str, context: &str, title: &str, messages: &[Value]) -> Result<()> {
    ensure!(!id.is_empty(), "会话编号不能为空");
    let mut db = lib.db()?;
    let transaction = db.transaction()?;
    let changed = transaction.execute(
        "INSERT INTO ai_records(id,kind,payload,created_at) VALUES(?1,'chat',?2,?3)
         ON CONFLICT(id) DO UPDATE SET payload=excluded.payload,created_at=excluded.created_at
         WHERE ai_records.kind='chat' AND
         CASE WHEN json_valid(ai_records.payload) THEN json_extract(ai_records.payload,'$.context') END=?4",
        params![id, json!({"id":id,"context":context,"title":title,"messages":messages}).to_string(),
            chrono::Utc::now().format("%Y-%m-%d %H:%M:%S%.9f").to_string(), context],
    )?;
    ensure!(changed == 1, "会话编号已属于其他材料或资料，请新建会话");
    transaction.execute(
        "DELETE FROM ai_records WHERE id IN (
            SELECT id FROM ai_records WHERE kind='chat' AND
            CASE WHEN json_valid(payload) THEN json_extract(payload,'$.context') END=?1
            ORDER BY created_at DESC,rowid DESC LIMIT -1 OFFSET 12
         )",
        [context],
    )?;
    transaction.commit()?;
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub context: String,
    pub id: String,
    generation: u64,
}

#[derive(Default, Debug)]
pub struct State {
    pub context: String,
    pub id: String,
    pub history: Vec<Value>,
    pub sessions: Vec<Session>,
    pub draft: String,
    pub running: bool,
    pub reasoning: String,
    pub reasoning_open: bool,
    pub output: String,
    pub error: String,
    generation: u64,
    request: Option<Request>,
}

impl State {
    pub(crate) fn loaded(&mut self, session: Option<Session>) {
        self.id.clear();
        self.history.clear();
        self.output.clear();
        self.reasoning.clear();
        if let Some(session) = session {
            self.id = session.id;
            self.history = session.messages;
            if let Some(message) = self.history.last() {
                self.output = message["content"].as_str().unwrap_or_default().into();
                self.reasoning = message["reasoning_content"]
                    .as_str()
                    .unwrap_or_default()
                    .into();
            }
        }
    }

    fn checked<T>(&mut self, result: Result<T>) -> Result<T> {
        match result {
            Ok(value) => {
                self.error.clear();
                Ok(value)
            }
            Err(error) => {
                self.error = format!("{error:#}");
                Err(error)
            }
        }
    }

    /// Load into temporaries first: a database failure never clears visible content.
    pub fn switch_context(&mut self, lib: &Library, context: &str) -> Result<bool> {
        if self.context == context {
            return Ok(false);
        }
        let loaded = (|| -> Result<_> {
            Ok((
                list(lib, context)?,
                lib.setting(&format!("chat_draft_{context}"))?,
            ))
        })();
        let (sessions, draft) = self.checked(loaded)?;
        let selected = sessions.first().cloned();
        self.invalidate_request();
        self.context = context.into();
        self.sessions = sessions;
        self.draft = draft;
        self.loaded(selected);
        Ok(true)
    }

    /// Explicitly called after persistence, deletion or backup restore, never by render.
    pub fn refresh(&mut self, lib: &Library) -> Result<()> {
        let loaded = (|| -> Result<_> {
            let sessions = list(lib, &self.context)?;
            let selected = if self.id.is_empty() || self.running {
                None
            } else if let Some(session) = sessions.iter().find(|session| session.id == self.id) {
                Some(session.clone())
            } else {
                get(lib, &self.context, &self.id)?
            };
            Ok((sessions, selected))
        })();
        let (sessions, selected) = self.checked(loaded)?;
        self.sessions = sessions;
        if !self.id.is_empty() && !self.running {
            if selected.is_none() {
                self.invalidate_request();
            }
            self.loaded(selected);
        }
        Ok(())
    }

    pub fn select(&mut self, lib: &Library, id: &str) -> Result<()> {
        let selected = if id.is_empty() {
            let cleared = lib.set_setting(&format!("chat_draft_{}", self.context), "");
            self.checked(cleared)?;
            None
        } else {
            let result = get(lib, &self.context, id)
                .and_then(|s| s.context("当前材料中没有这条会话，请刷新后重试"));
            Some(self.checked(result)?)
        };
        self.invalidate_request();
        self.error.clear();
        if selected.is_none() {
            self.draft.clear();
        }
        self.loaded(selected);
        Ok(())
    }

    /// The caller retains editor text on failure; `draft` tracks the persisted value.
    pub fn save_draft(&mut self, lib: &Library, draft: &str) -> Result<()> {
        let saved = lib.set_setting(&format!("chat_draft_{}", self.context), draft);
        self.checked(saved)?;
        self.draft = draft.into();
        Ok(())
    }

    pub fn begin_request(&mut self) -> Request {
        self.generation = self.generation.wrapping_add(1);
        let request = Request {
            context: self.context.clone(),
            id: self.id.clone(),
            generation: self.generation,
        };
        self.running = true;
        self.request = Some(request.clone());
        request
    }

    pub fn current_request(&self) -> Option<Request> {
        self.request.as_ref().filter(|r| self.accepts(r)).cloned()
    }

    pub fn accepts(&self, request: &Request) -> bool {
        self.running
            && self.context == request.context
            && self.id == request.id
            && self.request.as_ref() == Some(request)
    }

    pub fn finish_request(&mut self, request: &Request) -> bool {
        if !self.accepts(request) {
            return false;
        }
        self.running = false;
        self.request = None;
        true
    }

    pub fn invalidate_request(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.running = false;
        self.request = None;
    }
}
