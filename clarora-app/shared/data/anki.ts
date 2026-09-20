import { unzipSync } from "fflate";
import { NativeModules, Platform } from "react-native";
import SQLite from "../services/sqlite";
import { FileSystem } from "../services/platform";
import { upsertWords } from "./database";

// ── Anki 卡组导入(.apkg / .colpkg)────────────────────────────────────────
// .apkg 是一个 ZIP,内含 collection.anki2(SQLite,经典格式)或
// collection.anki21(SQLite)。2.1.50+ 默认导出的 collection.anki21b 是
// zstd + protobuf,无法直接读,明确提示用户导出时勾选「支持旧版本」。
// 笔记字段是 HTML,清洗为纯文本;填空题把 {{cN::答案}} 还原为答案文本。

type AnkiField = { name?: string; ord?: number };
type AnkiModel = { name?: string; flds?: AnkiField[]; type?: number };
type AnkiSqlite = {
  executeSql: (sql: string, params?: unknown[]) => Promise<Array<{ rows: Array<Record<string, unknown>> }>>;
};

const BASE64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const FIELD_SEPARATOR = String.fromCharCode(31);
const CLOZE_RE = /\{\{c\d+::([^:{}]*)(?::([^{}]*))?\}\}/g;
const ANKI_MEDIA_RE = /(?:\[sound:[^\]]*\]|<img[^>]*>)/gi;
const INLINE_TAG_RE = /<\/?(b|i|u|s|em|strong|span|code|sup|sub|small|kbd|a|mark|font)\b[^>]*>/gi;

