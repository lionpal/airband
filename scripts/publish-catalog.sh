#!/bin/sh
# Publish the catalogue (the files under catalog/) so apps and towers already installed take it up, in both
# forms of the document, each with its signature beside it:
#
#   https://airband.dev/catalog/2/catalog.json   what current builds read
#   https://airband.dev/catalog/1/catalog.json   the same rows in the older form, for builds from before the
#                                                interfaces were named (the app up to build 83, the tower up
#                                                to 0.2.0); see crates/airband-core/src/catalog_v1.rs
#
#   scripts/publish-catalog.sh            publish
#   scripts/publish-catalog.sh --dry-run  make, sign and check both, and upload nothing
#
# Run scripts/catalog-stamp.sh first when a row has changed: a copy is only taken when its rev is higher than
# the one in hand. A published copy runs out after 180 days, so publish again at least that often.
# The documents are exactly what this build has compiled in, signed with the catalogue key
# (AIRBAND_CATALOG_KEY, or ~/code/keys/airband-catalog.key). Nothing is uploaded unless every check passes:
# each document reads whole in the builds it is for, and neither changes a program or an address a build
# already out has. Needs the AWS CLI signed in to the account that holds the site.
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
mkdir "$out/1" "$out/2"
"$airband" catalog export > "$out/2/catalog.json"
"$airband" catalog export --form 1 > "$out/1/catalog.json"
for form in 1 2; do
  "$airband" catalog sign "$key" "$out/$form/catalog.json" > "$out/$form/catalog.json.sig"
done
# against the copy published before, when there is one: a build that has it must be able to take this one
if curl -sf https://airband.dev/catalog/2/catalog.json -o "$out/before.json"; then
  "$airband" catalog check "$out/2/catalog.json" "$out/2/catalog.json.sig" --against "$out/before.json"
else
  "$airband" catalog check "$out/2/catalog.json" "$out/2/catalog.json.sig"
fi
"$airband" catalog check "$out/1/catalog.json" "$out/1/catalog.json.sig" --form 1
if [ "$1" = "--dry-run" ]; then
  echo "Both forms of rev $(head -1 catalog/rev) are made, signed and checked. Nothing was uploaded."
  exit 0
fi
bucket=$(aws cloudformation describe-stack-resources --stack-name AirbandSite --region us-east-1 --query "StackResources[?ResourceType=='AWS::S3::Bucket'].PhysicalResourceId" --output text)
for form in 2 1; do
  aws s3 cp --only-show-errors --cache-control "public, max-age=300" --content-type application/json "$out/$form/catalog.json" "s3://$bucket/catalog/$form/catalog.json"
  aws s3 cp --only-show-errors --cache-control "public, max-age=300" --content-type text/plain "$out/$form/catalog.json.sig" "s3://$bucket/catalog/$form/catalog.json.sig"
done
echo "Published rev $(head -1 catalog/rev) in both forms. It is served within five minutes."
