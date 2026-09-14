import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ActivityIndicator, AppState, NativeModules, PanResponder, Platform, Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';
import { getReviewCards, getDueReviewCards, listListeningPractices, getDiscoverFavorites, setDiscoverFavorite, getSetting, setSetting } from '../data/database';
import { audioSegments, discoverSubtitles, fromCard, pickNext, type DiscoverItem } from '../data/discover';
import { parseSubtitleCues, type SubtitleCue } from '../data/subtitles';
import { FileSystem, nativePath } from '../services/platform';
import { DiscoverAudio } from '../services/discoverAudio';
import { useAppTheme } from '../ui/ThemeContext';
import { MarkdownView } from '../ui/markdown';
import { DiscoverSubtitles } from '../ui/DiscoverSubtitles';
import { useAIChat, useAIChatEntry } from '../ui/AIChatProvider';

export default function DiscoverScreen({ onManageAudio }: {
  onManageAudio?: (practiceId: string, audioId: string) => void;
} = {}) {
  const { theme } = useAppTheme();
  const [pool, setPool] = useState<DiscoverItem[]>([]);
  const [favorites, setFavorites] = useState<DiscoverItem[]>([]);
  const [savedOnly, setSavedOnly] = useState(false);
  const [listeningOnly, setListeningOnly] = useState(false);
  const [filterReady, setFilterReady] = useState(false);
  const [session, setSession] = useState<{ items: DiscoverItem[]; index: number }>({ items: [], index: 0 });
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [revealed, setRevealed] = useState(false);
  const [playing, setPlaying] = useState(false);
  const [position, setPosition] = useState(0);
  const [saving, setSaving] = useState(false);
  const [reload, setReload] = useState(0);
  const [more, setMore] = useState(false);
  const { visible: asking } = useAIChat();
  const [openingAudio, setOpeningAudio] = useState(false);
  const [audioTrackWidth, setAudioTrackWidth] = useState(0);
  const [subtitleLibrary, setSubtitleLibrary] = useState<Record<string, SubtitleCue[]>>({});
  const due = useRef(new Set<string>());
  const player = useRef<DiscoverAudio | null>(null);
  const initialized = useRef(false);
  const savedKeys = useMemo(() => new Set(favorites.map(i => i.key)), [favorites]);
  const available = useMemo(() => (savedOnly ? favorites : pool)
    .filter(item => !listeningOnly || (item.kind === 'clip' && !!item.audio)), [savedOnly, favorites, pool, listeningOnly]);
  const current = session.items[session.index];
  const next = session.items[session.index + 1];
  const subtitles = useMemo(() => current?.audio
    ? subtitleLibrary[current.audio.uri]
      ? discoverSubtitles(subtitleLibrary[current.audio.uri], current.audio.startMs, current.audio.endMs)
      : current.subtitles ?? []
    : [], [current, subtitleLibrary]);
  const playback = useRef({ current, next, playing });
  playback.current = { current, next, playing };
  const togglePlayback = useCallback(() => {
    const { current: item, next: upcoming, playing: active } = playback.current;
    if (!item?.audio) return;
    if (active) player.current?.pause();
    else { setError(''); player.current?.resume(item, upcoming); }
  }, []);
  const playFrom = (ms: number) => {
    if (!current?.audio) return;
    setError('');
    player.current?.show(current, next, ms);
  };
  const seekFromTrack = (locationX: number) => {
    if (!current?.audio || audioTrackWidth <= 0) return;
    const ratio = Math.max(0, Math.min(1, locationX / audioTrackWidth));
    playFrom(current.audio.startMs + (current.audio.endMs - current.audio.startMs) * ratio);
  };
  const manageCurrentAudio = async () => {
    if (!current?.audio || !onManageAudio || openingAudio) return;
    const item = current;
    setOpeningAudio(true);
    setError('');
    try {
      const practices = await listListeningPractices();
      // Resolve against the current library, including old favorite cards
      // that do not carry practice/audio IDs.
      for (const practice of practices) {
        const audio = practice.audios.find(candidate => nativePath(candidate.audio_uri) === nativePath(item.audio!.uri));
        if (!audio) continue;
        if (playback.current.current !== item) return;
        player.current?.pause();
        onManageAudio(practice.id, audio.id);
        return;
      }
      setError('找不到这张卡片的原音频，可能已从音频管理中删除。');
    } catch (e: any) { setError(e?.message ?? String(e)); }
    finally { setOpeningAudio(false); }
  };

  useEffect(() => {
    let cancelled = false;
    void getSetting('discover_listening_only').then(value => {
      if (!cancelled) setListeningOnly(value === 'true');
    }).catch(() => {}).finally(() => { if (!cancelled) setFilterReady(true); });
    return () => { cancelled = true; };
  }, []);

  useEffect(() => {
    const audio = new DiscoverAudio((active, ms) => { setPlaying(active); setPosition(ms); }, setError);
    player.current = audio;
    const sub = AppState.addEventListener('change', state => {
      if (state !== 'active') { audio.pause(); setPlaying(false); }
    });
    return () => { sub.remove(); audio.dispose(); player.current = null; };
  }, []);

  useEffect(() => {
    let cancelled = false;
    initialized.current = false;
    setLoading(true);
    setError('');
    void (async () => {
      const [cards, practices, saved, review] = await Promise.all([
        getReviewCards(), listListeningPractices(), getDiscoverFavorites(), getDueReviewCards('all', 0),
      ]);
      if (cancelled) return;
      due.current = new Set(review.cards.map(c => `${c.kind}:${c.id}`));
      setFavorites(saved);
      const items = cards.map(fromCard).filter((i): i is DiscoverItem => !!i);
      setPool([...items]);
      // Publish cards immediately; parse large libraries incrementally in the
      // background, yielding between files instead of blocking first entry.
      const existingAudio = new Set(items.filter(i => i.audio).map(i => `${i.audio!.uri}:${i.audio!.startMs}:${i.audio!.endMs}`));
      for (const practice of practices) {
        for (const audio of practice.audios) {
          if (cancelled) return;
          if (!audio.subtitle_uri) continue;
          try {
            const contents = await FileSystem.readAsStringAsync(audio.subtitle_uri);
            if (cancelled) return;
            const cues = parseSubtitleCues(contents);
            setSubtitleLibrary(previous => ({ ...previous, [audio.audio_uri]: cues }));
            const segments = audioSegments(audio, cues).filter(i => !existingAudio.has(`${i.audio!.uri}:${i.audio!.startMs}:${i.audio!.endMs}`));
            items.push(...segments);
            setPool([...items]);
            await new Promise(resolve => setTimeout(resolve, 0));
          } catch { /* Missing local subtitles do not block the rest of the library. */ }
        }
      }
    })().catch(e => { if (!cancelled) setError(String(e?.message ?? e)); })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [reload]);

  useEffect(() => {
    if (!filterReady || initialized.current || !available.length) return;
    initialized.current = true;
    const first = pickNext(available, [], due.current, savedKeys);
    const second = first && pickNext(available, [first], due.current, savedKeys);
    setSession({ items: first ? [first, ...(second ? [second] : [])] : [], index: 0 });
  }, [available, savedKeys, filterReady]);

  useEffect(() => {
    setRevealed(false);
    setMore(false);
    setError('');
    if (current) player.current?.show(current, next);
    else player.current?.pause();
  }, [current, next, session.index]);

  const move = useCallback((direction: number) => {
    setSession(previous => {
      if (direction < 0) return previous.index > 0 ? { ...previous, index: previous.index - 1 } : previous;
      if (!available.length) return previous;
      const items = [...previous.items];
      const index = previous.index + 1;
      while (items.length <= index + 1) {
        const item = pickNext(available, items, due.current, savedKeys);
        if (!item) break;
        items.push(item);
      }
      return index < items.length ? { items, index } : previous;
    });
  }, [available, savedKeys]);

  // The native key monitor is also used by flashcards. It returns keys through
  // a promise, which avoids AppKit-to-JS thread callbacks on macOS.
  useEffect(() => {
    if (asking) return;
    if (Platform.OS !== 'macos' && Platform.OS !== 'windows') return;
    const keyboard = NativeModules.RNKeyboard as {
      startListening?: () => void;
      stopListening?: () => void;
      getNextKey?: () => Promise<string | null>;
    } | undefined;
    if (!keyboard?.getNextKey) return;
    let cancelled = false;
    keyboard.startListening?.();
    void (async () => {
      while (!cancelled) {
        const key = await keyboard.getNextKey!();
        if (cancelled || key == null) return;
        if (key === 'ArrowRight' || key === 'Right' || key.toLowerCase() === 'd') move(1);
        if (key === 'ArrowLeft' || key === 'Left' || key.toLowerCase() === 'a') move(-1);
        if (key === ' ' || key === 'Space' || key === 'Spacebar') {
          if (playback.current.current?.audio) togglePlayback();
          else setRevealed(value => !value);
        }
        if (key === 'Enter' || key === 'Return') setRevealed(value => !value);
        if (key.toLowerCase() === 'p') togglePlayback();
      }
    })();
    return () => { cancelled = true; keyboard.stopListening?.(); };
  }, [move, togglePlayback, asking]);

  const horizontal = Platform.OS === 'macos' || Platform.OS === 'windows';
  const gesture = useMemo(() => PanResponder.create({
    onMoveShouldSetPanResponder: (_, g) => {
      const primary = horizontal ? g.dx : g.dy;
      const cross = horizontal ? g.dy : g.dx;
      // Expanded answers and timed subtitles own vertical gestures so long
      // transcripts can be read without accidentally changing the card.
      return !(revealed || subtitles.length) && Math.abs(primary) > 24 && Math.abs(primary) > Math.abs(cross) * 1.5;
    },
    onPanResponderRelease: (_, g) => {
      const delta = horizontal ? g.dx : g.dy;
      if (Math.abs(delta) > 60) move(delta < 0 ? 1 : -1);
    },
  }), [horizontal, move, revealed, subtitles.length]);

  const switchCollection = (saved: boolean, onlyAudio = listeningOnly) => {
    setSavedOnly(saved);
    setListeningOnly(onlyAudio);
    const candidates = (saved ? favorites : pool).filter(item => !onlyAudio || (item.kind === 'clip' && !!item.audio));
    const first = pickNext(candidates, [], due.current, savedKeys);
    const second = first && pickNext(candidates, [first], due.current, savedKeys);
    initialized.current = !!first;
    setSession({ items: first ? [first, ...(second ? [second] : [])] : [], index: 0 });
  };

  const save = async () => {
    if (!current || saving) return;
    const item = current;
    const saved = !savedKeys.has(item.key);
    setSaving(true);
    try {
      await setDiscoverFavorite(item, saved);
      setFavorites(list => saved ? [...list.filter(i => i.key !== item.key), item] : list.filter(i => i.key !== item.key));
    } catch (e: any) { setError(e?.message ?? String(e)); }
    finally { setSaving(false); }
  };

  useAIChatEntry(current ? [subtitles.length ? subtitles.map(cue => [cue.text, revealed ? cue.translation : ''].filter(Boolean).join('\n')).join('\n\n') : current.front,
      revealed ? current.context : '', revealed ? current.back : ''].filter(Boolean).join('\n\n') : '', '随便学学', () => { player.current?.pause(); });

  const button = (label: string, action: () => void, disabled = false) => (
    <Pressable accessibilityRole="button" accessibilityLabel={label} disabled={disabled} onPress={action}
      style={[styles.button, { backgroundColor: theme.surfaceHover, opacity: disabled ? .4 : 1 }]}>
      <Text style={{ color: theme.accent, fontSize: 15 }}>{label}</Text>
    </Pressable>
  );



  return <View style={[styles.root, { backgroundColor: theme.bg }]}>
    <View style={styles.header}>
      <View><Text style={[styles.title, { color: theme.text }]}>随便学学</Text>
        <Text style={{ color: theme.textMuted, marginTop: 6 }}>这一条，就从这里开始。</Text></View>
      {button(savedOnly ? '返回随便学学' : '我的收藏', () => switchCollection(!savedOnly))}
    </View>
    <View style={{ flexDirection: 'row', gap: 8, marginBottom: 12 }}>
      {[{ label: '混合学习', value: false }, { label: '只听听力', value: true }].map(option => <Pressable key={option.label}
        accessibilityRole="button" accessibilityState={{ selected: listeningOnly === option.value, disabled: !filterReady }} disabled={!filterReady}
        onPress={() => {
          if (listeningOnly === option.value) return;
          switchCollection(savedOnly, option.value);
          void setSetting('discover_listening_only', String(option.value)).catch(() => {});
        }}
        style={[styles.button, { backgroundColor: listeningOnly === option.value ? theme.sidebarAccent : theme.surfaceHover }]}>
        <Text style={{ color: listeningOnly === option.value ? theme.onSidebarAccent : theme.textSecondary }}>{option.label}</Text>
      </Pressable>)}
    </View>
    {!!error && <Text accessibilityRole="alert" style={{ color: theme.danger, padding: 10 }}>{error}</Text>}
    {current ? <View style={[styles.card, { backgroundColor: theme.surface, borderColor: theme.border }]} {...gesture.panHandlers}>
      <Text numberOfLines={1} style={{ color: theme.accent, marginBottom: 20 }}>{current.source}{current.generated ? ' · 随机片段' : ''}</Text>
      {subtitles.length ? <DiscoverSubtitles key={`${current.key}:${session.index}`} cues={subtitles}
        position={position} revealed={revealed} theme={theme} onSeek={playFrom}>
        {revealed && !subtitles.some(cue => cue.translation) && <MarkdownView text={current.back || '这段音频暂时没有中文翻译。'} theme={theme} baseFontSize={18} />}
      </DiscoverSubtitles> : <ScrollView scrollEnabled={revealed || horizontal} style={{ flex: 1 }} contentContainerStyle={{ flexGrow: 1, justifyContent: 'center', paddingVertical: 16 }}>
        <Text accessibilityRole={current.audio ? 'button' : undefined}
          accessibilityHint={current.audio ? '从音频片段开头播放' : undefined}
          onPress={current.audio ? () => playFrom(current.audio!.startMs) : undefined}
          style={{ color: theme.text, fontSize: current.kind === 'word' ? 38 : 23, lineHeight: current.kind === 'word' ? 48 : 35 }} numberOfLines={!revealed && !horizontal ? 8 : undefined}>{current.front}</Text>
        {revealed && <View style={{ marginTop: 24 }}>
          {!!current.context && <Text style={{ color: theme.textSecondary, marginBottom: 16, fontSize: 16 }}>{current.context}</Text>}
          {!subtitles.some(cue => cue.translation) && <MarkdownView text={current.back || '这段音频暂时没有中文翻译。'} theme={theme} baseFontSize={18} />}
        </View>}
      </ScrollView>}
      {revealed && current.audio && !current.back.trim() && !subtitles.some(cue => cue.translation) && onManageAudio &&
        <View style={{ marginTop: 12, alignItems: 'flex-start' }}>
          {button(openingAudio ? '正在定位音频…' : '管理此音频 ↗', () => { void manageCurrentAudio(); }, openingAudio)}
        </View>}
      <View style={styles.answerRow}>
        {button(revealed ? '收起答案' : current.kind === 'clip' ? '显示翻译' : '显示答案', () => setRevealed(v => !v))}

        {!!current.audio && button('⋯', () => setMore(value => !value))}
      </View>
      {more && !!current.audio && <View style={styles.actions}>
        {button('从头播放', () => { playFrom(current.audio!.startMs); setMore(false); })}
      </View>}
      <View style={[styles.transport, { borderTopColor: theme.border }]}>
        <View style={styles.transportSide}>
          {!!current.audio && <>
            <Text style={{ color: theme.textSecondary, fontSize: 12, fontVariant: ['tabular-nums'] }}>{formatTime(Math.max(0, Math.min(position, current.audio.endMs) - current.audio.startMs))} / {formatTime(current.audio.endMs - current.audio.startMs)}</Text>
            <Pressable accessibilityRole="adjustable" accessibilityLabel="音轨"
              accessibilityHint="点击可跳转到对应播放位置"
              onLayout={event => setAudioTrackWidth(event.nativeEvent.layout.width)}
              onPress={event => seekFromTrack(event.nativeEvent.locationX)}
              style={{ height: 28, justifyContent: 'center', marginTop: 3 }}>
              <View style={{ height: 5, backgroundColor: theme.border, borderRadius: 3, overflow: 'hidden' }}>
                <View style={{ height: 5, backgroundColor: theme.accent, width: `${Math.min(100, Math.max(0, (position - current.audio.startMs) / Math.max(1, current.audio.endMs - current.audio.startMs) * 100))}%` }} />
              </View>
            </Pressable>
          </>}
        </View>
        {!!current.audio && <Pressable accessibilityRole="button" accessibilityLabel={playing ? '暂停音频' : '播放音频'}
          accessibilityHint={horizontal ? '快捷键：空格' : undefined} onPress={togglePlayback}
          style={[styles.playButton, { backgroundColor: theme.sidebarAccent }]}>
          <Text style={{ color: theme.onSidebarAccent, fontSize: 25 }}>{playing ? 'Ⅱ' : '▶'}</Text>
        </Pressable>}
        <View style={[styles.transportSide, { alignItems: 'flex-end' }]}>
          <Pressable accessibilityRole="button" accessibilityLabel={savedKeys.has(current.key) ? '取消收藏' : '收藏'}
            accessibilityState={{ selected: savedKeys.has(current.key), disabled: saving }} disabled={saving}
            onPress={() => { void save(); }} style={{ padding: 10, opacity: saving ? .4 : 1 }}>
            <Text style={{ color: theme.accent, fontSize: 26 }}>{savedKeys.has(current.key) ? '★' : '☆'}</Text>
          </Pressable>
        </View>
      </View>
    </View> : <View style={styles.empty}>
      {loading || !filterReady ? <ActivityIndicator color={theme.accent} /> : <>
        <Text style={{ color: theme.textSecondary, textAlign: 'center', marginBottom: 20 }}>{listeningOnly ? savedOnly ? '还没有收藏听力内容。' : '还没有可学习的听力内容。\n添加音频片段卡，或导入带时间轴字幕的音频后再来。' : savedOnly ? '收藏喜欢的内容，稍后在这里再看。' : '还没有可学习的内容。\n导入单词卡，或添加带时间轴字幕的音频后再来。'}</Text>
        {!savedOnly && button('重新加载', () => setReload(v => v + 1))}
      </>}
    </View>}
    <View style={styles.footer}>
      {button('上一条', () => move(-1), session.index === 0)}
      <Text style={{ color: theme.textMuted, flex: 1, textAlign: 'center', fontSize: 12 }}>{horizontal ? current?.audio ? 'A / D 切换 · 空格播放/暂停 · Enter 答案' : 'A / D 切换 · 空格 / Enter 答案' : subtitles.length ? '字幕可上下滚动 · 点下一条继续' : revealed ? '收起后上下滑动，也可点下一条' : '上滑下一条 · 下滑返回'}</Text>
      {button('下一条', () => move(1), !current)}
    </View>
  </View>;
}

function formatTime(ms: number) {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`;
}

const styles = StyleSheet.create({
  root: { flex: 1, padding: 18 },
  header: { flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between', gap: 10, marginBottom: 18 },
  title: { fontSize: 26, fontWeight: '700' },
  card: { flex: 1, width: '100%', maxWidth: 840, alignSelf: 'center', padding: 24, borderRadius: 24, borderWidth: 1 },
  button: { paddingHorizontal: 15, paddingVertical: 12, borderRadius: 16 },
  actions: { flexDirection: 'row', flexWrap: 'wrap', gap: 8, marginTop: 16 },
  answerRow: { flexDirection: 'row', justifyContent: 'space-between', alignItems: 'center', marginTop: 12 },
  transport: { flexDirection: 'row', alignItems: 'center', gap: 14, paddingTop: 16, marginTop: 16, borderTopWidth: 1 },
  transportSide: { flex: 1 },
  playButton: { width: 56, height: 56, borderRadius: 28, alignItems: 'center', justifyContent: 'center' },
  footer: { flexDirection: 'row', alignItems: 'center', gap: 8, marginTop: 16 },
  empty: { flex: 1, alignItems: 'center', justifyContent: 'center' },
});
