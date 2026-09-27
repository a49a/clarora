const { test } = require('node:test');
const assert = require('node:assert/strict');
const load = require('./helpers/load-ts.cjs')();

const { resolveStudySettingsMaskTap } = load(require('node:path').join(__dirname, '../shared/ui/studySettingsMask.ts'));

// 与听力页遮罩命中一致的状态:无打开设置、AI 空闲、已有选中练习组
const idle = { activeSettings: null, aiBusy: false, hasSelectedPractice: true };

test('M-005: AI 忙碌时遮罩命中练习组/音频入口只收起设置,不打开选择列表', () => {
  assert.equal(resolveStudySettingsMaskTap('practice', { ...idle, aiBusy: true }).kind, 'dismiss');
  assert.equal(resolveStudySettingsMaskTap('audio', { ...idle, aiBusy: true }).kind, 'dismiss');
});

test('M-005: AI 空闲时遮罩命中练习组/音频入口与入口按钮一致', () => {
  assert.equal(resolveStudySettingsMaskTap('practice', idle).kind, 'practice');
  assert.equal(resolveStudySettingsMaskTap('audio', idle).kind, 'audio');
  // 与入口 disabled={!selectedPractice || !!aiStatus} 一致:无选中练习组时音频入口不可用
  assert.equal(resolveStudySettingsMaskTap('audio', { ...idle, hasSelectedPractice: false }).kind, 'dismiss');
});

test('设置触发器:再点已打开的收起,点另一个切换', () => {
  assert.equal(resolveStudySettingsMaskTap('subtitle', { ...idle, activeSettings: 'subtitle' }).kind, 'close');
  assert.equal(resolveStudySettingsMaskTap('speed', { ...idle, activeSettings: 'speed' }).kind, 'close');
  const toSubtitle = resolveStudySettingsMaskTap('subtitle', { ...idle, activeSettings: 'speed' });
  assert.equal(toSubtitle.kind, 'open');
  assert.equal(toSubtitle.settings, 'subtitle');
  assert.equal(resolveStudySettingsMaskTap('subtitle', idle).kind, 'open');
  assert.equal(resolveStudySettingsMaskTap('speed', idle).kind, 'open');
});

test('遮罩命中 AI 入口与退出不受 AI 忙碌影响(与原行为一致)', () => {
  assert.equal(resolveStudySettingsMaskTap('ai', { ...idle, aiBusy: true }).kind, 'ai');
  assert.equal(resolveStudySettingsMaskTap('exit', { ...idle, aiBusy: true }).kind, 'exit');
});
