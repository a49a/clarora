import { unzipSync } from "fflate";
import { NativeModules, Platform } from "react-native";
import { FileSystem } from "../services/platform";
import { upsertWords } from "./database";

// ── Anki 卡组与文本导入(.apkg / .colpkg / .txt / .tsv)────────────────────────────────────────
// .apkg 是一个 ZIP,内含 collection.anki2(SQLite,经典格式)或
// collection.anki21(SQLite)。2.1.50+ 默认导出的 collection.anki21b 是
// 压缩的新格式,无法直接读,明确提示用户导出时勾选「支持旧版本」。
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

/** Anki 的选择提示:优先新版 SQLite,兼容经典格式,拒绝暂不支持的新版压缩格式。 */
export function pickCollectionEntry(entries: Record<string, Uint8Array>): Uint8Array {
  if (entries["collection.anki21b"]) {
    throw new Error("该卡组使用新版压缩格式(anki21b)。请在 Anki 导出时勾选「支持旧版本」后重新导出");
  }
  if (entries["collection.anki21"]) return entries["collection.anki21"];
  if (entries["collection.anki2"]) return entries["collection.anki2"];
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

/** Anki UTF-8 text: headers describe special columns; ordinary columns are note fields. */
export function parseAnkiText(text: string): Array<{ word: string; meaning: string }> {
  const lines = text.replace(/^\uFEFF/, "").replace(/\r\n?/g, "\n").split("\n");
  const headers = new Map<string, string>();
  let start = 0;
  while (start < lines.length && (!lines[start].trim() || lines[start].startsWith("#"))) {
    const match = /^#([^:]+):(.*)$/.exec(lines[start]);
    if (match) headers.set(match[1].trim().toLowerCase(), match[2]);
    start++;
  }
  const separators: Record<string, string> = {
    tab: "\t", comma: ",", semicolon: ";", space: " ", pipe: "|", colon: ":",
  };
  const rawSeparator = headers.get("separator");
  const separator = rawSeparator === undefined ? "\t"
    : separators[rawSeparator.trim().toLowerCase()] ?? rawSeparator;
  if (!["\t", ",", ";", " ", "|", ":"].includes(separator)) {
    throw new Error("Anki 文本的 separator 无效，请使用 Tab、Comma 或其他 Anki 支持的分隔符");
  }
  const html = (headers.get("html") ?? "false").trim().toLowerCase();
  if (html !== "true" && html !== "false") throw new Error("Anki 文本的 html 必须为 true 或 false");
  const special = new Set<number>();
  for (const key of ["tags column", "deck column", "notetype column", "guid column"]) {
    const raw = headers.get(key);
    if (raw === undefined) continue;
    const index = Number(raw.trim()) - 1;
    if (!/^\d+$/.test(raw.trim()) || !Number.isSafeInteger(index) || index < 0 || special.has(index)) {
      throw new Error(`Anki 文本的 ${key} 列号无效或与其他特殊列重复`);
    }
    special.add(index);
  }

  // Parse records before writing anything. Quotes can contain separators and physical newlines.
  const body = lines.slice(start).join("\n");
  const rows: string[][] = [];
  let row: string[] = [];
  let field = "";
  let quoted = false;
  let closedQuote = false;
  for (let i = 0; i <= body.length; i++) {
    const ch = body[i];
    if (quoted) {
      if (ch === undefined) throw new Error("Anki 文本存在未闭合的双引号，未导入任何卡片");
      if (ch === '"') {
        if (body[i + 1] === '"') { field += '"'; i++; }
        else { quoted = false; closedQuote = true; }
      } else field += ch;
      continue;
    }
    if (ch === "#" && row.length === 0 && field === "" && !closedQuote) {
      while (i < body.length && body[i] !== "\n") i++;
      continue;
    }
    if (ch === separator || ch === "\n" || ch === undefined) {
      row.push(field);
      field = "";
      closedQuote = false;
      if (ch !== separator) {
        if (row.some(value => value.trim())) rows.push(row);
        row = [];
      }
    } else if (closedQuote) {
      throw new Error("Anki 文本的双引号后应为分隔符或换行，未导入任何卡片");
    } else if (ch === '"' && field === "") {
      quoted = true;
    } else field += ch;
  }

  const columnNames = headers.get("columns")?.split(separator);
  const width = columnNames?.length ?? rows[0]?.length ?? 0;
  if (!rows.length) return [];
  if ([...special].some(index => index >= width)) throw new Error("Anki 文本的特殊列号超出实际列数");
  const ordinary = Array.from({ length: width }, (_, index) => index).filter(index => !special.has(index));
  const indexes = resolveFieldIndexes({ flds: ordinary.map(index => ({ name: columnNames?.[index] })) });
  const words: Array<{ word: string; meaning: string }> = [];
  for (const [index, values] of rows.entries()) {
    if (values.length !== width) throw new Error(`Anki 文本第 ${index + 1} 条记录的列数不一致，未导入任何卡片`);
    if (indexes.skip) continue;
    const clean = (value: string) => html === "true" ? htmlToText(stripCloze(value)) : stripCloze(value).trim();
    const word = clean(values[ordinary[indexes.front]]);
    const meaning = clean(values[ordinary[indexes.back]]);
    if (word && meaning) words.push({ word, meaning });
  }
  return words;
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

/** Import text notes through an independent native read-only SQLite handle. */
export async function importAnkiDeck(fileUri: string): Promise<{ imported: number; notice?: string }> {
  if (Platform.OS !== "macos" && Platform.OS !== "windows") {
    throw new Error("Anki 卡组导入目前支持桌面端(macOS / Windows);移动端无需导入,桌面导入的卡片会经「同步与备份」下发");
  }
  if (/\.(txt|tsv)$/i.test(fileUri)) {
    const words = parseAnkiText(await FileSystem.readAsStringAsync(fileUri));
    const imported = await upsertWords(words);
    return { imported, notice: "仅导入正面和背面文字，标签与牌组层级不迁移" };
  }
  const database = (Platform.OS === "windows" ? NativeModules.RNWindowsDatabase : NativeModules.RNFilePicker) as {
    querySnapshot?: (path: string, sql: string) => Promise<string>;
  } | undefined;
  if (!database?.querySnapshot) throw new Error("当前客户端缺少 Anki 读取组件，请更新后重试");
  const base64 = await FileSystem.readBase64Async(fileUri);
  // Media is not imported. Only inflate the collection entries, not every ZIP member.
  const entries = unzipSync(base64ToBytes(base64), {
    filter: (entry) => ["collection.anki2", "collection.anki21", "collection.anki21b"].includes(entry.name),
  });
  const collectionBytes = pickCollectionEntry(entries);
  const documents = await FileSystem.getDocumentDirectoryAsync();
  const tempName = `clarora-anki-${Date.now()}-${Math.random().toString(36).slice(2)}.anki2`;
  const tempPath = `${documents.replace(/[\\/]$/, "")}/${tempName}`;
  try {
    await FileSystem.writeBase64Async(tempPath, bytesToBase64(collectionBytes));
    const words = await parseAnkiNotes({
      executeSql: async (sql) => [{ rows: JSON.parse(await database.querySnapshot!(tempPath, sql)) }],
    });
    const imported = await upsertWords(words);
    return { imported };
  } finally {
    try { await FileSystem.deleteFileAsync(tempPath); } catch { /* Preserve the original import error. */ }
  }
}
