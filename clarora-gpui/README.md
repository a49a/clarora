# Clarora GPUI / Rust 桌面客户端

与 `clarora-app/` 的 React Native 客户端并行保留，目标为 macOS、Windows 全功能对照。使用 GPUI 原生窗口和 Rust 业务代码，不需要 Metro，也不内嵌 React Native。当前是开发对照版，尚不能视为全部功能、全部平台验收完成。

## 启动

在仓库根目录执行 `npm run gpui`。macOS 会先构建带 Clarora Dock 图标的 `.app` 再启动；可用 `npm run gpui -- --data-dir /absolute/path` 传入测试目录。Windows 请使用下方 PowerShell 启动方法。直接调试 Rust 二进制也可执行（macOS 裸二进制不包含应用图标资源）：

```sh
cargo run --manifest-path clarora-gpui/Cargo.toml --locked
```

需要 Rust 1.93.1。macOS 需要 Xcode Command Line Tools；Windows 使用 Rust MSVC 工具链、Visual Studio 2022 的 C++ 桌面开发工作负载与 Windows SDK。

macOS 媒体工具：

```sh
brew install mpv ffmpeg yt-dlp
sh clarora-gpui/scripts/package-macos.sh
python3 clarora-gpui/scripts/install-pdfium.py clarora-gpui/target/Clarora-GPUI.app/Contents/MacOS
codesign --force --deep --sign - clarora-gpui/target/Clarora-GPUI.app
open clarora-gpui/target/Clarora-GPUI.app
```

打包脚本生成本机调试 `.app`，不是已经完成签名、公证和运行库分发的安装包。`libmpv`、FFmpeg、yt-dlp 仍需安装。Intel Mac 的 PDFium 需自行提供匹配架构的库，安装脚本仅包含 Apple Silicon 和 Windows x64 的固定版本及 SHA-256。

Windows PowerShell：

```powershell
./clarora-gpui/scripts/build-windows.ps1
python ./clarora-gpui/scripts/install-pdfium.py ./clarora-gpui/target/package
./clarora-gpui/target/package/clarora-gpui.exe
```

将匹配架构的 `mpv-2.dll` 及其依赖放在 EXE 旁；将 `ffmpeg.exe`、`ffprobe.exe`、`yt-dlp.exe` 放在 EXE 旁的 `tools/`，或加入 PATH。Windows 程序是普通桌面 EXE，不使用现有 React Native Windows 的 UWP 下载伴随进程。两平台共享下载实现；进程组取消分别使用 Unix 进程组、Windows Job Object。

## 界面与快捷操作

桌面界面采用暖灰与青绿色主题，导航按学习空间、资料库和个人工具分组。页面左上角可收起/展开左侧导航，也可按 `Cmd/Ctrl+B`；导航从左侧滑动展开/收起，内容区同步扩缩；按钮用加粗双箭头指示方向，并记住状态，重启后保留。侧栏底部可切换浅色/深色，也可按 `Cmd/Ctrl+Shift+L`。闪卡页默认聚焦学习卡片，点击“添加句子卡”展开编辑区；`Cmd/Ctrl+Shift+N` 可直接打开句子编辑并定位输入框。视频页采用左右工作台：左侧播放器，右侧“字幕／选集／AI 提问”页签；拖动中间分隔条调整宽度，窄窗口自动上下排列。字幕按正文长度展开，支持点击播放、逐句复制及提问；选集独立滚动，不再占用播放器下方空间。点击标题栏的 P 数可切到选集。字幕叠加在画面底部，可从控制栏“设置”菜单选择 14–32px 字号（默认 16px），立即生效并自动保存，普通与全屏播放共用；控制栏提供播放、前后 10 秒、进度拖动、倍速菜单、音量滑杆/静音；“设置”菜单集中管理字幕字号、循环方式和精听工具。选择“A–B 精听工具”展开起终点、清除、逐句切换、重听本句及片段闪卡；设置有效 A/B 后自动循环，进度条显示范围和金色边界，切换字幕句时退出旧循环。B 必须晚于 A；重设 A 超过旧 B 会清除旧 B。标题栏“更多”展开本地视频、bilibili 导入、字幕轨道、SRT 导出和最近视频；首次打开没有视频时自动显示。控制栏的“全屏”、双击视频画面或 `F11` 可进入视频专用原生全屏，`Esc` 或“退出全屏”返回；播放继续，退出后保留原导航/助手状态。

