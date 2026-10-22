#include <cstdio>
#include <cstring>
#include <cstdlib>
#include <filesystem>
#include <string>
#include <vector>
#include <ggml-backend.h>
#ifdef _WIN32
#include <windows.h>
#endif
extern "C" const char *ClaroraSenseVoiceTranscribe(const char *, const char *, const char *);
extern "C" const char *ClaroraWhisperTranscribe(const char *, const char *, const char *);
extern "C" const char *ClaroraWhisperTranscribeSegmented(const char *, const char *, const char *);
int run(int argc, char **argv) {
  if (argc != 5) { std::fprintf(stderr, "Usage: clarora-asr whisper|whisper-segmented|sense-voice audio model language|tokens\n"); return 2; }
  // Dynamic GGML installations (including Homebrew) require backend discovery.
  // A linked libwhisper alone can otherwise have no registered CPU device.
  if (std::strcmp(argv[1], "sense-voice") != 0 && !ggml_backend_dev_by_type(GGML_BACKEND_DEVICE_TYPE_CPU)) {
    const char *backend = std::getenv("CLARORA_GGML_CPU_BACKEND");
    const auto parent = std::filesystem::u8path(argv[0]).parent_path();
    std::string nearby;
    for (const char *name : {"ggml-cpu.dll", "libggml-cpu.so", "libggml-cpu.dylib"}) {
      auto path = parent / name;
      if (std::filesystem::is_regular_file(path)) { nearby = path.u8string(); break; }
    }
    if ((!backend || !backend[0]) && !nearby.empty()) backend = nearby.c_str();
#ifdef CLARORA_GGML_CPU_BACKEND
    if (!backend || !backend[0]) backend = CLARORA_GGML_CPU_BACKEND;
#endif
    if (backend && backend[0]) ggml_backend_load(backend);
    else ggml_backend_load_all();
  }
  if (std::strcmp(argv[1], "sense-voice") != 0 && !ggml_backend_dev_by_type(GGML_BACKEND_DEVICE_TYPE_CPU)) {
    std::fprintf(stderr, "No GGML backend found; install its matching CPU backend runtime.\n");
    return 1;
  }
  const char *result = std::strcmp(argv[1], "sense-voice") == 0
    ? ClaroraSenseVoiceTranscribe(argv[2], argv[3], argv[4])
    : std::strcmp(argv[1], "whisper-segmented") == 0
      ? ClaroraWhisperTranscribeSegmented(argv[2], argv[3], argv[4])
      : ClaroraWhisperTranscribe(argv[2], argv[3], argv[4]);
  if (!result) return 1;
  std::puts(result);
  return std::strstr(result, "\"error\"") ? 1 : 0;
}
#ifdef _WIN32
int wmain(int argc, wchar_t **argv) {
  std::vector<std::string> strings;
  for (int i = 0; i < argc; ++i) {
    int length = WideCharToMultiByte(CP_UTF8, 0, argv[i], -1, nullptr, 0, nullptr, nullptr);
    if (length <= 0) return 2;
    std::string text(length, '\0');
    WideCharToMultiByte(CP_UTF8, 0, argv[i], -1, text.data(), length, nullptr, nullptr);
    text.pop_back();
    strings.push_back(std::move(text));
  }
  std::vector<char *> pointers;
  for (auto &text : strings) pointers.push_back(text.data());
  return run(argc, pointers.data());
}
#else
int main(int argc, char **argv) { return run(argc, argv); }
#endif
