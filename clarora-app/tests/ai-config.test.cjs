const { test } = require('node:test');
const assert = require('node:assert/strict');
const path = require('node:path');
const loader = require('./helpers/load-ts.cjs');

function setup({ dev = false, denyRead = false, denyWrite = false, failOn = '', noVaultModule = false, discardWrite = false, failReadOn = 0, failReadFrom = 0, seed = null, status = 200 } = {}) {
  let failOnUsed = false;
  let stored = seed;
  const vault = new Map();
  const requests = [];
  // mount():每次调用都创建全新模块实例(共享同一持久状态),
  // 用于验证"重建服务实例后结论仍成立"。读取失败计数按实例独立,
  // 否则新实例的读取会被旧实例的故障配置连坐。
  const mount = () => {
    let settingReads = 0;
    return loader({
    '../data/database': {
      getSetting: async () => {
        settingReads += 1;
        if (failReadOn && settingReads === failReadOn) throw new Error('read failed');
        if (failReadFrom && settingReads >= failReadFrom) throw new Error('read failed');
        return stored;
      },
      setSetting: async (_, value) => { if (!discardWrite) stored = value; },
    },
    './platform': { currentPlatform: 'macos', getNativeModules: () => noVaultModule ? {} : ({ RNMacKeychain: {
      getSecret: async key => { if (denyRead) throw new Error('denied'); return vault.get(key) ?? null; },
      setSecret: async (key, value) => {
        if (denyWrite) throw new Error('denied');
        if (key === failOn && !failOnUsed) { failOnUsed = true; throw new Error('denied'); }
        vault.set(key, value);
      },
      deleteSecret: async key => { vault.delete(key); },
    } }) },
  }, {
    __DEV__: dev,
    fetch: async (url, options) => {
      requests.push({ url, options });
      return { ok: status === 200, status, json: async () => ({ choices: [{ message: { content: 'ok' } }] }) };
    },
  })(path.join(__dirname, '../shared/services/ai.ts'));
  };
  const api = mount();
  const config = { ...api.DEFAULT_AI, baseUrl: 'https://example.com/v1', model: 'test', apiKey: 'test-key' };
  return { api, mount, config, requests, vault };
}

for (const dev of [true, false]) {
  test(`saved key survives reload and is sent in Authorization (dev=${dev})`, async () => {
    const s = setup({ dev });
    const saved = await s.api.saveAiConfig({ ...s.config, apiKey: '  test-key  ' });
    assert.equal(saved.apiKey, 'test-key');
    assert.equal((await s.api.loadAiConfig()).apiKey, 'test-key');
    // 重建服务实例(新模块实例,同一持久状态)后结论仍成立。
    const fresh = s.mount();
    assert.equal((await fresh.loadAiConfig()).apiKey, 'test-key');
    await s.api.askAboutPassage('text', 'question');
    assert.equal(s.requests[0].options.headers.Authorization, 'Bearer test-key');
  });
}

test('vault read denial surfaces before sending any request', async () => {
  const s = setup({ denyRead: true });
  await assert.rejects(s.api.askAboutPassage('text', 'question'), /无法读取已保存的 AI 密钥/);
  assert.equal(s.requests.length, 0);
});

test('a failed vault save is rejected loudly and the database keeps no plaintext', async () => {
  const s = setup({ denyWrite: true });
  s.vault.set('clarora.ai.api-key', 'old-key');
  await assert.rejects(s.api.saveAiConfig(s.config), /保险库写入失败/);
  // 数据库未写入明文:加载时只会读到保险库旧值,而不是刚输入的 test-key。
  assert.equal((await s.api.loadAiConfig()).apiKey, 'old-key');
});

test('a failed read of the current config aborts saving instead of staging over live keys', async () => {
  const s = setup({ failReadOn: 1, seed: JSON.stringify({ baseUrl: 'https://old.example/v1', secretGen: 1 }) });
  s.vault.set('clarora.ai.api-key.v1', 'live-key');
  await assert.rejects(s.api.saveAiConfig(s.config), /中止/);
  // 审查复现:读不到旧配置不能按 0 代继续,否则暂存会覆盖在用的 v1。
  assert.equal(s.vault.get('clarora.ai.api-key.v1'), 'live-key');
});

test('an unparseable existing config aborts saving instead of guessing generation 0', async () => {
  const s = setup({ seed: '{corrupt' });
  s.vault.set('clarora.ai.api-key.v1', 'live-key');
  await assert.rejects(s.api.saveAiConfig(s.config), /中止/);
  assert.equal(s.vault.get('clarora.ai.api-key.v1'), 'live-key');
});

test('an unknown commit outcome keeps the staged generation instead of deleting it', async () => {
  const s = setup({ failReadFrom: 2, seed: JSON.stringify({ baseUrl: 'https://old.example/v1', apiKey: 'old' }) });
  s.vault.set('clarora.ai.api-key', 'old-key');
  await assert.rejects(s.api.saveAiConfig(s.config), /校验失败/);
  // 审查复现:两次回读都失败时提交结果不确定,新代可能已生效,绝不能清理。
  assert.equal(s.vault.get('clarora.ai.api-key.v1'), 'test-key');
  // 重建实例(读取计数独立):提交已随成功的行写入生效,读到的是
  // 同一代的新地址与新密钥——无论当时提交是否成功,地址与密钥都成对。
  const fresh = s.mount();
  const loaded = await fresh.loadAiConfig();
  assert.equal(loaded.baseUrl, 'https://example.com/v1');
  assert.equal(loaded.apiKey, 'test-key');
});

