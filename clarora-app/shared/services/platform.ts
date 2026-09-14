import { NativeModules, Platform } from "react-native";
import RNFS from "./rnfs";

const usesRNFS = Platform.OS === "android" || Platform.OS === "ios";
export const nativeLearningAudio = () => Platform.OS === "ios"
  ? NativeModules.RNIOSAudio
  : Platform.OS === "windows" ? NativeModules.RNWindowsAudio
  : Platform.OS === "android" ? NativeModules.RNAndroidAudio : NativeModules.RNMacAudio;
export const nativeRecordingAudio = () => Platform.OS === "macos"
  ? NativeModules.RNMacAudioRecorder : nativeLearningAudio();

type NativeFilePicker = {
  pickFile: (options: { type?: string }) => Promise<PickedFile | null>;
  pickDirectory: () => Promise<string | null>;
  listFiles: (dirPath: string) => Promise<string[]>;
  getDocumentDirectory: () => Promise<string>;
  makeDirectory: (path: string) => Promise<void>;
  copyFile: (source: string, destination: string) => Promise<void>;
  readFile: (path: string) => Promise<string>;
  readBase64: (path: string) => Promise<string>;
  writeFile: (path: string, contents: string) => Promise<void>;
  deleteFile: (path: string) => Promise<void>;
};

function getNativeFilePicker(): NativeFilePicker {
  const picker = (Platform.OS === "windows" ? NativeModules.RNWindowsFiles
    : Platform.OS === "ios" ? NativeModules.RNIOSFilePicker : NativeModules.RNFilePicker) as NativeFilePicker | undefined;
  if (!picker) {
    throw new Error(`${Platform.OS} 文件选择器尚未配置`);
  }
  return picker;
}

type NativeAndroidFilePicker = {
  pickFile: (type?: string) => Promise<PickedFile | null>;
  pickDirectory: () => Promise<string | null>;
  captureImage: () => Promise<PickedFile | null>;
};

function getAndroidFilePicker(): NativeAndroidFilePicker {
  const picker = NativeModules.RNAndroidFilePicker as NativeAndroidFilePicker | undefined;
  if (!picker) {
    throw new Error("Android 文件选择器尚未配置");
  }
  return picker;
}

export function nativePath(uri: string): string {
  if (!uri.startsWith("file://")) return uri;
  const path = decodeURIComponent(uri.slice(7));
  if (Platform.OS === "windows") {
    if (/^\/[a-z]:[\\/]/i.test(path)) return path.slice(1);
    if (!path.startsWith("/") && !/^[a-z]:/i.test(path)) return `//${path}`;
  }
  return path;
}

/** Copies text to the system clipboard. Returns false if unsupported on this device. */
export function copyToClipboard(text: string): boolean {
  const clipboard = (Platform.OS === "android"
    ? NativeModules.RNAndroidClipboard
    : Platform.OS === "windows" ? NativeModules.RNWindowsFiles
    : Platform.OS === "ios" ? NativeModules.RNIOSClipboard : NativeModules.RNClipboard) as { setText?: (text: string) => void } | undefined;
  if (!clipboard?.setText) return false;
  clipboard.setText(text);
  return true;
}

type PickedFile = { uri: string; name: string };
type PickResult = { canceled: boolean; assets?: PickedFile[] };

// The native picker is optional so the app can still launch before the macOS bridge is installed.
export const DocumentPicker = {
  async getDocumentAsync(options: { type?: string; copyToCacheDirectory?: boolean }): Promise<PickResult> {
    const file = Platform.OS === "android"
      ? await getAndroidFilePicker().pickFile(options.type)
      : await getNativeFilePicker().pickFile(options);
    return file ? { canceled: false, assets: [file] } : { canceled: true };
  },

  async getDirectoryAsync(): Promise<{ canceled: boolean; path?: string }> {
    if (Platform.OS === "ios") throw new Error("iOS 请逐个导入文件，或在设置中同步桌面端资料");
    const path = Platform.OS === "android"
      ? await getAndroidFilePicker().pickDirectory()
      : await getNativeFilePicker().pickDirectory();
    return path ? { canceled: false, path } : { canceled: true };
  },

  /** Android only: launch the camera app and save a full-size page photo. */
  async captureImageAsync(): Promise<PickedFile | null> {
    if (Platform.OS !== "android") {
      throw new Error("拍照仅支持 Android 端");
    }
    const file = await getAndroidFilePicker().captureImage();
    return file ? { uri: file.uri, name: file.name } : null;
  },
};

