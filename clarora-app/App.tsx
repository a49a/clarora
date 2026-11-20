import { AIChatProvider } from "./shared/ui/AIChatProvider";
import { type ReactNode, useCallback, useEffect, useState } from "react";
import { Platform, Pressable, ScrollView, StyleSheet, Text, TextInput, View } from "react-native";
import { SafeAreaProvider, SafeAreaView as SafeAreaContextView } from "react-native-safe-area-context";
import HomeScreen from "./shared/screens/index";
import DiscoverScreen from "./shared/screens/discover";
import ListeningScreen from "./shared/screens/listening";
import AudioManagerScreen from "./shared/screens/audioManager";
import GraphScreen from "./shared/screens/graph";
import MeditationScreen from "./shared/screens/meditation";
import ProfileScreen from "./shared/screens/profile";
import OcrScreen from "./shared/screens/ocr";
import PdfScreen from "./shared/screens/pdf";
import SpeakingScreen from "./shared/screens/speaking";
import SentenceSearchScreen from "./shared/screens/sentences";
import StatsScreen from "./shared/screens/stats";
import SettingsScreen from "./shared/screens/settings";
import VideoLearningScreen from "./shared/screens/video";
import { type Theme } from "./shared/ui/theme";
import { ThemeProvider, useAppTheme } from "./shared/ui/ThemeContext";
import { getSetting, setSetting } from "./shared/data/database";
import { PomodoroProvider, formatPomodoroClock, usePomodoro } from "./shared/ui/PomodoroContext";

const screens = [
  { key: "discover", label: "随便学学", detail: "Discover" },
  { key: "flashcard", label: "闪卡复习", detail: "Words" },
  { key: "listening", label: "音频学习", detail: "Focus" },
  { key: "speaking", label: "口语跟读", detail: "Speaking" },
  { key: "video", label: "视频学习", detail: "Video" },
  { key: "ocr", label: "拍照识字", detail: "OCR" },
  { key: "pdf", label: "文档阅读", detail: "PDF" },
  { key: "audio-manager", label: "音频管理", detail: "Library" },
  { key: "graph", label: "词汇表", detail: "Vocabulary" },
  { key: "stats", label: "学习统计", detail: "Stats" },
  { key: "meditation", label: "冥想放松", detail: "Mindfulness" },
  { key: "settings", label: "设置", detail: "Settings" },
  { key: "profile", label: "我的", detail: "Profile" },
] as const;

// Mobile clients keep only the learning surfaces; bulk library management and
// the PDF reader stay desktop-only. Discover and Speaking are hidden on phones
// for now while their mobile experience is reworked, and Vocabulary is hidden
// because the desktop client replaced it with the flashcard list. Screens and
// desktop entries remain, so narrowing the mobile scope never deletes code.
const mobileHiddenScreens = new Set([
  "audio-manager",
  "pdf",
  "discover",
  "speaking",
  "graph",
]);

type ScreenKey = (typeof screens)[number]["key"];

// Four fixed bottom tabs, following the daily-listening app pattern: three
// learning surfaces plus a profile hub for statistics, tools and settings.
// Icons are text glyphs so they follow the theme colour without an icon font.
const mobileTabs: Array<{ key: ScreenKey; label: string; icon: string; accessibilityLabel: string }> = [
  { key: "flashcard", label: "闪卡", icon: "▤", accessibilityLabel: "闪卡复习" },
  { key: "listening", label: "音频", icon: "♫", accessibilityLabel: "音频学习" },
  { key: "video", label: "视频", icon: "▷", accessibilityLabel: "视频学习" },
  { key: "profile", label: "我的", icon: "☰", accessibilityLabel: "我的" },
];
const mobileProfileKeys: ScreenKey[] = ["stats", "ocr", "meditation", "settings"];
const primaryKeys: ScreenKey[] = ["discover", "flashcard", "listening"];
const navigationGroups: Array<{ key: string; label: string; screens: ScreenKey[] }> = [
  { key: "practice", label: "专项练习", screens: ["speaking", "video"] },
  { key: "library", label: "资料库", screens: ["audio-manager", "pdf", "graph", "ocr"] },
  { key: "tools", label: "学习工具", screens: ["stats", "meditation"] },
];
type ListeningSelection = {
  practiceId: string | null;
  audioId: string | null;
  /** One-shot cue position opened from sentence search. */
  positionMs: number | null;
};

