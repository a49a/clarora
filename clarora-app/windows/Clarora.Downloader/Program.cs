using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json;
using System.Text.RegularExpressions;
using Microsoft.Win32.SafeHandles;

// Full-trust companion: the UWP never launches arbitrary commands or supplies output paths.
internal static class Program
{
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetCurrentPackageFamilyName(ref uint length, StringBuilder? name);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
    private static extern SafeFileHandle CreateJobObject(IntPtr attributes, string? name);
    [DllImport("kernel32.dll")]
    private static extern bool SetInformationJobObject(SafeFileHandle job, int kind, ref JobLimits limits, uint size);
    [DllImport("kernel32.dll")]
    private static extern bool AssignProcessToJobObject(SafeFileHandle job, IntPtr process);
    [StructLayout(LayoutKind.Sequential)]
    private struct BasicLimits { public long ProcessTime, JobTime; public uint Flags; public UIntPtr Min, Max; public uint Active; public UIntPtr Affinity; public uint Priority, Scheduling; }
    [StructLayout(LayoutKind.Sequential)]
    private struct IoCounters { public ulong ReadOps, WriteOps, OtherOps, ReadBytes, WriteBytes, OtherBytes; }
    [StructLayout(LayoutKind.Sequential)]
    private struct JobLimits { public BasicLimits Basic; public IoCounters Io; public UIntPtr ProcessMemory, JobMemory, PeakProcessMemory, PeakJobMemory; }

