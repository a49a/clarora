import { useAIChatEntry } from "../ui/AIChatProvider";
import { Details } from "../ui/Details";
import { learningDesign } from "../ui/learningDesign";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  Alert,
  PermissionsAndroid,
  Platform,
  Pressable,
  ScrollView,
  StatusBar,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import {
  deleteSpeakingAttempt,
  listSpeakingAttempts,
  speakingAttempt,
  waitForSpeaking,
  type SpeakingAttemptSummary,
  type SpeakingScore,
} from "../services/ai";
import { audioRecorder } from "../services/platform";
import { useAppTheme } from "../ui/ThemeContext";

// 口语跟读：输入/粘贴一句英文 → 录音跟读 → 本机 ASR + 词级对齐打分 +
// LLM 点评;历史记录存在本机,可通过存储备份合并。

export default function SpeakingScreen() {
  const { theme, scheme } = useAppTheme();
  const styles = makeStyles(theme);

  const [reference, setReference] = useState("");
  const [recording, setRecording] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  const [lastScore, setLastScore] = useState<SpeakingScore | null>(null);
  const [lastDetail, setLastDetail] = useState<{
    reference: string;
    transcript: string;
    feedback: string;
  } | null>(null);
  const [attempts, setAttempts] = useState<SpeakingAttemptSummary[]>([]);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const showToast = useCallback((msg: string) => {
    setToast(msg);
    setTimeout(() => setToast(null), 2300);
  }, []);

  const refresh = useCallback(async () => {
    try {
      setAttempts(await listSpeakingAttempts());
    } catch (e: any) {
      setError(`获取跟读记录失败：${e?.message ?? e}`);
    }
  }, []);

  useEffect(() => {
    void refresh();
    return () => {
      if (timerRef.current) clearInterval(timerRef.current);
    };
  }, [refresh]);

  const startRecording = useCallback(async () => {
    if (!reference.trim()) {
      showToast("先在上方输入要跟读的英文句子");
      return;
    }
    setError(null);
    if (Platform.OS === "android") {
      const granted = await PermissionsAndroid.request(
        PermissionsAndroid.PERMISSIONS.RECORD_AUDIO,
        {
          title: "麦克风权限",
          message: "口语跟读需要使用麦克风录制你的朗读",
          buttonPositive: "确定",
        }
      ).catch(() => "denied");
      if (granted !== PermissionsAndroid.RESULTS.GRANTED) {
        setError("麦克风权限被拒绝，请在系统设置中允许");
        return;
      }
    }
    try {
      await audioRecorder.start();
      setRecording(true);
      setElapsed(0);
      timerRef.current = setInterval(() => setElapsed((s) => s + 1), 1000);
    } catch (e: any) {
      setError(`录音失败：${e?.message ?? e}`);
    }
  }, [reference, showToast]);

  const stopAndSubmit = useCallback(async () => {
    if (timerRef.current) clearInterval(timerRef.current);
    setRecording(false);
    setBusy("正在上传录音…");
    setError(null);
    try {
      const file = await audioRecorder.stop();
      const { attemptId } = await speakingAttempt(file.uri, file.name, reference.trim());
      setBusy("正在转写与评分…");
      const detail = await waitForSpeaking(attemptId, () => undefined);
      setLastScore(detail.score);
      setLastDetail({
        reference: detail.reference ?? reference.trim(),
        transcript: detail.transcript ?? "",
        feedback: detail.feedback ?? "",
      });
      setBusy(null);
      showToast("评分完成");
      await refresh();
    } catch (e: any) {
      setBusy(null);
      setError(`跟读评分失败：${e?.message ?? e}`);
    }
  }, [reference, refresh, showToast]);

  const removeAttempt = useCallback(
    (attempt: SpeakingAttemptSummary) => {
      Alert.alert("删除跟读记录？", "删除后不可恢复。", [
        { text: "取消", style: "cancel" },
        {
          text: "删除",
          style: "destructive",
          onPress: () => {
            void (async () => {
              await deleteSpeakingAttempt(attempt.attemptId).catch(() => {});
              if (expandedId === attempt.attemptId) setExpandedId(null);
              await refresh();
            })();
          },
        },
      ]);
    },
    [expandedId, refresh]
  );

  const scoreChip = (label: string, value: string | number) => (
    <View key={label} style={styles.scoreChip}>
      <Text style={styles.scoreChipValue}>{value}</Text>
      <Text style={styles.scoreChipLabel}>{label}</Text>
    </View>
  );

  const renderDiff = (score: SpeakingScore) => (
    <View style={styles.diffBox}>
      {score.missed.length > 0 && (
        <Text style={styles.diffLine}>
          <Text style={styles.diffLabel}>漏读：</Text>
          <Text style={styles.diffMissed}>{score.missed.join("、")}</Text>
        </Text>
      )}
      {score.wrong.length > 0 && (
        <Text style={styles.diffLine}>
          <Text style={styles.diffLabel}>读错：</Text>
          <Text style={styles.diffWrong}>{score.wrong.join("、")}</Text>
        </Text>
      )}
      {score.extra.length > 0 && (
        <Text style={styles.diffLine}>
          <Text style={styles.diffLabel}>多读：</Text>
          <Text style={styles.diffExtra}>{score.extra.join("、")}</Text>
        </Text>
      )}
    </View>
  );

  const renderAttempt = (attempt: SpeakingAttemptSummary) => {
    const expanded = expandedId === attempt.attemptId;
    const statusLabel =
      attempt.status === "completed"
        ? "✓"
        : attempt.status === "failed"
        ? "⚠️"
        : "⏳";
    return (
      <View key={attempt.attemptId} style={styles.attemptCard}>
        <Pressable
          style={styles.attemptHeader}
          onPress={() => {
            if (attempt.status === "completed") {
              setExpandedId(expanded ? null : attempt.attemptId);
            }
          }}
        >
          <Text style={styles.attemptTitle} numberOfLines={expanded ? undefined : 2}>
            {statusLabel} {attempt.reference || "（无原句）"}
          </Text>
          <Text style={styles.attemptMeta}>
            {attempt.status === "failed"
              ? "评分失败"
              : attempt.status === "completed"
              ? "已完成"
              : "处理中…"}
          </Text>
        </Pressable>
        {expanded ? (
          <>
            {attempt.transcript ? (
              <>
                <Text style={styles.attemptSectionLabel}>我的转写</Text>
                <Text style={styles.attemptBody}>{attempt.transcript}</Text>
              </>
            ) : null}
            {attempt.feedback ? (
              <>
                <Text style={styles.attemptSectionLabel}>教练点评</Text>
                <Text style={styles.attemptBody}>{attempt.feedback}</Text>
              </>
            ) : null}
            {attempt.error ? <Text style={styles.attemptBody}>{attempt.error}</Text> : null}
            <Pressable style={styles.attemptDelete} onPress={() => removeAttempt(attempt)}>
              <Text style={[styles.attemptBody, { color: theme.danger }]}>删除记录</Text>
            </Pressable>
          </>
        ) : null}
      </View>
    );
  };

  useAIChatEntry(lastDetail ? `原句：${lastDetail.reference}\n我的转写：${lastDetail.transcript || ''}\n教练点评：${lastDetail.feedback || ''}` : "", "口语跟读");

  return (
    <View style={styles.safeArea}>
      <StatusBar barStyle={scheme === "dark" ? "light-content" : "dark-content"} />
      <ScrollView style={styles.container} contentContainerStyle={styles.content} showsVerticalScrollIndicator={false}>
        <View style={styles.header}>
          <Text style={styles.eyebrow}>SPEAKING</Text>
          <Text style={styles.title}>口语跟读</Text>
          <Text style={styles.subtitle}>
            粘贴一句英文，朗读录音；本机转写后打分（完整度 / 准确度 / 语速）并给出点评。
          </Text>
        </View>

        <View style={styles.panel}>
          <Text style={styles.panelLabel}>跟读句子</Text>
          <TextInput
            style={styles.referenceInput}
            value={reference}
            onChangeText={setReference}
            placeholder="输入或粘贴要跟读的英文句子"
            placeholderTextColor={theme.textMuted}
            multiline
          />
          {recording ? (
            <Pressable style={[styles.recordBtn, styles.recordBtnStop]} onPress={() => { void stopAndSubmit(); }}>
              <Text style={styles.recordBtnText}>⏹ 停止并评分（{elapsed}s）</Text>
            </Pressable>
          ) : (
            <Pressable style={styles.recordBtn} onPress={() => { void startRecording(); }}>
              <Text style={styles.recordBtnText}>● 开始录音跟读</Text>
            </Pressable>
          )}
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

        {lastDetail && lastScore ? (
          <View style={styles.panel}>
            <Text style={styles.panelLabel}>最近一次评分</Text>
            <View style={styles.scoreRow}>
              {scoreChip("完整度", `${lastScore.completeness}%`)}
              {scoreChip("准确度", `${lastScore.accuracy}%`)}
              {scoreChip("语速", `${lastScore.wpm} wpm`)}
            </View>
            <Text style={styles.sectionLabel}>原句</Text>
            <Text style={styles.bodyText}>{lastDetail.reference}</Text>
            <Text style={styles.sectionLabel}>我的转写</Text>
            <Text style={styles.bodyText}>{lastDetail.transcript || "（无识别结果）"}</Text>
            {renderDiff(lastScore)}
            {lastDetail.feedback ? (
              <>
                <Text style={styles.sectionLabel}>教练点评</Text>
                <Text style={styles.bodyText}>{lastDetail.feedback}</Text>
              </>
            ) : null}
          </View>
        ) : null}

        {attempts.length > 0 && (
          <Details title="跟读记录（可通过存储备份合并）">
            <View style={styles.panel}>

            {attempts.map(renderAttempt)}
          </View>
          </Details>
        )}
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
    sectionLabel: { color: theme.textMuted, fontSize: 11, fontWeight: "700", marginTop: 4 },
    bodyText: { color: theme.text, fontSize: 13, lineHeight: 20 },

    referenceInput: {
      borderWidth: 1,
      color: theme.text,
      fontSize: 13,
      minHeight: 56,
      textAlignVertical: "top",
      ...ui.input,
    },

    recordBtn: {
      paddingVertical: 12,
      backgroundColor: theme.accent,
      alignItems: "center",
      ...ui.button,
    },
    recordBtnStop: { backgroundColor: theme.danger },
    recordBtnText: { color: "#fff", fontSize: 14, fontWeight: "700" },
    busyText: { color: theme.textSecondary, fontSize: 13 },

    errorBanner: {
      backgroundColor: `${theme.danger}14`,
      borderRadius: 6,
      paddingVertical: 10,
      paddingHorizontal: 14,
    },
    errorText: { color: theme.danger, fontSize: 13 },

    scoreRow: { flexDirection: "row", gap: 10 },
    scoreChip: {
      alignItems: "center",
      paddingVertical: 8,
      paddingHorizontal: 14,
      borderRadius: 8,
      backgroundColor: theme.bg,
      borderWidth: 1,
      borderColor: theme.border,
    },
    scoreChipValue: { color: theme.accent, fontSize: 16, fontWeight: "700" },
    scoreChipLabel: { color: theme.textMuted, fontSize: 10, marginTop: 2 },

    diffBox: { gap: 4 },
    diffLine: { fontSize: 12, lineHeight: 18 },
    diffLabel: { color: theme.textMuted },
    diffMissed: { color: theme.danger, fontWeight: "600" },
    diffWrong: { color: "#b8860b", fontWeight: "600" },
    diffExtra: { color: theme.textMuted },

    attemptCard: {
      borderWidth: 1,
      borderColor: theme.border,
      padding: 10,
      gap: 6,
      backgroundColor: theme.bg,
      ...ui.row,
    },
    attemptHeader: { gap: 2 },
    attemptTitle: { color: theme.text, fontSize: 13, fontWeight: "600", lineHeight: 19 },
    attemptMeta: { color: theme.textMuted, fontSize: 11 },
    attemptSectionLabel: { color: theme.textMuted, fontSize: 11, fontWeight: "700", marginTop: 4 },
    attemptBody: { color: theme.textSecondary, fontSize: 13, lineHeight: 19 },
    attemptDelete: { alignSelf: "flex-end", paddingVertical: 4, paddingHorizontal: 8 },
  });
}
