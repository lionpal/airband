#!/bin/sh
# Publish the catalogue (the files under catalog/) so apps and towers already installed take it up:
# https://airband.dev/catalog/1/catalog.json and its signature beside it.
#
#   scripts/publish-catalog.sh
#
# Raise the number in catalog/rev first: a copy is only taken when its rev is higher than the one in hand.
# The document is exactly what this build has compiled in, signed with the catalogue key
# (AIRBAND_CATALOG_KEY, or ~/code/keys/airband-catalog.key). Needs the AWS CLI signed in to the account that
# holds the site.
set -e
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
key="${AIRBAND_CATALOG_KEY:-$HOME/code/keys/airband-catalog.key}"
[ -f "$key" ] || { echo "no catalogue key at $key"; exit 2; }
cargo build -q --release -p airband-cli
out="$(mktemp -d)"
target/release/airband catalog export > "$out/catalog.json"
target/release/airband catalog sign "$key" > "$out/catalog.json.sig"
bucket=$(aws cloudformation describe-stack-resources --stack-name AirbandSite --region us-east-1 --query "StackResources[?ResourceType=='AWS::S3::Bucket'].PhysicalResourceId" --output text)
aws s3 cp --only-show-errors --cache-control "public, max-age=300" --content-type application/json "$out/catalog.json" "s3://$bucket/catalog/1/catalog.json"
aws s3 cp --only-show-errors --cache-control "public, max-age=300" --content-type text/plain "$out/catalog.json.sig" "s3://$bucket/catalog/1/catalog.json.sig"
echo "Published rev $(cat catalog/rev). It is served within five minutes."
rm -rf "$out"
