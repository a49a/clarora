const fs = require('node:fs');
const path = require('node:path');
const { spawnSync } = require('node:child_process');

if (process.platform !== 'darwin') {
  console.error('macOS builds require macOS, Xcode and CocoaPods.');
  process.exit(1);
}

const directory = path.resolve(__dirname, '../macos');
const lock = path.join(directory, 'Podfile.lock');
const manifest = path.join(directory, 'Pods/Manifest.lock');
const configurations = ['debug', 'release'].map(mode =>
  path.join(directory, `Pods/Target Support Files/Pods-Clarora-macOS/Pods-Clarora-macOS.${mode}.xcconfig`),
);
const installed = configurations.every(file => fs.existsSync(file))
  && fs.existsSync(lock) && fs.existsSync(manifest)
  && fs.readFileSync(lock).equals(fs.readFileSync(manifest));

if (!installed) {
  console.log('Preparing macOS native dependencies with pod install…');
  const result = spawnSync('pod', ['install'], {
    cwd: directory,
    stdio: 'inherit',
    env: { ...process.env, LANG: 'en_US.UTF-8', LC_ALL: 'en_US.UTF-8' },
  });
  if (result.error) {
    console.error(`Could not run CocoaPods: ${result.error.message}. Install CocoaPods and retry npm run macos.`);
  }
  if (result.error || result.status !== 0) process.exit(result.status || 1);
}
