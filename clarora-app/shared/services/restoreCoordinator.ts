import { exportVaultData, getSetting, setSetting, importVaultData } from '../data/database';
import { validateVault, type VaultData } from '../data/vault';
import { FileSystem } from './platform';
import { mapVaultMedia } from './librarySync';
import { ObjectStorage, loadStorageConfig } from './objectStorage';

// ── 恢复协调器:备份合并的 inspect → checkpoint → stage → commit → finalize ──
// 目标(设计 4.2):崩溃可恢复、提交原子、恢复点保护旧资料。文件系统与
// SQLite 不共享事务,因此每一步都必须幂等:先准备文件,再在数据库提交
// 数据与操作状态;未被提交引用的暂存文件按操作清单回收。
//
// 恢复点数据(合并前的完整 VaultData)与操作状态一样持久化在
// app_settings 中:用户确认合并满意后清除,或一键回退到合并前状态。

type RestorePhase = "inspect" | "checkpoint" | "stage" | "commit" | "finalize";
type RestoreOperation = {
  operation_id: string;
  backup_key: string;
  backup_id: string;
  phase: RestorePhase;
  attachments_dir: string;
  fingerprint: string;        // 预览时本机资料指纹(闸门:本机未变)
  backup_fingerprint: string; // 预览时备份指纹(闸门:备份未变)
  created_at: string;
};

const OPERATION_KEY = "restore_operation";
const RESTORE_POINT_KEY = "restore_point_data";

function fingerprintOf(data: VaultData): string {
  // 全量 JSON 摘要:任何字段的任何修改都会改变指纹。
  return JSON.stringify(data);
}

/** 进程级互斥:同一时间只允许一个恢复操作推进。必须等待整个任务完成,
 * 否则并发调用会在前一个操作完成前解锁(复审问题 7)。 */
let running = false;
async function exclusive<T>(action: () => Promise<T>): Promise<T> {
  if (running) throw new Error("已有恢复任务正在进行");
  running = true;
  try { return await action(); } finally { running = false; }
}

export type RestorePreview = {
  operation_id: string;
  summary: { words: number; practices: number; audios: number; clips: number; aiCards: number; schedules: number };
  attachments: number;
  /** 预览时本机资料指纹:提交前校验本机未变。 */
  local_fingerprint: string;
  /** 预览时备份内容指纹:提交前校验备份未变。 */
  backup_fingerprint: string;
  /** 展示用快照字段。 */
  words: number;
  aiCards: number;
};

/**
 * inspect:拉取并校验 manifest,记录本机与备份两侧指纹。
 * 校验失败不会创建任何操作状态。
 */
export async function inspectBackup(backupKey: string): Promise<RestorePreview> {
  return exclusive(async () => {
    const store = new ObjectStorage(await loadStorageConfig());
    const manifest = JSON.parse(await store.request("GET", backupKey)) as { version: number; id: string; data: VaultData; files: Record<string, string> };
    validateVault(manifest.data);
    const attachments = Object.keys(manifest.files ?? {}).length;
    const operation_id = new Date().toISOString().replace(/[-:T.]/g, "").slice(0, 14) + Math.floor(Math.random() * 1000);
    const local = await exportVaultData();
    return {
      operation_id,
      summary: {
        words: manifest.data.words.length,
        practices: manifest.data.listening_practices.length,
        audios: manifest.data.listening_audios.length,
        clips: manifest.data.clip_cards.length,
        aiCards: manifest.data.ai_cards.length,
        schedules: manifest.data.review_schedule.length,
      },
      attachments,
      local_fingerprint: fingerprintOf(local),
      backup_fingerprint: JSON.stringify(manifest),
      words: manifest.data.words.length,
      aiCards: manifest.data.ai_cards.length,
    };
  });
}

async function readOperation(): Promise<RestoreOperation | null> {
  const raw = await getSetting(OPERATION_KEY);
  return raw ? (JSON.parse(raw) as RestoreOperation) : null;
}

async function writeOperation(operation: RestoreOperation | null): Promise<void> {
  await setSetting(OPERATION_KEY, operation ? JSON.stringify(operation) : "");
}

/**
 * 启动时调用:处理未完成的恢复操作。
 * - 提交(commit)已完成:合并已生效,附件目录保留,只清除操作状态;
 * - 提交前中断:撤销——删除附件目录(本操作产物),本机资料未动,
 *   用户可重新发起合并。
 * 返回被撤销清理的操作标识列表,供诊断展示。
 */
export async function resumePendingRestore(): Promise<string[]> {
  return exclusive(async () => {
    const operation = await readOperation();
    if (!operation) return [];
    const cleaned: string[] = [];
    try {
      if (operation.phase === "commit" || operation.phase === "finalize") {
        // 已提交:合并已生效,只清除操作状态;附件目录保留。
      } else {
        // 提交前中断:撤销——删除本操作的附件目录。
        await FileSystem.deleteAsync(operation.attachments_dir).catch(() => {});
        cleaned.push(operation.operation_id);
      }
    } finally {
      await writeOperation(null);
    }
    return cleaned;
  });
}

