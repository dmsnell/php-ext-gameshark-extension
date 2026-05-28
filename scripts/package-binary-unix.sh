#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

case "$(uname -s)" in
  Linux|Darwin)
    ;;
  *)
    echo "binary packaging currently supports Linux and macOS only" >&2
    exit 1
    ;;
esac

VERSION="${VERSION:-$(awk -F'"' '/PHP_GAMESHARK_VERSION/ { print $2; exit }' php_gameshark.h)}"
EXTENSION="${EXTENSION:-$(pwd)/modules/gameshark.so}"
if [[ ! -f "$EXTENSION" ]]; then
  echo "extension not found: $EXTENSION" >&2
  exit 1
fi

if [[ -z "${PHP_CONFIG:-}" ]]; then
  if command -v php-config >/dev/null 2>&1; then
    PHP_CONFIG="$(command -v php-config)"
  elif [[ -x ../php-src/.install/bin/php-config ]]; then
    PHP_CONFIG="../php-src/.install/bin/php-config"
  else
    echo "php-config not found; set PHP_CONFIG=/path/to/php-config" >&2
    exit 1
  fi
fi

if [[ -z "${PHP_BIN:-}" ]]; then
  PHP_BIN="$("$PHP_CONFIG" --php-binary 2>/dev/null || true)"
fi
if [[ -z "${PHP_BIN:-}" ]]; then
  PHP_BIN="$(dirname "$PHP_CONFIG")/php"
fi
if [[ ! -x "$PHP_BIN" ]]; then
  echo "php binary not found; set PHP_BIN=/path/to/php" >&2
  exit 1
fi

PHP_VERSION="$("$PHP_CONFIG" --version)"
PHP_MINOR="$(printf '%s\n' "$PHP_VERSION" | awk -F. '{ print $1 "." $2 }')"
PHP_VERSION_ID="$("$PHP_CONFIG" --vernum)"
PHP_EXTENSION_DIR="$("$PHP_CONFIG" --extension-dir)"
PHP_INFO="$("$PHP_BIN" -i)"
PHP_API="$(awk -F'=> ' '/^PHP API/ { gsub(/^[ \t]+|[ \t]+$/, "", $2); print $2; exit }' <<< "$PHP_INFO")"
THREAD_SAFETY="$(awk -F'=> ' '/^Thread Safety/ { gsub(/^[ \t]+|[ \t]+$/, "", $2); print $2; exit }' <<< "$PHP_INFO")"
DEBUG_BUILD="$(awk -F'=> ' '/^Debug Build/ { gsub(/^[ \t]+|[ \t]+$/, "", $2); print $2; exit }' <<< "$PHP_INFO")"

if (( PHP_VERSION_ID < 80200 )); then
  echo "gameshark binary packages require PHP 8.2 or newer; got $PHP_VERSION" >&2
  exit 1
fi

if [[ "$THREAD_SAFETY" == "enabled" ]]; then
  echo "gameshark does not support ZTS PHP builds" >&2
  exit 1
else
  THREAD_TAG="nts"
fi
if [[ "$DEBUG_BUILD" == "yes" ]]; then
  DEBUG_TAG="debug"
else
  DEBUG_TAG="nodebug"
fi

OS_TAG="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH_TAG="$(uname -m)"
PACKAGE_NAME="gameshark-$VERSION-php$PHP_MINOR-$OS_TAG-$ARCH_TAG-$DEBUG_TAG-$THREAD_TAG"
STAGE_PARENT="$(mktemp -d "${TMPDIR:-/tmp}/gameshark-bin.XXXXXX")"
trap 'rm -rf "$STAGE_PARENT"' EXIT
PACKAGE_ROOT="$STAGE_PARENT/$PACKAGE_NAME"

if ! "$PHP_BIN" -n -d "extension=$EXTENSION" --ri gameshark >/dev/null 2>&1; then
  echo "selected PHP binary cannot load $EXTENSION; rebuild with matching php-config/PHP binary" >&2
  exit 1
fi

mkdir -p "$PACKAGE_ROOT" dist
cp "$EXTENSION" "$PACKAGE_ROOT/gameshark.so"

cat > "$PACKAGE_ROOT/manifest.json" <<JSON
{
  "name": "gameshark",
  "version": "$VERSION",
  "php_version": "$PHP_VERSION",
  "php_version_id": "$PHP_VERSION_ID",
  "php_minor": "$PHP_MINOR",
  "php_api": "$PHP_API",
  "php_extension_dir": "$PHP_EXTENSION_DIR",
  "os": "$OS_TAG",
  "arch": "$ARCH_TAG",
  "debug": "$DEBUG_TAG",
  "thread_safety": "$THREAD_TAG",
  "binary": "gameshark.so"
}
JSON

cat > "$PACKAGE_ROOT/README-install.md" <<EOF
# gameshark binary package

This package was built for:

- PHP: $PHP_VERSION
- PHP API: $PHP_API
- OS: $OS_TAG
- Architecture: $ARCH_TAG
- Debug: $DEBUG_TAG
- Thread safety: $THREAD_TAG

Install only into a PHP build with matching ABI metadata. To test without
installing:

\`\`\`sh
php -d extension=/absolute/path/to/gameshark.so --ri gameshark
\`\`\`

For permanent installation, copy \`gameshark.so\` into the matching PHP
extension directory and add this to the target ini file:

\`\`\`ini
extension=gameshark.so
\`\`\`
EOF

(
  cd "$PACKAGE_ROOT"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum gameshark.so manifest.json README-install.md > SHA256SUMS
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 gameshark.so manifest.json README-install.md > SHA256SUMS
  fi
)

ARTIFACT="$(pwd)/dist/$PACKAGE_NAME.tar.gz"
tar -czf "$ARTIFACT" -C "$STAGE_PARENT" "$PACKAGE_NAME"

if command -v sha256sum >/dev/null 2>&1; then
  sha256sum "$ARTIFACT" > "$ARTIFACT.sha256"
elif command -v shasum >/dev/null 2>&1; then
  shasum -a 256 "$ARTIFACT" > "$ARTIFACT.sha256"
fi

printf 'created %s\n' "$ARTIFACT"
