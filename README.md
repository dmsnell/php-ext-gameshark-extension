# php-gameshark

`gameshark` is a PHP extension for collecting differential and value-tracing
data from PHP executions. The native extension is written in C and delegates
SQLite-backed report storage to a Rust static library.

## Supported build targets

- PHP 8.2 or newer.
- Linux and macOS.
- Non-ZTS PHP builds.
- Rust toolchain with `cargo`.
- PHP development tools for the target PHP binary: `phpize` and `php-config`.

PHP 7.4 and Windows are intentionally out of scope for the current package.
See [SUPPORTED.md](SUPPORTED.md) for details.

## Build from source

```sh
PHP_CONFIG=/path/to/php-config scripts/build-unix.sh
```

The build script runs `phpize`, configures the extension, builds the Rust core
with `cargo build --release --locked`, runs the PHPT suite, and performs a
small load/trace smoke test.

For the local checkout that contains the bundled `php-src` submodule, this is
the usual command:

```sh
PHP_CONFIG=../php-src/.install/bin/php-config scripts/build-unix.sh
```

Load the built module directly:

```sh
php -d extension="$(pwd)/modules/gameshark.so" --ri gameshark
```

## Package

Create a vendored source tarball:

```sh
scripts/package-source.sh
```

Create a binary tarball for the exact PHP ABI that built `modules/gameshark.so`:

```sh
PHP_CONFIG=/path/to/php-config scripts/package-binary-unix.sh
```

Binary packages are convenience artifacts only. They are tied to a specific
PHP minor version, extension ABI, operating system, architecture, debug mode,
and ZTS/NTS setting.

## Runtime configuration

Differential mode:

```sh
GAMESHARK_DB=/tmp/run.sqlite GAMESHARK_SIDE=left php -d extension=gameshark.so script.php
GAMESHARK_DB=/tmp/run.sqlite GAMESHARK_SIDE=right php -d extension=gameshark.so script.php
GAMESHARK_DB=/tmp/run.sqlite php -d extension=gameshark.so -r 'echo gameshark_compare();'
```

Trace-value mode:

```sh
GAMESHARK_DB=/tmp/trace.sqlite GAMESHARK_TRACE_VALUE=needle php -d extension=gameshark.so script.php
GAMESHARK_DB=/tmp/trace.sqlite php -d extension=gameshark.so -r 'echo gameshark_trace_report("json");'
```

Unused runtime coverage mode:

```sh
GAMESHARK_DB=/tmp/unused.sqlite GAMESHARK_UNUSED=1 php -d extension=gameshark.so script.php
GAMESHARK_DB=/tmp/unused.sqlite php -d extension=gameshark.so -r 'echo gameshark_unused_report();'
GAMESHARK_DB=/tmp/unused.sqlite php -d extension=gameshark.so -r 'echo gameshark_unused_report("json");'
```

The unused report lists userland functions, concrete methods, classes, global
constants, and class constants that were declared during the run but had no
matching runtime access observed. This is request loaded-code coverage, not
proof of dead code. The default report selects the latest completed unused run;
pass a run id as the second argument to inspect an earlier run. Human text
output shows the first 50 rows per section, while JSON and array output are
complete. Opcode observations for dynamic names and optimizer-folded constants
are best effort. Direct constant fetches and `defined()` checks are recorded as
pre-dispatch observations, but only successful `constant()` reads count as
constant reads.

Trace limiting can be applied with a Rust regex over canonical function names:

```sh
GAMESHARK_TRACE_ALLOW_PATTERN='^(?:preg_match|WP_HTML_Tag_Processor::next_token)$'
```

Invariant mode is configured by loading a PHP hook file:

```sh
php -d extension=gameshark.so \
  -d gameshark.invariants=1 \
  -d gameshark.invariants_file=/path/to/invariants.php \
  script.php
```
