#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
if [ "${1:-}" = "--release" ]; then
  shift
  exec python3 "$root/scripts/release-macos.py" "$@"
fi
output=${1:-"$root/target/Clarora-GPUI.app"}
cargo build --manifest-path "$root/Cargo.toml" --locked
# Use the same public Clarora artwork as the React Native macOS app.
icon_source="$root/../clarora-app/macos/Clarora-macOS/Assets.xcassets/AppIcon.appiconset"
icon_work=$(mktemp -d)
trap 'rm -rf "$icon_work"' EXIT HUP INT TERM
mkdir "$icon_work/Clarora.iconset"
for image in "$icon_source"/icon_*.png; do
  cp "$image" "$icon_work/Clarora.iconset/"
done
mkdir -p "$output/Contents/MacOS" "$output/Contents/Resources"
iconutil --convert icns "$icon_work/Clarora.iconset" --output "$output/Contents/Resources/Clarora.icns"
cp "$root/../LICENSE" "$output/Contents/Resources/LICENSE"
cp "$root/README.md" "$output/Contents/Resources/README.md"
cp "$root/target/debug/clarora-gpui" "$output/Contents/MacOS/clarora-gpui.new"
mv -f "$output/Contents/MacOS/clarora-gpui.new" "$output/Contents/MacOS/clarora-gpui"
if [ -f "$root/target/native/clarora-asr" ]; then
  mkdir -p "$output/Contents/MacOS/tools"
  cp "$root/target/native/clarora-asr" "$output/Contents/MacOS/tools/clarora-asr"
fi
if [ -f "$root/target/tts/clarora-tts" ]; then
  mkdir -p "$output/Contents/MacOS/tools"
  cp "$root/target/tts/clarora-tts" "$output/Contents/MacOS/tools/clarora-tts.new"
  mv -f "$output/Contents/MacOS/tools/clarora-tts.new" "$output/Contents/MacOS/tools/clarora-tts"
  for runtime in "$root/target/tts/"*.dylib; do
    [ -f "$runtime" ] || continue
    name=$(basename "$runtime")
    cp "$runtime" "$output/Contents/MacOS/tools/$name.new"
    mv -f "$output/Contents/MacOS/tools/$name.new" "$output/Contents/MacOS/tools/$name"
  done
fi
cat > "$output/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>com.clarora.gpui</string>
<key>CFBundleName</key><string>Clarora GPUI</string>
<key>CFBundleDisplayName</key><string>Clarora GPUI</string>
<key>CFBundleExecutable</key><string>clarora-gpui</string>
<key>CFBundleIconFile</key><string>Clarora.icns</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>0.2.1</string>
<key>CFBundleVersion</key><string>1</string>
<key>NSHighResolutionCapable</key><true/>
<key>NSMicrophoneUsageDescription</key><string>录制朗读音频，用于本机保存、转写和跟读文本对齐。</string>
</dict></plist>
PLIST
if [ -d "$output/Contents/MacOS/pdfium-licenses" ]; then
  mv "$output/Contents/MacOS/pdfium-licenses" "$output/Contents/Resources/"
fi
codesign --force --deep --sign - "$output"
printf '%s\n' "$output"
