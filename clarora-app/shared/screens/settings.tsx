import { useEffect, useRef, useState } from 'react';
import { Platform, Pressable, ScrollView, StyleSheet, Switch, Text, TextInput, View } from 'react-native';
import { Details } from '../ui/Details';
import { SecretInput } from '../ui/SecretInput';
import { learningDesign } from '../ui/learningDesign';
import { useAppTheme } from '../ui/ThemeContext';
import { getTheme, THEME_DETAILS, THEME_LABELS, THEME_ORDER, type ThemeName } from '../ui/theme';
import { getSetting, setSetting } from '../data/database';
import { loadStorageConfig, saveStorageConfig, DEFAULT_STORAGE, type StorageConfig } from '../services/objectStorage';
import { listBackups, uploadLibrary, downloadLibrary, type BackupInfo } from '../services/librarySync';
import { loadAiConfig, saveAiConfig, DEFAULT_AI, LOCAL_ASR_MODELS, downloadLocalModel, deleteLocalModel, localModelDownloaded, type AiConfig } from '../services/ai';

export default function SettingsScreen() {
  const { theme, themeName, setThemeName, systemScheme } = useAppTheme();
  const ui = learningDesign(theme);
  const styles = StyleSheet.create({
    page: { flex: 1, backgroundColor: theme.bg }, content: { padding: 24, gap: 18, width: '100%', maxWidth: 920, alignSelf: 'center' },
    title: { fontSize: 30, fontWeight: '700', color: theme.text }, label: { fontSize: 15, fontWeight: '600', color: theme.text },
    hint: { color: theme.textSecondary, lineHeight: 22 }, section: { padding: 18, gap: 12, backgroundColor: theme.surface, borderRadius: 16 },
    row: { flexDirection: 'row', flexWrap: 'wrap', alignItems: 'center', gap: 10 },
    input: { borderWidth: 1, borderColor: theme.border, borderRadius: 9, padding: 12, color: theme.text, backgroundColor: theme.bg, minHeight: 44 },
    button: { paddingHorizontal: 16, paddingVertical: 12, borderRadius: 10, backgroundColor: theme.surfaceHover },
    selected: { borderWidth: 1, borderColor: theme.accent }, status: { color: theme.accent, lineHeight: 22 },
    segGroup: { flexDirection: 'row', flexWrap: 'wrap', gap: 8 },
    themeSwatches: { flexDirection: 'row', gap: 5, marginBottom: 5 },
    themeSwatch: { width: 18, height: 18, borderRadius: 9, borderWidth: 1, borderColor: 'rgba(128,128,128,0.25)' },
    segBtn: {
      flex: 1,
      minWidth: 120,
      alignItems: 'center',
      gap: 3,
      paddingVertical: 12,
      borderWidth: 1,
      borderColor: theme.border,
      backgroundColor: theme.surfaceHover,
      ...ui.button,
    },
    segBtnActive: { backgroundColor: theme.accent, borderColor: theme.accent },
    segBtnTitle: { color: theme.text, fontSize: 14, fontWeight: '700' },
    segBtnTitleActive: { color: '#fff' },
    segBtnDetail: { color: theme.textMuted, fontSize: 11 },
    segBtnDetailActive: { color: 'rgba(255,255,255,0.85)' },
  });
  const [storage, setStorage] = useState<StorageConfig>({ ...DEFAULT_STORAGE });
  const [ai, setAi] = useState<AiConfig>({ ...DEFAULT_AI });
  const [backups, setBackups] = useState<BackupInfo[]>([]);
  const [selectedBackup, setSelectedBackup] = useState('');
  const [busy, setBusy] = useState(false);
  const busyRef = useRef(false);
  const [ready, setReady] = useState(false);
  const [status, setStatus] = useState('');
  const [aiStatus, setAiStatus] = useState('');
  const [savedAiKey, setSavedAiKey] = useState(false);
  const [fontSize, setFontSize] = useState(24);
  const [scrollSpeed, setScrollSpeed] = useState('normal');
  const [command, setCommand] = useState('');
  const [localReady, setLocalReady] = useState<Record<string, boolean>>({});
  useEffect(() => {
    let mounted = true;
    if (Platform.OS === 'macos' || Platform.OS === 'windows') {
      (async () => {
        const entries = await Promise.all(LOCAL_ASR_MODELS.map(async model => [model.id, await localModelDownloaded(model.id)] as const));
        if (mounted) setLocalReady(Object.fromEntries(entries));
      })().catch(() => {});
    }
    return () => { mounted = false; };
  }, []);
  const refreshLocalModels = async () => {
    const entries = await Promise.all(LOCAL_ASR_MODELS.map(async model => [model.id, await localModelDownloaded(model.id)] as const));
    setLocalReady(Object.fromEntries(entries));
  };
  useEffect(() => {
    let mounted = true;
    Promise.all([loadStorageConfig(), loadAiConfig().catch((error: Error) => { if (mounted) setAiStatus(error.message); return loadAiConfig({ includeSecrets: false }); }), getSetting('flashcard_font_size'), getSetting('scroll_speed'), getSetting('import_command')]).then(([s, a, f, speed, cmd]) => {
      if (!mounted) return;
      setStorage(s); setAi(a); setSavedAiKey(!!a.apiKey); setFontSize(Number(f) || 24); setScrollSpeed(speed || 'normal'); setCommand(cmd || ''); setReady(true);
    }).catch(() => { if (mounted) setStatus('设置读取失败，请重新打开设置页'); });
    return () => { mounted = false; };
  }, []);
  const run = async (action: () => Promise<string>) => {
    if (busyRef.current) return;
    busyRef.current = true; setBusy(true); setStatus('正在处理…');
    try { setStatus(await action()); } catch (error) { setStatus((error as Error).message || '操作失败'); }
    finally { busyRef.current = false; setBusy(false); }
  };
  const saveAi = async () => {
    if (busyRef.current) return;
    busyRef.current = true; setBusy(true); setAiStatus('正在保存…');
    try {
      const saved = await saveAiConfig(ai);
      setAi(saved);
      setSavedAiKey(!!saved.apiKey);
      setAiStatus(saved.apiKey ? 'AI 配置已保存，API Key 已保存（尚未验证服务端授权）' : 'AI 配置已保存；未填写 API Key，远程 AI 服务无法使用');
    } catch (error) {
      setAiStatus(`保存失败：${(error as Error).message || '请重试'}；输入内容已保留`);
    } finally { busyRef.current = false; setBusy(false); }
  };
  const button = (label: string, onPress: () => void, selected = false) => <Pressable accessibilityRole="button" accessibilityState={{ disabled: busy || !ready, selected }} disabled={busy || !ready} onPress={onPress}
    style={[styles.button, selected && styles.selected, (busy || !ready) && { opacity: 0.45 }]}><Text style={styles.label}>{label}</Text></Pressable>;
  const field = (label: string, value: string, onChange: (s: string) => void, placeholder = '', secret = false) => <View style={{ gap: 6 }}>
    <Text style={styles.label}>{label}</Text>{secret ? <View style={styles.input}>
      <SecretInput accessibilityLabel={label} value={value} textColor={theme.text}
        onChangeText={text => { if (!busyRef.current) onChange(text); }}
        placeholder={placeholder} autoCorrect={false} autoCapitalize="none" editable={ready} />
    </View> : <TextInput accessibilityLabel={label} style={styles.input} value={value}
      onChangeText={text => { if (!busyRef.current) onChange(text); }}
      placeholder={placeholder} placeholderTextColor={theme.textMuted} autoCorrect={false} autoCapitalize="none" editable={ready && !busy} />}
  </View>;
  const storageField = (key: keyof StorageConfig, label: string, placeholder = '', secret = false) => field(label, String(storage[key]), value => { setStorage(s => ({ ...s, [key]: value })); setBackups([]); setSelectedBackup(''); }, placeholder, secret);
  const aiField = (key: keyof AiConfig, label: string, placeholder = '', secret = false) => field(label, ai[key], value => { setAi(s => ({ ...s, [key]: value })); setAiStatus('有未保存的更改'); if (key === 'apiKey') setSavedAiKey(false); }, placeholder, secret);
  const refreshBackups = async () => { const list = await listBackups(); setBackups(list); setSelectedBackup(list[0]?.key || ''); return list; };
  return <View style={styles.page}><ScrollView contentContainerStyle={styles.content} keyboardShouldPersistTaps="handled">
    <Text style={styles.title}>设置</Text>
    <Text style={styles.hint}>资料保存在本机。所有客户端功能开放，无需 Clarora 账号。</Text>
    <View accessibilityLiveRegion="polite"><Text style={styles.status}>{status}</Text></View>
    <View style={styles.section}><Text style={styles.label}>外观主题</Text><View style={styles.segGroup}>
      {THEME_ORDER.map((name: ThemeName) => {
        const selected = themeName === name;
        const preview = getTheme(name, systemScheme);
        return <Pressable key={name} accessibilityRole="button" accessibilityState={{ selected }}
          accessibilityLabel={`${THEME_LABELS[name]}主题`}
          style={[styles.segBtn, selected && styles.segBtnActive]} onPress={() => setThemeName(name)}>
          <View style={styles.themeSwatches}>
            {[preview.bg, preview.sidebar, preview.accent].map((color, index) => <View key={index} style={[styles.themeSwatch, { backgroundColor: color }]} />)}
          </View>
          <Text style={[styles.segBtnTitle, selected && styles.segBtnTitleActive]}>{THEME_LABELS[name]}</Text>
          <Text style={[styles.segBtnDetail, selected && styles.segBtnDetailActive]}>{THEME_DETAILS[name]}</Text>
        </Pressable>;
      })}
    </View>
    <Text style={styles.hint}>{themeName === 'night' ? '当前主题：夜航。固定使用深色外观，适合夜间学习。' : `当前主题：${THEME_LABELS[themeName]}。明暗自动跟随系统外观。`}</Text></View>
    <View style={styles.section}><Text style={styles.label}>卡片文字大小 · {fontSize}</Text><View style={styles.row}>
      {[-2, 2].map(delta => <View key={delta}>{button(delta < 0 ? 'A−' : 'A+', () => { const size = Math.max(16, Math.min(34, fontSize + delta)); void run(async () => { await setSetting('flashcard_font_size', String(size)); setFontSize(size); return '文字大小已保存'; }); })}</View>)}
    </View></View>
    <Details title="同步与备份 · 自有存储" initialOpen><View style={styles.section}>
      <Text style={styles.hint}>在各设备配置相同的 Bucket 和资料库前缀。先在一台设备备份，再在另一台选择版本合并。每次备份保留独立版本。</Text>
      <View style={styles.row}>{(['s3', 'oss'] as const).map(provider => <View key={provider}>{button(provider === 's3' ? 'S3 兼容存储' : '阿里云 OSS', () => { setStorage(s => ({ ...s, provider })); setBackups([]); setSelectedBackup(''); }, storage.provider === provider)}</View>)}</View>
      {storageField('endpoint', 'Endpoint（服务域名，不含 Bucket）', storage.provider === 'oss' ? 'https://oss-cn-hangzhou.aliyuncs.com' : 'https://s3.ap-southeast-1.amazonaws.com')}
      {storageField('region', 'Region', storage.provider === 'oss' ? 'cn-hangzhou' : 'ap-southeast-1')}
      {storageField('bucket', 'Bucket')}{storageField('prefix', '资料库前缀', 'clarora')}
      {storageField('accessKeyId', 'Access Key ID')}{storageField('secretAccessKey', 'Secret Access Key', '', true)}
      {storageField('sessionToken', '临时凭证 Token（可选）', '', true)}
      {storage.provider === 's3' && <View style={styles.row}><Switch accessibilityLabel="Path-style 地址" disabled={busy || !ready} value={storage.pathStyle} onValueChange={pathStyle => { setStorage(s => ({ ...s, pathStyle })); setBackups([]); setSelectedBackup(''); }} /><Text style={styles.hint}>Path-style（按存储商要求启用）</Text></View>}
      <Text style={styles.hint}>凭证保存在本机：正式版存入系统凭证保险库（macOS 钥匙串 / Windows 凭证管理器），开发版与移动端存应用数据库；均不进入云备份。请使用仅可访问该资料库的专用密钥。备份通过 HTTPS 传输，未提供端到端加密。</Text>
      <View style={styles.row}>
        {button('保存并读取备份', () => { void run(async () => { await saveStorageConfig(storage); const list = await refreshBackups(); return `连接成功，找到 ${list.length} 个备份（已验证读取权限）`; }); })}
        {button('备份本机资料', () => { void run(async () => { await saveStorageConfig(storage); const result = await uploadLibrary(setStatus); await refreshBackups(); return `备份完成：${result.words} 个单词，${result.audios} 个音频，${result.files} 个附件`; }); })}
      </View>
      <Text style={styles.hint}>合并保留本机已有内容，补入缺少的记录；复习进度采用较晚评分。不会传播删除，也不会自动覆盖本机编辑。同日统计取较大值，不累加多设备时长。</Text>
      {backups.length > 0 && <>
        <Text style={styles.label}>选择要合并的版本</Text>
        <ScrollView style={{ maxHeight: 220 }} nestedScrollEnabled>{backups.map(backup => <View key={backup.key} style={{ marginBottom: 6 }}>
          {button(`${new Date(parseInt(backup.id.split('-')[0], 36)).toLocaleString()} · ${backup.id.slice(-6)}`, () => setSelectedBackup(backup.key), selectedBackup === backup.key)}
        </View>)}</ScrollView>
        {button('将选中版本合并到本机', () => { if (!selectedBackup) return; void run(async () => { const result = await downloadLibrary(selectedBackup, setStatus); return `合并完成：${result.words} 个单词，${result.audios} 个音频。返回学习页面查看。`; }); })}
      </>}
    </View></Details>
    {(Platform.OS === 'macos' || Platform.OS === 'windows') && <Details title="字幕转写引擎"><View style={styles.section}>
      <Text style={styles.hint}>「端侧模型」在本机离线生成字幕，音频不上传、不消耗转写 API；英文字幕的翻译仍使用「AI 服务」中的聊天模型。macOS 依赖 whisper.cpp 与 sherpa-onnx 本机库，Windows 依赖 clarora_asr.dll（安装与构建见平台文档），模型首次使用前需在本区块下载。</Text>
      <View style={styles.row}>
        {button('端侧模型（离线）', () => { void run(async () => { await saveAiConfig({ ...ai, asrEngine: 'local' }); setAi(s => ({ ...s, asrEngine: 'local' })); return '已选择端侧模型转写'; }); }, ai.asrEngine === 'local')}
        {button('自定义转写 API', () => { void run(async () => { await saveAiConfig({ ...ai, asrEngine: 'compatible' }); setAi(s => ({ ...s, asrEngine: 'compatible' })); return '已选择自定义转写 API'; }); }, ai.asrEngine === 'compatible')}
      </View>
      {ai.asrEngine === 'local' && <>
        <Text style={styles.label}>端侧模型</Text>
        <View style={{ gap: 8 }}>
          {LOCAL_ASR_MODELS.map(model => <View key={model.id} style={styles.row}>
            {button(`${model.label} · ${model.size}`, () => { void run(async () => { await saveAiConfig({ ...ai, localModel: model.id, asrEngine: 'local' }); setAi(s => ({ ...s, localModel: model.id, asrEngine: 'local' })); return localReady[model.id] ? `已选择 ${model.label}` : `已选择 ${model.label}，请先下载`; }); }, ai.localModel === model.id)}
            {localReady[model.id]
              ? <Text style={styles.hint}>已下载</Text>
              : button('下载', () => { void run(async () => { const message = await downloadLocalModel(model.id); await refreshLocalModels(); return message; }); })}
            {localReady[model.id] && ai.localModel !== model.id && button('删除', () => { void run(async () => { const message = await deleteLocalModel(model.id); await refreshLocalModels(); return message; }); })}
          </View>)}
        </View>
        <Text style={styles.hint}>中文内容选 SenseVoice（支持中英日韩粤，自动检测语言）；纯英文内容选 Whisper .en 模型（更小更准）。英文字幕的中文翻译都由「AI 服务」的聊天模型完成。</Text>
      </>}
    </View></Details>}
    <Details title="AI 服务 · 自定义 API"><View style={styles.section}>
      <Text style={styles.hint}>客户端直接调用你配置的 OpenAI 兼容 API。聊天、翻译、OCR 和转写按需发送当前材料；未配置时仍可使用本地学习。</Text>
      {ai.asrEngine === 'local' && <Text style={styles.hint}>当前转写引擎为端侧模型，下方转写配置仅在切换到「自定义转写 API」引擎时生效。</Text>}
      {aiField('baseUrl', 'API Base URL', 'https://your-provider.example/v1')}{aiField('apiKey', 'API Key（本机服务可留空）', '', true)}
      {aiField('model', '聊天模型')}{aiField('visionModel', '视觉模型（OCR）')}
      {aiField('asrBaseUrl', '转写 Base URL（留空沿用上方）')}{aiField('asrApiKey', '转写 API Key（留空沿用上方）', '', true)}{aiField('asrModel', '转写模型')}
      <Text style={styles.hint}>转写使用 /audio/transcriptions，模型须支持 verbose_json 与 segments 时间轴。跟读评分基于识别文本对齐，记录保存在本机并随资料备份。端侧模型引擎下，字幕与跟读转写同样在本机完成。</Text>
      <Text style={styles.hint}>{savedAiKey ? 'API Key：已保存（掩码显示）' : ai.apiKey ? 'API Key：已填写，待保存' : 'API Key：未配置'}</Text>
      {button(busy ? '正在处理…' : '保存 AI 配置', () => { void saveAi(); })}
      <View accessibilityLiveRegion="polite"><Text style={styles.status}>{aiStatus}</Text></View>
    </View></Details>
    {Platform.OS === 'macos' && <Details title="本机命令导入"><View style={styles.section}>
      {field('导入命令', command, setCommand, 'find ~/Music -maxdepth 2 -name "*.mp3"')}
      <Text style={styles.hint}>在音频管理中主动执行。命令在这台 Mac 上运行，每行输出一个音频文件路径。</Text>
      {button('保存命令', () => { void run(async () => { await setSetting('import_command', command); return '导入命令已保存'; }); })}
    </View></Details>}
    <Details title="闪卡滚动速度与快捷键"><View style={styles.section}><View style={styles.row}>
      {(['slow', 'normal', 'fast'] as const).map((speed, i) => <View key={speed}>{button(['慢', '标准', '快'][i], () => { void run(async () => { await setSetting('scroll_speed', speed); setScrollSpeed(speed); return '滚动速度已保存'; }); }, scrollSpeed === speed)}</View>)}
    </View><Text style={styles.hint}>空格：播放 / 暂停 / 翻面；Enter：显示答案；← / →：切换卡片；↑ / ↓：滚动卡片。</Text></View></Details>
    <View accessibilityLiveRegion="polite"><Text style={styles.status}>{status}</Text></View>
  </ScrollView></View>;
}
