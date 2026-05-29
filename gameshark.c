#ifdef HAVE_CONFIG_H
# include "config.h"
#endif

#include "php.h"

#if PHP_VERSION_ID < 80200
# error "gameshark requires PHP 8.2.0 or newer"
#endif

#ifdef PHP_WIN32
# error "gameshark currently supports Linux and macOS builds only"
#endif

#include "SAPI.h"
#include "ext/standard/info.h"
#include "ext/json/php_json.h"
#include "Zend/zend_observer.h"
#include "Zend/zend_operators.h"
#include "Zend/zend_smart_str.h"
#include "Zend/zend_compile.h"
#include "Zend/zend_execute.h"
#include "Zend/zend_constants.h"
#include "Zend/zend_stream.h"
#include "php_gameshark.h"
#include "gameshark_core.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <stdarg.h>

#ifndef zend_ini_string_literal
# define zend_ini_string_literal(name) zend_ini_string((name), sizeof("" name) - 1, false)
#endif

#ifndef ZEND_VIRTUAL_PROPERTY_OFFSET
# define ZEND_VIRTUAL_PROPERTY_OFFSET ((uint32_t)-1)
#endif

#ifndef IS_PROP_LAZY
# define IS_PROP_LAZY 0
#endif

#define GAMESHARK_KIND_FUNCTION 1
#define GAMESHARK_KIND_METHOD 2
#define GAMESHARK_KIND_CLOSURE 3
#define GAMESHARK_KIND_INTERNAL_FUNCTION 4
#define GAMESHARK_KIND_INTERNAL_METHOD 5

#define GAMESHARK_TRACE_MATCH_STRING_CONTAINS 1
#define GAMESHARK_TRACE_MATCH_NUMBER_EQUALS 2
#define GAMESHARK_TRACE_MATCH_NUMERIC_STRING_CONTAINS 3

#define GAMESHARK_TRACE_MAX_DEPTH 6
#define GAMESHARK_TRACE_MAX_VISITED 64
#define GAMESHARK_TRACE_PREVIEW_CONTEXT 80
#define GAMESHARK_TRACE_ARG_PREVIEW_CONTEXT 80
#define GAMESHARK_TRACE_ARG_STRING_MAX 160
#define GAMESHARK_TRACE_MAX_ARG_MATCHES 32
#define GAMESHARK_TRACE_STACK_MAX_FRAMES 32
#define GAMESHARK_TRACE_MAX_TRACKED_VALUES 64
#define GAMESHARK_TRACE_MAX_TRANSFORM_DEPTH 4
#define GAMESHARK_TRACE_MAX_TRANSFORM_VALUE_LEN 4096
#define GAMESHARK_TRACE_MAX_ACTIVE_FRAMES 128
#define GAMESHARK_INVARIANT_MAX_ACTIVE_FRAMES 128

#define GAMESHARK_REPORT_TEXT 0
#define GAMESHARK_REPORT_ARRAY 1
#define GAMESHARK_REPORT_JSON 2

#define GAMESHARK_INVARIANT_TARGET_FUNCTION 1
#define GAMESHARK_INVARIANT_TARGET_METHOD 2
#define GAMESHARK_INVARIANT_PHASE_PRE 1
#define GAMESHARK_INVARIANT_PHASE_POST 2
#define GAMESHARK_INVARIANT_RESOLVED_UNKNOWN 0
#define GAMESHARK_INVARIANT_RESOLVED_USER_FUNCTION 1
#define GAMESHARK_INVARIANT_RESOLVED_USER_METHOD 2
#define GAMESHARK_INVARIANT_RESOLVED_INTERNAL_FUNCTION 3
#define GAMESHARK_INVARIANT_RESOLVED_INTERNAL_METHOD 4
#define GAMESHARK_INVARIANT_RESOLVED_UNSUPPORTED 5

#define GAMESHARK_UNUSED_DECL_FUNCTION 1
#define GAMESHARK_UNUSED_DECL_METHOD 2
#define GAMESHARK_UNUSED_DECL_CLASS 3
#define GAMESHARK_UNUSED_DECL_GLOBAL_CONSTANT 4
#define GAMESHARK_UNUSED_DECL_CLASS_CONSTANT 5

#define GAMESHARK_UNUSED_ACCESS_FUNCTION_CALL 1
#define GAMESHARK_UNUSED_ACCESS_METHOD_CALL 2
#define GAMESHARK_UNUSED_ACCESS_CLOSURE_CALL 3
#define GAMESHARK_UNUSED_ACCESS_NEW_OPCODE 4
#define GAMESHARK_UNUSED_ACCESS_GLOBAL_CONSTANT_FETCH 5
#define GAMESHARK_UNUSED_ACCESS_CLASS_CONSTANT_FETCH 6
#define GAMESHARK_UNUSED_ACCESS_GLOBAL_CONSTANT_READ 7
#define GAMESHARK_UNUSED_ACCESS_CLASS_CONSTANT_READ 8
#define GAMESHARK_UNUSED_ACCESS_GLOBAL_CONSTANT_PROBE 9
#define GAMESHARK_UNUSED_ACCESS_CLASS_CONSTANT_PROBE 10

PHP_FUNCTION(gameshark_loaded);
PHP_FUNCTION(gameshark_side);
PHP_FUNCTION(gameshark_db_path);
PHP_FUNCTION(gameshark_compare);
PHP_FUNCTION(gameshark_trace_report);
PHP_FUNCTION(gameshark_unused_report);
PHP_FUNCTION(gameshark_invariants_status);

typedef struct {
	zend_execute_data *execute_data;
	gameshark_core_function_meta function;
	uint64_t matched_value_mask;
	HashTable *visited_arrays[GAMESHARK_TRACE_MAX_VISITED];
	size_t visited_array_count;
	zend_object *visited_objects[GAMESHARK_TRACE_MAX_VISITED];
	size_t visited_object_count;
} gameshark_trace_context;

typedef struct {
	smart_str paths_json;
	smart_str matches_json;
	bool needs_comma;
	size_t count;
	zend_string *first_path;
	zend_string *first_preview;
	HashTable *visited_arrays[GAMESHARK_TRACE_MAX_VISITED];
	size_t visited_array_count;
	zend_object *visited_objects[GAMESHARK_TRACE_MAX_VISITED];
	size_t visited_object_count;
} gameshark_match_context;

typedef struct {
	uint32_t value_id;
	uint32_t parent_value_id;
	uint32_t depth;
	zend_string *value;
} gameshark_tracked_trace_value;

typedef struct {
	zend_execute_data *execute_data;
	gameshark_core_function_meta function;
	uint64_t matched_value_mask;
} gameshark_trace_frame;

typedef struct {
	zend_string *id;
	zend_string *target;
	zend_string *match_key;
	uint8_t target_kind;
	uint8_t phase;
	uint8_t resolved_kind;
	zval hook;
	bool matched;
	uint64_t executions;
	uint64_t hook_exceptions;
} gameshark_invariant_spec;

typedef struct {
	zend_execute_data *execute_data;
	zval args;
	zval object;
	bool has_object;
} gameshark_invariant_frame;

static bool gameshark_request_active = false;
static bool gameshark_count_active = false;
static bool gameshark_trace_active = false;
static bool gameshark_trace_follow_transforms = false;
static bool gameshark_trace_filter_active = false;
static bool gameshark_invariants_enabled = false;
static bool gameshark_invariants_loaded = false;
static bool gameshark_invariants_active = false;
static bool gameshark_invariants_executing_hook = false;
static bool gameshark_unused_active = false;
static char *gameshark_request_side = NULL;
static char *gameshark_request_db_path = NULL;
static char *gameshark_invariants_file = NULL;
static zend_string *gameshark_invariants_load_error = NULL;
static char *gameshark_trace_value = NULL;
static size_t gameshark_trace_value_len = 0;
static bool gameshark_trace_value_is_numeric = false;
static bool gameshark_trace_value_is_long = false;
static zend_long gameshark_trace_long_value = 0;
static double gameshark_trace_double_value = 0.0;
static char *gameshark_trace_string_match = NULL;
static size_t gameshark_trace_string_match_len = 0;
static gameshark_tracked_trace_value gameshark_trace_values[GAMESHARK_TRACE_MAX_TRACKED_VALUES];
static size_t gameshark_trace_value_count = 0;
static gameshark_trace_frame gameshark_trace_frames[GAMESHARK_TRACE_MAX_ACTIVE_FRAMES];
static size_t gameshark_trace_frame_count = 0;
static gameshark_invariant_spec *gameshark_invariant_specs = NULL;
static size_t gameshark_invariant_spec_count = 0;
static size_t gameshark_invariant_spec_capacity = 0;
static gameshark_invariant_frame gameshark_invariant_frames[GAMESHARK_INVARIANT_MAX_ACTIVE_FRAMES];
static size_t gameshark_invariant_frame_count = 0;
static uint64_t gameshark_invariant_reentrancy_suppressed = 0;
static bool gameshark_invariant_has_internal_hooks = false;
static bool gameshark_invariant_warn_builtins = true;
static bool gameshark_invariant_internal_warning_emitted = false;
static uint64_t gameshark_invariant_internal_pre_invocations = 0;
static uint64_t gameshark_invariant_internal_post_invocations = 0;
static uint64_t gameshark_invariant_internal_original_exceptions = 0;
static uint64_t gameshark_invariant_internal_hook_exceptions = 0;
static bool gameshark_execute_internal_previous_present = false;
static zend_op_array *(*gameshark_original_compile_file)(zend_file_handle *file_handle, int type) = NULL;
static void (*gameshark_original_execute_ex)(zend_execute_data *execute_data) = NULL;
static void (*gameshark_original_execute_internal)(zend_execute_data *execute_data, zval *return_value) = NULL;
static bool gameshark_new_opcode_handler_owned = false;
static bool gameshark_constant_opcode_handler_owned = false;
static bool gameshark_class_constant_opcode_handler_owned = false;
static bool gameshark_defined_opcode_handler_owned = false;

static bool gameshark_numeric_matches_long(zend_long value);
static bool gameshark_numeric_matches_double(double value);
static bool gameshark_string_has_nul(zend_string *string);
static zend_string *gameshark_scalar_value(zval *value);
static bool gameshark_is_absolute_path(const char *path);

ZEND_BEGIN_ARG_WITH_RETURN_TYPE_INFO_EX(arginfo_gameshark_loaded, 0, 0, _IS_BOOL, 0)
ZEND_END_ARG_INFO()

ZEND_BEGIN_ARG_WITH_RETURN_TYPE_MASK_EX(arginfo_gameshark_side, 0, 0, MAY_BE_STRING | MAY_BE_NULL)
ZEND_END_ARG_INFO()

ZEND_BEGIN_ARG_WITH_RETURN_TYPE_MASK_EX(arginfo_gameshark_db_path, 0, 0, MAY_BE_STRING | MAY_BE_NULL)
ZEND_END_ARG_INFO()

ZEND_BEGIN_ARG_WITH_RETURN_TYPE_MASK_EX(arginfo_gameshark_compare, 0, 0, MAY_BE_ARRAY | MAY_BE_STRING)
	ZEND_ARG_TYPE_INFO(0, format, IS_STRING, 0)
ZEND_END_ARG_INFO()

ZEND_BEGIN_ARG_WITH_RETURN_TYPE_MASK_EX(arginfo_gameshark_trace_report, 0, 0, MAY_BE_ARRAY | MAY_BE_STRING)
	ZEND_ARG_TYPE_INFO(0, format, IS_STRING, 0)
ZEND_END_ARG_INFO()

ZEND_BEGIN_ARG_WITH_RETURN_TYPE_MASK_EX(arginfo_gameshark_unused_report, 0, 0, MAY_BE_ARRAY | MAY_BE_STRING)
	ZEND_ARG_TYPE_INFO(0, format, IS_STRING, 0)
	ZEND_ARG_TYPE_INFO(0, run_id, IS_LONG, 1)
ZEND_END_ARG_INFO()

ZEND_BEGIN_ARG_WITH_RETURN_TYPE_INFO_EX(arginfo_gameshark_invariants_status, 0, 0, IS_ARRAY, 0)
ZEND_END_ARG_INFO()

static const zend_function_entry gameshark_functions[] = {
	PHP_FE(gameshark_loaded, arginfo_gameshark_loaded)
	PHP_FE(gameshark_side, arginfo_gameshark_side)
	PHP_FE(gameshark_db_path, arginfo_gameshark_db_path)
	PHP_FE(gameshark_compare, arginfo_gameshark_compare)
	PHP_FE(gameshark_trace_report, arginfo_gameshark_trace_report)
	PHP_FE(gameshark_unused_report, arginfo_gameshark_unused_report)
	PHP_FE(gameshark_invariants_status, arginfo_gameshark_invariants_status)
	PHP_FE_END
};

PHP_INI_BEGIN()
	PHP_INI_ENTRY("gameshark.trace_allow_pattern", "", PHP_INI_SYSTEM | PHP_INI_PERDIR, NULL)
	PHP_INI_ENTRY("gameshark.invariants", "", PHP_INI_SYSTEM | PHP_INI_PERDIR, NULL)
	PHP_INI_ENTRY("gameshark.invariants_file", "", PHP_INI_SYSTEM | PHP_INI_PERDIR, NULL)
	PHP_INI_ENTRY("gameshark.invariants_warn_builtins", "1", PHP_INI_SYSTEM | PHP_INI_PERDIR, NULL)
	PHP_INI_ENTRY("gameshark.unused", "", PHP_INI_SYSTEM | PHP_INI_PERDIR, NULL)
	PHP_INI_ENTRY("gameshark.unused_capture_query", "0", PHP_INI_SYSTEM | PHP_INI_PERDIR, NULL)
PHP_INI_END()

static gameshark_core_str gameshark_zstr_to_core_str(zend_string *string)
{
	if (string == NULL) {
		return (gameshark_core_str){NULL, 0};
	}
	return (gameshark_core_str){ZSTR_VAL(string), ZSTR_LEN(string)};
}

static gameshark_core_str gameshark_mem_to_core_str(const char *ptr, size_t len)
{
	if (ptr == NULL) {
		return (gameshark_core_str){NULL, 0};
	}
	return (gameshark_core_str){ptr, len};
}

static const char *gameshark_find_bytes(const char *haystack, size_t haystack_len, const char *needle, size_t needle_len)
{
	if (needle_len == 0 || haystack_len < needle_len) {
		return NULL;
	}

	size_t limit = haystack_len - needle_len;
	for (size_t i = 0; i <= limit; i++) {
		if (haystack[i] == needle[0] && memcmp(haystack + i, needle, needle_len) == 0) {
			return haystack + i;
		}
	}
	return NULL;
}

static uint8_t gameshark_kind_for_function(zend_function *function)
{
	if (function->type == ZEND_USER_FUNCTION && (function->common.fn_flags & ZEND_ACC_CLOSURE)) {
		return GAMESHARK_KIND_CLOSURE;
	}
	if (function->type == ZEND_INTERNAL_FUNCTION) {
		return function->common.scope != NULL ? GAMESHARK_KIND_INTERNAL_METHOD : GAMESHARK_KIND_INTERNAL_FUNCTION;
	}
	return function->common.scope != NULL ? GAMESHARK_KIND_METHOD : GAMESHARK_KIND_FUNCTION;
}

static gameshark_core_function_meta gameshark_function_meta(zend_function *function)
{
	gameshark_core_str file = {NULL, 0};
	uint32_t start_line = 0;
	uint32_t end_line = 0;

	if (function->type == ZEND_USER_FUNCTION) {
		file = gameshark_zstr_to_core_str(function->op_array.filename);
		start_line = function->op_array.line_start;
		end_line = function->op_array.line_end;
	}

	return (gameshark_core_function_meta){
		gameshark_kind_for_function(function),
		function->common.scope != NULL ? gameshark_zstr_to_core_str(function->common.scope->name) : (gameshark_core_str){NULL, 0},
		gameshark_zstr_to_core_str(function->common.function_name),
		file,
		start_line,
		end_line,
	};
}

static bool gameshark_unused_file_is_invariant(zend_string *file)
{
	if (file == NULL || gameshark_invariants_file == NULL) {
		return false;
	}
	return ZSTR_LEN(file) == strlen(gameshark_invariants_file) &&
		memcmp(ZSTR_VAL(file), gameshark_invariants_file, ZSTR_LEN(file)) == 0;
}

static void gameshark_unused_record_declaration(
	uint8_t kind,
	zend_string *scope_name,
	zend_string *name,
	zend_string *file,
	uint32_t start_line,
	uint32_t end_line,
	uint32_t flags
) {
	if (!gameshark_unused_active || name == NULL || ZSTR_LEN(name) == 0 || gameshark_unused_file_is_invariant(file)) {
		return;
	}

	gameshark_core_unused_declaration declaration = {
		kind,
		gameshark_zstr_to_core_str(scope_name),
		gameshark_zstr_to_core_str(name),
		gameshark_zstr_to_core_str(file),
		start_line,
		end_line,
		flags,
	};
	gameshark_core_record_unused_declaration(&declaration);
}

