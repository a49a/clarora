import { exportVaultData, importVaultData } from '../data/database';
import { newVaultId, validateVault, type VaultData } from '../data/vault';
import { FileSystem } from './platform';
import { ObjectStorage, loadStorageConfig } from './objectStorage';

type Manifest = { version: 2; id: string; createdAt: string; data: VaultData; files: Record<string, string> };
export type BackupInfo = { key: string; id: string };
export type SyncSummary = { words: number; practices: number; audios: number; clips: number; aiCards: number; sentenceCards: number; schedules: number; files: number };
let running = false;
async function exclusive<T>(action: () => Promise<T>): Promise<T> {
  if (running) throw new Error('已有同步任务正在进行');
  running = true;
  try { return await action(); } finally { running = false; }
}
function summary(data: VaultData, files: number): SyncSummary {
  return { words: data.words.length, practices: data.listening_practices.length, audios: data.listening_audios.length,
    clips: data.clip_cards.length, aiCards: data.ai_cards.length, sentenceCards: data.sentence_cards.length, schedules: data.review_schedule.length, files };
}
/** Rewrite only known media fields; arbitrary text is never interpreted as a local file. */
export async function mapVaultMedia(data: VaultData, map: (uri: string) => Promise<string>): Promise<void> {
  for (const [table, fields] of [['listening_audios', ['audio_uri', 'subtitle_uri']], ['clip_cards', ['audio_uri']], ['video_clips', ['video_uri']]] as const) {
    for (const row of data[table]) for (const field of fields) if (row[field]) row[field] = await map(String(row[field]));
  }
  for (const row of data.discover_favorites) {
    const payload = JSON.parse(String(row.payload));
    if (payload.audio?.uri) payload.audio.uri = await map(String(payload.audio.uri));
    row.payload = JSON.stringify(payload);
  }
}
export async function listBackups(): Promise<BackupInfo[]> {
  const store = new ObjectStorage(await loadStorageConfig());
  const prefix = store.key('snapshots/');
  return (await store.list(prefix)).filter(key => /^[a-z0-9-]+\.json$/.test(key.slice(prefix.length)))
    .reverse().map(key => ({ key, id: key.slice(prefix.length, -5) }));
}
export async function uploadLibrary(onProgress?: (message: string) => void): Promise<SyncSummary> {
  return exclusive(async () => {
    const store = new ObjectStorage(await loadStorageConfig());
    const id = newVaultId();
    const data = await exportVaultData();
    const files: Record<string, string> = {};
    const byUri = new Map<string, string>();
    await mapVaultMedia(data, async uri => {
      if (byUri.has(uri)) return byUri.get(uri)!;
      const ext = /\.([A-Za-z0-9]{1,8})$/.exec(uri)?.[1] ?? 'bin';
      const reference = `media:${byUri.size}`;
      const key = store.key(`media/${id}/${byUri.size}.${ext}`);
      onProgress?.(`正在上传附件 ${byUri.size + 1}…`);
      await store.putFile(key, uri);
      files[reference] = key;
      byUri.set(uri, reference);
      return reference;
    });
    const manifest: Manifest = { version: 2, id, createdAt: new Date().toISOString(), data, files };
    onProgress?.('正在提交备份清单…');
    // An immutable manifest is the commit point: interrupted media uploads are never listed as backups.
    await store.request('PUT', store.key(`snapshots/${id}.json`), {}, JSON.stringify(manifest));
    return summary(data, byUri.size);
  });
}
export async function downloadLibrary(backupKey: string, onProgress?: (message: string) => void): Promise<SyncSummary> {
  return exclusive(async () => {
    const store = new ObjectStorage(await loadStorageConfig());
    const prefix = store.key('snapshots/');
    if (!backupKey.startsWith(prefix) || !/^[a-z0-9-]+\.json$/.test(backupKey.slice(prefix.length))) throw new Error('备份路径无效');
    const manifest = JSON.parse(await store.request('GET', backupKey)) as Manifest;
    if (manifest.version !== 2 || !/^[a-z0-9-]+$/.test(manifest.id) || backupKey !== `${prefix}${manifest.id}.json`) throw new Error('不支持的备份版本或标识');
    validateVault(manifest.data);
    if (!manifest.files || typeof manifest.files !== 'object' || Array.isArray(manifest.files)) throw new Error('附件清单无效');
    const mediaPrefix = store.key(`media/${manifest.id}/`);
    const keys = Object.values(manifest.files);
    for (const key of keys) if (typeof key !== 'string' || !key.startsWith(mediaPrefix) || !/^\d+\.[A-Za-z0-9]{1,8}$/.test(key.slice(mediaPrefix.length))) throw new Error('附件路径无效');
    // Check every reference before downloading or modifying the database.
    await mapVaultMedia(manifest.data, async reference => {
      if (!/^media:\d+$/.test(reference) || !Object.hasOwnProperty.call(manifest.files, reference)) throw new Error('备份缺少附件');
      return reference;
    });
    const root = `${await FileSystem.getDocumentDirectoryAsync()}Clarora/Restored/${newVaultId()}`;
    await FileSystem.makeDirectoryAsync(root);
    const local = new Map<string, string>();
    const downloaded: string[] = [];
    try {
      await mapVaultMedia(manifest.data, async reference => {
        if (local.has(reference)) return local.get(reference)!;
        const key = manifest.files[reference];
        const target = `${root}/${key.slice(mediaPrefix.length)}`;
        downloaded.push(target);
        onProgress?.(`正在下载附件 ${local.size + 1}/${keys.length}…`);
        await store.getFile(key, target);
        local.set(reference, target);
        return target;
      });
      onProgress?.('正在合并到本机资料库…');
      await importVaultData(manifest.data);
      return summary(manifest.data, local.size);
    } catch (error) {
      for (const file of downloaded) await FileSystem.deleteAsync(file).catch(() => {});
      throw error;
    }
  });
}
