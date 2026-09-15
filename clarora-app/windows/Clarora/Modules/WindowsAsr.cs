using Microsoft.ReactNative.Managed;
using System;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading.Tasks;

namespace Clarora
{
    /// <summary>
    /// 端侧转写（Windows）：P/Invoke clarora_asr.dll，包装 whisper.cpp 与
    /// sherpa-onnx SenseVoice。返回 JSON 字符串
    /// {"duration":秒,"segments":[{"start","end","text"}]}，由 JS 解析。
    /// DLL 缺失时抛出可操作的错误，应用其余功能不受影响。
    /// </summary>
    [ReactModule("RNWindowsAsr")]
    public sealed class WindowsAsr
    {
        [DllImport("clarora_asr", CallingConvention = CallingConvention.Cdecl)]
        private static extern IntPtr ClaroraWhisperTranscribe(
            [MarshalAs(UnmanagedType.LPUTF8Str)] string audioPath,
            [MarshalAs(UnmanagedType.LPUTF8Str)] string modelPath,
            [MarshalAs(UnmanagedType.LPUTF8Str)] string language);

        [DllImport("clarora_asr", CallingConvention = CallingConvention.Cdecl)]
        private static extern IntPtr ClaroraSenseVoiceTranscribe(
            [MarshalAs(UnmanagedType.LPUTF8Str)] string audioPath,
            [MarshalAs(UnmanagedType.LPUTF8Str)] string modelPath,
            [MarshalAs(UnmanagedType.LPUTF8Str)] string tokensPath);

        [DllImport("clarora_asr", CallingConvention = CallingConvention.Cdecl)]
        private static extern void ClaroraFreeString(IntPtr pointer);

        [ReactMethod("transcribeWhisper")]
        public Task<string> TranscribeWhisper(string audioPath, string modelPath, string language) =>
            Call(() => ClaroraWhisperTranscribe(audioPath, modelPath, language));

        [ReactMethod("transcribeSenseVoice")]
        public Task<string> TranscribeSenseVoice(string audioPath, string modelPath, string tokensPath) =>
            Call(() => ClaroraSenseVoiceTranscribe(audioPath, modelPath, tokensPath));

        private static Task<string> Call(Func<IntPtr> invoke)
        {
            return Task.Run(() =>
            {
                IntPtr pointer;
                try
                {
                    pointer = invoke();
                }
                catch (DllNotFoundException)
                {
                    throw new InvalidOperationException("Windows 端侧转写库尚未安装（clarora_asr.dll），请阅读平台文档后运行 scripts/build-windows-asr.ps1");
                }
                if (pointer == IntPtr.Zero) throw new InvalidOperationException("端侧转写返回了空结果");
                try
                {
                    int length = 0;
                    while (Marshal.ReadByte(pointer, length) != 0) length++;
                    var bytes = new byte[length];
                    Marshal.Copy(pointer, bytes, 0, length);
                    var json = Encoding.UTF8.GetString(bytes);
                    if (json.Contains("\"error\":")) throw new InvalidOperationException(ParseError(json));
                    return json;
                }
                finally
                {
                    ClaroraFreeString(pointer);
                }
            });
        }

        private static string ParseError(string json)
        {
            // 轻量提取 {"error":"…"}，避免为一条错误引入 JSON 依赖。
            const string key = "\"error\":\"";
            int start = json.IndexOf(key, StringComparison.Ordinal);
            if (start < 0) return "端侧转写失败，请重试";
            start += key.Length;
            var builder = new StringBuilder();
            for (int i = start; i < json.Length && json[i] != '"'; i++)
            {
                if (json[i] == '\\' && i + 1 < json.Length) i++;
                builder.Append(json[i]);
            }
            return builder.ToString();
        }
    }
}
