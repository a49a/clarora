//! Plain English reading material, cached when video subtitle cues change.
use crate::subtitles::Cue;

#[derive(Default)]
pub struct Transcript {
    pub text: String,
    pub html: String,
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
                (!paragraph.is_empty()).then_some(paragraph)
            })
            .collect::<Vec<_>>();
        let cue_count = paragraphs.len();
        let text = paragraphs.join("\n\n");
        drop(paragraphs);
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
            cue_count,
            word_count,
            character_count,
        }
    }
}
