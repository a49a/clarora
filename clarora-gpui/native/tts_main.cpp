// Kokoro runs in a child process: native runtime failures and cancellation never block GPUI.
#include <sherpa-onnx/c-api/c-api.h>
#include <cmath>
#include <algorithm>
#include <cstdint>
#include <stdexcept>
#include <cstdio>
#include <filesystem>
#include <fstream>
#include <iterator>
#include <string>
#include <vector>
#include <memory>
#include <sstream>
#ifdef _WIN32
#include <windows.h>
#endif
bool write_wav(const SherpaOnnxGeneratedAudio *audio, const char *path) {
  if (!audio || !audio->samples || audio->n <= 0 || audio->sample_rate <= 0 || audio->n > 1000000000) return false;
  std::ofstream out(std::filesystem::u8path(path), std::ios::binary);
  auto u16 = [&out](uint16_t n) { out.put(n & 255); out.put((n >> 8) & 255); };
  auto u32 = [&u16](uint32_t n) { u16(n & 65535); u16(n >> 16); };
  out.write("RIFF", 4); u32(36 + uint32_t(audio->n) * 2); out.write("WAVEfmt ", 8);
  u32(16); u16(1); u16(1); u32(audio->sample_rate); u32(audio->sample_rate * 2); u16(2); u16(16);
  out.write("data", 4); u32(uint32_t(audio->n) * 2);
  for (int32_t i = 0; i < audio->n; ++i) {
    if (!std::isfinite(audio->samples[i])) return false;
    u16(static_cast<uint16_t>(static_cast<int16_t>(std::clamp(audio->samples[i], -1.f, 1.f) * 32767.f)));
  }
  out.flush(); return out.good();
}
int run(int argc, char **argv) {
  if (argc != 6) { std::fprintf(stderr, "Usage: clarora-tts model-dir text-file output.wav speaker speed\n"); return 2; }
  try {
    const auto root = std::filesystem::u8path(argv[1]);
    std::ifstream input(std::filesystem::u8path(argv[2]), std::ios::binary);
    if (!input) throw std::runtime_error("Cannot read text file");
    std::string text((std::istreambuf_iterator<char>(input)), {});
    const int sid = std::stoi(argv[4]);
    const float speed = std::stof(argv[5]);
    if (text.empty() || text.size() > 40000 || text.find('\0') != std::string::npos ||
        sid < 0 || sid >= 54 || !std::isfinite(speed) || speed < 0.5f || speed > 2.f)
      throw std::runtime_error("Invalid text, speaker or speed");
    const auto model = (root / "model.onnx").u8string();
    const auto voices = (root / "voices.bin").u8string();
    const auto tokens = (root / "tokens.txt").u8string();
    const auto data = (root / "espeak-ng-data").u8string();
    const auto lexicon = (root / "lexicon-us-en.txt").u8string() + "," + (root / "lexicon-zh.txt").u8string();
    SherpaOnnxOfflineTtsConfig config{};
    config.model.kokoro.model = model.c_str();
    config.model.kokoro.voices = voices.c_str();
    config.model.kokoro.tokens = tokens.c_str();
    config.model.kokoro.data_dir = data.c_str();
    config.model.kokoro.lexicon = lexicon.c_str();
    config.model.kokoro.length_scale = 1.f;
    config.model.num_threads = 2;
    config.model.provider = "cpu";
    config.max_num_sentences = 1;
    config.silence_scale = 0.2f;
    const auto *tts = SherpaOnnxCreateOfflineTts(&config);
    if (!tts) throw std::runtime_error("Cannot initialize Kokoro");
    if (sid >= SherpaOnnxOfflineTtsNumSpeakers(tts)) {
      SherpaOnnxDestroyOfflineTts(tts);
      throw std::runtime_error("Speaker is unavailable in model");
    }
    SherpaOnnxGenerationConfig generation{};
    generation.sid = sid;
    generation.speed = speed;
    generation.silence_scale = 0.2f;
    // Rust sends one subtitle sentence per line. One engine loads the model once;
    // each sentence's actual generated sample count defines its subtitle boundary.
    std::unique_ptr<const SherpaOnnxOfflineTts, decltype(&SherpaOnnxDestroyOfflineTts)>
        engine(tts, &SherpaOnnxDestroyOfflineTts);
    const auto sample_rate = SherpaOnnxOfflineTtsSampleRate(tts);
    std::vector<float> samples;
    std::vector<std::pair<int64_t, int64_t>> boundaries;
    std::istringstream lines(text);
    std::string sentence;
    while (std::getline(lines, sentence)) {
      if (sentence.find_first_not_of(" \t\r") == std::string::npos) continue;
      std::unique_ptr<const SherpaOnnxGeneratedAudio, decltype(&SherpaOnnxDestroyOfflineTtsGeneratedAudio)>
          audio(SherpaOnnxOfflineTtsGenerateWithConfig(tts, sentence.c_str(), &generation, nullptr, nullptr),
                &SherpaOnnxDestroyOfflineTtsGeneratedAudio);
      if (!audio || !audio->samples || audio->n <= 0 || audio->sample_rate != sample_rate ||
          samples.size() + audio->n > 1000000000)
        throw std::runtime_error("Cannot generate sentence audio");
      const auto start = samples.size();
      samples.insert(samples.end(), audio->samples, audio->samples + audio->n);
      boundaries.emplace_back(start, samples.size());
    }
    SherpaOnnxGeneratedAudio combined{samples.data(), static_cast<int32_t>(samples.size()), sample_rate};
    if (!write_wav(&combined, argv[3])) throw std::runtime_error("Cannot write WAV");
    auto timing_path = std::filesystem::u8path(argv[3]);
    timing_path.replace_extension(".segments");
    std::ofstream timings(timing_path, std::ios::binary);
    timings << sample_rate << '\n';
    for (const auto &boundary : boundaries) timings << boundary.first << ' ' << boundary.second << '\n';
    timings.flush();
    if (!timings.good()) throw std::runtime_error("Cannot write sentence timings");
    return 0;
  } catch (const std::exception &e) { std::fprintf(stderr, "%s\n", e.what()); return 1; }
}
#ifdef _WIN32
int wmain(int argc, wchar_t **argv) {
  std::vector<std::string> strings;
  for (int i = 0; i < argc; ++i) {
    int length = WideCharToMultiByte(CP_UTF8, 0, argv[i], -1, nullptr, 0, nullptr, nullptr);
    if (length <= 0) return 2;
    std::string text(length, '\0');
    WideCharToMultiByte(CP_UTF8, 0, argv[i], -1, text.data(), length, nullptr, nullptr);
    text.pop_back(); strings.push_back(std::move(text));
  }
  std::vector<char *> pointers;
  for (auto &text : strings) pointers.push_back(text.data());
  return run(argc, pointers.data());
}
#else
int main(int argc, char **argv) { return run(argc, argv); }
#endif
