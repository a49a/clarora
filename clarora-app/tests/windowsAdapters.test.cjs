const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const ts = require('typescript');

function load(file, mocks) {
  const exports = {};
  const source = ts.transpileModule(fs.readFileSync(path.join(__dirname, '..', file), 'utf8'), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
  }).outputText;
  vm.runInNewContext(source, { exports, require: name => {
    if (!(name in mocks)) throw new Error(`Unexpected dependency: ${name}`);
    return mocks[name];
  }, setInterval, clearInterval });
  return exports;
}
test('Windows database preserves parameters and returns the shared row interface', async () => {
  const calls = [];
  const native = {
    open: async name => calls.push(name),
    batch: async (statements, atomic) => {
      calls.push(JSON.parse(JSON.stringify({ statements, atomic })));
      return [{ rows: [{ id: 1, word: 'hello', meaning: '你好' }], rowsAffected: 1, insertId: 1 }];
    },
  };
  const SQLite = load('shared/services/sqlite.windows.ts', { 'react-native': { NativeModules: { RNWindowsDatabase: native } } }).default;
  const db = await SQLite.openDatabase({ name: 'clarora.db' });
  const [result] = await db.executeSql('SELECT * FROM words WHERE meaning = ?', ['你好']);
  assert.equal(result.rows.length, 1);
  assert.equal(result.rows.item(0).meaning, '你好');
  assert.equal(result.rows.raw()[0].word, 'hello');
  assert.deepEqual(calls[1], { statements: [{ sql: 'SELECT * FROM words WHERE meaning = ?', params: ['你好'] }], atomic: false });
  await db.transaction(tx => { tx.executeSql('DELETE FROM words'); tx.executeSql('INSERT INTO words VALUES (?)', ["it's"]); });
  assert.equal(calls[2].atomic, true);
  assert.equal(calls[2].statements.length, 2);
  assert.equal(calls[2].statements[1].params[0], "it's");
  native.batch = async () => { throw new Error('constraint failed'); };
  await assert.rejects(db.transaction(tx => tx.executeSql('invalid')), /constraint failed/);
});
test('Windows file operations use native storage and HTTP, preserving Unicode paths and multipart fields', async () => {
  const calls = [];
  const files = {
    readFile: async p => { calls.push(['read', p]); return '中文'; },
    upload: async (...args) => { calls.push(['upload', ...args]); return { status: 201, body: 'ok' }; },
    download: async (...args) => { calls.push(['download', ...args]); },
    pickFile: async () => null,
  };
  const platform = load('shared/services/platform.ts', {
    'react-native': { Platform: { OS: 'windows' }, NativeModules: { RNWindowsFiles: files } },
    './rnfs': new Proxy({}, { get() { throw new Error('Must not use RNFS on Windows'); } }),
  });
  const uri = 'file:///C:/study%20notes/%E4%BD%A0%E5%A5%BD.txt';
  assert.equal(await platform.FileSystem.readAsStringAsync(uri), '中文');
  assert.deepEqual(calls[0], ['read', 'C:/study notes/你好.txt']);
  assert.equal(platform.nativePath('file://server/share/lesson.mp3'), '//server/share/lesson.mp3');
  await platform.FileSystem.uploadFileAsync('https://example.test/upload', uri, { Authorization: 'test' }, 'audio', 'audio/mpeg');
  assert.deepEqual(calls[1], ['upload', 'https://example.test/upload', 'C:/study notes/你好.txt', { Authorization: 'test' }, 'audio', 'audio/mpeg']);
  await platform.FileSystem.downloadFileAsync('https://example.test/a', uri);
  assert.equal(calls[2][2], 'C:/study notes/你好.txt');
  assert.equal((await platform.DocumentPicker.getDocumentAsync({})).canceled, true);
});
