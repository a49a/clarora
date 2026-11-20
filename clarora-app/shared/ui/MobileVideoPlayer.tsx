import { forwardRef, useImperativeHandle, useRef } from "react";
import { Platform, type StyleProp, type ViewStyle } from "react-native";

// Android/iOS playback for video learning. The macOS and Windows RN builds
// keep their libmpv view (see services/nativeVideo.ts), so react-native-video
// is required lazily and never initializes outside phones.
const NativeVideo: any =
  Platform.OS === "android" || Platform.OS === "ios"
    // eslint-disable-next-line @typescript-eslint/no-require-imports -- 只在手机端加载原生播放器，桌面 RN 继续用 libmpv
    ? require("react-native-video").default
    : null;

export type MobileVideoHandle = {
  seek: (seconds: number) => void;
};

type MobileVideoPlayerProps = {
  uri: string;
  playing: boolean;
  rate: number;
  style?: StyleProp<ViewStyle>;
  onReady?: (durationMs: number, positionMs: number) => void;
  onProgress?: (positionMs: number, durationMs: number) => void;
  onEnd?: () => void;
  onError?: (message: string) => void;
};

type VideoPlayerRef = { seek: (seconds: number) => void };

export const MobileVideoPlayer = forwardRef<MobileVideoHandle, MobileVideoPlayerProps>(
  function MobileVideoPlayer({ uri, playing, rate, style, onReady, onProgress, onEnd, onError }, ref) {
    const player = useRef<VideoPlayerRef>(null);
    const durationMs = useRef(0);
    useImperativeHandle(ref, () => ({
      seek: (seconds: number) => {
        player.current?.seek(Math.max(0, seconds));
      },
    }), []);
    if (!NativeVideo) return null;
    return (
      <NativeVideo
        ref={player}
        source={{ uri }}
        style={style}
        paused={!playing}
        rate={rate}
        resizeMode="contain"
        progressUpdateInterval={250}
        onLoad={(data: { duration: number; currentTime: number }) => {
          durationMs.current = data.duration * 1000;
          onReady?.(durationMs.current, data.currentTime * 1000);
        }}
        onProgress={(data: { currentTime: number }) => {
          onProgress?.(data.currentTime * 1000, durationMs.current);
        }}
        onEnd={onEnd}
        onError={(event: any) => {
          const detail = event?.error;
          onError?.(
            detail?.errorString || detail?.localizedDescription || detail?.error
              || "视频无法播放，请确认文件格式。"
          );
        }}
      />
    );
  }
);
