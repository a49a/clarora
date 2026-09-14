import { learningDesign } from "../ui/learningDesign";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  FlatList,
  NativeModules,
  Platform,
  Pressable,
  ScrollView,
  StatusBar,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import { useAppTheme } from "../ui/ThemeContext";

// ── Types ────────────────────────────────────────────────────────────────────

interface Sense {
  en: string;
  zh: string;
}

interface GraphNode {
  id: string;
  label: string;
  list_num: number;
  category?: string;
  order?: number;
  pron?: string;
  senses: Sense[];
  kaofa?: string;
  etymology?: string;
  mnemonic?: string;
  node_type: "main" | "related";
}

interface GraphLinkRaw {
  source: string;
  target: string;
  link_type: string;
}

type GraphData = {
  nodes: GraphNode[];
  links: GraphLinkRaw[];
};

function sourceOrder(a: GraphNode, b: GraphNode): number {
  return a.list_num - b.list_num || (a.order ?? 0) - (b.order ?? 0);
}

function listColor(n: number): string {
  if (n === 0) return "#484850";
  const hue = (n * 14) % 360;
  const h = hue;
  const s = 0.65;
  const l = 0.55;
  // HSL to hex approximation
  const a = s * Math.min(l, 1 - l);
  const f = (k: number) => {
    const t = (k + h / 30) % 12;
    return l - a * Math.max(Math.min(t - 3, 9 - t, 1), -1);
  };
  const toHex = (x: number) =>
    Math.round(x * 255)
      .toString(16)
      .padStart(2, "0");
  return `#${toHex(f(0))}${toHex(f(8))}${toHex(f(4))}`;
}

function isGraphData(value: unknown): value is GraphData {
  if (!value || typeof value !== "object") return false;

  const data = value as Partial<GraphData>;
  return Array.isArray(data.nodes) && Array.isArray(data.links);
}

// ── Component ────────────────────────────────────────────────────────────────

