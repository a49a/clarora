import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from 'react';
import { BackHandler, Platform, Pressable, SafeAreaView, Text, View } from 'react-native';
import { getSetting, setSetting, saveAiCard } from '../data/database';
import { askAboutPassage, streamChatAnswer } from '../services/ai';
import { copyToClipboard } from '../services/platform';
import { useAppTheme } from './ThemeContext';
import { ChatPanel } from './ChatPanel';
import { newChat, openReference, appendReference, restoreChats, chatHistory, type ChatSession as Session, type ChatReference as Reference } from '../data/chat';

const ChatContext = createContext<{ visible: boolean; open: (reference?: Reference) => void; registerEntry: (entry: () => void) => () => void } | null>(null);
export function useAIChat() {
  const value = useContext(ChatContext);
  if (!value) throw new Error('AIChatProvider is missing');
  return value;
}
const storageKey = 'ai_chat_sessions_v1';
const LauncherContainer = Platform.OS === 'ios' ? SafeAreaView : View;

// The single global button uses the currently mounted learning page's material.
export function useAIChatEntry(text: string, source: string, onOpen?: () => void) {
  const { open, registerEntry } = useAIChat();
  useEffect(() => registerEntry(() => {
    onOpen?.();
    open(text.trim() ? { text, source } : undefined);
  }), [open, registerEntry, text, source, onOpen]);
}

