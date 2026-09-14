# React Native 多平台工程

这两个平台当前提供源码和构建入口，还没有已发布的安装包。

| 平台 | 实现 | 当前范围 |
| --- | --- | --- |
| Windows | `clarora-app/windows/`，React Native Windows 0.76.17 | 共用 `App.tsx` / `shared/`；Windows 原生文件、SQLite、音频、片段预加载、录音、键盘与视频适配 |
| iOS | `clarora-app/ios/` + `clarora-app/shared/` | 复用移动学习页面；新增文件选择、剪贴板、播放/倍速/循环、片段双播放器预加载、跟读录音及休息音乐适配；需设备验证 |

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

也可打开 `clarora-app/windows/Clarora.sln`，选择 Debug / x64 启动。`MainPage.xaml` 注册 `clarora`，直接加载与其他平台相同的 `index.js` 和 `App.tsx`。

Release 产物位于 `clarora-app/windows/AppPackages/`，包括 JavaScript，无需 Metro。GitHub Actions 可手动运行 **React Native Windows**，下载 `Clarora-Windows-x64-unsigned` artifact。流程构建未签名应用包，不自动发布。安装与分发前需要自己的签名证书（Publisher 与 manifest 一致）或 Microsoft Store 签名。

Windows 使用同一个 SQLite schema、复习算法和自有存储备份逻辑。原生 `winsqlite3` 负责存储，批量导入在同一个事务内执行，失败回滚。旧依赖包的 Windows 工程已禁用自动链接，文件与数据库由应用内的 C# 模块提供。

Windows 使用范围：

- 随便学学、闪卡、音频管理与学习、口语跟读、词汇表、OCR 图片导入、统计、冥想和自有存储备份均复用共享页面。
- 桌面信息流左右拖动，←/→ 切条，空格显示答案；输入文本时不拦截快捷键。
- 普通音频与两路预加载播放器分开；支持倍速、循环、跟读录音和合并音频。
- 视频学习使用系统 MediaPlayer，支持播放、倍速、A/B 循环、主字幕轨和视频片段截取保存。Windows 截取输出音视频，不保留内嵌字幕轨；Mac 的双字幕同显仍为 Mac 专属能力。
- 目录导入复制所选目录的直接子文件到应用私有目录；不递归子目录。重启后不依赖原路径授权。
- Shell 命令导入仍仅在 macOS 显示；Windows 的文件上传和下载使用原生 HTTP，不调用 Mac 的 `curl`。
- 本地开发服务可能需要 UWP loopback exemption，可在 Visual Studio 调试时启用网络回环；AI 本机服务需填写该设备可访问的地址。


参考：[RNW 0.76 入门](https://microsoft.github.io/react-native-windows/v1/docs/0.76/getting-started)、[Windows 媒体编辑](https://learn.microsoft.com/en-us/windows/uwp/audio-video-camera/media-compositions-and-editing)。

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

真机运行前，在 Xcode Signing & Capabilities 中设置自己的 Team 和唯一 Bundle Identifier。Release 构建会自动打包 JavaScript；分发需使用自己的签名、描述文件和 App Store/TestFlight 配置。

使用与限制：

- 随便学学使用上下滑动，点卡片显示答案，支持收藏和下一条音频预加载。
- iOS 从「文件」导入文本词卡和 OCR 图片；音频学习库通过设置中的对象存储备份合并获得。目录批量导入、命令导入、Mac 视频播放器和桌面快捷键入口不在 iOS 开放。
- 录音会请求麦克风权限；拒绝后可在系统设置重新开启。电话等音频会话中断时暂停播放，回到页面后手动继续。
- 本版没有 iOS 相机拍摄、后台音频或锁屏控制适配。
- 对象存储使用 HTTPS Endpoint；AI 本机服务中的 `127.0.0.1` 是手机本身。客户端不连接 Clarora 服务器。
- 首次安装可导入文本词卡或同步已有资料，再验证播放、自动换曲、切片、收藏、录音和重启后的数据保留。

平台验收需分别执行共享代码检查、JavaScript 打包、原生构建和设备运行测试。Windows 原生构建需在 Windows 环境完成；iOS 构建需安装对应 Xcode 平台组件。共享代码测试不能替代原生及真机验收。

## 客户端分发

构建并实际验证安装包后，再添加对应下载渠道。Windows 使用已发布的安装包地址，iOS 使用 TestFlight 或 App Store 链接；生成源码不代表已经可供用户下载。
