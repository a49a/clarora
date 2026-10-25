//! Immutable displayed transcript and UTF-8-safe clipboard selection, independent of playback.
use crate::subtitles::{self, Cue};
use std::{ops::Range, sync::Arc};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CueText {
    pub original: Range<usize>,
    pub translation: Range<usize>,
    pub words: Vec<Range<usize>>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Document {
    pub key: String,
    pub text: String,
    pub cues: Vec<CueText>,
}
impl Document {
    pub fn new(key: String, cues: &[Cue]) -> Self {
        let mut text = String::new();
        let mut parts = vec![];
        for cue in cues {
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            let (original, translation) = subtitles::languages(std::slice::from_ref(cue));
            let start = text.len();
            text.push_str(&original);
            let end = text.len();
            let mut cursor = 0;
            let mut words = vec![];
            for word in original.split_whitespace() {
                let i = original[cursor..].find(word).unwrap() + cursor;
                words.push(start + i..start + i + word.len());
                cursor = i + word.len();
            }
            if !translation.is_empty() {
                text.push('\n');
            }
            let zh_start = text.len();
            text.push_str(&translation);
            parts.push(CueText {
                original: start..end,
                translation: zh_start..text.len(),
                words,
            });
        }
        Self {
            key,
            text,
            cues: parts,
        }
    }
    fn boundary(&self, mut index: usize) -> usize {
        index = index.min(self.text.len());
        while !self.text.is_char_boundary(index) {
            index -= 1;
        }
        index
    }
}
#[derive(Default)]
pub struct Selection {
    pub document: Option<Arc<Document>>,
    pub anchor: usize,
    pub head: usize,
    pub selecting: bool,
    pub dragged: bool,
}
impl Selection {
    pub fn begin(&mut self, document: Arc<Document>, offset: usize) {
        let offset = document.boundary(offset);
        *self = Self {
            document: Some(document),
            anchor: offset,
            head: offset,
            selecting: true,
            dragged: false,
        };
    }
    pub fn extend(&mut self, offset: usize, moved: bool) {
        if self.selecting {
            if let Some(doc) = &self.document {
                self.head = doc.boundary(offset);
                self.dragged |= moved;
            }
        }
    }
    pub fn finish(&mut self) {
        self.selecting = false;
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn has_selection(&self) -> bool {
        self.dragged && self.anchor != self.head
    }
    pub fn range(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }
    pub fn text(&self) -> String {
        if !self.has_selection() {
            return String::new();
        }
        self.document
            .as_ref()
            .map(|d| d.text[self.range()].to_string())
            .unwrap_or_default()
    }
}
