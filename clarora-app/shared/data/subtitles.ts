export type SubtitleCue = {
  id: string;
  start: number;
  end: number;
  text: string;
};

function toSeconds(timestamp: string): number | null {
  const normalized = timestamp.replace(",", ".").trim();
  const parts = normalized.split(":");
  if (parts.length !== 3) return null;

  const hours = Number(parts[0]);
  const minutes = Number(parts[1]);
  const seconds = Number(parts[2]);

  if (Number.isNaN(hours) || Number.isNaN(minutes) || Number.isNaN(seconds))
    return null;

  return hours * 3600 + minutes * 60 + seconds;
}

export function parseSubtitleCues(content: string): SubtitleCue[] {
  const normalized = content.replace(/^\uFEFF/, "").replace(/\r+/g, "");

  // ── Format 1: [HH:MM:SS.mmm][HH:MM:SS.mmm]text (one cue per line) ──
  const bracketLineRe = /^\[([^\]]+)\]\[([^\]]+)\](.+)$/;
  const lines = normalized
    .split("\n")
    .map((l) => l.trim())
    .filter(Boolean);
  const isBracketFormat = lines.length > 0 && bracketLineRe.test(lines[0]);

  if (isBracketFormat) {
    const cues: SubtitleCue[] = [];
    for (const line of lines) {
      const m = line.match(bracketLineRe);
      if (!m) continue;
      const start = toSeconds(m[1]);
      const end = toSeconds(m[2]);
      const text = m[3].trim();
      if (start === null || end === null || !text) continue;
      cues.push({ id: `${start}-${end}-${cues.length}`, start, end, text });
    }
    return cues;
  }

  // ── Format 2: SRT / VTT (blocks separated by blank lines) ──
  const blocks = normalized
    .split(/\n\s*\n/)
    .map((block) => block.trim())
    .filter(Boolean);

  const cues: SubtitleCue[] = [];

  for (const block of blocks) {
    const bLines = block
      .split("\n")
      .map((line) => line.trim())
      .filter(Boolean);
    if (!bLines.length) continue;

    const timeLineIndex = bLines.findIndex((line) => line.includes("-->"));
    if (timeLineIndex === -1) continue;

    const [rawStart, rawEnd] = bLines[timeLineIndex]
      .split("-->")
      .map((part) => part.trim());
    const start = toSeconds(rawStart.split(" ")[0]);
    const end = toSeconds(rawEnd.split(" ")[0]);
    if (start === null || end === null) continue;

    const text = bLines
      .slice(timeLineIndex + 1)
      .join("\n")
      .trim();
    if (!text) continue;

    cues.push({ id: `${start}-${end}-${cues.length}`, start, end, text });
  }

  return cues;
}

const CJK_PATTERN = /[\u3040-\u30ff\u3400-\u4dbf\u4e00-\u9fff\uac00-\ud7af]/;

export type SubtitleLanguage = "bilingual" | "chinese" | "original";

/**
 * 按 cue 文本判断字幕的语言形态：
 * - bilingual：≥30% 的 cue 在首行之外带有中文（译中字的双语输出格式）
 * - chinese：整体以中文为主（replace 模式的翻译结果）
 * - original：其余情况
 */
export function classifySubtitleLanguage(cues: SubtitleCue[]): SubtitleLanguage {
  if (!cues.length) return "original";
  let withChinese = 0;
  let bilingual = 0;
  for (const cue of cues) {
    if (CJK_PATTERN.test(cue.text)) withChinese += 1;
    const lines = cue.text.split("\n").map(line => line.trim()).filter(Boolean);
    if (lines.length >= 2 && CJK_PATTERN.test(lines.slice(1).join("\n"))) bilingual += 1;
  }
  if (bilingual / cues.length >= 0.3) return "bilingual";
  if (withChinese / cues.length >= 0.6) return "chinese";
  return "original";
}

/** Add/replace one language, preserving the other language's timing and text. */
export function mergeSubtitleLanguage(existing: SubtitleCue[], incoming: SubtitleCue[], language: 'en' | 'zh'): SubtitleCue[] {
  const chinese = /[\u3400-\u4dbf\u4e00-\u9fff]/;
  const take = (cue: SubtitleCue, target: 'en' | 'zh') => cue.text.split('\n').map(line => line.trim()).filter(line => line && (target === 'zh' ? chinese.test(line) : !chinese.test(line))).join('\n');
  const valid = (cue: SubtitleCue) => Number.isFinite(cue.start) && Number.isFinite(cue.end) && cue.start >= 0 && cue.end > cue.start;
  const imported = incoming.filter(valid).map(cue => ({ ...cue, text: take(cue, language) })).filter(cue => cue.text);
  if (!imported.length) throw new Error(language === 'zh' ? '文件中没有可用的中文字幕，请选择带时间轴的中文 SRT 或 VTT 文件。' : '文件中没有可用的英文字幕，请选择带时间轴的英文 SRT 或 VTT 文件。');
  const other = existing.filter(valid).map(cue => ({ ...cue, text: take(cue, language === 'zh' ? 'en' : 'zh') })).filter(cue => cue.text);
  const english = language === 'en' ? imported : other;
  const translations = language === 'zh' ? imported : other;
  const points = [...new Set([...english, ...translations].flatMap(cue => [cue.start, cue.end]))].sort((a, b) => a - b);
  const result: SubtitleCue[] = [];
  for (let i = 0; i < points.length - 1; i++) {
    const start = points[i], end = points[i + 1];
    const active = (cues: SubtitleCue[]) => [...new Set(cues.filter(cue => cue.start < end && cue.end > start).map(cue => cue.text))].join('\n');
    const text = [active(english), active(translations)].filter(Boolean).join('\n');
    if (!text) continue;
    const last = result[result.length - 1];
    if (last && last.end === start && last.text === text) last.end = end;
    else result.push({ id: String(result.length + 1), start, end, text });
  }
  return result;
}

export function serializeSubtitleCues(cues: SubtitleCue[]): string {
  const time = (seconds: number) => {
    const ms = Math.round(seconds * 1000);
    return `${String(Math.floor(ms / 3600000)).padStart(2, '0')}:${String(Math.floor(ms / 60000) % 60).padStart(2, '0')}:${String(Math.floor(ms / 1000) % 60).padStart(2, '0')},${String(ms % 1000).padStart(3, '0')}`;
  };
  return cues.map((cue, i) => `${i + 1}\n${time(cue.start)} --> ${time(cue.end)}\n${cue.text}\n`).join('\n');
}
