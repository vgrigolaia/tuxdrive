#!/usr/bin/env bash
# Bump the TuxDrive version across every place it's duplicated:
#   - Cargo.toml            [workspace.package] version (all Rust crates inherit it)
#   - frontend/flutter/pubspec.yaml
#   - frontend/flutter/lib/version.dart (kAppVersion, shown in the About tab
#     and compared against GitHub releases by the update checker)
#
# Usage: ./scripts/bump-version.sh 0.1.3
set -euo pipefail

NEW_VERSION="${1:-}"
if [[ -z "$NEW_VERSION" ]]; then
    echo "Usage: $0 <new-version>   (e.g. $0 0.1.3)" >&2
    exit 1
fi
if [[ ! "$NEW_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "error: version must be exactly x.y.z (got: $NEW_VERSION)" >&2
    exit 1
fi

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_DIR"

sed -i "s/^version     = \"[0-9]\+\.[0-9]\+\.[0-9]\+\"/version     = \"${NEW_VERSION}\"/" Cargo.toml
sed -i "s/^version: [0-9]\+\.[0-9]\+\.[0-9]\+/version: ${NEW_VERSION}/" frontend/flutter/pubspec.yaml
sed -i "s/^const String kAppVersion = '[0-9]\+\.[0-9]\+\.[0-9]\+';/const String kAppVersion = '${NEW_VERSION}';/" frontend/flutter/lib/version.dart

echo "Bumped to ${NEW_VERSION} in:"
grep -n "^version" Cargo.toml
grep -n "^version:" frontend/flutter/pubspec.yaml
grep -n "kAppVersion" frontend/flutter/lib/version.dart

echo
echo "Don't forget: CHANGELOG.md entry, then commit + tag (git tag v${NEW_VERSION})."
