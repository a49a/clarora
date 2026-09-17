import { sha256 } from '@noble/hashes/sha256';
import { hmac } from '@noble/hashes/hmac';
import { bytesToHex } from '@noble/hashes/utils';
import { XMLParser } from 'fast-xml-parser';
import { getSetting, setSetting } from '../data/database';
import { FileSystem } from './platform';
import { SECRET_NAMES, isDevBuild, migrateSecret, secretStore } from './secrets';

export type StorageConfig = {
  provider: 's3' | 'oss'; endpoint: string; region: string; bucket: string;
  accessKeyId: string; secretAccessKey: string; sessionToken: string;
  prefix: string; pathStyle: boolean;
};
export const DEFAULT_STORAGE: StorageConfig = {
  provider: 's3', endpoint: '', region: '', bucket: '', accessKeyId: '',
  secretAccessKey: '', sessionToken: '', prefix: 'clarora', pathStyle: false,
};
export async function loadStorageConfig(): Promise<StorageConfig> {
  const value = await getSetting('object_storage_config');
  const config: StorageConfig = { ...DEFAULT_STORAGE, ...(value ? JSON.parse(value) : {}) };
  // __DEV__ 构建跳过钥匙串（重签名 ACL 弹窗会阻塞）；保险库只在 Release 启用。
  const store = isDevBuild() ? null : secretStore();
  if (store) {
    try {
      config.secretAccessKey = await migrateSecret(store, SECRET_NAMES.storageSecretAccessKey, config.secretAccessKey);
      config.sessionToken = await migrateSecret(store, SECRET_NAMES.storageSessionToken, config.sessionToken);
      await setSetting('object_storage_config', JSON.stringify({ ...config, secretAccessKey: '', sessionToken: '' }));
    } catch { /* 保险库写入失败：沿用数据库明文 */ }
  }
  return config;
}
export async function saveStorageConfig(config: StorageConfig): Promise<void> {
  validateStorageConfig(config);
  const store = isDevBuild() ? null : secretStore();
  const persisted: StorageConfig = { ...config };
  if (store) {
    // 密钥优先写入系统凭证保险库；写入被拒时退回数据库明文，配置不丢。
    try {
      await store.setSecret(SECRET_NAMES.storageSecretAccessKey, config.secretAccessKey);
      await store.setSecret(SECRET_NAMES.storageSessionToken, config.sessionToken);
      persisted.secretAccessKey = '';
      persisted.sessionToken = '';
    } catch { /* 沿用数据库明文 */ }
  }
  await setSetting('object_storage_config', JSON.stringify(persisted));
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
