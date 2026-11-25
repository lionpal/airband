#!/bin/sh
# Build the `airband` command for one target and pack it for release:
#   scripts/package.sh aarch64-apple-darwin [more targets…]
# leaves two archives of the same binary in dist/, each with its checksum alone in a file of the same name
# plus .sha256, named as scripts/release-files.sh names them:
#   airband-<version>-<target>.tar.gz          the binary, LICENSE and NOTICE, for anyone to download
#   airband-<version>.<tag>.bottle.tar.gz      the same laid out as a Homebrew bottle, which Homebrew installs
#                                              without asking for a compiler or up-to-date developer tools
# The version is the workspace's, in Cargo.toml. The Release workflow runs this; when GitHub has no macOS
# runner to give it is run by hand on a Mac (`rustup target add x86_64-apple-darwin` first), and
# scripts/publish-mac.sh publishes what it made.
set -eu
cd "$(dirname "$0")/.."
version=$(scripts/version.sh)
[ $# -gt 0 ] || { echo "usage: scripts/package.sh <target> [<target>…]" >&2; exit 1; }
# a Mac binary built on a new macOS still runs on an older one
export MACOSX_DEPLOYMENT_TARGET=11.0
# keep macOS's tar from adding its own bookkeeping files
export COPYFILE_DISABLE=1
mkdir -p dist
for target in "$@"; do
  # the two files a release has for it, by which each is: a system it is not released for stops here
  files=$(scripts/release-files.sh "$target")
  plain="dist/$(printf '%s\n' "$files" | awk '$2 == "binary" { print $3 }')"
  bottle="dist/$(printf '%s\n' "$files" | awk '$2 == "bottle" { print $3 }')"
  cargo build --release --locked -p airband-cli --target "$target"
  # where cargo put it: CARGO_TARGET_DIR when set, never a copy an earlier build left in target/
  bin="${CARGO_TARGET_DIR:-target}/$target/release/airband"
  # A Mac binary is signed with the Developer ID certificate, when this Mac has one, and sent to Apple's
  # notary service, when the App Store Connect key is in the environment (ASC_KEY_ID, ASC_ISSUER_ID,
  # ASC_KEY_PATH, as scripts/testflight.sh uses). A bare binary cannot carry the notary's ticket with it, so
  # macOS asks Apple about it the first time it runs. Built where there is no certificate (CI), it is left
  # unsigned, and the release notes say so.
  case "$target" in
    *-apple-darwin)
      scripts/sign-mac.sh "$bin"
      # who signed it: the first Authority field codesign gives is the certificate that signed (the chain follows)
      signer=$(codesign -d --verbose=2 "$bin" 2>&1 | awk -F= '$1 == "Authority" { print $2; exit }')
      case "$signer" in "Developer ID Application:"*) developer_id=yes ;; *) developer_id=no ;; esac
      if [ -n "${ASC_KEY_ID:-}" ] && [ "$developer_id" = yes ]; then
        sent=$(mktemp -d)
        ditto -c -k "$bin" "$sent/airband.zip"
        xcrun notarytool submit "$sent/airband.zip" --key "$ASC_KEY_PATH" --key-id "$ASC_KEY_ID" --issuer "$ASC_ISSUER_ID" --wait --output-format json > "$sent/notary.json"
        cat "$sent/notary.json"
        [ "$(plutil -extract status raw -o - "$sent/notary.json")" = Accepted ] || { echo "Apple's notary service did not accept the $target binary" >&2; exit 1; }
        rm -rf "$sent"
      fi ;;
  esac
  stage=$(mktemp -d)
  keg="$stage/airband/$version"
  mkdir -p "$keg/bin"
  cp "$bin" "$keg/bin/"
  cp LICENSE NOTICE "$keg/"
  tar -czf "$plain" -C "$keg/bin" airband -C "$keg" LICENSE NOTICE
  tar -czf "$bottle" -C "$stage" airband
  rm -rf "$stage"
  for f in "$plain" "$bottle"; do
    shasum -a 256 "$f" | cut -d' ' -f1 > "$f.sha256"
    echo "$f  $(cat "$f.sha256")"
  done
done
