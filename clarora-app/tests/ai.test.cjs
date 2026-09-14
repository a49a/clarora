const { test } = require('node:test');
const assert = require('node:assert/strict');
const path = require('node:path');
const loader = require('./helpers/load-ts.cjs');
const file = path.join(__dirname, '../shared/services/ai.ts');
function setup(config = {}, reply = { choices: [{ message: { content: 'answer' } }] }) {
  const records = new Map(), requests = [], uploads = [];
  const api = loader({ '../data/database': {
    getSetting: async () => JSON.stringify(config), setSetting: async () => {},
    saveAiRecord: async (id, kind, value) => records.set(id, { kind, value: JSON.parse(JSON.stringify(value)) }),
    listAiRecords: async kind => [...records.values()].filter(r => r.kind === kind).map(r => r.value),
    deleteAiRecord: async id => records.delete(id),
  }, './platform': { FileSystem: {
    readAsStringAsync: async () => '1\n00:00:01,200 --> 00:00:02,300\nHello\n\n2\n00:00:03,000 --> 00:00:04,000\nWorld\n',
    readBase64Async: async () => 'aGVsbG8=',
    uploadFileAsync: async (...args) => { uploads.push(args); return { status: 200, body: JSON.stringify({ text: 'hello world', duration: 2, segments: [{ start: 0, end: 2, text: 'hello world' }] }) }; },
  } } }, { fetch: async (url, init) => { requests.push({ url, ...init }); return { ok: true, json: async () => reply }; } })(file);
  return { api, records, requests, uploads };
}
const config = { baseUrl: 'https://chat.example/v1', apiKey: 'chat-key', model: 'chat-model', visionModel: 'vision-model', asrBaseUrl: 'https://speech.example/v1', asrApiKey: 'speech-key', asrModel: 'speech-model' };
test('unconfigured AI does not issue requests and local lists still work', async () => {
  const s = setup(); await assert.rejects(s.api.askAboutPassage('text', 'question'), /请在设置/);
  assert.equal(s.requests.length, 0); assert.equal((await s.api.listOcrPages()).length, 0); assert.equal((await s.api.listSpeakingAttempts()).length, 0);
});
test('chat uses only the explicitly configured endpoint and preserves history', async () => {
  const s = setup(config); await s.api.askAboutPassage('material', 'question', undefined, undefined, [{ role: 'user', content: 'first' }, { role: 'assistant', content: 'reply' }]);
  assert.equal(s.requests[0].url, 'https://chat.example/v1/chat/completions');
  assert.equal(s.requests[0].headers.Authorization, 'Bearer chat-key');
  assert.equal(JSON.parse(s.requests[0].body).messages[1].content, 'first');
});
test('ASR streams multipart file with configured fields to the separate provider', async () => {
  const s = setup(config); const { jobId } = await s.api.transcribeAudio('/audio.m4a', 'audio.m4a');
  assert.equal(s.uploads[0][0], 'https://speech.example/v1/audio/transcriptions');
  assert.equal(s.uploads[0][2].Authorization, 'Bearer speech-key');
  assert.equal(s.uploads[0][5].model, 'speech-model'); assert.equal(s.uploads[0][5].response_format, 'verbose_json');
  assert.match(await s.api.waitForSubtitles(jobId), /00:00:00,000 --> 00:00:02,000/);
  await s.api.deleteJob(jobId); await assert.rejects(s.api.waitForSubtitles(jobId), /已失效/);
});
test('translation preserves original timing and rejects missing translated lines', async () => {
  const s = setup(config, { choices: [{ message: { content: '["你好","世界"]' } }] });
  const result = await s.api.translateSubtitlesFile('/s.srt', 's.srt', { mode: 'bilingual' });
  const text = await s.api.waitForTranslation(result.jobId);
  assert.match(text, /00:00:01,200 --> 00:00:02,300/); assert.match(text, /Hello\n你好/);
  const bad = setup(config, { choices: [{ message: { content: '["你好"]' } }] });
  await assert.rejects(bad.api.translateSubtitlesFile('/s.srt', 's.srt'), /数量不符/);
});
test('OCR supports a vision-only configuration and persists local results', async () => {
  const s = setup({ ...config, model: '' }); const { pageId } = await s.api.ocrPage('/image.png', 'image.png');
  assert.equal(await s.api.waitForOcr(pageId), 'answer');
  assert.equal(JSON.parse(s.requests[0].body).model, 'vision-model');
  assert.equal((await s.api.listOcrPages())[0].textLength, 6);
  await s.api.deleteOcrPage(pageId); assert.equal((await s.api.listOcrPages()).length, 0);
});
test('speaking score aligns substitutions, omissions and extra recognized words', () => {
  const { api } = setup();
  const score = api.scoreTranscript('the quick brown fox', 'the slow fox jumps', 2);
  assert.ok(score.accuracy < 100); assert.ok(score.wrong.length + score.missed.length + score.extra.length > 0);
  const exact = api.scoreTranscript('Hello, world!', 'hello world', 2);
  assert.equal(exact.accuracy, 100); assert.equal(exact.completeness, 100); assert.equal(exact.wpm, 60);
});
