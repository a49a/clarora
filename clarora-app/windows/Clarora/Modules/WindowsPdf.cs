using Microsoft.ReactNative.Managed;
using System;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading.Tasks;
using Windows.Graphics.Imaging;
using Windows.Storage.Streams;

namespace Clarora
{
    /// <summary>
    /// PDF 阅读（Windows）：P/Invoke pdfium.dll（Chrome 同源渲染引擎）。
    /// Open 返回页面尺寸与目录树 JSON；RenderPage 把整页渲染为 PNG base64，
    /// 与 macOS 的 RNMacPdf 共享同一接口，由 JS 阅读器排版。
    /// pdfium.dll 由 scripts/build-windows-asr.ps1 下载并随应用打包。
    /// </summary>
    [ReactModule("RNWindowsPdf")]
    public sealed class WindowsPdf
    {
        private const string Dll = "pdfium";

        [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
        private static extern void FPDF_InitLibrary();

        [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
        private static extern IntPtr FPDF_LoadDocument(
            [MarshalAs(UnmanagedType.LPUTF8Str)] string path, [MarshalAs(UnmanagedType.LPUTF8Str)] string password);

        [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
        private static extern void FPDF_CloseDocument(IntPtr document);

        [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
        private static extern int FPDF_GetPageCount(IntPtr document);

        [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
        private static extern IntPtr FPDF_LoadPage(IntPtr document, int index);

        [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
        private static extern void FPDF_ClosePage(IntPtr page);

        [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
        private static extern float FPDF_GetPageWidthF(IntPtr page);

        [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
        private static extern float FPDF_GetPageHeightF(IntPtr page);

        [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
        private static extern IntPtr FPDFBitmap_Create(int width, int height, int alpha);

        [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
        private static extern void FPDFBitmap_FillRect(IntPtr bitmap, int left, int top, int width, int height, uint color);

        [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
        private static extern IntPtr FPDFBitmap_GetBuffer(IntPtr bitmap);

        [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
        private static extern void FPDFBitmap_Destroy(IntPtr bitmap);

        [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
        private static extern void FPDF_RenderPageBitmap(IntPtr bitmap, IntPtr page, int start_x, int start_y, int size_x, int size_y, int rotate, int flags);

        [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
        private static extern IntPtr FPDF_BOOKMARK_GetFirstChild(IntPtr bookmark);

        [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
        private static extern IntPtr FPDF_BOOKMARK_GetNextSibling(IntPtr document, IntPtr bookmark);

        [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
        private static extern IntPtr FPDF_BOOKMARK_GetDest(IntPtr document, IntPtr bookmark);

        [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
        private static extern int FPDFDest_GetDestPageIndex(IntPtr document, IntPtr dest);

        [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
        private static extern ulong FPDF_BOOKMARK_GetTitle(IntPtr bookmark, IntPtr buffer, ulong length);

        private static readonly object Gate = new object();
        private static bool initialized;
        private static IntPtr document = IntPtr.Zero;
        private static string documentPath = string.Empty;

        private static void EnsureLibrary()
        {
            lock (Gate)
            {
                if (!initialized)
                {
                    FPDF_InitLibrary();
                    initialized = true;
                }
            }
        }

        private static IntPtr LoadDocument(string path)
        {
            EnsureLibrary();
            lock (Gate)
            {
                if (document != IntPtr.Zero && documentPath == path) return document;
                if (document != IntPtr.Zero) FPDF_CloseDocument(document);
                document = FPDF_LoadDocument(path, null);
                documentPath = document != IntPtr.Zero ? path : string.Empty;
                if (document == IntPtr.Zero) throw new InvalidOperationException("无法读取 PDF 文件（可能已加密或损坏）");
                return document;
            }
        }

        private static string Utf16(IntPtr pointer)
        {
            if (pointer == IntPtr.Zero) return string.Empty;
            int length = 0;
            while (Marshal.ReadInt16(pointer, length * 2) != 0) length++;
            var bytes = new byte[length * 2];
            Marshal.Copy(pointer, bytes, 0, bytes.Length);
            return Encoding.Unicode.GetString(bytes);
        }

        private static string BookmarkTitle(IntPtr bookmark)
        {
            var buffer = Marshal.AllocHGlobal(4096);
            try
            {
                // UTF-16LE，最长 2048 字符；超长标题按平台行为截断即可。
                FPDF_BOOKMARK_GetTitle(bookmark, buffer, 4096);
                return Utf16(buffer);
            }
            finally
            {
                Marshal.FreeHGlobal(buffer);
            }
        }

        private static void CollectOutline(IntPtr document, IntPtr bookmark, StringBuilder json)
        {
            while (bookmark != IntPtr.Zero)
            {
                if (json[json.Length - 1] != '[') json.Append(",");
                json.Append("{\"title\":").Append(JsonEscape(BookmarkTitle(bookmark)));
                var dest = FPDF_BOOKMARK_GetDest(document, bookmark);
                int page = dest != IntPtr.Zero ? FPDFDest_GetDestPageIndex(document, dest) : -1;
                json.Append(",\"page\":").Append(page);
                json.Append(",\"children\":[");
                CollectOutline(document, FPDF_BOOKMARK_GetFirstChild(bookmark), json);
                json.Append("]}");
                bookmark = FPDF_BOOKMARK_GetNextSibling(document, bookmark);
            }
        }

        private static string JsonEscape(string text)
        {
            var builder = new StringBuilder(text.Length + 8);
            foreach (var character in text)
            {
                switch (character)
                {
                    case '"': builder.Append("\\\""); break;
                    case '\\': builder.Append("\\\\"); break;
                    case '\n': builder.Append("\\n"); break;
                    case '\r': builder.Append("\\r"); break;
                    case '\t': builder.Append("\\t"); break;
                    default:
                        if (character < 0x20) builder.AppendFormat("\\u{0:x4}", (int)character);
                        else builder.Append(character);
                        break;
                }
            }
            return builder.ToString();
        }

        [ReactMethod("open")]
        public async Task<string> Open(string path)
        {
            return await Task.Run(() =>
            {
                var doc = LoadDocument(path);
                int count = FPDF_GetPageCount(doc);
                var json = new StringBuilder("{\"pages\":[");
                for (int i = 0; i < count; i++)
                {
                    var page = FPDF_LoadPage(doc, i);
                    if (i > 0) json.Append(",");
                    json.Append("{\"width\":").Append(FPDF_GetPageWidthF(page).ToString("0.##"))
                        .Append(",\"height\":").Append(FPDF_GetPageHeightF(page).ToString("0.##")).Append("}");
                    FPDF_ClosePage(page);
                }
                json.Append("],\"outline\":[");
                CollectOutline(doc, FPDF_BOOKMARK_GetFirstChild(IntPtr.Zero), json);
                json.Append("]}");
                return json.ToString();
            });
        }

        [ReactMethod("renderPage")]
        public async Task<string> RenderPage(string path, int index, double width)
        {
            return await Task.Run(() => RenderPageSync(path, index, width));
        }

        private string RenderPageSync(string path, int index, double width)
        {
            var doc = LoadDocument(path);
            var page = FPDF_LoadPage(doc, index);
            if (page == IntPtr.Zero) throw new InvalidOperationException("页面渲染失败");
            try
            {
                float widthPt = FPDF_GetPageWidthF(page);
                float heightPt = FPDF_GetPageHeightF(page);
                int pixelWidth = Math.Max(1, (int)Math.Round(width));
                int pixelHeight = Math.Max(1, (int)Math.Round(heightPt * pixelWidth / widthPt));
                var bitmap = FPDFBitmap_Create(pixelWidth, pixelHeight, 0);
                if (bitmap == IntPtr.Zero) throw new InvalidOperationException("页面渲染失败");
                try
                {
                    FPDFBitmap_FillRect(bitmap, 0, 0, pixelWidth, pixelHeight, 0xFFFFFFFF);
                    FPDF_RenderPageBitmap(bitmap, page, 0, 0, pixelWidth, pixelHeight, 0, 0);
                    return ConvertToPngBase64(FPDFBitmap_GetBuffer(bitmap), pixelWidth, pixelHeight);
                }
                finally
                {
                    FPDFBitmap_Destroy(bitmap);
                }
            }
            finally
            {
                FPDF_ClosePage(page);
            }
        }

        /// <summary>pdfium 输出 BGRA；转 RGBA 后经 BitmapEncoder 编码为 PNG base64。</summary>
        private static async System.Threading.Tasks.Task<string> ConvertToPngBase64Async(IntPtr buffer, int width, int height)
        {
            int byteCount = width * height * 4;
            var pixels = new byte[byteCount];
            Marshal.Copy(buffer, pixels, 0, byteCount);
            for (int i = 0; i < pixels.Length; i += 4)
            {
                var blue = pixels[i];
                pixels[i] = pixels[i + 2];
                pixels[i + 2] = blue;
            }

            var stream = new InMemoryRandomAccessStream();
            var encoder = await BitmapEncoder.CreateAsync(BitmapEncoder.PngEncoderId, stream);
            encoder.SetPixelData(BitmapPixelFormat.Rgba8, BitmapAlphaMode.Ignore, (uint)width, (uint)height, 96, 96, pixels);
            await encoder.FlushAsync();

            var bytes = new byte[stream.Size];
            var reader = new DataReader(stream.GetInputStreamAt(0));
            await reader.LoadAsync((uint)stream.Size);
            reader.ReadBytes(bytes);
            return Convert.ToBase64String(bytes);
        }

        private string ConvertToPngBase64(IntPtr buffer, int width, int height)
        {
            return ConvertToPngBase64Async(buffer, width, height).GetAwaiter().GetResult();
        }
    }
}