static void gameshark_unused_record_access(
	uint8_t kind,
	zend_string *scope_name,
	zend_string *name,
	zend_string *file,
	uint32_t start_line,
	uint32_t end_line
) {
	if (!gameshark_unused_active || name == NULL || ZSTR_LEN(name) == 0 || gameshark_unused_file_is_invariant(file)) {
		return;
	}

	gameshark_core_unused_access access = {
		kind,
		gameshark_zstr_to_core_str(scope_name),
		gameshark_zstr_to_core_str(name),
		gameshark_zstr_to_core_str(file),
		start_line,
		end_line,
	};
	gameshark_core_record_unused_access(&access);
}

static void gameshark_unused_record_access_mem(
	uint8_t kind,
	const char *scope_name,
	size_t scope_name_len,
	const char *name,
	size_t name_len
) {
	if (!gameshark_unused_active || name == NULL || name_len == 0) {
		return;
	}

	gameshark_core_unused_access access = {
		kind,
		gameshark_mem_to_core_str(scope_name, scope_name_len),
		gameshark_mem_to_core_str(name, name_len),
		{NULL, 0},
		0,
		0,
	};
	gameshark_core_record_unused_access(&access);
}

static void gameshark_unused_record_caveat(const char *caveat)
{
	if (gameshark_unused_active && caveat != NULL) {
		gameshark_core_record_unused_caveat(caveat);
	}
}

static void gameshark_unused_record_included_file(zend_string *file)
{
	if (!gameshark_unused_active || file == NULL || ZSTR_LEN(file) == 0) {
		return;
	}
	if (!gameshark_is_absolute_path(ZSTR_VAL(file))) {
		return;
	}
	if (gameshark_unused_file_is_invariant(file)) {
		return;
	}
	gameshark_core_record_unused_included_file(ZSTR_VAL(file));
}

static void gameshark_unused_record_call(gameshark_core_function_meta *meta)
{
	if (!gameshark_unused_active || meta == NULL || meta->function_name.ptr == NULL || meta->function_name.len == 0) {
		return;
	}

	uint8_t access_kind = GAMESHARK_UNUSED_ACCESS_FUNCTION_CALL;
	if (meta->kind == GAMESHARK_KIND_METHOD) {
		access_kind = GAMESHARK_UNUSED_ACCESS_METHOD_CALL;
	} else if (meta->kind == GAMESHARK_KIND_CLOSURE) {
		access_kind = GAMESHARK_UNUSED_ACCESS_CLOSURE_CALL;
	} else if (meta->kind != GAMESHARK_KIND_FUNCTION) {
		return;
	}

	gameshark_core_unused_access access = {
		access_kind,
		meta->scope_name,
		meta->function_name,
		meta->file,
		meta->start_line,
		meta->end_line,
	};
	gameshark_core_record_unused_access(&access);
}

static zend_string *gameshark_trace_canonical_function_name(zend_function *function)
{
	if (function == NULL || function->common.function_name == NULL) {
		return NULL;
	}
	if (function->type == ZEND_USER_FUNCTION && (function->common.fn_flags & ZEND_ACC_CLOSURE)) {
		return NULL;
	}

	smart_str name = {0};
	if (function->common.scope != NULL) {
		smart_str_append(&name, function->common.scope->name);
		smart_str_appends(&name, "::");
	}
	smart_str_append(&name, function->common.function_name);
	smart_str_0(&name);
	if (name.s == NULL) {
		return NULL;
	}

	zend_string *canonical = zend_string_tolower(name.s);
	zend_string_release(name.s);
	return canonical;
}

static void gameshark_reset_tracked_trace_values(void)
{
	for (size_t i = 0; i < gameshark_trace_value_count; i++) {
		if (gameshark_trace_values[i].value != NULL) {
			zend_string_release(gameshark_trace_values[i].value);
			gameshark_trace_values[i].value = NULL;
		}
	}
	gameshark_trace_value_count = 0;
	gameshark_trace_frame_count = 0;
}

static gameshark_tracked_trace_value *gameshark_find_tracked_trace_value(uint32_t value_id)
{
	for (size_t i = 0; i < gameshark_trace_value_count; i++) {
		if (gameshark_trace_values[i].value_id == value_id) {
			return &gameshark_trace_values[i];
		}
	}
	return NULL;
}

static uint32_t gameshark_add_tracked_trace_value(
	const char *value,
	size_t value_len,
	uint32_t parent_value_id,
	uint32_t depth,
	bool *added
) {
	if (added != NULL) {
		*added = false;
	}
	if (value == NULL || value_len == 0) {
		return 0;
	}
	if (parent_value_id != 0 && value_len > GAMESHARK_TRACE_MAX_TRANSFORM_VALUE_LEN) {
		return 0;
	}

	for (size_t i = 0; i < gameshark_trace_value_count; i++) {
		zend_string *existing = gameshark_trace_values[i].value;
		if (existing != NULL && ZSTR_LEN(existing) == value_len && memcmp(ZSTR_VAL(existing), value, value_len) == 0) {
			return gameshark_trace_values[i].value_id;
		}
	}

	if (gameshark_trace_value_count >= GAMESHARK_TRACE_MAX_TRACKED_VALUES) {
		return 0;
	}

	uint32_t value_id = (uint32_t) gameshark_trace_value_count + 1;
	gameshark_trace_values[gameshark_trace_value_count++] = (gameshark_tracked_trace_value){
		value_id,
		parent_value_id,
		depth,
		zend_string_init(value, value_len, 0)
	};
	if (added != NULL) {
		*added = true;
	}
	return value_id;
}

static void gameshark_reset_trace_config(void)
{
	if (gameshark_trace_value != NULL) {
		efree(gameshark_trace_value);
		gameshark_trace_value = NULL;
	}
	if (gameshark_trace_string_match != NULL) {
		efree(gameshark_trace_string_match);
		gameshark_trace_string_match = NULL;
	}
	gameshark_trace_value_len = 0;
	gameshark_trace_string_match_len = 0;
	gameshark_trace_value_is_numeric = false;
	gameshark_trace_value_is_long = false;
	gameshark_trace_long_value = 0;
	gameshark_trace_double_value = 0.0;
	gameshark_trace_follow_transforms = false;
	gameshark_trace_filter_active = false;
	gameshark_reset_tracked_trace_values();
}

static void gameshark_set_invariant_error(const char *format, ...)
{
	if (gameshark_invariants_load_error != NULL) {
		zend_string_release(gameshark_invariants_load_error);
		gameshark_invariants_load_error = NULL;
	}

	va_list args;
	va_start(args, format);
	gameshark_invariants_load_error = zend_vstrpprintf(0, format, args);
	va_end(args);
}

static bool gameshark_config_truthy(const char *value)
{
	if (value == NULL || value[0] == '\0') {
		return false;
	}
	return strcmp(value, "0") != 0 && strcmp(value, "false") != 0 && strcmp(value, "off") != 0 && strcmp(value, "no") != 0;
}

static bool gameshark_config_has_value(const char *value)
{
	if (value == NULL) {
		return false;
	}
	while (*value != '\0') {
		if (*value != ' ' && *value != '\t' && *value != '\r' && *value != '\n') {
			return true;
		}
		value++;
	}
	return false;
}

static bool gameshark_is_absolute_path(const char *path)
{
	return path != NULL && path[0] == '/';
}

static void gameshark_clear_invariant_frames(void)
{
	for (size_t i = 0; i < gameshark_invariant_frame_count; i++) {
		zval_ptr_dtor(&gameshark_invariant_frames[i].args);
		if (gameshark_invariant_frames[i].has_object) {
			zval_ptr_dtor(&gameshark_invariant_frames[i].object);
		}
	}
	gameshark_invariant_frame_count = 0;
}

static void gameshark_reset_invariant_config(void)
{
	for (size_t i = 0; i < gameshark_invariant_spec_count; i++) {
		if (gameshark_invariant_specs[i].id != NULL) {
			zend_string_release(gameshark_invariant_specs[i].id);
		}
		if (gameshark_invariant_specs[i].target != NULL) {
			zend_string_release(gameshark_invariant_specs[i].target);
		}
		if (gameshark_invariant_specs[i].match_key != NULL) {
			zend_string_release(gameshark_invariant_specs[i].match_key);
		}
		zval_ptr_dtor(&gameshark_invariant_specs[i].hook);
	}
	if (gameshark_invariant_specs != NULL) {
		efree(gameshark_invariant_specs);
		gameshark_invariant_specs = NULL;
	}
	gameshark_invariant_spec_count = 0;
	gameshark_invariant_spec_capacity = 0;

	gameshark_clear_invariant_frames();

	if (gameshark_invariants_file != NULL) {
		efree(gameshark_invariants_file);
		gameshark_invariants_file = NULL;
	}
	if (gameshark_invariants_load_error != NULL) {
		zend_string_release(gameshark_invariants_load_error);
		gameshark_invariants_load_error = NULL;
	}

	gameshark_invariants_enabled = false;
	gameshark_invariants_loaded = false;
	gameshark_invariants_active = false;
	gameshark_invariants_executing_hook = false;
	gameshark_invariant_reentrancy_suppressed = 0;
	gameshark_invariant_has_internal_hooks = false;
	gameshark_invariant_warn_builtins = true;
	gameshark_invariant_internal_warning_emitted = false;
	gameshark_invariant_internal_pre_invocations = 0;
	gameshark_invariant_internal_post_invocations = 0;
	gameshark_invariant_internal_original_exceptions = 0;
	gameshark_invariant_internal_hook_exceptions = 0;
}

static bool gameshark_validate_spec_keys(HashTable *spec, uint32_t spec_index)
{
	zend_string *key;
	zend_ulong num_key;

	ZEND_HASH_FOREACH_KEY(spec, num_key, key) {
		if (key == NULL) {
			gameshark_set_invariant_error("spec %u has numeric key " ZEND_LONG_FMT, spec_index, (zend_long) num_key);
			return false;
		}
		if (
			!zend_string_equals_literal(key, "id") &&
			!zend_string_equals_literal(key, "target") &&
			!zend_string_equals_literal(key, "when") &&
			!zend_string_equals_literal(key, "hook")
		) {
			gameshark_set_invariant_error("spec %u has unsupported key \"%s\"", spec_index, ZSTR_VAL(key));
			return false;
		}
	} ZEND_HASH_FOREACH_END();

	return true;
}

static bool gameshark_normalize_invariant_target(
	zend_string *target,
	uint8_t *target_kind,
	zend_string **display_target,
	zend_string **match_key,
	uint32_t spec_index
) {
	const char *target_start = ZSTR_VAL(target);
	size_t target_len = ZSTR_LEN(target);

	if (target_len > 0 && target_start[0] == '\\') {
		target_start++;
		target_len--;
	}
	if (target_len == 0) {
		gameshark_set_invariant_error("spec %u has an empty target", spec_index);
		return false;
	}

	const char *method_separator = gameshark_find_bytes(target_start, target_len, "::", 2);
	if (method_separator != NULL) {
		size_t class_len = (size_t) (method_separator - target_start);
		size_t method_len = target_len - class_len - 2;
		if (class_len == 0 || method_len == 0) {
			gameshark_set_invariant_error("spec %u has invalid method target \"%.*s\"", spec_index, (int) target_len, target_start);
			return false;
		}
		*target_kind = GAMESHARK_INVARIANT_TARGET_METHOD;
	} else if (gameshark_find_bytes(target_start, target_len, ":", 1) != NULL) {
		gameshark_set_invariant_error("spec %u target must omit function:/method: prefixes", spec_index);
		return false;
	} else {
		*target_kind = GAMESHARK_INVARIANT_TARGET_FUNCTION;
	}

	*display_target = zend_string_init(target_start, target_len, 0);
	zend_string *raw_key = zend_string_init(target_start, target_len, 0);
	*match_key = zend_string_tolower(raw_key);
	zend_string_release(raw_key);
	return true;
}

static const char *gameshark_invariant_resolved_kind_name(uint8_t resolved_kind)
{
	switch (resolved_kind) {
		case GAMESHARK_INVARIANT_RESOLVED_USER_FUNCTION:
			return "user_function";
		case GAMESHARK_INVARIANT_RESOLVED_USER_METHOD:
			return "user_method";
		case GAMESHARK_INVARIANT_RESOLVED_INTERNAL_FUNCTION:
			return "internal_function";
		case GAMESHARK_INVARIANT_RESOLVED_INTERNAL_METHOD:
			return "internal_method";
		case GAMESHARK_INVARIANT_RESOLVED_UNSUPPORTED:
			return "unsupported";
		case GAMESHARK_INVARIANT_RESOLVED_UNKNOWN:
		default:
			return "unknown";
	}
}

static bool gameshark_invariant_resolved_kind_is_internal(uint8_t resolved_kind)
{
	return resolved_kind == GAMESHARK_INVARIANT_RESOLVED_INTERNAL_FUNCTION ||
		resolved_kind == GAMESHARK_INVARIANT_RESOLVED_INTERNAL_METHOD;
}

static uint8_t gameshark_invariant_resolved_kind_for_function(zend_function *function, uint8_t target_kind)
{
	if (function == NULL || function->common.function_name == NULL) {
		return GAMESHARK_INVARIANT_RESOLVED_UNKNOWN;
	}
	if (function->type == ZEND_USER_FUNCTION && (function->common.fn_flags & ZEND_ACC_CLOSURE) == 0) {
		return target_kind == GAMESHARK_INVARIANT_TARGET_METHOD
			? GAMESHARK_INVARIANT_RESOLVED_USER_METHOD
			: GAMESHARK_INVARIANT_RESOLVED_USER_FUNCTION;
	}
	if (function->type == ZEND_INTERNAL_FUNCTION) {
		return target_kind == GAMESHARK_INVARIANT_TARGET_METHOD
			? GAMESHARK_INVARIANT_RESOLVED_INTERNAL_METHOD
			: GAMESHARK_INVARIANT_RESOLVED_INTERNAL_FUNCTION;
	}
	return GAMESHARK_INVARIANT_RESOLVED_UNSUPPORTED;
}

static uint8_t gameshark_resolve_invariant_spec_kind(gameshark_invariant_spec *spec)
{
	if (spec->target_kind == GAMESHARK_INVARIANT_TARGET_FUNCTION) {
		zend_function *function = zend_hash_find_ptr(EG(function_table), spec->match_key);
		return gameshark_invariant_resolved_kind_for_function(function, spec->target_kind);
	}

	const char *match_start = ZSTR_VAL(spec->match_key);
	size_t match_len = ZSTR_LEN(spec->match_key);
	const char *method_separator = gameshark_find_bytes(match_start, match_len, "::", 2);
	if (method_separator == NULL) {
		return GAMESHARK_INVARIANT_RESOLVED_UNKNOWN;
	}

	size_t class_len = (size_t) (method_separator - match_start);
	const char *method_start = method_separator + 2;
	size_t method_len = match_len - class_len - 2;
	zend_string *class_key = zend_string_init(match_start, class_len, 0);
	zend_class_entry *ce = zend_hash_find_ptr(EG(class_table), class_key);
	zend_string_release(class_key);
	if (ce == NULL) {
		return GAMESHARK_INVARIANT_RESOLVED_UNKNOWN;
	}

	zend_function *function = zend_hash_str_find_ptr(&ce->function_table, method_start, method_len);
	return gameshark_invariant_resolved_kind_for_function(function, spec->target_kind);
}

static void gameshark_refresh_invariant_resolution(void)
{
	gameshark_invariant_has_internal_hooks = false;
	for (size_t i = 0; i < gameshark_invariant_spec_count; i++) {
		gameshark_invariant_spec *spec = &gameshark_invariant_specs[i];
		spec->resolved_kind = gameshark_resolve_invariant_spec_kind(spec);
		if (gameshark_invariant_resolved_kind_is_internal(spec->resolved_kind)) {
			gameshark_invariant_has_internal_hooks = true;
		}
	}
}

static void gameshark_emit_builtin_invariant_warning(void)
{
	if (!gameshark_invariant_warn_builtins ||
		gameshark_invariant_internal_warning_emitted ||
		!gameshark_invariant_has_internal_hooks) {
		return;
	}

	const char *message = "php-gameshark: invariant hooks include built-in PHP targets; this may affect performance and program behavior.";
	if (sapi_module.name != NULL && strcmp(sapi_module.name, "cli") == 0) {
		fprintf(stderr, "%s\n", message);
	} else {
		php_log_err(message);
	}
	gameshark_invariant_internal_warning_emitted = true;
}

static bool gameshark_invariant_id_exists(zend_string *id)
{
	for (size_t i = 0; i < gameshark_invariant_spec_count; i++) {
		if (zend_string_equals(gameshark_invariant_specs[i].id, id)) {
			return true;
		}
	}
	return false;
}

static bool gameshark_append_invariant_spec(
	zend_string *id,
	zend_string *target,
	zend_string *match_key,
	uint8_t target_kind,
	uint8_t phase,
	zval *hook
) {
	if (gameshark_invariant_spec_count == gameshark_invariant_spec_capacity) {
		size_t new_capacity = gameshark_invariant_spec_capacity == 0 ? 8 : gameshark_invariant_spec_capacity * 2;
		gameshark_invariant_specs = erealloc(gameshark_invariant_specs, new_capacity * sizeof(gameshark_invariant_spec));
		gameshark_invariant_spec_capacity = new_capacity;
	}

	gameshark_invariant_spec *spec = &gameshark_invariant_specs[gameshark_invariant_spec_count++];
	spec->id = zend_string_copy(id);
	spec->target = zend_string_copy(target);
	spec->match_key = zend_string_copy(match_key);
	spec->target_kind = target_kind;
		spec->phase = phase;
		spec->resolved_kind = GAMESHARK_INVARIANT_RESOLVED_UNKNOWN;
		ZVAL_COPY(&spec->hook, hook);
		spec->matched = false;
		spec->executions = 0;
		spec->hook_exceptions = 0;
		return true;
	}

