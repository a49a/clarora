// clarora_asr — Windows 端侧转写包装（P/Invoke 目标 DLL）。
//
// 职责与 macOS 的 RNMacWhisper / RNMacSenseVoice 一致：
//   1. miniaudio 将任意受支持音频（MP3/WAV/FLAC 等）解码重采样为 16 kHz 单声道 Float32；
//   2. whisper.cpp（英文/多语种）与 sherpa-onnx SenseVoice（中英日韩粤）离线转写；
//   3. 返回 {"duration":秒,"segments":[{"start":秒,"end":秒,"text":"…"}]} JSON，
//      由 C# 模块原样传回 JS 解析。
//
// 构建见 scripts/build-windows-asr.ps1；依赖 whisper.cpp（vcpkg）与
// sherpa-onnx 发行包，运行时 DLL 需与应用同目录。

#include "miniaudio.h"

#include <whisper.h>
#include <sherpa-onnx/c-api/c-api.h>

#include <cctype>
#include <map>
#include <mutex>
#include <string>
#include <vector>

#if defined(_WIN32)
#define CLARORA_EXPORT extern "C" __declspec(dllexport)
#else
// 非 Windows 环境仅用于语法检查。
#define CLARORA_EXPORT extern "C"
#endif

namespace {

std::string JsonEscape(const std::string &text) {
  std::string out;
  out.reserve(text.size() + 8);
  for (char character : text) {
    switch (character) {
      case '"': out += "\\\""; break;
      case '\\': out += "\\\\"; break;
      case '\n': out += "\\n"; break;
      case '\r': out += "\\r"; break;
      case '\t': out += "\\t"; break;
      default:
        if (static_cast<unsigned char>(character) < 0x20) {
          char escape[8];
          snprintf(escape, sizeof(escape), "\\u%04x", character);
          out += escape;
        } else {
          out += character;
        }
    }
  }
  return out;
}

std::string SegmentsJson(double duration,
                         const std::vector<double> &starts,
                         const std::vector<double> &ends,
                         const std::vector<std::string> &texts) {
  std::string json = "{\"duration\":" + std::to_string(duration) + ",\"segments\":[";
  for (size_t i = 0; i < texts.size(); i++) {
    if (i != 0) json += ",";
    json += "{\"start\":" + std::to_string(starts[i]) + ",\"end\":" + std::to_string(ends[i]);
    json += ",\"text\":\"" + JsonEscape(texts[i]) + "\"}";
  }
  json += "]}";
  return json;
}

// miniaudio 解码并重采样到 16 kHz 单声道 Float32，内存有界。
bool DecodeAudio16kMono(const char *path, std::vector<float> *samples, std::string *error) {
  ma_decoder_config config = ma_decoder_config_init(ma_format_f32, 1, 16000);
  ma_decoder decoder;
  ma_result status = ma_decoder_init_file(path, &config, &decoder);
  if (status != MA_SUCCESS) {
    *error = "无法读取音频文件（支持 MP3/M4A/WAV/FLAC）";
    return false;
  }
  float buffer[4096];
  ma_uint64 read = 0;
  while (true) {
    status = ma_decoder_read_pcm_frames(&decoder, buffer, 4096, &read);
    if (status != MA_SUCCESS || read == 0) break;
    samples->insert(samples->end(), buffer, buffer + read);
  }
  ma_decoder_uninit(&decoder);
  if (samples->empty()) {
    *error = "音频内容为空";
    return false;
  }
  return true;
}

// SenseVoice 只输出逐 token 时间戳；按标点、停顿和长度聚合成字幕段。
void SenseVoiceCues(const SherpaOnnxOfflineRecognizerResult *result,
                    double duration,
                    std::vector<double> *starts,
                    std::vector<double> *ends,
                    std::vector<std::string> *texts) {
  if (result->tokens_arr == nullptr || result->timestamps == nullptr) {
    std::string text = result->text ? result->text : "";
    if (!text.empty()) {
      starts->push_back(0); ends->push_back(duration); texts->push_back(text);
    }
    return;
  }
  const std::string punctuation = "。！？!?.,;:;:、，";
  double cueStart = -1, prevEnd = -1;
  int32_t tokensInCue = 0;
  std::string cueText;
  for (int32_t i = 0; i < result->count; i++) {
    const char *raw = result->tokens_arr[i];
    if (raw == nullptr) continue;
    std::string token = raw;
    if (token.rfind("<|", 0) == 0) continue;  // 语言/情感等特殊标记
    double timestamp = result->timestamps[i];
    if (timestamp < 0 || timestamp > duration) continue;
    if (cueStart < 0) cueStart = timestamp > 0.05 ? timestamp - 0.05 : 0;
    if (prevEnd >= 0 && timestamp - prevEnd > 0.6 && !cueText.empty()) {
      starts->push_back(cueStart);
      ends->push_back(prevEnd + 0.2 < duration ? prevEnd + 0.2 : duration);
      texts->push_back(cueText);
      cueText.clear();
      cueStart = timestamp > 0.05 ? timestamp - 0.05 : 0;
      tokensInCue = 0;
    }
    cueText += token;
    tokensInCue++;
    prevEnd = timestamp;
    bool isPunctuation = punctuation.find(token) != std::string::npos;
    if (isPunctuation || tokensInCue >= 20 || timestamp - cueStart >= 6.0) {
      starts->push_back(cueStart);
      ends->push_back(timestamp + 0.2 < duration ? timestamp + 0.2 : duration);
      texts->push_back(cueText);
      cueText.clear();
      cueStart = -1;
      tokensInCue = 0;
    }
  }
  if (!cueText.empty()) {
    starts->push_back(cueStart < 0 ? 0 : cueStart);
    ends->push_back(prevEnd + 0.2 < duration ? prevEnd + 0.2 : duration);
    texts->push_back(cueText);
  }
}

std::mutex &WhisperMutex() { static std::mutex mutex; return mutex; }
std::map<std::string, whisper_context *> &WhisperContexts() {
  static std::map<std::string, whisper_context *> contexts;
  return contexts;
}
std::mutex &SenseVoiceMutex() { static std::mutex mutex; return mutex; }
std::map<std::string, const SherpaOnnxOfflineRecognizer *> &SenseVoiceRecognizers() {
  static std::map<std::string, const SherpaOnnxOfflineRecognizer *> recognizers;
  return recognizers;
}

}  // namespace

