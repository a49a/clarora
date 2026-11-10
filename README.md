# Clarora — 语言学习工具

- **闪卡与复习**：单词、音频片段、AI 问答卡；SM-2 计划复习；文本、目录和 Anki 导入（Anki 卡组支持 macOS / Windows 桌面端；移动端不做导入，桌面导入的卡片经「同步与备份」下发）。
- **音视频学习**：逐句字幕、倍速、循环、双语字幕合并、片段收藏、音频管理与原生播放。
- **随便学学**：混合推荐本地学习内容，支持收藏和音频片段预加载。
- **学习辅助**：统计、番茄钟、冥想、主题、词汇关系图。
- **自有存储备份**：客户端直连 S3 兼容存储或阿里云 OSS，保留版本，在其他设备选择备份合并。
- **自定义 AI API**：聊天、流式回答、重点标注、翻译、OCR、转写与跟读文本对齐评分。用户自行配置模型与密钥。
- **端侧字幕（macOS / Windows）**：whisper.cpp 与 SenseVoice 本地模型离线生成字幕，音频不上传；英文转中文再走聊天 API。模型在设置中一键下载。
- **文档阅读（macOS / Windows）**：内置 PDF 阅读器，目录侧栏、整页渲染与缩放，学习资料离线阅读。

## 下载安装

项目官网(功能介绍、使用指南与下载入口):<https://a49a.github.io/clarora-web/>