static bool gameshark_validate_and_store_invariants(zval *config)
{
	if (Z_TYPE_P(config) != IS_ARRAY) {
		gameshark_set_invariant_error("invariant file must return an array");
		return false;
	}

	HashTable *specs = Z_ARRVAL_P(config);
	zend_ulong expected_index = 0;
	zend_string *top_key;
	zend_ulong top_num_key;
	zval *spec_zv;

	ZEND_HASH_FOREACH_KEY_VAL(specs, top_num_key, top_key, spec_zv) {
		if (top_key != NULL || top_num_key != expected_index) {
			gameshark_set_invariant_error("invariant file must return a zero-indexed list of specs");
			return false;
		}
		expected_index++;

		uint32_t spec_index = (uint32_t) top_num_key;
		if (Z_TYPE_P(spec_zv) != IS_ARRAY) {
			gameshark_set_invariant_error("spec %u must be an array", spec_index);
			return false;
		}

		HashTable *spec = Z_ARRVAL_P(spec_zv);
		if (!gameshark_validate_spec_keys(spec, spec_index)) {
			return false;
		}

		zval *id_zv = zend_hash_str_find(spec, "id", sizeof("id") - 1);
		zval *target_zv = zend_hash_str_find(spec, "target", sizeof("target") - 1);
		zval *when_zv = zend_hash_str_find(spec, "when", sizeof("when") - 1);
		zval *hook_zv = zend_hash_str_find(spec, "hook", sizeof("hook") - 1);

		if (id_zv == NULL || Z_TYPE_P(id_zv) != IS_STRING || Z_STRLEN_P(id_zv) == 0) {
			gameshark_set_invariant_error("spec %u requires a non-empty string id", spec_index);
			return false;
		}
		if (gameshark_invariant_id_exists(Z_STR_P(id_zv))) {
			gameshark_set_invariant_error("duplicate invariant id \"%s\"", Z_STRVAL_P(id_zv));
			return false;
		}
		if (target_zv == NULL || Z_TYPE_P(target_zv) != IS_STRING || Z_STRLEN_P(target_zv) == 0) {
			gameshark_set_invariant_error("spec %u requires a non-empty string target", spec_index);
			return false;
		}
		if (when_zv == NULL || Z_TYPE_P(when_zv) != IS_STRING) {
			gameshark_set_invariant_error("spec %u requires when to be \"pre\" or \"post\"", spec_index);
			return false;
		}
		if (hook_zv == NULL || !zend_is_callable(hook_zv, 0, NULL)) {
			gameshark_set_invariant_error("spec %u hook is not callable", spec_index);
			return false;
		}

		uint8_t phase;
		if (zend_string_equals_literal(Z_STR_P(when_zv), "pre")) {
			phase = GAMESHARK_INVARIANT_PHASE_PRE;
		} else if (zend_string_equals_literal(Z_STR_P(when_zv), "post")) {
			phase = GAMESHARK_INVARIANT_PHASE_POST;
		} else {
			gameshark_set_invariant_error("spec %u requires when to be \"pre\" or \"post\"", spec_index);
			return false;
		}

		uint8_t target_kind;
		zend_string *target = NULL;
		zend_string *match_key = NULL;
		if (!gameshark_normalize_invariant_target(Z_STR_P(target_zv), &target_kind, &target, &match_key, spec_index)) {
			return false;
		}

		gameshark_append_invariant_spec(Z_STR_P(id_zv), target, match_key, target_kind, phase, hook_zv);
		zend_string_release(target);
		zend_string_release(match_key);
	} ZEND_HASH_FOREACH_END();

	gameshark_invariants_loaded = true;
	gameshark_invariants_active = gameshark_invariant_spec_count > 0;
	return true;
}

static bool gameshark_load_invariants_file(const char *path)
{
	if (path == NULL || path[0] == '\0') {
		gameshark_set_invariant_error("gameshark.invariants_file is required when invariant mode is enabled");
		zend_error(E_ERROR, "Gameshark invariant config error: %s", ZSTR_VAL(gameshark_invariants_load_error));
		return false;
	}
	if (!gameshark_is_absolute_path(path)) {
		gameshark_set_invariant_error("gameshark.invariants_file must be an absolute path");
		zend_error(E_ERROR, "Gameshark invariant config error: %s", ZSTR_VAL(gameshark_invariants_load_error));
		return false;
	}
	if (access(path, R_OK) != 0) {
		gameshark_set_invariant_error("cannot read invariant file \"%s\"", path);
		zend_error(E_ERROR, "Gameshark invariant config error: %s", ZSTR_VAL(gameshark_invariants_load_error));
		return false;
	}

	zval retval;
	ZVAL_UNDEF(&retval);
	zend_file_handle file_handle;
	zend_stream_init_filename(&file_handle, path);
	zend_result result = zend_execute_scripts(ZEND_REQUIRE, &retval, 1, &file_handle);
	zend_destroy_file_handle(&file_handle);

	if (result != SUCCESS) {
		if (Z_TYPE(retval) != IS_UNDEF) {
			zval_ptr_dtor(&retval);
		}
		if (gameshark_invariants_load_error == NULL) {
			gameshark_set_invariant_error("failed to execute invariant file \"%s\"", path);
		}
		return false;
	}

	bool ok = gameshark_validate_and_store_invariants(&retval);
	if (Z_TYPE(retval) != IS_UNDEF) {
		zval_ptr_dtor(&retval);
	}

	if (!ok) {
		zend_error(E_ERROR, "Gameshark invariant config error: %s", ZSTR_VAL(gameshark_invariants_load_error));
		return false;
	}

	return true;
}

static bool gameshark_configure_trace_value(const char *trace_value)
{
	if (trace_value == NULL || trace_value[0] == '\0') {
		return false;
	}

	gameshark_trace_value = estrdup(trace_value);
	gameshark_trace_value_len = strlen(trace_value);

	zend_long long_value = 0;
	double double_value = 0.0;
	uint8_t numeric_type = is_numeric_string(trace_value, gameshark_trace_value_len, &long_value, &double_value, 0);

	if (numeric_type == IS_LONG || numeric_type == IS_DOUBLE) {
		zval number;
		zend_string *number_string;

		gameshark_trace_value_is_numeric = true;
		gameshark_trace_value_is_long = numeric_type == IS_LONG;
		gameshark_trace_long_value = long_value;
		gameshark_trace_double_value = numeric_type == IS_LONG ? (double) long_value : double_value;

		if (gameshark_trace_value_is_long) {
			ZVAL_LONG(&number, long_value);
		} else {
			ZVAL_DOUBLE(&number, double_value);
		}
		number_string = zval_get_string(&number);
		gameshark_trace_string_match = estrndup(ZSTR_VAL(number_string), ZSTR_LEN(number_string));
		gameshark_trace_string_match_len = ZSTR_LEN(number_string);
		zend_string_release(number_string);
	} else {
		gameshark_trace_string_match = estrndup(trace_value, gameshark_trace_value_len);
		gameshark_trace_string_match_len = gameshark_trace_value_len;
	}

	if (gameshark_trace_string_match_len == 0) {
		return false;
	}

	bool added = false;
	gameshark_add_tracked_trace_value(gameshark_trace_string_match, gameshark_trace_string_match_len, 0, 0, &added);
	return gameshark_trace_value_count > 0;
}

static zend_string *gameshark_make_preview(const char *value, size_t value_len, size_t match_offset, size_t match_len)
{
	size_t start = match_offset > GAMESHARK_TRACE_PREVIEW_CONTEXT ? match_offset - GAMESHARK_TRACE_PREVIEW_CONTEXT : 0;
	size_t end = match_offset + match_len + GAMESHARK_TRACE_PREVIEW_CONTEXT;
	if (end > value_len) {
		end = value_len;
	}

	smart_str preview = {0};
	if (start > 0) {
		smart_str_appends(&preview, "...");
	}
	smart_str_appendl(&preview, value + start, end - start);
	if (end < value_len) {
		smart_str_appends(&preview, "...");
	}
	smart_str_0(&preview);

	if (preview.s == NULL) {
		return zend_empty_string;
	}
	return preview.s;
}

static void gameshark_append_escaped_path_key(smart_str *path, zend_string *key)
{
	for (size_t i = 0; i < ZSTR_LEN(key); i++) {
		char ch = ZSTR_VAL(key)[i];
		if (ch == '"' || ch == '\\') {
			smart_str_appendc(path, '\\');
			smart_str_appendc(path, ch);
		} else if (ch == '\0') {
			smart_str_appends(path, "\\0");
		} else {
			smart_str_appendc(path, ch);
		}
	}
}

static zend_string *gameshark_array_child_path(zend_string *path, zend_string *string_key, zend_ulong numeric_key)
{
	smart_str child = {0};
	smart_str_append(&child, path);
	if (string_key != NULL) {
		smart_str_appends(&child, "[\"");
		gameshark_append_escaped_path_key(&child, string_key);
		smart_str_appends(&child, "\"]");
	} else {
		smart_str_appendc(&child, '[');
		smart_str_append_unsigned(&child, numeric_key);
		smart_str_appendc(&child, ']');
	}
	smart_str_0(&child);
	return child.s;
}

static zend_string *gameshark_object_child_path(zend_string *path, zend_string *property_name)
{
	smart_str child = {0};
	smart_str_append(&child, path);
	smart_str_appends(&child, "->");
	gameshark_append_escaped_path_key(&child, property_name);
	smart_str_0(&child);
	return child.s;
}

static void gameshark_json_append_string(smart_str *json, const char *value, size_t value_len)
{
	smart_str_appendc(json, '"');
	for (size_t i = 0; i < value_len; i++) {
		unsigned char ch = (unsigned char) value[i];
		switch (ch) {
			case '"':
				smart_str_appends(json, "\\\"");
				break;
			case '\\':
				smart_str_appends(json, "\\\\");
				break;
			case '\b':
				smart_str_appends(json, "\\b");
				break;
			case '\f':
				smart_str_appends(json, "\\f");
				break;
			case '\n':
				smart_str_appends(json, "\\n");
				break;
			case '\r':
				smart_str_appends(json, "\\r");
				break;
			case '\t':
				smart_str_appends(json, "\\t");
				break;
			default:
				if (ch < 0x20) {
					char escaped[7];
					snprintf(escaped, sizeof(escaped), "\\u%04x", ch);
					smart_str_appends(json, escaped);
				} else {
					smart_str_appendc(json, (char) ch);
				}
				break;
		}
	}
	smart_str_appendc(json, '"');
}

static void gameshark_json_append_zstr(smart_str *json, zend_string *value)
{
	gameshark_json_append_string(json, ZSTR_VAL(value), ZSTR_LEN(value));
}

static void gameshark_json_append_core_str_or_null(smart_str *json, gameshark_core_str value)
{
	if (value.ptr == NULL) {
		smart_str_appends(json, "null");
		return;
	}
	gameshark_json_append_string(json, value.ptr, value.len);
}

static void gameshark_append_text_escaped(smart_str *text, const char *value, size_t value_len)
{
	for (size_t i = 0; i < value_len; i++) {
		unsigned char ch = (unsigned char) value[i];
		switch (ch) {
			case '"':
				smart_str_appends(text, "\\\"");
				break;
			case '\\':
				smart_str_appends(text, "\\\\");
				break;
			case '\n':
				smart_str_appends(text, "\\n");
				break;
			case '\r':
				smart_str_appends(text, "\\r");
				break;
			case '\t':
				smart_str_appends(text, "\\t");
				break;
			default:
				if (ch < 0x20) {
					smart_str_appendc(text, '?');
				} else {
					smart_str_appendc(text, (char) ch);
				}
				break;
		}
	}
}

static zend_string *gameshark_make_bounded_preview(const char *value, size_t value_len)
{
	if (value_len <= GAMESHARK_TRACE_ARG_STRING_MAX) {
		return zend_string_init(value, value_len, 0);
	}

	smart_str preview = {0};
	smart_str_appendl(&preview, value, GAMESHARK_TRACE_ARG_STRING_MAX);
	smart_str_appends(&preview, "...");
	smart_str_0(&preview);
	return preview.s != NULL ? preview.s : zend_empty_string;
}

static void gameshark_match_context_add(
	gameshark_match_context *context,
	zend_string *path,
	uint32_t matched_value_id,
	const char *matched_value,
	size_t matched_value_len,
	const char *preview_value,
	size_t preview_value_len,
	size_t preview_match_offset,
	size_t preview_match_len
) {
	zend_string *preview = gameshark_make_preview(preview_value, preview_value_len, preview_match_offset, preview_match_len);

	if (context->count < GAMESHARK_TRACE_MAX_ARG_MATCHES) {
		if (context->needs_comma) {
			smart_str_appendc(&context->paths_json, ',');
			smart_str_appendc(&context->matches_json, ',');
		}
		gameshark_json_append_zstr(&context->paths_json, path);
		smart_str_appendc(&context->matches_json, '{');
		smart_str_appends(&context->matches_json, "\"path\":");
		gameshark_json_append_zstr(&context->matches_json, path);
		smart_str_appends(&context->matches_json, ",\"matched_value_id\":");
		smart_str_append_unsigned(&context->matches_json, matched_value_id);
		smart_str_appends(&context->matches_json, ",\"matched_value\":");
		gameshark_json_append_string(&context->matches_json, matched_value, matched_value_len);
		smart_str_appends(&context->matches_json, ",\"preview\":");
		gameshark_json_append_zstr(&context->matches_json, preview);
		smart_str_appends(&context->matches_json, ",\"value\":");
		gameshark_json_append_string(&context->matches_json, preview_value, preview_value_len);
		smart_str_appendc(&context->matches_json, '}');
		context->needs_comma = true;
	}

	if (context->first_path == NULL) {
		context->first_path = zend_string_copy(path);
		context->first_preview = zend_string_copy(preview);
	}

	context->count++;

	if (preview != zend_empty_string) {
		zend_string_release(preview);
	}
}

static void gameshark_match_context_free(gameshark_match_context *context)
{
	smart_str_free(&context->paths_json);
	smart_str_free(&context->matches_json);
	if (context->first_path != NULL) {
		zend_string_release(context->first_path);
		context->first_path = NULL;
	}
	if (context->first_preview != NULL) {
		zend_string_release(context->first_preview);
		context->first_preview = NULL;
	}
}

static bool gameshark_match_array_seen(gameshark_match_context *context, HashTable *array)
{
	for (size_t i = 0; i < context->visited_array_count; i++) {
		if (context->visited_arrays[i] == array) {
			return true;
		}
	}
	if (context->visited_array_count < GAMESHARK_TRACE_MAX_VISITED) {
		context->visited_arrays[context->visited_array_count++] = array;
	}
	return false;
}

static bool gameshark_match_object_seen(gameshark_match_context *context, zend_object *object)
{
	for (size_t i = 0; i < context->visited_object_count; i++) {
		if (context->visited_objects[i] == object) {
			return true;
		}
	}
	if (context->visited_object_count < GAMESHARK_TRACE_MAX_VISITED) {
		context->visited_objects[context->visited_object_count++] = object;
	}
	return false;
}

static void gameshark_collect_value_matches(gameshark_match_context *context, zval *value, zend_string *path, uint32_t depth);

static gameshark_tracked_trace_value *gameshark_find_string_trace_match(const char *value, size_t value_len, const char **match)
{
	for (size_t i = 0; i < gameshark_trace_value_count; i++) {
		zend_string *tracked = gameshark_trace_values[i].value;
		if (tracked == NULL) {
			continue;
		}
		const char *found = gameshark_find_bytes(value, value_len, ZSTR_VAL(tracked), ZSTR_LEN(tracked));
		if (found != NULL) {
			if (match != NULL) {
				*match = found;
			}
			return &gameshark_trace_values[i];
		}
	}
	return NULL;
}

static void gameshark_collect_array_matches(gameshark_match_context *context, HashTable *array, zend_string *path, uint32_t depth)
{
	if (depth >= GAMESHARK_TRACE_MAX_DEPTH || gameshark_match_array_seen(context, array)) {
		return;
	}

	zend_ulong numeric_key;
	zend_string *string_key;
	zval *entry;

	ZEND_HASH_FOREACH_KEY_VAL_IND(array, numeric_key, string_key, entry) {
		zend_string *child_path = gameshark_array_child_path(path, string_key, numeric_key);
		gameshark_collect_value_matches(context, entry, child_path, depth + 1);
		zend_string_release(child_path);
	} ZEND_HASH_FOREACH_END();
}

