#!/bin/sh
# The version airband is released as: the `airband-cli` package's, as cargo reads it from the workspace (never
# a line matched out of Cargo.toml). Exactly one line, or it stops.
set -eu
cd "$(dirname "$0")/.."
cargo metadata --no-deps --format-version 1 | python3 -I -c '
import json, sys
found = [p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "airband-cli"]
if len(found) != 1:
    sys.exit(f"cargo lists {len(found)} airband-cli packages")
print(found[0])
'
