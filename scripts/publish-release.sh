#!/bin/sh
# Publish, from dist/, every file a release has for the systems given (scripts/release-files.sh names them;
# scripts/package.sh made them), to s3://$BUCKET/releases/<version>/, which airband.dev serves:
#   BUCKET=<the site's bucket> scripts/publish-release.sh <target> [<target>…]
# The Release workflow runs it for what it built; scripts/publish-mac.sh runs it for the Mac's two systems.
# It never writes latest.json: the workflow does that once scripts/check-release.sh finds every file served.
#
# Nothing goes until every file is in dist/ with a checksum that matches it: a release is never half sent
# because a file was left out. Each checksum goes before its file, so a file is never served without it. A
# file that is already published is never replaced, because the formula pins its checksum; when the one in
# dist/ is another build of it, it says so and leaves the published one.
set -eu
cd "$(dirname "$0")/.."
[ $# -gt 0 ] || { echo "usage: BUCKET=<bucket> scripts/publish-release.sh <target> [<target>…]" >&2; exit 1; }
[ -n "${BUCKET:-}" ] || { echo "BUCKET is to name the site's bucket" >&2; exit 1; }
version=$(scripts/version.sh)
# a system it is not released for stops here
files=$(scripts/release-files.sh "$@")
names=$(printf '%s\n' "$files" | awk '{ print $3 }')
for name in $names; do
  f="dist/$name"
  [ -f "$f" ] && [ -f "$f.sha256" ] || { echo "dist/ has no $name with its checksum: run scripts/package.sh for its system first. Nothing was published." >&2; exit 1; }
  [ "$(shasum -a 256 "$f" | cut -d' ' -f1)" = "$(cat "$f.sha256")" ] || { echo "$f does not match its checksum. Nothing was published." >&2; exit 1; }
done
for name in $names; do
  f="dist/$name"
  at="https://airband.dev/releases/$version/$name"
  code=$(curl -s -o /dev/null -w '%{http_code}' "$at")
  if [ "$code" = 200 ]; then
    if [ "$(curl -fsS "$at.sha256")" = "$(cat "$f.sha256")" ]; then
      echo "$name is already published"
    else
      echo "$name is already published as another build; it was left as it is, and dist/ has a different one"
    fi
    continue
  fi
  aws s3 cp "$f.sha256" "s3://$BUCKET/releases/$version/$name.sha256" --content-type text/plain --cache-control 'public, max-age=300' --only-show-errors
  aws s3 cp "$f" "s3://$BUCKET/releases/$version/$name" --content-type application/gzip --cache-control 'public, max-age=31536000, immutable' --only-show-errors
  echo "published $name  $(cat "$f.sha256")"
done
