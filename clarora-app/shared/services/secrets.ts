import { currentPlatform, getNativeModules } from './platform';

/**
 * 系统凭证保险库抽象：macOS 走钥匙串（RNMacKeychain），Windows 走
 * PasswordVault（RNWindowsCredentials）。返回 null 表示当前平台没有
 * 原生保险库（iOS/Android），调用方退回原来的本机数据库存储。
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

export function secretStore(): SecretStore | null {
  if (currentPlatform === 'macos') {
    const module = getNativeModules().RNMacKeychain as NativeSecretStore | undefined;
    if (!module?.setSecret) return null;
    return module;
  }
  if (currentPlatform === 'windows') {
    const module = getNativeModules().RNWindowsCredentials as NativeSecretStore | undefined;
    if (!module?.setSecret) return null;
    return module;
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
  const stored = await store.getSecret(name).catch(() => null);
  if (stored != null) return stored;
  if (plaintext) await store.setSecret(name, plaintext).catch(() => {});
  return plaintext;
}