export const FileSystem = {
  async getDocumentDirectoryAsync(): Promise<string> {
    if (usesRNFS) return `${RNFS.DocumentDirectoryPath}/`;
    return `${await getNativeFilePicker().getDocumentDirectory()}/`;
  },
  async listFilesAsync(dirPath: string): Promise<string[]> {
    if (usesRNFS) {
      const entries = await RNFS.readDir(nativePath(dirPath));
      return entries.filter((entry) => entry.isFile()).map((entry) => entry.path);
    }
    return getNativeFilePicker().listFiles(dirPath);
  },
  async readAsStringAsync(uri: string): Promise<string> {
    if (usesRNFS) return RNFS.readFile(nativePath(uri), "utf8");
    return getNativeFilePicker().readFile(nativePath(uri));
  },
  async readBase64Async(uri: string): Promise<string> {
    if (usesRNFS) return RNFS.readFile(nativePath(uri), "base64");
    return getNativeFilePicker().readBase64(nativePath(uri));
  },
  async putFileAsync(url: string, uri: string, headers: Record<string, string>): Promise<{ status: number; body: string }> {
    const path = nativePath(uri);
    if (Platform.OS === "windows") return NativeModules.RNWindowsFiles.putFile(url, path, headers);
    if (usesRNFS) {
      const result = await RNFS.uploadFiles({ toUrl: url, method: "PUT", headers, binaryStreamOnly: true,
        files: [{ name: "file", filename: path.split("/").pop() || "file", filepath: path, filetype: "application/octet-stream" }] }).promise;
      return { status: result.statusCode, body: result.body };
    }
    const out = await runCommandArgs("/usr/bin/curl", ["-sS", "--max-time", "1800", "-X", "PUT",
      ...Object.entries(headers).flatMap(([key, value]) => ["-H", `${key}: ${value}`]),
      "-T", path, "-w", "\n%{http_code}", url]);
    const split = out.lastIndexOf("\n");
    return { status: Number(out.slice(split + 1)), body: out.slice(0, split) };
  },
  async writeFileAsync(uri: string, contents: string): Promise<void> {
    if (usesRNFS) {
      await RNFS.writeFile(nativePath(uri), contents, "utf8");
      return;
    }
    await getNativeFilePicker().writeFile(nativePath(uri), contents);
  },
  async makeDirectoryAsync(uri: string): Promise<void> {
    if (usesRNFS) {
      await RNFS.mkdir(nativePath(uri));
      return;
    }
    await getNativeFilePicker().makeDirectory(nativePath(uri));
  },
  async copyAsync(options: { from: string; to: string }): Promise<void> {
    if (usesRNFS) {
      await RNFS.copyFile(nativePath(options.from), nativePath(options.to));
      return;
    }
    await getNativeFilePicker().copyFile(nativePath(options.from), nativePath(options.to));
  },
  async deleteAsync(uri: string): Promise<void> {
    if (usesRNFS) {
      await RNFS.unlink(nativePath(uri));
      return;
    }
    await getNativeFilePicker().deleteFile(nativePath(uri));
  },
  async uploadFileAsync(
    url: string,
    fileUri: string,
    headers: Record<string, string> = {},
    fieldName = "file",
    mime = "application/octet-stream",
    fields: Record<string, string> = {}
  ): Promise<{ status: number; body: string }> {
    const path = nativePath(fileUri);
    const fileName = path.split(/[\\/]/).pop() || "upload";
    if (Platform.OS === "windows") {
      return Object.keys(fields).length
        ? NativeModules.RNWindowsFiles.uploadForm(url, path, headers, fieldName, mime, fields)
        : NativeModules.RNWindowsFiles.upload(url, path, headers, fieldName, mime);
    }
    if (usesRNFS) {
      const result = await RNFS.uploadFiles({
        toUrl: url,
        method: "POST",
        fields,
        headers,
        files: [{ name: fieldName, filename: fileName, filepath: path, filetype: mime }],
      }).promise;
      return { status: result.statusCode, body: result.body };
    }
    const headerArgs = Object.entries(headers).flatMap(([name, value]) => ["-H", `${name}: ${value}`]);
    const output = await runCommandArgs("/usr/bin/curl", [
      "-sS",
      "--max-time",
      "1800",
      "-X",
      "POST",
      ...headerArgs,
      ...Object.entries(fields).flatMap(([key, value]) => ["--form-string", `${key}=${value}`]),
      "-F",
      `${fieldName}=@${path};type=${mime}`,
      "-w",
      "\n%{http_code}",
      url,
    ]);
    const separator = output.lastIndexOf("\n");
    const status = Number.parseInt(output.slice(separator + 1), 10);
    if (!status) throw new Error("同步文件上传请求发送失败");
    return { status, body: separator === -1 ? "" : output.slice(0, separator) };
  },
  async downloadFileAsync(
    url: string,
    destination: string,
    headers: Record<string, string> = {}
  ): Promise<void> {
    const target = nativePath(destination);
    if (Platform.OS === "windows") {
      await NativeModules.RNWindowsFiles.download(url, target, headers);
      return;
    }
    if (usesRNFS) {
      const result = await RNFS.downloadFile({ fromUrl: url, toFile: target, headers }).promise;
      if (result.statusCode < 200 || result.statusCode >= 300) {
        throw new Error(`下载同步文件失败（HTTP ${result.statusCode}）`);
      }
      return;
    }
    const headerArgs = Object.entries(headers).flatMap(([name, value]) => ["-H", `${name}: ${value}`]);
    await runCommandArgs("/usr/bin/curl", ["-fsSL", ...headerArgs, "-o", target, url]);
  },
};

