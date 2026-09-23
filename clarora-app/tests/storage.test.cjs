const { test } = require('node:test');
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const path = require('node:path');
const loader = require('./helpers/load-ts.cjs');
const file = path.join(__dirname, '../shared/services/objectStorage.ts');
const config = { provider: 's3', endpoint: 'https://s3.example.com', region: 'us-east-1', bucket: 'examplebucket', accessKeyId: 'test-access', secretAccessKey: 'test-secret', sessionToken: '', prefix: '学习', pathStyle: false };
const hmac = (key, text) => crypto.createHmac('sha256', key).update(text).digest();
function expectedSignature(prefix, service, end, canonical) {
  const scope = `20260914/us-east-1/${service}/${end}`;
  const key = [ '20260914', 'us-east-1', service, end ].reduce((key, value) => hmac(key, value), prefix + config.secretAccessKey);
  return hmac(key, `${service === 's3' ? 'AWS4' : 'OSS4'}-HMAC-SHA256\n20260914T010203Z\n${scope}\n${crypto.createHash('sha256').update(canonical).digest('hex')}`).toString('hex');
}
test('S3 signature binds Unicode object paths, sorted query and session token without a Node crypto runtime', () => {
  const { signStorageRequest } = loader({ '../data/database': {}, './platform': {} })(file);
  const signed = signStorageRequest({ ...config, sessionToken: 'session-token' }, 'GET', '学习/a +.txt', { z: '/', a: '+' }, new Date('2026-09-14T01:02:03Z'));
  const canonical = 'GET\n/%E5%AD%A6%E4%B9%A0/a%20%2B.txt\na=%2B&z=%2F\nhost:examplebucket.s3.example.com\nx-amz-content-sha256:UNSIGNED-PAYLOAD\nx-amz-date:20260914T010203Z\nx-amz-security-token:session-token\n\nhost;x-amz-content-sha256;x-amz-date;x-amz-security-token\nUNSIGNED-PAYLOAD';
  assert.ok(signed.headers.Authorization.endsWith(expectedSignature('AWS4', 's3', 'aws4_request', canonical)));
  assert.equal(signed.url, 'https://examplebucket.s3.example.com/%E5%AD%A6%E4%B9%A0/a%20%2B.txt?a=%2B&z=%2F');
  assert.equal(signed.headers.host, undefined);
});
test('OSS v4 uses OSS headers, bucket canonical path and Alibaba signing scope', () => {
  const { signStorageRequest } = loader({ '../data/database': {}, './platform': {} })(file);
  const signed = signStorageRequest({ ...config, provider: 'oss' }, 'PUT', 'file.json', {}, new Date('2026-09-14T01:02:03Z'), 'application/json');
  const canonical = 'PUT\n/examplebucket/file.json\n\ncontent-type:application/json\nx-oss-content-sha256:UNSIGNED-PAYLOAD\nx-oss-date:20260914T010203Z\n\n\nUNSIGNED-PAYLOAD';
  assert.ok(signed.headers.Authorization.endsWith(expectedSignature('aliyun_v4', 'oss', 'aliyun_v4_request', canonical)));
  assert.ok(!signed.headers.Authorization.includes('SignedHeaders'));
});
test('path-style S3 and invalid endpoint/prefix checks', () => {
  const { signStorageRequest, validateStorageConfig } = loader({ '../data/database': {}, './platform': {} })(file);
  assert.equal(signStorageRequest({ ...config, pathStyle: true }, 'GET', 'x').url, 'https://s3.example.com/examplebucket/x');
  for (const change of [{ endpoint: 'http://s3.example.com' }, { endpoint: 'https://user:pass@host' }, { endpoint: 'https://host/path' }, { prefix: '../other' }, { prefix: '' }, { sessionToken: 'x\ny' }]) assert.throws(() => validateStorageConfig({ ...config, ...change }));
});
test('list handles pagination, XML escaping and repeated-token failure', async () => {
  const calls = [];
  const pages = [ '<ListBucketResult><Contents><Key>学习/a&amp;b.json</Key></Contents><IsTruncated>true</IsTruncated><NextContinuationToken>a+b</NextContinuationToken></ListBucketResult>', '<ListBucketResult><Contents><Key>学习/z.json</Key></Contents><IsTruncated>false</IsTruncated></ListBucketResult>' ];
  const { ObjectStorage } = loader({ '../data/database': {}, './platform': {} }, { fetch: async url => { calls.push(url); return { ok: true, text: async () => pages.shift() }; } })(file);
  assert.deepEqual(Array.from(await new ObjectStorage(config).list('学习/')), ['学习/a&b.json', '学习/z.json']);
  assert.ok(calls[1].includes('continuation-token=a%2Bb'));
  const { ObjectStorage: Broken } = loader({ '../data/database': {}, './platform': {} }, { fetch: async () => ({ ok: true, text: async () => '<ListBucketResult><IsTruncated>true</IsTruncated><NextContinuationToken>same</NextContinuationToken></ListBucketResult>' }) })(file);
  await assert.rejects(new Broken(config).list('x'), /分页响应异常/);
});

