import { useCallback, useEffect, useRef, useState } from "react";
import {
  ActivityIndicator,
  KeyboardAvoidingView,
  Platform,
  processColor,
  useWindowDimensions,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import { MarkdownView } from "./markdown";
import { NativeSelectableSubtitleView } from "./NativeSelectableSubtitle";
import { useAppTheme } from "./ThemeContext";

// 统一的 AI 助手对话面板：完整展示多轮对话（上下文挂载的选文字幕 +
// 问答历史），底部输入框发送/追问，回答可复制或存为问答闪卡。

import { type ChatTurn } from "../data/chat";

type ChatPanelProps = {
  visible: boolean;
  messages: ChatTurn[];
  busy: boolean;
  /** 挂载的上下文（选中的字幕原文），回答都围绕它展开。 */
  contextText: string;
  error: string | null;
  source: string;
  pending: string;
  /** 流式增量：reasoning = 思考过程，answer = 已生成的回答文本。 */
  stream: { reasoning: string; answer: string } | null;
  notice: string;
  draft: string;
  onDraftChange: (text: string) => void;
  onReference: (text: string) => void;
  onCollectReference: () => void;
  sessions: { id: string; title: string }[];
  sessionId: string;
  onSelectSession: (id: string) => void;
  onClose: () => void;
  onSend: (text: string) => void;
  onCancel: () => void;
  /** 开启新一轮对话（清空历史，上下文不变）。 */
  onNewConversation: () => void;
  onSaveCard: (question: string, answer: string) => void;
  onCopy: (text: string) => void;
};

/** 找到某条回答对应的提问（向上最近的一条 user 消息）。 */
function questionFor(messages: ChatTurn[], assistantIndex: number): string {
  for (let i = assistantIndex - 1; i >= 0; i--) {
    if (messages[i].role === "user") return messages[i].content;
  }
  return "";
}

export function ChatPanel(props: ChatPanelProps) {
  const { theme } = useAppTheme();
  const styles = makeStyles(theme);
  const { draft, onDraftChange: setDraft } = props;
  const [referenceExpanded, setReferenceExpanded] = useState(false);
  const [historyExpanded, setHistoryExpanded] = useState(false);
  // null = 跟随自动策略（思考中展开、回答开始后收起），true/false = 用户手动指定。
  const [thinkingPinned, setThinkingPinned] = useState<boolean | null>(null);
  useEffect(() => { if (props.busy) setThinkingPinned(null); }, [props.busy]);
  const [range, setRange] = useState({ start: 0, end: 0 });
  const { width } = useWindowDimensions();
  useEffect(() => { setRange({ start: 0, end: 0 }); setReferenceExpanded(false); }, [props.sessionId]);
  const scrollRef = useRef<ScrollView>(null);
  const { visible, messages, busy } = props;

  useEffect(() => {
    if (visible) scrollRef.current?.scrollToEnd({ animated: false });
  }, [visible, messages.length, busy, props.stream?.answer, props.stream?.reasoning]);

  const submit = useCallback(() => {
    const text = draft.trim();
    if (!text || busy) return;
    props.onSend(text);
  }, [draft, busy, props]);

  if (!visible) return null;

  const contextBrief = props.contextText
    ? props.contextText.replace(/\s+/g, " ").slice(0, 48) + (props.contextText.length > 48 ? "…" : "")
    : "";

  return (
    <KeyboardAvoidingView behavior={Platform.OS === "ios" ? "padding" : undefined} style={styles.overlay}>
      <Pressable accessibilityRole="button" style={styles.dismissArea} onPress={props.onClose} />
      <View style={[styles.panel, width < 700 && { width: "100%", maxWidth: "100%", borderRadius: 0 }]}>
        <View style={styles.header}>
          <View style={styles.headerCopy}>
            <Text style={styles.title}>AI 学习助手</Text>
            <Text style={styles.context} numberOfLines={1}>
              {props.source} · 对话保存在本机
            </Text>
          </View>
          <View style={styles.headerActions}>
            <Pressable accessibilityRole="button" onPress={() => setHistoryExpanded(v => !v)}><Text style={styles.actionText}>历史</Text></Pressable>
            <Pressable accessibilityRole="button" onPress={props.onNewConversation} hitSlop={6}>
              <Text style={styles.actionText}>新对话</Text>
            </Pressable>
            <Pressable accessibilityRole="button" onPress={props.onClose} hitSlop={6}>
              <Text style={styles.closeText}>关闭</Text>
            </Pressable>
          </View>
        </View>

        {historyExpanded && <ScrollView style={{ maxHeight: 130 }}>
          {props.sessions.map(session => <Pressable accessibilityRole="button" key={session.id} onPress={() => { props.onSelectSession(session.id); setHistoryExpanded(false); }} style={{ padding: 10, backgroundColor: session.id === props.sessionId ? theme.surfaceHover : theme.bg }}>
            <Text numberOfLines={1} style={{ color: theme.text }}>{session.title}</Text>
          </Pressable>)}
        </ScrollView>}
        {!!props.contextText && <View style={{ backgroundColor: theme.surface, borderRadius: 12, padding: 12, gap: 8 }}>
          <View style={{ flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between', gap: 8 }}>
            <Text style={styles.context}>已引用 {props.contextText.length} 字</Text>
            <Pressable accessibilityRole="button" disabled={busy} onPress={props.onCollectReference} style={{ padding: 8, borderRadius: 8, backgroundColor: theme.surfaceHover, opacity: busy ? 0.45 : 1 }}>
              <Text style={styles.actionText}>＋ 继续选文</Text>
            </Pressable>
          </View>
          <Pressable accessibilityRole="button" onPress={() => setReferenceExpanded(v => !v)}>
            <Text style={styles.actionText}>{referenceExpanded ? '收起引用 ▴' : '引用材料 · 点击查看或选取片段 ▾'}</Text>
            {!referenceExpanded && <Text numberOfLines={2} style={styles.context}>{contextBrief}</Text>}
          </Pressable>
          {referenceExpanded && <>
            <Text style={styles.context}>可一次拖选多句。点击「继续选文」回到学习页，再选片段即可合并；下方选文提问则单独讨论该片段。</Text>
            {NativeSelectableSubtitleView ? <NativeSelectableSubtitleView text={props.contextText} selectionEnabled fontSize={17} textColor={processColor(theme.text)} style={{ height: 150 }} onAskSelection={e => props.onReference(e.nativeEvent.text)} />
              : <><TextInput value={props.contextText} editable={false} multiline onSelectionChange={e => setRange(e.nativeEvent.selection)} style={{ height: 130, color: theme.text }} />
                <Pressable accessibilityRole="button" disabled={range.start === range.end} onPress={() => props.onReference(props.contextText.slice(range.start, range.end))}><Text style={styles.actionText}>引用所选片段</Text></Pressable></>}
          </>}
        </View>}
        <ScrollView
          ref={scrollRef}
          style={styles.body}
          contentContainerStyle={styles.bodyContent}
          keyboardShouldPersistTaps="handled"
          onContentSizeChange={() => scrollRef.current?.scrollToEnd({ animated: true })}
          nestedScrollEnabled
        >
          {messages.length === 0 && !busy ? (
            <View style={{ gap: 16, paddingVertical: 20 }}>
              <Text style={{ color: theme.text, fontSize: 22, fontWeight: '700' }}>把不懂的地方，聊明白。</Text>
              <Text style={styles.emptyText}>{props.contextText ? '材料已引用。直接输入问题，或从下面开始。' : '可以问词义、语法和表达，也可以从学习页面带入材料。'}</Text>
              {(props.contextText ? ['解释这段内容', '拆解重点词汇和语法', '出一道题，检验我是否理解'] : ['帮我区分 used to 和 be used to', '陪我练习一段日常英语对话']).map(prompt => <Pressable accessibilityRole="button" key={prompt} onPress={() => setDraft(prompt)} style={{ padding: 12, borderRadius: 10, borderWidth: 1, borderColor: theme.border }}><Text style={styles.actionText}>{prompt} ↗</Text></Pressable>)}
            </View>
          ) : null}
          {messages.map((turn, index) =>
            turn.role === "user" ? (
              <View key={index} style={[styles.bubble, styles.userBubble]}>
                <Text style={[styles.bubbleText, styles.userBubbleText]}>{turn.content}</Text>
              </View>
            ) : (
              <View key={index} style={[styles.bubble, styles.botBubble]}>
                <MarkdownView text={turn.content} theme={theme} baseFontSize={16} />
                <View style={styles.messageActions}>
                  <Pressable accessibilityRole="button" onPress={() => props.onCopy(turn.content)} hitSlop={6}>
                    <Text style={styles.messageAction}>复制</Text>
                  </Pressable>
                  <Pressable
                    onPress={() => props.onSaveCard(questionFor(messages, index), turn.content)}
                    hitSlop={6}
                  >
                    <Text style={styles.messageAction}>存为闪卡</Text>
                  </Pressable>
                </View>
              </View>
            )
          )}
          {!!props.pending && <View style={[styles.bubble, styles.userBubble]}><Text style={styles.bubbleText}>{props.pending}</Text></View>}
          {busy ? (
            <View style={[styles.bubble, styles.botBubble]}>
              {!!props.stream?.reasoning && (
                <View style={styles.thinkingBox}>
                  <Pressable accessibilityRole="button" hitSlop={6}
                    onPress={() => setThinkingPinned(!(thinkingPinned ?? (props.stream!.answer === '')))}>
                    <Text style={styles.thinkingToggle}>
                      {props.stream.answer ? '思考过程' : '思考中'}
                      {(thinkingPinned ?? (props.stream.answer === '')) ? ' ▾' : ' ▴'}
                    </Text>
                  </Pressable>
                  {(thinkingPinned ?? (props.stream.answer === '')) && (
                    <Text style={styles.thinkingText} numberOfLines={thinkingPinned === false ? 3 : undefined}>
                      {props.stream.reasoning}
                    </Text>
                  )}
                </View>
              )}
              {props.stream?.answer ? (
                <MarkdownView text={props.stream.answer} theme={theme} baseFontSize={16} />
              ) : (
                <View style={styles.busyBubble}>
                  <ActivityIndicator size="small" color={theme.accent} />
                  <Text style={styles.busyText}>
                    {props.stream?.reasoning ? '思考中，马上作答…' : '正在回答…'}
                  </Text>
                </View>
              )}
            </View>
          ) : null}
        </ScrollView>

        {props.error ? <Text accessibilityRole="alert" style={styles.errorText}>{props.error} 点击发送可重试。</Text> : null}
        {!!props.notice && <Text accessibilityLiveRegion="polite" style={styles.actionText}>{props.notice}</Text>}

        <View style={styles.inputRow}>
          <TextInput
            style={styles.input}
            value={draft}
            onChangeText={setDraft}
            accessibilityLabel="给 AI 学习助手发消息"
            placeholder={messages.length ? "继续追问…" : "想弄懂什么？"}
            maxLength={2000}
            multiline
            blurOnSubmit={false}
            placeholderTextColor={theme.textMuted}
            editable={!busy}
          />
          {busy ? (
            <Pressable accessibilityRole="button" style={[styles.sendBtn, styles.cancelBtn]} onPress={props.onCancel}>
              <Text style={styles.sendText}>取消</Text>
            </Pressable>
          ) : (
            <Pressable
              style={[styles.sendBtn, !draft.trim() && styles.sendDisabled]}
              onPress={submit}
              disabled={!draft.trim()}
            >
              <Text style={styles.sendText}>发送</Text>
            </Pressable>
          )}
        </View>
      </View>
    </KeyboardAvoidingView>
  );
}

function makeStyles(theme: ReturnType<typeof useAppTheme>["theme"]) {
  return StyleSheet.create({
    overlay: {
      ...StyleSheet.absoluteFillObject,
      backgroundColor: "rgba(0,0,0,0.45)",
      alignItems: "flex-end",
      justifyContent: "center",
      zIndex: 100,
    },
    dismissArea: { ...StyleSheet.absoluteFillObject },
    panel: {
      width: 560,
      maxWidth: "100%",
      height: "100%",
      maxHeight: "100%",
      backgroundColor: theme.bg,
      borderWidth: 1,
      borderColor: theme.border,
      borderRadius: 12,
      padding: 20,
      gap: 10,
    },
    header: { flexDirection: "row", justifyContent: "space-between", alignItems: "center" },
    headerCopy: { flex: 1, marginRight: 10, gap: 2 },
    title: { color: theme.accent, fontSize: 20, fontWeight: "800" },
    context: { color: theme.textMuted, fontSize: 13 },
    headerActions: { flexDirection: "row", alignItems: "center", gap: 14 },
    actionText: { color: theme.accent, fontSize: 12, fontWeight: "700" },
    closeText: { color: theme.textSecondary, fontSize: 12 },

    body: { flex: 1 },
    bodyContent: { gap: 8, paddingVertical: 4 },
    emptyText: { color: theme.textSecondary, fontSize: 16, lineHeight: 24, textAlign: "left" },

    bubble: {
      borderRadius: 10,
      paddingVertical: 8,
      paddingHorizontal: 12,
      maxWidth: "88%",
    },
    userBubble: {
      alignSelf: "flex-end",
      backgroundColor: `${theme.accent}26`,
    },
    botBubble: {
      alignSelf: "flex-start",
      backgroundColor: theme.surface,
      borderWidth: 1,
      borderColor: theme.border,
    },
    bubbleText: { color: theme.text, fontSize: 16, lineHeight: 24 },
    userBubbleText: { color: theme.text },
    messageActions: { flexDirection: "row", gap: 14, marginTop: 6 },
    messageAction: { color: theme.accent, fontSize: 11, fontWeight: "700" },
    busyBubble: { flexDirection: "row", alignItems: "center", gap: 8 },
    thinkingBox: {
      marginBottom: 8,
      paddingVertical: 6,
      paddingHorizontal: 10,
      borderRadius: 8,
      backgroundColor: theme.bg,
      gap: 4,
    },
    thinkingToggle: { color: theme.textMuted, fontSize: 11, fontWeight: "700" },
    thinkingText: { color: theme.textMuted, fontSize: 12, lineHeight: 18 },
    busyText: { color: theme.textSecondary, fontSize: 13 },

    errorText: { color: theme.danger, fontSize: 12 },

    inputRow: { flexDirection: "row", alignItems: "center", gap: 10 },
    input: {
      flex: 1,
      borderWidth: 1,
      borderColor: theme.border,
      borderRadius: 8,
      paddingHorizontal: 10,
      paddingVertical: 8,
      color: theme.text,
      fontSize: 16,
      minHeight: 64,
      maxHeight: 140,
      textAlignVertical: "top",
      backgroundColor: theme.surface,
    },
    sendBtn: {
      paddingVertical: 9,
      paddingHorizontal: 16,
      borderRadius: 8,
      backgroundColor: theme.accent,
      alignItems: "center",
    },
    sendDisabled: { opacity: 0.4 },
    cancelBtn: { backgroundColor: theme.danger },
    sendText: { color: "#fff", fontSize: 13, fontWeight: "700" },
  });
}
