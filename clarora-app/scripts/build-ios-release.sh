#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
for name in SOURCE_SHA IOS_CERTIFICATE_BASE64 IOS_CERTIFICATE_PASSWORD IOS_PROFILE_BASE64 IOS_TEAM_ID CLARORA_BUILD_NUMBER; do
  if [ -z "${!name:-}" ]; then echo "Missing release configuration: $name" >&2; exit 1; fi
done
[[ "$SOURCE_SHA" =~ ^[0-9a-f]{40}$ ]] && [ "$(git rev-parse HEAD)" = "$SOURCE_SHA" ] || { echo "Source SHA does not match checkout" >&2; exit 1; }
method="${IOS_EXPORT_METHOD:-app-store-connect}"
case "$method" in app-store-connect|release-testing) ;; *) echo 'Unsupported iOS distribution method' >&2; exit 1;; esac
[[ "$CLARORA_BUILD_NUMBER" =~ ^[1-9][0-9]{0,8}$ ]] || { echo 'Invalid build number' >&2; exit 1; }
work="$(mktemp -d)"
keychain="$work/signing.keychain-db"
profile_path=""
cp ios/Clarora.xcodeproj/project.pbxproj "$work/project.pbxproj"
security list-keychains -d user > "$work/keychains.txt"
cleanup() {
  cp "$work/project.pbxproj" ios/Clarora.xcodeproj/project.pbxproj
  python3 - "$work/keychains.txt" <<'PYKEYS'
import shlex, subprocess, sys
subprocess.run(['security', 'list-keychains', '-d', 'user', '-s', *shlex.split(open(sys.argv[1]).read())], check=False)
PYKEYS
  security delete-keychain "$keychain" >/dev/null 2>&1 || true
  [ -z "$profile_path" ] || rm -f "$profile_path"
  rm -rf "$work"
}
trap cleanup EXIT
umask 077
export IOS_EXPORT_METHOD="$method" IOS_SIGNING_WORK="$work"
printf '%s' "$IOS_CERTIFICATE_BASE64" | base64 --decode > "$work/certificate.p12"
printf '%s' "$IOS_PROFILE_BASE64" | base64 --decode > "$work/profile.mobileprovision"
security cms -D -i "$work/profile.mobileprovision" > "$work/profile.plist"
python3 - <<'PY'
import datetime, os, pathlib, plistlib
w = pathlib.Path(os.environ['IOS_SIGNING_WORK'])
p = plistlib.loads((w/'profile.plist').read_bytes())
team = os.environ['IOS_TEAM_ID']
assert team in p['TeamIdentifier'], 'Profile team mismatch'
assert p['Entitlements']['application-identifier'] == team + '.com.clarora.app', 'Profile app ID mismatch'
assert not p['Entitlements'].get('get-task-allow'), 'Development profile is forbidden'
assert p['ExpirationDate'] > datetime.datetime.now(datetime.timezone.utc).replace(tzinfo=None), 'Profile expired'
assert not p.get('ProvisionsAllDevices'), 'Enterprise profile is unsupported'
method = os.environ['IOS_EXPORT_METHOD']
assert bool(p.get('ProvisionedDevices')) == (method == 'release-testing'), 'Profile distribution method mismatch'
(w/'uuid').write_text(p['UUID'])
options = dict(method=method, destination='export', signingStyle='manual', teamID=team,
               signingCertificate='Apple Distribution', manageAppVersionAndBuildNumber=False,
               provisioningProfiles={'com.clarora.app': p['UUID']})
(w/'ExportOptions.plist').write_bytes(plistlib.dumps(options))
PY
uuid="$(cat "$work/uuid")"
profile_dir="$HOME/Library/MobileDevice/Provisioning Profiles"
mkdir -p "$profile_dir"
profile_path="$profile_dir/$uuid.mobileprovision"
# Do not replace a user's existing profile when running locally.
if [ -e "$profile_path" ]; then profile_path=""; echo 'Profile already installed; use a clean signing runner' >&2; exit 1; fi
cp "$work/profile.mobileprovision" "$profile_path"
password="$(openssl rand -hex 24)"
security create-keychain -p "$password" "$keychain"
python3 - "$work/keychains.txt" "$keychain" <<'PYKEYS'
import shlex, subprocess, sys
subprocess.run(['security', 'list-keychains', '-d', 'user', '-s', *shlex.split(open(sys.argv[1]).read()), sys.argv[2]], check=True)
PYKEYS
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$password" "$keychain"
security import "$work/certificate.p12" -P "$IOS_CERTIFICATE_PASSWORD" -A -t cert -f pkcs12 -k "$keychain" >/dev/null
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$password" "$keychain" >/dev/null
export IOS_PROFILE_UUID="$uuid" IOS_SIGNING_KEYCHAIN="$keychain"
export CLARORA_VERSION="$(node -p "require('./package.json').version")"
# Set signing only on the app, not on CocoaPods targets. This runs in a disposable checkout.
ruby -rxcodeproj -e '
p = Xcodeproj::Project.open("ios/Clarora.xcodeproj")
p.targets.select { |t| t.product_type == "com.apple.product-type.application" }.each do |t|
  t.build_configurations.each do |c|
    c.build_settings.merge!({"CODE_SIGN_STYLE" => "Manual", "DEVELOPMENT_TEAM" => ENV.fetch("IOS_TEAM_ID"),
      "CODE_SIGN_IDENTITY" => "Apple Distribution", "PROVISIONING_PROFILE_SPECIFIER" => ENV.fetch("IOS_PROFILE_UUID"),
      "OTHER_CODE_SIGN_FLAGS" => "--keychain #{ENV.fetch("IOS_SIGNING_KEYCHAIN")}",
      "MARKETING_VERSION" => ENV.fetch("CLARORA_VERSION"), "CURRENT_PROJECT_VERSION" => ENV.fetch("CLARORA_BUILD_NUMBER")})
  end
end
p.save'
xcodebuild -workspace ios/Clarora.xcworkspace -scheme Clarora-iOS -configuration Release \
  -destination 'generic/platform=iOS' -archivePath "$work/Clarora.xcarchive" archive
xcodebuild -exportArchive -archivePath "$work/Clarora.xcarchive" \
  -exportOptionsPlist "$work/ExportOptions.plist" -exportPath "$work/export"
mkdir -p dist/mobile
test -f "$work/export/Clarora.ipa"
cp "$work/export/Clarora.ipa" "dist/mobile/Clarora-ios-$method.ipa"
python3 - <<'PY'
import hashlib, json, os, pathlib, plistlib, zipfile
method = os.environ['IOS_EXPORT_METHOD']
path = pathlib.Path('dist/mobile') / f'Clarora-ios-{method}.ipa'
with zipfile.ZipFile(path) as z:
    names = [n for n in z.namelist() if n.startswith('Payload/') and n.count('/') == 2 and n.endswith('.app/Info.plist')]
    assert len(names) == 1, 'Expected one app in IPA'
    info = plistlib.loads(z.read(names[0]))
assert info['CFBundleIdentifier'] == 'com.clarora.app'
assert info['CFBundleShortVersionString'] == os.environ['CLARORA_VERSION']
assert info['CFBundleVersion'] == os.environ['CLARORA_BUILD_NUMBER']
report = dict(source_sha=os.environ.get('SOURCE_SHA'), version=info['CFBundleShortVersionString'],
              build_number=info['CFBundleVersion'], method=method, asset_name=path.name,
              sha256=hashlib.sha256(path.read_bytes()).hexdigest(), distribution_status='awaiting-channel-validation')
(path.parent/'ios-build.json').write_text(json.dumps(report, indent=2)+'\n')
PY
