const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const load = require('./helpers/load-ts.cjs')();

const { parseSubtitleCues } = load(path.join(__dirname, '../shared/data/subtitles.ts'));
const { createBlindListeningMachine, resolveCueAtPosition } = load(path.join(__dirname, '../shared/ui/blindListening.ts'));

const json = (v) => JSON.stringify(v);
const sameJson = (a, b) => json(a) === json(b);


// 固定样本:internal-docs/blind-listening-design.md §9。
// 有效段:c1 0.5–2.5(双语)、c2 5–7、c3 7–9、c4 7–8(同起点)、c5 8.2–9.8(晚到重叠)、
// c6 11–12(仅中文);无效段:c7 end>duration、c8 零长度、c9 start>duration。
const FIXTURE = fs.readFileSync(path.join(__dirname, 'fixtures/blind-listening.srt'), 'utf8');
const DURATION_MS = 15_000;

function fixtureCues() {
  return parseSubtitleCues(FIXTURE).map((cue) => ({
    id: cue.id, startMs: cue.start * 1000, endMs: cue.end * 1000, text: cue.text,
  }));
}

test('fixture parses with the expected shape (3+ valid cues, bilingual and zh-only)', () => {
  const cues = fixtureCues();
  assert.ok(cues.length >= 6, `expected >=6 parsed cues, got ${cues.length}`);
  assert.match(cues[0].text, /Hello there\./);
  assert.match(cues[0].text, /你好。/);
  assert.ok(cues.some((c) => !/[A-Za-z]/.test(c.text)), 'zh-only cue present');
});

test('§4.2 covering cue: [start,end), overlap → latest start, same start → original order', () => {
  const cues = fixtureCues();
  const at = (ms) => resolveCueAtPosition(cues, ms, DURATION_MS);
  assert.ok(sameJson(at(1000), { kind: 'cue', index: 0, sparse: false }));
  // 同起点 7s:c3(先出现)胜出,不是更短的 c4。
  assert.ok(sameJson(at(7500), { kind: 'cue', index: 2, sparse: false }));
  // 重叠区间 8.2–9.8:更晚起点的 c5 胜出。
  assert.ok(sameJson(at(8500), { kind: 'cue', index: 4, sparse: false }));
  // 区间右端点排他:2.5ms 不再覆盖 c1,落到「最近结束」。
  assert.ok(sameJson(at(2500), { kind: 'last-ended', index: 0, sparse: false }));
});

test('§4.2 gaps and end: nearest ended cue, sparse label for long gaps', () => {
  const cues = fixtureCues();
  const at = (ms) => resolveCueAtPosition(cues, ms, DURATION_MS);
  assert.ok(sameJson(at(4900), { kind: 'last-ended', index: 0, sparse: false })); // 2.4s ago
  assert.ok(sameJson(at(10500), { kind: 'last-ended', index: 4, sparse: false })); // 9.8–11 间隙,最近结束是 c5
  assert.ok(sameJson(at(14900), { kind: 'last-ended', index: 5, sparse: false })); // 末尾空白 2.9s
  // 稀疏:距最近结束超过 10s(构造 cues,避免 fixture 段随时长变化转有效)。
  const sparseCues = [{ id: 'only', startMs: 0, endMs: 1000, text: 'x' }];
  assert.ok(sameJson(resolveCueAtPosition(sparseCues, 15_000, 20_000), { kind: 'last-ended', index: 0, sparse: true }));
  assert.ok(sameJson(resolveCueAtPosition(sparseCues, 10_900, 20_000), { kind: 'last-ended', index: 0, sparse: false }));
});

test('§4.2 before the first cue: no fallback to a future cue', () => {
  const cues = fixtureCues();
  assert.ok(sameJson(resolveCueAtPosition(cues, 200, DURATION_MS), { kind: 'before-first' }));
});

test('§4.2 invalid cues are ignored (0 ≤ start < end ≤ duration, finite)', () => {
  const cues = fixtureCues();
  const at = (ms) => resolveCueAtPosition(cues, ms, DURATION_MS);
  // c7(13–20,end>duration)若未被过滤会在 13.5s 覆盖;实际应落到 c6 的末尾空白。
  assert.ok(sameJson(at(13_500), { kind: 'last-ended', index: 5, sparse: false }));
  // c8(3–3 零长度)若未被过滤会在 3s 覆盖;实际是 c1 后的间隙。
  assert.ok(sameJson(at(3000), { kind: 'last-ended', index: 0, sparse: false }));
  // 直接构造的非法输入。
  const bad = [
    { id: 'neg', startMs: -1000, endMs: 1000, text: 'x' },
    { id: 'nan', startMs: NaN, endMs: 1000, text: 'x' },
    { id: 'inv', startMs: 2000, endMs: 1000, text: 'x' },
  ];
  assert.ok(sameJson(resolveCueAtPosition(bad, 500, DURATION_MS), { kind: 'unavailable' }));
});

test('§4.2 unknown duration or position → unavailable', () => {
  const cues = fixtureCues();
  assert.ok(sameJson(resolveCueAtPosition(cues, 1000, 0), { kind: 'unavailable' }));
  assert.ok(sameJson(resolveCueAtPosition(cues, NaN, DURATION_MS), { kind: 'unavailable' }));
});

// ── 状态机(§4.1/§4.2 请求时序)───────────────────────────────────────────

function snapshot(machine) {
  const s = machine.getSnapshot();
  return { p: s.presentation, r: s.revealed?.cueId ?? null, pending: s.revealPending };
}

