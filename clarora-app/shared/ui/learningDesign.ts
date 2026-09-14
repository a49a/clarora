import { Platform, StyleSheet } from 'react-native';
import type { Theme } from './theme';

// Shared proportions taken from the discovery feed. Screens keep their own
// playback, chart and gesture geometry; only the surrounding chrome is shared.
export function learningDesign(theme: Theme) {
  return StyleSheet.create({
    page: { width: '100%', maxWidth: 960, alignSelf: 'center', paddingHorizontal: 18, paddingTop: 18, paddingBottom: 24 },
    header: { marginBottom: 18 },
    title: { color: theme.text, fontSize: 26, fontWeight: '700', letterSpacing: 0 },
    subtitle: { color: theme.textMuted, fontSize: 14, lineHeight: 22, marginTop: 6 },
    eyebrow: { display: 'none' },
    panel: { backgroundColor: theme.surface, borderColor: theme.border, borderWidth: 1, borderRadius: 24, padding: Platform.OS === 'android' ? 18 : 24, shadowOpacity: 0, elevation: 0 },
    card: { backgroundColor: theme.surface, borderColor: theme.border, borderWidth: 1, borderRadius: 24, shadowOpacity: 0, elevation: 0 },
    label: { color: theme.textSecondary, fontSize: 14, fontWeight: '600', letterSpacing: 0 },
    button: { borderRadius: 14, minHeight: 40, justifyContent: 'center', shadowOpacity: 0, elevation: 0 },
    input: { borderRadius: 12, backgroundColor: theme.bg, borderColor: theme.border, paddingHorizontal: 14, paddingVertical: 12 },
    row: { borderRadius: 14, shadowOpacity: 0, elevation: 0 },
  });
}
