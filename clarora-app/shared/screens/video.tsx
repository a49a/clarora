import { useAIChat } from "../ui/AIChatProvider";
import { Details } from "../ui/Details";
import { learningDesign } from "../ui/learningDesign";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  findNodeHandle,
  NativeModules,
  Platform,
  Pressable,
  ScrollView,
  StatusBar,
  StyleSheet,
  Text,
  TextInput,
  View,
  type NativeSyntheticEvent,
} from "react-native";
import { useAppTheme } from "../ui/ThemeContext";
import { getSetting, setSetting, saveVideoClip } from "../data/database";
import { DocumentPicker, FileSystem, runCommandArgs } from "../services/platform";
import { NativeVideoPlayer } from "../services/nativeVideo";

// English video learning is a desktop feature: the phone screen is too small
// for comfortable caption study, so Android never shows this tab.

const RECENT_VIDEOS_KEY = "recent_videos";
const RECENT_LIMIT = 12;
const RATES = [0.6, 0.8, 1.0, 1.25, 1.5];

// ffmpeg from Homebrew, same convention as the hardcoded /usr/bin/curl in
// services/platform.ts. Clips re-encode (veryfast) for frame-accurate cuts.
const FFMPEG_PATH = "/opt/homebrew/bin/ffmpeg";

type RecentVideo = { path: string; name: string };

type SubtitleTrack = {
  id: number;
  name: string;
  lang: string;
  codec: string;
  selected: boolean;
  textTrack: boolean;
  external: boolean;
  externalFilename: string;
  subStreamIndex: number;
};

type SubtitleTrackInfo = { tracks: SubtitleTrack[]; secondaryId: number };

type VideoControlModule = {
  seek?: (tag: number, ms: number) => void;
  setVolume?: (tag: number, volume: number) => void;
  getSubtitleTracks?: (tag: number) => Promise<SubtitleTrackInfo>;
  selectSubtitle?: (tag: number, trackId: number) => void;
  selectSecondarySubtitle?: (tag: number, trackId: number) => void;
};

function videoControl(): VideoControlModule | undefined {
  return NativeModules.RNVideoControl as VideoControlModule | undefined;
}

type RepeatMode = "off" | "one" | "range";

