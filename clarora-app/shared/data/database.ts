import { VAULT_TABLES, VAULT_COLUMNS, validateVault, type VaultData, type VaultRow } from "./vault";
import { Platform } from "react-native";
import SQLite, { type SQLiteDatabase } from "../services/sqlite";
import type { DiscoverItem } from './discover';

SQLite.enablePromise(true);

export type ListeningAudio = { id: string; name: string; audio_uri: string; subtitle_uri: string; created_at: string };
export type ListeningPractice = { id: string; name: string; audios: ListeningAudio[]; created_at: string };
export type SyncedWord = { word: string; meaning: string; source: string; created_at: string };
export type SyncedClip = {
  id: string;
  en_text: string;
  zh_text: string;
  audio_id: string;
  start_ms: number;
  end_ms: number;
  created_at: string;
};
export type SyncedAiCard = {
  id: string;
  question: string;
  answer: string;
  context_text: string;
  created_at: string;
};
export type SyncedSchedule = {
  card_kind: string;
  card_id: string;
  ease: number;
  interval_days: number;
  due_at: string;
  reps: number;
  lapses: number;
  suspended: number;
  last_graded_at: string;
  created_at: string;
};
export type SyncedAudio = Omit<ListeningAudio, "audio_uri" | "subtitle_uri"> & {
  audio_uri: string;
  subtitle_uri: string;
};
export type SyncedPractice = Omit<ListeningPractice, "audios"> & { audios: SyncedAudio[] };

let db: SQLiteDatabase | null = null;
let databaseReady: Promise<SQLiteDatabase> | null = null;

function getDb(): Promise<SQLiteDatabase> {
  if (!databaseReady) databaseReady = initializeDb().catch((error) => {
    db = null;
    databaseReady = null;
    throw error;
  });
  return databaseReady;
}

async function initializeDb(): Promise<SQLiteDatabase> {
  if (!db) {
    db = await SQLite.openDatabase({ name: "clarora.db", location: "default" });
    await db.transaction((tx: any) => {
      tx.executeSql("PRAGMA foreign_keys = ON");
      tx.executeSql("CREATE TABLE IF NOT EXISTS words (id INTEGER PRIMARY KEY AUTOINCREMENT, word TEXT NOT NULL UNIQUE, meaning TEXT NOT NULL, source TEXT NOT NULL DEFAULT 'manual', created_at TEXT NOT NULL DEFAULT (datetime('now')))");
      tx.executeSql("CREATE TABLE IF NOT EXISTS listening_practices (id TEXT PRIMARY KEY, name TEXT NOT NULL, created_at TEXT NOT NULL DEFAULT (datetime('now')))");
      tx.executeSql("CREATE TABLE IF NOT EXISTS listening_audios (id TEXT PRIMARY KEY, practice_id TEXT NOT NULL, name TEXT NOT NULL, audio_uri TEXT NOT NULL, subtitle_uri TEXT NOT NULL, created_at TEXT NOT NULL DEFAULT (datetime('now')), FOREIGN KEY (practice_id) REFERENCES listening_practices(id) ON DELETE CASCADE)");
      tx.executeSql("CREATE TABLE IF NOT EXISTS clip_cards (id TEXT PRIMARY KEY, en_text TEXT NOT NULL, zh_text TEXT, audio_uri TEXT NOT NULL, start_ms INTEGER NOT NULL, end_ms INTEGER NOT NULL, created_at TEXT NOT NULL DEFAULT (datetime('now')))");
      tx.executeSql("CREATE TABLE IF NOT EXISTS ai_cards (id TEXT PRIMARY KEY, question TEXT NOT NULL, answer TEXT NOT NULL, context_text TEXT NOT NULL DEFAULT '', created_at TEXT NOT NULL DEFAULT (datetime('now')))");
      // SM-2 调度表：所有卡片类型共用，card_id 统一按 TEXT 存（words 的整型 id 也转成字符串）。
      tx.executeSql("CREATE TABLE IF NOT EXISTS review_schedule (card_kind TEXT NOT NULL, card_id TEXT NOT NULL, ease REAL NOT NULL DEFAULT 2.5, interval_days REAL NOT NULL DEFAULT 0, due_at TEXT NOT NULL DEFAULT (datetime('now')), reps INTEGER NOT NULL DEFAULT 0, lapses INTEGER NOT NULL DEFAULT 0, suspended INTEGER NOT NULL DEFAULT 0, last_graded_at TEXT, created_at TEXT NOT NULL DEFAULT (datetime('now')), PRIMARY KEY (card_kind, card_id))");
      // 每日学习统计（设备本地，day = 本地日期 YYYY-MM-DD）。
      tx.executeSql("CREATE TABLE IF NOT EXISTS study_stats (day TEXT PRIMARY KEY, listen_seconds REAL NOT NULL DEFAULT 0, review_count INTEGER NOT NULL DEFAULT 0, pomodoro_count INTEGER NOT NULL DEFAULT 0)");
      tx.executeSql("CREATE TABLE IF NOT EXISTS video_clips (id TEXT PRIMARY KEY, en_text TEXT NOT NULL, zh_text TEXT, video_uri TEXT NOT NULL, created_at TEXT NOT NULL DEFAULT (datetime('now')))");
      tx.executeSql("CREATE TABLE IF NOT EXISTS ai_records (id TEXT PRIMARY KEY, kind TEXT NOT NULL, payload TEXT NOT NULL, created_at TEXT NOT NULL)");
      tx.executeSql("CREATE TABLE IF NOT EXISTS app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)");
      tx.executeSql("CREATE TABLE IF NOT EXISTS discover_favorites (key TEXT PRIMARY KEY, payload TEXT NOT NULL)");
    });
    // Enforce unique names. If an older database already contains duplicates,
    // the index creation fails and we fall back to app-level checks below.
    try {
      await db.executeSql("CREATE UNIQUE INDEX IF NOT EXISTS idx_listening_practices_name ON listening_practices(name COLLATE NOCASE)");
      await db.executeSql("CREATE UNIQUE INDEX IF NOT EXISTS idx_listening_audios_name ON listening_audios(practice_id, name COLLATE NOCASE)");
    } catch {
      // existing duplicate rows — rely on application-level checks
    }
    // Directory imports replace the whole collection, so each word tracks its
    // origin ('directory' | 'manual'). Older databases get the column added.
    try {
      await db.executeSql("ALTER TABLE words ADD COLUMN source TEXT NOT NULL DEFAULT 'manual'");
    } catch {
      // column already exists
    }
  }
  return db;
}

