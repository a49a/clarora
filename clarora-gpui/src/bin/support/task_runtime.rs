//! Real thread/channel/SQLite integration for independent task lanes.
use anyhow::{Result, ensure};
use clarora_gpui::{
    data::Library,
    task_runtime::{Event, Lane, Scope},
};
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

fn wait(
    lane: &mut Lane<String, String>,
    traces: &mut Vec<Value>,
) -> Result<clarora_gpui::task_runtime::Completion<String>> {
    let started = Instant::now();
    loop {
        match lane.poll() {
            Some(Event::Progress { id, value }) => {
                traces.push(json!({"id":id.value(),"update":value}))
            }
            Some(Event::Completed(done)) => return Ok(done),
            None => {}
        }
        ensure!(
            started.elapsed() < Duration::from_secs(3),
            "task completion timeout"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
pub fn run(root: &Path, results: &mut Vec<Value>) -> Result<()> {
    let out = root.join(format!("task-runtime-{}", clarora_gpui::data::id()));
    fs::create_dir_all(&out)?;
    let mut checks = vec![];
    let mut traces = vec![];
    let result = (|| -> Result<()> {
        let lib = Library::open(out.join("library"))?;
        let mut foreground = Lane::<String, String>::default();
        let pending = foreground.begin(Scope::Material)?;
        let first = pending.id();
        let token = pending.cancel_token();
        ensure!(
            foreground.begin(Scope::Application).is_err() && foreground.current_id() == Some(first),
            "busy start replaced active task"
        );
        pending.reporter().send("before-cancel".into()).unwrap();
        foreground.cancel_material();
        ensure!(
            token.load(Ordering::SeqCst),
            "material leave did not request cancellation"
        );
        pending.complete(Err("cancelled fixture".into()));
        let done = wait(&mut foreground, &mut traces)?;
        ensure!(
            done.id == first && done.cancelled && done.result.is_err() && !foreground.is_running(),
            "cancelled task did not finish once"
        );
        checks.push(json!({"case":"busy start refused and material cancellation completes once","passed":true}));

        let old = foreground.begin(Scope::Material)?;
        let old_id = old.id();
        let old_token = old.cancel_token();
        foreground.abort();
        ensure!(
            old_token.load(Ordering::SeqCst),
            "abort did not cancel old task"
        );
        let next = foreground.begin(Scope::Application)?;
        let next_id = next.id();
        ensure!(next_id != old_id, "task generation reused");
        let _ = old.reporter().send("stale".into());
        old.complete(Ok("old".into()));
        ensure!(
            foreground.poll().is_none() && foreground.current_id() == Some(next_id),
            "late old event changed current task"
        );
        foreground.cancel_material();
        ensure!(
            !next.cancel_token().load(Ordering::SeqCst),
            "leaving material cancelled application task"
        );
        next.complete(Ok("new".into()));
        let done = wait(&mut foreground, &mut traces)?;
        ensure!(
            done.result.as_deref() == Ok("new") && !done.cancelled,
            "new application task changed"
        );
        checks.push(json!({"case":"aborted generation cannot publish into application-owned successor","passed":true}));

        let mut models = Lane::<String, String>::default();
        let mut subtitles = Lane::<String, String>::default();
        let a = models.begin(Scope::Application)?;
        let b = subtitles.begin(Scope::Application)?;
        let b_token = b.cancel_token();
        models.cancel();
        a.complete(Err("model stopped".into()));
        ensure!(
            !b_token.load(Ordering::SeqCst) && subtitles.is_running(),
            "model cancellation affected subtitles"
        );
        b.complete(Ok("subtitles committed".into()));
        ensure!(
            wait(&mut models, &mut traces)?.cancelled
                && !wait(&mut subtitles, &mut traces)?.cancelled,
            "lane cancellation crossed ownership"
        );
        checks.push(json!({"case":"model and subtitle lanes cancel independently","passed":true}));

        let writer = lib.clone();
        let committed = Arc::new(AtomicBool::new(false));
        let committed_worker = committed.clone();
        foreground.spawn(Scope::Application, move |_cancel, report| {
            report.send("writing database".into()).unwrap();
            writer
                .set_setting("task_committed", "yes")
                .map_err(|e| e.to_string())?;
            committed_worker.store(true, Ordering::SeqCst);
            Ok("committed".into())
        })?;
        let began = Instant::now();
        while !committed.load(Ordering::SeqCst) {
            ensure!(began.elapsed() < Duration::from_secs(2), "commit timeout");
            std::thread::sleep(Duration::from_millis(5));
        }
        foreground.cancel();
        let done = wait(&mut foreground, &mut traces)?;
        ensure!(
            done.cancelled
                && done.result.as_deref() == Ok("committed")
                && lib.setting("task_committed")? == "yes",
            "executor discarded committed success on cancel"
        );
        checks.push(json!({"case":"committed success is retained alongside concurrent cancel request","passed":true}));

        foreground.spawn(
            Scope::Material,
            |_, _| -> std::result::Result<String, String> { panic!("isolated task panic") },
        )?;
        ensure!(
            wait(&mut foreground, &mut traces)?.result.is_err() && !foreground.is_running(),
            "panic left task busy"
        );
        let abandoned = foreground.begin(Scope::Material)?;
        drop(abandoned);
        ensure!(
            wait(&mut foreground, &mut traces)?.result.is_err() && foreground.poll().is_none(),
            "abandoned handle lost or duplicated completion"
        );
        checks.push(
            json!({"case":"panic and abandoned handles produce one terminal error","passed":true}),
        );

        let drop_token = {
            let mut lane = Lane::<String, String>::default();
            let handle = lane.begin(Scope::Application)?;
            let token = handle.cancel_token();
            drop(lane);
            handle.complete(Ok("late".into()));
            token
        };
        ensure!(
            drop_token.load(Ordering::SeqCst),
            "dropping a lane did not cancel work"
        );
        checks.push(json!({"case":"lane drop requests cancellation without waiting on worker","passed":true}));
        let reopened = Library::open(lib.root.clone())?;
        ensure!(
            reopened.setting("task_committed")? == "yes",
            "committed data did not survive reopen"
        );
        checks
            .push(json!({"case":"real SQLite survives lane completion and reopen","passed":true}));
        Ok(())
    })();
    let report = out.join("task-runtime-report.json");
    fs::write(
        &report,
        serde_json::to_vec_pretty(
            &json!({"passed":result.is_ok(),"checks":checks,"traces":traces,"error":result.as_ref().err().map(|e|format!("{e:#}")),"real_threads":true,"real_sqlite":true}),
        )?,
    )?;
    result?;
    results.push(json!({"workflow":"任务生命周期：身份/取消/迟到/提交竞争/异常/独立lane","passed":true,"checks":checks.len(),"report":report}));
    Ok(())
}
