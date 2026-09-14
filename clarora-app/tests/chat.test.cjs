const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');
const vm = require('node:vm');
const exportsObject = {};
vm.runInNewContext(ts.transpileModule(fs.readFileSync(path.join(__dirname, '../shared/data/chat.ts'), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText, { exports: exportsObject });
const { newChat, openReference, appendReference, restoreChats, chatHistory } = exportsObject;
const turns = () => [{ role: 'user', content: '为什么？' }, { role: 'assistant', content: '因为……' }];
test('collecting multiple sentences keeps the conversation, draft and line breaks', () => {
  const original = newChat({ text: 'First sentence.\nSecond sentence.', source: '听力' });
  original.draft = '一起解释这几句话'; original.messages = turns();
  const merged = appendReference(original, { text: ' Third sentence.\n第四句。 ', source: '听力' });
  assert.equal(merged.text, 'First sentence.\nSecond sentence.\n\nThird sentence.\n第四句。');
  assert.equal(merged.id, original.id);
  assert.equal(merged.draft, original.draft);
  assert.equal(merged.messages, original.messages);
  assert.equal(original.text, 'First sentence.\nSecond sentence.');
  assert.equal(restoreChats(JSON.stringify([merged]))[0].text, merged.text);
});
test('collecting rejects oversized material without losing the original', () => {
  const original = newChat({ text: 'a'.repeat(11998), source: '听力' });
  assert.throws(() => appendReference(original, { text: 'extra', source: '听力' }), /12000/);
  assert.equal(original.text.length, 11998);
  assert.equal(appendReference(original, { text: ' ', source: '听力' }), original);
});
test('reopening a reference restores its conversation and unsent draft', () => {
  const a = newChat({ text: 'Hello', source: '闪卡' });
  a.messages = turns(); a.draft = '再举一个例子';
  const b = newChat({ text: 'Other', source: '听力' });
  const restored = openReference([b, a], { text: ' Hello ', source: '闪卡' });
  assert.equal(restored[0], a);
  assert.equal(restored[0].draft, '再举一个例子');
  assert.equal(restored[0].messages.length, 2);
});
test('different reference starts a separate conversation without destroying the old one', () => {
  const a = newChat({ text: 'Hello', source: '闪卡' }); a.messages = turns();
  const next = openReference([a], { text: 'World', source: '闪卡' });
  assert.equal(next[0].messages.length, 0);
  assert.equal(next[1], a);
  assert.equal(a.text, 'Hello');
});
test('chat storage round trips reference, answers and draft; malformed data is rejected', () => {
  const a = newChat({ text: '引用', source: '听力' }); a.messages = turns(); a.draft = '追问';
  assert.equal(JSON.stringify(restoreChats(JSON.stringify([a]))), JSON.stringify([a]));
  for (const raw of ['invalid', 'null', '{}', '[null]', '[{"messages":[]}]']) assert.equal(restoreChats(raw).length, 0);
  a.messages.push({ role: 'user', content: 'failed request' });
  assert.equal(restoreChats(JSON.stringify([a])).length, 0);
});
test('request history keeps complete exchanges within server limits without mutating saved answers', () => {
  const messages = Array.from({ length: 15 }, turns).flat();
  messages[messages.length - 1].content = 'a'.repeat(13000);
  const history = chatHistory(messages);
  assert.equal(history.length, 24);
  assert.equal(history[0].role, 'user');
  assert.equal(history[23].content.length, 12000);
  assert.equal(messages[29].content.length, 13000);
});
test('reference history is bounded to twelve recent conversations', () => {
  let sessions = [];
  for (let i = 0; i < 20; i++) sessions = openReference(sessions, { text: String(i), source: '闪卡' });
  assert.equal(sessions.length, 12);
  assert.equal(sessions[0].text, '19');
});
