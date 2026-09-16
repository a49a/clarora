import { FileSystem, currentPlatform, getNativeModules, nativePath } from './platform';

export type PdfOutlineNode = { title: string; page: number; children: PdfOutlineNode[] };
export type PdfMeta = { path: string; pages: Array<{ width: number; height: number }>; outline: PdfOutlineNode[] };
export type LibraryEntry = { path: string; name: string };

// 原生模块返回 JSON 字符串（Windows C#）或已解析的对象（macOS 字典）；包装层统一为对象。
type RawNativePdf = {
  open: (path: string) => Promise<string | { pages: Array<{ width: number; height: number }>; outline: PdfOutlineNode[] }>;
  renderPage: (path: string, index: number, width: number) => Promise<string | { png: string }>;
};
type NativePdf = {
  open: (path: string) => Promise<{ pages: Array<{ width: number; height: number }>; outline: PdfOutlineNode[] }>;
  renderPage: (path: string, index: number, width: number) => Promise<{ png: string }>;
};

function parseNative<T>(value: string | T): T {
  return typeof value === 'string' ? JSON.parse(value) : value;
}

function nativePdf(): NativePdf {
  if (currentPlatform === 'macos') {
    const module = getNativeModules().RNMacPdf as RawNativePdf | undefined;
    if (!module?.open) throw new Error('PDF 阅读目前支持 macOS 和 Windows');
    return {
      open: async path => parseNative(await module.open(path)),
      renderPage: async (path, index, width) => parseNative(await module.renderPage(path, index, width)),
    };
  }
  if (currentPlatform === 'windows') {
    const module = getNativeModules().RNWindowsPdf as RawNativePdf | undefined;
    if (!module?.open) throw new Error('Windows PDF 阅读依赖 pdfium.dll，请按平台文档运行构建脚本');
    return {
      open: async path => parseNative(await module.open(path)),
      renderPage: async (path, index, width) => parseNative(await module.renderPage(path, index, width)),
    };
  }
  throw new Error('PDF 阅读目前支持 macOS 和 Windows');
}

export async function pdfLibraryDir() {
  const dir = (await FileSystem.getDocumentDirectoryAsync()) + 'pdf/';
  await FileSystem.makeDirectoryAsync(dir).catch(() => {});
  return dir;
}

export async function listPdfLibrary(): Promise<LibraryEntry[]> {
  const dir = await pdfLibraryDir();
  const files = await FileSystem.listFilesAsync(dir).catch(() => [] as string[]);
  return files
    .filter(file => file.toLowerCase().endsWith('.pdf'))
    .map(file => ({ path: file, name: decodeURIComponent(file.split('/').pop() || file).replace(/\.pdf$/i, '') }))
    .sort((a, b) => a.name.localeCompare(b.name));
}

export async function importPdf(uri: string, name: string): Promise<LibraryEntry> {
  if (!name.toLowerCase().endsWith('.pdf')) throw new Error('请选择 PDF 文件');
  const dir = await pdfLibraryDir();
  const safe = `${Date.now()}_${name.replace(/[\\/:*?"<>|]/g, '_').toLowerCase()}`;
  const destination = `${dir}${safe.endsWith('.pdf') ? safe : `${safe}.pdf`}`;
  await FileSystem.copyAsync({ from: uri, to: destination });
  return { path: destination, name: decodeURIComponent(destination.split('/').pop() || '').replace(/\.pdf$/i, '') };
}

export async function openPdf(path: string): Promise<PdfMeta> {
  const result = await nativePdf().open(nativePath(path));
  return { path, pages: result.pages ?? [], outline: result.outline ?? [] };
}

const renderCache = new Map<string, string>();
const RENDER_CACHE_LIMIT = 40;

export async function renderPdfPage(path: string, index: number, width: number): Promise<string> {
  const key = `${path}#${index}@${width}`;
  const cached = renderCache.get(key);
  if (cached) return cached;
  const result = await nativePdf().renderPage(nativePath(path), index, Math.round(width));
  const dataUri = `data:image/png;base64,${result.png}`;
  if (renderCache.size >= RENDER_CACHE_LIMIT) {
    const oldest = renderCache.keys().next().value;
    if (oldest !== undefined) renderCache.delete(oldest);
  }
  renderCache.set(key, dataUri);
  return dataUri;
}
