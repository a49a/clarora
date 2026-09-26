# 移动端签名与发布

Android 正式 Release 上传签名 APK；iOS 同时上传 `Clarora-ios-unsigned.ipa`，供用户自行签名后安装。未签名 IPA 不能下载后直接安装，也不代表 TestFlight／App Store 已开放。另保留可选的正式签名构建入口。构建成功不等于设备验收完成。

## 无 Apple 开发者会员时：未签名 IPA

默认 Release 使用 iPhoneOS SDK 构建 arm64 真机 Release 应用，包含 JavaScript bundle，打包为 `Payload/Clarora.app`。无需 Apple 账号、分发证书或描述文件；这是明确选择的未签名产物，不是签名失败后的降级包。

GitHub Release 提供 `Clarora-ios-unsigned.ipa`、`ios-unsigned-build.json` 和包含 IPA 的 `SHA256SUMS.txt`。下载后先核对校验和，再使用自己的有效签名资源重新签名安装；重新签名后的文件校验和会改变。项目当前不提供一键重签名工具，也未完成重签名后的真机验收。不要把此包当作可直接安装的 TestFlight／App Store 包。

如需免费个人真机开发测试，可在 Mac 的 Xcode 登录自己的 Apple 账号，打开 `clarora-app/ios/Clarora.xcworkspace`，选择 Clarora-iOS scheme，在 Signing & Capabilities 中启用自动签名、选择 Personal Team 并使用可用的唯一 Bundle Identifier，连接自己的设备构建运行。此方式是从源码构建；免费描述文件有效期为 7 天，过期需重新构建安装。参见 [Apple 免费账号说明](https://developer.apple.com/help/account/basics/about-your-developer-account)。

本地生成未签名 IPA（先安装 npm 依赖与 Pods）：

```sh
cd clarora-app
SOURCE_SHA="$(git rev-parse HEAD)" CLARORA_BUILD_NUMBER=1001 bash scripts/build-ios-unsigned.sh
```

输出位于 `clarora-app/dist/mobile/`。需要 macOS、Xcode 的 iPhoneOS SDK、Node 24 和 CocoaPods。使用干净且固定到目标 SHA 的 checkout；发布报告记录 SHA、版本、构建号与校验和。

## 准备签名资源（Android 正式包 / 可选 iOS 正式分发）

在仓库 Settings → Secrets and variables → Actions 配置下表。不要把密钥、密码或描述文件提交到仓库。使用专用发布签名密钥并妥善备份；Android 后续升级需要保持签名身份一致。

| 类型 | 名称 | 用途 |
| --- | --- | --- |
| Secret | `ANDROID_KEYSTORE_BASE64` | 正式 keystore 文件的 base64，不能使用仓库 Debug keystore |
| Secret | `ANDROID_STORE_PASSWORD` | keystore 密码 |
| Secret | `ANDROID_KEY_ALIAS` | 正式签名 alias |
| Secret | `ANDROID_KEY_PASSWORD` | 私钥密码 |
| Secret | `ANDROID_CERT_SHA256` | 正式证书 SHA-256 指纹，支持带冒号格式；与最终 APK 签名核对 |
| Variable | `CLARORA_ANDROID_RELEASE_ENABLED` | 设为 `true` 才在 GitHub Release 中构建并发布签名 Android APK |
| Secret | `IOS_CERTIFICATE_BASE64` | 包含私钥的 Apple Distribution `.p12` 的 base64 |
| Secret | `IOS_CERTIFICATE_PASSWORD` | `.p12` 导出密码 |
| Secret | `IOS_PROFILE_BASE64` | 对应 `com.clarora.app` 的分发描述文件 base64 |
| Variable | `IOS_TEAM_ID` | Apple Developer Team ID |

启用 Android 发布前，签名资源必须齐全；未启用时正式 Release 会把 Android 标为 `planned`，不会上传 APK。iOS 描述文件必须未过期、Team/Bundle ID 匹配；`app-store-connect` 使用商店分发 profile，`release-testing` 使用包含已登记设备的 Ad Hoc profile。工作流临时导入签名材料，结束时清理；不上传 archive、profile 或私钥。

## 先构建产物，再执行分发

1. 在包含这些脚本的提交上创建版本 tag，保证 tag 的 `v<版本>` 与 `clarora-app/package.json` 一致。运行旧 tag 不会自动使用新脚本；不要移动已经发布的 tag。
2. Actions → **Mobile release artifacts (no publishing)**：填写 tag、platform 和 build_number。工作流解析 tag 的完整 SHA，再按此 SHA 构建。选择 iOS 时填写 `ios_method`：默认 `unsigned`，无需任何 Apple secrets；已有付费会员和分发资源时可选 `app-store-connect` 或 `release-testing`。
3. 下载该 run 的 `Clarora-android` 或 `Clarora-ios-<method>` artifact，核对 `android-build.json`／`ios-unsigned-build.json`／`ios-build.json` 的 SHA、版本、构建号和产物 SHA-256。正式签名 iOS artifact 保留 7 天，应及时保存到受控位置。
4. Android 安装最终签名 APK，验证首次安装、覆盖升级和用户数据保留。正式 Release 工作流使用相同构建入口并上传 `Clarora-android.apk`、构建报告及校验和。
5. `unsigned` 产物须自行签名后安装；它不能上传商店作为正式签名包。iOS 商店导出的 IPA 交由有权限的发布者通过 Apple Transporter 上传 App Store Connect，等待处理后配置 TestFlight 或提交 App Store 审核；Ad Hoc IPA 只在 profile 登记设备上验收。当前工作流只导出，不自动上传 Apple 或公开 IPA。
6. 在实际可用的分发渠道上完成安装、升级、凭证重启读取、文件选择、播放/录音及备份恢复验证，保留构建号和截图。缺账号、证书或设备时记录未完成项。

Android 正式 tag 构建的 `versionName` 读取 package.json，`versionCode = major × 1000000 + minor × 1000 + patch + 1`（major < 999，minor/patch < 1000）。例如 0.1.0 为 1001。手动 Android 构建应填写相同代码；不要分发大于下一正式版本的手动构建号。同一 tag 重建不会提高版本代码。iOS build_number 由执行者指定，必须满足所选渠道版本递增要求；脚本接受 1～9 位正整数。

## 官网 iOS 渠道入口

只有渠道已实际开放、且对应本次版本时，才设置仓库 Variables：

- `IOS_DISTRIBUTION_URL`：`https://testflight.apple.com/join/<邀请代码>` 或 `https://apps.apple.com/.../id<数字>`。
- `IOS_DISTRIBUTION_VERSION`：与本次 package.json 相同的版本，不带 `v`。

Release 工作流校验 URL 与版本后，把 `channel: testflight` 或 `app-store` 写入 `release-metadata.json`。没配置 URL 时 iOS 保持 `planned`；配置过期版本时阻断发布，需重新核对渠道或清空 URL。构建 IPA（包括 Release 附带的未签名 IPA）不会自动把 iOS 渠道标为可用。官网继续只显示实际开放的渠道；未签名包从 GitHub Release 资产列表获取。

官网构建从实际 Release APK 资产识别 Android，从同版本、同 SHA 的发布元数据识别 iOS 渠道。客户端发布后仍需触发官网构建部署才能更新静态页面；两仓库之间尚无自动部署触发。

## 本地入口与限制

Android：配置 `CLARORA_ANDROID_KEYSTORE`（绝对路径）、`CLARORA_ANDROID_STORE_PASSWORD`、`CLARORA_ANDROID_KEY_ALIAS`、`CLARORA_ANDROID_KEY_PASSWORD`、`CLARORA_ANDROID_CERT_SHA256`、`CLARORA_BUILD_NUMBER`、`SOURCE_SHA`、`ANDROID_HOME` 后，运行 `bash clarora-app/scripts/build-android-release.sh`。需要 Node 24、JDK 17、Android SDK Build Tools 36.0.0；密钥只能从安全环境注入。

iOS：使用上述 iOS 环境变量、`CLARORA_BUILD_NUMBER`、`SOURCE_SHA` 和可选 `IOS_EXPORT_METHOD`，安装 npm 依赖与 Pods 后，运行 `bash clarora-app/scripts/build-ios-release.sh`。需要 Xcode、CocoaPods 及其 xcodeproj Ruby 库。建议使用干净 checkout；脚本临时修改应用签名设置，退出时恢复工程文件及 keychain 搜索列表。

官方参考：[Android 应用签名](https://developer.android.com/studio/publish/app-signing)、[APK 签名校验](https://developer.android.com/tools/apksigner)、[Apple 测试与发布分发](https://developer.apple.com/documentation/xcode/distributing-your-app-for-beta-testing-and-releases)。