/**
 * 执行备份合并:预览指纹闸门 → 恢复点 → 附件下载并改写引用 → 提交合并。
 * 预览后本机资料或备份内容发生变化都会拒绝执行,要求重新预览。
 */
export async function runRestore(backupKey: string, preview: RestorePreview,
                                 onProgress?: (message: string) => void): Promise<{ operation_id: string }> {
  return exclusive(async () => {
    const operation = await readOperation();
    if (operation) throw new Error("存在未完成的恢复操作,请先重启应用完成清理");
    const store = new ObjectStorage(await loadStorageConfig());
    const manifest = JSON.parse(await store.request("GET", backupKey)) as { version: number; id: string; data: VaultData; files: Record<string, string> };
    validateVault(manifest.data);
    const operation_id = new Date().toISOString().replace(/[-:T.]/g, "").slice(0, 14) + Math.floor(Math.random() * 1000);
    const documents = await FileSystem.getDocumentDirectoryAsync();
    const attachments_dir = `${documents}Clarora/Restore/${operation_id}/attachments`;
    const operation_state: RestoreOperation = {
      operation_id, backup_key: backupKey, backup_id: manifest.id, phase: "checkpoint",
      attachments_dir,
      fingerprint: preview.local_fingerprint, backup_fingerprint: preview.backup_fingerprint,
      created_at: new Date().toISOString(),
    };
    await writeOperation(operation_state);

    // checkpoint:本机学习数据恢复点(合并前的完整 VaultData 存入
    // app_settings,用户可通过「从恢复点回退」一键还原)。
    onProgress?.("正在创建本机恢复点…");
    const current = await exportVaultData();
    await setSetting(RESTORE_POINT_KEY, JSON.stringify(current));

    // 指纹闸门:本机资料必须与预览时一致(预览后本机编辑则要求重新预览);
    // 备份内容也必须与预览时一致(备份被覆盖发布则要求重新选择版本)。
    if (fingerprintOf(current) !== preview.local_fingerprint) {
      await writeOperation(null);
      throw new Error("本机资料在预览后发生了变化,请重新预览后再合并");
    }
    if (JSON.stringify(manifest) !== preview.backup_fingerprint) {
      await writeOperation(null);
      throw new Error("备份内容与预览时不一致,请重新选择版本并预览");
    }

    // stage:附件下载到本操作目录,并把数据中的 media: 引用改写为本地
    // 稳定路径;改写后的数据才会进入合并。
    await FileSystem.makeDirectoryAsync(attachments_dir);
    const local = new Map<string, string>();
    operation_state.phase = "stage";
    await writeOperation(operation_state);
    await mapVaultMedia(manifest.data, async reference => {
      if (!/^media:\d+$/.test(reference) || !Object.hasOwnProperty.call(manifest.files, reference)) throw new Error("备份缺少附件");
      if (local.has(reference)) return local.get(reference)!;
      const key = manifest.files[reference];
      // 只取文件名部分(不含目录前缀),校验安全后再拼接本地路径。
      const filename = key.split("/").pop() ?? "";
      if (!/^\d+\.[A-Za-z0-9]{1,8}$/.test(filename)) throw new Error(`附件文件名非法:${filename}`);
      const target = `${attachments_dir}/${filename}`;
      onProgress?.(`正在下载附件 ${local.size + 1}/${Object.keys(manifest.files).length}…`);
      await store.getFile(key, target);
      local.set(reference, target);
      return target;
    });

    // commit:引用已指向本地文件,合并提交;提交成功后本目录即稳定附件
    // 目录,绝不可删除。
    operation_state.phase = "commit";
    await writeOperation(operation_state);
    onProgress?.("正在合并到本机资料库…");
    await importVaultData(manifest.data);
    operation_state.phase = "finalize";
    await writeOperation(operation_state);

    // finalize:清除操作状态;附件目录(已引用)保留供回退。
    await writeOperation(null);
    return { operation_id };
  });
}

/** 读取恢复点是否存在(供 UI 判断是否显示「从恢复点回退」按钮)。 */
export async function hasRestorePoint(): Promise<boolean> {
  return !!(await getSetting(RESTORE_POINT_KEY));
}

/** 从恢复点回退:将合并前的学习数据写回数据库,清除恢复点。 */
export async function restoreFromCheckpoint(): Promise<void> {
  const raw = await getSetting(RESTORE_POINT_KEY);
  if (!raw) throw new Error("没有可用的恢复点");
  const data: VaultData = JSON.parse(raw);
  await importVaultData(data);
  await setSetting(RESTORE_POINT_KEY, "");
}
