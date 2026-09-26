# 发布流程（Release Runbook）

版本唯一真源是 git tag（`v0.2.0` 格式）。打 tag 并推送后，Release 工作流自动构建
macOS dmg、Windows zip 与未签名 iOS IPA，生成校验和，并创建 GitHub Release。
配置 Android 签名 secrets 且将仓库变量 `CLARORA_ANDROID_RELEASE_ENABLED` 设为 `true` 时，工作流还会构建签名 Android APK；未启用时元数据把 Android 标为 `planned`。
默认 iOS IPA 不需要 Apple 账号，下载后须用户自行签名才能安装。可选 iOS 签名构建与渠道交付见 [移动端发布](mobile-releases.md)。

## 发版步骤

1. 确认 master 的四条 CI 流水线（test / android / macos / windows）全绿。
2. 更新 `CHANGELOG.md`：把 Unreleased 下的条目归入新版本号与日期。
3. 提交并推送 master。
4. 打 tag 并推送（这一步触发 Release 构建）：

   ```sh
   git tag -a v0.2.0 -m "Clarora v0.2.0"
   git push origin v0.2.0
   ```

5. 在 Actions 确认 Release 工作流的 resolve / macos / windows / ios / release job 全绿；只有启用 Android 发布时才要求 android job 全绿，
   GitHub Releases 页面出现产物、`SHA256SUMS.txt` 与 `release-metadata.json`。
   iOS 资产为 `Clarora-ios-unsigned.ipa`，另附 `ios-unsigned-build.json`，IPA 纳入校验和；Release 说明会注明安装需自行签名。
   Windows job 会对最终 zip 做安装包能力验收（主程序、端侧转写、PDF 运行时），
   缺失即失败，不会发布不完整包。
6. 客户端发布后需触发官网构建部署：`clarora-web` 构建时自动从 GitHub Releases API 拉取最新
   版本并渲染下载卡片（`config.js` 是生成产物，不要手改）。也可把 Release 随包
   发布的 `release-metadata.json` 交给官网构建消费，两者是同一 schema。

## Windows 依赖步骤长时间无输出

`All requested installations completed successfully` 表示 vcpkg 已结束；下一阶段是
sherpa-onnx 下载与解压，然后是 pdfium 和 CMake 编译。脚本会输出各阶段名称和下载进度，
下载连续 60 秒低于 1 KiB/s 会失败，单次请求最多 10 分钟，瞬时错误最多重试两次。
ASR 步骤总上限为 35 分钟。标准安装包声明端侧转写能力，因此 **ASR 失败会阻断发布**：
发布流程在打包后验证最终 zip 内确实包含 `clarora_asr.dll`、`pdfium.dll` 与
`Pdfium.dll`，缺失即失败（提示改用独立命名的降级包）。若后续需要能力降级的精简包，
使用独立产物名并在 Release 说明中声明，不得静默替换标准包。

已启动的运行不会加载后续提交中的修复；重新运行旧任务也仍然使用旧提交。
验证修复时需要在包含修复的新提交上启动工作流。

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
- **Windows**：包为未签名 sideload 包，需在「设置 → 开发者选项」开启旁加载，
  解压后通过 PowerShell `Add-AppxPackage` 安装 `.msix`；SmartScreen 提示属正常。

## 签名路线（Phase 2，可选加固）

- macOS：Developer ID 签名 + 公证。需要 Apple Developer 账号；在 CI 中以
  `notarytool`（App Store Connect API 密钥存 Actions secrets）提交并装订，
  之后产物双击即开。
- Windows：OV/EV 证书 `signtool` 签名，或提交 Microsoft Store（免自签但有审核）。