## 数据和使用

- macOS 资料目录：`~/Library/Application Support/Clarora-GPUI/`；Windows：`%LOCALAPPDATA%\Clarora-GPUI\`。使用独立 SQLite 和附件目录，可与原版同时运行。
- 可通过 `--data-dir /absolute/path` 指定测试资料目录。不会自动打开或修改原版数据库。
- 设置页可只读导入原版 SQLite 快照、导入/导出 v2 ZIP 资料备份，或配置 S3/OSS。密钥保存于系统凭证库，不写入资料备份；需在新版重新配置。
- 视频页支持本地视频、bilibili BV/av/完整地址及 `?p=` 分 P、取消下载、字幕主副轨、外部字幕、A/B、倍速和片段卡。需本机媒体工具，网络和视频可用性由视频来源决定。
- 视频字幕可直接提问：生成字幕或加载 UTF-8 SRT / VTT 后，点“问当前句”“问整段字幕”，或某条字幕的“问 AI”，在右侧输入问题并发送。引用包含时间戳；当前句和指定行会暂停播放，并固定本次引用。不同视频、分P、字幕内容和引用范围各自保留会话，回答可存为问答卡。整段字幕超过 10,000 句或 120,000 字符时提示改用单句，不静默截断。内嵌字幕未解析为完整时间轴时只支持播放器当前句。使用设置中的聊天 API；本地 Whisper / SenseVoice 仅负责转写，不负责问答。
- AI 可配置兼容接口的聊天、视觉、带时间轴转写模型；独立侧栏按固定材料保留最多 12 个聊天会话和草稿，推理内容可展开。聊天流不受支持（HTTP 404/405）时回退一次非流式接口；断流、截断和模型错误不会保存为成功回答。
- 听力显示真实解码波形，支持点击/拖动定位、拖动 A/B 标记、逐词选择、Shift 扩展片段和 AI 重点释义。词语时间按字幕字符长度估算，不代表声学逐词对齐。
- 口语分数表示识别文本对齐，包含漏读、读错、多读、完整度与语速。
- 本地转写支持 Whisper 模型和 SenseVoice。模型由设置页下载。点击“选用”后，模型行高亮并显示“已选择 · 待保存”；点击“保存 AI 与学习设置”成功后显示“已选用”和“当前生效”，重启后保留。Whisper 可使用 PATH 中的 `whisper-cli`；SenseVoice 需要下述原生辅助程序。
- `Cmd/Ctrl+1…9` 切换前九个页面；`Cmd/Ctrl+O` 按当前页面打开/导入文件，`Cmd/Ctrl+J` 切换 AI 侧栏，`Cmd/Ctrl+Enter` 发送问题，`Cmd/Ctrl+N` 新会话，`Cmd/Ctrl+←/→` 上一项/下一项（PDF 为翻页）。页面获得焦点后空格翻面/播放，方向键切卡/快进，`1/2/3` 评分。输入框编辑时不拦截无修饰键的输入。

## 端侧语音模型：名称、语言和运行要求

这里的“端侧模型”指 **ASR（语音识别）模型**，用于音频转写、视频字幕和跟读文本识别。模型文件下载后，本地转写不上传音轨；聊天、图片识字和 AI 点评仍调用设置中的远端 API。`engine=local` 使用下面的本地模型，`engine=compatible` 使用远端 `asr_model`，两者的模型 ID 不可混用。

| 设置中的 ID | 实际模型 | 语言 | 参数量 / 下载体积 | 本地文件（相对资料目录） |
| --- | --- | --- | --- | --- |
| `tiny.en` | OpenAI Whisper Tiny.en | 仅英语 | 39M / 约 75 MiB | `models/ggml-tiny.en.bin` |
| `base.en` | OpenAI Whisper Base.en | 仅英语 | 74M / 约 142 MiB | `models/ggml-base.en.bin` |
| `small.en` | OpenAI Whisper Small.en | 仅英语 | 244M / 约 466 MiB | `models/ggml-small.en.bin` |
| `base` | OpenAI Whisper Base 多语言版 | 含中文、英语、日语等 | 74M / 约 142 MiB | `models/ggml-base.bin` |
| `small` | OpenAI Whisper Small 多语言版 | 含中文、英语、日语等 | 244M / 约 466 MiB | `models/ggml-small.bin` |
| `sense-voice` | FunAudioLLM SenseVoiceSmall，sherpa-onnx 2024-07-17 导出版 | 普通话、英语、日语、韩语、粤语 | INT8 ONNX，体积以实际文件为准 | `models/sense-voice/model.int8.onnx` 和 `tokens.txt` |

英语可从较小的 `tiny.en` 开始试用；中文 bilibili 视频使用 `base` / `small` 或 SenseVoice。`.en` 表示英语专用，不适用于中文。`base.en` 是首次配置的默认值，不会按视频语言自动换模型。更大版本的耗时和识别效果应在同一音轨上实测比较，表中体积不是最低内存要求，也不是本机速度测评。

Whisper 使用 [whisper.cpp 的 GGML 权重](https://huggingface.co/ggerganov/whisper.cpp/tree/main)，不是 PyTorch 权重；语言和参数说明见 [OpenAI Whisper](https://github.com/openai/whisper)，文件体积见 [whisper.cpp](https://github.com/ggml-org/whisper.cpp)。SenseVoice 固定下载 [2024-07-17 模型仓库](https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/tree/main) 的 INT8 模型和词表；说明见 [sherpa-onnx 预训练模型](https://k2-fsa.github.io/sherpa/onnx/sense-voice/pretrained.html)。它不是后续的粤语微调版。下载先访问 Hugging Face，失败时尝试 hf-mirror.com。

设置页依次点击“下载模型”→“选用”→“保存 AI 与学习设置”。下载时显示当前文件的百分比、已下载/总 MiB、平均速度、来源和文件序号；来源未提供总大小时只显示已下载大小。SenseVoice 分别显示模型与词表的进度，百分比指当前文件。换源重试会重置该文件进度，切换页面后下载继续，底部持续显示进度；返回设置可查看完整面板（`Cmd/Ctrl+,` 打开设置）。只有主动取消或退出客户端才中止；取消可中断网络等待，只有校验并落盘后才报告完成。选用只将模型 ID 和 `local` 引擎填入表单，保存后才生效。“文件已下载”只检查文件存在，SenseVoice 必须同时有模型和词表；不表示推理运行库已安装或模型质量已验收。

两种模型都需要 FFmpeg 处理音轨。Whisper 使用 `clarora-asr`；未发现该辅助程序时回退到 `whisper-cli`。SenseVoice 必须使用带 sherpa-onnx 运行库的 `clarora-asr`，不能通过 `whisper-cli` 运行。当前原生辅助程序按 CPU 路径接入；下载模型不会自动安装这些工具。**已实际验证的是 macOS 的 Whisper tiny.en；其余五个模型只完成接入，尚未逐个实测，Windows 也未实机验证。**

原生转写辅助程序使用本仓库已有公开 C++ ASR 适配器，不依赖 React Native 运行时：

```sh
cmake -S clarora-gpui/native -B clarora-gpui/target/native -DCMAKE_BUILD_TYPE=Release -DCMAKE_PREFIX_PATH=/path/to/whisper-and-sherpa
cmake --build clarora-gpui/target/native --config Release
```

需 `whisper.h`、`libwhisper`、GGML 头文件/库、`sherpa-onnx/c-api/c-api.h`、`sherpa-onnx-c-api` 及相应运行库。将产出的 `clarora-asr`（Windows 为 `.exe`）和运行依赖放在程序旁的 `tools/` 或 PATH。动态 GGML 的 CPU 后端也需提供；辅助程序优先寻找自身旁的 CPU 后端，然后使用编译时找到的路径。可用 `CLARORA_GGML_CPU_BACKEND` 指定 CPU 后端文件。

## bilibili 分P选集与自动续播

粘贴 BV / av 号或普通视频完整链接，点击“读取分P / 选集”获取标题、时长和完整顺序，再点击“视频选集”中的任意一集下载并播放。也可点击“识别并导入”，默认导入 P1；已有 `?p=` 的完整链接会导入指定集。分P列表与播放器“下一P”、列表“上一P / 下一P”均不需要手写参数；`Cmd/Ctrl+Shift+←/→` 也可切集。列表超过五行可滚动。

“自动下一P”默认开启，可随时关闭。当前集真正播放结束后自动准备下一集：已下载则直接播放，未下载则先下载；最后一集播完停止。单集循环或 A/B 循环优先，此时不会自动换集。失败或取消停止在当前集，可手动重试，不会循环重试下载。所有后台操作仍共用任务槽，转写等任务结束后才准备下一P。分P列表、当前集和已下载路径保存在 GPUI 本机资料中；重启恢复，删除的文件会重新下载。这里只识别同一 BV/av 视频的分P，不将合集里的不同 BV 视频合并成队列。

读取分P使用 bilibili 的公开 `x/player/pagelist` 接口（BV/av参数），下载仍交给 yt-dlp，受视频可用性与来源限制。该接口用法也可见 [yt-dlp 的 Bilibili 提取器](https://github.com/yt-dlp/yt-dlp/blob/master/yt_dlp/extractor/bilibili.py)。模型转写仍按每个实际视频文件独立保存字幕。

## 视频转写与同步字幕

在“视频学习”打开已下载的 bilibili 视频或本地视频，在播放器右侧点击“生成视频字幕”（也可用 `Cmd/Ctrl+Shift+T`）。先在设置中下载并选择本地模型，或配置支持带时间轴转写的兼容接口。中文视频使用多语言 `base` / `small` 或 SenseVoice；`.en` 模型用于英语。

程序提取音轨后生成 SRT，按视频文件路径、大小和修改时间绑定保存。重开同一文件自动恢复列表；替换或移动视频后需要重新生成。当前句随真实播放时钟高亮，暂停保留当前句，跳转和倒退重新定位；句间空白不高亮。列表采用虚拟化，可关闭“跟随播放”手动浏览，点击某句定位视频。每句右侧“复制”可复制完整正文，顶部“复制全部”按顺序复制所有字幕文字（不含时间戳），复制不改变播放状态；“导出 SRT”可保存完整字幕文件。

转写需要时间，底部“取消任务”可取消。本地临时音轨在结束/失败时清理；失败或取消不会用空结果覆盖原字幕。远端接口有自身上传大小限制，超限会报告失败；本轮未实现远端长视频自动切块。字幕缓存是当前安装的本机资料，跨设备请显式导出 SRT，不将其视为已完成备份互通。

## 功能对照和验证边界

| 模块 | 已实现链路 | 仍需对照验收 |
| --- | --- | --- |
| 随便学学 | 多种卡片、到期/探索/收藏选取、下一条预加载 | 长时间连续播放、边界手势与焦点体验 |
| 闪卡 | 五类卡、SM-2、每日新卡限额、暂停恢复、编辑保留进度、文本/目录/Anki 导入 | 各导入来源的大资料集、编辑管理交互验收 |
| 听力/音频管理 | 练习组、批量导入删除、重命名、合并及字幕时间轴、真实波形和 A/B 拖动、逐词/Shift 选区、重点释义、循环、转写翻译、片段卡、上次音频/位置恢复 | 原版鼠标拖选、右键菜单等细节与真实界面回归 |
| 视频/bilibili | 下载导入取消、本地播放、主副字幕、外部字幕、倍速、循环、最近文件、片段导出 | Windows 真实下载播放；GPU 视频渲染路径 |
| 跟读/OCR/AI | 录音/选录音、转写对齐明细、AI 点评、图片识字、记录、材料会话/草稿、推理显示、流式错误与回退、聊天/OCR/远端转写请求取消；macOS Whisper tiny.en 已真实推理验证 | 真实麦克风权限、其余模型/服务、不同提供商的长对话验收 |
| PDF | PDFium、书库、直接图像绘制、实际缩放/水平滚动、目录、重启位置恢复、当前页文字作为 AI 材料 | 原版连续滚动布局、大文档与密码文档 |
| 词汇关系 | 同一公开词表、搜索、List 筛选、词义/词源等详情、关联词导航 | 布局与键盘/辅助功能体验 |
| 统计/冥想 | 累计量、热力图、听力/跟读趋势、计时暂停、按系统时间计算专注休息轮换、本地背景媒体 | 完整样式、长期计时及系统挂起恢复实机验收 |
| 存储 | 本机 ZIP、附件、事务合并、原版数据库只读迁移、S3/OSS 签名/分页/上传恢复 | 真实云桶互通与中断场景、原版双向全量备份兼容 |

当前视频采用 libmpv 软件解码/软件渲染，960×540 帧传递给 GPUI。该路径与原版原生播放器的 GPU 路径不同，不能用视频帧率或 CPU 占用直接归因于 GPUI 和 React Native 框架差异。整个客户端仍使用 GPUI 原生界面。

已做 macOS 编译和实际界面操作；Windows CI 工作流已加入，但未在本机或远端验证通过。原生界面可访问性支持有限。未完成独立 review、正式签名公证和可分发安装包，不作为发布验收结论。

## 可复现验证

```sh
cargo fmt --manifest-path clarora-gpui/Cargo.toml --check
cargo check --manifest-path clarora-gpui/Cargo.toml --locked --all-targets
PDFIUM_LIB_PATH=/absolute/path/containing/pdfium cargo run --manifest-path clarora-gpui/Cargo.toml --locked --bin e2e -- /absolute/path/to/evidence
```

E2E 使用独立临时资料，经过真实 SQLite、文件、ZIP、FFmpeg、libmpv、PDFium 和本机 HTTP 服务，输出 `e2e-report.json`、播放帧与 PDF 图片。libmpv 使用 null 音频输出，供 CI 验证解码和时钟，不验证系统声卡。HTTP 用固定本机响应验证协议，不代表外部 AI 或云桶已通过。未设置 `PDFIUM_LIB_PATH` 时 PDF 检查会明确跳过。

2026-10-17 本机执行含真实 Whisper 的 17 条链路通过。HTTP 夹具额外覆盖中文跨块、CRLF/多行 SSE、推理字段、405 回退、401 不重试、缺少结束帧、错误帧、截断和响应等待取消；聊天、OCR、远端转写的等待取消测得约 171–185 ms。波形结果保存在 `waveform.json`。另经 macOS 界面验证三页 PDF 导入/翻页/重启、实际缩放和恢复，以及本机模拟接口的连续问答、材料隔离、草稿切换恢复和重启续聊；第三轮请求携带前两轮历史。PDF 页面改为后台解码后直接绘制，修复翻页/重启只恢复页码但内容空白的问题。这不是完整桌面 GUI、Windows 或独立 review 的通过结论。

可额外设置 `CLARORA_ASR_MODEL` 为已下载的 `ggml-tiny.en.bin`，`CLARORA_ASR_SPEECH` 为包含 “Hello … learning” 的英文语音文件，并将编译好的 `clarora-asr` 放入 PATH；E2E 会执行真实本地转写，输出 `offline-transcription.srt`。macOS 可用 `say -v Samantha -o speech.aiff 'Hello world. This is a simple English reading exercise. Keep learning every day.'` 生成相同语音夹具。

依赖入口：[GPUI](https://crates.io/crates/gpui)、[gpui-component](https://github.com/longbridge/gpui-component)、[libmpv](https://mpv.io/manual/stable/)、[yt-dlp](https://github.com/yt-dlp/yt-dlp)、[PDFium binaries](https://github.com/bblanchon/pdfium-binaries)。分发第三方运行库时需附带对应许可证。
