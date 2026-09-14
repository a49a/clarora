import { learningDesign } from "../ui/learningDesign";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  Platform,
  Pressable,
  StatusBar,
  StyleSheet,
  Text,
  TextInput,
  View,
  ScrollView,} from "react-native";
import { Audio, DocumentPicker, type SoundLike } from "../services/platform";
import { getSetting, setSetting } from "../data/database";
import { useAppTheme } from "../ui/ThemeContext";
import { usePomodoro } from "../ui/PomodoroContext";

const DURATION_PRESETS = [5, 10, 15, 20, 30];
const FOCUS_PRESETS = [25, 45, 60];
const BREAK_PRESETS = [5, 10, 15];
type PageMode = "meditation" | "pomodoro";

function formatClock(totalSeconds: number): string {
  const m = Math.floor(totalSeconds / 60);
  const s = totalSeconds % 60;
  return `${m.toString().padStart(2, "0")}:${s.toString().padStart(2, "0")}`;
}

export default function MeditationScreen() {
  const { theme, scheme } = useAppTheme();
  const styles = makeStyles(theme);

  const [musicUri, setMusicUri] = useState<string | null>(null);
  const [musicName, setMusicName] = useState<string | null>(null);
  const [durationMin, setDurationMin] = useState(10);
  const [durationText, setDurationText] = useState("10");
  const [pageMode, setPageMode] = useState<PageMode>("meditation");
  const {
    focusMin,
    breakMin,
    phase: pomodoroPhase,
    running: pomodoroRunning,
    paused: pomodoroPaused,
    remainingSec: pomodoroRemainingSec,
    totalSec: pomodoroTotalSec,
    completedFocusRounds,
    message: pomodoroMessage,
    setFocusMin: selectFocusDuration,
    setBreakMin: selectBreakDuration,
    start: startPomodoro,
    togglePause: togglePomodoroPause,
    reset: resetPomodoro,
  } = usePomodoro();

  // 学习番茄钟的自定义时长输入（与预设/番茄钟状态双向同步）。
  const [focusText, setFocusText] = useState(String(focusMin));
  const [breakText, setBreakText] = useState(String(breakMin));
  useEffect(() => { setFocusText(String(focusMin)); }, [focusMin]);
  useEffect(() => { setBreakText(String(breakMin)); }, [breakMin]);
  // Session state
  const [running, setRunning] = useState(false);
  const [paused, setPaused] = useState(false);
  const [completed, setCompleted] = useState(false);
  const [remainingSec, setRemainingSec] = useState(0);
  const [totalSec, setTotalSec] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const soundRef = useRef<SoundLike | null>(null);

  // Breathing pulse (JS-driven; avoids native Animated on macOS)
  const [breathePhase, setBreathePhase] = useState(0);
  const breatheDirRef = useRef(1);

  const unloadSound = useCallback(async () => {
    if (soundRef.current) {
      try {
        await soundRef.current.unloadAsync();
      } catch {
        // already released
      }
      soundRef.current = null;
    }
  }, []);

  // Restore saved music and duration
  useEffect(() => {
    (async () => {
      try {
        const savedUri = await getSetting("meditation_music_uri");
        const savedName = await getSetting("meditation_music_name");
        const savedDuration = await getSetting("meditation_duration_min");
        if (savedUri) {
          setMusicUri(savedUri);
          setMusicName(savedName ?? "已保存的音乐");
        }
        if (savedDuration) {
          const minutes = parseInt(savedDuration, 10);
          if (minutes > 0 && minutes <= 180) {
            setDurationMin(minutes);
            setDurationText(String(minutes));
          }
        }
      } catch (e: any) {
        setError(String(e?.message ?? e));
      }
    })();
  }, []);


  const finishSession = useCallback(
    async (didComplete: boolean) => {
      await unloadSound();
      setRunning(false);
      setPaused(false);
      setCompleted(didComplete);
    },
    [unloadSound]
  );

  // Countdown timer
  useEffect(() => {
    if (!running || paused) return;
    if (remainingSec <= 0) {
      finishSession(true);
      return;
    }
    const timer = setTimeout(() => setRemainingSec((s) => s - 1), 1000);
    return () => clearTimeout(timer);
  }, [running, paused, remainingSec, finishSession]);

  // Breathing pulse loop
  useEffect(() => {
    if (!running || paused) return;
    const interval = setInterval(() => {
      setBreathePhase((p) => {
        let next = p + 0.04 * breatheDirRef.current;
        if (next >= 1) {
          next = 1;
          breatheDirRef.current = -1;
        } else if (next <= 0) {
          next = 0;
          breatheDirRef.current = 1;
        }
        return next;
      });
    }, 80);
    return () => clearInterval(interval);
  }, [running, paused]);

  // Release audio when leaving the screen
  useEffect(() => {
    return () => {
      soundRef.current?.unloadAsync();
      soundRef.current = null;
    };
  }, []);

  const pickMusic = useCallback(async () => {
    setError(null);
    try {
      const result = await DocumentPicker.getDocumentAsync({
        type: "audio/*",
        copyToCacheDirectory: true,
      });
      if (result.canceled || !result.assets?.length) return;
      const asset = result.assets[0];
      setMusicUri(asset.uri);
      setMusicName(asset.name);
      await Promise.all([
        setSetting("meditation_music_uri", asset.uri),
        setSetting("meditation_music_name", asset.name),
      ]);
    } catch (e: any) {
      setError(String(e?.message ?? e));
    }
  }, []);

  const changeDuration = useCallback((minutes: number) => {
    setDurationMin(minutes);
    setDurationText(String(minutes));
  }, []);

  const applyCustomDuration = useCallback((text: string) => {
    setDurationText(text);
    const minutes = parseInt(text, 10);
    if (!Number.isNaN(minutes) && minutes > 0 && minutes <= 180) {
      setDurationMin(minutes);
    }
  }, []);

  const startSession = useCallback(async () => {
    setError(null);
    setCompleted(false);
    const total = Math.max(1, Math.round(durationMin * 60));
    setTotalSec(total);
    setRemainingSec(total);
    setPaused(false);
    setRunning(true);
    setSetting("meditation_duration_min", String(durationMin));
    if (musicUri) {
      try {
        await unloadSound();
        const { sound } = await Audio.Sound.createAsync(
          { uri: musicUri },
          { shouldPlay: true, loop: true, progressUpdateIntervalMillis: 1000 }
        );
        soundRef.current = sound;
      } catch (e: any) {
        setError(`音乐播放失败：${e?.message ?? e}`);
      }
    }
  }, [durationMin, musicUri, unloadSound]);

  const togglePause = useCallback(async () => {
    const next = !paused;
    setPaused(next);
    try {
      if (soundRef.current) {
        if (next) {
          await soundRef.current.pauseAsync();
        } else {
          await soundRef.current.playAsync();
        }
      }
    } catch {
      // keep timer state even if audio control fails
    }
  }, [paused]);

  // ── Render ──────────────────────────────────────────────────────────────────

  const circleSize = 170 + 80 * breathePhase;
  const inhaling = breatheDirRef.current > 0;
  const progress = totalSec > 0 ? 1 - remainingSec / totalSec : 0;
  const pomodoroProgress = pomodoroTotalSec > 0
    ? 1 - pomodoroRemainingSec / pomodoroTotalSec
    : 0;

  return (
    <View style={styles.safeArea}>
      <StatusBar barStyle={scheme === "dark" ? "light-content" : "dark-content"} />
      <ScrollView
        style={styles.container}
        contentContainerStyle={styles.contentInner}
        showsVerticalScrollIndicator={false}
      >
        {/* Header */}
        <View style={styles.header}>
          <Text style={styles.eyebrow}>MINDFULNESS</Text>
          <Text style={styles.title}>冥想放松</Text>
          <Text style={styles.subtitle}>学习间隙，用一段安静的时间恢复专注。</Text>
        </View>

        {error && (
          <View style={styles.errorBanner}>
            <Text style={styles.errorText}>{error}</Text>
          </View>
        )}

        <View style={styles.modeTabs}>
          {(["meditation", "pomodoro"] as const).map((mode) => (
            <Pressable
              key={mode}
              style={[styles.modeTab, pageMode === mode && styles.modeTabActive]}
              onPress={() => setPageMode(mode)}
            >
              <Text style={[styles.modeTabText, pageMode === mode && styles.modeTabTextActive]}>
                {mode === "meditation" ? "冥想" : "学习番茄钟"}
              </Text>
            </Pressable>
          ))}
        </View>

        {/* Setup panel */}
        {pageMode === "meditation" && <View style={styles.panel}>
          <Text style={styles.panelLabel}>冥想时长</Text>
          <View style={styles.presetRow}>
            {DURATION_PRESETS.map((minutes) => (
              <Pressable
                key={minutes}
                style={[styles.presetBtn, durationMin === minutes && styles.presetBtnActive]}
                onPress={() => changeDuration(minutes)}
              >
                <Text
                  style={[
                    styles.presetBtnText,
                    durationMin === minutes && styles.presetBtnTextActive,
                  ]}
                >
                  {minutes} 分钟
                </Text>
              </Pressable>
            ))}
          </View>
          <View style={styles.customRow}>
            <Text style={styles.customLabel}>自定义</Text>
            <TextInput
              style={styles.customInput}
              value={durationText}
              onChangeText={applyCustomDuration}
              keyboardType="numbers-and-punctuation"
              maxLength={3}
            />
            <Text style={styles.customLabel}>分钟（1–180）</Text>
          </View>

          <Text style={styles.panelLabel}>冥想音乐</Text>
          <View style={styles.musicRow}>
            <Pressable style={styles.musicBtn} onPress={pickMusic}>
              <Text style={styles.musicBtnText}>{musicUri ? "更换音乐" : "选择音乐"}</Text>
            </Pressable>
            <Text style={styles.musicName} numberOfLines={1}>
              {musicName ?? "未选择（将进行无音乐冥想）"}
            </Text>
          </View>

          <Pressable style={({ pressed }) => [styles.startBtn, pressed && { opacity: 0.85 }]} onPress={startSession}>
            <Text style={styles.startBtnText}>开始冥想</Text>
          </Pressable>
        </View>}

        {pageMode === "pomodoro" && (
          <View style={styles.panel}>
            {!pomodoroRunning && (
              <>
                <View style={styles.durationColumns}>
                <View style={styles.durationColumn}>
                <Text style={styles.panelLabel}>专注时长</Text>
                <View style={styles.presetRow}>
                  {FOCUS_PRESETS.map((minutes) => (
                    <Pressable
                      key={minutes}
                      style={[styles.presetBtn, focusMin === minutes && styles.presetBtnActive]}
                      onPress={() => selectFocusDuration(minutes)}
                    >
                      <Text style={[styles.presetBtnText, focusMin === minutes && styles.presetBtnTextActive]}>
                        {minutes} 分钟
                      </Text>
                    </Pressable>
                  ))}
                </View>
                <View style={styles.customRow}>
                  <Text style={styles.customLabel}>自定义</Text>
                  <TextInput
                    style={styles.customInput}
                    value={focusText}
                    onChangeText={(text) => {
                      setFocusText(text);
                      const n = Number.parseInt(text, 10);
                      if (Number.isFinite(n) && n >= 1) selectFocusDuration(Math.min(180, n));
                    }}
                    keyboardType="number-pad"
                    maxLength={3}
                  />
                  <Text style={styles.customLabel}>分钟（1–180）</Text>
                </View>
                <Text style={styles.panelLabel}>休息时长</Text>
                <View style={styles.presetRow}>
                  {BREAK_PRESETS.map((minutes) => (
                    <Pressable
                      key={minutes}
                      style={[styles.presetBtn, breakMin === minutes && styles.presetBtnActive]}
                      onPress={() => selectBreakDuration(minutes)}
                    >
                      <Text style={[styles.presetBtnText, breakMin === minutes && styles.presetBtnTextActive]}>
                        {minutes} 分钟
                      </Text>
                    </Pressable>
                  ))}
                </View>
                <View style={styles.customRow}>
                  <Text style={styles.customLabel}>自定义</Text>
                  <TextInput
                    style={styles.customInput}
                    value={breakText}
                    onChangeText={(text) => {
                      setBreakText(text);
                      const n = Number.parseInt(text, 10);
                      if (Number.isFinite(n) && n >= 1) selectBreakDuration(Math.min(60, n));
                    }}
                    keyboardType="number-pad"
                    maxLength={2}
                  />
                  <Text style={styles.customLabel}>分钟（1–60）</Text>
                </View>
                </View>
                </View>
              </>
            )}

            <View style={styles.pomodoroTimer}>
              <Text style={[
                styles.pomodoroPhase,
                pomodoroPhase === "break" && styles.pomodoroBreakPhase,
              ]}>
                {pomodoroPhase === "focus" ? "专注学习" : "休息恢复"}
              </Text>
              <Text style={styles.pomodoroClock}>{formatClock(pomodoroRemainingSec)}</Text>
              <View style={styles.pomodoroProgressTrack}>
                <View style={[
                  styles.pomodoroProgressFill,
                  pomodoroPhase === "break" && styles.pomodoroBreakProgress,
                  { width: `${Math.min(100, pomodoroProgress * 100)}%` },
                ]} />
              </View>
              <Text style={styles.pomodoroMessage}>{pomodoroMessage}</Text>
              <Text style={styles.pomodoroRounds}>已完成 {completedFocusRounds} 轮专注</Text>
            </View>

            {!pomodoroRunning ? (
              <Pressable style={styles.startBtn} onPress={() => startPomodoro()}>
                <Text style={styles.startBtnText}>开始专注</Text>
              </Pressable>
            ) : (
              <View style={styles.sessionActions}>
                <Pressable style={styles.sessionBtnGhost} onPress={togglePomodoroPause}>
                  <Text style={styles.sessionBtnGhostText}>{pomodoroPaused ? "继续" : "暂停"}</Text>
                </Pressable>
                <Pressable style={styles.sessionBtnDanger} onPress={resetPomodoro}>
                  <Text style={styles.sessionBtnDangerText}>重置</Text>
                </Pressable>
              </View>
            )}
          </View>
        )}

        {/* Immersive meditation overlay */}
        {running && (
          <View style={styles.sessionOverlay}>
            <View style={[styles.breathCircle, { width: circleSize, height: circleSize }]}>
              <Text style={styles.breathLabel}>{inhaling ? "吸气" : "呼气"}</Text>
            </View>
            <Text style={styles.clock}>{formatClock(remainingSec)}</Text>
            <View style={styles.sessionProgressTrack}>
              <View style={[styles.sessionProgressFill, { width: `${Math.min(100, progress * 100)}%` }]} />
            </View>
            <Text style={styles.sessionMeta}>
              共 {durationMin} 分钟{musicName ? ` · ${musicName}` : " · 无音乐"}
            </Text>
            <View style={styles.sessionActions}>
              <Pressable style={styles.sessionBtnGhost} onPress={togglePause}>
                <Text style={styles.sessionBtnGhostText}>{paused ? "继续" : "暂停"}</Text>
              </Pressable>
              <Pressable style={styles.sessionBtnDanger} onPress={() => finishSession(false)}>
                <Text style={styles.sessionBtnDangerText}>结束冥想</Text>
              </Pressable>
            </View>
          </View>
        )}

        {/* Completion view */}
        {pageMode === "meditation" && completed && !running && (
          <View style={styles.panel}>
            <Text style={styles.completedTitle}>冥想完成</Text>
            <Text style={styles.completedText}>
              本次冥想 {durationMin} 分钟，欢迎回来。
            </Text>
            <Pressable style={styles.startBtn} onPress={() => setCompleted(false)}>
              <Text style={styles.startBtnText}>返回</Text>
            </Pressable>
          </View>
        )}
      </ScrollView>
    </View>
  );
}

