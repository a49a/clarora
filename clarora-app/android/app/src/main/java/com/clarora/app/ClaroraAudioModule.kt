package com.clarora.app

import android.media.MediaPlayer
import android.media.MediaRecorder
import android.media.PlaybackParams
import android.os.Build
import com.facebook.react.bridge.Arguments
import com.facebook.react.bridge.Promise
import com.facebook.react.bridge.ReactApplicationContext
import com.facebook.react.bridge.ReactContextBaseJavaModule
import com.facebook.react.bridge.ReactMethod
import java.io.File

class ClaroraAudioModule(private val context: ReactApplicationContext) : ReactContextBaseJavaModule(context) {
  private var player: MediaPlayer? = null
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
    mediaPlayer.start()
    promise.resolve(status())
  }

  @ReactMethod
  fun pause(promise: Promise) {
    player?.takeIf { it.isPlaying }?.pause()
    promise.resolve(status())
  }

  @ReactMethod
  fun setPosition(milliseconds: Double, promise: Promise) {
    player?.seekTo(milliseconds.toInt().coerceAtLeast(0))
    playbackCompleted = false
    promise.resolve(status())
  }

  @ReactMethod
  fun setRate(rate: Double, promise: Promise) {
    playbackRate = rate.toFloat()
    player?.let { applyRate(it) }
    promise.resolve(status())
  }

  @ReactMethod
  fun setLoop(loop: Boolean, promise: Promise) {
    player?.isLooping = loop
    promise.resolve(status())
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
    putDouble("positionMillis", (mediaPlayer?.currentPosition ?: 0).toDouble())
    if (mediaPlayer != null) putDouble("durationMillis", mediaPlayer.duration.toDouble())
    putBoolean("isPlaying", mediaPlayer?.isPlaying == true)
    putBoolean("didJustFinish", playbackCompleted && mediaPlayer?.isPlaying != true)
  }

  private fun unloadPlayer() {
    player?.run {
      reset()
      release()
    }
    player = null
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
      val mediaPlayer = MediaPlayer()
      breakPlayer = mediaPlayer
      mediaPlayer.setDataSource(File(path.removePrefix("file://")).absolutePath)
      mediaPlayer.isLooping = true
      mediaPlayer.prepare()
      mediaPlayer.start()
    } catch (error: Exception) {
      breakPlayer = null
    }
  }

  @ReactMethod
  fun pauseBreak() {
    breakPlayer?.takeIf { it.isPlaying }?.pause()
  }

  @ReactMethod
  fun resumeBreak() {
    breakPlayer?.start()
  }

  @ReactMethod
  fun stopBreakPlayer() {
    breakPlayer?.let {
      it.stop()
      it.release()
    }
    breakPlayer = null
  }
}
