const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { DatabaseSync } = require('node:sqlite');
const loader = require('./helpers/load-ts.cjs');

// Exercise the public importer and real database.ts against SQLite; only platform I/O is adapted.
function setup(t, platform = 'macos') {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'anki-text-'));
  const db = new DatabaseSync(path.join(dir, 'learning.db'));
  t.after(() => { db.close(); fs.rmSync(dir, { recursive: true, force: true }); });
  let failWord;
  const executeSql = (sql, params = []) => {
    if (failWord && params[0] === failWord) throw new Error('injected write failure');
    const stmt = db.prepare(sql);
    const rows = stmt.columns().length ? stmt.all(...params) : (stmt.run(...params), []);
    return [{ rows: { raw: () => rows } }];
  };
  const load = loader({
    'react-native': { Platform: { OS: platform }, NativeModules: {} },
    '../services/platform': { FileSystem: { readAsStringAsync: async file => fs.readFileSync(file, 'utf8') } },
    '../services/sqlite': { default: {
      enablePromise() {},
      openDatabase: async () => ({ executeSql, transaction: async fn => {
        db.exec('BEGIN');
        try { await fn({ executeSql }); db.exec('COMMIT'); }
        catch (error) { db.exec('ROLLBACK'); throw error; }
      } }),
    } },
  });
  const api = load(path.join(__dirname, '../shared/data/anki.ts'));
  const database = load(path.join(__dirname, '../shared/data/database.ts'));
  return { api, database, db, fail: word => { failWord = word; }, file: (text, name = '中文 cards.txt') => {
    const target = path.join(dir, name); fs.writeFileSync(target, text); return target;
  } };
}

test('sentence lifecycle: isolate categories, schedule, sync and delete', async t => {
  const { database: api, db } = setup(t);
  await assert.rejects(api.saveSentenceCard({ text: '  ', translation: '', notes: '' }));
  await api.saveSentenceCard({ text: ' Stay curious. ', translation: '保持好奇。', notes: '每天提醒自己' });
  await api.upsertWords([{word: 'curious', meaning: '好奇的'}]);
  const cards = await api.getReviewCards('sentence');
  assert.equal(cards.length, 1);
  assert.equal(cards[0].front, 'Stay curious.');
  assert.match(cards[0].back, /保持好奇/);
  assert.match(cards[0].back, /每天提醒自己/);
  assert.equal((await api.getReviewCards('word')).length, 1);
  assert.equal((await api.getReviewCards('all')).length, 2);
  assert.equal((await api.getDueReviewCards('sentence')).cards.length, 1);
  await api.gradeReviewCard(cards[0], 'good');
  assert.equal((await api.getDueReviewCards('sentence')).cards.length, 0);
  db.prepare("UPDATE review_schedule SET due_at = '2000-01-01T00:00:00.000Z' WHERE card_kind = 'sentence'").run();
  assert.equal((await api.getDueReviewCards('sentence', 0)).dueCount, 1);
  await api.replaceWords([{word: 'new', meaning: '新'}]);
  assert.equal((await api.getReviewCards('sentence')).length, 1);
  const snapshot = await api.getLibraryForSync();
  assert.equal(snapshot.sentenceCards.length, 1);
  await api.deleteReviewCard(cards[0]);
  assert.equal((await api.getReviewCards('sentence')).length, 0);
  assert.equal(db.prepare("SELECT count(*) n FROM review_schedule WHERE card_kind='sentence'").get().n, 0);
  await api.mergeSyncedLibrary({ words: [], practices: [], clips: [], sentenceCards: snapshot.sentenceCards, schedules: snapshot.schedules });
  assert.equal((await api.getReviewCards('sentence')).length, 1);
  assert.equal((await api.getDueReviewCards('sentence', 0)).dueCount, 1);
  await api.saveSentenceCard({text: 'Just a sentence.', translation: '', notes: ''});
  assert.equal((await api.getReviewCards('sentence')).length, 2);
});

test('sentence vault round trip and older vault without sentence table', async t => {
  const { database: api } = setup(t);
  await api.saveSentenceCard({text: 'A sentence.', translation: '一个句子。', notes: ''});
  const data = await api.exportVaultData();
  assert.equal(data.sentence_cards.length, 1);
  await api.replaceVaultData(data);
  assert.equal((await api.getReviewCards('sentence')).length, 1);
  delete data.sentence_cards;
  await api.replaceVaultData(data);
  assert.equal((await api.getReviewCards('sentence')).length, 0);
});
