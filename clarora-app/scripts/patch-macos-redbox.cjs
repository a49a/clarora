const fs = require('node:fs');
const path = require('node:path');

const original = '  [[RCTKeyWindow() contentViewController] dismissViewController:self];';
const replacement = `  // Clarora: the key window may be the error sheet or a new reload window.
  // Only the controller that presented this sheet may dismiss it. A second
  // dismissal during bridge invalidation is harmless once it is detached.
  NSViewController *presenter = self.presentingViewController;
  if (presenter != nil && [presenter.presentedViewControllers containsObject:self]) {
    [presenter dismissViewController:self];
  }`;

function patchRedBox(source) {
  if (source.includes(replacement)) return source;
  if (!source.includes(original)) {
    throw new Error('RCTRedBox dismissal changed; review the macOS reload patch before upgrading.');
  }
  return source.replace(original, replacement);
}

if (require.main === module) {
  // The workspace installs macOS under its own name and the react-native alias.
  for (const name of ['react-native-macos', 'react-native']) {
    const file = path.join(__dirname, '..', 'node_modules', name, 'React', 'CoreModules', 'RCTRedBox.mm');
    const source = fs.readFileSync(file, 'utf8');
    const patched = patchRedBox(source);
    if (patched !== source) fs.writeFileSync(file, patched);
  }
}

module.exports = { patchRedBox };
