#!/bin/sh
# The checks, run on this computer: what CI ran on every push, without spending build minutes on it.
#
#   scripts/check.sh           the linter (for this Mac, Linux and Windows), then the unit tests of every crate
#                              (under a minute once built)
#   scripts/check.sh --full    those, then the end-to-end suites, side by side: the relay and its queue, the pull,
#                              agents the tower runs itself, links, the paths the audits keep finding faults in
#                              (e2e-scenarios), notifications (e2e-push), the phone's own core in a simulator of its
#                              own (e2e-phone), and the whole tower with real model calls (a few minutes, and it needs
#                              `claude` signed in)
#
# `git push` runs the first form by itself (.githooks/pre-push, turned on once with
# `git config core.hooksPath .githooks`). CI on GitHub runs on every push too: every system the tower is meant
# for, the phone build, and the end-to-end suites that need no model.
set -e
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
# the linter first: a warning from it fails the check
cargo clippy -q --workspace --all-targets -- -D warnings
# The same linter as it reads the code for Linux and for Windows, which CI builds the tower on: code built on
# one system only and reached from code built on every one is caught here, before a push. Read, never linked,
# so the C the crates compile (sqlite, ring) is stood in for by a compiler and an archiver that make empty
# files; the build cache is left out of that, as it would try to compile the C itself.
none=$(mktemp -d "${TMPDIR:-/tmp}/airband-cross.XXXXXX")
trap 'rm -rf "$none"' EXIT
printf '#!/bin/sh\nfor a; do case $a in -out:*) : >"${a#-out:}" ;; *.a|*.lib) : >"$a" ;; esac; done\n' >"$none/ar"
chmod +x "$none/ar"
installed=$(rustup target list --installed)
for target in x86_64-unknown-linux-gnu x86_64-pc-windows-msvc; do
  if ! printf '%s\n' "$installed" | grep -qx "$target"; then
    echo "The Rust target $target is not installed:  rustup target add $target"
    exit 1
  fi
  t=$(printf '%s' "$target" | tr - _)
  env -u RUSTC_WRAPPER "CC_$t=/usr/bin/true" "AR_$t=$none/ar" cargo clippy -q --workspace --all-targets --target "$target" -- -D warnings
done
cargo test -q --release -p airband-core -p airband-io -p airband-relay -p airband-radio -p airband-cli
[ "$1" = "--full" ] || exit 0
# The suites run a build with the stand-ins in it (test accounts and addresses read from the environment). It is
# built into a folder of its own, never where the README installs from, so it can never become his copy.
# made whole from the start: the suites start programs in other folders, which find it by this path
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target}/stand-ins"
case "$CARGO_TARGET_DIR" in /*) ;; *) CARGO_TARGET_DIR="$PWD/$CARGO_TARGET_DIR" ;; esac
export CARGO_TARGET_DIR
cargo build -q --release -p airband-cli -p airband-relay --features airband-cli/stand-ins
# Side by side: each suite has a throwaway home, and its own relay, tower and stand-ins on ports nothing else is on
# (tools/wire.py free_port), so none reaches another's. Each is told apart in what it prints, with how long it took.
logs=$(mktemp -d "${TMPDIR:-/tmp}/airband-suites.XXXXXX")
began=$(date +%s)
for suite in e2e-relay e2e-pull e2e-held e2e-links e2e-scenarios "e2e --claude" e2e-push e2e-phone; do
  name=${suite%% *}
  (
    start=$(date +%s)
    # shellcheck disable=SC2086
    if python3 tools/$name.py ${suite#"$name"} >"$logs/$name.log" 2>&1; then echo ok >"$logs/$name.result"; else echo failed >"$logs/$name.result"; fi
    echo $(( $(date +%s) - start )) >"$logs/$name.seconds"
  ) &
done
wait
failed=0
for name in e2e-relay e2e-pull e2e-held e2e-links e2e-scenarios e2e e2e-push e2e-phone; do
  result=$(cat "$logs/$name.result")
  printf '%-14s %-6s %4ss  %s\n' "$name" "$result" "$(cat "$logs/$name.seconds")" "$logs/$name.log"
  if [ "$result" != ok ]; then failed=1; tail -n 30 "$logs/$name.log"; fi
done
echo "all suites: $(( $(date +%s) - began ))s"
exit $failed
