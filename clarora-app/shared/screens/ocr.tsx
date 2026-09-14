import { useAIChatEntry } from "../ui/AIChatProvider";
import { Details } from "../ui/Details";
import { learningDesign } from "../ui/learningDesign";
import { useCallback, useEffect, useState } from "react";
import {
  Alert,
  Platform,
  Pressable,
  ScrollView,
  StatusBar,
  StyleSheet,
  Text,
  View,
} from "react-native";
import {
  deleteOcrPage,
  getOcrPage,
  listOcrPages,
  ocrPage,
  waitForOcr,
  type OcrPageSummary,
} from "../services/ai";
import { copyToClipboard, DocumentPicker } from "../services/platform";
import { useAppTheme } from "../ui/ThemeContext";

// 拍照识字：Android 拍一页书上传，本机视觉 LLM 转录文本；Mac 端直接选图，
// 两端共用本机的识别结果列表。

function formatTime(iso: string | null): string {
  if (!iso) return "";
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${date.getMonth() + 1}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

export default function OcrScreen() {
  const { theme, scheme } = useAppTheme();
  const styles = makeStyles(theme);

  const [pages, setPages] = useState<OcrPageSummary[]>([]);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [expandedText, setExpandedText] = useState<string>("");
  useAIChatEntry(expandedId && !expandedText.startsWith("加载") ? expandedText : "", "拍照识字");
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [toast, setToast] = useState<string | null>(null);

  const showToast = useCallback((msg: string) => {
    setToast(msg);
    setTimeout(() => setToast(null), 2300);
  }, []);

  const refresh = useCallback(async () => {
    try {
      setPages(await listOcrPages());
    } catch (e: any) {
      setError(`获取识别列表失败：${e?.message ?? e}`);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const processFile = useCallback(
    async (file: { uri: string; name: string }) => {
      setBusy("正在上传图片…");
      setError(null);
      try {
        const { pageId } = await ocrPage(file.uri, file.name);
        setBusy("正在识别…");
        const text = await waitForOcr(pageId, () => undefined);
        showToast(`识别完成（${text.length} 字），已保存到本机列表`);
        await refresh();
      } catch (e: any) {
        setError(`识别失败：${e?.message ?? e}`);
        // 失败的页面也留在本机列表里，方便查看错误后删除。
        await refresh().catch(() => {});
      } finally {
        setBusy(null);
      }
    },
    [refresh, showToast]
  );

  const capturePhoto = useCallback(async () => {
    try {
      const file = await DocumentPicker.captureImageAsync();
      if (file) void processFile(file);
    } catch (e: any) {
      setError(`拍照失败：${e?.message ?? e}`);
    }
  }, [processFile]);

  const pickImage = useCallback(async () => {
    try {
      const result = await DocumentPicker.getDocumentAsync({ type: "image/*" });
      const file = result.assets?.[0];
      if (result.canceled || !file) return;
      void processFile(file);
    } catch (e: any) {
      setError(`选择图片失败：${e?.message ?? e}`);
    }
  }, [processFile]);

  const togglePage = useCallback(async (page: OcrPageSummary) => {
    if (expandedId === page.pageId) {
      setExpandedId(null);
      return;
    }
    if (page.status !== "completed") return;
    setExpandedId(page.pageId);
    setExpandedText("加载中…");
    try {
      const full = await getOcrPage(page.pageId);
      setExpandedText(full.text ?? "");
    } catch (e: any) {
      setExpandedText(`加载失败：${e?.message ?? e}`);
    }
  }, [expandedId]);

  const copyText = useCallback(
    (text: string) => {
      copyToClipboard(text);
      showToast("已复制");
    },
    [showToast]
  );

  const removePage = useCallback(
    (page: OcrPageSummary) => {
      Alert.alert("删除识别记录？", "删除后不可恢复。", [
        { text: "取消", style: "cancel" },
        {
          text: "删除",
          style: "destructive",
          onPress: () => {
            void (async () => {
              await deleteOcrPage(page.pageId).catch(() => {});
              setExpandedId(null);
              await refresh();
            })();
          },
        },
      ]);
    },
    [refresh]
  );

  return (
    <View style={styles.safeArea}>
      <StatusBar barStyle={scheme === "dark" ? "light-content" : "dark-content"} />
      <ScrollView style={styles.container} contentContainerStyle={styles.content} showsVerticalScrollIndicator={false}>
        <View style={styles.header}>
          <Text style={styles.eyebrow}>OCR</Text>
          <Text style={styles.title}>拍照识字</Text>
          <Text style={styles.subtitle}>
            把书页变成可以学习的文字。
          </Text>
        </View>

        <View style={styles.panel}>
          <View style={styles.actionsRow}>
            {Platform.OS === "android" && (
              <Pressable
                style={[styles.pickBtn, busy && styles.pickBtnDisabled]}
                disabled={!!busy}
                onPress={() => { void capturePhoto(); }}
              >
                <Text style={styles.pickBtnText}>📷 拍照识字</Text>
              </Pressable>
            )}
            <Pressable
              style={[styles.pickBtn, busy && styles.pickBtnDisabled]}
              disabled={!!busy}
              onPress={() => { void pickImage(); }}
            >
              <Text style={styles.pickBtnText}>{Platform.OS === "android" ? "🖼 从相册选择" : "🖼 选择图片"}</Text>
            </Pressable>
            <Pressable style={styles.refreshBtn} onPress={() => { void refresh(); }}>
              <Text style={styles.refreshBtnText}>刷新</Text>
            </Pressable>
          </View>
          {busy ? <Text style={styles.busyText}>{busy}</Text> : null}
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

        {pages.length > 0 && (
          <Details title="识别记录（可通过存储备份合并）">
            <View style={styles.panel}>

            {pages.map((page) => (
              <View key={page.pageId} style={styles.pageCard}>
                <Pressable
                  style={styles.pageHeader}
                  onPress={() => {
                    if (page.status === "completed") void togglePage(page);
                  }}
                >
                  <Text style={styles.pageName} numberOfLines={1}>
                    {page.status === "completed"
                      ? `📄 ${page.sourceName ?? "未命名页面"}`
                      : page.status === "failed"
                      ? `⚠️ ${page.sourceName ?? "未命名页面"}`
                      : `⏳ ${page.sourceName ?? "未命名页面"}`}
                  </Text>
                  <Text style={styles.pageMeta}>
                    {page.status === "completed"
                      ? `${page.textLength} 字 · ${formatTime(page.createdAt)}`
                      : page.status === "failed"
                      ? "识别失败"
                      : "识别中…"}
                  </Text>
                </Pressable>
                {expandedId === page.pageId && page.status === "completed" ? (
                  <>
                    <Text style={styles.pageText} selectable>
                      {expandedText}
                    </Text>
                    <View style={styles.pageActions}>
                      <Pressable style={styles.pageActionBtn} onPress={() => copyText(expandedText)}>
                        <Text style={styles.pageActionText}>复制文本</Text>
                      </Pressable>
                      <Pressable style={styles.pageActionBtnDanger} onPress={() => removePage(page)}>
                        <Text style={[styles.pageActionText, { color: theme.danger }]}>删除</Text>
                      </Pressable>
                    </View>
                  </>
                ) : null}
                {page.status === "failed" && page.error ? (
                  <Text style={styles.pageError}>{page.error}</Text>
                ) : null}
              </View>
            ))}
          </View>
          </Details>
        )}
        {pages.length === 0 && !busy ? (
          <View style={styles.panel}>
            <Text style={styles.emptyText}>
              还没有识别记录。拍照或选择一页书的照片，识别出的文本会同步显示在两端的这个列表里。
            </Text>
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
    content: {
      gap: 16,
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

    panel: {
      gap: 10,
      ...ui.panel,
    },
    panelLabel: {
      ...ui.label,
    },
    emptyText: { color: theme.textSecondary, fontSize: 13, lineHeight: 20 },

    actionsRow: { flexDirection: "row", alignItems: "center", gap: 10, flexWrap: "wrap" },
    pickBtn: {
      paddingVertical: 9,
      paddingHorizontal: 16,
      backgroundColor: theme.accent,
      ...ui.button,
    },
    pickBtnDisabled: { opacity: 0.5 },
    pickBtnText: { color: "#fff", fontSize: 13, fontWeight: "700" },
    refreshBtn: {
      paddingVertical: 9,
      paddingHorizontal: 12,
      borderWidth: 1,
      borderColor: theme.border,
      ...ui.button,
    },
    refreshBtnText: { color: theme.textSecondary, fontSize: 13 },
    busyText: { color: theme.textSecondary, fontSize: 13 },

    errorBanner: {
      backgroundColor: `${theme.danger}14`,
      borderRadius: 6,
      paddingVertical: 10,
      paddingHorizontal: 14,
    },
    errorText: { color: theme.danger, fontSize: 13 },

    pageCard: {
      borderWidth: 1,
      borderColor: theme.border,
      padding: 10,
      gap: 6,
      backgroundColor: theme.bg,
      ...ui.row,
    },
    pageHeader: { gap: 2 },
    pageName: { color: theme.text, fontSize: 13, fontWeight: "600" },
    pageMeta: { color: theme.textMuted, fontSize: 11 },
    pageText: { color: theme.text, fontSize: 13, lineHeight: 20 },
    pageError: { color: theme.danger, fontSize: 12 },
    pageActions: { flexDirection: "row", gap: 8, marginTop: 2 },
    pageActionBtn: {
      paddingVertical: 6,
      paddingHorizontal: 12,
      borderRadius: 6,
      borderWidth: 1,
      borderColor: theme.border,
    },
    pageActionBtnDanger: {
      paddingVertical: 6,
      paddingHorizontal: 12,
      borderRadius: 6,
      borderWidth: 1,
      borderColor: `${theme.danger}55`,
    },
    pageActionText: { color: theme.textSecondary, fontSize: 12 },
  });
}