export function htmlToText(html: string): string {
  return html
    .replace(/<(style|script)[^>]*>[\s\S]*?<\/\1>/gi, " ")
    .replace(ANKI_MEDIA_RE, " ")
    .replace(INLINE_TAG_RE, "")
    .replace(/<br\s*\/?>/gi, "\n")
    .replace(/<\/(p|div|li|tr|h[1-6])>/gi, "\n")
    .replace(/<[^>]+>/g, " ")
    .replace(/&nbsp;/gi, " ")
    .replace(/&lt;/gi, "<")
    .replace(/&gt;/gi, ">")
    .replace(/&quot;/gi, '"')
    .replace(/&#39;|&apos;/gi, "'")
    .replace(/&amp;/gi, "&")
    .split("\n")
    .map((line) => line.replace(/[ \t]+/g, " ").trim())
    .filter(Boolean)
    .join("\n")
    .trim();
}

export function stripCloze(text: string): string {
  return text.replace(CLOZE_RE, (_, answer: string, hint?: string) => (hint ? `${answer}(${hint})` : answer));
}

function fieldIndex(fields: AnkiField[], candidates: string[]): number {
  for (const candidate of candidates) {
    const index = fields.findIndex((field) => (field.name ?? "").trim().toLowerCase() === candidate);
    if (index >= 0) return index;
  }
  return -1;
}

/** 从模型字段表推断正面/背面字段下标;识别不了时退回首两个字段。 */
export function resolveFieldIndexes(model: AnkiModel): { front: number; back: number; skip: boolean } {
  const fields = (model.flds ?? []).slice().sort((a, b) => (a.ord ?? 0) - (b.ord ?? 0));
  if (fields.length < 2) return { front: 0, back: -1, skip: true };
  const front = fieldIndex(fields, ["front", "frontside", "正面", "前面"]);
  const back = fieldIndex(fields, ["back", "背面", "后面", "答案"]);
  if (front >= 0 && back >= 0 && front !== back) return { front, back, skip: false };
  return { front: 0, back: 1, skip: false };
}

/** Anki 的选择提示:优先新版 SQLite,兼容经典格式,拒绝 zstd 加密格式。 */
export function pickCollectionEntry(entries: Record<string, Uint8Array>): Uint8Array {
  if (entries["collection.anki21"]) return entries["collection.anki21"];
  if (entries["collection.anki2"]) return entries["collection.anki2"];
  if (entries["collection.anki21b"]) {
    throw new Error("该卡组是新版加密格式(anki21b)。请在 Anki 导出时勾选「支持旧版本(Anki 2.1)」后重新导出");
  }
  throw new Error("卡包里没有找到 Anki 数据库(collection.anki2),请确认选择的是 .apkg / .colpkg 卡组文件");
}

/** 把 notes 行(mid/flds)按模型字段表映射为 (正面, 背面) 对。 */
export function notesFromRows(
  modelsJson: string,
  noteRows: Array<{ mid: number | string; flds: string }>,
): Array<{ word: string; meaning: string }> {
  const models: Record<string, AnkiModel> = JSON.parse(String(modelsJson || "{}"));
  const results: Array<{ word: string; meaning: string }> = [];
  for (const row of noteRows) {
    const model = models[String(row.mid)] ?? {};
    const values = String(row.flds ?? "").split(FIELD_SEPARATOR);
    const indexes = resolveFieldIndexes(model);
    if (indexes.skip) continue;
    const word = htmlToText(stripCloze(values[indexes.front] ?? ""));
    const meaning = htmlToText(stripCloze(values[indexes.back] ?? ""));
    if (word) results.push({ word, meaning });
  }
  return results;
}

/** 解析已打开的 Anki 库,返回 (正面, 背面) 对。 */
export async function parseAnkiNotes(sqlite: AnkiSqlite): Promise<Array<{ word: string; meaning: string }>> {
  const modelRows = await sqlite.executeSql("SELECT models FROM col LIMIT 1");
  const modelsJson = String(modelRows[0]?.rows?.[0]?.models ?? "{}");
  const noteRows = await sqlite.executeSql("SELECT mid, flds FROM notes");
  return notesFromRows(modelsJson, (noteRows[0]?.rows ?? []) as Array<{ mid: number; flds: string }>);
}

function base64ToBytes(base64: string): Uint8Array {
  const clean = base64.replace(/[^A-Za-z0-9+/]/g, "");
  const bytes = new Uint8Array(Math.floor((clean.length * 3) / 4));
  let out = 0;
  let buffer = 0;
  let bits = 0;
  for (const ch of clean) {
    const code = BASE64.indexOf(ch);
    if (code < 0) continue;
    buffer = (buffer << 6) | code;
    bits += 6;
    if (bits >= 8) {
      bits -= 8;
      bytes[out++] = (buffer >> bits) & 0xff;
    }
  }
  return bytes.subarray(0, out);
}

function bytesToBase64(bytes: Uint8Array): string {
  let out = "";
  for (let i = 0; i < bytes.length; i += 3) {
    const b0 = bytes[i];
    const b1 = bytes[i + 1];
    const b2 = bytes[i + 2];
    out += BASE64[b0 >> 2];
    out += BASE64[((b0 & 3) << 4) | ((b1 ?? 0) >> 4)];
    out += b1 === undefined ? "=" : BASE64[((b1 & 15) << 2) | ((b2 ?? 0) >> 6)];
    out += b2 === undefined ? "=" : BASE64[b2 & 63];
  }
  return out;
}

/**
 * 从 .apkg / .colpkg 卡组导入闪卡(upsert 合并进现有卡组)。
 * macOS 经原生 readOnly 打开;Windows 经 RNWindowsDatabase 的只读快照
 * 查询。移动端不做 Anki 导入,桌面导入的卡片经「同步与备份」下发。
 */
export async function importAnkiDeck(fileUri: string): Promise<{ imported: number }> {
  if (Platform.OS !== "macos" && Platform.OS !== "windows") {
    throw new Error("Anki 卡组导入目前支持桌面端(macOS / Windows);移动端无需导入,桌面导入的卡片会经「同步与备份」下发");
  }
  const base64 = await FileSystem.readBase64Async(fileUri);
  const collectionBytes = pickCollectionEntry(unzipSync(base64ToBytes(base64)));

  // 解出的 SQLite 临时文件写到应用目录:
  // - macOS 经原生 readOnly + assetFilename 直接只读打开;
  // - Windows 经 RNWindowsDatabase.querySnapshot 读取任意路径的行集。
  // 两者都不触碰应用自己的数据库。
  const documents = await FileSystem.getDocumentDirectoryAsync();
  const tempName = `clarora-anki-${Date.now()}.anki2`;
  const tempPath = `${documents}${tempName}`;
  await FileSystem.writeBase64Async(tempPath, bytesToBase64(collectionBytes));

  try {
    let words: Array<{ word: string; meaning: string }>;
    if (Platform.OS === "macos") {
      const db = await SQLite.openDatabase({
        name: `anki-${Date.now()}`,
        createFromLocation: tempName,
        readOnly: true,
      } as Parameters<typeof SQLite.openDatabase>[0]);
      try {
        words = await parseAnkiNotes({
          executeSql: async (sql, params) => {
            const results = await db.executeSql(sql, params as never[]);
            const first = Array.isArray(results) ? results[0] : results;
            const rows = first?.rows;
            const array = typeof rows?.raw === "function" ? rows.raw() : (rows?._array ?? []);
            return [{ rows: array as Array<Record<string, unknown>> }];
          },
        });
      } finally {
        try { await db.close(); } catch { /* 已断开则忽略 */ }
      }
    } else {
      const database = NativeModules.RNWindowsDatabase as {
        querySnapshot: (path: string, sql: string) => Promise<string>;
      };
      const modelRow = JSON.parse(await database.querySnapshot(tempPath, "SELECT models FROM col LIMIT 1"));
      const noteRows = JSON.parse(await database.querySnapshot(tempPath, "SELECT mid, flds FROM notes"));
      words = notesFromRows(modelRow[0]?.models ?? "{}", noteRows);
    }
    const imported = await upsertWords(words);
    return { imported };
  } finally {
    try { await FileSystem.deleteFileAsync(tempPath); } catch { /* 临时文件可能已不存在 */ }
  }
}
