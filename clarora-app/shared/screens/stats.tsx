import { learningDesign } from "../ui/learningDesign";
import { useCallback, useEffect, useState } from "react";
import { Pressable, ScrollView, StatusBar, StyleSheet, Text, View } from "react-native";
import {
  getStudyStats,
  type StudyStats,
} from "../data/database";
import {
  listSpeakingAttempts,
  type SpeakingAttemptSummary,
} from "../services/ai";
import { useAppTheme } from "../ui/ThemeContext";

// 学习统计：复习量热力图 + 听力时长 + 跟读分数趋势 + 番茄钟轮数。
// 数据源：study_stats（设备本地）与本机跟读记录。

const HEATMAP_WEEKS = 17;
const TREND_BARS = 20;
const LISTEN_BARS = 14;

function fmtDuration(totalSeconds: number): string {
  const minutes = Math.floor(totalSeconds / 60);
  if (minutes < 60) return `${minutes} 分钟`;
  return `${Math.floor(minutes / 60)} 小时 ${minutes % 60} 分`;
}

function heatColor(count: number, accent: string, surface: string): string {
  if (count <= 0) return surface;
  if (count < 5) return `${accent}40`;
  if (count < 10) return `${accent}73`;
  if (count < 20) return `${accent}b3`;
  return accent;
}

function trendColor(accuracy: number): string {
  if (accuracy >= 90) return "#2e7d32";
  if (accuracy >= 70) return "#b8860b";
  return "#c0392b";
}

