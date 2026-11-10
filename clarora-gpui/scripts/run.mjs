import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
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
const pdfium = process.platform === 'darwin'
  ? 'target/debug/libpdfium.dylib'
  : process.platform === 'win32'
    ? 'target/debug/pdfium.dll'
    : null;
if (pdfium && !fs.existsSync(path.join(root, pdfium))) {
  console.log(`正在安装固定版本 PDFium：${pdfium}`);
  const python = process.platform === 'win32' ? 'python' : 'python3';
  const status = run(python, [path.join(root, 'scripts/install-pdfium.py'), path.join(root, 'target/debug')]);
  if (status !== 0) process.exit(status);
}
if (process.platform === 'darwin') {
  const status = run('sh', [path.join(root, 'scripts/package-macos.sh')]);
  if (status !== 0) process.exit(status);
  // Running inside the bundle lets macOS resolve the Dock icon and metadata.
  process.exit(run(path.join(root, 'target/Clarora.app/Contents/MacOS/clarora-gpui'), args));
}
process.exit(run('cargo', ['run', '--manifest-path', path.join(root, 'Cargo.toml'), '--locked', '--', ...args]));