test('release build without a credential module refuses to save plaintext secrets', async () => {
  const stored = [];
  const { saveStorageConfig } = loader({
    '../data/database': { getSetting: async () => null, setSetting: async (_k, v) => { stored.push(v); } },
    './platform': { currentPlatform: 'macos', getNativeModules: () => ({}) },
  })(file);
  await assert.rejects(saveStorageConfig(config), /保险库不可用/);
  assert.equal(stored.length, 0);
});

test('a partial staging failure aborts cleanly so the storage endpoint keeps its old keys', async () => {
  const vault = new Map([['clarora.storage.secret-access-key', 'old-secret']]);
  const stored = [];
  const { saveStorageConfig } = loader({
    '../data/database': { getSetting: async () => stored[stored.length - 1] ?? null, setSetting: async (_k, v) => { stored.push(v); } },
    './platform': { currentPlatform: 'macos', getNativeModules: () => ({ RNMacKeychain: {
      getSecret: async key => vault.get(key) ?? null,
      setSecret: async (key, value) => { if (key === 'clarora.storage.session-token.v1') throw new Error('denied'); vault.set(key, value); },
      deleteSecret: async key => vault.delete(key),
    } }) },
  })(file);
  await assert.rejects(saveStorageConfig(config), /保险库写入失败/);
  // 暂存半成品已清理,数据库仍引用旧代:老端点配老密钥,不会错代。
  assert.equal(vault.get('clarora.storage.secret-access-key'), 'old-secret');
  assert.equal(vault.get('clarora.storage.secret-access-key.v1'), undefined);
  assert.equal(stored.length, 0);
});

test('a failed config write keeps the previous generation referenced and discards the stage', async () => {
  const vault = new Map([['clarora.storage.secret-access-key', 'old-secret']]);
  const { saveStorageConfig } = loader({
    '../data/database': { getSetting: async () => null, setSetting: async () => {} },
    './platform': { currentPlatform: 'macos', getNativeModules: () => ({ RNMacKeychain: {
      getSecret: async key => vault.get(key) ?? null,
      setSecret: async (key, value) => { vault.set(key, value); },
      deleteSecret: async key => { vault.delete(key); },
    } }) },
  })(file);
  await assert.rejects(saveStorageConfig(config), /校验失败/);
  assert.equal(vault.get('clarora.storage.secret-access-key'), 'old-secret');
  assert.equal(vault.get('clarora.storage.secret-access-key.v1'), undefined);
});

test('a failed read of the current config aborts saving instead of staging over live keys', async () => {
  const vault = new Map([['clarora.storage.secret-access-key.v1', 'live-secret']]);
  let reads = 0;
  const { saveStorageConfig } = loader({
    '../data/database': {
      getSetting: async () => { if (++reads === 1) throw new Error('read failed'); return null; },
      setSetting: async () => {},
    },
    './platform': { currentPlatform: 'macos', getNativeModules: () => ({ RNMacKeychain: {
      getSecret: async key => vault.get(key) ?? null,
      setSecret: async (key, value) => { vault.set(key, value); },
      deleteSecret: async key => { vault.delete(key); },
    } }) },
  })(file);
  await assert.rejects(saveStorageConfig(config), /中止/);
  assert.equal(vault.get('clarora.storage.secret-access-key.v1'), 'live-secret');
});

test('an unknown commit outcome keeps the staged generation instead of deleting it', async () => {
  const vault = new Map();
  let reads = 0;
  const { saveStorageConfig } = loader({
    '../data/database': {
      getSetting: async () => { if (++reads >= 2) throw new Error('read failed'); return null; },
      setSetting: async () => {},
    },
    './platform': { currentPlatform: 'macos', getNativeModules: () => ({ RNMacKeychain: {
      getSecret: async key => vault.get(key) ?? null,
      setSecret: async (key, value) => { vault.set(key, value); },
      deleteSecret: async key => { vault.delete(key); },
    } }) },
  })(file);
  await assert.rejects(saveStorageConfig(config), /校验失败/);
  // 提交结果不确定(两次回读都失败):暂存的新代可能已生效,绝不能清理。
  assert.equal(vault.get('clarora.storage.secret-access-key.v1'), 'test-secret');
});

test('a stale load no longer writes back over a newer saved config', async () => {
  const vault = new Map();
  const writes = [];
  let reads = 0;
  const oldRow = JSON.stringify({ provider: 's3', endpoint: 'https://s3.old', region: 'r', bucket: 'b', accessKeyId: 'a', secretAccessKey: 'old-secret', sessionToken: '', prefix: 'p', pathStyle: false });
  const newRow = JSON.stringify({ provider: 's3', endpoint: 'https://s3.new', region: 'r', bucket: 'b', accessKeyId: 'a', secretAccessKey: '', sessionToken: '', prefix: 'p', pathStyle: false, secretGen: 1 });
  const { loadStorageConfig } = loader({
    '../data/database': {
      getSetting: async () => (reads++ === 0 ? oldRow : newRow),
      setSetting: async (_k, v) => { writes.push(v); },
    },
    './platform': { currentPlatform: 'macos', getNativeModules: () => ({ RNMacKeychain: {
      getSecret: async key => vault.get(key) ?? null,
      setSecret: async (key, value) => { vault.set(key, value); },
      deleteSecret: async key => { vault.delete(key); },
    } }) },
  })(file);
  await loadStorageConfig();
  // 审查复现:行在读取期间被更新时,旧读取任务必须跳过迁移回写。
  assert.equal(writes.length, 0);
});
