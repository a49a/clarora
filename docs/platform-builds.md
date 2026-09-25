# React Native 多平台工程

四个平台当前提供源码和构建入口；v0.1.0 起提供 macOS / Windows 安装包（见 GitHub Releases）。

| 平台 | 实现 | 当前范围 |
| --- | --- | --- |
| macOS | `clarora-app/macos/`，React Native macOS 0.76 + libmpv | 共用 `App.tsx` / `shared/`；端侧离线字幕（whisper.cpp / SenseVoice）、PDF 阅读（PDFKit）、目录与命令导入、音频合并、libmpv 视频与双字幕同显、桌面快捷键 |
| Windows | `clarora-app/windows/`，React Native Windows 0.76.17 | 共用 `App.tsx` / `shared/`；端侧离线字幕（clarora_asr）、PDF 阅读（pdfium）、Windows 原生文件、SQLite、音频、片段预加载、录音、键盘与视频适配 |
| Android | `clarora-app/android/` + `clarora-app/shared/` | 复用移动学习页面；系统文件选择、音频播放与变速、上滑切换；macOS 合并的音频经同步下发 |
| iOS | `clarora-app/ios/` + `clarora-app/shared/` | 复用移动学习页面；新增文件选择、剪贴板、播放/倍速/循环、片段双播放器预加载、跟读录音及休息音乐适配；需设备验证 |

> **Anki 卡组导入**为桌面端能力：macOS / Windows 均提供“Anki 卡组”入口，格式和合并规则见 [Anki 导入说明](anki-import.md)。移动端不做 Anki 导入——在桌面端导入的卡片经「同步与备份」下发到移动端。

## macOS

需要 macOS 11 或更新版本，以及 Node.js 24、Xcode、CocoaPods。视频播放使用 libmpv（`brew install mpv`），端侧字幕使用 whisper.cpp（`brew install whisper-cpp`），均从 `/opt/homebrew`（Homebrew 默认位置）查找；其他安装路径需要调整 `macos/Clarora.xcodeproj` 的 Header / Library Search Paths。

SenseVoice 端侧转写还需要 sherpa-onnx 动态库（无 Homebrew formula）。推荐用仓库脚本一键安装（GitHub 不可达时自动回退 api.github.com；自动识别 arm64 / x86_64）：

```sh
sh clarora-app/scripts/install-macos-asr-libs.sh
```

手动安装到同一前缀的等价步骤：

```sh
curl -fsSL -o /tmp/sherpa-libs.tar.bz2 \
  https://github.com/k2-fsa/sherpa-onnx/releases/download/v1.13.8/sherpa-onnx-v1.13.8-osx-arm64-shared-no-tts-lib.tar.bz2
tar -xjf /tmp/sherpa-libs.tar.bz2 -C /tmp
cp sherpa-onnx-v1.13.8-osx-arm64-shared-no-tts-lib/lib/*.dylib /opt/homebrew/lib/
mkdir -p /opt/homebrew/include/sherpa-onnx/c-api
curl -fsSL -o /opt/homebrew/include/sherpa-onnx/c-api/c-api.h \
  https://cdn.jsdelivr.net/gh/k2-fsa/sherpa-onnx@v1.13.8/sherpa-onnx/c-api/c-api.h
```

Intel Mac 使用 `osx-x86_64-shared-no-tts-lib.tar.bz2` 对应包。GitHub 不可达时，资产下载可改走 `api.github.com`（`Accept: application/octet-stream`）。

```sh
cd clarora-app
npm ci
npm start
# 另开终端，在 clarora-app 目录：
npm run macos
```

`npm run macos` 与 `npm run macos:release` 会在 Pods 缺失或与 `Podfile.lock` 不一致时自动执行 `pod install`；修改 Podfile 后执行 `npm run macos:pods`。仅修改 JS/TS 时保持 Metro 运行刷新即可，修改原生代码后需要重新执行 `npm run macos`。

使用范围与限制：

