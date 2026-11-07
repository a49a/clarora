//! Local schema metadata shared by repository, snapshot and revision tracking.
use super::Row;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardKind {
    Word,
    Sentence,
    Ai,
    Clip,
    Video,
}
impl CardKind {
    pub(super) const ALL: [Self; 5] = [
        Self::Word,
        Self::Sentence,
        Self::Ai,
        Self::Clip,
        Self::Video,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Word => "word",
            Self::Sentence => "sentence",
            Self::Ai => "ai",
            Self::Clip => "clip",
            Self::Video => "video",
        }
    }
    pub(super) fn from_name(kind: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|item| item.as_str() == kind)
    }
    pub(super) fn table(self) -> &'static TableSpec {
        TABLES
            .iter()
            .find(|spec| spec.card == Some(self))
            .expect("card table")
    }
    pub(super) fn text_fields(self) -> (&'static str, &'static str) {
        match self {
            Self::Word => ("word", "meaning"),
            Self::Sentence => ("text", "translation"),
            Self::Ai => ("question", "answer"),
            Self::Clip | Self::Video => ("en_text", "zh_text"),
        }
    }
}

pub(crate) struct TableSpec {
    pub name: &'static str,
    pub columns: &'static [&'static str],
    // A word's identity across devices is its spelling. SQLite still owns its integer ID.
    pub sync_key: &'static [&'static str],
    pub card: Option<CardKind>,
    pub media: &'static [&'static str],
}
impl TableSpec {
    pub fn identity(&self, row: &Row) -> Value {
        Value::Array(
            self.sync_key
                .iter()
                .map(|key| row.get(*key).cloned().unwrap_or_default())
                .collect(),
        )
    }
}

// Parent tables precede dependent rows. Keep this order for restore and reverse deletion.
pub(crate) const TABLES: &[TableSpec] = &[
    TableSpec {
        name: "words",
        columns: &["id", "word", "meaning", "source", "created_at"],
        sync_key: &["word"],
        card: Some(CardKind::Word),
        media: &[],
    },
    TableSpec {
        name: "listening_practices",
        columns: &["id", "name", "created_at"],
        sync_key: &["id"],
        card: None,
        media: &[],
    },
    TableSpec {
        name: "listening_audios",
        columns: &[
            "id",
            "practice_id",
            "name",
            "audio_uri",
            "subtitle_uri",
            "created_at",
        ],
        sync_key: &["id"],
        card: None,
        media: &["audio_uri", "subtitle_uri"],
    },
    TableSpec {
        name: "clip_cards",
        columns: &[
            "id",
            "en_text",
            "zh_text",
            "audio_uri",
            "start_ms",
            "end_ms",
            "created_at",
        ],
        sync_key: &["id"],
        card: Some(CardKind::Clip),
        media: &["audio_uri"],
    },
    TableSpec {
        name: "sentence_cards",
        columns: &["id", "text", "translation", "notes", "created_at"],
        sync_key: &["id"],
        card: Some(CardKind::Sentence),
        media: &[],
    },
    TableSpec {
        name: "ai_cards",
        columns: &["id", "question", "answer", "context_text", "created_at"],
        sync_key: &["id"],
        card: Some(CardKind::Ai),
        media: &[],
    },
    TableSpec {
        name: "video_clips",
        columns: &["id", "en_text", "zh_text", "video_uri", "created_at"],
        sync_key: &["id"],
        card: Some(CardKind::Video),
        media: &["video_uri"],
    },
    TableSpec {
        name: "review_schedule",
        columns: &[
            "card_kind",
            "card_id",
            "ease",
            "interval_days",
            "due_at",
            "reps",
            "lapses",
            "suspended",
            "last_graded_at",
            "created_at",
        ],
        sync_key: &["card_kind", "card_id"],
        card: None,
        media: &[],
    },
    TableSpec {
        name: "study_stats",
        columns: &["day", "listen_seconds", "review_count", "pomodoro_count"],
        sync_key: &["day"],
        card: None,
        media: &[],
    },
    TableSpec {
        name: "discover_favorites",
        columns: &["key", "payload"],
        sync_key: &["key"],
        card: None,
        media: &[],
    },
    TableSpec {
        name: "ai_records",
        columns: &["id", "kind", "payload", "created_at"],
        sync_key: &["id"],
        card: None,
        media: &[],
    },
];

pub(crate) const SETTINGS: TableSpec = TableSpec {
    name: "app_settings",
    columns: &["key", "value"],
    sync_key: &["key"],
    card: None,
    media: &[],
};

pub(crate) fn table_spec(name: &str) -> Option<&'static TableSpec> {
    TABLES.iter().find(|spec| spec.name == name)
}

pub(crate) const PREFERENCES: &[&str] = &[
    "video_captions_enabled",
    "video_subtitle_size",
    "subtitle_font_size",
    "blind_chinese_hint",
    "video_auto_next",
    "navigation_open",
    "last_page",
    "new_cards_per_day",
    "daily_limit",
    "review_new_card_next_kind",
    "tts_voice",
    "tts_speed",
    "library_group",
    "last_listening_audio",
    "last_listening_position",
    "last_video_position",
    "pdf_current",
];
