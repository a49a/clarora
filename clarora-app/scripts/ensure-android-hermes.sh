#!/usr/bin/env bash
# Restore the upstream Hermes compiler that the react-native-macos alias omits.
set -euo pipefail
cd "$(dirname "$0")/.."

installed="$(cat node_modules/react-native/sdks/.hermesversion 2>/dev/null || true)"
if [ -z "$installed" ]; then
  echo "Missing node_modules/react-native/sdks/.hermesversion; run npm ci first." >&2
  exit 1
fi

case "$(uname -s)" in
  Darwin) platform="osx-bin"; binary="hermesc" ;;
  Linux) platform="linux64-bin"; binary="hermesc" ;;
  MINGW*|MSYS*|CYGWIN*) platform="win64-bin"; binary="hermesc.exe" ;;
  *)
    echo "Unsupported Android release host: $(uname -s)" >&2
    exit 1
    ;;
esac

compiler="node_modules/react-native/sdks/hermesc/$platform/$binary"
if [ -f "$compiler" ] && { [ "$binary" = "hermesc.exe" ] || [ -x "$compiler" ]; }; then
  exit 0
fi

upstream="${CLARORA_HERMES_RN_VERSION:-0.81.6}"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
package="$(npm pack "react-native@$upstream" --pack-destination "$tmp" --silent | tail -n 1)"
tarball="$tmp/$package"
if [ ! -f "$tarball" ]; then
  echo "npm pack did not produce react-native@$upstream." >&2
  exit 1
fi

compiler_version="$(tar -xOzf "$tarball" package/sdks/.hermesversion)"
if [ "$installed" != "$compiler_version" ]; then
  echo "Hermes version mismatch: alias has $installed, react-native@$upstream has $compiler_version." >&2
  exit 1
fi

mkdir -p "$(dirname "$compiler")"
tar -xOzf "$tarball" "package/sdks/hermesc/$platform/$binary" > "$compiler"
chmod +x "$compiler"
echo "Restored Hermes compiler for react-native@$upstream ($platform)."
