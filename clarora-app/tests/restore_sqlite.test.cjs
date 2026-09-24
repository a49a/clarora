// 恢复与回退的真实 SQLite 集成验证:真实 database.ts + restoreCoordinator.ts +
// librarySync.mapVaultMedia,经 node:sqlite 适配器驱动临时数据库与临时附件文件。
// 故障注入在适配器层按 SQL 片段命中,保证事务原子性与标识阶段可以被断言。

const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { DatabaseSync } = require('node:sqlite');
const loader = require('./helpers/load-ts.cjs');

const ROOT = path.join(__dirname, '..');

function emptyVault() {
  return {
    words: [], listening_practices: [], listening_audios: [], clip_cards: [],
    ai_cards: [], video_clips: [], review_schedule: [], study_stats: [],
    discover_favorites: [], ai_records: [],
  };
}

const BACKUP_DATA = (() => {
  const data = emptyVault();
  data.words.push({ id: 7, word: 'beta', meaning: 'from backup', source: 'directory', created_at: 't0' });
  data.listening_practices.push({ id: 'p1', name: 'Backup Practice', created_at: 't0' });
  data.listening_audios.push({ id: 'a1', practice_id: 'p1', name: 'sample', audio_uri: 'media:0', subtitle_uri: 'media:1', created_at: 't0' });
  data.ai_cards.push({ id: 'c1', question: 'q', answer: 'a', context_text: '', created_at: 't0' });
  data.review_schedule.push({ card_kind: 'word', card_id: '7', ease: 2.6, interval_days: 1, due_at: 't2', reps: 1, lapses: 0, suspended: 0, last_graded_at: 't2', created_at: 't0' });
  return data;
})();

const BACKUP_FILES = {
  'media:0': 'snapshots/bk1/a/0.bin',
  'media:1': 'snapshots/bk1/b/0.bin', // 与 media:0 同 basename、不同内容
};
const FILE_CONTENTS = { 'snapshots/bk1/a/0.bin': 'AUDIO-BYTES', 'snapshots/bk1/b/0.bin': 'SUBTITLE-BYTES' };

