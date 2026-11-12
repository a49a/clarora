use super::{Card, CardKind, Library, column_text, now, text};
use anyhow::{Context, Result, ensure};
use chrono::{Local, Utc};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use std::collections::{BTreeMap, VecDeque};

impl Library {
    pub fn cards(&self, due: bool) -> Result<Vec<Card>> {
        self.cards_for_kind(due, "all")
    }
    pub fn cards_for_kind(&self, due: bool, kind: &str) -> Result<Vec<Card>> {
        let selected = if kind == "all" {
            None
        } else {
            Some(CardKind::from_name(kind).context("未知卡片类型")?)
        };
        let db = self.db()?;
        let mut cards = vec![];
        for card_kind in CardKind::ALL {
            if selected.is_some_and(|kind| kind != card_kind) {
                continue;
            }
            let spec = card_kind.table();
            let table = spec.name;
            let (front, back) = card_kind.text_fields();
            let media = spec.media.first().copied().unwrap_or("NULL");
            let timing = if card_kind == CardKind::Clip {
                "start_ms,end_ms"
            } else {
                "NULL,NULL"
            };
            let mut query = db.prepare(&format!(
                "SELECT id,{front},{back},{media},{timing} FROM {table} ORDER BY created_at,id"
            ))?;
            for card in query.query_map([], |row| {
                Ok(Card {
                    kind: card_kind.as_str().into(),
                    id: column_text(row, 0)?,
                    front: column_text(row, 1)?,
                    back: column_text(row, 2)?,
                    media: column_text(row, 3)?,
                    start: row
                        .get::<_, f64>(4)
                        .ok()
                        .filter(|n| n.is_finite())
                        .unwrap_or_default()
                        / 1000.,
                    end: row
                        .get::<_, f64>(5)
                        .ok()
                        .filter(|n| n.is_finite())
                        .unwrap_or_default()
                        / 1000.,
                })
            })? {
                cards.push(card?);
            }
        }
        if !due {
            return Ok(cards);
        }
        let schedules = self
            .list("review_schedule")?
            .into_iter()
            .map(|schedule| {
                (
                    (text(&schedule, "card_kind"), text(&schedule, "card_id")),
                    schedule,
                )
            })
            .collect::<BTreeMap<_, _>>();
        let now = now();
        let limit = self
            .setting("new_cards_per_day")?
            .parse::<usize>()
            .unwrap_or(20);
        let introduced: usize = self.db()?.query_row(
            "SELECT COUNT(*) FROM review_schedule WHERE reps>0 AND date(created_at,'localtime')=date('now','localtime')",
            [],
            |row| row.get(0),
        )?;
        let mut remaining = limit.saturating_sub(introduced);
        let mut scheduled = vec![];
        let mut new_cards: [VecDeque<Card>; CardKind::ALL.len()] =
            std::array::from_fn(|_| VecDeque::new());
        for card in cards {
            if let Some(schedule) = schedules.get(&(card.kind.clone(), card.id.clone())) {
                if schedule["suspended"].as_i64() != Some(0) {
                    continue;
                }
                if schedule["reps"].as_i64().unwrap_or(0) > 0 {
                    let due_at = text(schedule, "due_at");
                    if due_at <= now {
                        scheduled.push((due_at, card));
                    }
                    continue;
                }
            }
            // A schedule created only by suspend/resume is still an unintroduced card.
            let index = CardKind::ALL
                .iter()
                .position(|kind| kind.as_str() == card.kind)
                .unwrap();
            new_cards[index].push_back(card);
        }
        scheduled.sort_by(|(a_due, a), (b_due, b)| {
            (a_due, &a.kind, &a.id).cmp(&(b_due, &b.kind, &b.id))
        });
        let mut cards = scheduled
            .into_iter()
            .map(|(_, card)| card)
            .collect::<Vec<_>>();
        let next_kind = self.setting("review_new_card_next_kind")?;
        let start = CardKind::ALL
            .iter()
            .position(|kind| kind.as_str() == next_kind)
            .unwrap_or(0);
        while remaining > 0 {
            let mut added = false;
            for offset in 0..CardKind::ALL.len() {
                if remaining == 0 {
                    break;
                }
                if let Some(card) = new_cards[(start + offset) % CardKind::ALL.len()].pop_front() {
                    cards.push(card);
                    remaining -= 1;
                    added = true;
                }
            }
            if !added {
                break;
            }
        }
        Ok(cards)
    }
    pub fn update_card(&self, card: &Card, front: &str, back: &str) -> Result<()> {
        ensure!(!front.trim().is_empty(), "原文不能为空");
        let kind = CardKind::from_name(&card.kind).context("未知卡片类型")?;
        let table = kind.table().name;
        let (a, b) = kind.text_fields();
        let count = self.db()?.execute(
            &format!("UPDATE {table} SET {a}=?,{b}=? WHERE id=?"),
            params![front.trim(), back.trim(), card.id],
        )?;
        ensure!(count == 1, "卡片尚未保存，请先收藏后编辑");
        Ok(())
    }
    pub fn grade(&self, card: &Card, grade: &str) -> Result<String> {
        ensure!(["again", "hard", "good"].contains(&grade), "评分无效");
        let mut db = self.db()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (reps, ease, interval, mut lapses) = tx
            .query_row(
                "SELECT reps,ease,interval_days,lapses FROM review_schedule WHERE card_kind=? AND card_id=?",
                params![card.kind, card.id],
                |row| Ok((row.get::<_, f64>(0)?, row.get::<_, f64>(1)?, row.get::<_, f64>(2)?, row.get::<_, f64>(3)?)),
            )
            .optional()?
            .unwrap_or((0., 2.5, 0., 0.));
        let (next_ease, days) = if grade == "again" {
            if reps > 0. {
                lapses += 1.;
            }
            (
                if reps == 0. {
                    2.5
                } else {
                    (ease - 0.2).max(1.3)
                },
                10. / 1440.,
            )
        } else if reps == 0. {
            (2.5, if grade == "good" { 4. } else { 1. })
        } else if grade == "hard" {
            ((ease - 0.15).max(1.3), (interval * 1.2).clamp(1., 730.))
        } else {
            ((ease + 0.1).min(3.2), (interval * ease).clamp(1., 730.))
        };
        let graded_at = Utc::now();
        let due = (graded_at + chrono::Duration::seconds((days * 86400.) as i64))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let timestamp = graded_at.format("%Y-%m-%d %H:%M:%S").to_string();
        tx.execute("INSERT INTO review_schedule(card_kind,card_id,ease,interval_days,due_at,reps,lapses,last_graded_at,created_at) VALUES(?,?,?,?,?,?,?,?,?) ON CONFLICT(card_kind,card_id) DO UPDATE SET ease=excluded.ease,interval_days=excluded.interval_days,due_at=excluded.due_at,reps=excluded.reps,lapses=excluded.lapses,last_graded_at=excluded.last_graded_at,created_at=CASE WHEN review_schedule.reps=0 THEN excluded.created_at ELSE review_schedule.created_at END",params![card.kind,card.id,next_ease,days,due,reps+1.,lapses,timestamp,timestamp])?;
        tx.execute("INSERT INTO study_stats(day,review_count) VALUES(?,1) ON CONFLICT(day) DO UPDATE SET review_count=review_count+1",[graded_at.with_timezone(&Local).format("%Y-%m-%d").to_string()])?;
        if reps == 0.
            && let Some(index) = CardKind::ALL
                .iter()
                .position(|kind| kind.as_str() == card.kind)
        {
            tx.execute("INSERT INTO app_settings(key,value) VALUES('review_new_card_next_kind',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[CardKind::ALL[(index + 1) % CardKind::ALL.len()].as_str()])?;
        }
        tx.commit()?;
        Ok(due)
    }
}