- 闪卡、音频学习、随便学学、口语跟读、统计、冥想、词汇关系图与自有存储备份均可用。
- 「AI 生成字幕」默认走端侧模型（设置 → 字幕转写引擎中下载）：whisper.cpp 适合英文，SenseVoice 支持中/英/日/韩/粤并自动检测语言，离线运行；英译中翻译仍使用「AI 服务」的聊天 API。也可切换为自定义转写 API。
- 目录批量导入、命令导入、音频合并（AVFoundation）、双字幕同显与视频学习（libmpv）为 macOS 专属能力。
- 命令导入需要 App Sandbox 保持关闭（`macos/Clarora-macOS/Clarora.entitlements` 中 `com.apple.security.app-sandbox` 为 `false`），此配置用于直接分发的构建。

## Windows

使用 Windows 10 2004（19041）或更新版本。开发需要 Node.js 24、Visual Studio 2022 的 UWP / C++ 开发工具、Windows SDK 10.0.22621、.NET 8 SDK。使用官方 RNW 0.76 的 UWP C# 模板和旧架构，避免同时升级现有 Mac、Android、iOS 桥接层。

在仓库根目录执行：

```powershell
npm run setup
# 另开终端启动 Metro：npm start
npm run windows:dev
# 编译 Release 并生成未签名应用包
npm run windows:build
```

端侧转写依赖在本机构建一次（产物不进仓库）：

```powershell
npm run setup
powershell -NoProfile -ExecutionPolicy Bypass -File clarora-app/scripts/build-windows-asr.ps1
```

脚本会用 vcpkg 安装 whisper.cpp、下载 sherpa-onnx 发行包，并把 `clarora_asr.dll` 与运行时 DLL 复制到 `clarora-app/windows/Clarora/`（GitHub Actions 的 Windows 构建已自动包含此步骤）。之后打开 `clarora-app/windows/Clarora.sln`，选择 Debug / x64 启动。`MainPage.xaml` 注册 `clarora`，直接加载与其他平台相同的 `index.js` 和 `App.tsx`。

Release 产物位于 `clarora-app/windows/AppPackages/`，包括 JavaScript，无需 Metro。GitHub Actions 可手动运行 **React Native Windows**，下载 `Clarora-Windows-x64-unsigned` artifact。流程构建未签名应用包，不自动发布。安装与分发前需要自己的签名证书（Publisher 与 manifest 一致）或 Microsoft Store 签名。

Windows 使用同一个 SQLite schema、复习算法和自有存储备份逻辑。原生 `winsqlite3` 负责存储，批量导入在同一个事务内执行，失败回滚。旧依赖包的 Windows 工程已禁用自动链接，文件与数据库由应用内的 C# 模块提供。

Windows 使用范围：

- 「AI 生成字幕」支持端侧模型（clarora_asr.dll：whisper.cpp + SenseVoice），离线运行；英译中翻译走「AI 服务」的聊天 API。DLL 由 `scripts/build-windows-asr.ps1` 构建（需要 vcpkg 与 Visual Studio 2022 的 C++ 工具；同一脚本会下载 PDF 渲染所需的 pdfium.dll），缺失时应用其余功能不受影响，端侧转写在设置中提示不可用。
- 「文档阅读」为内置 PDF 阅读器（pdfium 渲染）：目录侧栏、整页渲染、缩放与页码导航，PDF 保存在应用目录的 `pdf/` 文件夹。
- 随便学学、闪卡、音频管理与学习、口语跟读、词汇表、OCR 图片导入、统计、冥想和自有存储备份均复用共享页面。
- 桌面信息流左右拖动，←/→ 切条，空格显示答案；输入文本时不拦截快捷键。
- 普通音频与两路预加载播放器分开；支持倍速、循环、跟读录音和合并音频。
- 视频学习使用系统 MediaPlayer，支持播放、倍速、A/B 循环、主字幕轨和视频片段截取保存。Windows 截取输出音视频，不保留内嵌字幕轨；Mac 的双字幕同显仍为 Mac 专属能力。
- 目录导入复制所选目录的直接子文件到应用私有目录；不递归子目录。重启后不依赖原路径授权。
- Shell 命令导入仍仅在 macOS 显示；Windows 的文件上传和下载使用原生 HTTP，不调用 Mac 的 `curl`。
- 本地开发服务可能需要 UWP loopback exemption，可在 Visual Studio 调试时启用网络回环；AI 本机服务需填写该设备可访问的地址。


