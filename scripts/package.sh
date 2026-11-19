#!/bin/sh
# Build the `airband` command for one target and pack it for release:
#   scripts/package.sh aarch64-apple-darwin [more targets…]
# leaves dist/airband-<version>-<target>.tar.gz and the same name with .sha256 (the checksum alone, which the
# Homebrew formula pins). The version is the workspace's, in Cargo.toml. The Release workflow runs this; it can be
# run by hand on a Mac when GitHub has no macOS runner to give (`rustup target add x86_64-apple-darwin` first).
set -eu
cd "$(dirname "$0")/.."
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
[ -n "$version" ] || { echo "no version in Cargo.toml" >&2; exit 1; }
[ $# -gt 0 ] || { echo "usage: scripts/package.sh <target> [<target>…]" >&2; exit 1; }
# a Mac binary built on a new macOS still runs on an older one
export MACOSX_DEPLOYMENT_TARGET=11.0
mkdir -p dist
for target in "$@"; do
  cargo build --release --locked -p airband-cli --target "$target"
  name="airband-$version-$target"
  stage=$(mktemp -d)
  cp "target/$target/release/airband" LICENSE NOTICE "$stage/"
  tar -czf "dist/$name.tar.gz" -C "$stage" airband LICENSE NOTICE
  rm -rf "$stage"
  shasum -a 256 "dist/$name.tar.gz" | cut -d' ' -f1 > "dist/$name.tar.gz.sha256"
  echo "dist/$name.tar.gz  $(cat "dist/$name.tar.gz.sha256")"
done
