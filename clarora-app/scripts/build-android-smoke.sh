#!/usr/bin/env bash
# Build a debug-signed Release APK with bundled JavaScript for android:smoke.
# This is not a production signing build.
set -euo pipefail
cd "$(dirname "$0")/.."

export CLARORA_ANDROID_KEYSTORE="${CLARORA_ANDROID_KEYSTORE:-$PWD/android/app/debug.keystore}"
export CLARORA_ANDROID_STORE_PASSWORD="${CLARORA_ANDROID_STORE_PASSWORD:-android}"
export CLARORA_ANDROID_KEY_ALIAS="${CLARORA_ANDROID_KEY_ALIAS:-androiddebugkey}"
export CLARORA_ANDROID_KEY_PASSWORD="${CLARORA_ANDROID_KEY_PASSWORD:-android}"
export CLARORA_BUILD_NUMBER="${CLARORA_BUILD_NUMBER:-1}"

test -f "$CLARORA_ANDROID_KEYSTORE" || {
  echo "Android debug keystore not found: $CLARORA_ANDROID_KEYSTORE" >&2
  exit 1
}
[[ "$CLARORA_BUILD_NUMBER" =~ ^[1-9][0-9]{0,8}$ ]] || {
  echo "CLARORA_BUILD_NUMBER must be a positive integer, at most 9 digits." >&2
  exit 1
}

echo "Building debug-signed smoke APK (not production signing)."
bash scripts/ensure-android-hermes.sh
cd android
./gradlew --no-daemon :app:assembleRelease
echo "Smoke APK: android/app/build/outputs/apk/release/app-release.apk"
