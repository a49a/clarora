// 盲听(先听后核对)核心逻辑:定位规则、请求状态机与 React 绑定。
// 设计:internal-docs/blind-listening-design.md §4.2/§7。
// 播放位置、倍速、播放状态均以听力页现有播放器为权威;本模块不创建
// Sound 实例、不保存第二份播放状态,只管理呈现模式与揭晓快照。

import { useSyncExternalStore } from 'react';

export type ListeningPresentation = 'subtitles' | 'blind';

export interface BlindCue {
  id: string;
  startMs: number;
  endMs: number;
  text: string;
}

export interface RevealedCue {
  audioId: string;
  cueId: string;
  startMs: number;
  endMs: number;
  text: string;
  /** 揭晓目标来自句间/末尾空白(最近结束的一段),且距今已久。 */
  sparse: boolean;
}

export type RevealTarget =
  | { kind: 'cue'; index: number; sparse: false }
  | { kind: 'last-ended'; index: number; sparse: boolean }
  | { kind: 'before-first' }
  | { kind: 'unavailable' };

/** 「上一条字幕」标签的稀疏阈值:揭晓点距该段结束超过该值时提示。 */
export const SPARSE_GAP_MS = 10_000;

function isValidCue(cue: BlindCue, durationMs: number): boolean {
  return (
    Number.isFinite(cue.startMs) &&
    Number.isFinite(cue.endMs) &&
    cue.startMs >= 0 &&
    cue.startMs < cue.endMs &&
    (durationMs <= 0 || cue.endMs <= durationMs)
  );
}

/**
 * §4.2 定位:以点击时的最新有效播放位置确定目标字幕段。
 * - 覆盖位置的有效 cue,区间 [start, end);重叠取开始最晚,同起点按原顺序首个。
 * - 句间空白/末尾取最近结束的一段;距其结束超过 SPARSE_GAP_MS 标记 sparse。
 * - 第一段开始前没有候选(before-first);时长或位置不可用为 unavailable。
 * - 只使用时间有限且 0 ≤ start < end ≤ duration 的有效 cue。
 */
export function resolveCueAtPosition(
  cues: readonly BlindCue[],
  positionMs: number,
  durationMs: number,
): RevealTarget {
  if (!Number.isFinite(positionMs) || positionMs < 0 || durationMs <= 0 || !Number.isFinite(durationMs)) {
    return { kind: 'unavailable' };
  }
  // 覆盖判定:同起点取原顺序首个 → 先遇到的先记录,只有更晚起点才替换。
  let coveringIndex = -1;
  let coveringStart = -1;
  let lastEndedIndex = -1;
  let lastEndedEnd = -1;
  let earliestStart = Number.POSITIVE_INFINITY;
  for (let i = 0; i < cues.length; i++) {
    const cue = cues[i];
    if (!isValidCue(cue, durationMs)) continue;
    if (cue.startMs < earliestStart) earliestStart = cue.startMs;
    if (cue.startMs <= positionMs && positionMs < cue.endMs) {
      if (cue.startMs > coveringStart) {
        coveringStart = cue.startMs;
        coveringIndex = i;
      }
    }
    if (cue.endMs <= positionMs && cue.endMs > lastEndedEnd) {
      lastEndedEnd = cue.endMs;
      lastEndedIndex = i;
    }
  }
  if (coveringIndex >= 0) return { kind: 'cue', index: coveringIndex, sparse: false };
  if (lastEndedIndex >= 0) {
    return {
      kind: 'last-ended',
      index: lastEndedIndex,
      sparse: positionMs - lastEndedEnd > SPARSE_GAP_MS,
    };
  }
  if (Number.isFinite(earliestStart) && positionMs < earliestStart) {
    return { kind: 'before-first' };
  }
  return { kind: 'unavailable' };
}

export interface RequestRevealArgs {
  audioId: string;
  cues: readonly BlindCue[];
  positionMs: number;
  durationMs: number;
  isPlaying: boolean;
  /** 暂停当前音频;由页面注入播放器调用,失败应 reject。 */
  pause: (requestIsCurrent: () => boolean) => Promise<void>;
  /** Validate the captured sound and subtitle source before and after pausing. */
  isCurrent?: () => boolean;
}

export type RevealOutcome =
  | { ok: true }
  | { ok: false; reason: 'not-blind' | 'pending' | 'no-target' | 'pause-failed' | 'invalidated' };

