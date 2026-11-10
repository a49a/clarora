//! The shared player's current material and policy. Page preferences live elsewhere.
use crate::video_repeat;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Normal,
    Track,
    Ab,
    Group,
    Continuous,
    Sentence,
    SentenceThree,
    SentenceFive,
}
impl Mode {
    /// The persisted values and menu indices predate typed playback modes.
    pub fn from_index(index: usize) -> Self {
        match index {
            1 => Self::Track,
            2 => Self::Ab,
            3 => Self::Group,
            4 => Self::Continuous,
            5 => Self::Sentence,
            6 => Self::SentenceThree,
            7 => Self::SentenceFive,
            _ => Self::Normal,
        }
    }
    pub fn index(self) -> usize {
        match self {
            Self::Normal => 0,
            Self::Track => 1,
            Self::Ab => 2,
            Self::Group => 3,
            Self::Continuous => 4,
            Self::Sentence => 5,
            Self::SentenceThree => 6,
            Self::SentenceFive => 7,
        }
    }
    pub fn is_sentence(self) -> bool {
        matches!(
            self,
            Self::Sentence | Self::SentenceThree | Self::SentenceFive
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Owner {
    Audio(String),
    Speaking,
    Video,
    Card { kind: String, id: String },
    Background,
}

/// Invalidated even when reopening the same path for the same page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lease(u64);

#[derive(Default)]
pub struct Session {
    owner: Option<Owner>,
    generation: u64,
    pub path: Option<PathBuf>,
    pub mode: Mode,
    pub loop_a: Option<f64>,
    pub loop_b: Option<f64>,
    pub end_handled: bool,
    sentence: Option<video_repeat::Session>,
}
impl Session {
    pub fn claim(&mut self, owner: Owner, path: PathBuf, mode: Mode) -> Lease {
        self.release();
        self.owner = Some(owner);
        self.path = Some(path);
        self.mode = mode;
        Lease(self.generation)
    }
    pub fn release(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.owner = None;
        self.path = None;
        self.mode = Mode::Normal;
        self.loop_a = None;
        self.loop_b = None;
        self.sentence = None;
        self.end_handled = false;
    }
    pub fn revision(&self) -> u64 {
        self.generation
    }
    pub fn owner(&self) -> Option<&Owner> {
        self.owner.as_ref()
    }
    pub fn lease(&self) -> Option<Lease> {
        self.owner.as_ref().map(|_| Lease(self.generation))
    }
    pub fn is_current(&self, lease: Lease) -> bool {
        self.lease() == Some(lease)
    }
    pub fn is_audio(&self, id: &str) -> bool {
        matches!(self.owner(), Some(Owner::Audio(current)) if current == id)
    }
    pub fn is_video(&self) -> bool {
        self.owner() == Some(&Owner::Video)
    }
    pub fn sentence(&self) -> Option<video_repeat::Session> {
        self.sentence
    }
    pub fn audio_sentence(&self) -> Option<video_repeat::Session> {
        matches!(self.owner(), Some(Owner::Audio(_)))
            .then_some(self.sentence)
            .flatten()
    }
    pub fn video_sentence(&self) -> Option<video_repeat::Session> {
        self.is_video().then_some(self.sentence).flatten()
    }
    pub fn set_sentence(&mut self, lease: Lease, sentence: Option<video_repeat::Session>) -> bool {
        if !self.is_current(lease) {
            return false;
        }
        self.sentence = sentence;
        true
    }
    pub fn clear_sentence(&mut self, lease: Lease) -> bool {
        self.set_sentence(lease, None)
    }
    pub fn set_audio_sentence(&mut self, sentence: Option<video_repeat::Session>) -> bool {
        if !matches!(self.owner(), Some(Owner::Audio(_))) {
            return false;
        }
        self.sentence = sentence;
        true
    }
    pub fn set_video_sentence(&mut self, sentence: Option<video_repeat::Session>) -> bool {
        if !self.is_video() {
            return false;
        }
        self.sentence = sentence;
        true
    }
}