参考：[RNW 0.76 入门](https://microsoft.github.io/react-native-windows/v1/docs/0.76/getting-started)、[Windows 媒体编辑](https://learn.microsoft.com/en-us/windows/uwp/audio-video-camera/media-compositions-and-editing)。

## Android

需要 Android Studio、Android SDK（API 35 或以上）、JDK 17，以及已启动的模拟器或连接的真机。

```sh
cd clarora-app
npm ci
npm start
# 另开终端，在 clarora-app 目录：
npm run android
```

只生成 APK 时执行 `npm run android:apk`，产物位于 `android/app/build/outputs/apk/debug/app-debug.apk`。

使用范围与限制：

- 随便学学上滑下一条、下滑返回；展开长原文/答案后上下滑动用于阅读，收起后恢复切换。
- 使用系统文件选择器导入音频/字幕，支持音频播放与变速，字幕长按可复制。
- macOS 合并的音频经存储同步下发后可直接学习；目录批量导入、命令导入与原生字幕右键“问 AI”菜单不在 Android 端显示。

## iOS

需要 macOS、Xcode（含 iOS SDK / Simulator）、Node.js 和 CocoaPods。工程使用 iOS 15.1+，支持 iPhone / iPad；复用仓库现有 React Native 0.76 macOS fork 的 iOS 实现，保持旧架构，与当前原生模块匹配。

```sh
cd clarora-app
npm ci
LC_ALL=en_US.UTF-8 npm run ios:pods
npm start
# 另一个终端，在 clarora-app 目录：
npm run ios
```

也可打开 `clarora-app/ios/Clarora.xcworkspace`，选择 **Clarora-iOS** scheme 和模拟器运行。Xcode 工程已提交，无需执行生成器；`create-project.rb` 仅用于重建工程，执行会覆盖工程设置，之后需要重新 `pod install`。

安装好 Xcode 的 iOS 平台组件后，可以只编译、不启动模拟器：

```sh
cd clarora-app/ios
xcodebuild -workspace Clarora.xcworkspace -scheme Clarora-iOS \
  -configuration Debug -sdk iphonesimulator \
  -destination 'generic/platform=iOS Simulator' CODE_SIGNING_ALLOWED=NO build
```

真机运行前，在 Xcode Signing & Capabilities 中设置自己的 Team 和唯一 Bundle Identifier。Release 构建会自动打包 JavaScript。GitHub Release 提供的 `Clarora-ios-unsigned.ipa` 需要用户自行签名后安装；构建此包无需 Apple 开发者会员。未签名构建命令、免费个人真机开发步骤与可选正式分发见 [移动端发布](mobile-releases.md)。

使用与限制：

- 随便学学使用上下滑动，点卡片显示答案，支持收藏和下一条音频预加载。
- iOS 从「文件」导入文本词卡和 OCR 图片；音频学习库通过设置中的对象存储备份合并获得。目录批量导入、命令导入、Mac 视频播放器和桌面快捷键入口不在 iOS 开放。
- 录音会请求麦克风权限；拒绝后可在系统设置重新开启。电话等音频会话中断时暂停播放，回到页面后手动继续。
- 本版没有 iOS 相机拍摄、后台音频或锁屏控制适配。
- 对象存储使用 HTTPS Endpoint；AI 本机服务中的 `127.0.0.1` 是手机本身。客户端不连接 Clarora 服务器。
- 首次安装可导入文本词卡或同步已有资料，再验证播放、自动换曲、切片、收藏、录音和重启后的数据保留。

平台验收需分别执行共享代码检查、JavaScript 打包、原生构建和设备运行测试。Windows 原生构建需在 Windows 环境完成；iOS 构建需安装对应 Xcode 平台组件。共享代码测试不能替代原生及真机验收。

## 客户端分发

构建并实际验证安装包后，再添加对应下载渠道。Windows 使用已发布的安装包地址，iOS 官网渠道使用 TestFlight 或 App Store 链接，GitHub Release 可另提供明确标注、需要用户自行签名的 IPA；未签名 IPA 不代表渠道已开放。tag 触发的发布流水线（macOS dmg / Windows zip / Android APK / 未签名 iOS IPA / SHA256SUMS）与发版步骤见 [发布流程](releases.md)。
