#!/bin/sh
# Sign the tower's binary on a Mac, so the keychain knows it as the same program from one build to the next
# and gives it its keys without asking (see crates/airband-cli/src/vault.rs). Without a signature the tower
# works the same, with its keys in its settings file.
#
#   scripts/sign-mac.sh [path to the binary]     default: target/release/airband
#
# The certificate is the one named in AIRBAND_SIGN, else the one named in ~/.airband/signing, else the one
# "Developer ID Application" certificate in the keychain, else the one "Apple Distribution" certificate. With
# several of a kind and none named it stops and lists them: which to sign with is his choice, not the first
# one found. Does nothing, and says so, when there is none.
set -e
cd "$(dirname "$0")/.."
bin="${1:-target/release/airband}"
who="${AIRBAND_SIGN:-}"
# this computer's own choice, when it has several: the file holds the certificate's name, one line
if [ -z "$who" ] && [ -f "$HOME/.airband/signing" ]; then
  [ "$(grep -c . "$HOME/.airband/signing")" -le 1 ] || { echo "$HOME/.airband/signing names more than one certificate; leave one line" >&2; exit 1; }
  who=$(cat "$HOME/.airband/signing")
fi
# the keychain's signing identities, read by field: "  1) <40 hex> "<name>"": the hash, then the quoted name
identities() {
  security find-identity -v -p codesigning | awk -v kind="$1" '
    $2 ~ /^[0-9A-F]{40}$/ {
      name = $0; sub(/^[^"]*"/, "", name); sub(/"[^"]*$/, "", name)
      if (index(name, kind) == 1) print name
    }'
}
for kind in "Developer ID Application:" "Apple Distribution:"; do
  [ -z "$who" ] || break
  found=$(identities "$kind")
  case "$(printf '%s' "$found" | grep -c .)" in
    0) ;;
    1) who=$found ;;
    *) printf 'More than one "%s" certificate here; name the one to sign with in AIRBAND_SIGN or ~/.airband/signing:\n%s\n' "$kind" "$found" >&2; exit 1 ;;
  esac
done
if [ -z "$who" ]; then
  echo "No signing certificate here: the binary is left as it is, and its keys stay in the settings file."
  exit 0
fi
# A Developer ID signature is made the way Apple's notary service asks: with the hardened runtime and a
# timestamp from Apple. Any other certificate signs plainly (it is for this computer only).
case "$who" in
  "Developer ID Application:"*) codesign --force --options runtime --timestamp --sign "$who" --identifier dev.airband.tower "$bin" ;;
  *) codesign --force --sign "$who" --identifier dev.airband.tower "$bin" ;;
esac
echo "Signed."
