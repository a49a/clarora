import { currentPlatform, getNativeModules } from './platform';

/**
 * 系统凭证保险库抽象：macOS 走钥匙串（RNMacKeychain），Windows 走
 * PasswordVault（RNWindowsCredentials），iOS 走钥匙串（RNIOSKeychain），
 * Android 走 Keystore 加密存储（RNAndroidKeychain）。移动端返回 null 表示
 * 原生模块尚未随包安装（旧安装包），调用方退回本机数据库存储。
 */
export type SecretStore = {
  setSecret: (name: string, value: string) => Promise<void>;
  getSecret: (name: string) => Promise<string | null>;
  deleteSecret: (name: string) => Promise<void>;
};

export const SECRET_NAMES = {
  aiApiKey: 'clarora.ai.api-key',
  aiAsrApiKey: 'clarora.ai.asr-api-key',
  storageSecretAccessKey: 'clarora.storage.secret-access-key',
  storageSessionToken: 'clarora.storage.session-token',
} as const;

/**
 * 密钥代数命名:0 代保持原名(兼容既有保险库条目),更代的密钥带 .v<N>
 * 后缀。保存先暂存新一代,再由数据库行一次性切换端点与 secretGen 引用;
 * 未被引用的代只是暂存数据,任何时刻都不会与端点错代搭配。
 */
export function versionedSecretName(base: string, generation: number): string {
  return generation > 0 ? `${base}.v${generation}` : base;
}

/** 读取配置行中的密钥代数;缺失或非法按 0 代(原名)处理。 */
export function secretGeneration(config: { secretGen?: number } | null | undefined): number {
  return typeof config?.secretGen === 'number' && Number.isInteger(config.secretGen) && config.secretGen > 0
    ? config.secretGen
    : 0;
}

/** 尽力删除一代密钥(切换后旧代不再被引用;暂存失败时清掉半成品)。
 * 清理失败不影响正确性,调用方可安全忽略。 */
export async function discardGeneration(store: SecretStore, bases: readonly string[], generation: number): Promise<void> {
  for (const base of bases) {
    try { await store.deleteSecret(versionedSecretName(base, generation)); } catch { /* 尽力清理 */ }
  }
}

type NativeSecretStore = {
  setSecret: (name: string, value: string) => Promise<void>;
  getSecret: (name: string) => Promise<string | null>;
  deleteSecret: (name: string) => Promise<void>;
};

// Keep reads serialized and cached for this JS session. In particular, once
// access is denied, queued reads for other accounts must not open more dialogs.
let macStore: SecretStore | undefined;
function sessionStore(native: NativeSecretStore): SecretStore {
  const values = new Map<string, string | null>();
  let readFailure: { error: unknown } | undefined;
  let pending: Promise<unknown> = Promise.resolve();
  function enqueue<T>(action: () => Promise<T>): Promise<T> {
    const result = pending.then(action);
    pending = result.catch(() => {});
    return result;
  }
  return {
    getSecret: name => enqueue(async () => {
      if (values.has(name)) return values.get(name)!;
      if (readFailure) throw readFailure.error;
      try {
        const value = await native.getSecret(name);
        values.set(name, value);
        return value;
      } catch (error) {
        readFailure = { error };
        throw error;
      }
    }),
    setSecret: (name, value) => enqueue(async () => {
      await native.setSecret(name, value);
      values.set(name, value);
      // 写入成功说明授权已授予（如用户在授权框点了"始终允许"），
      // 解除读取熔断，让后续读取重试。
      readFailure = undefined;
    }),
    deleteSecret: name => enqueue(async () => {
      await native.deleteSecret(name);
      values.set(name, null);
      readFailure = undefined;
    }),
  };
}

/** RN 注入的 __DEV__;node 测试环境没有该全局,视为非 DEV。 */
export function isDevBuild(): boolean {
  return typeof __DEV__ !== 'undefined' && __DEV__ === true;
}

export function secretStore(): SecretStore | null {
  if (currentPlatform === 'macos') {
    const module = getNativeModules().RNMacKeychain as NativeSecretStore | undefined;
    if (!module?.setSecret) return null;
    return macStore ??= sessionStore(module);
  }
  if (currentPlatform === 'windows') {
    const module = getNativeModules().RNWindowsCredentials as NativeSecretStore | undefined;
    if (!module?.setSecret) return null;
    return module;
  }
  if (currentPlatform === 'ios') {
    const module = getNativeModules().RNIOSKeychain as NativeSecretStore | undefined;
    if (!module?.setSecret) return null;
    return macStore ??= sessionStore(module);
  }
  if (currentPlatform === 'android') {
    const module = getNativeModules().RNAndroidKeychain as NativeSecretStore | undefined;
    if (!module?.setSecret) return null;
    return macStore ??= sessionStore(module);
  }
  return null;
}

/**
 * 成组写入凭证(value 为 null 表示删除该密钥):任一写入失败时,把已写入
 * 的恢复为原值,避免"一半新一半旧"的密钥组与数据库旧配置搭配使用。
 * 恢复动作失败不掩盖原始错误,由调用方统一抛出并提示重试。
 */
export async function setSecretsAtomic(
  store: SecretStore,
  entries: ReadonlyArray<{ name: string; value: string | null }>,
): Promise<void> {
  const previous = new Map<string, string | null>();
  const written: string[] = [];
  try {
    for (const { name, value } of entries) {
      // 预读失败(如读取被拒)按 null 处理:回滚改为删除该名。
      // 当前调用方只暂存全新代名,删除即为正确的回滚,且不阻塞保存。
      previous.set(name, await store.getSecret(name).catch(() => null));
      if (value == null) await store.deleteSecret(name);
      else await store.setSecret(name, value);
      written.push(name);
    }
  } catch (error) {
    let restored = true;
    for (const name of written) {
      const old = previous.get(name);
      try {
        if (old == null) await store.deleteSecret(name);
        else await store.setSecret(name, old);
      } catch { restored = false; }
    }
    // 如实标记:调用方不得在恢复未完成时声称"已恢复"。
    // 不用 instanceof 判定:错误可能来自其他 vm域,构造器不同。
    if (!restored && error && typeof error === 'object') {
      (error as { rollbackIncomplete?: boolean }).rollbackIncomplete = true;
    }
    throw error;
  }
}

/**
 * 读取密钥并完成一次性迁移：保险库里没有、而旧配置（本机数据库）里有
 * 明文时，写入保险库并返回明文；调用方负责把数据库中的明文抹掉。
 * 保险库已有值时直接返回（覆盖任何数据库残留）。
 */
export async function migrateSecret(
  store: SecretStore,
  name: string,
  plaintext: string,
): Promise<string> {
  const stored = await store.getSecret(name);
  if (stored != null) return stored;
  if (plaintext) await store.setSecret(name, plaintext);
  return plaintext;
}
