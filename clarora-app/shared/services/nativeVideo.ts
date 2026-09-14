import { NativeModules, Platform, requireNativeComponent, type NativeSyntheticEvent } from "react-native";

// Shared binding for the macOS libmpv view (RNMpvVideoPlayer.mm). Used by the
// video learning screen and by video flashcards on the review screen.

export type NativeVideoProps = {
  src?: string;
  playing?: boolean;
  rate?: number;
  style?: unknown;
  onProgress?: (e: NativeSyntheticEvent<{ positionMs: number; durationMs: number }>) => void;
  onEnd?: (e: NativeSyntheticEvent<Record<string, never>>) => void;
  onError?: (e: NativeSyntheticEvent<{ message: string }>) => void;
};

export const NativeVideoPlayer =
  Platform.OS === "macos" || Platform.OS === "windows" ? requireNativeComponent<NativeVideoProps>("RNVideoPlayerView") : null;

type VideoControlModule = {
  seek?: (tag: number, ms: number) => void;
  setVolume?: (tag: number, volume: number) => void;
  getSubtitleTracks?: (tag: number) => Promise<unknown>;
  selectSubtitle?: (tag: number, trackId: number) => void;
  selectSecondarySubtitle?: (tag: number, trackId: number) => void;
  exportClip?: (source: string, destination: string, startMs: number, endMs: number) => Promise<void>;
};

export function videoControl(): VideoControlModule | undefined {
  return NativeModules.RNVideoControl as VideoControlModule | undefined;
}
