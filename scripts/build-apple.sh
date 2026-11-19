#!/bin/sh
# Build the radio core for iPhone, generate its Swift module, and generate the Xcode project.
# Then: open apps/apple/Airband.xcodeproj and run it on a phone, or  scripts/build-apple.sh --device  to
# compile the app for a device from here (unsigned, as a check).
set -e
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
# the oldest iPhone software the app runs on: the C parts of the Rust core are compiled for it too, not for
# whatever this Mac's SDK is
export IPHONEOS_DEPLOYMENT_TARGET=17.0
cargo build --release -p airband-radio --target aarch64-apple-ios
# the generator reads the API out of a host build of the same library
cargo build -p airband-radio
rm -rf apps/apple/Generated && mkdir -p apps/apple/Generated
cargo run -q -p uniffi-bindgen -- generate --library target/debug/libairband_radio.dylib --language swift --out-dir apps/apple/Generated
mv apps/apple/Generated/airband_radioFFI.modulemap apps/apple/Generated/module.modulemap
(cd apps/apple && xcodegen generate --quiet)
if [ "$1" = "--device" ]; then
  # by SDK, not by destination: this works on a Mac that has the iOS SDK but not the device platform download
  (cd apps/apple && xcodebuild -project Airband.xcodeproj -target Airband -configuration Release -sdk iphoneos CODE_SIGNING_ALLOWED=NO SYMROOT="$PWD/build" build)
fi
