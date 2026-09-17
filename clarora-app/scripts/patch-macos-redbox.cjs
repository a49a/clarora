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

// react-native-macos 的 fork-only 版本(如 0.81.9)在 Maven Central 没有
// 对应的 react-android/hermes-android 预编译产物,Android 构建会 404。
// RN gradle 插件从 ReactAndroid/gradle.properties 读 VERSION_NAME 来锁定
// 这两个产物;把该值钉到同发布线的最新上游版本即可全局对齐。
const ANDROID_ARTIFACT_VERSION = '0.81.6';
const androidArtifactVersions = ['0.81'];

function pinAndroidArtifactVersion(packageDir) {
  const pkgVersion = JSON.parse(fs.readFileSync(path.join(packageDir, 'package.json'), 'utf8')).version;
  if (!androidArtifactVersions.some(prefix => pkgVersion.startsWith(prefix))) return false;
  const file = path.join(packageDir, 'ReactAndroid', 'gradle.properties');
  let source = fs.readFileSync(file, 'utf8');
  const pinned = source.replace(/^VERSION_NAME=.*$/m, `VERSION_NAME=${ANDROID_ARTIFACT_VERSION}`);
  if (pinned === source) return false;
  fs.writeFileSync(file, pinned);
  return true;
}

// react-native-macos 0.81:React-jsinspector podspec 只平铺匹配
// jsinspector-modern/*.{cpp,h}，但 tarball 把 CdpJson.* 留在了 cdp/ 子目录，
// pod install 生成的 Pods 工程因此引用不存在的文件。拷贝平铺即可；
// tarball 将来若自带平铺文件则本补丁自动跳过。
const jsinspectorCdpFiles = ['CdpJson.cpp', 'CdpJson.h'];

function flattenJsinspectorCdp(packageDir) {
  const dir = path.join(packageDir, 'ReactCommon', 'jsinspector-modern');
  const cdpDir = path.join(dir, 'cdp');
  if (jsinspectorCdpFiles.every(name => fs.existsSync(path.join(dir, name)))) return false;
  if (!jsinspectorCdpFiles.every(name => fs.existsSync(path.join(cdpDir, name)))) {
    throw new Error('jsinspector cdp sources missing; review the flatten patch before upgrading.');
  }
  for (const name of jsinspectorCdpFiles) fs.copyFileSync(path.join(cdpDir, name), path.join(dir, name));
  return true;
}

