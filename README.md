# Clarora — 语言学习工具

- **闪卡与复习**：单词、音频片段、AI 问答卡；SM-2 计划复习；文本、目录和 Anki 导入（Anki 卡组支持 macOS / Windows 桌面端；移动端不做导入，桌面导入的卡片经「同步与备份」下发）。
- **音视频学习**：逐句字幕、倍速、循环、双语字幕合并、片段收藏、音频管理与原生播放。
- **随便学学**：混合推荐本地学习内容，支持收藏和音频片段预加载。
- **学习辅助**：统计、专注模式、冥想、主题、闪卡列表。
- **自有存储备份**：客户端直连 S3 兼容存储或阿里云 OSS，保留版本，在其他设备选择备份合并。
- **自定义 AI API**：聊天、流式回答、重点标注、翻译、OCR、转写与跟读文本对齐评分。用户自行配置模型与密钥。
- **端侧字幕（macOS / Windows）**：whisper.cpp 与 SenseVoice 本地模型离线生成字幕，音频不上传；英文转中文再走聊天 API。模型在设置中一键下载。
- **文档阅读（macOS / Windows）**：内置 PDF 阅读器，目录侧栏、整页渲染与缩放，学习资料离线阅读。

## 学习模块

| 模块 | 功能概览 |
| --- | --- |
| 音频学习 | 逐句字幕、翻译与真实波形；A/B 选取重点并保存片段闪卡；AI 标重点在原文用下划线标记；收藏片段用绿色下划线标记并可连续复习；盲听、单句循环、每句三/五遍、拖选复制与问 AI、跨音视频句型搜索。 |
| 视频学习 | 本地视频、bilibili 分 P 与 YouTube 单视频；主副字幕、翻译、文稿与 SRT；A/B 精听与片段闪卡；字幕行一键收藏本句；每句三/四/五/六遍，四/六遍前半隐藏字幕；全屏播放与逐句定位。 |
| 文档阅读 | PDF/EPUB/TXT，书库与目录侧栏，缩放、重排、独立阅读进度，当前片段问 AI。 |
| 口语与写作 | 口语分组、音频题目、草稿与目标词数、AI 批改、按命令导入/删除题目；写作多篇草稿、AI 批改、快照与历史。 |
| 跟读与图转文字 | 录音/选录音、逐词对齐、AI 点评、图片转文字。 |
| 闪卡与助手 | 五类卡片、SM-2、每日额度、文本/目录/Anki 导入；全局学习助手支持选区引用、会话历史和问答卡。 |
| 专注与资料 | 专注目标、分心/疲劳、随时记录的想法、训练建议；统计页分开展示专注时长与媒体播放时长；本机 ZIP、命名云存储配置、S3/R2 同步与 OSS 兼容备份。 |

