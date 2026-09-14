import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { NativeModules, Platform } from "react-native";
import { getSetting, setSetting, recordPomodoroRound } from "../data/database";
import { nativeLearningAudio, nativeRecordingAudio } from "../services/platform";

export type PomodoroPhase = "focus" | "break";

// 阶段切换提示音：Mac 用系统音效，Android 用 ToneGenerator。
function playPhaseChime() {
  const module = nativeRecordingAudio() as { playChime?: () => void } | undefined;
  module?.playChime?.();
}

// 停止共享播放器（RNMacAudio/RNAndroidAudio 的 player 全 App 单例），
// 兜底确保休息音乐不会残留到专注阶段。
function stopSharedPlayer() {
  const module = nativeLearningAudio() as { unload?: () => unknown } | undefined;
  try {
    // Promise 型原生方法:无参调用,桥自动注入回调;拒绝静默吞掉。
    const result = module?.unload?.() as unknown;
    if (result && typeof (result as Promise<void>).catch === "function") {
      (result as Promise<void>).catch(() => {});
    }
  } catch {}
}

// 休息音乐独立播放器:Android 复用 RNAndroidAudio(方法名带 Break 后缀避免和学习音频撞名),
// macOS 是专用的 RNBreakAudio 模块(方法名没有后缀)。这里统一成一套接口,调用侧不用关心平台差异。
const breakPlayer = {
  play(uri: string) {
    if (Platform.OS !== "macos") nativeLearningAudio()?.playBreak?.(uri);
    else NativeModules.RNBreakAudio?.play?.(uri);
  },
  pause() {
    if (Platform.OS !== "macos") nativeLearningAudio()?.pauseBreak?.();
    else NativeModules.RNBreakAudio?.pause?.();
  },
  resume() {
    if (Platform.OS !== "macos") nativeLearningAudio()?.resumeBreak?.();
    else NativeModules.RNBreakAudio?.resume?.();
  },
  stop() {
    if (Platform.OS !== "macos") nativeLearningAudio()?.stopBreak?.();
    else NativeModules.RNBreakAudio?.stop?.();
  },
};

const PomodoroContext = createContext<PomodoroContextValue | null>(null);

type PomodoroContextValue = {
  focusMin: number;
  breakMin: number;
  phase: PomodoroPhase;
  running: boolean;
  paused: boolean;
  remainingSec: number;
  totalSec: number;
  completedFocusRounds: number;
  message: string;
  setFocusMin: (minutes: number) => void;
  setBreakMin: (minutes: number) => void;
  start: (overrideMinutes?: number) => void;
  togglePause: () => void;
  reset: () => void;
};

