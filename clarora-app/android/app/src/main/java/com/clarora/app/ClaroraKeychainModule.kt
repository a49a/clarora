package com.clarora.app

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import com.facebook.react.bridge.Promise
import com.facebook.react.bridge.ReactApplicationContext
import com.facebook.react.bridge.ReactContextBaseJavaModule
import com.facebook.react.bridge.ReactMethod

/**
 * Android 安全存储:Android Keystore 生成不可导出的 AES-256 密钥,
 * 学习凭证以 AES/GCM 加密后存入应用私有 Preferences。
 * 密钥不出 Keystore,替换密文即可完成轮换;清除应用数据时密文与
 * 密钥一同销毁。协议与桌面端模块一致:setSecret / getSecret / deleteSecret。
 */
class ClaroraKeychainModule(private val context: ReactApplicationContext) :
  ReactContextBaseJavaModule(context) {

  override fun getName(): String = "RNAndroidKeychain"

  private fun key(): SecretKey {
    val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
    (ks.getKey("clarora.vault", null) as? SecretKey)?.let { return it }
    val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
    generator.init(
      KeyGenParameterSpec.Builder("clarora.vault", KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
        .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
        .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
        .setKeySize(256)
        .build(),
    )
    return generator.generateKey()
  }

  private fun prefs() = context.getSharedPreferences("clarora.secrets", Context.MODE_PRIVATE)

  private fun encrypt(plain: String): Pair<ByteArray, ByteArray> {
    val cipher = Cipher.getInstance("AES/GCM/NoPadding")
    cipher.init(Cipher.ENCRYPT_MODE, key())
    return cipher.iv to cipher.doFinal(plain.toByteArray(Charsets.UTF_8))
  }

  private fun decrypt(iv: ByteArray, data: ByteArray): String {
    val cipher = Cipher.getInstance("AES/GCM/NoPadding")
    cipher.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, iv))
    return String(cipher.doFinal(data), Charsets.UTF_8)
  }

  @ReactMethod
  fun setSecret(account: String, value: String, promise: Promise) {
    try {
      val (iv, data) = encrypt(value)
      prefs().edit()
        .putString("iv.$account", android.util.Base64.encodeToString(iv, android.util.Base64.NO_WRAP))
        .putString("data.$account", android.util.Base64.encodeToString(data, android.util.Base64.NO_WRAP))
        .apply()
      promise.resolve(true)
    } catch (error: Exception) {
      promise.reject("keystore_error", "加密存储写入失败:${error.message}", error)
    }
  }

  @ReactMethod
  fun getSecret(account: String, promise: Promise) {
    try {
      val iv = prefs().getString("iv.$account", null)
      val data = prefs().getString("data.$account", null)
      if (iv == null || data == null) { promise.resolve(null); return }
      promise.resolve(decrypt(android.util.Base64.decode(iv, android.util.Base64.NO_WRAP),
                             android.util.Base64.decode(data, android.util.Base64.NO_WRAP)))
    } catch (error: Exception) {
      promise.reject("keystore_error", "加密存储读取失败:${error.message}", error)
    }
  }

  @ReactMethod
  fun deleteSecret(account: String, promise: Promise) {
    prefs().edit().remove("iv.$account").remove("data.$account").apply()
    promise.resolve(true)
  }
}
