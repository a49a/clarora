import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const root = fileURLToPath(new URL('../', import.meta.url));
const args = process.argv.slice(2);
function run(command, argv) {
  const result = spawnSync(command, argv, { stdio: 'inherit' });
  if (result.error) {
    console.error(result.error.message);
    process.exit(1);
  }
  return result.status ?? 1;
}
if (process.platform === 'darwin') {
  const status = run('sh', [path.join(root, 'scripts/package-macos.sh')]);
  if (status !== 0) process.exit(status);
  // Running inside the bundle lets macOS resolve the Dock icon and metadata.
  process.exit(run(path.join(root, 'target/Clarora.app/Contents/MacOS/clarora-gpui'), args));
}
process.exit(run('cargo', ['run', '--manifest-path', path.join(root, 'Cargo.toml'), '--locked', '--', ...args]));