function makeStyles(theme: ReturnType<typeof useAppTheme>["theme"]) {
  const ui = learningDesign(theme);
  return StyleSheet.create({
    safeArea: { flex: 1, backgroundColor: theme.bg },
    container: {
      flex: 1,
      width: "100%",
      maxWidth: 1040,
      alignSelf: "center",
    },
    contentInner: {
      ...ui.page,
    },
    header: {
      alignItems: "flex-start",
      ...ui.header,
    },
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
    errorBanner: {
      backgroundColor: `${theme.danger}14`,
      borderRadius: 6,
      paddingVertical: 10,
      paddingHorizontal: 16,
      marginBottom: 16,
    },
    errorText: { color: theme.danger, fontSize: 13 },

    modeTabs: {
      flexDirection: "row",
      alignSelf: "flex-start",
      backgroundColor: theme.surfaceHover,
      borderRadius: 7,
      padding: 3,
      marginBottom: 16,
    },
    modeTab: {
      paddingVertical: 8,
      paddingHorizontal: 18,
      ...ui.button,
    },
    modeTabActive: { backgroundColor: theme.surface, ...theme.cardShadow },
    modeTabText: { color: theme.textMuted, fontSize: 13, fontWeight: "600" },
    modeTabTextActive: { color: theme.text, fontWeight: "700" },

    // Setup panel
    panel: {
      ...ui.panel,
    },
    panelLabel: {
      marginBottom: 10,
      ...ui.label,
    },
    presetRow: { flexDirection: "row", flexWrap: "wrap", gap: 8, marginBottom: 16 },
    presetBtn: {
      paddingVertical: 9,
      paddingHorizontal: 16,
      borderWidth: 1,
      borderColor: theme.border,
      backgroundColor: theme.surfaceHover,
      ...ui.button,
    },
    presetBtnActive: { backgroundColor: theme.accent, borderColor: theme.accent },
    presetBtnText: { color: theme.textSecondary, fontSize: 13, fontWeight: "600" },
    presetBtnTextActive: { color: "#fff" },
    durationColumns: { flexDirection: "row", gap: 24, flexWrap: "wrap" },
    durationColumn: { flex: 1, minWidth: 260 },
    customRow: { flexDirection: "row", alignItems: "center", gap: 10, marginBottom: 22 },
    customLabel: { color: theme.textSecondary, fontSize: 13 },
    customInput: {
      width: 64,
      borderWidth: 1,
      borderColor: theme.border,
      borderRadius: 6,
      paddingVertical: 7,
      paddingHorizontal: 10,
      color: theme.text,
      fontSize: 14,
      backgroundColor: theme.bg,
      textAlign: "center",
    },
    musicRow: { flexDirection: "row", alignItems: "center", gap: 12, marginBottom: 24 },
    musicBtn: {
      paddingVertical: 9,
      paddingHorizontal: 16,
      borderRadius: 6,
      borderWidth: 1,
      borderColor: theme.border,
      backgroundColor: theme.surfaceHover,
    },
    musicBtnText: { color: theme.text, fontSize: 13, fontWeight: "600" },
    musicName: { flex: 1, color: theme.textSecondary, fontSize: 13 },
    startBtn: {
      alignItems: "center",
      backgroundColor: theme.accent,
      paddingVertical: 13,
      ...ui.button,
    },
    startBtnText: { color: "#fff", fontSize: 15, fontWeight: "700" },

    // Pomodoro
    pomodoroTimer: { alignItems: "center", paddingVertical: 10, marginBottom: 20 },
    pomodoroPhase: {
      color: theme.accent,
      backgroundColor: `${theme.accent}18`,
      borderRadius: 999,
      paddingVertical: 5,
      paddingHorizontal: 12,
      fontSize: 12,
      fontWeight: "700",
      marginBottom: 14,
    },
    pomodoroBreakPhase: { color: "#c77800", backgroundColor: "#c7780018" },
    pomodoroClock: {
      color: theme.text,
      fontSize: 62,
      lineHeight: 72,
      fontWeight: "700",
      fontVariant: ["tabular-nums"],
      marginBottom: 16,
    },
    pomodoroProgressTrack: {
      width: 380,
      maxWidth: "100%",
      height: 6,
      borderRadius: 3,
      backgroundColor: theme.border,
      overflow: "hidden",
      marginBottom: 14,
    },
    pomodoroProgressFill: { height: "100%", borderRadius: 3, backgroundColor: theme.accent },
    pomodoroBreakProgress: { backgroundColor: "#c77800" },
    pomodoroMessage: { color: theme.textSecondary, fontSize: 14, textAlign: "center" },
    pomodoroRounds: { color: theme.textMuted, fontSize: 12, marginTop: 7 },

    // Immersive session
    sessionOverlay: {
      position: "absolute",
      top: 0,
      right: 0,
      bottom: 0,
      left: 0,
      backgroundColor: theme.bg,
      alignItems: "center",
      justifyContent: "center",
      paddingHorizontal: Platform.select({ android: 24, default: 52 }),
    },
    breathCircle: {
      alignItems: "center",
      justifyContent: "center",
      borderRadius: 500,
      backgroundColor: theme.surface,
      borderWidth: 2,
      borderColor: theme.accent,
      marginBottom: 34,
      ...theme.cardShadow,
    },
    breathLabel: { color: theme.textSecondary, fontSize: 15, fontWeight: "600" },
    clock: {
      color: theme.text,
      fontSize: 58,
      fontWeight: "700",
      fontVariant: ["tabular-nums"],
      marginBottom: 18,
    },
    sessionProgressTrack: {
      width: 340,
      maxWidth: "100%",
      height: 4,
      borderRadius: 2,
      backgroundColor: theme.border,
      overflow: "hidden",
      marginBottom: 10,
    },
    sessionProgressFill: { height: "100%", backgroundColor: theme.accent, borderRadius: 2 },
    sessionMeta: { color: theme.textMuted, fontSize: 12, marginBottom: 30 },
    sessionActions: { flexDirection: "row", gap: 10 },
    sessionBtnGhost: {
      paddingVertical: 11,
      paddingHorizontal: 26,
      borderRadius: 6,
      borderWidth: 1,
      borderColor: theme.border,
      backgroundColor: theme.surface,
    },
    sessionBtnGhostText: { color: theme.text, fontSize: 14, fontWeight: "600" },
    sessionBtnDanger: {
      paddingVertical: 11,
      paddingHorizontal: 26,
      borderRadius: 6,
      backgroundColor: theme.danger,
    },
    sessionBtnDangerText: { color: "#fff", fontSize: 14, fontWeight: "600" },

    // Completion
    completedTitle: { fontSize: 22, fontWeight: "700", color: theme.text, marginBottom: 6 },
    completedText: { color: theme.textSecondary, fontSize: 14, marginBottom: 20 },
  });
}
