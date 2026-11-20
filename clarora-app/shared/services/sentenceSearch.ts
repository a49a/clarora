import { listListeningPractices, listLocalVideos } from "../data/database";
import { cueLanguages, parseSubtitleCues } from "../data/subtitles";
import { FileSystem } from "./platform";

// Library-wide sentence search for phones. It mirrors the desktop rules:
// tokens are compared case-insensitively, "prefix" matches from the start of a
// cue and "contains" matches any contiguous run. Only subtitle files already
// on the device are scanned; nothing is uploaded.

export type SentenceMatchMode = "prefix" | "contains";
export type SentenceScope = "all" | "audio" | "video";

export type SentenceQuery = {
  phrase: string;
  mode: SentenceMatchMode;
  scope: SentenceScope;
};

export type SentenceHit = {
  key: string;
  kind: "audio" | "video";
  sourceId: string;
  /** Practice id for audio hits; empty for videos. */
  practiceId: string;
  label: string;
  group: string;
  startMs: number;
  endMs: number;
  english: string;
  translation: string;
};

export type SentenceSearchResult = {
  hits: SentenceHit[];
  total: number;
  scanned: number;
  withoutSubtitles: number;
  unavailable: number;
  problems: string[];
};

export type SearchControl = { cancelled: boolean };

const MAX_RESULTS = 2000;
const MAX_PROBLEMS = 20;

function tokens(text: string): string[] {
  return text
    .replace(/’/g, "'")
    .split(/[^A-Za-z0-9']+/)
    .map((token) => token.replace(/^'+|'+$/g, "").toLowerCase())
    .filter(Boolean);
}

function matches(candidate: string[], query: string[], mode: SentenceMatchMode): boolean {
  if (!query.length || candidate.length < query.length) return false;
  if (mode === "prefix") {
    return query.every((word, index) => candidate[index] === word);
  }
  for (let start = 0; start + query.length <= candidate.length; start += 1) {
    if (query.every((word, index) => candidate[start + index] === word)) return true;
  }
  return false;
}

function problem(result: SentenceSearchResult, message: string) {
  if (result.problems.length < MAX_PROBLEMS) result.problems.push(message.slice(0, 400));
}

/** Let the UI breathe between subtitle files on slower phones. */
function yieldToUi(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

export async function searchSentences(
  query: SentenceQuery,
  control: SearchControl,
  onProgress?: (result: SentenceSearchResult) => void
): Promise<SentenceSearchResult> {
  const words = tokens(query.phrase);
  if (!words.length) throw new Error("请输入英文单词或短语，例如 When is");
  if (query.phrase.length > 120) throw new Error("句型最多 120 个字符");
  const result: SentenceSearchResult = {
    hits: [],
    total: 0,
    scanned: 0,
    withoutSubtitles: 0,
    unavailable: 0,
    problems: [],
  };
  const seen = new Set<string>();

  const scan = async (
    kind: "audio" | "video",
    sourceId: string,
    practiceId: string,
    label: string,
    group: string,
    subtitleUri: string
  ) => {
    if (control.cancelled) return;
    if (!subtitleUri || !/\.(srt|vtt|txt)$/i.test(subtitleUri)) {
      result.withoutSubtitles += 1;
      return;
    }
    result.scanned += 1;
    let cues;
    try {
      cues = parseSubtitleCues(await FileSystem.readAsStringAsync(subtitleUri));
    } catch {
      result.unavailable += 1;
      problem(result, `${label}：字幕文件不可用`);
      return;
    }
    if (!cues.length) {
      result.withoutSubtitles += 1;
      return;
    }
    for (const cue of cues) {
      if (control.cancelled) return;
      if (
        !Number.isFinite(cue.start) ||
        !Number.isFinite(cue.end) ||
        cue.start < 0 ||
        cue.end <= cue.start
      ) {
        continue;
      }
      const lines = cueLanguages(cue.text);
      const english = lines.english || cue.text;
      if (!matches(tokens(english), words, query.mode)) continue;
      result.total += 1;
      if (result.hits.length < MAX_RESULTS) {
        result.hits.push({
          key: `${kind}:${sourceId}:${cue.start}:${cue.end}:${result.total}`,
          kind,
          sourceId,
          practiceId,
          label,
          group,
          startMs: Math.round(cue.start * 1000),
          endMs: Math.round(cue.end * 1000),
          english,
          translation: lines.translation,
        });
      }
    }
  };

  if (query.scope !== "video") {
    try {
      const practices = await listListeningPractices();
      for (const practice of practices) {
        if (control.cancelled) break;
        for (const audio of practice.audios) {
          if (control.cancelled) break;
          const uri = audio.audio_uri;
          if (seen.has(uri)) continue;
          seen.add(uri);
          await scan("audio", audio.id, practice.id, audio.name, practice.name, audio.subtitle_uri);
        }
        onProgress?.({ ...result, hits: result.hits.slice() });
        await yieldToUi();
      }
    } catch (e: any) {
      problem(result, `音频库读取失败：${e?.message ?? e}`);
    }
  }


  if (query.scope !== "audio") {
    try {
      const videos = await listLocalVideos();
      for (const video of videos) {
        if (control.cancelled) break;
        if (seen.has(video.video_uri)) continue;
        seen.add(video.video_uri);
        await scan("video", video.id, "", video.name, "视频", video.subtitle_uri);
        onProgress?.({ ...result, hits: result.hits.slice() });
        await yieldToUi();
      }
    } catch (e: any) {
      problem(result, `视频库读取失败：${e?.message ?? e}`);
    }
  }

  return result;
}
