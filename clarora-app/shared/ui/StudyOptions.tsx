import { type ReactNode } from 'react';
import { Platform, Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';
import { useAppTheme } from './ThemeContext';

// macOS 的 react-native-macos 没有 Modal（RCTModalHostView 未注册），下拉面板用
// 绝对定位浮层实现。开关状态由使用方持有（受控组件）：open/onVisibilityChange
// 与使用方渲染的屏幕级遮罩保持同一份状态，点面板外关闭时面板一定同步消失。
export function StudyOptions({ label, title, children, open, onVisibilityChange, direction = 'down', align = 'left' }: {
  open: boolean;
  onVisibilityChange: (visible: boolean) => void;
  direction?: 'down' | 'up';
  align?: 'left' | 'right';
  label: string; title: string; children: ReactNode | ((close: () => void) => ReactNode);
}) {
  const { theme } = useAppTheme();
  const close = () => onVisibilityChange(false);
  const touchHeight = Platform.OS === 'ios' || Platform.OS === 'android' ? 44 : 34;
  const styles = StyleSheet.create({
    panel: {
      position: 'absolute', width: 380, maxWidth: '100%', maxHeight: 560, zIndex: 30,
      ...(direction === 'down' ? { top: touchHeight + 6 } : { bottom: touchHeight + 6 }),
      ...(align === 'left' ? { left: 0 } : { right: 0 }),
      borderRadius: 12, borderWidth: 1, borderColor: theme.border, backgroundColor: theme.surface,
      shadowColor: '#000', shadowOpacity: 0.14, shadowRadius: 18, shadowOffset: { width: 0, height: 6 }, elevation: 8,
    },
    panelHead: {
      flexDirection: 'row', justifyContent: 'space-between', alignItems: 'center',
      paddingHorizontal: 16, paddingVertical: 10, borderBottomWidth: 1, borderBottomColor: theme.border,
    },
    panelTitle: { color: theme.text, fontSize: 16, fontWeight: '700' },
    panelDone: { minHeight: 34, justifyContent: 'center', paddingHorizontal: 10 },
    panelDoneText: { color: theme.accent, fontWeight: '600' },
    panelBody: { padding: 16, gap: 12 },
  });
  return <>
    <Pressable accessibilityRole="button" accessibilityLabel={title} accessibilityState={{ expanded: open }}
      onPress={() => onVisibilityChange(!open)} style={({ pressed }) => ({ minHeight: touchHeight, paddingHorizontal: 12,
        justifyContent: 'center', borderRadius: 8, backgroundColor: theme.surfaceHover, opacity: pressed ? .7 : 1 })}>
      <Text style={{ color: theme.textSecondary, fontSize: 13, fontWeight: '600' }}>{label} {direction === 'down' ? '▾' : '▴'}</Text>
    </Pressable>
    {open && (
      <View accessibilityViewIsModal style={styles.panel}>
        <View style={styles.panelHead}>
          <Text accessibilityRole="header" style={styles.panelTitle}>{title}</Text>
          <Pressable accessibilityRole="button" onPress={close} style={styles.panelDone}>
            <Text style={styles.panelDoneText}>完成</Text>
          </Pressable>
        </View>
        <ScrollView contentContainerStyle={styles.panelBody} keyboardShouldPersistTaps="handled">
          {typeof children === 'function' ? children(close) : children}
        </ScrollView>
      </View>
    )}
  </>;
}
