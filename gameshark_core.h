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

int gameshark_core_request_start(
	const char *db_path,
	const char *side,
	const char *php_version,
	const char *sapi_name,
	uint32_t pid,
	const char *script_filename
);
void gameshark_core_record_call(const gameshark_core_function_meta *meta);
void gameshark_core_request_finish(void);
char *gameshark_core_compare_json(const char *db_path);
void gameshark_core_string_free(char *ptr);

#endif
