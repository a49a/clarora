import { Pressable, Text, View } from 'react-native';
import { useAppTheme } from './ThemeContext';
import type { SubtitleLanguage } from '../data/subtitles';

export function StudySubtitleToolbar({ kind, hasSubtitles, busy, status, onEnglish, onChinese, onTranslate, onGenerate }: {
  kind: SubtitleLanguage; hasSubtitles: boolean; busy: boolean; status: string | null;
  onEnglish: () => void; onChinese: () => void; onTranslate: () => void; onGenerate: () => void;
}) {
  const { theme } = useAppTheme();
  const button = (label: string, action: () => void, primary = false) => <Pressable accessibilityRole="button" disabled={busy} onPress={action}
    style={({ pressed }) => ({ minHeight: 36, justifyContent: 'center', borderRadius: 12, paddingHorizontal: 12, paddingVertical: 8, backgroundColor: primary ? theme.accent : theme.surfaceHover, opacity: busy ? .45 : pressed ? .7 : 1 })}>
    <Text style={{ color: primary ? '#fff' : theme.accent, fontSize: 13, fontWeight: '600' }}>{label}</Text>
  </Pressable>;
  return <View style={{ gap: 8, marginBottom: 10 }}>
    <View style={{ flexDirection: 'row', flexWrap: 'wrap', alignItems: 'center', gap: 8 }}>
      <Text style={{ color: theme.textSecondary, fontSize: 13, marginRight: 8 }}>{!hasSubtitles ? '暂无字幕' : kind === 'bilingual' ? '中英字幕已就绪' : kind === 'chinese' ? '已有中文 · 缺少英文' : '已有英文 · 缺少中文'}</Text>
      {hasSubtitles && kind === 'original' && button('生成中文字幕', onTranslate, true)}
      {!hasSubtitles && button('AI 生成字幕', onGenerate, true)}
      {button('导入英文', onEnglish)}
      {button('导入中文', onChinese)}
    </View>
    {!!status && <Text accessibilityLiveRegion="polite" style={{ color: theme.accent, fontSize: 13 }}>{status}</Text>}
    {!hasSubtitles && !status && <Text style={{ color: theme.textMuted, fontSize: 12 }}>可导入 SRT / VTT，或先生成原文，再生成中文。导入另一种语言时会按时间轴合并。</Text>}
  </View>;
}