static void gameshark_collect_object_matches(gameshark_match_context *context, zend_object *object, zend_string *path, uint32_t depth)
{
	if (depth >= GAMESHARK_TRACE_MAX_DEPTH || gameshark_match_object_seen(context, object)) {
		return;
	}

	zend_property_info *property_info;
	ZEND_HASH_FOREACH_PTR(&object->ce->properties_info, property_info) {
		if ((property_info->flags & ZEND_ACC_STATIC) || property_info->offset == ZEND_VIRTUAL_PROPERTY_OFFSET) {
			continue;
		}

		zval *property = OBJ_PROP(object, property_info->offset);
		if (Z_TYPE_P(property) == IS_UNDEF || (Z_PROP_FLAG_P(property) & (IS_PROP_UNINIT | IS_PROP_LAZY))) {
			continue;
		}

		zend_string *child_path = gameshark_object_child_path(path, property_info->name);
		gameshark_collect_value_matches(context, property, child_path, depth + 1);
		zend_string_release(child_path);
	} ZEND_HASH_FOREACH_END();

	if (object->properties == NULL) {
		return;
	}

	zend_ulong numeric_key;
	zend_string *string_key;
	zval *property;

	ZEND_HASH_FOREACH_KEY_VAL_IND(object->properties, numeric_key, string_key, property) {
		if (string_key == NULL || gameshark_string_has_nul(string_key) || zend_hash_exists(&object->ce->properties_info, string_key)) {
			continue;
		}

		zend_string *child_path = gameshark_object_child_path(path, string_key);
		gameshark_collect_value_matches(context, property, child_path, depth + 1);
		zend_string_release(child_path);
	} ZEND_HASH_FOREACH_END();
}

static void gameshark_collect_value_matches(gameshark_match_context *context, zval *value, zend_string *path, uint32_t depth)
{
	ZVAL_DEREF(value);

	switch (Z_TYPE_P(value)) {
		case IS_STRING: {
			const char *match = NULL;
			gameshark_tracked_trace_value *tracked = gameshark_find_string_trace_match(Z_STRVAL_P(value), Z_STRLEN_P(value), &match);
			if (tracked != NULL && match != NULL) {
				gameshark_match_context_add(
					context,
					path,
					tracked->value_id,
					ZSTR_VAL(tracked->value),
					ZSTR_LEN(tracked->value),
					Z_STRVAL_P(value),
					Z_STRLEN_P(value),
					(size_t) (match - Z_STRVAL_P(value)),
					ZSTR_LEN(tracked->value)
				);
			}
			break;
		}
		case IS_LONG:
			if (gameshark_numeric_matches_long(Z_LVAL_P(value))) {
				zend_string *observed_value = gameshark_scalar_value(value);
				gameshark_match_context_add(
					context,
					path,
					1,
					ZSTR_VAL(observed_value),
					ZSTR_LEN(observed_value),
					ZSTR_VAL(observed_value),
					ZSTR_LEN(observed_value),
					0,
					ZSTR_LEN(observed_value)
				);
				zend_string_release(observed_value);
			}
			break;
		case IS_DOUBLE:
			if (gameshark_numeric_matches_double(Z_DVAL_P(value))) {
				zend_string *observed_value = gameshark_scalar_value(value);
				gameshark_match_context_add(
					context,
					path,
					1,
					ZSTR_VAL(observed_value),
					ZSTR_LEN(observed_value),
					ZSTR_VAL(observed_value),
					ZSTR_LEN(observed_value),
					0,
					ZSTR_LEN(observed_value)
				);
				zend_string_release(observed_value);
			}
			break;
		case IS_ARRAY:
			gameshark_collect_array_matches(context, Z_ARRVAL_P(value), path, depth);
			break;
		case IS_OBJECT:
			gameshark_collect_object_matches(context, Z_OBJ_P(value), path, depth);
			break;
		default:
			break;
	}
}

static zend_string *gameshark_value_preview(zval *value, gameshark_match_context *matches)
{
	ZVAL_DEREF(value);

	switch (Z_TYPE_P(value)) {
		case IS_STRING:
			if (matches->first_preview != NULL) {
				return zend_string_copy(matches->first_preview);
			}
			return gameshark_make_bounded_preview(Z_STRVAL_P(value), Z_STRLEN_P(value));
		case IS_LONG:
			return strpprintf(0, ZEND_LONG_FMT, Z_LVAL_P(value));
		case IS_DOUBLE: {
			zval copy;
			zend_string *preview;
			ZVAL_DOUBLE(&copy, Z_DVAL_P(value));
			preview = zval_get_string(&copy);
			return preview;
		}
		case IS_TRUE:
			return zend_string_init("true", sizeof("true") - 1, 0);
		case IS_FALSE:
			return zend_string_init("false", sizeof("false") - 1, 0);
		case IS_NULL:
			return zend_string_init("null", sizeof("null") - 1, 0);
		case IS_ARRAY:
			return strpprintf(0, "array(%u)", zend_hash_num_elements(Z_ARRVAL_P(value)));
		case IS_OBJECT:
			return strpprintf(0, "object(%s)", ZSTR_VAL(Z_OBJCE_P(value)->name));
		default: {
			const char *type_name = zend_zval_type_name(value);
			return zend_string_init(type_name, strlen(type_name), 0);
		}
	}
}

static zend_string *gameshark_scalar_value(zval *value)
{
	ZVAL_DEREF(value);

	switch (Z_TYPE_P(value)) {
		case IS_STRING:
			return zend_string_copy(Z_STR_P(value));
		case IS_LONG:
			return strpprintf(0, ZEND_LONG_FMT, Z_LVAL_P(value));
		case IS_DOUBLE: {
			zval copy;
			ZVAL_DOUBLE(&copy, Z_DVAL_P(value));
			return zval_get_string(&copy);
		}
		case IS_TRUE:
			return zend_string_init("true", sizeof("true") - 1, 0);
		case IS_FALSE:
			return zend_string_init("false", sizeof("false") - 1, 0);
		case IS_NULL:
			return zend_string_init("null", sizeof("null") - 1, 0);
		default:
			return NULL;
	}
}

static void gameshark_append_arg_text(smart_str *text, uint32_t index, zval *argument, zend_string *preview, gameshark_match_context *matches)
{
	zval *value = argument;
	ZVAL_DEREF(value);

	smart_str_appends(text, "arg");
	smart_str_append_unsigned(text, index);
	smart_str_appendc(text, '=');

	if (Z_TYPE_P(value) == IS_STRING) {
		smart_str_appendc(text, '"');
		gameshark_append_text_escaped(text, ZSTR_VAL(preview), ZSTR_LEN(preview));
		smart_str_appendc(text, '"');
	} else {
		gameshark_append_text_escaped(text, ZSTR_VAL(preview), ZSTR_LEN(preview));
	}

	if (matches->first_path != NULL && matches->first_preview != NULL) {
		smart_str_appends(text, " matches ");
		smart_str_append(text, matches->first_path);
		smart_str_appends(text, "=\"");
		gameshark_append_text_escaped(text, ZSTR_VAL(matches->first_preview), ZSTR_LEN(matches->first_preview));
		smart_str_appendc(text, '"');
		if (matches->count > 1) {
			smart_str_appends(text, " +");
			smart_str_append_unsigned(text, matches->count - 1);
			smart_str_appends(text, " more");
		}
	}
}

static uint32_t gameshark_frame_line(zend_execute_data *frame, zend_execute_data *matched_frame)
{
	zend_function *function = frame->func;
	if (function == NULL || function->type != ZEND_USER_FUNCTION) {
		return 0;
	}
	if (frame != matched_frame && frame->opline != NULL) {
		return frame->opline->lineno;
	}
	return function->op_array.line_start;
}

static void gameshark_append_function_display(smart_str *stack, zend_function *function)
{
	if (function == NULL || function->common.function_name == NULL) {
		smart_str_appends(stack, "{main}");
		return;
	}

	if (function->common.scope != NULL) {
		smart_str_append(stack, function->common.scope->name);
		smart_str_appends(stack, "::");
	}

	if (function->type == ZEND_USER_FUNCTION && (function->common.fn_flags & ZEND_ACC_CLOSURE)) {
		smart_str_appends(stack, "{closure}");
	} else {
		smart_str_append(stack, function->common.function_name);
	}
}

static zend_string *gameshark_build_stack(zend_execute_data *execute_data, zend_string **stack_json)
{
	smart_str stack = {0};
	smart_str json = {0};
	uint32_t frame_count = 0;
	bool needs_frame_comma = false;

	smart_str_appendc(&json, '[');
	for (zend_execute_data *frame = execute_data; frame != NULL && frame_count < GAMESHARK_TRACE_STACK_MAX_FRAMES; frame = frame->prev_execute_data) {
		zend_function *function = frame->func;
		uint32_t line = gameshark_frame_line(frame, execute_data);

		if (frame_count > 0) {
			smart_str_appendc(&stack, '\n');
		}
		smart_str_appendc(&stack, '#');
		smart_str_append_unsigned(&stack, frame_count);
		smart_str_appendc(&stack, ' ');
		gameshark_append_function_display(&stack, function);
		smart_str_appendc(&stack, '(');

		if (needs_frame_comma) {
			smart_str_appendc(&json, ',');
		}
		needs_frame_comma = true;
		smart_str_appendc(&json, '{');
		smart_str_appends(&json, "\"index\":");
		smart_str_append_unsigned(&json, frame_count);
		smart_str_appends(&json, ",\"display_name\":");
		{
			smart_str display_name = {0};
			gameshark_append_function_display(&display_name, function);
			smart_str_0(&display_name);
			if (display_name.s != NULL) {
				gameshark_json_append_zstr(&json, display_name.s);
				zend_string_release(display_name.s);
			} else {
				gameshark_json_append_string(&json, "", 0);
			}
		}
		smart_str_appends(&json, ",\"function\":");
		if (function == NULL || function->common.function_name == NULL) {
			gameshark_json_append_string(&json, "{main}", sizeof("{main}") - 1);
		} else if (function->type == ZEND_USER_FUNCTION && (function->common.fn_flags & ZEND_ACC_CLOSURE)) {
			gameshark_json_append_string(&json, "{closure}", sizeof("{closure}") - 1);
		} else {
			gameshark_json_append_zstr(&json, function->common.function_name);
		}
		smart_str_appends(&json, ",\"class\":");
		if (function != NULL && function->common.scope != NULL) {
			gameshark_json_append_zstr(&json, function->common.scope->name);
		} else {
			smart_str_appends(&json, "null");
		}
		smart_str_appends(&json, ",\"type\":");
		if (function != NULL && function->common.scope != NULL) {
			gameshark_json_append_string(&json, "::", 2);
		} else {
			smart_str_appends(&json, "null");
		}
		smart_str_appends(&json, ",\"file\":");
		if (function != NULL && function->type == ZEND_USER_FUNCTION && function->op_array.filename != NULL) {
			gameshark_json_append_zstr(&json, function->op_array.filename);
		} else {
			smart_str_appends(&json, "null");
		}
		smart_str_appends(&json, ",\"line\":");
		smart_str_append_unsigned(&json, line);
		smart_str_appends(&json, ",\"args\":[");

		if (function != NULL) {
			uint32_t num_args = ZEND_CALL_NUM_ARGS(frame);
			for (uint32_t i = 0; i < num_args; i++) {
				zval *argument = ZEND_CALL_ARG(frame, i + 1);
				zend_string *path = strpprintf(0, "arg%u", i);
				gameshark_match_context matches = {0};
				zend_string *preview;
				zend_string *scalar_value;

				gameshark_collect_value_matches(&matches, argument, path, 0);
				preview = gameshark_value_preview(argument, &matches);
				scalar_value = gameshark_scalar_value(argument);

				if (i > 0) {
					smart_str_appends(&stack, ", ");
					smart_str_appendc(&json, ',');
				}
				gameshark_append_arg_text(&stack, i, argument, preview, &matches);

				smart_str_appendc(&json, '{');
				smart_str_appends(&json, "\"index\":");
				smart_str_append_unsigned(&json, i);
				smart_str_appends(&json, ",\"type\":");
				gameshark_json_append_string(&json, zend_zval_type_name(argument), strlen(zend_zval_type_name(argument)));
				smart_str_appends(&json, ",\"preview\":");
				gameshark_json_append_zstr(&json, preview);
				smart_str_appends(&json, ",\"value\":");
				if (scalar_value != NULL) {
					gameshark_json_append_zstr(&json, scalar_value);
				} else {
					smart_str_appends(&json, "null");
				}
				smart_str_appends(&json, ",\"contains_trace_value\":");
				smart_str_appends(&json, matches.count > 0 ? "true" : "false");
				smart_str_appends(&json, ",\"matched_paths\":[");
				if (matches.paths_json.s != NULL) {
					smart_str_append(&json, matches.paths_json.s);
				}
				smart_str_appends(&json, "],\"matches\":[");
				if (matches.matches_json.s != NULL) {
					smart_str_append(&json, matches.matches_json.s);
				}
				smart_str_appends(&json, "]}");

				if (preview != zend_empty_string) {
					zend_string_release(preview);
				}
				if (scalar_value != NULL) {
					zend_string_release(scalar_value);
				}
				gameshark_match_context_free(&matches);
				zend_string_release(path);
			}
		}

		smart_str_appendc(&stack, ')');
		smart_str_appendc(&json, ']');

		if (function != NULL && function->type == ZEND_USER_FUNCTION && function->op_array.filename != NULL) {
			smart_str_appendc(&stack, ' ');
			smart_str_append(&stack, function->op_array.filename);
			smart_str_appendc(&stack, ':');
			smart_str_append_unsigned(&stack, line);
		}
		smart_str_appendc(&json, '}');
		frame_count++;
	}
	smart_str_appendc(&json, ']');

	smart_str_0(&stack);
	smart_str_0(&json);
	if (stack.s == NULL) {
		stack.s = zend_empty_string;
	}
	if (json.s == NULL) {
		json.s = zend_string_init("[]", sizeof("[]") - 1, 0);
	}
	*stack_json = json.s;
	return stack.s;
}

static void gameshark_record_trace_match(
	gameshark_trace_context *context,
	zend_string *argument_path,
	const zval *value,
	uint32_t matched_value_id,
	uint8_t match_kind,
	const char *matched_value,
	size_t matched_value_len,
	const char *preview_value,
	size_t preview_value_len,
	size_t preview_match_offset
) {
	zend_string *preview = gameshark_make_preview(preview_value, preview_value_len, preview_match_offset, matched_value_len);
	zend_string *observed_value = gameshark_scalar_value((zval *) value);
	zend_string *stack_json = NULL;
	zend_string *stack = gameshark_build_stack(context->execute_data, &stack_json);
	const char *type_name = zend_zval_type_name(value);
	if (observed_value == NULL) {
		observed_value = zend_string_copy(preview);
	}

	gameshark_core_trace_event event = {
		context->function,
		gameshark_zstr_to_core_str(argument_path),
		gameshark_mem_to_core_str(type_name, strlen(type_name)),
		matched_value_id,
		match_kind,
		gameshark_mem_to_core_str(matched_value, matched_value_len),
		gameshark_zstr_to_core_str(preview),
		gameshark_zstr_to_core_str(observed_value),
		gameshark_zstr_to_core_str(stack),
		gameshark_zstr_to_core_str(stack_json),
	};
	gameshark_core_record_trace_event(&event);
	if (matched_value_id > 0 && matched_value_id <= GAMESHARK_TRACE_MAX_TRACKED_VALUES) {
		context->matched_value_mask |= ((uint64_t) 1) << (matched_value_id - 1);
	}

	if (preview != zend_empty_string) {
		zend_string_release(preview);
	}
	if (observed_value != NULL) {
		zend_string_release(observed_value);
	}
	if (stack != zend_empty_string) {
		zend_string_release(stack);
	}
	if (stack_json != NULL) {
		zend_string_release(stack_json);
	}
}

static bool gameshark_numeric_matches_long(zend_long value)
{
	if (!gameshark_trace_value_is_numeric) {
		return false;
	}
	if (gameshark_trace_value_is_long) {
		return value == gameshark_trace_long_value;
	}
	return (double) value == gameshark_trace_double_value;
}

static bool gameshark_numeric_matches_double(double value)
{
	if (!gameshark_trace_value_is_numeric) {
		return false;
	}
	if (gameshark_trace_value_is_long) {
		return value == (double) gameshark_trace_long_value;
	}
	return value == gameshark_trace_double_value;
}

static bool gameshark_array_seen(gameshark_trace_context *context, HashTable *array)
{
	for (size_t i = 0; i < context->visited_array_count; i++) {
		if (context->visited_arrays[i] == array) {
			return true;
		}
	}
	if (context->visited_array_count < GAMESHARK_TRACE_MAX_VISITED) {
		context->visited_arrays[context->visited_array_count++] = array;
	}
	return false;
}

static bool gameshark_object_seen(gameshark_trace_context *context, zend_object *object)
{
	for (size_t i = 0; i < context->visited_object_count; i++) {
		if (context->visited_objects[i] == object) {
			return true;
		}
	}
	if (context->visited_object_count < GAMESHARK_TRACE_MAX_VISITED) {
		context->visited_objects[context->visited_object_count++] = object;
	}
	return false;
}

static bool gameshark_string_has_nul(zend_string *string)
{
	return memchr(ZSTR_VAL(string), '\0', ZSTR_LEN(string)) != NULL;
}

static void gameshark_trace_zval(gameshark_trace_context *context, zval *value, zend_string *path, uint32_t depth);