export function PomodoroProvider({ children }: { children: ReactNode }) {
  const [focusMin, setFocusMinState] = useState(25);
  const [breakMin, setBreakMinState] = useState(5);
  const [phase, setPhase] = useState<PomodoroPhase>("focus");
  const [running, setRunning] = useState(false);
  const [paused, setPaused] = useState(false);
  const [remainingSec, setRemainingSec] = useState(25 * 60);
  const [totalSec, setTotalSec] = useState(25 * 60);
  const [completedFocusRounds, setCompletedFocusRounds] = useState(0);
  const [message, setMessage] = useState("准备开始一段专注学习。");

  // 计时引擎的权威状态全部走 ref:唯一的 interval 挂载后永不重注册,
  // tick 只读 ref,不存在闭包过期或 effect 重注册竞态。
  const phaseRef = useRef<PomodoroPhase>("focus");
  const phaseEndsAtRef = useRef(0);
  const runningRef = useRef(false);
  const pausedRef = useRef(false);
  const remainingSecRef = useRef(25 * 60);
  const durationsRef = useRef({ focus: 25, break: 5 });
  durationsRef.current = { focus: focusMin, break: breakMin };

  useEffect(() => {
    Promise.all([getSetting("pomodoro_focus_min"), getSetting("pomodoro_break_min")])
      .then(([savedFocus, savedBreak]) => {
        const restoredFocus = savedFocus ? Number.parseInt(savedFocus, 10) : 25;
        const restoredBreak = savedBreak ? Number.parseInt(savedBreak, 10) : 5;
        if (restoredFocus > 0 && restoredFocus <= 180) {
          setFocusMinState(restoredFocus);
          durationsRef.current = { ...durationsRef.current, focus: restoredFocus };
          if (!runningRef.current) setRemainingSec(restoredFocus * 60);
        }
        if (restoredBreak > 0 && restoredBreak <= 60) {
          setBreakMinState(restoredBreak);
          durationsRef.current = { ...durationsRef.current, break: restoredBreak };
        }
      })
      .catch(() => {
        // Keep defaults when local storage is unavailable.
      });
  }, []);

  // 休息开始:暂停学习中的产品音频(保住进度),再播放休息音乐
  // (走独立播放器,不占用学习音频的共享 player)。
  const startBreakMusic = useCallback(async () => {
    try {
      const uri = await getSetting("meditation_music_uri");
      if (!uri) return;
      const shared = nativeLearningAudio() as { pause?: () => unknown } | undefined;
      const paused = shared?.pause?.() as unknown;
      if (paused && typeof (paused as Promise<void>).catch === "function") {
        (paused as Promise<void>).catch(() => {});
      }
      breakPlayer.play(uri);
    } catch {
      // 休息音乐播放失败不影响计时
    }
  }, []);

  const stopBreakMusic = useCallback(() => {
    breakPlayer.stop();
  }, []);

  // 进入一个阶段:同步写 ref + state,deadline 一步到位。
  const enterPhase = useCallback((next: PomodoroPhase) => {
    const minutes = next === "focus" ? durationsRef.current.focus : durationsRef.current.break;
    phaseRef.current = next;
    phaseEndsAtRef.current = Date.now() + minutes * 60 * 1000;
    remainingSecRef.current = minutes * 60;
    setPhase(next);
    setTotalSec(minutes * 60);
    setRemainingSec(minutes * 60);
  }, []);

  const tick = useCallback(() => {
    if (!runningRef.current || pausedRef.current) return;
    const remaining = Math.max(0, Math.ceil((phaseEndsAtRef.current - Date.now()) / 1000));
    if (remaining > 0) {
      remainingSecRef.current = remaining;
      setRemainingSec(remaining);
      return;
    }
    const wasFocus = phaseRef.current === "focus";
    if (wasFocus) {
      setCompletedFocusRounds((rounds) => rounds + 1);
      void recordPomodoroRound().catch(() => {});
      setMessage("本轮专注完成，休息一下，听听音乐。");
      playPhaseChime();
      void startBreakMusic();
    } else {
      setMessage("休息结束，开始下一轮专注。");
      playPhaseChime();
      void stopBreakMusic();
    }
    enterPhase(phaseRef.current === "focus" ? "break" : "focus");
  }, [startBreakMusic, stopBreakMusic]);

  // 唯一的 interval,挂载期注册一次,tick 只读 ref。
  useEffect(() => {
    const timer = setInterval(tick, 500);
    return () => clearInterval(timer);
  }, [tick]);

  const setFocusMin = useCallback((minutes: number) => {
    durationsRef.current = { ...durationsRef.current, focus: minutes };
    setFocusMinState(minutes);
    if (phaseRef.current === "focus" && !runningRef.current) {
      const total = minutes * 60;
      phaseEndsAtRef.current = Date.now() + total * 1000;
      remainingSecRef.current = total;
      setTotalSec(total);
      setRemainingSec(total);
    }
    void setSetting("pomodoro_focus_min", String(minutes)).catch(() => {});
  }, []);

  const setBreakMin = useCallback((minutes: number) => {
    durationsRef.current = { ...durationsRef.current, break: minutes };
    setBreakMinState(minutes);
    void setSetting("pomodoro_break_min", String(minutes)).catch(() => {});
  }, []);

  const start = useCallback((overrideMinutes?: number) => {
    // 自定义时长直接传入:state 更新是异步的,同一次点击里闭包中的
    // focusMin 还是旧值,必须用参数覆盖。
    const minutes =
      overrideMinutes && overrideMinutes > 0 && overrideMinutes <= 180
        ? overrideMinutes
        : durationsRef.current.focus;
    durationsRef.current = { ...durationsRef.current, focus: minutes };
    setFocusMinState(minutes);
    void setSetting("pomodoro_focus_min", String(minutes)).catch(() => {});
    stopSharedPlayer();
    runningRef.current = true;
    pausedRef.current = false;
    setRunning(true);
    setPaused(false);
    enterPhase("focus");
    setMessage("保持专注，只做当前这一件事。");
  }, [enterPhase]);

  // 休息音乐:暂停/继续(独立播放器)。
  const pauseBreakMusic = useCallback(() => {
    breakPlayer.pause();
  }, []);

  const resumeBreakMusic = useCallback(() => {
    breakPlayer.resume();
  }, []);

  const togglePause = useCallback(() => {
    if (!runningRef.current) return;
    if (pausedRef.current) {
      phaseEndsAtRef.current = Date.now() + remainingSecRef.current * 1000;
      pausedRef.current = false;
      setPaused(false);
      resumeBreakMusic();
      return;
    }
    pausedRef.current = true;
    setPaused(true);
    pauseBreakMusic();
  }, [pauseBreakMusic]);

  const reset = useCallback(() => {
    runningRef.current = false;
    pausedRef.current = false;
    setRunning(false);
    setPaused(false);
    void stopBreakMusic();
    enterPhase("focus");
    setMessage("准备开始一段专注学习。");
  }, [enterPhase]);

  return (
    <PomodoroContext.Provider value={{
      focusMin, breakMin, phase, running, paused, remainingSec, totalSec,
      completedFocusRounds, message, setFocusMin, setBreakMin, start, togglePause, reset,
    }}>
      {children}
    </PomodoroContext.Provider>
  );
}

export function usePomodoro(): PomodoroContextValue {
  const value = useContext(PomodoroContext);
  if (!value) throw new Error("usePomodoro must be used within PomodoroProvider");
  return value;
}

export function formatPomodoroClock(totalSeconds: number): string {
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes.toString().padStart(2, "0")}:${seconds.toString().padStart(2, "0")}`;
}
