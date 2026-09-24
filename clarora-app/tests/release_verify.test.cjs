// verify-release.sh 场景矩阵:以生成的 fixture 驱动真实脚本,覆盖嵌套 Appx
// 完整包/缺件包、无输入、不存在输入与 tar 回退分支。默认 `npm test` 收集。
// Windows runner 上跳过(依赖 POSIX bash/符号链接),由 Ubuntu/macOS job 覆盖。

const { test } = require('node:test');
const assert = require('node:assert/strict');
const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const ROOT = path.join(__dirname, '..');
const SCRIPT = path.join(ROOT, 'scripts', 'verify-release.sh');

const hasBash = spawnSync('bash', ['-c', 'true']).status === 0;
const isPosix = process.platform !== 'win32';

function runScript(args, env = {}) {
  return spawnSync('bash', [SCRIPT, ...args], { encoding: 'utf8', env: { ...process.env, ...env } });
}

function workspace() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'clarora-verify-'));
  return { dir, pkg: path.join(dir, 'pkg') };
}

// 生成一个仅 STORE(不压缩)的 zip:node 内建实现,不依赖外部 zip 命令。
// 结构:本地文件头 + 数据 + 中央目录 + EOCD;CRC32 为查表实现。
const CRC_TABLE = (() => {
  const table = new Int32Array(256);
  for (let n = 0; n < 256; n += 1) {
    let c = n;
    for (let k = 0; k < 8; k += 1) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c;
  }
  return table;
})();
function crc32(buf) {
  let c = 0xffffffff;
  for (const byte of buf) c = CRC_TABLE[(c ^ byte) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}
function makeZip(files) {
  const local = [];
  const central = [];
  let offset = 0;
  for (const [name, content] of Object.entries(files)) {
    const nameBuf = Buffer.from(name);
    const data = Buffer.from(content);
    const crc = crc32(data);
    const localHeader = Buffer.alloc(30);
    localHeader.writeUInt32LE(0x04034b50, 0);
    localHeader.writeUInt16LE(20, 4);
    localHeader.writeUInt16LE(0, 6);
    localHeader.writeUInt16LE(0, 8);
    localHeader.writeUInt16LE(0, 10);
    localHeader.writeUInt16LE(0, 12);
    localHeader.writeUInt32LE(crc, 14);
    localHeader.writeUInt32LE(data.length, 18);
    localHeader.writeUInt32LE(data.length, 22);
    localHeader.writeUInt16LE(nameBuf.length, 26);
    local.push(localHeader, nameBuf, data);
    const centralHeader = Buffer.alloc(46);
    centralHeader.writeUInt32LE(0x02014b50, 0);
    centralHeader.writeUInt16LE(20, 4);
    centralHeader.writeUInt16LE(20, 6);
    centralHeader.writeUInt32LE(crc, 16);
    centralHeader.writeUInt32LE(data.length, 20);
    centralHeader.writeUInt32LE(data.length, 24);
    centralHeader.writeUInt16LE(nameBuf.length, 28);
    centralHeader.writeUInt32LE(offset, 42);
    central.push(centralHeader, nameBuf);
    offset += 30 + nameBuf.length + data.length;
  }
  const centralBuf = Buffer.concat(central);
  const eocd = Buffer.alloc(22);
  eocd.writeUInt32LE(0x06054b50, 0);
  eocd.writeUInt16LE(Object.keys(files).length, 8);
  eocd.writeUInt16LE(Object.keys(files).length, 10);
  eocd.writeUInt32LE(centralBuf.length, 12);
  eocd.writeUInt32LE(offset, 16);
  return Buffer.concat([...local, centralBuf, eocd]);
}

const FULL_PKG = { 'Clarora.exe': 'exe', 'clarora_asr.dll': 'asr', 'pdfium.dll': 'pdfium' };

test('no input and nonexistent input fail immediately', () => {
  assert.equal(runScript([]).status, 1, '无参数应失败');
  assert.equal(runScript(['/tmp/clarora-no-such.app']).status, 1, '不存在的 .app 应失败');
  assert.equal(runScript(['/tmp/clarora-no-such.app', '/tmp/clarora-no-such.zip']).status, 1, '缺 zip 应失败');
});

test('a complete package nested inside an msix passes on the unzip branch', () => {
  const w = workspace();
  const msix = makeZip(FULL_PKG);
  fs.writeFileSync(path.join(w.dir, 'Clarora_0.1.0_x64.msix'), msix);
  fs.writeFileSync(path.join(w.dir, 'outer.zip'), makeZip({ 'Clarora_0.1.0_x64.msix': msix }));
  const result = runScript([path.join(w.dir, 'outer.zip')]);
  assert.equal(result.status, 0, result.stdout + result.stderr);
  assert.match(result.stdout, /clarora_asr\.dll/);
  fs.rmSync(w.dir, { recursive: true, force: true });
});

test('a nested msix missing a required component fails without touching the working directory', () => {
  const w = workspace();
  const missing = { 'Clarora.exe': 'exe', 'clarora_asr.dll': 'asr' }; // 缺 pdfium.dll
  fs.writeFileSync(path.join(w.dir, 'm.msix'), makeZip(missing));
  fs.writeFileSync(path.join(w.dir, 'outer.zip'), makeZip({ 'Clarora_0.1.0_x64.msix': makeZip(missing) }));
  const result = runScript([path.join(w.dir, 'outer.zip')]);
  assert.equal(result.status, 1, result.stdout + result.stderr);
  assert.match(result.stdout, /pdfium\.dll/);
  fs.rmSync(w.dir, { recursive: true, force: true });
});

test('the tar fallback branch accepts the complete package and rejects a missing component', () => {
  if (!hasBash || !isPosix) return; // Windows runner 由 unzip 分支覆盖
  // GNU tar 不支持 zip:必须有 bsdtar(或系统 tar 即 bsdtar)才执行本场景。
  // 缺失时响亮失败而不是静默跳过;CI 的 ubuntu job 已预装 libarchive-tools,
  // 本地环境请执行 `sudo apt-get install -y libarchive-tools`。
  const tarVersion = spawnSync('bash', ['-c', 'tar --version'], { encoding: 'utf8' }).stdout ?? '';
  const bsdtarPath = spawnSync('bash', ['-c', 'command -v bsdtar || true'], { encoding: 'utf8' }).stdout.trim();
  if (!/bsdtar/.test(tarVersion) && !bsdtarPath) {
    assert.fail('系统仅有 GNU tar,无法验证 tar 回退分支:请安装 libarchive-tools(bsdtar);CI 的 ubuntu job 已预装');
  }
  const w = workspace();
  fs.mkdirSync(path.join(w.dir, 'pkg'), { recursive: true });
  for (const [name, content] of Object.entries(FULL_PKG)) fs.writeFileSync(path.join(w.pkg, name), content);
  fs.writeFileSync(path.join(w.dir, 'outer.zip'), makeZip({ 'Clarora_0.1.0_x64.msix': makeZip(FULL_PKG) }));
  // 受限 PATH 隐藏 unzip,迫使脚本走 tar 回退分支
  const fakebin = path.join(w.dir, 'fakebin');
  fs.mkdirSync(fakebin);
  let linked = 0;
  for (const tool of ['bash', 'sh', 'grep', 'awk', 'find', 'tar', 'bsdtar', 'mktemp', 'rm', 'cp', 'basename', 'dirname', 'cat', 'mkdir', 'printf', 'uname']) {
    const resolved = spawnSync('bash', ['-c', `command -v ${tool} || true`], { encoding: 'utf8' }).stdout.trim();
    if (resolved && fs.existsSync(resolved)) { fs.symlinkSync(resolved, path.join(fakebin, tool)); linked += 1; }
  }
  assert.ok(linked >= 10, '受限 PATH 需要足够的基础工具');
  const restricted = spawnSync('bash', [SCRIPT, path.join(w.dir, 'outer.zip')], {
    encoding: 'utf8',
    env: { ...process.env, PATH: fakebin },
  });
  assert.equal(restricted.status, 0, restricted.stdout + restricted.stderr);
  assert.match(restricted.stdout, /pdfium\.dll/);
  // tar 分支同样拒绝缺件包
  fs.writeFileSync(path.join(w.dir, 'o2.zip'), makeZip({ 'm2.msix': makeZip({ 'Clarora.exe': 'exe', 'clarora_asr.dll': 'asr' }) }));
  const missing = spawnSync('bash', [SCRIPT, path.join(w.dir, 'o2.zip')], {
    encoding: 'utf8',
    env: { ...process.env, PATH: fakebin },
  });
  assert.equal(missing.status, 1, 'tar 分支应拒绝缺件包');
  fs.rmSync(w.dir, { recursive: true, force: true });
});

test('top-level zip contents are still verified directly', () => {
  const w = workspace();
  fs.writeFileSync(path.join(w.dir, 'flat.zip'), makeZip(FULL_PKG));
  assert.equal(runScript([path.join(w.dir, 'flat.zip')]).status, 0);
  fs.writeFileSync(path.join(w.dir, 'bad.zip'), makeZip({ 'Clarora.exe': 'exe' }));
  assert.equal(runScript([path.join(w.dir, 'bad.zip')]).status, 1);
  fs.rmSync(w.dir, { recursive: true, force: true });
});
