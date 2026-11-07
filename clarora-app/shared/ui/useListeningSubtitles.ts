import { useCallback, useRef, useState } from 'react';
import { type ListeningAudio, updateAudioSubtitle } from '../data/database';
import { mergeSubtitleLanguage, serializeSubtitleCues, parseSubtitleCues, type SubtitleCue } from '../data/subtitles';
import { DocumentPicker, FileSystem } from '../services/platform';
import {
  transcribeAudio,
  waitForSubtitles,
  translateSubtitlesFile,
  waitForTranslation,
  deleteJob,
  getActiveAsrEngine,
} from '../services/ai';

export const ASR_ENGINE_LABELS: Record<string, string> = {
  local: '端侧转写',
  compatible: '自定义转写 API',
  qwen3_asr: 'Qwen3-ASR',
  moss: 'MOSS',
  sensevoice: 'SenseVoice',
  faster_whisper: 'Whisper',
};

export function getFileNameFromUri(uri: string): string {
  const parts = uri.split(/[\\/]/);
  const name = parts[parts.length - 1] || uri;
  return decodeURIComponent(name);
}

type SubtitleActions = {
  selectedAudio: ListeningAudio | null;
  refreshPractices: () => Promise<void>;
  showToast: (message: string) => void;
  setError: (message: string | null) => void;
  onSaved?: (audio: ListeningAudio) => Promise<void>;
  onPreview?: (cues: SubtitleCue[]) => void;
};

export function useListeningSubtitles({
  selectedAudio,
  refreshPractices,
  showToast,
  setError,
  onSaved,
  onPreview,
}: SubtitleActions) {
  const [aiStatus, setAiStatus] = useState<string | null>(null);
  const subtitleTaskRef = useRef(false);
  const handleUploadSubtitle = useCallback(
    async (language?: 'en' | 'zh') => {
      if (!selectedAudio || subtitleTaskRef.current) return;
      subtitleTaskRef.current = true;
      setError(null);
      setAiStatus('请选择字幕文件…');
      try {
        const result = await DocumentPicker.getDocumentAsync({ type: 'text/*', copyToCacheDirectory: true });
        if (result.canceled || !result.assets?.length) return;
        setAiStatus('正在导入字幕…');
        const content = await FileSystem.readAsStringAsync(result.assets[0].uri);
        const imported = parseSubtitleCues(content);
        if (!imported.length) throw new Error('没有找到带时间轴的字幕，请选择 SRT 或 VTT 文件。');
        const existing =
          language && selectedAudio.subtitle_uri
            ? parseSubtitleCues(await FileSystem.readAsStringAsync(selectedAudio.subtitle_uri))
            : [];
        const cues = language ? mergeSubtitleLanguage(existing, imported, language) : imported;
        const dir = (await FileSystem.getDocumentDirectoryAsync()) + 'listening/';
        await FileSystem.makeDirectoryAsync(dir);
        const uri = `${dir}${Date.now()}_import.srt`;
        await FileSystem.writeFileAsync(uri, serializeSubtitleCues(cues));
        await updateAudioSubtitle(selectedAudio.id, uri);
        await onSaved?.({ ...selectedAudio, subtitle_uri: uri });
        await refreshPractices();
        showToast(language ? `${language === 'zh' ? '中文' : '英文'}字幕已导入` : '字幕已更新');
      } catch (e: any) {
        setError(`添加字幕失败：${e?.message ?? e}`);
      } finally {
        subtitleTaskRef.current = false;
        setAiStatus(null);
      }
    },
    [selectedAudio, onSaved, refreshPractices, showToast, setError],
  );

  // Generate subtitles directly through the configured transcription API.
  const handleAiSubtitle = useCallback(async () => {
    if (!selectedAudio || subtitleTaskRef.current) return;
    subtitleTaskRef.current = true;
    setError(null);
    const jobIdRef: { current: string | null } = { current: null };
    try {
      const ext = (selectedAudio.audio_uri.split('.').pop() ?? 'mp3').toLowerCase();
      const fileName = `${selectedAudio.name}.${ext}`;
      setAiStatus('正在准备转写…');
      const { jobId } = await transcribeAudio(selectedAudio.audio_uri, fileName);
      jobIdRef.current = jobId;
      const engine = await getActiveAsrEngine();
      const engineLabel = engine ? `${ASR_ENGINE_LABELS[engine.backend] ?? engine.backend} ${engine.model}`.trim() : '';
      setAiStatus(engineLabel ? `正在转写（${engineLabel}）…` : '正在转写…');
      const srt = await waitForSubtitles(jobId, (progress) => {
        setAiStatus(`正在转写（${engineLabel}）… ${Math.round(progress * 100)}%`);
      });
      const dir = (await FileSystem.getDocumentDirectoryAsync()) + 'listening/';
      await FileSystem.makeDirectoryAsync(dir);
      const srtUri = `${dir}${Date.now()}_ai.srt`;
      await FileSystem.writeFileAsync(srtUri, srt);
      await updateAudioSubtitle(selectedAudio.id, srtUri);
      await onSaved?.({ ...selectedAudio, subtitle_uri: srtUri });
      await refreshPractices();
      setAiStatus(null);
      showToast('字幕已生成');
      // 生成完直接把结果摆到用户面前。
      onPreview?.(parseSubtitleCues(srt));
    } catch (e: any) {
      setAiStatus(null);
      setError(`AI 生成字幕失败：${e?.message ?? e}`);
    } finally {
      subtitleTaskRef.current = false;
      if (jobIdRef.current) {
        deleteJob(jobIdRef.current);
      }
    }
  }, [selectedAudio, onSaved, onPreview, refreshPractices, showToast, setError]);

  // Actually run the translate-then-swap flow.
  const runTranslate = useCallback(async () => {
    if (!selectedAudio || subtitleTaskRef.current) return;
    subtitleTaskRef.current = true;
    setError(null);
    const jobIdRef: { current: string | null } = { current: null };
    try {
      const fileName = getFileNameFromUri(selectedAudio.subtitle_uri ?? '');
      setAiStatus('正在上传字幕…');
      const { jobId } = await translateSubtitlesFile(selectedAudio.subtitle_uri, fileName, {
        lang: 'zh',
        mode: 'bilingual',
      });
      jobIdRef.current = jobId;
      const srt = await waitForTranslation(jobId, (message) => setAiStatus(message));
      const dir = (await FileSystem.getDocumentDirectoryAsync()) + 'listening/';
      await FileSystem.makeDirectoryAsync(dir);
      const srtUri = `${dir}${Date.now()}_zh.srt`;
      await FileSystem.writeFileAsync(srtUri, srt);
      await updateAudioSubtitle(selectedAudio.id, srtUri);
      await onSaved?.({ ...selectedAudio, subtitle_uri: srtUri });
      await refreshPractices();
      setAiStatus(null);
      showToast('已生成中英双语字幕');
      onPreview?.(parseSubtitleCues(srt));
    } catch (e: any) {
      setAiStatus(null);
      setError(`翻译字幕失败：${e?.message ?? e}`);
    } finally {
      subtitleTaskRef.current = false;
      if (jobIdRef.current) {
        deleteJob(jobIdRef.current);
      }
    }
  }, [selectedAudio, onSaved, onPreview, refreshPractices, showToast, setError]);

  // 直接翻看当前音频的字幕转录结果（生成/导入后均可查看）。
  return { aiStatus, subtitleTaskRef, handleUploadSubtitle, handleAiSubtitle, runTranslate };
}