    private static string Tool(string name)
    {
        var paths = (Environment.GetEnvironmentVariable("PATH") ?? "").Split(Path.PathSeparator)
            .Concat(new[] { Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "Microsoft", "WinGet", "Links") });
        foreach (var directory in paths) {
            if (!Path.IsPathFullyQualified(directory)) continue;
            var file = Path.Combine(directory.Trim('"'), name + ".exe");
            if (File.Exists(file)) return file;
        }
        throw new InvalidOperationException("请先安装 yt-dlp 和 ffmpeg 并加入 PATH，再重启 Clarora。可运行 winget install yt-dlp.yt-dlp 和 winget install Gyan.FFmpeg");
    }

    private static async Task Drain(StreamReader reader, StringBuilder tail)
    {
        var buffer = new char[4096]; int count;
        while ((count = await reader.ReadAsync(buffer)) > 0) {
            tail.Append(buffer, 0, count);
            if (tail.Length > 65536) tail.Remove(0, tail.Length - 65536);
        }
    }

    private static async Task Main()
    {
        uint length = 0;
        GetCurrentPackageFamilyName(ref length, null);
        if (length == 0) return;
        var family = new StringBuilder((int)length);
        if (GetCurrentPackageFamilyName(ref length, family) != 0) return;
        var root = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "Packages", family.ToString(), "LocalState");
        string? task = null, path = null, error = null;
        try {
            using var request = JsonDocument.Parse(await File.ReadAllTextAsync(Path.Combine(root, "bilibili-request.json")));
            var id = request.RootElement.GetProperty("id").GetString()!;
            if (!Guid.TryParseExact(id, "N", out _)) return;
            task = Path.Combine(root, "bilibili", id);
            var url = request.RootElement.GetProperty("url").GetString()!;
            if (!Regex.IsMatch(url, @"\Ahttps://www\.bilibili\.com/video/(BV[0-9A-Za-z]{10}|av[1-9][0-9]*)/?(\?p=[1-9][0-9]{0,4})?\z"))
                throw new InvalidOperationException("bilibili 视频链接无效");
            if (!Directory.Exists(task)) return;
            // File sharing locks survive async continuations and prevent duplicate helpers.
            FileStream lease;
            try { lease = new FileStream(Path.Combine(root, "bilibili.lock"), FileMode.OpenOrCreate, FileAccess.ReadWrite, FileShare.None); }
            catch (IOException) { return; }
            using (lease) {
                File.WriteAllText(Path.Combine(task, "started"), "");
                var tool = Tool("yt-dlp"); var ffmpeg = Tool("ffmpeg");
                using var job = CreateJobObject(IntPtr.Zero, null);
                var limits = new JobLimits { Basic = new BasicLimits { Flags = 0x2000 } }; // KILL_ON_JOB_CLOSE
                if (job.IsInvalid || !SetInformationJobObject(job, 9, ref limits, (uint)Marshal.SizeOf<JobLimits>()))
                    throw new InvalidOperationException("无法创建下载进程组");
                var start = new ProcessStartInfo(tool) {
                    UseShellExecute = false, CreateNoWindow = true, RedirectStandardOutput = true,
                    RedirectStandardError = true, StandardOutputEncoding = Encoding.UTF8, StandardErrorEncoding = Encoding.UTF8
                };
                foreach (var arg in new[] {
                    "--ignore-config", "--no-playlist", "--no-simulate", "--no-progress", "--no-color", "--encoding", "utf-8",
                    "--socket-timeout", "30", "--retries", "2", "--fragment-retries", "2", "--ffmpeg-location", Path.GetDirectoryName(ffmpeg)!,
                    "--format", "bv*[vcodec^=avc1]+ba[acodec^=mp4a]/b[ext=mp4]", "--merge-output-format", "mp4", "--remux-video", "mp4",
                    "--write-subs", "--sub-langs", "en.*,zh.*,ai-en.*,ai-zh.*", "--embed-subs", "--windows-filenames",
                    "--print", "after_move:%(filepath)j", "--paths", task, "--output", "%(title).100B [%(id)s].%(ext)s", "--", url
                }) start.ArgumentList.Add(arg);
                using var process = Process.Start(start) ?? throw new InvalidOperationException("无法启动 yt-dlp");
                if (!AssignProcessToJobObject(job, process.Handle)) {
                    process.Kill(true); await process.WaitForExitAsync();
                    throw new InvalidOperationException("无法管理下载进程组");
                }
                var stdout = new StringBuilder(); var stderr = new StringBuilder();
                var readers = Task.WhenAll(Drain(process.StandardOutput, stdout), Drain(process.StandardError, stderr));
                var exited = process.WaitForExitAsync(); var deadline = DateTime.UtcNow.AddMinutes(30);
                try {
                    while (!exited.IsCompleted) {
                        if (File.Exists(Path.Combine(task, "cancel")) || !File.Exists(Path.Combine(task, "heartbeat")) ||
                            DateTime.UtcNow - File.GetLastWriteTimeUtc(Path.Combine(task, "heartbeat")) > TimeSpan.FromSeconds(30))
                            throw new OperationCanceledException("已取消下载");
                        if (DateTime.UtcNow > deadline) throw new TimeoutException("下载超过 30 分钟，已停止");
                        await Task.WhenAny(exited, Task.Delay(250));
                    }
                    await exited;
                } finally {
                    // Closing the job also stops ffmpeg and releases inherited pipe handles.
                    job.Dispose();
                    await process.WaitForExitAsync();
                    await readers;
                }
                if (process.ExitCode != 0) throw new InvalidOperationException("下载失败，请检查网络、视频访问权限或更新 yt-dlp。\n" + stderr.ToString()[Math.Max(0, stderr.Length - 2000)..]);
                foreach (var line in stdout.ToString().Split('\n')) {
                    string? candidate;
                    try { candidate = JsonSerializer.Deserialize<string>(line); } catch (JsonException) { continue; }
                    if (candidate == null) continue;
                    candidate = Path.GetFullPath(candidate);
                    if (candidate.StartsWith(task + Path.DirectorySeparatorChar, StringComparison.OrdinalIgnoreCase) &&
                        string.Equals(Path.GetExtension(candidate), ".mp4", StringComparison.OrdinalIgnoreCase) && File.Exists(candidate) && new FileInfo(candidate).Length > 0)
                        path = candidate;
                }
                if (path == null) throw new InvalidOperationException("下载器未返回完整视频文件");
                if (File.Exists(Path.Combine(task, "cancel"))) throw new OperationCanceledException("已取消下载");
            }
        } catch (Exception e) { path = null; error = e.Message; }
        if (task == null || !Directory.Exists(task)) return;
        try {
            if (path == null) foreach (var file in Directory.GetFiles(task)) {
                if (Path.GetFileName(file) is not ("heartbeat" or "cancel")) File.Delete(file);
            }
            var result = JsonSerializer.Serialize(new { path, error });
            await File.WriteAllTextAsync(Path.Combine(task, "result.tmp"), result);
            File.Move(Path.Combine(task, "result.tmp"), Path.Combine(task, "result.json"), true);
        } catch { /* UWP reports a stopped helper when no result arrives. */ }
    }
}
