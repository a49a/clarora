import { learningDesign } from "../ui/learningDesign";
import { Details } from "../ui/Details";
import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ActivityIndicator,
  Alert,
  Animated,
  Dimensions,
  Easing,
  findNodeHandle,
  NativeModules,
  PanResponder,
  Platform,
  Pressable,
  ScrollView,
  StatusBar,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { DocumentPicker, FileSystem } from "../services/platform";
import { NativeVideoPlayer, videoControl } from "../services/nativeVideo";

import {
  getReviewCards,
  getDueReviewCards,
  getSetting,
  setSetting,
  upsertWords,
  replaceWords,
  deleteReviewCard,
  gradeReviewCard,
  type ReviewCard,
  type ReviewGrade,
} from "../data/database";
import { Audio, type AVPlaybackStatus, type SoundLike } from "../services/platform";
import { importFromTextFile, importFromDirectory } from "../data/importer";
import { MarkdownView } from "../ui/markdown";
import { useAIChat, useAIChatEntry } from "../ui/AIChatProvider";
import { useAppTheme } from "../ui/ThemeContext";

const SCROLL_SPEEDS = [
  { id: "slow", label: "慢", step: 40 },
  { id: "normal", label: "标准", step: 100 },
  { id: "fast", label: "快", step: 240 },
] as const;

// Markdown parsing is expensive; skip re-renders while scroll frames update state.
const MemoMarkdownView = React.memo(MarkdownView);

type ScrollSpeed = (typeof SCROLL_SPEEDS)[number]["id"];

const REVIEW_KINDS = [
  { id: "all", label: "全部" },
  { id: "word", label: "单词" },
  { id: "clip", label: "听力片段" },
  { id: "ai", label: "AI 问答" },
] as const;
type ReviewKindId = (typeof REVIEW_KINDS)[number]["id"] | "video";

// Video flashcards play through the macOS-only libmpv view, so the kind only
// exists on Mac builds.
const REVIEW_KINDS_ALL: ReadonlyArray<{ id: ReviewKindId; label: string }> =
  Platform.OS === "macos"
    ? [...REVIEW_KINDS, { id: "video", label: "视频片段" }]
    : REVIEW_KINDS;

