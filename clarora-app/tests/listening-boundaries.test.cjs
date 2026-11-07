// Exercise the same playback session boundary as RN audio study.
const { test } = require('node:test');
const assert = require('node:assert/strict');
const path = require('node:path');
const load = require('./helpers/load-ts.cjs')();
const root = path.join(__dirname, '../shared/services');
const tick = () => new Promise(resolve => setImmediate(resolve));

test('switching or closing a listening session suppresses late loads and status, then the next session takes over', async () => {
  const { ListeningPlayback } = load(path.join(root, 'listeningPlayback.ts'));
  const events = [];
  let releaseA;
  let oldStatus;
  const a = new ListeningPlayback(async (uri, _rate, status) => {
    events.push(`load:${uri}`);
    oldStatus = status;
    await new Promise(resolve => { releaseA = resolve; });
    return { unloadAsync: async () => { events.push(`unload:${uri}`); } };
  });
  const b = new ListeningPlayback(async (uri, _rate, status) => {
    events.push(`load:${uri}`);
    status({ isLoaded: true, positionMillis: 3 });
    return { unloadAsync: async () => { events.push(`unload:${uri}`); } };
  });
  const first = a.load('A', 1, () => events.push('stale-status'));
  await tick();
  const closing = a.unload();
  const second = b.load('B', 1, status => events.push(`B:${status.positionMillis}`));
  oldStatus({ isLoaded: true, positionMillis: 100 });
  releaseA();
  assert.equal(await first, null);
  await closing;
  assert.equal(await second, b.generation);
  assert.deepEqual(events, ['load:A', 'unload:A', 'load:B', 'B:3']);
  await b.unload();
});
