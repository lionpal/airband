#!/usr/bin/env bash
# Send the TestFlight invite of every address on the homepage's waiting list that has none on record: those who
# signed up before invites were set up, and any Apple did not take at the time. Run from this Mac against the
# relay's own table and key; the relay keeps running. Safe to run again: Apple is asked only for those with no
# invite on record, and one Apple already has as a tester is recorded as such.
#
#   scripts/testflight-invite-waiting.sh
#
# Prints how many, never an address. Needs the aws CLI signed in to the relay's account (it hands this run its
# credentials), and the key already in Secrets Manager (scripts/testflight-invite-key.sh).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export AWS_REGION="${AWS_REGION:-us-east-2}"     # where the relay's server and tables are
export AIRBAND_RELAY_DYNAMO=airband-relay         # infra/cdk.mjs: the tables' prefix
export AIRBAND_TESTFLIGHT_SECRET=airband/testflight-invites
eval "$(aws configure export-credentials --format env)"
cd "$ROOT"
cargo run --quiet --release -p airband-relay -- invite-waiting
