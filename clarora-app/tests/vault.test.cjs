const { test } = require('node:test');
const assert = require('node:assert/strict');
const { DatabaseSync } = require('node:sqlite');
const path = require('node:path');
const loader = require('./helpers/load-ts.cjs');
const file = path.join(__dirname, '../shared/data/database.ts');
function setup() {
  const sqlite = new DatabaseSync(':memory:'); sqlite.exec('PRAGMA foreign_keys=ON');
  let injectFailure = false;
  const execute = (sql, params = []) => {
    if (injectFailure && sql.startsWith('INSERT INTO ai_cards')) throw new Error('injected write failure');
    const stmt = sqlite.prepare(sql);
    const rows = stmt.columns().length ? stmt.all(...params) : (stmt.run(...params), []);
    return [{ rows: { raw: () => rows, length: rows.length, item: i => rows[i] } }];
  };
  const db = { executeSql: async (...args) => execute(...args), transaction: async fn => {
    sqlite.exec('BEGIN');
    try { fn({ executeSql: execute }); sqlite.exec('COMMIT'); } catch (e) { sqlite.exec('ROLLBACK'); throw e; }
  } };
  const api = loader({ 'react-native': { Platform: { OS: 'macos' } }, '../services/sqlite': { default: { enablePromise() {}, openDatabase: async () => db } } })(file);
  return { api, sqlite, fail: () => { injectFailure = true; } };
}
const plain = value => JSON.parse(JSON.stringify(value));
const date = '2026-09-14T00:00:00Z';
test('vault round trip excludes all credentials and remaps word schedules/favorites across devices', async () => {
  const source = setup(), target = setup();
  await source.api.setSetting('object_storage_config', '{"secretAccessKey":"do-not-export"}');
  await target.api.setSetting('ai_config', 'private-key');
  source.sqlite.exec("INSERT INTO words(id,word,meaning,source) VALUES(1,'apple','苹果','manual'),(2,'book','书','manual')");
  source.sqlite.prepare('INSERT INTO review_schedule(card_kind,card_id,last_graded_at,reps) VALUES(?,?,?,?)').run('word', '1', date, 3);
  source.sqlite.prepare('INSERT INTO discover_favorites(key,payload) VALUES(?,?)').run('word:1', JSON.stringify({ key: 'word:1', kind: 'word', front: 'apple' }));
  await source.api.saveAiRecord('ocr-a', 'ocr', { pageId: 'ocr-a', text: 'hello' });
  target.sqlite.exec("INSERT INTO words(id,word,meaning,source) VALUES(1,'book','本机书','manual')");
  const backup = plain(await source.api.exportVaultData());
  assert.ok(!JSON.stringify(backup).includes('do-not-export'));
  await target.api.importVaultData(backup);
  await target.api.importVaultData(backup);
  assert.equal(target.sqlite.prepare('SELECT COUNT(*) AS n FROM words').get().n, 2);
  const appleId = target.sqlite.prepare("SELECT id FROM words WHERE word='apple'").get().id;
  assert.notEqual(appleId, 1);
  assert.equal(target.sqlite.prepare('SELECT card_id FROM review_schedule').get().card_id, String(appleId));
  const favorite = target.sqlite.prepare('SELECT key,payload FROM discover_favorites').get();
  assert.equal(favorite.key, `word:${appleId}`); assert.equal(JSON.parse(favorite.payload).key, favorite.key);
  assert.equal(target.sqlite.prepare("SELECT meaning FROM words WHERE word='book'").get().meaning, '本机书');
  assert.equal(await target.api.getSetting('ai_config'), 'private-key');
  assert.equal((await target.api.listAiRecords('ocr'))[0].text, 'hello');
  source.sqlite.close(); target.sqlite.close();
});
test('a later restore does not regress review grades or double-count statistics', async () => {
  const { api, sqlite } = setup(); const data = plain(await api.exportVaultData());
  data.words = [{ id: 1, word: 'word', meaning: 'meaning', source: 'manual', created_at: date }];
  data.review_schedule = [{ card_kind: 'word', card_id: '1', ease: 2.5, interval_days: 3, due_at: date, reps: 4, lapses: 0, suspended: 0, last_graded_at: date, created_at: date }];
  data.study_stats = [{ day: '2026-09-14', listen_seconds: 60, review_count: 4, pomodoro_count: 1 }];
  await api.importVaultData(data); data.review_schedule[0].last_graded_at = '2026-09-13T00:00:00Z'; data.review_schedule[0].reps = 1;
  await api.importVaultData(data);
  assert.equal(sqlite.prepare('SELECT reps FROM review_schedule').get().reps, 4);
  assert.equal(sqlite.prepare('SELECT listen_seconds FROM study_stats').get().listen_seconds, 60);
  sqlite.close();
});
test('restore rolls back earlier table writes if a later insert fails', async () => {
  const { api, sqlite, fail } = setup(); const data = plain(await api.exportVaultData());
  data.words = [{ id: 1, word: 'new', meaning: '新', source: 'manual', created_at: date }];
  data.ai_cards = [{ id: 'a', question: 'q', answer: 'a', context_text: '', created_at: date }];
  fail(); await assert.rejects(api.importVaultData(data), /injected/);
  assert.equal(sqlite.prepare('SELECT COUNT(*) AS n FROM words').get().n, 0);
  sqlite.close();
});
test('invalid backups reject before any database mutation', async () => {
  const { api, sqlite } = setup(); const data = plain(await api.exportVaultData());
  data.words = [{ id: 1, word: 'new', meaning: {}, source: 'manual', created_at: date }];
  await assert.rejects(api.importVaultData(data), /字段/);
  assert.equal(sqlite.prepare('SELECT COUNT(*) AS n FROM words').get().n, 0);
  sqlite.close();
});
