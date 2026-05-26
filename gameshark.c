#ifdef HAVE_CONFIG_H
# include "config.h"
#endif

#include "php.h"
#include "ext/standard/info.h"
#include "php_gameshark.h"

PHP_FUNCTION(gameshark_loaded);

ZEND_BEGIN_ARG_WITH_RETURN_TYPE_INFO_EX(arginfo_gameshark_loaded, 0, 0, _IS_BOOL, 0)
ZEND_END_ARG_INFO()

static const zend_function_entry gameshark_functions[] = {
	PHP_FE(gameshark_loaded, arginfo_gameshark_loaded)
	PHP_FE_END
};

PHP_FUNCTION(gameshark_loaded)
{
	ZEND_PARSE_PARAMETERS_NONE();

	RETURN_TRUE;
}

PHP_MINFO_FUNCTION(gameshark)
{
	php_info_print_table_start();
	php_info_print_table_header(2, "gameshark support", "enabled");
	php_info_print_table_row(2, "Version", PHP_GAMESHARK_VERSION);
	php_info_print_table_end();
}

zend_module_entry gameshark_module_entry = {
	STANDARD_MODULE_HEADER,
	"gameshark",
	gameshark_functions,
	NULL,
	NULL,
	NULL,
	NULL,
	PHP_MINFO(gameshark),
	PHP_GAMESHARK_VERSION,
	STANDARD_MODULE_PROPERTIES
};

#ifdef COMPILE_DL_GAMESHARK
# ifdef ZTS
ZEND_TSRMLS_CACHE_DEFINE()
# endif
ZEND_GET_MODULE(gameshark)
#endif
