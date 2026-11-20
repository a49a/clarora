import { useCallback, useEffect, useRef, useState } from "react";
import {
  BackHandler,
  Platform,
  Pressable,
  ScrollView,
  StatusBar,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import { learningDesign } from "../ui/learningDesign";
import { useAppTheme } from "../ui/ThemeContext";
import {
  searchSentences,
  type SearchControl,
  type SentenceHit,
  type SentenceMatchMode,
  type SentenceScope,
  type SentenceSearchResult,
} from "../services/sentenceSearch";

type SentenceSearchScreenProps = {
  onClose: () => void;
  onOpenAudio: (practiceId: string, audioId: string, positionMs: number) => void;
  onOpenVideo: (videoId: string, positionMs: number) => void;
};

const PAGE_SIZE = 20;
const MODES: Array<{ key: SentenceMatchMode; label: string }> = [
  { key: "prefix", label: "句首匹配" },
  { key: "contains", label: "包含匹配" },
];
const SCOPES: Array<{ key: SentenceScope; label: string }> = [
  { key: "all", label: "全部" },
  { key: "audio", label: "音频" },
  { key: "video", label: "视频" },
];

function formatClock(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return `${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
}

export default function SentenceSearchScreen({
  onClose,
  onOpenAudio,
  onOpenVideo,
}: SentenceSearchScreenProps) {
  const { theme, scheme } = useAppTheme();
  const styles = makeStyles(theme);
  const controlRef = useRef<SearchControl | null>(null);
  const [phrase, setPhrase] = useState("");
  const [mode, setMode] = useState<SentenceMatchMode>("prefix");
  const [scope, setScope] = useState<SentenceScope>("all");
  const [searching, setSearching] = useState(false);
  const [result, setResult] = useState<SentenceSearchResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [visible, setVisible] = useState(PAGE_SIZE);

  useEffect(() => () => {
    if (controlRef.current) controlRef.current.cancelled = true;
  }, []);

  useEffect(() => {
    if (Platform.OS !== "android") return;
    const subscription = BackHandler.addEventListener("hardwareBackPress", () => {
      if (controlRef.current) controlRef.current.cancelled = true;
      onClose();
      return true;
    });
    return () => subscription.remove();
  }, [onClose]);

  const run = useCallback(async (nextPhrase: string, nextMode: SentenceMatchMode, nextScope: SentenceScope) => {
    const trimmed = nextPhrase.trim();
    if (!trimmed) {
      setError("请输入英文句型，例如 When is 或 Why did");
      return;
    }
    if (controlRef.current) controlRef.current.cancelled = true;
    const control: SearchControl = { cancelled: false };
    controlRef.current = control;
    setError(null);
    setSearching(true);
    setResult({ hits: [], total: 0, scanned: 0, withoutSubtitles: 0, unavailable: 0, problems: [] });
    setVisible(PAGE_SIZE);
    try {
      const found = await searchSentences(
        { phrase: trimmed, mode: nextMode, scope: nextScope },
        control,
        (partial) => {
          if (!control.cancelled) setResult({ ...partial, hits: partial.hits.slice() });
        }
      );
      if (control.cancelled) return;
      setResult(found);
    } catch (e: any) {
      if (!control.cancelled) setError(e?.message ?? String(e));
    } finally {
      if (controlRef.current === control) setSearching(false);
    }
  }, []);

  const openHit = useCallback((hit: SentenceHit) => {
    if (hit.kind === "audio") {
      onOpenAudio(hit.practiceId, hit.sourceId, hit.startMs);
    } else {
      onOpenVideo(hit.sourceId, hit.startMs);
    }
  }, [onOpenAudio, onOpenVideo]);

  const hits = result?.hits ?? [];
  const shown = hits.slice(0, visible);

  return (
    <View style={styles.safeArea}>
      <StatusBar barStyle={scheme === "dark" ? "light-content" : "dark-content"} />
      <ScrollView
        style={styles.container}
        contentContainerStyle={styles.content}
        keyboardShouldPersistTaps="handled"
        showsVerticalScrollIndicator={false}
      >
        <View style={styles.topBar}>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="返回"
            onPress={() => {
              if (controlRef.current) controlRef.current.cancelled = true;
              onClose();
            }}
            style={({ pressed }) => [styles.backBtn, pressed && styles.pressed]}
          >
            <Text style={styles.backText}>← 返回</Text>
          </Pressable>
        </View>

        <View style={styles.header}>
          <Text style={styles.eyebrow}>SENTENCE SEARCH</Text>
          <Text style={styles.title}>句型搜索</Text>
          <Text style={styles.subtitle}>
            搜遍本机音频与视频字幕，点结果直接跳到对应句子。字幕只在本机读取。
          </Text>
        </View>

        <View style={styles.panel}>
          <View style={styles.inputRow}>
            <TextInput
              testID="sentence-query"
              accessibilityLabel="句型"
              value={phrase}
              onChangeText={setPhrase}
              onSubmitEditing={() => void run(phrase, mode, scope)}
              placeholder="When is / Why did …"
              placeholderTextColor={theme.textMuted}
              autoCapitalize="none"
              autoCorrect={false}
              returnKeyType="search"
              style={styles.input}
            />
            <Pressable
              accessibilityRole="button"
              accessibilityLabel="搜索句型"
              onPress={() => void run(phrase, mode, scope)}
              style={({ pressed }) => [styles.searchBtn, pressed && styles.pressed]}
            >
              <Text style={styles.searchBtnText}>{searching ? "搜索中…" : "搜索"}</Text>
            </Pressable>
          </View>

          <View style={styles.chipRow}>
            {MODES.map((item) => (
              <Pressable
                key={item.key}
                accessibilityRole="button"
                accessibilityState={{ selected: mode === item.key }}
                onPress={() => {
                  setMode(item.key);
                  if (phrase.trim()) void run(phrase, item.key, scope);
                }}
                style={({ pressed }) => [styles.chip, mode === item.key && styles.chipActive, pressed && styles.pressed]}
              >
                <Text style={[styles.chipText, mode === item.key && styles.chipTextActive]}>{item.label}</Text>
              </Pressable>
            ))}
            <View style={styles.chipDivider} />
            {SCOPES.map((item) => (
              <Pressable
                key={item.key}
                accessibilityRole="button"
                accessibilityState={{ selected: scope === item.key }}
                onPress={() => {
                  setScope(item.key);
                  if (phrase.trim()) void run(phrase, mode, item.key);
                }}
                style={({ pressed }) => [styles.chip, scope === item.key && styles.chipActive, pressed && styles.pressed]}
              >
                <Text style={[styles.chipText, scope === item.key && styles.chipTextActive]}>{item.label}</Text>
              </Pressable>
            ))}
          </View>

          {error ? <Text style={styles.error}>{error}</Text> : null}

          {result ? (
            <Text style={styles.summary}>
              {searching
                ? `已找到 ${result.total} 句 · 已扫描 ${result.scanned} 个材料…`
                : `共 ${result.total} 句匹配 · 扫描 ${result.scanned} 个材料，${result.withoutSubtitles} 个无字幕${result.unavailable ? `，${result.unavailable} 个不可用` : ""}`}
            </Text>
          ) : (
            <Text style={styles.summary}>先输入句型，再点搜索。视频需要先导入字幕后才能搜到。</Text>
          )}
        </View>

        {shown.map((hit) => (
          <Pressable
            key={hit.key}
            accessibilityRole="button"
            accessibilityLabel={`${hit.label} ${formatClock(hit.startMs)}：${hit.english}`}
            onPress={() => openHit(hit)}
            style={({ pressed }) => [styles.hit, pressed && styles.pressed]}
          >
            <View style={styles.hitHeader}>
              <Text style={styles.hitSource} numberOfLines={1}>
                {hit.kind === "audio" ? "🎧" : "🎬"} {hit.label}
              </Text>
              <Text style={styles.hitMeta}>{formatClock(hit.startMs)}</Text>
            </View>
            {hit.kind === "audio" && !!hit.group && (
              <Text style={styles.hitGroup}>{hit.group}</Text>
            )}
            <Text style={styles.hitEnglish}>{hit.english}</Text>
            {!!hit.translation && <Text style={styles.hitTranslation}>{hit.translation}</Text>}
          </Pressable>
        ))}

        {hits.length > visible ? (
          <Pressable
            accessibilityRole="button"
            onPress={() => setVisible((value) => value + PAGE_SIZE)}
            style={({ pressed }) => [styles.moreBtn, pressed && styles.pressed]}
          >
            <Text style={styles.moreText}>显示更多（剩余 {hits.length - visible}）</Text>
          </Pressable>
        ) : null}

        {!searching && result && result.total === 0 ? (
          <Text style={styles.empty}>没有找到匹配的句子，换个词组或切到「包含匹配」试试。</Text>
        ) : null}

        {result?.problems.length ? (
          <View style={styles.problems}>
            {result.problems.map((item) => (
              <Text key={item} style={styles.problem}>{item}</Text>
            ))}
          </View>
        ) : null}
      </ScrollView>
    </View>
  );
}

function makeStyles(theme: ReturnType<typeof useAppTheme>["theme"]) {
  const ui = learningDesign(theme);
  return StyleSheet.create({
    safeArea: { flex: 1, backgroundColor: theme.bg },
    container: { flex: 1 },
    content: { gap: 12, ...ui.page },
    topBar: { flexDirection: "row", alignItems: "center" },
    backBtn: { minHeight: 40, justifyContent: "center", paddingRight: 12 },
    backText: { color: theme.accent, fontSize: 14, fontWeight: "600" },
    header: { ...ui.header, marginBottom: 0 },
    eyebrow: { color: theme.textMuted, fontSize: 11, fontWeight: "700", letterSpacing: 1.2 },
    title: { ...ui.title, marginTop: 2 },
    subtitle: { ...ui.subtitle },
    panel: { ...ui.panel, gap: 12 },
    inputRow: { flexDirection: "row", alignItems: "center", gap: 10 },
    input: { ...ui.input, flex: 1, color: theme.text, fontSize: 15 },
    searchBtn: {
      ...ui.button,
      alignItems: "center",
      paddingHorizontal: 18,
      backgroundColor: theme.accent,
    },
    searchBtnText: { color: "#fff", fontSize: 14, fontWeight: "700" },
    chipRow: { flexDirection: "row", flexWrap: "wrap", alignItems: "center", gap: 8 },
    chip: {
      minHeight: 32,
      justifyContent: "center",
      paddingHorizontal: 12,
      borderRadius: 999,
      borderWidth: 1,
      borderColor: theme.border,
    },
    chipActive: { borderColor: theme.accent, backgroundColor: `${theme.accent}22` },
    chipText: { color: theme.textSecondary, fontSize: 12, fontWeight: "600" },
    chipTextActive: { color: theme.accent },
    chipDivider: { width: 1, height: 18, backgroundColor: theme.border, marginHorizontal: 2 },
    error: { color: theme.danger, fontSize: 13, lineHeight: 19 },
    summary: { color: theme.textMuted, fontSize: 12, lineHeight: 18 },
    hit: {
      ...ui.card,
      gap: 4,
      padding: 14,
    },
    hitHeader: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", gap: 10 },
    hitSource: { flex: 1, color: theme.textSecondary, fontSize: 12, fontWeight: "600" },
    hitMeta: { color: theme.textMuted, fontSize: 11, fontVariant: ["tabular-nums"] },
    hitGroup: { color: theme.textMuted, fontSize: 11 },
    hitEnglish: { color: theme.text, fontSize: 15, lineHeight: 22, marginTop: 2 },
    hitTranslation: { color: theme.textSecondary, fontSize: 13, lineHeight: 19 },
    moreBtn: {
      minHeight: 44,
      alignItems: "center",
      justifyContent: "center",
      borderRadius: 14,
      borderWidth: 1,
      borderColor: theme.border,
      backgroundColor: theme.surface,
    },
    moreText: { color: theme.textSecondary, fontSize: 13, fontWeight: "600" },
    empty: { color: theme.textMuted, fontSize: 13, lineHeight: 20, textAlign: "center", paddingVertical: 18 },
    problems: { gap: 4 },
    problem: { color: theme.textMuted, fontSize: 11, lineHeight: 16 },
    pressed: { opacity: 0.72 },
  });
}
