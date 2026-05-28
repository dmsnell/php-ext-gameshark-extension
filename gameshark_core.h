#ifndef GAMESHARK_CORE_H
#define GAMESHARK_CORE_H

#include <stddef.h>
#include <stdint.h>

typedef struct {
	const char *ptr;
	size_t len;
} gameshark_core_str;

typedef struct {
	uint8_t kind;
	gameshark_core_str scope_name;
	gameshark_core_str function_name;
	gameshark_core_str file;
	uint32_t start_line;
	uint32_t end_line;
} gameshark_core_function_meta;

typedef struct {
	gameshark_core_function_meta function;
	gameshark_core_str argument_path;
	gameshark_core_str zval_type;
	uint32_t matched_value_id;
	uint8_t match_kind;
	gameshark_core_str matched_value;
	gameshark_core_str preview;
	gameshark_core_str observed_value;
	gameshark_core_str stack;
	gameshark_core_str stack_json;
} gameshark_core_trace_event;

typedef struct {
	uint32_t value_id;
	uint32_t parent_value_id;
	gameshark_core_function_meta function;
	gameshark_core_str transform_kind;
	gameshark_core_str value;
	gameshark_core_str preview;
} gameshark_core_transformed_value;

typedef struct {
	uint8_t kind;
	gameshark_core_str scope_name;
	gameshark_core_str name;
	gameshark_core_str file;
	uint32_t start_line;
	uint32_t end_line;
	uint32_t flags;
} gameshark_core_unused_declaration;

typedef struct {
	uint8_t kind;
	gameshark_core_str scope_name;
	gameshark_core_str name;
	gameshark_core_str file;
	uint32_t start_line;
	uint32_t end_line;
} gameshark_core_unused_access;

int gameshark_core_request_start(
	const char *db_path,
	const char *side,
	const char *trace_value,
	const char *trace_allow_pattern,
	const char *php_version,
	const char *sapi_name,
	uint32_t pid,
	const char *script_filename,
	int unused_enabled,
	const char *request_path,
	const char *request_uri_full,
	const char *query_string,
	int new_opcode_handler_active,
	int constant_opcode_handler_active,
	int class_constant_opcode_handler_active
);
void gameshark_core_record_call(const gameshark_core_function_meta *meta);
int gameshark_core_trace_filter_allows(const char *canonical_name);
void gameshark_core_trace_filter_record_argument_result(int matched, int transform_frame_started);
char *gameshark_core_trace_filter_error(void);
void gameshark_core_record_trace_event(const gameshark_core_trace_event *event);
void gameshark_core_record_transformed_value(const gameshark_core_transformed_value *value);
void gameshark_core_record_unused_declaration(const gameshark_core_unused_declaration *declaration);
void gameshark_core_record_unused_access(const gameshark_core_unused_access *access);
void gameshark_core_record_unused_included_file(const char *file);
void gameshark_core_record_unused_caveat(const char *caveat);
void gameshark_core_request_finish(void);
char *gameshark_core_compare_json(const char *db_path);
char *gameshark_core_compare_text(const char *db_path, int color);
char *gameshark_core_trace_report_json(const char *db_path);
char *gameshark_core_trace_report_text(const char *db_path, int color);
char *gameshark_core_unused_report_json(const char *db_path, int64_t run_id);
char *gameshark_core_unused_report_text(const char *db_path, int color, int64_t run_id);
void gameshark_core_string_free(char *ptr);

#endif
