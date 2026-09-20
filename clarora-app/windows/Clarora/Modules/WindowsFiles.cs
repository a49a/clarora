using Microsoft.ReactNative.Managed;
using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Net.Http;
using System.Net.Http.Headers;
using System.Threading;
using System.Threading.Tasks;
using Windows.ApplicationModel.DataTransfer;
using Windows.Storage;
using Windows.Storage.Pickers;

namespace Clarora
{
    [ReactModule("RNWindowsFiles")]
    public sealed class WindowsFiles
    {
        private ReactContext context;
        private readonly SemaphoreSlim pickerLock = new SemaphoreSlim(1, 1);
        [ReactInitializer] public void Initialize(ReactContext value) { context = value; }

        private Task<T> UI<T>(Func<Task<T>> action)
        {
            var result = new TaskCompletionSource<T>();
            context.UIDispatcher.Post(async () => {
                try { result.TrySetResult(await action()); }
                catch (Exception e) { result.TrySetException(e); }
            });
            return result.Task;
        }
        internal static string PathOf(string path) => path.StartsWith("file:", StringComparison.OrdinalIgnoreCase) ? new Uri(path).LocalPath : path;
        internal static JSValue Picked(StorageFile file) => new JSValueObject { ["uri"] = file.Path, ["name"] = file.Name };
        private static async Task<StorageFolder> ImportFolder() => await ApplicationData.Current.LocalFolder.CreateFolderAsync("imports-" + Guid.NewGuid().ToString("N"));

