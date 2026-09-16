import { useEffect, useRef, useState } from 'react';
import { BackHandler, KeyboardAvoidingView, Platform, Pressable, ScrollView, StyleSheet, Text, TextInput, View } from 'react-native';
import { useAppTheme } from './ThemeContext';
import { learningDesign } from './learningDesign';

type Props = {
  name: string;
  kind: 'audio' | 'practice';
  onClose: () => void;
  onRename: (name: string) => Promise<void>;
  onDelete: () => Promise<void>;
  onBatch: () => void;
};

export function LibraryActionMenu({ name, kind, onClose, onRename, onDelete, onBatch }: Props) {
  const { theme } = useAppTheme();
  const ui = learningDesign(theme);
  const [step, setStep] = useState<'menu' | 'rename' | 'delete'>('menu');
  const [draft, setDraft] = useState(name);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const locked = useRef(false);
  const label = kind === 'audio' ? '音频' : '练习组';
  const back = () => { if (!locked.current) { setError(''); if (step === 'menu') onClose(); else setStep('menu'); } };
  useEffect(() => {
    const handler = BackHandler.addEventListener('hardwareBackPress', () => { back(); return true; });
    return () => handler.remove();
  }, [step, onClose]);
  const run = async (action: () => Promise<void>) => {
    if (locked.current) return;
    locked.current = true; setBusy(true); setError('');
    try { await action(); onClose(); }
    catch (e: any) { setError(e?.message ?? '操作失败，请重试。'); }
    finally { locked.current = false; setBusy(false); }
  };
  const actionRow = (title: string, detail: string, action: () => void, danger = false) => (
    <Pressable accessibilityRole="button" accessibilityLabel={title} onPress={action}
      style={({ pressed }) => [ui.row, { flexDirection: 'row', alignItems: 'center', padding: 16, gap: 12, backgroundColor: pressed ? theme.surfaceHover : theme.bg }]}>
      <View style={{ flex: 1, gap: 5 }}>
        <Text style={{ color: danger ? theme.danger : theme.text, fontSize: 16, fontWeight: '600' }}>{title}</Text>
        <Text style={{ color: theme.textMuted, fontSize: 13, lineHeight: 20 }}>{detail}</Text>
      </View>
      <Text style={{ color: theme.textMuted, fontSize: 20 }}>›</Text>
    </Pressable>
  );
  return <KeyboardAvoidingView behavior={Platform.OS === 'ios' ? 'padding' : undefined}
    style={[StyleSheet.absoluteFillObject, { zIndex: 80, backgroundColor: 'rgba(0,0,0,0.3)', justifyContent: 'center', alignItems: 'center', padding: 18 }]}>
    <Pressable accessibilityLabel="关闭操作面板" accessibilityRole="button" disabled={busy} onPress={onClose} style={StyleSheet.absoluteFillObject} />
    <View accessibilityViewIsModal style={[ui.panel, { width: '100%', maxWidth: 440, maxHeight: '90%', gap: 20 }]}>
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 12 }}>
        <Text style={{ flex: 1, color: theme.text, fontSize: 22, fontWeight: '700' }}>{step === 'menu' ? `${label}操作` : step === 'rename' ? `重命名${label}` : `删除${label}？`}</Text>
        <Pressable accessibilityRole="button" disabled={busy} onPress={onClose} hitSlop={8}><Text style={{ color: theme.textSecondary, fontSize: 14 }}>关闭</Text></Pressable>
      </View>
      <ScrollView style={{ flexShrink: 1 }} keyboardShouldPersistTaps="handled" contentContainerStyle={{ gap: 16 }}>
        <Text selectable style={{ color: theme.textSecondary, fontSize: 15, lineHeight: 23 }}>{name}</Text>
        {step === 'menu' ? <View style={{ gap: 10 }}>
          {actionRow('重命名', `修改${label}的显示名称`, () => setStep('rename'))}
          {actionRow('批量管理', kind === 'audio' ? '选择多个音频，一起整理或删除' : '选择多个练习组，一起整理或删除', () => { onClose(); onBatch(); })}
          <View style={{ height: 1, backgroundColor: theme.border, marginVertical: 4 }} />
          {actionRow(`删除${label}`, '移除前需要再次确认', () => setStep('delete'), true)}
        </View> : step === 'rename' ? <View style={{ gap: 8 }}>
          <Text style={ui.label}>新名称</Text>
          <TextInput accessibilityLabel="新名称" value={draft} onChangeText={setDraft} editable={!busy}
            onSubmitEditing={() => { if (draft.trim() && draft.trim() !== name) void run(() => onRename(draft.trim())); }}
            style={[ui.input, { borderWidth: 1, color: theme.text, fontSize: 16 }]} />
          <Text style={{ color: theme.textMuted, fontSize: 13 }}>仅修改名称，不改变学习内容。</Text>
        </View> : <Text style={{ color: theme.textSecondary, fontSize: 14, lineHeight: 22 }}>
          {kind === 'audio' ? '将从资料库移除此音频，并清理 App 内保存的音频及字幕文件。此操作无法撤销。' : '将删除此练习组及组内音频，并清理 App 内保存的相关文件。此操作无法撤销。'}
        </Text>}
        {!!error && <Text accessibilityRole="alert" style={{ color: theme.danger, lineHeight: 22 }}>{error}</Text>}
      </ScrollView>
      {step !== 'menu' && <View style={{ flexDirection: 'row', justifyContent: 'flex-end', gap: 10 }}>
        <Pressable accessibilityRole="button" disabled={busy} onPress={back} style={[ui.button, { paddingHorizontal: 18, backgroundColor: theme.surfaceHover }]}><Text style={{ color: theme.textSecondary }}>返回</Text></Pressable>
        <Pressable accessibilityRole="button" disabled={busy || (step === 'rename' && (!draft.trim() || draft.trim() === name))}
          onPress={() => void run(step === 'delete' ? onDelete : () => onRename(draft.trim()))}
          style={[ui.button, { paddingHorizontal: 20, backgroundColor: step === 'delete' ? theme.danger : theme.accent, opacity: busy || (step === 'rename' && (!draft.trim() || draft.trim() === name)) ? 0.5 : 1 }]}>
          <Text style={{ color: '#fff', fontWeight: '600' }}>{busy ? '处理中…' : step === 'delete' ? '确认删除' : '保存名称'}</Text>
        </Pressable>
      </View>}
    </View>
  </KeyboardAvoidingView>;
}
