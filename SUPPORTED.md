# Supported Targets

## Current package target

| Target | Status | Notes |
| --- | --- | --- |
| PHP 8.2+ | Supported | Requires declaration observer APIs used by unused coverage mode. |
| PHP 8.0-8.1 | Unsupported | These versions lack declaration observer APIs used by unused coverage mode. |
| PHP 7.4 | Unsupported | PHP 7.4 lacks the observer API this extension uses. |
| Linux | Supported | Source and binary package scripts are provided. |
| macOS | Supported | Source and binary package scripts are provided. |
| Windows | Deferred | Requires a separate `config.w32`, PHP SDK build lane, and Rust MSVC target validation. |
| Non-ZTS PHP | Supported | This is the current build target. |
| ZTS PHP | Unsupported | Request state currently lives in process globals and needs a ZTS globals refactor. |

## Compatibility policy

The extension fails closed during `configure` when a target is outside the
supported surface:

- `php-config --vernum` must report PHP 8.2.0 or newer.
- `Zend/zend_observer.h` must exist in the target PHP include directory.
- The target PHP build must not be ZTS.
- The host OS must be Linux or macOS.
- `cargo` must be available.

The Rust dependency graph is locked with `rust/Cargo.lock`. Source release
tarballs should vendor crates so a user can build without network access.

## Binary compatibility

`gameshark.so` is not portable across arbitrary PHP installations. Binary
artifacts must be built and named per PHP ABI:

```text
gameshark-<version>-php<major.minor>-<os>-<arch>-<debug>-<nts|zts>.tar.gz
```

The package metadata includes the PHP version, PHP API number, extension
directory, OS, architecture, debug flag, and thread-safety flag used for the
build. Users should prefer source builds unless they are installing into an
identical PHP ABI.

## Deferred Windows work

Windows support should be treated as a separate project:

- Add and maintain `config.w32`.
- Replace Unix-only headers and process calls with portable equivalents.
- Build with the PHP SDK and the same Visual Studio toolset as the target PHP.
- Build the Rust static library for the matching MSVC target.
- Test NTS and ZTS separately if ZTS support is added.
