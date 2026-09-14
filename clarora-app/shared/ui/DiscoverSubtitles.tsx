import { useEffect, useRef, useState, type ReactNode } from 'react';
import { Pressable, ScrollView, Text, View } from 'react-native';
import type { DiscoverSubtitle } from '../data/discover';
import type { Theme } from './theme';

export function DiscoverSubtitles({ cues, position, revealed, theme, onSeek, children }: {
  cues: DiscoverSubtitle[]; position: number; revealed: boolean; theme: Theme;
  onSeek: (ms: number) => void; children?: ReactNode;
}) {
  const scroll = useRef<ScrollView>(null);
  const [height, setHeight] = useState(0);
  const [contentHeight, setContentHeight] = useState(0);
  const [rows, setRows] = useState<Record<number, { y: number; height: number }>>({});
  const active = cues.findIndex(cue => position >= cue.startMs && position < cue.endMs);
  const row = rows[active];
  const precedingRows = Array.from({ length: Math.max(0, active) }, (_, index) => rows[index]);
  const offset = row && precedingRows.every(Boolean)
    ? precedingRows.reduce((total, item) => total + item.height + 8, 0) + row.height / 2 : undefined;

  useEffect(() => {
    if (offset === undefined || !height || !contentHeight) return;
    // Half a viewport of padding cancels the viewport offset. Sum measured
    // heights to avoid mixing coordinates from nested native views.
    const frame = requestAnimationFrame(() => {
      scroll.current?.scrollTo({ y: offset, animated: true });
    });
    return () => cancelAnimationFrame(frame);
  }, [active, offset, height, contentHeight, revealed]);

  return <View style={{ flex: 1 }}>
    <Text style={{ color: theme.textMuted, fontSize: 12, marginBottom: 8 }}>点击字幕，从这一句播放</Text>
    <ScrollView ref={scroll} testID="discover-subtitles" style={{ flex: 1 }}
    onLayout={event => setHeight(event.nativeEvent.layout.height)}
    onContentSizeChange={(_, value) => setContentHeight(value)}
    contentContainerStyle={{ paddingVertical: height / 2, gap: 8 }}>
    {cues.map((cue, index) => <Pressable key={`${cue.startMs}:${index}`}
      accessibilityRole="button" accessibilityLabel={`从这句播放：${cue.text}`}
      onPress={() => onSeek(cue.startMs)}
      onLayout={event => {
        const { y, height: rowHeight } = event.nativeEvent.layout;
        setRows(previous => previous[index]?.y === y && previous[index]?.height === rowHeight
          ? previous : { ...previous, [index]: { y, height: rowHeight } });
      }}
      style={{ padding: 10, borderRadius: 12, backgroundColor: index === active ? theme.surfaceHover : 'transparent' }}>
      <Text style={{ color: theme.text, fontSize: 23, lineHeight: 35 }}>{cue.text}</Text>
      {revealed && !!cue.translation && <Text style={{ color: theme.textSecondary, fontSize: 18, lineHeight: 28, marginTop: 6 }}>{cue.translation}</Text>}
    </Pressable>)}
    {children}
  </ScrollView></View>;
}
