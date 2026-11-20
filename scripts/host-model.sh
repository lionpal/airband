#!/bin/sh
# Put a model's files where phones fetch them: https://airband.dev/models/<name>/<version>/…
#
#   scripts/host-model.sh <folder of the files as the phone runs them> <name> <version>
#
# Prints each file's path, size and SHA-256 as a row for `MODELS` in crates/airband-radio/src/models.rs. A
# version, once phones have it, is never changed: a different file is a new version. Needs the AWS CLI signed
# in to the account that holds the site.
set -e
from="$1"; name="$2"; version="$3"
[ -d "$from" ] && [ -n "$name" ] && [ -n "$version" ] || { echo "usage: $0 <folder> <name> <version>"; exit 2; }
bucket=$(aws cloudformation describe-stack-resources --stack-name AirbandSite --region us-east-1 --query "StackResources[?ResourceType=='AWS::S3::Bucket'].PhysicalResourceId" --output text)
find "$from" -name '._*' -delete
aws s3 cp --recursive --only-show-errors --cache-control "public, max-age=31536000, immutable" "$from" "s3://$bucket/models/$name/$version/"
(cd "$from" && find . -type f | sed 's|^\./||' | sort | while read -r f; do
  printf '            ("%s", %s, "%s"),\n' "$f" "$(stat -f %z "$f")" "$(shasum -a 256 "$f" | cut -d' ' -f1)"
done)
echo "base: \"https://airband.dev/models/$name/$version\""
