#!/bin/sh
# Publish the Mac's files of a release from this Mac, when the Release workflow was run with "skip macOS":
#   scripts/package.sh aarch64-apple-darwin x86_64-apple-darwin
#   BUCKET=<the site's bucket> scripts/publish-mac.sh
# Every Mac file package.sh makes goes (both systems' binaries and both Homebrew bottles, each checksum before
# its file, with the workflow's cache-control: scripts/publish-release.sh), then each is checked as airband.dev
# serves it. latest.json is not written here: run the workflow again, which finds these published, checks
# every file of the release is served, and only then names the version as the latest.
set -eu
cd "$(dirname "$0")/.."
# Only a binary signed with the Developer ID certificate goes out: one built without it (as on a GitHub runner)
# would be published unsigned, and a published file is never replaced.
version=$(scripts/version.sh)
for target in aarch64-apple-darwin x86_64-apple-darwin; do
  look=$(mktemp -d)
  tar -xzf "dist/airband-$version-$target.tar.gz" -C "$look" airband
  signer=$(codesign -d --verbose=2 "$look/airband" 2>&1 | awk -F= '$1 == "Authority" { print $2; exit }')
  rm -rf "$look"
  case "$signer" in
    "Developer ID Application:"*) ;;
    *) echo "airband-$version-$target is not signed with the Developer ID certificate; nothing was published" >&2; exit 1 ;;
  esac
done
scripts/publish-release.sh aarch64-apple-darwin x86_64-apple-darwin
scripts/check-release.sh aarch64-apple-darwin x86_64-apple-darwin