function formatClock(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return `${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
}

function videoName(path: string): string {
  return path.split("/").pop() || path;
}

export default function VideoLearningScreen() {
  const { visible: chatOpen } = useAIChat();
  const { theme, scheme } = useAppTheme();
  const styles = makeStyles(theme, scheme);

  const playerRef = useRef<any>(null);
  const trackWidthRef = useRef(0);
  const [recents, setRecents] = useState<RecentVideo[]>([]);
  const [currentPath, setCurrentPath] = useState<string | null>(null);
  const [playing, setPlaying] = useState(false);
  const [rate, setRate] = useState(1.0);
  const [positionMs, setPositionMs] = useState(0);
  const [durationMs, setDurationMs] = useState(0);
  const [repeatMode, setRepeatMode] = useState<RepeatMode>("off");
  const [loopStartMs, setLoopStartMs] = useState<number | null>(null);
  const [loopEndMs, setLoopEndMs] = useState<number | null>(null);
  const [subtitleTracks, setSubtitleTracks] = useState<SubtitleTrack[]>([]);
  const [primarySubtitleId, setPrimarySubtitleId] = useState<number>(0);
  const [secondarySubtitleId, setSecondarySubtitleId] = useState<number>(0);
  const [showSubtitlePicker, setShowSubtitlePicker] = useState(false);
  const [videoHover, setVideoHover] = useState(false);
  const [showSpeedStrip, setShowSpeedStrip] = useState(false);
  const [volume, setVolume] = useState(100);
  const volumeWidthRef = useRef(0);
  const lastVolumeRef = useRef(100);
  // Marks the current preview file as saved so the unmount/changelog cleanup
  // below never deletes a clip that a flashcard row now points at.
  const previewSavedRef = useRef(false);
  const [error, setError] = useState<string | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  const [clipFormVisible, setClipFormVisible] = useState(false);
  const [clipEn, setClipEn] = useState("");
  const [clipZh, setClipZh] = useState("");
  const [clipSaving, setClipSaving] = useState(false);
  const [previewUri, setPreviewUri] = useState<string | null>(null);
  const [previewPreparing, setPreviewPreparing] = useState(false);

  // IINA-style: the control bar lives on the video and shows on hover, but
  // stays pinned while paused or while any popup above the bar is open.
  const barVisible =
    videoHover ||
    !playing ||
    showSubtitlePicker ||
    showSpeedStrip ||
    clipFormVisible ||
    previewUri != null;

  // A clip needs a closed A→B range from the loop popup.
  const clipRangeOk =
    repeatMode === "range" && loopStartMs != null && loopEndMs != null && loopEndMs > loopStartMs;

  const showToast = useCallback((msg: string) => {
    setToast(msg);
    setTimeout(() => setToast(null), 2300);
  }, []);

  const stateRef = useRef({ repeatMode, loopStartMs, loopEndMs });
  stateRef.current = { repeatMode, loopStartMs, loopEndMs };

  useEffect(() => {
    getSetting(RECENT_VIDEOS_KEY)
      .then((value) => {
        if (!value) return;
        const parsed = JSON.parse(value);
        if (Array.isArray(parsed)) {
          const recents = parsed.slice(0, RECENT_LIMIT);
          setRecents(recents);
          // Resume the last video automatically, paused at the start, so
          // reopening the app never requires re-picking the file.
          if (recents[0]?.path) {
            setCurrentPath(recents[0].path);
          }
        }
      })
      .catch(() => {});
  }, []);

  const rememberRecent = useCallback((path: string) => {
    setRecents((current) => {
      const next = [
        { path, name: videoName(path) },
        ...current.filter((item) => item.path !== path),
      ].slice(0, RECENT_LIMIT);
      setSetting(RECENT_VIDEOS_KEY, JSON.stringify(next)).catch(() => {});
      return next;
    });
  }, []);

  const openVideo = useCallback(
    (path: string) => {
      if (previewUri) void FileSystem.deleteAsync(previewUri).catch(() => {});
      setPreviewUri(null);
      setClipFormVisible(false);
      setCurrentPath(path);
      setPositionMs(0);
      setDurationMs(0);
      setLoopStartMs(null);
      setLoopEndMs(null);
      setRepeatMode("off");
      setShowSubtitlePicker(false);
      setPlaying(true);
      rememberRecent(path);
    },
    [rememberRecent, previewUri]
  );

  const pickVideo = useCallback(async () => {
    setError(null);
    try {
      const result = await DocumentPicker.getDocumentAsync({ type: "video/*" });
      const file = result.assets?.[0];
      if (result.canceled || !file) return;
      openVideo(file.uri);
    } catch (e: any) {
      setError(String(e?.message ?? e));
    }
  }, [openVideo]);

  const seekTo = useCallback((ms: number) => {
    const tag = findNodeHandle(playerRef.current);
    if (tag == null) return;
    videoControl()?.seek?.(tag, ms);
  }, []);

  const toggleSubtitlePicker = useCallback(async () => {
    if (showSubtitlePicker) {
      setShowSubtitlePicker(false);
      return;
    }
    const tag = findNodeHandle(playerRef.current);
    if (tag == null) return;
    try {
      const info = await videoControl()?.getSubtitleTracks?.(tag);
      const tracks = Array.isArray(info?.tracks) ? info.tracks : [];
      setSubtitleTracks(tracks);
      setPrimarySubtitleId(tracks.find((t) => t.selected)?.id ?? 0);
      setSecondarySubtitleId(info?.secondaryId ?? 0);
    } catch {
      setSubtitleTracks([]);
    }
    setShowSubtitlePicker(true);
  }, [showSubtitlePicker]);

  const pickSubtitle = useCallback((trackId: number) => {
    const tag = findNodeHandle(playerRef.current);
    if (tag == null) return;
    videoControl()?.selectSubtitle?.(tag, trackId);
    setPrimarySubtitleId(trackId);
  }, []);

  const pickSecondarySubtitle = useCallback((trackId: number) => {
    const tag = findNodeHandle(playerRef.current);
    if (tag == null) return;
    videoControl()?.selectSecondarySubtitle?.(tag, trackId);
    setSecondarySubtitleId(trackId);
  }, []);

  const applyVolume = useCallback((value: number) => {
    const clamped = Math.max(0, Math.min(100, value));
    setVolume(clamped);
    if (clamped > 0) lastVolumeRef.current = clamped;
    const tag = findNodeHandle(playerRef.current);
    if (tag == null) return;
    videoControl()?.setVolume?.(tag, clamped);
  }, []);

  const toggleMute = useCallback(() => {
    applyVolume(volume > 0 ? 0 : lastVolumeRef.current || 100);
  }, [applyVolume, volume]);

  const seekBy = useCallback(
    (deltaMs: number) => {
      const target = positionMsRef.current + deltaMs;
      seekTo(durationMs > 0 ? Math.max(0, Math.min(durationMs, target)) : Math.max(0, target));
    },
    [durationMs, seekTo]
  );

  // Keyboard transport, mirroring the listening screen: Space toggles
  // playback, ←/→ seek five seconds, ↑/↓ step the volume. The native
  // monitor only queues plain presses, so system shortcuts pass through.
  const volumeRef = useRef(100);
  volumeRef.current = volume;

  useEffect(() => {
    if (!currentPath || chatOpen) return;
    const keyboard = NativeModules.RNKeyboard as
      | {
          startListening?: () => void;
          getNextKey?: () => Promise<string | null>;
          stopListening?: () => void;
        }
      | undefined;
    if (!keyboard?.startListening || !keyboard?.getNextKey || !keyboard.stopListening) return;
    let cancelled = false;
    keyboard.startListening();
    const pump = async () => {
      while (!cancelled) {
        const key = await keyboard.getNextKey!();
        if (cancelled || key == null) break;
        if (key === " " || key === "Space" || key === "Spacebar") {
          setPlaying((current) => !current);
        } else if (key === "ArrowLeft" || key === "Left") {
          seekBy(-5000);
        } else if (key === "ArrowRight" || key === "Right") {
          seekBy(5000);
        } else if (key === "ArrowUp" || key === "Up") {
          applyVolume(volumeRef.current + 10);
        } else if (key === "ArrowDown" || key === "Down") {
          applyVolume(volumeRef.current - 10);
        }
      }
    };
    void pump();
    return () => {
      cancelled = true;
      keyboard.stopListening?.();
    };
  }, [chatOpen, currentPath, seekBy, applyVolume]);

  const handleProgress = useCallback(
    (e: NativeSyntheticEvent<{ positionMs: number; durationMs: number }>) => {
      const { positionMs: pos, durationMs: dur } = e.nativeEvent;
      setPositionMs(pos);
      setDurationMs(dur);
      const { repeatMode: mode, loopStartMs: a, loopEndMs: b } = stateRef.current;
      if (mode === "range" && a != null && b != null && b > a && pos >= b) {
        seekTo(a);
      }
    },
    [seekTo]
  );

  const handleEnd = useCallback(() => {
    // A pending preview loops until the user saves or discards it.
    if (previewUri) {
      seekTo(0);
      setPlaying(true);
      return;
    }
    const { repeatMode: mode } = stateRef.current;
    if (mode === "one") {
      seekTo(0);
      setPlaying(true);
    } else {
      setPlaying(false);
    }
  }, [previewUri, seekTo]);

  const handleVideoError = useCallback((e: NativeSyntheticEvent<{ message: string }>) => {
    setError(e.nativeEvent.message.trim());
    setPlaying(false);
  }, []);

  const cycleRepeat = useCallback(() => {
    setRepeatMode((current) =>
      current === "off" ? "one" : current === "one" ? "range" : "off"
    );
  }, []);

  const positionMsRef = useRef(0);
  positionMsRef.current = positionMs;

  const setLoopPoint = useCallback((which: "A" | "B") => {
    if (which === "A") setLoopStartMs(positionMsRef.current);
    else setLoopEndMs(positionMsRef.current);
  }, []);

  const clearLoop = useCallback(() => {
    setLoopStartMs(null);
    setLoopEndMs(null);
    setRepeatMode("off");
  }, []);

  // Cut the A→B range into a self-contained mp4 under the app's videos dir
  // (frame-accurate via re-encode) so the user can preview it before saving.
  // Subtitle streams ride along as mov_text so the card keeps them; an
  // external text sub selected in the player is embedded with its timestamps
  // shifted by the clip start. VobSub bitmaps cannot be muxed into mp4.
  const startPreview = useCallback(async () => {
    if (!clipRangeOk || !currentPath || previewPreparing) return;
    setPreviewPreparing(true);
    previewSavedRef.current = false;
    try {
      const docs = await FileSystem.getDocumentDirectoryAsync();
      const dir = `${docs}videos`;
      try {
        await FileSystem.makeDirectoryAsync(dir);
      } catch {}
      const dst = `${dir}/${Date.now()}.mp4`;
      if (Platform.OS === "windows") {
        const exporter = NativeModules.RNVideoControl?.exportClip;
        if (!exporter) throw new Error("Windows 视频截取模块未加载");
        await exporter(currentPath, dst, loopStartMs!, loopEndMs!);
        previewSavedRef.current = true;
        setPreviewUri(dst);
        setPlaying(true);
        return;
      }
      const args = ["-y", "-ss", String(loopStartMs! / 1000), "-to", String(loopEndMs! / 1000), "-i", currentPath];

      const tag = findNodeHandle(playerRef.current);
      const info = tag != null ? await videoControl()?.getSubtitleTracks?.(tag) : undefined;
      const tracks = Array.isArray(info?.tracks) ? info!.tracks : [];
      const selectedTrack = tracks.find((t) => t.selected);
      const extTextSub =
        selectedTrack?.external &&
        selectedTrack.externalFilename.toLowerCase().endsWith(".srt")
          ? selectedTrack.externalFilename
          : null;
      if (extTextSub) {
        args.push("-itsoffset", String(loopStartMs! / 1000), "-i", extTextSub);
      }
      args.push("-map", "0:v", "-map", "0:a", "-map", "0:s?");
      if (extTextSub) args.push("-map", "1:s?");
      args.push(
        "-map_metadata:s:s", "0:s",
        "-c:v", "libx264",
        "-preset", "veryfast",
        "-crf", "20",
        "-c:a", "aac",
        "-c:s", "mov_text",
        "-movflags", "+faststart",
        dst
      );
      await runCommandArgs(FFMPEG_PATH, args);
      previewSavedRef.current = true;
      setPreviewUri(dst);
      setPlaying(true);
    } catch (e: any) {
      setError(`截取失败：${e?.message ?? e}`);
    } finally {
      setPreviewPreparing(false);
    }
  }, [clipRangeOk, currentPath, previewPreparing, loopStartMs, loopEndMs]);

  const discardPreview = useCallback(() => {
    if (previewUri) void FileSystem.deleteAsync(previewUri).catch(() => {});
    setPreviewUri(null);
  }, [previewUri]);

  const openSaveForm = useCallback(() => {
    setClipEn("");
    setClipZh("");
    setClipFormVisible(true);
  }, []);

  const confirmSaveClip = useCallback(async () => {
    if (!previewUri || clipSaving) return;
    setClipSaving(true);
    try {
      await saveVideoClip({ enText: clipEn.trim(), zhText: clipZh.trim(), videoUri: previewUri });
      setClipFormVisible(false);
      setPreviewUri(null);
      setClipSaving(false);
      setLoopStartMs(null);
      setLoopEndMs(null);
      setRepeatMode("off");
      showToast("已存入视频闪卡，复习页可查看");
    } catch (e: any) {
      setClipSaving(false);
      setError(`保存失败：${e?.message ?? e}`);
    }
  }, [previewUri, clipSaving, clipEn, clipZh, showToast]);

  // Drop an unsaved preview file when leaving the screen. The saved flag
  // guards against deleting a clip that a flashcard now references.
  useEffect(() => {
    return () => {
      if (previewUri && !previewSavedRef.current) {
        void FileSystem.deleteAsync(previewUri).catch(() => {});
      }
    };
  }, [previewUri]);

  if (Platform.OS !== "macos" && Platform.OS !== "windows") {
    return (
      <View style={styles.safeArea}>
        <StatusBar barStyle={scheme === "dark" ? "light-content" : "dark-content"} />
        <View style={styles.centered}>
          <Text style={styles.emptyTitle}>视频学习仅在 Mac 端提供</Text>
          <Text style={styles.emptyText}>手机屏幕不适合视频精听，请在 Mac 上使用。</Text>
        </View>
      </View>
    );
  }

  return (
    <View style={styles.safeArea}>
      <StatusBar barStyle={scheme === "dark" ? "light-content" : "dark-content"} />
      <ScrollView style={styles.container} contentContainerStyle={styles.content} showsVerticalScrollIndicator={false}>
        <View style={styles.header}>
          <Text style={styles.eyebrow}>VIDEO LEARNING</Text>
          <Text style={styles.title}>视频学习</Text>
          <Text style={styles.subtitle}>播放英文视频，配合倍速与片段循环做精听。</Text>
        </View>

        {recents.length > 0 && (
          <Details title="最近打开">
            <View style={styles.panel}>

            {recents.map((item) => (
              <Pressable
                key={item.path}
                style={({ pressed }) => [styles.recentItem, pressed && styles.recentItemPressed]}
                onPress={() => openVideo(item.path)}
              >
                <Text style={styles.recentName} numberOfLines={1}>
                  {item.name}
                </Text>
              </Pressable>
            ))}
          </View>
          </Details>
        )}

        <View style={styles.panel}>
          <View style={styles.libraryRow}>
            <Pressable style={({ pressed }) => [styles.pickBtn, pressed && styles.btnPressed]} onPress={() => { void pickVideo(); }}>
              <Text style={styles.pickBtnText}>选择视频</Text>
            </Pressable>
            <Text style={styles.libraryCaption}>
              {currentPath ? videoName(currentPath) : "支持本地视频文件，最近打开的会列在下方。"}
            </Text>
          </View>

          {currentPath && NativeVideoPlayer ? (
            <View
              style={styles.playerWrap}
              onMouseEnter={() => setVideoHover(true)}
              onMouseLeave={() => setVideoHover(false)}
            >
              <NativeVideoPlayer
                ref={playerRef}
                style={styles.player}
                src={previewUri ?? currentPath}
                playing={playing}
                rate={rate}
                onProgress={handleProgress}
                onEnd={handleEnd}
                onError={handleVideoError}
              />
              <Pressable
                style={styles.videoClickArea}
                onPress={() => setPlaying((current) => !current)}
              />

              {/* One always-mounted overlay sibling: toggling opacity keeps
                  the layer tree stable — inserting/removing views next to the
                  GL layer makes CAOpenGLLayer drop its output to black. */}
              <View
                style={[styles.overlayRoot, { opacity: barVisible ? 1 : 0 }]}
                pointerEvents={barVisible ? "box-none" : "none"}
              >
                {showSpeedStrip && (
                    <View style={styles.overlayPopup}>
                      <View style={styles.speedStripRow}>
                        {RATES.map((value) => (
                          <Pressable
                            key={value}
                            style={[styles.speedBtn, rate === value && styles.speedBtnActive]}
                            onPress={() => { setRate(value); setShowSpeedStrip(false); }}
                          >
                            <Text style={[styles.speedBtnText, rate === value && styles.speedBtnTextActive]}>
                              {value}x
                            </Text>
                          </Pressable>
                        ))}
                      </View>
                    </View>
                  )}

                  {repeatMode === "range" && !clipFormVisible && previewUri == null && (
                    <View style={styles.overlayPopup}>
                      <View style={styles.speedStripRow}>
                        <Pressable style={[styles.loopBtn, loopStartMs != null && styles.loopBtnActive]} onPress={() => setLoopPoint("A")}>
                          <Text style={[styles.loopBtnText, loopStartMs != null && styles.loopBtnTextActive]}>
                            {loopStartMs != null ? `A ${formatClock(loopStartMs)}` : "设为 A 点"}
                          </Text>
                        </Pressable>
                        <Pressable style={[styles.loopBtn, loopEndMs != null && styles.loopBtnActive]} onPress={() => setLoopPoint("B")}>
                          <Text style={[styles.loopBtnText, loopEndMs != null && styles.loopBtnTextActive]}>
                            {loopEndMs != null ? `B ${formatClock(loopEndMs)}` : "设为 B 点"}
                          </Text>
                        </Pressable>
                        <Pressable style={styles.loopBtn} onPress={clearLoop}>
                          <Text style={styles.loopBtnText}>清除</Text>
                        </Pressable>
                        <Pressable
                          style={[styles.loopBtn, clipRangeOk && styles.saveClipBtn]}
                          onPress={() => {
                            if (clipRangeOk) {
                              void startPreview();
                              return;
                            }
                            showToast(
                              loopStartMs == null && loopEndMs == null
                                ? "先点「设为 A 点」和「设为 B 点」框出片段"
                                : loopStartMs == null
                                ? "还差 A 点：拖到片段开头点「设为 A 点」"
                                : "还差 B 点：拖到片段结尾点「设为 B 点」"
                            );
                          }}
                        >
                          <Text style={[styles.loopBtnText, clipRangeOk && styles.saveClipBtnText]}>
                            {previewPreparing ? "生成预览…" : "预览并保存"}
                          </Text>
                        </Pressable>
                      </View>
                    </View>
                  )}

                  {previewUri != null && !clipFormVisible && (
                    <View style={styles.overlayPopup}>
                      <Text style={styles.clipFormRange}>
                        预览中：片段 {formatClock(loopStartMs ?? 0)} → {formatClock(loopEndMs ?? 0)}（循环播放，含字幕）
                      </Text>
                      <View style={styles.speedStripRow}>
                        <Pressable style={styles.pickBtn} onPress={openSaveForm}>
                          <Text style={styles.pickBtnText}>保存为闪卡</Text>
                        </Pressable>
                        <Pressable style={styles.loopBtn} onPress={discardPreview}>
                          <Text style={styles.loopBtnText}>重新截取</Text>
                        </Pressable>
                      </View>
                    </View>
                  )}

                  {clipFormVisible && (
                    <View style={[styles.overlayPopup, styles.clipForm]}>
                      <Text style={styles.subtitleSectionLabel}>视频闪卡</Text>
                      <Text style={styles.clipFormRange}>
                        片段 {formatClock(loopStartMs ?? 0)} → {formatClock(loopEndMs ?? 0)}
                      </Text>
                      <TextInput
                        style={styles.clipInput}
                        value={clipEn}
                        onChangeText={setClipEn}
                        placeholder="英文（可选）"
                        placeholderTextColor={theme.textMuted}
                      />
                      <TextInput
                        style={styles.clipInput}
                        value={clipZh}
                        onChangeText={setClipZh}
                        placeholder="中文（可选）"
                        placeholderTextColor={theme.textMuted}
                      />
                      <View style={styles.speedStripRow}>
                        <Pressable
                          style={[styles.pickBtn, clipSaving && styles.btnPressed]}
                          onPress={() => { void confirmSaveClip(); }}
                        >
                          <Text style={styles.pickBtnText}>{clipSaving ? "截取中…" : "保存"}</Text>
                        </Pressable>
                        <Pressable style={styles.loopBtn} onPress={() => setClipFormVisible(false)}>
                          <Text style={styles.loopBtnText}>取消</Text>
                        </Pressable>
                      </View>
                    </View>
                  )}

                  {showSubtitlePicker && (
                    <View style={[styles.overlayPopup, styles.subtitlePopup]}>
                      <Text style={styles.subtitleSectionLabel}>主字幕（底部）</Text>
                      {subtitleTracks.map((track) => (
                        <Pressable key={track.id} style={styles.subtitleRow} onPress={() => pickSubtitle(track.id)}>
                          <Text
                            style={[styles.subtitleRowText, primarySubtitleId === track.id && styles.subtitleRowTextActive]}
                            numberOfLines={1}
                          >
                            {primarySubtitleId === track.id ? "✓ " : ""}{track.name}
                          </Text>
                        </Pressable>
                      ))}
                      <Pressable style={styles.subtitleRow} onPress={() => pickSubtitle(0)}>
                        <Text
                          style={[styles.subtitleRowText, primarySubtitleId === 0 && styles.subtitleRowTextActive]}
                        >
                          {primarySubtitleId === 0 ? "✓ " : ""}关闭字幕
                        </Text>
                      </Pressable>

                      {Platform.OS === "macos" && <>
                      <Text style={styles.subtitleSectionLabel}>顶部副字幕（双语同显，仅文本字幕）</Text>
                      <Pressable style={styles.subtitleRow} onPress={() => pickSecondarySubtitle(0)}>
                        <Text
                          style={[styles.subtitleRowText, secondarySubtitleId === 0 && styles.subtitleRowTextActive]}
                        >
                          {secondarySubtitleId === 0 ? "✓ " : ""}关闭
                        </Text>
                      </Pressable>
                      {subtitleTracks
                        .filter((track) => track.textTrack)
                        .map((track) => (
                          <Pressable key={track.id} style={styles.subtitleRow} onPress={() => pickSecondarySubtitle(track.id)}>
                            <Text
                              style={[styles.subtitleRowText, secondarySubtitleId === track.id && styles.subtitleRowTextActive]}
                              numberOfLines={1}
                            >
                              {secondarySubtitleId === track.id ? "✓ " : ""}{track.name}
                            </Text>
                          </Pressable>
                        ))}

                      </>}
                      <Pressable style={styles.subtitleDoneRow} onPress={() => setShowSubtitlePicker(false)}>
                        <Text style={styles.subtitleDoneText}>完成</Text>
                      </Pressable>
                    </View>
                  )}

                  <View style={styles.controlOverlay}>
                    <View style={styles.barRow}>
                      <View style={styles.barGroup}>
                        <Pressable style={styles.barBtn} onPress={toggleMute}>
                          <Text style={styles.barText}>{volume > 0 ? "🔊" : "🔇"}</Text>
                        </Pressable>
                        <Pressable
                          style={styles.volumeTrack}
                          onLayout={(e) => { volumeWidthRef.current = e.nativeEvent.layout.width; }}
                          onPress={(e) => {
                            const width = volumeWidthRef.current;
                            if (!width) return;
                            applyVolume(Math.round(Math.max(0, Math.min(1, e.nativeEvent.locationX / width)) * 100));
                          }}
                        >
                          <View style={[styles.volumeFill, { width: `${volume}%` }]} />
                        </Pressable>
                        <Pressable style={styles.barBtn} onPress={() => setShowSpeedStrip((current) => !current)}>
                          <Text style={styles.barText}>{rate}x</Text>
                        </Pressable>
                      </View>

                      <View style={styles.barGroup}>
                        <Pressable style={styles.barBtn} onPress={() => seekBy(-5000)}>
                          <Text style={styles.barText}>⏪</Text>
                        </Pressable>
                        <Pressable style={styles.barBtn} onPress={() => setPlaying((current) => !current)}>
                          <Text style={styles.barText}>{playing ? "⏸" : "▶"}</Text>
                        </Pressable>
                        <Pressable style={styles.barBtn} onPress={() => seekBy(5000)}>
                          <Text style={styles.barText}>⏩</Text>
                        </Pressable>
                      </View>

                      <View style={[styles.barGroup, styles.barGroupEnd]}>
                        <Pressable
                          style={[styles.barBtn, repeatMode !== "off" && styles.barBtnActive]}
                          onPress={cycleRepeat}
                        >
                          <Text style={[styles.barText, repeatMode !== "off" && styles.barTextActive]}>
                            {repeatMode === "off" ? "循环" : repeatMode === "one" ? "单篇" : "片段"}
                          </Text>
                        </Pressable>
                        <Pressable
                          style={[styles.barBtn, showSubtitlePicker && styles.barBtnActive]}
                          onPress={() => { void toggleSubtitlePicker(); }}
                        >
                          <Text style={[styles.barText, showSubtitlePicker && styles.barTextActive]}>字幕</Text>
                        </Pressable>
                      </View>
                    </View>

                    <View style={styles.barRow}>
                      <Text style={styles.barText}>{formatClock(positionMs)}</Text>
                      <Pressable
                        style={styles.seekTrack}
                        onLayout={(e) => { trackWidthRef.current = e.nativeEvent.layout.width; }}
                        onPress={(e) => {
                          const width = trackWidthRef.current;
                          if (!width || !durationMs) return;
                          const ratio = Math.max(0, Math.min(1, e.nativeEvent.locationX / width));
                          seekTo(ratio * durationMs);
                        }}
                      >
                        <View style={styles.seekTrackBg}>
                          <View
                            style={[styles.seekFill, { width: durationMs > 0 ? `${(positionMs / durationMs) * 100}%` : "0%" }]}
                          />
                        </View>
                      </Pressable>
                      <Text style={styles.barText}>
                        -{formatClock(Math.max(0, durationMs - positionMs))}
                      </Text>
                    </View>
                  </View>
              </View>
            </View>
          ) : (
            <View style={styles.playerPlaceholder}>
              <Text style={styles.playerPlaceholderText}>选择一个视频开始学习</Text>
            </View>
          )}

          {error ? (
            <View style={styles.errorBanner}>
              <Text style={styles.errorText}>{error}</Text>
            </View>
          ) : null}
          {toast ? (
            <View style={styles.errorBanner}>
              <Text style={[styles.errorText, { color: theme.accent }]}>{toast}</Text>
            </View>
          ) : null}
        </View>
      </ScrollView>
    </View>
  );
}

function makeStyles(
  theme: ReturnType<typeof useAppTheme>["theme"],
  scheme: "light" | "dark" | null | undefined
) {
  const dark = scheme === "dark";
  const overlayBg = dark ? "rgba(28,28,30,0.94)" : "rgba(248,248,250,0.97)";
  const barBg = dark ? "rgba(28,28,30,0.9)" : "rgba(246,246,248,0.95)";
  const barFg = dark ? "#f2f2f4" : theme.text;
  const barBtnActiveBg = dark ? "rgba(255,255,255,0.16)" : "rgba(0,0,0,0.08)";
  const trackBg = dark ? "rgba(255,255,255,0.28)" : "rgba(0,0,0,0.18)";
  const ui = learningDesign(theme);
  return StyleSheet.create({
    safeArea: { flex: 1, backgroundColor: theme.bg },
    container: { flex: 1 },
    content: {
      gap: 18,
      ...ui.page,
    },
    centered: { flex: 1, justifyContent: "center", alignItems: "center", padding: 32 },
    emptyTitle: { color: theme.text, fontSize: 18, fontWeight: "700" },
    emptyText: { color: theme.textSecondary, fontSize: 13, marginTop: 8, textAlign: "center" },

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

    panel: {
      gap: 12,
      ...ui.panel,
    },
    panelLabel: {
      ...ui.label,
    },

    libraryRow: { flexDirection: "row", alignItems: "center", gap: 12 },
    pickBtn: {
      paddingVertical: 9,
      paddingHorizontal: 16,
      backgroundColor: theme.accent,
      ...ui.button,
    },
    pickBtnText: { color: "#fff", fontSize: 13, fontWeight: "700" },
    libraryCaption: { color: theme.textSecondary, fontSize: 12, flex: 1 },
    btnPressed: { opacity: 0.8 },

    playerWrap: {
      borderRadius: 8,
      overflow: "hidden",
      backgroundColor: "#000",
      aspectRatio: 16 / 9,
    },
    player: { flex: 1 },
    playerPlaceholder: {
      borderRadius: 8,
      borderWidth: 1,
      borderStyle: "dashed",
      borderColor: theme.border,
      aspectRatio: 16 / 9,
      alignItems: "center",
      justifyContent: "center",
      backgroundColor: theme.bg,
    },
    playerPlaceholderText: { color: theme.textMuted, fontSize: 13 },

    videoClickArea: { position: "absolute", left: 0, right: 0, top: 0, bottom: 0 },
    overlayRoot: { position: "absolute", left: 0, right: 0, top: 0, bottom: 0 },
    overlayPopup: {
      position: "absolute",
      left: 10,
      right: 10,
      bottom: 70,
      borderRadius: 10,
      padding: 8,
      backgroundColor: overlayBg,
    },
    subtitlePopup: { maxHeight: "80%" },
    speedStripRow: { flexDirection: "row", flexWrap: "wrap", alignItems: "center", gap: 6 },
    controlOverlay: {
      position: "absolute",
      left: 10,
      right: 10,
      bottom: 10,
      borderRadius: 10,
      paddingHorizontal: 12,
      paddingTop: 6,
      paddingBottom: 4,
      gap: 2,
      backgroundColor: barBg,
    },
    barRow: { flexDirection: "row", alignItems: "center", gap: 8 },
    barGroup: { flexDirection: "row", alignItems: "center", gap: 4, flex: 1 },
    barGroupEnd: { justifyContent: "flex-end" },
    barBtn: { paddingVertical: 4, paddingHorizontal: 8, borderRadius: 6 },
    barBtnActive: { backgroundColor: barBtnActiveBg },
    barText: { color: barFg, fontSize: 12, fontVariant: ["tabular-nums"] },
    barTextActive: { color: theme.accent, fontWeight: "700" },
    volumeTrack: {
      width: 64,
      height: 8,
      borderRadius: 4,
      backgroundColor: trackBg,
      overflow: "hidden",
    },
    volumeFill: { height: "100%", backgroundColor: "#4d8df7" },
    seekTrack: { flex: 1, height: 16, justifyContent: "center" },
    seekTrackBg: {
      height: 5,
      borderRadius: 2.5,
      backgroundColor: trackBg,
      overflow: "hidden",
    },
    seekFill: { height: "100%", backgroundColor: theme.accent },

    speedBtn: {
      paddingVertical: 6,
      paddingHorizontal: 10,
      borderWidth: 1,
      borderColor: theme.border,
      ...ui.button,
    },
    speedBtnActive: { backgroundColor: theme.accent, borderColor: theme.accent },
    speedBtnText: { color: theme.textSecondary, fontSize: 12 },
    speedBtnTextActive: { color: "#fff", fontWeight: "700" },

    loopBtn: {
      paddingVertical: 7,
      paddingHorizontal: 12,
      borderWidth: 1,
      borderColor: theme.border,
      backgroundColor: theme.bg,
      ...ui.button,
    },
    loopBtnActive: { borderColor: theme.accent, backgroundColor: `${theme.accent}20` },
    loopBtnText: { color: theme.textSecondary, fontSize: 12, fontVariant: ["tabular-nums"] },
    loopBtnTextActive: { color: theme.accent, fontWeight: "700" },

    saveClipBtn: { borderColor: theme.accent },
    saveClipBtnText: { color: theme.accent, fontWeight: "700" },
    clipForm: { gap: 8 },
    clipFormRange: { color: theme.textSecondary, fontSize: 12 },
    clipInput: {
      borderWidth: 1,
      borderColor: theme.border,
      borderRadius: 8,
      paddingHorizontal: 10,
      paddingVertical: 8,
      color: theme.text,
      fontSize: 13,
      backgroundColor: theme.bg,
    },
    subtitleSectionLabel: {
      color: theme.textMuted,
      fontSize: 11,
      fontWeight: "700",
      marginTop: 6,
      paddingHorizontal: 10,
    },
    subtitleRow: { paddingVertical: 8, paddingHorizontal: 10, borderRadius: 6 },
    subtitleRowText: { color: theme.textSecondary, fontSize: 12 },
    subtitleRowTextActive: { color: theme.accent, fontWeight: "700" },
    subtitleDoneRow: { alignSelf: "flex-end", paddingVertical: 8, paddingHorizontal: 12 },
    subtitleDoneText: { color: theme.accent, fontSize: 12, fontWeight: "700" },

    errorBanner: {
      backgroundColor: `${theme.danger}14`,
      borderRadius: 6,
      paddingVertical: 10,
      paddingHorizontal: 14,
    },
    errorText: { color: theme.danger, fontSize: 13 },

    recentItem: {
      paddingVertical: 10,
      paddingHorizontal: 12,
      backgroundColor: theme.bg,
      borderWidth: 1,
      borderColor: theme.border,
      ...ui.row,
    },
    recentItemPressed: { opacity: 0.7 },
    recentName: { color: theme.text, fontSize: 13 },
  });
}
