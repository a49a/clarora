import { StudyOptions } from "../ui/StudyOptions";
import { PopoverRoot } from "../ui/PopoverRoot";
import { StudySubtitleToolbar } from "../ui/StudySubtitleToolbar";
import { LibraryActionMenu } from "../ui/LibraryActionMenu";
import { AudioLibraryManager } from "../ui/AudioLibraryManager";
import { NativeSelectableSubtitleView } from "../ui/NativeSelectableSubtitle";
import { learningDesign } from "../ui/learningDesign";
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import {
  ActivityIndicator,
  Alert,
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
  TextInput,
  View,
} from "react-native";
import {
  Audio,
  audioRecorder,
  AVPlaybackStatus,
  copyToClipboard,
  DocumentPicker,
  FileSystem,
  mergeAudios,
  runShellCommand,
  type SoundLike,
} from "../services/platform";

import {
  ListeningPractice,
  ListeningAudio,
  listListeningPractices,
  createListeningPractice,
  addAudioToPractice,
  renameListeningPractice,
  renameListeningAudio,
  updateAudioSubtitle,
  deleteListeningPractice,
  deleteListeningAudio,
  getSetting,
  setSetting,
  saveClipCard,
  recordListenSeconds,
} from "../data/database";
import { mergeSubtitleLanguage, serializeSubtitleCues, classifySubtitleLanguage, parseSubtitleCues, SubtitleCue, type SubtitleLanguage } from "../data/subtitles";
import {
  transcribeAudio,
  waitForSubtitles,
  translateSubtitlesFile,
  waitForTranslation,
  deleteJob,
  askAboutPassage,
  getActiveAsrEngine,
  listAsrEngines,
  updateAsrEngine,
  speakingAttempt,
  waitForSpeaking,
  type AsrEngineInfo,
  type SpeakingScore,
} from "../services/ai";
import { useAppTheme } from "../ui/ThemeContext";
import { AIChatInlineButton, useAIChat, useAIChatEntry } from "../ui/AIChatProvider";

const ASR_ENGINE_LABELS: Record<string, string> = {
  local: "端侧转写",
  compatible: "自定义转写 API",
  qwen3_asr: "Qwen3-ASR",
  moss: "MOSS",
  sensevoice: "SenseVoice",
  faster_whisper: "Whisper",
};

type CueToken = { text: string; isWord: boolean; offset: number };
type TranscriptWord = { token: CueToken; start: number; length: number };
type TranscriptCue = {
  cueIndex: number;
  start: number;
  end: number;
  primaryEnd: number;
  words: TranscriptWord[];
};

type ListeningScreenProps = {
  mode?: "study" | "manage";
  initialPracticeId?: string | null;
  initialAudioId?: string | null;
  onSelectionChange?: (practiceId: string | null, audioId: string | null) => void;
  onStartStudy?: (practiceId: string, audioId: string) => void;
  onExitStudy?: () => void;
  studySidebarStatus?: ReactNode;
};

const LAST_LISTENING_SELECTION_SETTING = "last_listening_selection";
const AI_QUESTION_TIMEOUT_MS = 60_000;

// RCTViewManager's custom NSColor props receive an already processed native
// color value, unlike ordinary React Native style colors which are processed
// by the style system automatically.
function toNativeColor(color: string): number | null {
  const processed = processColor(color);
  return typeof processed === "number" ? processed : null;
}

