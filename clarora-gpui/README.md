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

桌面界面采用暖灰与青绿色主题，导航按学习空间、资料库和个人工具分组。页面左上角可收起/展开左侧导航，也可按 `Cmd/Ctrl+B`；导航从左侧滑动展开/收起，内容区同步扩缩；按钮用加粗双箭头指示方向，并记住状态，重启后保留。侧栏底部可切换浅色/深色，也可按 `Cmd/Ctrl+Shift+L`。闪卡页默认聚焦学习卡片，点击“添加句子卡”展开编辑区；`Cmd/Ctrl+Shift+N` 可直接打开句子编辑并定位输入框。视频控制栏“字幕：开／关”控制画面字幕（含第二轨），普通／全屏共用并记住偏好；字幕列表保留供定位、复制和翻译。字幕页“导入英文字幕”加载 SRT／VTT，“英文生成中文”调用共用聊天 API 分批生成双语字幕，保留英文和时间轴，全部成功后才保存，失败／取消保留原缓存；已有中文不重复发送给模型。视频页采用左右工作台：左侧播放器，右侧“字幕／选集／AI 提问”页签；拖动中间分隔条调整宽度，窄窗口自动上下排列。字幕按正文长度展开，支持点击播放、逐句复制及提问；选集独立滚动，不再占用播放器下方空间。点击标题栏的 P 数可切到选集。字幕叠加在画面底部，可从控制栏“设置”菜单选择 14–32px 字号（默认 16px），立即生效并自动保存，普通与全屏播放共用；控制栏提供播放、前后 10 秒、进度拖动、倍速菜单、音量滑杆/静音；“设置”菜单集中管理字幕字号、循环方式和精听工具。选择“A–B 精听工具”展开起终点、清除、逐句切换、重听本句及片段闪卡；设置有效 A/B 后自动循环，进度条显示范围和金色边界，切换字幕句时退出旧循环。B 必须晚于 A；重设 A 超过旧 B 会清除旧 B。标题栏“更多”展开本地视频、bilibili 导入、字幕轨道、SRT 导出和最近视频；首次打开没有视频时自动显示。控制栏的“全屏”、双击视频画面或 `F11` 可进入视频专用原生全屏，`Esc` 或“退出全屏”返回；播放继续，退出后保留原导航/助手状态。

## 数据和使用