export type AVPlaybackStatus = {
  isLoaded: boolean;
  positionMillis: number;
  durationMillis?: number;
  isPlaying: boolean;
  didJustFinish?: boolean;
};

export type SoundLike = {
  unloadAsync: () => Promise<void>;
  playAsync: () => Promise<void>;
  pauseAsync: () => Promise<void>;
  setPositionAsync: (position: number) => Promise<void>;
  setRateAsync: (rate: number, shouldCorrectPitch: boolean) => Promise<void>;
  setLoopAsync: (loop: boolean) => Promise<void>;
};

type NativeAudio = {
  load: (path: string, rate: number) => Promise<AVPlaybackStatus>;
  play: () => Promise<AVPlaybackStatus>;
  pause: () => Promise<AVPlaybackStatus>;
  setPosition: (milliseconds: number) => Promise<AVPlaybackStatus>;
  setRate: (rate: number) => Promise<AVPlaybackStatus | null>;
  setLoop: (loop: boolean) => Promise<AVPlaybackStatus | null>;
  status: () => Promise<AVPlaybackStatus>;
  unload: () => Promise<void>;
};

function getNativeAudio(): NativeAudio {
  const audio = nativeLearningAudio() as NativeAudio | undefined;
  if (!audio) throw new Error(`${Platform.OS} 音频播放器尚未配置`);
  return audio;
}