test('reveal: locks snapshot from click position, awaits pause, then commits', async () => {
  const machine = createBlindListeningMachine();
  machine.enterBlind();
  let pauses = 0;
  let resolvePause;
  const pause = () => { pauses++; return new Promise((res) => { resolvePause = res; }); };
  const promise = machine.requestReveal({
    audioId: 'a1', cues: fixtureCues(), positionMs: 6000, durationMs: DURATION_MS,
    isPlaying: true, pause,
  });
  // 暂停等待期间:pending 且不随播放推进提交。
  assert.ok(sameJson(snapshot(machine), { p: 'blind', r: null, pending: true }));
  resolvePause();
  assert.ok(sameJson(await promise, { ok: true }));
  assert.equal(pauses, 1);
  const s = machine.getSnapshot();
  assert.equal(s.revealed.cueId, fixtureCues()[1].id);
  assert.equal(s.revealed.sparse, false);
});

test('reveal: already paused → commits without pausing again', async () => {
  const machine = createBlindListeningMachine();
  machine.enterBlind();
  const outcome = await machine.requestReveal({
    audioId: 'a1', cues: fixtureCues(), positionMs: 11500, durationMs: DURATION_MS,
    isPlaying: false, pause: async () => { throw new Error('must not pause'); },
  });
  assert.ok(sameJson(outcome, { ok: true }));
  assert.equal(machine.getSnapshot().revealed.sparse, false); // 11.5s 在 c6 内
});

test('reveal: pause failure keeps hidden and clears pending', async () => {
  const machine = createBlindListeningMachine();
  machine.enterBlind();
  const outcome = await machine.requestReveal({
    audioId: 'a1', cues: fixtureCues(), positionMs: 6000, durationMs: DURATION_MS,
    isPlaying: true, pause: async () => { throw new Error('boom'); },
  });
  assert.ok(sameJson(outcome, { ok: false, reason: 'pause-failed' }));
  assert.ok(sameJson(snapshot(machine), { p: 'blind', r: null, pending: false }));
});

test('reveal: invalidated during pause window drops the stale snapshot', async () => {
  const machine = createBlindListeningMachine();
  machine.enterBlind();
  let resolvePause;
  const pause = () => new Promise((res) => { resolvePause = res; });
  const promise = machine.requestReveal({
    audioId: 'a1', cues: fixtureCues(), positionMs: 6000, durationMs: DURATION_MS,
    isPlaying: true, pause,
  });
  machine.invalidate(); // 途中 seek/切音频
  resolvePause();
  assert.ok(sameJson(await promise, { ok: false, reason: 'invalidated' }));
  assert.ok(sameJson(snapshot(machine), { p: 'blind', r: null, pending: false }));
});

test('reveal: no target before the first cue / unavailable / while pending', async () => {
  const machine = createBlindListeningMachine();
  machine.enterBlind();
  assert.ok(sameJson(
    await machine.requestReveal({ audioId: 'a1', cues: fixtureCues(), positionMs: 200, durationMs: DURATION_MS, isPlaying: false, pause: async () => {} }),
    { ok: false, reason: 'no-target' },
  ));
  let hold;
  const promise = machine.requestReveal({
    audioId: 'a1', cues: fixtureCues(), positionMs: 6000, durationMs: DURATION_MS,
    isPlaying: true, pause: () => new Promise((res) => { hold = res; }),
  });
  assert.ok(sameJson(
    await machine.requestReveal({ audioId: 'a1', cues: fixtureCues(), positionMs: 7000, durationMs: DURATION_MS, isPlaying: true, pause: async () => {} }),
    { ok: false, reason: 'pending' },
  ));
  hold();
  assert.ok(sameJson(await promise, { ok: true }));
});

test('reveal: not in blind mode is refused', async () => {
  const machine = createBlindListeningMachine();
  assert.ok(sameJson(
    await machine.requestReveal({ audioId: 'a1', cues: fixtureCues(), positionMs: 6000, durationMs: DURATION_MS, isPlaying: false, pause: async () => {} }),
    { ok: false, reason: 'not-blind' },
  ));
});

test('lifecycle: mode switch and clearReveal reset reveal state', () => {
  const machine = createBlindListeningMachine();
  machine.enterBlind();
  assert.equal(machine.getSnapshot().presentation, 'blind');
  machine.exitBlind();
  assert.equal(machine.getSnapshot().presentation, 'subtitles');
  assert.equal(machine.getSnapshot().revealed, null);
  machine.enterBlind();
  machine.clearReveal();
  assert.ok(sameJson(snapshot(machine), { p: 'blind', r: null, pending: false }));
});

test('BL-001 regression: getSnapshot is referentially stable until state changes', async () => {
  const machine = createBlindListeningMachine();
  const a = machine.getSnapshot();
  const b = machine.getSnapshot();
  assert.ok(a === b, 'unchanged state must return the same snapshot object');
  machine.enterBlind();
  const c = machine.getSnapshot();
  assert.ok(c !== b, 'entering blind returns a new snapshot');
  assert.ok(c === machine.getSnapshot(), 'stable again after the change');
  // no-op transitions must not rebuild the snapshot
  machine.enterBlind();
  assert.ok(machine.getSnapshot() === c, 'idempotent enterBlind keeps the snapshot');
  // 无内容可清时 clearReveal 是值层面的 no-op:可见状态未变,快照不应重建。
  machine.clearReveal();
  assert.ok(machine.getSnapshot() === c, 'clearReveal with nothing revealed keeps the snapshot');
  // 揭晓后 clearReveal 改变可见状态 → 重建。
  await machine.requestReveal({ audioId: 'a1', cues: fixtureCues(), positionMs: 6000, durationMs: DURATION_MS, isPlaying: false, pause: async () => {} });
  const withReveal = machine.getSnapshot();
  machine.clearReveal();
  assert.ok(machine.getSnapshot() !== withReveal, 'clearReveal after a reveal rebuilds');
});
