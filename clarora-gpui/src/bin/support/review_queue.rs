use anyhow::{Context, Result, ensure};
use clarora_gpui::data::{Card, Library, text};
use rusqlite::params;
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs};

const KINDS: [&str; 5] = ["word", "sentence", "ai", "clip", "video"];

fn seed(lib: &Library) -> Result<()> {
    for index in 0..24 {
        lib.put(
            "words",
            json!({"word":format!("word-{index:02}"),"meaning":"word meaning"}),
        )?;
    }
    for index in 0..4 {
        let id = format!("card-{index:02}");
        lib.put(
            "sentence_cards",
            json!({"id":id,"text":format!("Sentence {index}"),"translation":"句子"}),
        )?;
        lib.put(
            "ai_cards",
            json!({"id":id,"question":format!("Question {index}"),"answer":"回答"}),
        )?;
        lib.put("clip_cards", json!({"id":id,"en_text":format!("Clip {index}"),"zh_text":"片段","audio_uri":"fixture.wav","start_ms":0,"end_ms":1000}))?;
        lib.put("video_clips", json!({"id":id,"en_text":format!("Video {index}"),"zh_text":"视频","video_uri":"fixture.mp4"}))?;
    }
    Ok(())
}

fn keys(cards: &[Card]) -> Vec<String> {
    cards
        .iter()
        .map(|card| format!("{}:{}", card.kind, card.id))
        .collect()
}

fn introduced(lib: &Library) -> Result<usize> {
    Ok(lib.db()?.query_row(
        "SELECT COUNT(*) FROM review_schedule WHERE reps>0 AND date(created_at,'localtime')=date('now','localtime')",
        [],
        |row| row.get(0),
    )?)
}

