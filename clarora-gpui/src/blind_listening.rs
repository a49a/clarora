//! Presentation-only blind listening. Playback remains owned by the existing player.
use crate::subtitles::Cue;
#[derive(Clone)]
pub struct Revealed {
    pub cue: Cue,
    pub sparse: bool,
}
#[derive(Default)]
pub struct State {
    pub enabled: bool,
    pub revealed: Option<Revealed>,
}
impl State {
    pub fn invalidate(&mut self) {
        self.revealed = None;
    }
    pub fn reference(&self) -> String {
        self.reference_for(false)
    }
    pub fn reference_for(&self, chinese_only: bool) -> String {
        if self.enabled {
            self.revealed
                .as_ref()
                .map(|r| {
                    if chinese_only {
                        chinese_hint(&r.cue.text)
                    } else {
                        r.cue.text.clone()
                    }
                })
                .unwrap_or_default()
        } else {
            String::new()
        }
    }
}
pub fn resolve(cues: &[Cue], position: f64, duration: f64) -> Option<Revealed> {
    if !position.is_finite() || position < 0. || !duration.is_finite() || duration <= 0. {
        return None;
    }
    let mut covering: Option<&Cue> = None;
    let mut ended: Option<&Cue> = None;
    for c in cues.iter().filter(|c| {
        c.start.is_finite()
            && c.end.is_finite()
            && c.start >= 0.
            && c.end > c.start
            && c.end <= duration
    }) {
        if c.start <= position && position < c.end && covering.is_none_or(|p| c.start > p.start) {
            covering = Some(c);
        }
        if c.end <= position && ended.is_none_or(|p| c.end > p.end) {
            ended = Some(c);
        }
    }
    covering.or(ended).map(|c| Revealed {
        cue: c.clone(),
        sparse: covering.is_none() && position - c.end > 10.,
    })
}

/// Resolve sentence navigation without changing the original timeline.
/// A revealed snapshot stays the anchor until the user explicitly resumes/seeks.
/// A/B navigation only offers cues wholly inside the existing loop.
pub fn sentence_target(
    cues: &[Cue],
    position: f64,
    duration: f64,
    revealed: Option<&Revealed>,
    direction: i8,
    loop_range: Option<(f64, f64)>,
) -> Option<Revealed> {
    if !position.is_finite() || position < 0. || !duration.is_finite() || duration <= 0. {
        return None;
    }
    let mut timeline: Vec<Cue> = cues
        .iter()
        .filter(|c| {
            c.start.is_finite()
                && c.end.is_finite()
                && c.start >= 0.
                && c.end > c.start
                && c.end <= duration
                && loop_range.is_none_or(|(a, b)| c.start >= a && c.end <= b)
        })
        .cloned()
        .collect();
    timeline.sort_by(|a, b| a.start.total_cmp(&b.start));
    let anchor = revealed
        .filter(|r| {
            timeline
                .iter()
                .any(|c| c.start == r.cue.start && c.end == r.cue.end && c.text == r.cue.text)
        })
        .cloned()
        .or_else(|| resolve(&timeline, position, duration));
    if direction == 0 {
        return anchor;
    }
    let start = anchor.as_ref().map(|r| r.cue.start).unwrap_or(position);
    let cue = if direction < 0 {
        timeline.iter().rev().find(|c| c.start < start)
    } else if anchor.is_none() {
        timeline.iter().find(|c| c.start >= start)
    } else {
        timeline.iter().find(|c| c.start > start)
    }?;
    Some(Revealed {
        cue: cue.clone(),
        sparse: false,
    })
}

/// Show only Chinese hints. Never fall back to an English-only cue.
/// Mixed-language lines keep Chinese while masking non-Chinese letters.
pub fn chinese_hint(text: &str) -> String {
    let han = |c: char| matches!(c as u32, 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff | 0x20000..=0x323af);
    text.lines()
        .filter(|line| line.chars().any(han))
        .map(|line| {
            let mut masked = String::new();
            let mut in_word = false;
            for c in line.chars() {
                if c.is_alphabetic() && !han(c) {
                    if !in_word {
                        masked.push('▰');
                    }
                    in_word = true;
                } else {
                    masked.push(c);
                    in_word = false;
                }
            }
            masked.trim().to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A click must still refer to a valid cue in the current source timeline.
pub fn row_target(cues: &[Cue], clicked: &Cue, duration: f64) -> Option<f64> {
    (duration.is_finite()
        && duration > 0.
        && clicked.start.is_finite()
        && clicked.end.is_finite()
        && clicked.start >= 0.
        && clicked.end > clicked.start
        && clicked.end <= duration
        && cues
            .iter()
            .any(|c| c.start == clicked.start && c.end == clicked.end && c.text == clicked.text))
    .then_some(clicked.start)
}

/// Use exactly the same estimated word timing as the ordinary transcript.
pub fn word_target(cues: &[Cue], clicked: &Cue, word_index: usize, duration: f64) -> Option<f64> {
    row_target(cues, clicked, duration)?;
    let word = crate::subtitles::words(clicked)
        .into_iter()
        .nth(word_index)?;
    (word.start.is_finite()
        && word.end.is_finite()
        && word.start >= clicked.start
        && word.start < word.end
        && word.end <= clicked.end)
        .then_some(word.start)
}
