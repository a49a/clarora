# bilibili 视频下载与导入

macOS 和 Windows 的「视频学习」中展开「从 bilibili 下载导入」，粘贴完整视频链接、分享文案里的完整链接或 BV / av 编号，点击「下载并导入」。带 `?p=2` 的链接只下载指定分 P；不填写分 P 时下载默认视频，不批量下载合集。

下载完成后自动打开播放，并加入最近打开列表；重启后仍从本机文件打开。支持取消，失败或取消的下载不加入列表。离开视频学习页面会取消进行中的下载。音画合并为 H.264 / AAC MP4，优先获取该格式，下载器提供的英文、中文字幕会嵌入视频；没有字幕时仍能导入。下载中显示任务状态，可随时取消。

目前不处理登录、Cookie、会员及地区权限配置，只下载无需登录即可访问的普通视频。`b23.tv` 短链接请先在浏览器打开，再复制 `bilibili.com/video/…` 完整链接。站点限制或下载器版本过旧可能造成失败，错误显示在页面中。

## 安装依赖

两端都调用用户本机安装的 [yt-dlp](https://github.com/yt-dlp/yt-dlp) 和 ffmpeg，不需要 Clarora 服务端。

macOS：

```sh
brew install yt-dlp ffmpeg
# 站点变化后可以更新：
brew upgrade yt-dlp
```

程序查找 PATH、`/opt/homebrew/bin`、`/usr/local/bin` 和 `~/.local/bin`。

Windows：

```powershell
winget install yt-dlp.yt-dlp
winget install Gyan.FFmpeg
```

确保 `yt-dlp.exe`、`ffmpeg.exe` 和 `ffprobe.exe` 在 PATH 或 `%LOCALAPPDATA%\Microsoft\WinGet\Links` 中，安装后重启 Clarora。也可从各项目官方发行页安装。

## 文件与平台实现

- macOS 文件在 `~/Library/Application Support/Clarora/bilibili/<任务标识>/`。
- Windows 文件在应用 `LocalState/bilibili/<任务标识>/`。卸载应用可能清除该目录。
- 完整下载保留在本机，最近列表只记录入口，不是下载文件的完整管理列表。完整视频不随「同步与备份」上传，收藏生成的视频片段仍走现有闪卡备份。
- macOS 使用独立进程组；Windows UWP 通过包内 full-trust 下载助手调用下载器。安装包因此声明 `runFullTrust`，助手仅接受视频链接，使用参数数组启动下载器，忽略用户的 yt-dlp 配置。取消和超时会停止下载器及 ffmpeg 子进程。
- Windows 助手使用 .NET 8 自包含发布，运行时不需要另装 .NET；开发构建需要 .NET 8 SDK。`Clarora.csproj` 在构建时自动发布并把助手加入包，Debug 与 Release 共用此入口。

## 验证

`npm run typecheck` 与 `npm test` 验证共享回归。macOS 原生下载链路可用以下驱动检查（不代替界面 E2E）：

```sh
cd clarora-app
xcrun clang++ -fobjc-arc -fblocks -framework Foundation scripts/check-bilibili-download.mm -o /tmp/clarora-download-check
/tmp/clarora-download-check 'https://www.bilibili.com/video/<BV编号>'
# 第三个参数存在时，在启动后取消：
/tmp/clarora-download-check 'https://www.bilibili.com/video/<BV编号>' cancel
```

Windows 必须通过 `npm run windows:build` 检查 UWP 与包清单，安装产物后验证输入、真实下载、播放、最近列表、取消和失败重试。单独编译下载助手不能替代这些检查。
