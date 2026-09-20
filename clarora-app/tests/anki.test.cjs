const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { DatabaseSync } = require('node:sqlite');
const { zipSync } = require('fflate');
const loader = require('./helpers/load-ts.cjs');

const SEP = String.fromCharCode(31);

// 造一个最小但真实的 Anki collection.anki2(SQLite),再打包成 .apkg。
function buildApkg(notes, models) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'clarora-anki-'));
  const dbPath = path.join(directory, 'collection.anki2');
  const db = new DatabaseSync(dbPath);
  db.exec('CREATE TABLE col (id INTEGER PRIMARY KEY, models TEXT)');
  db.exec('CREATE TABLE notes (id INTEGER PRIMARY KEY, mid INTEGER, flds TEXT)');
  db.prepare('INSERT INTO col (id, models) VALUES (1, ?)').run(JSON.stringify(models));
  const insert = db.prepare('INSERT INTO notes (id, mid, flds) VALUES (?, ?, ?)');
  notes.forEach((note, index) => insert.run(index + 1, note.mid, note.flds.join(SEP)));
  db.close();
  const zip = zipSync({ 'collection.anki2': fs.readFileSync(dbPath) });
  const apkgPath = path.join(directory, 'deck.apkg');
  fs.writeFileSync(apkgPath, zip);
  return { directory, apkgPath, apkgBase64: fs.readFileSync(apkgPath).toString('base64') };
}

function setup({ platform = 'macos', apkgBase64, failWriteBase64 = false } = {}) {
  const documents = fs.mkdtempSync(path.join(os.tmpdir(), 'clarora-anki-docs-'));
  const upserted = [];
  const deleted = [];
  const api = loader({
    './database': {
      upsertWords: async words => { upserted.push(...words); return words.length; },
    },
    '../services/platform': {
      currentPlatform: platform,
      nativePath: value => value,
      FileSystem: {
        readBase64Async: async () => apkgBase64,
        getDocumentDirectoryAsync: async () => documents + '/',
        writeBase64Async: async (target, base64) => {
          if (failWriteBase64) throw new Error('denied');
          fs.writeFileSync(target, Buffer.from(base64, 'base64'));
        },
        deleteFileAsync: async target => { deleted.push(target); fs.unlinkSync(target); },
      },
    },
    '../services/sqlite': {
      default: {
        openDatabase: options => {
          const handle = new DatabaseSync(path.join(documents, options.createFromLocation));
          return {
            executeSql: async sql => [{ rows: { raw: () => handle.prepare(sql).all() } }],
            close: async () => handle.close(),
          };
        },
      },
    },
    'react-native': { Platform: { OS: platform } },
  }, {})(path.join(__dirname, '../shared/data/anki.ts'));
  return { api, documents, upserted, deleted };
}

test('imports Basic notes with named Front/Back fields, stripping HTML and entities', async () => {
  const fixture = buildApkg(
    [
      { mid: 1, flds: ['What is <b>Anki</b>?', 'A <i>spaced repetition</i> tool &amp; more'] },
      { mid: 1, flds: ['second card front', 'second card back'] },
    ],
    { 1: { name: 'Basic', flds: [{ name: 'Front', ord: 0 }, { name: 'Back', ord: 1 }] } },
  );
  const { api, upserted, deleted } = setup({ apkgBase64: fixture.apkgBase64 });
  const { imported } = await api.importAnkiDeck('/picked/deck.apkg');
  assert.equal(imported, 2);
  assert.equal(JSON.stringify(upserted), JSON.stringify([
    { word: 'What is Anki?', meaning: 'A spaced repetition tool & more' },
    { word: 'second card front', meaning: 'second card back' },
  ]));
  assert.ok(deleted.every(target => !fs.existsSync(target)), 'temp database should be cleaned up');
});

test('falls back to the first two fields when field names are unknown', async () => {
  const fixture = buildApkg(
    [{ mid: 7, flds: ['capital of France', 'Paris'] }],
    { 7: { name: 'Mystery', flds: [{ name: 'A', ord: 0 }, { name: 'B', ord: 1 }] } },
  );
  const { api, upserted } = setup({ apkgBase64: fixture.apkgBase64 });
  await api.importAnkiDeck('/picked/deck.apkg');
  assert.equal(JSON.stringify(upserted), JSON.stringify([{ word: 'capital of France', meaning: 'Paris' }]));
});

