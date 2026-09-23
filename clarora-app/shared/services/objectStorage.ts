import { sha256 } from '@noble/hashes/sha256';
import { hmac } from '@noble/hashes/hmac';
import { bytesToHex } from '@noble/hashes/utils';
import { XMLParser } from 'fast-xml-parser';
import { getSetting, setSetting } from '../data/database';
import { FileSystem } from './platform';
import { SECRET_NAMES, discardGeneration, isDevBuild, secretGeneration, secretStore, setSecretsAtomic, versionedSecretName } from './secrets';

export type StorageConfig = {
  provider: 's3' | 'oss'; endpoint: string; region: string; bucket: string;
  accessKeyId: string; secretAccessKey: string; sessionToken: string;
  prefix: string; pathStyle: boolean; secretGen?: number;
};
export const DEFAULT_STORAGE: StorageConfig = {
  provider: 's3', endpoint: '', region: '', bucket: '', accessKeyId: '',
  secretAccessKey: '', sessionToken: '', prefix: 'clarora', pathStyle: false,
};
// 配置读取、迁移与保存互斥:旧读取任务的迁移回写不得覆盖新保存的配置。
let storageConfigOp: Promise<unknown> = Promise.resolve();
function withStorageConfigLock<T>(action: () => Promise<T>): Promise<T> {
  const result = storageConfigOp.then(action);
  storageConfigOp = result.catch(() => {});
  return result;
}
export function loadStorageConfig(): Promise<StorageConfig> {
  return withStorageConfigLock(() => readStorageConfig());
}
async function readStorageConfig(): Promise<StorageConfig> {
  const value = await getSetting('object_storage_config');
  const config: StorageConfig = { ...DEFAULT_STORAGE, ...(value ? JSON.parse(value) : {}) };
  // __DEV__ 构建跳过钥匙串（重签名 ACL 弹窗会阻塞）；保险库只在 Release 启用。
  const store = isDevBuild() ? null : secretStore();
  if (store) {
    // 密钥按代读取:代数与端点同存于这一行,由数据库行保证成对。
    const generation = secretGeneration(config);
    const accessKeyName = versionedSecretName(SECRET_NAMES.storageSecretAccessKey, generation);
    const tokenName = versionedSecretName(SECRET_NAMES.storageSessionToken, generation);
    const hasPlaintext = !!(config.secretAccessKey || config.sessionToken);
    try {
      if (config.secretAccessKey) await store.setSecret(accessKeyName, config.secretAccessKey);
      else config.secretAccessKey = (await store.getSecret(accessKeyName)) ?? '';
      if (config.sessionToken) await store.setSecret(tokenName, config.sessionToken);
      else config.sessionToken = (await store.getSecret(tokenName)) ?? '';
      if (hasPlaintext) {
        // 只有旧明文行需要迁移回写,且回写前确认行未被并发更新,
        // 避免旧读取任务把刚保存的新配置(引用已清理的旧代)盖回去。
        const current = await getSetting('object_storage_config');
        if (current === value) {
          await setSetting('object_storage_config', JSON.stringify({ ...config, secretAccessKey: '', sessionToken: '' }));
        } else {
          console.warn('存储配置在读取期间被更新,跳过明文迁移回写');
        }
      }
    } catch (error) {
      // 旧值迁移失败:数据库里的既有明文仍可读(不丢配置),但迁移状态要可见。
      console.warn('凭证迁移到系统保险库失败,继续使用数据库旧值:', (error as Error).message ?? error);
    }
  }
  return config;
}
export function saveStorageConfig(config: StorageConfig): Promise<void> {
  return withStorageConfigLock(() => persistStorageConfig(config));
}
async function persistStorageConfig(config: StorageConfig): Promise<void> {
  validateStorageConfig(config);
  const store = isDevBuild() ? null : secretStore();
  if (!isDevBuild() && !store) {
    throw new Error('本机系统凭证保险库不可用,为避免密钥明文落库,已拒绝保存;请更新安装包以包含凭证模块');
  }
  // 原子切换方案:先按代暂存新一代密钥,全部就绪后由数据库行一次性切换
  // 端点与 secretGen 引用。提交点只有这一行 SQLite 写入,任何失败路径
  // (含崩溃)都不会出现"端点与密钥来自不同代"。
  // 读取旧配置失败必须中止:猜错代数会让暂存覆盖正在使用的密钥。
  const previousRaw = await getSetting('object_storage_config').catch(() => undefined);
  if (previousRaw === undefined) {
    throw new Error('读取当前存储配置失败,为避免密钥错代,已中止保存,请重试');
  }
  let previousGen = 0;
  if (previousRaw) {
    try {
      previousGen = secretGeneration(JSON.parse(previousRaw) as StorageConfig);
    } catch {
      throw new Error('当前存储配置无法解析,为避免密钥错代,已中止保存,请重试');
    }
  }
  const persisted: StorageConfig = { ...config };
  const nextGen = store ? previousGen + 1 : 0;
  if (store) {
    persisted.secretGen = nextGen;
    persisted.secretAccessKey = '';
    persisted.sessionToken = '';
  }
  const serialized = JSON.stringify(persisted);
  const stagedNames = [SECRET_NAMES.storageSecretAccessKey, SECRET_NAMES.storageSessionToken] as const;

  // 1) 暂存新一代密钥:写失败(含部分失败)只留下无引用的暂存数据,
  //    数据库仍引用上一代,直接中止即可,无需任何回滚。
  if (store) {
    try {
      await setSecretsAtomic(store, [
        { name: versionedSecretName(SECRET_NAMES.storageSecretAccessKey, nextGen), value: config.secretAccessKey },
        { name: versionedSecretName(SECRET_NAMES.storageSessionToken, nextGen), value: config.sessionToken },
      ]);
    } catch (error) {
      await discardGeneration(store, stagedNames, nextGen);
      throw new Error(`系统凭证保险库写入失败,配置未变更,请重试:${(error as Error).message ?? error}`);
    }
  }

  // 2) 提交点:端点与密钥代数同处一行,SQLite 单写原子生效。
  await setSetting('object_storage_config', serialized);
  const readOnce = async (): Promise<{ ok: boolean; value: string | null }> => {
    try { return { ok: true, value: await getSetting('object_storage_config') }; } catch { return { ok: false, value: null }; }
  };
  const first = await readOnce();
  if (!first.ok || first.value !== serialized) {
    const second = await readOnce();
    if (!(second.ok && second.value === serialized)) {
      // 提交结果不确定(回读失败)时保留暂存的新代——它可能已经生效;
      // 只有确认未提交(读到旧行)才清理。
      if (store && second.ok) await discardGeneration(store, stagedNames, nextGen);
      throw new Error('存储配置保存校验失败,请重试;输入内容已保留');
    }
  }

  // 3) 旧代已无引用:尽力清理,失败只留下垃圾数据,不影响正确性。
  if (store) await discardGeneration(store, stagedNames, previousGen);
}
export function validateStorageConfig(config: StorageConfig): void {
  if (!['s3', 'oss'].includes(config.provider)) throw new Error('不支持的存储类型');
  if (!/^https:\/\/[a-z0-9.-]+(?::\d+)?\/?$/i.test(config.endpoint)) {
    throw new Error('Endpoint 请填写 HTTPS 服务域名，不含 Bucket、路径或查询参数');
  }
  if (!/^[a-z0-9][a-z0-9.-]{1,61}[a-z0-9]$/.test(config.bucket)) throw new Error('Bucket 格式不正确');
  if (!/^[a-z0-9-]+$/.test(config.region)) throw new Error('请填写 Region');
  if (!config.accessKeyId || !config.secretAccessKey || /[\r\n]/.test(config.accessKeyId + config.secretAccessKey + config.sessionToken)) throw new Error('请填写有效的访问密钥');
  // eslint-disable-next-line no-control-regex -- 控制字符正是要拦截的对象
  if (!config.prefix || config.prefix.split('/').some(p => !p || p === '.' || p === '..') || /[\\\x00-\x1f]/.test(config.prefix)) throw new Error('资料库前缀不能为空，不能包含空目录或 ..');
}
// Hermes versions without TextEncoder still need correct UTF-8 signing bytes.
const utf8ToBytes = (text: string) => Uint8Array.from(encodeURIComponent(text).replace(/%([0-9A-F]{2})/g, (_, hex) => String.fromCharCode(parseInt(hex, 16))), c => c.charCodeAt(0));
const encode = (value: string) => encodeURIComponent(value).replace(/[!'()*]/g, c => `%${c.charCodeAt(0).toString(16).toUpperCase()}`);
const pathEncode = (value: string) => value.split('/').map(encode).join('/');
const digest = (value: string) => bytesToHex(sha256(utf8ToBytes(value)));
const mac = (key: string | Uint8Array, value: string) => hmac(sha256, typeof key === 'string' ? utf8ToBytes(key) : key, utf8ToBytes(value));

/** Pure SigV4 signer: OSS uses its own scope and bucket-prefixed canonical URI. */
export function signStorageRequest(config: StorageConfig, method: string, key = '', query: Record<string, string> = {}, now = new Date(), contentType = '') {
  validateStorageConfig(config);
  const oss = config.provider === 'oss';
  const host = config.endpoint.replace(/^https:\/\//, '').replace(/\/$/, '');
  const virtual = oss || !config.pathStyle;
  const requestHost = virtual ? `${config.bucket}.${host}` : host;
  const path = pathEncode(`${virtual ? '' : '/' + config.bucket}/${key}`);
  const canonicalPath = oss ? pathEncode(`/${config.bucket}/${key}`) : path;
  const queryString = Object.entries(query).map(([k, v]) => [encode(k), encode(v)]).sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0).map(([k, v]) => `${k}=${v}`).join('&');
  const timestamp = now.toISOString().replace(/[:-]|\.\d{3}/g, '');
  const date = timestamp.slice(0, 8);
  const prefix = oss ? 'x-oss' : 'x-amz';
  const headers: Record<string, string> = {
    [`${prefix}-date`]: timestamp,
    [`${prefix}-content-sha256`]: 'UNSIGNED-PAYLOAD',
  };
  if (contentType) headers['content-type'] = contentType;
  if (config.sessionToken) headers[`${prefix}-security-token`] = config.sessionToken;
  const canonicalHeaders: Record<string, string> = { ...headers, ...(!oss ? { host: requestHost } : {}) };
  const names = Object.keys(canonicalHeaders).sort();
  const signedHeaders = oss ? '' : names.join(';');
  const canonical = [method, canonicalPath, queryString, names.map(k => `${k}:${canonicalHeaders[k].trim()}\n`).join(''), signedHeaders, 'UNSIGNED-PAYLOAD'].join('\n');
  const service = oss ? 'oss' : 's3';
  const terminator = oss ? 'aliyun_v4_request' : 'aws4_request';
  const algorithm = oss ? 'OSS4-HMAC-SHA256' : 'AWS4-HMAC-SHA256';
  const scope = `${date}/${config.region}/${service}/${terminator}`;
  const signingKey = mac(mac(mac(mac(`${oss ? 'aliyun_v4' : 'AWS4'}${config.secretAccessKey}`, date), config.region), service), terminator);
  const signature = bytesToHex(mac(signingKey, [algorithm, timestamp, scope, digest(canonical)].join('\n')));
  headers.Authorization = `${algorithm} Credential=${config.accessKeyId}/${scope}, ${oss ? '' : `SignedHeaders=${signedHeaders}, `}Signature=${signature}`;
  return { url: `https://${requestHost}${path}${queryString ? '?' + queryString : ''}`, headers };
}

export class ObjectStorage {
  constructor(readonly config: StorageConfig) { validateStorageConfig(config); }
  key(relative: string) { return `${this.config.prefix}/${relative}`; }
  async request(method: string, key = '', query: Record<string, string> = {}, body?: string) {
    const signed = signStorageRequest(this.config, method, key, query, new Date(), body === undefined ? '' : 'application/json');
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), 60000);
    try {
      const response = await fetch(signed.url, { method, headers: signed.headers, body, signal: controller.signal, redirect: 'error' });
      if (!response.ok) throw new Error(`存储请求失败（HTTP ${response.status}）。请检查 Endpoint、Region、密钥和 Bucket 权限。`);
      return await response.text();
    } finally { clearTimeout(timer); }
  }
  async putFile(key: string, uri: string) {
    const signed = signStorageRequest(this.config, 'PUT', key, {}, new Date(), 'application/octet-stream');
    const result = await FileSystem.putFileAsync(signed.url, uri, signed.headers);
    if (result.status < 200 || result.status >= 300) throw new Error(`附件上传失败（HTTP ${result.status}），备份未提交`);
  }
  async getFile(key: string, uri: string) {
    const signed = signStorageRequest(this.config, 'GET', key);
    await FileSystem.downloadFileAsync(signed.url, uri, signed.headers);
  }
  async list(prefix: string): Promise<string[]> {
    const keys: string[] = [];
    let token = '';
    const seen = new Set<string>();
    const parser = new XMLParser({ ignoreAttributes: true, parseTagValue: false, processEntities: true });
    do {
      const xml = await this.request('GET', '', { 'list-type': '2', prefix, ...(token ? { 'continuation-token': token } : {}) });
      if (/<!DOCTYPE|<!ENTITY/i.test(xml)) throw new Error('存储返回了不支持的 XML');
      const result = parser.parse(xml)?.ListBucketResult;
      if (!result) throw new Error('存储未返回有效的对象列表');
      const contents = result.Contents ? (Array.isArray(result.Contents) ? result.Contents : [result.Contents]) : [];
      for (const item of contents) if (typeof item.Key === 'string' && item.Key.startsWith(prefix)) keys.push(item.Key);
      const truncated = String(result.IsTruncated) === 'true';
      token = truncated ? String(result.NextContinuationToken || '') : '';
      if (truncated && (!token || seen.has(token))) throw new Error('存储分页响应异常');
      seen.add(token);
    } while (token);
    return keys.sort();
  }
}