static void gameshark_trace_array(gameshark_trace_context *context, HashTable *array, zend_string *path, uint32_t depth)
{
	if (depth >= GAMESHARK_TRACE_MAX_DEPTH || gameshark_array_seen(context, array)) {
		return;
	}

	zend_ulong numeric_key;
	zend_string *string_key;
	zval *entry;

	ZEND_HASH_FOREACH_KEY_VAL_IND(array, numeric_key, string_key, entry) {
		zend_string *child_path = gameshark_array_child_path(path, string_key, numeric_key);
		gameshark_trace_zval(context, entry, child_path, depth + 1);
		zend_string_release(child_path);
	} ZEND_HASH_FOREACH_END();
}

static void gameshark_trace_object(gameshark_trace_context *context, zend_object *object, zend_string *path, uint32_t depth)
{
	if (depth >= GAMESHARK_TRACE_MAX_DEPTH || gameshark_object_seen(context, object)) {
		return;
	}

	zend_property_info *property_info;
	ZEND_HASH_FOREACH_PTR(&object->ce->properties_info, property_info) {
		if ((property_info->flags & ZEND_ACC_STATIC) || property_info->offset == ZEND_VIRTUAL_PROPERTY_OFFSET) {
			continue;
		}

		zval *property = OBJ_PROP(object, property_info->offset);
		if (Z_TYPE_P(property) == IS_UNDEF || (Z_PROP_FLAG_P(property) & (IS_PROP_UNINIT | IS_PROP_LAZY))) {
			continue;
		}

		zend_string *child_path = gameshark_object_child_path(path, property_info->name);
		gameshark_trace_zval(context, property, child_path, depth + 1);
		zend_string_release(child_path);
	} ZEND_HASH_FOREACH_END();

	if (object->properties == NULL) {
		return;
	}

	zend_ulong numeric_key;
	zend_string *string_key;
	zval *property;

	ZEND_HASH_FOREACH_KEY_VAL_IND(object->properties, numeric_key, string_key, property) {
		if (string_key == NULL || gameshark_string_has_nul(string_key) || zend_hash_exists(&object->ce->properties_info, string_key)) {
			continue;
		}

		zend_string *child_path = gameshark_object_child_path(path, string_key);
		gameshark_trace_zval(context, property, child_path, depth + 1);
		zend_string_release(child_path);
	} ZEND_HASH_FOREACH_END();
}

static void gameshark_trace_zval(gameshark_trace_context *context, zval *value, zend_string *path, uint32_t depth)
{
	ZVAL_DEREF(value);

	switch (Z_TYPE_P(value)) {
		case IS_STRING: {
			const char *match = NULL;
			gameshark_tracked_trace_value *tracked = gameshark_find_string_trace_match(Z_STRVAL_P(value), Z_STRLEN_P(value), &match);
			if (tracked != NULL && match != NULL) {
				gameshark_record_trace_match(
					context,
					path,
					value,
					tracked->value_id,
					gameshark_trace_value_is_numeric ? GAMESHARK_TRACE_MATCH_NUMERIC_STRING_CONTAINS : GAMESHARK_TRACE_MATCH_STRING_CONTAINS,
					ZSTR_VAL(tracked->value),
					ZSTR_LEN(tracked->value),
					Z_STRVAL_P(value),
					Z_STRLEN_P(value),
					(size_t) (match - Z_STRVAL_P(value))
				);
			}
			break;
		}
		case IS_LONG:
			if (gameshark_numeric_matches_long(Z_LVAL_P(value))) {
				gameshark_record_trace_match(
					context,
					path,
					value,
					1,
					GAMESHARK_TRACE_MATCH_NUMBER_EQUALS,
					gameshark_trace_string_match,
					gameshark_trace_string_match_len,
					gameshark_trace_string_match,
					gameshark_trace_string_match_len,
					0
				);
			}
			break;
		case IS_DOUBLE:
			if (gameshark_numeric_matches_double(Z_DVAL_P(value))) {
				gameshark_record_trace_match(
					context,
					path,
					value,
					1,
					GAMESHARK_TRACE_MATCH_NUMBER_EQUALS,
					gameshark_trace_string_match,
					gameshark_trace_string_match_len,
					gameshark_trace_string_match,
					gameshark_trace_string_match_len,
					0
				);
			}
			break;
		case IS_ARRAY:
			gameshark_trace_array(context, Z_ARRVAL_P(value), path, depth);
			break;
		case IS_OBJECT:
			gameshark_trace_object(context, Z_OBJ_P(value), path, depth);
			break;
		default:
			break;
	}
}

static uint64_t gameshark_trace_arguments(zend_execute_data *execute_data, gameshark_core_function_meta function)
{
	uint32_t num_args = ZEND_CALL_NUM_ARGS(execute_data);
	gameshark_trace_context context = {
		execute_data,
		function,
		0,
		{NULL},
		0,
		{NULL},
		0,
	};

	for (uint32_t i = 0; i < num_args; i++) {
		zval *argument = ZEND_CALL_ARG(execute_data, i + 1);
		zend_string *path = strpprintf(0, "arg%u", i);
		context.visited_array_count = 0;
		context.visited_object_count = 0;
		gameshark_trace_zval(&context, argument, path, 0);
		zend_string_release(path);
	}

	return context.matched_value_mask;
}

static zend_string *gameshark_transform_addslashes(zend_string *value)
{
	smart_str out = {0};
	bool changed = false;
	for (size_t i = 0; i < ZSTR_LEN(value); i++) {
		char ch = ZSTR_VAL(value)[i];
		if (ch == '\'' || ch == '"' || ch == '\\' || ch == '\0') {
			smart_str_appendc(&out, '\\');
			changed = true;
			if (ch == '\0') {
				smart_str_appendc(&out, '0');
				continue;
			}
		}
		smart_str_appendc(&out, ch);
	}
	if (!changed) {
		smart_str_free(&out);
		return NULL;
	}
	smart_str_0(&out);
	return out.s;
}

static zend_string *gameshark_transform_sql_quotes(zend_string *value)
{
	smart_str out = {0};
	bool changed = false;
	for (size_t i = 0; i < ZSTR_LEN(value); i++) {
		char ch = ZSTR_VAL(value)[i];
		smart_str_appendc(&out, ch);
		if (ch == '\'' || ch == '"') {
			smart_str_appendc(&out, ch);
			changed = true;
		}
	}
	if (!changed) {
		smart_str_free(&out);
		return NULL;
	}
	smart_str_0(&out);
	return out.s;
}

static zend_string *gameshark_transform_html(zend_string *value)
{
	smart_str out = {0};
	bool changed = false;
	for (size_t i = 0; i < ZSTR_LEN(value); i++) {
		switch (ZSTR_VAL(value)[i]) {
			case '&':
				smart_str_appends(&out, "&amp;");
				changed = true;
				break;
			case '<':
				smart_str_appends(&out, "&lt;");
				changed = true;
				break;
			case '>':
				smart_str_appends(&out, "&gt;");
				changed = true;
				break;
			case '"':
				smart_str_appends(&out, "&quot;");
				changed = true;
				break;
			case '\'':
				smart_str_appends(&out, "&#039;");
				changed = true;
				break;
			default:
				smart_str_appendc(&out, ZSTR_VAL(value)[i]);
				break;
		}
	}
	if (!changed) {
		smart_str_free(&out);
		return NULL;
	}
	smart_str_0(&out);
	return out.s;
}

static bool gameshark_url_unreserved(unsigned char ch)
{
	return (ch >= 'A' && ch <= 'Z') || (ch >= 'a' && ch <= 'z') || (ch >= '0' && ch <= '9') || ch == '-' || ch == '_' || ch == '.';
}

static zend_string *gameshark_transform_url(zend_string *value, bool raw)
{
	static const char hex[] = "0123456789ABCDEF";
	smart_str out = {0};
	bool changed = false;
	for (size_t i = 0; i < ZSTR_LEN(value); i++) {
		unsigned char ch = (unsigned char) ZSTR_VAL(value)[i];
		if (gameshark_url_unreserved(ch)) {
			smart_str_appendc(&out, (char) ch);
		} else if (!raw && ch == ' ') {
			smart_str_appendc(&out, '+');
			changed = true;
		} else {
			smart_str_appendc(&out, '%');
			smart_str_appendc(&out, hex[ch >> 4]);
			smart_str_appendc(&out, hex[ch & 0x0f]);
			changed = true;
		}
	}
	if (!changed) {
		smart_str_free(&out);
		return NULL;
	}
	smart_str_0(&out);
	return out.s;
}

static zend_string *gameshark_transform_json_escape(zend_string *value)
{
	smart_str out = {0};
	bool changed = false;
	for (size_t i = 0; i < ZSTR_LEN(value); i++) {
		switch (ZSTR_VAL(value)[i]) {
			case '"':
				smart_str_appends(&out, "\\\"");
				changed = true;
				break;
			case '\\':
				smart_str_appends(&out, "\\\\");
				changed = true;
				break;
			case '\n':
				smart_str_appends(&out, "\\n");
				changed = true;
				break;
			case '\r':
				smart_str_appends(&out, "\\r");
				changed = true;
				break;
			case '\t':
				smart_str_appends(&out, "\\t");
				changed = true;
				break;
			default:
				smart_str_appendc(&out, ZSTR_VAL(value)[i]);
				break;
		}
	}
	if (!changed) {
		smart_str_free(&out);
		return NULL;
	}
	smart_str_0(&out);
	return out.s;
}

static bool gameshark_regex_needs_quote(char ch)
{
	switch (ch) {
		case '.':
		case '\\':
		case '+':
		case '*':
		case '?':
		case '[':
		case '^':
		case ']':
		case '$':
		case '(':
		case ')':
		case '{':
		case '}':
		case '=':
		case '!':
		case '<':
		case '>':
		case '|':
		case ':':
		case '-':
		case '#':
			return true;
		default:
			return false;
	}
}

static zend_string *gameshark_transform_regex_quote(zend_string *value)
{
	smart_str out = {0};
	bool changed = false;
	for (size_t i = 0; i < ZSTR_LEN(value); i++) {
		char ch = ZSTR_VAL(value)[i];
		if (gameshark_regex_needs_quote(ch)) {
			smart_str_appendc(&out, '\\');
			changed = true;
		}
		smart_str_appendc(&out, ch);
	}
	if (!changed) {
		smart_str_free(&out);
		return NULL;
	}
	smart_str_0(&out);
	return out.s;
}

static zend_string *gameshark_transform_like_escape(zend_string *value)
{
	smart_str out = {0};
	bool changed = false;
	for (size_t i = 0; i < ZSTR_LEN(value); i++) {
		char ch = ZSTR_VAL(value)[i];
		if (ch == '%' || ch == '_' || ch == '\\') {
			smart_str_appendc(&out, '\\');
			changed = true;
		}
		smart_str_appendc(&out, ch);
	}
	if (!changed) {
		smart_str_free(&out);
		return NULL;
	}
	smart_str_0(&out);
	return out.s;
}

static zend_string *gameshark_transform_strip_slashes(zend_string *value)
{
	smart_str out = {0};
	bool changed = false;
	for (size_t i = 0; i < ZSTR_LEN(value); i++) {
		char ch = ZSTR_VAL(value)[i];
		if (ch == '\\' && i + 1 < ZSTR_LEN(value)) {
			char next = ZSTR_VAL(value)[i + 1];
			if (next == '\\' || next == '\'' || next == '"' || next == '0') {
				smart_str_appendc(&out, next == '0' ? '\0' : next);
				i++;
				changed = true;
				continue;
			}
		}
		smart_str_appendc(&out, ch);
	}
	if (!changed) {
		smart_str_free(&out);
		return NULL;
	}
	smart_str_0(&out);
	return out.s;
}

static void gameshark_consider_transform_candidate(
	gameshark_tracked_trace_value *parent,
	zend_string *candidate,
	const char *kind,
	zval *retval,
	gameshark_core_function_meta function
) {
	if (candidate == NULL) {
		return;
	}
	if (ZSTR_LEN(candidate) == 0 || ZSTR_LEN(candidate) > GAMESHARK_TRACE_MAX_TRANSFORM_VALUE_LEN) {
		zend_string_release(candidate);
		return;
	}
	if (ZSTR_LEN(candidate) == ZSTR_LEN(parent->value) && memcmp(ZSTR_VAL(candidate), ZSTR_VAL(parent->value), ZSTR_LEN(candidate)) == 0) {
		zend_string_release(candidate);
		return;
	}

	const char *match = gameshark_find_bytes(Z_STRVAL_P(retval), Z_STRLEN_P(retval), ZSTR_VAL(candidate), ZSTR_LEN(candidate));
	if (match == NULL) {
		zend_string_release(candidate);
		return;
	}

	bool added = false;
	uint32_t value_id = gameshark_add_tracked_trace_value(
		ZSTR_VAL(candidate),
		ZSTR_LEN(candidate),
		parent->value_id,
		parent->depth + 1,
		&added
	);
	if (value_id != 0 && added) {
		zend_string *preview = gameshark_make_preview(
			Z_STRVAL_P(retval),
			Z_STRLEN_P(retval),
			(size_t) (match - Z_STRVAL_P(retval)),
			ZSTR_LEN(candidate)
		);
		gameshark_core_transformed_value transformed = {
			value_id,
			parent->value_id,
			function,
			gameshark_mem_to_core_str(kind, strlen(kind)),
			gameshark_zstr_to_core_str(candidate),
			gameshark_zstr_to_core_str(preview),
		};
		gameshark_core_record_transformed_value(&transformed);
		if (preview != zend_empty_string) {
			zend_string_release(preview);
		}
	}

	zend_string_release(candidate);
}

static void gameshark_follow_return_transforms(zend_execute_data *execute_data, zval *retval)
{
	if (!gameshark_trace_follow_transforms) {
		return;
	}

	size_t frame_index = gameshark_trace_frame_count;
	while (frame_index > 0) {
		frame_index--;
		if (gameshark_trace_frames[frame_index].execute_data == execute_data) {
			break;
		}
	}
	if (frame_index == gameshark_trace_frame_count || gameshark_trace_frames[frame_index].execute_data != execute_data) {
		return;
	}

	gameshark_trace_frame frame = gameshark_trace_frames[frame_index];
	if (frame_index + 1 < gameshark_trace_frame_count) {
		memmove(
			&gameshark_trace_frames[frame_index],
			&gameshark_trace_frames[frame_index + 1],
			(gameshark_trace_frame_count - frame_index - 1) * sizeof(gameshark_trace_frame)
		);
	}
	gameshark_trace_frame_count--;

	if (retval == NULL) {
		return;
	}
	ZVAL_DEREF(retval);
	if (Z_TYPE_P(retval) != IS_STRING || Z_STRLEN_P(retval) == 0) {
		return;
	}

	for (uint32_t value_id = 1; value_id <= GAMESHARK_TRACE_MAX_TRACKED_VALUES; value_id++) {
		if ((frame.matched_value_mask & (((uint64_t) 1) << (value_id - 1))) == 0) {
			continue;
		}
		gameshark_tracked_trace_value *parent = gameshark_find_tracked_trace_value(value_id);
		if (parent == NULL || parent->value == NULL || parent->depth >= GAMESHARK_TRACE_MAX_TRANSFORM_DEPTH) {
			continue;
		}

		gameshark_consider_transform_candidate(parent, gameshark_transform_addslashes(parent->value), "addslashes", retval, frame.function);
		gameshark_consider_transform_candidate(parent, gameshark_transform_sql_quotes(parent->value), "sql_quote_doubling", retval, frame.function);
		gameshark_consider_transform_candidate(parent, gameshark_transform_html(parent->value), "html_escape", retval, frame.function);
		gameshark_consider_transform_candidate(parent, gameshark_transform_url(parent->value, false), "urlencode", retval, frame.function);
		gameshark_consider_transform_candidate(parent, gameshark_transform_url(parent->value, true), "rawurlencode", retval, frame.function);
		gameshark_consider_transform_candidate(parent, gameshark_transform_json_escape(parent->value), "json_escape", retval, frame.function);
		gameshark_consider_transform_candidate(parent, gameshark_transform_regex_quote(parent->value), "regex_quote", retval, frame.function);
		gameshark_consider_transform_candidate(parent, gameshark_transform_like_escape(parent->value), "like_escape", retval, frame.function);
		gameshark_consider_transform_candidate(parent, gameshark_transform_strip_slashes(parent->value), "slash_stripping", retval, frame.function);
	}
}

static bool gameshark_invariant_function_supported(zend_function *function)
{
	return function != NULL &&
		function->common.function_name != NULL &&
		(function->type == ZEND_USER_FUNCTION || function->type == ZEND_INTERNAL_FUNCTION) &&
		(function->common.fn_flags & ZEND_ACC_CLOSURE) == 0;
}

static bool gameshark_invariant_user_function_supported(zend_function *function)
{
	return gameshark_invariant_function_supported(function) && function->type == ZEND_USER_FUNCTION;
}

static bool gameshark_invariant_internal_function_supported(zend_function *function)
{
	return gameshark_invariant_function_supported(function) && function->type == ZEND_INTERNAL_FUNCTION;
}

