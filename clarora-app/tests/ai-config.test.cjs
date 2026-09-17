const { test } = require('node:test');
const assert = require('node:assert/strict');
const path = require('node:path');
const loader = require('./helpers/load-ts.cjs');

function setup({ dev = false, denyRead = false, denyWrite = false, discardWrite = false, status = 200 } = {}) {
  let stored = null;
  const vault = new Map();
  const requests = [];
  const api = loader({
    '../data/database': {
      getSetting: async () => stored,
      setSetting: async (_, value) => { if (!discardWrite) stored = value; },
    },
    './platform': { currentPlatform: 'macos', getNativeModules: () => ({ RNMacKeychain: {
      getSecret: async key => { if (denyRead) throw new Error('denied'); return vault.get(key) ?? null; },
      setSecret: async (key, value) => { if (denyWrite) throw new Error('denied'); vault.set(key, value); },
      deleteSecret: async key => { vault.delete(key); },
    } }) },
  }, {
    __DEV__: dev,
    fetch: async (url, options) => {
      requests.push({ url, options });
      return { ok: status === 200, status, json: async () => ({ choices: [{ message: { content: 'ok' } }] }) };
    },
  })(path.join(__dirname, '../shared/services/ai.ts'));
  const config = { ...api.DEFAULT_AI, baseUrl: 'https://example.com/v1', model: 'test', apiKey: 'test-key' };
  return { api, config, requests, vault };
}

for (const dev of [true, false]) {
  test(`saved key survives reload and is sent in Authorization (dev=${dev})`, async () => {
    const s = setup({ dev });
    const saved = await s.api.saveAiConfig({ ...s.config, apiKey: '  test-key  ' });
    assert.equal(saved.apiKey, 'test-key');
    assert.equal((await s.api.loadAiConfig()).apiKey, 'test-key');
    await s.api.askAboutPassage('text', 'question');
    assert.equal(s.requests[0].options.headers.Authorization, 'Bearer test-key');
  });
}

test('vault read denial surfaces before sending any request', async () => {
  const s = setup({ denyRead: true });
  await assert.rejects(s.api.askAboutPassage('text', 'question'), /无法读取已保存的 AI 密钥/);
  assert.equal(s.requests.length, 0);
});

test('a failed vault save retains the newer fallback key instead of using a stale vault value', async () => {
  const s = setup({ denyWrite: true });
  s.vault.set('clarora.ai.api-key', 'old-key');
  await s.api.saveAiConfig(s.config);
  assert.equal((await s.api.loadAiConfig()).apiKey, 'test-key');
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
