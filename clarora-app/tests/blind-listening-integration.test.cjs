// Failure cases first: collecting bypasses the page entry; old cues are attributed
// to a newly selected sound; a queued pause survives seek/switch invalidation.
// Exercise actual page/provider callbacks with controlled native dependencies.
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const ts = require('typescript');
const load = require('./helpers/load-ts.cjs')();
const { createBlindListeningMachine } = load(path.join(__dirname, '../shared/ui/blindListening.ts'));
function callback(file, name, env) {
  const src = ts.createSourceFile(file, fs.readFileSync(path.join(__dirname, '../shared', file), 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  let expr;
  function visit(node) {
    if (ts.isVariableDeclaration(node) && node.name.getText(src) === name) {
      expr = (ts.isCallExpression(node.initializer) ? node.initializer.arguments[0] : node.initializer).getText(src);
    }
    ts.forEachChild(node, visit);
  }
  visit(src);
  assert.ok(expr, `missing callback: ${name}`);
  return vm.runInNewContext(ts.transpileModule(`const extracted = ${expr}; extracted;`, {
    compilerOptions: { target: ts.ScriptTarget.ES2020 },
  }).outputText, { ...env });
}

test('blind AI entry takes precedence over collecting; normal collecting still returns to its chat', () => {
  for (const overrideCollecting of [false, true]) {
    let entered = 0, reopened = 0;
    const launch = callback('ui/AIChatProvider.tsx', 'launch', {
      collecting: true, open: () => reopened++,
      entryRef: { current: { run: () => entered++, overrideCollecting } },
    });
    launch();
    assert.equal(entered, overrideCollecting ? 1 : 0);
    assert.equal(reopened, overrideCollecting ? 0 : 1);
  }
});

test('fresh and resetContext end collecting while retaining history', () => {
  for (const name of ['fresh', 'resetContext']) {
    let collecting = true, visible = true;
    let sessions = [{ id: 'old', text: 'old answer', messages: [], draft: '' }];
    const original = sessions[0];
    callback('ui/AIChatProvider.tsx', name, {
      cancel() {}, setError() {}, setNotice() {}, changed: { current: false },
      setCollecting: x => { collecting = x; }, setVisible: x => { visible = x; },
      newChat: () => ({ id: 'new', text: '', messages: [], draft: '' }),
      setSessions: fn => { sessions = fn(sessions); },
    })();
    assert.equal(collecting, false);
    assert.equal(visible, name === 'fresh');
    assert.equal(sessions[0].text, '');
    assert.ok(sessions.includes(original));
  }
});

function context() {
  const machine = createBlindListeningMachine(); machine.enterBlind();
  const cues = [{ id: 'cue', start: 0, end: 2, text: 'correct' }];
  const source = { audioId: 'B', generation: 4, revision: 1, cues };
  return {
    blindMachine: machine, selectedAudio: { id: 'B' }, mountedRef: { current: true },
    soundRef: { current: { pauseAsync: async () => {} } },
    audioGenerationRef: { current: 4 }, loadedAudioIdRef: { current: 'B' },
    loadingAudioIdRef: { current: null }, subtitleStateRef: { current: source },
    subtitleCues: cues, subtitleTaskRef: { current: false },
  };
}
test('page rejects loading, mismatched audio/cues/generation and missing sound', () => {
  const mutations = [
    x => { x.loadingAudioIdRef.current = 'B'; },
    x => { x.loadedAudioIdRef.current = 'A'; },
    x => { x.subtitleStateRef.current = { ...x.subtitleStateRef.current, audioId: 'A' }; },
    x => { x.subtitleStateRef.current = { ...x.subtitleStateRef.current, generation: 3 }; },
    x => { x.soundRef.current = null; },
    x => { x.subtitleCues = []; },
  ];
  for (const mutate of mutations) {
    const env = context(); mutate(env);
    assert.equal(callback('screens/listening.tsx', 'getBlindPlaybackContext', env)(), null);
  }
});
test('captured playback context expires on subtitle replacement and audio switching', () => {
  for (const mutate of [
    x => { x.subtitleStateRef.current = { ...x.subtitleStateRef.current, revision: 2 }; },
    x => { x.audioGenerationRef.current++; },
    x => { x.loadingAudioIdRef.current = 'C'; },
  ]) {
    const env = context();
    const captured = callback('screens/listening.tsx', 'getBlindPlaybackContext', env)();
    assert.equal(captured.isCurrent(), true);
    mutate(env);
    assert.equal(captured.isCurrent(), false);
  }
});
test('actual reveal callback skips an invalidated queued pause and can reveal normally afterward', async () => {
  const env = context(); let pauses = 0, runQueued;
  env.soundRef.current.pauseAsync = async () => { pauses++; };
  env.getBlindPlaybackContext = callback('screens/listening.tsx', 'getBlindPlaybackContext', env);
  env.positionMsRef = { current: 1000 }; env.durationMs = 5000;
  env.isPlayingRef = { current: true }; env.showToast = () => {};
  env.enqueueAudioOperation = operation => new Promise((resolve, reject) => {
    runQueued = () => Promise.resolve().then(operation).then(resolve, reject);
  });
  const reveal = callback('screens/listening.tsx', 'handleBlindReveal', env);
  const pending = reveal();
  env.blindMachine.invalidate();
  await runQueued(); await pending;
  assert.equal(pauses, 0);
  assert.equal(env.blindMachine.getSnapshot().revealed, null);
  const valid = reveal(); await runQueued(); await valid;
  assert.equal(pauses, 1);
  assert.equal(env.blindMachine.getSnapshot().revealed.text, 'correct');
});
