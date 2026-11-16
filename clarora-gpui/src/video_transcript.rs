//! Plain English reading material, cached when video subtitle cues change.
use crate::subtitles::Cue;
use sha2::{Digest, Sha256};

#[derive(Default)]
pub struct Transcript {
    pub text: String,
    pub html: String,
    pub paragraphs: Vec<String>,
    /// Cue start time for each English paragraph, used to restore the reading
    /// position when the subtitle revision changes.
    pub paragraph_cues: Vec<f64>,
    pub paragraph_ends: Vec<f64>,
    pub cue_count: usize,
    pub word_count: usize,
    pub character_count: usize,
}

fn chinese(ch: char) -> bool {
    matches!(ch as u32, 0x3400..=0x9fff | 0xf900..=0xfaff | 0x20000..=0x323af)
}

impl Transcript {
    pub fn from_cues(cues: &[Cue]) -> Self {
        let paragraphs = cues
            .iter()
            .filter_map(|cue| {
                let paragraph = cue
                    .text
                    .lines()
                    .map(|line| quick_xml::escape::unescape(line).unwrap_or_else(|_| line.into()))
                    .filter(|line| {
                        line.chars().any(|ch| ch.is_ascii_alphabetic())
                            && !line.chars().any(chinese)
                    })
                    .flat_map(|line| {
                        line.split_whitespace()
                            .map(str::to_owned)
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                (!paragraph.is_empty()).then_some((paragraph, cue.start, cue.end))
            })
            .collect::<Vec<_>>();
        let cue_count = paragraphs.len();
        let text = paragraphs
            .iter()
            .map(|(paragraph, _, _)| paragraph.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        let (paragraphs, rest): (Vec<_>, Vec<_>) = paragraphs
            .into_iter()
            .map(|(paragraph, start, end)| (paragraph, (start, end)))
            .unzip();
        let (paragraph_cues, paragraph_ends): (Vec<_>, Vec<_>) = rest.into_iter().unzip();
        let word_count = text.split_whitespace().count();
        let character_count = text.chars().count();
        let html = if text.is_empty() {
            String::new()
        } else {
            format!(
                "<p>{}</p>",
                text.replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
                    .replace("\n\n", "</p><p>")
            )
        };
        Self {
            text,
            html,
            paragraphs,
            paragraph_cues,
            paragraph_ends,
            cue_count,
            word_count,
            character_count,
        }
    }

    /// Content revision of the English paragraphs, independent of translated
    /// cue text added later.
    pub fn revision(&self) -> String {
        let mut hash = Sha256::new();
        for ((paragraph, start), end) in self
            .paragraphs
            .iter()
            .zip(&self.paragraph_cues)
            .zip(&self.paragraph_ends)
        {
            hash.update(start.to_le_bytes());
            hash.update(end.to_le_bytes());
            hash.update(paragraph.as_bytes());
            hash.update([0xff]);
        }
        hex::encode(hash.finalize())
    }

    /// Paragraph whose cue start is closest to `time`. Used when an earlier
    /// English revision inserted or removed paragraphs.
    pub fn paragraph_for_time(&self, time: f64) -> usize {
        if let Some((index, _)) = self
            .paragraph_cues
            .iter()
            .enumerate()
            .filter(|(_, start)| (*start - time).abs() < 0.05)
            .min_by(|(_, a), (_, b)| {
                (*a - time)
                    .abs()
                    .partial_cmp(&(*b - time).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        {
            return index.min(self.paragraphs.len().saturating_sub(1));
        }
        if let Some(index) = self
            .paragraph_cues
            .iter()
            .zip(&self.paragraph_ends)
            .position(|(start, end)| time >= *start && time < *end)
        {
            return index.min(self.paragraphs.len().saturating_sub(1));
        }
        self.paragraph_cues
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                (*a - time)
                    .abs()
                    .partial_cmp(&(*b - time).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(index, _)| index)
            .unwrap_or(0)
            .min(self.paragraphs.len().saturating_sub(1))
    }

    /// Paragraph containing `time`; the last paragraph starts at or before it.
    pub fn paragraph_containing_time(&self, time: f64) -> usize {
        self.paragraph_cues
            .iter()
            .rposition(|start| *start <= time)
            .unwrap_or(0)
            .min(self.paragraphs.len().saturating_sub(1))
    }
}