test('cloze deletions resolve to the hidden text', async () => {
  const fixture = buildApkg(
    [{ mid: 2, flds: ['{{c1::Paris}} is the capital of {{c2::France}}', 'geography'] }],
    { 2: { name: 'Cloze', type: 1, flds: [{ name: 'Text', ord: 0 }, { name: 'Extra', ord: 1 }] } },
  );
  const { api, upserted } = setup({ apkgBase64: fixture.apkgBase64 });
  await api.importAnkiDeck('/picked/deck.apkg');
  assert.equal(JSON.stringify(upserted), JSON.stringify([{ word: 'Paris is the capital of France', meaning: 'geography' }]));
});

test('single-field models are skipped, empty fronts are skipped', async () => {
  const fixture = buildApkg(
    [
      { mid: 3, flds: ['only one field'] },
      { mid: 4, flds: ['', 'no front'] },
    ],
    {
      3: { name: 'One', flds: [{ name: 'Text', ord: 0 }] },
      4: { name: 'Two', flds: [{ name: 'Front', ord: 0 }, { name: 'Back', ord: 1 }] },
    },
  );
  const { api, upserted } = setup({ apkgBase64: fixture.apkgBase64 });
  const { imported } = await api.importAnkiDeck('/picked/deck.apkg');
  assert.equal(imported, 0);
  assert.deepEqual(upserted, []);
});

test('new encrypted anki21b decks explain the export option instead of failing cryptically', async () => {
  const fixture = buildApkg([{ mid: 1, flds: ['a', 'b'] }], { 1: { flds: [{ name: 'F', ord: 0 }, { name: 'B', ord: 1 }] } });
  fs.rmSync(path.join(fixture.directory, 'collection.anki2'));
  const zip = zipSync({ 'collection.anki21b': Buffer.from('zstd-bytes') });
  const apkgPath = path.join(fixture.directory, 'deck.apkg');
  fs.writeFileSync(apkgPath, zip);
  const { api } = setup({ apkgBase64: fs.readFileSync(apkgPath).toString('base64') });
  await assert.rejects(api.importAnkiDeck('/picked/deck.apkg'), /支持旧版本/);
});

test('non-macOS platforms are rejected with a clear message', async () => {
  const fixture = buildApkg([{ mid: 1, flds: ['a', 'b'] }], { 1: { flds: [{ name: 'F', ord: 0 }, { name: 'B', ord: 1 }] } });
  const { api } = setup({ platform: 'android', apkgBase64: fixture.apkgBase64 });
  await assert.rejects(api.importAnkiDeck('/picked/deck.apkg'), /移动端无需导入/);
});

test('imports on Windows through the native read-only snapshot query', async () => {
  const fixture = buildApkg(
    [{ mid: 1, flds: ['Windows front', 'Windows back'] }],
    { 1: { name: 'Basic', flds: [{ name: 'Front', ord: 0 }, { name: 'Back', ord: 1 }] } },
  );
  const documents = fs.mkdtempSync(path.join(os.tmpdir(), 'clarora-anki-win-'));
  const upserted = [];
  const deleted = [];
  const api = loader({
    './database': { upsertWords: async words => { upserted.push(...words); return words.length; } },
    '../services/platform': {
      currentPlatform: 'windows',
      nativePath: value => value,
      FileSystem: {
        readBase64Async: async () => fixture.apkgBase64,
        getDocumentDirectoryAsync: async () => documents,
        writeBase64Async: async (target, base64) => fs.writeFileSync(target, Buffer.from(base64, 'base64')),
        deleteFileAsync: async target => { deleted.push(target); fs.unlinkSync(target); },
      },
    },
    '../services/sqlite': { default: { openDatabase: () => { throw new Error('Windows 不应走 openDatabase'); } } },
    'react-native': {
      Platform: { OS: 'windows' },
      NativeModules: {
        RNWindowsDatabase: {
          querySnapshot: async (target, sql) => {
            const handle = new DatabaseSync(target);
            try { return JSON.stringify(handle.prepare(sql).all()); } finally { handle.close(); }
          },
        },
      },
    },
  }, {})(path.join(__dirname, '../shared/data/anki.ts'));
  const { imported } = await api.importAnkiDeck('/picked/deck.apkg');
  assert.equal(imported, 1);
  assert.equal(JSON.stringify(upserted), JSON.stringify([{ word: 'Windows front', meaning: 'Windows back' }]));
  assert.ok(deleted.length > 0, 'temp database should be cleaned up');
});
