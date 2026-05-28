# Release Checklist

1. Build from a clean checkout on Linux and macOS.
2. Run the complete PHPT suite for every supported PHP minor.
3. Run `scripts/smoke-load.sh` against every binary artifact.
4. Build source tarballs with vendored crates:

   ```sh
   scripts/package-source.sh
   ```

5. Build ABI-specific binary tarballs only after tests pass:

   ```sh
   PHP_CONFIG=/path/to/php-config scripts/package-binary-unix.sh
   ```

6. Verify the generated `manifest.json` for each binary artifact.
7. Publish checksums beside every tarball.

## CI matrix

The intended CI matrix is:

- Linux, PHP 8.0 through the latest available PHP 8.x stable.
- macOS, PHP 8.0 through the latest available PHP 8.x stable.
- Non-blocking current or nightly PHP lane.

Each lane should run:

```sh
PHP_CONFIG=/path/to/php-config scripts/build-unix.sh
scripts/package-source.sh --no-vendor
PHP_CONFIG=/path/to/php-config scripts/package-binary-unix.sh
```

