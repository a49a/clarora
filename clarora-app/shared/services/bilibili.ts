import { getNativeModules, currentPlatform } from './platform';

type Downloader = { download(url: string): Promise<string>; cancel(): void };

/** Accept a video URL, copied share text or a BV/av identifier. Discard tracking parameters. */
export function normalizeBilibiliInput(input: string): string {
  const text = input.trim();
  const id = /^(BV[0-9A-Za-z]{10}|av[1-9][0-9]*)$/.exec(text)?.[1];
  if (id) return `https://www.bilibili.com/video/${id}`;
  const link = text.match(/https:\/\/[^\s<>「」【】]+/)?.[0]?.replace(/[，。；！、）)]+$/, '');
  if (!link) throw new Error('请输入 bilibili 视频链接或 BV 号');
  // A strict grammar also works on React Native versions without full URL support.
  const match = /^https:\/\/(?:www\.|m\.)?bilibili\.com\/video\/(BV[0-9A-Za-z]{10}|av[1-9][0-9]*)\/?(?:\?([^#]*))?(?:#.*)?$/.exec(link);
  if (!match) throw new Error('仅支持 bilibili.com/video/ 视频链接；短链接请在浏览器打开后复制完整链接');
  const pages = (match[2] || '').split('&').filter(part => part.split('=')[0] === 'p');
  if (pages.length > 1 || (pages.length === 1 && !/^p=[1-9][0-9]{0,4}$/.test(pages[0]))) {
    throw new Error('分 P 参数无效，请填写正整数 p');
  }
  return `https://www.bilibili.com/video/${match[1]}${pages.length ? `?${pages[0]}` : ''}`;
}

function downloader(): Downloader {
  if (currentPlatform !== 'macos' && currentPlatform !== 'windows') throw new Error('bilibili 下载仅支持 macOS 和 Windows');
  const module = getNativeModules().RNBilibiliDownloader as Downloader | undefined;
  if (!module?.download) throw new Error('下载模块未加载，请重新构建桌面客户端');
  return module;
}

export function downloadBilibiliVideo(input: string): Promise<string> {
  return downloader().download(normalizeBilibiliInput(input));
}

export function cancelBilibiliDownload(): void {
  (getNativeModules().RNBilibiliDownloader as Downloader | undefined)?.cancel();
}