pub fn run(parent: &Library, results: &mut Vec<Value>) -> Result<()> {
    let root = parent.root.join("review-queue");
    let lib = Library::open(root.join("mixed"))?;
    seed(&lib)?;
    let mut checks = vec![];

    for limit in [0, 1, 20] {
        lib.set_setting("new_cards_per_day", &limit.to_string())?;
        for kind in KINDS {
            let cards = lib.cards_for_kind(true, kind)?;
            let total = if kind == "word" { 24 } else { 4 };
            ensure!(
                cards.len() == limit.min(total),
                "{kind} 在限额 {limit} 下的数量错误"
            );
            ensure!(
                cards.iter().all(|card| card.kind == kind),
                "队列混入其他类型"
            );
            ensure!(
                lib.cards_for_kind(false, kind)?.len() == total,
                "非计划入口受新卡限额影响"
            );
            checks.push(
                json!({"case":"type_before_quota","limit":limit,"kind":kind,"queue":keys(&cards)}),
            );
        }
    }
    let all = lib.cards(true)?;
    ensure!(
        keys(&all) == keys(&lib.cards_for_kind(true, "all")?),
        "旧入口与 all 不兼容"
    );
    ensure!(all.len() == 20, "全类型没有正确使用 20 个新卡名额");
    for chunk in all.chunks(5) {
        ensure!(
            chunk.iter().map(|card| card.kind.as_str()).eq(KINDS),
            "全类型新卡没有按类型轮流入队"
        );
    }
    ensure!(
        lib.cards_for_kind(true, "invalid").is_err(),
        "错误类型没有拒绝"
    );
    let empty = Library::open(root.join("empty"))?;
    ensure!(empty.cards(true)?.is_empty(), "空库队列非空");
    checks.push(json!({"case":"all_types_round_robin","queue":keys(&all)}));

    let state = Library::open(root.join("schedule-states"))?;
    seed(&state)?;
    let sentence = state.cards_for_kind(false, "sentence")?;
    let db = state.db()?;
    for (index, reps, suspended, due) in [
        (0, 2, 0, "2000-01-01 00:00:00"),
        (1, 2, 0, "2999-01-01 00:00:00"),
        (2, 2, 1, "2000-01-01 00:00:00"),
        (3, 0, 1, "2000-01-01 00:00:00"),
    ] {
        db.execute("INSERT INTO review_schedule(card_kind,card_id,reps,suspended,due_at,created_at) VALUES('sentence',?,?,?,?,datetime('now','-1 day'))", params![sentence[index].id,reps,suspended,due])?;
    }
    // This is the same INSERT/UPDATE path as the app's suspend/resume action.
    let resumed = state.cards_for_kind(false, "video")?[0].clone();
    db.execute(
        "INSERT INTO review_schedule(card_kind,card_id,suspended) VALUES(?,?,1)",
        params![resumed.kind, resumed.id],
    )?;
    db.execute(
        "UPDATE review_schedule SET suspended=0 WHERE card_kind=? AND card_id=?",
        params![resumed.kind, resumed.id],
    )?;
    state.set_setting("new_cards_per_day", "0")?;
    ensure!(
        keys(&state.cards_for_kind(true, "sentence")?) == keys(&sentence[..1]),
        "零额度误挡到期旧卡或纳入未来/暂停卡"
    );
    ensure!(
        state.cards_for_kind(true, "video")?.is_empty(),
        "恢复的未评分卡绕过零额度"
    );
    ensure!(introduced(&state)? == 0, "仅暂停或恢复消耗当日新卡额度");
    state.set_setting("new_cards_per_day", "1")?;
    ensure!(
        state.cards_for_kind(true, "video")?.len() == 1,
        "恢复未评分卡没有参与新卡限额"
    );
    checks.push(json!({"case":"due_future_suspended_and_resumed","sentence_queue":keys(&state.cards_for_kind(true,"sentence")?),"video_queue":keys(&state.cards_for_kind(true,"video")?),"introduced":introduced(&state)?}));
    drop(db);

    let mut quota = Library::open(root.join("shared-quota"))?;
    seed(&quota)?;
    quota.set_setting("new_cards_per_day", "2")?;
    let first = quota.cards_for_kind(false, "sentence")?[0].clone();
    quota.db()?.execute("INSERT INTO review_schedule(card_kind,card_id,suspended,created_at) VALUES(?,?,0,datetime('now','-1 day'))",params![first.kind,first.id])?;
    quota.grade(&first, "good")?;
    ensure!(introduced(&quota)? == 1, "旧暂停调度首次评分未计入今天");
    let created = text(&quota.list("review_schedule")?[0], "created_at");
    for kind in KINDS {
        ensure!(
            quota.cards_for_kind(true, kind)?.len() == 1,
            "首次评分后的跨类型剩余额度不是 1"
        );
    }
    quota = Library::open(quota.root.clone())?;
    quota.grade(&first, "hard")?;
    ensure!(introduced(&quota)? == 1, "重复评分重复计入新卡额度");
    ensure!(
        text(&quota.list("review_schedule")?[0], "created_at") == created,
        "重复评分改变首次评分日期"
    );
    let second = quota
        .cards_for_kind(true, "video")?
        .into_iter()
        .next()
        .context("剩余额度没有视频新卡")?;
    quota.grade(&second, "good")?;
    quota = Library::open(quota.root.clone())?;
    ensure!(introduced(&quota)? == 2, "跨类型首次评分没有共享计数");
    for kind in KINDS {
        ensure!(
            quota.cards_for_kind(true, kind)?.is_empty(),
            "限额用尽后其他类型仍有新卡"
        );
    }
    let count: i64 =
        quota
            .db()?
            .query_row("SELECT SUM(review_count) FROM study_stats", [], |row| {
                row.get(0)
            })?;
    ensure!(count == 3, "重复评分没有正常写入复习统计");
    checks.push(json!({"case":"shared_quota_reopen_and_regrade","introduced":introduced(&quota)?,"review_count":count,"first_created_at":created,"schedules":quota.list("review_schedule")?}));

    let mut rotation = Library::open(root.join("small-quota-rotation"))?;
    seed(&rotation)?;
    rotation.set_setting("new_cards_per_day", "1")?;
    let mut cycle = vec![];
    for _ in 0..5 {
        let next = rotation
            .cards(true)?
            .into_iter()
            .next()
            .context("小限额轮转没有新卡")?;
        let before_read = rotation.setting("review_new_card_next_kind")?;
        ensure!(
            keys(&rotation.cards(true)?) == keys(std::slice::from_ref(&next)),
            "读取队列消耗候选或改变顺序"
        );
        ensure!(
            rotation.setting("review_new_card_next_kind")? == before_read,
            "读取队列推进轮转游标"
        );
        cycle.push(next.kind.clone());
        rotation.grade(&next, "good")?;
        let cursor = rotation.setting("review_new_card_next_kind")?;
        rotation.grade(&next, "hard")?;
        ensure!(
            rotation.setting("review_new_card_next_kind")? == cursor,
            "重复评分推进新卡轮转游标"
        );
        ensure!(
            rotation.cards(true)?.is_empty(),
            "限额 1 在评分后仍返回新卡"
        );
        // Move first-study dates back one day to repeat the next-day quota path deterministically.
        rotation.db()?.execute(
            "UPDATE review_schedule SET created_at=datetime('now','-1 day')",
            [],
        )?;
        rotation = Library::open(rotation.root.clone())?;
    }
    ensure!(
        cycle.iter().map(String::as_str).eq(KINDS),
        "小限额和重开后仍偏向固定表"
    );
    ensure!(
        cycle.iter().collect::<BTreeSet<_>>().len() == 5,
        "五类没有各自获得机会"
    );
    checks.push(json!({"case":"one_new_card_per_day_rotation_reopen","introduced_types":cycle,"next_kind":rotation.setting("review_new_card_next_kind")?}));

    let free = Library::open(root.join("unscheduled"))?;
    seed(&free)?;
    free.set_setting("new_cards_per_day", "0")?;
    let card = free.cards_for_kind(false, "ai")?[0].clone();
    free.grade(&card, "good")?;
    ensure!(
        introduced(&free)? == 1 && free.cards(true)?.is_empty(),
        "非计划学习被限额禁止或计划额度失效"
    );
    checks.push(json!({"case":"unscheduled_study_allowed","introduced":introduced(&free)?}));

    fs::write(
        root.join("review-queue-evidence.json"),
        serde_json::to_vec_pretty(
            &json!({"checks":checks,"databases":["mixed/clarora.db","schedule-states/clarora.db","shared-quota/clarora.db","small-quota-rotation/clarora.db","unscheduled/clarora.db"]}),
        )?,
    )?;
    results.push(json!({"workflow":"复习队列：真实 SQLite 五类卡片 → 类型先于额度 → 到期/暂停/恢复 → 首次评分共享额度与重开 → 小额度跨类型轮转", "passed":true,"evidence":"review-queue/review-queue-evidence.json"}));
    Ok(())
}