// react-native-windows 0.81:UWP 源码构建已不被上游维护，留了几处坏死：
// ① HermesSamplingProfiler.cpp 使用 std::coroutine_handle 但全链路无人
//    包含 <coroutine>；② Microsoft.ReactNative.Managed.csproj 强制
//    VisualStudioVersion=18.0 并导入 v18 的 XAML targets，VS2022 没有；
// ③ React.Cpp.props 在 v143 工具集下追加旧版协程开关 /await，与 C++20
//    协程互斥（WinRTWebSocketResource.h 又按 _MSC_VER 分支，两者矛盾）。
// 统一改走 /await:strict 的 std 命名空间，并补齐头文件与分支。
const windowsPatches = [
  {
    file: 'Shared/Hermes/HermesSamplingProfiler.cpp',
    original: '#include <future>',
    replacement: '#include <coroutine>\n#include <future>',
  },
  {
    file: 'Microsoft.ReactNative.Managed/Microsoft.ReactNative.Managed.csproj',
    original: "'$(VisualStudioVersion)' == '' or '$(VisualStudioVersion)' &lt; '18.0' ",
    replacement: "'$(VisualStudioVersion)' == '' or '$(VisualStudioVersion)' &lt; '17.0' ",
  },
  {
    file: 'Microsoft.ReactNative.Managed/Microsoft.ReactNative.Managed.csproj',
    original: '<VisualStudioVersion>18.0</VisualStudioVersion>',
    replacement: '<VisualStudioVersion>17.0</VisualStudioVersion>',
  },
  {
    // C# 应用经 ProjectReference 触发 Managed 工程时全局 Platform 不会传到,
    // 它回退默认 x86,与 x64 编译出的 Microsoft.ReactNative.winmd 冲突
    // (MSB3271)。默认值改为 x64,与发布脚本的 x64-only 一致。
    file: 'Microsoft.ReactNative.Managed/Microsoft.ReactNative.Managed.csproj',
    original: "<Platform Condition=\" '$(Platform)' == '' \">x86</Platform>",
    replacement: "<Platform Condition=\" '$(Platform)' == '' \">x64</Platform>",
  },
  {
    // UWP 打包链路会对引用工程调用 Pack target(新 SDK 工程才有),旧式
    // Managed.csproj 没有,补一个空目标让打包继续;产出的 DLL 已通过
    // packaging outputs 收集。
    file: 'Microsoft.ReactNative.Managed/Microsoft.ReactNative.Managed.csproj',
    original: '<Target Name="Deploy" />',
    replacement: '<Target Name="Deploy" />\n  <Target Name="Pack" />',
  },
  {
    // Managed 会被若干路径触发(应用引用、CodeGen 的裸 MSBuild 调用),
    // 后者拿不到全局 Platform/Configuration,OutDir 里平台段为空,生成的
    // PRI 与应用打包期期望的 target\\x64\\Release 路径对不上(PRI252)。
    // 固定到发布脚本唯一的 x64/Release 组合。
    file: 'Microsoft.ReactNative.Managed/Microsoft.ReactNative.Managed.csproj',
    original: '<OutputType>Library</OutputType>',
    replacement: '<OutputType>Library</OutputType>\n    <OutDir>$(ReactNativeWindowsDir)target\\x64\\Release\\$(MSBuildProjectName)\\</OutDir>',
  },
  {
    file: 'PropertySheets/React.Cpp.props',
    original: '%(AdditionalOptions) /await</AdditionalOptions>',
    replacement: '%(AdditionalOptions) /await:strict</AdditionalOptions>',
  },
  {
    file: 'Microsoft.ReactNative/Microsoft.ReactNative.vcxproj',
    original: '%(AdditionalOptions) /await</AdditionalOptions>',
    replacement: '%(AdditionalOptions) /await:strict</AdditionalOptions>',
  },
  {
    file: 'Shared/Networking/WinRTWebSocketResource.h',
    original: '#include <queue>',
    replacement: '#include <coroutine>\n#include <queue>',
  },
  {
    file: 'Shared/Networking/WinRTWebSocketResource.h',
    original: '#if _MSC_VER >= 1951',
    replacement: '#if 1 // Clarora: strict C++20 coroutines',
  },
  {
    file: 'Shared/Networking/WinRTWebSocketResource.h',
    original: 'using CoroHandle = std::experimental::coroutine_handle<>;',
    replacement: 'using CoroHandle = std::coroutine_handle<>; // Clarora strict',
  },
];

function applyWindowsPatches(packageDir) {
  let applied = 0;
  for (const patch of windowsPatches) {
    const file = path.join(packageDir, patch.file);
    let source = fs.readFileSync(file, 'utf8');
    if (source.includes(patch.replacement)) continue;
    if (!source.includes(patch.original)) {
      throw new Error(`${patch.file} changed; review the windows patch before upgrading.`);
    }
    fs.writeFileSync(file, source.replace(patch.original, patch.replacement));
    applied++;
  }
  return applied > 0;
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

    try {
      if (flattenJsinspectorCdp(packageDir)) {
        console.log('clarora patch: flattened', `ReactCommon/jsinspector-modern/{${jsinspectorCdpFiles.join(',')}}`);
      }
    } catch (error) {
      console.error(`clarora jsinspector flatten (${name}):`, error.message);
      process.exitCode = 1;
    }

    try {
      if (pinAndroidArtifactVersion(packageDir)) {
        console.log('clarora patch: pinned Android react-android artifacts to', ANDROID_ARTIFACT_VERSION);
      }
    } catch (error) {
      console.error(`clarora android version pin (${name}):`, error.message);
      process.exitCode = 1;
    }
  }

  try {
    if (applyWindowsPatches(path.join(__dirname, '..', 'node_modules', 'react-native-windows'))) {
      console.log('clarora patch: applied react-native-windows UWP fixes');
    }
  } catch (error) {
    console.error('clarora windows patch:', error.message);
    process.exitCode = 1;
  }
}

module.exports = { patchRedBox };
