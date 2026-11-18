package com.clarora.app

import android.content.Context
import android.media.AudioAttributes
import android.media.AudioFocusRequest
import android.media.AudioManager
import android.media.MediaPlayer
import android.media.MediaRecorder
import android.media.PlaybackParams
import android.os.Build
import com.facebook.react.bridge.Arguments
import com.facebook.react.bridge.LifecycleEventListener
import com.facebook.react.bridge.Promise
import com.facebook.react.bridge.ReactApplicationContext
import com.facebook.react.bridge.ReactContextBaseJavaModule
import com.facebook.react.bridge.ReactMethod
import java.io.File

class ClaroraAudioModule(private val context: ReactApplicationContext) :
  ReactContextBaseJavaModule(context), LifecycleEventListener {
  private var player: MediaPlayer? = null
  private val audioManager = context.getSystemService(Context.AUDIO_SERVICE) as AudioManager
  private var focusRequest: Any? = null
  private var hasAudioFocus = false
  private var playbackRate = 1f
  // OnCompletionListener 落位后置 true，status() 用它上报 didJustFinish，
  // JS 侧的单篇循环/组循环推进都依赖这个信号。
  private var playbackCompleted = false
  // 休息音乐专用播放器(独立于学习音频的共享 player,互不打断进度)。
  private var breakPlayer: MediaPlayer? = null
  private var recorder: MediaRecorder? = null
  private var recordingPath: String? = null

  override fun getName(): String = "RNAndroidAudio"

  private val feedPlayers = mutableMapOf<Int, MediaPlayer>()

  private val focusListener = AudioManager.OnAudioFocusChangeListener { change ->
    when (change) {
      AudioManager.AUDIOFOCUS_LOSS -> {
        hasAudioFocus = false
        pauseForFocusLoss()
      }
      AudioManager.AUDIOFOCUS_LOSS_TRANSIENT -> pauseForFocusLoss()
      AudioManager.AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK -> setPlaybackVolume(0.3f)
      AudioManager.AUDIOFOCUS_GAIN -> setPlaybackVolume(1f)
    }
  }

  init {
    context.addLifecycleEventListener(this)
  }

  private fun requestAudioFocus(): Boolean {
    if (hasAudioFocus) return true
    val granted = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
      val request = (focusRequest as? AudioFocusRequest) ?: AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
        .setAudioAttributes(
          AudioAttributes.Builder()
            .setUsage(AudioAttributes.USAGE_MEDIA)
            .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
            .build(),
        )
        .setOnAudioFocusChangeListener(focusListener)
        .build()
        .also { focusRequest = it }
      audioManager.requestAudioFocus(request)
    } else {
      @Suppress("DEPRECATION")
      audioManager.requestAudioFocus(
        focusListener,
        AudioManager.STREAM_MUSIC,
        AudioManager.AUDIOFOCUS_GAIN,
      )
    }
    hasAudioFocus = granted == AudioManager.AUDIOFOCUS_REQUEST_GRANTED
    return hasAudioFocus
  }

  private fun abandonAudioFocusIfIdle() {
    if (feedPlayers.values.any { it.isPlaying } ||
      player?.isPlaying == true ||
      breakPlayer?.isPlaying == true) return
    if (!hasAudioFocus && focusRequest == null) return
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
      (focusRequest as? AudioFocusRequest)?.let { audioManager.abandonAudioFocusRequest(it) }
    } else {
      @Suppress("DEPRECATION")
      audioManager.abandonAudioFocus(focusListener)
    }
    hasAudioFocus = false
  }

  private fun pauseForFocusLoss() {
    runCatching { player?.takeIf { it.isPlaying }?.pause() }
    runCatching { breakPlayer?.takeIf { it.isPlaying }?.pause() }
    feedPlayers.values.forEach { runCatching { if (it.isPlaying) it.pause() } }
  }

  private fun setPlaybackVolume(volume: Float) {
    runCatching { player?.setVolume(volume, volume) }
    runCatching { breakPlayer?.setVolume(volume, volume) }
  }

  override fun onHostResume() = Unit

  override fun onHostPause() {
    pauseForFocusLoss()
    discardRecording()
    abandonAudioFocusIfIdle()
  }

  override fun onHostDestroy() {
    releaseAll()
  }

  override fun invalidate() {
    context.removeLifecycleEventListener(this)
    releaseAll()
    super.invalidate()
  }

  private fun releaseAll() {
    feedPlayers.values.forEach { runCatching { it.release() } }
    feedPlayers.clear()
    unloadPlayer()
    stopBreakPlayer()
    discardRecording()
  }

  private fun discardRecording() {
    recorder?.let {
      runCatching { it.stop() }
      runCatching { it.release() }
    }
    recordingPath?.let { path -> runCatching { File(path).delete() } }
    recorder = null
    recordingPath = null
  }

  @ReactMethod
  fun feedPrepare(slot: Int, path: String, start: Double, promise: Promise) {
    feedPlayers.remove(slot)?.release()
    val p = MediaPlayer()
    feedPlayers[slot] = p
    try {
      p.setDataSource(path)
      p.setOnErrorListener { _, what, extra ->
        promise.reject("feed_audio", "无法加载音频（$what/$extra）")
        true
      }
      p.setOnPreparedListener {
        if (start > 0) {
          it.setOnSeekCompleteListener { ready ->
            ready.setOnSeekCompleteListener(null)
            promise.resolve(null)
          }
          it.seekTo(start.toInt().coerceAtMost(it.duration))
        } else promise.resolve(null)
      }
      p.prepareAsync()
    } catch (error: Exception) {
      feedPlayers.remove(slot)?.release()
      promise.reject("feed_audio", error.message, error)
    }
  }

  @ReactMethod
  fun feedPlay(slot: Int, promise: Promise) {
    try {
      if (!requestAudioFocus()) {
        promise.reject("feed_audio", "其他应用正在占用音频")
        return
      }
      feedPlayers.filterKeys { it != slot }.values.forEach { if (it.isPlaying) it.pause() }
      val p = feedPlayers[slot] ?: error("音频尚未就绪")
      p.start()
      promise.resolve(null)
    } catch (error: Exception) { promise.reject("feed_audio", error.message, error) }
  }

  @ReactMethod
  fun feedPause(slot: Int, promise: Promise) {
    try {
      feedPlayers[slot]?.let { if (it.isPlaying) it.pause() }
      abandonAudioFocusIfIdle()
      promise.resolve(null)
    } catch (error: Exception) { promise.reject("feed_audio", error.message, error) }
  }

  @ReactMethod
  fun feedStatus(slot: Int, promise: Promise) {
    try {
      val p = feedPlayers[slot]
      promise.resolve(Arguments.createMap().apply {
        putDouble("positionMillis", (p?.currentPosition ?: 0).toDouble())
        putBoolean("isPlaying", p?.isPlaying == true)
      })
    } catch (error: Exception) { promise.reject("feed_audio", error.message, error) }
  }

  @ReactMethod
  fun feedUnload(promise: Promise) {
    feedPlayers.values.forEach { it.release() }
    feedPlayers.clear()
    promise.resolve(null)
  }

  @ReactMethod
  fun load(path: String, rate: Double, promise: Promise) {
    unloadPlayer()
    playbackRate = rate.toFloat()
    try {
      val mediaPlayer = MediaPlayer()
      player = mediaPlayer
      playbackCompleted = false
      mediaPlayer.setOnCompletionListener { playbackCompleted = true }
      mediaPlayer.setDataSource(File(path.removePrefix("file://")).absolutePath)
      mediaPlayer.setOnPreparedListener {
        applyRate(it)
        promise.resolve(status())
      }
      mediaPlayer.setOnErrorListener { _, what, extra ->
        promise.reject("audio_load_error", "无法加载音频（$what/$extra）")
        true
      }
      mediaPlayer.prepareAsync()
    } catch (error: Exception) {
      unloadPlayer()
      promise.reject("audio_load_error", error.message, error)
    }
  }

  @ReactMethod
  fun play(promise: Promise) {
    val mediaPlayer = player
    if (mediaPlayer == null) {
      promise.reject("audio_play_error", "没有可播放的音频")
      return
    }
    try {
      if (!requestAudioFocus()) {
        promise.reject("audio_focus", "其他应用正在占用音频")
        return
      }
      mediaPlayer.start()
      promise.resolve(status())
    } catch (error: Exception) {
      promise.reject("audio_play_error", error.message, error)
    }
  }

  @ReactMethod
  fun pause(promise: Promise) {
    try {
      player?.takeIf { it.isPlaying }?.pause()
      abandonAudioFocusIfIdle()
      promise.resolve(status())
    } catch (error: Exception) {
      promise.reject("audio_pause_error", error.message, error)
    }
  }

  @ReactMethod
  fun setPosition(milliseconds: Double, promise: Promise) {
    try {
      player?.seekTo(milliseconds.toInt().coerceAtLeast(0))
      playbackCompleted = false
      promise.resolve(status())
    } catch (error: Exception) {
      promise.reject("audio_seek_error", error.message, error)
    }
  }

  @ReactMethod
  fun setRate(rate: Double, promise: Promise) {
    try {
      playbackRate = rate.toFloat()
      player?.let { applyRate(it) }
      promise.resolve(status())
    } catch (error: Exception) {
      promise.reject("audio_rate_error", error.message, error)
    }
  }

  @ReactMethod
  fun setLoop(loop: Boolean, promise: Promise) {
    try {
      player?.isLooping = loop
      promise.resolve(status())
    } catch (error: Exception) {
      promise.reject("audio_loop_error", error.message, error)
    }
  }

  @ReactMethod
  fun status(promise: Promise) {
    promise.resolve(status())
  }

  @ReactMethod
  fun unload(promise: Promise) {
    unloadPlayer()
    promise.resolve(null)
  }

  private fun applyRate(mediaPlayer: MediaPlayer) {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
      mediaPlayer.playbackParams = PlaybackParams().setSpeed(playbackRate).setPitch(1f)
    }
  }

  private fun status() = Arguments.createMap().apply {
    val mediaPlayer = player
    putBoolean("isLoaded", mediaPlayer != null)
    putDouble("positionMillis", runCatching { mediaPlayer?.currentPosition }.getOrNull()?.toDouble() ?: 0.0)
    runCatching { mediaPlayer?.duration }.getOrNull()?.let { putDouble("durationMillis", it.toDouble()) }
    putBoolean("isPlaying", runCatching { mediaPlayer?.isPlaying == true }.getOrDefault(false))
    putBoolean("didJustFinish", playbackCompleted && runCatching { mediaPlayer?.isPlaying == true }.getOrDefault(false).not())
  }

  private fun unloadPlayer() {
    player?.let {
      runCatching { it.reset() }
      runCatching { it.release() }
    }
    player = null
    abandonAudioFocusIfIdle()
  }

  // ── 口语跟读录音（AAC m4a，ASR 友好）──

  // 番茄钟阶段切换提示音（ToneGenerator 合成音，无需音频文件）。
  @ReactMethod
  fun playChime(promise: Promise) {
    try {
      val tone = android.media.ToneGenerator(android.media.AudioManager.STREAM_NOTIFICATION, 100)
      tone.startTone(android.media.ToneGenerator.TONE_PROP_BEEP2, 350)
      android.os.Handler(android.os.Looper.getMainLooper()).postDelayed({ tone.release() }, 800)
      promise.resolve(true)
    } catch (error: Exception) {
      promise.reject("chime_failed", error.message, error)
    }
  }


  @ReactMethod
  fun startRecording(promise: Promise) {
    if (recorder != null) {
      promise.reject("record_busy", "已有录音进行中")
      return
    }
    try {
      val targetDirectory = File(context.filesDir, "Clarora/Recordings")
      targetDirectory.mkdirs()
      val target = File(targetDirectory, "rec_${System.currentTimeMillis()}.m4a")
      val mediaRecorder = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
        MediaRecorder(context)
      } else {
        @Suppress("DEPRECATION") MediaRecorder()
      }
      recorder = mediaRecorder
      recordingPath = target.absolutePath
      mediaRecorder.setAudioSource(MediaRecorder.AudioSource.MIC)
      mediaRecorder.setOutputFormat(MediaRecorder.OutputFormat.MPEG_4)
      mediaRecorder.setAudioEncoder(MediaRecorder.AudioEncoder.AAC)
      mediaRecorder.setAudioSamplingRate(16000)
      mediaRecorder.setAudioEncodingBitRate(64000)
      mediaRecorder.setOutputFile(target.absolutePath)
      mediaRecorder.prepare()
      mediaRecorder.start()
      promise.resolve(true)
    } catch (error: Exception) {
      recorder?.release()
      recorder = null
      recordingPath = null
      promise.reject("record_failed", error.message, error)
    }
  }

  @ReactMethod
  fun stopRecording(promise: Promise) {
    val mediaRecorder = recorder
    val path = recordingPath
    if (mediaRecorder == null || path == null) {
      promise.reject("not_recording", "当前没有录音")
      return
    }
    try {
      mediaRecorder.stop()
    } catch (error: Exception) {
      mediaRecorder.release()
      recorder = null
      recordingPath = null
      File(path).delete()
      promise.reject("record_stop_error", error.message, error)
      return
    }
    mediaRecorder.release()
    recorder = null
    recordingPath = null
    val file = File(path)
    if (!file.exists() || file.length() == 0L) {
      file.delete()
      promise.reject("record_empty", "录音为空，请重试")
      return
    }
    promise.resolve(Arguments.createMap().apply {
      putString("uri", "file://$path")
      putString("name", file.name)
    })
  }

  // ── 休息音乐(独立播放器)──

  @ReactMethod
  fun playBreak(path: String) {
    stopBreakPlayer()
    try {
      if (!requestAudioFocus()) return
      val mediaPlayer = MediaPlayer()
      breakPlayer = mediaPlayer
      mediaPlayer.setDataSource(File(path.removePrefix("file://")).absolutePath)
      mediaPlayer.isLooping = true
      mediaPlayer.prepare()
      mediaPlayer.start()
    } catch (error: Exception) {
      breakPlayer?.let { runCatching { it.release() } }
      breakPlayer = null
    }
  }

  @ReactMethod
  fun pauseBreak() {
    runCatching { breakPlayer?.takeIf { it.isPlaying }?.pause() }
    abandonAudioFocusIfIdle()
  }

  @ReactMethod
  fun resumeBreak() {
    if (!requestAudioFocus()) return
    runCatching { breakPlayer?.start() }
  }

  @ReactMethod
  fun stopBreakPlayer() {
    breakPlayer?.let {
      runCatching { it.stop() }
      runCatching { it.release() }
    }
    breakPlayer = null
    abandonAudioFocusIfIdle()
  }
}
