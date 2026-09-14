import { useState, type ReactNode } from 'react';
import { Pressable, ScrollView, StyleSheet, Text, TextInput, View } from 'react-native';
import type { ListeningAudio, ListeningPractice } from '../data/database';
import type { SubtitleLanguage } from '../data/subtitles';
import { useAppTheme } from './ThemeContext';
import { Details } from './Details';

type Props = {
  practices: ListeningPractice[];
  practice: ListeningPractice | null | undefined;
  audio: ListeningAudio | null | undefined;
  /** audioId → 字幕语言形态（missing = 尚未解析或读取失败）。 */
  subtitleKinds: Record<string, SubtitleLanguage>;
  busy: boolean;
  status: string | null;
  onSelectPractice: (practice: ListeningPractice) => void;
  onSelectAudio: (audio: ListeningAudio) => void;
  onStart?: () => void;
  onCreate: () => void;
  onAdd: () => void;
  onPracticeMenu: () => void;
  onAudioMenu: () => void;
  onBatch: () => void;
  onGenerate: () => void;
  onView: () => void;
  onTranslate: () => void;
  onUploadSubtitle: () => void;
  advanced: ReactNode;
};

export function AudioLibraryManager(props: Props) {
  const { theme } = useAppTheme();
  const [query, setQuery] = useState('');
  const [groupQuery, setGroupQuery] = useState('');
  const [filter, setFilter] = useState<'all' | 'bilingual' | 'missing'>('all');
  const [viewport, setViewport] = useState({ width: 0, height: 0 });
  // Only split when both panes have enough room. Short/narrow windows use
  // one page scrollbar, avoiding nested scroll areas below the window edge.
  const wide = viewport.width >= 820 && viewport.height >= 560;
  const Layout = wide ? View : ScrollView;
  const AudioList = wide ? ScrollView : View;
  const DetailBody = wide ? ScrollView : View;
  const s = StyleSheet.create({
    content: { gap: 18, paddingBottom: 24 },
    row: { flexDirection: 'row', alignItems: 'center', gap: 10, flexWrap: 'wrap' },
    between: { flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between', gap: 10, flexWrap: 'wrap' },
    label: { fontSize: 13, fontWeight: '600', color: theme.textSecondary },
    muted: { fontSize: 12, color: theme.textMuted, lineHeight: 19 },
    button: { minHeight: 40, paddingHorizontal: 14, paddingVertical: 10, borderRadius: 12, backgroundColor: theme.surfaceHover, justifyContent: 'center', alignItems: 'center' },
    buttonText: { color: theme.accent, fontSize: 13, fontWeight: '600' },
    primary: { backgroundColor: theme.accent },
    primaryText: { color: '#fff', fontSize: 14, fontWeight: '700' },
    disabled: { opacity: 0.4 },
    chip: { minHeight: 42, paddingHorizontal: 14, paddingVertical: 10, borderRadius: 12, borderWidth: 1, borderColor: theme.border, backgroundColor: theme.surface, flexDirection: 'row', alignItems: 'center', gap: 10, maxWidth: 230 },
    active: { borderColor: theme.accent, backgroundColor: `${theme.accent}12` },
    chipText: { fontSize: 13, color: theme.text, flexShrink: 1 },
    panel: { backgroundColor: theme.surface, borderWidth: 1, borderColor: theme.border, borderRadius: 20, overflow: 'hidden', minWidth: 0 },
    panelHead: { padding: 18, gap: 12, borderBottomWidth: 1, borderBottomColor: theme.border },
    title: { color: theme.text, fontSize: 17, fontWeight: '700' },
    input: { borderRadius: 10, backgroundColor: theme.bg, color: theme.text, paddingHorizontal: 12, paddingVertical: 10, minHeight: 42, fontSize: 13 },
    filter: { paddingVertical: 7, paddingHorizontal: 11, borderRadius: 9 },
    item: { minHeight: 76, paddingHorizontal: 16, paddingVertical: 14, flexDirection: 'row', alignItems: 'center', gap: 12, borderBottomWidth: 1, borderBottomColor: theme.border },
    itemActive: { backgroundColor: `${theme.accent}10`, borderLeftWidth: 3, borderLeftColor: theme.accent, paddingLeft: 13 },
    number: { color: theme.textMuted, fontSize: 12, width: 22, fontVariant: ['tabular-nums'] },
    itemName: { color: theme.text, fontSize: 14, fontWeight: '600' },
    badge: { color: theme.accent, fontSize: 11, marginTop: 5 },
    detail: { padding: 22, gap: 20 },
    detailName: { fontSize: 24, fontWeight: '700', color: theme.text, lineHeight: 32 },
    divider: { height: 1, backgroundColor: theme.border },
    empty: { padding: 28, alignItems: 'center', gap: 12 },
    path: { color: theme.textMuted, fontSize: 12, lineHeight: 20 },
    status: { padding: 12, backgroundColor: `${theme.accent}10`, borderRadius: 12, color: theme.accent, fontSize: 13 },
  });
  const { practice, audio } = props;
  const all = practice?.audios ?? [];
  const missing = all.filter(item => !item.subtitle_uri).length;
  const bilingual = all.filter(item => props.subtitleKinds[item.id] === 'bilingual').length;
  const badgeFor = (item: ListeningAudio): { text: string; known: boolean } => {
    if (!item.subtitle_uri) return { text: '待配字幕', known: false };
    const kind = props.subtitleKinds[item.id];
    if (kind === 'bilingual') return { text: '双语字幕', known: true };
    if (kind === 'chinese') return { text: '中文字幕', known: true };
    if (kind === 'original') return { text: '仅原文', known: true };
    return { text: '字幕就绪', known: false };
  };
  const rows = all.filter(item =>
    (filter === 'all'
      || (filter === 'missing' && !item.subtitle_uri)
      || (filter === 'bilingual' && props.subtitleKinds[item.id] === 'bilingual'))
    && item.name.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase()));
  const action = (label: string, onPress: () => void, disabled = false, primary = false) => (
    <Pressable accessibilityRole="button" accessibilityState={{ disabled }} disabled={disabled} onPress={onPress}
      style={({ pressed }) => [s.button, primary && s.primary, disabled && s.disabled, pressed && { opacity: 0.7 }]}>
      <Text style={primary ? s.primaryText : s.buttonText}>{label}</Text>
    </Pressable>
  );
  return <View style={{ flex: 1, minHeight: 0 }} onLayout={event => {
    const { width, height } = event.nativeEvent.layout;
    setViewport(previous => previous.width === width && previous.height === height ? previous : { width, height });
  }}>
    <Layout style={wide ? [s.content, { flex: 1, minHeight: 0 }] : { flex: 1, minHeight: 0 }}
      {...(!wide ? { contentContainerStyle: s.content, keyboardShouldPersistTaps: 'handled' as const } : {})}>

    <View style={s.between}>
      <Text style={s.label}>练习组 · {props.practices.length}</Text>
      <TextInput accessibilityLabel="查找练习组" placeholder="查找练习组" placeholderTextColor={theme.textMuted}
        style={[s.input, { flexGrow: 1, flexBasis: 180, maxWidth: 280, backgroundColor: theme.surface, borderWidth: 1, borderColor: theme.border }]} value={groupQuery} onChangeText={setGroupQuery} clearButtonMode="while-editing" />
      {action('＋ 新建练习组', props.onCreate, props.busy)}
    </View>
    <ScrollView horizontal style={{ flexGrow: 0, flexShrink: 0 }} showsHorizontalScrollIndicator contentContainerStyle={{ gap: 8, paddingBottom: 6 }}>
      {props.practices.filter(item => item.name.toLocaleLowerCase().includes(groupQuery.trim().toLocaleLowerCase())).map(item => <Pressable key={item.id} accessibilityRole="button" accessibilityState={{ selected: item.id === practice?.id, disabled: props.busy }}
        disabled={props.busy} onPress={() => { setQuery(''); setFilter('all'); props.onSelectPractice(item); }}
        style={[s.chip, item.id === practice?.id && s.active, props.busy && s.disabled]}>
        <Text numberOfLines={1} style={s.chipText}>{item.name}</Text><Text style={s.muted}>{item.audios.length}</Text>
      </Pressable>)}
      {!!groupQuery && !props.practices.some(item => item.name.toLocaleLowerCase().includes(groupQuery.trim().toLocaleLowerCase())) && <Text style={s.muted}>没有匹配的练习组</Text>}
    </ScrollView>
    {!practice ? <View style={[s.panel, s.empty]}>
      <Text style={s.title}>把想学的音频放在一起</Text>
      <Text style={s.muted}>新建一个练习组，导入音频后就可以开始学习。</Text>
      {action('新建第一个练习组', props.onCreate, props.busy, true)}
    </View> : <View style={[{ flexDirection: wide ? 'row' : 'column', gap: 16, alignItems: 'stretch' }, wide && { flex: 1, minHeight: 0 }]}>
      <View style={[s.panel, wide && { flex: 1.15, minHeight: 0 }]}>
        <View style={s.panelHead}>
          <View style={s.between}>
            <View style={{ flex: 1, minWidth: 100 }}><Text style={s.title} numberOfLines={1}>{practice.name}</Text><Text style={s.muted}>{all.length} 条音频 · {bilingual} 条双语 · {missing} 条待配字幕</Text></View>
            {action('管理组', props.onPracticeMenu, props.busy)}
            {action('＋ 导入音频', props.onAdd, props.busy, true)}
          </View>
          <TextInput accessibilityLabel="搜索当前练习组的音频" placeholder="搜索音频名称" placeholderTextColor={theme.textMuted} style={s.input} value={query} onChangeText={setQuery} clearButtonMode="while-editing" />
          <View style={s.row}>
            {([['all', `全部 ${all.length}`], ['bilingual', `双语 ${bilingual}`], ['missing', `待配字幕 ${missing}`]] as const).map(([value, label]) => <Pressable key={value} accessibilityRole="button" accessibilityState={{ selected: filter === value }} onPress={() => setFilter(value)} style={[s.filter, filter === value && { backgroundColor: theme.surfaceHover }]}>
              <Text style={filter === value ? s.buttonText : s.muted}>{label}</Text>
            </Pressable>)}
            {action('批量管理', props.onBatch, !all.length || props.busy)}
          </View>
        </View>
        <AudioList testID="audio-library-list" style={wide ? { flex: 1, minHeight: 0 } : undefined} {...(wide ? { keyboardShouldPersistTaps: "handled" as const } : {})}>
          {rows.map(item => {
            const badge = badgeFor(item);
            return <Pressable key={item.id} accessibilityRole="button" accessibilityLabel={`${item.name}，${badge.text}`}
            accessibilityState={{ selected: item.id === audio?.id, disabled: props.busy }} disabled={props.busy} onPress={() => props.onSelectAudio(item)}
            style={({ pressed }) => [s.item, item.id === audio?.id && s.itemActive, props.busy && s.disabled, pressed && { opacity: 0.65 }]}>
            <Text style={s.number}>{String(all.indexOf(item) + 1).padStart(2, '0')}</Text>
            <View style={{ flex: 1 }}><Text numberOfLines={2} style={s.itemName}>{item.name}</Text><Text style={[s.badge, !badge.known && { color: theme.textMuted }]}>{badge.text}</Text></View>
            <Text style={s.buttonText}>{item.id === audio?.id ? '已选' : '›'}</Text>
          </Pressable>; })}
          {!rows.length && <View style={s.empty}><Text style={s.muted}>{!all.length ? '还没有音频，点击上方「导入音频」。' : query ? '没有匹配的音频，试试其他关键词。' : filter === 'bilingual' ? '这一组还没有双语字幕，可先选中音频后点「翻译成中文」。' : '这一组的字幕都准备好了。'}</Text>
            {!!all.length && action('显示全部', () => { setQuery(''); setFilter('all'); })}</View>}
        </AudioList>
      </View>
      <View style={[s.panel, wide && { flex: 1, minHeight: 0 }]}>
        <DetailBody testID="audio-library-detail" style={wide ? { flex: 1, minHeight: 0 } : undefined}>

        {audio ? <View style={s.detail}>
          <Text style={s.label}>当前音频</Text>
          <Text style={s.detailName}>{audio.name}</Text>
          {action('▶ 开始学习', props.onStart ?? (() => {}), props.busy || !props.onStart, true)}
          <View style={s.divider} />
          <View style={s.between}><Text style={s.title}>字幕</Text><Text style={s.muted}>{
            !audio.subtitle_uri ? '尚未添加'
              : props.subtitleKinds[audio.id] === 'bilingual' ? '双语 · 已就绪'
              : props.subtitleKinds[audio.id] === 'chinese' ? '中文 · 已就绪'
              : '已就绪'
          }</Text></View>
          <Text style={s.muted}>{
            !audio.subtitle_uri ? '可以直接听，也可以先生成或导入字幕。'
              : props.subtitleKinds[audio.id] === 'bilingual' ? '原文 + 中文对照已就绪，可逐句对照学习。'
              : props.subtitleKinds[audio.id] === 'chinese' ? '中文字幕已就绪。'
              : '查看原文，或添加中文翻译辅助理解。'
          }</Text>
          <View style={s.row}>
            {audio.subtitle_uri ? <>
              {action('查看字幕', props.onView, props.busy)}
              {action('翻译成中文', props.onTranslate, props.busy)}
            </> : action('AI 生成字幕', props.onGenerate, props.busy)}
            {action(audio.subtitle_uri ? '更换字幕' : '导入字幕', props.onUploadSubtitle, props.busy)}
          </View>
          {!!props.status && <Text accessibilityLiveRegion="polite" style={s.status}>{props.status}</Text>}
          <Details title="更多操作">
            <View style={{ gap: 12 }}>
              {audio.subtitle_uri && action('重新生成字幕', props.onGenerate, props.busy)}
              {action('音频操作', props.onAudioMenu, props.busy)}
              <Text style={s.label}>文件位置</Text><Text selectable style={s.path}>{audio.audio_uri}</Text>
            </View>
          </Details>
        </View> : <View style={s.empty}><Text style={s.title}>选择一条音频</Text><Text style={s.muted}>选中后即可开始学习或处理字幕。</Text></View>}
        <View style={{ padding: 18, paddingBottom: 72 }}>
          <Details title="高级导入与转写设置">{props.advanced}</Details>
        </View>
        </DetailBody>
      </View>
    </View>}
    {!practice && <Details title="高级导入与转写设置">{props.advanced}</Details>}
    </Layout>
  </View>;
}
