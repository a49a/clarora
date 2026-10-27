//! Real SQLite locks, faults, debounce, material switches and controller restart.
use anyhow::{Result, ensure};
use clarora_gpui::{chat_controller::Controller, chat_store, data::Library};
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

fn record(root: &Path, evidence: &mut Vec<Value>, name: &str, details: Value) -> Result<()> {
    evidence.push(json!({"case":name,"passed":true,"details":details}));
    fs::write(
        root.join("chat-controller-evidence.json"),
        serde_json::to_vec_pretty(evidence)?,
    )?;
    Ok(())
}

fn wait(
    controller: &mut Controller,
    condition: impl Fn(&Controller) -> bool,
) -> Result<Vec<String>> {
    let start = Instant::now();
    let mut errors = vec![];
    loop {
        errors.extend(controller.poll().errors);
        if condition(controller) {
            return Ok(errors);
        }
        ensure!(
            start.elapsed() < Duration::from_secs(15),
            "controller timeout: context={}, error={}",
            controller.context,
            controller.error
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn request_context(controller: &mut Controller, context: &str, editor: &str) -> Result<()> {
    let start = Instant::now();
    loop {
        if controller.switch_context(context, editor)? || controller.requested_context() == context
        {
            return Ok(());
        }
        ensure!(
            start.elapsed() < Duration::from_secs(5),
            "context retry backoff never completed"
        );
        controller.poll();
        thread::sleep(Duration::from_millis(5));
    }
}

pub fn run(parent: &Library, results: &mut Vec<Value>) -> Result<()> {
    let root = parent.root.join("chat-controller");
    let lib = Library::open(root.join("library"))?;
    let mut evidence = vec![];
    for (context, id) in [("A", "session-A"), ("B", "session-B"), ("C", "session-C")] {
        chat_store::save(
            &lib,
            id,
            context,
            id,
            &[json!({"role":"assistant","content":context})],
        )?;
    }
    let mut controller = Controller::new(lib.clone());
    controller.switch_context("A", "")?;
    wait(&mut controller, |c| c.context == "A" && !c.is_loading())?;
    controller.observe_draft("initial A");
    controller.flush_timeout(Duration::from_secs(2))?;
    ensure!(
        lib.setting("chat_draft_A")? == "initial A",
        "initial draft not saved"
    );
    record(
        &root,
        &mut evidence,
        "initial-load-and-explicit-flush",
        json!({"context":controller.context,"draft":lib.setting("chat_draft_A")?}),
    )?;

    let initial_revision = controller.draft_revision();
    controller.observe_draft("different draft before returning to same text");
    let changed_revision = controller.draft_revision();
    controller.observe_draft("initial A");
    let returned_revision = controller.draft_revision();
    controller.observe_draft("initial A");
    ensure!(
        initial_revision < changed_revision
            && changed_revision < returned_revision
            && controller.draft_revision() == returned_revision,
        "same-text round trip did not advance the version exactly for edits"
    );
    controller
        .flush_async()
        .recv_timeout(Duration::from_secs(2))?
        .map_err(|error| anyhow::anyhow!(error))?;
    controller.poll();
    ensure!(
        controller.draft_revision() == returned_revision
            && lib.setting("chat_draft_A")? == "initial A",
        "barrier acknowledgement changed the editor version or lost the final text"
    );
    controller.switch_context("B", "initial A")?;
    wait(&mut controller, |c| c.context == "B" && !c.is_loading())?;
    controller.switch_context("A", "")?;
    wait(&mut controller, |c| c.context == "A" && !c.is_loading())?;
    ensure!(
        controller.draft_revision() == returned_revision && controller.draft == "initial A",
        "context loads changed the draft edit revision"
    );
    controller.select("", "initial A")?;
    let cleared_revision = controller.draft_revision();
    ensure!(
        cleared_revision > returned_revision,
        "new-session clear did not advance draft revision"
    );
    wait(&mut controller, |c| !c.is_loading())?;
    controller.flush_timeout(Duration::from_secs(2))?;
    ensure!(
        controller.draft_revision() == cleared_revision && lib.setting("chat_draft_A")?.is_empty(),
        "new-session clear/barrier lost its revision or persisted value"
    );
    controller.observe_draft("initial A");
    controller.flush_timeout(Duration::from_secs(2))?;
    record(
        &root,
        &mut evidence,
        "close-barrier-draft-version-same-text-roundtrip-context-and-clear",
        json!({"initial_revision":initial_revision,"changed_revision":changed_revision,"returned_revision":returned_revision,"cleared_revision":cleared_revision,"restored_revision":controller.draft_revision()}),
    )?;

    let old_request = controller.begin_request();
    let lock = lib.db()?;
    lock.execute_batch("BEGIN IMMEDIATE")?;
    let start = Instant::now();
    for n in 0..1000 {
        controller.observe_draft(&format!("coalesced A {n}"));
    }
    controller.switch_context("B", "coalesced A 999")?;
    controller.switch_context("C", "coalesced A 999")?;
    for _ in 0..100 {
        controller.poll();
    }
    let ui_ms = start.elapsed().as_millis();
    ensure!(
        ui_ms < 250,
        "UI-facing calls waited on SQLite lock: {ui_ms}ms"
    );
    ensure!(
        !controller.accepts(&old_request),
        "material switch accepted old AI request"
    );
    lock.execute_batch("ROLLBACK")?;
    wait(&mut controller, |c| c.context == "C" && !c.is_loading())?;
    ensure!(controller.output == "C", "stale B result replaced C");
    controller.flush_timeout(Duration::from_secs(2))?;
    ensure!(
        lib.setting("chat_draft_A")? == "coalesced A 999",
        "coalesced draft lost"
    );
    record(
        &root,
        &mut evidence,
        "write-lock-ui-latency-coalescing-and-stale-load",
        json!({"ui_ms":ui_ms,"draft_updates":1000,"context":controller.context,"old_request_rejected":true}),
    )?;

    controller.switch_context("A", "")?;
    wait(&mut controller, |c| c.context == "A" && !c.is_loading())?;
    ensure!(
        controller.draft == "coalesced A 999",
        "return lost material draft"
    );
    let lock = lib.db()?;
    lock.execute_batch("BEGIN IMMEDIATE")?;
    controller.switch_context("B", "before switch")?;
    controller.observe_draft("typed during material loading");
    lock.execute_batch("ROLLBACK")?;
    wait(&mut controller, |c| c.context == "B" && !c.is_loading())?;
    controller.switch_context("A", "")?;
    wait(&mut controller, |c| c.context == "A" && !c.is_loading())?;
    ensure!(
        controller.draft == "typed during material loading",
        "late outgoing input lost"
    );
    controller.select("session-A", "existing session draft")?;
    wait(&mut controller, |c| !c.is_loading())?;
    ensure!(
        controller.draft == "existing session draft",
        "session select discarded material draft"
    );
    controller.select("", "existing session draft")?;
    controller.observe_draft("new input after new-session request");
    wait(&mut controller, |c| !c.is_loading())?;
    controller.flush_timeout(Duration::from_secs(2))?;
    ensure!(
        controller.id.is_empty()
            && controller.draft == "new input after new-session request"
            && lib.setting("chat_draft_A")? == controller.draft,
        "new session cleared subsequent input"
    );
    record(
        &root,
        &mut evidence,
        "switch-late-input-session-select-and-new-session",
        json!({"draft":controller.draft,"new_session":controller.id.is_empty()}),
    )?;

    let db = lib.db()?;
    db.execute_batch("CREATE TRIGGER controller_draft_failure BEFORE INSERT ON app_settings WHEN NEW.key LIKE 'chat_draft_%' BEGIN SELECT RAISE(ABORT, 'controller draft failure'); END;")?;
    controller.observe_draft("retained after failed write");
    controller.flush();
    let errors = wait(&mut controller, |c| !c.error.is_empty())?;
    ensure!(
        controller.draft == "retained after failed write" && !errors.is_empty(),
        "failed write discarded input or hid failure"
    );
    request_context(&mut controller, "B", "retained after failed write")?;
    wait(&mut controller, |c| !c.is_loading())?;
    ensure!(
        controller.context == "A" && controller.draft == "retained after failed write",
        "failed outgoing save switched away/discarded content"
    );
    db.execute_batch("DROP TRIGGER controller_draft_failure")?;
    controller.flush_timeout(Duration::from_secs(2))?;
    controller.poll();
    ensure!(
        lib.setting("chat_draft_A")? == "retained after failed write",
        "failed draft retry lost value"
    );
    db.execute_batch("ALTER TABLE ai_records RENAME TO unavailable_controller_records")?;
    request_context(&mut controller, "B", "retained after failed write")?;
    wait(&mut controller, |c| !c.is_loading())?;
    ensure!(
        controller.context == "A" && controller.draft == "retained after failed write",
        "failed read replaced visible state"
    );
    db.execute_batch("ALTER TABLE unavailable_controller_records RENAME TO ai_records")?;
    request_context(&mut controller, "B", "retained after failed write")?;
    wait(&mut controller, |c| c.context == "B" && !c.is_loading())?;
    record(
        &root,
        &mut evidence,
        "write-and-read-failure-retain-input-and-retry",
        json!({"write_errors":errors,"recovered_context":controller.context,"draft_A":lib.setting("chat_draft_A")?}),
    )?;

    let request = controller.begin_request();
    controller.output = "live streaming output".into();
    controller.observe_draft("draft while refreshing");
    controller.refresh()?;
    wait(&mut controller, |c| !c.is_loading())?;
    ensure!(
        controller.accepts(&request)
            && controller.output == "live streaming output"
            && controller.draft == "draft while refreshing",
        "refresh overwrote active stream or input"
    );
    controller.observe_draft("final pending before drop");
    drop(controller);
    let start = Instant::now();
    while lib.setting("chat_draft_B")? != "final pending before drop" {
        ensure!(
            start.elapsed() < Duration::from_secs(3),
            "orderly drop did not flush pending draft"
        );
        thread::sleep(Duration::from_millis(5));
    }
    let reopened_lib = Library::open(lib.root.clone())?;
    let mut reopened = Controller::new(reopened_lib.clone());
    reopened.switch_context("B", "")?;
    wait(&mut reopened, |c| c.context == "B" && !c.is_loading())?;
    ensure!(
        reopened.draft == "final pending before drop",
        "reopened controller lost final draft"
    );
    reopened.flush_timeout(Duration::from_secs(2))?;
    reopened_lib.db()?.execute(
        "VACUUM INTO ?1",
        [root
            .join("chat-controller.sqlite")
            .to_string_lossy()
            .as_ref()],
    )?;
    record(
        &root,
        &mut evidence,
        "active-refresh-orderly-disposal-and-real-sqlite-reopen",
        json!({"draft":reopened.draft,"sqlite":root.join("chat-controller.sqlite")}),
    )?;
    results.push(json!({"workflow":"后台聊天仓储 → 防抖草稿 → SQLite写锁不阻塞界面 → 材料/会话代次隔离 → 故障保留及重开","passed":true,"cases":evidence.len(),"report":root.join("chat-controller-evidence.json")}));
    Ok(())
}
