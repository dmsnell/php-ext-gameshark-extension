PHP_ARG_ENABLE([gameshark],
  [whether to enable gameshark support],
  [AS_HELP_STRING([--enable-gameshark],
    [Enable gameshark support])],
  [no])

if test "$PHP_GAMESHARK" != "no"; then
  PHP_NEW_EXTENSION([gameshark], [gameshark.c], [$ext_shared])
fi
