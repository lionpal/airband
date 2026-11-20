#!/bin/sh
# Sign the tower's binary on a Mac, so the keychain knows it as the same program from one build to the next
# and gives it its keys without asking (see crates/airband-cli/src/vault.rs). Without a signature the tower
# works the same, with its keys in its settings file.
#
#   scripts/sign-mac.sh [path to the binary]     default: target/release/airband
#
# The certificate is the one named in AIRBAND_SIGN, else the one named in ~/.airband/signing, else the first
# "Developer ID Application" or
# "Apple Distribution" one in the keychain. Does nothing, and says so, when there is none.
set -e
cd "$(dirname "$0")/.."
bin="${1:-target/release/airband}"
who="$AIRBAND_SIGN"
# this computer's own choice, when it has several: one line, the certificate's name
if [ -z "$who" ] && [ -f "$HOME/.airband/signing" ]; then
  who=$(head -1 "$HOME/.airband/signing")
fi
if [ -z "$who" ]; then
  who=$(security find-identity -v -p codesigning | sed -n 's/.*"\(Developer ID Application: [^"]*\)".*/\1/p' | head -1)
fi
if [ -z "$who" ]; then
  who=$(security find-identity -v -p codesigning | sed -n 's/.*"\(Apple Distribution: [^"]*\)".*/\1/p' | head -1)
fi
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
