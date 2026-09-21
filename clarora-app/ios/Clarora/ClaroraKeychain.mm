// RN iOS Keychain 模块:系统钥匙串存取(kSecClassGenericPassword)。
// 与 macOS 端 RNMacKeychain 同协议:setSecret / getSecret / deleteSecret。
// 服务名 clarora.secrets,account 区分用途(与桌面端保持一致的键名)。

#import <Foundation/Foundation.h>
#import <Security/Security.h>
#import <React/RCTBridgeModule.h>

@interface RNIOSKeychain : NSObject <RCTBridgeModule>
@end

@implementation RNIOSKeychain

RCT_EXPORT_MODULE(RNIOSKeychain);

static NSDictionary *KeychainQuery(NSString *account) {
  return @{
    (id)kSecClass: (id)kSecClassGenericPassword,
    (id)kSecAttrService: @"clarora.secrets",
    (id)kSecAttrAccount: account,
  };
}

RCT_EXPORT_METHOD(setSecret:(NSString *)account
                  value:(NSString *)value
                  resolver:(RCTPromiseResolveBlock)resolve
                  rejecter:(RCTPromiseRejectBlock)reject)
{
  NSData *data = [value dataUsingEncoding:NSUTF8StringEncoding] ?: [NSData data];
  NSDictionary *query = KeychainQuery(account);
  // 先更新既有条目:重装/换签名后旧条目仍挂在旧访问组上,更新会触发
  // 授权框并允许"始终允许"完成迁移;直接删旧建新会让删除被拒后
  // SecItemAdd 撞 errSecDuplicateItem,写入从此失败。
  OSStatus status = SecItemUpdate((__bridge CFDictionaryRef)query, (__bridge CFDictionaryRef)@{
    (__bridge NSString *)kSecValueData: data,
  });
  if (status == errSecItemNotFound) {
    NSMutableDictionary *attributes = [query mutableCopy];
    attributes[(__bridge NSString *)kSecValueData] = data;
    attributes[(__bridge NSString *)kSecAttrAccessible] = (id)kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly;
    status = SecItemAdd((__bridge CFDictionaryRef)attributes, NULL);
  }
  if (status == errSecSuccess) resolve(@YES);
  else reject(@"keychain_error", [NSString stringWithFormat:@"钥匙串写入失败(OSStatus %d)", (int)status], nil);
}

RCT_EXPORT_METHOD(getSecret:(NSString *)account
                  resolver:(RCTPromiseResolveBlock)resolve
                  rejecter:(RCTPromiseRejectBlock)reject)
{
  NSMutableDictionary *query = [KeychainQuery(account) mutableCopy];
  query[(__bridge NSString *)kSecReturnData] = @YES;
  query[(__bridge NSString *)kSecMatchLimit] = (id)kSecMatchLimitOne;
  CFDataRef data = NULL;
  OSStatus status = SecItemCopyMatching((__bridge CFDictionaryRef)query, (CFTypeRef *)&data);
  if (status == errSecItemNotFound) { resolve([NSNull null]); return; }
  if (status != errSecSuccess) {
    reject(@"keychain_error", [NSString stringWithFormat:@"钥匙串读取失败(OSStatus %d)", (int)status], nil);
    return;
  }
  NSString *value = [[NSString alloc] initWithData:(__bridge NSData *)data encoding:NSUTF8StringEncoding];
  CFRelease(data);
  resolve(value ?: @"");
}

RCT_EXPORT_METHOD(deleteSecret:(NSString *)account
                  resolver:(RCTPromiseResolveBlock)resolve
                  rejecter:(RCTPromiseRejectBlock)reject)
{
  OSStatus status = SecItemDelete((__bridge CFDictionaryRef)KeychainQuery(account));
  if (status == errSecSuccess || status == errSecItemNotFound) resolve(@YES);
  else reject(@"keychain_error", [NSString stringWithFormat:@"钥匙串删除失败(OSStatus %d)", (int)status], nil);
}

@end