static zend_string *gameshark_invariant_function_match_key(zend_function *function, uint8_t *target_kind)
{
	if (!gameshark_invariant_function_supported(function)) {
		return NULL;
	}

	if (function->common.scope != NULL) {
		*target_kind = GAMESHARK_INVARIANT_TARGET_METHOD;
		smart_str key = {0};
		smart_str_append(&key, function->common.scope->name);
		smart_str_appends(&key, "::");
		smart_str_append(&key, function->common.function_name);
		smart_str_0(&key);
		zend_string *lower = zend_string_tolower(key.s);
		smart_str_free(&key);
		return lower;
	}

	*target_kind = GAMESHARK_INVARIANT_TARGET_FUNCTION;
	return zend_string_tolower(function->common.function_name);
}

static bool gameshark_invariant_has_hook(zend_string *match_key, uint8_t target_kind, uint8_t phase)
{
	for (size_t i = 0; i < gameshark_invariant_spec_count; i++) {
		gameshark_invariant_spec *spec = &gameshark_invariant_specs[i];
		if (spec->phase == phase && spec->target_kind == target_kind && zend_string_equals(spec->match_key, match_key)) {
			return true;
		}
	}
	return false;
}

static void gameshark_mark_invariant_spec_matched(gameshark_invariant_spec *spec, uint8_t resolved_kind)
{
	spec->matched = true;
	if (resolved_kind != GAMESHARK_INVARIANT_RESOLVED_UNKNOWN) {
		spec->resolved_kind = resolved_kind;
		if (gameshark_invariant_resolved_kind_is_internal(resolved_kind)) {
			gameshark_invariant_has_internal_hooks = true;
		}
	}
}

static void gameshark_capture_call_args(zend_execute_data *execute_data, zval *args)
{
	array_init(args);
	uint32_t argc = ZEND_CALL_NUM_ARGS(execute_data);
	for (uint32_t i = 0; i < argc; i++) {
		zval *argument = ZEND_CALL_ARG(execute_data, i + 1);
		Z_TRY_ADDREF_P(argument);
		add_next_index_zval(args, argument);
	}
}

static zval *gameshark_build_pre_hook_params(zend_execute_data *execute_data, bool has_object, uint32_t *param_count)
{
	uint32_t argc = ZEND_CALL_NUM_ARGS(execute_data);
	*param_count = argc + (has_object ? 1 : 0);
	if (*param_count == 0) {
		return NULL;
	}

	zval *params = safe_emalloc(*param_count, sizeof(zval), 0);
	uint32_t offset = 0;
	if (has_object) {
		ZVAL_COPY(&params[offset++], &execute_data->This);
	}
	for (uint32_t i = 0; i < argc; i++) {
		ZVAL_COPY(&params[offset++], ZEND_CALL_ARG(execute_data, i + 1));
	}
	return params;
}

static void gameshark_init_invariant_frame(gameshark_invariant_frame *frame, zend_execute_data *execute_data, bool has_object)
{
	frame->execute_data = execute_data;
	frame->has_object = has_object;
	gameshark_capture_call_args(execute_data, &frame->args);
	if (has_object) {
		ZVAL_COPY(&frame->object, &execute_data->This);
	} else {
		ZVAL_UNDEF(&frame->object);
	}
}

static void gameshark_clear_invariant_frame(gameshark_invariant_frame *frame)
{
	zval_ptr_dtor(&frame->args);
	if (frame->has_object) {
		zval_ptr_dtor(&frame->object);
	}
}

static zval *gameshark_build_post_hook_params(
	zval *retval,
	gameshark_invariant_frame *frame,
	uint32_t *param_count
) {
	*param_count = frame->has_object ? 3 : 2;
	zval *params = safe_emalloc(*param_count, sizeof(zval), 0);
	uint32_t offset = 0;
	if (frame->has_object) {
		ZVAL_COPY(&params[offset++], &frame->object);
	}
	ZVAL_COPY(&params[offset++], retval);
	ZVAL_COPY(&params[offset++], &frame->args);
	return params;
}

static void gameshark_free_hook_params(zval *params, uint32_t param_count)
{
	if (params == NULL) {
		return;
	}
	for (uint32_t i = 0; i < param_count; i++) {
		zval_ptr_dtor(&params[i]);
	}
	efree(params);
}

static void gameshark_call_invariant_hook(gameshark_invariant_spec *spec, zval *params, uint32_t param_count)
{
	zval hook_retval;
	ZVAL_UNDEF(&hook_retval);

	bool previous_executing_hook = gameshark_invariants_executing_hook;
	gameshark_invariants_executing_hook = true;
	spec->executions++;
	call_user_function(NULL, NULL, &spec->hook, &hook_retval, param_count, params);
	gameshark_invariants_executing_hook = previous_executing_hook;
	if (EG(exception)) {
		spec->hook_exceptions++;
	}

	if (Z_TYPE(hook_retval) != IS_UNDEF) {
		zval_ptr_dtor(&hook_retval);
	}
}

static void gameshark_run_pre_invariants(
	zend_execute_data *execute_data,
	zend_string *match_key,
	uint8_t target_kind,
	uint8_t resolved_kind,
	bool has_object
) {
	for (size_t i = 0; i < gameshark_invariant_spec_count; i++) {
		gameshark_invariant_spec *spec = &gameshark_invariant_specs[i];
		if (spec->phase != GAMESHARK_INVARIANT_PHASE_PRE || spec->target_kind != target_kind || !zend_string_equals(spec->match_key, match_key)) {
			continue;
		}

		gameshark_mark_invariant_spec_matched(spec, resolved_kind);
		uint32_t param_count = 0;
		zval *params = gameshark_build_pre_hook_params(execute_data, has_object, &param_count);
		if (gameshark_invariant_resolved_kind_is_internal(resolved_kind)) {
			gameshark_invariant_internal_pre_invocations++;
		}
		gameshark_call_invariant_hook(spec, params, param_count);
		gameshark_free_hook_params(params, param_count);
		if (EG(exception)) {
			if (gameshark_invariant_resolved_kind_is_internal(resolved_kind)) {
				gameshark_invariant_internal_hook_exceptions++;
			}
			return;
		}
	}
}

static void gameshark_push_invariant_frame(zend_execute_data *execute_data, bool has_object)
{
	if (gameshark_invariant_frame_count >= GAMESHARK_INVARIANT_MAX_ACTIVE_FRAMES) {
		return;
	}

	gameshark_invariant_frame *frame = &gameshark_invariant_frames[gameshark_invariant_frame_count++];
	gameshark_init_invariant_frame(frame, execute_data, has_object);
}

static gameshark_invariant_frame *gameshark_find_invariant_frame(zend_execute_data *execute_data, size_t *frame_index)
{
	for (size_t i = gameshark_invariant_frame_count; i > 0; i--) {
		size_t index = i - 1;
		if (gameshark_invariant_frames[index].execute_data == execute_data) {
			if (frame_index != NULL) {
				*frame_index = index;
			}
			return &gameshark_invariant_frames[index];
		}
	}
	return NULL;
}

static void gameshark_remove_invariant_frame(size_t frame_index)
{
	gameshark_invariant_frame *frame = &gameshark_invariant_frames[frame_index];
	gameshark_clear_invariant_frame(frame);

	if (frame_index + 1 < gameshark_invariant_frame_count) {
		memmove(
			&gameshark_invariant_frames[frame_index],
			&gameshark_invariant_frames[frame_index + 1],
			(gameshark_invariant_frame_count - frame_index - 1) * sizeof(gameshark_invariant_frame)
		);
	}
	gameshark_invariant_frame_count--;
}

static void gameshark_run_post_invariants_for_frame(
	zend_string *match_key,
	uint8_t target_kind,
	uint8_t resolved_kind,
	gameshark_invariant_frame *frame,
	zval *retval
)
{
	for (size_t i = 0; i < gameshark_invariant_spec_count; i++) {
		gameshark_invariant_spec *spec = &gameshark_invariant_specs[i];
		if (spec->phase != GAMESHARK_INVARIANT_PHASE_POST || spec->target_kind != target_kind || !zend_string_equals(spec->match_key, match_key)) {
			continue;
		}

		gameshark_mark_invariant_spec_matched(spec, resolved_kind);
		uint32_t param_count = 0;
		zval *params = gameshark_build_post_hook_params(retval, frame, &param_count);
		if (gameshark_invariant_resolved_kind_is_internal(resolved_kind)) {
			gameshark_invariant_internal_post_invocations++;
		}
		gameshark_call_invariant_hook(spec, params, param_count);
		gameshark_free_hook_params(params, param_count);
		if (EG(exception)) {
			if (gameshark_invariant_resolved_kind_is_internal(resolved_kind)) {
				gameshark_invariant_internal_hook_exceptions++;
			}
			break;
		}
	}
}

static void gameshark_run_post_invariants(zend_execute_data *execute_data, zval *retval)
{
	zend_function *function = execute_data->func;
	uint8_t target_kind;
	zend_string *match_key = gameshark_invariant_function_match_key(function, &target_kind);
	if (match_key == NULL) {
		return;
	}

	size_t frame_index = 0;
	gameshark_invariant_frame *frame = gameshark_find_invariant_frame(execute_data, &frame_index);
	if (frame == NULL) {
		zend_string_release(match_key);
		return;
	}

	uint8_t resolved_kind = gameshark_invariant_resolved_kind_for_function(function, target_kind);
	gameshark_run_post_invariants_for_frame(match_key, target_kind, resolved_kind, frame, retval);

	gameshark_remove_invariant_frame(frame_index);
	zend_string_release(match_key);
}

static void gameshark_observer_begin(zend_execute_data *execute_data)
{
	zend_function *function = execute_data->func;
	if (!gameshark_request_active || function == NULL || function->common.function_name == NULL) {
		return;
	}

	gameshark_core_function_meta meta = gameshark_function_meta(function);

	if (gameshark_count_active && function->type == ZEND_USER_FUNCTION) {
		gameshark_core_record_call(&meta);
	}

	if (gameshark_unused_active && function->type == ZEND_USER_FUNCTION) {
		gameshark_unused_record_call(&meta);
	}

	if (gameshark_trace_active) {
		if (gameshark_trace_filter_active) {
			zend_string *canonical_name = gameshark_trace_canonical_function_name(function);
			if (canonical_name == NULL) {
				return;
			}
			bool allowed = gameshark_core_trace_filter_allows(ZSTR_VAL(canonical_name)) != 0;
			zend_string_release(canonical_name);
			if (!allowed) {
				return;
			}
		}
		uint64_t matched_value_mask = gameshark_trace_arguments(execute_data, meta);
		bool transform_frame_started = false;
		if (gameshark_trace_follow_transforms && matched_value_mask != 0 && gameshark_trace_frame_count < GAMESHARK_TRACE_MAX_ACTIVE_FRAMES) {
			gameshark_trace_frames[gameshark_trace_frame_count++] = (gameshark_trace_frame){
				execute_data,
				meta,
				matched_value_mask
			};
			transform_frame_started = true;
		}
		if (gameshark_trace_filter_active) {
			gameshark_core_trace_filter_record_argument_result(matched_value_mask != 0 ? 1 : 0, transform_frame_started ? 1 : 0);
		}
	}
}

static void gameshark_observer_end(zend_execute_data *execute_data, zval *retval)
{
	if (gameshark_request_active && gameshark_trace_active && gameshark_trace_follow_transforms) {
		gameshark_follow_return_transforms(execute_data, retval);
	}

	if (gameshark_invariants_active &&
		!gameshark_invariants_executing_hook &&
		!EG(exception) &&
		gameshark_invariant_user_function_supported(execute_data->func)) {
		gameshark_run_post_invariants(execute_data, retval);
	}
}

static void gameshark_unused_function_declared(zend_op_array *op_array, zend_string *name)
{
	if (!gameshark_unused_active || op_array == NULL || name == NULL || (op_array->fn_flags & ZEND_ACC_CLOSURE)) {
		return;
	}
	gameshark_unused_record_declaration(
		GAMESHARK_UNUSED_DECL_FUNCTION,
		NULL,
		name,
		op_array->filename,
		op_array->line_start,
		op_array->line_end,
		op_array->fn_flags
	);
}

static void gameshark_unused_class_linked(zend_class_entry *ce, zend_string *name)
{
	(void) name;
	if (!gameshark_unused_active || ce == NULL || ce->type != ZEND_USER_CLASS || ce->name == NULL) {
		return;
	}

	if (ce->ce_flags & ZEND_ACC_ANON_CLASS) {
		return;
	}

	zend_string *file = ce->info.user.filename;
	if (gameshark_unused_file_is_invariant(file)) {
		return;
	}

	gameshark_unused_record_declaration(
		GAMESHARK_UNUSED_DECL_CLASS,
		NULL,
		ce->name,
		file,
		ce->info.user.line_start,
		ce->info.user.line_end,
		ce->ce_flags
	);

	zend_function *method;
	ZEND_HASH_MAP_FOREACH_PTR(&ce->function_table, method) {
		if (method == NULL ||
			method->type != ZEND_USER_FUNCTION ||
			method->common.scope != ce ||
			method->common.function_name == NULL) {
			continue;
		}
		gameshark_unused_record_declaration(
			GAMESHARK_UNUSED_DECL_METHOD,
			ce->name,
			method->common.function_name,
			method->op_array.filename,
			method->op_array.line_start,
			method->op_array.line_end,
			method->common.fn_flags
		);
	} ZEND_HASH_FOREACH_END();

	zend_string *constant_name;
	zval *constant_zv;
	ZEND_HASH_MAP_FOREACH_STR_KEY_VAL(&ce->constants_table, constant_name, constant_zv) {
		if (constant_name == NULL || Z_TYPE_P(constant_zv) != IS_PTR) {
			continue;
		}
		zend_class_constant *constant = (zend_class_constant *) Z_PTR_P(constant_zv);
		if (constant == NULL || constant->ce != ce || (ZEND_CLASS_CONST_FLAGS(constant) & ZEND_CLASS_CONST_IS_CASE)) {
			continue;
		}
		gameshark_unused_record_declaration(
			GAMESHARK_UNUSED_DECL_CLASS_CONSTANT,
			ce->name,
			constant_name,
			file,
			ce->info.user.line_start,
			ce->info.user.line_end,
			ZEND_CLASS_CONST_FLAGS(constant)
		);
	} ZEND_HASH_FOREACH_END();
}

static void gameshark_unused_record_global_constants(void)
{
	if (!gameshark_unused_active) {
		return;
	}

	zend_constant *constant;
	ZEND_HASH_MAP_FOREACH_PTR(EG(zend_constants), constant) {
		if (constant == NULL ||
			constant->name == NULL ||
			ZEND_CONSTANT_MODULE_NUMBER(constant) != PHP_USER_CONSTANT) {
			continue;
		}
		zend_string *filename = NULL;
#if PHP_VERSION_ID >= 80600
		filename = constant->filename;
#endif
		gameshark_unused_record_declaration(
			GAMESHARK_UNUSED_DECL_GLOBAL_CONSTANT,
			NULL,
			constant->name,
			filename,
			0,
			0,
			ZEND_CONSTANT_FLAGS(constant)
		);
	} ZEND_HASH_FOREACH_END();
}

static void gameshark_unused_record_constant_name(zend_string *name, uint8_t global_kind, uint8_t class_kind)
{
	if (!gameshark_unused_active || name == NULL || ZSTR_LEN(name) == 0) {
		return;
	}

	const char *separator = gameshark_find_bytes(ZSTR_VAL(name), ZSTR_LEN(name), "::", 2);
	if (separator != NULL && separator > ZSTR_VAL(name) && (separator + 2) < (ZSTR_VAL(name) + ZSTR_LEN(name))) {
		size_t scope_len = (size_t)(separator - ZSTR_VAL(name));
		const char *constant_name = separator + 2;
		size_t constant_name_len = ZSTR_LEN(name) - scope_len - 2;
		gameshark_unused_record_access_mem(class_kind, ZSTR_VAL(name), scope_len, constant_name, constant_name_len);
		return;
	}

	gameshark_unused_record_access(global_kind, NULL, name, NULL, 0, 0);
}

static int gameshark_unused_new_opcode_handler(zend_execute_data *execute_data)
{
	if (!gameshark_unused_active || execute_data == NULL || execute_data->opline == NULL) {
		return ZEND_USER_OPCODE_DISPATCH;
	}

	const zend_op *opline = execute_data->opline;
	if (opline->op1_type == IS_CONST) {
		zval *class_zv = RT_CONSTANT(opline, opline->op1);
		if (Z_TYPE_P(class_zv) == IS_STRING) {
			gameshark_unused_record_access(GAMESHARK_UNUSED_ACCESS_NEW_OPCODE, NULL, Z_STR_P(class_zv), NULL, 0, 0);
		} else {
			gameshark_unused_record_caveat("new opcode target was not a string constant");
		}
	} else {
		gameshark_unused_record_caveat("dynamic or special new opcode target skipped");
	}

	return ZEND_USER_OPCODE_DISPATCH;
}

