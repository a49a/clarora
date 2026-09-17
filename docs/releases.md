# 发布流程（Release Runbook）

版本唯一真源是 git tag（`v0.1.0` 格式）。打 tag 并推送后，Release 工作流自动构建
macOS dmg 与 Windows zip，生成校验和，并创建 GitHub Release。

## 发版步骤

1. 确认 master 的四条 CI 流水线（test / android / macos / windows）全绿。
2. 更新 `CHANGELOG.md`：把 Unreleased 下的条目归入新版本号与日期。
3. 提交并推送 master。
4. 打 tag 并推送（这一步触发 Release 构建）：

   ```sh
   git tag -a v0.1.0 -m "Clarora v0.1.0"
   git push origin v0.1.0
   ```

5. 在 Actions 确认 Release 工作流三个 job（macos / windows / release）全绿，
   GitHub Releases 页面出现产物与 `SHA256SUMS.txt`。
6. 更新官网 `clarora-web/config.js` 的 `downloads`（模板见下），提交推送后
   官网下载卡片生效。

## Windows 依赖步骤长时间无输出

`All requested installations completed successfully` 表示 vcpkg 已结束；下一阶段是
sherpa-onnx 下载与解压，然后是 pdfium 和 CMake 编译。脚本会输出各阶段名称和下载进度，
下载连续 60 秒低于 1 KiB/s 会失败，单次请求最多 10 分钟，瞬时错误最多重试两次。
ASR 步骤总上限为 35 分钟，发布流程中失败会阻止不完整的安装包发布。

已启动的运行不会加载后续提交中的修复；重新运行旧任务也仍然使用旧提交。
验证修复时需要在包含修复的新提交上启动工作流。

## 下载链接模板（clarora-web/config.js）

`releases/latest/download/<文件名>` 是 GitHub 的永久链接，始终指向最新
Release 的同名产物（产物名保持固定，不要加版本号）：

```js
downloads: {
  macos: { url: "https://github.com/a49a/clarora/releases/latest/download/Clarora-macos-arm64.dmg", version: "0.1.0" },
  windows: { url: "https://github.com/a49a/clarora/releases/latest/download/Clarora-windows-x64.zip", version: "0.1.0" },
  android: null,
  ios: null,
},
```

## 未签名分发的用户提示

当前产物未做代码签名（见下文签名路线），首次打开会有系统拦截：

- **macOS**：下载 dmg 安装后，终端执行 `xattr -cr /Applications/Clarora.app`，
  或在「系统设置 → 隐私与安全性」中允许。
- **Windows**：包为未签名 sideload 包，需在「设置 → 开发者选项」开启旁加载，
  解压后通过 PowerShell `Add-AppxPackage` 安装 `.msix`；SmartScreen 提示属正常。

## 签名路线（Phase 2，可选加固）

- macOS：Developer ID 签名 + 公证。需要 Apple Developer 账号；在 CI 中以
  `notarytool`（App Store Connect API 密钥存 Actions secrets）提交并装订，
  之后产物双击即开。
- Windows：OV/EV 证书 `signtool` 签名，或提交 Microsoft Store（免自签但有审核）。
