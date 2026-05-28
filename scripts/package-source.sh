#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

WITH_VENDOR=1
for arg in "$@"; do
  case "$arg" in
    --vendor)
      WITH_VENDOR=1
      ;;
    --no-vendor)
      WITH_VENDOR=0
      ;;
    *)
      echo "usage: $0 [--vendor|--no-vendor]" >&2
      exit 1
      ;;
  esac
done

VERSION="${VERSION:-$(awk -F'"' '/PHP_GAMESHARK_VERSION/ { print $2; exit }' php_gameshark.h)}"
if [[ -z "$VERSION" ]]; then
  echo "could not determine version from php_gameshark.h" >&2
  exit 1
fi

STAGE_PARENT="$(mktemp -d "${TMPDIR:-/tmp}/gameshark-src.XXXXXX")"
trap 'rm -rf "$STAGE_PARENT"' EXIT
PACKAGE_ROOT="$STAGE_PARENT/gameshark-$VERSION"

mkdir -p "$PACKAGE_ROOT/rust/src" "$PACKAGE_ROOT/tests" "$PACKAGE_ROOT/docs" "$PACKAGE_ROOT/scripts" dist

cp .gitignore Makefile.frag config.m4 gameshark.c gameshark_core.h php_gameshark.h package.xml README.md SUPPORTED.md "$PACKAGE_ROOT/"
cp rust/Cargo.toml rust/Cargo.lock "$PACKAGE_ROOT/rust/"
cp rust/src/*.rs "$PACKAGE_ROOT/rust/src/"
cp tests/*.phpt "$PACKAGE_ROOT/tests/"
cp docs/*.md "$PACKAGE_ROOT/docs/"
cp scripts/*.sh "$PACKAGE_ROOT/scripts/"

if [[ "$WITH_VENDOR" == "1" ]]; then
  if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo is required to vendor Rust dependencies" >&2
    exit 1
  fi
  (
    cd "$PACKAGE_ROOT"
    mkdir -p .cargo
    cargo vendor --locked --manifest-path rust/Cargo.toml vendor > .cargo/config.toml
  )
fi

ARTIFACT="$(pwd)/dist/gameshark-$VERSION-src.tar.gz"
tar -czf "$ARTIFACT" -C "$STAGE_PARENT" "gameshark-$VERSION"

if command -v sha256sum >/dev/null 2>&1; then
  sha256sum "$ARTIFACT" > "$ARTIFACT.sha256"
elif command -v shasum >/dev/null 2>&1; then
  shasum -a 256 "$ARTIFACT" > "$ARTIFACT.sha256"
fi

printf 'created %s\n' "$ARTIFACT"
