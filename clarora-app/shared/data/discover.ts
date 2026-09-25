import type { ReviewCard, ListeningAudio } from './database';
import type { SubtitleCue } from './subtitles';

export type DiscoverItem = {
  key: string;
  kind: 'word' | 'ai' | 'clip';
  front: string;
  back: string;
  context?: string;
  source: string;
  audio?: { uri: string; startMs: number; endMs: number };
  generated?: boolean;
  subtitles?: DiscoverSubtitle[];
};

export type DiscoverSubtitle = { startMs: number; endMs: number; text: string; translation: string };

export function discoverSubtitles(cues: SubtitleCue[], startMs: number, endMs: number): DiscoverSubtitle[] {
  return cues.filter(c => Number.isFinite(c.start) && Number.isFinite(c.end) && c.end > c.start && c.end * 1000 > startMs && c.start * 1000 < endMs)
    .sort((a, b) => a.start - b.start).map(c => {
      const lines = c.text.split('\n').map(s => s.replace(/<[^>]+>/g, '').trim()).filter(Boolean);
      const original = lines.filter(s => !/[\u3400-\u9fff]/.test(s));
      return { startMs: Math.max(startMs, Math.round(c.start * 1000)), endMs: Math.min(endMs, Math.round(c.end * 1000)),
        text: (original.length ? original : lines).join(' '),
        translation: original.length ? lines.filter(s => /[\u3400-\u9fff]/.test(s)).join('\n') : '' };
    }).filter(c => c.text && c.endMs > c.startMs);
}

export function fromCard(card: ReviewCard): DiscoverItem | null {
  if (card.kind === 'video' || card.kind === 'sentence') return null;
  return {
    key: `${card.kind}:${card.id}`, kind: card.kind, front: card.front,
    back: card.back, source: card.kind === 'word' ? '单词卡' : card.kind === 'ai' ? 'AI 问答' : '音频片段卡',
    context: card.kind === 'ai' ? card.contextText : undefined,
    audio: card.kind === 'clip' ? { uri: card.audioUri, startMs: card.startMs, endMs: card.endMs } : undefined,
  };
}

export function audioSegments(audio: ListeningAudio, cues: SubtitleCue[]): DiscoverItem[] {
  const sorted = cues.filter(c => Number.isFinite(c.start) && Number.isFinite(c.end) && c.start >= 0 && c.end > c.start && c.text.trim())
    .sort((a, b) => a.start - b.start);
  const result: DiscoverItem[] = [];
  let group: SubtitleCue[] = [];
  const flush = () => {
    if (!group.length) return;
    const startMs = Math.round(group[0].start * 1000);
    const endMs = Math.round(group[group.length - 1].end * 1000);
    if (endMs - startMs >= 3000 && endMs - startMs <= 60000) {
      const lines = group.flatMap(c => c.text.split('\n')).map(s => s.replace(/<[^>]+>/g, '').trim()).filter(Boolean);
      const en = lines.filter(s => !/[\u3400-\u9fff]/.test(s));
      const zh = lines.filter(s => /[\u3400-\u9fff]/.test(s));
      result.push({ key: `segment:${audio.id}:${startMs}:${endMs}`, kind: 'clip',
        front: (en.length ? en : lines).join(' '), back: zh.join('\n'), source: audio.name,
        audio: { uri: audio.audio_uri, startMs, endMs }, subtitles: discoverSubtitles(group, startMs, endMs), generated: true });
    }
    group = [];
  };
  for (const cue of sorted) {
    if (group.length && (cue.end - group[0].start > 60 || cue.start - group[group.length - 1].end > 2)) flush();
    group.push(cue);
    if (cue.end - group[0].start >= 20 && /[.!?。！？]["”']?\s*$/.test(cue.text)) flush();
  }
  flush();
  return result;
}

// Exhaust unseen items before repeating. Category-first sampling prevents a
// large word deck from burying the audio and question cards.
export function pickNext(pool: DiscoverItem[], history: DiscoverItem[], due: Set<string>, favorites: Set<string>, random = Math.random): DiscoverItem | undefined {
  const seen = new Set(history.map(i => i.key));
  let candidates = pool.filter(i => !seen.has(i.key));
  if (!candidates.length) {
    const count = Math.min(20, Math.max(0, pool.length - 1));
    const recent = new Set((count ? history.slice(-count) : []).map(i => i.key));
    candidates = pool.filter(i => !recent.has(i.key));
  }
  const last = history[history.length - 1];
  const varied = candidates.filter(i => i.kind !== last?.kind && (!i.audio || i.audio.uri !== last?.audio?.uri));
  if (varied.length) candidates = varied;
  const roll = random();
  const preferred = candidates.filter(i => roll < .4 ? due.has(i.key) : roll < .8 ? !seen.has(i.key) : favorites.has(i.key));
  if (preferred.length) candidates = preferred;
  const kinds = [...new Set(candidates.map(i => i.kind))];
  const kind = kinds[Math.floor(random() * kinds.length)];
  const bucket = candidates.filter(i => i.kind === kind);
  return bucket[Math.floor(random() * bucket.length)];
}
