# 发布流程（Release Runbook）

> **RN 桌面发行弃用（2026-11-17）：** RN 的 Windows 桌面工程与构建链已移除，macOS 桌面构建仅保留历史维护代码。新桌面发布走 `clarora-gpui/` 的打包流程。本文件保留用于 RN 移动端和历史版本维护；RN Release 工作流不再构建 Windows 产物。

RN 版本唯一真源是 `rn-v<版本>` tag（版本与 `clarora-app/package.json` 一致）；GPUI 桌面使用独立的 `v<版本>` tag。打 RN tag 并推送后，RN Release 工作流自动构建
macOS dmg 与未签名 iOS IPA，生成校验和，并创建 GitHub Release；Windows 产物由 GPUI 桌面流程单独发布。
配置 Android 签名 secrets 且将仓库变量 `CLARORA_ANDROID_RELEASE_ENABLED` 设为 `true` 时，工作流还会构建签名 Android APK；未启用时元数据把 Android 标为 `planned`。
默认 iOS IPA 不需要 Apple 账号，下载后须用户自行签名才能安装。可选 iOS 签名构建与渠道交付见 [移动端发布](mobile-releases.md)。

## 发版步骤

1. 确认 master 的 CI（test / android / macos / ios）与 GPUI desktop（macOS / Windows）全绿。
2. 更新 `CHANGELOG.md`：把 Unreleased 下的条目归入新版本号与日期。
3. 提交并推送 master。
4. 打 RN tag 并推送（这一步触发 RN Release 构建）：

   ```sh
   git tag -a rn-v0.2.1 -m "Clarora RN v0.2.1"
   git push origin rn-v0.2.1
   ```

5. 在 Actions 确认 RN Release 工作流的 resolve / macos / ios / release job 全绿；只有启用 Android 发布时才要求 android job 全绿，
   GitHub Releases 页面出现产物、`SHA256SUMS.txt` 与 `release-metadata.json`。
   iOS 资产为 `Clarora-ios-unsigned.ipa`，另附 `ios-unsigned-build.json`，IPA 纳入校验和；Release 说明会注明安装需自行签名。
   RN Release 不再构建或校验 Windows 产物；Windows 使用 GPUI 桌面发布流程，见下文。
6. 客户端发布后需触发官网构建部署：`clarora-web` 构建时自动从 GitHub Releases API 拉取最新
   版本并渲染下载卡片（`config.js` 是生成产物，不要手改）。也可把 Release 随包
   发布的 `release-metadata.json` 交给官网构建消费，两者是同一 schema。

## macOS 最低系统版本验收

客户端支持 macOS 14.0+（Apple Silicon）。macOS Release 暂时使用 `macos-14`
runner 安装 Homebrew 依赖：在 macOS 15 上安装的预编译库可能要求 macOS 15，
即使 Xcode 的 `MACOSX_DEPLOYMENT_TARGET` 设为 14.0 也不会重新编译这些库。
不能通过修改 dylib 的版本标记修复缺失的系统符号。

macOS 14 runner 默认 Xcode 15.4，React Native 0.81 要求至少 16.1；发布 job
显式选择镜像内的 Xcode 16.2，再安装 Pods 和编译。切换 Xcode 不改变宿主系统
及 Homebrew bottle 的系统版本。

`verify-release.sh` 调用 `verify-macos-compatibility.py` 检查包内所有 Mach-O
文件（包括嵌套 framework），任何库缺少 arm64、最低系统版本信息或要求高于
14.0 都会阻断发布。随后在 macOS 14 上直接启动已打包的 Release，确认存活
至少 10 秒；`Clarora-macos-smoke` artifact 保存兼容性清单、启动日志和结果。
此检查验证启动，不代表音视频播放、转写等完整功能已经验收。

GitHub 将于 2026-11-02 移除 `macos-14` runner；此前需迁移为自建 macOS 14
runner，或为新版 runner 提供按 macOS 14 编译且经过真实系统验证的依赖闭包。
迁移时保留上述兼容性检查，不得直接切回新版 Homebrew bottle。
参见 [GitHub runner 退役公告](https://github.blog/changelog/2026-10-01-github-actions-macos-14-runner-image-retirement/)。

本机复现验收：

```sh
bash clarora-app/scripts/verify-release.sh /path/to/Clarora.app
python3 clarora-app/scripts/smoke-macos-release.py /path/to/Clarora.app /tmp/clarora-macos-smoke
```

## Windows 桌面发布（GPUI）

RN Release 不构建 Windows。Windows 桌面产物由 `GPUI desktop` 工作流在 `windows-2022`
上生成，包括便携 ZIP、按用户安装器和安装器验证证据；发布时用 `Upload GPUI Windows
release assets` 工作流把 `Clarora-windows-x64.zip` 与
`Clarora-windows-x64-setup-unsigned.exe` 上传到对应 Release，并重新生成
`SHA256SUMS.txt`、`source-provenance.json` 和 `release-metadata.json`。
构建与验证细节见 [`clarora-gpui/scripts/PACKAGING.md`](../clarora-gpui/scripts/PACKAGING.md)。

## 官网下载链接的来源

下载卡片由 `clarora-web/release-metadata.json` 与 Release 工作流发布的同 schema
元数据在构建期生成，链接一律指向固定版本目录
`releases/download/<tag>/<文件名>`（tag 移动会导致固定链接失效，产物名保持固定，
不要加版本号）。不要使用 `releases/latest/download/...`——官网静态检查会直接
拒绝这种链接。

## 未签名分发的用户提示

当前桌面产物未做代码签名（见下文签名路线），首次打开会有系统拦截：

- **macOS**：下载 dmg 安装后，终端执行 `xattr -cr /Applications/Clarora.app`，
  或在「系统设置 → 隐私与安全性」中允许。
- **Windows**：优先运行未签名的按用户安装器 `Clarora-windows-x64-setup-unsigned.exe`；
  也可解压便携 ZIP。SmartScreen 提示时点「更多信息」→「仍要运行」，无需开发者模式或侧载脚本。

## 签名路线（Phase 2，可选加固）

- macOS：Developer ID 签名 + 公证。需要 Apple Developer 账号；在 CI 中以
  `notarytool`（App Store Connect API 密钥存 Actions secrets）提交并装订，
  之后产物双击即开。
- Windows：使用 OV/EV 证书对 GPUI 安装器和 EXE 执行 `signtool` 签名；也可提交 Microsoft Store（免自签但有审核）。