CLARORA_EXPORT const char *ClaroraWhisperTranscribe(
    const char *audioPath, const char *modelPath, const char *language) {
  static thread_local std::string response;
  std::vector<float> samples;
  std::string error;
  if (!DecodeAudio16kMono(audioPath, &samples, &error)) {
    response = "{\"error\":\"" + JsonEscape(error) + "\"}";
    return response.c_str();
  }

  std::lock_guard<std::mutex> guard(WhisperMutex());
  auto found = WhisperContexts().find(modelPath);
  whisper_context *context = found != WhisperContexts().end() ? found->second : nullptr;
  if (context == nullptr) {
    whisper_context_params contextParams = whisper_context_default_params();
    // Windows 默认为 CPU 构建：注册不到 GPU 后端时 use_gpu=true 会 GGML_ABORT。
    contextParams.use_gpu = false;
    context = whisper_init_from_file_with_params(modelPath, contextParams);
    if (context == nullptr) {
      response = "{\"error\":\"无法加载模型文件，请到设置中重新下载\"}";
      return response.c_str();
    }
    WhisperContexts()[modelPath] = context;
  }

  whisper_full_params params = whisper_full_default_params(WHISPER_SAMPLING_GREEDY);
  params.print_progress = false;
  params.print_special = false;
  params.print_realtime = false;
  params.print_timestamps = false;
  params.translate = false;
  params.language = (language != nullptr && language[0] != '\0') ? language : "auto";

  double duration = static_cast<double>(samples.size()) / 16000.0;
  std::vector<double> starts, ends;
  std::vector<std::string> texts;
  if (whisper_full(context, params, samples.data(), static_cast<int>(samples.size())) != 0) {
    response = "{\"error\":\"端侧转写失败，请重试或更换模型\"}";
    return response.c_str();
  }
  int count = whisper_full_n_segments(context);
  for (int i = 0; i < count; i++) {
    double start = whisper_full_get_segment_t0(context, i) / 100.0;  // whisper 时刻单位 10ms
    double end = whisper_full_get_segment_t1(context, i) / 100.0;
    if (end < start) end = start;
    starts.push_back(start);
    ends.push_back(end);
    texts.push_back(whisper_full_get_segment_text(context, i));
  }
  response = SegmentsJson(duration, starts, ends, texts);
  return response.c_str();
}

CLARORA_EXPORT const char *ClaroraSenseVoiceTranscribe(
    const char *audioPath, const char *modelPath, const char *tokensPath) {
  static thread_local std::string response;
  std::vector<float> samples;
  std::string error;
  if (!DecodeAudio16kMono(audioPath, &samples, &error)) {
    response = "{\"error\":\"" + JsonEscape(error) + "\"}";
    return response.c_str();
  }

  std::lock_guard<std::mutex> guard(SenseVoiceMutex());
  auto found = SenseVoiceRecognizers().find(modelPath);
  const SherpaOnnxOfflineRecognizer *recognizer =
      found != SenseVoiceRecognizers().end() ? found->second : nullptr;
  if (recognizer == nullptr) {
    SherpaOnnxOfflineRecognizerConfig config;
    memset(&config, 0, sizeof(config));
    config.feat_config.sample_rate = 16000;
    config.feat_config.feature_dim = 80;
    config.model_config.sense_voice.model = modelPath;
    config.model_config.sense_voice.language = "auto";  // zh/en/ja/ko/yue 自动检测
    config.model_config.sense_voice.use_itn = 1;
    config.model_config.tokens = tokensPath;
    config.model_config.num_threads = 2;
    recognizer = SherpaOnnxCreateOfflineRecognizer(&config);
    if (recognizer == nullptr) {
      response = "{\"error\":\"无法加载 SenseVoice 模型，请到设置中重新下载\"}";
      return response.c_str();
    }
    SenseVoiceRecognizers()[modelPath] = recognizer;
  }

  const SherpaOnnxOfflineStream *stream = SherpaOnnxCreateOfflineStream(recognizer);
  if (stream == nullptr) {
    response = "{\"error\":\"端侧转写失败，请重试\"}";
    return response.c_str();
  }
  SherpaOnnxAcceptWaveformOffline(stream, 16000, samples.data(), static_cast<int32_t>(samples.size()));
  SherpaOnnxDecodeOfflineStream(recognizer, stream);
  const SherpaOnnxOfflineRecognizerResult *result = SherpaOnnxGetOfflineStreamResult(stream);
  double duration = static_cast<double>(samples.size()) / 16000.0;
  std::vector<double> starts, ends;
  std::vector<std::string> texts;
  SenseVoiceCues(result, duration, &starts, &ends, &texts);
  SherpaOnnxDestroyOfflineRecognizerResult(result);
  SherpaOnnxDestroyOfflineStream(stream);
  response = SegmentsJson(duration, starts, ends, texts);
  return response.c_str();
}

// 预留的释放接口：当前响应保存在 thread_local 缓冲中，C# 拷贝后调用即可。
CLARORA_EXPORT void ClaroraFreeString(char *) {
}

