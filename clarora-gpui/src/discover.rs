use crate::data::Card;
use std::collections::HashSet;
fn random(n: usize) -> usize {
    (uuid::Uuid::new_v4().as_u128() % n as u128) as usize
}
pub fn next(
    cards: &[Card],
    history: &[usize],
    current: usize,
    due: &HashSet<String>,
    favorites: &HashSet<String>,
) -> Option<usize> {
    if cards.is_empty() {
        return None;
    }
    let mut seen = history.iter().copied().collect::<HashSet<_>>();
    seen.insert(current);
    let mut pool = (0..cards.len())
        .filter(|i| !seen.contains(i))
        .collect::<Vec<_>>();
    if pool.is_empty() {
        let recent = history
            .iter()
            .rev()
            .take(20.min(cards.len().saturating_sub(1)))
            .copied()
            .collect::<HashSet<_>>();
        pool = (0..cards.len())
            .filter(|i| *i != current && !recent.contains(i))
            .collect();
    }
    if pool.is_empty() {
        pool.push(current.min(cards.len() - 1));
    }
    if let Some(last) = cards.get(current) {
        let varied = pool
            .iter()
            .copied()
            .filter(|i| {
                cards[*i].kind != last.kind
                    && (cards[*i].media.is_empty() || cards[*i].media != last.media)
            })
            .collect::<Vec<_>>();
        if !varied.is_empty() {
            pool = varied;
        }
    }
    let roll = random(100);
    let preferred = pool
        .iter()
        .copied()
        .filter(|i| {
            let key = format!("{}:{}", cards[*i].kind, cards[*i].id);
            if roll < 40 {
                due.contains(&key)
            } else if roll < 80 {
                !seen.contains(i)
            } else {
                favorites.contains(&key)
            }
        })
        .collect::<Vec<_>>();
    if !preferred.is_empty() {
        pool = preferred;
    }
    let mut kinds = pool
        .iter()
        .map(|i| cards[*i].kind.clone())
        .collect::<Vec<_>>();
    kinds.sort();
    kinds.dedup();
    let kind = &kinds[random(kinds.len())];
    let bucket = pool
        .into_iter()
        .filter(|i| &cards[*i].kind == kind)
        .collect::<Vec<_>>();
    Some(bucket[random(bucket.len())])
}