export interface BlindListeningSnapshot {
  presentation: ListeningPresentation;
  revealed: RevealedCue | null;
  revealPending: boolean;
}

/**
 * 盲听状态机:呈现模式 + 揭晓快照 + 请求代际。
 * 切音频、seek、模式变化或字幕变化由页面调用 invalidate() 使待完成请求失效;
 * 失效检查在暂停等副作用之后、提交快照之前执行,旧请求不会暂停新音频的
 * 状态被错误提交(暂停动作本身作用于当时的播放器实例,由页面保证)。
 */
export interface BlindListeningMachine {
  getSnapshot(): BlindListeningSnapshot;
  /** 便捷判断:当前是否处于盲听呈现(供回调/守卫读取,等价于 getSnapshot().presentation === 'blind')。 */
  isBlind(): boolean;
  subscribe(listener: () => void): () => void;
  enterBlind(): void;
  exitBlind(): void;
  clearReveal(): void;
  invalidate(): void;
  requestReveal(args: RequestRevealArgs): Promise<RevealOutcome>;
}

export function createBlindListeningMachine(): BlindListeningMachine {
  let presentation: ListeningPresentation = 'subtitles';
  let revealed: RevealedCue | null = null;
  let revealPending = false;
  let generation = 0;
  const listeners = new Set<() => void>();
  // useSyncExternalStore 以 Object.is 比较 getSnapshot 结果:快照必须缓存,
  // 且仅在状态实际变化时重建,否则每次渲染都返回新对象会陷入无限更新。
  let cached: BlindListeningSnapshot = { presentation, revealed, revealPending };
  const commit = () => {
    if (
      cached.presentation === presentation &&
      cached.revealed === revealed &&
      cached.revealPending === revealPending
    ) {
      return;
    }
    cached = { presentation, revealed, revealPending };
    listeners.forEach((fn) => fn());
  };
  const emit = commit;
  const snapshot = (): BlindListeningSnapshot => cached;
  const invalidate = () => {
    generation++;
    revealed = null;
    revealPending = false;
    emit();
  };
  return {
    getSnapshot: snapshot,
    isBlind: () => presentation === 'blind',
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    enterBlind() {
      if (presentation === 'blind') return;
      presentation = 'blind';
      invalidate();
    },
    exitBlind() {
      if (presentation === 'subtitles') return;
      presentation = 'subtitles';
      invalidate();
    },
    clearReveal() {
      generation++;
      revealed = null;
      revealPending = false;
      emit();
    },
    invalidate,
    async requestReveal(args) {
      if (presentation !== 'blind') return { ok: false, reason: 'not-blind' };
      if (revealPending) return { ok: false, reason: 'pending' };
      if (args.isCurrent && !args.isCurrent()) return { ok: false, reason: 'invalidated' };
      const target = resolveCueAtPosition(args.cues, args.positionMs, args.durationMs);
      if (target.kind === 'unavailable' || target.kind === 'before-first') {
        return { ok: false, reason: 'no-target' };
      }
      const cue = args.cues[target.index];
      const pendingSnapshot: RevealedCue = {
        audioId: args.audioId,
        cueId: cue.id,
        startMs: cue.startMs,
        endMs: cue.endMs,
        text: cue.text,
        sparse: target.sparse,
      };
      const requestGeneration = generation;
      const requestIsCurrent = () => generation === requestGeneration && (args.isCurrent?.() ?? true);
      revealPending = true;
      emit();
      try {
        if (args.isPlaying) await args.pause(requestIsCurrent);
      } catch {
        if (generation !== requestGeneration) return { ok: false, reason: 'invalidated' };
        revealPending = false;
        emit();
        return { ok: false, reason: 'pause-failed' };
      }
      // 暂停等待期间发生切换/seek/模式变化:丢弃,不提交旧快照。
      if (!requestIsCurrent()) {
        if (generation === requestGeneration) { revealPending = false; emit(); }
        return { ok: false, reason: 'invalidated' };
      }
      revealed = pendingSnapshot;
      revealPending = false;
      emit();
      return { ok: true };
    },
  };
}

/** React 绑定:快照经 useSyncExternalStore 订阅状态机。 */
export function useBlindListening(machine: BlindListeningMachine): BlindListeningSnapshot {
  return useSyncExternalStore(machine.subscribe, machine.getSnapshot, machine.getSnapshot);
}
