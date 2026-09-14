package com.clarora.app

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import com.facebook.react.bridge.ReactApplicationContext
import com.facebook.react.bridge.ReactContextBaseJavaModule
import com.facebook.react.bridge.ReactMethod

class ClaroraClipboardModule(private val context: ReactApplicationContext) :
  ReactContextBaseJavaModule(context) {

  override fun getName(): String = "RNAndroidClipboard"

  @ReactMethod
  fun setText(text: String?) {
    val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
    clipboard.setPrimaryClip(ClipData.newPlainText("Clarora", text ?: ""))
  }
}
