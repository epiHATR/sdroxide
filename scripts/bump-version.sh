#!/usr/bin/env bash
# Bump the workspace package version and open release/vX.Y.Z.md for editing.
#
#   ./scripts/bump-version.sh 1.6.12
#
# Then fill in release/v1.6.12.md, commit, tag v1.6.12, and push the tag.
# The Release workflow turns that file + a commit list into the GitHub notes
# (download/install help stays in release/DOWNLOAD.md, not the release body).
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <major.minor.patch>" >&2
  echo "  current: $(sed -n 's/^version = \"\(.*\)\"/\1/p' Cargo.toml | head -1)" >&2
  exit 1
fi

NEW=$1
if [[ ! "$NEW" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "version must look like 1.6.12, got: $NEW" >&2
  exit 1
fi

TAG="v${NEW}"
NOTES="release/${TAG}.md"
OLD=$(sed -n 's/^version = \"\(.*\)\"/\1/p' Cargo.toml | head -1)

if [[ "$OLD" == "$NEW" ]]; then
  echo "Cargo.toml is already $NEW" >&2
else
  # workspace.package.version only — crate manifests use version.workspace.
  if [[ "$(uname -s)" == Darwin ]]; then
    sed -i '' "s/^version = \"${OLD}\"/version = \"${NEW}\"/" Cargo.toml
  else
    sed -i "s/^version = \"${OLD}\"/version = \"${NEW}\"/" Cargo.toml
  fi
  echo "Cargo.toml: $OLD → $NEW"
fi

mkdir -p release
if [[ -f "$NOTES" ]]; then
  echo "already exists: $NOTES (left unchanged)"
else
  cat > "$NOTES" <<EOF
## What's new

- 

EOF
  echo "created $NOTES — edit it before you tag ${TAG}"
fi

echo
echo "Next:"
echo "  1. Edit $NOTES with operator-facing highlights"
echo "  2. Commit Cargo.toml + $NOTES"
echo "  3. git tag ${TAG} && git push origin ${TAG}"
