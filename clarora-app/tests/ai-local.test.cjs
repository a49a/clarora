const { test } = require('node:test');
const assert = require('node:assert/strict');
const path = require('node:path');
const loader = require('./helpers/load-ts.cjs');
const file = path.join(__dirname, '../shared/services/ai.ts');

const segments = [
  { start: 0, end: 2, text: 'hello world' },
  { start: 2, end: 4, text: 'once more' },
];

function setup(config, whisper) {
  const files = new Set(['ggml-base.en.bin']);
  const api = loader({
    '../data/database': {
      getSetting: async () => JSON.stringify(config), setSetting: async () => {},
      saveAiRecord: async () => {}, listAiRecords: async () => [], deleteAiRecord: async () => {},
    },
    './platform': {
      currentPlatform: 'macos',
      nativePath: value => value,
      getNativeModules: () => ({ RNMacWhisper: whisper ? { transcribe: whisper } : undefined }),
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
  return { api, files };
}

test('local engine transcribes on device and never uploads audio', async () => {
  let calls = 0;
  let progress = -1;
  const s = setup({ asrEngine: 'local', localModel: 'base.en' }, async () => { calls += 1; return { duration: 4, segments }; });
  const { jobId } = await s.api.transcribeAudio('file:///a.mp3', 'a.mp3');
  const srt = await s.api.waitForSubtitles(jobId, value => { progress = value; });
  assert.equal(calls, 1);
  assert.ok(progress >= 0 && progress <= 1);
  assert.match(srt, /hello world/);
  assert.match(srt, /once more/);
});

test('missing model download surfaces in waitForSubtitles with actionable message', async () => {
  const s = setup({ asrEngine: 'local', localModel: 'small.en' }, async () => ({ duration: 0, segments }));
  const { jobId } = await s.api.transcribeAudio('file:///a.mp3', 'a.mp3');
  await assert.rejects(s.api.waitForSubtitles(jobId), /尚未下载/);
});

test('model registry, download flow and language resolution', async () => {
  const s = setup({ asrEngine: 'local', localModel: 'base.en' }, async () => ({ duration: 0, segments }));
  assert.equal(await s.api.localModelDownloaded('base.en'), true);
  assert.equal(await s.api.localModelDownloaded('small.en'), false);
  assert.equal(s.api.resolveWhisperLanguage('base.en'), 'en');
  assert.equal(s.api.resolveWhisperLanguage('base'), 'auto');
  const message = await s.api.downloadLocalModel('small.en');
  assert.match(message, /下载完成/);
  assert.equal(await s.api.localModelDownloaded('small.en'), true);
  const engines = await s.api.listAsrEngines();
  assert.equal(engines.backends[0].backend, 'local');
  assert.equal(engines.backends[0].available, true);
  await s.api.deleteLocalModel('small.en');
  assert.equal(await s.api.localModelDownloaded('small.en'), false);
});
