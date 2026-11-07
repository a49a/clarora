import { nextRepeatAudio } from '../services/listeningPlayback';
import { captureBlindPlayback, revealListeningCue } from '../services/blindPlayback';
import { useListeningPlayback } from '../ui/useListeningPlayback';
import { makeListeningStyles } from '../ui/listeningStyles';
import { useListeningLibrary, type ListeningSelection } from '../ui/useListeningLibrary';
import { useListeningSubtitles } from '../ui/useListeningSubtitles';
import { StudyOptions } from '../ui/StudyOptions';
import { PopoverRoot } from '../ui/PopoverRoot';
import { resolveStudySettingsMaskTap, type StudySettingsTrigger } from '../ui/studySettingsMask';
import { StudySubtitleToolbar } from '../ui/StudySubtitleToolbar';
import { BlindListeningPanel, type RevealDisabledReason } from '../ui/BlindListeningPanel';
import {
  createBlindListeningMachine,
  resolveCueAtPosition,
  useBlindListening,
  type BlindCue,
  type BlindListeningMachine,
} from '../ui/blindListening';
import { NativeSelectableSubtitleView } from '../ui/NativeSelectableSubtitle';
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import {
  ActivityIndicator,
  AppState,
  BackHandler,
  NativeModules,
  PanResponder,
  PermissionsAndroid,
  Platform,
  Pressable,
  processColor,
  ScrollView,
  StatusBar,
  StyleSheet,
  Text,
  View,
} from 'react-native';
import { audioRecorder, FileSystem } from '../services/platform';

import { ListeningPractice, ListeningAudio, getSetting, setSetting, saveClipCard } from '../data/database';
import { classifySubtitleLanguage, parseSubtitleCues, SubtitleCue } from '../data/subtitles';
import { askAboutPassage, speakingAttempt, waitForSpeaking, type SpeakingScore } from '../services/ai';
import { useAppTheme } from '../ui/ThemeContext';
import { AIChatInlineButton, useAIChat, useAIChatEntry } from '../ui/AIChatProvider';
type CueToken = { text: string; isWord: boolean; offset: number };
type TranscriptWord = { token: CueToken; start: number; length: number };
type TranscriptCue = {
  cueIndex: number;
  start: number;
  end: number;
  primaryEnd: number;
  words: TranscriptWord[];
};

type ListeningScreenProps = ListeningSelection & {
  onExitStudy?: () => void;
  studySidebarStatus?: ReactNode;
};

const AI_QUESTION_TIMEOUT_MS = 60_000;

// RCTViewManager's custom NSColor props receive an already processed native
// color value, unlike ordinary React Native style colors which are processed
// by the style system automatically.
function toNativeColor(color: string): number | null {
  const processed = processColor(color);
  return typeof processed === "number" ? processed : null;
}

// AI 标重点：让模型只返回 JSON，客户端把短语匹配回转写文本的字符区间。
const KEY_MARK_COLOR = "#b8860b";
const KEY_POINTS_PROMPT =
  '请从这段文本中挑出学习重点（核心词汇、关键短语、重要表达），只返回严格的 JSON，格式：' +
  '{"points":[{"phrase":"原文中连续出现的原词或原短语","note":"一句话中文说明（含义或为什么重要）"}]}。' +
  "要求：phrase 必须逐字符出现在原文中（大小写可以不同），每条不超过 6 个单词；最多 10 条，按重要性排序；不要返回 JSON 以外的任何文字。";

type KeyPoint = { phrase: string; note: string };
type KeyMark = { start: number; length: number };

function parseKeyPoints(answer: string): KeyPoint[] {
  const start = answer.indexOf("{");
  const end = answer.lastIndexOf("}");
  if (start === -1 || end <= start) return [];
  try {
    const parsed = JSON.parse(answer.slice(start, end + 1));
    const points = Array.isArray(parsed?.points) ? parsed.points : [];
    const result: KeyPoint[] = [];
    for (const item of points) {
      const phrase = typeof item?.phrase === "string" ? item.phrase.trim() : "";
      const note = typeof item?.note === "string" ? item.note.trim() : "";
      if (phrase) result.push({ phrase, note });
      if (result.length >= 16) break;
    }
    return result;
  } catch {
    return [];
  }
}

/** Locate each phrase in the transcript (case-insensitive), dropping overlaps. */
function matchKeyPoints(text: string, points: KeyPoint[]): { marks: KeyMark[]; matched: KeyPoint[] } {
  const lowerText = text.toLowerCase();
  const candidates = points
    .map((point) => {
      const start = lowerText.indexOf(point.phrase.toLowerCase());
      return { point, start, length: point.phrase.length };
    })
    .filter((candidate) => candidate.start >= 0)
    .sort((a, b) => a.start - b.start);
  const marks: KeyMark[] = [];
  const matched: KeyPoint[] = [];
  let cursor = 0;
  for (const candidate of candidates) {
    if (candidate.start < cursor) continue;
    marks.push({ start: candidate.start, length: candidate.length });
    matched.push(candidate.point);
    cursor = candidate.start + candidate.length;
  }
  return { marks, matched };
}

function formatTime(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  return `${m}:${s.toString().padStart(2, "0")}`;
}

function ABTimelineBar(props: {
  durationMs: number;
  startMs: number;
  endMs: number;
  onChange: (which: "A" | "B", ms: number) => void;
}) {
  const { theme } = useAppTheme();
  const [width, setWidth] = useState(0);
  const dragRef = useRef<"A" | "B" | null>(null);
  const trackRef = useRef<any>(null);
  // Absolute window X of the track. Touch events on the handles report
  // locationX relative to the handle view itself, which made the B handle
  // impossible to grab — so always compute from pageX instead.
  const trackXRef = useRef(0);
  const stateRef = useRef({ durationMs: 0, startMs: 0, endMs: 0, width: 0 });
  stateRef.current = {
    durationMs: props.durationMs,
    startMs: props.startMs,
    endMs: props.endMs,
    width,
  };
  const onChangeRef = useRef(props.onChange);
  onChangeRef.current = props.onChange;

  const measureTrack = useCallback(() => {
    trackRef.current?.measureInWindow?.((x: number) => {
      if (typeof x === "number") trackXRef.current = x;
    });
  }, []);

  const pan = useRef(
    PanResponder.create({
      onStartShouldSetPanResponder: () => true,
      onMoveShouldSetPanResponder: () => true,
      onPanResponderGrant: (e: any) => {
        const s = stateRef.current;
        measureTrack();
        if (s.width <= 0 || s.durationMs <= 0) return;
        const x = Math.max(0, Math.min(s.width, e.nativeEvent.pageX - trackXRef.current));
        const aX = (s.startMs / s.durationMs) * s.width;
        const bX = (s.endMs / s.durationMs) * s.width;
        dragRef.current = Math.abs(x - aX) <= Math.abs(x - bX) ? "A" : "B";
        onChangeRef.current(dragRef.current, Math.max(0, Math.min(s.durationMs, (x / s.width) * s.durationMs)));
      },
      onPanResponderMove: (e: any) => {
        const s = stateRef.current;
        if (!dragRef.current || s.width <= 0 || s.durationMs <= 0) return;
        const x = Math.max(0, Math.min(s.width, e.nativeEvent.pageX - trackXRef.current));
        onChangeRef.current(dragRef.current, Math.max(0, Math.min(s.durationMs, (x / s.width) * s.durationMs)));
      },
    })
  ).current;

  const styles = makeListeningStyles(theme);
  const aPct = props.durationMs > 0 ? Math.min(100, (props.startMs / props.durationMs) * 100) : 0;
  const bPct = props.durationMs > 0 ? Math.min(100, (props.endMs / props.durationMs) * 100) : 100;

  return (
    <View
      ref={trackRef}
      {...pan.panHandlers}
      onLayout={(e) => {
        setWidth(e.nativeEvent.layout.width);
        measureTrack();
      }}
      style={styles.abTrack}
    >
      <View
        style={[
          styles.abRange,
          {
            left: `${aPct}%`,
            width: `${Math.max(0, bPct - aPct)}%`,
          },
        ]}
      />
      <View style={[styles.abHandle, { left: `${aPct}%` }]}>
        <View style={[styles.abLine, { backgroundColor: theme.accent }]} />
        <Text style={[styles.abArrow, { color: theme.accent }]}>▶</Text>
      </View>
      <View style={[styles.abHandleB, { left: `${bPct}%` }]}>
        <Text style={[styles.abArrow, { color: theme.accent }]}>◀</Text>
        <View style={[styles.abLine, { backgroundColor: theme.accent }]} />
      </View>
    </View>
  );
}

// Split a cue into word/whitespace tokens, keeping character offsets so each
// word can be mapped to an estimated time range (linear interpolation).
function tokenizeCue(text: string): CueToken[] {
  const tokens: CueToken[] = [];
  let cursor = 0;
  const wordPattern = /\S+/g;
  let match: RegExpExecArray | null;
  while ((match = wordPattern.exec(text)) !== null) {
    if (match.index > cursor) {
      tokens.push({ text: text.slice(cursor, match.index), isWord: false, offset: cursor });
    }
    tokens.push({ text: match[0], isWord: true, offset: match.index });
    cursor = match.index + match[0].length;
  }
  if (cursor < text.length) {
    tokens.push({ text: text.slice(cursor), isWord: false, offset: cursor });
  }
  return tokens;
}

