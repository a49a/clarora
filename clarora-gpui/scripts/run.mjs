// npm compatibility alias; the native launchers are scripts/dev.sh and dev.ps1.
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const root = fileURLToPath(new URL('../', import.meta.url));
const args = process.argv.slice(2);
const command = process.platform === 'win32'
  ? {
      file: 'powershell',
      args: [
        '-NoProfile',
        '-ExecutionPolicy',
        'Bypass',
        '-File',
        path.join(root, 'scripts/dev.ps1'),
        ...args,
      ],
    }
  : {
      file: 'sh',
      args: [path.join(root, 'scripts/dev.sh'), ...args],
    };
const result = spawnSync(command.file, command.args, { stdio: 'inherit' });
if (result.error) {
  console.error(result.error.message);
  process.exit(1);
}
process.exit(result.status ?? 1);
