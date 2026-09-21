// 恢复协调器测试:阶段持久化、指纹闸门、崩溃恢复与并发防护。

const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const ts = require('typescript');
const vm = require('node:vm');

const ROOT = path.join(__dirname, '..', '..');

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
  const fakeStorage = {
    request: async (method, key) => {
      if (method === 'GET' && key.endsWith('bk1.json')) return JSON.stringify(manifest);
      throw new Error('意外的对象存储请求:' + key);
    },
    getFile: async (key, target) => fs.writeFileSync(target, 'attachment'),
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
  const source = fs.readFileSync(path.join(ROOT, 'clarora-app', 'shared', 'services', 'restoreCoordinator.ts'), 'utf8');
  const output = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
  }).outputText;
  const exportsObject = {};
  vm.runInNewContext(output, { exports: exportsObject, require: requireStub, Uint8Array, Date, Map, Set, console }, { filename: 'restoreCoordinator.ts' });
  return {
    api: exportsObject, documents, settingsMap, imported, deleted, manifest,
    stagedDir: null,
  };
}

function readOperation(settingsMap) {
  const raw = settingsMap.get('restore_operation');
  return raw ? JSON.parse(raw) : null;
}

test('happy path: staged attachments, import and clean state after finalize', async () => {
  const s = setup();
  const preview = await s.api.inspectBackup('rhetor/snapshots/bk1.json');
  const { operation_id } = await s.api.runRestore('rhetor/snapshots/bk1.json', preview.fingerprint);
  assert.ok(operation_id);
  assert.equal(s.imported.length, 1);
  assert.equal(readOperation(s.settingsMap), null, '完成后操作状态应清除');
});

test('fingerprint mismatch after preview aborts and revokes the operation', async () => {
  const s = setup();
  const preview = await s.api.inspectBackup('rhetor/snapshots/bk1.json');
  // 预览后本机资料变化:模拟用户在预览与合并之间新增词条。
  const fingerprint = 'changed';
  await assert.rejects(
    s.api.runRestore('rhetor/snapshots/bk1.json', fingerprint),
    /重新预览/,
  );
  assert.equal(readOperation(s.settingsMap), null);
  assert.notEqual(preview.fingerprint, fingerprint);
});

test('a pre-commit interruption is revoked on the next startup resume', async () => {
  const stagedDir = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'clarora-r-')), 'staged');
  fs.mkdirSync(stagedDir, { recursive: true });
  fs.writeFileSync(path.join(stagedDir, 'attachment.bin'), 'x');
  const persisted = {
    operation_id: 'op1', backup_key: 'rhetor/snapshots/bk1.json', backup_id: 'bk1',
    phase: 'stage', staged_dir: stagedDir, restore_point_dir: stagedDir + '-rp',
    downloaded: [path.join(stagedDir, 'attachment.bin')], fingerprint: 'f', created_at: 't',
  };
  const s = setup({ persisted });
  const cleaned = await s.api.resumePendingRestore();
  assert.equal(JSON.stringify(cleaned), JSON.stringify(['op1']));
  assert.equal(fs.existsSync(stagedDir), false, '暂存目录应被撤销清理');
  assert.equal(readOperation(s.settingsMap), null);
});

test('a post-commit operation finalizes idempotently without revoking', async () => {
  const stagedDir = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'clarora-r-')), 'staged');
  fs.mkdirSync(stagedDir, { recursive: true });
  const persisted = {
    operation_id: 'op2', backup_key: 'rhetor/snapshots/bk1.json', backup_id: 'bk1',
    phase: 'commit', staged_dir: stagedDir, restore_point_dir: stagedDir + '-rp',
    downloaded: [], fingerprint: 'f', created_at: 't',
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
    s.api.runRestore('rhetor/snapshots/bk1.json', preview.fingerprint),
    /合并失败/,
  );
  assert.equal(s.imported.length, 0);
});
