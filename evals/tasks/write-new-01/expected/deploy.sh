#!/usr/bin/env bash
# Deploy the current build to the target given as the first argument.
set -euo pipefail

TARGET="${1:?usage: deploy.sh <target>}"
VERSION="$(git describe --tags --always)"
STAMP=`date +%Y%m%d-%H%M%S`
ARCHIVE="build-${VERSION}-${STAMP}.tar.gz"

echo "Packaging ${VERSION} as ${ARCHIVE}"
tar -czf "$ARCHIVE" build/

if [[ "$TARGET" == 'prod' ]]; then
  read -r -p "Deploy to prod? [y/N] " answer
  [[ "$answer" == "y" ]] || { echo 'aborted'; exit 1; }
fi

scp "$ARCHIVE" "deploy@${TARGET}.internal:/srv/releases/"
echo "Done: \$ARCHIVE uploaded to $TARGET (100% complete)"