export default function HomeScreen() {
  const { theme, scheme } = useAppTheme();

  const [cards, setCards] = useState<ReviewCard[]>([]);
  const [loading, setLoading] = useState(false);
  const [importing, setImporting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [currentIndex, setCurrentIndex] = useState(0);
  const [showMeaning, setShowMeaning] = useState(false);
  const { visible: chatOpen } = useAIChat();
  const [toast, setToast] = useState<string | null>(null);
  const [scrollSpeed, setScrollSpeed] = useState<ScrollSpeed>("normal");
  const [reviewKind, setReviewKindState] = useState<ReviewKindId>("all");
  // 计划复习：到期卡优先 + 新卡限量，翻面后按 SM-2 评分；自刷：原有的随机浏览。
  const [reviewMode, setReviewModeState] = useState<"scheduled" | "cram">("scheduled");
  const [newLimit, setNewLimit] = useState(20);
  const [dueInfo, setDueInfo] = useState<{ due: number; newTotal: number; todayNewCount: number } | null>(null);
  // The first deck load waits for the persisted review kind; otherwise the
  // default "all" load can finish after the restored kind's load and show
  // word cards under the 听力片段 tab.
  const [kindReady, setKindReady] = useState(false);
  const cardLoadIdRef = useRef(0);
  const [cardFontSize, setCardFontSize] = useState(22);

  // Restore the persisted review kind on mount.
  useEffect(() => {
    getSetting("review_kind")
      .then((value) => {
        if (value && REVIEW_KINDS_ALL.some((k) => k.id === value)) {
          setReviewKindState(value as ReviewKindId);
        }
      })
      .catch(() => {
        // database not ready — keep default
      });
    getSetting("review_mode")
      .then((value) => {
        if (value === "scheduled" || value === "cram") setReviewModeState(value);
      })
      .catch(() => {});
    getSetting("new_cards_per_day")
      .then((value) => {
        const size = value ? Number.parseInt(value, 10) : NaN;
        if (Number.isFinite(size) && size >= 0 && size <= 500) setNewLimit(size);
      })
      .catch(() => {})
      .finally(() => setKindReady(true));
  }, []);

  const setReviewKind = useCallback((id: ReviewKindId) => {
    setReviewKindState(id);
    setSetting("review_kind", id).catch(() => {
      // persistence is best-effort
    });
  }, []);

  const setReviewMode = useCallback((mode: "scheduled" | "cram") => {
    setReviewModeState(mode);
    setSetting("review_mode", mode).catch(() => {});
  }, []);

  useEffect(() => {
    getSetting("scroll_speed")
      .then((value) => {
        if (value && SCROLL_SPEEDS.some((speed) => speed.id === value)) {
          setScrollSpeed(value as ScrollSpeed);
        }
      })
      .catch(() => {});
  }, []);

  // Card text size applies to words, meanings, and listening clip captions.
  useEffect(() => {
    getSetting("flashcard_font_size")
      .then((value) => {
        const size = value ? Number.parseInt(value, 10) : NaN;
        if (Number.isFinite(size) && size >= 16 && size <= 34) setCardFontSize(size);
      })
      .catch(() => {});
  }, []);

  const currentCard = useMemo(() => cards[currentIndex], [cards, currentIndex]);
  const progress = cards.length ? ((currentIndex + 1) / cards.length) * 100 : 0;

  // Toast
  const showToast = useCallback((msg: string) => {
    setToast(msg);
    setTimeout(() => setToast(null), 2300);
  }, []);

  // Load flashcards (respects the review kind filter). Only the newest load
  // may commit: an older in-flight result must never overwrite a newer deck.
  const refreshCards = useCallback(async () => {
    const loadId = ++cardLoadIdRef.current;
    setLoading(true);
    setError(null);
    try {
      if (reviewMode === "scheduled") {
        const res = await getDueReviewCards(reviewKind, newLimit);
        if (loadId !== cardLoadIdRef.current) return;
        setDueInfo({ due: res.dueCount, newTotal: res.newTotal, todayNewCount: res.todayNewCount });
        setCards(res.cards);
      } else {
        const list = await getReviewCards(reviewKind);
        if (loadId !== cardLoadIdRef.current) return;
        setDueInfo(null);
        setCards(list);
      }
      setCurrentIndex(0);
      setShowMeaning(false);
    } catch (e: any) {
      if (loadId !== cardLoadIdRef.current) return;
      setError(String(e?.message ?? e));
    } finally {
      if (loadId === cardLoadIdRef.current) setLoading(false);
    }
  }, [reviewKind, reviewMode, newLimit]);

  useEffect(() => {
    if (!kindReady) return;
    refreshCards();
  }, [kindReady, refreshCards]);

  // Import text file
  const importWords = useCallback(async () => {
    setImporting(true);
    setError(null);
    try {
      const result = await DocumentPicker.getDocumentAsync({
        type: "text/*",
        copyToCacheDirectory: true,
      });

      if (result.canceled || !result.assets?.length) return;

      const fileUri = result.assets[0].uri;
      const parsed = await importFromTextFile(fileUri);
      const count = await upsertWords(parsed);
      showToast(`成功导入 ${count} 条单词`);
      await refreshCards();
    } catch (e: any) {
      setError(String(e?.message ?? e));
    } finally {
      setImporting(false);
    }
  }, [refreshCards, showToast]);

  // Import a directory tree: filename = word, file content = meaning.
  // The directory is the source of truth — replaces the whole deck so words
  // deleted from the directory (e.g. already-learned) disappear here too.
  const importDirectory = useCallback(async () => {
    setImporting(true);
    setError(null);
    try {
      const picked = await DocumentPicker.getDirectoryAsync();
      if (picked.canceled || !picked.path) return;
      const parsed = await importFromDirectory(picked.path);
      if (parsed.length === 0) {
        showToast("目录中没有可导入的单词文件");
        return;
      }
      const { imported, replaced } = await replaceWords(parsed);
      showToast(`已导入 ${imported} 条单词，替换原有 ${replaced} 条`);
      await refreshCards();
    } catch (e: any) {
      setError(String(e?.message ?? e));
    } finally {
      setImporting(false);
    }
  }, [refreshCards, showToast]);

  // ── Clip card audio playback ────────────────────────────────────────────
  const clipAudioRef = useRef<{ sound: SoundLike; uri: string } | null>(null);
  const clipRangeRef = useRef<{ startMs: number; endMs: number } | null>(null);
  // Native audio is a shared player. A quick swipe can otherwise let an older
  // card's asynchronous unload or seek run after the newer card has loaded.
  const clipPlaybackRequestRef = useRef(0);
  // Playback speed for clip audio (ref mirror keeps playClip stable).
  const [clipRate, setClipRate] = useState(1);
  const clipRateRef = useRef(1);
  clipRateRef.current = clipRate;
  // Live playhead position for the in-card track (absolute audio ms).
  const [clipPosMs, setClipPosMs] = useState(0);
  // Whether the clip audio is currently playing (drives the pause button).
  const [clipPlaying, setClipPlaying] = useState(false);
  const playClip = useCallback(
    async (audioUri: string, startMs: number, endMs: number) => {
      const requestId = ++clipPlaybackRequestRef.current;
      try {
        clipRangeRef.current = { startMs, endMs };
        let entry = clipAudioRef.current;
        if (!entry || entry.uri !== audioUri) {
          // Clear the shared reference before awaiting: a second swipe must
          // not try to unload the same native player a second time.
          clipAudioRef.current = null;
          await entry?.sound.unloadAsync();
          if (requestId !== clipPlaybackRequestRef.current) return;
          const { sound } = await Audio.Sound.createAsync(
            { uri: audioUri },
            { shouldPlay: false },
            (status: AVPlaybackStatus) => {
              if (!status.isLoaded) return;
              setClipPosMs(status.positionMillis);
              setClipPlaying(!!status.isPlaying);
              const range = clipRangeRef.current;
              if (range && status.positionMillis >= range.endMs) {
                sound.pauseAsync();
                sound.setPositionAsync(range.startMs);
              }
            }
          );
          if (requestId !== clipPlaybackRequestRef.current) {
            await sound.unloadAsync();
            return;
          }
          entry = { sound, uri: audioUri };
          clipAudioRef.current = entry;
        }
        clipRangeRef.current = { startMs, endMs };
        await entry.sound.setPositionAsync(startMs);
        if (requestId !== clipPlaybackRequestRef.current) return;
        await entry.sound.setRateAsync(clipRateRef.current, true);
        if (requestId !== clipPlaybackRequestRef.current) return;
        await entry.sound.playAsync();
      } catch {
        // playback errors are non-fatal during review
      }
    },
    []
  );

  // Re-apply the speed to a loaded clip when the setting changes.
  useEffect(() => {
    const entry = clipAudioRef.current;
    if (entry) void entry.sound.setRateAsync(clipRate, true).catch(() => {});
  }, [clipRate]);

  // Auto-play the captured segment when its card becomes current.
  useEffect(() => {
    if (currentCard?.kind === "clip") {
      void playClip(currentCard.audioUri, currentCard.startMs, currentCard.endMs);
    }
  }, [currentCard, playClip]);

  // Reset the playhead whenever a different card appears. Switching between
  // clip cards is owned by playClip above; issuing a second asynchronous pause
  // here races with its new load because the native player is shared.
  useEffect(() => {
    setClipPosMs(0);
    setClipPlaying(false);
  }, [currentCard?.id]);

  // Leaving clip cards still stops their audio, without competing with a
  // clip-to-clip transition.
  useEffect(() => {
    if (currentCard?.kind === "clip") return;
    ++clipPlaybackRequestRef.current;
    void clipAudioRef.current?.sound.pauseAsync().catch(() => {});
  }, [currentCard?.kind]);

  useEffect(
    () => () => {
      clipAudioRef.current?.sound.unloadAsync();
      clipAudioRef.current = null;
    },
    []
  );

  // ── Video clip card playback (macOS) ────────────────────────────────────
  // The extracted clip is a self-contained file: play it from the top and
  // loop on end (keep-open parks the player there; JS seeks back to 0).
  const videoPlayerRef = useRef<any>(null);
  const [videoClipPlaying, setVideoClipPlaying] = useState(false);
  const [videoPosMs, setVideoPosMs] = useState(0);
  const [videoDurMs, setVideoDurMs] = useState(0);
  const [videoRate, setVideoRate] = useState(1);

  const handleVideoProgress = useCallback(
    (e: { nativeEvent: { positionMs: number; durationMs: number } }) => {
      setVideoPosMs(e.nativeEvent.positionMs);
      if (e.nativeEvent.durationMs > 0) setVideoDurMs(e.nativeEvent.durationMs);
    },
    []
  );

  const replayVideoClip = useCallback(() => {
    const tag = findNodeHandle(videoPlayerRef.current);
    if (tag != null) videoControl()?.seek?.(tag, 0);
    setVideoClipPlaying(true);
  }, []);

  const handleVideoClipEnd = useCallback(() => {
    replayVideoClip();
  }, [replayVideoClip]);

  const toggleVideoClipPlayback = useCallback(() => {
    setVideoClipPlaying((current) => !current);
  }, []);

  const seekVideoClipWord = useCallback(
    (token: { text: string; offset: number }) => {
      const total = Math.max(1, currentCard?.kind === "video" ? currentCard.front.length : 1);
      const ms = Math.round((token.offset / total) * Math.max(1, videoDurMs));
      const tag = findNodeHandle(videoPlayerRef.current);
      if (tag != null) videoControl()?.seek?.(tag, ms);
      setVideoClipPlaying(true);
    },
    [currentCard, videoDurMs]
  );

  const handleVideoTrackSeek = useCallback((relMs: number) => {
    const tag = findNodeHandle(videoPlayerRef.current);
    if (tag != null) videoControl()?.seek?.(tag, Math.max(0, relMs));
    setVideoClipPlaying(true);
  }, []);

  // Auto-play a video clip when its card becomes current.
  useEffect(() => {
    if (currentCard?.kind === "video") {
      setVideoPosMs(0);
      setVideoClipPlaying(true);
    }
  }, [currentCard?.id]);

  // Flip card. Listening cards don't flip — pressing/space replays the
  // full clip instead. Video cards behave the same way.
  const toggleMeaning = useCallback(() => {
    const card = currentCard;
    if (card?.kind === "clip") {
      void playClip(card.audioUri, card.startMs, card.endMs);
      return;
    }
    if (card?.kind === "video") {
      replayVideoClip();
      return;
    }
    setShowMeaning((v) => !v);
  }, [currentCard, playClip, replayVideoClip]);

  // Play from a clicked word to the end of the clip (char-offset
  // interpolation, same linear model as the listening page).
  const playClipWord = useCallback(
    (card: Extract<ReviewCard, { kind: "clip" }>, token: { text: string; offset: number }) => {
      const total = Math.max(1, card.front.length);
      const dur = Math.max(1, card.endMs - card.startMs);
      const s = Math.round(card.startMs + (token.offset / total) * dur);
      void playClip(card.audioUri, s, card.endMs);
    },
    [playClip]
  );

  // Click/drag on the track: play from that point to the end of the clip.
  const handleClipTrackSeek = useCallback(
    (relMs: number) => {
      if (currentCard?.kind !== "clip") return;
      const card = currentCard;
      const dur = Math.max(1, card.endMs - card.startMs);
      const abs = Math.round(card.startMs + Math.max(0, Math.min(dur, relMs)));
      void playClip(card.audioUri, abs, card.endMs);
    },
    [currentCard, playClip]
  );

  const replayClip = useCallback(() => {
    if (currentCard?.kind !== "clip") return;
    void playClip(currentCard.audioUri, currentCard.startMs, currentCard.endMs);
  }, [currentCard, playClip]);

  // Pause/resume the clip; falls back to starting playback if no audio
  // has been loaded yet.
  const toggleClipPlayback = useCallback(() => {
    const entry = clipAudioRef.current;
    if (!entry) {
      replayClip();
      return;
    }
    void (clipPlaying
      ? entry.sound.pauseAsync()
      : entry.sound.playAsync()
    ).catch(() => {});
  }, [clipPlaying, replayClip]);

  const deleteCurrentCard = useCallback(async () => {
    const card = currentCard;
    if (!card) return;
    try {
      // Stop playback so a deleted clip doesn't keep looping.
      try {
        await clipAudioRef.current?.sound.pauseAsync();
      } catch {}
      await deleteReviewCard(card);
      // A video card owns its extracted mp4; remove it with the card.
      if (card.kind === "video") {
        try {
          await FileSystem.deleteAsync(card.videoUri);
        } catch {}
      }
      const next = cards.filter((_, i) => i !== currentIndex);
      setCards(next);
      setCurrentIndex(next.length === 0 ? 0 : currentIndex >= next.length ? 0 : currentIndex);
      setShowMeaning(false);
      showToast(
        card.kind === "clip"
          ? "已删除听力片段"
          : card.kind === "video"
          ? "已删除视频片段"
          : card.kind === "ai"
          ? "已删除 AI 问答"
          : "已删除单词"
      );
    } catch (e: any) {
      setError(String(e?.message ?? e));
    }
  }, [currentCard, cards, currentIndex, showToast]);

  // Delete the current card (word or clip) from the library and move on.
  // Always confirms first — deletion is irreversible.
  const confirmDeleteCurrentCard = useCallback(() => {
    const card = currentCard;
    if (!card) return;
    const kindLabel =
      card.kind === "clip"
        ? "听力片段"
        : card.kind === "video"
        ? "视频片段"
        : card.kind === "ai"
        ? "AI 问答"
        : "单词";
    Alert.alert(`删除${kindLabel}？`, "删除后不可恢复。", [
      { text: "取消", style: "cancel" },
      {
        text: "删除",
        style: "destructive",
        onPress: () => {
          void deleteCurrentCard();
        },
      },
    ]);
  }, [currentCard, deleteCurrentCard]);

  // 计划复习评分：写 SM-2 调度；「忘了」把卡挪到队尾当次再练，其余出队。
  const gradeCurrentCard = useCallback(
    async (grade: ReviewGrade) => {
      const card = currentCard;
      if (!card) return;
      try {
        const label = await gradeReviewCard(card, grade);
        const next = cards.filter((_, i) => i !== currentIndex);
        if (grade === "again") next.push(card);
        setCards(next);
        setCurrentIndex(next.length === 0 ? 0 : currentIndex >= next.length ? 0 : currentIndex);
        setShowMeaning(false);
        showToast(grade === "again" ? `再练一次 · ${label}` : `已安排 ${label}`);
      } catch (e: any) {
        setError(String(e?.message ?? e));
      }
    },
    [currentCard, cards, currentIndex, showToast]
  );


  // Card transition: the incoming card slides in from the swipe direction
  // (next → from the right, previous → from the left) with a quick fade, so
  // it is obvious on touch devices that the card changed.
  const cardSlide = useRef(new Animated.Value(0)).current;
  const cardFade = useRef(new Animated.Value(1)).current;
  const playCardTransition = useCallback(
    (direction: number) => {
      if (direction === 0) {
        cardSlide.setValue(0);
        cardFade.setValue(0.4);
      } else {
        cardSlide.setValue(direction * 90);
        cardFade.setValue(0);
      }
      Animated.parallel([
        Animated.timing(cardSlide, {
          toValue: 0,
          duration: 200,
          easing: Easing.out(Easing.cubic),
          useNativeDriver: false,
        }),
        Animated.timing(cardFade, {
          toValue: 1,
          duration: 200,
          useNativeDriver: false,
        }),
      ]).start();
    },
    [cardFade, cardSlide]
  );

  // Navigation
  const nextCard = useCallback(() => {
    if (!cards.length) return;
    playCardTransition(1);
    setCurrentIndex((prev) => (prev + 1) % cards.length);
    setShowMeaning(false);
  }, [cards.length, playCardTransition]);

  const prevCard = useCallback(() => {
    if (!cards.length) return;
    playCardTransition(-1);
    setCurrentIndex((prev) => (prev - 1 + cards.length) % cards.length);
    setShowMeaning(false);
  }, [cards.length, playCardTransition]);

  // Swipe detection
  const touchStartX = useRef(0);
  const touchStartY = useRef(0);
  const handleTouchStart = (x: number) => {
    touchStartX.current = x;
  };
  const handleTouchEnd = (x: number) => {
    const delta = x - touchStartX.current;
    if (Math.abs(delta) > 60) {
      if (delta > 0) prevCard();
      else nextCard();
    }
  };
  // The back face uses a plain View so the nested ScrollView retains native
  // trackpad scrolling. Touch events still bubble to it, letting a stationary
  // click flip the card without taking ownership of drag/scroll gestures.
  const handleMeaningTouchStart = (x: number, y: number) => {
    touchStartX.current = x;
    touchStartY.current = y;
  };
  const handleMeaningTouchEnd = (x: number, y: number) => {
    const deltaX = x - touchStartX.current;
    const deltaY = y - touchStartY.current;
    if (Math.abs(deltaX) <= 8 && Math.abs(deltaY) <= 8) {
      toggleMeaning();
    }
  };

  // Keyboard control (macOS): Space flips, ←/→ navigate, ↑/↓ scroll the card back.
  // Keep the native ScrollView uncontrolled so its trackpad momentum remains
  // intact; use its ref only for keyboard-initiated scrolling.
  const meaningScrollRef = useRef<ScrollView>(null);
  const [meaningRequestedOffset, setMeaningRequestedOffset] = useState({ x: 0, y: 0 });
  const meaningOffsetRef = useRef(0);
  const meaningContentH = useRef(0);
  const meaningViewportH = useRef(0);
  const selectedScrollSpeed = SCROLL_SPEEDS.find((speed) => speed.id === scrollSpeed) ?? SCROLL_SPEEDS[1];
  const clampOffset = useCallback((value: number) => {
    const content = meaningContentH.current;
    const viewport = meaningViewportH.current;
    // Measurements arrive after the back face mounts. Do not turn an early
    // keyboard press into zero while the native view is still laying out.
    if (content <= 0 || viewport <= 0) return Math.max(0, value);
    const max = Math.max(0, content - viewport);
    return Math.min(Math.max(0, value), max);
  }, []);

  const setMeaningScrollPosition = useCallback((value: number) => {
    const next = clampOffset(value);
    meaningOffsetRef.current = next;
    // Updating contentOffset is the reliable macOS path for keyboard-driven
    // scrolling. It changes only for key presses, so trackpad scrolling stays
    // native and uncontrolled between requests.
    setMeaningRequestedOffset({ x: 0, y: next });
    meaningScrollRef.current?.scrollTo({ x: 0, y: next, animated: false });
  }, [clampOffset]);

  const scrollMeaning = useCallback((direction: 1 | -1) => {
    // A small step lets macOS key-repeat produce a slow, continuous scroll
    // instead of jumping a large distance on every repeat event.
    setMeaningScrollPosition(meaningOffsetRef.current + selectedScrollSpeed.step * direction);
  }, [selectedScrollSpeed.step, setMeaningScrollPosition]);

  // Reset the scroll position whenever the card or face changes.
  useEffect(() => {
    meaningOffsetRef.current = 0;
    setMeaningRequestedOffset({ x: 0, y: 0 });
    meaningScrollRef.current?.scrollTo({ x: 0, y: 0, animated: false });
  }, [currentCard?.id, showMeaning]);
  const handleKey = useCallback(
    (key: string) => {
      if (chatOpen) return;
      // 计划复习：翻面后 1/2/3 对应 忘了/模糊/认识。
      if (
        (key === "1" || key === "2" || key === "3") &&
        reviewMode === "scheduled" &&
        showMeaning &&
        currentCard
      ) {
        void gradeCurrentCard(key === "1" ? "again" : key === "2" ? "hard" : "good");
        return;
      }
      if (key === " " || key === "Space" || key === "Spacebar") {
        // A listening clip is already an audio-first card: Space should pause
        // and resume it instead of replaying the segment from its beginning.
        if (currentCard?.kind === "clip") {
          toggleClipPlayback();
        } else if (currentCard?.kind === "video") {
          toggleVideoClipPlayback();
        } else {
          toggleMeaning();
        }
      } else if (key === "Enter" || key === "Return") {
        toggleMeaning();
      } else if (key.toLowerCase() === "p") {
        if (currentCard?.kind === "clip") toggleClipPlayback();
        else if (currentCard?.kind === "video") toggleVideoClipPlayback();
      } else if (key === "ArrowRight" || key === "Right") {
        nextCard();
      } else if (key === "ArrowLeft" || key === "Left") {
        prevCard();
      } else if (key === "ArrowDown" || key === "Down") {
        // On the word face, ↓ flips to the meaning first; scroll from there.
        if (!showMeaning) {
          toggleMeaning();
          return;
        }
        scrollMeaning(1);
      } else if (key === "ArrowUp" || key === "Up") {
        if (!showMeaning) return;
        scrollMeaning(-1);
      }
    },
    [chatOpen, currentCard, toggleClipPlayback, toggleMeaning, nextCard, prevCard, scrollMeaning, showMeaning, reviewMode, gradeCurrentCard]
  );

  // Native NSEvent monitor: keys are queued natively and pulled via promise
  // (promise resolvers are main-thread safe; direct callbacks are not).
  useEffect(() => {
    if (chatOpen) return;
    const keyboard = NativeModules.RNKeyboard as
      | {
          startListening: () => void;
          getNextKey: () => Promise<string | null>;
          stopListening: () => void;
        }
      | undefined;
    if (!keyboard?.getNextKey) return;

    let cancelled = false;
    keyboard.startListening();
    const pump = async () => {
      while (!cancelled) {
        // Promise methods auto-inject resolve/reject — call with business args only.
        const key = await keyboard.getNextKey();
        if (cancelled || key == null) break;
        handleKey(key);
      }
    };
    pump();
    return () => {
      cancelled = true;
      keyboard.stopListening();
    };
  }, [handleKey, chatOpen]);

  const handleKeyDown = useCallback(
    (e: any) => {
      handleKey(e?.nativeEvent?.key ?? "");
    },
    [handleKey]
  );

  useAIChatEntry(currentCard ? [
    currentCard.front,
    currentCard.kind === "ai" ? currentCard.contextText : "",
    currentCard.kind === "clip" || currentCard.kind === "video" ? currentCard.zhText : "",
    showMeaning ? currentCard.back : "",
  ].filter(Boolean).join("\n\n") : "", "闪卡复习", () => {
    ++clipPlaybackRequestRef.current;
    void clipAudioRef.current?.sound.pauseAsync().catch(() => {});
    setVideoClipPlaying(false);
  });

  // react-native-macos keyboard props (typing not fully covered upstream)
  const keyboardProps: any = Platform.OS === 'windows' ? { tabIndex: 0 } : { tabIndex: 0, onKeyDown: handleKeyDown };

  const styles = makeStyles(theme);
  // A selectable macOS Text must be the actual mouse hit target. Keeping the
  // back face inside Pressable prevents AppKit's NSTextView from owning the
  // drag gesture, even when Pressable itself has no onPress handler.
  const CardInteractionContainer: any = showMeaning ? View : Pressable;

  return (
    <View style={styles.safeArea}>
      <StatusBar barStyle={scheme === "dark" ? "light-content" : "dark-content"} />
      <View
        style={styles.container}
        pointerEvents={chatOpen ? "none" : "auto"}
        accessibilityElementsHidden={chatOpen}
        importantForAccessibility={chatOpen ? "no-hide-descendants" : "auto"}
      >
        {/* Header */}
        <View style={styles.header}>
          <Text style={styles.eyebrow}>FLASHCARD SESSION</Text>
          <Text style={styles.title}>闪卡复习</Text>
          <Text style={styles.subtitle}>先回忆，再揭晓。</Text>
        </View>

        {/* Review kind selector */}
        <Details title={`${reviewMode === 'scheduled' ? '计划复习' : '自由浏览'} · ${REVIEW_KINDS_ALL.find(kind => kind.id === reviewKind)?.label ?? '全部'} · 学习选项`}>
        <View style={styles.kindRow}>
          {REVIEW_KINDS_ALL.map((k) => {
            const selected = reviewKind === k.id;
            return (
              <Pressable
                key={k.id}
                style={[styles.kindBtn, selected && styles.kindBtnActive]}
                onPress={() => setReviewKind(k.id)}
              >
                <Text
                  style={[
                    styles.kindBtnText,
                    selected && styles.kindBtnTextActive,
                  ]}
                >
                  {k.label}
                </Text>
              </Pressable>
            );
          })}
          <View style={styles.modeSpacer} />
          <Pressable
            style={[styles.modeBtn, reviewMode === "scheduled" && styles.modeBtnActive]}
            onPress={() => setReviewMode("scheduled")}
          >
            <Text style={[styles.modeBtnText, reviewMode === "scheduled" && styles.modeBtnTextActive]}>
              计划复习
            </Text>
          </Pressable>
          <Pressable
            style={[styles.modeBtn, reviewMode === "cram" && styles.modeBtnActive]}
            onPress={() => setReviewMode("cram")}
          >
            <Text style={[styles.modeBtnText, reviewMode === "cram" && styles.modeBtnTextActive]}>
              自刷
            </Text>
          </Pressable>
        </View>
        {reviewMode === "scheduled" && dueInfo && (
          <Text style={styles.dueInfoText}>
            到期 {dueInfo.due} · 新卡今日已学 {dueInfo.todayNewCount}/{newLimit}
          </Text>
        )}

        {/* Toolbar — word import stays desktop-only; phones sync from the server. */}
        <View style={styles.toolbar}>
          {Platform.OS !== "android" && (
            <>
          <Pressable
            style={({ pressed }) => [
              styles.btn,
              styles.btnSecondary,
              pressed && styles.btnPressed,
              importing && styles.btnDisabled,
            ]}
            onPress={importWords}
            disabled={importing}
          >
            <Text style={styles.btnSecondaryText}>
              {importing ? "导入中..." : "导入文本"}
            </Text>
          </Pressable>
          {(Platform.OS === "macos" || Platform.OS === "windows") && <Pressable
            style={({ pressed }) => [
              styles.btn,
              styles.btnSecondary,
              pressed && styles.btnPressed,
              importing && styles.btnDisabled,
            ]}
            onPress={importDirectory}
            disabled={importing}
          >
            <Text style={styles.btnSecondaryText}>导入目录</Text>
          </Pressable>}
            </>
          )}
          <Pressable
            style={({ pressed }) => [
              styles.btn,
              styles.btnGhost,
              pressed && styles.btnGhostPressed,
              loading && styles.btnDisabled,
            ]}
            onPress={refreshCards}
            disabled={loading}
          >
            {loading ? (
              <ActivityIndicator size="small" color={theme.accent} />
            ) : (
              <Text style={[styles.btnGhostText, { color: theme.accent }]}>
                刷新卡组
              </Text>
            )}
          </Pressable>
        </View>

        </Details>
        {/* Error */}
        {error && (
          <View style={styles.errorBanner}>
            <Text style={styles.errorText}>{error}</Text>
          </View>
        )}

        {/* Card area */}
        {!currentCard ? (
          <View style={styles.emptyState}>
            <Text style={styles.emptyIndex}>00</Text>
            <Text style={styles.emptyText}>
              {reviewMode === "scheduled" && (!dueInfo || dueInfo.due === 0) && reviewKind === "all"
                ? "今日到期已完成 🎉\n可切到「自刷」自由浏览，或稍后再来"
                : reviewMode === "scheduled"
                ? "该类别暂时没有到期的卡片 🎉\n可切到「自刷」自由浏览"
                : reviewKind === "clip"
                ? "还没有收藏的听力片段\n听力页选段循环后点「⭐ 收藏到闪卡」"
                : reviewKind === "ai"
                ? "还没有 AI 问答卡\n听力页选中字幕后向 AI 提问，回答面板里点「存为卡片」"
                : Platform.OS === "android"
                  ? "还没有可复习的单词\n请先在 Mac 端导入词库，再到设置里同步下载。"
                  : "还没有可复习的单词\n从文本或目录开始建立你的词库。"}
            </Text>
          </View>
        ) : (
          <View style={styles.cardArea} {...keyboardProps}>
            <CardInteractionContainer
              style={showMeaning
                ? styles.cardPressable
                : ({ pressed }: { pressed: boolean }) => [
                    styles.cardPressable,
                    pressed && styles.cardPressed,
                  ]}
              {...(showMeaning
                ? {
                    onTouchStart: (e: any) =>
                      handleMeaningTouchStart(e.nativeEvent.pageX, e.nativeEvent.pageY),
                    onTouchEnd: (e: any) =>
                      handleMeaningTouchEnd(e.nativeEvent.pageX, e.nativeEvent.pageY),
                  }
                : {
                    onPress: toggleMeaning,
                    onTouchStart: (e: any) => handleTouchStart(e.nativeEvent.pageX),
                    onTouchEnd: (e: any) => handleTouchEnd(e.nativeEvent.pageX),
                  })}
            >
              <Animated.View
                style={[
                  styles.card,
                  showMeaning && styles.cardMeaning,
                  { transform: [{ translateX: cardSlide }], opacity: cardFade },
                ]}
              >
                {showMeaning ? (
                  <View style={styles.meaningScrollContainer}>
                    <ScrollView
                      ref={meaningScrollRef}
                      style={styles.meaningScroll}
                      contentContainerStyle={styles.meaningContent}
                      contentOffset={meaningRequestedOffset}
                      showsVerticalScrollIndicator={false}
                      scrollEventThrottle={16}
                      onScroll={(e) => {
                        const content = e.nativeEvent.contentSize.height;
                        const viewport = e.nativeEvent.layoutMeasurement.height;
                        meaningContentH.current = content;
                        meaningViewportH.current = viewport;
                        const y = clampOffset(e.nativeEvent.contentOffset.y);
                        meaningOffsetRef.current = y;
                      }}
                      onLayout={(e) => {
                        const viewport = e.nativeEvent.layout.height;
                        meaningViewportH.current = viewport;
                      }}
                      onContentSizeChange={(_, contentHeight) => {
                        meaningContentH.current = contentHeight;
                      }}
                    >
                      <View style={styles.meaningTextWrap}>
                        <MemoMarkdownView
                          text={currentCard.back}
                          theme={theme}
                          baseFontSize={cardFontSize}
                          singleText
                        />
                      </View>
                    </ScrollView>
                  </View>
                ) : currentCard.kind === "clip" ? (
                  <ScrollView
                    style={styles.clipScroll}
                    contentContainerStyle={styles.clipScrollContent}
                    showsVerticalScrollIndicator={false}
                  >
                  <View style={styles.clipWordsWrap}>
                    {tokenizeClipText(currentCard.front).map((tok, tokIndex) =>
                      tok.isWord ? (
                        <Pressable
                          key={tokIndex}
                          onPress={() => playClipWord(currentCard, tok)}
                          style={({ pressed }) => pressed && styles.clipWordPressed}
                        >
                          <Text
                            style={[
                              styles.clipWord,
                              { fontSize: cardFontSize, lineHeight: cardFontSize + 10 },
                            ]}
                          >
                            {tok.text}
                          </Text>
                        </Pressable>
                      ) : (
                        <Text key={tokIndex} style={[styles.clipWordGap, { fontSize: cardFontSize }]}>
                          {tok.text}
                        </Text>
                      )
                    )}
                  </View>
                    {/* Chinese translation subtitle under the clip text */}
                    {currentCard.zhText ? (
                      <Text
                        style={[
                          styles.clipZhText,
                          {
                            fontSize: Math.max(14, cardFontSize - 5),
                            lineHeight: Math.max(21, cardFontSize + 3),
                          },
                        ]}
                      >
                        {currentCard.zhText}
                      </Text>
                    ) : null}
                    {/* In-card audio track: click/drag anywhere to play
                        from that point; fill + line follow the playhead. */}
                    <ClipTrack
                      clipDurMs={Math.max(1, currentCard.endMs - currentCard.startMs)}
                      posRelMs={clipPosMs - currentCard.startMs}
                      onSeek={handleClipTrackSeek}
                    />
                    <View style={styles.clipActionsRow}>
                      <Pressable
                        style={[
                          styles.clipActionBtn,
                          clipPlaying && styles.clipActionBtnActive,
                        ]}
                        onPress={toggleClipPlayback}
                        hitSlop={6}
                      >
                        <Text
                          style={[
                            styles.clipActionText,
                            clipPlaying && styles.clipActionTextActive,
                          ]}
                        >
                          {clipPlaying ? "⏸ 暂停" : "▶ 播放"}
                        </Text>
                      </Pressable>
                      <Pressable
                        style={styles.clipActionBtn}
                        onPress={replayClip}
                        hitSlop={6}
                      >
                        <Text style={styles.clipActionText}>🔊 重播整段</Text>
                      </Pressable>
                    </View>
                    <View style={styles.clipRateRow}>
                      {[0.6, 0.7, 0.8, 1.0].map((rate) => (
                        <Pressable
                          key={rate}
                          style={[
                            styles.clipRateBtn,
                            clipRate === rate && styles.clipRateBtnActive,
                          ]}
                          onPress={() => setClipRate(rate)}
                        >
                          <Text
                            style={[
                              styles.clipRateText,
                              clipRate === rate && styles.clipRateTextActive,
                            ]}
                          >
                            {rate}x
                          </Text>
                        </Pressable>
                      ))}
                    </View>
                  </ScrollView>
                ) : currentCard.kind === "video" ? (
                  <ScrollView
                    style={styles.clipScroll}
                    contentContainerStyle={styles.clipScrollContent}
                    showsVerticalScrollIndicator={false}
                  >
                    {NativeVideoPlayer ? (
                      <View style={styles.videoClipWrap}>
                        <NativeVideoPlayer
                          ref={videoPlayerRef}
                          style={styles.videoClipPlayer}
                          src={currentCard.videoUri}
                          playing={videoClipPlaying}
                          rate={videoRate}
                          onProgress={handleVideoProgress}
                          onEnd={handleVideoClipEnd}
                        />
                      </View>
                    ) : null}
                    <View style={styles.clipWordsWrap}>
                      {tokenizeClipText(currentCard.front).map((tok, tokIndex) =>
                        tok.isWord ? (
                          <Pressable
                            key={tokIndex}
                            onPress={() => seekVideoClipWord(tok)}
                            style={({ pressed }) => pressed && styles.clipWordPressed}
                          >
                            <Text
                              style={[
                                styles.clipWord,
                                { fontSize: cardFontSize, lineHeight: cardFontSize + 10 },
                              ]}
                            >
                              {tok.text}
                            </Text>
                          </Pressable>
                        ) : (
                          <Text key={tokIndex} style={[styles.clipWordGap, { fontSize: cardFontSize }]}>
                            {tok.text}
                          </Text>
                        )
                      )}
                      {tokenizeClipText(currentCard.front).length === 0 ? (
                        <Text style={[styles.clipZhText, { fontSize: 13 }]}>这段视频没有添加文本</Text>
                      ) : null}
                    </View>
                    {currentCard.zhText ? (
                      <Text
                        style={[
                          styles.clipZhText,
                          {
                            fontSize: Math.max(14, cardFontSize - 5),
                            lineHeight: Math.max(21, cardFontSize + 3),
                          },
                        ]}
                      >
                        {currentCard.zhText}
                      </Text>
                    ) : null}
                    <ClipTrack
                      clipDurMs={Math.max(1, videoDurMs)}
                      posRelMs={videoPosMs}
                      onSeek={handleVideoTrackSeek}
                    />
                    <View style={styles.clipActionsRow}>
                      <Pressable
                        style={[styles.clipActionBtn, videoClipPlaying && styles.clipActionBtnActive]}
                        onPress={toggleVideoClipPlayback}
                        hitSlop={6}
                      >
                        <Text style={[styles.clipActionText, videoClipPlaying && styles.clipActionTextActive]}>
                          {videoClipPlaying ? "⏸ 暂停" : "▶ 播放"}
                        </Text>
                      </Pressable>
                      <Pressable style={styles.clipActionBtn} onPress={replayVideoClip} hitSlop={6}>
                        <Text style={styles.clipActionText}>🔊 重播整段</Text>
                      </Pressable>
                    </View>
                    <View style={styles.clipRateRow}>
                      {[0.6, 0.7, 0.8, 1.0].map((rate) => (
                        <Pressable
                          key={rate}
                          style={[styles.clipRateBtn, videoRate === rate && styles.clipRateBtnActive]}
                          onPress={() => setVideoRate(rate)}
                        >
                          <Text style={[styles.clipRateText, videoRate === rate && styles.clipRateTextActive]}>
                            {rate}x
                          </Text>
                        </Pressable>
                      ))}
                    </View>
                  </ScrollView>
                ) : currentCard.kind === "ai" ? (
                  <ScrollView
                    style={styles.clipScroll}
                    contentContainerStyle={styles.clipScrollContent}
                    showsVerticalScrollIndicator={false}
                  >
                    <Text style={styles.aiCardTag}>AI 问答</Text>
                    <Text
                      style={[
                        styles.aiCardQuestion,
                        { fontSize: cardFontSize, lineHeight: cardFontSize + 10 },
                      ]}
                    >
                      {currentCard.front}
                    </Text>
                    {currentCard.contextText ? (
                      <Text
                        style={[
                          styles.aiCardContext,
                          {
                            fontSize: Math.max(14, cardFontSize - 5),
                            lineHeight: Math.max(21, cardFontSize + 3),
                          },
                        ]}
                      >
                        {currentCard.contextText}
                      </Text>
                    ) : null}
                    <Text style={styles.aiCardFlipHint}>点击翻面查看 AI 回答</Text>
                  </ScrollView>
                ) : (
                  <Text style={[styles.wordFace, { fontSize: cardFontSize * 2.1 }]}>
                    {currentCard.front}
                  </Text>
                )}
                {showMeaning && reviewMode === "scheduled" && (
                  <View style={styles.gradeRow}>
                    <Pressable
                      style={[styles.gradeBtn, styles.gradeAgain]}
                      onPress={() => void gradeCurrentCard("again")}
                    >
                      <Text style={styles.gradeBtnText}>忘了</Text>
                    </Pressable>
                    <Pressable
                      style={[styles.gradeBtn, styles.gradeHard]}
                      onPress={() => void gradeCurrentCard("hard")}
                    >
                      <Text style={styles.gradeBtnText}>模糊</Text>
                    </Pressable>
                    <Pressable
                      style={[styles.gradeBtn, styles.gradeGood]}
                      onPress={() => void gradeCurrentCard("good")}
                    >
                      <Text style={styles.gradeBtnText}>认识</Text>
                    </Pressable>
                  </View>
                )}
                <Text style={styles.cardHint}>
                  {currentCard.kind === "clip"
                    ? "点单词或音轨任意位置播放 · ←/→ 切换卡片 · 学会的点 🗑 删除"
                    : currentCard.kind === "video"
                    ? "点单词或进度条定位 · 空格 播放/暂停 · ←/→ 切换卡片 · 学会的点 🗑 删除"
                    : currentCard.kind === "ai"
                    ? showMeaning
                      ? `空格 返回问题 · ←/→ 切换卡片 · ↑/↓ ${selectedScrollSpeed.label}速滚动（可在设置中调整）`
                      : "空格或点击 查看回答 · ←/→ 切换卡片 · 学会的点 🗑 删除"
                    : showMeaning
                    ? reviewMode === "scheduled"
                      ? `1 忘了 · 2 模糊 · 3 认识 评分 · ←/→ 切换 · ↑/↓ ${selectedScrollSpeed.label}速滚动`
                      : `空格 返回单词 · ←/→ 切换卡片 · ↑/↓ ${selectedScrollSpeed.label}速滚动（可在设置中调整）`
                    : reviewMode === "scheduled"
                    ? "空格或↓ 翻面后评分 · ←/→ 切换卡片"
                    : "空格或↓ 查看释义 · ←/→ 切换卡片"}
                </Text>
              </Animated.View>
            </CardInteractionContainer>

            {/* Progress */}
            <View style={styles.cardFooter}>
              <Text style={styles.progressText}>
                {currentIndex + 1} / {cards.length}
              </Text>
              <View style={styles.progressTrack}>
                <View
                  style={[
                    styles.progressFill,
                    { width: `${progress}%` },
                  ]}
                />
              </View>
            </View>

            {/* Nav buttons */}
            <View style={styles.navButtons}>
              <Pressable
                style={({ pressed }) => [
                  styles.btn,
                  styles.btnSecondary,
                  styles.navBtn,
                  pressed && styles.btnPressed,
                ]}
                onPress={prevCard}
              >
                <Text style={styles.btnSecondaryText}>← 上一张</Text>
              </Pressable>
              <Pressable
                style={({ pressed }) => [
                  styles.btn,
                  styles.btnPrimary,
                  styles.navBtn,
                  pressed && styles.btnPrimaryPressed,
                ]}
                onPress={nextCard}
              >
                <Text style={styles.btnPrimaryText}>下一张 →</Text>
              </Pressable>
              <Pressable
                style={({ pressed }) => [
                  styles.btn,
                  styles.btnSecondary,
                  styles.navBtn,
                  styles.deleteBtn,
                  pressed && styles.btnPressed,
                ]}
                onPress={confirmDeleteCurrentCard}
              >
                <Text style={[styles.btnSecondaryText, styles.deleteBtnText]}>
                  🗑 删除
                </Text>
              </Pressable>
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
  );
}

/** Split clip text into word/non-word tokens with character offsets. */
function tokenizeClipText(
  text: string
): Array<{ text: string; isWord: boolean; offset: number }> {
  const tokens: Array<{ text: string; isWord: boolean; offset: number }> = [];
  const wordPattern = /\S+/g;
  let cursor = 0;
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

/** Audio track for a clip card: click/drag anywhere to play from that
 *  point; fill + playhead line show progress. Coordinates use pageX and
 *  the track's absolute window position — locationX is relative to the
 *  view under the touch and unreliable here. */
function ClipTrack(props: {
  clipDurMs: number;
  posRelMs: number;
  onSeek: (relMs: number) => void;
}) {
  const { theme } = useAppTheme();
  const [width, setWidth] = useState(0);
  const trackRef = useRef<any>(null);
  const trackXRef = useRef(0);
  const lastSeekRef = useRef(0);
  const stateRef = useRef({ clipDurMs: 0, width: 0 });
  stateRef.current = { clipDurMs: props.clipDurMs, width };
  const onSeekRef = useRef(props.onSeek);
  onSeekRef.current = props.onSeek;

  const measureTrack = useCallback(() => {
    trackRef.current?.measureInWindow?.((x: number) => {
      if (typeof x === "number") trackXRef.current = x;
    });
  }, []);

  const relFromEvent = (e: any) => {
    const s = stateRef.current;
    const x = Math.max(0, Math.min(s.width, e.nativeEvent.pageX - trackXRef.current));
    return (x / Math.max(1, s.width)) * s.clipDurMs;
  };

  const pan = useRef(
    PanResponder.create({
      onStartShouldSetPanResponder: () => true,
      onMoveShouldSetPanResponder: () => true,
      onPanResponderGrant: (e: any) => {
        measureTrack();
        if (stateRef.current.width <= 0 || stateRef.current.clipDurMs <= 0) return;
        lastSeekRef.current = Date.now();
        onSeekRef.current(relFromEvent(e));
      },
      onPanResponderMove: (e: any) => {
        const s = stateRef.current;
        if (s.width <= 0 || s.clipDurMs <= 0) return;
        // Throttle scrubbing so rapid move events don't stutter playback.
        const now = Date.now();
        if (now - lastSeekRef.current < 180) return;
        lastSeekRef.current = now;
        onSeekRef.current(relFromEvent(e));
      },
    })
  ).current;

  const styles = makeStyles(theme);
  const dur = Math.max(1, props.clipDurMs);
  const posPct = Math.min(100, Math.max(0, (props.posRelMs / dur) * 100));

  return (
    <View
      ref={trackRef}
      {...pan.panHandlers}
      onLayout={(e) => {
        setWidth(e.nativeEvent.layout.width);
        measureTrack();
      }}
      style={styles.clipTrack}
    >
      <View style={[styles.clipRange, { width: `${posPct}%` }]} />
      <View style={[styles.clipPlayhead, { left: `${posPct}%` }]} />
    </View>
  );
}

function makeStyles(theme: ReturnType<typeof useAppTheme>["theme"]) {
  const { width } = Dimensions.get("window");

  const ui = learningDesign(theme);
  return StyleSheet.create({
    safeArea: {
      flex: 1,
      backgroundColor: theme.bg,
    },
    container: {
      flex: 1,
      ...ui.page,
    },

    // Header
    header: {
      alignItems: "flex-start",
      ...ui.header,
    },
    kindRow: {
      flexDirection: "row",
      flexWrap: "wrap",
      gap: 8,
      marginBottom: 10,
    },
    kindBtn: {
      paddingHorizontal: 12,
      alignItems: "center",
      paddingVertical: 9,
      borderWidth: 1,
      borderColor: theme.border,
      backgroundColor: theme.surface,
      ...ui.button,
    },
    kindBtnActive: {
      backgroundColor: theme.accent,
      borderColor: theme.accent,
    },
    kindBtnText: { color: theme.text, fontSize: 13 },
    kindBtnTextActive: { color: "#fff", fontWeight: "700" },
    // 计划复习：模式切换 + 到期信息 + 评分按钮
    modeSpacer: { flex: 1 },
    modeBtn: {
      alignItems: "center",
      paddingVertical: 9,
      paddingHorizontal: 14,
      borderWidth: 1,
      borderColor: theme.border,
      backgroundColor: theme.surface,
      ...ui.button,
    },
    modeBtnActive: { backgroundColor: theme.text, borderColor: theme.text },
    modeBtnText: { color: theme.textSecondary, fontSize: 12 },
    modeBtnTextActive: { color: theme.bg, fontWeight: "700" },
    dueInfoText: { color: theme.textMuted, fontSize: 12, marginBottom: 8 },
    gradeRow: {
      flexDirection: "row",
      gap: 10,
      marginTop: 10,
    },
    gradeBtn: {
      flex: 1,
      alignItems: "center",
      paddingVertical: 10,
      ...ui.button,
    },
    gradeAgain: { backgroundColor: theme.danger },
    gradeHard: { backgroundColor: theme.textSecondary },
    gradeGood: { backgroundColor: theme.accent },
    gradeBtnText: { color: "#fff", fontSize: 14, fontWeight: "700" },
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

    // Toolbar
    toolbar: {
      flexDirection: "row",
      flexWrap: "wrap",
      justifyContent: "flex-start",
      gap: 8,
      marginBottom: 32,
    },

    // Buttons
    btn: {
      flexDirection: "row",
      alignItems: "center",
      gap: 6,
      paddingVertical: 10,
      paddingHorizontal: 15,
      ...ui.button,
    },
    btnPressed: {
      opacity: 0.7,
      transform: [{ scale: 0.97 }],
    },
    btnDisabled: {
      opacity: 0.4,
    },
    btnPrimary: {
      backgroundColor: theme.accent,
    },
    btnPrimaryPressed: {
      backgroundColor: theme.accentHover,
    },
    btnPrimaryText: {
      color: "#fff",
      fontSize: 14,
      fontWeight: "600",
    },
    btnSecondary: {
      backgroundColor: theme.surface,
      borderWidth: 1,
      borderColor: theme.border,
    },
    btnSecondaryText: {
      color: theme.text,
      fontSize: 14,
      fontWeight: "500",
    },
    btnGhost: {
      backgroundColor: "transparent",
      paddingHorizontal: 15,
    },
    btnGhostPressed: {
      backgroundColor: `${theme.accent}14`,
    },
    btnGhostText: {
      fontSize: 14,
      fontWeight: "500",
    },

    // Error
    errorBanner: {
      backgroundColor: `${theme.danger}14`,
      borderRadius: 6,
      paddingVertical: 10,
      paddingHorizontal: 16,
      marginBottom: 16,
    },
    errorText: {
      color: theme.danger,
      fontSize: 13,
      textAlign: "center",
    },

    // Empty state
    emptyState: {
      flex: 1,
      justifyContent: "center",
      alignItems: "flex-start",
      paddingVertical: 80,
      paddingHorizontal: 44,
      backgroundColor: theme.surface,
      borderWidth: 1,
      borderColor: theme.border,
      borderRadius: 8,
    },
    emptyIndex: { fontSize: 13, fontWeight: "700", color: theme.accent, marginBottom: 12 },
    emptyText: {
      fontSize: 15,
      color: theme.textSecondary,
      textAlign: "left",
      lineHeight: 22,
    },

    // Card
    cardArea: {
      flex: 1,
      width: "100%",
      alignItems: "center",
      position: "relative",
    },
    cardFontSizeControls: {
      position: "absolute",
      zIndex: 2,
      top: 8,
      right: 0,
      flexDirection: "row",
      alignItems: "center",
      gap: 6,
      paddingVertical: 4,
      paddingHorizontal: 7,
      borderRadius: 7,
      borderWidth: 1,
      borderColor: theme.border,
      backgroundColor: theme.surface,
    },
    cardFontSizeLabel: { color: theme.textSecondary, fontSize: 11, marginRight: 2 },
    cardFontSizeBtn: {
      minWidth: 25,
      alignItems: "center",
      paddingVertical: 2,
      borderRadius: 4,
      backgroundColor: theme.bg,
    },
    cardFontSizeBtnText: { color: theme.textSecondary, fontSize: 12, fontWeight: "700" },
    cardFontSizeValue: {
      minWidth: 17,
      color: theme.textSecondary,
      fontSize: 11,
      fontVariant: ["tabular-nums"],
      textAlign: "center",
    },
    card: {
      width: "100%",
      maxWidth: "100%",
      height: "100%",
      paddingVertical: 28,
      paddingHorizontal: Platform.OS === 'android' ? 20 : 32,
      alignItems: "center",
      justifyContent: "center",
      ...theme.cardShadow,
      ...ui.card,
    },
    // Pressable must own the card's dimensions. Otherwise a long child can
    // make the pressable grow even though the card itself has a fixed height.
    // Use the available parent width instead of the word's intrinsic width,
    // while keeping a comfortable minimum width on normal desktop windows.
    cardPressable: {
      width: "100%",
      maxWidth: 840,
      minWidth: Math.min(420, Math.max(0, width - 104)),
      flex: 1,
      minHeight: 180,
      maxHeight: 560,
    },
    cardMeaning: { justifyContent: "flex-start" },
    cardPressed: { opacity: 0.88 },
    // The native scroll view clips its content on macOS, unlike a translated View.
    // The hint remains outside this view, so it is always visible.
    meaningScrollContainer: {
      flex: 1,
      minHeight: 0,
      alignSelf: "stretch",
      position: "relative",
    },
    meaningScroll: { width: "100%", height: "100%" },
    meaningContent: { paddingBottom: 10, alignItems: "stretch" },
    meaningTextWrap: { alignSelf: "stretch", width: "100%" },
    clipScroll: { flex: 1, alignSelf: "stretch" },
    videoClipWrap: {
      borderRadius: 8,
      overflow: "hidden",
      backgroundColor: "#000",
      height: 240,
    },
    videoClipPlayer: { flex: 1, width: "100%" },
    clipScrollContent: {
      flexGrow: 1,
      justifyContent: "center",
      paddingVertical: 8,
    },
    wordFace: {
      fontSize: 46,
      fontWeight: "700",
      color: theme.text,
      textAlign: "center",
    },
    aiCardTag: {
      alignSelf: "center",
      color: theme.accent,
      fontSize: 11,
      fontWeight: "800",
      letterSpacing: 1,
      marginBottom: 10,
    },
    aiCardQuestion: {
      color: theme.text,
      fontWeight: "700",
      textAlign: "center",
      alignSelf: "stretch",
    },
    aiCardContext: {
      color: theme.textSecondary,
      textAlign: "center",
      alignSelf: "stretch",
      marginTop: 14,
    },
    aiCardFlipHint: {
      color: theme.textSecondary,
      fontSize: 12,
      textAlign: "center",
      marginTop: 18,
    },
    clipWordsWrap: {
      flexDirection: "row",
      flexWrap: "wrap",
      justifyContent: "center",
      alignItems: "center",
      alignSelf: "stretch",
      rowGap: 4,
    },
    clipWord: {
      color: theme.text,
      fontSize: 22,
      lineHeight: 32,
      fontWeight: "500",
      paddingHorizontal: 1,
    },
    clipWordGap: {
      color: theme.text,
      fontSize: 22,
    },
    clipWordPressed: {
      backgroundColor: `${theme.accent}33`,
      borderRadius: 4,
    },
    clipTrack: {
      alignSelf: "stretch",
      height: 26,
      marginHorizontal: 8,
      marginTop: 14,
      borderRadius: 6,
      backgroundColor: theme.bg,
      borderWidth: StyleSheet.hairlineWidth,
      borderColor: theme.border,
    },
    clipRange: {
      position: "absolute",
      top: 0,
      bottom: 0,
      left: 0,
      backgroundColor: `${theme.accent}33`,
    },
    clipPlayhead: {
      position: "absolute",
      top: 2,
      bottom: 2,
      width: 2,
      backgroundColor: theme.accent,
    },
    clipActionsRow: {
      flexDirection: "row",
      justifyContent: "center",
      gap: 12,
      marginTop: 10,
    },
    clipActionBtn: {
      paddingVertical: 6,
      paddingHorizontal: 14,
      borderRadius: 100,
      borderWidth: 1,
      borderColor: theme.border,
    },
    clipActionBtnActive: {
      borderColor: theme.accent,
      backgroundColor: `${theme.accent}22`,
    },
    clipActionText: {
      color: theme.textSecondary,
      fontSize: 13,
    },
    clipActionTextActive: {
      color: theme.accent,
      fontWeight: "700",
    },
    clipZhText: {
      alignSelf: "center",
      marginTop: 12,
      fontSize: 17,
      lineHeight: 25,
      color: theme.textSecondary,
      textAlign: "center",
      paddingHorizontal: 8,
    },
    clipRateRow: {
      flexDirection: "row",
      justifyContent: "center",
      gap: 8,
      marginTop: 12,
    },
    clipRateBtn: {
      paddingVertical: 5,
      paddingHorizontal: 12,
      borderRadius: 100,
      borderWidth: 1,
      borderColor: theme.border,
    },
    clipRateBtnActive: {
      borderColor: theme.accent,
      backgroundColor: `${theme.accent}22`,
    },
    clipRateText: { color: theme.textSecondary, fontSize: 13 },
    clipRateTextActive: { color: theme.accent, fontWeight: "700" },
    meaningFace: {
      fontSize: 24,
      fontWeight: "400",
      lineHeight: 35,
      color: theme.text,
      textAlign: "center",
    },
    cardHint: {
      fontSize: 12,
      color: theme.textSecondary,
      marginTop: 26,
      opacity: 0.7,
    },

    // Card footer
    cardFooter: {
      flexDirection: "row",
      alignItems: "center",
      width: 680,
      maxWidth: "100%",
      marginTop: 18,
    },
    progressText: {
      fontSize: 13,
      color: theme.textSecondary,
      fontVariant: ["tabular-nums"],
    },
    progressTrack: {
      flex: 1,
      height: 4,
      backgroundColor: theme.border,
      borderRadius: 2,
      marginHorizontal: 16,
      overflow: "hidden",
    },
    progressFill: {
      height: "100%",
      backgroundColor: theme.accent,
      borderRadius: 2,
    },

    // Nav buttons
    navButtons: {
      flexDirection: "row",
      gap: 10,
      width: 680,
      maxWidth: "100%",
      marginTop: 18,
    },
    navBtn: {
      flex: 1,
      justifyContent: "center",
      alignItems: "center",
    },
    deleteBtn: {
      flex: 0.7,
      borderColor: theme.danger,
    },
    deleteBtnText: {
      color: theme.danger,
    },

    // Toast
    toast: {
      position: "absolute",
      bottom: 24,
      alignSelf: "center",
      backgroundColor: theme.toastBg,
      paddingVertical: 10,
      paddingHorizontal: 22,
      borderRadius: 6,
      shadowColor: "#000",
      shadowOffset: { width: 0, height: 4 },
      shadowOpacity: 0.18,
      shadowRadius: 16,
      elevation: 8,
    },
    toastText: {
      color: theme.toastText,
      fontSize: 14,
      fontWeight: "500",
    },
  });
}