function generateId(): string { return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`; }

export async function getDiscoverFavorites(): Promise<DiscoverItem[]> {
  const database = await getDb();
  const [result] = await database.executeSql('SELECT payload FROM discover_favorites');
  return result.rows.raw().flatMap((row: { payload: string }) => {
    try { return [JSON.parse(row.payload) as DiscoverItem]; } catch { return []; }
  });
}

export async function setDiscoverFavorite(item: DiscoverItem, saved: boolean): Promise<void> {
  const database = await getDb();
  if (!saved) { await database.executeSql('DELETE FROM discover_favorites WHERE key = ?', [item.key]); return; }
  if (item.generated && item.audio) {
    // A generated segment becomes a normal, syncable audio flashcard too.
    await database.executeSql('INSERT INTO clip_cards (id, en_text, zh_text, audio_uri, start_ms, end_ms) SELECT ?, ?, ?, ?, ?, ? WHERE NOT EXISTS (SELECT 1 FROM clip_cards WHERE audio_uri = ? AND start_ms = ? AND end_ms = ?)',
      [generateId(), item.front, item.back, item.audio.uri, item.audio.startMs, item.audio.endMs, item.audio.uri, item.audio.startMs, item.audio.endMs]);
  }
  await database.executeSql('INSERT OR REPLACE INTO discover_favorites (key, payload) VALUES (?, ?)', [item.key, JSON.stringify(item)]);
}

type ReviewKind = "all" | "word" | "clip" | "video" | "ai";

export type ReviewCard =
  | { kind: "word"; id: number; front: string; back: string }
  | {
      kind: "clip";
      id: string;
      front: string;
      back: string;
      zhText: string;
      audioUri: string;
      startMs: number;
      endMs: number;
    }
  | {
      kind: "video";
      id: string;
      front: string;
      back: string;
      zhText: string;
      videoUri: string;
    }
  | {
      kind: "ai";
      id: string;
      front: string;
      back: string;
      contextText: string;
    };

// ── SM-2 计划复习 ───────────────────────────────────────────────────────────

export type ReviewGrade = "again" | "hard" | "good";
export type ScheduleState = { ease: number; intervalDays: number; reps: number; lapses: number };

const MAX_INTERVAL_DAYS = 730;
const MINUTES_PER_DAY = 1 / 1440;

/**
 * Anki 风格的 SM-2 简化版：
 * - 新卡首评：忘了 10 分钟后重练，模糊 1 天，认识 4 天
 * - 复习通过：间隔 × ease（认识额外 +0.1 ease），模糊按 1.2 倍但 ease -0.15
 * - 复习忘了：间隔重置为 10 分钟，ease -0.2（下限 1.3），lapses +1
 */
export function nextSm2(prev: ScheduleState, grade: ReviewGrade): ScheduleState {
  if (grade === "again") {
    return {
      ease: prev.reps === 0 ? 2.5 : Math.max(1.3, prev.ease - 0.2),
      intervalDays: 10 * MINUTES_PER_DAY,
      reps: prev.reps + 1,
      lapses: prev.reps === 0 ? prev.lapses : prev.lapses + 1,
    };
  }
  if (prev.reps === 0) {
    return {
      ease: 2.5,
      intervalDays: grade === "good" ? 4 : 1,
      reps: prev.reps + 1,
      lapses: prev.lapses,
    };
  }
  if (grade === "hard") {
    return {
      ease: Math.max(1.3, prev.ease - 0.15),
      intervalDays: Math.min(MAX_INTERVAL_DAYS, Math.max(1, prev.intervalDays * 1.2)),
      reps: prev.reps + 1,
      lapses: prev.lapses,
    };
  }
  return {
    ease: Math.min(3.2, prev.ease + 0.1),
    intervalDays: Math.min(MAX_INTERVAL_DAYS, Math.max(1, prev.intervalDays * prev.ease)),
    reps: prev.reps + 1,
    lapses: prev.lapses,
  };
}

export function formatDueLabel(intervalDays: number): string {
  if (intervalDays < 1) return `${Math.max(1, Math.round(intervalDays * 1440))} 分钟后`;
  const days = Math.round(intervalDays);
  if (days >= 365) return `${(days / 365).toFixed(days % 365 === 0 ? 0 : 1)} 年后`;
  if (days >= 30) return `${Math.round(days / 30)} 个月后`;
  return `${days} 天后`;
}

// ── 每日学习统计（设备本地）────────────────────────────────────────────────

function localDayKey(date = new Date()): string {
  const day = (value: number) => String(value).padStart(2, "0");
  return `${date.getFullYear()}-${day(date.getMonth() + 1)}-${day(date.getDate())}`;
}

async function bumpStudyStat(column: "listen_seconds" | "review_count" | "pomodoro_count", amount: number): Promise<void> {
  if (!(amount > 0)) return;
  const database = await getDb();
  await database.executeSql(
    `INSERT INTO study_stats (day, ${column}) VALUES (?, ?) ON CONFLICT(day) DO UPDATE SET ${column} = ${column} + excluded.${column}`,
    [localDayKey(), amount]
  );
}

/** 累计今日听力播放秒数（专注学习页播放中按实际时长上报）。 */
export function recordListenSeconds(seconds: number): Promise<void> {
  return bumpStudyStat("listen_seconds", seconds);
}

/** 番茄钟完成一个专注轮。 */
export function recordPomodoroRound(): Promise<void> {
  return bumpStudyStat("pomodoro_count", 1);
}

export type StudyDayStat = { day: string; listenSeconds: number; reviewCount: number; pomodoroCount: number };

export type StudyStats = {
  days: StudyDayStat[];
  listenTodaySec: number;
  listenWeekSec: number;
  listenTotalSec: number;
  reviewsToday: number;
  reviewsWeek: number;
  pomodoroToday: number;
  pomodoroTotal: number;
};

/** 最近 ``days`` 天的统计（含今天，缺失的天补 0），加常用汇总值。 */
export async function getStudyStats(days = 119): Promise<StudyStats> {
  const database = await getDb();
  const start = new Date();
  start.setDate(start.getDate() - (days - 1));
  const [rows] = await database.executeSql(
    "SELECT day, listen_seconds, review_count, pomodoro_count FROM study_stats WHERE day >= ? ORDER BY day ASC",
    [localDayKey(start)]
  );
  const byDay = new Map<string, StudyDayStat>();
  for (const row of rows.rows.raw() as Array<{ day: string; listen_seconds: number; review_count: number; pomodoro_count: number }>) {
    byDay.set(row.day, {
      day: row.day,
      listenSeconds: Number(row.listen_seconds) || 0,
      reviewCount: Number(row.review_count) || 0,
      pomodoroCount: Number(row.pomodoro_count) || 0,
    });
  }
  const series: StudyDayStat[] = [];
  const cursor = new Date(start);
  for (let i = 0; i < days; i++) {
    const key = localDayKey(cursor);
    series.push(byDay.get(key) ?? { day: key, listenSeconds: 0, reviewCount: 0, pomodoroCount: 0 });
    cursor.setDate(cursor.getDate() + 1);
  }
  const last = series[series.length - 1] ?? { day: "", listenSeconds: 0, reviewCount: 0, pomodoroCount: 0 };
  const week = series.slice(-7);
  const sum = (list: StudyDayStat[], pick: (day: StudyDayStat) => number) =>
    list.reduce((total, day) => total + pick(day), 0);
  return {
    days: series,
    listenTodaySec: last.listenSeconds,
    listenWeekSec: sum(week, (day) => day.listenSeconds),
    listenTotalSec: sum(series, (day) => day.listenSeconds),
    reviewsToday: last.reviewCount,
    reviewsWeek: sum(week, (day) => day.reviewCount),
    pomodoroToday: last.pomodoroCount,
    pomodoroTotal: sum(series, (day) => day.pomodoroCount),
  };
}

/** 给当前卡片评分并写入调度表，返回下次复习的可读时间。 */
export async function gradeReviewCard(card: ReviewCard, grade: ReviewGrade): Promise<string> {
  const database = await getDb();
  const kind = card.kind;
  const cardId = String(card.id);
  const [rows] = await database.executeSql(
    "SELECT ease, interval_days, reps, lapses FROM review_schedule WHERE card_kind = ? AND card_id = ?",
    [kind, cardId]
  );
  const row = (rows.rows.raw() as Array<{
    ease: number;
    interval_days: number;
    reps: number;
    lapses: number;
  }>)[0];
  const prev: ScheduleState = row
    ? {
        ease: Number(row.ease) || 2.5,
        intervalDays: Number(row.interval_days) || 0,
        reps: Number(row.reps) || 0,
        lapses: Number(row.lapses) || 0,
      }
    : { ease: 2.5, intervalDays: 0, reps: 0, lapses: 0 };
  const next = nextSm2(prev, grade);
  const nowIso = new Date().toISOString();
  const dueIso = new Date(Date.now() + next.intervalDays * 86_400_000).toISOString();
  await database.executeSql(
    "INSERT INTO review_schedule (card_kind, card_id, ease, interval_days, due_at, reps, lapses, suspended, last_graded_at) VALUES (?, ?, ?, ?, ?, ?, ?, 0, ?) ON CONFLICT(card_kind, card_id) DO UPDATE SET ease = excluded.ease, interval_days = excluded.interval_days, due_at = excluded.due_at, reps = excluded.reps, lapses = excluded.lapses, last_graded_at = excluded.last_graded_at",
    [kind, cardId, next.ease, next.intervalDays, dueIso, next.reps, next.lapses, nowIso]
  );
  // 复习量统计（热力图数据源）。
  await database.executeSql(
    "INSERT INTO study_stats (day, review_count) VALUES (?, 1) ON CONFLICT(day) DO UPDATE SET review_count = review_count + 1",
    [localDayKey()]
  );
  return formatDueLabel(next.intervalDays);
}

/**
 * 到期队列：到期卡在前（按 due_at），从未评分的新卡在后（限量）。
 * 新卡每日限量按"当天首次评分"的调度行数计。
 */
export async function getDueReviewCards(
  kind: ReviewKind = "all",
  newLimit = 20
): Promise<{ cards: ReviewCard[]; dueCount: number; newTotal: number; todayNewCount: number }> {
  const database = await getDb();
  const nowIso = new Date().toISOString();

  const countTodayNew = async (): Promise<number> => {
    const [rows] = await database.executeSql(
      "SELECT created_at FROM review_schedule WHERE reps >= 1"
    );
    const today = new Date();
    const sameLocalDay = (iso: string) => {
      const date = new Date(iso);
      return (
        date.getFullYear() === today.getFullYear() &&
        date.getMonth() === today.getMonth() &&
        date.getDate() === today.getDate()
      );
    };
    return (rows.rows.raw() as Array<{ created_at: string }>).filter(
      (row) => row.created_at && sameLocalDay(row.created_at)
    ).length;
  };

  const todayNewCount = await countTodayNew();
  const newQuota = Math.max(0, newLimit - todayNewCount);

  type DueCardEntry = { card: ReviewCard; isNew: boolean; dueAt: string };
  const collect = async (): Promise<DueCardEntry[]> => {
    const out: DueCardEntry[] = [];
    if (kind === "all" || kind === "word") {
      const [rows] = await database.executeSql(
        "SELECT w.id AS id, w.word, w.meaning, s.card_id IS NULL AS is_new, s.due_at FROM words w LEFT JOIN review_schedule s ON s.card_kind = 'word' AND s.card_id = CAST(w.id AS TEXT) WHERE s.card_id IS NULL OR (s.suspended = 0 AND s.due_at <= ?)",
        [nowIso]
      );
      for (const row of rows.rows.raw() as Array<{ id: number; word: string; meaning: string; is_new: number; due_at: string }>) {
        out.push({ card: { kind: "word", id: row.id, front: row.word, back: row.meaning }, isNew: !!row.is_new, dueAt: row.due_at ?? "" });
      }
    }
    if (kind === "all" || kind === "clip") {
      const [rows] = await database.executeSql(
        "SELECT c.id, c.en_text, c.zh_text, c.audio_uri, c.start_ms, c.end_ms, s.card_id IS NULL AS is_new, s.due_at FROM clip_cards c LEFT JOIN review_schedule s ON s.card_kind = 'clip' AND s.card_id = c.id WHERE s.card_id IS NULL OR (s.suspended = 0 AND s.due_at <= ?)",
        [nowIso]
      );
      for (const row of rows.rows.raw() as Array<{ id: string; en_text: string; zh_text: string | null; audio_uri: string; start_ms: number; end_ms: number; is_new: number; due_at: string }>) {
        out.push({
          card: { kind: "clip", id: row.id, front: row.en_text, back: row.zh_text ?? "", zhText: row.zh_text ?? "", audioUri: row.audio_uri, startMs: Number(row.start_ms) || 0, endMs: Number(row.end_ms) || 0 },
          isNew: !!row.is_new,
          dueAt: row.due_at ?? "",
        });
      }
    }
    if ((kind === "all" || kind === "video") && Platform.OS === "macos") {
      const [rows] = await database.executeSql(
        "SELECT v.id, v.en_text, v.zh_text, v.video_uri, s.card_id IS NULL AS is_new, s.due_at FROM video_clips v LEFT JOIN review_schedule s ON s.card_kind = 'video' AND s.card_id = v.id WHERE s.card_id IS NULL OR (s.suspended = 0 AND s.due_at <= ?)",
        [nowIso]
      );
      for (const row of rows.rows.raw() as Array<{ id: string; en_text: string; zh_text: string | null; video_uri: string; is_new: number; due_at: string }>) {
        out.push({
          card: { kind: "video", id: row.id, front: row.en_text, back: row.zh_text ?? "", zhText: row.zh_text ?? "", videoUri: row.video_uri },
          isNew: !!row.is_new,
          dueAt: row.due_at ?? "",
        });
      }
    }
    if (kind === "all" || kind === "ai") {
      const [rows] = await database.executeSql(
        "SELECT a.id, a.question, a.answer, a.context_text, s.card_id IS NULL AS is_new, s.due_at FROM ai_cards a LEFT JOIN review_schedule s ON s.card_kind = 'ai' AND s.card_id = a.id WHERE s.card_id IS NULL OR (s.suspended = 0 AND s.due_at <= ?)",
        [nowIso]
      );
      for (const row of rows.rows.raw() as Array<{ id: string; question: string; answer: string; context_text: string; is_new: number; due_at: string }>) {
        out.push({
          card: { kind: "ai", id: row.id, front: row.question, back: row.answer, contextText: row.context_text ?? "" },
          isNew: !!row.is_new,
          dueAt: row.due_at ?? "",
        });
      }
    }
    return out;
  };

  const all = await collect();
  const due = all.filter((item) => !item.isNew).sort((a, b) => (a.dueAt < b.dueAt ? -1 : 1));
  const fresh = all.filter((item) => item.isNew).slice(0, newQuota);
  return {
    cards: [...due.map((item) => item.card), ...fresh.map((item) => item.card)],
    dueCount: due.length,
    newTotal: all.length - due.length,
    todayNewCount,
  };
}

/** Unified review queue: word cards + captured clip cards + AI answer cards, shuffled. */
export async function getReviewCards(kind: ReviewKind = "all"): Promise<ReviewCard[]> {
  const database = await getDb();
  const wordCards: ReviewCard[] =
    kind === "all" || kind === "word"
      ? (
          (await database.executeSql("SELECT id, word, meaning FROM words ORDER BY RANDOM()"))[0]
            .rows.raw() as Array<{ id: number; word: string; meaning: string }>
        ).map((w) => ({ kind: "word" as const, id: w.id, front: w.word, back: w.meaning }))
      : [];
  const clipCards: ReviewCard[] =
    kind === "all" || kind === "clip"
      ? (
          (
            await database.executeSql(
              "SELECT id, en_text, zh_text, audio_uri, start_ms, end_ms FROM clip_cards ORDER BY RANDOM()"
            )
          )[0].rows.raw() as Array<{
            id: string;
            en_text: string;
            zh_text: string | null;
            audio_uri: string;
            start_ms: number;
            end_ms: number;
          }>
        ).map((c) => ({
          kind: "clip" as const,
          id: c.id,
          front: c.en_text,
          back: c.zh_text ?? "",
          zhText: c.zh_text ?? "",
          audioUri: c.audio_uri,
          startMs: Number(c.start_ms) || 0,
          endMs: Number(c.end_ms) || 0,
        }))
      : [];
  const videoCards: ReviewCard[] =
    // Video clip review needs the native libmpv view, which only exists on
    // the Mac build; other platforms keep video cards out of the queue.
    kind === "all" || kind === "video"
      ? Platform.OS === "macos"
        ? (
            (
              await database.executeSql(
                "SELECT id, en_text, zh_text, video_uri FROM video_clips ORDER BY RANDOM()"
              )
            )[0].rows.raw() as Array<{
              id: string;
              en_text: string;
              zh_text: string | null;
              video_uri: string;
            }>
          ).map((c) => ({
            kind: "video" as const,
            id: c.id,
            front: c.en_text,
            back: c.zh_text ?? "",
            zhText: c.zh_text ?? "",
            videoUri: c.video_uri,
          }))
        : []
      : [];
  const aiCards: ReviewCard[] =
    kind === "all" || kind === "ai"
      ? (
          (
            await database.executeSql(
              "SELECT id, question, answer, context_text FROM ai_cards ORDER BY RANDOM()"
            )
          )[0].rows.raw() as Array<{
            id: string;
            question: string;
            answer: string;
            context_text: string;
          }>
        ).map((card) => ({
          kind: "ai" as const,
          id: card.id,
          front: card.question,
          back: card.answer,
          contextText: card.context_text ?? "",
        }))
      : [];
  return [...wordCards, ...clipCards, ...videoCards, ...aiCards];
}

/** Save a captured video segment (video screen A/B loop) as a review card. */
export async function saveVideoClip(input: {
  enText: string;
  zhText: string;
  videoUri: string;
}): Promise<void> {
  const database = await getDb();
  await database.executeSql(
    "INSERT INTO video_clips (id, en_text, zh_text, video_uri) VALUES (?, ?, ?, ?)",
    [generateId(), input.enText, input.zhText, input.videoUri]
  );
}

/** Save a captured audio segment (listening AB-loop) as a review card. */
export async function saveClipCard(input: {
  enText: string;
  zhText: string;
  audioUri: string;
  startMs: number;
  endMs: number;
}): Promise<void> {
  const database = await getDb();
  await database.executeSql(
    "INSERT INTO clip_cards (id, en_text, zh_text, audio_uri, start_ms, end_ms) VALUES (?, ?, ?, ?, ?, ?)",
    [
      generateId(),
      input.enText,
      input.zhText,
      input.audioUri,
      Math.round(input.startMs),
      Math.round(input.endMs),
    ]
  );
}

/** Save an AI answer (listening page "向 AI 提问") as a Q&A review card. */
export async function saveAiCard(input: {
  question: string;
  answer: string;
  contextText: string;
}): Promise<void> {
  const question = input.question.trim();
  const answer = input.answer.trim();
  if (!question) throw new Error("问题不能为空");
  if (!answer) throw new Error("回答不能为空");
  const database = await getDb();
  await database.executeSql(
    "INSERT INTO ai_cards (id, question, answer, context_text) VALUES (?, ?, ?, ?)",
    [generateId(), question, answer, input.contextText.trim()]
  );
}

export async function upsertWords(words: Array<{ word: string; meaning: string }>): Promise<number> {
  const database = await getDb();
  let imported = 0;
  await database.transaction((tx: any) => {
    for (const item of words) {
      const word = item.word.trim();
      const meaning = item.meaning.trim();
      if (!word || !meaning) continue;
      tx.executeSql("INSERT INTO words (word, meaning) VALUES (?, ?) ON CONFLICT(word) DO UPDATE SET meaning = excluded.meaning", [word, meaning]);
      imported += 1;
    }
  });
  return imported;
}

/**
 * Replace the entire word collection with ``words``.
 *
 * Used by directory import: the directory is the source of truth, so words
 * removed from the directory (e.g. already-learned ones) disappear here too.
 * Retained words keep their row id (upsert instead of delete-all) so their
 * SM-2 schedules survive a re-import; removed words' schedules are cleaned up.
 */
export async function replaceWords(
  words: Array<{ word: string; meaning: string }>
): Promise<{ imported: number; replaced: number }> {
  const database = await getDb();
  const [countResult] = await database.executeSql("SELECT COUNT(*) AS n FROM words");
  const replaced: number = countResult.rows.raw()[0]?.n ?? 0;

  const incoming = new Map<string, string>();
  for (const item of words) {
    const word = item.word.trim();
    const meaning = item.meaning.trim();
    if (word && meaning) incoming.set(word, meaning);
  }

  const [existing] = await database.executeSql("SELECT id, word FROM words");
  const rows = existing.rows.raw() as Array<{ id: number; word: string }>;
  const removedIds = rows.filter((row) => !incoming.has(row.word)).map((row) => String(row.id));

  await database.transaction((tx: any) => {
    for (const id of removedIds) {
      tx.executeSql("DELETE FROM words WHERE id = ?", [id]);
      tx.executeSql("DELETE FROM review_schedule WHERE card_kind = 'word' AND card_id = ?", [id]);
    }
    for (const [word, meaning] of incoming) {
      tx.executeSql(
        "INSERT INTO words (word, meaning, source) VALUES (?, ?, 'directory') ON CONFLICT(word) DO UPDATE SET meaning = excluded.meaning, source = excluded.source",
        [word, meaning]
      );
    }
  });
  return { imported: incoming.size, replaced };
}

export async function listListeningPractices(): Promise<ListeningPractice[]> {
  const database = await getDb();
  const [practicesResult] = await database.executeSql("SELECT id, name, created_at FROM listening_practices ORDER BY created_at DESC");
  const practices: ListeningPractice[] = [];
  for (const practice of practicesResult.rows.raw() as Array<Omit<ListeningPractice, "audios">>) {
    const [audioResult] = await database.executeSql("SELECT id, name, audio_uri, subtitle_uri, created_at FROM listening_audios WHERE practice_id = ? ORDER BY created_at ASC", [practice.id]);
    practices.push({ ...practice, audios: audioResult.rows.raw() as ListeningAudio[] });
  }
  return practices;
}

async function practiceNameExists(database: SQLiteDatabase, name: string, excludeId?: string): Promise<boolean> {
  const [result] = excludeId
    ? await database.executeSql("SELECT id FROM listening_practices WHERE name = ? COLLATE NOCASE AND id != ?", [name, excludeId])
    : await database.executeSql("SELECT id FROM listening_practices WHERE name = ? COLLATE NOCASE", [name]);
  return result.rows.length > 0;
}

async function audioNameExists(database: SQLiteDatabase, practiceId: string, name: string, excludeId?: string): Promise<boolean> {
  const [result] = excludeId
    ? await database.executeSql("SELECT id FROM listening_audios WHERE practice_id = ? AND name = ? COLLATE NOCASE AND id != ?", [practiceId, name, excludeId])
    : await database.executeSql("SELECT id FROM listening_audios WHERE practice_id = ? AND name = ? COLLATE NOCASE", [practiceId, name]);
  return result.rows.length > 0;
}

export async function createListeningPractice(name: string): Promise<ListeningPractice> {
  const database = await getDb();
  const trimmed = name.trim();
  if (!trimmed) throw new Error("练习组名称不能为空");
  if (await practiceNameExists(database, trimmed)) throw new Error(`练习组“${trimmed}”已存在`);
  const id = generateId();
  await database.executeSql("INSERT INTO listening_practices (id, name) VALUES (?, ?)", [id, trimmed]);
  return { id, name: trimmed, audios: [], created_at: new Date().toISOString() };
}

export async function addAudioToPractice(practiceId: string, name: string, audioUri: string, subtitleUri: string): Promise<ListeningAudio> {
  const database = await getDb();
  const trimmed = name.trim();
  if (!trimmed) throw new Error("音频名称不能为空");
  if (await audioNameExists(database, practiceId, trimmed)) throw new Error(`音频“${trimmed}”已存在`);
  const id = generateId();
  await database.executeSql("INSERT INTO listening_audios (id, practice_id, name, audio_uri, subtitle_uri) VALUES (?, ?, ?, ?, ?)", [id, practiceId, trimmed, audioUri, subtitleUri]);
  return { id, name: trimmed, audio_uri: audioUri, subtitle_uri: subtitleUri, created_at: new Date().toISOString() };
}

export async function renameListeningPractice(practiceId: string, newName: string): Promise<void> {
  const database = await getDb();
  const trimmed = newName.trim();
  if (!trimmed) throw new Error("练习组名称不能为空");
  if (await practiceNameExists(database, trimmed, practiceId)) throw new Error(`练习组“${trimmed}”已存在`);
  await database.executeSql("UPDATE listening_practices SET name = ? WHERE id = ?", [trimmed, practiceId]);
}
export async function renameListeningAudio(audioId: string, newName: string): Promise<void> {
  const database = await getDb();
  const trimmed = newName.trim();
  if (!trimmed) throw new Error("音频名称不能为空");
  const [row] = await database.executeSql("SELECT practice_id FROM listening_audios WHERE id = ?", [audioId]);
  const practiceId = (row.rows.raw() as Array<{ practice_id: string }>)[0]?.practice_id;
  if (practiceId && (await audioNameExists(database, practiceId, trimmed, audioId))) {
    throw new Error(`音频“${trimmed}”已存在`);
  }
  await database.executeSql("UPDATE listening_audios SET name = ? WHERE id = ?", [trimmed, audioId]);
}
export async function updateAudioSubtitle(audioId: string, subtitleUri: string): Promise<void> {
  const database = await getDb();
  await database.executeSql("UPDATE listening_audios SET subtitle_uri = ? WHERE id = ?", [subtitleUri, audioId]);
}

export async function getSetting(key: string): Promise<string | null> {
  const database = await getDb();
  const [result] = await database.executeSql("SELECT value FROM app_settings WHERE key = ?", [key]);
  return (result.rows.raw() as Array<{ value: string }>)[0]?.value ?? null;
}

export async function setSetting(key: string, value: string): Promise<void> {
  const database = await getDb();
  await database.executeSql("INSERT INTO app_settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value", [key, value]);
}

/** Return the portable library metadata used by the private sync service. */
export async function getLibraryForSync(): Promise<{
  words: SyncedWord[];
  practices: SyncedPractice[];
  clips: Array<Omit<SyncedClip, "audio_id"> & { audio_uri: string }>;
  aiCards: SyncedAiCard[];
  schedules: SyncedSchedule[];
}> {
  const database = await getDb();
  const [wordsResult] = await database.executeSql(
    "SELECT word, meaning, source, created_at FROM words ORDER BY id ASC"
  );
  const [clipsResult] = await database.executeSql(
    "SELECT id, en_text, COALESCE(zh_text, '') AS zh_text, audio_uri, start_ms, end_ms, created_at FROM clip_cards ORDER BY created_at ASC"
  );
  const [aiCardsResult] = await database.executeSql(
    "SELECT id, question, answer, COALESCE(context_text, '') AS context_text, created_at FROM ai_cards ORDER BY created_at ASC"
  );
  const [schedulesResult] = await database.executeSql(
    "SELECT card_kind, card_id, ease, interval_days, due_at, reps, lapses, suspended, COALESCE(last_graded_at, '') AS last_graded_at, created_at FROM review_schedule"
  );
  return {
    words: wordsResult.rows.raw() as SyncedWord[],
    practices: await listListeningPractices(),
    clips: clipsResult.rows.raw() as Array<Omit<SyncedClip, "audio_id"> & { audio_uri: string }>,
    aiCards: aiCardsResult.rows.raw() as SyncedAiCard[],
    schedules: schedulesResult.rows.raw().map((row: SyncedSchedule) => ({ ...row, suspended: Number(row.suspended) || 0 })),
  };
}

/** Merge a downloaded library. Existing local-only records remain untouched. */
export async function mergeSyncedLibrary(input: {
  words: SyncedWord[];
  practices: SyncedPractice[];
  clips: SyncedClip[];
  aiCards?: SyncedAiCard[];
  schedules?: SyncedSchedule[];
}): Promise<{ words: number; practices: number; audios: number; clips: number; aiCards: number; schedules: number }> {
  const database = await getDb();
  let words = 0;
  let practices = 0;
  let audios = 0;
  let clips = 0;
  let aiCards = 0;
  let schedules = 0;

  for (const item of input.words) {
    const word = item.word.trim();
    const meaning = item.meaning.trim();
    if (!word || !meaning) continue;
    await database.executeSql(
      "INSERT INTO words (word, meaning, source, created_at) VALUES (?, ?, ?, ?) ON CONFLICT(word) DO UPDATE SET meaning = excluded.meaning, source = excluded.source",
      [word, meaning, item.source || "manual", item.created_at || new Date().toISOString()]
    );
    words += 1;
  }

  for (const practice of input.practices) {
    const name = practice.name.trim();
    if (!practice.id || !name) continue;
    const [byId] = await database.executeSql("SELECT id FROM listening_practices WHERE id = ?", [practice.id]);
    const [byName] = await database.executeSql(
      "SELECT id FROM listening_practices WHERE name = ? COLLATE NOCASE", [name]
    );
    const localId = (byId.rows.raw() as Array<{ id: string }>)[0]?.id
      ?? (byName.rows.raw() as Array<{ id: string }>)[0]?.id
      ?? practice.id;
    if (byId.rows.length || byName.rows.length) {
      await database.executeSql("UPDATE listening_practices SET name = ? WHERE id = ?", [name, localId]);
    } else {
      await database.executeSql(
        "INSERT INTO listening_practices (id, name, created_at) VALUES (?, ?, ?)",
        [localId, name, practice.created_at || new Date().toISOString()]
      );
    }
    practices += 1;

    for (const audio of practice.audios) {
      const audioName = audio.name.trim();
      if (!audio.id || !audioName || !audio.audio_uri) continue;
      const [audioById] = await database.executeSql("SELECT id FROM listening_audios WHERE id = ?", [audio.id]);
      const [audioByName] = await database.executeSql(
        "SELECT id FROM listening_audios WHERE practice_id = ? AND name = ? COLLATE NOCASE",
        [localId, audioName]
      );
      const localAudioId = (audioById.rows.raw() as Array<{ id: string }>)[0]?.id
        ?? (audioByName.rows.raw() as Array<{ id: string }>)[0]?.id
        ?? audio.id;
      if (audioById.rows.length || audioByName.rows.length) {
        await database.executeSql(
          "UPDATE listening_audios SET practice_id = ?, name = ?, audio_uri = ?, subtitle_uri = ? WHERE id = ?",
          [localId, audioName, audio.audio_uri, audio.subtitle_uri || "", localAudioId]
        );
      } else {
        await database.executeSql(
          "INSERT INTO listening_audios (id, practice_id, name, audio_uri, subtitle_uri, created_at) VALUES (?, ?, ?, ?, ?, ?)",
          [localAudioId, localId, audioName, audio.audio_uri, audio.subtitle_uri || "", audio.created_at || new Date().toISOString()]
        );
      }
      audios += 1;
    }
  }

  for (const clip of input.clips) {
    if (!clip.id || !clip.en_text.trim() || !clip.audio_id) continue;
    const practiceAudio = input.practices.flatMap((practice) => practice.audios)
      .find((audio) => audio.id === clip.audio_id);
    if (!practiceAudio) continue;
    await database.executeSql(
      "INSERT INTO clip_cards (id, en_text, zh_text, audio_uri, start_ms, end_ms, created_at) VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET en_text = excluded.en_text, zh_text = excluded.zh_text, audio_uri = excluded.audio_uri, start_ms = excluded.start_ms, end_ms = excluded.end_ms",
      [clip.id, clip.en_text, clip.zh_text || "", practiceAudio.audio_uri, clip.start_ms, clip.end_ms, clip.created_at || new Date().toISOString()]
    );
    clips += 1;
  }

  for (const card of input.aiCards ?? []) {
    if (!card.id || !card.question.trim() || !card.answer.trim()) continue;
    await database.executeSql(
      "INSERT INTO ai_cards (id, question, answer, context_text, created_at) VALUES (?, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET question = excluded.question, answer = excluded.answer, context_text = excluded.context_text",
      [card.id, card.question, card.answer, card.context_text || "", card.created_at || new Date().toISOString()]
    );
    aiCards += 1;
  }

  // 调度行按 last_graded_at 最后写入者获胜，避免多设备互相回退。
  for (const item of input.schedules ?? []) {
    if (!item.card_kind || !item.card_id) continue;
    await database.executeSql(
      "INSERT INTO review_schedule (card_kind, card_id, ease, interval_days, due_at, reps, lapses, suspended, last_graded_at, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(card_kind, card_id) DO UPDATE SET ease = excluded.ease, interval_days = excluded.interval_days, due_at = excluded.due_at, reps = excluded.reps, lapses = excluded.lapses, suspended = excluded.suspended, last_graded_at = excluded.last_graded_at WHERE COALESCE(excluded.last_graded_at, '') >= COALESCE(review_schedule.last_graded_at, '')",
      [
        item.card_kind,
        item.card_id,
        Number(item.ease) || 2.5,
        Number(item.interval_days) || 0,
        item.due_at || new Date().toISOString(),
        Number(item.reps) || 0,
        Number(item.lapses) || 0,
        item.suspended ? 1 : 0,
        item.last_graded_at || "",
        item.created_at || new Date().toISOString(),
      ]
    );
    schedules += 1;
  }
  return { words, practices, audios, clips, aiCards, schedules };
}
export async function deleteListeningPractice(practiceId: string): Promise<string[]> {
  const database = await getDb();
  const [rows] = await database.executeSql("SELECT audio_uri, subtitle_uri FROM listening_audios WHERE practice_id = ?", [practiceId]);
  const files = (rows.rows.raw() as Array<{ audio_uri: string; subtitle_uri: string }>)
    .flatMap((row) => [row.audio_uri, row.subtitle_uri])
    .filter(Boolean);
  await database.executeSql("DELETE FROM listening_audios WHERE practice_id = ?", [practiceId]);
  await database.executeSql("DELETE FROM listening_practices WHERE id = ?", [practiceId]);
  return files;
}
export async function deleteListeningAudio(audioId: string): Promise<string[]> {
  const database = await getDb();
  const [row] = await database.executeSql("SELECT audio_uri, subtitle_uri FROM listening_audios WHERE id = ?", [audioId]);
  const files = (row.rows.raw() as Array<{ audio_uri: string; subtitle_uri: string }>)[0];
  const list = files ? [files.audio_uri, files.subtitle_uri].filter(Boolean) : [];
  await database.executeSql("DELETE FROM listening_audios WHERE id = ?", [audioId]);
  return list;
}

/** Delete the current review card — a word, a captured clip, a video clip, or an AI answer. */
export async function deleteReviewCard(card: ReviewCard): Promise<void> {
  const database = await getDb();
  if (card.kind === "word") {
    await database.executeSql("DELETE FROM words WHERE id = ?", [card.id]);
  } else if (card.kind === "video") {
    await database.executeSql("DELETE FROM video_clips WHERE id = ?", [card.id]);
  } else if (card.kind === "ai") {
    await database.executeSql("DELETE FROM ai_cards WHERE id = ?", [card.id]);
  } else {
    await database.executeSql("DELETE FROM clip_cards WHERE id = ?", [card.id]);
  }
  await database.executeSql("DELETE FROM review_schedule WHERE card_kind = ? AND card_id = ?", [
    card.kind,
    String(card.id),
  ]);
}

/** Snapshot exports use explicit column allowlists: never app_settings or auth tokens. */
export async function exportVaultData(): Promise<VaultData> {
  const database = await getDb();
  const data = Object.fromEntries(VAULT_TABLES.map(table => [table, []])) as unknown as VaultData;
  const width = Math.max(...VAULT_TABLES.map(table => VAULT_COLUMNS[table].length));
  // One SQL statement gives all tables the same read snapshot, without JSON1 or native backup extensions.
  const query = VAULT_TABLES.map(table => {
    const columns: readonly string[] = VAULT_COLUMNS[table];
    return `SELECT '${table}' AS vault_table,${Array.from({ length: width }, (_, index) => `${columns[index] || 'NULL'} AS c${index}`).join(',')} FROM ${table}`;
  }).join(' UNION ALL ');
  const [result] = await database.executeSql(query);
  for (const record of result.rows.raw() as Array<Record<string, string | number | null>>) {
    const table = record.vault_table as keyof VaultData;
    const row: VaultRow = {};
    VAULT_COLUMNS[table].forEach((column, index) => { row[column] = record[`c${index}`]; });
    data[table].push(row);
  }
  return data;
}

/** Settings upserts applied inside the caller's transaction: markers such as
 * the restore-commit flag must commit (or roll back) atomically with the
 * vault rows they describe. */
function applySettingsUpdates(tx: any, settingsUpdates?: Record<string, string>): void {
  for (const [key, value] of Object.entries(settingsUpdates ?? {})) {
    tx.executeSql(
      "INSERT INTO app_settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
      [key, value],
    );
  }
}

/** Full replacement: clears all vault tables and inserts the backup data
 * inside a single transaction, so a mid-import failure rolls back to the
 * original state instead of leaving the database empty. */
export async function replaceVaultData(data: VaultData, settingsUpdates?: Record<string, string>): Promise<void> {
  validateVault(data);
  const database = await getDb();
  await database.transaction((tx: any) => {
    for (const table of VAULT_TABLES) tx.executeSql(`DELETE FROM ${table}`);
    for (const table of VAULT_TABLES) {
      for (const row of data[table]) {
        const columns = Object.keys(row);
        const placeholders = columns.map(() => '?').join(', ');
        const values = columns.map(col => (row as Record<string, unknown>)[col]);
        tx.executeSql(
          `INSERT INTO ${table} (${columns.join(', ')}) VALUES (${placeholders})`,
          values,
        );
      }
    }
    applySettingsUpdates(tx, settingsUpdates);
  });
}

/** Atomic additive restore. Local edits win; schedules take the later grade. */
export async function importVaultData(data: VaultData, settingsUpdates?: Record<string, string>): Promise<void> {
  validateVault(data);
  const database = await getDb();
  const current = await exportVaultData();
  const wordIds = new Map<string, number>();
  let nextWordId = current.words.reduce((max, row) => Math.max(max, Number(row.id)), 0) + 1;
  const wordsByText = new Map(current.words.map(row => [row.word, Number(row.id)]));
  for (const row of data.words) {
    const id = wordsByText.get(row.word) ?? nextWordId++;
    wordsByText.set(row.word, id);
    wordIds.set(String(row.id), id);
  }
  const practiceIds = new Map<string, string>();
  const practiceNames = new Map(current.listening_practices.map(row => [String(row.name).toLowerCase(), String(row.id)]));
  for (const row of data.listening_practices) {
    const id = current.listening_practices.find(local => local.id === row.id)?.id ?? practiceNames.get(String(row.name).toLowerCase()) ?? row.id;
    practiceIds.set(String(row.id), String(id));
    practiceNames.set(String(row.name).toLowerCase(), String(id));
  }
  await database.transaction((tx: any) => {
    for (const table of VAULT_TABLES) for (const original of data[table]) {
      const row = { ...original };
      if (table === 'words') row.id = wordIds.get(String(row.id))!;
      if (table === 'listening_practices') row.id = practiceIds.get(String(row.id))!;
      if (table === 'listening_audios') {
        const id = practiceIds.get(String(row.practice_id));
        if (!id) throw new Error('备份音频缺少练习组');
        row.practice_id = id;
      }
      if (table === 'review_schedule' && row.card_kind === 'word') {
        const id = wordIds.get(String(row.card_id));
        if (id === undefined) continue;
        row.card_id = String(id);
      }
      if (table === 'discover_favorites' && String(row.key).startsWith('word:')) {
        const id = wordIds.get(String(row.key).slice(5));
        if (id === undefined) continue;
        row.key = `word:${id}`;
        const payload = JSON.parse(String(row.payload));
        row.payload = JSON.stringify({ ...payload, key: row.key });
      }
      const columns = VAULT_COLUMNS[table];
      let suffix = '';
      if (table === 'review_schedule') {
        suffix = " ON CONFLICT(card_kind,card_id) DO UPDATE SET " + columns.filter(c => c !== 'card_kind' && c !== 'card_id').map(c => `${c}=excluded.${c}`).join(',') + " WHERE COALESCE(excluded.last_graded_at,'') > COALESCE(review_schedule.last_graded_at,'')";
      }
      // MAX is idempotent when the same backup is imported twice. It is not a sum of device activity.
      if (table === 'study_stats') suffix = ' ON CONFLICT(day) DO UPDATE SET ' + ['listen_seconds', 'review_count', 'pomodoro_count'].map(c => `${c}=MAX(study_stats.${c},excluded.${c})`).join(',');
      tx.executeSql(`INSERT INTO ${table} (${columns.join(',')}) VALUES (${columns.map(() => '?').join(',')})${suffix || " ON CONFLICT DO NOTHING"}`, columns.map(c => row[c]));
    }
    applySettingsUpdates(tx, settingsUpdates);
  });
}

export async function saveAiRecord(id: string, kind: string, payload: unknown): Promise<void> {
  const database = await getDb();
  await database.executeSql('INSERT INTO ai_records(id,kind,payload,created_at) VALUES(?,?,?,?) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload', [id, kind, JSON.stringify(payload), new Date().toISOString()]);
}
export async function listAiRecords<T>(kind: string): Promise<T[]> {
  const database = await getDb();
  const [result] = await database.executeSql('SELECT payload FROM ai_records WHERE kind=? ORDER BY created_at DESC', [kind]);
  return (result.rows.raw() as Array<{payload: string}>).map(row => JSON.parse(row.payload) as T);
}
export async function deleteAiRecord(id: string): Promise<void> {
  const database = await getDb();
  await database.executeSql('DELETE FROM ai_records WHERE id=?', [id]);
}
