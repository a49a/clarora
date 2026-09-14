const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const ts = require('typescript');
const vm = require('node:vm');
const path = require('node:path');

function setup() {
  const source = fs.readFileSync(path.join(__dirname, '../shared/data/subtitles.ts'), 'utf8');
  const exports = {};
  vm.runInNewContext(ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
  }).outputText, { exports });
  return { classify: exports.classifySubtitleLanguage, cue: (text) => ({ id: text, start: 0, end: 1, text }) };
}

test('bilingual when most cues carry a Chinese second line', async () => {
  const { classify, cue } = setup();
  const cues = [
    cue('Where do we go from here?\n我们该何去何从？'),
    cue('I have no idea.\n我不知道。'),
    cue('Let me think about it.\n让我想想。'),
  ];
  assert.equal(classify(cues), 'bilingual');
});

test('english only stays original', async () => {
  const { classify, cue } = setup();
  const cues = [cue('Where do we go from here?'), cue('I have no idea.'), cue('Let me think.')];
  assert.equal(classify(cues), 'original');
});

test('chinese-only lines count as chinese, not bilingual', async () => {
  const { classify, cue } = setup();
  const cues = [cue('我们该何去何从？'), cue('我不知道。'), cue('让我想想。')];
  assert.equal(classify(cues), 'chinese');
});

test('rare cjk tokens do not flip an english track', async () => {
  const { classify, cue } = setup();
  const cues = [cue('Where do we go from here?'), cue('I have no idea about 功夫.'), cue('Let me think. 功夫'), cue('Fine.')];
  assert.equal(classify(cues), 'original');
});
