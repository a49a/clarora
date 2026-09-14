const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const ts = require('typescript');
const vm = require('node:vm');
const path = require('node:path');
function load(file, mocks = {}) {
  const source = ts.transpileModule(fs.readFileSync(path.join(__dirname, '..', file), 'utf8'), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
  }).outputText;
  const exports = {};
  vm.runInNewContext(source, { exports, require: name => mocks[name] ?? require(name), setInterval, clearInterval, console });
  return exports;
}
const { audioSegments, discoverSubtitles, pickNext } = load('shared/data/discover.ts');
const audio = { id: 'a', name: 'Lesson', audio_uri: '/lesson.m4a' };
const cue = (start, end, text) => ({ start, end, text });
test('segments preserve cue boundaries, bilingual text, and natural breaks', () => {
  const result = audioSegments(audio, [cue(0, 10, 'Hello\n你好'), cue(10, 25, 'World.\n世界。'), cue(28, 40, 'Another sentence.')]);
  assert.equal(result.length, 2);
  assert.equal(result[0].audio.startMs, 0);
  assert.equal(result[0].audio.endMs, 25000);
  assert.equal(result[0].front, 'Hello World.');
  assert.equal(result[0].back, '你好\n世界。');
  assert.equal(result[1].audio.startMs, 28000);
  assert.equal(result[0].subtitles[1].startMs, 10000);
  assert.equal(result[0].subtitles[1].translation, '世界。');
});
test('clickable subtitles keep actual timestamps and stay inside the clip', () => {
  const result = discoverSubtitles([cue(0, 3, 'Before'), cue(3, 8, '<i>Hello</i>\n你好'), cue(8, 15, 'World'), cue(20, 25, 'After')], 5000, 10000);
  assert.equal(result.length, 2);
  assert.equal(result[0].startMs, 5000);
  assert.equal(result[0].text, 'Hello');
  assert.equal(result[1].startMs, 8000);
  assert.equal(result[1].endMs, 10000);
});
test('invalid cues and oversized single cues cannot create oversized segments', () => {
  const result = audioSegments(audio, [cue(-1, 4, 'bad'), cue(3, 2, 'bad'), cue(0, 80, 'too long'), cue(80, 90, 'Valid.')]);
  assert.equal(result.length, 1);
  assert.equal(result[0].audio.startMs, 80000);
});
test('selection avoids repeats until exhaustion and varies categories', () => {
  const a = { key: 'a', kind: 'word' }, b = { key: 'b', kind: 'ai' }, c = { key: 'c', kind: 'word' };
  const pool = [a, b, c];
  const history = [a];
  const second = pickNext(pool, history, new Set(), new Set(), () => 0);
  assert.equal(second.key, 'b');
  history.push(second);
  assert.equal(pickNext(pool, history, new Set(), new Set(), () => 0).key, 'c');
  assert.equal(pickNext([a], [a], new Set(), new Set(), () => 0).key, 'a');
  assert.equal(pickNext([], [], new Set(), new Set()), undefined);
});
for (const [os, moduleName] of [['macos', 'RNMacAudio'], ['android', 'RNAndroidAudio'], ['ios', 'RNIOSAudio'], ['windows', 'RNWindowsAudio']]) {
test(`${os}: pause resumes in place; subtitle seek preserves the next preload and clip end`, async () => {
  const calls = [];
  let position = 3000;
  let state;
  const api = Object.fromEntries(['feedPrepare', 'feedPause', 'feedPlay', 'feedUnload'].map(name => [name, async (...args) => { calls.push([name, ...args]); }]));
  api.feedStatus = async () => ({ positionMillis: position, isPlaying: true });
  const { DiscoverAudio } = load('shared/services/discoverAudio.ts', { './platform': { nativeLearningAudio: () => api, nativePath: uri => uri } });
  const a = { key: 'a', audio: { uri: '/a', startMs: 1000, endMs: 10000 } };
  const b = { key: 'b', audio: { uri: '/b', startMs: 2000, endMs: 15000 } };
  const failures = [];
  const player = new DiscoverAudio((playing, ms) => { state = { playing, ms }; }, e => failures.push(e));
  const settle = (ms = 15) => new Promise(resolve => setTimeout(resolve, ms));
  try {
    player.show(a, b);
    await settle(130);
    player.pause();
    await settle();
    assert.equal(state.playing, false);
    assert.equal(state.ms, 3000);
    const prepares = () => calls.filter(c => c[0] === 'feedPrepare');
    player.resume(a, b);
    await settle();
    assert.equal(prepares().length, 2, 'resume must retain the loaded player position');
    assert.equal(state.playing, true);
    player.show(a, b, 6000);
    await settle();
    assert.equal(prepares().at(-1)[3], 6000);
    assert.equal(prepares().filter(c => c[2] === '/b').length, 1, 'seek must preserve preloaded next audio');
    position = 11000;
    await settle(130);
    assert.equal(state.playing, false);
    assert.equal(state.ms, 10000);
    player.resume(a, b);
    await settle();
    assert.equal(prepares().at(-1)[3], 1000, 'completed clips replay from the beginning');
    player.pause();
    player.show(b);
    await settle();
    assert.equal(state.playing, true, 'the next card auto-plays even after pausing');
    assert.equal(prepares().filter(c => c[2] === '/b').length, 1);
    assert.deepEqual(failures, []);
  } finally { player.dispose(); }
});
test(`${os}: preload does not play next audio; rapid navigation discards obsolete requests`, async () => {
  const calls = [];
  const api = Object.fromEntries(['feedPrepare', 'feedPause', 'feedPlay', 'feedUnload'].map(name => [name, async (...args) => { calls.push([name, ...args]); }]));
  api.feedStatus = async () => ({ positionMillis: 0, isPlaying: true });
  const platform = load('shared/services/platform.ts', {
    'react-native': { Platform: { OS: os }, NativeModules: { [moduleName]: api } },
    './rnfs': {},
  });
  const { DiscoverAudio } = load('shared/services/discoverAudio.ts', { './platform': platform });
  const a = { key: 'a', audio: { uri: '/a', startMs: 100, endMs: 10000 } };
  const b = { key: 'b', audio: { uri: '/b', startMs: 200, endMs: 10000 } };
  const failures = [];
  const player = new DiscoverAudio(() => {}, e => failures.push(e));
  try {
    player.show(a, b);
    await new Promise(resolve => setTimeout(resolve, 10));
    assert.equal(calls.filter(c => c[0] === 'feedPrepare').length, 2);
    assert.equal(calls.filter(c => c[0] === 'feedPlay').length, 1);
    player.show(b);
    await new Promise(resolve => setTimeout(resolve, 10));
    assert.equal(calls.filter(c => c[0] === 'feedPrepare').length, 2, 'cached audio should not reload');
    calls.length = 0;
    player.show(a);
    player.show(b);
    await new Promise(resolve => setTimeout(resolve, 10));
    assert.equal(calls.filter(c => c[0] === 'feedPlay').length, 1);
    assert.equal(calls.find(c => c[0] === 'feedPrepare')[2], '/b');
    assert.deepEqual(failures, []);
  } finally { player.dispose(); }
});
}