static int gameshark_unused_constant_opcode_handler(zend_execute_data *execute_data)
{
	if (!gameshark_unused_active || execute_data == NULL || execute_data->opline == NULL) {
		return ZEND_USER_OPCODE_DISPATCH;
	}

	const zend_op *opline = execute_data->opline;
	if (opline->op2_type == IS_CONST) {
		zval *constant_zv = RT_CONSTANT(opline, opline->op2);
		if (Z_TYPE_P(constant_zv) == IS_STRING) {
			gameshark_unused_record_access(
				GAMESHARK_UNUSED_ACCESS_GLOBAL_CONSTANT_FETCH,
				NULL,
				Z_STR_P(constant_zv),
				NULL,
				0,
				0
			);
		} else {
			gameshark_unused_record_caveat("global constant fetch target was not a string constant");
		}
	} else {
		gameshark_unused_record_caveat("dynamic global constant fetch target skipped");
	}

	return ZEND_USER_OPCODE_DISPATCH;
}

static int gameshark_unused_class_constant_opcode_handler(zend_execute_data *execute_data)
{
	if (!gameshark_unused_active || execute_data == NULL || execute_data->opline == NULL) {
		return ZEND_USER_OPCODE_DISPATCH;
	}

	const zend_op *opline = execute_data->opline;
	if (opline->op1_type == IS_CONST && opline->op2_type == IS_CONST) {
		zval *class_zv = RT_CONSTANT(opline, opline->op1);
		zval *constant_zv = RT_CONSTANT(opline, opline->op2);
		if (Z_TYPE_P(class_zv) == IS_STRING && Z_TYPE_P(constant_zv) == IS_STRING) {
			gameshark_unused_record_access(
				GAMESHARK_UNUSED_ACCESS_CLASS_CONSTANT_FETCH,
				Z_STR_P(class_zv),
				Z_STR_P(constant_zv),
				NULL,
				0,
				0
			);
		} else {
			gameshark_unused_record_caveat("class constant fetch target was not fully string constant");
		}
	} else {
		gameshark_unused_record_caveat("dynamic or special class constant fetch target skipped");
	}

	return ZEND_USER_OPCODE_DISPATCH;
}

static int gameshark_unused_defined_opcode_handler(zend_execute_data *execute_data)
{
	if (!gameshark_unused_active || execute_data == NULL || execute_data->opline == NULL) {
		return ZEND_USER_OPCODE_DISPATCH;
	}

	const zend_op *opline = execute_data->opline;
	if (opline->op1_type == IS_CONST) {
		zval *constant_zv = RT_CONSTANT(opline, opline->op1);
		if (Z_TYPE_P(constant_zv) == IS_STRING) {
			gameshark_unused_record_constant_name(
				Z_STR_P(constant_zv),
				GAMESHARK_UNUSED_ACCESS_GLOBAL_CONSTANT_PROBE,
				GAMESHARK_UNUSED_ACCESS_CLASS_CONSTANT_PROBE
			);
		} else {
			gameshark_unused_record_caveat("defined opcode target was not a string constant");
		}
	} else {
		gameshark_unused_record_caveat("dynamic defined opcode target skipped");
	}

	return ZEND_USER_OPCODE_DISPATCH;
}

static zend_observer_fcall_handlers gameshark_observer_fcall_init(zend_execute_data *execute_data)
{
	zend_function *function = execute_data->func;
	if ((!gameshark_request_active && !gameshark_invariants_active) || function == NULL || function->common.function_name == NULL) {
		return (zend_observer_fcall_handlers){NULL, NULL};
	}

	if (gameshark_invariants_executing_hook) {
		gameshark_invariant_reentrancy_suppressed++;
		return (zend_observer_fcall_handlers){NULL, NULL};
	}

	bool wants_invariant_end = false;
	if (gameshark_invariants_active && gameshark_invariant_user_function_supported(function)) {
		uint8_t target_kind;
		zend_string *match_key = gameshark_invariant_function_match_key(function, &target_kind);
		if (match_key != NULL) {
			wants_invariant_end = gameshark_invariant_has_hook(match_key, target_kind, GAMESHARK_INVARIANT_PHASE_POST);
			zend_string_release(match_key);
		}
	}

	if (((gameshark_count_active || gameshark_unused_active) && function->type == ZEND_USER_FUNCTION) || gameshark_trace_active || wants_invariant_end) {
		return (zend_observer_fcall_handlers){
			(((gameshark_count_active || gameshark_unused_active) && function->type == ZEND_USER_FUNCTION) || gameshark_trace_active) ? gameshark_observer_begin : NULL,
			(gameshark_trace_follow_transforms || wants_invariant_end) ? gameshark_observer_end : NULL
		};
	}

	return (zend_observer_fcall_handlers){NULL, NULL};
}

static void gameshark_execute_ex(zend_execute_data *execute_data)
{
	if (gameshark_invariants_active && !gameshark_invariants_executing_hook && gameshark_invariant_user_function_supported(execute_data->func)) {
		uint8_t target_kind;
		zend_string *match_key = gameshark_invariant_function_match_key(execute_data->func, &target_kind);
		if (match_key != NULL) {
			bool has_object = target_kind == GAMESHARK_INVARIANT_TARGET_METHOD && Z_TYPE(execute_data->This) == IS_OBJECT;
			uint8_t resolved_kind = gameshark_invariant_resolved_kind_for_function(execute_data->func, target_kind);
			gameshark_run_pre_invariants(execute_data, match_key, target_kind, resolved_kind, has_object);
			if (EG(exception)) {
				zend_string_release(match_key);
				return;
			}
			if (gameshark_invariant_has_hook(match_key, target_kind, GAMESHARK_INVARIANT_PHASE_POST)) {
				gameshark_push_invariant_frame(execute_data, has_object);
			}
			zend_string_release(match_key);
		}
	}

	if (gameshark_original_execute_ex != NULL) {
		gameshark_original_execute_ex(execute_data);
		return;
	}

	execute_ex(execute_data);
}

static void gameshark_call_original_execute_internal(zend_execute_data *execute_data, zval *return_value)
{
	if (gameshark_original_execute_internal != NULL) {
		gameshark_original_execute_internal(execute_data, return_value);
		return;
	}
	execute_data->func->internal_function.handler(execute_data, return_value);
}

static bool gameshark_unused_internal_function_supported(zend_function *function)
{
	if (!gameshark_unused_active ||
		gameshark_invariants_executing_hook ||
		function == NULL ||
		function->type != ZEND_INTERNAL_FUNCTION ||
		function->common.scope != NULL ||
		function->common.function_name == NULL) {
		return false;
	}

	return zend_string_equals_literal(function->common.function_name, "constant") ||
		zend_string_equals_literal(function->common.function_name, "define") ||
		zend_string_equals_literal(function->common.function_name, "defined");
}

static void gameshark_unused_observe_internal(zend_execute_data *execute_data, zval *return_value)
{
	zend_function *function = execute_data != NULL ? execute_data->func : NULL;
	if (!gameshark_unused_internal_function_supported(function) || EG(exception) || return_value == NULL || Z_TYPE_P(return_value) == IS_UNDEF) {
		return;
	}
	if (ZEND_CALL_NUM_ARGS(execute_data) < 1) {
		return;
	}

	zval *argument = ZEND_CALL_ARG(execute_data, 1);
	if (Z_TYPE_P(argument) != IS_STRING) {
		return;
	}

	if (zend_string_equals_literal(function->common.function_name, "define")) {
		if (Z_TYPE_P(return_value) == IS_TRUE) {
			zend_string *file = zend_get_executed_filename_ex();
			uint32_t line = zend_get_executed_lineno();
			gameshark_unused_record_declaration(
				GAMESHARK_UNUSED_DECL_GLOBAL_CONSTANT,
				NULL,
				Z_STR_P(argument),
				file,
				line,
				line,
				0
			);
		}
		return;
	}

	if (zend_string_equals_literal(function->common.function_name, "defined")) {
		if (Z_TYPE_P(return_value) == IS_TRUE) {
			gameshark_unused_record_constant_name(
				Z_STR_P(argument),
				GAMESHARK_UNUSED_ACCESS_GLOBAL_CONSTANT_PROBE,
				GAMESHARK_UNUSED_ACCESS_CLASS_CONSTANT_PROBE
			);
		}
		return;
	}

	gameshark_unused_record_constant_name(
		Z_STR_P(argument),
		GAMESHARK_UNUSED_ACCESS_GLOBAL_CONSTANT_READ,
		GAMESHARK_UNUSED_ACCESS_CLASS_CONSTANT_READ
	);
}

static void gameshark_execute_internal(zend_execute_data *execute_data, zval *return_value)
{
	zend_function *function = execute_data != NULL ? execute_data->func : NULL;
	if (!gameshark_invariants_active ||
		!gameshark_invariant_has_internal_hooks ||
		!gameshark_invariant_internal_function_supported(function)) {
		gameshark_call_original_execute_internal(execute_data, return_value);
		gameshark_unused_observe_internal(execute_data, return_value);
		return;
	}

	if (gameshark_invariants_executing_hook) {
		gameshark_invariant_reentrancy_suppressed++;
		gameshark_call_original_execute_internal(execute_data, return_value);
		return;
	}

	uint8_t target_kind;
	zend_string *match_key = gameshark_invariant_function_match_key(function, &target_kind);
	if (match_key == NULL) {
		gameshark_call_original_execute_internal(execute_data, return_value);
		gameshark_unused_observe_internal(execute_data, return_value);
		return;
	}

	bool has_pre_hook = gameshark_invariant_has_hook(match_key, target_kind, GAMESHARK_INVARIANT_PHASE_PRE);
	bool has_post_hook = gameshark_invariant_has_hook(match_key, target_kind, GAMESHARK_INVARIANT_PHASE_POST);
	if (!has_pre_hook && !has_post_hook) {
		zend_string_release(match_key);
		gameshark_call_original_execute_internal(execute_data, return_value);
		gameshark_unused_observe_internal(execute_data, return_value);
		return;
	}

	uint8_t resolved_kind = gameshark_invariant_resolved_kind_for_function(function, target_kind);
	if (gameshark_invariant_resolved_kind_is_internal(resolved_kind)) {
		gameshark_emit_builtin_invariant_warning();
	}

	bool has_object = target_kind == GAMESHARK_INVARIANT_TARGET_METHOD && Z_TYPE(execute_data->This) == IS_OBJECT;
	if (has_pre_hook) {
		gameshark_run_pre_invariants(execute_data, match_key, target_kind, resolved_kind, has_object);
		if (EG(exception)) {
			if (return_value != NULL) {
				zval_ptr_dtor(return_value);
				ZVAL_UNDEF(return_value);
			}
			zend_string_release(match_key);
			return;
		}
	}

	gameshark_invariant_frame frame;
	bool has_frame = false;
	if (has_post_hook) {
		gameshark_init_invariant_frame(&frame, execute_data, has_object);
		has_frame = true;
	}

	gameshark_call_original_execute_internal(execute_data, return_value);
	if (EG(exception)) {
		gameshark_invariant_internal_original_exceptions++;
		if (has_frame) {
			gameshark_clear_invariant_frame(&frame);
		}
		zend_string_release(match_key);
		return;
	}

	gameshark_unused_observe_internal(execute_data, return_value);

	if (has_post_hook && return_value != NULL && Z_TYPE_P(return_value) != IS_UNDEF) {
		gameshark_run_post_invariants_for_frame(match_key, target_kind, resolved_kind, &frame, return_value);
	}

	if (has_frame) {
		gameshark_clear_invariant_frame(&frame);
	}
	if (EG(exception) && return_value != NULL && Z_TYPE_P(return_value) != IS_UNDEF) {
		zval_ptr_dtor(return_value);
		ZVAL_UNDEF(return_value);
	}
	zend_string_release(match_key);
}

static zend_op_array *gameshark_compile_file(zend_file_handle *file_handle, int type)
{
	zend_op_array *op_array = NULL;
	if (gameshark_original_compile_file != NULL) {
		op_array = gameshark_original_compile_file(file_handle, type);
	} else {
		op_array = compile_file(file_handle, type);
	}

	if (op_array != NULL && gameshark_unused_active && (file_handle == NULL || !file_handle->primary_script)) {
		if (op_array->filename != NULL) {
			gameshark_unused_record_included_file(op_array->filename);
		} else if (file_handle != NULL && file_handle->opened_path != NULL) {
			gameshark_unused_record_included_file(file_handle->opened_path);
		} else if (file_handle != NULL && file_handle->filename != NULL) {
			gameshark_unused_record_included_file(file_handle->filename);
		}
	}

	return op_array;
}

PHP_MINIT_FUNCTION(gameshark)
{
	if (type != MODULE_TEMPORARY) {
		zend_observer_fcall_register(gameshark_observer_fcall_init);
		zend_observer_function_declared_register(gameshark_unused_function_declared);
		zend_observer_class_linked_register(gameshark_unused_class_linked);
	}
	if (zend_get_user_opcode_handler(ZEND_NEW) == NULL &&
		zend_set_user_opcode_handler(ZEND_NEW, gameshark_unused_new_opcode_handler) == SUCCESS) {
		gameshark_new_opcode_handler_owned = true;
	}
	if (zend_get_user_opcode_handler(ZEND_FETCH_CONSTANT) == NULL &&
		zend_set_user_opcode_handler(ZEND_FETCH_CONSTANT, gameshark_unused_constant_opcode_handler) == SUCCESS) {
		gameshark_constant_opcode_handler_owned = true;
	}
	if (zend_get_user_opcode_handler(ZEND_FETCH_CLASS_CONSTANT) == NULL &&
		zend_set_user_opcode_handler(ZEND_FETCH_CLASS_CONSTANT, gameshark_unused_class_constant_opcode_handler) == SUCCESS) {
		gameshark_class_constant_opcode_handler_owned = true;
	}
	if (zend_get_user_opcode_handler(ZEND_DEFINED) == NULL &&
		zend_set_user_opcode_handler(ZEND_DEFINED, gameshark_unused_defined_opcode_handler) == SUCCESS) {
		gameshark_defined_opcode_handler_owned = true;
	}
	gameshark_original_execute_ex = zend_execute_ex;
	zend_execute_ex = gameshark_execute_ex;
	gameshark_original_execute_internal = zend_execute_internal;
	gameshark_execute_internal_previous_present = gameshark_original_execute_internal != NULL;
	zend_execute_internal = gameshark_execute_internal;
	gameshark_original_compile_file = zend_compile_file;
	zend_compile_file = gameshark_compile_file;
	REGISTER_INI_ENTRIES();
	return SUCCESS;
}

PHP_MSHUTDOWN_FUNCTION(gameshark)
{
	if (zend_compile_file == gameshark_compile_file && gameshark_original_compile_file != NULL) {
		zend_compile_file = gameshark_original_compile_file;
	}
	gameshark_original_compile_file = NULL;
	if (gameshark_new_opcode_handler_owned && zend_get_user_opcode_handler(ZEND_NEW) == gameshark_unused_new_opcode_handler) {
		zend_set_user_opcode_handler(ZEND_NEW, NULL);
	}
	gameshark_new_opcode_handler_owned = false;
	if (gameshark_constant_opcode_handler_owned && zend_get_user_opcode_handler(ZEND_FETCH_CONSTANT) == gameshark_unused_constant_opcode_handler) {
		zend_set_user_opcode_handler(ZEND_FETCH_CONSTANT, NULL);
	}
	gameshark_constant_opcode_handler_owned = false;
	if (gameshark_class_constant_opcode_handler_owned && zend_get_user_opcode_handler(ZEND_FETCH_CLASS_CONSTANT) == gameshark_unused_class_constant_opcode_handler) {
		zend_set_user_opcode_handler(ZEND_FETCH_CLASS_CONSTANT, NULL);
	}
	gameshark_class_constant_opcode_handler_owned = false;
	if (gameshark_defined_opcode_handler_owned && zend_get_user_opcode_handler(ZEND_DEFINED) == gameshark_unused_defined_opcode_handler) {
		zend_set_user_opcode_handler(ZEND_DEFINED, NULL);
	}
	gameshark_defined_opcode_handler_owned = false;
	if (zend_execute_ex == gameshark_execute_ex && gameshark_original_execute_ex != NULL) {
		zend_execute_ex = gameshark_original_execute_ex;
	}
	gameshark_original_execute_ex = NULL;
	if (zend_execute_internal == gameshark_execute_internal) {
		zend_execute_internal = gameshark_original_execute_internal;
	}
	gameshark_original_execute_internal = NULL;
	gameshark_execute_internal_previous_present = false;
	UNREGISTER_INI_ENTRIES();
	return SUCCESS;
}