        [ReactMethod("pickFile")]
        public async Task<JSValue> PickFile(JSValue options)
        {
            if (!await pickerLock.WaitAsync(0)) throw new InvalidOperationException("请先完成当前文件选择");
            try {
                return await UI(async () => {
                    var picker = new FileOpenPicker();
                    var mime = options["type"].AsString();
                    var extensions = mime.StartsWith("audio/") ? new[] { ".mp3", ".m4a", ".wav", ".aac", ".flac", ".ogg" }
                        : mime.StartsWith("image/") ? new[] { ".png", ".jpg", ".jpeg", ".webp", ".bmp" } : new[] { "*" };
                    foreach (var extension in extensions) picker.FileTypeFilter.Add(extension);
                    var source = await picker.PickSingleFileAsync();
                    if (source == null) return JSValue.Null;
                    return Picked(await source.CopyAsync(await ImportFolder(), source.Name));
                });
            } finally { pickerLock.Release(); }
        }
        [ReactMethod("pickDirectory")]
        public async Task<JSValue> PickDirectory()
        {
            if (!await pickerLock.WaitAsync(0)) throw new InvalidOperationException("请先完成当前文件选择");
            try {
                return await UI(async () => {
                    var picker = new FolderPicker();
                    picker.FileTypeFilter.Add("*");
                    var source = await picker.PickSingleFolderAsync();
                    if (source == null) return JSValue.Null;
                    var destination = await ImportFolder();
                    // Match listFiles on other platforms: import immediate files.
                    // Copies keep access valid after restart without broad disk access.
                    foreach (var file in await source.GetFilesAsync()) await file.CopyAsync(destination, file.Name);
                    return (JSValue)destination.Path;
                });
            } finally { pickerLock.Release(); }
        }
        [ReactMethod("getDocumentDirectory")] public Task<string> Documents() => Task.FromResult(ApplicationData.Current.LocalFolder.Path.Replace('\\', '/'));
        [ReactMethod("listFiles")] public Task<string[]> List(string path) => Task.Run(() => Directory.GetFiles(PathOf(path)));
        [ReactMethod("readFile")] public Task<string> Read(string path) => Task.Run(() => File.ReadAllText(PathOf(path)));
        [ReactMethod("writeFile")] public Task Write(string path, string contents) => Task.Run(() => File.WriteAllText(PathOf(path), contents));
        [ReactMethod("writeBase64")] public Task WriteBase64(string path, string contents) => Task.Run(() => File.WriteAllBytes(PathOf(path), Convert.FromBase64String(contents)));
        [ReactMethod("makeDirectory")] public Task Mkdir(string path) => Task.Run(() => { Directory.CreateDirectory(PathOf(path)); });
        [ReactMethod("copyFile")] public Task Copy(string source, string target) => Task.Run(() => File.Copy(PathOf(source), PathOf(target), true));
        [ReactMethod("deleteFile")] public Task Delete(string path) => Task.Run(() => File.Delete(PathOf(path)));
        [ReactMethod("setText")] public void ClipboardText(string text) => context.UIDispatcher.Post(() => {
            var data = new DataPackage(); data.SetText(text); Clipboard.SetContent(data);
        });
        private static HttpClient Client(JSValue headers)
        {
            var client = new HttpClient { Timeout = TimeSpan.FromMinutes(30) };
            foreach (var header in headers.AsObject()) client.DefaultRequestHeaders.TryAddWithoutValidation(header.Key, header.Value.AsString());
            return client;
        }
        [ReactMethod("upload")]
        public async Task<JSValue> Upload(string url, string path, JSValue headers, string field, string mime)
        {
            using (var client = Client(headers))
            using (var content = new MultipartFormDataContent())
            using (var stream = File.OpenRead(PathOf(path))) {
                var file = new StreamContent(stream);
                file.Headers.ContentType = new MediaTypeHeaderValue(mime);
                content.Add(file, field, Path.GetFileName(PathOf(path)));
                using (var response = await client.PostAsync(url, content)) {
                    return new JSValueObject { ["status"] = (int)response.StatusCode, ["body"] = await response.Content.ReadAsStringAsync() };
                }
            }
        }
        [ReactMethod("uploadForm")]
        public async Task<JSValue> UploadForm(string url, string path, JSValue headers, string field, string mime, JSValue fields)
        {
            using (var client = Client(headers))
            using (var content = new MultipartFormDataContent())
            using (var stream = File.OpenRead(PathOf(path))) {
                foreach (var item in fields.AsObject()) content.Add(new StringContent(item.Value.AsString()), item.Key);
                var file = new StreamContent(stream);
                file.Headers.ContentType = new MediaTypeHeaderValue(mime);
                content.Add(file, field, Path.GetFileName(PathOf(path)));
                using (var response = await client.PostAsync(url, content)) {
                    return new JSValueObject { ["status"] = (int)response.StatusCode, ["body"] = await response.Content.ReadAsStringAsync() };
                }
            }
        }
        [ReactMethod("readBase64")]
        public Task<string> ReadBase64(string path) => Task.FromResult(Convert.ToBase64String(File.ReadAllBytes(PathOf(path))));

        [ReactMethod("putFile")]
        public async Task<JSValue> PutFile(string url, string path, JSValue headers)
        {
            using (var client = Client(headers))
            using (var stream = File.OpenRead(PathOf(path)))
            using (var content = new StreamContent(stream)) {
                content.Headers.ContentType = new MediaTypeHeaderValue("application/octet-stream");
                using (var response = await client.PutAsync(url, content)) {
                    return new JSValueObject { ["status"] = (int)response.StatusCode, ["body"] = await response.Content.ReadAsStringAsync() };
                }
            }
        }
        [ReactMethod("download")]
        public async Task Download(string url, string path, JSValue headers)
        {
            var target = PathOf(path);
            var temporary = target + "." + Guid.NewGuid().ToString("N") + ".part";
            try {
                using (var client = Client(headers))
                using (var response = await client.GetAsync(url, HttpCompletionOption.ResponseHeadersRead)) {
                    response.EnsureSuccessStatusCode();
                    using (var output = File.Create(temporary)) await response.Content.CopyToAsync(output);
                }
                File.Copy(temporary, target, true);
            } finally { if (File.Exists(temporary)) File.Delete(temporary); }
        }
    }
}
