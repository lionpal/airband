#!/usr/bin/env bash
# Put the admin page's one allowed address and its password into AWS Secrets Manager, where the relay reads them at
# each sign-in (crates/airband-relay/src/admin.rs; infra/cdk.mjs lets the relay's server read the secret).
#
#   scripts/admin-password.sh
#
# Asks for the address and the password (twice); neither is ever printed, put on a command line or in the
# environment. The password is kept only as PBKDF2-HMAC-SHA256 with 600,000 rounds and a random salt. Run it again
# to change either: the next sign-in uses the new one. Signing in once more is all a change needs.
#
# Needs python3 and the aws CLI signed in to the relay's account.
set -euo pipefail
NAME="airband/admin"                # infra/cdk.mjs: ADMIN_SECRET
REGION="${AWS_REGION:-us-east-2}"   # where the relay's server is (.github/workflows/deploy-relay.yml)

dir="$(mktemp -d)"
trap 'rm -rf "$dir"' EXIT
chmod 700 "$dir"
# the secret's text, written by Python from what was typed, straight into a private file
python3 -I - "$dir/secret.json" <<'PY'
import getpass, hashlib, json, os, re, secrets, sys
email = input("Email allowed into the admin page: ").strip().lower()
if not re.fullmatch(r"[^@\s]+@[^@\s]+\.[^@\s]+", email):
    sys.exit("✗ that is not an email address")
password = getpass.getpass("Password: ")
if len(password) < 12:
    sys.exit("✗ use at least 12 characters")
if getpass.getpass("The same password again: ") != password:
    sys.exit("✗ the two passwords are not the same")
rounds, salt = 600_000, secrets.token_bytes(16)
hashed = hashlib.pbkdf2_hmac("sha256", password.encode(), salt, rounds, dklen=32)
fd = os.open(sys.argv[1], os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
with os.fdopen(fd, "w") as out:
    json.dump({"email": email, "password": {"iterations": rounds, "salt": salt.hex(), "hash": hashed.hex()}}, out)
PY

there="$(aws secretsmanager list-secrets --region "$REGION" --filters Key=name,Values="$NAME" --query "length(SecretList[?Name=='$NAME'])" --output text)"
if [[ "$there" == "0" ]]; then
  version="$(aws secretsmanager create-secret --region "$REGION" --name "$NAME" --description "The one address allowed into the airband admin page, and its hashed password" --secret-string "file://$dir/secret.json" --query VersionId --output text)"
  echo "✓ made the secret $NAME in $REGION (version $version)"
else
  version="$(aws secretsmanager put-secret-value --region "$REGION" --secret-id "$NAME" --secret-string "file://$dir/secret.json" --query VersionId --output text)"
  echo "✓ replaced the address and password in $NAME in $REGION (version $version)"
fi
