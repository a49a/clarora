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

// react-native-macos 0.77:iOS 侧 RCTInstance.mm 把 NSString 直接传给 BOOL
// 参数(fork 不维护 iOS 编译)。仅在 0.77 生效,升级到更高版本时跳过并复核。
const rctInstancePatch = {
  file: 'ReactCommon/react/runtime/platform/ios/ReactCommon/RCTInstance.mm',
  original: 'isFatal:errorData[@"isFatal"]',
  replacement: 'isFatal:[errorData[@"isFatal"] boolValue]',
  versions: ['0.77', '0.81'],
};

function applyRctInstancePatch(packageDir) {
  const pkgVersion = JSON.parse(fs.readFileSync(path.join(packageDir, 'package.json'), 'utf8')).version;
  if (!rctInstancePatch.versions.some(prefix => pkgVersion.startsWith(prefix))) return false;
  const file = path.join(packageDir, rctInstancePatch.file);
  let source = fs.readFileSync(file, 'utf8');
  if (source.includes(rctInstancePatch.replacement)) return false;
  if (!source.includes(rctInstancePatch.original)) {
    throw new Error('RCTInstance isFatal call changed; review the iOS patch before upgrading.');
  }
  fs.writeFileSync(file, source.replace(rctInstancePatch.original, rctInstancePatch.replacement));
  return true;
}

// react-native-macos 0.81:React-perflogger podspec 的 glob 引用了
// ReactPerfLogger.cpp,但 npm 包未携带该文件(0.81.9 打包疏漏)。
// 创建空编译单元满足构建输入;fork 代码实际使用 ReactPerfetto* 系列。
const reactPerfLoggerStubRel = 'ReactCommon/reactperflogger/reactperflogger/ReactPerfLogger.cpp';

function ensureReactPerfLoggerStub(packageDir) {
  const file = path.join(packageDir, reactPerfLoggerStubRel);
  if (fs.existsSync(file)) return false;
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, '// Intentionally empty: satisfies the React-perflogger podspec source glob.\n');
  return true;
}

// react-native-macos 0.81:RCTView setShadowColor 没有 retain 传入的
// CGColor(属性管线传入的是临时对象),autorelease 后 ivar 悬空,下一次
// 阴影更新即崩溃(启动白屏/闪退)。补上 retain/release 配对。
const shadowColorPatch = {
  file: 'React/Views/RCTView.m',
  original: `- (void)setShadowColor:(CGColorRef)shadowColor
{
    if (_shadowColor != shadowColor)
    {
        _shadowColor = shadowColor;
        [self didUpdateShadow];
    }
}`,
  replacement: `- (void)setShadowColor:(CGColorRef)shadowColor
{
    if (_shadowColor != shadowColor)
    {
        // Clarora: retain the color — the props pipeline hands us a temporary
        // CGColor that is freed before the next shadow update.
        CGColorRetain(shadowColor);
        CGColorRelease(_shadowColor);
        _shadowColor = shadowColor;
        [self didUpdateShadow];
    }
}`,
};

function applyShadowColorPatch(packageDir) {
  const file = path.join(packageDir, shadowColorPatch.file);
  let source = fs.readFileSync(file, 'utf8');
  if (source.includes('CGColorRetain(shadowColor)')) return false;
  if (!source.includes(shadowColorPatch.original)) {
    throw new Error('RCTView setShadowColor changed; review the shadow retain patch before upgrading.');
  }
  fs.writeFileSync(file, source.replace(shadowColorPatch.original, shadowColorPatch.replacement));
  return true;
}

if (require.main === module) {
  // The workspace installs macOS under its own name and the react-native alias.
  for (const name of ['react-native-macos', 'react-native']) {
    const packageDir = path.join(__dirname, '..', 'node_modules', name);

    const redBoxFile = path.join(packageDir, 'React', 'CoreModules', 'RCTRedBox.mm');
    const source = fs.readFileSync(redBoxFile, 'utf8');
    const patched = patchRedBox(source);
    if (patched !== source) fs.writeFileSync(redBoxFile, patched);

    try {
      applyShadowColorPatch(packageDir);
    } catch (error) {
      console.error(`clarora shadow patch (${name}):`, error.message);
      process.exitCode = 1;
    }

    try {
      applyRctInstancePatch(packageDir);
    } catch (error) {
      console.error(`clarora patch (${name}):`, error.message);
      process.exitCode = 1;
    }

    if (ensureReactPerfLoggerStub(packageDir)) {
      console.log('clarora patch: created stub', reactPerfLoggerStubRel);
    }
  }
}

module.exports = { patchRedBox };
