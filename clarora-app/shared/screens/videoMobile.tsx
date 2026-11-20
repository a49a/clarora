import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Platform,
  Pressable,
  ScrollView,
  StatusBar,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { Details } from "../ui/Details";
import { learningDesign } from "../ui/learningDesign";
import { useAppTheme } from "../ui/ThemeContext";
import { MobileVideoPlayer, type MobileVideoHandle } from "../ui/MobileVideoPlayer";
import {
  getLocalVideo,
  listLocalVideos,
  saveLocalVideo,
  setLocalVideoSubtitle,
  touchLocalVideo,
  type LocalVideo,
} from "../data/database";
import { cueLanguages, parseSubtitleCues, serializeSubtitleCues, type SubtitleCue } from "../data/subtitles";
import { DocumentPicker, FileSystem } from "../services/platform";
import RNFS from "../services/rnfs";

const RATES = [0.6, 0.8, 1.0, 1.25, 1.5];
const SEEK_STEP_MS = 2_000;
const LIST_LIMIT = 200;

export type MobileVideoScreenProps = {
  initialVideoId?: string | null;
  initialPositionMs?: number | null;
  onOpenSentenceSearch?: () => void;
};

function formatClock(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  if (minutes >= 60) {
    const hours = Math.floor(minutes / 60);
    return `${hours}:${String(minutes % 60).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
  }
  return `${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
}

function videoName(path: string): string {
  const name = path.split(/[\\/]/).pop() || path;
  try {
    return decodeURIComponent(name);
  } catch {
    // A literal percent sign in the file name is not an escape sequence.
    return name;
  }
}

export default function MobileVideoScreen({
  initialVideoId,
  initialPositionMs,
  onOpenSentenceSearch,
}: MobileVideoScreenProps) {
  const { theme, scheme } = useAppTheme();
  const styles = useMemo(() => makeStyles(theme), [theme]);
  const playerRef = useRef<MobileVideoHandle>(null);
  const pendingSeekRef = useRef<number | null>(null);
  const barWidthRef = useRef(0);

  const [videos, setVideos] = useState<LocalVideo[]>([]);
  const [current, setCurrent] = useState<LocalVideo | null>(null);
  const [cues, setCues] = useState<SubtitleCue[]>([]);
  const [playing, setPlaying] = useState(false);
  const [rate, setRate] = useState(1.0);
  const [positionMs, setPositionMs] = useState(0);
  const [durationMs, setDurationMs] = useState(0);
  const [subtitlesVisible, setSubtitlesVisible] = useState(true);
  const [rangeStartMs, setRangeStartMs] = useState<number | null>(null);
  const [rangeEndMs, setRangeEndMs] = useState<number | null>(null);
  const [rangeLoop, setRangeLoop] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const showNotice = useCallback((message: string) => {
    setNotice(message);
    setTimeout(() => setNotice(null), 2400);
  }, []);

  const refresh = useCallback(async () => {
    const rows = await listLocalVideos();
    setVideos(rows);
    return rows;
  }, []);

  useEffect(() => {
    let active = true;
    void (async () => {
      try {
        const rows = await listLocalVideos();
        if (!active) return;
        setVideos(rows);
        const target = initialVideoId
          ? await getLocalVideo(initialVideoId)
          : rows[0] ?? null;
        if (!active || !target) return;
        setCurrent(target);
        if (initialVideoId) {
          // Wait for onReady so playback starts at the searched sentence
          // instead of flashing the beginning of the video.
          pendingSeekRef.current = initialPositionMs ?? 0;
          setPlaying(false);
        }
        await touchLocalVideo(target.id);
      } catch (e: any) {
        if (active) setError(`读取视频失败：${e?.message ?? e}`);
      }
    })();
    return () => {
      active = false;
    };
  }, [initialVideoId, initialPositionMs]);

  useEffect(() => {
    let active = true;
    setCues([]);
    const uri = current?.subtitle_uri;
    if (!uri) return;
    void (async () => {
      try {
        const text = await FileSystem.readAsStringAsync(uri);
        const parsed = parseSubtitleCues(text);
        if (active && parsed.length) setCues(parsed);
      } catch {
        // A missing sidecar file simply means no subtitles for this video.
      }
    })();
    return () => {
      active = false;
    };
  }, [current?.id, current?.subtitle_uri]);

  const pickVideo = useCallback(async () => {
    setError(null);
    try {
      const result = await DocumentPicker.getDocumentAsync({ type: "video/*" });
      const file = result.assets?.[0];
      if (result.canceled || !file) return;
      const rawPath = file.uri.replace(/^file:\/\//, "");
      let nativePath = rawPath;
      try {
        nativePath = decodeURIComponent(rawPath);
      } catch {
        // Keep the undecoded path when it contains a literal percent sign.
      }
      const stat = await RNFS.stat(nativePath).catch(() => null);
      if (!stat || Number(stat.size) <= 0) {
        throw new Error("导入的视频文件为空或不可读");
      }
      const name = file.name || videoName(file.uri);
      let videoUri = file.uri;
      if (Platform.OS === "ios") {
        // The picker copies into NSCachesDirectory; move videos into
        // Documents so iOS cannot purge a recent entry.
        const dir = `${await FileSystem.getDocumentDirectoryAsync()}videos/`;
        await FileSystem.makeDirectoryAsync(dir).catch(() => {});
        const safeName = name.replace(/[\\/:*?"<>|]/g, "_");
        const target = `${dir}${Date.now()}_${safeName}`;
        await FileSystem.copyAsync({ from: file.uri, to: target });
        await FileSystem.deleteAsync(file.uri).catch(() => {});
        // Encode each segment so spaces and reserved characters stay a valid file URL.
        videoUri = `file://${target.split("/").map(encodeURIComponent).join("/")}`;
      }
      const saved = await saveLocalVideo(name, videoUri);
      setCurrent(saved);
      setCues([]);
      setPositionMs(0);
      setDurationMs(0);
      setRangeStartMs(null);
      setRangeEndMs(null);
      setRangeLoop(false);
      setPlaying(true);
      pendingSeekRef.current = 0;
      await refresh();
    } catch (e: any) {
      setError(`打开视频失败：${e?.message ?? e}`);
    }
  }, [refresh]);

  const openVideo = useCallback(
    (video: LocalVideo) => {
      setError(null);
      setCurrent(video);
      setPositionMs(0);
      setDurationMs(0);
      pendingSeekRef.current = 0;
      setRangeStartMs(null);
      setRangeEndMs(null);
      setRangeLoop(false);
      setPlaying(true);
      void touchLocalVideo(video.id).then(refresh).catch(() => {});
    },
    [refresh]
  );

  const importSubtitle = useCallback(async () => {
    if (!current) return;
    setError(null);
    setBusy(true);
    try {
      const result = await DocumentPicker.getDocumentAsync({ type: "text/*", copyToCacheDirectory: true });
      const file = result.assets?.[0];
      if (result.canceled || !file) return;
      const parsed = parseSubtitleCues(await FileSystem.readAsStringAsync(file.uri));
      if (!parsed.length) throw new Error("没有找到带时间轴的字幕，请选择 SRT 或 VTT 文件。");
      const dir = `${await FileSystem.getDocumentDirectoryAsync()}videos/`;
      await FileSystem.makeDirectoryAsync(dir).catch(() => {});
      const target = `${dir}${Date.now()}_${current.id}.srt`;
      await FileSystem.writeFileAsync(target, serializeSubtitleCues(parsed));
      await setLocalVideoSubtitle(current.id, target);
      const updated = { ...current, subtitle_uri: target };
      setCurrent(updated);
      setCues(parsed);
      await refresh();
      showNotice(`已导入 ${parsed.length} 句字幕`);
    } catch (e: any) {
      setError(`导入字幕失败：${e?.message ?? e}`);
    } finally {
      setBusy(false);
    }
  }, [current, refresh, showNotice]);

  const seekTo = useCallback((ms: number) => {
    const clamped = Math.min(Math.max(0, ms), durationMs > 0 ? durationMs : ms);
    playerRef.current?.seek(clamped / 1000);
    setPositionMs(clamped);
  }, [durationMs]);

  const seekBy = useCallback((deltaMs: number) => {
    seekTo(positionMs + deltaMs);
  }, [positionMs, seekTo]);

  const activeCueIndex = useMemo(() => {
    return cues.findIndex(
      (cue) => positionMs / 1000 >= cue.start && positionMs / 1000 < cue.end
    );
  }, [cues, positionMs]);

  const activeCue = activeCueIndex >= 0 ? cues[activeCueIndex] : null;
  const loopReady = rangeStartMs != null && rangeEndMs != null && rangeEndMs > rangeStartMs;

  const onProgress = useCallback(
    (nextMs: number, nextDurationMs: number) => {
      if (nextDurationMs > 0 && Math.abs(nextDurationMs - durationMs) > 1) {
        setDurationMs(nextDurationMs);
      }
      if (rangeLoop && loopReady && rangeEndMs != null && rangeStartMs != null && nextMs >= rangeEndMs) {
        playerRef.current?.seek(rangeStartMs / 1000);
        setPositionMs(rangeStartMs);
        return;
      }
      setPositionMs(nextMs);
    },
    [durationMs, loopReady, rangeEndMs, rangeLoop, rangeStartMs]
  );

  const currentCue = activeCue ? cueLanguages(activeCue.text) : null;

  return (
    <View style={styles.safeArea}>
      <StatusBar barStyle={scheme === "dark" ? "light-content" : "dark-content"} />
      <ScrollView
        style={styles.container}
        contentContainerStyle={styles.content}
        showsVerticalScrollIndicator={false}
      >
        <View style={styles.header}>
          <Text style={styles.eyebrow}>VIDEO LEARNING</Text>
          <Text style={styles.title}>视频学习</Text>
          <Text style={styles.subtitle}>播放本地视频，配合倍速与片段循环做精听；字幕可手动导入 SRT / VTT。</Text>
          {onOpenSentenceSearch ? (
            <Pressable
              accessibilityRole="button"
              accessibilityLabel="打开句型搜索"
              onPress={onOpenSentenceSearch}
              style={({ pressed }) => [styles.searchEntry, pressed && styles.pressed]}
            >
              <Text style={styles.searchEntryText}>🔍 句型搜索</Text>
            </Pressable>
          ) : null}
        </View>

        {error ? <Text style={styles.error}>{error}</Text> : null}
        {notice ? <Text style={styles.notice}>{notice}</Text> : null}

        <View style={styles.panel}>
          <View style={styles.row}>
            <Pressable
              accessibilityRole="button"
              onPress={() => void pickVideo()}
              style={({ pressed }) => [styles.primaryBtn, pressed && styles.pressed]}
            >
              <Text style={styles.primaryBtnText}>选择视频</Text>
            </Pressable>
            <Text style={styles.caption} numberOfLines={1}>
              {current ? current.name : "支持本地视频文件，最近打开的会列在下方。"}
            </Text>
          </View>
        </View>

        {current ? (
          <View style={styles.panel}>
            <View style={styles.playerWrap}>
              <MobileVideoPlayer
                ref={playerRef}
                uri={current.video_uri}
                playing={playing}
                rate={rate}
                style={styles.player}
                onReady={(readyDuration, readyPosition) => {
                  setDurationMs(readyDuration);
                  setPositionMs(readyPosition);
                  const target = pendingSeekRef.current;
                  pendingSeekRef.current = null;
                  if (target != null) {
                    if (target > 0) {
                      playerRef.current?.seek(target / 1000);
                      setPositionMs(target);
                    }
                    setPlaying(true);
                  }
                }}
                onProgress={onProgress}
                onEnd={() => {
                  setPlaying(false);
                  if (durationMs > 0) setPositionMs(durationMs);
                }}
                onError={(message) => {
                  setPlaying(false);
                  setError(message);
                }}
              />
              {subtitlesVisible && currentCue ? (
                <View pointerEvents="none" style={styles.subtitleOverlay}>
                  {!!currentCue.english && (
                    <Text style={styles.subtitleEnglish}>{currentCue.english}</Text>
                  )}
                  {!!currentCue.translation && (
                    <Text style={styles.subtitleChinese}>{currentCue.translation}</Text>
                  )}
                </View>
              ) : null}
            </View>

            <View style={styles.timelineRow}>
              <Text style={styles.time}>{formatClock(positionMs)}</Text>
              <Pressable
                accessibilityRole="button"
                accessibilityLabel="播放进度"
                onLayout={(event) => {
                  barWidthRef.current = event.nativeEvent.layout.width;
                }}
                onPress={(event) => {
                  if (barWidthRef.current <= 0 || durationMs <= 0) return;
                  const ratio = Math.min(1, Math.max(0, event.nativeEvent.locationX / barWidthRef.current));
                  seekTo(ratio * durationMs);
                }}
                style={styles.timelineTouch}
              >
                <View style={styles.timelineTrack}>
                  <View
                    style={[
                      styles.timelineFill,
                      { width: durationMs > 0 ? `${Math.min(100, (positionMs / durationMs) * 100)}%` : "0%" },
                    ]}
                  />
                </View>
              </Pressable>
              <Text style={styles.time}>{formatClock(durationMs)}</Text>
            </View>

            <View style={styles.controls}>
              <Pressable accessibilityRole="button" accessibilityLabel="后退 2 秒"
                onPress={() => seekBy(-SEEK_STEP_MS)} style={({ pressed }) => [styles.controlBtn, pressed && styles.pressed]}>
                <Text style={styles.controlText}>−2s</Text>
              </Pressable>
              <Pressable accessibilityRole="button" accessibilityLabel={playing ? "暂停" : "播放"}
                onPress={() => setPlaying((value) => !value)} style={({ pressed }) => [styles.playBtn, pressed && styles.pressed]}>
                <Text style={styles.playText}>{playing ? "⏸" : "▶"}</Text>
              </Pressable>
              <Pressable accessibilityRole="button" accessibilityLabel="前进 2 秒"
                onPress={() => seekBy(SEEK_STEP_MS)} style={({ pressed }) => [styles.controlBtn, pressed && styles.pressed]}>
                <Text style={styles.controlText}>+2s</Text>
              </Pressable>
              <Pressable accessibilityRole="button" accessibilityLabel={`播放速度 ${rate}x`}
                onPress={() => {
                  const index = RATES.indexOf(rate);
                  setRate(RATES[(index + 1) % RATES.length]);
                }} style={({ pressed }) => [styles.controlBtn, pressed && styles.pressed]}>
                <Text style={styles.controlText}>{rate}x</Text>
              </Pressable>
              <Pressable accessibilityRole="button" accessibilityLabel={subtitlesVisible ? "关闭字幕" : "显示字幕"}
                onPress={() => setSubtitlesVisible((value) => !value)}
                style={({ pressed }) => [styles.controlBtn, subtitlesVisible && styles.controlBtnActive, pressed && styles.pressed]}>
                <Text style={[styles.controlText, subtitlesVisible && styles.controlTextActive]}>字幕</Text>
              </Pressable>
            </View>

            <View style={styles.rangeRow}>
              <Pressable accessibilityRole="button" accessibilityLabel="设 A 点为当前位置"
                onPress={() => {
                  setRangeStartMs(positionMs);
                  if (rangeEndMs != null) {
                    if (rangeEndMs <= positionMs) setRangeEndMs(null);
                    else setRangeLoop(true);
                  }
                }} style={({ pressed }) => [styles.rangeBtn, pressed && styles.pressed]}>
                <Text style={styles.rangeBtnText}>设 A</Text>
              </Pressable>
              <Pressable accessibilityRole="button" accessibilityLabel="设 B 点为当前位置"
                onPress={() => {
                  setRangeEndMs(positionMs);
                  if (rangeStartMs != null && rangeStartMs < positionMs) setRangeLoop(true);
                }} style={({ pressed }) => [styles.rangeBtn, pressed && styles.pressed]}>
                <Text style={styles.rangeBtnText}>设 B</Text>
              </Pressable>
              <Pressable accessibilityRole="button" accessibilityLabel={rangeLoop ? "关闭片段循环" : "循环所选片段"}
                disabled={!loopReady}
                onPress={() => setRangeLoop((value) => !value)}
                style={({ pressed }) => [styles.rangeBtn, !loopReady && styles.disabled, rangeLoop && styles.rangeBtnActive, pressed && styles.pressed]}>
                <Text style={[styles.rangeBtnText, rangeLoop && styles.rangeBtnTextActive]}>
                  {rangeLoop ? "循环中" : "循环片段"}
                </Text>
              </Pressable>
              <Pressable accessibilityRole="button" accessibilityLabel="清除 A B 片段"
                onPress={() => {
                  setRangeStartMs(null);
                  setRangeEndMs(null);
                  setRangeLoop(false);
                }} style={({ pressed }) => [styles.rangeBtn, pressed && styles.pressed]}>
                <Text style={styles.rangeBtnText}>清除</Text>
              </Pressable>
            </View>
            <Text style={styles.caption}>
              {loopReady
                ? `A ${formatClock(rangeStartMs ?? 0)} → B ${formatClock(rangeEndMs ?? 0)}`
                : "先设 A 和 B，再开启片段循环；设好 B 会自动开始循环。"}
            </Text>

            <View style={styles.row}>
              <Pressable accessibilityRole="button" disabled={busy}
                onPress={() => void importSubtitle()}
                style={({ pressed }) => [styles.secondaryBtn, busy && styles.disabled, pressed && styles.pressed]}>
                <Text style={styles.secondaryBtnText}>{busy ? "导入中…" : cues.length ? "更换字幕" : "导入字幕"}</Text>
              </Pressable>
              <Text style={styles.caption}>{cues.length ? `${cues.length} 句字幕` : "导入 SRT / VTT 后可显示字幕并参与句型搜索。"}</Text>
            </View>
          </View>
        ) : null}

        {cues.length > 0 && current ? (
          <Details title={`字幕 ${cues.length} 句`}>
            <SubtitleRows cues={cues} activeIndex={activeCueIndex} onSeek={seekTo} styles={styles} />
          </Details>
        ) : null}

        {videos.length > 0 ? (
          <Details title="最近打开">
            <View style={styles.panel}>
              {videos.map((video) => (
                <Pressable
                  key={video.id}
                  accessibilityRole="button"
                  onPress={() => openVideo(video)}
                  style={({ pressed }) => [styles.recentRow, pressed && styles.pressed]}
                >
                  <Text style={[styles.recentName, current?.id === video.id && styles.recentNameActive]} numberOfLines={1}>
                    {video.name}
                  </Text>
                  <Text style={styles.caption}>{video.subtitle_uri ? "有字幕" : "无字幕"}</Text>
                </Pressable>
              ))}
            </View>
          </Details>
        ) : null}
      </ScrollView>
    </View>
  );
}

// Memoized so 4 Hz playback progress does not re-render up to 200 rows; only
// a cue change or a new video updates the list.
const SubtitleRows = memo(function SubtitleRows({
  cues,
  activeIndex,
  onSeek,
  styles,
}: {
  cues: SubtitleCue[];
  activeIndex: number;
  onSeek: (ms: number) => void;
  styles: ReturnType<typeof makeStyles>;
}) {
  return (
    <View style={styles.subtitleList}>
      {cues.slice(0, LIST_LIMIT).map((cue, index) => {
        const lines = cueLanguages(cue.text);
        return (
          <Pressable
            key={cue.id}
            accessibilityRole="button"
            accessibilityLabel={`从 ${formatClock(cue.start * 1000)} 播放`}
            onPress={() => onSeek(cue.start * 1000)}
            style={({ pressed }) => [
              styles.subtitleRow,
              index === activeIndex && styles.subtitleRowActive,
              pressed && styles.pressed,
            ]}
          >
            <Text style={styles.subtitleTime}>{formatClock(cue.start * 1000)}</Text>
            <View style={styles.subtitleTextWrap}>
              <Text style={styles.subtitleRowText}>{lines.english || cue.text}</Text>
              {!!lines.translation && (
                <Text style={styles.subtitleRowTranslation}>{lines.translation}</Text>
              )}
            </View>
          </Pressable>
        );
      })}
      {cues.length > LIST_LIMIT ? (
        <Text style={styles.caption}>仅显示前 {LIST_LIMIT} 句，搜索仍覆盖全部字幕。</Text>
      ) : null}
    </View>
  );
});

function makeStyles(theme: ReturnType<typeof useAppTheme>["theme"]) {
  const ui = learningDesign(theme);
  return StyleSheet.create({
    safeArea: { flex: 1, backgroundColor: theme.bg },
    container: { flex: 1 },
    content: { gap: 14, ...ui.page },
    header: { ...ui.header, marginBottom: 0 },
    eyebrow: { color: theme.textMuted, fontSize: 11, fontWeight: "700", letterSpacing: 1.2 },
    title: { ...ui.title, marginTop: 2 },
    subtitle: { ...ui.subtitle },
    searchEntry: {
      alignSelf: "flex-start",
      marginTop: 10,
      paddingHorizontal: 12,
      paddingVertical: 7,
      borderRadius: 999,
      backgroundColor: theme.surfaceHover,
    },
    searchEntryText: { color: theme.textSecondary, fontSize: 13, fontWeight: "600" },
    panel: { ...ui.panel, gap: 12 },
    row: { flexDirection: "row", alignItems: "center", gap: 12 },
    caption: { flexShrink: 1, color: theme.textMuted, fontSize: 12, lineHeight: 18 },
    error: {
      color: theme.danger,
      backgroundColor: theme.surface,
      borderColor: theme.border,
      borderWidth: 1,
      borderRadius: 12,
      padding: 12,
      fontSize: 13,
    },
    notice: {
      color: theme.text,
      backgroundColor: theme.surfaceHover,
      borderRadius: 12,
      padding: 12,
      fontSize: 13,
    },
    primaryBtn: {
      ...ui.button,
      alignItems: "center",
      paddingHorizontal: 16,
      backgroundColor: theme.accent,
    },
    primaryBtnText: { color: "#fff", fontSize: 14, fontWeight: "700" },
    secondaryBtn: {
      ...ui.button,
      alignItems: "center",
      paddingHorizontal: 16,
      borderWidth: 1,
      borderColor: theme.border,
      backgroundColor: theme.surfaceHover,
    },
    secondaryBtnText: { color: theme.text, fontSize: 13, fontWeight: "600" },
    playerWrap: {
      width: "100%",
      aspectRatio: 16 / 9,
      borderRadius: 16,
      overflow: "hidden",
      backgroundColor: "#000",
    },
    player: { ...StyleSheet.absoluteFillObject },
    subtitleOverlay: {
      position: "absolute",
      left: 12,
      right: 12,
      bottom: 10,
      alignItems: "center",
      gap: 2,
    },
    subtitleEnglish: {
      color: "#fff",
      fontSize: 16,
      lineHeight: 22,
      fontWeight: "600",
      textAlign: "center",
      textShadowColor: "#000",
      textShadowRadius: 6,
    },
    subtitleChinese: {
      color: "#e9e2d4",
      fontSize: 13,
      lineHeight: 18,
      textAlign: "center",
      textShadowColor: "#000",
      textShadowRadius: 6,
    },
    timelineRow: { flexDirection: "row", alignItems: "center", gap: 10 },
    time: { color: theme.textMuted, fontSize: 11, fontVariant: ["tabular-nums"] },
    timelineTouch: { flex: 1, paddingVertical: 10 },
    timelineTrack: { height: 6, borderRadius: 999, backgroundColor: theme.border, overflow: "hidden" },
    timelineFill: { height: "100%", borderRadius: 999, backgroundColor: theme.accent },
    controls: {
      flexDirection: "row",
      flexWrap: "wrap",
      alignItems: "center",
      justifyContent: "space-between",
      gap: 8,
    },
    controlBtn: {
      minWidth: 52,
      minHeight: 40,
      alignItems: "center",
      justifyContent: "center",
      borderRadius: 12,
      borderWidth: 1,
      borderColor: theme.border,
      paddingHorizontal: 8,
    },
    controlBtnActive: { borderColor: theme.accent, backgroundColor: `${theme.accent}22` },
    controlText: { color: theme.text, fontSize: 13, fontWeight: "600" },
    controlTextActive: { color: theme.accent },
    playBtn: {
      width: 56,
      height: 46,
      alignItems: "center",
      justifyContent: "center",
      borderRadius: 14,
      backgroundColor: theme.accent,
    },
    playText: { color: "#fff", fontSize: 18, fontWeight: "700" },
    rangeRow: { flexDirection: "row", flexWrap: "wrap", gap: 8 },
    rangeBtn: {
      minHeight: 36,
      justifyContent: "center",
      paddingHorizontal: 12,
      borderRadius: 10,
      borderWidth: 1,
      borderColor: theme.border,
    },
    rangeBtnActive: { borderColor: theme.accent, backgroundColor: `${theme.accent}22` },
    rangeBtnText: { color: theme.textSecondary, fontSize: 12, fontWeight: "600" },
    rangeBtnTextActive: { color: theme.accent },
    disabled: { opacity: 0.45 },
    pressed: { opacity: 0.72 },
    subtitleList: { gap: 6 },
    subtitleRow: {
      flexDirection: "row",
      alignItems: "flex-start",
      gap: 10,
      padding: 10,
      borderRadius: 12,
      backgroundColor: theme.surface,
      borderWidth: 1,
      borderColor: theme.border,
    },
    subtitleRowActive: { borderColor: theme.accent, backgroundColor: `${theme.accent}12` },
    subtitleTime: { width: 44, color: theme.textMuted, fontSize: 11, fontVariant: ["tabular-nums"] },
    subtitleTextWrap: { flex: 1, gap: 2 },
    subtitleRowText: { color: theme.text, fontSize: 14, lineHeight: 20 },
    subtitleRowTranslation: { color: theme.textSecondary, fontSize: 12, lineHeight: 17 },
    recentRow: {
      minHeight: 44,
      flexDirection: "row",
      alignItems: "center",
      justifyContent: "space-between",
      gap: 10,
      paddingHorizontal: 10,
      borderRadius: 10,
    },
    recentName: { flex: 1, color: theme.text, fontSize: 13 },
    recentNameActive: { color: theme.accent, fontWeight: "600" },
  });
}