export default function GraphScreen() {
  const { theme, scheme } = useAppTheme();

  const [nodes, setNodes] = useState<GraphNode[]>([]);
  const [links, setLinks] = useState<GraphLinkRaw[]>([]);
  const [loading, setLoading] = useState(true);
  const [dataUnavailable, setDataUnavailable] = useState(false);

  const [searchQuery, setSearchQuery] = useState("");
  const [activeList, setActiveList] = useState(0);
  const [selectedNode, setSelectedNode] = useState<GraphNode | null>(null);
  const [railWidth, setRailWidth] = useState(0);

  const flatListRef = useRef<FlatList>(null);

  // ── Load data ──────────────────────────────────────────────────────────────
  useEffect(() => {
    (async () => {
      try {
        // Metro may expose JSON directly or through `default`, depending on the
        // native platform and bundle mode. Support both without assuming data.
        const graphModule = require("../../assets/graph-data.json") as
          | GraphData
          | { default?: GraphData };
        const data = "default" in graphModule && graphModule.default
          ? graphModule.default
          : graphModule;

        if (!isGraphData(data)) {
          setDataUnavailable(true);
          return;
        }

        setNodes(data.nodes);
        setLinks(data.links);
      } catch {
        // This screen is an optional, imported vocabulary book. A missing or
        // malformed book should not take the whole native app down.
        setDataUnavailable(true);
      } finally {
        setLoading(false);
      }
    })();
  }, []);

  // ── Derived data ───────────────────────────────────────────────────────────
  const mainNodes = useMemo(
    () => nodes.filter((n) => n.node_type === "main"),
    [nodes]
  );

  const nodeMap = useMemo(
    () => new Map(nodes.map((n) => [n.id, n])),
    [nodes]
  );

  const listNums = useMemo(() => {
    return [...new Set(mainNodes.map((n) => n.list_num))].filter((ln) => ln > 0).sort((a, b) => a - b);
  }, [mainNodes]);

  const filteredNodes = useMemo(() => {
    const q = searchQuery.trim().toLowerCase();
    return mainNodes
      .filter((n) => {
        if (activeList !== 0 && n.list_num !== activeList) return false;
        if (q && !n.label.toLowerCase().includes(q)) return false;
        return true;
      })
      .sort(sourceOrder);
  }, [mainNodes, activeList, searchQuery]);

  const carouselCardWidth = Math.min(280, Math.max(188, railWidth * 0.68));
  const carouselStep = carouselCardWidth + 12;
  const carouselInset = Math.max(0, (railWidth - carouselCardWidth) / 2);
  const carouselOffsets = filteredNodes.map((_, index) => index * carouselStep);

  // Related nodes
  const getRelated = useCallback(
    (node: GraphNode, linkType: string): GraphNode[] => {
      const results: GraphNode[] = [];
      const seen = new Set<string>();
      for (const l of links) {
        if (l.link_type !== linkType) continue;
        if (l.source === node.id) {
          const n = nodeMap.get(l.target);
          if (n && !seen.has(n.id)) {
            seen.add(n.id);
            results.push(n);
          }
        } else if (l.target === node.id) {
          const n = nodeMap.get(l.source);
          if (n && !seen.has(n.id)) {
            seen.add(n.id);
            results.push(n);
          }
        }
      }
      return results;
    },
    [links, nodeMap]
  );

  const selectNode = useCallback((node: GraphNode) => {
    setSelectedNode(node);
  }, []);

  const scrollToNode = useCallback((node: GraphNode, animated = true) => {
    const index = filteredNodes.findIndex((candidate) => candidate.id === node.id);
    const offset = carouselOffsets[index];
    if (offset !== undefined) {
      flatListRef.current?.scrollToOffset({ offset, animated });
    }
  }, [carouselOffsets, filteredNodes]);

  const selectAndScroll = useCallback((node: GraphNode) => {
    selectNode(node);
    scrollToNode(node);
  }, [scrollToNode, selectNode]);

  // Keep a detail panel open. Changing List or search query moves the reader
  // to the first available word in that source-ordered result set.
  useEffect(() => {
    if (!filteredNodes.length) {
      setSelectedNode(null);
      return;
    }
    if (!selectedNode || !filteredNodes.some((node) => node.id === selectedNode.id)) {
      setSelectedNode(filteredNodes[0]);
      requestAnimationFrame(() => scrollToNode(filteredNodes[0], false));
    }
  }, [filteredNodes, scrollToNode, selectedNode]);

  const moveSelection = useCallback((offset: number) => {
    if (!selectedNode || !filteredNodes.length) return;
    const currentIndex = filteredNodes.findIndex((node) => node.id === selectedNode.id);
    const nextIndex = Math.max(0, Math.min(filteredNodes.length - 1, currentIndex + offset));
    selectAndScroll(filteredNodes[nextIndex]);
  }, [filteredNodes, selectAndScroll, selectedNode]);

  const handleKey = useCallback((key: string) => {
    if (key === "ArrowRight") moveSelection(1);
    if (key === "ArrowLeft") moveSelection(-1);
  }, [moveSelection]);

  useEffect(() => {
    const keyboard = NativeModules.RNKeyboard as
      | { startListening: () => void; getNextKey: () => Promise<string | null>; stopListening: () => void }
      | undefined;
    if (!keyboard?.getNextKey) return;
    let cancelled = false;
    keyboard.startListening();
    const pump = async () => {
      while (!cancelled) {
        const key = await keyboard.getNextKey();
        if (cancelled || key == null) break;
        handleKey(key);
      }
    };
    void pump();
    return () => {
      cancelled = true;
      keyboard.stopListening();
    };
  }, [handleKey]);

  const keyboardProps: any = {
    tabIndex: 0,
    onKeyDown: (event: any) => handleKey(event?.nativeEvent?.key ?? ""),
  };

  // ── Stats ──────────────────────────────────────────────────────────────────
  const stats = useMemo(() => {
    const linkCount = links.length;
    return { total: mainNodes.length, linkCount };
  }, [mainNodes, links]);

  const styles = makeStyles(theme);

  if (loading) {
    return (
      <View style={styles.safeArea}>
        <View style={styles.centered}>
          <Text style={styles.loadingText}>正在加载词汇表…</Text>
        </View>
      </View>
    );
  }

  if (dataUnavailable || nodes.length === 0) {
    return (
      <View style={styles.safeArea}>
        <StatusBar barStyle={scheme === "dark" ? "light-content" : "dark-content"} />
        <View style={styles.centered}>
          <Text style={styles.emptyTitle}>词汇表暂未导入</Text>
          <Text style={styles.emptyText}>
            生成单词学习数据后，这里会显示词汇表。
          </Text>
        </View>
      </View>
    );
  }

  const synonyms = selectedNode ? getRelated(selectedNode, "syn") : [];
  const antonyms = selectedNode ? getRelated(selectedNode, "ant") : [];

  return (
    <View style={styles.safeArea}>
      <StatusBar barStyle={scheme === "dark" ? "light-content" : "dark-content"} />
      <View style={styles.container} {...keyboardProps}>
        {/* Header */}
        <View style={styles.header}>
          <Text style={styles.eyebrow}>VOCABULARY</Text>
          <Text style={styles.title}>词汇表</Text>
          <Text style={styles.statsText}>
            当前已收录 {stats.total} 个单词，{stats.linkCount} 条词义关联。
          </Text>
        </View>

        {/* List selection and horizontal word carousel */}
        <ScrollView
          horizontal
          showsHorizontalScrollIndicator={false}
          style={styles.listTabScroll}
          contentContainerStyle={styles.listTabContent}
        >
          <Pressable
            style={[styles.listTab, activeList === 0 && styles.listTabActive]}
            onPress={() => setActiveList(0)}
          >
            <Text style={[styles.listTabText, activeList === 0 && styles.listTabTextActive]}>全部</Text>
          </Pressable>
          {listNums.map((ln) => (
            <Pressable
              key={ln}
              style={[styles.listTab, activeList === ln && styles.listTabActive]}
              onPress={() => setActiveList(ln)}
            >
              <Text style={[styles.listTabText, activeList === ln && styles.listTabTextActive, { color: activeList === ln ? "#fff" : listColor(ln) }]}>
                {ln}
              </Text>
            </Pressable>
          ))}
        </ScrollView>

        <View style={styles.rail}>
          <FlatList
            ref={flatListRef}
            data={filteredNodes}
            keyExtractor={(item) => item.id}
            horizontal
            decelerationRate="fast"
            disableIntervalMomentum
            showsHorizontalScrollIndicator={false}
            snapToOffsets={carouselOffsets}
            contentContainerStyle={{ paddingHorizontal: carouselInset, gap: 12 }}
            onLayout={(event) => setRailWidth(event.nativeEvent.layout.width)}
            onMomentumScrollEnd={(event) => {
              if (!railWidth) return;
              const index = Math.round(event.nativeEvent.contentOffset.x / carouselStep);
              const node = filteredNodes[index];
              if (node) selectNode(node);
            }}
            onScrollToIndexFailed={() => {}}
            renderItem={({ item }) => {
              const ln = item.list_num;
              const isCurrent = selectedNode?.id === item.id;
              return (
                <Pressable
                  style={[
                    styles.wordItem,
                    { width: carouselCardWidth },
                    isCurrent && styles.wordItemActive,
                  ]}
                  onPress={() => selectAndScroll(item)}
                >
                  <View style={[styles.wordDot, { backgroundColor: listColor(ln) }]} />
                  <View style={styles.wordInfo}>
                    <Text style={[styles.wordLabel, isCurrent && styles.wordLabelActive]}>{item.label}</Text>
                    {(item.senses[0]?.zh || item.senses[0]?.en) && (
                      <Text style={styles.wordMeaning} numberOfLines={1}>
                        {item.senses[0]?.zh || item.senses[0]?.en}
                      </Text>
                    )}
                  </View>
                  <Text style={styles.wordListNum}>L{ln}</Text>
                </Pressable>
              );
            }}
            style={styles.wordList}
            initialNumToRender={5}
            getItemLayout={(_, index) => ({
              length: carouselStep,
              offset: carouselStep * index,
              index,
            })}
          />
        </View>

        {/* Touch prev/next for one-handed phone use */}
        {Platform.OS === "android" && (
          <View style={styles.railControls}>
            <Pressable style={styles.railControlBtn} onPress={() => moveSelection(-1)}>
              <Text style={styles.railControlText}>‹</Text>
            </Pressable>
            <Pressable style={styles.railControlBtn} onPress={() => moveSelection(1)}>
              <Text style={styles.railControlText}>›</Text>
            </Pressable>
          </View>
        )}

        {/* Search */}
        <View style={styles.searchRow}>
          <TextInput
            style={styles.searchInput}
            placeholder="搜索单词…"
            placeholderTextColor={theme.textSecondary}
            value={searchQuery}
            onChangeText={setSearchQuery}
            autoCapitalize="none"
            autoCorrect={false}
          />
          {searchQuery.length > 0 && (
            <Pressable style={styles.clearBtn} onPress={() => setSearchQuery("")}>
              <Text style={styles.clearBtnText}>✕</Text>
            </Pressable>
          )}
        </View>

        {/* Current word detail */}
        {selectedNode && (
          <View style={styles.detailCard}>
          <ScrollView style={styles.detailScroll} contentContainerStyle={styles.detailContent} showsVerticalScrollIndicator={false}>
            <View style={styles.detailHeader}>
              <Text style={styles.detailWord}>{selectedNode.label}</Text>
              <Text style={styles.detailPosition}>
                {filteredNodes.findIndex((node) => node.id === selectedNode.id) + 1} / {filteredNodes.length}
              </Text>
            </View>
            {selectedNode.pron ? (
              <Text style={styles.detailPron}>/{selectedNode.pron}/</Text>
            ) : null}

            {/* Category + List */}
            <View style={styles.sourceRow}>
              <View style={[styles.sourceBadge, styles.sourceBadgePresent]}>
                <Text style={[styles.sourceBadgeText, styles.sourceBadgeTextPresent]}>
                  {selectedNode.category || "自定义词汇"}
                </Text>
              </View>
              <View style={[styles.sourceBadge, styles.sourceBadgeMissing]}>
                <Text style={[styles.sourceBadgeText, styles.sourceBadgeTextMissing]}>
                  List {selectedNode.list_num}
                  {selectedNode.order ? ` · ${selectedNode.order}` : ""}
                </Text>
              </View>
            </View>

            {/* Senses */}
            {selectedNode.senses.length > 0 && (
              <View style={styles.section}>
                <Text style={styles.sectionLabel}>释义</Text>
                {selectedNode.senses.map((s, i) => (
                  <View key={i} style={styles.senseItem}>
                    {s.en ? <Text style={styles.senseEn}>{s.en}</Text> : null}
                    {s.zh ? <Text style={styles.senseZh}>{s.zh}</Text> : null}
                  </View>
                ))}
              </View>
            )}

            {/* 学习提示 */}
            {selectedNode.kaofa ? (
              <View style={styles.section}>
                <Text style={styles.sectionLabel}>学习提示</Text>
                <Text style={styles.infoText}>{selectedNode.kaofa}</Text>
              </View>
            ) : null}

            {/* 词源 */}
            {selectedNode.etymology ? (
              <View style={styles.section}>
                <Text style={styles.sectionLabel}>词源</Text>
                <Text style={styles.infoText}>{selectedNode.etymology}</Text>
              </View>
            ) : null}

            {/* 记忆方法 */}
            {selectedNode.mnemonic ? (
              <View style={styles.section}>
                <Text style={styles.sectionLabel}>记忆方法</Text>
                <Text style={styles.infoText}>{selectedNode.mnemonic}</Text>
              </View>
            ) : null}

            {/* Synonyms */}
            {synonyms.length > 0 && (
              <View style={styles.section}>
                <Text style={styles.sectionLabel}>同义词</Text>
                <View style={styles.chipRow}>
                  {synonyms.map((n) => (
                    <Pressable
                      key={n.id}
                      style={[styles.chip, styles.chipSyn]}
                      onPress={() => selectAndScroll(n)}
                    >
                      <Text style={styles.chipText}>{n.label}</Text>
                    </Pressable>
                  ))}
                </View>
              </View>
            )}

            {/* Antonyms */}
            {antonyms.length > 0 && (
              <View style={styles.section}>
                <Text style={styles.sectionLabel}>反义词</Text>
                <View style={styles.chipRow}>
                  {antonyms.map((n) => (
                    <Pressable
                      key={n.id}
                      style={[styles.chip, styles.chipAnt]}
                      onPress={() => selectAndScroll(n)}
                    >
                      <Text style={styles.chipText}>{n.label}</Text>
                    </Pressable>
                  ))}
                </View>
              </View>
            )}
          </ScrollView>
          </View>
        )}

      </View>
    </View>
  );
}

