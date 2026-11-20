import { Pressable, ScrollView, StatusBar, StyleSheet, Text, View } from "react-native";
import { learningDesign } from "../ui/learningDesign";
import { useAppTheme } from "../ui/ThemeContext";

type ProfileDestination = "stats" | "ocr" | "meditation" | "settings";

type ProfileScreenProps = {
  onOpen: (key: ProfileDestination) => void;
  onOpenSentenceSearch: () => void;
};

type ProfileRow = {
  key: ProfileDestination | "sentences";
  title: string;
  detail: string;
};

const LEARNING_ROWS: ProfileRow[] = [
  {
    key: "stats",
    title: "学习统计",
    detail: "复习热力图、听力时长、跟读趋势与番茄钟。",
  },
  {
    key: "sentences",
    title: "句型搜索",
    detail: "搜本机音频和视频字幕，跳到对应句子。",
  },
];

const TOOL_ROWS: ProfileRow[] = [
  {
    key: "ocr",
    title: "拍照识字",
    detail: "拍照或选图，提取文字并保存记录。",
  },
  {
    key: "meditation",
    title: "冥想放松",
    detail: "自定义冥想时长与背景音乐。",
  },
];

export default function ProfileScreen({ onOpen, onOpenSentenceSearch }: ProfileScreenProps) {
  const { theme, scheme } = useAppTheme();
  const styles = makeStyles(theme);

  const open = (key: ProfileDestination | "sentences") => {
    if (key === "sentences") onOpenSentenceSearch();
    else onOpen(key);
  };

  const renderRow = (row: ProfileRow) => (
    <Pressable
      key={row.key}
      accessibilityRole="button"
      accessibilityLabel={row.title}
      onPress={() => open(row.key)}
      style={({ pressed }) => [styles.row, pressed && styles.pressed]}
    >
      <View style={styles.rowText}>
        <Text style={styles.rowTitle}>{row.title}</Text>
        <Text style={styles.rowDetail}>{row.detail}</Text>
      </View>
      <Text style={styles.chevron}>›</Text>
    </Pressable>
  );

  return (
    <View style={styles.safeArea}>
      <StatusBar barStyle={scheme === "dark" ? "light-content" : "dark-content"} />
      <ScrollView
        style={styles.container}
        contentContainerStyle={styles.content}
        showsVerticalScrollIndicator={false}
      >
        <View style={styles.header}>
          <Text style={styles.eyebrow}>PROFILE</Text>
          <Text style={styles.title}>我的</Text>
          <Text style={styles.subtitle}>学习数据、跨材料查找、工具与应用设置。</Text>
        </View>

        <View style={styles.panel}>
          <Text style={styles.sectionLabel}>学习</Text>
          {LEARNING_ROWS.map(renderRow)}
        </View>

        <View style={styles.panel}>
          <Text style={styles.sectionLabel}>工具</Text>
          {TOOL_ROWS.map(renderRow)}
        </View>

        <View style={styles.panel}>
          <Text style={styles.sectionLabel}>应用</Text>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="设置"
            onPress={() => onOpen("settings")}
            style={({ pressed }) => [styles.row, styles.lastRow, pressed && styles.pressed]}
          >
            <View style={styles.rowText}>
              <Text style={styles.rowTitle}>设置</Text>
              <Text style={styles.rowDetail}>模型、同步与备份、外观。</Text>
            </View>
            <Text style={styles.chevron}>›</Text>
          </Pressable>
        </View>
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
    header: { ...ui.header, marginBottom: 0 },
    eyebrow: { color: theme.textMuted, fontSize: 11, fontWeight: "700", letterSpacing: 1.2 },
    title: { ...ui.title, marginTop: 2 },
    subtitle: { ...ui.subtitle },
    panel: { ...ui.panel, paddingVertical: 6 },
    sectionLabel: {
      color: theme.textMuted,
      fontSize: 11,
      fontWeight: "700",
      letterSpacing: 0.4,
      paddingHorizontal: 14,
      paddingTop: 10,
      paddingBottom: 4,
    },
    row: {
      minHeight: 62,
      flexDirection: "row",
      alignItems: "center",
      gap: 12,
      paddingHorizontal: 14,
      paddingVertical: 10,
      borderRadius: 14,
    },
    lastRow: { marginBottom: 2 },
    rowText: { flex: 1, gap: 2 },
    rowTitle: { color: theme.text, fontSize: 15, fontWeight: "600" },
    rowDetail: { color: theme.textMuted, fontSize: 12, lineHeight: 17 },
    chevron: { color: theme.textMuted, fontSize: 22 },
    pressed: { opacity: 0.68 },
  });
}
