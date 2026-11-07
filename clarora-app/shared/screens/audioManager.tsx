import {
  commandAudioFiles,
  importCommandAudio,
  mergeLibraryAudio,
  removeLibraryAudio,
  removeLibraryPractice,
  type CommandAudioFile,
} from '../services/listeningLibrary';
import { LibraryActionMenu } from '../ui/LibraryActionMenu';
import { AudioLibraryManager } from '../ui/AudioLibraryManager';
import { useCallback, useEffect, useRef, useState } from 'react';
import { Alert, AppState, Platform, Pressable, ScrollView, StatusBar, Text, TextInput, View } from 'react-native';
import { copyToClipboard, DocumentPicker, FileSystem, runShellCommand } from '../services/platform';

import {
  ListeningPractice,
  ListeningAudio,
  createListeningPractice,
  addAudioToPractice,
  renameListeningPractice,
  renameListeningAudio,
  getSetting,
} from '../data/database';
import { classifySubtitleLanguage, parseSubtitleCues, SubtitleCue, type SubtitleLanguage } from '../data/subtitles';
import { getActiveAsrEngine, listAsrEngines, updateAsrEngine, type AsrEngineInfo } from '../services/ai';
import { useAppTheme } from '../ui/ThemeContext';

import { makeListeningStyles } from '../ui/listeningStyles';
import { useListeningLibrary, type ListeningSelection } from '../ui/useListeningLibrary';
import { useListeningSubtitles, ASR_ENGINE_LABELS, getFileNameFromUri } from '../ui/useListeningSubtitles';

function formatTime(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  return `${m}:${s.toString().padStart(2, '0')}`;
}

type AudioManagerProps = ListeningSelection & {
  onStartStudy: (practiceId: string, audioId: string) => void;
};