更完整的操作说明见[桌面客户端说明](clarora-gpui/README.md#学习模块功能)。

## 下载安装

项目官网（功能介绍、使用指南与下载入口）：<https://a49a.github.io/clarora-web/>

当前桌面主版本从 [GitHub Releases](https://github.com/a49a/clarora/releases/latest) 下载：

- macOS：`Clarora-macos-arm64.dmg`，要求 macOS 14+、Apple Silicon；运行库已内置，无需 Homebrew。
- Windows：优先使用 `Clarora-windows-x64-setup-unsigned.exe` 按用户安装，或解压 `Clarora-windows-x64.zip` 运行便携版；无需开发者模式或侧载脚本。

安装包未做代码签名：macOS 首次打开如被 Gatekeeper 拦截，右键 App 选「打开」，或执行 `xattr -dr com.apple.quarantine /Applications/Clarora.app`；Windows 安装器未签名，SmartScreen 提示属正常现象。应用内下载语音模型，安装包不包含模型或个人学习资料。

RN 的 macOS / Windows 桌面构建已弃用，请使用上面的 GPUI 桌面主版本；Android / iOS RN 继续维护。

移动端继续使用独立标注的 RN 版本；Android APK 与 iOS 渠道以下载页和对应 Release 资产为准。iOS 未签名包需要自行签名后安装，不能下载后直接安装。构建与分发配置见[移动端发布](docs/mobile-releases.md)。

## 开发

桌面 GPUI 原生客户端位于 `clarora-gpui/`，使用 Rust；React Native 客户端位于 `clarora-app/`，两者启动方式独立。GPUI 日常启动用 `make gpui`（Windows 用 `powershell -File clarora-gpui/scripts/dev.ps1`），完整打包步骤见下方[从源码打包桌面应用](#从源码打包桌面应用)。

RN 的 macOS / Windows 桌面构建已弃用，不再提供使用或构建入口；桌面请使用 GPUI。Android / iOS 仍维护。

RN 移动端开发需要 Node.js 24、Android Studio 或 Xcode：

```sh
npm run setup
npm start
# 另开终端，选择移动平台：
npm run android
npm run ios
```

GPUI 的 macOS 开发环境还需要 FFmpeg、yt-dlp 和 CMake：

```sh
brew install mpv ffmpeg yt-dlp cmake whisper.cpp
```

直接 `cargo run` 打开 PDF 时如果提示缺少 `libpdfium.dylib`，执行 `python3 clarora-gpui/scripts/install-pdfium.py clarora-gpui/target/debug`，或改用会自动准备 PDFium 的 `make gpui`。

Android / iOS 的 RN 构建步骤和平台限制见[平台说明](docs/platform-builds.md)。桌面 GPUI 需要 Rust 1.93.1；Windows 还需要 Visual Studio 2022 的 C++ 桌面工作负载与 Windows SDK。`.github/workflows/` 中的 CI 在推送与 PR 时运行共享代码的类型检查和测试，并编译验证 Android / iOS。共享代码和自动化测试不等同于移动端真机验收。

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
brew install mpv ffmpeg yt-dlp cmake whisper.cpp
```

**本机调试应用**（不用于分发）：

```sh
make gpui
# 产物：clarora-gpui/target/Clarora.app
```

**可分发的 release 包：**

1. 准备 sherpa-onnx。Homebrew 没有现成 formula，需要从 [sherpa-onnx Releases](https://github.com/k2-fsa/sherpa-onnx/releases) 下载匹配架构的 shared 包并解压。下面以当前验证的 1.13.8 为例，换版本时同步修改路径和 `runtime.json` 中的版本：

```sh
tar -xjf sherpa-onnx-v1.13.8-osx-arm64-shared.tar.bz2
export SHERPA_PREFIX="$PWD/sherpa-onnx-v1.13.8-osx-arm64-shared"
```

2. 编译 ASR/TTS 辅助程序。whisper.cpp 由 Homebrew 提供；sherpa-onnx 使用上一步的目录：

```sh
cmake -S clarora-gpui/native -B clarora-gpui/target/native \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_PREFIX_PATH="/opt/homebrew/opt/whisper.cpp;/opt/homebrew/opt/ggml;$SHERPA_PREFIX"
cmake --build clarora-gpui/target/native --config Release

cmake -S clarora-gpui/native/tts -B clarora-gpui/target/tts \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_PREFIX_PATH="$SHERPA_PREFIX"
cmake --build clarora-gpui/target/tts --config Release
```

如果下载的 sherpa-onnx 包不是启用 TTS 的版本，`clarora-tts` 会链接失败或无法生成语音；请换用 shared 包中带 TTS 的运行库。

3. 准备 PDFium：

```sh
mkdir -p /absolute/path/clarora-runtime
python3 clarora-gpui/scripts/install-pdfium.py /absolute/path/clarora-runtime
```

4. 复制运行库配置并填写绝对路径：

```sh
cp clarora-gpui/scripts/macos-runtime.example.json /absolute/path/macos-runtime.json
```

至少填写 `libmpv`、`ffmpeg`、`ffprobe`、`yt_dlp`、`pdfium`、`asr`、`tts`、`ggml_cpu`，以及 ASR/TTS/PDFium 的 `providers` 来源和许可证路径、`python` 段。`asr` 指向 `clarora-gpui/target/native/clarora-asr`，`tts` 指向 `clarora-gpui/target/tts/clarora-tts`；示例中的占位符不能直接用于打包。

5. 缓存 Rust 依赖、打包并验证：

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

`release-macos.py` 要求输出目录位于仓库上一级的 `artifacts/clarora/` 下，并且目录不存在；重复打包请换新目录。默认使用本机 ad-hoc 签名，不做公证；正式分发需要 `--sign-identity` 和 `--notarize-profile`。产物包含 `.app`、ZIP、可选 DMG，以及 `release.json`、`release-files.json`。

### Windows

要求 Windows 10 1809+ / 11 x64、Rust 1.93.1 MSVC、Visual Studio 2022 的 C++ 桌面开发工作负载、Windows SDK、CMake、Python 3、7-Zip 和 Inno Setup 6。使用 x64 开发者 PowerShell 或 Native Tools 命令提示符：

```powershell
git clone https://github.com/a49a/clarora.git
cd clarora\clarora-gpui
./scripts/prepare-windows-runtime.ps1
./scripts/build-windows.ps1 -Distribution -RuntimeDir ./target/windows-runtime -Installer
```

`prepare-windows-runtime.ps1` 会按 `windows-runtime-lock.json` 下载固定依赖并编译 ASR/TTS 运行库，第一次执行可能需要较长时间。`-Installer` 生成按用户安装的安装器；不加时只生成便携 ZIP。

默认输出到仓库上一级 `artifacts/clarora/gpui/windows/`，包含：

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

备份包括词卡、音视频学习附件、字幕、收藏、复习计划、统计、OCR 与跟读结果。凭证、设备设置、未收藏的最近视频、聊天会话草稿、原始 OCR 图片和跟读录音不纳入备份。较新桌面主版本默认把密钥保存在本机数据库，可在「密钥存储」开启系统凭证库（钥匙串 / 凭据库）；RN 客户端暂存本机应用数据库，具体以版本说明为准。云端备份通过 HTTPS 传输，尚无端到端加密。

详细配置、权限、数据格式和恢复策略见 [存储同步说明](docs/storage-sync.md)。

## AI

在「设置 → AI 服务」配置兼容 API 的 Base URL、API Key 和模型名称。聊天/翻译使用 `/chat/completions`；OCR 需要视觉模型；转写使用 `/audio/transcriptions`，模型必须支持 `verbose_json` 与 `segments` 时间轴。可以分别配置聊天和转写服务，也可使用用户自行运行的兼容本地服务。

本地模型可按准确度和耗时取舍：英语转写可先用 `small.en` 完成大部分字幕；`medium.en` 更准确，但耗时更长，后续只对字幕明显不准确的音频单独改用 `medium.en`。中文或多语言请使用 `small`、SenseVoice 或 `large-v3-turbo`，`.en` 模型仅适用于英语。

OCR 与跟读结果保存在本机。跟读评分比较识别文本与参考句，并非声学发音评测。没有 AI 配置时，本地导入、播放和复习仍可使用。

## 支持 Clarora

如果 Clarora 对你的学习有帮助，欢迎请我喝杯咖啡。打赏完全自愿，不影响任何功能。

<table>
  <tr>
    <th align="center">支付宝</th>
    <th align="center">微信</th>
  </tr>
  <tr>
    <td align="center"><img src="docs/assets/alipay.png" alt="支付宝打赏码" width="240" /></td>
    <td align="center"><img src="docs/assets/wechat-pay.png" alt="微信打赏码" width="240" /></td>
  </tr>
</table>

## 仓库结构

| 路径 | 内容 |
| --- | --- |
| `clarora-gpui/` | Rust + GPUI 桌面主版本（macOS / Windows） |
| `clarora-app/` | 四平台 React Native 客户端、共享数据层、对象存储与 AI 适配 |
| `tool/` | 客户端图标生成工具 |
| `docs/` | 客户端构建与存储同步说明 |

根 LICENSE 保留现有 BSD-3-Clause 声明。RN 端内置图谱使用本项目原创的小型示例数据；第三方词书和个人学习素材不随源码发布。第三方依赖、模型服务及 libmpv 等的许可需按实际分发内容另行核对。


桌面 Anki 导入：macOS / Windows 在闪卡页面选择“Anki 导入”，支持卡包和 Anki `.txt` / `.tsv` 文本；格式、合并规则与限制见 [导入说明](docs/anki-import.md)。
