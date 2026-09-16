const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const ts = require('typescript');
const vm = require('node:vm');
const path = require('node:path');
function setup() {
  let xhr;
  class Request {
    responseText = ''; status = 200;
    // eslint-disable-next-line @typescript-eslint/no-this-alias -- 捕获 XHR 实例供断言使用
    constructor() { xhr = this; }
    open() {} setRequestHeader() {} send() { this.sent = true; }
    abort() { this.onabort?.(); }
    chunk(text) { this.responseText += text; this.onprogress?.(); }
    end() { this.onload(); }
  }
  const exports = {};
  vm.runInNewContext(ts.transpileModule(fs.readFileSync(path.join(__dirname, '../shared/services/ai.ts'), 'utf8'), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
  }).outputText, { exports, XMLHttpRequest: Request, require: name => name === 'react-native' ? { Platform: { OS: 'macos' } } : name === '../data/database' ? { getSetting: async () => JSON.stringify({ baseUrl: 'http://localhost:8080/v1', model: 'local-model' }) } : name === './secrets' ? { secretStore: () => null } : {}, setTimeout, clearTimeout });
  const events = [];
  return { events, xhr: () => xhr, start: async (signal) => {
    const promise = exports.streamChatAnswer({ passage: 'Hello', question: '解释', signal, onEvent: e => events.push(e) });
    // Wait for the async setting read before the XHR is created.
    await new Promise(resolve => setImmediate(resolve));
    return { promise };
  } };
}
const frame = (type, text) => type === 'done' ? 'data: [DONE]\n\n' : `data: ${JSON.stringify({ choices: [{ delta: { [type === 'answer' ? 'content' : 'reasoning_content']: text } }] })}\n\n`;
test('streams split Chinese deltas before request completion, including CRLF frames', async () => {
  const s = setup(); const { promise } = await s.start();
  const text = frame('reasoning', '分析片段');
  s.xhr().chunk(text.slice(0, 9)); assert.equal(s.events.length, 0);
  s.xhr().chunk(text.slice(9)); assert.equal(s.events[0].text, '分析片段');
  s.xhr().chunk(frame('answer', '你好').replaceAll('\n', '\r\n'));
  assert.equal(s.events[1].text, '你好');
  s.xhr().chunk(frame('done')); s.xhr().end();
  const result = await promise; assert.equal(result.answer, '你好'); assert.equal(result.reasoning, '分析片段');
});
test('truncated stream and empty answers cannot be saved as completed responses', async () => {
  for (const complete of [false, true]) {
    const s = setup(); const { promise } = await s.start();
    s.xhr().chunk(frame(complete ? 'done' : 'answer', complete ? undefined : '未完成'));
    s.xhr().end(); await assert.rejects(promise, complete ? /未返回回答/ : /提前中断/);
  }
});
test('stream error after partial output is surfaced', async () => {
  const s = setup(); const { promise } = await s.start(); s.xhr().chunk(frame('answer', '部分'));
  s.xhr().chunk('data: {"error":{"message":"服务错误"}}\n\n'); s.xhr().end(); await assert.rejects(promise, /模型返回错误/);
});
test('only unsupported endpoint signals legacy fallback', async () => {
  for (const status of [404, 405, 502]) {
    const s = setup(); const { promise } = await s.start(); s.xhr().status = status; s.xhr().end();
    await assert.rejects(promise, error => status === 502 ? !error.code : error.code === 'no-stream');
  }
});
test('cancelled requests ignore late deltas', async () => {
  const s = setup(); const controller = new AbortController(); const { promise } = await s.start(controller.signal);
  controller.abort(); await assert.rejects(promise, { name: 'AbortError' });
  s.xhr().chunk(frame('answer', '旧回答')); assert.equal(s.events.length, 0);
});