PHP_RINIT_FUNCTION(gameshark)
{
#if defined(ZTS) && defined(COMPILE_DL_GAMESHARK)
	ZEND_TSRMLS_CACHE_UPDATE();
#endif

	const char *db_path = getenv("GAMESHARK_DB");
	const char *side = getenv("GAMESHARK_SIDE");
	const char *trace_value = getenv("GAMESHARK_TRACE_VALUE");
	const char *follow_transforms = getenv("GAMESHARK_TRACE_FOLLOW_TRANSFORMS");
	const char *trace_allow_pattern_ini = zend_ini_string_literal("gameshark.trace_allow_pattern");
	const char *trace_allow_pattern_env = getenv("GAMESHARK_TRACE_ALLOW_PATTERN");
	const char *invariants_ini = zend_ini_string_literal("gameshark.invariants");
	const char *invariants_file_ini = zend_ini_string_literal("gameshark.invariants_file");
	const char *invariants_warn_builtins_ini = zend_ini_string_literal("gameshark.invariants_warn_builtins");
	const char *invariants_env = getenv("GAMESHARK_INVARIANTS");
	const char *invariants_file_env = getenv("GAMESHARK_INVARIANTS_FILE");
	const char *invariants_warn_builtins_env = getenv("GAMESHARK_INVARIANTS_WARN_BUILTINS");
	const char *unused_ini = zend_ini_string_literal("gameshark.unused");
	const char *unused_capture_query_ini = zend_ini_string_literal("gameshark.unused_capture_query");
	const char *unused_env = getenv("GAMESHARK_UNUSED");
	const char *unused_capture_query_env = getenv("GAMESHARK_UNUSED_CAPTURE_QUERY");
	bool valid_side = side != NULL && (strcmp(side, "left") == 0 || strcmp(side, "right") == 0);
	bool wants_trace = trace_value != NULL && trace_value[0] != '\0';
	const char *trace_allow_pattern = gameshark_config_has_value(trace_allow_pattern_ini)
		? trace_allow_pattern_ini
		: trace_allow_pattern_env;
	bool wants_invariants = invariants_ini != NULL && invariants_ini[0] != '\0'
		? gameshark_config_truthy(invariants_ini)
		: gameshark_config_truthy(invariants_env);
	bool wants_unused = unused_ini != NULL && unused_ini[0] != '\0'
		? gameshark_config_truthy(unused_ini)
		: gameshark_config_truthy(unused_env);
	bool unused_capture_query = gameshark_config_has_value(unused_capture_query_env)
		? gameshark_config_truthy(unused_capture_query_env)
		: gameshark_config_truthy(unused_capture_query_ini);
	const char *invariants_file = invariants_file_ini != NULL && invariants_file_ini[0] != '\0'
		? invariants_file_ini
		: invariants_file_env;

	gameshark_request_active = false;
	gameshark_count_active = false;
	gameshark_trace_active = false;
	gameshark_unused_active = false;
	gameshark_request_db_path = NULL;
	gameshark_request_side = NULL;
	gameshark_reset_trace_config();
	gameshark_reset_invariant_config();

	if (wants_invariants) {
		gameshark_invariants_enabled = true;
		gameshark_invariant_warn_builtins = invariants_warn_builtins_ini != NULL && invariants_warn_builtins_ini[0] != '\0'
			? gameshark_config_truthy(invariants_warn_builtins_ini)
			: (invariants_warn_builtins_env == NULL || invariants_warn_builtins_env[0] == '\0' || gameshark_config_truthy(invariants_warn_builtins_env));
		if (invariants_file != NULL && invariants_file[0] != '\0') {
			gameshark_invariants_file = estrdup(invariants_file);
		}
		if (gameshark_load_invariants_file(invariants_file)) {
			gameshark_refresh_invariant_resolution();
			gameshark_emit_builtin_invariant_warning();
		}
	}

	if (db_path == NULL || db_path[0] == '\0' || (!valid_side && !wants_trace && !wants_unused)) {
		return SUCCESS;
	}

	if (wants_trace && !gameshark_configure_trace_value(trace_value)) {
		return SUCCESS;
	}
	gameshark_trace_follow_transforms = wants_trace && follow_transforms != NULL && follow_transforms[0] != '\0' && strcmp(follow_transforms, "0") != 0;

	const char *script_filename = SG(request_info).path_translated;
	if (script_filename == NULL) {
		script_filename = SG(request_info).request_uri;
	}

	const char *request_uri = SG(request_info).request_uri;
	const char *query_string = SG(request_info).query_string;
	char *request_path = NULL;
	char *request_uri_full = NULL;
	if (request_uri != NULL && request_uri[0] != '\0') {
		const char *query_start = strchr(request_uri, '?');
		if (query_start != NULL) {
			request_path = estrndup(request_uri, (size_t)(query_start - request_uri));
		} else {
			request_path = estrdup(request_uri);
		}
		if (unused_capture_query) {
			if (query_start != NULL || query_string == NULL || query_string[0] == '\0') {
				request_uri_full = estrdup(request_uri);
			} else {
				size_t request_uri_len = strlen(request_uri);
				size_t query_string_len = strlen(query_string);
				request_uri_full = emalloc(request_uri_len + query_string_len + 2);
				memcpy(request_uri_full, request_uri, request_uri_len);
				request_uri_full[request_uri_len] = '?';
				memcpy(request_uri_full + request_uri_len + 1, query_string, query_string_len);
				request_uri_full[request_uri_len + query_string_len + 1] = '\0';
			}
		}
	}

	if (gameshark_core_request_start(
		db_path,
		valid_side ? side : NULL,
		wants_trace ? trace_value : NULL,
		wants_trace && gameshark_config_has_value(trace_allow_pattern) ? trace_allow_pattern : NULL,
			PHP_VERSION,
			sapi_module.name,
			(uint32_t)getpid(),
			script_filename,
			wants_unused ? 1 : 0,
			request_path,
			request_uri_full,
			unused_capture_query ? query_string : NULL,
			gameshark_new_opcode_handler_owned ? 1 : 0,
			gameshark_constant_opcode_handler_owned ? 1 : 0,
			gameshark_class_constant_opcode_handler_owned ? 1 : 0
	)) {
		gameshark_request_active = true;
		gameshark_count_active = valid_side;
		gameshark_trace_active = wants_trace;
		gameshark_unused_active = wants_unused;
		if (gameshark_unused_active && !gameshark_defined_opcode_handler_owned) {
			gameshark_unused_record_caveat("ZEND_DEFINED opcode handler unavailable because another extension already registered one");
		}
		gameshark_trace_filter_active = wants_trace && gameshark_config_has_value(trace_allow_pattern);
		if (gameshark_trace_filter_active) {
			char *trace_filter_error = gameshark_core_trace_filter_error();
			if (trace_filter_error != NULL) {
				zend_string *message = zend_strpprintf(0, "Gameshark trace allow pattern error: %s", trace_filter_error);
				if (message != NULL) {
					if (strcmp(sapi_module.name, "cli") == 0) {
						fprintf(stderr, "%s\n", ZSTR_VAL(message));
					} else {
						php_log_err(ZSTR_VAL(message));
					}
					zend_string_release(message);
				}
				gameshark_core_string_free(trace_filter_error);
			}
		}
		gameshark_request_db_path = estrdup(db_path);
		if (valid_side) {
			gameshark_request_side = estrdup(side);
		}
	}

	if (request_path != NULL) {
		efree(request_path);
	}
	if (request_uri_full != NULL) {
		efree(request_uri_full);
	}

	return SUCCESS;
}

PHP_RSHUTDOWN_FUNCTION(gameshark)
{
	if (gameshark_request_active) {
		gameshark_unused_record_global_constants();
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
	gameshark_reset_trace_config();
	gameshark_reset_invariant_config();
	gameshark_request_active = false;
	gameshark_count_active = false;
	gameshark_trace_active = false;
	gameshark_unused_active = false;

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

PHP_FUNCTION(gameshark_invariants_status)
{
	ZEND_PARSE_PARAMETERS_NONE();

	array_init(return_value);
	add_assoc_bool(return_value, "enabled", gameshark_invariants_enabled);
	add_assoc_bool(return_value, "loaded", gameshark_invariants_loaded);
	if (gameshark_invariants_file != NULL) {
		add_assoc_string(return_value, "file", gameshark_invariants_file);
	} else {
		add_assoc_null(return_value, "file");
	}
	if (gameshark_invariants_load_error != NULL) {
		add_assoc_str(return_value, "load_error", zend_string_copy(gameshark_invariants_load_error));
	} else {
		add_assoc_null(return_value, "load_error");
	}
	add_assoc_long(return_value, "spec_count", (zend_long) gameshark_invariant_spec_count);
	add_assoc_long(return_value, "reentrancy_suppressed", (zend_long) gameshark_invariant_reentrancy_suppressed);
	add_assoc_bool(return_value, "internal_interceptor_active", gameshark_invariant_has_internal_hooks);
	add_assoc_bool(return_value, "execute_internal_wrapped", zend_execute_internal == gameshark_execute_internal);
	add_assoc_bool(return_value, "execute_internal_previous_present", gameshark_execute_internal_previous_present);
	add_assoc_bool(return_value, "internal_warning_active", gameshark_invariant_has_internal_hooks);
	add_assoc_bool(return_value, "internal_warning_emitted", gameshark_invariant_internal_warning_emitted);
	add_assoc_bool(return_value, "internal_warning_enabled", gameshark_invariant_warn_builtins);
	add_assoc_long(return_value, "internal_pre_invocations", (zend_long) gameshark_invariant_internal_pre_invocations);
	add_assoc_long(return_value, "internal_post_invocations", (zend_long) gameshark_invariant_internal_post_invocations);
	add_assoc_long(return_value, "internal_original_exceptions", (zend_long) gameshark_invariant_internal_original_exceptions);
	add_assoc_long(return_value, "internal_hook_exceptions", (zend_long) gameshark_invariant_internal_hook_exceptions);

	zval specs;
	array_init(&specs);
	zend_ulong matched_count = 0;
	zend_ulong unmatched_count = 0;
	zend_ulong internal_hook_count = 0;
	zend_ulong internal_matched_count = 0;
	for (size_t i = 0; i < gameshark_invariant_spec_count; i++) {
		gameshark_invariant_spec *spec = &gameshark_invariant_specs[i];
		zval spec_status;
		array_init(&spec_status);
		add_assoc_str(&spec_status, "id", zend_string_copy(spec->id));
		add_assoc_str(&spec_status, "target", zend_string_copy(spec->target));
		add_assoc_string(&spec_status, "target_kind", spec->target_kind == GAMESHARK_INVARIANT_TARGET_METHOD ? "method" : "function");
		add_assoc_string(&spec_status, "when", spec->phase == GAMESHARK_INVARIANT_PHASE_PRE ? "pre" : "post");
		add_assoc_string(&spec_status, "resolved_kind", gameshark_invariant_resolved_kind_name(spec->resolved_kind));
		add_assoc_bool(&spec_status, "matched", spec->matched);
		add_assoc_long(&spec_status, "executions", (zend_long) spec->executions);
		add_assoc_long(&spec_status, "hook_exceptions", (zend_long) spec->hook_exceptions);
		add_next_index_zval(&specs, &spec_status);
		bool internal_spec = gameshark_invariant_resolved_kind_is_internal(spec->resolved_kind);
		if (internal_spec) {
			internal_hook_count++;
		}
		if (spec->matched) {
			matched_count++;
			if (internal_spec) {
				internal_matched_count++;
			}
		} else {
			unmatched_count++;
		}
	}
	add_assoc_long(return_value, "matched_count", (zend_long) matched_count);
	add_assoc_long(return_value, "unmatched_count", (zend_long) unmatched_count);
	add_assoc_long(return_value, "internal_hook_count", (zend_long) internal_hook_count);
	add_assoc_long(return_value, "internal_matched_count", (zend_long) internal_matched_count);
	add_assoc_zval(return_value, "specs", &specs);
}

static void gameshark_decode_json_report(INTERNAL_FUNCTION_PARAMETERS, char *json)
{
	if (json == NULL) {
		array_init(return_value);
		add_assoc_string(return_value, "error", "gameshark report failed");
		return;
	}

	if (php_json_decode(return_value, json, strlen(json), true, 512) != SUCCESS) {
		gameshark_core_string_free(json);
		array_init(return_value);
		add_assoc_string(return_value, "error", "gameshark report JSON decode failed");
		return;
	}

	gameshark_core_string_free(json);
}

static int gameshark_report_format(char *format, size_t format_len)
{
	if (format == NULL || (format_len == sizeof("text") - 1 && memcmp(format, "text", sizeof("text") - 1) == 0)) {
		return GAMESHARK_REPORT_TEXT;
	}
	if (format_len == sizeof("array") - 1 && memcmp(format, "array", sizeof("array") - 1) == 0) {
		return GAMESHARK_REPORT_ARRAY;
	}
	if (format_len == sizeof("json") - 1 && memcmp(format, "json", sizeof("json") - 1) == 0) {
		return GAMESHARK_REPORT_JSON;
	}

	zend_value_error("gameshark report format must be \"text\", \"array\", or \"json\"");
	return -1;
}

static bool gameshark_report_color_enabled(void)
{
	const char *color = getenv("GAMESHARK_COLOR");
	if (color != NULL) {
		if (strcmp(color, "always") == 0) {
			return true;
		}
		if (strcmp(color, "never") == 0) {
			return false;
		}
	}

	if (getenv("NO_COLOR") != NULL && color == NULL) {
		return false;
	}

	return isatty(STDOUT_FILENO);
}

static void gameshark_return_json_report(INTERNAL_FUNCTION_PARAMETERS, char *json)
{
	if (json == NULL) {
		RETURN_STRING("{\"error\":\"gameshark report failed\"}");
	}

	RETVAL_STRING(json);
	gameshark_core_string_free(json);
}

static void gameshark_return_text_report(INTERNAL_FUNCTION_PARAMETERS, char *text)
{
	if (text == NULL) {
		RETURN_STRING("Gameshark report failed\n");
	}

	RETVAL_STRING(text);
	gameshark_core_string_free(text);
}

PHP_FUNCTION(gameshark_compare)
{
	char *format = NULL;
	size_t format_len = 0;

	ZEND_PARSE_PARAMETERS_START(0, 1)
		Z_PARAM_OPTIONAL
		Z_PARAM_STRING(format, format_len)
	ZEND_PARSE_PARAMETERS_END();

	int report_format = gameshark_report_format(format, format_len);
	if (report_format < 0) {
		RETURN_THROWS();
	}

	const char *db_path = gameshark_request_db_path;
	if (db_path == NULL || db_path[0] == '\0') {
		db_path = getenv("GAMESHARK_DB");
	}

	if (report_format == GAMESHARK_REPORT_TEXT) {
		gameshark_return_text_report(INTERNAL_FUNCTION_PARAM_PASSTHRU, gameshark_core_compare_text(db_path, gameshark_report_color_enabled() ? 1 : 0));
		return;
	}

	char *json = gameshark_core_compare_json(db_path);
	if (report_format == GAMESHARK_REPORT_JSON) {
		gameshark_return_json_report(INTERNAL_FUNCTION_PARAM_PASSTHRU, json);
		return;
	}
	gameshark_decode_json_report(INTERNAL_FUNCTION_PARAM_PASSTHRU, json);
}

PHP_FUNCTION(gameshark_trace_report)
{
	char *format = NULL;
	size_t format_len = 0;

	ZEND_PARSE_PARAMETERS_START(0, 1)
		Z_PARAM_OPTIONAL
		Z_PARAM_STRING(format, format_len)
	ZEND_PARSE_PARAMETERS_END();

	int report_format = gameshark_report_format(format, format_len);
	if (report_format < 0) {
		RETURN_THROWS();
	}

	const char *db_path = gameshark_request_db_path;
	if (db_path == NULL || db_path[0] == '\0') {
		db_path = getenv("GAMESHARK_DB");
	}

	if (report_format == GAMESHARK_REPORT_TEXT) {
		gameshark_return_text_report(INTERNAL_FUNCTION_PARAM_PASSTHRU, gameshark_core_trace_report_text(db_path, gameshark_report_color_enabled() ? 1 : 0));
		return;
	}

	char *json = gameshark_core_trace_report_json(db_path);
	if (report_format == GAMESHARK_REPORT_JSON) {
		gameshark_return_json_report(INTERNAL_FUNCTION_PARAM_PASSTHRU, json);
		return;
	}
	gameshark_decode_json_report(INTERNAL_FUNCTION_PARAM_PASSTHRU, json);
}

PHP_FUNCTION(gameshark_unused_report)
{
	char *format = NULL;
	size_t format_len = 0;
	zend_long run_id = -1;
	bool run_id_is_null = true;

	ZEND_PARSE_PARAMETERS_START(0, 2)
		Z_PARAM_OPTIONAL
		Z_PARAM_STRING(format, format_len)
		Z_PARAM_LONG_OR_NULL(run_id, run_id_is_null)
	ZEND_PARSE_PARAMETERS_END();

	int report_format = gameshark_report_format(format, format_len);
	if (report_format < 0) {
		RETURN_THROWS();
	}

	const char *db_path = gameshark_request_db_path;
	if (db_path == NULL || db_path[0] == '\0') {
		db_path = getenv("GAMESHARK_DB");
	}

	int64_t selected_run_id = run_id_is_null ? -1 : (int64_t)run_id;
	if (report_format == GAMESHARK_REPORT_TEXT) {
		gameshark_return_text_report(INTERNAL_FUNCTION_PARAM_PASSTHRU, gameshark_core_unused_report_text(db_path, gameshark_report_color_enabled() ? 1 : 0, selected_run_id));
		return;
	}

	char *json = gameshark_core_unused_report_json(db_path, selected_run_id);
	if (report_format == GAMESHARK_REPORT_JSON) {
		gameshark_return_json_report(INTERNAL_FUNCTION_PARAM_PASSTHRU, json);
		return;
	}
	gameshark_decode_json_report(INTERNAL_FUNCTION_PARAM_PASSTHRU, json);
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
	PHP_MSHUTDOWN(gameshark),
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