export const Audio = {
  Sound: {
    async createAsync(
      source: { uri: string },
      options: { shouldPlay?: boolean; rate?: number; loop?: boolean; progressUpdateIntervalMillis?: number },
      onStatus?: (status: AVPlaybackStatus) => void
    ): Promise<{ sound: SoundLike }> {
      const audio = getNativeAudio();
      const update = async () => onStatus?.(await audio.status());
      const loaded = await audio.load(nativePath(source.uri), options.rate ?? 1);
      onStatus?.(loaded);
      if (options.loop) {
        await audio.setLoop(true);
      }
      if (options.shouldPlay) await audio.play();
      const interval = setInterval(update, options.progressUpdateIntervalMillis ?? 250);
      return {
        sound: {
          unloadAsync: async () => { clearInterval(interval); await audio.unload(); },
          playAsync: async () => { onStatus?.(await audio.play()); },
          pauseAsync: async () => { onStatus?.(await audio.pause()); },
          setPositionAsync: async (position) => { onStatus?.(await audio.setPosition(position)); },
          setRateAsync: async (rate) => { const status = await audio.setRate(rate); if (status) onStatus?.(status); },
          setLoopAsync: async (loop) => {
            const status = await audio.setLoop(loop);
            if (status) onStatus?.(status);
          },
        },
      };
    },
  },
};

export type MergedAudio = {
  /** Total duration of the concatenated audio, in milliseconds. */
  durationMs: number;
  /** Per-input durations in the same order as the requested paths. */
  segmentDurationMs: number[];
};

type NativeAudioMerge = {
  mergeAudios: (paths: string[], outputPath: string) => Promise<MergedAudio>;
};

/**
 * Concatenate audio files into one M4A file. macOS only — AVFoundation
 * re-encodes on export, so inputs may mix containers / sample rates. The
 * per-segment durations let callers shift subtitles when merging.
 */
export async function mergeAudios(paths: string[], outputPath: string): Promise<MergedAudio> {
  if (Platform.OS !== "macos" && Platform.OS !== "windows") {
    throw new Error("合并音频目前仅支持 Mac 端");
  }
  const audio = nativeLearningAudio() as NativeAudioMerge | undefined;
  if (!audio?.mergeAudios) {
    throw new Error("macOS 音频合并模块尚未配置");
  }
  const result = await audio.mergeAudios(paths, nativePath(outputPath));
  return {
    durationMs: Number(result.durationMs) || 0,
    segmentDurationMs: (result.segmentDurationMs ?? []).map((ms) => Number(ms) || 0),
  };
}

type NativeShell = {
  runCommand: (command: string) => Promise<string>;
  runArgs?: (launchPath: string, args: string[]) => Promise<string>;
};

/**
 * Run a shell command on the machine running the app (macOS only) and
 * resolve with its stdout. Used by the listening screen's command import.
 */
export async function runShellCommand(command: string): Promise<string> {
  const shell = NativeModules.RNShell as NativeShell | undefined;
  if (!shell) {
    throw new Error("命令执行模块尚未配置（仅支持 macOS）");
  }
  return shell.runCommand(command);
}

/** Run an executable with an argument array (no shell parsing involved). */
export async function runCommandArgs(
  launchPath: string,
  args: string[]
): Promise<string> {
  const shell = NativeModules.RNShell as NativeShell | undefined;
  if (!shell?.runArgs) {
    throw new Error("命令执行模块尚未配置（仅支持 macOS）");
  }
  return shell.runArgs(launchPath, args);
}

// ── 口语跟读录音（Android: ClaroraAudioModule / Mac: RNMacAudioRecorder）────

type NativeAudioRecorder = {
  startRecording: () => Promise<boolean | void>;
  stopRecording: () => Promise<{ uri: string; name: string }>;
};

export const audioRecorder = {
  async start(): Promise<void> {
    const module = nativeRecordingAudio() as NativeAudioRecorder | undefined;
    if (!module?.startRecording) {
      throw new Error(`${Platform.OS} 录音模块尚未配置`);
    }
    await module.startRecording();
  },
  async stop(): Promise<{ uri: string; name: string }> {
    const module = nativeRecordingAudio() as NativeAudioRecorder | undefined;
    if (!module?.stopRecording) {
      throw new Error(`${Platform.OS} 录音模块尚未配置`);
    }
    return module.stopRecording();
  },
};
