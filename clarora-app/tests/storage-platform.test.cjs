const { test } = require('node:test');
const assert = require('node:assert/strict');
const path = require('node:path');
const loader = require('./helpers/load-ts.cjs');
const file = path.join(__dirname, '../shared/services/platform.ts');
for (const platform of ['android', 'ios', 'macos', 'windows']) test(`${platform}: storage uses a raw PUT and AI multipart retains model fields`, async () => {
  const requests = [], shell = [], windows = [];
  const native = { RNWindowsFiles: {
    putFile: async (...args) => { windows.push(['PUT', ...args]); return { status: 200, body: '' }; },
    uploadForm: async (...args) => { windows.push(['POST', ...args]); return { status: 200, body: '' }; },
  }, RNShell: { runArgs: async (command, args) => { shell.push([command, args]); return '\n200'; } } };
  const { FileSystem } = loader({ 'react-native': { Platform: { OS: platform }, NativeModules: native }, './rnfs': { default: { uploadFiles: options => { requests.push(options); return { promise: Promise.resolve({ statusCode: 200, body: '' }) }; } } } })(file);
  await FileSystem.putFileAsync('https://bucket.example/object', '/local/录音.m4a', { Authorization: 'signed', 'content-type': 'application/octet-stream' });
  await FileSystem.uploadFileAsync('https://model.example/audio/transcriptions', '/local/录音.m4a', { Authorization: 'Bearer key' }, 'file', 'audio/mp4', { model: 'test', response_format: 'verbose_json' });
  if (requests.length) {
    assert.equal(requests[0].method, 'PUT'); assert.equal(requests[0].binaryStreamOnly, true);
    assert.equal(requests[1].method, 'POST'); assert.equal(requests[1].fields.model, 'test');
  } else if (shell.length) {
    assert.ok(shell[0][1].includes('-T')); assert.ok(!shell[0][1].includes('-F'));
    assert.ok(shell[1][1].includes('--form-string')); assert.ok(shell[1][1].includes('model=test'));
  } else { assert.equal(windows[0][0], 'PUT'); assert.equal(windows[1].at(-1).model, 'test'); }
});
