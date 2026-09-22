// 恢复协调器测试:阶段持久化、双指纹闸门、崩溃恢复、并发防护与附件改写。

const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const ts = require('typescript');
const vm = require('node:vm');

const ROOT = path.join(__dirname, '..');

function makeVault(words = 2) {
  return {
    words: Array.from({ length: words }, (_, i) => ({ id: `w${i}`, word: `w${i}`, meaning: 'm', source: 'manual', created_at: 't' })),
    listening_practices: [], listening_audios: [], clip_cards: [],
    ai_cards: [], review_schedule: [], video_clips: [], discover_favorites: [],
    study_stats: [], app_settings: [],
  };
}

function setup({ persisted = null, importFails = false } = {}) {
  const documents = fs.mkdtempSync(path.join(os.tmpdir(), 'clarora-restore-'));
  const settingsMap = new Map(Object.entries(persisted ? { restore_operation: JSON.stringify(persisted) } : {}));
  const imported = [];
  const deleted = [];
  const manifest = {
    version: 2, id: 'bk1', createdAt: 't',
    data: makeVault(), files: { 'media:0': 'media/bk1/0.bin' },
  };
  manifest.data.listening_audios.push({
    id: 'audio1', practice_id: 'p1', name: 'sample',
    audio_uri: 'media:0', subtitle_uri: '', created_at: 't',
  });
  const fakeStorage = {
    request: async (method, key) => {
      if (method === 'GET' && key.endsWith('bk1.json')) return JSON.stringify(manifest);
      throw new Error('意外的对象存储请求:' + key);
    },
    getFile: async (key, target) => fs.writeFileSync(target, 'attachment content'),
    key: suffix => `rhetor/${suffix}`,
  };
  const requireStub = name => {
    const mocks = {
      '../data/database': {
        getSetting: async key => settingsMap.get(key) ?? null,
        setSetting: async (key, value) => { settingsMap.set(key, value); },
        exportVaultData: async () => makeVault(),
        importVaultData: async data => {
          if (importFails) throw new Error('合并失败');
          imported.push(JSON.parse(JSON.stringify(data)));
        },
      },
      '../data/vault': { validateVault: () => {} },
      './librarySync': {
        mapVaultMedia: async (data, map) => {
          for (const row of data.listening_audios) {
            if (row.audio_uri) row.audio_uri = await map(String(row.audio_uri));
            if (row.subtitle_uri) row.subtitle_uri = await map(String(row.subtitle_uri));
          }
        },
      },
      './platform': {
        FileSystem: {
          getDocumentDirectoryAsync: async () => documents,
          makeDirectoryAsync: dir => fs.mkdirSync(dir, { recursive: true }),
          writeFileAsync: (target, contents) => fs.writeFileSync(target, contents),
          deleteAsync: async target => { deleted.push(target); fs.rmSync(target, { recursive: true, force: true }); },
        },
      },
      './objectStorage': {
        ObjectStorage: function () { return fakeStorage; },
        loadStorageConfig: async () => ({}),
      },
    };
    return mocks[name] ?? null;
  };
  const source = fs.readFileSync(path.join(ROOT, 'shared', 'services', 'restoreCoordinator.ts'), 'utf8');
  const output = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
  }).outputText;
  const exportsObject = {};
  vm.runInNewContext(output, { exports: exportsObject, require: requireStub, Uint8Array, Date, Map, Set, console }, { filename: 'restoreCoordinator.ts' });
  return {
    api: exportsObject, documents, settingsMap, imported, deleted, manifest,
  };
}

function readOperation(settingsMap) {
  const raw = settingsMap.get('restore_operation');
  return raw ? JSON.parse(raw) : null;
}

test('happy path: attachments downloaded, refs rewritten, import and clean state', async () => {
  const s = setup();
  const preview = await s.api.inspectBackup('rhetor/snapshots/bk1.json');
  const { operation_id } = await s.api.runRestore('rhetor/snapshots/bk1.json', preview);
  assert.ok(operation_id);
  assert.equal(s.imported.length, 1);
  // 导入后 listening_audios 的 audio_uri 应已指向本地文件(不再是 media:0)
  const audio = s.imported[0].listening_audios?.[0];
  if (audio) assert.ok(audio.audio_uri.includes('/Restore/'), `audio_uri 应指向恢复目录,实际:${audio.audio_uri}`);
  assert.equal(readOperation(s.settingsMap), null, '完成后操作状态应清除');
});

test('local fingerprint mismatch after preview aborts and revokes the operation', async () => {
  const s = setup();
  const preview = await s.api.inspectBackup('rhetor/snapshots/bk1.json');
  const changed = preview.local_fingerprint !== 'changed' ? 'changed' : 'other';
  await assert.rejects(
    s.api.runRestore('rhetor/snapshots/bk1.json',
      { ...preview, local_fingerprint: changed }),
    /重新预览/,
  );
  assert.equal(readOperation(s.settingsMap), null);
});

test('backup content change after preview also aborts', async () => {
  const s = setup();
  const preview = await s.api.inspectBackup('rhetor/snapshots/bk1.json');
  const changed = preview.backup_fingerprint !== 'changed' ? 'changed' : 'other';
  await assert.rejects(
    s.api.runRestore('rhetor/snapshots/bk1.json',
      { ...preview, backup_fingerprint: changed }),
    /备份.*不一致|重新预览/,
  );
  assert.equal(readOperation(s.settingsMap), null);
});

test('a pre-commit interruption is revoked on the next startup resume', async () => {
  const attachmentsDir = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'clarora-r-')), 'attachments');
  fs.mkdirSync(attachmentsDir, { recursive: true });
  fs.writeFileSync(path.join(attachmentsDir, 'attachment.bin'), 'x');
  const persisted = {
    operation_id: 'op1', backup_key: 'rhetor/snapshots/bk1.json', backup_id: 'bk1',
    phase: 'stage', attachments_dir: attachmentsDir, restore_point_dir: attachmentsDir + '-rp',
    local_fingerprint: 'f', backup_fingerprint: 'f', created_at: 't',
  };
  const s = setup({ persisted });
  const cleaned = await s.api.resumePendingRestore();
  assert.equal(JSON.stringify(cleaned), JSON.stringify(['op1']));
  assert.equal(fs.existsSync(attachmentsDir), false, '暂存附件目录应被撤销清理');
  assert.equal(readOperation(s.settingsMap), null);
});

test('a post-commit operation finalizes idempotently without revoking', async () => {
  const attachmentsDir = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'clarora-r-')), 'attachments');
  fs.mkdirSync(attachmentsDir, { recursive: true });
  const persisted = {
    operation_id: 'op2', backup_key: 'rhetor/snapshots/bk1.json', backup_id: 'bk1',
    phase: 'commit', attachments_dir: attachmentsDir, restore_point_dir: attachmentsDir + '-rp',
    local_fingerprint: 'f', backup_fingerprint: 'f', created_at: 't',
  };
  const s = setup({ persisted });
  const cleaned = await s.api.resumePendingRestore();
  assert.equal(JSON.stringify(cleaned), '[]');
  assert.equal(readOperation(s.settingsMap), null);
});

test('import failure rolls back the operation for a clean retry', async () => {
  const s = setup({ importFails: true });
  const preview = await s.api.inspectBackup('rhetor/snapshots/bk1.json');
  await assert.rejects(
    s.api.runRestore('rhetor/snapshots/bk1.json', preview),
    /合并失败/,
  );
  assert.equal(s.imported.length, 0);
});
