#!/usr/bin/env bash
# Build the iPhone app and send it to TestFlight.
#
#   scripts/testflight.sh            build number = one more than the last, then archive, export, upload
#   scripts/testflight.sh archive    stop after the archive (a check that it builds and signs)
#   scripts/testflight.sh upload     upload the .ipa already exported, without rebuilding
#
# Needs, once:
#   - a distribution certificate and its private key in the login keychain (team 6LYK439Z4F)
#   - fastlane (brew install fastlane) and XcodeGen (brew install xcodegen)
#   - an App Store Connect API key, named in apps/apple/.env.testflight (not in the repository):
#       export ASC_KEY_ID=…  ASC_ISSUER_ID=…  ASC_KEY_PATH=/path/to/AuthKey_….p8
#   - the app record for dev.airband.app in App Store Connect. Apple gives no way to create one except by hand:
#     appstoreconnect.apple.com → Apps → + → New App. Until it exists the upload is refused.
#
# The app has two parts, each signed with its own profile: the app (dev.airband.app) and its notification
# extension (dev.airband.app.notify). scripts/apple-ids.py registers both with Apple and turns on push
# notifications; it is run here each time and changes nothing once that is done.
# One thing has to be ticked by hand, once, for a notification to carry the agent's badge as its picture:
# developer.apple.com → Certificates, Identifiers & Profiles → Identifiers → dev.airband.app → Communication
# Notifications. This script sees whether it is on and builds accordingly.
# And once, for the folder the app and its extension share: developer.apple.com → Identifiers → App Groups → +
# → group.dev.airband, then tick it under App Groups on dev.airband.app, dev.airband.app.notify and
# dev.airband.app.widgets (the tile reads what it shows from there). Apple's
# API cannot do either. This script stops when a profile does not carry it.
# And once, for a request from an agent to come through a focus he has on: Identifiers → dev.airband.app →
# Time Sensitive Notifications. This script stops when the app's profile does not carry it.
#
# The signing profiles are regenerated each time against whichever distribution certificate this Mac holds, so the
# script does not depend on a profile made on one particular machine.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP="$ROOT/apps/apple"
BUILD="$APP/build"
ARCHIVE="$BUILD/Airband.xcarchive"
TEAM_ID="${TEAM_ID:-6LYK439Z4F}"
BUNDLE_ID="dev.airband.app"
NOTIFY_ID="dev.airband.app.notify"
WIDGETS_ID="dev.airband.app.widgets"
# shellcheck disable=SC1091
[[ -f "$APP/.env.testflight" ]] && source "$APP/.env.testflight"
: "${ASC_KEY_ID:?set ASC_KEY_ID (apps/apple/.env.testflight)}" "${ASC_ISSUER_ID:?set ASC_ISSUER_ID}" "${ASC_KEY_PATH:?set ASC_KEY_PATH}"
export ASC_KEY_ID ASC_ISSUER_ID ASC_KEY_PATH

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
# The archive links the core where that build put it (build-apple.sh reads CARGO_TARGET_DIR the same way), never a
# copy left in the repository's target folder by an earlier build.
RUST_TARGET="${CARGO_TARGET_DIR:-$ROOT/target}"
case "$RUST_TARGET" in /*) ;; *) RUST_TARGET="$ROOT/$RUST_TARGET" ;; esac

echo "▸ Apple's record of the app and its extension…"
python3 "$ROOT/scripts/apple-ids.py"

echo "▸ Signing profiles…"
mkdir -p "$BUILD/profiles"
# Which certificate signs the app: the one named in AIRBAND_IOS_SIGN (set it in apps/apple/.env.testflight when
# this Mac holds several), by the name the keychain shows. The profiles are then made with that certificate
# and no other. Without it, whichever distribution certificate fastlane finds or makes.
SIGN_AS="${AIRBAND_IOS_SIGN:-Apple Distribution}"
WITH_CERT=()
if [[ -n "${AIRBAND_IOS_SIGN:-}" ]]; then
  serial="$(security find-certificate -c "$AIRBAND_IOS_SIGN" -p | openssl x509 -noout -serial | cut -d= -f2)"
  WITH_CERT=(--cert_id "$(python3 "$ROOT/scripts/apple-ids.py" certificate "$serial")")
  echo "✓ signing as $AIRBAND_IOS_SIGN"
else
  fastlane cert --development false --team_id "$TEAM_ID" --api_key_path <(asc_key_json) --output_path "$BUILD/profiles" >/dev/null
fi
profile() { # profile <bundle id> <file>: make it, print its name
  fastlane sigh --force --app_identifier "$1" --team_id "$TEAM_ID" --platform ios --api_key_path <(asc_key_json) "${WITH_CERT[@]}" \
    --output_path "$BUILD/profiles" --filename "$2" >/dev/null
  security cms -D -i "$BUILD/profiles/$2" | plutil -extract Name raw -
}
PROFILE="$(profile "$BUNDLE_ID" airband_appstore.mobileprovision)"
NOTIFY_PROFILE="$(profile "$NOTIFY_ID" airband_notify_appstore.mobileprovision)"
WIDGETS_PROFILE="$(profile "$WIDGETS_ID" airband_widgets_appstore.mobileprovision)"
echo "✓ $PROFILE"
echo "✓ $NOTIFY_PROFILE"
# what the app's profile grants, read as the entitlement it is (its key under Entitlements), never found in its text
security cms -D -i "$BUILD/profiles/airband_appstore.mobileprovision" > "$BUILD/profiles/airband_appstore.plist"
security cms -D -i "$BUILD/profiles/airband_notify_appstore.mobileprovision" > "$BUILD/profiles/airband_notify_appstore.plist"
security cms -D -i "$BUILD/profiles/airband_widgets_appstore.mobileprovision" > "$BUILD/profiles/airband_widgets_appstore.plist"
entitled() { /usr/libexec/PlistBuddy -c "Print :Entitlements:$1" "$BUILD/profiles/${2:-airband_appstore}.plist" >/dev/null 2>&1; }
entitled aps-environment || { echo "✗ the app's profile has no push notifications"; exit 1; }
# the shared folder: the group the entitlements name, as one of the profile's groups (read as the list it is)
in_group() { python3 -c 'import plistlib, sys; sys.exit("group.dev.airband" not in plistlib.load(open(sys.argv[1], "rb")).get("Entitlements", {}).get("com.apple.security.application-groups", []))' "$BUILD/profiles/$1.plist"; }
in_group airband_appstore || { echo "✗ the app's profile does not have the App Group group.dev.airband (see the top of this script)"; exit 1; }
in_group airband_notify_appstore || { echo "✗ the notification extension's profile does not have the App Group group.dev.airband (see the top of this script)"; exit 1; }
in_group airband_widgets_appstore || { echo "✗ the widgets' profile does not have the App Group group.dev.airband (see the top of this script)"; exit 1; }
# the car's screen: the capability is a tick box on the identifier at developer.apple.com (Capability Requests)
entitled com.apple.developer.carplay-voice-based-conversation || { echo "✗ the app's profile has no CarPlay voice conversation capability"; exit 1; }
entitled com.apple.developer.usernotifications.time-sensitive || { echo "✗ the app's profile has no Time Sensitive Notifications (see the top of this script)"; exit 1; }
if entitled com.apple.developer.usernotifications.communication; then
  COMMUNICATION=(AIRBAND_COMMUNICATION=YES AIRBAND_COMMUNICATION_SUFFIX=-communication)
  echo "✓ Communication Notifications is on: a notification carries the agent's badge"
else
  COMMUNICATION=(AIRBAND_COMMUNICATION=NO)
  echo "· Communication Notifications is off for $BUNDLE_ID: a notification carries the agent's name and no badge (see the top of this script)"
fi

echo "▸ Archiving…"
# an archive left from an earlier run must never pass for this one
rm -rf "$ARCHIVE"
# the build's own status decides, not the filter on its output (pipefail is on; grep finding nothing is not a failure)
xcodebuild -project "$APP/Airband.xcodeproj" -scheme Airband -configuration Release -destination 'generic/platform=iOS' \
  -archivePath "$ARCHIVE" AIRBAND_RUST_TARGET="$RUST_TARGET" DEVELOPMENT_TEAM="$TEAM_ID" CODE_SIGN_STYLE=Manual AIRBAND_APP_PROFILE="$PROFILE" AIRBAND_NOTIFY_PROFILE="$NOTIFY_PROFILE" AIRBAND_WIDGETS_PROFILE="$WIDGETS_PROFILE" "${COMMUNICATION[@]}" \
  "CODE_SIGN_IDENTITY=$SIGN_AS" archive | { grep -E "error|warning: .*deprecated|ARCHIVE (SUCCEEDED|FAILED)" || true; }
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
  <key>signingCertificate</key><string>$SIGN_AS</string>
  <key>provisioningProfiles</key><dict><key>$BUNDLE_ID</key><string>$PROFILE</string><key>$NOTIFY_ID</key><string>$NOTIFY_PROFILE</string><key>$WIDGETS_ID</key><string>$WIDGETS_PROFILE</string></dict>
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