export function AIChatProvider({ children }: { children: ReactNode }) {
  const { theme } = useAppTheme();
  const [visible, setVisible] = useState(false);
  const [collecting, setCollecting] = useState(false);
  const [sessions, setSessions] = useState<Session[]>([newChat()]);
  const [ready, setReady] = useState(false);
  const [busy, setBusy] = useState(false);
  const [pending, setPending] = useState('');
  const [stream, setStream] = useState<{ reasoning: string; answer: string } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState('');
  const active = sessions[0];
  const request = useRef<AbortController | null>(null);
  const changed = useRef(false);
  const writes = useRef(Promise.resolve());
  const entryRef = useRef<(() => void) | null>(null);
  const registerEntry = useCallback((entry: () => void) => {
    entryRef.current = entry;
    return () => { if (entryRef.current === entry) entryRef.current = null; };
  }, []);
  const close = useCallback(() => setVisible(false), []);
  const cancel = useCallback(() => {
    request.current?.abort();
    request.current = null;
    setBusy(false);
    setPending('');
    setStream(null);
  }, []);
  useEffect(() => {
    let mounted = true;
    void getSetting(storageKey).then(async raw => {
      if (!raw) {
        const legacy = await getSetting('ai_chat_history').catch(() => null);
        try {
          const messages = legacy ? JSON.parse(legacy) : [];
          if (Array.isArray(messages) && messages.length) {
            raw = JSON.stringify([{ ...newChat(), source: '听力历史（原引用未保存）', messages }]);
          }
        } catch {}
      }
      if (!mounted || !raw) return;
      const valid = restoreChats(raw);
      if (valid.length) setSessions(previous => changed.current
        ? [...previous, ...valid.filter(saved => !previous.some(s => s.id === saved.id))].slice(0, 12)
        : valid);
    }).catch(() => {}).finally(() => { if (mounted) setReady(true); });
    return () => { mounted = false; request.current?.abort(); };
  }, []);
  useEffect(() => {
    if (!ready) return;
    const value = JSON.stringify(sessions);
    const timer = setTimeout(() => { writes.current = writes.current.then(() => setSetting(storageKey, value)).catch(() => {}); }, 250);
    return () => clearTimeout(timer);
  }, [ready, sessions]);
  useEffect(() => {
    if (!visible) return;
    const listener = BackHandler.addEventListener('hardwareBackPress', () => { close(); return true; });
    return () => listener.remove();
  }, [visible, close]);
  const open = useCallback((reference?: Reference) => {
    changed.current = true;
    setVisible(true);
    if (collecting) {
      setCollecting(false);
      if (reference) {
        try {
          const merged = appendReference(active, reference);
          setSessions(previous => [merged, ...previous.slice(1)]);
          setError(null); setNotice('已合并引用，可以继续选文或直接提问。');
        } catch (e: any) { setError(e.message); }
      }
      return;
    }
    if (!reference) return;
    if (active.text === reference.text.trim() && active.source === reference.source) return;
    cancel(); setError(null); setNotice('');
    setSessions(previous => openReference(previous, reference));
  }, [cancel, active, collecting]);
  const updateDraft = (draft: string) => {
    changed.current = true;
    setSessions(previous => previous.map((s, i) => i === 0 ? { ...s, draft } : s));
  };
  const send = async (question: string) => {
    if (request.current || !question.trim()) return;
    if (question.length > 2000) { setError('问题请控制在 2000 字以内。'); return; }
    if (active.text.length > 12000) { setError('材料较长，请展开引用，选取 12000 字以内的片段。'); return; }
    changed.current = true;
    const snapshot = active;
    const passage = snapshot.text || '用户正在学习英语，请根据用户的问题提供帮助。';
    const history = chatHistory(snapshot.messages);
    const controller = new AbortController();
    request.current = controller;
    setBusy(true); setPending(question); setError(null); setNotice('');
    setStream({ reasoning: '', answer: '' });
    // Keep the draft until success: cancellation and failure never lose the question.
    updateDraft(question);
    let timedOut = false;
    let streamedAny = false;
    const timer = setTimeout(() => { timedOut = true; controller.abort(); }, 90000);
    const appendAnswer = (answer: string) => setSessions(previous => previous.map(s => s.id === snapshot.id ? { ...s, draft: '', messages: [...s.messages, { role: 'user' as const, content: question }, { role: 'assistant' as const, content: answer }].slice(-60) } : s));
    try {
      const result = await streamChatAnswer({
        passage, question, signal: controller.signal, history,
        onEvent: event => {
          if (request.current !== controller || controller.signal.aborted) return;
          if (event.type === 'reasoning' || event.type === 'answer') streamedAny = true;
          if (event.type === 'reasoning') setStream(prev => ({ reasoning: (prev?.reasoning ?? '') + event.text, answer: prev?.answer ?? '' }));
          else if (event.type === 'answer') setStream(prev => ({ reasoning: prev?.reasoning ?? '', answer: (prev?.answer ?? '') + event.text }));
        },
      });
      if (request.current !== controller || controller.signal.aborted) return;
      appendAnswer(result.answer);
    } catch (streamError: any) {
      if (request.current !== controller) return;
      // Only an unsupported endpoint falls back; never resend failed or partial streams.
      if (streamError?.code === 'no-stream' && !streamedAny && !controller.signal.aborted) {
        try {
          const answer = await askAboutPassage(passage, question, undefined, controller.signal, history);
          if (request.current !== controller || controller.signal.aborted) return;
          appendAnswer(answer);
        } catch (fallbackError: any) {
          if (request.current !== controller) return;
          setError(fallbackError?.name === 'AbortError' ? '已取消。' : timedOut ? '回答超时，问题已保留，可重新发送。' : `提问失败：${fallbackError?.message ?? fallbackError}`);
        }
        return;
      }
      if (controller.signal.aborted && !timedOut) return;
      setError(timedOut ? '回答超时，问题已保留，可重新发送。' : `提问失败：${streamError?.message ?? streamError}`);
    } finally {
      clearTimeout(timer);
      if (request.current === controller) { request.current = null; setBusy(false); setPending(''); setStream(null); }
    }
  };
  return <ChatContext.Provider value={{ visible, open, registerEntry }}>
    <View style={{ flex: 1, backgroundColor: theme.bg }}>
      <View style={{ flex: 1, minHeight: 0 }} pointerEvents={visible ? 'none' : 'auto'} accessibilityElementsHidden={visible} importantForAccessibility={visible ? 'no-hide-descendants' : 'auto'}>{children}</View>
      {/* Reserve real layout space so the launcher cannot cover page actions or navigation. */}
      {!visible && <LauncherContainer style={{ flexShrink: 0 }}>
        <View style={{ alignItems: 'flex-end', paddingHorizontal: 18, paddingVertical: 10 }}>
          <Pressable accessibilityRole="button" accessibilityLabel={collecting ? '返回 AI 学习助手' : '打开 AI 学习助手'} onPress={() => collecting ? open() : entryRef.current ? entryRef.current() : open()}
            style={{ maxWidth: '100%', minHeight: 44, justifyContent: 'center', borderRadius: 22, backgroundColor: theme.accent, paddingHorizontal: 18, paddingVertical: 12 }}>
            <Text style={{ color: '#fff', fontWeight: '700', textAlign: 'center' }}>{collecting ? '选文后右键提问 · 返回 AI' : '✦ AI 助手'}</Text>
          </Pressable>
        </View>
      </LauncherContainer>}
      <ChatPanel visible={visible} messages={active.messages} busy={busy} pending={pending}
        stream={stream}
        contextText={active.text} source={active.source} error={error} notice={notice} draft={active.draft} onDraftChange={updateDraft}
        sessions={sessions.map(s => ({ id: s.id, title: s.messages[0]?.content || s.source }))} sessionId={active.id}
        onSelectSession={id => { cancel(); setError(null); setNotice(''); changed.current = true; setSessions(prev => [...prev.filter(s => s.id === id), ...prev.filter(s => s.id !== id)]); }}
        onReference={text => open({ text, source: active.source })}
        onCollectReference={() => { setCollecting(true); setVisible(false); }}
        onClose={close} onSend={text => void send(text)} onCancel={cancel}
        onNewConversation={() => { cancel(); setError(null); setNotice(''); changed.current = true; setSessions(prev => [newChat(), ...prev].slice(0, 12)); }}
        onCopy={text => setNotice(copyToClipboard(text) ? '已复制回答' : '当前设备暂不支持复制')}
        onSaveCard={(question, answer) => { void saveAiCard({ question, answer, contextText: active.text }).then(() => setNotice('已存为 AI 问答闪卡')).catch(e => setNotice(`保存失败：${e.message}`)); }} />
    </View>
  </ChatContext.Provider>;
}
