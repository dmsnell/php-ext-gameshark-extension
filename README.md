# php-gameshark

`gameshark` is a PHP extension for collecting differential and value-tracing
data from PHP executions. The native extension is written in C and delegates
report storage to a Rust library. SQLite is always available; MySQL/MariaDB and
Redis are available in `GAMESHARK_BACKENDS=all` builds.

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

SQLite is the default compiled storage backend. Build with MySQL/MariaDB and
Redis support when needed:

```sh
GAMESHARK_BACKENDS=all PHP_CONFIG=/path/to/php-config scripts/build-unix.sh
```

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

php -d extension=gameshark.so -d gameshark.db=/tmp/run.sqlite -d gameshark.side=left script.php
```

Trace-value mode:

```sh
GAMESHARK_DB=/tmp/trace.sqlite GAMESHARK_TRACE_VALUE=needle php -d extension=gameshark.so script.php
GAMESHARK_DB=/tmp/trace.sqlite php -d extension=gameshark.so -r 'echo gameshark_trace_report("json");'

php -d extension=gameshark.so -d gameshark.db=/tmp/trace.sqlite -d gameshark.trace_value=needle script.php
php -d extension=gameshark.so -d gameshark.db=/tmp/trace.sqlite -r 'echo gameshark_trace_report("json");'
```

Unused runtime coverage mode:

```sh
GAMESHARK_DB=/tmp/unused.sqlite GAMESHARK_UNUSED=1 php -d extension=gameshark.so script.php
GAMESHARK_DB=/tmp/unused.sqlite php -d extension=gameshark.so -r 'echo gameshark_unused_report();'
GAMESHARK_DB=/tmp/unused.sqlite php -d extension=gameshark.so -r 'echo gameshark_unused_report("json");'
GAMESHARK_DB=/tmp/unused.sqlite php -d extension=gameshark.so -r 'echo gameshark_unused_aggregate_report();'
php -d extension=gameshark.so -d gameshark.db=/tmp/unused.sqlite scripts/gameshark-unused-aggregate-report.php json
```

The unused report lists userland functions, concrete methods, classes, constants,
and included files that were loaded during the run but had no matching runtime
access observed. This is request loaded-code coverage, not proof of dead code.
The default report selects the latest completed unused run; pass a run id as the
second argument to inspect an earlier run.

For web traffic sampling, reuse the same SQLite database across many
instrumented requests and use the aggregate helper to merge completed runs into
one probabilistic coverage profile. The aggregate text and JSON reports are
complete; use `GAMESHARK_COLOR=always` with the aggregate text command when
writing ANSI output for `less -R`.

Storage can also be configured with INI settings:

```sh
php -d extension=gameshark.so \
  -d gameshark.storage=sqlite \
  -d gameshark.dsn=sqlite:/tmp/run.sqlite \
  -d gameshark.trace_value=needle \
  script.php
```

For clustered production sampling, build with `GAMESHARK_BACKENDS=all` and use
a dedicated MySQL/MariaDB database:

```sh
GAMESHARK_STORAGE=mysql
GAMESHARK_DSN='mysql://gameshark:secret@db.example.test:3306/gameshark'
GAMESHARK_CAPTURE=wp-prod-sample
GAMESHARK_UNUSED=1
```

The default MySQL schema mode is `validate`; run the DDL from the top-level
README or set `GAMESHARK_MYSQL_SCHEMA_MODE=auto` only for disposable databases.
Redis is intended for temporary sampling:

```sh
GAMESHARK_STORAGE=redis
GAMESHARK_DSN='redis://127.0.0.1:6379/4'
GAMESHARK_REDIS_KEY_PREFIX=gameshark:dev
GAMESHARK_REDIS_TTL=3600
```

Use `gameshark_storage_status()` to inspect the parsed backend, capture,
compiled backends, redacted target, and configuration errors.

Direct constant syntax and `constant('NAME')` count as value access;
`defined()` checks are recorded as probes only. Included-file sections
distinguish files with declarations that were never accessed from files with no
declarations at all. Opcode-derived constant observations are best effort and
should be treated as runtime coverage signals, not exact value-flow proof.

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