test('a partial staging failure aborts cleanly and keeps the previous generation live', async () => {
  const s = setup({ failOn: 'clarora.ai.asr-api-key.v1', seed: JSON.stringify({ baseUrl: 'https://old.example/v1' }) });
  s.vault.set('clarora.ai.api-key', 'old-key');
  await assert.rejects(s.api.saveAiConfig(s.config), /配置未变更/);
  // 暂存半成品已清理,数据库仍引用旧代:老地址配老密钥,不会错代。
  assert.equal((await s.api.loadAiConfig()).baseUrl, 'https://old.example/v1');
  assert.equal(s.vault.get('clarora.ai.api-key'), 'old-key');
  assert.equal(s.vault.get('clarora.ai.api-key.v1'), undefined);
});

test('two consecutive verification read failures leave the previous generation live', async () => {
  const s = setup({ discardWrite: true, seed: JSON.stringify({ baseUrl: 'https://old.example/v1', apiKey: 'legacy' }) });
  await assert.rejects(s.api.saveAiConfig(s.config), /校验失败/);
  // 审查复现路径:配置写不进去且回读不到时,旧代仍被引用,不会出现新地址。
  const loaded = await s.api.loadAiConfig();
  assert.equal(loaded.baseUrl, 'https://old.example/v1');
  assert.equal(loaded.apiKey, 'legacy');
  assert.equal(s.vault.get('clarora.ai.api-key.v1'), undefined, '未提交的新代应被清理');
});

test('a release build without the credential module refuses to save instead of storing plaintext', async () => {
  const s = setup({ noVaultModule: true });
  await assert.rejects(s.api.saveAiConfig(s.config), /保险库不可用/);
  assert.equal((await s.api.loadAiConfig()).apiKey, '');
});

test('a failed config write keeps the previous generation referenced and discards the stage', async () => {
  const s = setup({ discardWrite: true, seed: JSON.stringify({ baseUrl: 'https://old.example/v1' }) });
  s.vault.set('clarora.ai.api-key', 'old-key');
  await assert.rejects(s.api.saveAiConfig(s.config), /校验失败/);
  // 数据库未写入新配置:旧代仍被引用,暂存的新代已尽力清理。
  assert.equal(s.vault.get('clarora.ai.api-key'), 'old-key');
  assert.equal(s.vault.get('clarora.ai.api-key.v1'), undefined);
});

test('a denied vault read no longer blocks saving; the new generation stays self-consistent', async () => {
  const s = setup({ denyRead: true, seed: JSON.stringify({ baseUrl: 'https://old.example/v1' }) });
  // 审查复现路径:保存不再读取旧密钥,读取被拒只影响读取,不影响换代。
  await s.api.saveAiConfig(s.config);
  assert.equal(s.vault.get('clarora.ai.api-key.v1'), 'test-key');
  assert.equal(s.vault.get('clarora.ai.asr-api-key.v1'), '');
});

test('a rejected keychain write fails the save and the previous generation stays referenced', async () => {
  const s = setup({ denyWrite: true, seed: JSON.stringify({ baseUrl: 'https://old.example/v1', apiKey: 'old' }) });
  await assert.rejects(s.api.saveAiConfig(s.config), /保险库写入失败/);
  const loaded = await s.api.loadAiConfig();
  assert.equal(loaded.baseUrl, 'https://old.example/v1');
  assert.equal(loaded.apiKey, 'old');
});

test('missing remote key blocks the request; localhost can omit it', async () => {
  const s = setup({ dev: true });
  await s.api.saveAiConfig({ ...s.config, apiKey: '' });
  await assert.rejects(s.api.askAboutPassage('text', 'question'), /尚未配置 API Key/);
  assert.equal(s.requests.length, 0);
  await s.api.saveAiConfig({ ...s.config, apiKey: '', baseUrl: 'http://localhost:8080/v1' });
  await s.api.askAboutPassage('text', 'question');
  assert.equal(s.requests.length, 1);
});

test('a discarded database write cannot report save success', async () => {
  const s = setup({ dev: true, discardWrite: true });
  await assert.rejects(s.api.saveAiConfig(s.config), /保存校验失败/);
});

test('server 401 identifies rejected credentials separately from a missing key', async () => {
  const s = setup({ dev: true, status: 401 });
  await s.api.saveAiConfig(s.config);
  await assert.rejects(s.api.askAboutPassage('text', 'question'), /拒绝了 API Key（HTTP 401）/);
});

test('retrying after a staging failure succeeds and lands the referenced generation', async () => {
  const s = setup({ failOn: 'clarora.ai.asr-api-key.v1' });
  await assert.rejects(s.api.saveAiConfig(s.config), /配置未变更/);
  // 失败后无暂存残留;同一代号重试会重建暂存并成功提交。
  assert.equal(s.vault.get('clarora.ai.api-key.v1'), undefined);
  await s.api.saveAiConfig(s.config);
  assert.equal(s.vault.get('clarora.ai.api-key.v1'), 'test-key');
  const loaded = await s.api.loadAiConfig();
  assert.equal(loaded.apiKey, 'test-key');
  assert.equal(loaded.baseUrl, 'https://example.com/v1');
  assert.equal(loaded.secretGen, 1);
  const fresh = s.mount();
  assert.equal((await fresh.loadAiConfig()).apiKey, 'test-key');
});