- macOS 资料目录：`~/Library/Application Support/Clarora-GPUI/`；Windows：`%LOCALAPPDATA%\Clarora-GPUI\`。使用独立 SQLite 和附件目录，可与原版同时运行。
- 可通过 `--data-dir /absolute/path` 指定测试资料目录。不会自动打开或修改原版数据库。
- 设置页可只读导入原版 SQLite 快照、导入/导出 v2 ZIP 资料备份，或配置 S3/OSS。密钥保存于系统凭证库，不写入资料备份；需在新版重新配置。
- 视频页支持本地视频、bilibili BV/av/完整地址及 `?p=` 分 P、取消下载、字幕主副轨、外部字幕、A/B、倍速和片段卡。需本机媒体工具，网络和视频可用性由视频来源决定。
- 视频字幕可直接提问：生成字幕或加载 UTF-8 SRT / VTT 后，点“问当前句”“问整段字幕”，或某条字幕的“问 AI”，在右侧输入问题并发送。引用包含时间戳；当前句和指定行会暂停播放，并固定本次引用。不同视频、分P、字幕内容和引用范围各自保留会话，回答可存为问答卡。整段字幕超过 10,000 句或 120,000 字符时提示改用单句，不静默截断。内嵌字幕未解析为完整时间轴时只支持播放器当前句。使用设置中的共用 API；本地 Whisper / SenseVoice 仅负责转写，不负责问答。
- AI 设置提供一组共用 Base URL、模型 ID 和 API Key，供聊天与远端时间轴转写使用，保存后统一生效；模型需同时支持聊天与带时间轴转写接口。旧独立配置保存前保持原行为，旧版仅配置转写时表单沿用转写整组。视觉模型可单独选填，本地转写仍使用独立本地模型。独立侧栏按固定材料保留最多 12 个聊天会话和草稿，推理内容可展开。聊天流不受支持（HTTP 404/405）时回退一次非流式接口；断流、截断和模型错误不会保存为成功回答。
- AI 密钥按需读取：启动、播放和本地 ASR 仅读取非敏感配置，远端 AI 请求才访问系统凭证库；成功读取的密钥仅在当前进程内缓存。设置页不回填密钥，API Key 留空保存会保留原密钥，输入新值才替换。macOS 临时签名开发包重新构建后，首次访问已有凭证仍可能要求系统授权；正式固定签名身份的授权验收尚未完成。
- 音频学习对齐 RN 的阅读布局：顶部下拉选择练习组／音频并切换上一条、下一条，中间字幕独立滚动，底部集中波形、进度、播放、倍速和循环。每句右侧“复制”复制该句完整原文与译文，顶部“复制全部”按顺序复制全文（不含时间戳），不改变播放；盲听仅已翻开的句子显示复制按钮，且“仅中文”只复制中文。普通点击单词定位并播放；开启“截取片段”后，点击单词设置 A/B，Shift 点击扩展，拖动波形边界调整，按需编辑及保存闪卡。关闭截取会退出 A/B 循环。“字幕”菜单提供导入、生成、翻译、字号及当前句开关；“跟随”可关闭自动滚动。AI 标重点用下划线显示，悬停查看释义。词语时间按字幕字符长度估算，不代表声学逐词对齐；尚未对齐 RN 的原生自由拖选和右键菜单。
- 音频学习顶部可切换“字幕／盲听”。盲听右键点击任意字幕行（包括词块）可翻看该句，再次右键收起；每句右侧也有“查看／收起”，可暂停并翻看指定句，遵循“仅中文／中英字幕”设置，不改变播放位置；继续后重新遮住。盲听原文按词显示遮挡块，点击词块会收起答案并从对应单词位置播放，点击时间戳或行空白从句首播放；词时间沿用普通字幕的字符偏移估算，并非声学对齐。若句首在 A/B 范围外则退出旧循环。盲听保留原字幕列表的时间戳、当前行高亮和跟随滚动，以遮挡条替代原文与译文，不挂载被隐藏的文字；隐藏重点和片段编辑，可回退 5 秒；“看刚才一句”先暂停并确认，再在原列表位置显示当前位置覆盖的单句字幕，句间空白取最近结束的一句，超过 10 秒提示“上一条字幕”。首句前或没有有效字幕时不揭晓，无字幕仍可播放。收起文字保持暂停，“继续盲听”自动隐藏并播放；播完可从头再听或查看全文（切回字幕模式并保持暂停）。播放、定位、切音频及字幕重载会清除揭晓；离开学习页、重启后回到字幕模式。AI 隐藏态只允许自由提问，揭晓态只引用当前已揭晓句，与普通字幕会话隔离。盲听操作栏“查看”可选“中英字幕／仅中文”，偏好自动保存。平时两种语言都遮挡；选“仅中文”后，点击“看中文”只显示刚才一句已有的中文，英文始终遮挡，继续播放后中文也重新遮住。无中文显示提示，中英混排行中的非汉字字母同样遮住，不回退到英文；AI 自动引用仅限已揭晓的中文。也可从 macOS／Windows 原生“听力”菜单操作。新增“从句首重听”、上一句／下一句，核对后重听会先隐藏答案；连续播放方式不变，完整 A/B 循环中仅跳转到范围内的完整字幕句。播放结束也能核对最后一句。页面聚焦时可按 `R` 从句首重听、`↑/↓` 前后换句、`H` 查看／收起文字、`C` 隐藏并继续，`Space` 播放／暂停；编辑输入框时不触发这些单键快捷操作。
- 口语分数表示识别文本对齐，包含漏读、读错、多读、完整度与语速。
- 本地转写支持 Whisper 模型和 SenseVoice。模型由设置页下载。点击“选用”后，模型行高亮并显示“已选择 · 待保存”；点击“保存 AI 与学习设置”成功后显示“已选用”和“当前生效”，重启后保留。Whisper 可使用 PATH 中的 `whisper-cli`；SenseVoice 需要下述原生辅助程序。
- `Cmd/Ctrl+1…9` 切换前九个页面；`Cmd/Ctrl+O` 按当前页面打开/导入文件，`Cmd/Ctrl+J` 切换 AI 侧栏，`Cmd/Ctrl+Enter` 发送问题，`Cmd/Ctrl+N` 新会话，`Cmd/Ctrl+←/→` 上一项/下一项（PDF 为翻页）。页面获得焦点后空格翻面/播放，方向键切卡/快进，`1/2/3` 评分。输入框编辑时不拦截无修饰键的输入。

## 音频管理工作台

“音频管理”采用练习组、紧凑音频列表和所选详情的布局；大窗口显示练习组侧栏，普通窗口用可搜索的组菜单，窄窗口堆叠内容。练习组显示音频数量并记住所选组。选择列表行只查看详情，点击“开始学习”才播放；“查看 / 管理字幕”打开学习页并暂停，可在“字幕”菜单生成、导入和翻译。详情预览前五句字幕，并显示音频文件位置与缺失提示。

列表支持组内名称搜索、全部/附字幕/待配字幕筛选、最近/最早添加及名称排序。“附字幕”指字幕附件文件存在，详情进一步解析实际内容。导入音频与文字转音频位于顶部；TTS 编辑区按需打开，保留当前会话中的输入，不会长期挡住列表。新建组与重命名使用独立表单，重命名预填所选名称；同组同名音频与同名练习组会提示换名。

“多选”可全选当前搜索筛选结果、移动到其他组、合并或移除记录；切换练习组清空勾选，移动遇到同名冲突时整批回滚，字幕与媒体引用随音频保留。移除前显示记录名称并确认，只删除库记录，附件文件保留。后台任务执行时禁用修改操作；切组不改变已经启动的导入/TTS任务目标，原生文件选择期间切组会要求重新导入。“更多导入”保留本机导入命令入口。

## 本地文字转音频：Kokoro ONNX

在“音频管理”选择或新建练习组，找到“文字转音频 · Kokoro ONNX”。首次点击“下载中英模型”（约 334 MiB），完成后输入文字，选择中英文音色和 0.5–2.0 倍语速，点击“生成并保存音频”。文字最多 10,000 字；名称留空会自动命名。“试听 / 开始学习”打开生成结果，生成的 WAV 也会保存在音频库，可重命名、导出备份。音色与语速选择重启后保留，正文不发送到远端，不需要 API Key。

使用固定的 [sherpa-onnx Kokoro multi-lang v1.0](https://k2-fsa.github.io/sherpa/onnx/tts/all/Chinese-English/kokoro-multi-lang-v1_0.html)。音色菜单提供常用美式、英式和中文音色；不是音色克隆。首次下载联网，SHA-256 校验通过后才解压安装；之后生成全程离线。模型保存在资料目录 `models/kokoro-multi-lang-v1_0/`，模型包内许可证随模型保留。生成耗时取决于本机 CPU 和文字长度，可通过底部“取消任务”停止；切换页面后任务继续，结果保存到启动生成时选择的组，不自动开始播放。生成失败或取消不保存半成品。生成时按句合成并按每句真实采样长度保存 SRT，进入音频学习即可显示原文、逐句定位；原文随字幕附件保存。句级时间来自实际合成片段，单词时间仍沿用已有字符插值，不代表声学逐词对齐。此前版本生成的音频未保存原文，需在“音频学习→字幕→生成原文字幕”使用 ASR 补字幕，或用原文重新生成。

Kokoro 需要独立原生程序 `clarora-tts` 和启用 TTS 的 sherpa-onnx C API 运行库。**下载模型不会安装运行库。** macOS 开发环境已安装 sherpa-onnx 时可执行：

```sh
cmake -S clarora-gpui/native/tts -B clarora-gpui/target/tts -DCMAKE_BUILD_TYPE=Release -DCMAKE_PREFIX_PATH=/path/to/sherpa-onnx
cmake --build clarora-gpui/target/tts --config Release
sh clarora-gpui/scripts/package-macos.sh
```

macOS CMake 构建会将对应 sherpa-onnx C API / ONNX Runtime 动态库复制到辅助程序旁，打包脚本将它们与 `target/tts/clarora-tts` 一起放入 `.app/Contents/MacOS/tools/`。必须使用启用 TTS 的运行库，文件名含 `no-tts` 的发行包无法生成语音；若本机默认运行库仅用于 ASR，可用 `-DSHERPA_LIBRARY=/absolute/tts-enabled/lib/libsherpa-onnx-c-api.dylib` 显式指定。请同时提供与运行库版本匹配的头文件；二进制再分发需一并保留相关许可证。Windows 使用同一 CMake 项目与 MSVC，构建后将 `target/tts/Release/clarora-tts.exe` 和对应运行库放到桌面程序旁的 `tools/`；Windows 实机验证尚未完成。直接 `cargo run` 时将辅助程序目录加入 PATH。

运行默认 E2E 会检查输入校验、缺失模型、预取消；设置 `CLARORA_KOKORO_MODEL=/absolute/model-dir` 后额外实际生成中英文 WAV 和逐句 SRT，检查非静音、原文覆盖、连续时间、SQLite 重开、附件备份恢复及 libmpv 逐句定位，验证生成中取消清理。可额外设置 `CLARORA_KOKORO_ARCHIVE=/absolute/kokoro-multi-lang-v1_0.tar.bz2` 验证同一安装链的 SHA-256 / 安全解压。证据目录保留 `kokoro-english.wav`、`kokoro-chinese.wav` 和 `kokoro-evidence.json`。未配置模型时真实推理明确标记跳过。

## 端侧语音模型：名称、语言和运行要求

这里的“端侧模型”指 **ASR（语音识别）模型**，用于音频转写、视频字幕和跟读文本识别。模型文件下载后，本地转写不上传音轨；聊天、图片识字和 AI 点评仍调用设置中的远端 API。`engine=local` 使用下面的本地模型，`engine=compatible` 使用设置中保存的共用远端模型，两者的模型 ID 不可混用。

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

读取视频标题与分P使用 bilibili 的公开 `x/web-interface/view` 接口（BV/av参数），下载仍交给 yt-dlp，受视频可用性与来源限制。相关接口用法也可见 [yt-dlp 的 Bilibili 提取器](https://github.com/yt-dlp/yt-dlp/blob/master/yt_dlp/extractor/bilibili.py)。模型转写仍按每个实际视频文件独立保存字幕。

## 视频收藏与切换

“视频学习”右侧默认打开“我的视频”，显示所有已导入视频、当前选中项、来源和文件夹；播放器标题栏也有“切换收藏”菜单。可以一次选择多个本地视频，导入即收藏；bilibili 按同一来源只收藏一条视频，分P仅在“选集”中切换，下载其他P不会新增收藏卡片；卡片显示整条视频标题、总P数、已下载数量和上次选中P，并提供“查看选集”入口。旧收藏启动后在后台补齐来源标题；离线时优先显示已保存标题或下载文件名称，标题不随分P切换。已有分P收藏自动合并，保留所选P、文件和字幕缓存。重复导入同一文件会去重，同名但位于不同路径的文件分别保留，重启恢复收藏与上次选择。旧“最近打开”列表首次自动迁移，收藏不再按最近12条淘汰。

点击视频卡片或“选择播放”切换播放，已选视频可从头重播；“字幕 / 选集 / AI 提问”仍在相邻页签。“移除收藏”会弹出带背景遮罩的确认框，显示视频名称和红色“确认移除”按钮；取消或按 Esc 返回。确认后只移除收藏记录，视频文件和字幕缓存保留。移除当前视频会停止播放并清空当前画面、字幕和循环，等待选择其他视频；移除其他项不打断当前播放。文件已移动或删除时保留提示，仍可移除收藏。后台任务期间暂停切换/移除，先等待完成或取消任务。

每条收藏保存自己的 bilibili 选集信息，切回时恢复对应队列；本地视频不会沿用其他视频的自动下一P。新收藏保存在本机资料的设置中；此功能不代表已经完成跨设备备份/同步验收。

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
