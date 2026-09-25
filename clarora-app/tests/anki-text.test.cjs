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

for (const platform of ['macos', 'windows']) {
  test(`Anki text ${platform}: HTML, metadata, duplicate import and schedules survive`, async t => {
    const s = setup(t, platform);
    await s.database.upsertWords([{word: 'arrest', meaning: 'old'}, {word: 'untouched', meaning: 'keep'}]);
    const id = s.db.prepare('SELECT id FROM words WHERE word = ?').get('arrest').id;
    s.db.prepare("INSERT INTO review_schedule (card_kind, card_id, reps) VALUES ('word', ?, 7)").run(String(id));
    const file = s.file('\ufeff#separator:tab\r\n#html:true\r\n#tags column:3\r\n#deck column:4\r\narrest\t<h1>arrest</h1><p>阻止</p><table><tr><td>check</td><td>curb</td></tr></table>\tGRE vocab\tRoot::Child\r\nat a premium\t<p>稀缺的 &amp; 昂贵的</p>\t\tRoot\r\n');
    for (let i = 0; i < 2; i++) assert.equal((await s.api.importAnkiDeck(file)).imported, 2);
    assert.equal(s.db.prepare('SELECT count(*) AS n FROM words').get().n, 3);
    const row = s.db.prepare('SELECT * FROM words WHERE word = ?').get('arrest');
    assert.equal(row.id, id);
    assert.equal(row.meaning, 'arrest\n阻止\ncheck curb');
    assert.equal(s.db.prepare('SELECT reps FROM review_schedule').get().reps, 7);
    assert.equal(s.db.prepare('SELECT meaning FROM words WHERE word = ?').get('at a premium').meaning, '稀缺的 & 昂贵的');
  });
}

test('quoted multiline fields, escaped quotes, comments and plain-text HTML literals', async t => {
  const s = setup(t);
  const result = await s.api.importAnkiDeck(s.file('#html:false\n# comment\n"a\tb"\t"line 1\n#not a comment\nline ""2"" <b> &amp;"\n\nplain\t2 < 3 > 1\n', 'CARDS.TSV'));
  assert.equal(result.imported, 2);
  assert.equal(s.db.prepare('SELECT meaning FROM words WHERE word = ?').get('a\tb').meaning, 'line 1\n#not a comment\nline "2" <b> &amp;');
  assert.equal(s.db.prepare('SELECT meaning FROM words WHERE word = ?').get('plain').meaning, '2 < 3 > 1');
});

test('metadata columns may precede fields; column names can select Back/Front', async t => {
  const s = setup(t);
  const file = s.file('#separator:Semicolon\n#deck column:1\n#tags column:4\n#guid column:5\n#notetype column:6\n#columns:Deck;Back;Front;Tags;GUID;Type\nRoot;"meaning; with delimiter";word;tag;id;Basic\n');
  assert.equal((await s.api.importAnkiDeck(file)).imported, 1);
  assert.equal(s.db.prepare('SELECT meaning FROM words WHERE word = ?').get('word').meaning, 'meaning; with delimiter');
});

test('empty, single-field and media-only rows do not become cards', async t => {
  const s = setup(t);
  for (const text of ['', '#separator:tab\nonly one field\n', '#html:true\nword\t<img src="x.png">\n\tanswer\n']) {
    assert.equal((await s.api.importAnkiDeck(s.file(text))).imported, 0);
  }
});

test('malformed text fails before any write; write failures roll back and retry works', async t => {
  const s = setup(t);
  await s.database.upsertWords([{word: 'existing', meaning: 'keep'}]);
  const malformed = [
    '#separator:unknown\na\tb', '#tags column:0\na\tb',
    '#tags column:3\n#deck column:3\na\tb\ttag',
    '#tags column:5\na\tb', '#html:yes\na\tb',
    'good\tanswer\n"bad\tanswer', 'good\tanswer\nbad\tanswer\textra',
    'good\tanswer\n"bad"suffix\tanswer',
  ];
  for (const text of malformed) {
    await assert.rejects(s.api.importAnkiDeck(s.file(text)));
    assert.equal(s.db.prepare('SELECT count(*) AS n FROM words').get().n, 1);
  }
  const file = s.file('first\tanswer\nsecond\tanswer');
  s.fail('second');
  await assert.rejects(s.api.importAnkiDeck(file), /injected/);
  assert.equal(s.db.prepare('SELECT count(*) AS n FROM words').get().n, 1);
  s.fail(null);
  assert.equal((await s.api.importAnkiDeck(file)).imported, 2);
  await assert.rejects(s.api.importAnkiDeck('/missing/cards.txt'));
});
