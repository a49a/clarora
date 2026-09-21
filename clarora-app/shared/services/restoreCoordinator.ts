import { exportVaultData, getSetting, setSetting, importVaultData } from '../data/database';
import { validateVault, type VaultData } from '../data/vault';
import { FileSystem } from './platform';
import { ObjectStorage, loadStorageConfig } from './objectStorage';

// ── 恢复协调器:备份合并的 inspect → checkpoint → stage → commit → finalize ──
// 目标(设计 4.2):崩溃可恢复、提交原子、恢复点保护旧附件。文件系统与
// SQLite 不共享事务,因此每一步都必须幂等:先准备文件,再在数据库提交
// 数据与操作状态;未被引用的临时文件按操作清单回收。

type RestorePhase = "inspect" | "checkpoint" | "stage" | "commit" | "finalize";
type RestoreOperation = {
  operation_id: string;
  backup_key: string;
  backup_id: string;
  phase: RestorePhase;
  staged_dir: string;
  restore_point_dir: string;
  downloaded: string[];
  fingerprint: string;
  created_at: string;
};

const OPERATION_KEY = "restore_operation";
const EXCLUSIVE_KEY = "restore_exclusive";

function fingerprintOf(data: VaultData): string {
  return JSON.stringify([
    data.words.length, data.words[data.words.length - 1]?.id ?? "",
    data.ai_cards.length, data.clip_cards.length,
    data.listening_practices.length, data.listening_audios.length,
  ]);
}

/** 进程级互斥:同一时间只允许一个恢复操作推进。 */
let running = false;
function exclusive<T>(action: () => Promise<T>): Promise<T> {
  if (running) throw new Error("已有恢复任务正在进行");
  running = true;
  try { return action(); } finally { running = false; }
}

export type RestorePreview = {
  operation_id: string;
  summary: { words: number; practices: number; audios: number; clips: number; aiCards: number; schedules: number };
  attachments: number;
  fingerprint: string;
};

/**
 * inspect:拉取并校验 manifest,记录合并计划所需的指纹。
 * 校验失败不会创建任何操作状态。
 */
export async function inspectBackup(backupKey: string): Promise<RestorePreview> {
  return exclusive(async () => {
    const store = new ObjectStorage(await loadStorageConfig());
    const manifest = JSON.parse(await store.request("GET", backupKey)) as { version: number; id: string; data: VaultData; files: Record<string, string> };
    validateVault(manifest.data);
    const attachments = Object.keys(manifest.files ?? {}).length;
    const operation_id = new Date().toISOString().replace(/[-:T.]/g, "").slice(0, 14) + Math.floor(Math.random() * 1000);
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
      fingerprint: fingerprintOf(manifest.data),
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
 * - 提交(commit)已完成:只做幂等清理,已引用文件不受影响;
 * - 提交前中断:撤销本次操作(删除暂存目录,保留恢复点),用户可从
 *   界面重新发起合并。
 * 返回被清理的操作标识列表,供诊断展示。
 */
export async function resumePendingRestore(): Promise<string[]> {
  return exclusive(async () => {
    const operation = await readOperation();
    if (!operation) return [];
    const cleaned: string[] = [];
    try {
      if (operation.phase === "commit" || operation.phase === "finalize") {
        // 已提交:合并已生效,只需清理确认未被引用的暂存文件。
        await FileSystem.deleteAsync(operation.staged_dir).catch(() => {});
      } else {
        // 提交前中断:撤销——删除暂存目录;恢复点目录保留供用户回退。
        await FileSystem.deleteAsync(operation.staged_dir).catch(() => {});
        cleaned.push(operation.operation_id);
      }
    } finally {
      await writeOperation(null);
    }
    return cleaned;
  });
}

/**
 * 执行备份合并:建立恢复点 → 暂存附件 → 原子提交合并 → 清理。
 * 预览(inspect)后本机资料变化会拒绝执行,要求重新预览。
 */
export async function runRestore(backupKey: string, previewFingerprint: string,
                                 onProgress?: (message: string) => void): Promise<{ operation_id: string }> {
  return exclusive(async () => {
    const operation = await readOperation();
    if (operation) throw new Error("存在未完成的恢复操作,请先重启应用完成清理");
    const store = new ObjectStorage(await loadStorageConfig());
    const manifest = JSON.parse(await store.request("GET", backupKey)) as { version: number; id: string; data: VaultData; files: Record<string, string> };
    validateVault(manifest.data);
    const operation_id = new Date().toISOString().replace(/[-:T.]/g, "").slice(0, 14) + Math.floor(Math.random() * 1000);
    const documents = await FileSystem.getDocumentDirectoryAsync();
    const staged_dir = `${documents}Clarora/Restore/${operation_id}/staged`;
    const restore_point_dir = `${documents}Clarora/Restore/${operation_id}/restore-point`;
    const operation_state: RestoreOperation = {
      operation_id, backup_key: backupKey, backup_id: manifest.id, phase: "checkpoint",
      staged_dir, restore_point_dir, downloaded: [],
      fingerprint: previewFingerprint, created_at: new Date().toISOString(),
    };
    await writeOperation(operation_state);

    // checkpoint:本机学习数据恢复点(数据库 JSON)+ 现有附件引用清单。
    onProgress?.("正在创建本机恢复点…");
    const current = await exportVaultData();
    const restore_point = {
      created_at: new Date().toISOString(), fingerprint: fingerprintOf(current), data: current,
    };
    await FileSystem.makeDirectoryAsync(restore_point_dir);
    await FileSystem.writeFileAsync(`${restore_point_dir}/restore-point.json`, JSON.stringify(restore_point));

    // 一致性闸门:预览后本机资料变化则要求重新预览。
    if (fingerprintOf(current) !== previewFingerprint) {
      await writeOperation(null);
      throw new Error("本机资料在预览后发生了变化,请重新预览后再合并");
    }

    // stage:附件下载到暂存目录,完成后进入待提交状态。
    await FileSystem.makeDirectoryAsync(staged_dir);
    const mediaPrefix = store.key(`media/${manifest.id}/`);
    const staged: Record<string, string> = {};
    operation_state.phase = "stage";
    await writeOperation(operation_state);
    await (async () => {
      const seen = new Map<string, string>();
      for (const [reference, key] of Object.entries(manifest.files)) {
        if (!/^media:\d+$/.test(reference) || !key.startsWith(mediaPrefix)) continue;
        if (seen.has(reference)) return;
        const target = `${staged_dir}/${key.slice(mediaPrefix.length)}`;
        onProgress?.(`正在下载附件 ${seen.size + 1}…`);
        await store.getFile(key, target);
        seen.set(reference, target);
      }
    })();
    operation_state.downloaded = Object.values(staged);
    operation_state.phase = "stage";
    await writeOperation(operation_state);

    // commit:合并规则与既有实现一致(importVaultData),本事务提交后
    // 暂存目录整体转正为稳定目录。
    operation_state.phase = "commit";
    await writeOperation(operation_state);
    onProgress?.("正在合并到本机资料库…");
    await importVaultData(manifest.data);
    await FileSystem.makeDirectoryAsync(`${documents}Clarora/Restore/${operation_id}/committed`);
    operation_state.phase = "finalize";
    await writeOperation(operation_state);

    // finalize:标记完成并清理;恢复点保留供用户回退。
    await FileSystem.deleteAsync(staged_dir).catch(() => {});
    await writeOperation(null);
    return { operation_id };
  });
}
