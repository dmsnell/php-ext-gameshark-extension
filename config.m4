PHP_ARG_ENABLE([gameshark],
  [whether to enable gameshark support],
  [AS_HELP_STRING([--enable-gameshark],
    [Enable gameshark support])],
  [no])

if test "$PHP_GAMESHARK" != "no"; then
  if test -z "$PHP_CONFIG" || test ! -x "$PHP_CONFIG"; then
    AC_MSG_ERROR([php-config is required to build gameshark])
  fi

  AC_MSG_CHECKING([for supported PHP version])
  GAMESHARK_PHP_VERSION_ID=`$PHP_CONFIG --vernum 2>/dev/null`
  if test -z "$GAMESHARK_PHP_VERSION_ID"; then
    AC_MSG_ERROR([could not determine PHP version from php-config])
  fi
  if test "$GAMESHARK_PHP_VERSION_ID" -lt 80200; then
    AC_MSG_ERROR([gameshark requires PHP 8.2.0 or newer])
  fi
  AC_MSG_RESULT([$GAMESHARK_PHP_VERSION_ID])

  AC_MSG_CHECKING([for Zend observer API])
  GAMESHARK_PHP_INCLUDE_DIR=`$PHP_CONFIG --include-dir 2>/dev/null`
  if test -z "$GAMESHARK_PHP_INCLUDE_DIR" || test ! -r "$GAMESHARK_PHP_INCLUDE_DIR/Zend/zend_observer.h"; then
    AC_MSG_ERROR([gameshark requires Zend/zend_observer.h from PHP 8.2+])
  fi
  AC_MSG_RESULT([yes])

  AC_MSG_CHECKING([for non-ZTS PHP build])
  GAMESHARK_PHP_CONFIGURE_OPTIONS=`$PHP_CONFIG --configure-options 2>/dev/null`
  case "$GAMESHARK_PHP_CONFIGURE_OPTIONS" in
    *--enable-zts*|*--enable-maintainer-zts*)
      AC_MSG_ERROR([gameshark currently supports non-ZTS PHP builds only])
      ;;
  esac
  AC_MSG_RESULT([yes])

  AC_PATH_PROG([CARGO], [cargo], [no])
  if test "$CARGO" = "no"; then
    AC_MSG_ERROR([cargo is required to build gameshark])
  fi

  GAMESHARK_RUST_STATIC_LIB='./rust/target/release/libgameshark_core.a'
  GAMESHARK_RUST_DYLIB='./rust/target/release/libgameshark_core.dylib'
  case $host_os in
    linux*)
      GAMESHARK_RUST_DEPENDENCY="$GAMESHARK_RUST_STATIC_LIB"
      GAMESHARK_RUST_LINK_FLAGS="$GAMESHARK_RUST_STATIC_LIB"
      GAMESHARK_PLATFORM_LIBS="-ldl -lpthread -lm"
      ;;
    darwin*)
      GAMESHARK_RUST_DEPENDENCY="$GAMESHARK_RUST_DYLIB"
      GAMESHARK_RUST_LINK_FLAGS="-L./rust/target/release -lgameshark_core -Wl,-rpath,@loader_path"
      GAMESHARK_PLATFORM_LIBS="-lpthread -lm -framework Security -framework CoreFoundation -framework SystemConfiguration"
      ;;
    *)
      AC_MSG_ERROR([gameshark currently supports Linux and macOS builds only])
      ;;
  esac

  PHP_NEW_EXTENSION([gameshark], [gameshark.c], [$ext_shared])
  PHP_ADD_EXTENSION_DEP(gameshark, json, true)
  GAMESHARK_SHARED_DEPENDENCIES="$GAMESHARK_SHARED_DEPENDENCIES $GAMESHARK_RUST_DEPENDENCY"
  GAMESHARK_SHARED_LIBADD="$GAMESHARK_SHARED_LIBADD $GAMESHARK_RUST_LINK_FLAGS $GAMESHARK_PLATFORM_LIBS"
  PHP_SUBST([CARGO])
  PHP_SUBST([GAMESHARK_SHARED_DEPENDENCIES])
  PHP_SUBST([GAMESHARK_SHARED_LIBADD])
  PHP_ADD_MAKEFILE_FRAGMENT()
fi