export default function StatsScreen() {
  const { theme, scheme } = useAppTheme();
  const styles = makeStyles(theme);
  const [stats, setStats] = useState<StudyStats | null>(null);
  const [attempts, setAttempts] = useState<SpeakingAttemptSummary[]>([]);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setError(null);
    try {
      const [study, recent] = await Promise.all([
        getStudyStats(HEATMAP_WEEKS * 7),
        listSpeakingAttempts(100).catch(() => [] as SpeakingAttemptSummary[]),
      ]);
      setStats(study);
      setAttempts(
        recent
          .filter((item) => item.status === "completed" && item.score)
          .slice(0, TREND_BARS)
      );
    } catch (e: any) {
      setError(`加载统计失败：${e?.message ?? e}`);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const heatDays = stats?.days ?? [];
  const heatColumns: Array<typeof heatDays> = [];
  for (let i = 0; i < heatDays.length; i += 7) {
    heatColumns.push(heatDays.slice(i, i + 7));
  }

  const listenDays = (stats?.days ?? []).slice(-LISTEN_BARS);
  const listenMax = Math.max(300, ...listenDays.map((day) => day.listenSeconds));

  const trend = [...attempts].reverse();
  const avgAccuracy = trend.length
    ? Math.round(trend.reduce((total, item) => total + (item.score?.accuracy ?? 0), 0) / trend.length)
    : 0;

  const overviewChip = (label: string, value: string) => (
    <View key={label} style={styles.overviewChip}>
      <Text style={styles.overviewValue}>{value}</Text>
      <Text style={styles.overviewLabel}>{label}</Text>
    </View>
  );

  return (
    <View style={styles.safeArea}>
      <StatusBar barStyle={scheme === "dark" ? "light-content" : "dark-content"} />
      <ScrollView style={styles.container} contentContainerStyle={styles.content} showsVerticalScrollIndicator={false}>
        <View style={styles.header}>
          <Text style={styles.eyebrow}>STATS</Text>
          <Text style={styles.title}>学习统计</Text>
          <Text style={styles.subtitle}>复习量、听力时长、跟读分数与番茄钟，坚持看得见。</Text>
        </View>

        {error ? (
          <View style={styles.errorBanner}>
            <Text style={styles.errorText}>{error}</Text>
          </View>
        ) : null}

        {stats ? (
          <View style={styles.panel}>
            <Text style={styles.panelLabel}>总览</Text>
            <View style={styles.overviewRow}>
              {overviewChip("今日听力", fmtDuration(stats.listenTodaySec))}
              {overviewChip("本周听力", fmtDuration(stats.listenWeekSec))}
              {overviewChip("今日复习", `${stats.reviewsToday} 题`)}
              {overviewChip("本周复习", `${stats.reviewsWeek} 题`)}
              {overviewChip("今日番茄钟", `${stats.pomodoroToday} 轮`)}
              {overviewChip("番茄钟累计", `${stats.pomodoroTotal} 轮`)}
            </View>
          </View>
        ) : null}

        {stats ? (
          <View style={styles.panel}>
            <View style={styles.panelHeader}>
              <Text style={styles.panelLabel}>复习热力图</Text>
              <Text style={styles.panelMeta}>最近 {HEATMAP_WEEKS} 周 · 共 {stats.days.reduce((t, d) => t + d.reviewCount, 0)} 题</Text>
            </View>
            <View style={styles.heatmapRow}>
              {heatColumns.map((column, columnIndex) => (
                <View key={columnIndex} style={styles.heatmapColumn}>
                  {column.map((day) => (
                    <View
                      key={day.day}
                      style={[
                        styles.heatmapCell,
                        { backgroundColor: heatColor(day.reviewCount, theme.accent, theme.surface) },
                      ]}
                    />
                  ))}
                </View>
              ))}
            </View>
            <View style={styles.heatmapLegend}>
              <Text style={styles.legendText}>少</Text>
              {[0, 4, 9, 19].map((count) => (
                <View
                  key={count}
                  style={[styles.legendCell, { backgroundColor: heatColor(count, theme.accent, theme.surface) }]}
                />
              ))}
              <Text style={styles.legendText}>多</Text>
            </View>
          </View>
        ) : null}

        {stats ? (
          <View style={styles.panel}>
            <View style={styles.panelHeader}>
              <Text style={styles.panelLabel}>听力时长 · 最近 {LISTEN_BARS} 天</Text>
              <Text style={styles.panelMeta}>累计 {fmtDuration(stats.listenTotalSec)}</Text>
            </View>
            <View style={styles.barRow}>
              {listenDays.map((day) => (
                <View key={day.day} style={styles.barColumn}>
                  <View
                    style={[
                      styles.bar,
                      {
                        height: 6 + Math.round((day.listenSeconds / listenMax) * 84),
                        backgroundColor: day.listenSeconds > 0 ? theme.accent : theme.surface,
                      },
                    ]}
                  />
                  <Text style={styles.barLabel}>{day.day.slice(8)}</Text>
                </View>
              ))}
            </View>
          </View>
        ) : null}

        <View style={styles.panel}>
          <View style={styles.panelHeader}>
            <Text style={styles.panelLabel}>跟读准确度 · 最近 {trend.length || 0} 次</Text>
            <Text style={styles.panelMeta}>{trend.length ? `平均 ${avgAccuracy}%` : "暂无记录"}</Text>
          </View>
          {trend.length > 0 ? (
            <>
              <View style={styles.barRow}>
                {trend.map((item, index) => {
                  const accuracy = item.score?.accuracy ?? 0;
                  return (
                    <View key={item.attemptId || index} style={styles.barColumn}>
                      <View
                        style={[
                          styles.bar,
                          { height: 6 + Math.round((accuracy / 100) * 84), backgroundColor: trendColor(accuracy) },
                        ]}
                      />
                      <Text style={styles.barLabel}>{index + 1}</Text>
                    </View>
                  );
                })}
              </View>
              <Text style={styles.trendHint}>
                绿 ≥90% · 黄 ≥70% · 红 &lt;70%；完整度与语速在口语页查看详情。
              </Text>
            </>
          ) : (
            <Text style={styles.emptyHint}>
              还没有跟读评分记录，去听力页点「🎤 跟读」或到口语页练习。
            </Text>
          )}
        </View>

        <Pressable style={styles.refreshBtn} onPress={() => void load()}>
          <Text style={styles.refreshText}>刷新统计</Text>
        </Pressable>
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
    panelHeader: { flexDirection: "row", justifyContent: "space-between", alignItems: "baseline" },
    panelMeta: { color: theme.textMuted, fontSize: 11 },

    overviewRow: { flexDirection: "row", flexWrap: "wrap", gap: 8 },
    overviewChip: {
      alignItems: "center",
      paddingVertical: 8,
      paddingHorizontal: 12,
      borderRadius: 8,
      backgroundColor: theme.bg,
      borderWidth: 1,
      borderColor: theme.border,
      minWidth: 88,
    },
    overviewValue: { color: theme.accent, fontSize: 14, fontWeight: "700" },
    overviewLabel: { color: theme.textMuted, fontSize: 10, marginTop: 2 },

    heatmapRow: { flexDirection: "row", gap: 3 },
    heatmapColumn: { gap: 3 },
    heatmapCell: { width: 13, height: 13, borderRadius: 3, borderWidth: 1, borderColor: theme.border },
    heatmapLegend: { flexDirection: "row", alignItems: "center", gap: 4 },
    legendCell: { width: 11, height: 11, borderRadius: 3, borderWidth: 1, borderColor: theme.border },
    legendText: { color: theme.textMuted, fontSize: 10 },

    barRow: { flexDirection: "row", alignItems: "flex-end", gap: 5 },
    barColumn: { alignItems: "center", gap: 4 },
    bar: { width: 14, borderRadius: 3 },
    barLabel: { color: theme.textMuted, fontSize: 9 },
    trendHint: { color: theme.textMuted, fontSize: 11 },
    emptyHint: { color: theme.textSecondary, fontSize: 13, lineHeight: 20 },

    errorBanner: {
      backgroundColor: `${theme.danger}14`,
      borderRadius: 6,
      paddingVertical: 10,
      paddingHorizontal: 14,
    },
    errorText: { color: theme.danger, fontSize: 13 },

    refreshBtn: {
      alignSelf: "center",
      paddingVertical: 10,
      paddingHorizontal: 24,
      borderWidth: 1,
      borderColor: theme.border,
      backgroundColor: theme.surface,
      ...ui.button,
    },
    refreshText: { color: theme.accent, fontSize: 13, fontWeight: "700" },
  });
}
