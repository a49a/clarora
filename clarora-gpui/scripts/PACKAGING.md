# GPUI release 安装包

开发入口 `package-macos.sh`／`build-windows.ps1` 保留本机调试用法。下面的分发流程要求完整运行库、许可证和真实运行检查，失败会中止，不生成“已验证”结果。模型由用户在应用中下载，安装包不包含语音模型或学习资料。

## macOS

需要 Rust 1.93.1、Xcode 命令行工具、Python 3，以及与目标架构匹配的媒体和语音运行库。先按 `macos-runtime.example.json` 创建配置，填写实际文件、库搜索路径、版本、来源和许可证；示例中的占位符不能直接用于构建。ASR 和 TTS 必须使用相同版本且启用 TTS 的 Sherpa／ONNX 运行库和对应头文件。

在 `clarora-gpui/` 执行：

```sh
python3 scripts/release-macos.py \
  --runtime-config /absolute/path/runtime.json \
  --output ../../artifacts/clarora/gpui/macos/release-001 \
  --dmg
```

输出目录必须是工作区 `artifacts/clarora/` 下尚不存在的新目录。默认执行 `cargo build --release --locked --offline`，收集 Mach-O 传递依赖及 GGML CPU 后端，重写包内加载路径，生成图标、许可证与清单，再逐项签名和校验。`--binary` 仅用于明确复用已构建二进制，来源会写入清单。

`yt-dlp` 可配置已校验的独立可执行文件；开发机只有 Python 版本时，构建流程可随包收集解释器、标准库、模块与依赖，使用原生启动器隔离 Python 环境，不要求用户安装 Python。配置和收集结果均须通过真实提取检查。

产物包含 `.app`、ZIP、可选拖放安装 DMG，以及校验和与构建清单。默认使用本地 ad-hoc 签名。正式签名需显式传入 `--sign-identity 'Developer ID Application: …'`，公证另需 `--notarize-profile <已有的 notarytool profile>`。脚本不会创建凭证或默认提交公证。

```sh
python3 scripts/verify-macos-package.py \
  ../../artifacts/clarora/gpui/macos/release-001/Clarora-GPUI.app \
  --evidence ../../artifacts/clarora/gpui/macos/verify-001 \
  --launch-seconds 3
```

校验在隔离 HOME、精简 PATH 且没有开发机 Python／DYLD 配置的环境执行：签名、架构、最低系统版本、依赖闭包、文件哈希、FFmpeg 编解码、ffprobe、yt-dlp 本机 HTTP 提取、语音辅助程序加载、播放器与 PDFium 初始化。GUI 冒烟只启动本次独立进程，不操作已有应用；它不替代界面交互和真实声卡验收。解压 ZIP 到含空格／中文的新目录后，应再次执行同一校验。当前实际验证范围为 macOS arm64。

## Windows x64

在 Windows 上准备 Rust 1.93.1 MSVC、Visual Studio 2022 C++ 工作负载、CMake、Python 3、7-Zip 和 Inno Setup 6，并在 `clarora-gpui/` 执行：

```powershell
./scripts/prepare-windows-runtime.ps1
./scripts/build-windows.ps1 -Distribution -RuntimeDir ./target/windows-runtime -Installer
```

`windows-runtime-lock.json` 固定下载来源、版本和 SHA-256。准备流程构建 ASR／TTS，收集 mpv、FFmpeg／ffprobe、yt-dlp、PDFium、Whisper／GGML、Sherpa／ONNX 和 CRT。每个 EXE 的 DLL 闭包必须在自身目录或系统目录内可解析；辅助程序和 CPU 后端位于 `tools/`。

默认输出到工作区 `artifacts/clarora/gpui/windows/`；`-Output` 可指定路径。打包先检查运行库，再构建 ZIP 和可选安装器，使用 `PATH=System32` 执行运行检查并复验 ZIP。默认文件名标明 `unsigned`。已有代码签名证书时，可传 `-CertificateThumbprint <证书指纹>` 签署自有程序和安装器；第三方库保留原始签名。

```powershell
./scripts/verify-windows-installer.ps1 `
  -Installer /absolute/path/Clarora-GPUI-0.2.1-windows-x64-setup-unsigned.exe `
  -Evidence /absolute/path/installer-evidence
```

安装器按用户安装、无须管理员权限；验证覆盖安装、升级、运行环境检查和卸载，确认 `%LOCALAPPDATA%/Clarora-GPUI` 学习资料保留。验证拒绝接管已有同 AppId 安装。CI 已接入完整构建及安装生命周期；本次开发机为 macOS，Windows 原生构建、安装器和远端 CI 尚待执行。

## 诊断、许可证与发布前检查

完整包包含 `runtime-manifest.json` 标记。macOS 放在 `Contents/Resources/`，Windows 放在 EXE 旁；程序据此只加载包内组件，避免开发机安装的工具掩盖缺失依赖。设置页可查看并刷新运行环境，也可在不打开用户数据库的情况下生成诊断：

```sh
/absolute/path/clarora-gpui --runtime-check /absolute/path/runtime-check.json
```

各包附带第三方文本和 Rust 依赖清单，包含来源、版本、校验和及许可证证据。`third-party-licenses/` 保存固定上游版本的补充文本和 SPDX 标准条款。上游未提供许可证文件的条目保留原始 Cargo 声明及 `audit_required`，标准条款不冒充上游版权声明；`legal_audit_passed: false` 表示仍需人工审计。收集清单通过不表示法律审计通过。

正式公开发布仍需完成许可证审计、独立 review、目标系统交互／权限／声卡验收、正式签名与 macOS 公证，并复核最终归档哈希。候选包的本机隔离检查不能替代另一台干净机器或 Windows 的实际验收。
