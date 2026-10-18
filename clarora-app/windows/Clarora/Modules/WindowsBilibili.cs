using Microsoft.ReactNative.Managed;
using System;
using System.IO;
using System.Threading;
using System.Threading.Tasks;
using Windows.ApplicationModel;
using Windows.Data.Json;
using Windows.Storage;

namespace Clarora
{
    [ReactModule("RNBilibiliDownloader")]
    public sealed class WindowsBilibili
    {
        private static readonly SemaphoreSlim Gate = new SemaphoreSlim(1, 1);
        private volatile bool cancelled;
        [ReactMethod("cancel")]
        public void Cancel() { cancelled = true; }

        [ReactMethod("download")]
        public async Task<string> Download(string url)
        {
            if (!await Gate.WaitAsync(0)) throw new InvalidOperationException("已有下载任务，请等待或取消后重试");
            cancelled = false;
            string task = null;
            bool success = false;
            try {
                var root = ApplicationData.Current.LocalFolder.Path;
                var id = Guid.NewGuid().ToString("N");
                task = Path.Combine(root, "bilibili", id);
                Directory.CreateDirectory(task);
                var heartbeat = Path.Combine(task, "heartbeat");
                File.WriteAllText(heartbeat, "");
                var request = new JsonObject { ["id"] = JsonValue.CreateStringValue(id), ["url"] = JsonValue.CreateStringValue(url) };
                File.WriteAllText(Path.Combine(root, "bilibili-request.json"), request.Stringify());
                await FullTrustProcessLauncher.LaunchFullTrustProcessForCurrentAppAsync();
                var deadline = DateTime.UtcNow.AddMinutes(31);
                var startupDeadline = DateTime.UtcNow.AddSeconds(60);
                DateTime? cancelDeadline = null;
                while (true) {
                    if (cancelled && !cancelDeadline.HasValue) {
                        File.WriteAllText(Path.Combine(task, "cancel"), "");
                        cancelDeadline = DateTime.UtcNow.AddSeconds(15);
                    }
                    var resultFile = Path.Combine(task, "result.json");
                    if (File.Exists(resultFile)) {
                        var result = JsonObject.Parse(File.ReadAllText(resultFile));
                        if (cancelled) throw new OperationCanceledException("已取消下载");
                        var path = result.GetNamedValue("path");
                        if (path.ValueType != JsonValueType.String) throw new InvalidOperationException(result.GetNamedString("error", "下载失败"));
                        var file = Path.GetFullPath(path.GetString());
                        if (!file.StartsWith(task + Path.DirectorySeparatorChar, StringComparison.OrdinalIgnoreCase) ||
                            !string.Equals(Path.GetExtension(file), ".mp4", StringComparison.OrdinalIgnoreCase) || !File.Exists(file) || new FileInfo(file).Length == 0)
                            throw new InvalidOperationException("下载结果不是完整视频文件");
                        success = true;
                        return file;
                    }
                    if ((!File.Exists(Path.Combine(task, "started")) && DateTime.UtcNow > startupDeadline) ||
                        (cancelDeadline.HasValue && DateTime.UtcNow > cancelDeadline.Value) || DateTime.UtcNow > deadline)
                        throw new InvalidOperationException("下载助手未响应，已请求停止。请重启 Clarora 后重试。");
                    File.WriteAllText(heartbeat, "");
                    await Task.Delay(500);
                }
            } finally {
                try {
                    if (!success && task != null && Directory.Exists(task)) {
                        File.WriteAllText(Path.Combine(task, "cancel"), "");
                        // Await the helper's final result before deleting files it may still own.
                        for (var i = 0; i < 30 && !File.Exists(Path.Combine(task, "result.json")); i++) {
                            await Task.Delay(500);
                        }
                        if (File.Exists(Path.Combine(task, "result.json"))) Directory.Delete(task, true);
                    }
                } catch (IOException) { } catch (UnauthorizedAccessException) { }
                finally { Gate.Release(); }
            }
        }
    }
}
