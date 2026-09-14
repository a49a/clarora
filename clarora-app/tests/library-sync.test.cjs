const { test } = require('node:test');
const assert = require('node:assert/strict');
const path = require('node:path');
const loader = require('./helpers/load-ts.cjs');
const root = path.join(__dirname, '../shared');
const { VAULT_TABLES } = loader()(path.join(root, 'data/vault.ts'));
const clone = data => JSON.parse(JSON.stringify(data));
function setup({ uploadFails = false, downloadFails = false } = {}) {
  const objects = new Map(), media = new Map(), events = [], imports = [];
  const data = Object.fromEntries(VAULT_TABLES.map(t => [t, []]));
  data.listening_practices = [{ id: 'p', name: 'practice', created_at: '' }];
  data.listening_audios = [{ id: 'a', practice_id: 'p', name: 'audio', audio_uri: '/local/听力.mp3', subtitle_uri: '/local/a.srt', created_at: '' }];
  data.clip_cards = [{ id: 'c', en_text: 'hello', zh_text: '', audio_uri: '/local/听力.mp3', start_ms: 0, end_ms: 1000, created_at: '' }];
  data.video_clips = [{ id: 'v', en_text: 'video', zh_text: '', video_uri: '/local/video.mp4', created_at: '' }];
  class Storage {
    key(key) { return `vault/${key}`; }
    async list(prefix) { return [...objects.keys()].filter(k => k.startsWith(prefix)); }
    async request(method, key, _, body) { if (method === 'PUT') { events.push('manifest'); objects.set(key, body); } return objects.get(key); }
    async putFile(key, uri) { events.push('upload'); if (uploadFails) throw new Error('upload failed'); media.set(key, uri); }
    async getFile(key, uri) { events.push('download'); if (downloadFails) throw new Error('download failed'); assert.ok(media.has(key)); events.push(uri); }
  }
  const api = loader({ '../data/database': { exportVaultData: async () => clone(data), importVaultData: async d => { imports.push(clone(d)); } },
    './objectStorage': { ObjectStorage: Storage, loadStorageConfig: async () => ({}) },
    './platform': { FileSystem: { getDocumentDirectoryAsync: async () => '/documents/', makeDirectoryAsync: async () => {}, deleteAsync: async path => { events.push(`delete:${path}`); } } },
  })(path.join(root, 'services/librarySync.ts'));
  return { api, objects, media, events, imports, data };
}
test('backup commits immutable manifest last, deduplicates attachments and restores portable media paths', async () => {
  const s = setup(); const result = await s.api.uploadLibrary();
  assert.equal(result.files, 3); assert.equal(s.events.at(-1), 'manifest');
  const [key, raw] = [...s.objects][0]; assert.ok(!raw.includes('/local/'));
  await s.api.uploadLibrary(); assert.equal(s.objects.size, 2);
  await s.api.downloadLibrary(key);
  assert.equal(s.imports.length, 1);
  const d = s.imports[0]; assert.equal(d.clip_cards[0].audio_uri, d.listening_audios[0].audio_uri);
  assert.match(d.video_clips[0].video_uri, /^\/documents\/Clarora\/Restored\//);
});
test('failed upload does not publish any manifest', async () => {
  const s = setup({ uploadFails: true }); await assert.rejects(s.api.uploadLibrary(), /upload failed/); assert.equal(s.objects.size, 0);
});
test('failed download cleans partial files and never imports the database', async () => {
  const s = setup({ downloadFails: true }); await s.api.uploadLibrary();
  await assert.rejects(s.api.downloadLibrary([...s.objects.keys()][0]), /download failed/);
  assert.equal(s.imports.length, 0); assert.ok(s.events.some(e => e.startsWith('delete:')));
});
test('malicious manifest cannot fetch keys outside its backup or write arbitrary paths', async () => {
  const s = setup(); await s.api.uploadLibrary(); const [key, raw] = [...s.objects][0];
  const manifest = JSON.parse(raw); manifest.files['media:0'] = 'vault/media/../../secret'; s.objects.set(key, JSON.stringify(manifest));
  await assert.rejects(s.api.downloadLibrary(key), /附件路径无效/);
  assert.ok(!s.events.includes('download')); assert.equal(s.imports.length, 0);
  await assert.rejects(s.api.downloadLibrary('other/snapshots/file.json'), /备份路径无效/);
});