function getFileNameFromUri(uri: string): string {
  const parts = uri.split(/[\\/]/);
  const name = parts[parts.length - 1] || uri;
  return decodeURIComponent(name);
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

function formatSrtTimestamp(totalSeconds: number): string {
  const clamped = Math.max(0, totalSeconds);
  const hours = Math.floor(clamped / 3600);
  const minutes = Math.floor((clamped % 3600) / 60);
  const seconds = Math.floor(clamped % 60);
  const millis = Math.round((clamped - Math.floor(clamped)) * 1000);
  const pad = (value: number, width = 2) => value.toString().padStart(width, "0");
  return `${pad(hours)}:${pad(minutes)}:${pad(seconds)},${pad(millis, 3)}`;
}

/** Serialize cues (times in seconds) into an SRT document. */
function serializeSrt(cues: SubtitleCue[]): string {
  return (
    cues
      .map(
        (cue, index) =>
          `${index + 1}\n${formatSrtTimestamp(cue.start)} --> ${formatSrtTimestamp(cue.end)}\n${cue.text}\n`
      )
      .join("\n") + "\n"
  );
}

/** Draggable A/B timeline: drag the ▶/◀ handles to set the loop range. */
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

  const styles = makeStyles(theme);
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

export default function ListeningScreen({
  mode = "study",
  initialPracticeId = null,
  initialAudioId = null,
  onSelectionChange,
  onStartStudy,
  onExitStudy,
  studySidebarStatus,
}: ListeningScreenProps = {}) {
  const { theme, scheme } = useAppTheme();
  const isManageMode = mode === "manage";

  const [practices, setPractices] = useState<ListeningPractice[]>([]);
  const [libraryLoaded, setLibraryLoaded] = useState(false);
  const [selectedPracticeId, setSelectedPracticeId] = useState<string | null>(initialPracticeId);
  const [selectedAudioId, setSelectedAudioId] = useState<string | null>(initialAudioId);
  const [selectionRestored, setSelectionRestored] = useState(
    initialPracticeId !== null || initialAudioId !== null
  );
  const [error, setError] = useState<string | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  const [aiStatus, setAiStatus] = useState<string | null>(null);
  const subtitleTaskRef = useRef(false);
  const [scanImporting, setScanImporting] = useState(false);
  const [scanFiles, setScanFiles] = useState<Array<{ path: string; name: string }>>([]);
  const [scanPreviewVisible, setScanPreviewVisible] = useState(false);
  const canUseCommandImport = Platform.OS === "macos";
  const [activeAsrModel, setActiveAsrModel] = useState("");
  const [asrEngines, setAsrEngines] = useState<AsrEngineInfo[]>([]);
  const [activeAsrBackend, setActiveAsrBackend] = useState<string>("");
  const [asrEnginesLoading, setAsrEnginesLoading] = useState(false);
  const [showEnginePicker, setShowEnginePicker] = useState(false);

  const refreshCommandImportAccess = useCallback(async () => {
    void getActiveAsrEngine().then((engine) => {
      if (engine) {
        setActiveAsrModel(
          `${ASR_ENGINE_LABELS[engine.backend] ?? engine.backend} ${engine.model}`.trim()
        );
      }
    });
  }, []);

  useEffect(() => {
    void refreshCommandImportAccess();
    const subscription = AppState.addEventListener("change", (state) => {
      if (state === "active") void refreshCommandImportAccess();
    });
    return () => subscription.remove();
  }, [refreshCommandImportAccess]);

  // Segment loop (选段循环) state — range in milliseconds, draggable on the
  // AB timeline and adjustable word-by-word in the subtitle list.
  const [loopMode, setLoopMode] = useState(false);
  const [loopStage, setLoopStage] = useState<"A" | "B" | "done" | null>(null);
  const [loopStartMs, setLoopStartMs] = useState<number | null>(null);
  const [loopEndMs, setLoopEndMs] = useState<number | null>(null);
  const progressBarWidthRef = useRef(0);

  // Audio state
  const soundRef = useRef<SoundLike | null>(null);
  const audioOperationRef = useRef<Promise<void>>(Promise.resolve());
  const audioGenerationRef = useRef(0);
  const enqueueAudioOperation = useCallback((operation: () => Promise<void>) => {
    const result = audioOperationRef.current.then(operation, operation);
    audioOperationRef.current = result.then(() => {}, () => {});
    return result;
  }, []);
  // 听力时长统计：播放中按轮询间隔累计真实秒数，攒够 5 秒落一次库。
  const listenAccumRef = useRef(0);
  const listenLastTickRef = useRef<number | null>(null);
  const mountedRef = useRef(true);
  const loadedAudioIdRef = useRef<string | null>(null);
  const loadingAudioIdRef = useRef<string | null>(null);
  // React may run the selected-audio effect while a group transition is still
  // loading the same item. Keep an auto-play request through that overlap so
  // the effect's ordinary load cannot turn a group transition into a pause.
  const pendingAutoPlayAudioIdRef = useRef<string | null>(null);
  const [isPlaying, setIsPlaying] = useState(false);
  const [positionMs, setPositionMs] = useState(0);
  const [durationMs, setDurationMs] = useState(0);
  const [playbackRate, setPlaybackRate] = useState(1);
  const [pendingRate, setPendingRate] = useState<number | null>(null);
  const positionMsRef = useRef(0);

  // Repeat playback: off → single audio → whole practice group.
  const [repeatMode, setRepeatMode] = useState<"off" | "one" | "practice" | "pass">("off");
  const repeatModeRef = useRef(repeatMode);
  const repeatAdvanceRef = useRef<(() => void) | null>(null);
  const repeatAdvancingRef = useRef(false);
  const lastRepeatAtRef = useRef(0);

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
  const [subtitleCues, setSubtitleCues] = useState<SubtitleCue[]>([]);
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
  const { visible: chatOpen, open: openChat, launch: launchChat } = useAIChat();
  // 设置弹层统一互斥:同一时间只允许一个前景层(字幕/倍速)。
  const [activeSettings, setActiveSettings] = useState<'subtitle' | 'speed' | null>(null);
  const subtitleSettingsTriggerRef = useRef<View | null>(null);
  const speedSettingsTriggerRef = useRef<View | null>(null);
  const aiTriggerRef = useRef<View | null>(null);
  const practiceTriggerRef = useRef<View | null>(null);
  const audioTriggerRef = useRef<View | null>(null);
  const exitTriggerRef = useRef<View | null>(null);
  // 最新期望倍速:加载新音频与异步改速都从这里读取,避免闭包拿到旧值。
  const desiredRateRef = useRef(1);
  const rateApplyingRef = useRef(false);
  const [rateApplying, setRateApplying] = useState(false);

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
  const [batchMode, setBatchMode] = useState(false);
  const [batchSelected, setBatchSelected] = useState<string[]>([]);

  // Batch audio management
  const [batchAudioMode, setBatchAudioMode] = useState(false);
  const [batchAudioSelected, setBatchAudioSelected] = useState<string[]>([]);
  const [mergingAudios, setMergingAudios] = useState(false);

  // Prompt dialog state (cross-platform replacement for Alert.prompt)
  const [promptVisible, setPromptVisible] = useState(false);
  const [libraryMenu, setLibraryMenu] = useState<"audio" | "practice" | null>(null);
  const [promptTitle, setPromptTitle] = useState("");
  // 字幕查看器：生成或导入后可以直接翻看转录结果。
  const [subtitleViewerVisible, setSubtitleViewerVisible] = useState(false);
  const [subtitleViewerLoading, setSubtitleViewerLoading] = useState(false);
  const [subtitleViewerCues, setSubtitleViewerCues] = useState<SubtitleCue[]>([]);
  const [promptValue, setPromptValue] = useState("");
  const promptResolveRef = useRef<((value: string | null) => void) | null>(null);

  const showPrompt = useCallback((title: string, defaultValue = ""): Promise<string | null> => {
    return new Promise((resolve) => {
      setPromptTitle(title);
      setPromptValue(defaultValue);
      promptResolveRef.current = resolve;
      setPromptVisible(true);
    });
  }, []);

  const handlePromptConfirm = useCallback(() => {
    setPromptVisible(false);
    promptResolveRef.current?.(promptValue);
    promptResolveRef.current = null;
  }, [promptValue]);

  const handlePromptCancel = useCallback(() => {
    setPromptVisible(false);
    promptResolveRef.current?.(null);
    promptResolveRef.current = null;
  }, []);

  // NOTE: no programmatic focus here. react-native-macos's focus command
  // crashes in AppKit (NSWindow first-responder exception), so the user
  // clicks into the input manually.

  const selectedPractice = useMemo(
    () => practices.find((p) => p.id === selectedPracticeId) ?? null,
    [practices, selectedPracticeId]
  );

  const selectedAudio = useMemo(() => {
    if (!selectedPractice) return null;
    return selectedPractice.audios.find((a) => a.id === selectedAudioId) ?? null;
  }, [selectedPractice, selectedAudioId]);

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

  // Restore the last audio before persisting anything new. Without this gate,
  // the first null render after an app restart would overwrite the saved IDs.
  useEffect(() => {
    let cancelled = false;

    if (initialPracticeId !== null || initialAudioId !== null) {
      setSelectionRestored(true);
      return () => {
        cancelled = true;
      };
    }

    void (async () => {
      try {
        const raw = await getSetting(LAST_LISTENING_SELECTION_SETTING);
        if (!raw || cancelled) return;
        const saved: unknown = JSON.parse(raw);
        if (!saved || typeof saved !== "object") return;
        const record = saved as { practiceId?: unknown; audioId?: unknown };
        const practiceId = typeof record.practiceId === "string" ? record.practiceId : null;
        const audioId = practiceId && typeof record.audioId === "string" ? record.audioId : null;
        if (!cancelled) {
          setSelectedPracticeId(practiceId);
          setSelectedAudioId(audioId);
        }
      } catch {
        // A malformed or unavailable setting should not block the library.
      } finally {
        if (!cancelled) setSelectionRestored(true);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [initialAudioId, initialPracticeId]);

  useEffect(() => {
    onSelectionChange?.(selectedPracticeId, selectedAudioId);
    if (!selectionRestored) return;
    void setSetting(
      LAST_LISTENING_SELECTION_SETTING,
      JSON.stringify({ practiceId: selectedPracticeId, audioId: selectedAudioId })
    ).catch(() => {});
  }, [onSelectionChange, selectedAudioId, selectedPracticeId, selectionRestored]);

  useEffect(() => {
    if (!libraryLoaded) return;
    if (selectedPracticeId && !selectedPractice) {
      setSelectedPracticeId(null);
      setSelectedAudioId(null);
      return;
    }
    if (selectedAudioId && selectedPractice && !selectedAudio) {
      setSelectedAudioId(null);
    }
  }, [
    libraryLoaded,
    selectedAudio,
    selectedAudioId,
    selectedPractice,
    selectedPracticeId,
  ]);

  const showToast = useCallback(
    (msg: string) => {
      setToast(msg);
      setTimeout(() => setToast(null), 2300);
    },
    []
  );

  const loadAsrEngines = useCallback(async () => {
    setAsrEnginesLoading(true);
    try {
      const list = await listAsrEngines();
      setAsrEngines(list.backends);
      setActiveAsrBackend(list.config.backend ?? "");
    } catch (e: any) {
      setError(`读取转写引擎失败：${e?.message ?? e}`);
      setShowEnginePicker(false);
    } finally {
      setAsrEnginesLoading(false);
    }
  }, []);

  const selectAsrEngine = useCallback(
    async (engine: AsrEngineInfo) => {
      if (!engine.available || engine.backend === activeAsrBackend) return;
      setAsrEnginesLoading(true);
      try {
        const updated = await updateAsrEngine(engine.backend);
        setActiveAsrBackend(updated.backend);
        showToast(`转写模型已切换为 ${updated.label}`);
        await loadAsrEngines();
      } catch (e: any) {
        setError(`切换失败：${e?.message ?? e}`);
      } finally {
        setAsrEnginesLoading(false);
      }
    },
    [activeAsrBackend, loadAsrEngines, showToast]
  );

  const openChatWithSelection = useCallback((text: string) => {
    void soundRef.current?.pauseAsync().catch(() => {});
    setActiveSettings(null); // AI 面板成为前景前先收起设置
    openChat({ text, source: selectedAudio?.name || "音频学习" });
  }, [openChat, selectedAudio?.name]);

  useAIChatEntry(
    subtitleCues.length ? (subtitleCues[activeCueIndex >= 0 ? activeCueIndex : 0]?.text || subtitleTranscript.text) : "",
    selectedAudio?.name || "音频学习",
    () => { setActiveSettings(null); void soundRef.current?.pauseAsync().catch(() => {}); }
  );

  // ── 听力跟读 ────────────────────────────────────────────────────────────────

  const startShadowing = useCallback(async () => {
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
      await soundRef.current?.pauseAsync();
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

  // ── Audio playback ──────────────────────────────────────────────────────────

  const unloadSoundNow = useCallback(async () => {
    if (soundRef.current) {
      await soundRef.current.unloadAsync();
      soundRef.current = null;
    }
    loadedAudioIdRef.current = null;
    positionMsRef.current = 0;
    if (mountedRef.current) {
      setIsPlaying(false);
      setPositionMs(0);
      setDurationMs(0);
    }
  }, []);
  const unloadSound = useCallback(async () => {
    ++audioGenerationRef.current;
    await enqueueAudioOperation(unloadSoundNow);
  }, [enqueueAudioOperation, unloadSoundNow]);

  const handleExitStudy = useCallback(async () => {
    if (listenAccumRef.current > 0) {
      const flushed = listenAccumRef.current;
      listenAccumRef.current = 0;
      try {
        await recordListenSeconds(flushed);
      } catch {}
    }
    try {
      await unloadSound();
    } finally {
      onExitStudy?.();
    }
  }, [onExitStudy, unloadSound]);

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
    if (Platform.OS !== "android" || isManageMode) return;
    const subscription = BackHandler.addEventListener("hardwareBackPress", () => {
      if (activeSettings) { setActiveSettings(null); return true; }
      void handleExitStudy();
      return true;
    });
    return () => subscription.remove();
  }, [activeSettings, handleExitStudy, isManageMode]);

  const onPlaybackStatusUpdate = useCallback(
    async (status: AVPlaybackStatus) => {
      if (!status.isLoaded) return;
      // 听力时长：只在播放中累计，暂停/结束不计；跳变 >5s 视为 seek 丢弃。
      if (status.isPlaying) {
        const now = Date.now();
        const last = listenLastTickRef.current;
        if (last != null) {
          const delta = (now - last) / 1000;
          if (delta > 0 && delta < 5) listenAccumRef.current += delta;
        }
        listenLastTickRef.current = now;
        if (listenAccumRef.current >= 5) {
          const flushed = listenAccumRef.current;
          listenAccumRef.current = 0;
          void recordListenSeconds(flushed).catch(() => {});
        }
      } else {
        listenLastTickRef.current = null;
      }
      positionMsRef.current = status.positionMillis;
      setPositionMs(status.positionMillis);
      setDurationMs(status.durationMillis ?? 0);
      setIsPlaying(status.isPlaying);

      // Repeat handling. The status callback is registered once per loaded
      // sound, so the current repeat mode and queue step are read through refs.
      if (status.didJustFinish) {
        const now = Date.now();
        if (repeatAdvancingRef.current || now - lastRepeatAtRef.current < 800) return;
        lastRepeatAtRef.current = now;
        repeatAdvancingRef.current = true;
        if (repeatModeRef.current === "one") {
          const sound = soundRef.current;
          if (sound) {
            try {
              await sound.setPositionAsync(0);
              await sound.playAsync();
            } finally {
              repeatAdvancingRef.current = false;
            }
          } else {
            repeatAdvancingRef.current = false;
          }
        } else if (repeatModeRef.current === "practice" || repeatModeRef.current === "pass") {
          try {
            await repeatAdvanceRef.current?.();
          } finally {
            repeatAdvancingRef.current = false;
          }
        } else {
          repeatAdvancingRef.current = false;
        }
      }
    },
    []
  );

  const loadAudio = useCallback(
    async (audio: ListeningAudio) => {
      const generation = ++audioGenerationRef.current;
      let loaded = false;
      await enqueueAudioOperation(async () => {
        let sound: SoundLike | null = null;
        try {
          if (generation !== audioGenerationRef.current) return;
          await unloadSoundNow();
          if (generation !== audioGenerationRef.current) return;
          if (mountedRef.current) setPendingRate(desiredRateRef.current);
          ({ sound } = await Audio.Sound.createAsync(
            { uri: audio.audio_uri },
            { rate: desiredRateRef.current, progressUpdateIntervalMillis: 100 },
            onPlaybackStatusUpdate
          ));
          if (!mountedRef.current || generation !== audioGenerationRef.current) {
            await sound.unloadAsync().catch(() => {});
            return;
          }
          soundRef.current = sound;
          await sound.setRateAsync(desiredRateRef.current, true);
          if (!mountedRef.current || generation !== audioGenerationRef.current) {
            await sound.unloadAsync().catch(() => {});
            soundRef.current = null;
            return;
          }
          setPlaybackRate(desiredRateRef.current);
          setPendingRate(null);
          loaded = true;
        } catch (error: any) {
          if (sound) {
            await sound.unloadAsync().catch(() => {});
            if (soundRef.current === sound) soundRef.current = null;
          }
          if (mountedRef.current && generation === audioGenerationRef.current) {
            setPendingRate(null); // 加载失败时清除待应用标记,避免徽标长期驻留
            setError(`加载音频失败：${error?.message ?? error}`);
          }
        }
      });
      return loaded ? generation : null;
    },
    [enqueueAudioOperation, unloadSoundNow, onPlaybackStatusUpdate]
  );

  const loadSubtitle = useCallback(async (audio: ListeningAudio, generation?: number) => {
    if (!audio.subtitle_uri) {
      if (generation == null || generation === audioGenerationRef.current) setSubtitleCues([]);
      return;
    }
    try {
      const content = await FileSystem.readAsStringAsync(audio.subtitle_uri);
      const cues = parseSubtitleCues(content);
      if (mountedRef.current && (generation == null || generation === audioGenerationRef.current)) setSubtitleCues(cues);
    } catch (e: any) {
      if (mountedRef.current && (generation == null || generation === audioGenerationRef.current)) {
        setSubtitleCues([]);
        setError(`加载字幕失败：${e?.message ?? e}`);
      }
    }
  }, []);

  const prepareAudioForStudy = useCallback(
    async (audio: ListeningAudio, autoPlay = false) => {
      if (loadedAudioIdRef.current === audio.id) {
        if (autoPlay) await soundRef.current?.playAsync();
        return;
      }
      if (loadingAudioIdRef.current === audio.id) {
        if (autoPlay) pendingAutoPlayAudioIdRef.current = audio.id;
        return;
      }
      loadingAudioIdRef.current = audio.id;
      try {
        const generation = await loadAudio(audio);
        if (generation == null) return;
        await loadSubtitle(audio, generation);
        if (generation !== audioGenerationRef.current) return;
        loadedAudioIdRef.current = audio.id;
        if (autoPlay || pendingAutoPlayAudioIdRef.current === audio.id) {
          await enqueueAudioOperation(async () => {
            if (generation === audioGenerationRef.current) await soundRef.current?.playAsync();
          });
        }
      } finally {
        if (pendingAutoPlayAudioIdRef.current === audio.id) {
          pendingAutoPlayAudioIdRef.current = null;
        }
        if (loadingAudioIdRef.current === audio.id) loadingAudioIdRef.current = null;
      }
    },
    [enqueueAudioOperation, loadAudio, loadSubtitle]
  );

  const selectAudioItem = useCallback(
    async (practice: ListeningPractice, audio: ListeningAudio, autoPlay = false) => {
      if (subtitleTaskRef.current) return;
      setSelectedPracticeId(practice.id);
      setSelectedAudioId(audio.id);
      setLoopStartMs(null);
      setLoopEndMs(null);
      setLoopStage(null);
      // 换音频后旧的重点标记不再对应新转写，直接清掉。
      clearKeyPoints();
      if (mode === "study") {
        await prepareAudioForStudy(audio, autoPlay);
      } else {
        setSubtitleCues([]);
      }
    },
    [mode, prepareAudioForStudy, clearKeyPoints]
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
    if (mode !== "study" || !selectedAudio) return;
    void prepareAudioForStudy(selectedAudio);
  }, [mode, prepareAudioForStudy, selectedAudio]);

  // Keep the repeat refs in sync with the latest mode and practice queue.
  useEffect(() => {
    repeatModeRef.current = repeatMode;
  }, [repeatMode]);

  useEffect(() => {
    repeatAdvanceRef.current = async () => {
      const practice = selectedPractice;
      if (!practice || practice.audios.length === 0) return;
      const index = practice.audios.findIndex((audio) => audio.id === selectedAudioId);
      // 单次连播：整组按顺序过一遍，最后一篇播完即停，不回卷。
      if (repeatModeRef.current === "pass" && index >= practice.audios.length - 1) {
        setIsPlaying(false);
        return;
      }
      const next = practice.audios[(index + 1) % practice.audios.length];
      // Continue straight into the next audio without waiting for a tap —
      // that's the whole point of 组循环 for chaining short recordings.
      await selectAudioItem(practice, next, true);
    };
  }, [selectAudioItem, selectedAudioId, selectedPractice]);

  const cycleRepeatMode = useCallback(() => {
    setRepeatMode((current) =>
      current === "off" ? "one" : current === "one" ? "practice" : current === "practice" ? "pass" : "off"
    );
  }, []);

  const togglePlayback = useCallback(async () => {
    if (!soundRef.current) return;
    if (isPlaying) {
      await soundRef.current.pauseAsync();
    } else {
      await soundRef.current.playAsync();
    }
  }, [isPlaying]);

  const seekTo = useCallback(async (ms: number) => {
    if (!soundRef.current) return;
    positionMsRef.current = ms;
    await soundRef.current.setPositionAsync(ms);
  }, []);

  const seekBy = useCallback(
    async (deltaMs: number) => {
      const limit = durationMs > 0 ? durationMs : Number.POSITIVE_INFINITY;
      const next = Math.max(0, Math.min(limit, positionMsRef.current + deltaMs));
      await seekTo(next);
    },
    [durationMs, seekTo]
  );

  // In the focused macOS listening view: Space plays/pauses, and ←/→ move
  // through the passage in five-second steps without changing playback state.
  useEffect(() => {
    // A React Native management prompt is an editable field, so it must own
    // Space and arrow keys instead of the player monitor consuming them.
    if (mode !== "study" || !selectedAudio || promptVisible || libraryMenu || chatOpen || activeSettings) return;
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
  }, [activeSettings, chatOpen, libraryMenu, mode, promptVisible, seekBy, selectedAudio, togglePlayback]);

  const seekToCue = useCallback(
    async (cueIndex: number) => {
      if (cueIndex < 0 || cueIndex >= subtitleCues.length) return;
      const cue = subtitleCues[cueIndex];
      await seekTo(cue.start * 1000);
      if (!isPlaying && soundRef.current) {
        await soundRef.current.playAsync();
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
      if (!isPlaying && soundRef.current) {
        await soundRef.current.playAsync();
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

  const changeRate = useCallback(
    async (rate: number) => {
      if (rateApplyingRef.current) return; // 快速连点:忽略重复提交
      rateApplyingRef.current = true;
      setRateApplying(true);
      try {
        await enqueueAudioOperation(async () => {
          const sound = soundRef.current;
          if (!sound) {
            if (!selectedAudioId) throw new Error("请先选择音频");
            desiredRateRef.current = rate;
            if (mountedRef.current) {
              setPendingRate(rate);
              setActiveSettings(null);
            }
            return;
          }
          await sound.setRateAsync(rate, true);
          // 只在当前播放器确认成功后提交期望速度和显示档位。
          desiredRateRef.current = rate;
          if (mountedRef.current) {
            setPlaybackRate(rate);
            setPendingRate(null);
            setActiveSettings(null);
          }
        });
      } catch (error: any) {
        // 失败保留旧档,提示原因并允许重试。
        if (mountedRef.current) setError(`设置播放速度失败：${error?.message ?? error}`);
      } finally {
        rateApplyingRef.current = false;
        if (mountedRef.current) setRateApplying(false);
      }
    },
    [enqueueAudioOperation, selectedAudioId]
  );

  // ── Segment loop (选段循环) ────────────────────────────────────────────────

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
        if (!isPlaying && soundRef.current) {
          soundRef.current.playAsync();
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
        if (!isPlaying && soundRef.current) {
          soundRef.current.playAsync();
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
      mountedRef.current = false;
      const sound = soundRef.current;
      soundRef.current = null;
      void sound?.unloadAsync();
      if (listenAccumRef.current > 0) {
        const flushed = listenAccumRef.current;
        listenAccumRef.current = 0;
        void recordListenSeconds(flushed).catch(() => {});
      }
      // 正在跟读录音时直接退出：丢弃未提交的录音，避免泄漏麦克风。
      if (shadowTimerRef.current) clearInterval(shadowTimerRef.current);
      if (shadowingRef.current) {
        shadowingRef.current = false;
        audioRecorder.stop().catch(() => {});
      }
    };
  }, []);

  // ── Practice management ─────────────────────────────────────────────────────

  // 字幕语言形态（双语/中文/仅原文）：管理页按 audioId 展示徽标与筛选。
  const [subtitleKinds, setSubtitleKinds] = useState<Record<string, SubtitleLanguage>>({});
  const subtitleKindCache = useRef<Record<string, SubtitleLanguage | "unknown">>({});

  useEffect(() => {
    // 缓存键为 id:uri，翻译/更换字幕后自动重新判定。
    if (mode !== "manage") return;
    let cancelled = false;
    const run = async () => {
      for (const practice of practices) {
        for (const audio of practice.audios) {
          const cacheKey = `${audio.id}:${audio.subtitle_uri}`;
          if (!audio.subtitle_uri || subtitleKindCache.current[cacheKey]) continue;
          try {
            const content = await FileSystem.readAsStringAsync(audio.subtitle_uri);
            const kind = classifySubtitleLanguage(parseSubtitleCues(content));
            subtitleKindCache.current[cacheKey] = kind;
            if (cancelled) return;
            setSubtitleKinds(prev => ({ ...prev, [audio.id]: kind }));
          } catch {
            // 读取失败保持未知，列表回退显示「字幕就绪」。
            subtitleKindCache.current[cacheKey] = "unknown";
          }
        }
      }
    };
    void run();
    return () => { cancelled = true; };
  }, [mode, practices]);

  const refreshPractices = useCallback(async () => {
    try {
      const list = await listListeningPractices();
      setPractices(list);
      setLibraryLoaded(true);
    } catch (e: any) {
      setError(`加载练习列表失败：${e?.message ?? e}`);
    }
  }, []);

  useEffect(() => {
    refreshPractices();
  }, [refreshPractices]);

  const handleCreatePractice = useCallback(async () => {
    const name = await showPrompt(
      "请输入练习组名称",
      `我的听力练习 ${new Date().toLocaleDateString()}`
    );
    if (!name?.trim()) return;
    try {
      const created = await createListeningPractice(name.trim());
      await refreshPractices();
      setSelectedPracticeId(created.id);
      setSelectedAudioId(null);
      showToast(`练习已创建：${created.name}`);
    } catch (e: any) {
      setError(`创建失败：${e?.message ?? e}`);
    }
  }, [refreshPractices, showToast, showPrompt]);

  const handleAddAudio = useCallback(async () => {
    if (!selectedPracticeId) return;
    setError(null);
    try {
      // Pick audio file
      const audioResult = await DocumentPicker.getDocumentAsync({
        type: "audio/*",
        copyToCacheDirectory: true,
      });
      if (audioResult.canceled || !audioResult.assets?.length) return;
      const audioAsset = audioResult.assets[0];

      const defaultName = audioAsset.name.replace(/\.[^.]+$/, "");

      const inputName = await showPrompt("请输入音频名称", defaultName);
      const name = inputName?.trim() || defaultName;
      try {
        const created = await addAudioToPractice(selectedPracticeId, name, audioAsset.uri, "");
        await refreshPractices();
        setSelectedAudioId(created.id);
        showToast(`音频已添加：${name}`);
      } catch (e: any) {
        setError(`添加音频失败：${e?.message ?? e}`);
      }
    } catch (e: any) {
      setError(`添加音频失败：${e?.message ?? e}`);
    }
  }, [selectedPracticeId, refreshPractices, showToast]);

  const handleUploadSubtitle = useCallback(async (language?: "en" | "zh") => {
    if (!selectedAudio || subtitleTaskRef.current) return;
    subtitleTaskRef.current = true;
    setError(null);
    setAiStatus("请选择字幕文件…");
    try {
      const result = await DocumentPicker.getDocumentAsync({ type: "text/*", copyToCacheDirectory: true });
      if (result.canceled || !result.assets?.length) return;
      setAiStatus("正在导入字幕…");
      const content = await FileSystem.readAsStringAsync(result.assets[0].uri);
      const imported = parseSubtitleCues(content);
      if (!imported.length) throw new Error("没有找到带时间轴的字幕，请选择 SRT 或 VTT 文件。");
      const existing = language && selectedAudio.subtitle_uri
        ? parseSubtitleCues(await FileSystem.readAsStringAsync(selectedAudio.subtitle_uri)) : [];
      const cues = language ? mergeSubtitleLanguage(existing, imported, language) : imported;
      const dir = (await FileSystem.getDocumentDirectoryAsync()) + "listening/";
      await FileSystem.makeDirectoryAsync(dir);
      const uri = `${dir}${Date.now()}_import.srt`;
      await FileSystem.writeFileAsync(uri, serializeSubtitleCues(cues));
      await updateAudioSubtitle(selectedAudio.id, uri);
      await loadSubtitle({ ...selectedAudio, subtitle_uri: uri });
      clearKeyPoints();
      await refreshPractices();
      showToast(language ? `${language === "zh" ? "中文" : "英文"}字幕已导入` : "字幕已更新");
    } catch (e: any) {
      setError(`添加字幕失败：${e?.message ?? e}`);
    } finally {
      subtitleTaskRef.current = false;
      setAiStatus(null);
    }
  }, [selectedAudio, loadSubtitle, clearKeyPoints, refreshPractices, showToast]);

  // Run the configured shell command locally (client machine) and import the
  // audio files whose paths it prints, e.g. `find ~/Music -name "*.mp3"`.
  const handleScanImport = useCallback(async () => {
    if (!selectedPractice) return;
    const command = (await getSetting("import_command"))?.trim();
    if (!command) {
      Alert.alert("未配置导入命令", "请先到「设置 → 命令行导入」填写命令");
      return;
    }
    setScanImporting(true);
    setError(null);
    try {
      const output = await runShellCommand(command);
      const audioExtensions = new Set([
        ".mp3", ".m4a", ".wav", ".aac", ".aiff", ".flac", ".ogg", ".mp4", ".mov",
      ]);
      const files: Array<{ path: string; name: string }> = [];
      for (const rawLine of output.split("\n")) {
        const line = rawLine.trim().replace(/^["']|["']$/g, "");
        if (!line || line.endsWith("(deleted)")) continue;
        const dot = line.lastIndexOf(".");
        const ext = dot === -1 ? "" : line.slice(dot).toLowerCase();
        if (!audioExtensions.has(ext)) continue;
        const name = (line.split("/").pop() ?? line).replace(/\.[^.]+$/, "");
        if (!name) continue;
        files.push({ path: line, name });
      }
      if (files.length === 0) {
        showToast("命令没有返回音频文件");
        return;
      }
      setScanFiles(files);
      setScanPreviewVisible(true);
    } catch (e: any) {
      setError(`命令导入失败：${e?.message ?? e}`);
    } finally {
      setScanImporting(false);
    }
  }, [selectedPractice, showToast]);

  // Actually import the previewed files after the user confirms.
  const confirmScanImport = useCallback(async () => {
    if (!selectedPractice || scanFiles.length === 0) return;
    setScanPreviewVisible(false);
    let imported = 0;
    let skipped = 0;
    let lastImportedId: string | null = null;
    for (const file of scanFiles) {
      const name = file.name.trim();
      if (!name) {
        skipped += 1; // empty name
        continue;
      }
      try {
        const created = await addAudioToPractice(selectedPractice.id, name, file.path, "");
        lastImportedId = created.id;
        imported += 1;
      } catch {
        skipped += 1; // duplicate name etc.
      }
    }
    await refreshPractices();
    setScanFiles([]);
    if (lastImportedId) setSelectedAudioId(lastImportedId);
    showToast(
      skipped > 0
        ? `已导入 ${imported} 个音频，跳过 ${skipped} 个（重名）`
        : `已导入 ${imported} 个音频`
    );
  }, [selectedPractice, scanFiles, refreshPractices, showToast]);

  // Generate subtitles directly through the configured transcription API.
  const handleAiSubtitle = useCallback(async () => {
    if (!selectedAudio || subtitleTaskRef.current) return;
    subtitleTaskRef.current = true;
    setError(null);
    const jobIdRef: { current: string | null } = { current: null };
    try {
      const ext = (selectedAudio.audio_uri.split(".").pop() ?? "mp3").toLowerCase();
      const fileName = `${selectedAudio.name}.${ext}`;
      setAiStatus("正在准备转写…");
      const { jobId } = await transcribeAudio(selectedAudio.audio_uri, fileName);
      jobIdRef.current = jobId;
      const engine = await getActiveAsrEngine();
      const engineLabel = engine
        ? `${ASR_ENGINE_LABELS[engine.backend] ?? engine.backend} ${engine.model}`.trim()
        : "";
      setAiStatus(engineLabel ? `正在转写（${engineLabel}）…` : "正在转写…");
      const srt = await waitForSubtitles(jobId, (progress) => {
        setAiStatus(`正在转写（${engineLabel}）… ${Math.round(progress * 100)}%`);
      });
      const dir = (await FileSystem.getDocumentDirectoryAsync()) + "listening/";
      await FileSystem.makeDirectoryAsync(dir);
      const srtUri = `${dir}${Date.now()}_ai.srt`;
      await FileSystem.writeFileAsync(srtUri, srt);
      await updateAudioSubtitle(selectedAudio.id, srtUri);
      await loadSubtitle({ ...selectedAudio, subtitle_uri: srtUri });
      clearKeyPoints();
      await refreshPractices();
      setAiStatus(null);
      showToast("字幕已生成");
      // 生成完直接把结果摆到用户面前。
      setSubtitleViewerCues(parseSubtitleCues(srt));
      setSubtitleViewerLoading(false);
      if (isManageMode) setSubtitleViewerVisible(true);
    } catch (e: any) {
      setAiStatus(null);
      setError(`AI 生成字幕失败：${e?.message ?? e}`);
    } finally {
      subtitleTaskRef.current = false;
      if (jobIdRef.current) {
        deleteJob(jobIdRef.current);
      }
    }
  }, [isManageMode, selectedAudio, loadSubtitle, clearKeyPoints, refreshPractices, showToast]);

  // Actually run the translate-then-swap flow.
  const runTranslate = useCallback(async () => {
    if (!selectedAudio || subtitleTaskRef.current) return;
    subtitleTaskRef.current = true;
    setError(null);
    const jobIdRef: { current: string | null } = { current: null };
    try {
      const fileName = getFileNameFromUri(selectedAudio.subtitle_uri ?? "");
      setAiStatus("正在上传字幕…");
      const { jobId } = await translateSubtitlesFile(
        selectedAudio.subtitle_uri,
        fileName,
        { lang: "zh", mode: "bilingual" }
      );
      jobIdRef.current = jobId;
      const srt = await waitForTranslation(jobId, (message) => setAiStatus(message));
      const dir = (await FileSystem.getDocumentDirectoryAsync()) + "listening/";
      await FileSystem.makeDirectoryAsync(dir);
      const srtUri = `${dir}${Date.now()}_zh.srt`;
      await FileSystem.writeFileAsync(srtUri, srt);
      await updateAudioSubtitle(selectedAudio.id, srtUri);
      await loadSubtitle({ ...selectedAudio, subtitle_uri: srtUri });
      clearKeyPoints();
      await refreshPractices();
      setAiStatus(null);
      showToast("已生成中英双语字幕");
      setSubtitleViewerCues(parseSubtitleCues(srt));
      setSubtitleViewerLoading(false);
      if (isManageMode) setSubtitleViewerVisible(true);
    } catch (e: any) {
      setAiStatus(null);
      setError(`翻译字幕失败：${e?.message ?? e}`);
    } finally {
      subtitleTaskRef.current = false;
      if (jobIdRef.current) {
        deleteJob(jobIdRef.current);
      }
    }
  }, [isManageMode, selectedAudio, loadSubtitle, clearKeyPoints, refreshPractices, showToast]);

  // 直接翻看当前音频的字幕转录结果（生成/导入后均可查看）。
  const openSubtitleViewer = useCallback(async () => {
    if (!selectedAudio?.subtitle_uri) return;
    setSubtitleViewerVisible(true);
    setSubtitleViewerLoading(true);
    try {
      const content = await FileSystem.readAsStringAsync(selectedAudio.subtitle_uri);
      setSubtitleViewerCues(parseSubtitleCues(content));
    } catch (e: any) {
      setSubtitleViewerCues([]);
      setError(`读取字幕失败：${e?.message ?? e}`);
    } finally {
      setSubtitleViewerLoading(false);
    }
  }, [selectedAudio]);

  const copySubtitleViewerText = useCallback(() => {
    const text = subtitleViewerCues.map((cue) => cue.text).join("\n");
    if (!text) {
      showToast("字幕内容为空");
      return;
    }
    if (!copyToClipboard(text)) {
      showToast("当前设备暂不支持复制");
      return;
    }
    showToast("已复制字幕文本");
  }, [subtitleViewerCues, showToast]);

  // Translate the current subtitle through the configured model API.
  // Provider credentials belong to this device.
  // If the current subtitle is already a generated translation, confirm  // before overwriting the old one.
  const handleTranslateSubtitle = useCallback(async () => {
    if (!selectedAudio?.subtitle_uri) return;
    const currentName = getFileNameFromUri(selectedAudio.subtitle_uri);
    if (/_zh\.srt$/i.test(currentName)) {
      Alert.alert(
        "重新翻译",
        "当前字幕已是翻译版本，重新翻译会覆盖旧的中文翻译。继续吗？",
        [
          { text: "取消", style: "cancel" },
          { text: "重新翻译", onPress: () => void runTranslate() },
        ]
      );
      return;
    }
    await runTranslate();
  }, [selectedAudio, runTranslate]);

  const handleRenamePractice = useCallback(async (name: string) => {
    if (!selectedPractice || !name.trim() || name.trim() === selectedPractice.name) return;
    await renameListeningPractice(selectedPractice.id, name.trim());
    await refreshPractices();
    showToast("练习组已重命名");
  }, [selectedPractice, refreshPractices, showToast]);

  const handleRenameAudio = useCallback(async (name: string) => {
    if (!selectedAudio || !name.trim() || name.trim() === selectedAudio.name) return;
    await renameListeningAudio(selectedAudio.id, name.trim());
    await refreshPractices();
    showToast("音频已重命名");
  }, [selectedAudio, refreshPractices, showToast]);

  // Best-effort removal of copied audio/subtitle files inside the app data dir.
  const cleanupFiles = useCallback(async (uris: string[]) => {
    if (uris.length === 0) return;
    try {
      const base = await FileSystem.getDocumentDirectoryAsync();
      for (const uri of uris) {
        if (uri && uri.startsWith(base)) {
          try {
            await FileSystem.deleteAsync(uri);
          } catch {
            // file already gone — ignore
          }
        }
      }
    } catch {
      // base dir unavailable — skip cleanup
    }
  }, []);

  const handleDeletePractice = useCallback(async () => {
    if (!selectedPractice) return;
    await unloadSound();
    const files = await deleteListeningPractice(selectedPractice.id);
    await cleanupFiles(files);
    setSelectedPracticeId(null);
    setSelectedAudioId(null);
    setSubtitleCues([]);
    await refreshPractices();
    showToast("练习组已删除");
  }, [selectedPractice, unloadSound, refreshPractices, showToast, cleanupFiles]);

  const handleDeleteAudio = useCallback(async () => {
    if (!selectedAudio) return;
    await unloadSound();
    const files = await deleteListeningAudio(selectedAudio.id);
    await cleanupFiles(files);
    setSelectedAudioId(null);
    setSubtitleCues([]);
    await refreshPractices();
    showToast("音频已删除");
  }, [selectedAudio, unloadSound, refreshPractices, showToast, cleanupFiles]);

  // ── Batch practice management ───────────────────────────────────────────────

  const allBatchSelected = practices.length > 0 && batchSelected.length === practices.length;

  const toggleBatchMode = useCallback(() => {
    setBatchMode((v) => !v);
    setBatchSelected([]);
  }, []);

  const toggleBatchItem = useCallback((id: string) => {
    setBatchSelected((prev) => (prev.includes(id) ? prev.filter((x) => x !== id) : [...prev, id]));
  }, []);

  const toggleBatchAll = useCallback(() => {
    setBatchSelected(allBatchSelected ? [] : practices.map((p) => p.id));
  }, [allBatchSelected, practices]);

  const handleBatchDelete = useCallback(() => {
    if (batchSelected.length === 0) return;
    Alert.alert(
      "删除练习组",
      `确认删除所选 ${batchSelected.length} 个练习组及其全部音频？`,
      [
        { text: "取消", style: "cancel" },
        {
          text: "删除",
          style: "destructive",
          onPress: async () => {
            try {
              if (selectedPracticeId && batchSelected.includes(selectedPracticeId)) {
                await unloadSound();
                setSelectedPracticeId(null);
                setSelectedAudioId(null);
                setSubtitleCues([]);
              }
              const deletedFiles: string[] = [];
              for (const id of batchSelected) {
                deletedFiles.push(...(await deleteListeningPractice(id)));
              }
              await cleanupFiles(deletedFiles);
              await refreshPractices();
              setBatchSelected([]);
              setBatchMode(false);
              showToast(`已删除 ${batchSelected.length} 个练习组`);
            } catch (e: any) {
              setError(`批量删除失败：${e?.message ?? e}`);
            }
          },
        },
      ]
    );
  }, [batchSelected, selectedPracticeId, unloadSound, refreshPractices, showToast, cleanupFiles]);

  // ── Batch audio management ──────────────────────────────────────────────

  const allBatchAudioSelected =
    !!selectedPractice &&
    selectedPractice.audios.length > 0 &&
    batchAudioSelected.length === selectedPractice.audios.length;

  const toggleBatchAudioMode = useCallback(() => {
    setBatchAudioMode((visible) => !visible);
    setBatchAudioSelected([]);
  }, []);

  const toggleBatchAudioItem = useCallback((id: string) => {
    setBatchAudioSelected((prev) =>
      prev.includes(id) ? prev.filter((item) => item !== id) : [...prev, id]
    );
  }, []);

  const toggleBatchAudioAll = useCallback(() => {
    const audios = selectedPractice?.audios ?? [];
    setBatchAudioSelected(allBatchAudioSelected ? [] : audios.map((audio) => audio.id));
  }, [allBatchAudioSelected, selectedPractice]);

  const handleBatchAudioDelete = useCallback(() => {
    if (batchAudioSelected.length === 0) return;
    Alert.alert(
      "删除音频",
      `确认删除所选 ${batchAudioSelected.length} 个音频及其字幕文件？`,
      [
        { text: "取消", style: "cancel" },
        {
          text: "删除",
          style: "destructive",
          onPress: async () => {
            try {
              if (selectedAudioId && batchAudioSelected.includes(selectedAudioId)) {
                await unloadSound();
                setSelectedAudioId(null);
                setSubtitleCues([]);
              }
              const deletedFiles: string[] = [];
              for (const id of batchAudioSelected) {
                deletedFiles.push(...(await deleteListeningAudio(id)));
              }
              await cleanupFiles(deletedFiles);
              await refreshPractices();
              setBatchAudioSelected([]);
              setBatchAudioMode(false);
              showToast(`已删除 ${batchAudioSelected.length} 个音频`);
            } catch (e: any) {
              setError(`批量删除失败：${e?.message ?? e}`);
            }
          },
        },
      ]
    );
  }, [batchAudioSelected, selectedAudioId, unloadSound, cleanupFiles, refreshPractices, showToast]);

  // Concatenate the selected audios (in practice order) into one new audio so
  // short recordings can be studied as a single continuous item. Subtitles are
  // re-based on each segment's real duration and stored as one SRT file.
  const mergeSelectedAudios = useCallback(
    async (chosen: ListeningAudio[], name: string) => {
      if (!selectedPractice) return;
      setMergingAudios(true);
      setError(null);
      try {
        const documentDir = await FileSystem.getDocumentDirectoryAsync();
        const directory = `${documentDir}Clarora/Merged`;
        await FileSystem.makeDirectoryAsync(directory);
        const base = `${directory}/merged_${Date.now()}`;
        const audioUri = `${base}.m4a`;
        const merged = await mergeAudios(
          chosen.map((audio) => audio.audio_uri),
          audioUri
        );

        let subtitleUri = "";
        const cues: SubtitleCue[] = [];
        let offsetMs = 0;
        for (let index = 0; index < chosen.length; index++) {
          const audio = chosen[index];
          if (audio.subtitle_uri) {
            try {
              const content = await FileSystem.readAsStringAsync(audio.subtitle_uri);
              for (const cue of parseSubtitleCues(content)) {
                cues.push({
                  ...cue,
                  start: (cue.start * 1000 + offsetMs) / 1000,
                  end: (cue.end * 1000 + offsetMs) / 1000,
                });
              }
            } catch {
              // One unreadable subtitle should not fail the whole merge.
            }
          }
          offsetMs += merged.segmentDurationMs[index] ?? 0;
        }
        if (cues.length > 0) {
          subtitleUri = `${base}.srt`;
          await FileSystem.writeFileAsync(subtitleUri, serializeSrt(cues));
        }

        const created = await addAudioToPractice(selectedPractice.id, name, audioUri, subtitleUri);
        await refreshPractices();
        setBatchAudioSelected([]);
        setBatchAudioMode(false);
        setSelectedAudioId(created.id);
        showToast(`已合并为「${name}」`);
      } catch (e: any) {
        setError(`合并失败：${e?.message ?? e}`);
      } finally {
        setMergingAudios(false);
      }
    },
    [selectedPractice, refreshPractices, showToast]
  );

  const handleBatchAudioMerge = useCallback(() => {
    if (Platform.OS !== "macos" && Platform.OS !== "windows") {
      showToast("合并音频请在桌面端操作");
      return;
    }
    if (!selectedPractice || mergingAudios) return;
    // Practice order (导入顺序) — the order shown in the list is the order
    // that will play in the merged audio.
    const chosen = selectedPractice.audios.filter((audio) =>
      batchAudioSelected.includes(audio.id)
    );
    if (chosen.length < 2) {
      showToast("请至少选择两个音频");
      return;
    }
    const defaultName = `合并 · ${chosen[0].name} 等${chosen.length}篇`;
    void (async () => {
      const name = await showPrompt("请输入合并后的音频名称", defaultName);
      if (!name?.trim()) return;
      Alert.alert(
        "合并音频",
        `将按以下顺序拼接成一个音频（原音频保留）：\n${chosen
          .map((audio, index) => `${index + 1}. ${audio.name}`)
          .join("\n")}`,
        [
          { text: "取消", style: "cancel" },
          { text: "开始合并", onPress: () => void mergeSelectedAudios(chosen, name.trim()) },
        ]
      );
    })();
  }, [selectedPractice, batchAudioSelected, mergingAudios, mergeSelectedAudios, showPrompt, showToast]);

  const showPracticeMenu = useCallback(() => {
    if (selectedPractice) setLibraryMenu("practice");
  }, [selectedPractice]);
  const showAudioMenu = useCallback(() => {
    if (selectedAudio) setLibraryMenu("audio");
  }, [selectedAudio]);

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
  const selectLibraryItem = async (practice: ListeningPractice, audio?: ListeningAudio) => {
    if (switchingAudio || aiStatus || scanImporting) return;
    setSwitchingAudio(true);
    try {
      if (audio) await selectAudioItem(practice, audio);
      else {
        setSelectedPracticeId(practice.id);
        setSelectedAudioId(null);
        await unloadSound();
        setSubtitleCues([]);
      }
    } catch (error: any) { setError(`切换音频失败：${error?.message ?? error}`); }
    finally { setSwitchingAudio(false); }
  };
  const [studyWidth, setStudyWidth] = useState(1000);
  const [showCurrentCue, setShowCurrentCue] = useState(false);
  const compactStudy = studyWidth < 760;
  const styles = makeStyles(theme, subtitleSize);
  const hitTrigger = (ref: { current: View | null }, pageX: number, pageY: number) =>
    new Promise<boolean>(resolve => {
      if (!ref.current) { resolve(false); return; }
      ref.current.measureInWindow((x, y, width, height) => {
        resolve(pageX >= x && pageX <= x + width && pageY >= y && pageY <= y + height);
      });
    });
  const handleSettingsBackdropPress = async (pageX: number, pageY: number) => {
    if (await hitTrigger(subtitleSettingsTriggerRef, pageX, pageY)) {
      if (activeSettings === 'subtitle') return false;
      setActiveSettings('subtitle');
      return true;
    }
    if (await hitTrigger(speedSettingsTriggerRef, pageX, pageY)) {
      if (activeSettings === 'speed') return false;
      setActiveSettings('speed');
      return true;
    }
    if (await hitTrigger(aiTriggerRef, pageX, pageY)) {
      setActiveSettings(null);
      launchChat();
      return true;
    }
    if (await hitTrigger(practiceTriggerRef, pageX, pageY) && !aiStatus) {
      setActiveSettings(null);
      setShowAudioPicker(false);
      setShowPracticePicker(true);
      return true;
    }
    if (await hitTrigger(audioTriggerRef, pageX, pageY) && selectedPractice && !aiStatus) {
      setActiveSettings(null);
      setShowPracticePicker(false);
      setShowAudioPicker(true);
      return true;
    }
    if (await hitTrigger(exitTriggerRef, pageX, pageY)) {
      setActiveSettings(null);
      void handleExitStudy();
      return true;
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
      <View style={[styles.screenBody, !isManageMode && styles.studyScreenBody]}>
        {!isManageMode && (showPracticePicker || showAudioPicker) && <Pressable
          accessibilityRole="button" accessibilityLabel="关闭音频选择列表"
          style={[StyleSheet.absoluteFill, { zIndex: 5 }]}
          onPress={() => { setShowPracticePicker(false); setShowAudioPicker(false); setActiveSettings(null); }} />}
        {!isManageMode && (
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
        )}

        <View style={[styles.container, !isManageMode && styles.studyContainer]}>
          {/* Management header stays above the management workspace. */}
          {isManageMode && (
            <View style={styles.header}>
              <View style={styles.headerCopy}>
                <Text style={styles.title}>音频管理</Text>
                <Text style={styles.subtitle}>
                  {Platform.OS === "android"
                    ? "浏览同步的练习组与音频，直接开始学习。"
                    : "整理练习组、导入音频，并集中处理字幕。"}
                </Text>
              </View>
            </View>
          )}

          {error && (
            <View style={styles.errorBanner}>
              <Text style={styles.errorText}>{error}</Text>
            </View>
          )}

          {isManageMode && <AudioLibraryManager
            subtitleKinds={subtitleKinds}
            practices={practices} practice={selectedPractice} audio={selectedAudio}
            busy={switchingAudio || !!aiStatus || scanImporting} status={aiStatus}
            onSelectPractice={practice => { void selectLibraryItem(practice, practice.audios[0]); }}
            onSelectAudio={audio => { if (selectedPractice) void selectLibraryItem(selectedPractice, audio); }}
            onStart={selectedPractice && selectedAudio && onStartStudy ? () => onStartStudy(selectedPractice.id, selectedAudio.id) : undefined}
            onCreate={handleCreatePractice} onAdd={handleAddAudio}
            onPracticeMenu={showPracticeMenu} onAudioMenu={showAudioMenu}
            onBatch={toggleBatchAudioMode}
            onGenerate={handleAiSubtitle} onView={() => { void openSubtitleViewer(); }}
            onTranslate={handleTranslateSubtitle} onUploadSubtitle={() => void handleUploadSubtitle()}
            advanced={<View style={{ gap: 10 }}>
              {!!activeAsrModel && <Text style={styles.toolRowCaption}>当前模型：{activeAsrModel.split(/[\\/]/).pop()}</Text>}
              {Platform.OS === 'macos' && canUseCommandImport && <Pressable
                accessibilityRole="button" disabled={!selectedPractice || scanImporting || !!aiStatus}
                style={[styles.toolRow, (!selectedPractice || scanImporting || !!aiStatus) && styles.toolBtnDisabled]}
                onPress={handleScanImport}>
                <Text style={styles.toolRowTitle}>{scanImporting ? '导入中…' : '从命令导入音频'}</Text>
                <Text style={styles.toolRowCaption}>导入到当前练习组</Text>
              </Pressable>}
              {<Pressable accessibilityRole="button" accessibilityState={{ expanded: showEnginePicker }}
                disabled={!!aiStatus || asrEnginesLoading} style={[styles.toolRow, (!!aiStatus || asrEnginesLoading) && styles.toolBtnDisabled]}
                onPress={() => { setShowEnginePicker(value => !value); if (!showEnginePicker) void loadAsrEngines(); }}>
                <Text style={styles.toolRowTitle}>转写引擎</Text>
                <Text style={styles.toolRowCaption}>{asrEnginesLoading ? '读取中…' : ASR_ENGINE_LABELS[activeAsrBackend ?? ''] ?? '选择引擎'}</Text>
              </Pressable>}
        {/* 转写 API 状态：模型与地址在设置中配置。 */}
        {isManageMode && showEnginePicker && (
          <View style={styles.dropdown}>
            {asrEngines.map((engine) => (
              <Pressable
                key={engine.backend}
                style={[
                  styles.dropdownItem,
                  engine.backend === activeAsrBackend && styles.dropdownItemActive,
                  !engine.available && styles.toolBtnDisabled,
                ]}
                onPress={() => {
                  void selectAsrEngine(engine);
                  setShowEnginePicker(false);
                }}
              >
                <Text
                  style={[
                    styles.dropdownItemText,
                    engine.backend === activeAsrBackend && styles.dropdownItemTextActive,
                  ]}
                >
                  {engine.label || engine.backend}
                  {engine.backend === activeAsrBackend ? " ✓" : ""}
                </Text>
                <Text style={styles.dropdownEngineMeta}>
                  {engine.available
                    ? `${engine.model}（${engine.device}）`
                    : engine.reason || "当前环境不可用"}
                </Text>
              </Pressable>
            ))}
            {asrEngines.length === 0 && (
              <Text style={styles.dropdownEmpty}>
                {asrEnginesLoading ? "读取中…" : "暂无可用的转写引擎"}
              </Text>
            )}
          </View>
        )}


            </View>}
          />}

        {/* 字幕查看器：生成/导入的结果就地翻看，点空白处关闭。 */}
        {subtitleViewerVisible && (
          <Pressable
            style={styles.subtitleViewerScrim}
            onPress={() => setSubtitleViewerVisible(false)}
          >
            <Pressable style={styles.subtitleViewerPanel} onPress={() => {}}>
              <View style={styles.subtitleViewerHeader}>
                <Text style={styles.subtitleViewerTitle} numberOfLines={1}>
                  字幕内容{selectedAudio ? ` · ${selectedAudio.name}` : ""}
                </Text>
                <View style={styles.subtitleViewerActions}>
                  <Pressable style={styles.subtitleViewerBtn} onPress={copySubtitleViewerText}>
                    <Text style={styles.subtitleViewerBtnText}>复制</Text>
                  </Pressable>
                  <Pressable
                    style={styles.subtitleViewerBtn}
                    onPress={() => setSubtitleViewerVisible(false)}
                  >
                    <Text style={styles.subtitleViewerBtnText}>关闭</Text>
                  </Pressable>
                </View>
              </View>
              {subtitleViewerLoading ? (
                <Text style={styles.subtitleViewerLoading}>加载中…</Text>
              ) : (
                <ScrollView style={styles.subtitleViewerScroll} nestedScrollEnabled>
                  {subtitleViewerCues.map((cue, index) => (
                    <View key={`${index}-${cue.start}`} style={styles.subtitleViewerCue}>
                      <Text style={styles.subtitleViewerTime}>
                        {formatTime(cue.start)} → {formatTime(cue.end)}
                      </Text>
                      <Text style={styles.subtitleViewerText}>{cue.text}</Text>
                    </View>
                  ))}
                  {subtitleViewerCues.length === 0 && (
                    <Text style={styles.subtitleViewerLoading}>字幕内容为空</Text>
                  )}
                </ScrollView>
              )}
            </Pressable>
          </Pressable>
        )}


        {!isManageMode && (
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
                accessibilityLabel="跟读录音并评分"
                style={[styles.loopBtn, shadowing && styles.shadowBtnActive]}
                onPress={() => void (shadowing ? stopShadowing() : startShadowing())}
              >
                <Text style={[styles.loopBtnText, shadowing && styles.shadowBtnTextActive]}>
                  {shadowing ? `⏹ 停止跟读 ${shadowElapsed}s` : "🎤 跟读"}
                </Text>
              </Pressable>

              <Pressable
                accessibilityLabel="AI 标记重点"
                style={[styles.loopBtn, keyMarks.length > 0 && styles.loopBtnActive]}
                onPress={() => {
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
          </>
        )}

        {/* Batch management overlay */}
        {isManageMode && batchMode && (
          <View style={styles.batchOverlay}>
            <View style={styles.batchBox}>
              <Text style={styles.batchTitle}>批量管理练习组</Text>
              <Text style={styles.batchSubtitle}>
                共 {practices.length} 组 · 已选 {batchSelected.length} 组
              </Text>
              <ScrollView style={styles.batchList}>
                {practices.map((p) => {
                  const checked = batchSelected.includes(p.id);
                  return (
                    <Pressable
                      key={p.id}
                      style={styles.batchItem}
                      onPress={() => toggleBatchItem(p.id)}
                    >
                      <View style={[styles.batchCheckbox, checked && styles.batchCheckboxChecked]}>
                        {checked ? <Text style={styles.batchCheckMark}>✓</Text> : null}
                      </View>
                      <Text style={styles.batchItemName} numberOfLines={1}>
                        {p.name}
                      </Text>
                      <Text style={styles.batchItemCount}>{p.audios.length} 音频</Text>
                    </Pressable>
                  );
                })}
                {practices.length === 0 && (
                  <Text style={styles.dropdownEmpty}>暂无练习组</Text>
                )}
              </ScrollView>
              <View style={styles.batchActions}>
                <Pressable style={styles.batchBtnGhost} onPress={toggleBatchAll}>
                  <Text style={styles.batchBtnGhostText}>{allBatchSelected ? "全不选" : "全选"}</Text>
                </Pressable>
                <Pressable
                  style={[styles.batchBtnDanger, batchSelected.length === 0 && styles.batchBtnDisabled]}
                  onPress={handleBatchDelete}
                  disabled={batchSelected.length === 0}
                >
                  <Text style={styles.batchBtnDangerText}>删除所选</Text>
                </Pressable>
                <Pressable style={styles.batchBtnPrimary} onPress={toggleBatchMode}>
                  <Text style={styles.batchBtnPrimaryText}>完成</Text>
                </Pressable>
              </View>
            </View>
          </View>
        )}

        {/* Batch audio management overlay */}
        {isManageMode && batchAudioMode && selectedPractice && (
          <View style={styles.batchOverlay}>
            <View style={styles.batchBox}>
              <Text style={styles.batchTitle}>批量管理音频</Text>
              <Text style={styles.batchSubtitle}>
                「{selectedPractice.name}」共 {selectedPractice.audios.length} 个 · 已选{" "}
                {batchAudioSelected.length} 个
              </Text>
              <ScrollView style={styles.batchList}>
                {selectedPractice.audios.map((audio) => {
                  const checked = batchAudioSelected.includes(audio.id);
                  return (
                    <Pressable
                      key={audio.id}
                      style={styles.batchItem}
                      onPress={() => toggleBatchAudioItem(audio.id)}
                    >
                      <View
                        style={[styles.batchCheckbox, checked && styles.batchCheckboxChecked]}
                      >
                        {checked ? <Text style={styles.batchCheckMark}>✓</Text> : null}
                      </View>
                      <Text style={styles.batchItemName} numberOfLines={1}>
                        {audio.name}
                      </Text>
                      <Text style={styles.batchItemCount}>
                        {audio.subtitle_uri ? "有字幕" : "无字幕"}
                      </Text>
                    </Pressable>
                  );
                })}
                {selectedPractice.audios.length === 0 && (
                  <Text style={styles.dropdownEmpty}>该组暂无音频</Text>
                )}
              </ScrollView>
              <View style={styles.batchActions}>
                <Pressable style={styles.batchBtnGhost} onPress={toggleBatchAudioAll}>
                  <Text style={styles.batchBtnGhostText}>
                    {allBatchAudioSelected ? "全不选" : "全选"}
                  </Text>
                </Pressable>
                {Platform.OS === "macos" && (
                  <Pressable
                    style={[
                      styles.batchBtnPrimary,
                      (batchAudioSelected.length < 2 || mergingAudios) && styles.batchBtnDisabled,
                    ]}
                    onPress={handleBatchAudioMerge}
                    disabled={batchAudioSelected.length < 2 || mergingAudios}
                  >
                    <Text style={styles.batchBtnPrimaryText}>
                      {mergingAudios ? "合并中…" : "合并所选"}
                    </Text>
                  </Pressable>
                )}
                <Pressable
                  style={[
                    styles.batchBtnDanger,
                    batchAudioSelected.length === 0 && styles.batchBtnDisabled,
                  ]}
                  onPress={handleBatchAudioDelete}
                  disabled={batchAudioSelected.length === 0}
                >
                  <Text style={styles.batchBtnDangerText}>删除所选</Text>
                </Pressable>
                <Pressable style={styles.batchBtnPrimary} onPress={toggleBatchAudioMode}>
                  <Text style={styles.batchBtnPrimaryText}>完成</Text>
                </Pressable>
              </View>
            </View>
          </View>
        )}

        {/* Scan import confirmation */}
        {isManageMode && scanPreviewVisible && (
          <View style={styles.batchOverlay}>
            <View style={styles.batchBox}>
              <Text style={styles.batchTitle}>确认导入音频</Text>
              <Text style={styles.batchSubtitle}>
                命令返回 {scanFiles.length} 个音频 · 导入到「{selectedPractice?.name}」
              </Text>
              <ScrollView style={styles.batchList}>
                {scanFiles.map((file, idx) => (
                  <View key={file.path} style={styles.scanFileItem}>
                    <Text style={styles.scanFileLabel}>音频名称</Text>
                    <TextInput
                      style={styles.scanNameInput}
                      value={file.name}
                      onChangeText={(text) =>
                        setScanFiles((prev) =>
                          prev.map((f, i) => (i === idx ? { ...f, name: text } : f))
                        )
                      }
                      placeholder="音频名称"
                      placeholderTextColor={theme.textMuted}
                    />
                    <Text
                      style={styles.scanFilePath}
                      numberOfLines={2}
                      ellipsizeMode="middle"
                    >
                      {file.path}
                    </Text>
                  </View>
                ))}
              </ScrollView>
              <View style={styles.batchActions}>
                <Pressable
                  style={[styles.batchBtnPrimary, { flex: 1 }]}
                  onPress={confirmScanImport}
                >
                  <Text style={styles.batchBtnPrimaryText}>确认导入</Text>
                </Pressable>
                <Pressable style={styles.batchBtnGhost} onPress={() => setScanPreviewVisible(false)}>
                  <Text style={styles.batchBtnGhostText}>取消</Text>
                </Pressable>
              </View>
            </View>
          </View>
        )}

        {libraryMenu && (
          <LibraryActionMenu
            key={libraryMenu}
            kind={libraryMenu}
            name={(libraryMenu === "audio" ? selectedAudio?.name : selectedPractice?.name) || ""}
            onClose={() => setLibraryMenu(null)}
            onRename={libraryMenu === "audio" ? handleRenameAudio : handleRenamePractice}
            onDelete={libraryMenu === "audio" ? handleDeleteAudio : handleDeletePractice}
            onBatch={libraryMenu === "audio" ? toggleBatchAudioMode : toggleBatchMode}
          />
        )}

        {/* Prompt */}
        {promptVisible && (
          <View style={styles.modalOverlay}>
            <Pressable style={styles.modalDismissArea} onPress={handlePromptCancel} />
            <View style={styles.modalBox}>
              <Text style={styles.modalTitle}>{promptTitle}</Text>
              <TextInput
                style={styles.modalInput}
                value={promptValue}
                onChangeText={setPromptValue}
                onSubmitEditing={handlePromptConfirm}
                placeholderTextColor={theme.textSecondary}
              />
              <View style={styles.modalActions}>
                <Pressable style={styles.modalBtnPrimary} onPress={handlePromptConfirm}>
                  <Text style={styles.modalBtnPrimaryText}>确定</Text>
                </Pressable>
                <Pressable style={styles.modalBtnGhost} onPress={handlePromptCancel}>
                  <Text style={styles.modalBtnGhostText}>取消</Text>
                </Pressable>
              </View>
            </View>
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

function makeStyles(
  theme: ReturnType<typeof useAppTheme>["theme"],
  subtitleSize = 14
) {
  const ui = learningDesign(theme);
  return StyleSheet.create({
    safeArea: { flex: 1, backgroundColor: theme.bg },
    screenBody: { flex: 1 },
    studyScreenBody: { flexDirection: 'column' },
    container: {
      flex: 1,
      paddingHorizontal: 18,
      paddingTop: 18,
    },
    studyContainer: { paddingTop: 8, paddingBottom: 6, minHeight: 0 },
    focusSidebar: { flexDirection: 'row', flexWrap: 'wrap', alignItems: 'center', gap: 8, paddingHorizontal: 16, paddingVertical: 8, backgroundColor: theme.surface, borderBottomWidth: 1, borderBottomColor: theme.border, zIndex: 30 },
    focusSelectorRow: { flexDirection: 'row', alignItems: 'center', flex: 1, minWidth: 220, gap: 8, zIndex: 2 },
    focusSelectorRowCompact: { flexBasis: '100%' },
    focusSelectorSection: { flex: 1, minWidth: 0, position: 'relative' },
    studyHeaderActions: { flexDirection: 'row', flexGrow: 1, flexWrap: 'wrap', alignItems: 'center', justifyContent: 'flex-end', gap: 8 },
    studyHeadingActions: { flexDirection: 'row', flexWrap: 'wrap', alignItems: 'center', gap: 6, marginLeft: 'auto' },
    studySkipRow: { flexDirection: 'row', alignItems: 'center', gap: 4 },
    studySkipBtn: { minWidth: 34, minHeight: Platform.OS === 'ios' || Platform.OS === 'android' ? 44 : 34, alignItems: 'center', justifyContent: 'center', borderRadius: 8 },
    studySkipText: { color: theme.accent, fontSize: 22, fontWeight: '600' },
    studyPageText: { color: theme.textMuted, fontSize: 11, fontVariant: ['tabular-nums'] },
    studyExitBtn: { minHeight: Platform.OS === 'ios' || Platform.OS === 'android' ? 44 : 34, justifyContent: 'center', paddingHorizontal: 8, borderRadius: 8 },
    studyExitText: { color: theme.textSecondary, fontSize: 12 },
    studyStatus: { color: theme.accent, fontSize: 12, paddingVertical: 4 },
    subtitleSettingRow: { minHeight: Platform.OS === 'android' ? 48 : 44, justifyContent: 'center' },
    focusSelectorBtn: { flexDirection: 'row', alignItems: 'center', minHeight: Platform.OS === 'android' ? 48 : Platform.OS === 'ios' ? 44 : 34, paddingVertical: 6, paddingHorizontal: 10, borderRadius: 8, backgroundColor: theme.surfaceHover },
    focusSelectorBtnDisabled: { opacity: 0.52 },
    focusSelectorBtnText: {
      flex: 1,
      color: theme.text,
      fontSize: 13,
      fontWeight: "600",
    },
    focusSelectorArrow: { color: theme.textMuted, fontSize: 12, marginLeft: 6 },
    focusDropdown: {
      position: 'absolute',
      top: '100%',
      left: 0,
      right: 0,
      maxHeight: 220,
      marginTop: 6,
      zIndex: 20,
      elevation: 8,
      borderRadius: 12,
      borderWidth: 1,
      borderColor: theme.border,
      backgroundColor: theme.surface,
    },
    focusDropdownItem: {
      paddingVertical: 10,
      paddingHorizontal: 11,
      borderBottomWidth: StyleSheet.hairlineWidth,
      borderBottomColor: theme.border,
    },
    focusDropdownItemActive: { backgroundColor: theme.surfaceHover },
    focusDropdownItemText: { color: theme.text, fontSize: 13 },
    focusDropdownItemTextActive: { color: theme.accent, fontWeight: "700" },
    focusDropdownEmpty: {
      color: theme.textMuted,
      fontSize: 12,
      textAlign: "center",
      paddingVertical: 14,
    },
    header: {
      flexDirection: "row",
      alignItems: "flex-start",
      justifyContent: "space-between",
      gap: 20,
      ...ui.header,
    },
    headerCopy: { flex: 1, alignItems: "flex-start" },
    eyebrow: {
      color: theme.accent,
      fontSize: 11,
      fontWeight: "700",
      marginBottom: 7,
      ...ui.eyebrow,
    },
    title: {
      ...ui.title,
    },
    subtitle: {
      ...ui.subtitle,
    },
    exitStudyBtn: {
      alignItems: "center",
      paddingVertical: 8,
      paddingHorizontal: 14,
      borderRadius: 8,
      borderWidth: 1,
      borderColor: theme.border,
      backgroundColor: theme.sidebarActive,
    },
    exitStudyBtnText: { color: theme.sidebarText, fontSize: 13, fontWeight: "600" },
    buttonPressed: { opacity: 0.72 },
    errorBanner: {
      backgroundColor: `${theme.danger}14`,
      borderRadius: 10,
      paddingVertical: 8,
      paddingHorizontal: 14,
      marginBottom: 10,
    },
    errorText: { color: theme.danger, fontSize: 13, textAlign: "center" },

    // Batch management
    manageBtn: {
      paddingVertical: 9,
      paddingHorizontal: 12,
      borderWidth: 1,
      borderColor: theme.border,
      backgroundColor: theme.surface,
      ...ui.button,
    },
    manageBtnText: { color: theme.textSecondary, fontSize: 12, fontWeight: "600" },
    aiStatus: {
      alignItems: "center",
      paddingVertical: 8,
      marginBottom: 8,
      borderRadius: 6,
      backgroundColor: `${theme.accent}14`,
    },
    aiStatusText: { color: theme.accent, fontSize: 13, fontWeight: "600" },
    batchOverlay: {
      position: "absolute",
      top: 0,
      right: 0,
      bottom: 0,
      left: 0,
      backgroundColor: "rgba(0,0,0,0.45)",
      justifyContent: "center",
      alignItems: "center",
      padding: 32,
    },
    batchBox: {
      width: "100%",
      maxWidth: 480,
      backgroundColor: theme.surface,
      borderRadius: 8,
      borderWidth: 1,
      borderColor: theme.border,
      padding: 20,
      ...theme.cardShadow,
    },
    batchTitle: { fontSize: 18, fontWeight: "700", color: theme.text },
    batchSubtitle: { fontSize: 12, color: theme.textSecondary, marginTop: 4, marginBottom: 12 },
    batchList: {
      maxHeight: 320,
      borderWidth: 1,
      borderColor: theme.border,
      borderRadius: 6,
    },
    batchItem: {
      flexDirection: "row",
      alignItems: "center",
      gap: 10,
      paddingVertical: 10,
      paddingHorizontal: 12,
      borderBottomWidth: StyleSheet.hairlineWidth,
      borderBottomColor: theme.border,
    },
    batchCheckbox: {
      width: 20,
      height: 20,
      borderRadius: 4,
      borderWidth: 1.5,
      borderColor: theme.textMuted,
      alignItems: "center",
      justifyContent: "center",
    },
    batchCheckboxChecked: { backgroundColor: theme.accent, borderColor: theme.accent },
    batchCheckMark: { color: "#fff", fontSize: 13, fontWeight: "700" },
    batchItemName: { flex: 1, color: theme.text, fontSize: 14, fontWeight: "500" },
    batchItemCount: { color: theme.textMuted, fontSize: 11 },
    scanFileItem: {
      paddingVertical: 10,
      paddingHorizontal: 12,
      gap: 6,
      borderBottomWidth: StyleSheet.hairlineWidth,
      borderBottomColor: theme.border,
    },
    scanFileLabel: {
      color: theme.textMuted,
      fontSize: 10,
    },
    scanNameInput: {
      borderWidth: 1,
      borderColor: theme.border,
      borderRadius: 6,
      backgroundColor: theme.bg,
      color: theme.text,
      fontSize: 13,
      paddingVertical: 7,
      paddingHorizontal: 10,
    },
    scanFilePath: {
      color: theme.textSecondary,
      fontSize: 12,
      fontFamily: "Menlo",
      lineHeight: 16,
    },
    batchActions: { flexDirection: "row", alignItems: "center", gap: 8, marginTop: 14 },
    batchBtnGhost: {
      paddingVertical: 8,
      paddingHorizontal: 14,
      borderRadius: 6,
      borderWidth: 1,
      borderColor: theme.border,
    },
    batchBtnGhostText: { color: theme.textSecondary, fontSize: 13, fontWeight: "500" },
    batchBtnDanger: {
      flex: 1,
      alignItems: "center",
      backgroundColor: theme.danger,
      paddingVertical: 8,
      paddingHorizontal: 14,
      borderRadius: 6,
    },
    batchBtnDangerText: { color: "#fff", fontSize: 13, fontWeight: "600" },
    batchBtnPrimary: {
      backgroundColor: theme.accent,
      paddingVertical: 8,
      paddingHorizontal: 14,
      borderRadius: 6,
    },
    batchBtnPrimaryText: { color: "#fff", fontSize: 13, fontWeight: "600" },
    batchBtnDisabled: { opacity: 0.4 },

    // Selectors
    selectorRow: {
      flexDirection: "row",
      alignItems: "center",
      gap: 8,
      marginBottom: 10,
    },
    toolsList: {
      borderRadius: 14,
      borderWidth: 1,
      borderColor: theme.border,
      backgroundColor: theme.surface,
      overflow: "hidden",
      marginBottom: 10,
    },
    toolRow: {
      flexDirection: "row",
      alignItems: "center",
      justifyContent: "space-between",
      gap: 12,
      paddingVertical: 12,
      paddingHorizontal: 14,
      borderBottomWidth: StyleSheet.hairlineWidth,
      borderBottomColor: theme.border,
    },
    toolRowLast: { borderBottomWidth: 0 },
    toolBtnDisabled: { opacity: 0.4 },
    toolRowTitle: { color: theme.text, fontSize: 14, fontWeight: "600" },
    toolRowCaption: { color: theme.textMuted, fontSize: 12, flexShrink: 1, textAlign: "right" },
    selectorBtn: {
      flexDirection: "row",
      alignItems: "center",
      backgroundColor: theme.surface,
      paddingVertical: 10,
      paddingHorizontal: 14,
      ...theme.cardShadow,
      shadowRadius: 2,
      ...ui.button,
    },
    selectorBtnMain: { flex: 1 },
    selectorBtnText: {
      flex: 1,
      color: theme.text,
      fontSize: 14,
      fontWeight: "500",
    },
    selectorArrow: { color: theme.textSecondary, fontSize: 12, marginLeft: 6 },
    iconBtn: {
      width: 38,
      height: 38,
      borderRadius: 10,
      backgroundColor: theme.surface,
      alignItems: "center",
      justifyContent: "center",
      ...theme.cardShadow,
      shadowOpacity: 0.05,
      elevation: 1,
    },
    iconBtnText: { fontSize: 18, color: theme.text },

    // Audio management workspace
    managementPanel: {
      flex: 1,
      minHeight: 220,
      marginTop: 8,
      marginBottom: 24,
      ...theme.cardShadow,
      ...ui.panel,
    },
    managementTitleRow: {
      flexDirection: "row",
      alignItems: "flex-start",
      justifyContent: "space-between",
      gap: 16,
    },
    managementTitleCopy: { flex: 1 },
    managementName: { color: theme.text, fontSize: 22, fontWeight: "700" },
    managementMeta: { color: theme.textSecondary, fontSize: 13, marginTop: 5 },
    managementPath: {
      color: theme.textMuted,
      fontSize: 12,
      lineHeight: 17,
      fontFamily: "Menlo",
      marginTop: 18,
    },
    subtitleBadge: {
      paddingVertical: 5,
      paddingHorizontal: 9,
      borderRadius: 100,
      backgroundColor: `${theme.textMuted}18`,
    },
    subtitleBadgeReady: { backgroundColor: `${theme.accent}18` },
    subtitleBadgeText: { color: theme.textMuted, fontSize: 11, fontWeight: "700" },
    subtitleBadgeTextReady: { color: theme.accent },
    startStudyBtn: {
      alignSelf: "flex-start",
      marginTop: 24,
      paddingVertical: 10,
      paddingHorizontal: 18,
      borderRadius: 8,
      backgroundColor: theme.accent,
    },
    startStudyBtnText: { color: "#fff", fontSize: 14, fontWeight: "700" },
    managementEmpty: { flex: 1, alignItems: "center", justifyContent: "center" },
    managementEmptyTitle: { color: theme.text, fontSize: 18, fontWeight: "700" },
    managementEmptyText: {
      color: theme.textSecondary,
      fontSize: 13,
      lineHeight: 20,
      textAlign: "center",
      marginTop: 8,
    },

    // Dropdown
    dropdown: {
      backgroundColor: theme.surface,
      borderRadius: 10,
      marginBottom: 10,
      ...theme.cardShadow,
      overflow: "hidden",
    },
    dropdownItem: {
      paddingVertical: 12,
      paddingHorizontal: 16,
      borderBottomWidth: StyleSheet.hairlineWidth,
      borderBottomColor: theme.border,
    },
    dropdownItemActive: { backgroundColor: `${theme.accent}18` },
    dropdownItemText: { color: theme.text, fontSize: 14 },
    dropdownItemTextActive: { color: theme.accent, fontWeight: "600" },
    dropdownEmpty: {
      color: theme.textSecondary,
      fontSize: 13,
      textAlign: "center",
      paddingVertical: 16,
    },

    // Subtitle
    subtitleContainer: { flex: 1, minHeight: 0 },
    selectableSubtitleHint: { color: theme.textSecondary, fontSize: 12, marginTop: 8, marginBottom: 6 },
    nativeSelectableSubtitle: {
      flex: 1,
      minHeight: 60,
      borderWidth: 1,
      borderColor: theme.border,
      borderRadius: 12,
      backgroundColor: theme.surface,
      overflow: "hidden",
      paddingVertical: 8,
    },
    // A regular RN View owns the rounded mask; the embedded AppKit scroll
    // view stays inset so its background and scrollbar cannot expose corners.
    subtitleScrollSurface: { flex: 1 },
    androidSubtitleContent: { paddingHorizontal: 14, paddingVertical: 12 },
    androidSubtitleText: {
      color: theme.textSecondary,
      fontSize: subtitleSize,
      lineHeight: subtitleSize + 7,
    },
    selectableCueItem: { color: theme.textSecondary, fontSize: subtitleSize, lineHeight: subtitleSize + 7 },
    aiAnswerLabel: { color: theme.accent, fontSize: 11, fontWeight: "800", marginBottom: 5 },
    aiAnswerPanel: {
      maxHeight: 230,
      marginTop: 8,
      padding: 12,
      borderRadius: 8,
      borderWidth: 1,
      borderColor: theme.border,
      backgroundColor: theme.surface,
    },
    aiAnswerPanelHeader: { flexDirection: "row", justifyContent: "space-between" },
    aiAnswerClose: { color: theme.textSecondary, fontSize: 12 },
    aiAnswerActions: { flexDirection: "row", alignItems: "center", gap: 14 },
    aiAnswerCopy: { color: theme.accent, fontSize: 12, fontWeight: "700" },
    aiAnswerPanelBody: { maxHeight: 132 },
    aiLoadingRow: { flexDirection: "row", alignItems: "center", gap: 8, minHeight: 34 },
    aiLoadingText: { color: theme.textSecondary, fontSize: 13 },
    // 跟读面板：录音提示与评分结果
    shadowBtnActive: { backgroundColor: theme.danger, borderColor: theme.danger },
    shadowBtnTextActive: { color: "#fff" },
    shadowRecordingBody: { minHeight: 34 },
    shadowRecordingHint: { color: theme.textSecondary, fontSize: 11, fontWeight: "700" },
    shadowRecordingText: { color: theme.text, fontSize: 15, lineHeight: 22, marginTop: 4 },
    shadowScoreRow: { flexDirection: "row", gap: 10 },
    shadowScoreChip: {
      alignItems: "center",
      paddingVertical: 6,
      paddingHorizontal: 12,
      borderRadius: 8,
      backgroundColor: theme.bg,
      borderWidth: 1,
      borderColor: theme.border,
    },
    shadowScoreChipValue: { color: theme.accent, fontSize: 15, fontWeight: "700" },
    shadowScoreChipLabel: { color: theme.textMuted, fontSize: 10, marginTop: 2 },
    shadowSectionLabel: { color: theme.textMuted, fontSize: 11, fontWeight: "700", marginTop: 8 },
    shadowBodyText: { color: theme.text, fontSize: 13, lineHeight: 19 },
    shadowDiffLine: { fontSize: 12, lineHeight: 18, marginTop: 4 },
    shadowDiffLabel: { color: theme.textMuted },
    shadowDiffMissed: { color: theme.danger, fontWeight: "600" },
    shadowDiffWrong: { color: "#b8860b", fontWeight: "600" },
    shadowDiffExtra: { color: theme.textSecondary },
    // AI 重点标记：字幕里的下划线短语与重点列表
    keyMarkText: { color: KEY_MARK_COLOR, textDecorationLine: "underline" },
    keyPointItem: { paddingVertical: 6, gap: 2 },
    keyPointPhrase: { color: KEY_MARK_COLOR, fontSize: 13, fontWeight: "700" },
    keyPointNote: { color: theme.textSecondary, fontSize: 12, lineHeight: 17 },
    currentCue: {
      backgroundColor: theme.surface,
      borderRadius: 12,
      paddingVertical: 14,
      paddingHorizontal: 18,
      marginBottom: 10,
      ...theme.cardShadow,
      alignItems: "center",
    },
    subtitleSizeRow: {
      flexDirection: "row",
      alignItems: "center",
      gap: 8,
      paddingVertical: 4,
    },
    subtitleSizeLabel: {
      fontSize: 11,
      color: theme.textMuted,
    },
    subtitleSizeBtn: {
      minHeight: Platform.OS === 'android' ? 48 : 44, minWidth: Platform.OS === 'android' ? 48 : 44, justifyContent: "center", alignItems: "center",
      paddingHorizontal: 8,
      paddingVertical: 2,
      borderRadius: 6,
      borderWidth: 1,
      borderColor: theme.border,
      backgroundColor: theme.surface,
    },
    subtitleSizeBtnText: {
      fontSize: 12,
      color: theme.textSecondary,
      fontWeight: "600",
    },
    subtitleSizeVal: {
      fontSize: 12,
      color: theme.textSecondary,
      fontVariant: ["tabular-nums"],
      minWidth: 18,
      textAlign: "center",
    },
    currentCueText: {
      color: theme.text,
      fontSize: subtitleSize + 2,
      lineHeight: subtitleSize + 8,
      fontWeight: "600",
      textAlign: "center",
    },
    cueLine: {
      color: theme.textSecondary,
    },
    cueTranslation: {
      color: theme.textSecondary,
      fontSize: Math.max(11, subtitleSize - 2),
      lineHeight: Math.max(15, subtitleSize + 2),
      marginTop: 2,
    },
    cueTranslationActive: {
      color: theme.text,
    },
    cueWord: {
      color: theme.textSecondary,
      fontSize: subtitleSize,
      lineHeight: subtitleSize + 6,
    },
    cueItemActive: { backgroundColor: `${theme.accent}12` },
    cueWordInActiveCue: { color: theme.text },
    cueWordPlaying: {
      color: "#fff",
      backgroundColor: theme.accent,
      borderRadius: 4,
      paddingVertical: 1,
      paddingHorizontal: 3,
    },
    cueTextActive: { color: theme.accent, fontWeight: "600" },
    cueTextPast: { opacity: 0.5 },
    cueTime: {
      color: theme.textSecondary,
      fontSize: 11,
      marginLeft: 8,
      marginTop: 2,
      fontVariant: ["tabular-nums"],
    },
    cueItemLoop: { backgroundColor: `${theme.accent}10` },
    loopBadge: { color: theme.accent, fontWeight: "700" },
    loopArrow: { color: theme.accent, fontWeight: "700" },
    loopLineLeft: {
      borderLeftWidth: 2,
      borderLeftColor: theme.accent,
      paddingLeft: 1,
    },
    loopLineRight: {
      borderRightWidth: 2,
      borderRightColor: theme.accent,
      paddingRight: 1,
    },
    loopBtn: { alignItems: 'center', justifyContent: 'center', minHeight: Platform.OS === 'ios' || Platform.OS === 'android' ? 44 : 34, paddingHorizontal: 12, paddingVertical: 5, borderRadius: 8, borderWidth: 1, borderColor: theme.border, backgroundColor: theme.surface },
    loopBtnActive: { backgroundColor: theme.accent, borderColor: theme.accent },
    loopBtnText: { color: theme.text, fontSize: 12, fontWeight: "600" },
    loopBtnTextActive: { color: "#fff" },
    loopStatus: {
      flexWrap: "wrap", gap: 8,
      flexDirection: "row",
      alignItems: "center",
      justifyContent: "space-between",
      marginTop: 6,
      paddingHorizontal: 4,
    },
    loopStatusText: { color: theme.accent, fontSize: 12, fontWeight: "600" },
    loopClear: { color: theme.textMuted, fontSize: 12 },
    abTrack: {
      height: 26,
      borderRadius: 6,
      backgroundColor: theme.bg,
      borderWidth: StyleSheet.hairlineWidth,
      borderColor: theme.border,
    },
    abRange: {
      position: "absolute",
      top: 0,
      bottom: 0,
      backgroundColor: `${theme.accent}22`,
    },
    abHandle: {
      position: "absolute",
      top: -4,
      bottom: -4,
      width: 20,
      marginLeft: -10,
      flexDirection: "row",
      alignItems: "center",
      justifyContent: "center",
    },
    abLine: {
      width: 2,
      height: "100%",
    },
    abHandleB: {
      position: "absolute",
      top: -4,
      bottom: -4,
      width: 20,
      marginLeft: -10,
      flexDirection: "row",
      alignItems: "center",
      justifyContent: "center",
    },
    abArrow: {
      fontSize: 12,
      fontWeight: "700",
      paddingHorizontal: 2,
    },

    // Empty
    emptyState: {
      flex: 1,
      justifyContent: "center",
      alignItems: "center",
      paddingVertical: 60,
    },
    emptyText: {
      fontSize: 15,
      color: theme.textSecondary,
      textAlign: "center",
      lineHeight: 22,
    },

    // Player
    playerArea: { paddingTop: 4, paddingBottom: Platform.OS === 'android' ? 6 : 0 },
    progressRow: { flexDirection: 'row', alignItems: 'center', gap: 10 },
    progressBarTrack: { flex: 1, height: 28, justifyContent: 'center' },
    progressBarRail: {
      height: 5,
      backgroundColor: theme.border,
      borderRadius: 2.5,
      overflow: "hidden",
    },
    progressBarFill: {
      height: "100%",
      backgroundColor: theme.accent,
    },
    progressBarPlayhead: {
      position: "absolute",
      top: 8,
      width: 2,
      height: 12,
      borderRadius: 1,
      marginLeft: -1,
      backgroundColor: theme.text,
    },
    timeText: {
      color: theme.textSecondary,
      fontSize: 11,
      fontVariant: ["tabular-nums"],
    },
    controlsRow: { flexDirection: 'row', alignItems: 'center', gap: 12, paddingVertical: 2, zIndex: 30 },
    playerTools: { flex: 1 },
    playerToolsFixed: { zIndex: 30 },
    playerToolsContent: { flexGrow: 1, flexDirection: 'row', alignItems: 'center', justifyContent: 'flex-end', gap: 8 },
    speedOptions: { flexDirection: 'row', flexWrap: 'wrap', gap: 8 },
    speedBtn: { minWidth: 58, minHeight: Platform.OS === 'android' ? 48 : 44, alignItems: 'center', justifyContent: 'center', borderRadius: 10, backgroundColor: theme.surfaceHover },
    speedBtnActive: { backgroundColor: theme.accent },
    speedBtnText: { color: theme.text, fontSize: 12, fontWeight: "500" },
    speedBtnTextActive: { color: "#fff" },
    playBtn: { width: 44, height: 44, borderRadius: 22, backgroundColor: theme.accent, alignItems: 'center', justifyContent: 'center' },
    playBtnText: { color: '#fff', fontSize: 18 },

    // Toast
    toast: {
      position: "absolute",
      bottom: 24,
      alignSelf: "center",
      backgroundColor: theme.toastBg,
      paddingVertical: 10,
      paddingHorizontal: 22,
      borderRadius: 100,
      shadowColor: "#000",
      shadowOffset: { width: 0, height: 4 },
      shadowOpacity: 0.18,
      shadowRadius: 16,
      elevation: 8,
    },
    toastText: { color: theme.toastText, fontSize: 14, fontWeight: "500" },

    // Modal
    modalOverlay: {
      position: "absolute",
      top: 0,
      right: 0,
      bottom: 0,
      left: 0,
      backgroundColor: "rgba(0,0,0,0.4)",
      justifyContent: "center",
      alignItems: "center",
      padding: 32,
    },
    modalDismissArea: { position: "absolute", top: 0, right: 0, bottom: 0, left: 0 },
    modalBox: {
      width: "100%",
      maxWidth: 340,
      backgroundColor: theme.surface,
      borderRadius: 16,
      padding: 24,
      ...theme.cardShadow,
    },
    modalTitle: {
      fontSize: 17,
      fontWeight: "600",
      color: theme.text,
      marginBottom: 14,
      textAlign: "center",
    },
    modalInput: {
      backgroundColor: theme.bg,
      borderRadius: 10,
      paddingVertical: 10,
      paddingHorizontal: 14,
      fontSize: 15,
      color: theme.text,
      borderWidth: 1,
      borderColor: theme.border,
      marginBottom: 18,
    },
    modalActions: {
      flexDirection: "row",
      justifyContent: "center",
      gap: 10,
    },
    modalBtnPrimary: {
      backgroundColor: theme.accent,
      borderRadius: 10,
      paddingVertical: 10,
      paddingHorizontal: 28,
    },
    modalBtnPrimaryText: { color: "#fff", fontSize: 15, fontWeight: "600" },
    modalBtnGhost: {
      borderRadius: 10,
      paddingVertical: 10,
      paddingHorizontal: 28,
    },
    modalBtnGhostText: { color: theme.accent, fontSize: 15, fontWeight: "500" },

    subtitleViewerScrim: {
      position: "absolute",
      left: 0,
      right: 0,
      top: 0,
      bottom: 0,
      backgroundColor: "rgba(0,0,0,0.55)",
      alignItems: "center",
      justifyContent: "center",
      zIndex: 60,
    },
    subtitleViewerPanel: {
      width: "88%",
      maxHeight: "80%",
      borderRadius: 12,
      borderWidth: 1,
      borderColor: theme.border,
      backgroundColor: theme.surface,
      padding: 16,
      gap: 10,
    },
    subtitleViewerHeader: {
      flexDirection: "row",
      alignItems: "center",
      justifyContent: "space-between",
      gap: 10,
    },
    subtitleViewerTitle: { color: theme.text, fontSize: 15, fontWeight: "700", flexShrink: 1 },
    subtitleViewerActions: { flexDirection: "row", gap: 8 },
    subtitleViewerBtn: {
      borderWidth: 1,
      borderColor: theme.border,
      borderRadius: 8,
      paddingVertical: 6,
      paddingHorizontal: 12,
    },
    subtitleViewerBtnText: { color: theme.textSecondary, fontSize: 12 },
    subtitleViewerLoading: { color: theme.textMuted, fontSize: 13 },
    subtitleViewerScroll: { maxHeight: 420 },
    subtitleViewerCue: {
      paddingBottom: 10,
      marginBottom: 4,
      borderBottomWidth: StyleSheet.hairlineWidth,
      borderBottomColor: theme.border,
    },
    subtitleViewerTime: {
      color: theme.textMuted,
      fontSize: 11,
      fontVariant: ["tabular-nums"],
      marginBottom: 2,
    },
    subtitleViewerText: { color: theme.text, fontSize: 14, lineHeight: 21 },

    dropdownEngineMeta: {
      color: theme.textMuted,
      fontSize: 11,
      marginTop: 2,
    },
  });
}
