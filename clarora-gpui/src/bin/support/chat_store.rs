//! Real SQLite → bounded material sessions → cached state → ZIP restore.
use anyhow::{Context, Result, ensure};
use clarora_gpui::{ai, backup, chat_store, data::Library, documents};
use serde_json::{Value, json};
use std::{fs, path::Path};

fn record(root: &Path, evidence: &mut Vec<Value>, case: &str, details: Value) -> Result<()> {
    evidence.push(json!({"case":case,"passed":true,"details":details}));
    fs::write(
        root.join("chat-store-evidence.json"),
        serde_json::to_vec_pretty(evidence)?,
    )?;
    Ok(())
}

fn visible(state: &chat_store::State) -> Value {
    json!({"context":state.context,"id":state.id,"history":state.history,
        "sessions":state.sessions,"draft":state.draft,"running":state.running,
        "reasoning":state.reasoning,"output":state.output})
}

pub fn run(parent: &Library, results: &mut Vec<Value>) -> Result<()> {
    let root = parent.root.join("chat-store");
    let source = root.join("source");
    fs::create_dir_all(&source)?;
    // Create the legacy table before the guarded expression index is installed.
    let legacy = rusqlite::Connection::open(source.join("clarora.db"))?;
    legacy.execute_batch("CREATE TABLE ai_records(id TEXT PRIMARY KEY,kind TEXT NOT NULL,payload TEXT NOT NULL,created_at TEXT NOT NULL);
        INSERT INTO ai_records VALUES('broken-chat','chat','{broken','2000-01-01');
        INSERT INTO ai_records VALUES('broken-other','ocr','{broken','2000-01-01');")?;
    drop(legacy);
    let lib = Library::open(source)?;
    let mut evidence = vec![];
    let context = "material-A:'\"?";
    let other_context = "material-B";
    let messages = vec![
        json!({"role":"user","content":"原题","reference":"材料 A"}),
        json!({"role":"assistant","content":"完整回答","reasoning_content":"完整推理"}),
    ];
    let mut db = lib.db()?;
    let tx = db.transaction()?;
    for n in 0..5000 {
        tx.execute(
            "INSERT INTO ai_records VALUES(?1,?2,?3,'2001-01-01')",
            rusqlite::params![
                format!("unrelated-{n}"),
                if n % 2 == 0 { "ocr" } else { "speaking" },
                if n % 3 == 0 {
                    "{invalid".into()
                } else {
                    json!({"text":"unrelated"}).to_string()
                }
            ],
        )?;
    }
    for n in 0..20 {
        tx.execute("INSERT INTO ai_records VALUES(?1,'chat',?2,'2020-01-01')", rusqlite::params![
            format!("A-{n:02}"), json!({"id":format!("A-{n:02}"),"context":context,"title":format!("题目{n}"),"messages":messages}).to_string()])?;
    }
    tx.commit()?;
    let plan = db.prepare("EXPLAIN QUERY PLAN SELECT id,payload FROM ai_records WHERE kind='chat' AND CASE WHEN json_valid(payload) THEN json_extract(payload,'$.context') END=?1 ORDER BY created_at DESC,rowid DESC LIMIT 12")?
        .query_map([context], |r| r.get::<_, String>(3))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    ensure!(
        plan.iter().any(|p| p.contains("ai_records_context")),
        "context lookup has no indexed query plan: {plan:?}"
    );
    let sessions = chat_store::list(&lib, context)?;
    ensure!(
        sessions.len() == 12 && sessions[0].id == "A-19" && sessions[11].id == "A-08",
        "bounded same-timestamp ordering changed"
    );
    ensure!(
        chat_store::list(&lib, other_context)?.is_empty(),
        "unrelated material leaked sessions"
    );
    record(
        &root,
        &mut evidence,
        "legacy-malformed-json-index-and-bounded-query",
        json!({"unrelated_rows":5002,"loaded_sessions":sessions.len(),"query_plan":plan,"newest":sessions[0].id,"oldest":sessions[11].id}),
    )?;

    ai::save_chat(&lib, "other-session", other_context, "另一材料", &messages)?;
    ensure!(
        chat_store::get(&lib, other_context, "A-19")?.is_none(),
        "ID lookup exposed another context"
    );
    ensure!(
        chat_store::get(&lib, context, "other-session")?.is_none(),
        "ID lookup exposed another context"
    );
    ensure!(
        chat_store::get(&lib, context, "A-19")?
            .context("missing exact session")?
            .messages
            == messages,
        "exact ID changed messages"
    );
    let foreign_before = db.query_row(
        "SELECT payload FROM ai_records WHERE id='other-session'",
        [],
        |r| r.get::<_, String>(0),
    )?;
    ensure!(
        ai::save_chat(&lib, "other-session", context, "bad overwrite", &[]).is_err(),
        "cross-context overwrite succeeded"
    );
    ensure!(
        ai::save_chat(&lib, "unrelated-1", context, "bad overwrite", &[]).is_err(),
        "cross-kind overwrite succeeded"
    );
    ensure!(
        foreign_before
            == db.query_row(
                "SELECT payload FROM ai_records WHERE id='other-session'",
                [],
                |r| r.get::<_, String>(0)
            )?,
        "rejected update changed foreign context"
    );
    record(
        &root,
        &mut evidence,
        "exact-context-id-and-foreign-collision",
        json!({"cross_context_rejected":true,"cross_kind_rejected":true,"messages":messages}),
    )?;

    ai::save_chat(&lib, "A-new", context, "最新题目", &messages)?;
    let remaining: i64 = db.query_row("SELECT count(*) FROM ai_records WHERE kind='chat' AND CASE WHEN json_valid(payload) THEN json_extract(payload,'$.context') END=?1", [context], |r| r.get(0))?;
    ensure!(
        remaining == 12 && chat_store::list(&lib, context)?[0].id == "A-new",
        "save did not cap and promote matching sessions"
    );
    ensure!(
        chat_store::get(&lib, other_context, "other-session")?.is_some(),
        "cap removed another material"
    );
    let mut revised = messages.clone();
    revised.push(json!({"role":"user","content":"继续追问"}));
    revised.push(json!({"role":"assistant","content":"继续回答"}));
    ai::save_chat(&lib, "A-new", context, "继续追问", &revised)?;
    ensure!(
        chat_store::get(&Library::open(lib.root.clone())?, context, "A-new")?
            .unwrap()
            .messages
            == revised,
        "reopen lost appended chat"
    );
    record(
        &root,
        &mut evidence,
        "legacy-save-api-context-cap-and-reopen",
        json!({"remaining":remaining,"messages":revised.len(),"other_context_retained":true}),
    )?;

    // Generic callers still receive ascending created_at order and skip invalid JSON.
    let speaking = documents::records(&lib, "speaking")?;
    ensure!(
        speaking.len() == 1667 && speaking.iter().all(|r| r["text"] == "unrelated"),
        "generic records changed kind/invalid-json filtering"
    );
    record(
        &root,
        &mut evidence,
        "generic-kind-filtered-records",
        json!({"speaking_records":speaking.len(),"invalid_json_skipped":true}),
    )?;

    let mut state = chat_store::State::default();
    ensure!(
        state.switch_context(&lib, context)?,
        "initial context did not load"
    );
    ensure!(
        state.id == "A-new" && state.history == revised && state.output == "继续回答",
        "context initial selection/history incorrect"
    );
    state.save_draft(&lib, "尚未发送的草稿 A")?;
    let before = visible(&state);
    ensure!(
        !state.switch_context(&lib, context)? && before == visible(&state),
        "same context reloaded active state"
    );
    state.switch_context(&lib, other_context)?;
    state.save_draft(&lib, "草稿 B")?;
    state.switch_context(&lib, context)?;
    ensure!(
        state.draft == "尚未发送的草稿 A",
        "context draft did not restore"
    );
    record(
        &root,
        &mut evidence,
        "cached-context-switch-and-draft-reopen",
        json!({"context":state.context,"sessions":state.sessions.len(),"draft":state.draft}),
    )?;

    let old_request = state.begin_request();
    state.switch_context(&lib, other_context)?;
    state.switch_context(&lib, context)?;
    ensure!(
        !state.accepts(&old_request),
        "old request accepted after leaving and returning to same context"
    );
    let old_same_session = state.begin_request();
    state.select(&lib, "A-19")?;
    ensure!(
        !state.accepts(&old_same_session),
        "old request accepted after switching session"
    );
    let active = state.begin_request();
    ensure!(
        state.accepts(&active) && state.finish_request(&active) && !state.accepts(&active),
        "active request lifecycle failed"
    );
    record(
        &root,
        &mut evidence,
        "stale-request-generation-and-session-isolation",
        json!({"same_context_return_rejected":true,"session_switch_rejected":true,"current_completion_accepted_once":true}),
    )?;

    let before = visible(&state);
    ensure!(
        state.select(&lib, "other-session").is_err()
            && visible(&state) == before
            && !state.error.is_empty(),
        "failed select replaced current content"
    );
    db.execute_batch("ALTER TABLE ai_records RENAME TO unavailable_ai_records")?;
    let presented = (0..1000)
        .map(|_| state.sessions.iter().map(|s| s.title.len()).sum::<usize>())
        .sum::<usize>();
    ensure!(
        presented > 0,
        "cached sessions unavailable without database"
    );
    ensure!(
        state.refresh(&lib).is_err() && visible(&state) == before,
        "failed refresh discarded cache"
    );
    ensure!(
        state.switch_context(&lib, other_context).is_err() && visible(&state) == before,
        "failed context load replaced old context/content"
    );
    db.execute_batch("ALTER TABLE unavailable_ai_records RENAME TO ai_records")?;
    state.refresh(&lib)?;
    record(
        &root,
        &mut evidence,
        "read-failures-retain-cache-and-render-needs-no-db",
        json!({"cached_presentations":1000,"content_retained":true,"retried":true}),
    )?;

    state.save_draft(&lib, "已保存草稿")?;
    db.execute_batch("CREATE TRIGGER chat_fixture_draft_insert_failure BEFORE INSERT ON app_settings WHEN NEW.key LIKE 'chat_draft_%' BEGIN SELECT RAISE(ABORT, 'chat draft fixture failure'); END;
        CREATE TRIGGER chat_fixture_draft_update_failure BEFORE UPDATE ON app_settings WHEN NEW.key LIKE 'chat_draft_%' BEGIN SELECT RAISE(ABORT, 'chat draft fixture failure'); END;")?;
    let pending_input = "输入框里未丢失的新草稿";
    ensure!(
        state.save_draft(&lib, pending_input).is_err() && state.draft == "已保存草稿",
        "failed draft changed committed state"
    );
    ensure!(
        lib.setting(&format!("chat_draft_{context}"))? == "已保存草稿",
        "draft failure changed persistent content"
    );
    db.execute_batch("DROP TRIGGER chat_fixture_draft_insert_failure; DROP TRIGGER chat_fixture_draft_update_failure;")?;
    state.save_draft(&lib, pending_input)?;
    ensure!(
        state.draft == pending_input,
        "draft retry did not retain caller input"
    );
    record(
        &root,
        &mut evidence,
        "draft-write-failure-and-unchanged-input-retry",
        json!({"retained_input":pending_input,"persisted_after_retry":state.draft}),
    )?;

    ai::save_chat(&lib, "A-after-save", context, "显式刷新", &messages)?;
    state.refresh(&lib)?;
    ensure!(
        state.sessions[0].id == "A-after-save",
        "post-save refresh did not update index"
    );
    state.select(&lib, "A-after-save")?;
    db.execute("DELETE FROM ai_records WHERE id='A-after-save'", [])?;
    state.refresh(&lib)?;
    ensure!(
        state.id.is_empty() && state.history.is_empty() && state.output.is_empty(),
        "deleted selected session left stale content"
    );
    state.select(&lib, "")?;
    ensure!(
        state.draft.is_empty() && lib.setting(&format!("chat_draft_{context}"))?.is_empty(),
        "new chat retained old question"
    );
    record(
        &root,
        &mut evidence,
        "save-refresh-selected-deletion-and-new-chat",
        json!({"deleted_content_cleared":true,"sessions":state.sessions.len()}),
    )?;

    lib.db()?.execute(
        "VACUUM INTO ?1",
        [root.join("chat-source.sqlite").to_string_lossy().as_ref()],
    )?;
    let zip = root.join("chat-backup.zip");
    backup::export_local(&lib, &zip)?;
    let restored = Library::open(root.join("restored"))?;
    let mut restored_state = chat_store::State::default();
    restored_state.switch_context(&restored, context)?;
    ensure!(
        restored_state.sessions.is_empty(),
        "restore destination unexpectedly populated"
    );
    backup::import_local(&restored, &zip)?;
    restored_state.refresh(&restored)?;
    ensure!(
        restored_state.sessions == chat_store::list(&lib, context)?,
        "ZIP refresh changed bounded session index"
    );
    restored_state.select(&restored, "A-new")?;
    ensure!(
        restored_state.history == revised,
        "ZIP restore changed full messages"
    );
    restored.db()?.execute(
        "VACUUM INTO ?1",
        [root.join("chat-restored.sqlite").to_string_lossy().as_ref()],
    )?;
    record(
        &root,
        &mut evidence,
        "zip-restore-existing-cache-and-full-history",
        json!({"zip":zip,"restored_sessions":restored_state.sessions.len(),"restored_messages":restored_state.history.len()}),
    )?;
    results.push(json!({"workflow":"聊天按材料精确查询 → 缓存界面状态与草稿 → 慢请求隔离 → SQLite故障保护 → ZIP恢复刷新","passed":true,"cases":evidence.len(),"report":root.join("chat-store-evidence.json"),"source_sqlite":root.join("chat-source.sqlite"),"restored_sqlite":root.join("chat-restored.sqlite")}));
    Ok(())
}