function setup() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'clarora-sqlite-'));
  const documents = path.join(dir, 'Documents') + path.sep;
  fs.mkdirSync(documents, { recursive: true });
  const native = new DatabaseSync(path.join(dir, 'clarora.db'));
  let failOnSql = null;
  // 同步抛错:database.ts 的 tx.executeSql 不经 await,故障必须像生产驱动
  // 一样在调用点同步抛出,事务回调才能捕获并回滚。
  const executeSql = (sql, params = []) => {
    if (failOnSql && sql.toUpperCase().includes(failOnSql.toUpperCase())) throw new Error(`注入失败: ${failOnSql}`);
    const head = sql.trim().slice(0, 6).toUpperCase();
    if (head === 'SELECT' || head === 'PRAGM') {
      return [{ rows: { raw: () => native.prepare(sql).all(...params) } }];
    }
    native.prepare(sql).run(...params);
    return [{ rows: { raw: () => [] } }];
  };
  const sqliteMock = {
    enablePromise: () => {},
    openDatabase: async () => ({
      executeSql,
      async transaction(fn) {
        native.exec('BEGIN');
        const tx = { executeSql };
        try { await fn(tx); native.exec('COMMIT'); }
        catch (error) { try { native.exec('ROLLBACK'); } catch { /* 已回滚 */ } throw error; }
      },
    }),
  };
  const fakeStorage = {
    request: async (method, key) => {
      if (method === 'GET' && key.endsWith('bk1.json')) {
        return JSON.stringify({ version: 2, id: 'bk1', createdAt: 't0', data: BACKUP_DATA, files: BACKUP_FILES });
      }
      throw new Error('意外的对象存储请求:' + key);
    },
    getFile: async (key, target) => {
      if (!FILE_CONTENTS[key]) throw new Error('未知附件:' + key);
      fs.writeFileSync(target, FILE_CONTENTS[key]);
    },
    key: suffix => `rhetor/${suffix}`,
  };
  const mocks = {
    'react-native': { Platform: { OS: 'macos', select: options => options.macos } },
    // database.ts 使用 default 导入(TS 转译后取 .default),同时保留具名访问。
    '../services/sqlite': { default: sqliteMock, ...sqliteMock },
    './platform': { FileSystem: {
      getDocumentDirectoryAsync: async () => documents,
      makeDirectoryAsync: target => fs.mkdirSync(target, { recursive: true }),
      writeFileAsync: (target, contents) => fs.writeFileSync(target, contents),
      deleteAsync: async target => { fs.rmSync(target, { recursive: true, force: true }); },
    } },
    './objectStorage': { ObjectStorage: function () { return fakeStorage; }, loadStorageConfig: async () => ({}) },
  };
  // restoreCoordinator 真实模块;librarySync 不 mock(mapVaultMedia 为真实实现);
  // '../data/database' 与 '../data/vault' 均为真实模块,经由同一 sqlite 适配器落盘。
  const load = loader(mocks);
  const api = load(path.join(ROOT, 'shared', 'services', 'restoreCoordinator.ts'));
  // 同一 loader 实例下再次加载即得到协调器所用的同一数据库模块实例。
  const database = load(path.join(ROOT, 'shared', 'data', 'database.ts'));
  const db = { executeSql };
  const rows = async (sql, params = []) => (await db.executeSql(sql, params))[0].rows.raw().map(row => ({ ...row }));
  const attachmentsDirs = () => fs.readdirSync(path.join(documents, 'Clarora', 'Restore'));
  return {
    api, database, db, rows, documents, attachmentsDirs, dir,
    setFailOnSql: sql => { failOnSql = sql; },
    async seed() {
      await database.getSetting('bootstrap'); // 触发惰性建表
      // 原生 SQL 播种:importVaultData 会在空库上重映射 id,拿不到精确的 id=42。
      await db.executeSql("INSERT INTO words (id, word, meaning, source, created_at) VALUES (42, 'alpha', 'old', 'manual', 't0')");
      await db.executeSql("INSERT INTO review_schedule (card_kind, card_id, ease, interval_days, due_at, reps, lapses, suspended, last_graded_at, created_at) VALUES ('word', '42', 2.5, 3, 't1', 2, 0, 0, 't1', 't0')");
      await db.executeSql("INSERT INTO study_stats (day, listen_seconds, review_count, pomodoro_count) VALUES ('2026-09-24', 60, 5, 0)");
      await database.setSetting('vault_token', 'secret-token'); // 凭证类设置,不应进快照
    },
  };
}

test('merge into real SQLite remaps ids, pairs same-basename attachments and clears the marker', async () => {
  const s = setup();
  await s.seed();
  const preview = await s.api.inspectBackup('rhetor/snapshots/bk1.json');
  await s.api.runRestore('rhetor/snapshots/bk1.json', preview);
  const words = await s.rows('SELECT id, word, meaning FROM words ORDER BY word');
  const alpha = words.find(w => w.word === 'alpha');
  const beta = words.find(w => w.word === 'beta');
  assert.ok(alpha && beta, '合并后应同时有 alpha 与 beta');
  assert.equal(alpha.id, 42, '本机已有词条 id 不变');
  assert.equal(alpha.meaning, 'old', '合并时本机释义优先');
  const audios = await s.rows('SELECT practice_id, name, audio_uri, subtitle_uri FROM listening_audios');
  assert.equal(audios.length, 1);
  const practices = await s.rows('SELECT id, name FROM listening_practices');
  assert.equal(audios[0].practice_id, practices[0].id, '音频外键应指向重映射后的练习组');
  assert.match(audios[0].audio_uri, /media_0$/);
  assert.match(audios[0].subtitle_uri, /media_1$/);
  assert.equal(fs.readFileSync(audios[0].audio_uri, 'utf8'), 'AUDIO-BYTES');
  assert.equal(fs.readFileSync(audios[0].subtitle_uri, 'utf8'), 'SUBTITLE-BYTES', '同名附件不得互相覆盖');
  const schedule = await s.rows('SELECT card_id FROM review_schedule WHERE card_kind = ? ORDER BY card_id', ['word']);
  assert.deepEqual(schedule.map(r => r.card_id).sort(), ['42', String(beta.id)].sort(), '复习计划应跟随词条重映射');
  assert.ok(!await s.database.getSetting('restore_operation'), '完成后操作标识清除');
  assert.ok(await s.api.hasRestorePoint(), '合并后应存在恢复点');
  assert.equal((await s.rows("SELECT value FROM app_settings WHERE key = 'vault_token'"))[0].value, 'secret-token', '凭证类设置不进入快照流程');
});

