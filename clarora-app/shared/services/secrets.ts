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
