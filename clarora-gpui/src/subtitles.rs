use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Cue {
    pub start: f64,
    pub end: f64,
    pub text: String,
}
pub struct Word {
    pub text: String,
    pub start: f64,
    pub end: f64,
}
/// Matches the original client's character-offset interpolation; not acoustic word alignment.
pub fn words(cue: &Cue) -> Vec<Word> {
    let (original, _) = languages(std::slice::from_ref(cue));
    let count = original.chars().count().max(1) as f64;
    let mut cursor = 0;
    original
        .split_whitespace()
        .map(|word| {
            let start = original[cursor..]
                .find(word)
                .map(|i| i + cursor)
                .unwrap_or(cursor);
            let offset = original[..start].chars().count() as f64;
            cursor = start + word.len();
            Word {
                text: word.into(),
                start: cue.start + (cue.end - cue.start) * offset / count,
                end: cue.start
                    + (cue.end - cue.start) * (offset + word.chars().count() as f64) / count,
            }
        })
        .collect()
}
pub fn excerpt(cues: &[Cue], a: f64, b: f64) -> (String, String) {
    let selected = cues
        .iter()
        .filter(|c| c.end > a && c.start < b)
        .cloned()
        .collect::<Vec<_>>();
    let text = selected
        .iter()
        .flat_map(words)
        .filter(|w| w.end > a + 0.0001 && w.start < b - 0.0001)
        .map(|w| w.text)
        .collect::<Vec<_>>()
        .join(" ");
    let (_, translation) = languages(&selected);
    (text, translation)
}
fn timestamp(text: &str) -> Option<f64> {
    let parts = text
        .replace(',', ".")
        .split(':')
        .map(str::parse::<f64>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .ok()?;
    let time = match parts.as_slice() {
        [m, s] => m * 60. + s,
        [h, m, s] => h * 3600. + m * 60. + s,
        _ => return None,
    };
    (time.is_finite() && time >= 0.).then_some(time)
}
pub fn parse(body: &str) -> Vec<Cue> {
    let body = body.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let lines = body.lines().collect::<Vec<_>>();
    let mut cues = vec![];
    let mut i = 0;
    let tags = regex::Regex::new("<[^>]*>").unwrap();
    while i < lines.len() {
        if let Some((a, b)) = lines[i].split_once("-->") {
            if let (Some(start), Some(end)) = (
                timestamp(a.trim()),
                timestamp(b.split_whitespace().next().unwrap_or("")),
            ) {
                i += 1;
                let mut text = vec![];
                while i < lines.len() && !lines[i].trim().is_empty() {
                    text.push(lines[i]);
                    i += 1
                }
                let text = tags.replace_all(&text.join("\n"), "").to_string();
                if end > start && !text.trim().is_empty() {
                    cues.push(Cue { start, end, text });
                }
                continue;
            }
        }
        i += 1;
    }
    if cues.is_empty() && !body.contains("-->") {
        let mut start = 0.;
        for line in lines {
            if !line.trim().is_empty() {
                let end = start + (line.chars().count() as f64 / 13.).clamp(2., 12.);
                cues.push(Cue {
                    start,
                    end,
                    text: line.trim().into(),
                });
                start = end;
            }
        }
    }
    cues.sort_by(|a, b| a.start.total_cmp(&b.start));
    cues
}
fn stamp(t: f64) -> String {
    let n = (t * 1000.).round() as u64;
    format!(
        "{:02}:{:02}:{:02},{:03}",
        n / 3600000,
        n / 60000 % 60,
        n / 1000 % 60,
        n % 1000
    )
}
pub fn srt(cues: &[Cue]) -> String {
    cues.iter()
        .enumerate()
        .map(|(i, c)| {
            format!(
                "{}\n{} --> {}\n{}\n\n",
                i + 1,
                stamp(c.start),
                stamp(c.end),
                c.text
            )
        })
        .collect()
}
fn chinese(text: &str) -> bool {
    text.chars().any(|c| ('\u{3400}'..='\u{9fff}').contains(&c))
}
pub fn merge(existing: &[Cue], new: &[Cue], zh: bool) -> Result<Vec<Cue>> {
    ensure!(!new.is_empty(), "字幕没有有效时间轴");
    ensure!(
        new.iter().any(|c| c.text.lines().any(|s| chinese(s) == zh)),
        "字幕没有所选语言"
    );
    let mut times = existing
        .iter()
        .chain(new)
        .flat_map(|c| [c.start, c.end])
        .collect::<Vec<_>>();
    times.sort_by(f64::total_cmp);
    times.dedup();
    let mut output: Vec<Cue> = vec![];
    for pair in times.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let mid = (a + b) / 2.;
        let old = existing
            .iter()
            .filter(|c| c.start <= mid && c.end > mid)
            .flat_map(|c| c.text.lines())
            .filter(|line| chinese(line) != zh);
        let incoming = new
            .iter()
            .filter(|c| c.start <= mid && c.end > mid)
            .flat_map(|c| c.text.lines())
            .filter(|line| chinese(line) == zh);
        let mut text = old.chain(incoming).collect::<Vec<_>>();
        text.sort_by_key(|s| chinese(s));
        text.dedup();
        let text = text.join("\n");
        if text.is_empty() {
            continue;
        }
        if let Some(last) = output.last_mut() {
            if last.text == text && (last.end - a).abs() < 0.001 {
                last.end = b;
                continue;
            }
        }
        output.push(Cue {
            start: a,
            end: b,
            text,
        });
    }
    Ok(output)
}
pub fn languages(cues: &[Cue]) -> (String, String) {
    let all = cues.iter().flat_map(|c| c.text.lines()).collect::<Vec<_>>();
    let en = all
        .iter()
        .filter(|s| !chinese(s))
        .copied()
        .collect::<Vec<_>>()
        .join("\n");
    let zh = all
        .iter()
        .filter(|s| chinese(s))
        .copied()
        .collect::<Vec<_>>()
        .join("\n");
    if en.is_empty() {
        (zh, String::new())
    } else {
        (en, zh)
    }
}
