#!/usr/bin/env bash
# Build the iPhone app and send it to TestFlight.
#
#   scripts/testflight.sh            build number = one more than the last, then archive, export, upload
#   scripts/testflight.sh archive    stop after the archive (a check that it builds and signs)
#   scripts/testflight.sh upload     upload the .ipa already exported, without rebuilding
#
# Needs, once:
#   - the Apple Distribution certificate and its private key in the login keychain (team 6LYK439Z4F)
#   - fastlane (brew install fastlane) and XcodeGen (brew install xcodegen)
#   - an App Store Connect API key, named in apps/apple/.env.testflight (not in the repository):
#       export ASC_KEY_ID=…  ASC_ISSUER_ID=…  ASC_KEY_PATH=/path/to/AuthKey_….p8
#   - the app record for dev.airband.app in App Store Connect. Apple gives no way to create one except by hand:
#     appstoreconnect.apple.com → Apps → + → New App. Until it exists the upload is refused.
#
# The signing profile is regenerated each time against whichever distribution certificate this Mac holds, so the
# script does not depend on a profile made on one particular machine.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP="$ROOT/apps/apple"
BUILD="$APP/build"
ARCHIVE="$BUILD/Airband.xcarchive"
TEAM_ID="${TEAM_ID:-6LYK439Z4F}"
BUNDLE_ID="dev.airband.app"
# shellcheck disable=SC1091
[[ -f "$APP/.env.testflight" ]] && source "$APP/.env.testflight"
: "${ASC_KEY_ID:?set ASC_KEY_ID (apps/apple/.env.testflight)}" "${ASC_ISSUER_ID:?set ASC_ISSUER_ID}" "${ASC_KEY_PATH:?set ASC_KEY_PATH}"

# fastlane takes the key as a JSON file; made on the fly so the key's bytes are never on a command line
asc_key_json() {
  python3 -c "import json; print(json.dumps({'key_id':'$ASC_KEY_ID','issuer_id':'$ASC_ISSUER_ID','key':open('$ASC_KEY_PATH').read(),'in_house':False}))"
}

upload() {
  local ipa; ipa="$(find "$BUILD/export" -name '*.ipa' 2>/dev/null | head -1)"
  [[ -n "$ipa" ]] || { echo "✗ no .ipa to upload; run scripts/testflight.sh first"; exit 1; }
  echo "▸ Uploading $(basename "$ipa") to TestFlight…"
  # altool finds the key by its id in ~/.appstoreconnect/private_keys
  mkdir -p "$HOME/.appstoreconnect/private_keys"
  [[ -f "$HOME/.appstoreconnect/private_keys/AuthKey_$ASC_KEY_ID.p8" ]] || cp "$ASC_KEY_PATH" "$HOME/.appstoreconnect/private_keys/AuthKey_$ASC_KEY_ID.p8"
  xcrun altool --upload-app --type ios --file "$ipa" --apiKey "$ASC_KEY_ID" --apiIssuer "$ASC_ISSUER_ID"
  echo "✓ uploaded. It appears in App Store Connect → TestFlight once Apple has processed it."
}
if [[ "${1:-}" == "upload" ]]; then upload; exit 0; fi

# every upload needs a build number Apple has not seen: one more than the number in project.yml
if [[ "${1:-}" != "archive" ]]; then
  current="$(sed -n 's/^ *CURRENT_PROJECT_VERSION: "\([0-9]*\)"/\1/p' "$APP/project.yml")"
  next=$((current + 1))
  sed -i '' "s/CURRENT_PROJECT_VERSION: \"$current\"/CURRENT_PROJECT_VERSION: \"$next\"/" "$APP/project.yml"
  echo "▸ build number $next"
fi

echo "▸ Rust core, Swift module, Xcode project…"
"$ROOT/scripts/build-apple.sh"

echo "▸ Signing profile for ${BUNDLE_ID}…"
mkdir -p "$BUILD/profiles"
fastlane cert --development false --team_id "$TEAM_ID" --api_key_path <(asc_key_json) --output_path "$BUILD/profiles" >/dev/null
fastlane sigh --force --app_identifier "$BUNDLE_ID" --team_id "$TEAM_ID" --platform ios --api_key_path <(asc_key_json) \
  --output_path "$BUILD/profiles" --filename airband_appstore.mobileprovision >/dev/null
PROFILE="$(security cms -D -i "$BUILD/profiles/airband_appstore.mobileprovision" | plutil -extract Name raw -)"
echo "✓ $PROFILE"

echo "▸ Archiving…"
xcodebuild -project "$APP/Airband.xcodeproj" -scheme Airband -configuration Release -destination 'generic/platform=iOS' \
  -archivePath "$ARCHIVE" DEVELOPMENT_TEAM="$TEAM_ID" CODE_SIGN_STYLE=Manual PROVISIONING_PROFILE_SPECIFIER="$PROFILE" \
  "CODE_SIGN_IDENTITY=Apple Distribution" archive | grep -E "error|warning: .*deprecated|ARCHIVE (SUCCEEDED|FAILED)" || true
[[ -d "$ARCHIVE" ]] || { echo "✗ no archive was made"; exit 1; }
echo "✓ $ARCHIVE"
[[ "${1:-}" == "archive" ]] && exit 0

echo "▸ Exporting…"
cat > "$BUILD/ExportOptions.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>method</key><string>app-store-connect</string>
  <key>teamID</key><string>$TEAM_ID</string>
  <key>signingStyle</key><string>manual</string>
  <key>signingCertificate</key><string>Apple Distribution</string>
  <key>provisioningProfiles</key><dict><key>$BUNDLE_ID</key><string>$PROFILE</string></dict>
  <key>uploadSymbols</key><true/>
  <key>destination</key><string>export</string>
</dict></plist>
PLIST
rm -rf "$BUILD/export"
xcodebuild -exportArchive -archivePath "$ARCHIVE" -exportOptionsPlist "$BUILD/ExportOptions.plist" -exportPath "$BUILD/export" | grep -E "error|EXPORT (SUCCEEDED|FAILED)" || true
IPA="$(find "$BUILD/export" -name '*.ipa' | head -1)"
[[ -n "$IPA" ]] || { echo "✗ no .ipa was exported"; exit 1; }
echo "✓ $IPA"

upload