const LAST_ACTIVE_SCREEN_SETTING = "last_active_screen";

function Shell() {
  const { theme } = useAppTheme();
  const pomodoro = usePomodoro();
  const isMobile = Platform.OS === "android" || Platform.OS === "ios";
  const RootView = isMobile ? SafeAreaContextView : View;
  const [activeKey, setActiveKey] = useState<ScreenKey>("flashcard");
  const [navigationRestored, setNavigationRestored] = useState(false);
  const [listeningSelection, setListeningSelection] = useState<ListeningSelection>({
    practiceId: null,
    audioId: null,
    positionMs: null,
  });
  const [videoSelection, setVideoSelection] = useState<{ id: string; positionMs: number } | null>(null);
  const [sentenceOpen, setSentenceOpen] = useState(false);
  const styles = makeStyles(theme);
  const [customFocusInput, setCustomFocusInput] = useState("");
  const [expandedGroup, setExpandedGroup] = useState<string | null>(null);
  const [timerExpanded, setTimerExpanded] = useState(false);
  useEffect(() => {
    // Restoring a page or exiting focused listening reveals its location.
    setExpandedGroup(navigationGroups.find(group => group.screens.includes(activeKey))?.key ?? null);
  }, [activeKey]);
  const pomodoroStatus = pomodoro.paused
    ? "已暂停"
    : pomodoro.phase === "focus" ? "专注中" : "休息中";

  // Restore the last workspace as well as its audio selection. This keeps a
  // relaunch in focused listening study rather than dropping the learner back
  // onto the flashcard home screen.
  useEffect(() => {
    let cancelled = false;
    void getSetting(LAST_ACTIVE_SCREEN_SETTING)
      .then((saved) => {
        if (cancelled || !saved) return;
        // Never restore a screen the current platform does not show.
        if (isMobile && mobileHiddenScreens.has(saved)) return;
        if (screens.some((screen) => screen.key === saved)) {
          setActiveKey(saved as ScreenKey);
        }
      })
      .catch(() => {})
      .finally(() => {
        if (!cancelled) setNavigationRestored(true);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!navigationRestored) return;
    void setSetting(LAST_ACTIVE_SCREEN_SETTING, activeKey).catch(() => {});
  }, [activeKey, navigationRestored]);


  useEffect(() => {
    if (isMobile && mobileHiddenScreens.has(activeKey)) setActiveKey("flashcard");
  }, [activeKey, isMobile]);


  const rememberListeningSelection = useCallback(
    (practiceId: string | null, audioId: string | null) => {
      setListeningSelection((current) =>
        current.practiceId === practiceId && current.audioId === audioId
          ? current
          : { practiceId, audioId, positionMs: null }
      );
    },
    []
  );

  // Normal navigation clears one-shot cue positions; only sentence search and
  // deep links set them.
  const openScreen = useCallback((key: ScreenKey) => {
    setListeningSelection((current) =>
      current.positionMs == null ? current : { ...current, positionMs: null }
    );
    setVideoSelection(null);
    setActiveKey(key);
  }, []);

  const startListeningStudy = useCallback((practiceId: string, audioId: string) => {
    setListeningSelection({ practiceId, audioId, positionMs: null });
    setActiveKey("listening");
  }, []);

  // Opening search unmounts the current media screen. Drop the one-shot cue so
  // closing the search cannot replay the sentence position on remount.
  const openSentenceSearch = useCallback(() => {
    setListeningSelection((current) =>
      current.positionMs == null ? current : { ...current, positionMs: null }
    );
    setVideoSelection(null);
    setSentenceOpen(true);
  }, []);

  if (sentenceOpen) {
    return (
      <RootView style={styles.focusRoot}>
        <SentenceSearchScreen
          onClose={() => setSentenceOpen(false)}
          onOpenAudio={(practiceId, audioId, positionMs) => {
            setSentenceOpen(false);
            setVideoSelection(null);
            setListeningSelection({ practiceId, audioId, positionMs });
            setActiveKey("listening");
          }}
          onOpenVideo={(id, positionMs) => {
            setSentenceOpen(false);
            setListeningSelection((current) =>
              current.positionMs == null ? current : { ...current, positionMs: null }
            );
            setVideoSelection({ id, positionMs });
            setActiveKey("video");
          }}
        />
      </RootView>
    );
  }

  // Audio study is intentionally immersive: the global sidebar disappears
  // until the learner explicitly exits the session.
  if (activeKey === "listening") {
    return (
      <RootView style={styles.focusRoot}>
        <ListeningScreen
          key="listening-study"
          initialPracticeId={listeningSelection.practiceId}
          initialAudioId={listeningSelection.audioId}
          onSelectionChange={rememberListeningSelection}
          initialPositionMs={listeningSelection.positionMs}
          onOpenSentenceSearch={openSentenceSearch}
          onExitStudy={() => openScreen(isMobile ? "flashcard" : "audio-manager")}
          studySidebarStatus={pomodoro.running ? (
            <Text style={{ color: theme.textMuted, fontSize: 12, fontVariant: ['tabular-nums'] }}>
              {pomodoroStatus} · {formatPomodoroClock(pomodoro.remainingSec)}
            </Text>
          ) : null}
        />
      </RootView>
    );
  }

  let activeContent: ReactNode;
  switch (activeKey) {
    case "discover":
      activeContent = <DiscoverScreen onManageAudio={isMobile ? undefined : (practiceId, audioId) => {
        setListeningSelection({ practiceId, audioId, positionMs: null });
        setActiveKey("audio-manager");
      }} />;
      break;
    case "audio-manager":
      activeContent = (
        <AudioManagerScreen
          initialPracticeId={listeningSelection.practiceId}
          initialAudioId={listeningSelection.audioId}
          onSelectionChange={rememberListeningSelection}
          onStartStudy={startListeningStudy}
        />
      );
      break;
    case "graph":
      activeContent = <GraphScreen />;
      break;
    case "stats":
      activeContent = <StatsScreen />;
      break;
    case "video":
      activeContent = (
        <VideoLearningScreen
          initialVideoId={videoSelection?.id ?? null}
          initialPositionMs={videoSelection?.positionMs ?? null}
          onOpenSentenceSearch={openSentenceSearch}
        />
      );
      break;
    case "ocr":
      activeContent = <OcrScreen />;
      break;
    case "pdf":
      activeContent = <PdfScreen />;
      break;
    case "speaking":
      activeContent = <SpeakingScreen />;
      break;
    case "meditation":
      activeContent = <MeditationScreen />;
      break;
    case "settings":
      activeContent = <SettingsScreen />;
      break;
    case "profile":
      activeContent = (
        <ProfileScreen onOpen={openScreen} onOpenSentenceSearch={openSentenceSearch} />
      );
      break;
    default:
      activeContent = <HomeScreen />;
  }

  if (isMobile) {
    const mobileActiveTab: ScreenKey = mobileProfileKeys.includes(activeKey) ? "profile" : activeKey;
    return (
      <RootView style={styles.mobileRoot}>
        <View style={styles.mobileContent}>{activeContent}</View>
        <View style={styles.mobileNavShell}>
          <View style={styles.mobileTimer}>
            {pomodoro.running ? (
              <View style={styles.mobileTimerRow}>
                <View style={styles.mobileTimerInfo}>
                  <Text style={styles.mobileTimerLabel}>{pomodoroStatus}</Text>
                  <Text style={styles.mobileTimerClock}>
                    {formatPomodoroClock(pomodoro.remainingSec)}
                  </Text>
                </View>
                <View style={styles.mobileTimerActions}>
                  <Pressable style={styles.mobileTimerActionBtn} onPress={pomodoro.togglePause}
                    accessibilityRole="button" accessibilityLabel={pomodoro.paused ? "继续专注" : "暂停专注"}>
                    <Text style={styles.mobileTimerActionText}>{pomodoro.paused ? "继续" : "暂停"}</Text>
                  </Pressable>
                  <Pressable style={styles.mobileTimerActionBtn} onPress={pomodoro.reset}
                    accessibilityRole="button" accessibilityLabel="重置专注计时">
                    <Text style={styles.mobileTimerActionText}>重置</Text>
                  </Pressable>
                </View>
              </View>
            ) : (
              <>
                <View style={styles.mobileTimerRow}>
                  <Pressable
                    accessibilityRole="button"
                    accessibilityLabel={`番茄钟，专注 ${pomodoro.focusMin} 分钟`}
                    accessibilityState={{ expanded: timerExpanded }}
                    onPress={() => setTimerExpanded((value) => !value)}
                    style={({ pressed }) => [styles.mobileTimerInfo, pressed && styles.pressedTab]}
                  >
                    <Text style={styles.mobileTimerLabel}>⏱ 专注 {pomodoro.focusMin} 分钟</Text>
                    <Text style={styles.mobileTimerChevron}>{timerExpanded ? "⌄" : "›"}</Text>
                  </Pressable>
                  <Pressable
                    accessibilityRole="button"
                    accessibilityLabel="开始专注"
                    onPress={() => {
                      setTimerExpanded(false);
                      pomodoro.start();
                    }}
                    style={({ pressed }) => [styles.mobileTimerStart, pressed && styles.pressedTab]}
                  >
                    <Text style={styles.mobileTimerStartText}>▶ 开始</Text>
                  </Pressable>
                </View>
                {timerExpanded && (
                  <View style={styles.mobileTimerPresets}>
                    {[25, 45, 60].map((minutes) => (
                      <Pressable
                        key={minutes}
                        accessibilityRole="button"
                        accessibilityState={{ selected: pomodoro.focusMin === minutes }}
                        style={[styles.mobileTimerChip, pomodoro.focusMin === minutes && styles.mobileTimerChipActive]}
                        onPress={() => pomodoro.setFocusMin(minutes)}
                      >
                        <Text style={[styles.mobileTimerChipText, pomodoro.focusMin === minutes && styles.mobileTimerChipTextActive]}>
                          {minutes} 分
                        </Text>
                      </Pressable>
                    ))}
                  </View>
                )}
              </>
            )}
          </View>
          <View style={styles.mobileNav}>
            {mobileTabs.map((tab) => {
              const active = mobileActiveTab === tab.key;
              return (
                <Pressable
                  key={tab.key}
                  accessibilityRole="tab"
                  accessibilityLabel={tab.accessibilityLabel}
                  accessibilityState={{ selected: active }}
                  onPress={() => openScreen(tab.key)}
                  style={({ pressed }) => [styles.mobileTab, pressed && styles.pressedTab]}
                >
                  <Text style={[styles.mobileTabIcon, active && styles.mobileTabIconActive]}>
                    {tab.icon}
                  </Text>
                  <Text style={[styles.mobileTabText, active && styles.mobileTabTextActive]} numberOfLines={1}>
                    {tab.label}
                  </Text>
                </Pressable>
              );
            })}
          </View>
        </View>
      </RootView>
    );
  }

  const navigationItem = (key: ScreenKey, nested = false) => {
    const screen = screens.find(item => item.key === key)!;
    const active = activeKey === key;
    return <Pressable key={key} accessibilityRole="tab" accessibilityState={{ selected: active }}
      onPress={() => openScreen(key)}
      style={({ pressed }) => [styles.tab, nested && styles.nestedTab, active && styles.activeTab, pressed && styles.pressedTab]}>
      <View style={[styles.navMarker, active && styles.navMarkerActive]} />
      <Text style={[styles.tabText, active && styles.activeTabText]}>{screen.label}</Text>
    </Pressable>;
  };

  return (
    <View style={styles.root}>
      <View style={styles.sidebar}>
        <ScrollView showsVerticalScrollIndicator={false} contentContainerStyle={styles.sidebarContent}>
        <View>
          <View style={styles.brandRow}>
            <View style={styles.brandMark}><Text style={styles.brandMarkText}>C</Text></View>
            <View>
              <Text style={styles.brand}>Clarora</Text>
              <Text style={styles.brandDetail}>给学习留一点空间</Text>
            </View>
          </View>
          <Text style={styles.sectionLabel}>开始学习</Text>
          <View style={styles.navigation}>
            {primaryKeys.map(key => navigationItem(key))}
          </View>
          <View style={styles.groupList}>
            {navigationGroups.map(group => {
              const expanded = expandedGroup === group.key;
              const selected = group.screens.includes(activeKey);
              return <View key={group.key}>
                <Pressable accessibilityRole="button" accessibilityState={{ expanded }}
                  accessibilityLabel={`${group.label}，${expanded ? '收起' : '展开'}`}
                  onPress={() => setExpandedGroup(expanded ? null : group.key)}
                  style={({ pressed }) => [styles.groupHeading, pressed && styles.pressedTab]}>
                  <Text style={[styles.groupLabel, selected && styles.activeTabText]}>{group.label}</Text>
                  <Text style={styles.groupChevron}>{expanded ? '⌄' : '›'}</Text>
                </Pressable>
                {expanded && <View style={styles.groupChildren}>{group.screens.map(key => navigationItem(key, true))}</View>}
                {!expanded && selected && <Text style={styles.collapsedLocation}>{screens.find(screen => screen.key === activeKey)?.label}</Text>}
              </View>;
            })}
          </View>
        </View>
        <View style={styles.sidebarFooter}>
          <View style={styles.sidebarPomodoro}>
            <Pressable accessibilityRole="button" accessibilityState={{ expanded: timerExpanded }}
              onPress={() => setTimerExpanded(value => !value)} style={styles.timerHeader}>
              <Text style={styles.groupLabel}>专注计时</Text>
              <Text style={styles.groupChevron}>{timerExpanded ? '⌄' : '›'}</Text>
            </Pressable>
            {pomodoro.running ? (
              <>
                <Text
                  style={[
                    styles.pomodoroStatusTitle,
                    pomodoro.phase === "break" && { color: theme.sidebarAccent },
                    pomodoro.paused && { color: theme.textMuted },
                  ]}
                >
                  {pomodoroStatus}
                </Text>
                <Text style={styles.pomodoroStatusClock}>{formatPomodoroClock(pomodoro.remainingSec)}</Text>
                <View style={styles.pomodoroTrack}>
                  <View style={[styles.pomodoroFill, { width: `${Math.min(100, (1 - pomodoro.remainingSec / pomodoro.totalSec) * 100)}%` }]} />
                </View>
                <View style={styles.pomodoroActions}>
                  <Pressable style={styles.pomodoroActionBtn} onPress={pomodoro.togglePause}>
                    <Text style={styles.pomodoroActionText}>{pomodoro.paused ? "继续" : "暂停"}</Text>
                  </Pressable>
                  {timerExpanded && <Pressable style={styles.pomodoroActionBtn} onPress={pomodoro.reset}>
                    <Text style={styles.pomodoroActionText}>重置</Text>
                  </Pressable>}
                </View>
              </>
            ) : timerExpanded ? (
              <>
                <Text style={styles.pomodoroStatusTitle}>番茄钟 · 专注 {pomodoro.focusMin} 分钟</Text>
                <View style={styles.pomodoroPresetRow}>
                  {[25, 45, 60].map((minutes) => (
                    <Pressable
                      key={minutes}
                      style={[styles.pomodoroChip, pomodoro.focusMin === minutes && styles.pomodoroChipActive]}
                      onPress={() => pomodoro.setFocusMin(minutes)}
                    >
                      <Text
                        style={[
                          styles.pomodoroChipText,
                          pomodoro.focusMin === minutes && styles.pomodoroChipTextActive,
                        ]}
                      >
                        {minutes} 分
                      </Text>
                    </Pressable>
                  ))}
                </View>
                <View style={styles.pomodoroCustomRow}>
                  <TextInput
                    style={styles.pomodoroInput}
                    value={customFocusInput}
                    onChangeText={setCustomFocusInput}
                    placeholder="自定义"
                    placeholderTextColor={theme.textMuted}
                    keyboardType="number-pad"
                  />
                  <Text style={styles.pomodoroInputUnit}>分钟</Text>
                  <Pressable
                    style={styles.pomodoroStartBtn}
                    onPress={() => {
                      const minutes = Number.parseInt(customFocusInput, 10);
                      setCustomFocusInput("");
                      pomodoro.start(
                        Number.isFinite(minutes) && minutes > 0 ? Math.min(180, minutes) : undefined
                      );
                    }}
                  >
                    <Text style={styles.pomodoroStartText}>开始专注</Text>
                  </Pressable>
                </View>
              </>
            ) : <Pressable accessibilityRole="button" onPress={() => pomodoro.start()} style={styles.quickTimer}>
              <Text style={styles.quickTimerText}>开始 {pomodoro.focusMin} 分钟</Text>
              <Text style={styles.quickTimerText}>▶</Text>
            </Pressable>}
          </View>
          {navigationItem('settings')}
        </View>
        </ScrollView>
      </View>
      <View style={styles.content}>
        {activeContent}
      </View>
    </View>
  );
}

function MobileSafeArea({ children }: { children: ReactNode }) {
  if (Platform.OS !== "android" && Platform.OS !== "ios") return <>{children}</>;
  return <SafeAreaProvider>{children}</SafeAreaProvider>;
}

export default function App() {
  return (
    <MobileSafeArea>
      <ThemeProvider>
        <PomodoroProvider>
          <AIChatProvider><Shell /></AIChatProvider>
        </PomodoroProvider>
      </ThemeProvider>
    </MobileSafeArea>
  );
}

function makeStyles(theme: Theme) {
  return StyleSheet.create({
    root: { flex: 1, flexDirection: "row", backgroundColor: theme.bg },
    focusRoot: { flex: 1, backgroundColor: theme.bg },
    mobileRoot: { flex: 1, backgroundColor: theme.bg },
    mobileContent: { flex: 1, minHeight: 0 },
    mobileNavShell: {
      borderTopWidth: 1,
      borderTopColor: theme.border,
      backgroundColor: theme.sidebar,
      paddingBottom: 8,
    },
    mobileTimer: {
      width: "100%",
      maxWidth: 640,
      alignSelf: "center",
      paddingTop: 6,
      paddingBottom: 2,
    },
    mobileTimerRow: {
      minHeight: 38,
      flexDirection: "row",
      alignItems: "center",
      gap: 10,
      paddingHorizontal: 14,
    },
    mobileTimerInfo: {
      flex: 1,
      minHeight: 34,
      flexDirection: "row",
      alignItems: "center",
      gap: 6,
    },
    mobileTimerLabel: { color: theme.accent, fontSize: 12, fontWeight: "700" },
    mobileTimerClock: {
      color: theme.sidebarText,
      fontSize: 16,
      fontWeight: "700",
      fontVariant: ["tabular-nums"],
    },
    mobileTimerChevron: { color: theme.sidebarTextMuted, fontSize: 16 },
    mobileTimerActions: { flexDirection: "row", gap: 6 },
    mobileTimerActionBtn: {
      minHeight: 34,
      justifyContent: "center",
      paddingHorizontal: 12,
      borderRadius: 8,
      borderWidth: 1,
      borderColor: theme.border,
    },
    mobileTimerActionText: { color: theme.textSecondary, fontSize: 12, fontWeight: "600" },
    mobileTimerStart: {
      minHeight: 36,
      justifyContent: "center",
      paddingHorizontal: 16,
      borderRadius: 10,
      backgroundColor: theme.accent,
    },
    mobileTimerStartText: { color: "#fff", fontSize: 12, fontWeight: "700" },
    mobileTimerPresets: {
      flexDirection: "row",
      gap: 8,
      paddingHorizontal: 14,
      paddingTop: 4,
      paddingBottom: 2,
    },
    mobileTimerChip: {
      flex: 1,
      minHeight: 32,
      alignItems: "center",
      justifyContent: "center",
      borderRadius: 8,
      borderWidth: 1,
      borderColor: theme.border,
    },
    mobileTimerChipActive: { borderColor: theme.accent, backgroundColor: `${theme.accent}22` },
    mobileTimerChipText: { color: theme.textSecondary, fontSize: 12, fontWeight: "600" },
    mobileTimerChipTextActive: { color: theme.accent, fontWeight: "700" },
    mobileNav: {
      width: "100%",
      maxWidth: 640,
      alignSelf: "center",
      flexDirection: "row",
      alignItems: "stretch",
      gap: 4,
      paddingHorizontal: 8,
      paddingTop: 2,
    },
    mobileTab: {
      flex: 1,
      minHeight: 56,
      alignItems: "center",
      justifyContent: "center",
      gap: 2,
      borderRadius: 10,
      paddingHorizontal: 2,
    },
    mobileTabIcon: { color: theme.sidebarTextMuted, fontSize: 19, lineHeight: 21 },
    mobileTabIconActive: { color: theme.sidebarAccent },
    mobileTabText: { color: theme.sidebarTextMuted, fontSize: 11, fontWeight: "600" },
    mobileTabTextActive: { color: theme.sidebarAccent },
    sidebar: {
      width: 216,
      backgroundColor: theme.sidebar,
      borderRightWidth: 1,
      borderRightColor: theme.border,
    },
    content: { flex: 1, minWidth: 0 },
    brandRow: { flexDirection: "row", alignItems: "center", gap: 11, marginBottom: 32 },
    brandMark: {
      width: 34,
      height: 34,
      alignItems: "center",
      justifyContent: "center",
      borderRadius: 7,
      backgroundColor: theme.sidebarAccent,
    },
    brandMarkText: { color: theme.onSidebarAccent, fontSize: 17, fontWeight: "800" },
    brand: { color: theme.sidebarText, fontSize: 18, fontWeight: "700" },
    brandDetail: { color: theme.sidebarTextMuted, fontSize: 11, marginTop: 1 },
    sectionLabel: { color: theme.sidebarTextMuted, fontSize: 10, fontWeight: "700", marginLeft: 10, marginBottom: 10 },
    navigation: { gap: 4 },
    tab: { minHeight: 44, flexDirection: "row", alignItems: "center", gap: 12, paddingHorizontal: 12, borderRadius: 10 },
    activeTab: { backgroundColor: theme.sidebarActive },
    pressedTab: { opacity: 0.72 },
    navMarker: { width: 5, height: 5, borderRadius: 3, backgroundColor: theme.sidebarTextMuted, opacity: .5 },
    navMarkerActive: { backgroundColor: theme.sidebarAccent, opacity: 1, height: 16 },
    tabText: { color: theme.sidebarText, fontSize: 14, fontWeight: "600" },
    activeTabText: { color: theme.sidebarAccent },
    nestedTab: { minHeight: 38, paddingHorizontal: 10 },
    groupList: { borderTopWidth: 1, borderTopColor: theme.border, paddingTop: 14, marginTop: 22, gap: 4 },
    groupHeading: { minHeight: 42, flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between', paddingHorizontal: 12 },
    groupLabel: { color: theme.sidebarTextMuted, fontSize: 13, fontWeight: '600' },
    groupChevron: { color: theme.sidebarTextMuted, fontSize: 19 },
    groupChildren: { marginLeft: 12, paddingLeft: 6, borderLeftWidth: 1, borderLeftColor: theme.border, gap: 2, marginBottom: 10 },
    collapsedLocation: { color: theme.sidebarAccent, fontSize: 11, marginLeft: 12, marginBottom: 8 },
    timerHeader: { flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between', minHeight: 36 },
    quickTimer: { flexDirection: 'row', justifyContent: 'space-between', alignItems: 'center', padding: 10, borderRadius: 8, backgroundColor: theme.sidebarActive },
    quickTimerText: { color: theme.sidebarAccent, fontSize: 12, fontWeight: '600' },
    sidebarFooter: { borderTopWidth: 1, borderTopColor: theme.border, paddingTop: 12, marginTop: 28 },
    sidebarContent: {
      flexGrow: 1,
      justifyContent: "space-between",
      paddingHorizontal: 14,
      paddingTop: 24,
      paddingBottom: 14,
    },
    sidebarPomodoro: { borderBottomWidth: 1, borderBottomColor: theme.border, paddingBottom: 14, marginBottom: 14 },
    pomodoroActions: { flexDirection: "row", gap: 8, marginTop: 8 },
    pomodoroActionBtn: {
      flex: 1,
      alignItems: "center",
      paddingVertical: 6,
      borderRadius: 6,
      borderWidth: 1,
      borderColor: theme.border,
    },
    pomodoroActionText: { color: theme.sidebarText, fontSize: 11 },
    pomodoroPresetRow: { flexDirection: "row", gap: 6, marginTop: 6 },
    pomodoroChip: {
      flex: 1,
      alignItems: "center",
      paddingVertical: 6,
      borderRadius: 6,
      borderWidth: 1,
      borderColor: theme.border,
    },
    pomodoroChipActive: { borderColor: theme.accent, backgroundColor: `${theme.accent}22` },
    pomodoroChipText: { color: theme.sidebarTextMuted, fontSize: 11 },
    pomodoroChipTextActive: { color: theme.sidebarAccent, fontWeight: "700" },
    pomodoroCustomRow: { flexDirection: "row", alignItems: "center", gap: 6, marginTop: 8 },
    pomodoroInput: {
      width: 64,
      borderWidth: 1,
      borderColor: theme.border,
      borderRadius: 6,
      paddingVertical: 5,
      paddingHorizontal: 8,
      color: theme.sidebarText,
      fontSize: 12,
    },
    pomodoroInputUnit: { color: theme.sidebarTextMuted, fontSize: 11 },
    pomodoroStartBtn: {
      flex: 1,
      alignItems: "center",
      paddingVertical: 6,
      borderRadius: 6,
      backgroundColor: theme.accent,
    },
    pomodoroStartText: { color: "#fff", fontSize: 11, fontWeight: "700" },
    pomodoroStatusTitle: { color: theme.accent, fontSize: 11, fontWeight: "700" },
    pomodoroStatusClock: { color: theme.sidebarText, fontSize: 19, fontWeight: "700", fontVariant: ["tabular-nums"] },
    pomodoroTrack: { height: 3, backgroundColor: theme.border, borderRadius: 2, overflow: "hidden", marginTop: 7 },
    pomodoroFill: { height: "100%", backgroundColor: theme.accent, borderRadius: 2 },
  });
}
