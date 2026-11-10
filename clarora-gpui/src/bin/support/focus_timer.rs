//! Process restart, real SQLite and deterministic clock coverage for flexible focus.
use anyhow::{Result, ensure};
use clarora_gpui::{
    data::Library,
    focus_notes,
    focus_timer::{Event, KEY, Recommendation, Timer, Toggle},
};
use serde_json::{Value, json};
use std::{fs, path::Path, time::Duration};

const T: u64 = 1_800_000_000_000;
const MINUTE: u64 = 60_000;

pub fn worker(root: &Path) -> Result<()> {
    let lib = Library::open(root.to_path_buf())?;
    let mut timer = Timer::load(&lib, T)?.timer;
    timer.toggle(&lib, T)?;
    timer.tick(&lib, T + 137_500)?;
    std::process::exit(0);
}

fn count(lib: &Library) -> Result<i64> {
    Ok(lib.db()?.query_row(
        "SELECT COALESCE(SUM(pomodoro_count),0) FROM study_stats",
        [],
        |r| r.get(0),
    )?)
}

pub fn run(root: &Path, results: &mut Vec<Value>) -> Result<()> {
    let root = root.join("focus-timer");
    fs::create_dir_all(&root)?;
    let restart = root.join("restart");
    clarora_gpui::process::run(
        &std::env::current_exe()?,
        &[
            restart.to_string_lossy().into_owned(),
            "--focus-timer-worker".into(),
        ],
        &clarora_gpui::process::cancel_token(),
        Duration::from_secs(8),
    )?;
    let lib = Library::open(restart)?;
    let mut cases = vec![];
    let mut timer = Timer::load(&lib, T + 86_400_000)?.timer;
    ensure!(
        timer.paused()
            && timer.goal_seconds() == 1500
            && timer.display_seconds(T + 86_400_000) == 137,
        "process reopen counted offline time or lost checkpoint"
    );
    ensure!(timer.toggle(&lib, T + 200_000)? == Toggle::Resumed);
    ensure!(timer.toggle(&lib, T + 202_750)? == Toggle::Paused);
    ensure!(
        Timer::load(&lib, T + 300_000)?
            .timer
            .display_seconds(T + 300_000)
            == 140
    );
    cases.push(json!(
        "process exit preserves checkpoint; paused time is excluded"
    ));

    let lib = Library::open(root.join("rounds"))?;
    let mut timer = Timer::load(&lib, T)?.timer;
    timer.set_goal(&lib, 45 * 60)?;
    timer.toggle(&lib, T)?;
    ensure!(
        timer.tick(&lib, T + 45 * MINUTE)? == Some(Event::GoalReached)
            && timer.is_running()
            && timer.focus()
            && count(&lib)? == 0,
        "minimum goal ended or prematurely counted the session"
    );
    ensure!(timer.tick(&lib, T + 47 * MINUTE)?.is_none());
    timer.record_distraction(&lib)?;
    let before = lib.setting(KEY)?;
    lib.db()?.execute_batch("CREATE TRIGGER reject_round BEFORE INSERT ON study_stats BEGIN SELECT RAISE(ABORT,'fixture write failure'); END;")?;
    ensure!(
        timer.finish(&lib, T + 50 * MINUTE).is_err()
            && lib.setting(KEY)? == before
            && timer.recent().is_empty()
    );
    lib.db()?.execute_batch("DROP TRIGGER reject_round")?;
    timer.finish(&lib, T + 50 * MINUTE)?;
    let round = &timer.recent()[0];
    ensure!(
        round.actual_ms == 50 * MINUTE
            && round.goal_ms == 45 * MINUTE
            && round.distractions == 1
            && round.fatigue.is_none()
            && round.review_pending
            && count(&lib)? == 1
            && !timer.focus()
    );
    let id = round.id.clone();
    timer.review(&lib, &id, None)?;
    ensure!(!timer.recent()[0].review_pending && timer.recent()[0].fatigue.is_none());
    timer.review(&lib, &id, Some(1))?;
    let loaded = Timer::load(&lib, T + 56 * MINUTE)?;
    ensure!(
        loaded.completed == Some(Event::BreakEnded)
            && loaded.timer.awaiting_focus()
            && !loaded.timer.is_running()
            && loaded.timer.recent()[0].fatigue == Some(1)
            && loaded.timer.recent()[0].distractions == 1
            && count(&lib)? == 1
    );
    timer = loaded.timer;
    cases.push(json!(
        "goal fires once; explicit finish, optional review and break are durable and atomic"
    ));

    for n in 1..=2 {
        let start = T + n * 100 * MINUTE;
        timer.toggle(&lib, start)?;
        timer.finish(&lib, start + 50 * MINUTE)?;
        let id = timer.recent()[0].id.clone();
        ensure!(timer.recommendation() == Recommendation::NeedMore);
        timer.review(&lib, &id, Some(1))?;
        timer.tick(&lib, start + 55 * MINUTE)?;
    }
    ensure!(
        timer.recommendation() == Recommendation::IncreaseTo(3600) && timer.goal_seconds() == 2700
    );
    timer.set_goal(&lib, 3600)?;
    ensure!(timer.goal_seconds() == 3600 && timer.recommendation() == Recommendation::AtMaximum);
    timer.set_goal(&lib, 2700)?;
    let start = T + 500 * MINUTE;
    timer.toggle(&lib, start)?;
    timer.record_distraction(&lib)?;
    timer.record_distraction(&lib)?;
    timer.finish(&lib, start + 30 * MINUTE)?;
    let id = timer.recent()[0].id.clone();
    timer.review(&lib, &id, Some(3))?;
    ensure!(timer.recommendation() == Recommendation::Stay && count(&lib)? == 3);
    let rounds = serde_json::to_value(timer.recent())?;
    cases.push(json!("three comfortable rounds suggest 60 without changing the goal; early finish remains in the training history"));

    let mixed = Library::open(root.join("mixed-goals"))?;
    let mut timer = Timer::load(&mixed, T)?.timer;
    let mut start = T;
    for _ in 0..4 {
        for goal in [5, 20, 30] {
            timer.set_goal(&mixed, goal * 60)?;
            timer.toggle(&mixed, start)?;
            ensure!(timer.set_goal(&mixed, 3600).is_err(), "active goal changed");
            timer.finish(&mixed, start + goal * MINUTE)?;
            let id = timer.recent()[0].id.clone();
            ensure!(
                timer.recommendation() == Recommendation::NeedMore,
                "unrated session counted as low fatigue"
            );
            timer.review(&mixed, &id, Some(1))?;
            start += 100 * MINUTE;
            timer.tick(&mixed, start)?;
        }
    }
    for (goal, next) in [(5, 10), (20, 25), (30, 35)] {
        timer.set_goal(&mixed, goal * 60)?;
        timer = Timer::load(&mixed, start)?.timer;
        ensure!(
            timer.goal_seconds() == goal * 60
                && timer.recommendation() == Recommendation::IncreaseTo(next * 60),
            "mixed rounds lost the saved duration or used the wrong training sample"
        );
    }
    cases.push(json!(
        "mixed 5/20/30-minute targets persist and progress independently across reopen"
    ));

    let legacy = Library::open(root.join("legacy"))?;
    legacy.set_setting(KEY, &json!({"version":1,"strict":true,"focus":true,"paused":false,"duration_ms":1500000,"remaining_ms":1500000,"deadline_ms":T+10000}).to_string())?;
    let mut timer = Timer::load(&legacy, T)?.timer;
    ensure!(
        timer.paused()
            && !timer.strict()
            && timer.goal_seconds() == 1500
            && timer.display_seconds(T) == 1490
    );
    let mut stale = Timer::load(&legacy, T)?.timer;
    timer.resume_strict(&legacy, T)?;
    ensure!(stale.reset(&legacy).is_err());
    ensure!(timer.tick(&legacy, T + 10_000)? == Some(Event::GoalReached) && timer.strict());
    timer.interrupt(&legacy, T + 11_000, true)?;
    ensure!(timer.paused() && !timer.strict() && timer.distractions() == 1);
    for bad in ["not json", r#"{"version":999}"#] {
        legacy.set_setting(KEY, bad)?;
        let mut loaded = Timer::load(&legacy, T)?;
        ensure!(loaded.warning.is_some() && legacy.setting(KEY)? == bad);
        loaded.timer.reset(&legacy)?;
        ensure!(Timer::load(&legacy, T)?.warning.is_none());
    }
    cases.push(json!(
        "legacy countdown, strict interruption, concurrent writers and corrupt state recovery"
    ));

    let notes = Library::open(root.join("notes"))?;
    let mut timer = Timer::load(&notes, T)?.timer;
    timer.toggle(&notes, T)?;
    ensure!(
        focus_notes::save(&notes, "  ", T).is_err(),
        "blank note saved"
    );
    ensure!(
        focus_notes::save(&notes, &"长".repeat(focus_notes::MAX_CHARS + 1), T).is_err(),
        "oversized note saved"
    );
    let first = focus_notes::save(&notes, "  想到一个论文结构：先写结论  ", T + 1)?;
    ensure!(
        first.body == "想到一个论文结构：先写结论",
        "note whitespace not trimmed"
    );
    let second = focus_notes::save(&notes, "结束后查一下引用格式", T + 2)?;
    timer.record_distraction(&notes)?;
    timer.finish(&notes, T + 3 * MINUTE)?;
    let reopened = Library::open(notes.root.clone())?;
    let saved = focus_notes::list(&reopened, 10)?;
    ensure!(
        saved.len() == 2
            && saved[0].id == second.id
            && saved[1].body == "想到一个论文结构：先写结论"
            && saved[0].created_ms == T + 2
            && Timer::load(&reopened, T + 4 * MINUTE)?.timer.recent().len() == 1,
        "notes were lost or reordered after a finished round and reopen"
    );
    focus_notes::delete(&reopened, &saved[0].id)?;
    ensure!(
        focus_notes::list(&reopened, 10)?.len() == 1
            && focus_notes::delete(&reopened, &saved[0].id).is_err(),
        "note deletion was not durable or silently accepted a missing ID"
    );
    focus_notes::save(&reopened, "第三条", T + 5)?;
    ensure!(
        focus_notes::list(&reopened, 10)?.len() == 2,
        "new note after delete was not stored"
    );
    cases.push(json!(
        "focus notes survive finish and reopen; blank/oversized input rejected and deletion is durable"
    ));

    let evidence = root.join("focus-recovery.json");
    fs::write(
        &evidence,
        serde_json::to_vec_pretty(
            &json!({"passed":true,"cases":cases,"database":lib.root.join("clarora.db"),"rounds":rounds}),
        )?,
    )?;
    results.push(json!({"workflow":"最低专注目标/自由延长/中断恢复/实际时长与训练建议","passed":true,"cases":cases.len(),"evidence":evidence}));
    Ok(())
}