v0.1.0 起提供 macOS 与 Windows 安装包,从 [GitHub Releases](https://github.com/a49a/clarora/releases/latest) 下载:

- macOS:`Clarora-macos-arm64.dmg`(Apple Silicon,运行时依赖已内置,无需 Homebrew)
- Windows:`Clarora-windows-x64.zip`(解压后按内附说明侧载安装,需开启开发者模式)

安装包未做代码签名:macOS 首次打开如被 Gatekeeper 拦截,右键 App 选「打开」;Windows 如有 SmartScreen 提示,选择「仍要运行」。

移动端签名构建与分发配置见 [移动端发布](docs/mobile-releases.md)。Android APK 以 Release 实际资产为准；iOS Release 将提供 `Clarora-ios-unsigned.ipa`（以实际资产为准），需要用户自行签名后安装，不能下载后直接安装；无需付费账号即可构建该未签名包。

## 开发

需要 Node.js 24、对应平台开发工具。macOS 还需要 Xcode、CocoaPods、libmpv 和 whisper-cpp（`brew install mpv whisper-cpp`）；当前工程从 `/opt/homebrew` 查找这些库，其他安装路径需要调整 Xcode 的 Header / Library Search Paths。

```sh
npm run setup
npm start
# 另开终端，选择一个平台：
npm run macos
npm run android
npm run windows
npm run ios
```

桌面 GPUI 开发可直接在仓库根目录执行 `make gpui`，或运行 `sh clarora-gpui/scripts/dev.sh`；Windows 使用 `powershell -File clarora-gpui/scripts/dev.ps1`。脚本会在缺少 PDFium 时安装固定版本，macOS 会打包调试 `.app` 后启动，首次运行需要联网下载 PDFium。`npm run gpui` 保留为兼容别名。Windows/iOS 的构建步骤和平台限制见 [平台说明](docs/platform-builds.md)。`.github/workflows/` 中的 CI 在推送与 PR 时运行共享代码的类型检查和测试，并编译验证 Android / macOS / iOS；Windows 安装包随推送构建，也可手动触发。共享代码和自动化测试不等同于四个平台的真机验收。

`npm run macos` 会检查本机 Pods；首次构建或原生依赖锁文件不一致时自动执行 `pod install`，因此需要先安装 CocoaPods。修改 Podfile 后，可在 `clarora-app/` 中执行 `npm run macos:pods` 手动更新依赖。`Pods/` 是本地生成内容，不提交到仓库。

```sh
npm run typecheck
npm test
```

## 从源码打包桌面应用

以下步骤以当前 `master` 为准，产物版本取 `clarora-gpui/Cargo.toml` 的 `version`。先拉取最新代码并确认版本：

```sh
git clone https://github.com/a49a/clarora.git
cd clarora
git pull --ff-only
grep '^version' clarora-gpui/Cargo.toml
```

只想在本机运行、不生成安装包时，macOS 执行 `make gpui`，Windows 执行 `powershell -File clarora-gpui/scripts/dev.ps1`；`npm run gpui` 保留为兼容别名。真正的可分发包按下面平台分别构建。

| 目标 | 入口 | 产物 |
| --- | --- | --- |
| macOS 本机调试 | `make gpui` | `clarora-gpui/target/Clarora.app` |
| macOS 分发包 | `release-macos.py` + runtime 配置 | `artifacts/clarora/gpui/macos/release-*` 下的 `.app`、ZIP、DMG |
| Windows 分发包 | `prepare-windows-runtime.ps1` + `build-windows.ps1 -Distribution` | `artifacts/clarora/gpui/windows/` 下的 ZIP、安装器、校验和 |

### macOS

要求 macOS 14+ Apple Silicon、Xcode Command Line Tools、Rust 1.93.1、Python 3、Homebrew 和 CMake：

```sh
brew install mpv ffmpeg yt-dlp cmake
```

**本机调试应用**（不用于分发）：

```sh
make gpui
# 产物：clarora-gpui/target/Clarora.app
```

**可分发的 release 包：**

1. 编译 ASR/TTS 辅助程序。下面的 `/path/to/...` 换成自己的 whisper.cpp / sherpa-onnx 安装位置：

```sh
cmake -S clarora-gpui/native -B clarora-gpui/target/native \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_PREFIX_PATH=/path/to/whisper-and-sherpa
cmake --build clarora-gpui/target/native --config Release

cmake -S clarora-gpui/native/tts -B clarora-gpui/target/tts \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_PREFIX_PATH=/path/to/sherpa-onnx
cmake --build clarora-gpui/target/tts --config Release
```

2. 准备 PDFium：

```sh
mkdir -p /absolute/path/clarora-runtime
python3 clarora-gpui/scripts/install-pdfium.py /absolute/path/clarora-runtime
```

3. 复制运行库配置并填写绝对路径：

```sh
cp clarora-gpui/scripts/macos-runtime.example.json /absolute/path/macos-runtime.json
```

至少填写 `libmpv`、`ffmpeg`、`ffprobe`、`yt_dlp`、`pdfium`、`asr`、`tts`、`ggml_cpu`，以及 ASR/TTS/PDFium 的 `providers` 来源和许可证路径、`python` 段。示例中的占位符不能直接用于打包。

4. 缓存 Rust 依赖、打包并验证：

```sh
cargo fetch --manifest-path clarora-gpui/Cargo.toml --locked

cd clarora-gpui
python3 scripts/release-macos.py \
  --runtime-config /absolute/path/macos-runtime.json \
  --output ../../artifacts/clarora/gpui/macos/release-001 \
  --dmg
python3 scripts/verify-macos-package.py \
  ../../artifacts/clarora/gpui/macos/release-001/Clarora.app \
  --evidence ../../artifacts/clarora/gpui/macos/verify-001 \
  --launch-seconds 3
```

输出目录必须是工作区 `artifacts/clarora/` 下尚不存在的新目录；重复打包请换新目录。默认使用本机 ad-hoc 签名，不做公证；正式分发需要 `--sign-identity` 和 `--notarize-profile`。产物包含 `.app`、ZIP、可选 DMG，以及 `release.json`、`release-files.json`。

### Windows

要求 Windows 10/11 x64、Rust 1.93.1 MSVC、Visual Studio 2022 的 C++ 桌面开发工作负载、Windows SDK、CMake、Python 3、7-Zip 和 Inno Setup 6。使用 x64 开发者 PowerShell 或 Native Tools 命令提示符：

```powershell
git clone https://github.com/a49a/clarora.git
cd clarora\clarora-gpui
./scripts/prepare-windows-runtime.ps1
./scripts/build-windows.ps1 -Distribution -RuntimeDir ./target/windows-runtime -Installer
```

`prepare-windows-runtime.ps1` 会按 `windows-runtime-lock.json` 下载固定依赖并编译 ASR/TTS 运行库，第一次执行可能需要较长时间。`-Installer` 生成按用户安装的安装器；不加时只生成便携 ZIP。

默认输出到工作区 `artifacts/clarora/gpui/windows/`，包含：

- `Clarora-<version>-windows-x64-unsigned.zip`
- `Clarora-<version>-windows-x64-setup-unsigned.exe`（带 `-Installer` 时）
- `release-manifest.json`、`SHA256SUMS.txt` 和验证报告

验证安装、覆盖安装、运行检查和保留资料卸载：

```powershell
./scripts/verify-windows-installer.ps1 `
  -Installer /absolute/path/Clarora-<version>-windows-x64-setup-unsigned.exe `
  -Evidence /absolute/path/installer-evidence
```

只运行 `./scripts/build-windows.ps1` 得到的是不完整开发版 EXE，不能分发；发布必须带 `-Distribution -RuntimeDir`。

### 发布前检查

- 两个平台的产物默认未签名：macOS 为 ad-hoc，Windows 为 unsigned；正式分发需自行签名／公证。
- 语音模型不随包提供，由用户在应用内下载；ASR/TTS 运行库、FFmpeg、PDFium 和 libmpv 必须随包收集。
- 安装包版本来自 `clarora-gpui/Cargo.toml`；发布新版本时同时更新 `Cargo.lock` 中 `clarora-gpui` 的版本并提交。
- 运行库字段、许可证和验证边界详见 `clarora-gpui/scripts/PACKAGING.md`。

## 同步与备份

在「设置 → 同步与备份」填写存储类型、Endpoint、Region、Bucket、Access Key 和资料库前缀。各设备使用相同的存储位置；一台设备点“备份本机资料”，另一台读取版本并“合并到本机”。不需要部署数据库、API 或任务队列。

当前采用手动版本备份与增量合并：不自动传播删除，本机已有内容优先，复习进度采用较晚的评分，同日统计取较大值。每次上传是完整备份，不是后台实时双向同步。

备份包括词卡、音视频学习附件、字幕、收藏、复习计划、统计、OCR 与跟读结果。凭证、设备设置、未收藏的最近视频、聊天会话草稿、原始 OCR 图片和跟读录音不纳入备份。凭证在 macOS / Windows 保存在系统凭证保险库（钥匙串 / 凭据库），移动端暂存本机应用数据库；云端备份通过 HTTPS 传输，尚无端到端加密。

详细配置、权限、数据格式和恢复策略见 [存储同步说明](docs/storage-sync.md)。

## AI

在「设置 → AI 服务」配置兼容 API 的 Base URL、API Key 和模型名称。聊天/翻译使用 `/chat/completions`；OCR 需要视觉模型；转写使用 `/audio/transcriptions`，模型必须支持 `verbose_json` 与 `segments` 时间轴。可以分别配置聊天和转写服务，也可使用用户自行运行的兼容本地服务。

OCR 与跟读结果保存在本机。跟读评分比较识别文本与参考句，并非声学发音评测。没有 AI 配置时，本地导入、播放和复习仍可使用。

## 仓库结构

| 路径 | 内容 |
| --- | --- |
| `clarora-app/` | 四平台 React Native 客户端、共享数据层、对象存储与 AI 适配 |
| `tool/` | 客户端图标生成工具 |
| `docs/` | 客户端构建与存储同步说明 |

根 LICENSE 保留现有 BSD-3-Clause 声明。内置图谱使用本项目原创的小型示例数据；第三方词书和个人学习素材不随源码发布。第三方依赖、模型服务及 libmpv 等的许可需按实际分发内容另行核对。


桌面 Anki 导入：macOS / Windows 在闪卡页面选择“Anki 导入”，支持卡包和 Anki `.txt` / `.tsv` 文本；格式、合并规则与限制见 [导入说明](docs/anki-import.md)。