export default function ListeningScreen({ onExitStudy, studySidebarStatus, ...selection }: ListeningScreenProps = {}) {
  const { theme, scheme } = useAppTheme();
  const [error, setError] = useState<string | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  const { practices, selectedPractice, selectedAudio, selectedPracticeId, selectedAudioId,
    setSelectedPracticeId, setSelectedAudioId, refreshPractices } = useListeningLibrary(selection, setError);

  // Segment loop (选段循环) state — range in milliseconds, draggable on the
  // AB timeline and adjustable word-by-word in the subtitle list.
  const [loopMode, setLoopMode] = useState(false);
  const [loopStage, setLoopStage] = useState<"A" | "B" | "done" | null>(null);
  const [loopStartMs, setLoopStartMs] = useState<number | null>(null);
  const [loopEndMs, setLoopEndMs] = useState<number | null>(null);
  const progressBarWidthRef = useRef(0);

  // ── 盲听(先听后核对)───────────────────────────────────────────────────
  // 呈现模式与揭晓快照集中在状态机;播放位置/倍速/播放状态以播放器为权威。
  const blindMachineRef = useRef<BlindListeningMachine | null>(null);
  if (!blindMachineRef.current) blindMachineRef.current = createBlindListeningMachine();
  const blindMachine = blindMachineRef.current;
  const blind = useBlindListening(blindMachine);
  const [blindEnded, setBlindEnded] = useState(false);

  const {
    playback, enqueueAudioOperation, mountedRef, loadedAudioIdRef, loadingAudioIdRef,
    readyAudioId, clearReadyAudio, isPlaying,
    positionMs, positionMsRef, durationMs, playbackRate, pendingRate, isPlayingRef,
    repeatMode, repeatModeRef, repeatAdvanceRef, cycleRepeatMode, rateApplying, changeRate: applyRate,
    resetPlayback, prepareAudio, flushListeningTime,
  } = useListeningPlayback(blindMachine, setBlindEnded, setError);

  // Subtitle font size — scales the English words and Chinese translation
  // lines together; persisted in app_settings.
  const [subtitleSize, setSubtitleSize] = useState(14);
  useEffect(() => {
    void (async () => {
      try {
        const saved = await getSetting("subtitle_font_size");
        const n = saved ? parseInt(saved, 10) : NaN;
        if (Number.isFinite(n) && n >= 11 && n <= 28) setSubtitleSize(n);
      } catch {}
    })();
  }, []);
  const changeSubtitleSize = useCallback(
    (delta: number) => {
      const next = Math.min(28, Math.max(11, subtitleSize + delta));
      if (next === subtitleSize) return;
      setSubtitleSize(next);
      void setSetting("subtitle_font_size", String(next)).catch(() => {});
    },
    [subtitleSize]
  );

  // Subtitle state
  const [subtitleState, setSubtitleState] = useState<{
    audioId: string | null; generation: number; revision: number; cues: SubtitleCue[];
  }>({ audioId: null, generation: 0, revision: 0, cues: [] });
  const subtitleStateRef = useRef(subtitleState);
  const subtitleReadRef = useRef(0);
  const subtitleCues = subtitleState.cues;
  const setSubtitleCues = useCallback((cues: SubtitleCue[], audioId: string | null = null, generation = playback.generation) => {
    const next = { audioId, generation, revision: subtitleStateRef.current.revision + 1, cues };
    subtitleStateRef.current = next;
    blindMachine.invalidate();
    setSubtitleState(next);
  }, [blindMachine]);
  // Derived from playback position so the highlight always tracks the audio
  // (avoids stale subtitle closures inside the playback status callback).
  const activeCueIndex = useMemo(() => {
    const t = positionMs / 1000;
    for (let i = 0; i < subtitleCues.length; i++) {
      if (t >= subtitleCues[i].start && t <= subtitleCues[i].end) return i;
    }
    return -1;
  }, [positionMs, subtitleCues]);

  // Index of the word currently being spoken inside the active cue
  // (character-offset interpolation, same model as word click-to-seek).
  const activeWordIndex = useMemo(() => {
    if (activeCueIndex < 0) return -1;
    const cue = subtitleCues[activeCueIndex];
    const t = positionMs / 1000;
    // A cue may contain translated lines after the original sentence. Only
    // the original line corresponds to the audio timing and clickable words.
    const primaryText = cue.text.split("\n")[0] ?? "";
    const words = tokenizeCue(primaryText).filter((token) => token.isWord);
    if (words.length === 0) return -1;
    const total = Math.max(1, primaryText.length);
    let lastStarted = -1;
    for (let i = 0; i < words.length; i++) {
      const word = words[i];
      const wordStart = cue.start + (cue.end - cue.start) * (word.offset / total);
      const wordEnd = cue.start + (cue.end - cue.start) * ((word.offset + word.text.length) / total);
      if (t >= wordStart && t <= wordEnd) return i;
      if (t > wordEnd) lastStarted = i;
    }
    return lastStarted >= 0 ? lastStarted : 0;
  }, [activeCueIndex, positionMs, subtitleCues]);
  // Picker overlay
  const [showPracticePicker, setShowPracticePicker] = useState(false);
  const [showAudioPicker, setShowAudioPicker] = useState(false);

  // AI passage question state. Subtitles are always mouse-selectable on macOS;
  // a right click on the selection chooses either copy or asking AI.
  const { visible: chatOpen, open: openChat, resetContext: resetChatContext, launch: launchChat } = useAIChat();
  // 设置弹层统一互斥:同一时间只允许一个前景层(字幕/倍速)。
  const [activeSettings, setActiveSettings] = useState<'subtitle' | 'speed' | null>(null);
  const subtitleSettingsTriggerRef = useRef<View | null>(null);
  const speedSettingsTriggerRef = useRef<View | null>(null);
  const aiTriggerRef = useRef<View | null>(null);
  const practiceTriggerRef = useRef<View | null>(null);
  const audioTriggerRef = useRef<View | null>(null);
  const exitTriggerRef = useRef<View | null>(null);
  const changeRate = async (rate: number) => {
    if (await applyRate(rate, !!selectedAudioId)) setActiveSettings(null);
  };

  // ── 听力跟读：录下当前句的朗读，AI 服务转写对齐打分 ────────────────────────
  const [shadowing, setShadowing] = useState(false);
  const [shadowElapsed, setShadowElapsed] = useState(0);
  const [shadowBusy, setShadowBusy] = useState<string | null>(null);
  const [shadowResult, setShadowResult] = useState<{
    reference: string;
    transcript: string;
    feedback: string;
    score: SpeakingScore | null;
  } | null>(null);
  const shadowTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const shadowReferenceRef = useRef("");
  const shadowingRef = useRef(false);

  // ── AI 标重点：转写文本发给 AI，重点短语在字幕里以下划线标出 ──────────────
  const [keyPoints, setKeyPoints] = useState<KeyPoint[]>([]);
  const [keyMarks, setKeyMarks] = useState<KeyMark[]>([]);
  const [keyLoading, setKeyLoading] = useState(false);
  const keyAbortRef = useRef<AbortController | null>(null);
  const keyRequestIdRef = useRef(0);

  // Batch management state
  // Keep the selectable surface as one native NSTextView, rather than Fabric
  // Text (which currently cannot form a mouse-drag selection on macOS).
  const subtitleTranscript = useMemo(() => {
    let cursor = 0;
    const ranges: TranscriptCue[] = [];
    const text = subtitleCues.map((cue, cueIndex) => {
      const start = cursor;
      const end = start + cue.text.length;
      const primaryText = cue.text.split("\n")[0] ?? "";
      const words = tokenizeCue(primaryText)
        .filter((token) => token.isWord)
        .map((token) => ({
          token,
          start: start + token.offset,
          length: token.text.length,
        }));
      ranges.push({
        cueIndex,
        start,
        end,
        primaryEnd: start + primaryText.length,
        words,
      });
      cursor = end + (cueIndex < subtitleCues.length - 1 ? 2 : 0);
      return cue.text;
    }).join("\n\n");

    const activeCue = ranges[activeCueIndex];
    const activeWord = activeCue?.words[activeWordIndex];
    // Keep the A/B boundary markers at the nearest word edge, just as the
    // former React subtitle rows did. The native transcript draws the line
    // and arrow as an overlay so adding the markers does not alter selectable
    // subtitle text or shift its character ranges.
    const markerForBoundary = (ms: number | null) => {
      if (!loopMode || ms == null) return null;
      const tSec = ms / 1000;
      const range = ranges.find((candidate) => {
        const cue = subtitleCues[candidate.cueIndex];
        return cue && tSec >= cue.start - 0.001 && tSec <= cue.end + 0.001;
      });
      if (!range || range.words.length === 0) return null;

      const cue = subtitleCues[range.cueIndex];
      const primaryLength = Math.max(1, range.primaryEnd - range.start);
      const ratio = Math.min(1, Math.max(0, (tSec - cue.start) / Math.max(0.001, cue.end - cue.start)));
      const charOffset = Math.round(ratio * primaryLength);
      let best: { start: number; length: number; after: boolean; distance: number } | null = null;
      for (const word of range.words) {
        const wordStart = word.start - range.start;
        const wordEnd = wordStart + word.length;
        const leftDistance = Math.abs(charOffset - wordStart);
        const rightDistance = Math.abs(charOffset - wordEnd);
        if (!best || leftDistance < best.distance) {
          best = { start: word.start, length: word.length, after: false, distance: leftDistance };
        }
        if (!best || rightDistance < best.distance) {
          best = { start: word.start, length: word.length, after: true, distance: rightDistance };
        }
      }
      return best
        ? { index: best.after ? best.start + best.length - 1 : best.start, after: best.after }
        : null;
    };
    const loopStartMarker = markerForBoundary(loopStartMs);
    const loopEndMarker = markerForBoundary(loopEndMs);

    return {
      text,
      ranges,
      activeCueStart: activeCue?.start ?? -1,
      activeCueLength: activeCue ? activeCue.end - activeCue.start : 0,
      activeWordStart: activeWord?.start ?? -1,
      activeWordLength: activeWord?.length ?? 0,
      loopStartMarkerIndex: loopStartMarker?.index ?? -1,
      loopStartMarkerAfter: loopStartMarker?.after ?? false,
      loopEndMarkerIndex: loopEndMarker?.index ?? -1,
      loopEndMarkerAfter: loopEndMarker?.after ?? false,
    };
  }, [activeCueIndex, activeWordIndex, loopEndMs, loopMode, loopStartMs, subtitleCues]);

  const showToast = useCallback(
    (msg: string) => {
      setToast(msg);
      setTimeout(() => setToast(null), 2300);
    },
    []
  );

  const openChatWithSelection = useCallback((text: string) => {
    // 盲听中字幕视图已卸载,不存在选区;防御旧回调迟到注入。
    if (blindMachineRef.current?.isBlind()) return;
    void playback.sound?.pauseAsync().catch(() => {});
    setActiveSettings(null); // AI 面板成为前景前先收起设置
    openChat({ text, source: selectedAudio?.name || "音频学习" });
  }, [openChat, selectedAudio?.name]);

  // ── 盲听派生状态 ──────────────────────────────────────────────────────
  // cue 秒 → 毫秒,只在盲听边界转换(§7:避免把 cue 秒值直接传给 seek)。
  const blindCues = useMemo<BlindCue[]>(
    () => subtitleCues.map((cue) => ({ id: cue.id, startMs: cue.start * 1000, endMs: cue.end * 1000, text: cue.text })),
    [subtitleCues]
  );
  const blindHasValidCues = useMemo(
    () => blindCues.some((cue) => Number.isFinite(cue.startMs) && Number.isFinite(cue.endMs) && cue.startMs >= 0 && cue.startMs < cue.endMs && (durationMs <= 0 || cue.endMs <= durationMs)),
    [blindCues, durationMs]
  );
  // 揭晓快照仅在当前音频上有效(切音频由 invalidate 清理,这里再按 audioId 防御)。
  const blindRevealed = useMemo(() => {
    const revealed = blind.revealed;
    if (!revealed || !selectedAudio || revealed.audioId !== selectedAudio.id) return null;
    return {
      text: revealed.text,
      timeRange: `${formatTime(revealed.startMs / 1000)} → ${formatTime(revealed.endMs / 1000)}`,
      sparse: revealed.sparse,
    };
  }, [blind.revealed, selectedAudio]);

  useAIChatEntry(
    // 盲听:隐藏态只能自由提问;查看态只引用明确揭晓的那句快照。
    blind.presentation === 'blind'
      ? blindRevealed?.text ?? ''
      : subtitleCues.length ? (subtitleCues[activeCueIndex >= 0 ? activeCueIndex : 0]?.text || subtitleTranscript.text) : "",
    selectedAudio?.name || "音频学习",
    () => { setActiveSettings(null); void playback.sound?.pauseAsync().catch(() => {}); },
    // 盲听入口优先于重新选文;隐藏态新建对话,查看态只使用已揭晓引用。
    { isolateReference: blind.presentation === 'blind' }
  );

  // ── 听力跟读 ────────────────────────────────────────────────────────────────

  const startShadowing = useCallback(async () => {
    if (blindMachineRef.current?.isBlind()) {
      showToast("切换到字幕模式后使用");
      return;
    }
    if (subtitleCues.length === 0) {
      showToast("该音频没有字幕，无法跟读");
      return;
    }
    // 跟读原文取当前句（无激活句则取第一句）的英文行，中文译文行不参与评分。
    const cue = subtitleCues[activeCueIndex >= 0 ? activeCueIndex : 0];
    const reference = (cue.text.split("\n")[0] ?? "").trim();
    if (!reference) {
      showToast("当前句没有可跟读的文本");
      return;
    }
    setError(null);
    // 录音时暂停参考音频，避免播放声混进麦克风。
    try {
      await playback.sound?.pauseAsync();
    } catch {}
    if (Platform.OS === "android") {
      const granted = await PermissionsAndroid.request(
        PermissionsAndroid.PERMISSIONS.RECORD_AUDIO,
        {
          title: "麦克风权限",
          message: "跟读打分需要使用麦克风录制你的朗读",
          buttonPositive: "确定",
        }
      ).catch(() => "denied");
      if (granted !== PermissionsAndroid.RESULTS.GRANTED) {
        setError("麦克风权限被拒绝，请在系统设置中允许");
        return;
      }
    }
    try {
      await audioRecorder.start();
      shadowReferenceRef.current = reference;
      shadowingRef.current = true;
      setShadowing(true);
      setShadowElapsed(0);
      setShadowResult(null);
      shadowTimerRef.current = setInterval(() => setShadowElapsed((s) => s + 1), 1000);
    } catch (e: any) {
      setError(`录音失败：${e?.message ?? e}`);
    }
  }, [subtitleCues, activeCueIndex, showToast]);

  const stopShadowing = useCallback(async () => {
    if (!shadowingRef.current) return;
    shadowingRef.current = false;
    if (shadowTimerRef.current) {
      clearInterval(shadowTimerRef.current);
      shadowTimerRef.current = null;
    }
    setShadowing(false);
    const reference = shadowReferenceRef.current;
    setShadowBusy("正在上传录音…");
    try {
      const file = await audioRecorder.stop();
      const { attemptId } = await speakingAttempt(file.uri, file.name, reference, "shadow");
      const detail = await waitForSpeaking(attemptId, (message) => setShadowBusy(message));
      setShadowResult({
        reference: detail.reference ?? reference,
        transcript: detail.transcript ?? "",
        feedback: detail.feedback ?? "",
        score: detail.score,
      });
      setShadowBusy(null);
      showToast("跟读评分完成");
    } catch (e: any) {
      setShadowBusy(null);
      setError(`跟读评分失败：${e?.message ?? e}`);
    }
  }, [showToast]);

  const clearKeyPoints = useCallback(() => {
    keyRequestIdRef.current += 1;
    keyAbortRef.current?.abort();
    keyAbortRef.current = null;
    setKeyPoints([]);
    setKeyMarks([]);
    setKeyLoading(false);
  }, []);

  const cancelKeyPoints = useCallback(() => {
    keyRequestIdRef.current += 1;
    keyAbortRef.current?.abort();
    keyAbortRef.current = null;
    setKeyLoading(false);
  }, []);

  const markKeyPoints = useCallback(async () => {
    if (blindMachineRef.current?.isBlind()) {
      showToast("切换到字幕模式后使用");
      return;
    }
    const passage = subtitleTranscript.text.trim();
    if (!passage) {
      showToast("当前音频没有字幕文本");
      return;
    }
    if (passage.length > 12_000) {
      showToast("文本过长，请先截取片段再标重点");
      return;
    }
    if (keyLoading) return;
    setError(null);
    const controller = new AbortController();
    const requestId = keyRequestIdRef.current + 1;
    keyRequestIdRef.current = requestId;
    keyAbortRef.current = controller;
    let didTimeOut = false;
    const timeout = setTimeout(() => {
      didTimeOut = true;
      controller.abort();
    }, AI_QUESTION_TIMEOUT_MS);
    setKeyLoading(true);
    try {
      const answer = await askAboutPassage(passage, KEY_POINTS_PROMPT, undefined, controller.signal);
      if (keyRequestIdRef.current !== requestId) return;
      const points = parseKeyPoints(answer);
      if (points.length === 0) {
        setError("AI 没有返回有效的重点列表，请重试");
        return;
      }
      const { marks, matched } = matchKeyPoints(subtitleTranscript.text, points);
      if (marks.length === 0) {
        setError("AI 返回的重点没有匹配到原文，请重试");
        return;
      }
      setKeyMarks(marks);
      setKeyPoints(matched);
      showToast(`已标记 ${marks.length} 处重点`);
    } catch (e: any) {
      if (keyRequestIdRef.current !== requestId) return;
      setError(
        didTimeOut
          ? "标重点超时，请检查 AI 服务后重试"
          : controller.signal.aborted
            ? "已取消本次标重点"
            : `标重点失败：${e?.message ?? e}`
      );
    } finally {
      clearTimeout(timeout);
      if (keyRequestIdRef.current === requestId) {
        keyAbortRef.current = null;
        setKeyLoading(false);
      }
    }
  }, [subtitleTranscript, keyLoading, showToast]);

  // ── 盲听:模式切换与面板动作(§4.1/§4.3) ────────────────────────────────

  const enterBlindMode = useCallback(async () => {
    if (blindMachine.isBlind()) return;
    // 跟读录音尚未完成:先走现有停止/清理;清理未完成时不进入盲听。
    if (shadowingRef.current) {
      await stopShadowing();
      if (shadowingRef.current) return;
    }
    setActiveSettings(null);
    resetChatContext();
    // 仅设 A、尚未设 B 的截取:取消未完成选段;完整 A/B 范围保留。
    if (loopStartMs != null && loopEndMs == null) {
      setLoopStartMs(null);
      if (loopStage === 'B') setLoopStage('A');
      showToast('已取消未完成的截取');
    }
    setBlindEnded(false);
    blindMachine.enterBlind();
  }, [blindMachine, stopShadowing, resetChatContext, loopStartMs, loopEndMs, loopStage, showToast]);

  const exitBlindMode = useCallback(() => {
    setBlindEnded(false);
    blindMachine.exitBlind();
  }, [blindMachine]);

  const getBlindPlaybackContext = useCallback(() =>
    captureBlindPlayback(selectedAudio?.id, subtitleCues, () => ({
      sound: playback.sound,
      source: subtitleStateRef.current,
      generation: playback.generation,
      active: mountedRef.current && blindMachine.isBlind()
        && loadingAudioIdRef.current == null && loadedAudioIdRef.current === selectedAudio?.id
        && !subtitleTaskRef.current,
    })), [selectedAudio?.id, subtitleCues, blindMachine, playback, mountedRef, loadingAudioIdRef, loadedAudioIdRef]);

  const handleBlindReveal = useCallback(async () => {
    const context = getBlindPlaybackContext();
    if (!context || !selectedAudio) return;
    const outcome = await revealListeningCue(blindMachine, context, selectedAudio.id, {
      positionMs: positionMsRef.current,
      durationMs,
      isPlaying: isPlayingRef.current,
    }, enqueueAudioOperation, () => isPlayingRef.current);
    if (!outcome.ok && outcome.reason === 'pause-failed') showToast('暂停失败，请重试');
  }, [selectedAudio, blindMachine, durationMs, showToast, getBlindPlaybackContext, enqueueAudioOperation, isPlayingRef, positionMsRef]);

  const playBlindAudio = useCallback(async (restart: boolean) => {
    const sound = playback.sound;
    const generation = playback.generation;
    const audioId = selectedAudio?.id;
    const current = () => mountedRef.current && blindMachine.isBlind()
      && sound != null && playback.sound === sound && generation === playback.generation
      && loadedAudioIdRef.current === audioId && loadingAudioIdRef.current == null;
    if (!current()) return;
    blindMachine.clearReveal();
    setBlindEnded(false);
    try {
      await enqueueAudioOperation(async () => {
        if (!current()) return;
        if (restart) await sound!.setPositionAsync(0);
        if (current()) await sound!.playAsync();
      });
    } catch { showToast('播放失败，请重试'); }
  }, [selectedAudio?.id, blindMachine, enqueueAudioOperation, showToast]);
  const handleBlindResume = useCallback(() => playBlindAudio(false), [playBlindAudio]);
  const handleBlindRestart = useCallback(() => playBlindAudio(true), [playBlindAudio]);

  // Every route that hides an answer also detaches its pending AI reference.
  useEffect(() => {
    let previous = blindMachine.getSnapshot();
    return blindMachine.subscribe(() => {
      const next = blindMachine.getSnapshot();
      if (previous.revealed && !next.revealed) resetChatContext();
      previous = next;
    });
  }, [blindMachine, resetChatContext]);

  // 播放恢复(任何入口,含外部事件):先隐藏已揭晓内容、清掉结束态。
  useEffect(() => {
    if (!isPlaying) return;
    if (blindMachine.getSnapshot().revealed) blindMachine.clearReveal();
    setBlindEnded(false);
  }, [isPlaying, blindMachine]);

  // 切后台回前台:收起已揭晓内容(§4.1);不新增自动播放。
  useEffect(() => {
    const subscription = AppState.addEventListener('change', (state) => {
      if (state === 'active' && blindMachine.isBlind()) {
        blindMachine.clearReveal();
      }
    });
    return () => subscription.remove();
  }, [blindMachine]);

  // ── Audio playback ──────────────────────────────────────────────────────────

  const unloadSound = useCallback(async () => {
    const pending = playback.unload();
    resetPlayback();
    blindMachine.invalidate();
    clearReadyAudio();
    setSubtitleCues([]);
    await pending;
  }, [playback, resetPlayback, blindMachine, clearReadyAudio, setSubtitleCues]);

  const handleExitStudy = useCallback(async () => {
    // 退出学习:回到默认字幕模式,清空揭晓状态(§4.1,重进不保留盲听)。
    blindMachine.exitBlind();
    setBlindEnded(false);
    await flushListeningTime();
    try {
      await unloadSound();
    } finally {
      onExitStudy?.();
    }
  }, [blindMachine, onExitStudy, unloadSound, flushListeningTime]);

  // Keep Android swipe-to-exit inside the transcript so horizontal player
  // tools and A/B handles can own their gestures. The exit button remains visible.
  const exitSwipe = useMemo(
    () =>
      PanResponder.create({
        onMoveShouldSetPanResponder: (_, gesture) =>
          Math.abs(gesture.dx) > 20 && Math.abs(gesture.dx) > Math.abs(gesture.dy),
        onPanResponderRelease: (_, gesture) => {
          if (gesture.dx < -70 && Math.abs(gesture.dx) > Math.abs(gesture.dy)) {
            void handleExitStudy();
          }
        },
      }),
    [handleExitStudy]
  );

  useEffect(() => {
    if (Platform.OS !== "android") return;
    const subscription = BackHandler.addEventListener("hardwareBackPress", () => {
      if (activeSettings) { setActiveSettings(null); return true; }
      void handleExitStudy();
      return true;
    });
    return () => subscription.remove();
  }, [activeSettings, handleExitStudy]);

  const loadSubtitle = useCallback(async (audio: ListeningAudio, generation = playback.generation) => {
    if (generation !== playback.generation) return;
    const read = ++subtitleReadRef.current;
    setSubtitleCues([]);
    const current = () => mountedRef.current && generation === playback.generation && read === subtitleReadRef.current;
    try {
      const cues = audio.subtitle_uri
        ? parseSubtitleCues(await FileSystem.readAsStringAsync(audio.subtitle_uri)) : [];
      if (current()) setSubtitleCues(cues, audio.id, generation);
    } catch (e: any) {
      if (current()) {
        setSubtitleCues([]);
        setError(`加载字幕失败：${e?.message ?? e}`);
      }
    }
  }, [setSubtitleCues]);

  const onSubtitleSaved = useCallback(async (audio: ListeningAudio) => {
    await loadSubtitle(audio);
    clearKeyPoints();
  }, [loadSubtitle, clearKeyPoints]);
  const { aiStatus, subtitleTaskRef, handleUploadSubtitle, handleAiSubtitle, runTranslate } =
    useListeningSubtitles({ selectedAudio, refreshPractices, showToast, setError, onSaved: onSubtitleSaved });

  const prepareAudioForStudy = useCallback((audio: ListeningAudio, autoPlay = false) =>
    prepareAudio(audio, autoPlay, loadSubtitle, () => setSubtitleCues([])),
    [prepareAudio, loadSubtitle, setSubtitleCues]);

  const selectAudioItem = useCallback(
    async (practice: ListeningPractice, audio: ListeningAudio, autoPlay = false) => {
      if (subtitleTaskRef.current) return;
      // 切音频:立即同步清理盲听揭晓与结束态,不等字幕 effect——旧音频的
      // 字幕不得被保存为新音频的答案(§4.1)。
      blindMachine.invalidate();
      setBlindEnded(false);
      setSelectedPracticeId(practice.id);
      setSelectedAudioId(audio.id);
      setLoopStartMs(null);
      setLoopEndMs(null);
      setLoopStage(null);
      // 换音频后旧的重点标记不再对应新转写，直接清掉。
      clearKeyPoints();
      await prepareAudioForStudy(audio, autoPlay);
    },
    [prepareAudioForStudy, clearKeyPoints, blindMachine]
  );

  // Android 端的转写是普通 Text，把重点区间切成嵌套 Text 段落渲染下划线。
  const androidTranscriptSegments = useMemo(() => {
    if (keyMarks.length === 0) return null;
    const text = subtitleTranscript.text;
    const segments: Array<{ text: string; marked: boolean }> = [];
    let cursor = 0;
    for (const mark of keyMarks) {
      const start = Math.max(cursor, Math.min(text.length, mark.start));
      const end = Math.min(text.length, start + mark.length);
      if (end <= start) continue;
      if (start > cursor) segments.push({ text: text.slice(cursor, start), marked: false });
      segments.push({ text: text.slice(start, end), marked: true });
      cursor = end;
    }
    if (cursor < text.length) segments.push({ text: text.slice(cursor), marked: false });
    return segments;
  }, [keyMarks, subtitleTranscript]);

  // The study and management pages are separate component instances. When
  // entering study, restore the last selected audio after its database row is
  // available, then load it exactly once for the focused session.
  useEffect(() => {
    if (selectedAudio) void prepareAudioForStudy(selectedAudio);
    else if (loadedAudioIdRef.current || loadingAudioIdRef.current) void unloadSound().catch(() => {});
  }, [prepareAudioForStudy, selectedAudio, unloadSound, loadedAudioIdRef, loadingAudioIdRef]);

  useEffect(() => {
    repeatAdvanceRef.current = async () => {
      if (!selectedPractice) return;
      const next = nextRepeatAudio(selectedPractice.audios, selectedAudioId, repeatModeRef.current);
      if (next) await selectAudioItem(selectedPractice, next, true);
    };
  }, [repeatAdvanceRef, repeatModeRef, selectAudioItem, selectedAudioId, selectedPractice]);

  const togglePlayback = useCallback(async () => {
    if (!playback.sound) return;
    if (isPlaying) {
      await playback.sound.pauseAsync();
    } else {
      // 盲听:任何播放入口恢复播放前先隐藏答案(§4.1)。
      if (blindMachine.getSnapshot().presentation === 'blind') {
        blindMachine.clearReveal();
        setBlindEnded(false);
      }
      await playback.sound.playAsync();
    }
  }, [isPlaying, blindMachine]);

  const seekTo = useCallback(async (ms: number) => {
    const sound = playback.sound;
    const generation = playback.generation;
    if (!sound) return;
    if (blindMachine.isBlind()) {
      blindMachine.invalidate();
      setBlindEnded(false);
    }
    positionMsRef.current = ms;
    await enqueueAudioOperation(async () => {
      if (generation !== playback.generation || playback.sound !== sound || !mountedRef.current) return;
      await sound.setPositionAsync(ms);
    });
  }, [blindMachine, enqueueAudioOperation]);

  const seekBy = useCallback(
    async (deltaMs: number) => {
      const limit = durationMs > 0 ? durationMs : Number.POSITIVE_INFINITY;
      const next = Math.max(0, Math.min(limit, positionMsRef.current + deltaMs));
      await seekTo(next);
    },
    [durationMs, seekTo]
  );

  // ── 盲听:回退与揭晓可用性 ──────────────────────────────────────────────
  const handleBlindBackFive = useCallback(async () => {
    // §4.3:回退保持播放/暂停状态;有完整 A/B 选段时限制在该范围内。
    // seekTo 内部会先隐藏答案并使待完成揭晓失效。
    const floor = loopStartMs != null && loopEndMs != null ? Math.min(loopStartMs, loopEndMs) : 0;
    const target = Math.max(floor, positionMsRef.current - 5000);
    await seekTo(target);
  }, [loopStartMs, loopEndMs, seekTo]);

  // In the focused macOS listening view: Space plays/pauses, and ←/→ move
  // through the passage in five-second steps without changing playback state.
  useEffect(() => {
    if ((Platform.OS !== "macos" && Platform.OS !== "windows") || !selectedAudio || chatOpen || activeSettings) return;
    const keyboard = NativeModules.RNKeyboard as
      | {
          startPlaybackListening?: () => void;
          startListening?: () => void;
          getNextKey?: () => Promise<string | null>;
          stopListening?: () => void;
        }
      | undefined;
    if (
      (!keyboard?.startPlaybackListening && !keyboard?.startListening) ||
      !keyboard?.getNextKey ||
      !keyboard.stopListening
    ) {
      return;
    }

    let cancelled = false;
    if (keyboard.startPlaybackListening) {
      keyboard.startPlaybackListening();
    } else {
      keyboard.startListening?.();
    }
    const pump = async () => {
      while (!cancelled) {
        const key = await keyboard.getNextKey!();
        if (cancelled || key == null) break;
        if (key === " " || key === "Space" || key === "Spacebar") {
          await togglePlayback();
        } else if (key === "ArrowLeft" || key === "Left") {
          await seekBy(-5000);
        } else if (key === "ArrowRight" || key === "Right") {
          await seekBy(5000);
        }
      }
    };
    void pump();
    return () => {
      cancelled = true;
      keyboard.stopListening?.();
    };
  }, [activeSettings, chatOpen, seekBy, selectedAudio, togglePlayback]);

  const seekToCue = useCallback(
    async (cueIndex: number) => {
      if (cueIndex < 0 || cueIndex >= subtitleCues.length) return;
      const cue = subtitleCues[cueIndex];
      await seekTo(cue.start * 1000);
      if (!isPlaying && playback.sound) {
        await playback.sound.playAsync();
      }
    },
    [subtitleCues, seekTo, isPlaying]
  );

  // Seek to an approximate time for a word at the given character offset
  // (subtitles have no word-level timing, so interpolate across the cue).
  const seekToWord = useCallback(
    async (cue: SubtitleCue, charOffset: number) => {
      // Keep translation lines out of the interpolation denominator. Their
      // characters are not spoken in the audio and would shift every click
      // toward the start of the cue.
      const primaryText = cue.text.split("\n")[0] ?? "";
      const total = Math.max(1, primaryText.length);
      const progress = Math.min(1, Math.max(0, charOffset / total));
      const ms = Math.round((cue.start + (cue.end - cue.start) * progress) * 1000);
      await seekTo(ms);
      if (!isPlaying && playback.sound) {
        await playback.sound.playAsync();
      }
    },
    [seekTo, isPlaying]
  );

  // 重点面板点击：跳到包含该重点短语的字幕句并播放。
  const seekToKeyPoint = useCallback(
    (mark: KeyMark | undefined) => {
      if (!mark) return;
      const range = subtitleTranscript.ranges.find(
        (candidate) => mark.start >= candidate.start && mark.start < candidate.end
      );
      if (!range) return;
      void seekToCue(range.cueIndex);
    },
    [subtitleTranscript, seekToCue]
  );

  const loopRange = useMemo(() => {
    if (loopStartMs == null || loopEndMs == null) return null;
    const start = Math.min(loopStartMs, loopEndMs);
    const end = Math.max(loopStartMs, loopEndMs);
    return { startMs: start, endMs: end };
  }, [loopStartMs, loopEndMs]);

  const toggleLoopMode = useCallback(() => {
    setLoopMode((v) => !v);
    setLoopStartMs(null);
    setLoopEndMs(null);
    setLoopStage("A");
  }, []);

  const clearLoopRange = useCallback(() => {
    setLoopStartMs(null);
    setLoopEndMs(null);
    setLoopStage("A");
  }, []);

  // Tapping a cue row: first tap sets A at the cue start, second sets B at
  // the cue end, after that it simply jumps to the tapped cue.
  const handleCuePressForLoop = useCallback(
    (index: number) => {
      const cue = subtitleCues[index];
      if (!cue) return;
      if (loopStage === "A") {
        setLoopStartMs(Math.round(cue.start * 1000));
        setLoopStage("B");
        seekTo(cue.start * 1000);
        if (!isPlaying && playback.sound) {
          playback.sound.playAsync();
        }
        return;
      }
      if (loopStage === "B") {
        setLoopEndMs(Math.round(cue.end * 1000));
        setLoopStage("done");
        return;
      }
      seekToCue(index);
    },
    [loopStage, subtitleCues, seekToCue, seekTo, isPlaying]
  );

  // Tapping a word adjusts the loop boundary to that word's left/right edge;
  // tapping the boundary word again flips the edge (include/exclude it).
  const handleWordPressForLoop = useCallback(
    (index: number, token: CueToken) => {
      const cue = subtitleCues[index];
      if (!cue) return;
      const primary = cue.text.split("\n")[0];
      const total = Math.max(1, primary.length);
      const left = Math.round((cue.start + (cue.end - cue.start) * (token.offset / total)) * 1000);
      const right = Math.round(
        (cue.start + (cue.end - cue.start) * ((token.offset + token.text.length) / total)) * 1000
      );

      if (loopStage === "A") {
        setLoopStartMs(left);
        setLoopStage("B");
        seekTo(left);
        if (!isPlaying && playback.sound) {
          playback.sound.playAsync();
        }
        return;
      }
      if (loopStage === "B") {
        setLoopEndMs(right);
        setLoopStage("done");
        return;
      }
      // Range complete: move whichever boundary (A or B) is nearest to the
      // clicked word; clicking the boundary word on its current edge flips
      // to the word's other edge (include/exclude it).
      if (loopStartMs == null || loopEndMs == null) return;
      const distA = Math.min(
        Math.abs(loopStartMs - left),
        Math.abs(loopStartMs - right)
      );
      const distB = Math.min(
        Math.abs(loopEndMs - left),
        Math.abs(loopEndMs - right)
      );
      if (distA <= distB) {
        const nearestIsLeft =
          Math.abs(loopStartMs - left) <= Math.abs(loopStartMs - right);
        const value = nearestIsLeft
          ? loopStartMs === left
            ? right
            : left
          : loopStartMs === right
          ? left
          : right;
        setLoopStartMs(Math.min(value, loopEndMs));
      } else {
        const nearestIsLeft =
          Math.abs(loopEndMs - left) <= Math.abs(loopEndMs - right);
        const value = nearestIsLeft
          ? loopEndMs === left
            ? right
            : left
          : loopEndMs === right
          ? left
          : right;
        setLoopEndMs(Math.max(value, loopStartMs));
      }
    },
    [loopStage, loopStartMs, loopEndMs, subtitleCues, seekTo, isPlaying]
  );

  // The native selectable transcript reports a simple click as a character
  // position. Keep the established click-to-seek behavior while letting a
  // mouse drag remain an AppKit text selection.
  const handleSubtitleCharacterTap = useCallback(
    (characterIndex: number) => {
      if (!Number.isFinite(characterIndex) || characterIndex < 0) return;
      const range = subtitleTranscript.ranges.find(
        (candidate) => characterIndex >= candidate.start && characterIndex < candidate.end
      );
      // The two line breaks between cues are not spoken, so clicking them does
      // not move audio playback.
      if (!range) return;

      const cue = subtitleCues[range.cueIndex];
      if (!cue) return;
      const word = range.words.find(
        (candidate) =>
          characterIndex >= candidate.start &&
          characterIndex < candidate.start + candidate.length
      );

      if (loopMode) {
        if (word) {
          handleWordPressForLoop(range.cueIndex, word.token);
        } else {
          handleCuePressForLoop(range.cueIndex);
        }
      } else if (characterIndex < range.primaryEnd) {
        void seekToWord(cue, characterIndex - range.start);
      } else {
        void seekToCue(range.cueIndex);
      }
    },
    [
      handleCuePressForLoop,
      handleWordPressForLoop,
      loopMode,
      seekToCue,
      seekToWord,
      subtitleCues,
      subtitleTranscript.ranges,
    ]
  );

  // Wrap playback back to the range start once it passes the range end.
  useEffect(() => {
    if (!loopMode || !loopRange) return;
    if (positionMs >= loopRange.endMs) {
      seekTo(loopRange.startMs);
    }
  }, [positionMs, loopMode, loopRange, seekTo]);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      // 正在跟读录音时直接退出：丢弃未提交的录音，避免泄漏麦克风。
      if (shadowTimerRef.current) clearInterval(shadowTimerRef.current);
      if (shadowingRef.current) {
        shadowingRef.current = false;
        audioRecorder.stop().catch(() => {});
      }
    };
  }, []);

  // Save the current loop segment as a review flashcard.
  const saveLoopToFlashcards = useCallback(async () => {
    if (!loopRange || !selectedAudio) return;
    const cues = subtitleCues.filter(
      (cue) => cue.end * 1000 > loopRange.startMs && cue.start * 1000 < loopRange.endMs
    );
    const enParts: string[] = [];
    const zhParts: string[] = [];
    for (const cue of cues) {
      const lines = cue.text.split("\n");
      enParts.push(lines[0] ?? "");
      if (lines.length > 1) zhParts.push(lines.slice(1).join(" "));
    }
    try {
      await saveClipCard({
        enText: enParts.join(" ").trim(),
        zhText: zhParts.join(" ").trim(),
        audioUri: selectedAudio.audio_uri,
        startMs: Math.round(loopRange.startMs),
        endMs: Math.round(loopRange.endMs),
      });
      showToast("已收藏到闪卡");
    } catch (e: any) {
      setError(`收藏失败：${e?.message ?? e}`);
    }
  }, [loopRange, subtitleCues, selectedAudio, showToast]);

  // ── Render ──────────────────────────────────────────────────────────────────────

  const [switchingAudio, setSwitchingAudio] = useState(false);

  // 盲听「看刚才一句」可用性(§4.2):切换中/请求中/无有效字幕/首段前/时长未知。
  const blindRevealDisabledReason = useMemo<RevealDisabledReason | null>(() => {
    if (blind.presentation !== 'blind' || !selectedAudio) return 'switching';
    if (switchingAudio || aiStatus || readyAudioId !== selectedAudio.id || !getBlindPlaybackContext()) return 'switching';
    if (blind.revealPending) return 'pending';
    if (!blindHasValidCues) return 'no-subtitles';
    if (durationMs <= 0) return 'unavailable';
    const target = resolveCueAtPosition(blindCues, positionMs, durationMs);
    if (target.kind === 'unavailable') return 'unavailable';
    if (target.kind === 'before-first') return 'before-first';
    return null;
  }, [blind.presentation, blind.revealPending, selectedAudio, switchingAudio, aiStatus, readyAudioId, getBlindPlaybackContext, blindHasValidCues, blindCues, durationMs, positionMs]);

  const audioIndex = selectedPractice?.audios.findIndex(audio => audio.id === selectedAudioId) ?? -1;
  const changeStudyAudio = async (offset: number) => {
    const next = selectedPractice?.audios[audioIndex + offset];
    if (!selectedPractice || !next || switchingAudio || subtitleTaskRef.current) return;
    setSwitchingAudio(true);
    setShowAudioPicker(false);
    setShowPracticePicker(false);
    setActiveSettings(null);
    try { await selectAudioItem(selectedPractice, next, isPlaying); }
    catch (error: any) { setError(`切换音频失败：${error?.message ?? error}`); }
    finally { setSwitchingAudio(false); }
  };
  const [studyWidth, setStudyWidth] = useState(1000);
  const [showCurrentCue, setShowCurrentCue] = useState(false);
  const compactStudy = studyWidth < 760;
  const styles = makeListeningStyles(theme, subtitleSize);
  const hitTrigger = (ref: { current: View | null }, pageX: number, pageY: number) =>
    new Promise<boolean>(resolve => {
      if (!ref.current) { resolve(false); return; }
      ref.current.measureInWindow((x, y, width, height) => {
        resolve(pageX >= x && pageX <= x + width && pageY >= y && pageY <= y + height);
      });
    });
  const handleSettingsBackdropPress = async (pageX: number, pageY: number) => {
    const triggers: [StudySettingsTrigger, { current: View | null }][] = [
      ['subtitle', subtitleSettingsTriggerRef],
      ['speed', speedSettingsTriggerRef],
      ['ai', aiTriggerRef],
      ['practice', practiceTriggerRef],
      ['audio', audioTriggerRef],
      ['exit', exitTriggerRef],
    ];
    for (const [trigger, ref] of triggers) {
      if (!(await hitTrigger(ref, pageX, pageY))) continue;
      const action = resolveStudySettingsMaskTap(trigger, {
        activeSettings,
        aiBusy: !!aiStatus,
        hasSelectedPractice: !!selectedPractice,
      });
      switch (action.kind) {
        case 'close':
          return false;
        case 'open':
          setActiveSettings(action.settings);
          return true;
        case 'ai':
          setActiveSettings(null);
          launchChat();
          return true;
        case 'practice':
          setActiveSettings(null);
          setShowAudioPicker(false);
          setShowPracticePicker(true);
          return true;
        case 'audio':
          setActiveSettings(null);
          setShowPracticePicker(false);
          setShowAudioPicker(true);
          return true;
        case 'exit':
          setActiveSettings(null);
          void handleExitStudy();
          return true;
        default:
          return false;
      }
    }
    return false;
  };

  return (
    <PopoverRoot>
    <View
      style={styles.safeArea}
      onLayout={event => setStudyWidth(event.nativeEvent.layout.width)}
    >
      <StatusBar barStyle={scheme === "dark" ? "light-content" : "dark-content"} />
      <View style={[styles.screenBody, styles.studyScreenBody]}>
        {(showPracticePicker || showAudioPicker) && <Pressable
          accessibilityRole="button" accessibilityLabel="关闭音频选择列表"
          style={[StyleSheet.absoluteFill, { zIndex: 5 }]}
          onPress={() => { setShowPracticePicker(false); setShowAudioPicker(false); setActiveSettings(null); }} />}
          <View style={styles.focusSidebar}>
            <View style={[styles.focusSelectorRow, compactStudy && styles.focusSelectorRowCompact]}>
              <View style={styles.focusSelectorSection}>
                <Pressable
                  ref={practiceTriggerRef}
                  accessibilityRole="button"
                  accessibilityLabel="选择练习组"
                  disabled={!!aiStatus}
                  accessibilityState={{ expanded: showPracticePicker }}
                  style={styles.focusSelectorBtn}
                  onPress={() => {
                    setShowPracticePicker((visible) => !visible);
                    setShowAudioPicker(false);
                  }}
                >
                  <Text style={styles.focusSelectorBtnText} numberOfLines={1}>
                    {selectedPractice?.name ?? "选择练习组"}
                  </Text>
                  <Text style={styles.focusSelectorArrow}>▾</Text>
                </Pressable>
                {showPracticePicker && (
                  <ScrollView style={styles.focusDropdown} nestedScrollEnabled>
                    {practices.map((p) => (
                      <Pressable
                        key={p.id}
                        style={[
                          styles.focusDropdownItem,
                          p.id === selectedPracticeId && styles.focusDropdownItemActive,
                        ]}
                        onPress={async () => {
                          if (subtitleTaskRef.current) return;
                          setShowPracticePicker(false);
                          setSelectedPracticeId(p.id);
                          if (p.audios.length > 0) {
                            await selectAudioItem(p, p.audios[0]);
                          } else {
                            setSelectedAudioId(null);
                            await unloadSound();
                            setSubtitleCues([]);
                          }
                        }}
                      >
                        <Text
                          numberOfLines={1}
                          style={[
                            styles.focusDropdownItemText,
                            p.id === selectedPracticeId && styles.focusDropdownItemTextActive,
                          ]}
                        >
                          {p.name}
                        </Text>
                      </Pressable>
                    ))}
                    {practices.length === 0 && (
                      <Text style={styles.focusDropdownEmpty}>暂无练习组</Text>
                    )}
                  </ScrollView>
                )}
              </View>

              <View style={styles.focusSelectorSection}>
                <Pressable
                  ref={audioTriggerRef}
                  accessibilityRole="button"
                  accessibilityLabel="选择音频"
                  accessibilityState={{ expanded: showAudioPicker }}
                  disabled={!selectedPractice || !!aiStatus}
                  style={[styles.focusSelectorBtn, !selectedPractice && styles.focusSelectorBtnDisabled]}
                  onPress={() => {
                    setShowAudioPicker((visible) => !visible);
                    setShowPracticePicker(false);
                  }}
                >
                  <Text style={styles.focusSelectorBtnText} numberOfLines={1}>
                    {selectedAudio?.name ?? "选择音频"}
                  </Text>
                  <Text style={styles.focusSelectorArrow}>▾</Text>
                </Pressable>
                {showAudioPicker && selectedPractice && (
                  <ScrollView style={styles.focusDropdown} nestedScrollEnabled>
                    {selectedPractice.audios.map((a) => (
                      <Pressable
                        key={a.id}
                        style={[
                          styles.focusDropdownItem,
                          a.id === selectedAudioId && styles.focusDropdownItemActive,
                        ]}
                        onPress={async () => {
                          setShowAudioPicker(false);
                          await selectAudioItem(selectedPractice, a);
                        }}
                      >
                        <Text
                          numberOfLines={1}
                          style={[
                            styles.focusDropdownItemText,
                            a.id === selectedAudioId && styles.focusDropdownItemTextActive,
                          ]}
                        >
                          {a.name}
                        </Text>
                      </Pressable>
                    ))}
                    {selectedPractice.audios.length === 0 && (
                      <Text style={styles.focusDropdownEmpty}>该组暂无音频</Text>
                    )}
                  </ScrollView>
                )}
              </View>
            </View>
            <View style={styles.studyHeaderActions}>
              <View style={styles.studySkipRow}>
                <Text style={styles.studyPageText}>{audioIndex >= 0 ? `${audioIndex + 1}/${selectedPractice?.audios.length ?? 0}` : ''}</Text>
                <Pressable accessibilityRole="button" accessibilityLabel="上一条音频"
                  disabled={!!aiStatus || switchingAudio || audioIndex <= 0}
                  style={[styles.studySkipBtn, (switchingAudio || audioIndex <= 0) && styles.focusSelectorBtnDisabled]}
                  onPress={() => void changeStudyAudio(-1)}><Text style={styles.studySkipText}>‹</Text></Pressable>
                <Pressable accessibilityRole="button" accessibilityLabel="下一条音频"
                  disabled={!!aiStatus || switchingAudio || audioIndex < 0 || audioIndex >= (selectedPractice?.audios.length ?? 0) - 1}
                  style={[styles.studySkipBtn, (switchingAudio || audioIndex < 0 || audioIndex >= (selectedPractice?.audios.length ?? 0) - 1) && styles.focusSelectorBtnDisabled]}
                  onPress={() => void changeStudyAudio(1)}><Text style={styles.studySkipText}>›</Text></Pressable>
              </View>
              <View style={styles.studyHeadingActions}>
                {studySidebarStatus}
                <StudyOptions open={activeSettings === "subtitle"} onVisibilityChange={(visible) => setActiveSettings(visible ? "subtitle" : null)}
                  triggerRef={subtitleSettingsTriggerRef} onBackdropPress={handleSettingsBackdropPress} suppressReturnFocus={chatOpen}
                  align="right" label="字幕" title="字幕与阅读设置">
                  {selectedAudio && <StudySubtitleToolbar
                    kind={classifySubtitleLanguage(subtitleCues)} hasSubtitles={subtitleCues.length > 0}
                    busy={!!aiStatus || switchingAudio} status={aiStatus}
                    onEnglish={() => void handleUploadSubtitle("en")}
                    onChinese={() => void handleUploadSubtitle("zh")}
                    onTranslate={() => void runTranslate()}
                    onGenerate={() => void handleAiSubtitle()}
                  />}
                  <View style={styles.subtitleSizeRow}>
                    <Text style={styles.subtitleSizeLabel}>字幕大小</Text>
                    <Pressable accessibilityRole="button" accessibilityLabel="缩小字幕" style={styles.subtitleSizeBtn} onPress={() => changeSubtitleSize(-1)}><Text style={styles.subtitleSizeBtnText}>A−</Text></Pressable>
                    <Text style={styles.subtitleSizeVal}>{subtitleSize}</Text>
                    <Pressable accessibilityRole="button" accessibilityLabel="放大字幕" style={styles.subtitleSizeBtn} onPress={() => changeSubtitleSize(1)}><Text style={styles.subtitleSizeBtnText}>A+</Text></Pressable>
                  </View>
                  <Pressable accessibilityRole="checkbox" accessibilityState={{ checked: showCurrentCue }}
                    onPress={() => setShowCurrentCue(value => !value)} style={styles.subtitleSettingRow}>
                    <Text style={styles.loopBtnText}>{showCurrentCue ? '✓ ' : ''}单独显示当前句</Text>
                  </Pressable>
                  <Text style={styles.selectableSubtitleHint}>{Platform.OS === 'android'
                    ? '长按字幕可复制。开启截取后，拖动 A/B 标记设置片段。'
                    : '拖选字幕后右键可复制或向 AI 提问。开启截取后，点击字幕设置 A/B 边界。'}</Text>
                </StudyOptions>
                <View ref={aiTriggerRef} collapsable={false}><AIChatInlineButton /></View>
                {onExitStudy && <Pressable ref={exitTriggerRef} accessibilityRole="button" accessibilityLabel="退出音频学习"
                  style={({ pressed }) => [styles.studyExitBtn, pressed && styles.buttonPressed]}
                  onPress={() => void handleExitStudy()}>
                  <Text style={styles.studyExitText}>退出学习</Text>
                </Pressable>}
              </View>
            </View>
          </View>

        <View style={[styles.container, styles.studyContainer]}>
          {error && (
            <View style={styles.errorBanner}>
              <Text style={styles.errorText}>{error}</Text>
            </View>
          )}


          {selectedAudio && (
          <View style={styles.presentationToggleRow} accessibilityRole="tablist">
            <Pressable accessibilityRole="tab" accessibilityState={{ selected: blind.presentation === 'subtitles' }}
              style={[styles.presentationToggleBtn, blind.presentation === 'subtitles' && styles.presentationToggleBtnActive]}
              onPress={exitBlindMode}>
              <Text style={[styles.presentationToggleText, blind.presentation === 'subtitles' && styles.presentationToggleTextActive]}>字幕</Text>
            </Pressable>
            <Pressable accessibilityRole="tab" accessibilityState={{ selected: blind.presentation === 'blind' }}
              style={[styles.presentationToggleBtn, blind.presentation === 'blind' && styles.presentationToggleBtnActive]}
              onPress={() => void enterBlindMode()}>
              <Text style={[styles.presentationToggleText, blind.presentation === 'blind' && styles.presentationToggleTextActive]}>盲听</Text>
            </Pressable>
          </View>
          )}
          {blind.presentation === 'blind' && selectedAudio ? (
          <BlindListeningPanel
            audioTitle={selectedAudio.name}
            hasSubtitles={blindHasValidCues}
            revealDisabledReason={blindRevealDisabledReason}
            revealed={blindRevealed}
            finished={blindEnded}
            onBackFive={() => void handleBlindBackFive()}
            onReveal={() => void handleBlindReveal()}
            onHideText={() => blindMachine.clearReveal()}
            onResumeBlind={() => void handleBlindResume()}
            onRestart={() => void handleBlindRestart()}
            onViewFull={exitBlindMode}
            onManageSubtitles={exitBlindMode}
          />
          ) : blind.presentation === 'blind' ? (
          <View style={styles.emptyState}>
            <Text style={styles.emptyText}>
              选择练习组和音频后开始学习
            </Text>
          </View>
          ) : (
          <>
          {!!aiStatus && <Text accessibilityLiveRegion="polite" style={styles.studyStatus}>{aiStatus}</Text>}
          {selectedAudio && subtitleCues.length === 0 && <StudySubtitleToolbar
            kind={classifySubtitleLanguage(subtitleCues)} hasSubtitles={false}
            busy={!!aiStatus || switchingAudio} status={null}
            onEnglish={() => void handleUploadSubtitle("en")} onChinese={() => void handleUploadSubtitle("zh")}
            onTranslate={() => void runTranslate()} onGenerate={() => void handleAiSubtitle()}
          />}
          {/* Subtitle list */}
          {subtitleCues.length > 0 ? (
          <View style={styles.subtitleContainer} {...(Platform.OS === "android" ? exitSwipe.panHandlers : null)}>
            {/* Current cue highlight */}
            {showCurrentCue && <View style={styles.currentCue}>
              <Text style={styles.currentCueText}>
                {activeCueIndex >= 0
                  ? subtitleCues[activeCueIndex].text
                  : "·  ·  ·"}
              </Text>
            </View>}

            <View style={styles.nativeSelectableSubtitle}>
            {NativeSelectableSubtitleView ? (
              <NativeSelectableSubtitleView
                style={styles.subtitleScrollSurface}
                text={subtitleTranscript.text}
                fontSize={subtitleSize}
                selectionEnabled={!loopMode}
                textColor={toNativeColor(theme.textSecondary)}
                activeCueStart={subtitleTranscript.activeCueStart}
                activeCueLength={subtitleTranscript.activeCueLength}
                activeWordStart={subtitleTranscript.activeWordStart}
                activeWordLength={subtitleTranscript.activeWordLength}
                activeCueTextColor={toNativeColor(theme.text)}
                activeAccentColor={toNativeColor(theme.accent)}
                activeWordTextColor={toNativeColor("#fff")}
                loopStartMarkerIndex={subtitleTranscript.loopStartMarkerIndex}
                loopStartMarkerAfter={subtitleTranscript.loopStartMarkerAfter}
                loopEndMarkerIndex={subtitleTranscript.loopEndMarkerIndex}
                loopEndMarkerAfter={subtitleTranscript.loopEndMarkerAfter}
                keyRangesJson={JSON.stringify(keyMarks.map((mark) => [mark.start, mark.length]))}
                keyHighlightColor={toNativeColor(KEY_MARK_COLOR)}
                onAskSelection={(event) => openChatWithSelection(event.nativeEvent.text)}
                onTapAtCharacter={(event) => handleSubtitleCharacterTap(event.nativeEvent.index)}
              />
            ) : (
              <ScrollView style={styles.subtitleScrollSurface} contentContainerStyle={styles.androidSubtitleContent}>
                <Text selectable={!loopMode} style={styles.androidSubtitleText}>
                  {androidTranscriptSegments
                    ? androidTranscriptSegments.map((segment, index) =>
                        segment.marked ? (
                          <Text key={index} style={styles.keyMarkText}>
                            {segment.text}
                          </Text>
                        ) : (
                          <Text key={index}>{segment.text}</Text>
                        )
                      )
                    : subtitleTranscript.text}
                </Text>
              </ScrollView>
            )}
            </View>
            {(shadowing || shadowBusy || shadowResult) && (
              <View style={styles.aiAnswerPanel}>
                <View style={styles.aiAnswerPanelHeader}>
                  <Text style={styles.aiAnswerLabel}>
                    {shadowing ? "跟读录音中" : shadowBusy ? "跟读评分中" : "跟读评分"}
                  </Text>
                  <View style={styles.aiAnswerActions}>
                    {shadowing ? (
                      <Pressable onPress={() => void stopShadowing()} hitSlop={8}>
                        <Text style={styles.aiAnswerCopy}>⏹ 停止并评分</Text>
                      </Pressable>
                    ) : !shadowBusy && shadowResult ? (
                      <Pressable onPress={() => setShadowResult(null)} hitSlop={8}>
                        <Text style={styles.aiAnswerClose}>关闭</Text>
                      </Pressable>
                    ) : null}
                  </View>
                </View>
                {shadowing ? (
                  <View style={styles.shadowRecordingBody}>
                    <Text style={styles.shadowRecordingHint}>请朗读下面这句：</Text>
                    <Text style={styles.shadowRecordingText}>
                      {shadowReferenceRef.current}
                    </Text>
                  </View>
                ) : shadowBusy ? (
                  <View style={styles.aiLoadingRow}>
                    <ActivityIndicator size="small" color={theme.accent} />
                    <Text style={styles.aiLoadingText}>{shadowBusy}</Text>
                  </View>
                ) : shadowResult ? (
                  <ScrollView style={styles.aiAnswerPanelBody} nestedScrollEnabled>
                    <View style={styles.shadowScoreRow}>
                      {shadowResult.score ? (
                        <>
                          <View style={styles.shadowScoreChip}>
                            <Text style={styles.shadowScoreChipValue}>
                              {shadowResult.score.completeness}%
                            </Text>
                            <Text style={styles.shadowScoreChipLabel}>完整度</Text>
                          </View>
                          <View style={styles.shadowScoreChip}>
                            <Text style={styles.shadowScoreChipValue}>
                              {shadowResult.score.accuracy}%
                            </Text>
                            <Text style={styles.shadowScoreChipLabel}>准确度</Text>
                          </View>
                          <View style={styles.shadowScoreChip}>
                            <Text style={styles.shadowScoreChipValue}>
                              {shadowResult.score.wpm} wpm
                            </Text>
                            <Text style={styles.shadowScoreChipLabel}>语速</Text>
                          </View>
                        </>
                      ) : (
                        <Text style={styles.shadowDiffLine}>本次没有获得评分</Text>
                      )}
                    </View>
                    <Text style={styles.shadowSectionLabel}>原句</Text>
                    <Text style={styles.shadowBodyText}>{shadowResult.reference}</Text>
                    <Text style={styles.shadowSectionLabel}>我的转写</Text>
                    <Text style={styles.shadowBodyText}>
                      {shadowResult.transcript || "（无识别结果）"}
                    </Text>
                    {shadowResult.score && shadowResult.score.missed.length > 0 && (
                      <Text style={styles.shadowDiffLine}>
                        <Text style={styles.shadowDiffLabel}>漏读：</Text>
                        <Text style={styles.shadowDiffMissed}>
                          {shadowResult.score.missed.join("、")}
                        </Text>
                      </Text>
                    )}
                    {shadowResult.score && shadowResult.score.wrong.length > 0 && (
                      <Text style={styles.shadowDiffLine}>
                        <Text style={styles.shadowDiffLabel}>读错：</Text>
                        <Text style={styles.shadowDiffWrong}>
                          {shadowResult.score.wrong.join("、")}
                        </Text>
                      </Text>
                    )}
                    {shadowResult.score && shadowResult.score.extra.length > 0 && (
                      <Text style={styles.shadowDiffLine}>
                        <Text style={styles.shadowDiffLabel}>多读：</Text>
                        <Text style={styles.shadowDiffExtra}>
                          {shadowResult.score.extra.join("、")}
                        </Text>
                      </Text>
                    )}
                    {shadowResult.feedback ? (
                      <>
                        <Text style={styles.shadowSectionLabel}>教练点评</Text>
                        <Text style={styles.shadowBodyText}>{shadowResult.feedback}</Text>
                      </>
                    ) : null}
                  </ScrollView>
                ) : null}
              </View>
            )}
            {keyPoints.length > 0 && (
              <View style={styles.aiAnswerPanel}>
                <View style={styles.aiAnswerPanelHeader}>
                  <Text style={styles.aiAnswerLabel}>AI 重点 · {keyPoints.length} 处（点击句子跳转）</Text>
                  <View style={styles.aiAnswerActions}>
                    <Pressable onPress={clearKeyPoints} hitSlop={8}>
                      <Text style={styles.aiAnswerClose}>清除</Text>
                    </Pressable>
                  </View>
                </View>
                <ScrollView style={styles.aiAnswerPanelBody} nestedScrollEnabled>
                  {keyPoints.map((point, index) => (
                    <Pressable
                      key={`${point.phrase}-${index}`}
                      style={styles.keyPointItem}
                      onPress={() => seekToKeyPoint(keyMarks[index])}
                    >
                      <Text style={styles.keyPointPhrase}>{point.phrase}</Text>
                      {point.note ? (
                        <Text style={styles.keyPointNote}>{point.note}</Text>
                      ) : null}
                    </Pressable>
                  ))}
                </ScrollView>
              </View>
            )}
          </View>
        ) : selectedAudio ? (
          <View style={styles.emptyState}>
            <Text style={styles.emptyText}>
              该音频暂无字幕{"\n"}请到「音频管理」添加或生成字幕
            </Text>
          </View>
        ) : (
          <View style={styles.emptyState}>
            <Text style={styles.emptyText}>
              选择练习组和音频后开始学习
            </Text>
          </View>
        )}
          </>
          )}

          {/* Player controls */}
          {selectedAudio && (
          <View style={styles.playerArea}>
            {/* AB timeline: drag ▶/◀ handles to set the loop range. Hidden
                until 截取片段 is on — the always-on drag track was too easy
                to trigger by accident (now gated on every platform). */}
            {durationMs > 0 && loopMode && (
              <ABTimelineBar
                durationMs={durationMs}
                startMs={loopStartMs ?? 0}
                endMs={loopEndMs ?? durationMs}
                onChange={(which, ms) => {
                  const rounded = Math.round(ms);
                  const nextStart = which === "A" ? rounded : loopStartMs;
                  const nextEnd = which === "B" ? rounded : loopEndMs;
                  if (which === "A") setLoopStartMs(rounded);
                  else setLoopEndMs(rounded);
                  setLoopMode(true);
                  // Keep the word-tap state machine in sync with slider edits.
                  setLoopStage(
                    nextStart != null && nextEnd != null
                      ? "done"
                      : nextStart != null
                      ? "B"
                      : "A"
                  );
                }}
              />
            )}
            {/* Progress bar: the whole 28pt strip is clickable; the thin
                rail inside shows progress and the playhead line marks the
                current position. */}
            <View style={styles.progressRow}>
            <Text style={styles.timeText}>{formatTime(positionMs / 1000)}</Text>
            <Pressable
              style={styles.progressBarTrack}
              onLayout={(e) => {
                progressBarWidthRef.current = e.nativeEvent.layout.width;
              }}
              onPress={(e) => {
                const width = progressBarWidthRef.current;
                if (!width || !durationMs) return;
                const ratio = Math.max(0, Math.min(1, e.nativeEvent.locationX / width));
                seekTo(ratio * durationMs);
              }}
            >
              <View style={styles.progressBarRail}>
                <View
                  style={[
                    styles.progressBarFill,
                    {
                      width: durationMs > 0
                        ? `${(positionMs / durationMs) * 100}%`
                        : "0%",
                    },
                  ]}
                />
              </View>
              {durationMs > 0 && (
                <View
                  style={[
                    styles.progressBarPlayhead,
                    { left: `${Math.max(0, Math.min(100, (positionMs / durationMs) * 100))}%` },
                  ]}
                />
              )}
            </Pressable>
            <Text style={styles.timeText}>{formatTime(durationMs / 1000)}</Text>
            </View>

            {/* Play / Speed controls */}
            <View style={styles.controlsRow}>
              <Pressable
                accessibilityLabel={isPlaying ? "暂停播放" : "播放音频"}
                style={styles.playBtn}
                onPress={togglePlayback}
              >
                <Text style={styles.playBtnText}>
                  {isPlaying ? "⏸" : "▶"}
                </Text>
              </Pressable>

              <View style={styles.playerToolsFixed}>
              <StudyOptions open={activeSettings === "speed"} onVisibilityChange={(visible) => setActiveSettings(visible ? "speed" : null)}
                triggerRef={speedSettingsTriggerRef} onBackdropPress={handleSettingsBackdropPress} suppressReturnFocus={chatOpen}
                direction="up" align="left" label="播放速度" badge={pendingRate == null ? `当前 ${playbackRate}×` : `待应用 ${pendingRate}×`} title="播放速度">
                {<View style={styles.speedOptions}>
                  {[0.6, 0.7, 0.8, 1.0, 1.2].map(rate => <Pressable key={rate} accessibilityRole="button"
                    accessibilityLabel={`${rate} 倍速`} accessibilityState={{ selected: playbackRate === rate, busy: rateApplying && playbackRate !== rate }}
                    disabled={rateApplying}
                    style={[styles.speedBtn, playbackRate === rate && styles.speedBtnActive]}
                    onPress={() => { void changeRate(rate); }}>
                    <Text style={[styles.speedBtnText, playbackRate === rate && styles.speedBtnTextActive]}>{rate}×</Text>
                  </Pressable>)}
                </View>}
              </StudyOptions>
              </View>
              <ScrollView horizontal showsHorizontalScrollIndicator={false} style={styles.playerTools} contentContainerStyle={styles.playerToolsContent}>

              <Pressable
                style={[styles.loopBtn, loopMode && styles.loopBtnActive]}
                onPress={toggleLoopMode}
              >
                <Text style={[styles.loopBtnText, loopMode && styles.loopBtnTextActive]}>
                  {loopMode ? "截取中" : "截取片段"}
                </Text>
              </Pressable>

              <Pressable
                accessibilityLabel="切换循环模式"
                style={[styles.loopBtn, repeatMode !== "off" && styles.loopBtnActive]}
                onPress={cycleRepeatMode}
              >
                <Text style={[styles.loopBtnText, repeatMode !== "off" && styles.loopBtnTextActive]}>
                  {repeatMode === "off" ? "循环关" : repeatMode === "one" ? "单篇循环" : repeatMode === "practice" ? "组循环" : "单次连播"}
                </Text>
              </Pressable>

              <Pressable
                accessibilityLabel={blind.presentation === 'blind' ? '跟读（切换到字幕模式后使用）' : '跟读录音并评分'}
                style={[styles.loopBtn, shadowing && styles.shadowBtnActive, blind.presentation === 'blind' && !shadowing && styles.toolBtnDisabled]}
                onPress={() => {
                  if (blind.presentation === 'blind' && !shadowing) { showToast('切换到字幕模式后使用'); return; }
                  void (shadowing ? stopShadowing() : startShadowing());
                }}
              >
                <Text style={[styles.loopBtnText, shadowing && styles.shadowBtnTextActive]}>
                  {shadowing ? `⏹ 停止跟读 ${shadowElapsed}s` : "🎤 跟读"}
                </Text>
              </Pressable>

              <Pressable
                accessibilityLabel={blind.presentation === 'blind' ? '标重点（切换到字幕模式后使用）' : 'AI 标记重点'}
                style={[styles.loopBtn, keyMarks.length > 0 && styles.loopBtnActive, blind.presentation === 'blind' && styles.toolBtnDisabled]}
                onPress={() => {
                  if (blind.presentation === 'blind') { showToast('切换到字幕模式后使用'); return; }
                  if (keyLoading) {
                    cancelKeyPoints();
                  } else if (keyMarks.length > 0) {
                    clearKeyPoints();
                  } else {
                    void markKeyPoints();
                  }
                }}
              >
                <Text
                  style={[styles.loopBtnText, keyMarks.length > 0 && styles.loopBtnTextActive]}
                >
                  {keyLoading ? "标重点中…" : keyMarks.length > 0 ? "✓ 已标重点" : "✦ 标重点"}
                </Text>
              </Pressable>
              </ScrollView>
            </View>

            {loopMode && (
              <View style={styles.loopStatus}>
                <Text style={styles.loopStatusText}>
                  {loopRange
                    ? `片段 ${formatTime(loopRange.startMs / 1000)} → ${formatTime(
                        loopRange.endMs / 1000
                      )}`
                    : loopStartMs != null
                    ? "片段起点 A 已设，点击字幕设置终点 B（点单词可微调边界）"
                    : "点击字幕设置片段起点 A"}
                </Text>
                <View style={{ flexDirection: "row", alignItems: "center", gap: 12 }}>
                  {loopRange ? (
                    <Pressable onPress={() => void saveLoopToFlashcards()} hitSlop={8}>
                      <Text style={[styles.loopClear, { color: theme.accent, fontWeight: "600" }]}>
                        ⭐ 收藏到闪卡
                      </Text>
                    </Pressable>
                  ) : null}
                  <Pressable onPress={clearLoopRange} hitSlop={8}>
                    <Text style={styles.loopClear}>清除片段</Text>
                  </Pressable>
                </View>
              </View>
            )}
          </View>
          )}
        {/* Toast */}
        {toast && (
          <View style={styles.toast}>
            <Text style={styles.toastText}>{toast}</Text>
          </View>
        )}
        </View>
      </View>
    </View>
    </PopoverRoot>
  );
}
