#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

EXTENSION="${EXTENSION:-$(pwd)/modules/gameshark.so}"
if [[ ! -f "$EXTENSION" ]]; then
  echo "extension not found: $EXTENSION" >&2
  exit 1
fi

if [[ -z "${PHP_BIN:-}" && -n "${PHP_CONFIG:-}" ]]; then
  PHP_BIN="$("$PHP_CONFIG" --php-binary 2>/dev/null || true)"
fi
if [[ -z "${PHP_BIN:-}" && -n "${PHP_CONFIG:-}" ]]; then
  PHP_BIN="$(dirname "$PHP_CONFIG")/php"
fi
if [[ -z "${PHP_BIN:-}" ]]; then
  PHP_BIN="$(command -v php || true)"
fi
if [[ -z "$PHP_BIN" || ! -x "$PHP_BIN" ]]; then
  echo "php binary not found; set PHP_BIN=/path/to/php" >&2
  exit 1
fi

"$PHP_BIN" -n -d "extension=$EXTENSION" --ri gameshark >/dev/null
"$PHP_BIN" -n -d "extension=$EXTENSION" -r 'if (!extension_loaded("gameshark") || !gameshark_loaded()) { exit(1); }'

DB_PATH="$(mktemp "${TMPDIR:-/tmp}/gameshark-smoke.XXXXXX.sqlite")"
trap 'rm -f "$DB_PATH"' EXIT

GAMESHARK_DB="$DB_PATH" \
GAMESHARK_TRACE_VALUE="needle" \
"$PHP_BIN" -n -d "extension=$EXTENSION" -r 'function gameshark_smoke($value) { return "prefix " . $value; } gameshark_smoke("needle");'

REPORT="$(
  GAMESHARK_DB="$DB_PATH" "$PHP_BIN" -n -d "extension=$EXTENSION" -r 'echo gameshark_trace_report("json");'
)"

if [[ "$REPORT" != *"gameshark_smoke"* ]]; then
  echo "trace smoke report did not contain gameshark_smoke" >&2
  echo "$REPORT" >&2
  exit 1
fi

printf 'gameshark smoke check passed with %s\n' "$PHP_BIN"

