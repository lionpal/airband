#!/bin/sh
# Every file a release of airband is, one line each, for the systems given, or for every one:
#   scripts/release-files.sh <target> [<target>…]
#   scripts/release-files.sh --all
# Each line is the Rust target, which file it is (`binary`, for anyone, or `bottle`, the same as Homebrew
# wants it), and its name as it is published under releases/<version>/ (its checksum is beside it, at the same
# name plus .sha256). package.sh makes exactly these, publish-mac.sh and the Release workflow publish them, and
# check-release.sh will not let latest.json name a version until every one of them is served.
#
# The version is the workspace's (scripts/version.sh). A system that is not listed here stops it.
set -eu
cd "$(dirname "$0")/.."
version=$(scripts/version.sh)
# each system airband is released for, and Homebrew's name for the oldest version of it the binary runs on (a
# newer one takes the same bottle)
systems='aarch64-apple-darwin arm64_big_sur
x86_64-apple-darwin big_sur
aarch64-unknown-linux-gnu arm64_linux
x86_64-unknown-linux-gnu x86_64_linux'
[ $# -gt 0 ] || { echo "usage: scripts/release-files.sh <target> [<target>…] | --all" >&2; exit 1; }
if [ "$*" = --all ]; then set -- $(printf '%s\n' "$systems" | cut -d' ' -f1); fi
for target in "$@"; do
  tag=$(printf '%s\n' "$systems" | awk -v t="$target" '$1 == t { print $2 }')
  [ -n "$tag" ] || { echo "airband is not released for $target" >&2; exit 1; }
  echo "$target binary airband-$version-$target.tar.gz"
  echo "$target bottle airband-$version.$tag.bottle.tar.gz"
done
