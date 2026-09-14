using Microsoft.ReactNative;
using Microsoft.ReactNative.Managed;
using System;
using System.Collections.Generic;
using System.IO;
using System.Threading.Tasks;
using Windows.Media.Core;
using Windows.Media.Editing;
using Windows.Media.MediaProperties;
using Windows.Media.Playback;
using Windows.Media.Transcoding;
using Windows.Storage;
using Windows.UI.Xaml;
using Windows.UI.Xaml.Controls;

namespace Clarora
{
    public sealed class VideoSurface : Grid
    {
        internal readonly MediaPlayer Player = new MediaPlayer { AutoPlay = false };
        internal MediaPlaybackItem Item;
        internal readonly DispatcherTimer Timer = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(100) };
        internal bool Playing, Dropped;
        internal double Rate = 1;
        internal int Version;
        internal Action<string, JSValue> Emit;
        public VideoSurface() {
            var element = new MediaPlayerElement { AreTransportControlsEnabled = false };
            element.SetMediaPlayer(Player); Children.Add(element);
            Player.CommandManager.IsEnabled = false;
            Player.MediaOpened += (_, __) => Dispatcher.RunAsync(Windows.UI.Core.CoreDispatcherPriority.Normal, () => {
                if (Dropped) return;
                Player.PlaybackSession.PlaybackRate = Rate;
                if (Playing) Player.Play();
            });
            Player.MediaEnded += (_, __) => Dispatcher.RunAsync(Windows.UI.Core.CoreDispatcherPriority.Normal, () => {
                if (!Dropped) { Playing = false; Emit?.Invoke("topEnd", new JSValueObject()); }
            });
            Player.MediaFailed += (_, e) => Dispatcher.RunAsync(Windows.UI.Core.CoreDispatcherPriority.Normal, () => {
                if (!Dropped) Emit?.Invoke("topError", new JSValueObject { ["message"] = e.ErrorMessage });
            });
            Timer.Tick += (_, __) => {
                if (Item != null) Emit?.Invoke("topProgress", new JSValueObject {
                    ["positionMs"] = Player.PlaybackSession.Position.TotalMilliseconds,
                    ["durationMs"] = Player.PlaybackSession.NaturalDuration.TotalMilliseconds
                });
            };
            Loaded += (_, __) => Timer.Start();
            Unloaded += (_, __) => { Timer.Stop(); Player.Pause(); };
        }
        internal async void Source(string path) {
            var version = ++Version;
            Player.Pause(); Player.Source = null; Item = null;
            if (string.IsNullOrEmpty(path)) return;
            try {
                var file = await StorageFile.GetFileFromPathAsync(WindowsFiles.PathOf(path));
                if (Dropped || version != Version) return;
                Item = new MediaPlaybackItem(MediaSource.CreateFromStorageFile(file));
                Player.Source = Item;
            } catch (Exception e) { if (!Dropped && version == Version) Emit?.Invoke("topError", new JSValueObject { ["message"] = e.Message }); }
        }
        internal void Drop() { Dropped = true; ++Version; Timer.Stop(); Emit = null; Player.Source = null; Player.Dispose(); }
    }

    public sealed class RNVideoPlayerView : IViewManager, IViewManagerWithReactContext, IViewManagerWithNativeProperties,
        IViewManagerWithExportedEventTypeConstants, IViewManagerWithDropViewInstance
    {
        public string Name => "RNVideoPlayerView";
        public IReactContext ReactContext { get; set; }
        public FrameworkElement CreateView() {
            var view = new VideoSurface();
            var context = new ReactContext(ReactContext);
            view.Emit = (name, data) => context.DispatchEvent(view, name, data);
            return view;
        }
        public IReadOnlyDictionary<string, ViewManagerPropertyType> NativeProps => new Dictionary<string, ViewManagerPropertyType> {
            ["src"] = ViewManagerPropertyType.String, ["playing"] = ViewManagerPropertyType.Boolean, ["rate"] = ViewManagerPropertyType.Number
        };
        public void UpdateProperties(FrameworkElement element, IJSValueReader reader) {
            var view = (VideoSurface)element;
            reader.ReadValue(out JSValue properties);
            foreach (var prop in properties.AsObject()) {
                if (prop.Key == "src") view.Source(prop.Value.AsString());
                if (prop.Key == "playing") { view.Playing = prop.Value.AsBoolean(); if (view.Playing && view.Item != null) view.Player.Play(); else view.Player.Pause(); }
                if (prop.Key == "rate") { view.Rate = prop.Value.AsDouble(); view.Player.PlaybackSession.PlaybackRate = view.Rate; }
            }
        }
        public ConstantProviderDelegate ExportedCustomBubblingEventTypeConstants => writer => { };
        public ConstantProviderDelegate ExportedCustomDirectEventTypeConstants => writer => {
            foreach (var name in new[] { "Progress", "End", "Error" }) {
                writer.WritePropertyName("top" + name); writer.WriteObjectBegin();
                writer.WriteObjectProperty("registrationName", "on" + name); writer.WriteObjectEnd();
            }
        };
        public void OnDropViewInstance(FrameworkElement view) => ((VideoSurface)view).Drop();
    }

    [ReactModule("RNVideoControl")]
    public sealed class WindowsVideoControl
    {
        private ReactContext context;
        [ReactInitializer] public void Initialize(ReactContext value) { context = value; }
        private VideoSurface View(long tag) => XamlUIService.FromContext(context.Handle).ElementFromReactTag(tag) as VideoSurface;
        [ReactMethod("seek")] public void Seek(long tag, double ms) => context.UIDispatcher.Post(() => {
            var view = View(tag); if (view != null) view.Player.PlaybackSession.Position = TimeSpan.FromMilliseconds(Math.Max(0, ms));
        });
        [ReactMethod("setVolume")] public void Volume(long tag, double volume) => context.UIDispatcher.Post(() => { var view = View(tag); if (view != null) view.Player.Volume = Math.Max(0, Math.Min(1, volume / 100)); });
        [ReactMethod("getSubtitleTracks")]
        public Task<JSValue> Tracks(long tag) {
            var result = new TaskCompletionSource<JSValue>();
            context.UIDispatcher.Post(() => {
                try {
                    var tracks = new JSValueArray(); var item = View(tag)?.Item;
                    if (item != null) for (uint i = 0; i < item.TimedMetadataTracks.Count; i++) {
                        var track = item.TimedMetadataTracks[(int)i];
                        tracks.Add(new JSValueObject { ["id"] = (int)i + 1, ["name"] = string.IsNullOrEmpty(track.Label) ? "字幕 " + (i + 1) : track.Label, ["lang"] = track.Language,
                            ["codec"] = "", ["textTrack"] = true, ["subStreamIndex"] = (int)i,
                            ["selected"] = item.TimedMetadataTracks.GetPresentationMode(i) == TimedMetadataTrackPresentationMode.PlatformPresented,
                            ["external"] = false, ["externalFilename"] = "" });
                    }
                    result.SetResult(new JSValueObject { ["tracks"] = tracks, ["secondaryId"] = 0 });
                } catch (Exception e) { result.SetException(e); }
            });
            return result.Task;
        }
        [ReactMethod("selectSubtitle")] public void Subtitle(long tag, int selected) => context.UIDispatcher.Post(() => {
            var item = View(tag)?.Item; if (item == null) return;
            for (uint i = 0; i < item.TimedMetadataTracks.Count; i++) item.TimedMetadataTracks.SetPresentationMode(i,
                i + 1 == selected ? TimedMetadataTrackPresentationMode.PlatformPresented : TimedMetadataTrackPresentationMode.Disabled);
        });
        [ReactMethod("exportClip")]
        public async Task Export(string source, string destination, double startMs, double endMs) {
            var clip = await MediaClip.CreateFromFileAsync(await StorageFile.GetFileFromPathAsync(WindowsFiles.PathOf(source)));
            if (startMs < 0 || endMs <= startMs || endMs > clip.OriginalDuration.TotalMilliseconds) throw new ArgumentException("视频片段范围无效");
            clip.TrimTimeFromStart = TimeSpan.FromMilliseconds(startMs);
            clip.TrimTimeFromEnd = clip.OriginalDuration - TimeSpan.FromMilliseconds(endMs);
            var composition = new MediaComposition(); composition.Clips.Add(clip);
            var path = WindowsFiles.PathOf(destination);
            var folder = await StorageFolder.GetFolderFromPathAsync(Path.GetDirectoryName(path));
            var file = await folder.CreateFileAsync(Path.GetFileName(path), CreationCollisionOption.ReplaceExisting);
            var status = await composition.RenderToFileAsync(file, MediaTrimmingPreference.Precise, MediaEncodingProfile.CreateMp4(VideoEncodingQuality.Auto));
            if (status != TranscodeFailureReason.None) { await file.DeleteAsync(); throw new InvalidOperationException("视频截取失败：" + status); }
        }
    }
}
