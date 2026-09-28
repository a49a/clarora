// React Native's renderer rejects a different React runtime version at startup.
// Typecheck and native compilation do not catch this mismatch.
const fs = require('node:fs');
const path = require('node:path');

const reactVersion = require('react/package.json').version;
const rendererPath = path.join(
  path.dirname(require.resolve('react-native-macos/package.json')),
  'Libraries/Renderer/implementations/ReactNativeRenderer-dev.js',
);
const rendererSource = fs.readFileSync(rendererPath, 'utf8');
const rendererVersion = rendererSource.match(/reconcilerVersion: "([^"]+)"/)?.[1];

if (!rendererVersion) {
  console.error('Cannot determine the React Native renderer version. Check the installed react-native-macos package.');
  process.exit(1);
}
if (reactVersion !== rendererVersion) {
  console.error(`React ${reactVersion} does not match the React Native renderer ${rendererVersion}. Align react before launching the app.`);
  process.exit(1);
}
console.log(`React and React Native renderer match: ${reactVersion}`);
