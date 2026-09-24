#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
: "${SOURCE_SHA:?Missing SOURCE_SHA}"
: "${CLARORA_BUILD_NUMBER:?Missing CLARORA_BUILD_NUMBER}"
[[ "$SOURCE_SHA" =~ ^[0-9a-f]{40}$ ]] && [ "$(git rev-parse HEAD)" = "$SOURCE_SHA" ] || { echo 'Source SHA does not match checkout' >&2; exit 1; }
[[ "$CLARORA_BUILD_NUMBER" =~ ^[1-9][0-9]{0,8}$ ]] || { echo 'Invalid build number' >&2; exit 1; }
export CLARORA_VERSION="$(node -p "require('./package.json').version")"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
export CLARORA_IPA_WORK="$work"

# A device archive without signing needs no Apple account. Do not exportArchive:
# the export step requires distribution signing; package Payload ourselves.
xcodebuild -workspace ios/Clarora.xcworkspace -scheme Clarora-iOS -configuration Release \
  -sdk iphoneos -destination 'generic/platform=iOS' \
  -derivedDataPath ios/build/unsigned -archivePath "$work/Clarora.xcarchive" \
  CODE_SIGNING_ALLOWED=NO CODE_SIGNING_REQUIRED=NO CODE_SIGN_IDENTITY= \
  DEVELOPMENT_TEAM= MARKETING_VERSION="$CLARORA_VERSION" \
  CURRENT_PROJECT_VERSION="$CLARORA_BUILD_NUMBER" archive
mkdir "$work/Payload"
ditto "$work/Clarora.xcarchive/Products/Applications/Clarora.app" "$work/Payload/Clarora.app"
python3 - <<'PY'
import os, pathlib, plistlib, re, subprocess
app = pathlib.Path(os.environ['CLARORA_IPA_WORK']) / 'Payload/Clarora.app'
info = plistlib.loads((app / 'Info.plist').read_bytes())
assert info['CFBundleIdentifier'] == 'com.clarora.app', 'Unexpected bundle ID'
assert info['CFBundleShortVersionString'] == os.environ['CLARORA_VERSION'], 'Version mismatch'
assert info['CFBundleVersion'] == os.environ['CLARORA_BUILD_NUMBER'], 'Build number mismatch'
assert info['CFBundleSupportedPlatforms'] == ['iPhoneOS'], 'Expected device app, not simulator'
executable = app / info['CFBundleExecutable']
assert executable.is_file() and os.access(executable, os.X_OK), 'Missing executable'
subprocess.run(['xcrun', 'lipo', str(executable), '-verify_arch', 'arm64'], check=True)
build = subprocess.check_output(['xcrun', 'vtool', '-show-build', str(executable)], text=True)
assert re.search(r'\bplatform\s+IOS\s', build), 'Executable must target iOS devices'
assert (app / 'main.jsbundle').stat().st_size > 0, 'Missing Release JS bundle'
assert not list(app.rglob('embedded.mobileprovision')), 'Unexpected provisioning profile'
assert not list(app.rglob('_CodeSignature')), 'Unexpected code signature'
PY
(cd "$work" && /usr/bin/zip -qry Clarora-ios-unsigned.ipa Payload)
python3 - <<'PY'
import hashlib, json, os, pathlib, subprocess, zipfile
work = pathlib.Path(os.environ['CLARORA_IPA_WORK'])
ipa = work / 'Clarora-ios-unsigned.ipa'
with zipfile.ZipFile(ipa) as z:
    assert z.testzip() is None, 'Corrupt IPA'
    assert all(n.startswith('Payload/') for n in z.namelist()), 'Unexpected IPA contents'
report = dict(source_sha=os.environ['SOURCE_SHA'], version=os.environ['CLARORA_VERSION'],
              source_dirty=bool(subprocess.check_output(['git', 'status', '--porcelain']).strip()),
              build_number=os.environ['CLARORA_BUILD_NUMBER'], platform='ios', arch='arm64',
              signing='unsigned', distribution_status='requires-user-signing',
              asset_name=ipa.name, sha256=hashlib.sha256(ipa.read_bytes()).hexdigest())
(work / 'ios-unsigned-build.json').write_text(json.dumps(report, indent=2) + '\n')
PY
mkdir -p dist/mobile
cp "$work/Clarora-ios-unsigned.ipa" "$work/ios-unsigned-build.json" dist/mobile/
echo 'Created Clarora-ios-unsigned.ipa — user signing required before installation.'
