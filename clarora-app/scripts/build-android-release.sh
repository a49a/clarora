#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
for name in SOURCE_SHA CLARORA_ANDROID_KEYSTORE CLARORA_ANDROID_STORE_PASSWORD CLARORA_ANDROID_KEY_ALIAS CLARORA_ANDROID_KEY_PASSWORD CLARORA_ANDROID_CERT_SHA256 CLARORA_BUILD_NUMBER; do
  if [ -z "${!name:-}" ]; then echo "Missing release configuration: $name" >&2; exit 1; fi
done
[[ "$SOURCE_SHA" =~ ^[0-9a-f]{40}$ ]] && [ "$(git rev-parse HEAD)" = "$SOURCE_SHA" ] || { echo "Source SHA does not match checkout" >&2; exit 1; }
[[ "$CLARORA_BUILD_NUMBER" =~ ^[1-9][0-9]{0,8}$ ]] || { echo 'Invalid build number' >&2; exit 1; }
test -f "$CLARORA_ANDROID_KEYSTORE"
sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
test -n "$sdk"
# Keep this version aligned with buildToolsVersion in android/build.gradle.
tools="$sdk/build-tools/36.0.0"
test -x "$tools/apksigner"
version="$(node -p "require('./package.json').version")"
cd android
./gradlew --no-daemon :app:assembleRelease
cd ..
apk=android/app/build/outputs/apk/release/app-release.apk
certs="$("$tools/apksigner" verify --verbose --print-certs "$apk")"
expected="$(printf '%s' "$CLARORA_ANDROID_CERT_SHA256" | tr -d ':' | tr '[:upper:]' '[:lower:]')"
actual="$(printf '%s\n' "$certs" | sed -n 's/^Signer #1 certificate SHA-256 digest: //p' | tr '[:upper:]' '[:lower:]')"
[[ "$expected" =~ ^[0-9a-f]{64}$ ]] && [ "$actual" = "$expected" ] || { echo 'APK signer fingerprint mismatch' >&2; exit 1; }
if printf '%s' "$certs" | grep -q 'CN=Android Debug'; then echo 'Debug certificate is forbidden' >&2; exit 1; fi
badging="$("$tools/aapt" dump badging "$apk")"
printf '%s\n' "$badging" | grep -F "package: name='com.clarora.app' versionCode='$CLARORA_BUILD_NUMBER' versionName='$version'" >/dev/null
if printf '%s\n' "$badging" | grep -q '^application-debuggable'; then echo 'Debuggable APK is forbidden' >&2; exit 1; fi
mkdir -p dist/mobile
cp "$apk" dist/mobile/Clarora-android.apk
node -e 'const fs=require("fs"),crypto=require("crypto");const name="Clarora-android.apk";fs.writeFileSync("dist/mobile/android-build.json",JSON.stringify({version:require("./package.json").version,build_number:process.env.CLARORA_BUILD_NUMBER,source_sha:process.env.SOURCE_SHA,asset_name:name,sha256:crypto.createHash("sha256").update(fs.readFileSync("dist/mobile/"+name)).digest("hex")},null,2)+"\n")'
