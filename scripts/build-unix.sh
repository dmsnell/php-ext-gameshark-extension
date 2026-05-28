#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

case "$(uname -s)" in
  Linux|Darwin)
    ;;
  *)
    echo "gameshark currently supports Linux and macOS builds only" >&2
    exit 1
    ;;
esac

find_php_config() {
  if [[ -n "${PHP_CONFIG:-}" ]]; then
    printf '%s\n' "$PHP_CONFIG"
    return
  fi

  if command -v php-config >/dev/null 2>&1; then
    command -v php-config
    return
  fi

  if [[ -x ../php-src/.install/bin/php-config ]]; then
    printf '%s\n' "../php-src/.install/bin/php-config"
    return
  fi

  echo "php-config not found; set PHP_CONFIG=/path/to/php-config" >&2
  exit 1
}

PHP_CONFIG="$(find_php_config)"
if [[ ! -x "$PHP_CONFIG" ]]; then
  echo "php-config is not executable: $PHP_CONFIG" >&2
  exit 1
fi

if [[ -z "${PHPIZE:-}" ]]; then
  PHPIZE="$(dirname "$PHP_CONFIG")/phpize"
  if [[ ! -x "$PHPIZE" ]] && command -v phpize >/dev/null 2>&1; then
    PHPIZE="$(command -v phpize)"
  fi
fi
if [[ ! -x "$PHPIZE" ]]; then
  echo "phpize not found; set PHPIZE=/path/to/phpize" >&2
  exit 1
fi

PHP_VERSION_ID="$("$PHP_CONFIG" --vernum)"
if [[ -z "$PHP_VERSION_ID" || "$PHP_VERSION_ID" -lt 80200 ]]; then
  echo "gameshark requires PHP 8.2.0 or newer; php-config reported ${PHP_VERSION_ID:-unknown}" >&2
  exit 1
fi

PHP_INCLUDE_DIR="$("$PHP_CONFIG" --include-dir)"
if [[ ! -r "$PHP_INCLUDE_DIR/Zend/zend_observer.h" ]]; then
  echo "Zend observer header not found at $PHP_INCLUDE_DIR/Zend/zend_observer.h" >&2
  exit 1
fi

PHP_CONFIGURE_OPTIONS="$("$PHP_CONFIG" --configure-options 2>/dev/null || true)"
if [[ "$PHP_CONFIGURE_OPTIONS" == *"--enable-zts"* || "$PHP_CONFIGURE_OPTIONS" == *"--enable-maintainer-zts"* ]]; then
  echo "gameshark currently supports non-ZTS PHP builds only" >&2
  exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo is required to build gameshark" >&2
  exit 1
fi

if [[ -z "${JOBS:-}" ]]; then
  if command -v getconf >/dev/null 2>&1; then
    JOBS="$(getconf _NPROCESSORS_ONLN 2>/dev/null || true)"
  fi
  if [[ -z "${JOBS:-}" && "$(uname -s)" == "Darwin" ]]; then
    JOBS="$(sysctl -n hw.ncpu 2>/dev/null || true)"
  fi
  JOBS="${JOBS:-2}"
fi

"$PHPIZE"
./configure --with-php-config="$PHP_CONFIG" --enable-gameshark
make -j"$JOBS"

if [[ "${RUN_TESTS:-1}" != "0" ]]; then
  make test TESTS="${TESTS:-tests/*.phpt}"
fi

if [[ "${RUN_SMOKE:-1}" != "0" ]]; then
  PHP_CONFIG="$PHP_CONFIG" EXTENSION="$(pwd)/modules/gameshark.so" scripts/smoke-load.sh
fi

printf 'gameshark built successfully: %s\n' "$(pwd)/modules/gameshark.so"
