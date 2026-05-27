#ifdef HAVE_CONFIG_H
# include "config.h"
#endif

#include "php.h"
#include "SAPI.h"
#include "ext/standard/info.h"
#include "ext/json/php_json.h"
#include "Zend/zend_observer.h"
#include "php_gameshark.h"
#include "gameshark_core.h"

#include <string.h>
#include <unistd.h>

#define GAMESHARK_KIND_FUNCTION 1
#define GAMESHARK_KIND_METHOD 2
#define GAMESHARK_KIND_CLOSURE 3

PHP_FUNCTION(gameshark_loaded);
PHP_FUNCTION(gameshark_side);
PHP_FUNCTION(gameshark_db_path);
PHP_FUNCTION(gameshark_compare);

static bool gameshark_request_active = false;
static char *gameshark_request_side = NULL;
static char *gameshark_request_db_path = NULL;

ZEND_BEGIN_ARG_WITH_RETURN_TYPE_INFO_EX(arginfo_gameshark_loaded, 0, 0, _IS_BOOL, 0)
ZEND_END_ARG_INFO()

ZEND_BEGIN_ARG_WITH_RETURN_TYPE_MASK_EX(arginfo_gameshark_side, 0, 0, MAY_BE_STRING | MAY_BE_NULL)
ZEND_END_ARG_INFO()

ZEND_BEGIN_ARG_WITH_RETURN_TYPE_MASK_EX(arginfo_gameshark_db_path, 0, 0, MAY_BE_STRING | MAY_BE_NULL)
ZEND_END_ARG_INFO()

ZEND_BEGIN_ARG_WITH_RETURN_TYPE_INFO_EX(arginfo_gameshark_compare, 0, 0, IS_ARRAY, 0)
ZEND_END_ARG_INFO()

static const zend_function_entry gameshark_functions[] = {
	PHP_FE(gameshark_loaded, arginfo_gameshark_loaded)
	PHP_FE(gameshark_side, arginfo_gameshark_side)
	PHP_FE(gameshark_db_path, arginfo_gameshark_db_path)
	PHP_FE(gameshark_compare, arginfo_gameshark_compare)
	PHP_FE_END
};

static gameshark_core_str gameshark_zstr_to_core_str(zend_string *string)
{
	if (string == NULL) {
		return (gameshark_core_str){NULL, 0};
	}
	return (gameshark_core_str){ZSTR_VAL(string), ZSTR_LEN(string)};
}

static void gameshark_observer_begin(zend_execute_data *execute_data)
{
	zend_function *function = execute_data->func;
	if (!gameshark_request_active || function == NULL || function->type != ZEND_USER_FUNCTION || function->common.function_name == NULL) {
		return;
	}

	uint8_t kind = GAMESHARK_KIND_FUNCTION;
	if (function->common.fn_flags & ZEND_ACC_CLOSURE) {
		kind = GAMESHARK_KIND_CLOSURE;
	} else if (function->common.scope != NULL) {
		kind = GAMESHARK_KIND_METHOD;
	}

	gameshark_core_function_meta meta = {
		kind,
		function->common.scope != NULL ? gameshark_zstr_to_core_str(function->common.scope->name) : (gameshark_core_str){NULL, 0},
		gameshark_zstr_to_core_str(function->common.function_name),
		gameshark_zstr_to_core_str(function->op_array.filename),
		function->op_array.line_start,
		function->op_array.line_end,
	};
	gameshark_core_record_call(&meta);
}

static zend_observer_fcall_handlers gameshark_observer_fcall_init(zend_execute_data *execute_data)
{
	zend_function *function = execute_data->func;
	if (!gameshark_request_active || function == NULL || function->type != ZEND_USER_FUNCTION || function->common.function_name == NULL) {
		return (zend_observer_fcall_handlers){NULL, NULL};
	}

	return (zend_observer_fcall_handlers){gameshark_observer_begin, NULL};
}

PHP_MINIT_FUNCTION(gameshark)
{
	if (type != MODULE_TEMPORARY) {
		zend_observer_fcall_register(gameshark_observer_fcall_init);
	}
	return SUCCESS;
}

PHP_RINIT_FUNCTION(gameshark)
{
#if defined(ZTS) && defined(COMPILE_DL_GAMESHARK)
	ZEND_TSRMLS_CACHE_UPDATE();
#endif

	const char *db_path = getenv("GAMESHARK_DB");
	const char *side = getenv("GAMESHARK_SIDE");

	gameshark_request_active = false;
	gameshark_request_db_path = NULL;
	gameshark_request_side = NULL;

	if (db_path == NULL || db_path[0] == '\0' || side == NULL || (strcmp(side, "left") != 0 && strcmp(side, "right") != 0)) {
		return SUCCESS;
	}

	const char *script_filename = SG(request_info).path_translated;
	if (script_filename == NULL) {
		script_filename = SG(request_info).request_uri;
	}

	if (gameshark_core_request_start(db_path, side, PHP_VERSION, sapi_module.name, (uint32_t)getpid(), script_filename)) {
		gameshark_request_active = true;
		gameshark_request_db_path = estrdup(db_path);
		gameshark_request_side = estrdup(side);
	}

	return SUCCESS;
}

PHP_RSHUTDOWN_FUNCTION(gameshark)
{
	if (gameshark_request_active) {
		gameshark_core_request_finish();
	}

	if (gameshark_request_db_path != NULL) {
		efree(gameshark_request_db_path);
		gameshark_request_db_path = NULL;
	}
	if (gameshark_request_side != NULL) {
		efree(gameshark_request_side);
		gameshark_request_side = NULL;
	}
	gameshark_request_active = false;

	return SUCCESS;
}

PHP_FUNCTION(gameshark_loaded)
{
	ZEND_PARSE_PARAMETERS_NONE();

	RETURN_TRUE;
}

PHP_FUNCTION(gameshark_side)
{
	ZEND_PARSE_PARAMETERS_NONE();

	if (gameshark_request_side == NULL) {
		RETURN_NULL();
	}
	RETURN_STRING(gameshark_request_side);
}

PHP_FUNCTION(gameshark_db_path)
{
	ZEND_PARSE_PARAMETERS_NONE();

	if (gameshark_request_db_path == NULL) {
		RETURN_NULL();
	}
	RETURN_STRING(gameshark_request_db_path);
}

PHP_FUNCTION(gameshark_compare)
{
	ZEND_PARSE_PARAMETERS_NONE();

	const char *db_path = gameshark_request_db_path;
	if (db_path == NULL || db_path[0] == '\0') {
		db_path = getenv("GAMESHARK_DB");
	}

	char *json = gameshark_core_compare_json(db_path);
	if (json == NULL) {
		array_init(return_value);
		add_assoc_string(return_value, "error", "gameshark comparison failed");
		return;
	}

	if (php_json_decode(return_value, json, strlen(json), true, 512) != SUCCESS) {
		gameshark_core_string_free(json);
		array_init(return_value);
		add_assoc_string(return_value, "error", "gameshark comparison JSON decode failed");
		return;
	}

	gameshark_core_string_free(json);
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
	PHP_MINIT(gameshark),
	NULL,
	PHP_RINIT(gameshark),
	PHP_RSHUTDOWN(gameshark),
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
