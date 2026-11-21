#!/bin/sh
# The checks, run on this computer: what CI ran on every push, without spending build minutes on it.
#
#   scripts/check.sh           the unit tests of every crate (under a minute once built)
#   scripts/check.sh --full    those, then the three end-to-end suites: the relay and its queue (22 checks), agents
#                              the tower runs itself (30), and the whole tower with real model calls (125, a few
#                              minutes, and it needs `claude` signed in)
#
# `git push` runs the first form by itself (.githooks/pre-push, turned on once with
# `git config core.hooksPath .githooks`). CI on GitHub runs on every push too: every system the tower is meant
# for, the phone build, and the end-to-end suites that need no model.
set -e
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
cargo test -q --release -p airband-core -p airband-relay -p airband-radio -p airband-cli
[ "$1" = "--full" ] || exit 0
cargo build -q --release -p airband-cli -p airband-relay
python3 tools/e2e-relay.py
python3 tools/e2e-held.py
python3 tools/e2e.py --claude
