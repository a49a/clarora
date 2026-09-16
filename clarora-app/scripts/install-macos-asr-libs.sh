#!/bin/sh
# 安装 macOS 端侧转写所需的 sherpa-onnx 动态库与头文件。
# whisper.cpp 走 Homebrew（brew install whisper-cpp，会同时带上 ggml）。
# GitHub 不可达时自动回退 api.github.com 资产端点。CI 与本机通用。
set -e

SHERPA_VERSION=v1.13.8
ARCH=$(uname -m)
case "$ARCH" in
  arm64) SHERPA_ASSET="sherpa-onnx-$SHERPA_VERSION-osx-arm64-shared-no-tts-lib.tar.bz2" ;;
  x86_64) SHERPA_ASSET="sherpa-onnx-$SHERPA_VERSION-osx-x86_64-shared-no-tts-lib.tar.bz2" ;;
  *) echo "unsupported arch: $ARCH" >&2; exit 1 ;;
esac

PREFIX=/opt/homebrew
[ -d "$PREFIX" ] || PREFIX=/usr/local
mkdir -p "$PREFIX/lib" "$PREFIX/include/sherpa-onnx/c-api"

# 幂等：头文件已正确就位时直接跳过。
HEADER="$PREFIX/include/sherpa-onnx/c-api/c-api.h"
if [ -f "$HEADER" ] && grep -q "SherpaOnnxCreateOfflineRecognizer" "$HEADER" 2>/dev/null; then
  echo "sherpa-onnx already installed at $PREFIX"
  exit 0
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

download() {
  # download <url> <输出路径>：失败时回退 api.github.com（Accept: octet-stream）。
  url="$1"
  output="$2"
  if curl -fsSL --connect-timeout 15 -o "$output" "$url"; then return 0; fi
  echo "primary download failed, retrying via api.github.com…" >&2
  release_json="$TMP/release.json"
  curl -fsSL --connect-timeout 15 -o "$release_json" \
    "https://api.github.com/repos/k2-fsa/sherpa-onnx/releases/tags/$SHERPA_VERSION"
  asset_id=$(python3 -c "
import json, sys
data = json.load(open('$release_json'))
for asset in data.get('assets', []):
    if asset['name'] == '$SHERPA_ASSET':
        print(asset['id']); break
")
  if [ -z "$asset_id" ]; then
    echo "asset $SHERPA_ASSET not found" >&2
    return 1
  fi
  curl -fsSL --connect-timeout 15 -H "Accept: application/octet-stream" \
    -o "$output" "https://api.github.com/repos/k2-fsa/sherpa-onnx/releases/assets/$asset_id"
}

download "https://github.com/k2-fsa/sherpa-onnx/releases/download/$SHERPA_VERSION/$SHERPA_ASSET" \
  "$TMP/libs.tar.bz2"
tar -xjf "$TMP/libs.tar.bz2" -C "$TMP"
cp "$TMP"/sherpa-onnx-*/lib/*.dylib "$PREFIX/lib/"

# 头文件不是发布资产，不走 api 回退；依次尝试 raw 与 jsDelivr 镜像。
HEADER_OK=0
for header_url in \
  "https://raw.githubusercontent.com/k2-fsa/sherpa-onnx/$SHERPA_VERSION/sherpa-onnx/c-api/c-api.h" \
  "https://cdn.jsdelivr.net/gh/k2-fsa/sherpa-onnx@$SHERPA_VERSION/sherpa-onnx/c-api/c-api.h"; do
  if curl -fsSL --connect-timeout 15 -o "$HEADER" "$header_url"; then HEADER_OK=1; break; fi
done
if [ "$HEADER_OK" != 1 ] || ! grep -q "SherpaOnnxCreateOfflineRecognizer" "$HEADER" 2>/dev/null; then
  echo "c-api.h download failed" >&2
  exit 1
fi

echo "sherpa-onnx libraries installed to $PREFIX"
