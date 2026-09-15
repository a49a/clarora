# clarora_asr — Windows 端侧转写包装 DLL。
# 依赖（由 scripts/build-windows-asr.ps1 准备）：
#   * whisper.cpp —— vcpkg 安装（-DCMAKE_TOOLCHAIN_FILE=.../vcpkg.cmake）
#   * sherpa-onnx —— 发行包解压到 deps/sherpa-onnx/{include,lib}
cmake_minimum_required(VERSION 3.20)
project(clarora_asr CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_WINDOWS_EXPORT_ALL_SYMBOLS ON)

add_library(clarora_asr SHARED clarora_asr.cpp)

target_include_directories(clarora_asr PRIVATE
  ${CMAKE_CURRENT_LIST_DIR}
  ${CMAKE_CURRENT_LIST_DIR}/deps/whisper.cpp/include
  ${SHERPA_INCLUDE_DIR}
)

# vcpkg whisper-cpp 提供 whisper.lib；sherpa-onnx 包提供 sherpa-onnx-c-api.lib。
target_link_directories(clarora_asr PRIVATE
  ${CMAKE_CURRENT_LIST_DIR}/deps/whisper.cpp/lib
  ${SHERPA_LIB_DIR}
)
target_link_libraries(clarora_asr PRIVATE whisper sherpa-onnx-c-api)
