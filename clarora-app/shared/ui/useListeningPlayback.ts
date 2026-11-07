import { useCallback, useEffect, useRef, useState } from 'react';
import { recordListenSeconds, type ListeningAudio } from '../data/database';
import { ListeningPlayback, type ListeningRepeatMode } from '../services/listeningPlayback';
import { Audio, type AVPlaybackStatus } from '../services/platform';
import type { BlindListeningMachine } from './blindListening';

export function useListeningPlayback(
  blindMachine: BlindListeningMachine,
  setBlindEnded: (ended: boolean) => void,
  setError: (message: string | null) => void,
) {
  const [playback] = useState(
    () =>
      new ListeningPlayback(async (uri, rate, onStatus) => {
        const { sound } = await Audio.Sound.createAsync({ uri }, { rate, progressUpdateIntervalMillis: 100 }, onStatus);
        return sound;
      }),
  );
  const enqueueAudioOperation = useCallback(<T>(run: () => Promise<T>) => playback.enqueue(run), [playback]);
  // 听力时长统计：播放中按轮询间隔累计真实秒数，攒够 5 秒落一次库。
  const listenAccumRef = useRef(0);
  const listenLastTickRef = useRef<number | null>(null);
  const mountedRef = useRef(true);
  const loadedAudioIdRef = useRef<string | null>(null);
  const loadingAudioIdRef = useRef<string | null>(null);
  const [readyAudioId, setReadyAudioId] = useState<string | null>(null);
  // React may run the selected-audio effect while a group transition is still
  // loading the same item. Keep an auto-play request through that overlap so
  // the effect's ordinary load cannot turn a group transition into a pause.
  const pendingAutoPlayAudioIdRef = useRef<string | null>(null);
  const [isPlaying, setIsPlaying] = useState(false);
  const [positionMs, setPositionMs] = useState(0);
  const [durationMs, setDurationMs] = useState(0);
  const [playbackRate, setPlaybackRate] = useState(1);
  const [pendingRate, setPendingRate] = useState<number | null>(null);
  const positionMsRef = useRef(0);
  const isPlayingRef = useRef(false);

  // Repeat playback: off → single audio → whole practice group.
  const [repeatMode, setRepeatMode] = useState<ListeningRepeatMode>('off');
  const repeatModeRef = useRef(repeatMode);
  const repeatAdvanceRef = useRef<(() => Promise<void>) | null>(null);
  const repeatAdvancingRef = useRef(false);
  const lastRepeatAtRef = useRef(0);

  // 最新期望倍速:加载新音频与异步改速都从这里读取,避免闭包拿到旧值。
  const desiredRateRef = useRef(1);
  const rateApplyingRef = useRef(false);
  const [rateApplying, setRateApplying] = useState(false);

  const onPlaybackStatusUpdate = useCallback(
    async (status: AVPlaybackStatus) => {
      if (!status.isLoaded) return;
      // 听力时长：只在播放中累计，暂停/结束不计；跳变 >5s 视为 seek 丢弃。
      if (status.isPlaying) {
        const now = Date.now();
        const last = listenLastTickRef.current;
        if (last != null) {
          const delta = (now - last) / 1000;
          if (delta > 0 && delta < 5) listenAccumRef.current += delta;
        }
        listenLastTickRef.current = now;
        if (listenAccumRef.current >= 5) {
          const flushed = listenAccumRef.current;
          listenAccumRef.current = 0;
          void recordListenSeconds(flushed).catch(() => {});
        }
      } else {
        listenLastTickRef.current = null;
      }
      positionMsRef.current = status.positionMillis;
      setPositionMs(status.positionMillis);
      setDurationMs(status.durationMillis ?? 0);
      isPlayingRef.current = !!status.isPlaying;
      setIsPlaying(status.isPlaying);

      // Repeat handling. The status callback is registered once per loaded
      // sound, so the current repeat mode and queue step are read through refs.
      if (status.didJustFinish) {
        // 盲听到末尾:保持隐藏,由面板提供「从头再听/查看全文」。
        if (blindMachine.isBlind()) setBlindEnded(true);
        const now = Date.now();
        if (repeatAdvancingRef.current || now - lastRepeatAtRef.current < 800) return;
        lastRepeatAtRef.current = now;
        repeatAdvancingRef.current = true;
        if (repeatModeRef.current === 'one') {
          const sound = playback.sound;
          if (sound) {
            try {
              const generation = playback.generation;
              await playback.enqueue(async () => {
                if (generation !== playback.generation || playback.sound !== sound) return;
                await sound.setPositionAsync(0);
                if (generation === playback.generation) await sound.playAsync();
              });
            } finally {
              repeatAdvancingRef.current = false;
            }
          } else {
            repeatAdvancingRef.current = false;
          }
        } else if (repeatModeRef.current === 'practice' || repeatModeRef.current === 'pass') {
          try {
            await repeatAdvanceRef.current?.();
          } finally {
            repeatAdvancingRef.current = false;
          }
        } else {
          repeatAdvancingRef.current = false;
        }
      }
    },
    [blindMachine, setBlindEnded, playback],
  );

  const resetPlayback = useCallback(() => {
    loadedAudioIdRef.current = null;
    isPlayingRef.current = false;
    listenLastTickRef.current = null;
    positionMsRef.current = 0;
    if (mountedRef.current) {
      setIsPlaying(false);
      setPositionMs(0);
      setDurationMs(0);
    }
  }, []);

  const loadAudio = useCallback(
    async (audio: ListeningAudio) => {
      const expectedGeneration = playback.generation + 1;
      resetPlayback();
      setPendingRate(desiredRateRef.current);
      try {
        const generation = await playback.load(audio.audio_uri, desiredRateRef.current, onPlaybackStatusUpdate);
        if (generation == null || generation !== playback.generation || !mountedRef.current) return null;
        await playback.enqueue(async () => {
          if (generation === playback.generation) await playback.sound?.setRateAsync(desiredRateRef.current, true);
        });
        if (generation !== playback.generation || !mountedRef.current) return null;
        setPlaybackRate(desiredRateRef.current);
        return generation;
      } catch (error) {
        if (mountedRef.current && expectedGeneration === playback.generation) {
          await playback.discard(expectedGeneration).catch(() => {});
          if (!mountedRef.current || expectedGeneration !== playback.generation) return null;
          setError(`加载音频失败：${error instanceof Error ? error.message : String(error)}`);
        }
        return null;
      } finally {
        if (mountedRef.current && expectedGeneration === playback.generation) setPendingRate(null);
      }
    },
    [playback, resetPlayback, onPlaybackStatusUpdate, setError],
  );

  const prepareAudio = useCallback(
    async (
      audio: ListeningAudio,
      autoPlay: boolean,
      loadSubtitle: (audio: ListeningAudio, generation: number) => Promise<void>,
      clearSubtitle: () => void,
    ) => {
      if (loadedAudioIdRef.current === audio.id && loadingAudioIdRef.current == null) {
        if (autoPlay) {
          const generation = playback.generation;
          await enqueueAudioOperation(async () => {
            if (generation === playback.generation) await playback.sound?.playAsync();
          });
        }
        return;
      }
      // 兜底清理(selectAudioItem 已清理;直接调用本函数的路径也覆盖)。
      blindMachine.invalidate();
      setBlindEnded(false);
      if (loadingAudioIdRef.current === audio.id) {
        if (autoPlay) pendingAutoPlayAudioIdRef.current = audio.id;
        return;
      }
      loadingAudioIdRef.current = audio.id;
      pendingAutoPlayAudioIdRef.current = null;
      setReadyAudioId(null);
      clearSubtitle();
      const expectedGeneration = playback.generation + 1;
      try {
        const generation = await loadAudio(audio);
        if (generation == null) return;
        await loadSubtitle(audio, generation);
        if (generation !== playback.generation) return;
        loadedAudioIdRef.current = audio.id;
        if (autoPlay || pendingAutoPlayAudioIdRef.current === audio.id) {
          await enqueueAudioOperation(async () => {
            if (generation === playback.generation) await playback.sound?.playAsync();
          });
        }
      } finally {
        if (playback.generation === expectedGeneration && loadingAudioIdRef.current === audio.id) {
          pendingAutoPlayAudioIdRef.current = null;
          loadingAudioIdRef.current = null;
          if (mountedRef.current) setReadyAudioId(loadedAudioIdRef.current === audio.id ? audio.id : null);
        }
      }
    },
    [enqueueAudioOperation, loadAudio, playback, blindMachine, setBlindEnded],
  );

  const clearReadyAudio = useCallback(() => {
    loadingAudioIdRef.current = null;
    pendingAutoPlayAudioIdRef.current = null;
    setReadyAudioId(null);
  }, []);

  const flushListeningTime = useCallback(async () => {
    if (listenAccumRef.current <= 0) return;
    const seconds = listenAccumRef.current;
    listenAccumRef.current = 0;
    await recordListenSeconds(seconds).catch(() => {});
  }, []);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      void playback.unload().catch(() => {});
      void flushListeningTime();
    };
  }, [playback, flushListeningTime]);
  useEffect(() => {
    repeatModeRef.current = repeatMode;
  }, [repeatMode]);
  const cycleRepeatMode = useCallback(() => {
    setRepeatMode((current) =>
      current === 'off' ? 'one' : current === 'one' ? 'practice' : current === 'practice' ? 'pass' : 'off',
    );
  }, []);

  const changeRate = useCallback(
    async (rate: number, hasSelection: boolean) => {
      if (rateApplyingRef.current) return; // 快速连点:忽略重复提交
      rateApplyingRef.current = true;
      setRateApplying(true);
      try {
        await enqueueAudioOperation(async () => {
          const sound = playback.sound;
          if (!sound) {
            if (!hasSelection) throw new Error('请先选择音频');
            desiredRateRef.current = rate;
            if (mountedRef.current) {
              setPendingRate(rate);
            }
            return;
          }
          await sound.setRateAsync(rate, true);
          // 只在当前播放器确认成功后提交期望速度和显示档位。
          desiredRateRef.current = rate;
          if (mountedRef.current) {
            setPlaybackRate(rate);
            setPendingRate(null);
          }
        });
        return mountedRef.current;
      } catch (error: any) {
        // 失败保留旧档,提示原因并允许重试。
        if (mountedRef.current) setError(`设置播放速度失败：${error?.message ?? error}`);
      } finally {
        rateApplyingRef.current = false;
        if (mountedRef.current) setRateApplying(false);
      }
    },
    [enqueueAudioOperation, playback, setError],
  );

  return {
    playback,
    enqueueAudioOperation,
    mountedRef,
    loadedAudioIdRef,
    loadingAudioIdRef,
    readyAudioId,
    clearReadyAudio,
    isPlaying,
    positionMs,
    positionMsRef,
    durationMs,
    playbackRate,
    pendingRate,
    isPlayingRef,
    repeatMode,
    repeatModeRef,
    repeatAdvanceRef,
    cycleRepeatMode,
    rateApplying,
    changeRate,
    resetPlayback,
    prepareAudio,
    flushListeningTime,
  };
}
