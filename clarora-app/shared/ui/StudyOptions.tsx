import { useState, type ReactNode } from 'react';
import { Modal, Platform, Pressable, ScrollView, Text, View } from 'react-native';
import { useAppTheme } from './ThemeContext';

// Keep secondary tools out of the transcript; a scrollable dialog also works
// on small screens and avoids clipping menus inside native subtitle views.
export function StudyOptions({ label, title, children, onVisibilityChange }: {
  onVisibilityChange?: (visible: boolean) => void;
  label: string; title: string; children: ReactNode | ((close: () => void) => ReactNode);
}) {
  const { theme } = useAppTheme();
  const [visible, setVisible] = useState(false);
  const close = () => { setVisible(false); onVisibilityChange?.(false); };
  const touchHeight = Platform.OS === 'ios' || Platform.OS === 'android' ? 44 : 34;
  return <>
    <Pressable accessibilityRole="button" accessibilityLabel={title} accessibilityState={{ expanded: visible }}
      onPress={() => { setVisible(true); onVisibilityChange?.(true); }} style={({ pressed }) => ({ minHeight: touchHeight, paddingHorizontal: 12,
        justifyContent: 'center', borderRadius: 8, backgroundColor: theme.surfaceHover, opacity: pressed ? .7 : 1 })}>
      <Text style={{ color: theme.textSecondary, fontSize: 13, fontWeight: '600' }}>{label} ▾</Text>
    </Pressable>
    <Modal visible={visible} transparent animationType="fade" onRequestClose={close}>
      <View style={{ flex: 1, justifyContent: 'center', alignItems: 'center', padding: 24, backgroundColor: '#00000055' }}>
        <Pressable accessibilityRole="button" accessibilityLabel="关闭设置" onPress={close}
          style={{ position: 'absolute', top: 0, right: 0, bottom: 0, left: 0 }} />
        <View accessibilityViewIsModal style={{ width: '100%', maxWidth: 460, maxHeight: '85%', borderRadius: 18,
          padding: 20, backgroundColor: theme.surface, borderWidth: 1, borderColor: theme.border }}>
          <View style={{ flexDirection: 'row', justifyContent: 'space-between', alignItems: 'center', marginBottom: 14 }}>
            <Text accessibilityRole="header" style={{ color: theme.text, fontSize: 17, fontWeight: '700' }}>{title}</Text>
            <Pressable accessibilityRole="button" onPress={close} style={{ minHeight: 44, justifyContent: 'center', paddingHorizontal: 10 }}>
              <Text style={{ color: theme.accent, fontWeight: '600' }}>完成</Text>
            </Pressable>
          </View>
          <ScrollView style={{ flexShrink: 1 }} contentContainerStyle={{ gap: 12 }} keyboardShouldPersistTaps="handled">
            {typeof children === 'function' ? children(close) : children}
          </ScrollView>
        </View>
      </View>
    </Modal>
  </>;
}
