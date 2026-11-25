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
scripts/publish-release.sh aarch64-apple-darwin x86_64-apple-darwin
scripts/check-release.sh aarch64-apple-darwin x86_64-apple-darwin
