#!/bin/sh
# Launch the local GPUI desktop client without requiring Node/npm.
set -eu
root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)

if [ "$(uname -s)" = "Darwin" ]; then
  if [ ! -f "$root/target/debug/libpdfium.dylib" ]; then
    echo "正在安装固定版本 PDFium：target/debug/libpdfium.dylib"
    python3 "$root/scripts/install-pdfium.py" "$root/target/debug"
  fi
  sh "$root/scripts/package-macos.sh"
  exec "$root/target/Clarora.app/Contents/MacOS/clarora-gpui" "$@"
fi

exec cargo run --manifest-path "$root/Cargo.toml" --locked -- "$@"
