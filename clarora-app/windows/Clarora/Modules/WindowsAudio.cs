using Microsoft.ReactNative.Managed;
using System;
using System.IO;
using System.Threading;
using System.Threading.Tasks;
using Windows.Media.Capture;
using Windows.Media.Core;
using Windows.Media.Editing;
using Windows.Media.MediaProperties;
using Windows.Media.Playback;
using Windows.Media.Transcoding;
using Windows.Storage;

namespace Clarora
{
    [ReactModule("RNWindowsAudio")]
    public sealed class WindowsAudio : IDisposable
    {
        private sealed class Player : IDisposable
        {
            public readonly MediaPlayer Native = new MediaPlayer { AutoPlay = false };
            public bool Playing, Finished;
            public string Error;
            public Player() {
                Native.CommandManager.IsEnabled = false;
                Native.MediaEnded += (_, __) => { Playing = false; Finished = true; };
                Native.MediaFailed += (_, e) => { Playing = false; Error = e.ErrorMessage; };
            }
            public JSValue Status() {
                if (Error != null) throw new InvalidOperationException(Error);
                return new JSValueObject {
                    ["isLoaded"] = true, ["isPlaying"] = Playing, ["didJustFinish"] = Finished,
                    ["positionMillis"] = Finished ? Native.PlaybackSession.NaturalDuration.TotalMilliseconds : Native.PlaybackSession.Position.TotalMilliseconds,
                    ["durationMillis"] = Native.PlaybackSession.NaturalDuration.TotalMilliseconds
                };
            }
            public void Pause() { Native.Pause(); Playing = false; }
            public void Play() { Finished = false; Playing = true; Native.Play(); }
            public void Dispose() { Playing = false; Native.Source = null; Native.Dispose(); }
        }
        private ReactContext context;
        private readonly SemaphoreSlim gate = new SemaphoreSlim(1, 1);
        private Player main, rest;
        private readonly Player[] feed = new Player[2];
        private MediaCapture recorder;
        private StorageFile recording;
        [ReactInitializer] public void Initialize(ReactContext value) { context = value; }
        private async Task<T> Run<T>(Func<Task<T>> action)
        {
            await gate.WaitAsync();
            try {
                var result = new TaskCompletionSource<T>();
                context.UIDispatcher.Post(async () => {
                    try { result.TrySetResult(await action()); }
                    catch (Exception e) { result.TrySetException(e); }
                });
                return await result.Task;
            } finally { gate.Release(); }
        }
        private Task<JSValue> Change(Func<JSValue> action) => Run(() => Task.FromResult(action()));
        private static JSValue Status(Player player) => player == null
            ? new JSValueObject { ["isLoaded"] = false, ["isPlaying"] = false, ["positionMillis"] = 0 }
            : player.Status();
        private static async Task<Player> Open(string path, double rate, double start)
        {
            var player = new Player();
            var ready = new TaskCompletionSource<bool>();
            Windows.Foundation.TypedEventHandler<MediaPlayer, object> opened = (_, __) => ready.TrySetResult(true);
            Windows.Foundation.TypedEventHandler<MediaPlayer, MediaPlayerFailedEventArgs> failed = (_, e) => ready.TrySetException(new InvalidOperationException(e.ErrorMessage));
            player.Native.MediaOpened += opened;
            player.Native.MediaFailed += failed;
            var loaded = false;
            try {
                var file = await StorageFile.GetFileFromPathAsync(WindowsFiles.PathOf(path));
                player.Native.Source = MediaSource.CreateFromStorageFile(file);
                if (await Task.WhenAny(ready.Task, Task.Delay(20000)) != ready.Task) throw new TimeoutException("音频加载超时");
                await ready.Task;
                player.Native.PlaybackSession.PlaybackRate = rate;
                player.Native.PlaybackSession.Position = TimeSpan.FromMilliseconds(Math.Max(0, Math.Min(start, player.Native.PlaybackSession.NaturalDuration.TotalMilliseconds)));
                loaded = true;
                return player;
            } finally {
                player.Native.MediaOpened -= opened; player.Native.MediaFailed -= failed;
                if (!loaded) player.Dispose();
            }
        }
        private Player Required() => main ?? throw new InvalidOperationException("没有已加载的音频");
        private Player Slot(int slot) {
            if (slot < 0 || slot > 1) throw new ArgumentException("无效的预加载位置");
            return feed[slot];
        }
        [ReactMethod("load")] public Task<JSValue> Load(string path, double rate) => Run(async () => {
            main?.Dispose(); main = null; main = await Open(path, rate, 0); return Status(main);
        });
        [ReactMethod("play")] public Task<JSValue> Play() => Change(() => { Required().Play(); return Status(main); });
        [ReactMethod("pause")] public Task<JSValue> Pause() => Change(() => { main?.Pause(); return Status(main); });
        [ReactMethod("status")] public Task<JSValue> GetStatus() => Change(() => Status(main));
        [ReactMethod("setPosition")] public Task<JSValue> Seek(double ms) => Change(() => {
            var player = Required(); player.Finished = false;
            player.Native.PlaybackSession.Position = TimeSpan.FromMilliseconds(Math.Max(0, Math.Min(ms, player.Native.PlaybackSession.NaturalDuration.TotalMilliseconds)));
            return Status(player);
        });
        [ReactMethod("setRate")] public Task<JSValue> Rate(double rate) => Change(() => {
            if (main != null) main.Native.PlaybackSession.PlaybackRate = rate; return Status(main);
        });
        [ReactMethod("setLoop")] public Task<JSValue> Loop(bool loop) => Change(() => {
            if (main != null) main.Native.IsLoopingEnabled = loop; return Status(main);
        });
        [ReactMethod("unload")] public Task<JSValue> Unload() => Change(() => { main?.Dispose(); main = null; return JSValue.Null; });
        [ReactMethod("feedPrepare")] public Task<JSValue> Prepare(int slot, string path, double start) => Run(async () => {
            Slot(slot)?.Dispose(); feed[slot] = null;
            feed[slot] = await Open(path, 1, start); return Status(feed[slot]);
        });
        [ReactMethod("feedPlay")] public Task<JSValue> FeedPlay(int slot) => Change(() => {
            var player = Slot(slot) ?? throw new InvalidOperationException("片段尚未加载"); player.Play(); return Status(player);
        });
        [ReactMethod("feedPause")] public Task<JSValue> FeedPause(int slot) => Change(() => { Slot(slot)?.Pause(); return Status(Slot(slot)); });
        [ReactMethod("feedStatus")] public Task<JSValue> FeedStatus(int slot) => Change(() => Status(Slot(slot)));
        [ReactMethod("feedUnload")] public Task<JSValue> FeedUnload() => Change(() => {
            for (var i = 0; i < feed.Length; i++) { feed[i]?.Dispose(); feed[i] = null; } return JSValue.Null;
        });
        [ReactMethod("startRecording")] public Task<JSValue> StartRecording() => Run(async () => {
            if (recorder != null) throw new InvalidOperationException("请先结束当前录音");
            main?.Pause(); rest?.Pause(); foreach (var player in feed) player?.Pause();
            var capture = new MediaCapture();
            try {
                await capture.InitializeAsync(new MediaCaptureInitializationSettings { StreamingCaptureMode = StreamingCaptureMode.Audio });
                recording = await ApplicationData.Current.LocalFolder.CreateFileAsync("recording-" + Guid.NewGuid().ToString("N") + ".m4a");
                await capture.StartRecordToStorageFileAsync(MediaEncodingProfile.CreateM4a(AudioEncodingQuality.High), recording);
                recorder = capture;
                return (JSValue)true;
            } catch { capture.Dispose(); throw; }
        });
        [ReactMethod("stopRecording")] public Task<JSValue> StopRecording() => Run(async () => {
            if (recorder == null) throw new InvalidOperationException("没有进行中的录音");
            try { await recorder.StopRecordAsync(); return WindowsFiles.Picked(recording); }
            finally { recorder.Dispose(); recorder = null; }
        });
        private async void Background(Func<Task<JSValue>> action) { try { await Run(action); } catch { /* Optional timer sound must not crash the app. */ } }
        [ReactMethod("playBreak")] public void PlayBreak(string path) => Background(async () => {
            rest?.Dispose(); rest = null; rest = await Open(path, 1, 0); rest.Native.IsLoopingEnabled = true; rest.Play(); return JSValue.Null;
        });
        [ReactMethod("pauseBreak")] public void PauseBreak() => Background(() => { rest?.Pause(); return Task.FromResult(JSValue.Null); });
        [ReactMethod("resumeBreak")] public void ResumeBreak() => Background(() => { rest?.Play(); return Task.FromResult(JSValue.Null); });
        [ReactMethod("stopBreak")] public void StopBreak() => Background(() => { rest?.Dispose(); rest = null; return Task.FromResult(JSValue.Null); });
        [ReactMethod("playChime")] public void Chime() => context.UIDispatcher.Post(() => Windows.UI.Xaml.ElementSoundPlayer.Play(Windows.UI.Xaml.ElementSoundKind.Invoke));
        [ReactMethod("mergeAudios")]
        public Task<JSValue> Merge(string[] paths, string outputPath) => Run(async () => {
            if (paths.Length == 0) throw new ArgumentException("请选择需要合并的音频");
            var composition = new MediaComposition();
            var durations = new JSValueArray();
            var cursor = TimeSpan.Zero;
            foreach (var path in paths) {
                var track = await BackgroundAudioTrack.CreateFromFileAsync(await StorageFile.GetFileFromPathAsync(WindowsFiles.PathOf(path)));
                track.Delay = cursor; composition.BackgroundAudioTracks.Add(track);
                durations.Add(track.OriginalDuration.TotalMilliseconds); cursor += track.OriginalDuration;
            }
            var destination = WindowsFiles.PathOf(outputPath);
            var folder = await StorageFolder.GetFolderFromPathAsync(Path.GetDirectoryName(destination));
            var file = await folder.CreateFileAsync(Path.GetFileName(destination), CreationCollisionOption.ReplaceExisting);
            var result = await composition.RenderToFileAsync(file, MediaTrimmingPreference.Precise, MediaEncodingProfile.CreateM4a(AudioEncodingQuality.High));
            if (result != TranscodeFailureReason.None) { await file.DeleteAsync(); throw new InvalidOperationException("音频合并失败：" + result); }
            return (JSValue)new JSValueObject { ["durationMs"] = cursor.TotalMilliseconds, ["segmentDurationMs"] = durations };
        });
        public void Dispose() { main?.Dispose(); rest?.Dispose(); foreach (var player in feed) player?.Dispose(); recorder?.Dispose(); }
    }
}
