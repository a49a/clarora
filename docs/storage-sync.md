# 自有对象存储同步与备份

Clarora 是纯客户端应用。对象存储只保存备份 JSON 和附件，不执行应用逻辑；不需要 Clarora 账号、后端、数据库或 Redis。

## 配置

1. 自行创建一个私有 Bucket，并创建只能访问该 Bucket / 资料库前缀的专用凭证。
2. 在「设置 → 同步与备份」选择 S3 兼容存储或阿里云 OSS。
3. Endpoint 使用服务域名，不加 Bucket 或路径，例如 S3 的 `https://s3.ap-southeast-1.amazonaws.com`、OSS 的 `https://oss-cn-hangzhou.aliyuncs.com`。Region 与 Bucket 所在区域一致。
4. 填写 Bucket、Access Key ID、Secret Access Key、资料库前缀（例如 `clarora/personal`）。临时凭证另填 Token，到期后需自行更新。
5. S3 默认使用虚拟主机式地址；MinIO 或其他服务要求路径式地址时启用 Path-style。OSS 使用 OSS V4 原生签名，而非假设所有 OSS Endpoint 都兼容 S3 签名。
6. “保存并读取备份”检查列表读取权限。“备份本机资料”还需要对象写入权限，恢复需要对象读取权限。

客户端要求对象存储使用 HTTPS。凭证在 macOS / Windows 存于系统凭证保险库（钥匙串 / 凭据库），不随备份上传；移动端暂存本机应用 SQLite。备份没有端到端加密；存储商的服务端加密由用户在 Bucket 侧配置。

所需操作为 ListObjectsV2、GetObject、PutObject，不需要应用主动删除对象或管理 Bucket。S3 权限通常对应 `s3:ListBucket`、`s3:GetObject`、`s3:PutObject`；OSS 对应 `oss:ListObjects`、`oss:GetObject`、`oss:PutObject`。列表授权应限定资料库前缀，读写对象授权限定该前缀下的对象。

## 使用与冲突规则

- **备份本机资料**：生成一个新版本；先上传附件，再提交清单。失败不会把不完整备份加入版本列表。
- **合并到本机**：选择一个版本，先下载并验证附件引用，再用数据库事务合并。下载失败不修改本机数据库，数据库写入失败回滚。
- 每次备份使用独立路径，两台设备同时备份也不会互相覆盖清单。需要合并多个设备时，逐个选择它们的备份导入，再创建汇总备份。
- 合并流程：选择版本后先预览内容数量，确认后自动创建本机恢复点并下载校验附件，再提交合并；合并前中断可在下次启动时撤销清理，已提交的合并有权益流水可查，恢复点保留用于回退。
- 新内容加入本机；同名词卡或同 ID 已有内容保留本机版本。复习计划根据 `last_graded_at` 选择较晚记录，设备时间应准确。同日统计取较大值，避免重复恢复累加；不代表把多设备活动相加。
- 删除只作用于当前设备，不传播删除标记。重新合并旧版本可能重新带回已删除条目。
- 当前是手动全量版本备份，不是自动双向同步。每份备份内相同附件引用去重，版本之间会重新上传附件。
- 版本列表中的时间来自备份设备时钟。用户手动选择版本；客户端不会自动选一个版本覆盖本机。

## 数据范围

包含：单词、练习组、音频与字幕、独立音频片段卡、视频片段与文件、AI 问答卡、复习计划、收藏、每日学习统计、OCR 识别文本和跟读评分结果。

不包含：存储和 AI 密钥、设备设置、本机 shell 命令、主题偏好、聊天会话与草稿、最近打开但未收藏的视频、原始 OCR 图片、原始跟读录音、未引用的媒体文件。备份不是整个设备应用目录的镜像。


## 存储结构

```text
<prefix>/
  snapshots/<version-id>.json
  media/<version-id>/0.mp3
  media/<version-id>/1.srt
  media/<version-id>/2.mp4
```

清单 `version: 2`，包含版本 ID、创建时间、明确允许的学习表和附件引用映射；设备路径替换为 `media:N`，恢复后映射到设备私有 `Clarora/Restored/` 目录。数据格式定义在 `shared/data/vault.ts`，事务合并位于 `shared/data/database.ts`，不上传整个 SQLite 文件。

失败上传可能留下没有清单的附件目录，重复合并也可能留下不再引用的本地附件。当前不自动清理或删除历史版本。手动清理云端时，要保留每个保留版本清单引用的整个附件目录，不能只对 media/ 设置比 snapshots/ 更短的生命周期。

## 验证范围与协议参考

自动化覆盖 S3/OSS 签名构造、Unicode 路径、临时凭证、分页、上传失败、恢复失败、恶意路径、重复导入、跨设备单词 ID 映射和事务回滚。真实云服务、不同厂商的兼容性、原生文件上传与设备运行仍需使用专用测试 Bucket 实测。

- [AWS S3 SigV4](https://docs.aws.amazon.com/AmazonS3/latest/developerguide/sig-v4-header-based-auth.html)
- [OSS V4 Authorization](https://www.alibabacloud.com/help/en/oss/developer-reference/recommend-to-use-signature-version-4)
- [OpenAI 兼容转写格式参考](https://platform.openai.com/docs/api-reference/audio/createTranscription)
