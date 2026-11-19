#!/bin/sh
# Build the `airband` command for one target and pack it for release:
#   scripts/package.sh aarch64-apple-darwin [more targets…]
# leaves two archives of the same binary in dist/, each with its checksum alone in a file of the same name
# plus .sha256:
#   airband-<version>-<target>.tar.gz          the binary, LICENSE and NOTICE, for anyone to download
#   airband-<version>.<tag>.bottle.tar.gz      the same laid out as a Homebrew bottle, which Homebrew installs
#                                              without asking for a compiler or up-to-date developer tools
# The version is the workspace's, in Cargo.toml. The Release workflow runs this; it can be run by hand on a Mac
# when GitHub has no macOS runner to give (`rustup target add x86_64-apple-darwin` first).
set -eu
cd "$(dirname "$0")/.."
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
[ -n "$version" ] || { echo "no version in Cargo.toml" >&2; exit 1; }
[ $# -gt 0 ] || { echo "usage: scripts/package.sh <target> [<target>…]" >&2; exit 1; }
# a Mac binary built on a new macOS still runs on an older one
export MACOSX_DEPLOYMENT_TARGET=11.0
# keep macOS's tar from adding its own bookkeeping files
export COPYFILE_DISABLE=1
mkdir -p dist
for target in "$@"; do
  # Homebrew's name for the oldest system the binary runs on; a newer macOS takes the same bottle
  case "$target" in
    aarch64-apple-darwin) tag=arm64_big_sur ;;
    x86_64-apple-darwin) tag=big_sur ;;
    aarch64-unknown-linux-gnu) tag=arm64_linux ;;
    x86_64-unknown-linux-gnu) tag=x86_64_linux ;;
    *) echo "no Homebrew name is known for $target" >&2; exit 1 ;;
  esac
  cargo build --release --locked -p airband-cli --target "$target"
  stage=$(mktemp -d)
  keg="$stage/airband/$version"
  mkdir -p "$keg/bin"
  cp "target/$target/release/airband" "$keg/bin/"
  cp LICENSE NOTICE "$keg/"
  plain="dist/airband-$version-$target.tar.gz"
  bottle="dist/airband-$version.$tag.bottle.tar.gz"
  tar -czf "$plain" -C "$keg/bin" airband -C "$keg" LICENSE NOTICE
  tar -czf "$bottle" -C "$stage" airband
  rm -rf "$stage"
  for f in "$plain" "$bottle"; do
    shasum -a 256 "$f" | cut -d' ' -f1 > "$f.sha256"
    echo "$f  $(cat "$f.sha256")"
  done
done
