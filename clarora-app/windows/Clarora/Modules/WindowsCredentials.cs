using Microsoft.ReactNative.Managed;
using System;
using Windows.Security.Credentials;

namespace Clarora
{
    /// <summary>
    /// 系统凭证保险库（Windows）：使用 PasswordVault 存放 AI / 存储密钥等
    /// 敏感配置，按用户账户加密。接口与 macOS 的 RNMacKeychain 一致。
    /// </summary>
    [ReactModule("RNWindowsCredentials")]
    public sealed class WindowsCredentials
    {
        private const string Resource = "Clarora";
        private readonly PasswordVault vault = new PasswordVault();

        private PasswordCredential Find(string name)
        {
            try
            {
                return vault.Retrieve(Resource, name);
            }
            catch (Exception)
            {
                return null;  // PasswordVault 在条目不存在时抛异常
            }
        }

        [ReactMethod("setSecret")]
        public void SetSecret(string name, string value)
        {
            var existing = Find(name);
            if (existing != null) vault.Remove(existing);
            vault.Add(new PasswordCredential(Resource, name, value ?? string.Empty));
        }

        [ReactMethod("getSecret")]
        public string GetSecret(string name)
        {
            var credential = Find(name);
            return credential?.Password;
        }

        [ReactMethod("deleteSecret")]
        public void DeleteSecret(string name)
        {
            var credential = Find(name);
            if (credential != null) vault.Remove(credential);
        }
    }
}
