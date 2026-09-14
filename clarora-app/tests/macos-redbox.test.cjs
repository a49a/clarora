const { test } = require('node:test');
const assert = require('node:assert/strict');
const { patchRedBox } = require('../scripts/patch-macos-redbox.cjs');

test('macOS error sheet dismissal uses its presenter and guards repeated dismissal', () => {
  const source = `#if !TARGET_OS_OSX
  [self dismissViewControllerAnimated:YES completion:nil];
#else
  [[RCTKeyWindow() contentViewController] dismissViewController:self];
#endif`;
  const patched = patchRedBox(source);
  assert.ok(!patched.includes('RCTKeyWindow()'));
  assert.ok(patched.includes('self.presentingViewController'));
  assert.ok(patched.includes('[presenter.presentedViewControllers containsObject:self]'));
  assert.ok(patched.includes('[self dismissViewControllerAnimated:YES completion:nil]'));
  assert.equal(patchRedBox(patched), patched);
});

test('dependency upgrades cannot silently skip the reload crash fix', () => {
  assert.throws(() => patchRedBox('different upstream implementation'), /review the macOS reload patch/);
});
