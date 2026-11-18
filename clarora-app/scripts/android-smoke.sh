#!/usr/bin/env bash
# Install and launch an APK on one connected Android device/emulator.
# Usage: bash scripts/android-smoke.sh [path/to/Clarora-android.apk]
# Optional: CLARORA_ANDROID_SMOKE_OUT=/path writes screenshot, logcat and JSON.
# Uses a clean install, so it clears Clarora app data on the selected device.
set -euo pipefail
cd "$(dirname "$0")/.."
apk="${1:-android/app/build/outputs/apk/release/app-release.apk}"
out="${CLARORA_ANDROID_SMOKE_OUT:-}"
if [ -n "${ANDROID_HOME:-}" ] && [ -x "$ANDROID_HOME/platform-tools/adb" ]; then
  adb="$ANDROID_HOME/platform-tools/adb"
elif [ -n "${ANDROID_SDK_ROOT:-}" ] && [ -x "$ANDROID_SDK_ROOT/platform-tools/adb" ]; then
  adb="$ANDROID_SDK_ROOT/platform-tools/adb"
else
  adb="$(command -v adb || true)"
fi
[ -n "$adb" ] && [ -x "$adb" ] || { echo "adb not found; set ANDROID_HOME or ANDROID_SDK_ROOT." >&2; exit 1; }
[ -f "$apk" ] || { echo "APK not found: $apk (build an APK first)." >&2; exit 1; }
device="$("$adb" devices | awk 'NR>1 && $2=="device" {print $1; exit}')"
[ -n "$device" ] || { echo "No authorized Android device/emulator connected." >&2; exit 1; }

capture() {
  [ -n "$out" ] || return 0
  mkdir -p "$out"
  "$adb" -s "$device" exec-out screencap -p > "$out/screenshot.png" 2>/dev/null || true
  "$adb" -s "$device" logcat -d > "$out/logcat.txt" 2>/dev/null || true
}

fail() {
  local message="$1"
  capture
  if [ -n "$out" ]; then
    printf '{"passed":false,"device":"%s","apk":"%s","error":"%s"}\n' "$device" "$apk" "${message//\"/\\\"}" > "$out/android-smoke.json"
  fi
  echo "$message" >&2
  exit 1
}

# Start each smoke run from a clean install so local reruns do not depend on
# an older/lower version code left by a previous build.
echo "Installing clean smoke build on $device (clears Clarora app data)."
"$adb" -s "$device" logcat -c 2>/dev/null || true
"$adb" -s "$device" uninstall com.clarora.app >/dev/null 2>&1 || true
"$adb" -s "$device" install "$apk" || fail "APK install failed: $apk"
"$adb" -s "$device" shell am force-stop com.clarora.app
"$adb" -s "$device" shell am start -W -n com.clarora.app/.MainActivity || fail "MainActivity did not start."
sleep 4
pid="$("$adb" -s "$device" shell pidof com.clarora.app | tr -d '\r')"
[ -n "$pid" ] || fail "Clarora process did not stay running."
resumed="$("$adb" -s "$device" shell dumpsys activity activities | grep -E 'mResumedActivity|topResumedActivity' | grep -c 'com.clarora.app' || true)"
[ "$resumed" -ge 1 ] || fail "MainActivity is not the resumed activity."
fatal="$("$adb" -s "$device" logcat -d --pid="$pid" | grep -c 'FATAL EXCEPTION' || true)"
[ "$fatal" -eq 0 ] || fail "AndroidRuntime reported FATAL EXCEPTION."
bundle_error="$("$adb" -s "$device" logcat -d --pid="$pid" | grep -c 'Unable to load script' || true)"
[ "$bundle_error" -eq 0 ] || fail "JavaScript bundle failed to load; use a release APK with bundled JS."
capture
if [ -n "$out" ]; then
  printf '{"passed":true,"device":"%s","apk":"%s","pid":"%s","resumed":true,"fatal_exception":false,"screenshot":"%s","logcat":"%s"}\n' \
    "$device" "$apk" "$pid" "$out/screenshot.png" "$out/logcat.txt" > "$out/android-smoke.json"
fi
echo "Clarora Android smoke passed on $device (pid $pid)."
