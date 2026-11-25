#!/bin/sh
# Is every file of this release served from airband.dev, matching the checksum served beside it? For the
# systems given, or for every one (what latest.json may name):
#   scripts/check-release.sh <target> [<target>…]
#   scripts/check-release.sh --all
# Each file is downloaded whole and its checksum worked out here. It lists every file that is not served or
# does not match, and fails; the Release workflow writes latest.json only once this passes for --all, so the
# Homebrew tap (which reads latest.json, then every file's checksum) is never pointed at a release with a file
# missing.
set -eu
cd "$(dirname "$0")/.."
[ $# -gt 0 ] || { echo "usage: scripts/check-release.sh <target> [<target>…] | --all" >&2; exit 1; }
version=$(scripts/version.sh)
# a system it is not released for stops here
files=$(scripts/release-files.sh "$@")
names=$(printf '%s\n' "$files" | awk '{ print $3 }')
here=$(mktemp -d)
trap 'rm -rf "$here"' EXIT
wrong=""
for name in $names; do
  at="https://airband.dev/releases/$version/$name"
  if ! curl -fsS -o "$here/sum" "$at.sha256"; then
    wrong="$wrong
  $name: its checksum is not served"
    continue
  fi
  if ! curl -fsS -o "$here/file" "$at"; then
    wrong="$wrong
  $name: it is not served"
    continue
  fi
  got=$(shasum -a 256 "$here/file" | cut -d' ' -f1)
  if [ "$got" != "$(cat "$here/sum")" ]; then
    wrong="$wrong
  $name: it does not match its checksum"
    continue
  fi
  echo "$name  $got"
done
if [ -n "$wrong" ]; then
  echo "airband $version is not all published:$wrong" >&2
  echo "A Mac file missing is published from a Mac: scripts/package.sh aarch64-apple-darwin x86_64-apple-darwin, then BUCKET=<the site's bucket> scripts/publish-mac.sh." >&2
  exit 1
fi