export default function AudioManagerScreen({ onStartStudy, ...selection }: AudioManagerProps) {
  const { theme, scheme } = useAppTheme();
  const styles = makeListeningStyles(theme);
  const [error, setError] = useState<string | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  const {
    practices,
    selectedPractice,
    selectedAudio,
    selectedPracticeId,
    selectedAudioId,
    setSelectedPracticeId,
    setSelectedAudioId,
    refreshPractices,
  } = useListeningLibrary(selection, setError);
  const [scanImporting, setScanImporting] = useState(false);
  const [scanFiles, setScanFiles] = useState<CommandAudioFile[]>([]);
  const scanPracticeId = useRef<string | null>(null);
  const [scanPreviewVisible, setScanPreviewVisible] = useState(false);
  const canUseCommandImport = Platform.OS === 'macos';
  const [activeAsrModel, setActiveAsrModel] = useState('');
  const [asrEngines, setAsrEngines] = useState<AsrEngineInfo[]>([]);
  const [activeAsrBackend, setActiveAsrBackend] = useState<string>('');
  const [asrEnginesLoading, setAsrEnginesLoading] = useState(false);
  const [showEnginePicker, setShowEnginePicker] = useState(false);

  const refreshCommandImportAccess = useCallback(async () => {
    void getActiveAsrEngine().then((engine) => {
      if (engine) {
        setActiveAsrModel(`${ASR_ENGINE_LABELS[engine.backend] ?? engine.backend} ${engine.model}`.trim());
      }
    });
  }, []);

  useEffect(() => {
    void refreshCommandImportAccess();
    const subscription = AppState.addEventListener('change', (state) => {
      if (state === 'active') void refreshCommandImportAccess();
    });
    return () => subscription.remove();
  }, [refreshCommandImportAccess]);

  const [batchMode, setBatchMode] = useState(false);
  const [batchSelected, setBatchSelected] = useState<string[]>([]);

  // Batch audio management
  const [batchAudioMode, setBatchAudioMode] = useState(false);
  const [batchAudioSelected, setBatchAudioSelected] = useState<string[]>([]);
  const [mergingAudios, setMergingAudios] = useState(false);

  // Prompt dialog state (cross-platform replacement for Alert.prompt)
  const [promptVisible, setPromptVisible] = useState(false);
  const [libraryMenu, setLibraryMenu] = useState<'audio' | 'practice' | null>(null);
  const [promptTitle, setPromptTitle] = useState('');
  // 字幕查看器：生成或导入后可以直接翻看转录结果。
  const [subtitleViewerVisible, setSubtitleViewerVisible] = useState(false);
  const [subtitleViewerLoading, setSubtitleViewerLoading] = useState(false);
  const [subtitleViewerCues, setSubtitleViewerCues] = useState<SubtitleCue[]>([]);
  const [promptValue, setPromptValue] = useState('');
  const promptResolveRef = useRef<((value: string | null) => void) | null>(null);

  const showPrompt = useCallback((title: string, defaultValue = ''): Promise<string | null> => {
    return new Promise((resolve) => {
      setPromptTitle(title);
      setPromptValue(defaultValue);
      promptResolveRef.current = resolve;
      setPromptVisible(true);
    });
  }, []);

  useEffect(
    () => () => {
      promptResolveRef.current?.(null);
      promptResolveRef.current = null;
    },
    [],
  );

  const handlePromptConfirm = useCallback(() => {
    setPromptVisible(false);
    promptResolveRef.current?.(promptValue);
    promptResolveRef.current = null;
  }, [promptValue]);

  const handlePromptCancel = useCallback(() => {
    setPromptVisible(false);
    promptResolveRef.current?.(null);
    promptResolveRef.current = null;
  }, []);

  // NOTE: no programmatic focus here. react-native-macos's focus command
  // crashes in AppKit (NSWindow first-responder exception), so the user
  // clicks into the input manually.

  const showToast = useCallback((msg: string) => {
    setToast(msg);
    setTimeout(() => setToast(null), 2300);
  }, []);

  const loadAsrEngines = useCallback(async () => {
    setAsrEnginesLoading(true);
    try {
      const list = await listAsrEngines();
      setAsrEngines(list.backends);
      setActiveAsrBackend(list.config.backend ?? '');
    } catch (e: any) {
      setError(`读取转写引擎失败：${e?.message ?? e}`);
      setShowEnginePicker(false);
    } finally {
      setAsrEnginesLoading(false);
    }
  }, []);

  const selectAsrEngine = useCallback(
    async (engine: AsrEngineInfo) => {
      if (!engine.available || engine.backend === activeAsrBackend) return;
      setAsrEnginesLoading(true);
      try {
        const updated = await updateAsrEngine(engine.backend);
        setActiveAsrBackend(updated.backend);
        showToast(`转写模型已切换为 ${updated.label}`);
        await loadAsrEngines();
      } catch (e: any) {
        setError(`切换失败：${e?.message ?? e}`);
      } finally {
        setAsrEnginesLoading(false);
      }
    },
    [activeAsrBackend, loadAsrEngines, showToast],
  );

  const preview = useCallback((cues: SubtitleCue[]) => {
    setSubtitleViewerCues(cues);
    setSubtitleViewerLoading(false);
    setSubtitleViewerVisible(true);
  }, []);
  const { aiStatus, subtitleTaskRef, handleUploadSubtitle, handleAiSubtitle, runTranslate } = useListeningSubtitles({
    selectedAudio,
    refreshPractices,
    showToast,
    setError,
    onPreview: preview,
  });

  function selectLibraryItem(practice: ListeningPractice, audio?: ListeningAudio) {
    if (subtitleTaskRef.current || scanImporting) return;
    setSelectedPracticeId(practice.id);
    setSelectedAudioId(audio?.id ?? null);
  }

  // ── Practice management ─────────────────────────────────────────────────────

  // 字幕语言形态（双语/中文/仅原文）：管理页按 audioId 展示徽标与筛选。
  const [subtitleKinds, setSubtitleKinds] = useState<Record<string, SubtitleLanguage>>({});
  const subtitleKindCache = useRef<Record<string, SubtitleLanguage | 'unknown'>>({});

  useEffect(() => {
    // 缓存键为 id:uri，翻译/更换字幕后自动重新判定。
    let cancelled = false;
    const run = async () => {
      for (const practice of practices) {
        for (const audio of practice.audios) {
          const cacheKey = `${audio.id}:${audio.subtitle_uri}`;
          if (!audio.subtitle_uri || subtitleKindCache.current[cacheKey]) continue;
          try {
            const content = await FileSystem.readAsStringAsync(audio.subtitle_uri);
            const kind = classifySubtitleLanguage(parseSubtitleCues(content));
            subtitleKindCache.current[cacheKey] = kind;
            if (cancelled) return;
            setSubtitleKinds((prev) => ({ ...prev, [audio.id]: kind }));
          } catch {
            // 读取失败保持未知，列表回退显示「字幕就绪」。
            subtitleKindCache.current[cacheKey] = 'unknown';
          }
        }
      }
    };
    void run();
    return () => {
      cancelled = true;
    };
  }, [practices]);

  const handleCreatePractice = useCallback(async () => {
    const name = await showPrompt('请输入练习组名称', `我的听力练习 ${new Date().toLocaleDateString()}`);
    if (!name?.trim()) return;
    try {
      const created = await createListeningPractice(name.trim());
      await refreshPractices();
      setSelectedPracticeId(created.id);
      setSelectedAudioId(null);
      showToast(`练习已创建：${created.name}`);
    } catch (e: any) {
      setError(`创建失败：${e?.message ?? e}`);
    }
  }, [refreshPractices, showToast, showPrompt]);

  const handleAddAudio = useCallback(async () => {
    if (!selectedPracticeId) return;
    setError(null);
    try {
      // Pick audio file
      const audioResult = await DocumentPicker.getDocumentAsync({
        type: 'audio/*',
        copyToCacheDirectory: true,
      });
      if (audioResult.canceled || !audioResult.assets?.length) return;
      const audioAsset = audioResult.assets[0];

      const defaultName = audioAsset.name.replace(/\.[^.]+$/, '');

      const inputName = await showPrompt('请输入音频名称', defaultName);
      if (inputName == null) return;
      const name = inputName.trim() || defaultName;
      try {
        const created = await addAudioToPractice(selectedPracticeId, name, audioAsset.uri, '');
        await refreshPractices();
        setSelectedAudioId(created.id);
        showToast(`音频已添加：${name}`);
      } catch (e: any) {
        setError(`添加音频失败：${e?.message ?? e}`);
      }
    } catch (e: any) {
      setError(`添加音频失败：${e?.message ?? e}`);
    }
  }, [selectedPracticeId, refreshPractices, showToast, showPrompt]);

  // Run the configured shell command locally (client machine) and import the
  // audio files whose paths it prints, e.g. `find ~/Music -name "*.mp3"`.
  const handleScanImport = useCallback(async () => {
    if (!selectedPractice) return;
    const command = (await getSetting('import_command'))?.trim();
    if (!command) {
      Alert.alert('未配置导入命令', '请先到「设置 → 命令行导入」填写命令');
      return;
    }
    setScanImporting(true);
    setError(null);
    try {
      const output = await runShellCommand(command);
      const files = commandAudioFiles(output);
      if (files.length === 0) {
        showToast('命令没有返回音频文件');
        return;
      }
      scanPracticeId.current = selectedPractice.id;
      setScanFiles(files);
      setScanPreviewVisible(true);
    } catch (e: any) {
      setError(`命令导入失败：${e?.message ?? e}`);
    } finally {
      setScanImporting(false);
    }
  }, [selectedPractice, showToast]);

  // Actually import the previewed files after the user confirms.
  const confirmScanImport = useCallback(async () => {
    const practiceId = scanPracticeId.current;
    if (!practiceId || scanFiles.length === 0) return;
    scanPracticeId.current = null;
    setScanPreviewVisible(false);
    const { imported, skipped, lastImportedId } = await importCommandAudio(practiceId, scanFiles);
    await refreshPractices();
    setScanFiles([]);
    if (lastImportedId && selectedPractice?.id === practiceId) setSelectedAudioId(lastImportedId);
    showToast(skipped > 0 ? `已导入 ${imported} 个音频，跳过 ${skipped} 个（重名）` : `已导入 ${imported} 个音频`);
  }, [selectedPractice, scanFiles, refreshPractices, showToast]);

  const openSubtitleViewer = useCallback(async () => {
    if (!selectedAudio?.subtitle_uri) return;
    setSubtitleViewerVisible(true);
    setSubtitleViewerLoading(true);
    try {
      const content = await FileSystem.readAsStringAsync(selectedAudio.subtitle_uri);
      setSubtitleViewerCues(parseSubtitleCues(content));
    } catch (e: any) {
      setSubtitleViewerCues([]);
      setError(`读取字幕失败：${e?.message ?? e}`);
    } finally {
      setSubtitleViewerLoading(false);
    }
  }, [selectedAudio]);

  const copySubtitleViewerText = useCallback(() => {
    const text = subtitleViewerCues.map((cue) => cue.text).join('\n');
    if (!text) {
      showToast('字幕内容为空');
      return;
    }
    if (!copyToClipboard(text)) {
      showToast('当前设备暂不支持复制');
      return;
    }
    showToast('已复制字幕文本');
  }, [subtitleViewerCues, showToast]);

  // Translate the current subtitle through the configured model API.
  // Provider credentials belong to this device.
  // If the current subtitle is already a generated translation, confirm  // before overwriting the old one.
  const handleTranslateSubtitle = useCallback(async () => {
    if (!selectedAudio?.subtitle_uri) return;
    const currentName = getFileNameFromUri(selectedAudio.subtitle_uri);
    if (/_zh\.srt$/i.test(currentName)) {
      Alert.alert('重新翻译', '当前字幕已是翻译版本，重新翻译会覆盖旧的中文翻译。继续吗？', [
        { text: '取消', style: 'cancel' },
        { text: '重新翻译', onPress: () => void runTranslate() },
      ]);
      return;
    }
    await runTranslate();
  }, [selectedAudio, runTranslate]);

  const handleRenamePractice = useCallback(
    async (name: string) => {
      if (!selectedPractice || !name.trim() || name.trim() === selectedPractice.name) return;
      await renameListeningPractice(selectedPractice.id, name.trim());
      await refreshPractices();
      showToast('练习组已重命名');
    },
    [selectedPractice, refreshPractices, showToast],
  );

  const handleRenameAudio = useCallback(
    async (name: string) => {
      if (!selectedAudio || !name.trim() || name.trim() === selectedAudio.name) return;
      await renameListeningAudio(selectedAudio.id, name.trim());
      await refreshPractices();
      showToast('音频已重命名');
    },
    [selectedAudio, refreshPractices, showToast],
  );

  const handleDeletePractice = useCallback(async () => {
    if (!selectedPractice) return;
    await removeLibraryPractice(selectedPractice.id);
    setSelectedPracticeId(null);
    setSelectedAudioId(null);
    await refreshPractices();
    showToast('练习组已删除');
  }, [selectedPractice, refreshPractices, showToast]);

  const handleDeleteAudio = useCallback(async () => {
    if (!selectedAudio) return;
    await removeLibraryAudio(selectedAudio.id);
    setSelectedAudioId(null);
    await refreshPractices();
    showToast('音频已删除');
  }, [selectedAudio, refreshPractices, showToast]);

  // ── Batch practice management ───────────────────────────────────────────────

  const allBatchSelected = practices.length > 0 && batchSelected.length === practices.length;

  const toggleBatchMode = useCallback(() => {
    setBatchMode((v) => !v);
    setBatchSelected([]);
  }, []);

  const toggleBatchItem = useCallback((id: string) => {
    setBatchSelected((prev) => (prev.includes(id) ? prev.filter((x) => x !== id) : [...prev, id]));
  }, []);

  const toggleBatchAll = useCallback(() => {
    setBatchSelected(allBatchSelected ? [] : practices.map((p) => p.id));
  }, [allBatchSelected, practices]);

  const handleBatchDelete = useCallback(() => {
    if (batchSelected.length === 0) return;
    Alert.alert('删除练习组', `确认删除所选 ${batchSelected.length} 个练习组及其全部音频？`, [
      { text: '取消', style: 'cancel' },
      {
        text: '删除',
        style: 'destructive',
        onPress: async () => {
          try {
            if (selectedPracticeId && batchSelected.includes(selectedPracticeId)) {
              setSelectedPracticeId(null);
              setSelectedAudioId(null);
            }
            for (const id of batchSelected) await removeLibraryPractice(id);
            await refreshPractices();
            setBatchSelected([]);
            setBatchMode(false);
            showToast(`已删除 ${batchSelected.length} 个练习组`);
          } catch (e: any) {
            await refreshPractices();
            setError(`批量删除失败：${e?.message ?? e}`);
          }
        },
      },
    ]);
  }, [batchSelected, selectedPracticeId, refreshPractices, showToast]);

  // ── Batch audio management ──────────────────────────────────────────────

  const allBatchAudioSelected =
    !!selectedPractice &&
    selectedPractice.audios.length > 0 &&
    batchAudioSelected.length === selectedPractice.audios.length;

  const toggleBatchAudioMode = useCallback(() => {
    setBatchAudioMode((visible) => !visible);
    setBatchAudioSelected([]);
  }, []);

  const toggleBatchAudioItem = useCallback((id: string) => {
    setBatchAudioSelected((prev) => (prev.includes(id) ? prev.filter((item) => item !== id) : [...prev, id]));
  }, []);

  const toggleBatchAudioAll = useCallback(() => {
    const audios = selectedPractice?.audios ?? [];
    setBatchAudioSelected(allBatchAudioSelected ? [] : audios.map((audio) => audio.id));
  }, [allBatchAudioSelected, selectedPractice]);

  const handleBatchAudioDelete = useCallback(() => {
    if (batchAudioSelected.length === 0) return;
    Alert.alert('删除音频', `确认删除所选 ${batchAudioSelected.length} 个音频及其字幕文件？`, [
      { text: '取消', style: 'cancel' },
      {
        text: '删除',
        style: 'destructive',
        onPress: async () => {
          try {
            if (selectedAudioId && batchAudioSelected.includes(selectedAudioId)) {
              setSelectedAudioId(null);
            }
            for (const id of batchAudioSelected) await removeLibraryAudio(id);
            await refreshPractices();
            setBatchAudioSelected([]);
            setBatchAudioMode(false);
            showToast(`已删除 ${batchAudioSelected.length} 个音频`);
          } catch (e: any) {
            await refreshPractices();
            setError(`批量删除失败：${e?.message ?? e}`);
          }
        },
      },
    ]);
  }, [batchAudioSelected, selectedAudioId, refreshPractices, showToast]);

  // Concatenate the selected audios (in practice order) into one new audio so
  // short recordings can be studied as a single continuous item. Subtitles are
  // re-based on each segment's real duration and stored as one SRT file.
  const mergeSelectedAudios = useCallback(
    async (chosen: ListeningAudio[], name: string) => {
      if (!selectedPractice) return;
      setMergingAudios(true);
      setError(null);
      try {
        const created = await mergeLibraryAudio(selectedPractice.id, chosen, name);
        await refreshPractices();
        setBatchAudioSelected([]);
        setBatchAudioMode(false);
        setSelectedAudioId(created.id);
        showToast(`已合并为「${name}」`);
      } catch (e: any) {
        setError(`合并失败：${e?.message ?? e}`);
      } finally {
        setMergingAudios(false);
      }
    },
    [selectedPractice, refreshPractices, showToast],
  );

  const handleBatchAudioMerge = useCallback(() => {
    if (Platform.OS !== 'macos' && Platform.OS !== 'windows') {
      showToast('合并音频请在桌面端操作');
      return;
    }
    if (!selectedPractice || mergingAudios) return;
    // Practice order (导入顺序) — the order shown in the list is the order
    // that will play in the merged audio.
    const chosen = selectedPractice.audios.filter((audio) => batchAudioSelected.includes(audio.id));
    if (chosen.length < 2) {
      showToast('请至少选择两个音频');
      return;
    }
    const defaultName = `合并 · ${chosen[0].name} 等${chosen.length}篇`;
    void (async () => {
      const name = await showPrompt('请输入合并后的音频名称', defaultName);
      if (!name?.trim()) return;
      Alert.alert(
        '合并音频',
        `将按以下顺序拼接成一个音频（原音频保留）：\n${chosen
          .map((audio, index) => `${index + 1}. ${audio.name}`)
          .join('\n')}`,
        [
          { text: '取消', style: 'cancel' },
          { text: '开始合并', onPress: () => void mergeSelectedAudios(chosen, name.trim()) },
        ],
      );
    })();
  }, [selectedPractice, batchAudioSelected, mergingAudios, mergeSelectedAudios, showPrompt, showToast]);

  const showPracticeMenu = useCallback(() => {
    if (selectedPractice) setLibraryMenu('practice');
  }, [selectedPractice]);
  const showAudioMenu = useCallback(() => {
    if (selectedAudio) setLibraryMenu('audio');
  }, [selectedAudio]);

  return (
    <View style={styles.safeArea}>
      <StatusBar barStyle={scheme === 'dark' ? 'light-content' : 'dark-content'} />
      <View style={styles.screenBody}>
        <View style={styles.container}>
          {/* Management header stays above the management workspace. */}
          <View style={styles.header}>
            <View style={styles.headerCopy}>
              <Text style={styles.title}>音频管理</Text>
              <Text style={styles.subtitle}>
                {Platform.OS === 'android'
                  ? '浏览同步的练习组与音频，直接开始学习。'
                  : '整理练习组、导入音频，并集中处理字幕。'}
              </Text>
            </View>
          </View>

          {error && (
            <View style={styles.errorBanner}>
              <Text style={styles.errorText}>{error}</Text>
            </View>
          )}

          {
            <AudioLibraryManager
              subtitleKinds={subtitleKinds}
              practices={practices}
              practice={selectedPractice}
              audio={selectedAudio}
              busy={!!aiStatus || scanImporting}
              status={aiStatus}
              onSelectPractice={(practice) => {
                void selectLibraryItem(practice, practice.audios[0]);
              }}
              onSelectAudio={(audio) => {
                if (selectedPractice) void selectLibraryItem(selectedPractice, audio);
              }}
              onStart={
                selectedPractice && selectedAudio
                  ? () => onStartStudy(selectedPractice.id, selectedAudio.id)
                  : undefined
              }
              onCreate={handleCreatePractice}
              onAdd={handleAddAudio}
              onPracticeMenu={showPracticeMenu}
              onAudioMenu={showAudioMenu}
              onBatch={toggleBatchAudioMode}
              onGenerate={handleAiSubtitle}
              onView={() => {
                void openSubtitleViewer();
              }}
              onTranslate={handleTranslateSubtitle}
              onUploadSubtitle={() => void handleUploadSubtitle()}
              advanced={
                <View style={{ gap: 10 }}>
                  {!!activeAsrModel && (
                    <Text style={styles.toolRowCaption}>当前模型：{activeAsrModel.split(/[\\/]/).pop()}</Text>
                  )}
                  {Platform.OS === 'macos' && canUseCommandImport && (
                    <Pressable
                      accessibilityRole="button"
                      disabled={!selectedPractice || scanImporting || !!aiStatus}
                      style={[
                        styles.toolRow,
                        (!selectedPractice || scanImporting || !!aiStatus) && styles.toolBtnDisabled,
                      ]}
                      onPress={handleScanImport}>
                      <Text style={styles.toolRowTitle}>{scanImporting ? '导入中…' : '从命令导入音频'}</Text>
                      <Text style={styles.toolRowCaption}>导入到当前练习组</Text>
                    </Pressable>
                  )}
                  {
                    <Pressable
                      accessibilityRole="button"
                      accessibilityState={{ expanded: showEnginePicker }}
                      disabled={!!aiStatus || asrEnginesLoading}
                      style={[styles.toolRow, (!!aiStatus || asrEnginesLoading) && styles.toolBtnDisabled]}
                      onPress={() => {
                        setShowEnginePicker((value) => !value);
                        if (!showEnginePicker) void loadAsrEngines();
                      }}>
                      <Text style={styles.toolRowTitle}>转写引擎</Text>
                      <Text style={styles.toolRowCaption}>
                        {asrEnginesLoading ? '读取中…' : (ASR_ENGINE_LABELS[activeAsrBackend ?? ''] ?? '选择引擎')}
                      </Text>
                    </Pressable>
                  }
                  {/* 转写 API 状态：模型与地址在设置中配置。 */}
                  {showEnginePicker && (
                    <View style={styles.dropdown}>
                      {asrEngines.map((engine) => (
                        <Pressable
                          key={engine.backend}
                          style={[
                            styles.dropdownItem,
                            engine.backend === activeAsrBackend && styles.dropdownItemActive,
                            !engine.available && styles.toolBtnDisabled,
                          ]}
                          onPress={() => {
                            void selectAsrEngine(engine);
                            setShowEnginePicker(false);
                          }}>
                          <Text
                            style={[
                              styles.dropdownItemText,
                              engine.backend === activeAsrBackend && styles.dropdownItemTextActive,
                            ]}>
                            {engine.label || engine.backend}
                            {engine.backend === activeAsrBackend ? ' ✓' : ''}
                          </Text>
                          <Text style={styles.dropdownEngineMeta}>
                            {engine.available
                              ? `${engine.model}（${engine.device}）`
                              : engine.reason || '当前环境不可用'}
                          </Text>
                        </Pressable>
                      ))}
                      {asrEngines.length === 0 && (
                        <Text style={styles.dropdownEmpty}>{asrEnginesLoading ? '读取中…' : '暂无可用的转写引擎'}</Text>
                      )}
                    </View>
                  )}
                </View>
              }
            />
          }

          {/* 字幕查看器：生成/导入的结果就地翻看，点空白处关闭。 */}
          {subtitleViewerVisible && (
            <Pressable style={styles.subtitleViewerScrim} onPress={() => setSubtitleViewerVisible(false)}>
              <Pressable style={styles.subtitleViewerPanel} onPress={() => {}}>
                <View style={styles.subtitleViewerHeader}>
                  <Text style={styles.subtitleViewerTitle} numberOfLines={1}>
                    字幕内容{selectedAudio ? ` · ${selectedAudio.name}` : ''}
                  </Text>
                  <View style={styles.subtitleViewerActions}>
                    <Pressable style={styles.subtitleViewerBtn} onPress={copySubtitleViewerText}>
                      <Text style={styles.subtitleViewerBtnText}>复制</Text>
                    </Pressable>
                    <Pressable style={styles.subtitleViewerBtn} onPress={() => setSubtitleViewerVisible(false)}>
                      <Text style={styles.subtitleViewerBtnText}>关闭</Text>
                    </Pressable>
                  </View>
                </View>
                {subtitleViewerLoading ? (
                  <Text style={styles.subtitleViewerLoading}>加载中…</Text>
                ) : (
                  <ScrollView style={styles.subtitleViewerScroll} nestedScrollEnabled>
                    {subtitleViewerCues.map((cue, index) => (
                      <View key={`${index}-${cue.start}`} style={styles.subtitleViewerCue}>
                        <Text style={styles.subtitleViewerTime}>
                          {formatTime(cue.start)} → {formatTime(cue.end)}
                        </Text>
                        <Text style={styles.subtitleViewerText}>{cue.text}</Text>
                      </View>
                    ))}
                    {subtitleViewerCues.length === 0 && <Text style={styles.subtitleViewerLoading}>字幕内容为空</Text>}
                  </ScrollView>
                )}
              </Pressable>
            </Pressable>
          )}

          {/* Batch management overlay */}
          {batchMode && (
            <View style={styles.batchOverlay}>
              <View style={styles.batchBox}>
                <Text style={styles.batchTitle}>批量管理练习组</Text>
                <Text style={styles.batchSubtitle}>
                  共 {practices.length} 组 · 已选 {batchSelected.length} 组
                </Text>
                <ScrollView style={styles.batchList}>
                  {practices.map((p) => {
                    const checked = batchSelected.includes(p.id);
                    return (
                      <Pressable key={p.id} style={styles.batchItem} onPress={() => toggleBatchItem(p.id)}>
                        <View style={[styles.batchCheckbox, checked && styles.batchCheckboxChecked]}>
                          {checked ? <Text style={styles.batchCheckMark}>✓</Text> : null}
                        </View>
                        <Text style={styles.batchItemName} numberOfLines={1}>
                          {p.name}
                        </Text>
                        <Text style={styles.batchItemCount}>{p.audios.length} 音频</Text>
                      </Pressable>
                    );
                  })}
                  {practices.length === 0 && <Text style={styles.dropdownEmpty}>暂无练习组</Text>}
                </ScrollView>
                <View style={styles.batchActions}>
                  <Pressable style={styles.batchBtnGhost} onPress={toggleBatchAll}>
                    <Text style={styles.batchBtnGhostText}>{allBatchSelected ? '全不选' : '全选'}</Text>
                  </Pressable>
                  <Pressable
                    style={[styles.batchBtnDanger, batchSelected.length === 0 && styles.batchBtnDisabled]}
                    onPress={handleBatchDelete}
                    disabled={batchSelected.length === 0}>
                    <Text style={styles.batchBtnDangerText}>删除所选</Text>
                  </Pressable>
                  <Pressable style={styles.batchBtnPrimary} onPress={toggleBatchMode}>
                    <Text style={styles.batchBtnPrimaryText}>完成</Text>
                  </Pressable>
                </View>
              </View>
            </View>
          )}

          {/* Batch audio management overlay */}
          {batchAudioMode && selectedPractice && (
            <View style={styles.batchOverlay}>
              <View style={styles.batchBox}>
                <Text style={styles.batchTitle}>批量管理音频</Text>
                <Text style={styles.batchSubtitle}>
                  「{selectedPractice.name}」共 {selectedPractice.audios.length} 个 · 已选 {batchAudioSelected.length}{' '}
                  个
                </Text>
                <ScrollView style={styles.batchList}>
                  {selectedPractice.audios.map((audio) => {
                    const checked = batchAudioSelected.includes(audio.id);
                    return (
                      <Pressable key={audio.id} style={styles.batchItem} onPress={() => toggleBatchAudioItem(audio.id)}>
                        <View style={[styles.batchCheckbox, checked && styles.batchCheckboxChecked]}>
                          {checked ? <Text style={styles.batchCheckMark}>✓</Text> : null}
                        </View>
                        <Text style={styles.batchItemName} numberOfLines={1}>
                          {audio.name}
                        </Text>
                        <Text style={styles.batchItemCount}>{audio.subtitle_uri ? '有字幕' : '无字幕'}</Text>
                      </Pressable>
                    );
                  })}
                  {selectedPractice.audios.length === 0 && <Text style={styles.dropdownEmpty}>该组暂无音频</Text>}
                </ScrollView>
                <View style={styles.batchActions}>
                  <Pressable style={styles.batchBtnGhost} onPress={toggleBatchAudioAll}>
                    <Text style={styles.batchBtnGhostText}>{allBatchAudioSelected ? '全不选' : '全选'}</Text>
                  </Pressable>
                  {Platform.OS === 'macos' && (
                    <Pressable
                      style={[
                        styles.batchBtnPrimary,
                        (batchAudioSelected.length < 2 || mergingAudios) && styles.batchBtnDisabled,
                      ]}
                      onPress={handleBatchAudioMerge}
                      disabled={batchAudioSelected.length < 2 || mergingAudios}>
                      <Text style={styles.batchBtnPrimaryText}>{mergingAudios ? '合并中…' : '合并所选'}</Text>
                    </Pressable>
                  )}
                  <Pressable
                    style={[styles.batchBtnDanger, batchAudioSelected.length === 0 && styles.batchBtnDisabled]}
                    onPress={handleBatchAudioDelete}
                    disabled={batchAudioSelected.length === 0}>
                    <Text style={styles.batchBtnDangerText}>删除所选</Text>
                  </Pressable>
                  <Pressable style={styles.batchBtnPrimary} onPress={toggleBatchAudioMode}>
                    <Text style={styles.batchBtnPrimaryText}>完成</Text>
                  </Pressable>
                </View>
              </View>
            </View>
          )}

          {/* Scan import confirmation */}
          {scanPreviewVisible && (
            <View style={styles.batchOverlay}>
              <View style={styles.batchBox}>
                <Text style={styles.batchTitle}>确认导入音频</Text>
                <Text style={styles.batchSubtitle}>
                  命令返回 {scanFiles.length} 个音频 · 导入到「{selectedPractice?.name}」
                </Text>
                <ScrollView style={styles.batchList}>
                  {scanFiles.map((file, idx) => (
                    <View key={file.path} style={styles.scanFileItem}>
                      <Text style={styles.scanFileLabel}>音频名称</Text>
                      <TextInput
                        style={styles.scanNameInput}
                        value={file.name}
                        onChangeText={(text) =>
                          setScanFiles((prev) => prev.map((f, i) => (i === idx ? { ...f, name: text } : f)))
                        }
                        placeholder="音频名称"
                        placeholderTextColor={theme.textMuted}
                      />
                      <Text style={styles.scanFilePath} numberOfLines={2} ellipsizeMode="middle">
                        {file.path}
                      </Text>
                    </View>
                  ))}
                </ScrollView>
                <View style={styles.batchActions}>
                  <Pressable style={[styles.batchBtnPrimary, { flex: 1 }]} onPress={confirmScanImport}>
                    <Text style={styles.batchBtnPrimaryText}>确认导入</Text>
                  </Pressable>
                  <Pressable style={styles.batchBtnGhost} onPress={() => setScanPreviewVisible(false)}>
                    <Text style={styles.batchBtnGhostText}>取消</Text>
                  </Pressable>
                </View>
              </View>
            </View>
          )}

          {libraryMenu && (
            <LibraryActionMenu
              key={libraryMenu}
              kind={libraryMenu}
              name={(libraryMenu === 'audio' ? selectedAudio?.name : selectedPractice?.name) || ''}
              onClose={() => setLibraryMenu(null)}
              onRename={libraryMenu === 'audio' ? handleRenameAudio : handleRenamePractice}
              onDelete={libraryMenu === 'audio' ? handleDeleteAudio : handleDeletePractice}
              onBatch={libraryMenu === 'audio' ? toggleBatchAudioMode : toggleBatchMode}
            />
          )}

          {/* Prompt */}
          {promptVisible && (
            <View style={styles.modalOverlay}>
              <Pressable style={styles.modalDismissArea} onPress={handlePromptCancel} />
              <View style={styles.modalBox}>
                <Text style={styles.modalTitle}>{promptTitle}</Text>
                <TextInput
                  style={styles.modalInput}
                  value={promptValue}
                  onChangeText={setPromptValue}
                  onSubmitEditing={handlePromptConfirm}
                  placeholderTextColor={theme.textSecondary}
                />
                <View style={styles.modalActions}>
                  <Pressable style={styles.modalBtnPrimary} onPress={handlePromptConfirm}>
                    <Text style={styles.modalBtnPrimaryText}>确定</Text>
                  </Pressable>
                  <Pressable style={styles.modalBtnGhost} onPress={handlePromptCancel}>
                    <Text style={styles.modalBtnGhostText}>取消</Text>
                  </Pressable>
                </View>
              </View>
            </View>
          )}

          {/* Toast */}
          {toast && (
            <View style={styles.toast}>
              <Text style={styles.toastText}>{toast}</Text>
            </View>
          )}
        </View>
      </View>
    </View>
  );
}
