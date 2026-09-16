const { test } = require('node:test');
const assert = require('node:assert/strict');
const path = require('node:path');
const loader = require('./helpers/load-ts.cjs');
const file = path.join(__dirname, '../shared/services/ai.ts');

const segments = [
  { start: 0, end: 2, text: 'hello world' },
  { start: 2, end: 4, text: 'once more' },
];

function setup(config, natives = {}, platform = 'macos', secretMap = null) {
  const modules = { RNMacWhisper: natives.whisper, RNMacSenseVoice: natives.senseVoice, RNWindowsAsr: natives.windowsAsr };
  const files = new Set(['ggml-base.en.bin']);
  const settingsMap = new Map();
  if (config) settingsMap.set('ai_config', JSON.stringify(config));
  const settingsRecord = [];
  const api = loader({
    '../data/database': {
      getSetting: async name => settingsMap.get(name) ?? null,
      setSetting: async (name, value) => { settingsRecord.push(value); settingsMap.set(name, value); },
      saveAiRecord: async () => {}, listAiRecords: async () => [], deleteAiRecord: async () => {},
    },
    './secrets': {
      SECRET_NAMES: {
        aiApiKey: 'clarora.ai.api-key',
        aiAsrApiKey: 'clarora.ai.asr-api-key',
        storageSecretAccessKey: 'clarora.storage.secret-access-key',
        storageSessionToken: 'clarora.storage.session-token',
      },
      migrateSecret: async (store, key, plaintext) => {
        const stored = await store.getSecret(key).catch(() => null);
        if (stored != null) return stored;
        if (plaintext) await store.setSecret(key, plaintext);
        return plaintext;
      },
      secretStore: () => secretMap
        ? {
            setSecret: async (key, value) => { secretMap.set(key, value); },
            getSecret: async key => (secretMap.has(key) ? secretMap.get(key) : null),
            deleteSecret: async key => { secretMap.delete(key); },
          }
        : null,
    },
    './platform': {
      currentPlatform: platform,
      nativePath: value => value,
      getNativeModules: () => modules,
      FileSystem: {
        getDocumentDirectoryAsync: async () => '/docs/',
        makeDirectoryAsync: async () => {},
        listFilesAsync: async () => [...files],
        downloadFileAsync: async (url, destination) => { files.add(destination.split('/').pop()); },
        deleteAsync: async target => { files.delete(target.split('/').pop()); },
        uploadFileAsync: async () => { throw new Error('端侧转写不应上传音频'); },
      },
    },
  })(file);
  return { api, files, settingsMap, settingsRecord };
}

test('whisper engine transcribes on device and never uploads audio', async () => {
  let calls = 0;
  let progress = -1;
  const s = setup({ asrEngine: 'local', localModel: 'base.en' }, { whisper: { transcribe: async () => { calls += 1; return { duration: 4, segments }; } } });
  const { jobId } = await s.api.transcribeAudio('file:///a.mp3', 'a.mp3');
  const srt = await s.api.waitForSubtitles(jobId, value => { progress = value; });
  assert.equal(calls, 1);
  assert.ok(progress >= 0 && progress <= 1);
  assert.match(srt, /hello world/);
  assert.match(srt, /once more/);
});

test('sense-voice engine dispatches to its own native module with token file', async () => {
  const s = setup({ asrEngine: 'local', localModel: 'sense-voice' }, {
    whisper: { transcribe: async () => { throw new Error('不应调用 whisper'); } },
    senseVoice: { transcribe: async (audio, modelPath, tokensPath) => {
      assert.equal(modelPath, '/docs/sensevoice/model.int8.onnx');
      assert.equal(tokensPath, '/docs/sensevoice/tokens.txt');
      return { duration: 3, segments: [{ start: 0, end: 3, text: '你好世界' }] };
    } },
  });
  s.files.add('model.int8.onnx'); s.files.add('tokens.txt');
  const { jobId } = await s.api.transcribeAudio('file:///a.mp3', 'a.mp3');
  const srt = await s.api.waitForSubtitles(jobId);
  assert.match(srt, /你好世界/);
});

test('missing model download surfaces in waitForSubtitles with actionable message', async () => {
  const s = setup({ asrEngine: 'local', localModel: 'small.en' }, { whisper: async () => ({ duration: 0, segments }) });
  const { jobId } = await s.api.transcribeAudio('file:///a.mp3', 'a.mp3');
  await assert.rejects(s.api.waitForSubtitles(jobId), /尚未下载/);
});

