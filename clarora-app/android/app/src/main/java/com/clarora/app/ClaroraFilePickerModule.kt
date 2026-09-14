package com.clarora.app

import android.app.Activity
import android.content.Intent
import android.database.Cursor
import android.provider.MediaStore
import android.provider.OpenableColumns
import com.facebook.react.bridge.ActivityEventListener
import com.facebook.react.bridge.Arguments
import com.facebook.react.bridge.Promise
import com.facebook.react.bridge.ReactApplicationContext
import com.facebook.react.bridge.ReactContextBaseJavaModule
import com.facebook.react.bridge.ReactMethod
import java.io.File
import java.io.FileOutputStream

class ClaroraFilePickerModule(private val context: ReactApplicationContext) :
  ReactContextBaseJavaModule(context), ActivityEventListener {

  private var pendingPick: Promise? = null
  private var pendingCapture: Promise? = null
  private var pendingCapturePath: String? = null

  init {
    context.addActivityEventListener(this)
  }

  override fun getName(): String = "RNAndroidFilePicker"

  /** Launch the camera app and save the full-resolution photo into Imports. */
  @ReactMethod
  fun captureImage(promise: Promise) {
    if (pendingCapture != null) {
      promise.reject("picker_busy", "已有拍照任务进行中")
      return
    }
    val activity = currentActivity
    if (activity == null) {
      promise.reject("picker_unavailable", "当前没有可用的 Android 窗口")
      return
    }
    val targetDirectory = File(context.filesDir, "Clarora/Imports")
    targetDirectory.mkdirs()
    val target = File(targetDirectory, "capture_${System.currentTimeMillis()}.jpg")
    val imageUri = androidx.core.content.FileProvider.getUriForFile(
      context, "${context.packageName}.fileprovider", target
    )
    pendingCapture = promise
    pendingCapturePath = target.absolutePath
    val intent = Intent(MediaStore.ACTION_IMAGE_CAPTURE).apply {
      putExtra(MediaStore.EXTRA_OUTPUT, imageUri)
      addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION)
    }
    activity.startActivityForResult(intent, REQUEST_CAPTURE_IMAGE)
  }

  @ReactMethod
  fun pickFile(type: String?, promise: Promise) {
    if (pendingPick != null) {
      promise.reject("picker_busy", "已有文件选择窗口正在打开")
      return
    }
    val activity = currentActivity
    if (activity == null) {
      promise.reject("picker_unavailable", "当前没有可用的 Android 窗口")
      return
    }
    pendingPick = promise
    val mimeType = when (type) {
      "audio/*" -> "audio/*"
      "text/*" -> "text/*"
      else -> "*/*"
    }
    val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
      addCategory(Intent.CATEGORY_OPENABLE)
      this.type = mimeType
      addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    }
    activity.startActivityForResult(intent, REQUEST_PICK_FILE)
  }

  @ReactMethod
  fun pickDirectory(promise: Promise) {
    // Android's Storage Access Framework returns a content:// tree URI. The
    // desktop directory-import format depends on normal recursive file paths,
    // so leave it unavailable rather than silently importing an incomplete set.
    promise.resolve(null)
  }

  override fun onActivityResult(activity: Activity, requestCode: Int, resultCode: Int, data: Intent?) {
    if (requestCode == REQUEST_CAPTURE_IMAGE) {
      val promise = pendingCapture
      pendingCapture = null
      val path = pendingCapturePath
      pendingCapturePath = null
      if (promise == null) return
      val file = if (path != null) File(path) else null
      if (resultCode != Activity.RESULT_OK || file == null || !file.exists() || file.length() == 0L) {
        file?.delete()
        promise.resolve(null)
        return
      }
      promise.resolve(Arguments.createMap().apply {
        putString("uri", "file://$path")
        putString("name", file.name)
      })
      return
    }
    if (requestCode != REQUEST_PICK_FILE) return
    val promise = pendingPick ?: return
    pendingPick = null
    val source = data?.data
    if (resultCode != Activity.RESULT_OK || source == null) {
      promise.resolve(null)
      return
    }
    try {
      val name = displayName(source) ?: "imported-file"
      val targetDirectory = File(context.filesDir, "Clarora/Imports")
      targetDirectory.mkdirs()
      val target = File(targetDirectory, "${System.currentTimeMillis()}_$name")
      context.contentResolver.openInputStream(source).use { input ->
        if (input == null) throw IllegalStateException("无法读取所选文件")
        FileOutputStream(target).use { output -> input.copyTo(output) }
      }
      promise.resolve(Arguments.createMap().apply {
        putString("uri", "file://${target.absolutePath}")
        putString("name", name)
      })
    } catch (error: Exception) {
      promise.reject("file_copy_error", error.message, error)
    }
  }

  override fun onNewIntent(intent: Intent) = Unit

  private fun displayName(uri: android.net.Uri): String? {
    var cursor: Cursor? = null
    return try {
      cursor = context.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)
      if (cursor?.moveToFirst() == true) cursor.getString(0) else null
    } finally {
      cursor?.close()
    }
  }

  private companion object {
    const val REQUEST_PICK_FILE = 7301
    const val REQUEST_CAPTURE_IMAGE = 7302
  }
}
