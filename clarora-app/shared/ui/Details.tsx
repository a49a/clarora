import { useState, type ReactNode } from 'react';
import { Pressable, Text, View } from 'react-native';
import { useAppTheme } from './ThemeContext';

/** Secondary tools stay one click away without crowding the learning surface. */
export function Details({ title, children, initialOpen = false }: { title: string; children: ReactNode; initialOpen?: boolean }) {
  const [open, setOpen] = useState(initialOpen);
  const { theme } = useAppTheme();
  return <View style={{ marginBottom: 12 }}>
    <Pressable accessibilityRole="button" accessibilityState={{ expanded: open }} onPress={() => setOpen(value => !value)}
      style={{ flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between', minHeight: 44, paddingHorizontal: 14, borderRadius: 14, backgroundColor: theme.surfaceHover }}>
      <Text style={{ color: theme.textSecondary, fontSize: 14, fontWeight: '500' }}>{title}</Text>
      <Text style={{ color: theme.textMuted, fontSize: 18 }}>{open ? '−' : '+'}</Text>
    </Pressable>
    <View accessibilityElementsHidden={!open} importantForAccessibility={open ? 'auto' : 'no-hide-descendants'}
      style={{ display: open ? 'flex' : 'none', paddingTop: 14 }}>{children}</View>
  </View>;
}