test('windows engine dispatches through RNWindowsAsr JSON bridge', async () => {
  let modelPath = '', language = '';
  const s = setup({ asrEngine: 'local', localModel: 'base.en' }, {
    windowsAsr: {
      transcribeWhisper: async (audio, model, lang) => {
        modelPath = model; language = lang;
        return JSON.stringify({ duration: 4, segments });
      },
    },
  }, 'windows');
  const { jobId } = await s.api.transcribeAudio('C:/music/a.mp3', 'a.mp3');
  const srt = await s.api.waitForSubtitles(jobId);
  assert.equal(modelPath, '/docs/whisper/ggml-base.en.bin');
  assert.equal(language, 'en');
  assert.match(srt, /hello world/);
  assert.match(srt, /once more/);
});

test('model registry, download flow and language resolution', async () => {
  const s = setup({ asrEngine: 'local', localModel: 'base.en' }, {
    whisper: { transcribe: async () => ({ duration: 0, segments }) },
    senseVoice: { transcribe: async () => ({ duration: 0, segments }) },
  });
  assert.equal(await s.api.localModelDownloaded('base.en'), true);
  assert.equal(await s.api.localModelDownloaded('small.en'), false);
  assert.equal(await s.api.localModelDownloaded('sense-voice'), false);
  assert.equal(s.api.resolveAsrLanguage('base.en'), 'en');
  assert.equal(s.api.resolveAsrLanguage('base'), 'auto');
  const message = await s.api.downloadLocalModel('small.en');
  assert.match(message, /下载完成/);
  assert.equal(await s.api.localModelDownloaded('small.en'), true);
  await s.api.downloadLocalModel('sense-voice');
  assert.equal(await s.api.localModelDownloaded('sense-voice'), true);
  const engines = await s.api.listAsrEngines();
  assert.equal(engines.backends[0].backend, 'local');
  assert.equal(engines.backends[0].available, true);
  await s.api.deleteLocalModel('small.en');
  assert.equal(await s.api.localModelDownloaded('small.en'), false);
});

test('ai api keys migrate into the system vault and the database is scrubbed', async () => {
  const secretMap = new Map();
  const s = setup({ baseUrl: 'https://chat.example/v1', model: 'm', apiKey: 'plain-key' }, {}, 'macos', secretMap);
  const config = await s.api.loadAiConfig();
  assert.equal(config.apiKey, 'plain-key');
  assert.equal(secretMap.get('clarora.ai.api-key'), 'plain-key');
  assert.ok(!s.settingsMap.get('ai_config').includes('plain-key'));
});

test('saving puts keys in the vault and stores a scrubbed config', async () => {
  const secretMap = new Map();
  const s = setup({}, {}, 'macos', secretMap);
  await s.api.saveAiConfig({ ...s.api.DEFAULT_AI, baseUrl: 'https://chat.example/v1', model: 'm', apiKey: 'new-key' });
  assert.equal(secretMap.get('clarora.ai.api-key'), 'new-key');
  const stored = JSON.parse(s.settingsMap.get('ai_config'));
  assert.equal(stored.apiKey, '');
});

test('platforms without a vault keep the legacy behavior', async () => {
  const s = setup({ baseUrl: 'https://chat.example/v1', model: 'm', apiKey: 'kept' }, {}, 'android');
  await s.api.saveAiConfig({ ...s.api.DEFAULT_AI, baseUrl: 'https://chat.example/v1', model: 'm', apiKey: 'kept' });
  assert.ok(s.settingsMap.get('ai_config').includes('kept'));
});

test('real secrets helper stores once and reuses the vault value', async () => {
  const secretsFile = path.join(__dirname, '../shared/services/secrets.ts');
  const map = new Map();
  const keychain = {
    setSecret: async (key, value) => { map.set(key, value); },
    getSecret: async key => (map.has(key) ? map.get(key) : null),
    deleteSecret: async key => { map.delete(key); },
  };
  const secrets = loader({
    './platform': { currentPlatform: 'macos', getNativeModules: () => ({ RNMacKeychain: keychain }) },
  })(secretsFile);
  const first = await secrets.migrateSecret(await secrets.secretStore(), 'clarora.ai.api-key', 'plain-key');
  assert.equal(first, 'plain-key');
  assert.equal(map.size, 1);
  const second = await secrets.migrateSecret(await secrets.secretStore(), 'clarora.ai.api-key', 'ignored');
  assert.equal(second, 'plain-key');
  assert.equal(map.size, 1);
  assert.equal(secrets.SECRET_NAMES.aiApiKey, 'clarora.ai.api-key');
});
