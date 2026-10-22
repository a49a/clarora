PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS words (id INTEGER PRIMARY KEY AUTOINCREMENT, word TEXT NOT NULL UNIQUE, meaning TEXT NOT NULL, source TEXT NOT NULL DEFAULT 'manual', created_at TEXT NOT NULL DEFAULT (datetime('now')));
CREATE TABLE IF NOT EXISTS listening_practices (id TEXT PRIMARY KEY, name TEXT NOT NULL, created_at TEXT NOT NULL DEFAULT (datetime('now')));
CREATE TABLE IF NOT EXISTS listening_audios (id TEXT PRIMARY KEY, practice_id TEXT NOT NULL, name TEXT NOT NULL, audio_uri TEXT NOT NULL, subtitle_uri TEXT NOT NULL, created_at TEXT NOT NULL DEFAULT (datetime('now')), FOREIGN KEY (practice_id) REFERENCES listening_practices(id) ON DELETE CASCADE);
CREATE TABLE IF NOT EXISTS clip_cards (id TEXT PRIMARY KEY, en_text TEXT NOT NULL, zh_text TEXT, audio_uri TEXT NOT NULL, start_ms INTEGER NOT NULL, end_ms INTEGER NOT NULL, created_at TEXT NOT NULL DEFAULT (datetime('now')));
CREATE TABLE IF NOT EXISTS sentence_cards (id TEXT PRIMARY KEY, text TEXT NOT NULL, translation TEXT NOT NULL DEFAULT '', notes TEXT NOT NULL DEFAULT '', created_at TEXT NOT NULL DEFAULT (datetime('now')));
CREATE TABLE IF NOT EXISTS ai_cards (id TEXT PRIMARY KEY, question TEXT NOT NULL, answer TEXT NOT NULL, context_text TEXT NOT NULL DEFAULT '', created_at TEXT NOT NULL DEFAULT (datetime('now')));
CREATE TABLE IF NOT EXISTS review_schedule (card_kind TEXT NOT NULL, card_id TEXT NOT NULL, ease REAL NOT NULL DEFAULT 2.5, interval_days REAL NOT NULL DEFAULT 0, due_at TEXT NOT NULL DEFAULT (datetime('now')), reps INTEGER NOT NULL DEFAULT 0, lapses INTEGER NOT NULL DEFAULT 0, suspended INTEGER NOT NULL DEFAULT 0, last_graded_at TEXT, created_at TEXT NOT NULL DEFAULT (datetime('now')), PRIMARY KEY (card_kind, card_id));
CREATE TABLE IF NOT EXISTS study_stats (day TEXT PRIMARY KEY, listen_seconds REAL NOT NULL DEFAULT 0, review_count INTEGER NOT NULL DEFAULT 0, pomodoro_count INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS video_clips (id TEXT PRIMARY KEY, en_text TEXT NOT NULL, zh_text TEXT, video_uri TEXT NOT NULL, created_at TEXT NOT NULL DEFAULT (datetime('now')));
CREATE TABLE IF NOT EXISTS ai_records (id TEXT PRIMARY KEY, kind TEXT NOT NULL, payload TEXT NOT NULL, created_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS discover_favorites (key TEXT PRIMARY KEY, payload TEXT NOT NULL);

CREATE TABLE IF NOT EXISTS local_credentials (name TEXT PRIMARY KEY, value TEXT NOT NULL);
