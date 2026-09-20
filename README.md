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

v0.1.0 起提供 macOS 与 Windows 安装包,从 [GitHub Releases](https://github.com/a49a/clarora/releases/latest) 下载:

- macOS:`Clarora-macos-arm64.dmg`(Apple Silicon,运行时依赖已内置,无需 Homebrew)
- Windows:`Clarora-windows-x64.zip`(解压后按内附说明侧载安装,需开启开发者模式)

安装包未做代码签名:macOS 首次打开如被 Gatekeeper 拦截,右键 App 选「打开」;Windows 如有 SmartScreen 提示,选择「仍要运行」。

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

Windows/iOS 的构建步骤和平台限制见 [平台说明](docs/platform-builds.md)。`.github/workflows/` 中的 CI 在推送与 PR 时运行共享代码的类型检查和测试，并编译验证 Android / macOS / iOS；Windows 安装包随推送构建，也可手动触发。共享代码和自动化测试不等同于四个平台的真机验收。

`npm run macos` 会检查本机 Pods；首次构建或原生依赖锁文件不一致时自动执行 `pod install`，因此需要先安装 CocoaPods。修改 Podfile 后，可在 `clarora-app/` 中执行 `npm run macos:pods` 手动更新依赖。`Pods/` 是本地生成内容，不提交到仓库。

```sh
npm run typecheck
npm test
```

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
