#!/bin/sh
# The checks, run on this computer: what CI ran on every push, without spending build minutes on it.
#
#   scripts/check.sh           the linter, then the unit tests of every crate (under a minute once built)
#   scripts/check.sh --full    those, then the end-to-end suites: the relay and its queue (26 checks), agents the
#                              tower runs itself (84), links (10), the paths the audits keep finding faults in
#                              (e2e-scenarios), and the whole tower with real model calls (139, a few minutes, and
#                              it needs `claude` signed in)
#
# `git push` runs the first form by itself (.githooks/pre-push, turned on once with
# `git config core.hooksPath .githooks`). CI on GitHub runs on every push too: every system the tower is meant
# for, the phone build, and the end-to-end suites that need no model.
set -e
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
# the linter first: a warning from it fails the check
cargo clippy -q --workspace --all-targets -- -D warnings
cargo test -q --release -p airband-core -p airband-io -p airband-relay -p airband-radio -p airband-cli
[ "$1" = "--full" ] || exit 0
# The suites run a build with the stand-ins in it (test accounts and addresses read from the environment). It is
# built into a folder of its own, never where the README installs from, so it can never become his copy.
# made whole from the start: the suites start programs in other folders, which find it by this path
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target}/stand-ins"
case "$CARGO_TARGET_DIR" in /*) ;; *) CARGO_TARGET_DIR="$PWD/$CARGO_TARGET_DIR" ;; esac
export CARGO_TARGET_DIR
cargo build -q --release -p airband-cli -p airband-relay --features airband-cli/stand-ins
python3 tools/e2e-relay.py
python3 tools/e2e-held.py
python3 tools/e2e-links.py
python3 tools/e2e-scenarios.py
python3 tools/e2e.py --claude
