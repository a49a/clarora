//! SQLite/process restart workflows; the UI uses this same durable timer controller.
use anyhow::{Result, ensure};
use clarora_gpui::{
    data::Library,
    focus_timer::{KEY, Timer, Toggle},
};
use serde_json::{Value, json};
use std::{fs, path::Path, time::Duration};

pub fn worker(root: &Path) -> Result<()> {
    let lib = Library::open(root.to_path_buf())?;
    let mut timer = Timer::load(&lib, 1_800_000_000_000)?.timer;
    timer.start(&lib, 1_800_000_000_000, 900)?;
    // No view shutdown hook or destructor may be necessary to preserve the timer.
    std::process::exit(0);
}
fn count(lib: &Library) -> Result<f64> {
    Ok(lib
        .list("study_stats")?
        .iter()
        .map(|r| r["pomodoro_count"].as_f64().unwrap_or(0.))
        .sum())
}
pub fn run(root: &Path, results: &mut Vec<Value>) -> Result<()> {
    let root = root.join("focus-timer");
    fs::create_dir_all(&root)?;
    let dbroot = root.join("restart");
    let t = 1_800_000_000_000u64;
    clarora_gpui::process::run(
        &std::env::current_exe()?,
        &[
            dbroot.to_string_lossy().into_owned(),
            "--focus-timer-worker".into(),
        ],
        &clarora_gpui::process::cancel_token(),
        Duration::from_secs(8),
    )?;
    let lib = Library::open(dbroot.clone())?;
    let mut cases = vec![];
    let mut timer = Timer::load(&lib, t + 137_500)?.timer;
    ensure!(
        timer.is_running() && timer.focus() && timer.remaining_seconds(t + 137_500) == 763,
        "重开未恢复900秒倒计时"
    );
    cases.push(json!({"case":"独立进程不走析构退出/真实SQLite重开/扣除关闭时间","passed":true}));
    ensure!(
        timer.toggle(&lib, t + 150_250)? == Toggle::Paused,
        "没有暂停"
    );
    timer = Timer::load(&lib, t + 500_000)?.timer;
    ensure!(
        timer.paused() && timer.remaining_seconds(t + 500_000) == 750,
        "暂停状态重开扣时"
    );
    ensure!(
        timer.toggle(&lib, t + 500_000)? == Toggle::Resumed,
        "继续状态错误"
    );
    ensure!(
        Timer::load(&lib, t + 550_000)?
            .timer
            .remaining_seconds(t + 550_000)
            == 700,
        "继续后毫秒精度丢失"
    );
    timer.reset(&lib)?;
    timer = Timer::load(&lib, t + 900_000)?.timer;
    ensure!(
        !timer.is_running()
            && !timer.paused()
            && timer.focus()
            && timer.remaining_seconds(t + 900_000) == 1500,
        "重置未持久化"
    );
    cases.push(json!({"case":"暂停与继续跨重开/保留毫秒/重置清理deadline","passed":true}));
    timer.start(&lib, t, 1)?;
    let loaded = Timer::load(&lib, t + 86_400_000)?;
    ensure!(
        loaded.completed == Some(true)
            && !loaded.timer.focus()
            && loaded.timer.paused()
            && loaded.timer.remaining_seconds(t + 86_400_000) == 300
            && count(&lib)? == 1.,
        "离线到期未停在休息或重复累计"
    );
    let loaded = Timer::load(&lib, t + 172_800_000)?;
    ensure!(
        loaded.completed.is_none() && count(&lib)? == 1.,
        "重开重复结算"
    );
    timer = loaded.timer;
    timer.toggle(&lib, t + 172_800_000)?;
    let loaded = Timer::load(&lib, t + 172_800_000 + 301_000)?;
    ensure!(
        loaded.completed == Some(false)
            && loaded.timer.focus()
            && loaded.timer.paused()
            && count(&lib)? == 1.,
        "休息结束统计或阶段错误"
    );
    cases.push(json!({"case":"离线多日只完成一轮/不重复统计/休息恢复","passed":true}));
    timer = loaded.timer;
    timer.start(&lib, t, 1)?;
    let old = lib.setting(KEY)?;
    lib.db()?.execute_batch("CREATE TRIGGER reject_timer_stats BEFORE INSERT ON study_stats BEGIN SELECT RAISE(ABORT,'fixture write failure'); END;")?;
    ensure!(
        timer.tick(&lib, t + 2000, true).is_err() && lib.setting(KEY)? == old && count(&lib)? == 1.,
        "状态与统计未原子回滚"
    );
    lib.db()?
        .execute_batch("DROP TRIGGER reject_timer_stats;")?;
    ensure!(
        timer.tick(&lib, t + 2000, true)? == Some(true)
            && timer.is_running()
            && !timer.focus()
            && count(&lib)? == 2.,
        "失败重试没有精确结算"
    );
    ensure!(
        timer.tick(&lib, t + 2001, true)?.is_none() && count(&lib)? == 2.,
        "tick重复统计"
    );
    cases.push(
        json!({"case":"实际SQLite统计写失败/事务回滚/重试只统计一次/在线自动休息","passed":true}),
    );
    let retry_root = root.join("startup-write-failure");
    let retry_lib = Library::open(retry_root)?;
    let mut running = Timer::load(&retry_lib, t)?.timer;
    running.start(&retry_lib, t, 1)?;
    retry_lib.db()?.execute_batch("CREATE TRIGGER reject_startup_stats BEFORE INSERT ON study_stats BEGIN SELECT RAISE(ABORT,'startup fixture failure'); END;")?;
    let restored = Timer::load(&retry_lib, t + 2000)?;
    ensure!(
        restored.warning.is_some() && restored.timer.is_running(),
        "恢复写失败丢失了原计时状态"
    );
    retry_lib
        .db()?
        .execute_batch("DROP TRIGGER reject_startup_stats;")?;
    let mut retry = restored.timer;
    ensure!(
        retry.tick(&retry_lib, t + 2100, true)? == Some(true)
            && retry.paused()
            && count(&retry_lib)? == 1.,
        "恢复失败重试未停在下一阶段"
    );
    cases.push(json!({"case":"启动结算写失败保留原计时/后续重试暂停下一阶段","passed":true}));
    let mut stale = Timer::load(&lib, t + 2100)?.timer;
    timer.start(&lib, t + 2200, 600)?;
    ensure!(
        stale.reset(&lib).is_err() && Timer::load(&lib, t + 2200)?.timer.is_running(),
        "旧实例覆盖新状态"
    );
    ensure!(
        timer.remaining_seconds(t - 1000) <= 600,
        "系统时间回拨增加到超过本轮时长"
    );
    cases.push(json!({"case":"多实例版本冲突/系统时间回拨夹取","passed":true}));
    for bad in ["not json", "{\"version\":999}"] {
        lib.set_setting(KEY, bad)?;
        let loaded = Timer::load(&lib, t)?;
        ensure!(
            loaded.warning.is_some() && lib.setting(KEY)? == bad && !loaded.timer.is_running(),
            "损坏状态被静默覆盖"
        );
        let mut recovered = loaded.timer;
        recovered.reset(&lib)?;
        ensure!(
            Timer::load(&lib, t)?.warning.is_none(),
            "用户显式重置无法恢复"
        );
    }
    cases.push(json!({"case":"损坏/未知版本保留原值并提示/显式重置修复","passed":true}));
    let strict_lib = Library::open(root.join("strict-timer"))?;
    let mut strict = Timer::default();
    strict.start_strict(&strict_lib, t, 60)?;
    let restored = Timer::load(&strict_lib, t + 10_000)?.timer;
    ensure!(
        restored.strict() && restored.deadline_ms() == Some(t + 60_000),
        "strict session deadline did not survive reopen"
    );
    strict.toggle(&strict_lib, t + 10_000)?;
    ensure!(
        !strict.strict() && strict.paused(),
        "emergency pause retained strict mode"
    );
    strict.start_strict(&strict_lib, t, 1)?;
    ensure!(
        strict.tick(&strict_lib, t + 1001, true)? == Some(true) && !strict.strict(),
        "deadline did not clear strict mode for break"
    );
    let completed = count(&strict_lib)?;
    ensure!(
        Timer::load(&strict_lib, t + 2000)?.completed.is_none() && count(&strict_lib)? == completed,
        "strict completion counted twice"
    );
    strict_lib.db()?.execute_batch("CREATE TRIGGER fail_strict BEFORE UPDATE ON app_settings WHEN NEW.key='focus_timer_v1' BEGIN SELECT RAISE(ABORT,'injected strict save failure'); END;")?;
    ensure!(
        strict.start_strict(&strict_lib, t, 30).is_err() && !strict.strict(),
        "failed strict start changed in-memory state"
    );
    strict_lib
        .db()?
        .execute_batch("DROP TRIGGER fail_strict;")?;
    cases.push(
        json!({"case":"电脑专注标志/截止重开/应急暂停/结束解除/保存失败原子保护","passed":true}),
    );
    let evidence = root.join("focus-recovery.json");
    fs::write(
        &evidence,
        serde_json::to_vec_pretty(
            &json!({"passed":true,"cases":cases,"database":dbroot.join("clarora.db"),"final_state":lib.setting(KEY)?}),
        )?,
    )?;
    results.push(json!({"workflow":"专注计时跨进程/暂停/离线到期/原子结算/异常恢复","passed":true,"cases":cases.len(),"evidence":evidence}));
    Ok(())
}
