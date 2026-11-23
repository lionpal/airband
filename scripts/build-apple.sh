#!/bin/sh
# Build the radio core for iPhone, generate its Swift module, and generate the Xcode project.
# Then: open apps/apple/Airband.xcodeproj and run it on a phone, or  scripts/build-apple.sh --device  to
# compile the app for a device from here (unsigned, as a check), or  --simulator  for the simulator.
set -e
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
# the oldest iPhone software the app runs on: the C parts of the Rust core are compiled for it too, not for
# whatever this Mac's SDK is
export IPHONEOS_DEPLOYMENT_TARGET=18.0
# where cargo builds, whole: the generator and the app read the libraries from there, not from a stale target/
rust="${CARGO_TARGET_DIR:-target}"
case "$rust" in /*) ;; *) rust="$PWD/$rust" ;; esac
cargo build --release -p airband-radio --target aarch64-apple-ios
# and for the simulator, which the project links when it builds for one
cargo build --release -p airband-radio --target aarch64-apple-ios-sim
# the generator reads the API out of a host build of the same library
cargo build -p airband-radio
rm -rf apps/apple/Generated && mkdir -p apps/apple/Generated
cargo run -q -p uniffi-bindgen -- generate --library "$rust/debug/libairband_radio.dylib" --language swift --out-dir apps/apple/Generated
# one Swift file and one C module for each crate whose types the phone sees (the core's, and the phone core's)
cat apps/apple/Generated/*FFI.modulemap > apps/apple/Generated/module.modulemap
rm apps/apple/Generated/*FFI.modulemap
(cd apps/apple && xcodegen generate --quiet)
if [ "$1" = "--device" ]; then
  # by SDK, not by destination: this works on a Mac that has the iOS SDK but not the device platform download
  (cd apps/apple && xcodebuild -project Airband.xcodeproj -target Airband -configuration Release -sdk iphoneos CODE_SIGNING_ALLOWED=NO SYMROOT="$PWD/build" AIRBAND_RUST_TARGET="$rust" build)
elif [ "$1" = "--simulator" ]; then
  (cd apps/apple && xcodebuild -project Airband.xcodeproj -target Airband -configuration Debug -sdk iphonesimulator ARCHS=arm64 CODE_SIGNING_ALLOWED=NO SYMROOT="$PWD/build" AIRBAND_RUST_TARGET="$rust" build)
fi
