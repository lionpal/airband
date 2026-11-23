#!/bin/sh
# Publish the catalogue (the files under catalog/) so apps and towers already installed take it up, with its
# signature beside it:
#
#   https://airband.dev/catalog/catalog.json
#
#   scripts/publish-catalog.sh            publish
#   scripts/publish-catalog.sh --dry-run  make, sign and check it, and upload nothing
#
# Run scripts/catalog-stamp.sh first when a row has changed: a copy is only taken when its rev is higher than
# the one in hand. A published copy runs out after 180 days, so publish again at least that often.
# The document is exactly what this build has compiled in, signed with the catalogue key
# (AIRBAND_CATALOG_KEY, or ~/code/keys/airband-catalog.key). Nothing is uploaded unless every check passes:
# it reads whole, its rev is higher than the copy published now, and the build made here (the one being
# released: no other build is kept working) would take it in that copy's place. Needs the AWS CLI signed in to
# the account that holds the site.
set -e
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
key="${AIRBAND_CATALOG_KEY:-$HOME/code/keys/airband-catalog.key}"
[ -f "$key" ] || { echo "no catalogue key at $key"; exit 2; }
cargo test -q --release -p airband-core catalog > /dev/null || { echo "the catalogue's own tests fail (rows changed without scripts/catalog-stamp.sh?)"; exit 2; }
cargo build -q --release -p airband-cli
airband="${CARGO_TARGET_DIR:-target}/release/airband"
out="$(mktemp -d)"
trap 'rm -rf "$out"' EXIT
"$airband" catalog export > "$out/catalog.json"
"$airband" catalog sign "$key" "$out/catalog.json" > "$out/catalog.json.sig"
# against the copy published now, when there is one: this build, holding it, must take this one in its place
if curl -sf https://airband.dev/catalog/catalog.json -o "$out/before.json"; then
  "$airband" catalog check "$out/catalog.json" "$out/catalog.json.sig" --against "$out/before.json"
else
  "$airband" catalog check "$out/catalog.json" "$out/catalog.json.sig"
fi
if [ "$1" = "--dry-run" ]; then
  echo "Rev $(head -1 catalog/rev) is made, signed and checked. Nothing was uploaded."
  exit 0
fi
bucket=$(aws cloudformation describe-stack-resources --stack-name AirbandSite --region us-east-1 --query "StackResources[?ResourceType=='AWS::S3::Bucket'].PhysicalResourceId" --output text)
aws s3 cp --only-show-errors --cache-control "public, max-age=300" --content-type application/json "$out/catalog.json" "s3://$bucket/catalog/catalog.json"
aws s3 cp --only-show-errors --cache-control "public, max-age=300" --content-type text/plain "$out/catalog.json.sig" "s3://$bucket/catalog/catalog.json.sig"
echo "Published rev $(head -1 catalog/rev). It is served within five minutes."
