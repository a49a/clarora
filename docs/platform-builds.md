# React Native 移动端工程

> **弃用说明（2026-11-13）：** React Native 的 **macOS / Windows 桌面构建已弃用**，不再作为桌面主版本推荐或发布目标。桌面请使用 [`clarora-gpui/`](../clarora-gpui/)；Android / iOS 的 RN 构建继续维护。历史桌面工程源码保留在仓库中，但不再提供构建或使用说明。

| 平台 | 实现 | 当前范围 |
| --- | --- | --- |
| Android | `clarora-app/android/` + `clarora-app/shared/` | 继续维护；复用移动学习页面、系统文件选择、音频播放与变速、上滑切换；桌面端合并的音频经同步下发 |
| iOS | `clarora-app/ios/` + `clarora-app/shared/` | 继续维护；复用移动学习页面、文件选择、剪贴板、播放/倍速/循环、片段双播放器预加载、跟读录音及休息音乐适配；需设备验证 |

> **Anki 卡组导入**为桌面端能力：GPUI 桌面主版本（macOS / Windows）提供“Anki 卡组”入口，格式和合并规则见 [Anki 导入说明](anki-import.md)。移动端不做 Anki 导入——在桌面端导入的卡片经「同步与备份」下发到移动端。

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
- 桌面端合并的音频经存储同步下发后可直接学习；目录批量导入、命令导入与原生字幕右键“问 AI”菜单不在 Android 端显示。

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
- iOS 从「文件」导入文本词卡和 OCR 图片；音频学习库通过设置中的对象存储备份合并获得。目录批量导入、命令导入、桌面视频播放器和桌面快捷键入口不在 iOS 开放。
- 录音会请求麦克风权限；拒绝后可在系统设置重新开启。电话等音频会话中断时暂停播放，回到页面后手动继续。
- 本版没有 iOS 相机拍摄、后台音频或锁屏控制适配。
- 对象存储使用 HTTPS Endpoint；AI 本机服务中的 `127.0.0.1` 是手机本身。客户端不连接 Clarora 服务器。
- 首次安装可导入文本词卡或同步已有资料，再验证播放、自动换曲、切片、收藏、录音和重启后的数据保留。

平台验收需分别执行共享代码检查、JavaScript 打包、Android / iOS 原生构建和设备运行测试。iOS 构建需安装对应 Xcode 平台组件。共享代码测试不能替代原生及真机验收。

## 客户端分发

构建并实际验证安装包后，再添加对应下载渠道。Android 使用 APK 地址，iOS 官网渠道使用 TestFlight 或 App Store 链接，GitHub Release 可另提供明确标注、需要用户自行签名的 IPA；未签名 IPA 不代表渠道已开放。移动端发布与发版步骤见 [移动端发布](mobile-releases.md)。桌面 GPUI 发行使用独立流程。

## bilibili / YouTube 视频下载导入

RN 移动端不提供桌面视频下载入口。桌面 GPUI 的 bilibili 与 YouTube 导入及验证见 [桌面说明](../clarora-gpui/README.md) 和 [bilibili 导入说明](bilibili-import.md)。