test('a mid-import failure rolls the transaction back, keeps the marker at stage and a retry succeeds', async () => {
  const s = setup();
  await s.seed();
  s.setFailOnSql('INSERT INTO ai_cards');
  const preview = await s.api.inspectBackup('rhetor/snapshots/bk1.json');
  await assert.rejects(s.api.runRestore('rhetor/snapshots/bk1.json', preview), /注入失败/);
  const words = await s.rows('SELECT word FROM words');
  assert.deepEqual(words, [{ word: 'alpha' }], '事务应整体回滚,备份词条不残留');
  assert.equal((await s.rows('SELECT COUNT(*) AS n FROM review_schedule'))[0].n, 1, '复习计划保持合并前状态');
  const marker = JSON.parse(await s.database.getSetting('restore_operation'));
  assert.equal(marker.phase, 'stage', '标识必须停在 stage 供 resume 撤销');
  assert.ok(await s.api.hasRestorePoint());
  const cleaned = await s.api.resumePendingRestore();
  assert.equal(cleaned.length, 1, 'resume 应撤销暂存附件');
  assert.ok(!fs.existsSync(path.join(s.documents, 'Clarora', 'Restore', cleaned[0], 'attachments')), '暂存附件目录应被删除');
  assert.ok(!await s.database.getSetting('restore_operation'));
  // 重试(撤除注入)成功
  s.setFailOnSql(null);
  try {
    await s.api.runRestore('rhetor/snapshots/bk1.json', preview);
  } catch (e) { console.log('RETRY ERROR:', e.message); }
  assert.equal((await s.rows('SELECT COUNT(*) AS n FROM words WHERE word = ?', ['beta']))[0].n, 1, 'beta');
});

test('rollback restores the snapshot exactly; a failed rollback keeps state and the retry matches', async () => {
  const s = setup();
  await s.seed();
  const preview = await s.api.inspectBackup('rhetor/snapshots/bk1.json');
  await s.api.runRestore('rhetor/snapshots/bk1.json', preview);
  // 合并后修改本机数据:改释义、加词条
  await s.db.executeSql("UPDATE words SET meaning = 'changed' WHERE word = 'alpha'");
  await s.db.executeSql("INSERT INTO words (word, meaning, source, created_at) VALUES ('gamma', 'g', 'manual', 't3')");
  // 注入回退失败:快照插入阶段失败,已 DELETE 的数据必须随事务整体恢复
  s.setFailOnSql('INSERT INTO review_schedule');
  await assert.rejects(s.api.restoreFromCheckpoint(), /注入失败/);
  assert.equal((await s.rows("SELECT meaning FROM words WHERE word = 'alpha'"))[0].meaning, 'changed', '回退失败不得破坏当前数据');
  assert.equal((await s.rows('SELECT COUNT(*) AS n FROM words WHERE word = ?', ['beta']))[0].n, 1, 'beta');
  assert.ok(await s.api.hasRestorePoint(), '回退失败后恢复点必须仍在');
  // 重试成功:数据与快照逐表一致
  s.setFailOnSql(null);
  await s.api.restoreFromCheckpoint();
  const words = await s.rows('SELECT id, word, meaning FROM words');
  assert.deepEqual(words, [{ id: 42, word: 'alpha', meaning: 'old' }]);
  assert.equal((await s.rows("SELECT card_id FROM review_schedule WHERE card_kind = 'word'"))[0].card_id, '42');
  assert.equal((await s.rows('SELECT listen_seconds FROM study_stats'))[0].listen_seconds, 60);
  assert.equal(await s.api.hasRestorePoint(), false, '回退成功后恢复点清除');
  assert.equal((await s.rows("SELECT value FROM app_settings WHERE key = 'vault_token'"))[0].value, 'secret-token', '回退不触碰凭证与设备设置');
});
