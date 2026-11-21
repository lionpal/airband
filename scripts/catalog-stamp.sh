#!/bin/sh
# Give the rows under catalog/ a new number when they have changed: catalog/rev holds the number and what
# the rows hashed to when it was given, and the tests fail while the two disagree. Run it after editing a
# row; it does nothing when no row changed.
set -e
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
cargo build -q --release -p airband-cli
"${CARGO_TARGET_DIR:-target}/release/airband" catalog stamp catalog/rev
