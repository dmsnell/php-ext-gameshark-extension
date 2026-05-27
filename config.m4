PHP_ARG_ENABLE([gameshark],
  [whether to enable gameshark support],
  [AS_HELP_STRING([--enable-gameshark],
    [Enable gameshark support])],
  [no])

if test "$PHP_GAMESHARK" != "no"; then
  AC_PATH_PROG([CARGO], [cargo], [no])
  if test "$CARGO" = "no"; then
    AC_MSG_ERROR([cargo is required to build gameshark])
  fi

  PHP_NEW_EXTENSION([gameshark], [gameshark.c], [$ext_shared])
  PHP_ADD_EXTENSION_DEP(gameshark, json, true)
  GAMESHARK_RUST_LIB='./rust/target/release/libgameshark_core.a'
  GAMESHARK_SHARED_DEPENDENCIES="$GAMESHARK_SHARED_DEPENDENCIES $GAMESHARK_RUST_LIB"
  GAMESHARK_SHARED_LIBADD="$GAMESHARK_SHARED_LIBADD $GAMESHARK_RUST_LIB -ldl -lpthread -lm"
  PHP_SUBST([CARGO])
  PHP_SUBST([GAMESHARK_SHARED_DEPENDENCIES])
  PHP_SUBST([GAMESHARK_SHARED_LIBADD])
  PHP_ADD_MAKEFILE_FRAGMENT()
fi
