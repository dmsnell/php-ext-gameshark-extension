# Installing php-gameshark

## Requirements

- Linux or macOS.
- PHP 8.2 or newer, non-ZTS.
- `phpize` and `php-config` for the PHP binary that will load the extension.
- Rust and `cargo`.

## Build

From the extension source directory:

```sh
PHP_CONFIG=/path/to/php-config scripts/build-unix.sh
```

The script derives `phpize` from `PHP_CONFIG` when possible. Set `PHPIZE`
explicitly if they are not installed in the same directory:

```sh
PHP_CONFIG=/opt/php/8.3/bin/php-config \
PHPIZE=/opt/php/8.3/bin/phpize \
scripts/build-unix.sh
```

Skip tests or the smoke check only when packaging infrastructure already runs
them elsewhere:

```sh
RUN_TESTS=0 RUN_SMOKE=0 PHP_CONFIG=/path/to/php-config scripts/build-unix.sh
```

## Load

Use an absolute path for ad-hoc testing:

```sh
php -d extension=/path/to/php-gameshark/modules/gameshark.so --ri gameshark
```

For installation into the target PHP extension directory:

```sh
make install
php --ini
```

Then add this to the appropriate ini file:

```ini
extension=gameshark.so
```

## Source package

Create a source tarball with vendored Rust crates:

```sh
scripts/package-source.sh
```

The artifact is written to `dist/`.

Use `--no-vendor` only for CI jobs or development environments that are allowed
to resolve crates from the network:

```sh
scripts/package-source.sh --no-vendor
```

## Binary package

After a successful build:

```sh
PHP_CONFIG=/path/to/php-config scripts/package-binary-unix.sh
```

The artifact contains `gameshark.so`, `manifest.json`, `README-install.md`,
and checksum files. It is only safe for a PHP installation with the same ABI
metadata recorded in the manifest.