function makeStyles(theme: ReturnType<typeof useAppTheme>["theme"]) {
  const ui = learningDesign(theme);
  return StyleSheet.create({
    safeArea: { flex: 1, backgroundColor: theme.bg },
    container: {
      flex: 1,
      ...ui.page,
    },
    centered: { flex: 1, justifyContent: "center", alignItems: "center" },
    loadingText: { color: theme.textSecondary, fontSize: 15 },
    emptyTitle: { color: theme.text, fontSize: 20, fontWeight: "700" },
    emptyText: {
      color: theme.textSecondary,
      fontSize: 14,
      lineHeight: 21,
      marginTop: 8,
      maxWidth: 300,
      paddingHorizontal: 20,
      textAlign: "center",
    },

    // Header
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
    statsText: { color: theme.textSecondary, fontSize: 14, marginTop: 6 },

    railControls: {
      flexDirection: "row",
      justifyContent: "center",
      gap: 28,
      marginBottom: 12,
    },
    railControlBtn: {
      width: 52,
      height: 52,
      borderRadius: 26,
      alignItems: "center",
      justifyContent: "center",
      backgroundColor: theme.surface,
      borderWidth: 1,
      borderColor: theme.border,
    },
    railControlText: { color: theme.accent, fontSize: 26, lineHeight: 30 },

    // Search
    searchRow: {
      flexDirection: "row",
      alignItems: "center",
      backgroundColor: theme.surface,
      borderRadius: 10,
      paddingHorizontal: 14,
      marginBottom: 10,
      ...theme.cardShadow,
      shadowOpacity: 0.05,
      elevation: 1,
    },
    searchInput: {
      flex: 1,
      height: 40,
      color: theme.text,
      fontSize: 15,
      ...ui.input,
    },
    clearBtn: { padding: 6 },
    clearBtnText: { color: theme.textSecondary, fontSize: 14 },

    // Search result
    searchResult: {
      backgroundColor: theme.surface,
      borderRadius: 12,
      padding: 14,
      marginBottom: 10,
      ...theme.cardShadow,
    },
    searchMatchCard: {},
    searchMatchWord: {
      fontSize: 20,
      fontWeight: "700",
      color: theme.text,
    },
    searchMatchPron: {
      fontSize: 13,
      color: theme.textSecondary,
      marginTop: 2,
    },
    searchMatchMeaning: {
      fontSize: 14,
      color: theme.text,
      marginTop: 6,
    },
    sourceRow: {
      flexDirection: "row",
      flexWrap: "wrap",
      gap: 6,
      marginTop: 8,
    },
    sourceBadge: {
      paddingVertical: 3,
      paddingHorizontal: 8,
      borderRadius: 6,
    },
    sourceBadgePresent: { backgroundColor: `${theme.accent}20` },
    sourceBadgeMissing: { backgroundColor: `${theme.textSecondary}15` },
    sourceBadgeText: { fontSize: 11, fontWeight: "600" },
    sourceBadgeTextPresent: { color: theme.accent },
    sourceBadgeTextMissing: { color: theme.textSecondary },
    suggestLabel: {
      color: theme.textSecondary,
      fontSize: 13,
      marginBottom: 6,
    },
    suggestItem: {
      flexDirection: "row",
      justifyContent: "space-between",
      alignItems: "center",
      paddingVertical: 8,
      borderBottomWidth: StyleSheet.hairlineWidth,
      borderBottomColor: theme.border,
    },
    suggestWord: { color: theme.text, fontSize: 14, fontWeight: "600" },
    suggestMeaning: {
      color: theme.textSecondary,
      fontSize: 12,
      flex: 1,
      marginLeft: 12,
      textAlign: "right",
    },
    noResult: {
      color: theme.textSecondary,
      fontSize: 14,
      textAlign: "center",
      paddingVertical: 14,
    },

    // Detail card
    detailCard: {
      flex: 1,
      ...theme.cardShadow,
      ...ui.panel,
    },
    // Keep the inset on a normal View: native ScrollView padding can leave
    // its document flush against the rounded border and clip the first row.
    detailScroll: { flex: 1 },
    detailContent: { paddingBottom: 20 },
    detailHeader: {
      flexDirection: "row",
      justifyContent: "space-between",
      alignItems: "flex-start",
    },
    detailWord: { fontSize: 26, fontWeight: "700", color: theme.text },
    detailPosition: {
      color: theme.textSecondary,
      fontSize: 12,
      fontVariant: ["tabular-nums"],
      paddingTop: 8,
    },
    detailClose: { color: theme.textSecondary, fontSize: 18, padding: 4 },
    detailPron: {
      fontSize: 14,
      color: theme.textSecondary,
      marginTop: 2,
    },
    section: { marginTop: 16 },
    sectionLabel: {
      fontSize: 13,
      fontWeight: "600",
      color: theme.textSecondary,
      marginBottom: 6,
      textTransform: "uppercase",
      letterSpacing: 0.3,
    },
    senseItem: { marginBottom: 8 },
    senseEn: { fontSize: 14, color: theme.text, lineHeight: 20 },
    senseZh: { fontSize: 13, color: theme.textSecondary, marginTop: 2 },
    infoText: {
      fontSize: 14,
      color: theme.text,
      lineHeight: 21,
    },
    chipRow: { flexDirection: "row", flexWrap: "wrap", gap: 6 },
    chip: {
      paddingVertical: 5,
      paddingHorizontal: 10,
      borderRadius: 8,
    },
    chipSyn: { backgroundColor: "#4da3ff25" },
    chipAnt: { backgroundColor: "#ff6b6b25" },
    chipText: { color: theme.text, fontSize: 13, fontWeight: "500" },

    // List tabs
    listTabScroll: { maxHeight: 34, marginBottom: 8 },
    listTabContent: { gap: 4, paddingRight: 12 },
    listTab: {
      paddingVertical: 5,
      paddingHorizontal: 10,
      borderRadius: 6,
      backgroundColor: theme.surface,
    },
    listTabActive: { backgroundColor: theme.accent },
    listTabText: {
      fontSize: 12,
      fontWeight: "500",
      color: theme.textSecondary,
    },
    listTabTextActive: { color: "#fff" },

    // Word list
    rail: { height: 128, marginBottom: 12 },
    wordList: { flex: 1 },
    wordItem: {
      flexDirection: "row",
      alignItems: "center",
      height: 104,
      paddingHorizontal: 16,
      borderBottomWidth: StyleSheet.hairlineWidth,
      borderBottomColor: theme.border,
      borderRadius: 8,
      opacity: 0.42,
    },
    wordItemActive: {
      backgroundColor: `${theme.accent}20`,
      borderWidth: 1,
      borderColor: `${theme.accent}70`,
      opacity: 1,
      transform: [{ scale: 1.14 }],
      ...theme.cardShadow,
      elevation: 8,
    },
    wordDot: {
      width: 8,
      height: 8,
      borderRadius: 4,
      marginRight: 10,
    },
    wordInfo: { flex: 1 },
    wordLabel: { color: theme.text, fontSize: 15, fontWeight: "600" },
    wordLabelActive: { color: theme.accent, fontSize: 22, fontWeight: "700" },
    wordMeaning: {
      color: theme.textSecondary,
      fontSize: 12,
      marginTop: 1,
    },
    wordListNum: {
      color: theme.textSecondary,
      fontSize: 11,
      fontVariant: ["tabular-nums"],
      marginLeft: 8,
    },
  });
}
