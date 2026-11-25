#!/usr/bin/env bash
# Put the App Store Connect key that the relay sends TestFlight invites with into AWS Secrets Manager, where the
# relay reads it (crates/airband-relay/src/testflight.rs; infra/cdk.mjs lets the relay's server read it).
#
#   scripts/testflight-invite-key.sh <AuthKey_….p8> <key id>
#
# The issuer id is read from apps/apple/.env.testflight (ASC_ISSUER_ID), as scripts/testflight.sh reads it.
# The key is the one made for invites alone (App Store Connect → Users and Access → Integrations → Team Keys,
# role App Manager), not the upload key. Run again with a new key to replace it; the relay reads the new one the
# next time Apple refuses the old one, or when it restarts.
#
# Nothing secret is printed or put on a command line: the key goes from its file into a private temporary file
# and from there to AWS, and the file is removed. Needs the aws CLI signed in to the relay's account.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NAME="airband/testflight-invites"   # infra/cdk.mjs: TESTFLIGHT_SECRET
REGION="${AWS_REGION:-us-east-2}"   # where the relay's server is (.github/workflows/deploy-relay.yml)
ENV_FILE="$ROOT/apps/apple/.env.testflight"

[[ $# -eq 2 ]] || { echo "usage: $0 <AuthKey_….p8> <key id>" >&2; exit 1; }
KEY_PATH="$1" KEY_ID="$2"
[[ -f "$KEY_PATH" ]] || { echo "✗ no file at $KEY_PATH" >&2; exit 1; }
[[ "$KEY_ID" =~ ^[A-Z0-9]{10}$ ]] || { echo "✗ a key id is ten capital letters and digits" >&2; exit 1; }
[[ -f "$ENV_FILE" ]] || { echo "✗ no $ENV_FILE to read ASC_ISSUER_ID from" >&2; exit 1; }
# shellcheck disable=SC1090
ISSUER_ID="$(set +u; . "$ENV_FILE" >/dev/null; printf %s "${ASC_ISSUER_ID:-}")"
[[ -n "$ISSUER_ID" ]] || { echo "✗ $ENV_FILE sets no ASC_ISSUER_ID" >&2; exit 1; }

dir="$(mktemp -d)"
trap 'rm -rf "$dir"' EXIT
chmod 700 "$dir"
# the secret's text, written by Python from the files, so the key is never in an argument or the environment
KEY_ID="$KEY_ID" ISSUER_ID="$ISSUER_ID" python3 -I - "$KEY_PATH" "$dir/secret.json" <<'PY'
import json, os, sys
key = open(sys.argv[1]).read()
if "-----BEGIN PRIVATE KEY-----" not in key or "-----END PRIVATE KEY-----" not in key:
    sys.exit("✗ that file is not a .p8 key")
fd = os.open(sys.argv[2], os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
with os.fdopen(fd, "w") as out:
    json.dump({"keyId": os.environ["KEY_ID"], "issuerId": os.environ["ISSUER_ID"], "key": key}, out)
PY

there="$(aws secretsmanager list-secrets --region "$REGION" --filters Key=name,Values="$NAME" --query "length(SecretList[?Name=='$NAME'])" --output text)"
if [[ "$there" == "0" ]]; then
  version="$(aws secretsmanager create-secret --region "$REGION" --name "$NAME" --description "App Store Connect key (App Manager) the airband relay sends TestFlight invites with" --secret-string "file://$dir/secret.json" --query VersionId --output text)"
  echo "✓ made the secret $NAME in $REGION (version $version)"
else
  version="$(aws secretsmanager put-secret-value --region "$REGION" --secret-id "$NAME" --secret-string "file://$dir/secret.json" --query VersionId --output text)"
  echo "✓ replaced the key in $NAME in $REGION (version $version)"
fi
