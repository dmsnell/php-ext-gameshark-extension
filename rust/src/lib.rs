#[cfg(feature = "backend-mysql")]
use mysql::prelude::*;
#[cfg(feature = "backend-redis")]
use redis::Commands;
use regex::Regex;
use rusqlite::{params, Connection, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, CString};
use std::fmt::Write as _;
use std::os::raw::{c_char, c_int};
use std::slice;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[repr(C)]
pub struct GamesharkCoreStr {
    ptr: *const c_char,
    len: usize,
}

#[repr(C)]
pub struct GamesharkCoreFunctionMeta {
    kind: u8,
    scope_name: GamesharkCoreStr,
    function_name: GamesharkCoreStr,
    file: GamesharkCoreStr,
    start_line: u32,
    end_line: u32,
}

#[repr(C)]
pub struct GamesharkCoreTraceEvent {
    function: GamesharkCoreFunctionMeta,
    argument_path: GamesharkCoreStr,
    zval_type: GamesharkCoreStr,
    matched_value_id: u32,
    match_kind: u8,
    matched_value: GamesharkCoreStr,
    preview: GamesharkCoreStr,
    observed_value: GamesharkCoreStr,
    stack: GamesharkCoreStr,
    stack_json: GamesharkCoreStr,
}

#[repr(C)]
pub struct GamesharkCoreTransformedValue {
    value_id: u32,
    parent_value_id: u32,
    function: GamesharkCoreFunctionMeta,
    transform_kind: GamesharkCoreStr,
    value: GamesharkCoreStr,
    preview: GamesharkCoreStr,
}

#[repr(C)]
pub struct GamesharkCoreUnusedDeclaration {
    kind: u8,
    scope_name: GamesharkCoreStr,
    name: GamesharkCoreStr,
    file: GamesharkCoreStr,
    start_line: u32,
    end_line: u32,
    flags: u32,
}

#[repr(C)]
pub struct GamesharkCoreUnusedAccess {
    kind: u8,
    scope_name: GamesharkCoreStr,
    name: GamesharkCoreStr,
    file: GamesharkCoreStr,
    start_line: u32,
    end_line: u32,
}

#[repr(C)]
pub struct GamesharkCoreStorageConfig {
    storage_ini: *const c_char,
    storage_env: *const c_char,
    dsn_ini: *const c_char,
    dsn_env: *const c_char,
    legacy_db_ini: *const c_char,
    legacy_db_env: *const c_char,
    capture_ini: *const c_char,
    capture_env: *const c_char,
    mysql_host_ini: *const c_char,
    mysql_host_env: *const c_char,
    mysql_port_ini: *const c_char,
    mysql_port_env: *const c_char,
    mysql_database_ini: *const c_char,
    mysql_database_env: *const c_char,
    mysql_username_ini: *const c_char,
    mysql_username_env: *const c_char,
    mysql_password_ini: *const c_char,
    mysql_password_env: *const c_char,
    mysql_password_file_ini: *const c_char,
    mysql_password_file_env: *const c_char,
    mysql_socket_ini: *const c_char,
    mysql_socket_env: *const c_char,
    mysql_ssl_mode_ini: *const c_char,
    mysql_ssl_mode_env: *const c_char,
    mysql_schema_mode_ini: *const c_char,
    mysql_schema_mode_env: *const c_char,
    mysql_connect_timeout_ms_ini: *const c_char,
    mysql_connect_timeout_ms_env: *const c_char,
    mysql_operation_timeout_ms_ini: *const c_char,
    mysql_operation_timeout_ms_env: *const c_char,
    mysql_report_timeout_ms_ini: *const c_char,
    mysql_report_timeout_ms_env: *const c_char,
    redis_host_ini: *const c_char,
    redis_host_env: *const c_char,
    redis_port_ini: *const c_char,
    redis_port_env: *const c_char,
    redis_database_ini: *const c_char,
    redis_database_env: *const c_char,
    redis_username_ini: *const c_char,
    redis_username_env: *const c_char,
    redis_password_ini: *const c_char,
    redis_password_env: *const c_char,
    redis_password_file_ini: *const c_char,
    redis_password_file_env: *const c_char,
    redis_key_prefix_ini: *const c_char,
    redis_key_prefix_env: *const c_char,
    redis_ttl_ini: *const c_char,
    redis_ttl_env: *const c_char,
    redis_connect_timeout_ms_ini: *const c_char,
    redis_connect_timeout_ms_env: *const c_char,
    redis_operation_timeout_ms_ini: *const c_char,
    redis_operation_timeout_ms_env: *const c_char,
    redis_report_timeout_ms_ini: *const c_char,
    redis_report_timeout_ms_env: *const c_char,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
struct FunctionKey {
    kind: FunctionKind,
    scope_name: Option<String>,
    function_name: String,
    file: Option<String>,
    start_line: u32,
    end_line: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum FunctionKind {
    Function,
    Method,
    Closure,
    InternalFunction,
    InternalMethod,
}

impl FunctionKind {
    fn from_u8(value: u8) -> Self {
        match value {
            2 => Self::Method,
            3 => Self::Closure,
            4 => Self::InternalFunction,
            5 => Self::InternalMethod,
            _ => Self::Function,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Method => "method",
            Self::Closure => "closure",
            Self::InternalFunction => "internal_function",
            Self::InternalMethod => "internal_method",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum UnusedSymbolKind {
    Function,
    Method,
    Closure,
    Class,
    GlobalConstant,
    ClassConstant,
}

impl UnusedSymbolKind {
    fn declaration_from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Function),
            2 => Some(Self::Method),
            3 => Some(Self::Class),
            4 => Some(Self::GlobalConstant),
            5 => Some(Self::ClassConstant),
            _ => None,
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Method => "method",
            Self::Closure => "closure",
            Self::Class => "class",
            Self::GlobalConstant => "global_constant",
            Self::ClassConstant => "class_constant",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum UnusedAccessKind {
    FunctionCall,
    MethodCall,
    ClosureCall,
    NewOpcodeObserved,
    GlobalConstantFetchObserved,
    ClassConstantFetchObserved,
    GlobalConstantRead,
    ClassConstantRead,
    GlobalConstantProbe,
    ClassConstantProbe,
}

impl UnusedAccessKind {
    fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::FunctionCall),
            2 => Some(Self::MethodCall),
            3 => Some(Self::ClosureCall),
            4 => Some(Self::NewOpcodeObserved),
            5 => Some(Self::GlobalConstantFetchObserved),
            6 => Some(Self::ClassConstantFetchObserved),
            7 => Some(Self::GlobalConstantRead),
            8 => Some(Self::ClassConstantRead),
            9 => Some(Self::GlobalConstantProbe),
            10 => Some(Self::ClassConstantProbe),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::FunctionCall => "function_call",
            Self::MethodCall => "method_call",
            Self::ClosureCall => "closure_call",
            Self::NewOpcodeObserved => "new_opcode_observed",
            Self::GlobalConstantFetchObserved => "global_constant_fetch_observed",
            Self::ClassConstantFetchObserved => "class_constant_fetch_observed",
            Self::GlobalConstantRead => "global_constant_read",
            Self::ClassConstantRead => "class_constant_read",
            Self::GlobalConstantProbe => "global_constant_probe",
            Self::ClassConstantProbe => "class_constant_probe",
        }
    }

    fn symbol_kind(self) -> UnusedSymbolKind {
        match self {
            Self::FunctionCall => UnusedSymbolKind::Function,
            Self::MethodCall => UnusedSymbolKind::Method,
            Self::ClosureCall => UnusedSymbolKind::Closure,
            Self::NewOpcodeObserved => UnusedSymbolKind::Class,
            Self::GlobalConstantFetchObserved
            | Self::GlobalConstantRead
            | Self::GlobalConstantProbe => UnusedSymbolKind::GlobalConstant,
            Self::ClassConstantFetchObserved
            | Self::ClassConstantRead
            | Self::ClassConstantProbe => UnusedSymbolKind::ClassConstant,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
struct UnusedSymbolKey {
    kind: UnusedSymbolKind,
    scope_name: Option<String>,
    name: String,
}

#[derive(Clone, Deserialize, Serialize)]
struct UnusedDeclaration {
    key: UnusedSymbolKey,
    display_name: String,
    scope_name: Option<String>,
    name: String,
    file: Option<String>,
    start_line: u32,
    end_line: u32,
    flags: u32,
}

#[derive(Clone, Deserialize, Serialize)]
struct UnusedAccess {
    key: UnusedSymbolKey,
    access_kind: UnusedAccessKind,
    display_name: String,
    scope_name: Option<String>,
    name: String,
    file: Option<String>,
    start_line: u32,
    end_line: u32,
    count: u64,
}

#[derive(Clone, Deserialize, Serialize)]
struct UnusedIncludedFile {
    file: String,
    include_count: u64,
}

#[derive(Clone, Debug)]
enum StorageTarget {
    Sqlite {
        path: String,
    },
    Mysql {
        dsn: String,
        display: String,
        host: Option<String>,
        port: Option<u16>,
        database: Option<String>,
        socket: Option<String>,
        schema_mode: SchemaMode,
        connect_timeout_ms: u64,
        operation_timeout_ms: u64,
        report_timeout_ms: u64,
    },
    Redis {
        dsn: String,
        display: String,
        host: Option<String>,
        port: Option<u16>,
        database: Option<i64>,
        key_prefix: String,
        ttl: u64,
        connect_timeout_ms: u64,
        operation_timeout_ms: u64,
        report_timeout_ms: u64,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SchemaMode {
    Auto,
    Validate,
}

impl SchemaMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Validate => "validate",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct StorageError {
    code: String,
    backend: Option<String>,
    message: String,
    hint: Option<String>,
}

#[derive(Clone, Debug)]
struct ParsedStorageConfig {
    target: Option<StorageTarget>,
    capture: String,
    sources: StorageSources,
    ignored_legacy_sqlite_path: Option<String>,
    error: Option<StorageError>,
}

#[derive(Clone, Debug)]
struct StorageSources {
    storage: String,
    dsn: String,
    legacy_db: String,
    capture: String,
    credentials: String,
    schema_mode: String,
    timeouts: String,
}

#[derive(Clone, Deserialize, Serialize)]
struct DiffPayload {
    run_id: i64,
    capture: String,
    side: String,
    started_at: i64,
    finished_at: i64,
    php_version: String,
    sapi: String,
    pid: u32,
    script_filename: Option<String>,
    functions: Vec<StoredFunctionCount>,
}

#[derive(Clone, Deserialize, Serialize)]
struct StoredFunctionCount {
    function: FunctionKey,
    call_count: u64,
}

#[derive(Clone, Deserialize, Serialize)]
struct UnusedSnapshot {
    run: UnusedRunReport,
    declarations: Vec<UnusedDeclaration>,
    accesses: Vec<UnusedAccess>,
    included_files: Vec<UnusedIncludedFile>,
}

struct State {
    storage: StorageTarget,
    capture: String,
    side: Option<String>,
    started_at: i64,
    started_monotonic: Instant,
    last_elapsed_ns: u64,
    trace_run_id: Option<i64>,
    trace_value: Option<String>,
    trace_value_kind: Option<String>,
    php_version: String,
    sapi_name: String,
    pid: u32,
    script_filename: Option<String>,
    request_path: Option<String>,
    request_uri_full: Option<String>,
    query_string: Option<String>,
    new_opcode_handler_active: bool,
    constant_opcode_handler_active: bool,
    class_constant_opcode_handler_active: bool,
    trace_filter: TraceFilter,
    counters: HashMap<FunctionKey, u64>,
    trace_events: Vec<TraceEvent>,
    transformed_values: Vec<TransformedValue>,
    unused_run_id: Option<i64>,
    unused_declarations: HashMap<UnusedSymbolKey, UnusedDeclaration>,
    unused_accesses: HashMap<(UnusedSymbolKey, UnusedAccessKind), UnusedAccess>,
    unused_included_files: HashMap<String, u64>,
    unused_caveats: HashSet<String>,
}

struct TraceFilter {
    mode: String,
    allow_pattern: Option<String>,
    allow_pattern_hash: Option<String>,
    allow_pattern_valid: bool,
    allow_pattern_error: Option<String>,
    regex: Option<Regex>,
    counters: TraceFilterCounters,
}

#[derive(Default)]
struct TraceFilterCounters {
    calls_seen: u64,
    calls_allowed: u64,
    calls_filtered_before_args: u64,
    args_inspected: u64,
    calls_with_value_matches: u64,
    transform_frames_started: u64,
}

struct TraceEvent {
    event_index: u64,
    elapsed_ns: u64,
    function: FunctionKey,
    argument_path: String,
    zval_type: String,
    matched_value_id: u32,
    match_kind: String,
    matched_value: String,
    preview: String,
    observed_value: String,
    stack: String,
    stack_json: String,
}

struct TransformedValue {
    value_id: u32,
    parent_value_id: u32,
    elapsed_ns: u64,
    function: FunctionKey,
    transform_kind: String,
    value: String,
    preview: String,
}

#[derive(Serialize)]
struct CompareReport {
    summary: CompareSummary,
    left_only: Vec<CompareRow>,
    right_only: Vec<CompareRow>,
    changed: Vec<CompareRow>,
    same: Vec<CompareRow>,
}

#[derive(Serialize)]
struct CompareSummary {
    left_total_calls: u64,
    right_total_calls: u64,
    left_function_count: usize,
    right_function_count: usize,
    changed_function_count: usize,
}

#[derive(Serialize)]
struct CompareRow {
    status: &'static str,
    kind: String,
    display_name: String,
    scope_name: Option<String>,
    function_name: String,
    file: Option<String>,
    start_line: u32,
    end_line: u32,
    left_count: u64,
    right_count: u64,
    delta: i64,
}

#[derive(Deserialize, Serialize)]
struct TraceReport {
    summary: TraceSummary,
    runs: Vec<TraceRunReport>,
}

#[derive(Deserialize, Serialize)]
struct TraceSummary {
    run_count: usize,
    event_count: usize,
    transformed_value_count: usize,
}

#[derive(Deserialize, Serialize)]
struct TraceRunReport {
    run_id: i64,
    started_at: i64,
    finished_at: Option<i64>,
    status: String,
    trace_value: String,
    trace_value_kind: String,
    php_version: String,
    sapi: String,
    pid: u32,
    script_filename: Option<String>,
    trace_filter: TraceFilterReport,
    event_count: usize,
    transformed_value_count: usize,
    transformed_values: Vec<TransformedValueReport>,
    events: Vec<TraceEventReport>,
}

#[derive(Deserialize, Serialize)]
struct TraceFilterReport {
    mode: String,
    allow_pattern: Option<String>,
    allow_pattern_hash: Option<String>,
    allow_pattern_valid: bool,
    allow_pattern_error: Option<String>,
    calls_seen: u64,
    calls_allowed: u64,
    calls_filtered_before_args: u64,
    args_inspected: u64,
    calls_with_value_matches: u64,
    transform_frames_started: u64,
}

#[derive(Deserialize, Serialize)]
struct TransformedValueReport {
    value_id: u32,
    parent_value_id: u32,
    elapsed_ns: u64,
    transform_kind: String,
    producer: String,
    scope_name: Option<String>,
    function_name: String,
    file: Option<String>,
    start_line: u32,
    end_line: u32,
    value: String,
    preview: String,
}

#[derive(Deserialize, Serialize)]
struct TraceEventReport {
    event_index: u64,
    elapsed_ns: u64,
    kind: String,
    display_name: String,
    scope_name: Option<String>,
    function_name: String,
    file: Option<String>,
    start_line: u32,
    end_line: u32,
    argument_path: String,
    zval_type: String,
    matched_value_id: u32,
    match_kind: String,
    matched_value: String,
    preview: String,
    observed_value: String,
    stack: Vec<String>,
    stack_frames: serde_json::Value,
}

#[derive(Deserialize, Serialize)]
struct UnusedReport {
    summary: UnusedSummary,
    run: Option<UnusedRunReport>,
    uncalled_functions: Vec<UnusedReportRow>,
    uncalled_concrete_methods: Vec<UnusedReportRow>,
    classes_with_no_new_opcode_observed: Vec<UnusedReportRow>,
    global_constants_without_value_access_observed: Vec<UnusedReportRow>,
    class_constants_without_value_access_observed: Vec<UnusedReportRow>,
    global_constants_without_read_observed: Vec<UnusedReportRow>,
    class_constants_without_read_observed: Vec<UnusedReportRow>,
    included_files_with_no_accessed_declarations: Vec<UnusedIncludedFileReport>,
    included_files_without_declarations: Vec<UnusedIncludedFileReport>,
}

#[derive(Deserialize, Serialize)]
struct UnusedSummary {
    run_count: usize,
    run_id: Option<i64>,
    declaration_count: usize,
    access_count: usize,
    uncalled_function_count: usize,
    uncalled_concrete_method_count: usize,
    class_without_new_count: usize,
    global_constant_without_value_access_count: usize,
    class_constant_without_value_access_count: usize,
    global_constant_without_read_count: usize,
    class_constant_without_read_count: usize,
    included_file_count: usize,
    included_file_with_no_accessed_declaration_count: usize,
    included_file_without_declaration_count: usize,
}

#[derive(Clone, Deserialize, Serialize)]
struct UnusedRunReport {
    run_id: i64,
    started_at: i64,
    finished_at: Option<i64>,
    status: String,
    php_version: String,
    sapi: String,
    pid: u32,
    script_filename: Option<String>,
    request_path: Option<String>,
    request_uri_full: Option<String>,
    query_string: Option<String>,
    new_opcode_handler_active: bool,
    constant_opcode_handler_active: bool,
    class_constant_opcode_handler_active: bool,
    caveats: Vec<String>,
}

#[derive(Clone, Deserialize, Serialize)]
struct UnusedReportRow {
    kind: String,
    display_name: String,
    scope_name: Option<String>,
    name: String,
    file: Option<String>,
    start_line: u32,
    end_line: u32,
    flags: u32,
    call_count: u64,
    new_opcode_observed_count: u64,
    fetch_observed_count: u64,
    read_observed_count: u64,
    defined_probe_count: u64,
    file_had_any_access: Option<bool>,
}

#[derive(Clone, Deserialize, Serialize)]
struct UnusedIncludedFileReport {
    file: String,
    include_count: u64,
    declaration_count: usize,
    accessed_declaration_count: usize,
    function_declaration_count: usize,
    method_declaration_count: usize,
    class_declaration_count: usize,
    global_constant_declaration_count: usize,
    class_constant_declaration_count: usize,
}

static STATE: LazyLock<Mutex<Option<State>>> = LazyLock::new(|| Mutex::new(None));
static LAST_STORAGE_ERROR: LazyLock<Mutex<Option<StorageError>>> =
    LazyLock::new(|| Mutex::new(None));

#[no_mangle]
pub extern "C" fn gameshark_core_request_start(
    storage_config: *const GamesharkCoreStorageConfig,
    side: *const c_char,
    trace_value: *const c_char,
    trace_allow_pattern: *const c_char,
    php_version: *const c_char,
    sapi_name: *const c_char,
    pid: u32,
    script_filename: *const c_char,
    unused_enabled: c_int,
    request_path: *const c_char,
    request_uri_full: *const c_char,
    query_string: *const c_char,
    new_opcode_handler_active: c_int,
    constant_opcode_handler_active: c_int,
    class_constant_opcode_handler_active: c_int,
) -> c_int {
    let side = c_string(side).filter(|value| !value.is_empty());
    if let Some(side) = side.as_deref() {
        if side != "left" && side != "right" {
            set_last_storage_error(Some(StorageError {
                code: "config_invalid_side".to_string(),
                backend: None,
                message: format!("invalid differential side {side:?}; expected left or right"),
                hint: Some("Set gameshark.side or GAMESHARK_SIDE to left or right.".to_string()),
            }));
            return 0;
        }
    }

    let trace_value = c_string(trace_value).filter(|value| !value.is_empty());
    let unused_enabled = unused_enabled != 0;
    if side.is_none() && trace_value.is_none() && !unused_enabled {
        set_last_storage_error(None);
        return 0;
    }

    let parsed = parse_storage_config(storage_config);
    if let Some(error) = parsed.error.clone() {
        set_last_storage_error(Some(error));
        return 0;
    }
    let Some(storage) = parsed.target.clone() else {
        set_last_storage_error(None);
        return 0;
    };
    if let Err(error) = validate_storage_for_start(&storage) {
        set_last_storage_error(Some(error));
        return 0;
    }
    set_last_storage_error(None);

    let php_version = c_string(php_version).unwrap_or_default();
    let sapi_name = c_string(sapi_name).unwrap_or_default();
    let script_filename = c_string(script_filename).filter(|value| !value.is_empty());
    let request_path = c_string(request_path).filter(|value| !value.is_empty());
    let request_uri_full = c_string(request_uri_full).filter(|value| !value.is_empty());
    let query_string = c_string(query_string).filter(|value| !value.is_empty());
    let started_at = now();
    let started_at_ns = now_ns();
    let trace_value_kind = trace_value.as_deref().map(trace_value_kind);
    let trace_filter = build_trace_filter(c_string(trace_allow_pattern).as_deref());

    if let StorageTarget::Sqlite { path } = &storage {
        if let Some(side) = side.as_deref() {
            if let Err(message) = initialize_side(
                path,
                side,
                started_at,
                &php_version,
                &sapi_name,
                pid,
                script_filename.as_deref(),
            ) {
                set_last_storage_error(Some(StorageError {
                    code: "storage_start_failed".to_string(),
                    backend: Some("sqlite".to_string()),
                    message,
                    hint: Some("Check that the SQLite path is writable.".to_string()),
                }));
                return 0;
            }
        }
    }

    let trace_run_id = if let Some(trace_value) = trace_value.as_deref() {
        match &storage {
            StorageTarget::Sqlite { path } => match initialize_trace_run(
                path,
                started_at_ns,
                trace_value,
                trace_value_kind.as_deref().unwrap_or("string"),
                &trace_filter,
                &php_version,
                &sapi_name,
                pid,
                script_filename.as_deref(),
            ) {
                Ok(run_id) => Some(run_id),
                Err(message) => {
                    set_last_storage_error(Some(StorageError {
                        code: "storage_start_failed".to_string(),
                        backend: Some("sqlite".to_string()),
                        message,
                        hint: Some("Check that the SQLite path is writable.".to_string()),
                    }));
                    return 0;
                }
            },
            _ => Some(started_at_ns),
        }
    } else {
        None
    };

    let mut unused_caveats = HashSet::new();
    let unused_run_id = if unused_enabled {
        if new_opcode_handler_active == 0 {
            unused_caveats.insert(
                "ZEND_NEW opcode handler unavailable because another extension already registered one"
                    .to_string(),
            );
        }
        if constant_opcode_handler_active == 0 {
            unused_caveats.insert(
                "ZEND_FETCH_CONSTANT opcode handler unavailable because another extension already registered one"
                    .to_string(),
            );
        }
        if class_constant_opcode_handler_active == 0 {
            unused_caveats.insert(
                "ZEND_FETCH_CLASS_CONSTANT opcode handler unavailable because another extension already registered one"
                    .to_string(),
            );
        }
        match &storage {
            StorageTarget::Sqlite { path } => match initialize_unused_run(
                path,
                started_at_ns,
                &php_version,
                &sapi_name,
                pid,
                script_filename.as_deref(),
                request_path.as_deref(),
                request_uri_full.as_deref(),
                query_string.as_deref(),
                new_opcode_handler_active != 0,
                constant_opcode_handler_active != 0,
                class_constant_opcode_handler_active != 0,
            ) {
                Ok(run_id) => Some(run_id),
                Err(message) => {
                    set_last_storage_error(Some(StorageError {
                        code: "storage_start_failed".to_string(),
                        backend: Some("sqlite".to_string()),
                        message,
                        hint: Some("Check that the SQLite path is writable.".to_string()),
                    }));
                    return 0;
                }
            },
            _ => Some(started_at_ns.saturating_add(1)),
        }
    } else {
        None
    };

    let mut state = STATE.lock().expect("gameshark state lock poisoned");
    *state = Some(State {
        storage,
        capture: parsed.capture,
        side,
        started_at,
        started_monotonic: Instant::now(),
        last_elapsed_ns: 0,
        trace_run_id,
        trace_value,
        trace_value_kind,
        php_version,
        sapi_name,
        pid,
        script_filename,
        request_path,
        request_uri_full,
        query_string,
        new_opcode_handler_active: new_opcode_handler_active != 0,
        constant_opcode_handler_active: constant_opcode_handler_active != 0,
        class_constant_opcode_handler_active: class_constant_opcode_handler_active != 0,
        trace_filter,
        counters: HashMap::new(),
        trace_events: Vec::new(),
        transformed_values: Vec::new(),
        unused_run_id,
        unused_declarations: HashMap::new(),
        unused_accesses: HashMap::new(),
        unused_included_files: HashMap::new(),
        unused_caveats,
    });
    1
}

#[no_mangle]
pub extern "C" fn gameshark_core_trace_filter_allows(canonical_name: *const c_char) -> c_int {
    let Some(canonical_name) = c_string(canonical_name) else {
        return 0;
    };

    let mut state = STATE.lock().expect("gameshark state lock poisoned");
    let Some(state) = state.as_mut() else {
        return 0;
    };
    if state.trace_run_id.is_none() {
        return 0;
    }

    state.trace_filter.counters.calls_seen += 1;
    let allowed = state
        .trace_filter
        .regex
        .as_ref()
        .is_some_and(|regex| regex.is_match(&canonical_name));
    if allowed {
        state.trace_filter.counters.calls_allowed += 1;
        1
    } else {
        state.trace_filter.counters.calls_filtered_before_args += 1;
        0
    }
}

#[no_mangle]
pub extern "C" fn gameshark_core_trace_filter_record_argument_result(
    matched: c_int,
    transform_frame_started: c_int,
) {
    let mut state = STATE.lock().expect("gameshark state lock poisoned");
    let Some(state) = state.as_mut() else {
        return;
    };
    if state.trace_run_id.is_none() {
        return;
    }

    state.trace_filter.counters.args_inspected += 1;
    if matched != 0 {
        state.trace_filter.counters.calls_with_value_matches += 1;
    }
    if transform_frame_started != 0 {
        state.trace_filter.counters.transform_frames_started += 1;
    }
}

#[no_mangle]
pub extern "C" fn gameshark_core_trace_filter_error() -> *mut c_char {
    let state = STATE.lock().expect("gameshark state lock poisoned");
    let Some(state) = state.as_ref() else {
        return std::ptr::null_mut();
    };
    let Some(error) = state.trace_filter.allow_pattern_error.as_deref() else {
        return std::ptr::null_mut();
    };

    CString::new(error)
        .unwrap_or_else(|_| CString::new("invalid trace allow pattern").unwrap())
        .into_raw()
}

#[no_mangle]
pub unsafe extern "C" fn gameshark_core_record_call(meta: *const GamesharkCoreFunctionMeta) {
    let Some(key) = function_key_from_meta(meta) else {
        return;
    };

    let mut state = STATE.lock().expect("gameshark state lock poisoned");
    let Some(state) = state.as_mut() else {
        return;
    };
    if state.side.is_none() {
        return;
    }
    *state.counters.entry(key).or_insert(0) += 1;
}

#[no_mangle]
pub unsafe extern "C" fn gameshark_core_record_trace_event(event: *const GamesharkCoreTraceEvent) {
    let Some(event) = event.as_ref() else {
        return;
    };
    let Some(function) = function_key_from_meta(&event.function) else {
        return;
    };

    let argument_path = ffi_str(&event.argument_path).unwrap_or_default();
    let zval_type = ffi_str(&event.zval_type).unwrap_or_default();
    let matched_value_id = event.matched_value_id;
    let matched_value = ffi_str(&event.matched_value).unwrap_or_default();
    let preview = ffi_str(&event.preview).unwrap_or_default();
    let observed_value = ffi_str(&event.observed_value).unwrap_or_default();
    let stack = ffi_str(&event.stack).unwrap_or_default();
    let stack_json = ffi_str(&event.stack_json).unwrap_or_else(|| "[]".to_string());
    let match_kind = match_kind_from_u8(event.match_kind).to_string();

    let mut state = STATE.lock().expect("gameshark state lock poisoned");
    let Some(state) = state.as_mut() else {
        return;
    };
    if state.trace_run_id.is_none() {
        return;
    }

    let mut elapsed_ns = state
        .started_monotonic
        .elapsed()
        .as_nanos()
        .min(u64::MAX as u128) as u64;
    if elapsed_ns <= state.last_elapsed_ns {
        elapsed_ns = state.last_elapsed_ns.saturating_add(1);
    }
    state.last_elapsed_ns = elapsed_ns;

    let event_index = state.trace_events.len() as u64 + 1;
    state.trace_events.push(TraceEvent {
        event_index,
        elapsed_ns,
        function,
        argument_path,
        zval_type,
        matched_value_id,
        match_kind,
        matched_value,
        preview,
        observed_value,
        stack,
        stack_json,
    });
}

#[no_mangle]
pub unsafe extern "C" fn gameshark_core_record_transformed_value(
    value: *const GamesharkCoreTransformedValue,
) {
    let Some(value) = value.as_ref() else {
        return;
    };
    let Some(function) = function_key_from_meta(&value.function) else {
        return;
    };

    let transform_kind = ffi_str(&value.transform_kind).unwrap_or_default();
    let transformed_value = ffi_str(&value.value).unwrap_or_default();
    let preview = ffi_str(&value.preview).unwrap_or_default();

    let mut state = STATE.lock().expect("gameshark state lock poisoned");
    let Some(state) = state.as_mut() else {
        return;
    };
    if state.trace_run_id.is_none() {
        return;
    }

    let mut elapsed_ns = state
        .started_monotonic
        .elapsed()
        .as_nanos()
        .min(u64::MAX as u128) as u64;
    if elapsed_ns <= state.last_elapsed_ns {
        elapsed_ns = state.last_elapsed_ns.saturating_add(1);
    }
    state.last_elapsed_ns = elapsed_ns;

    state.transformed_values.push(TransformedValue {
        value_id: value.value_id,
        parent_value_id: value.parent_value_id,
        elapsed_ns,
        function,
        transform_kind,
        value: transformed_value,
        preview,
    });
}

#[no_mangle]
pub unsafe extern "C" fn gameshark_core_record_unused_declaration(
    declaration: *const GamesharkCoreUnusedDeclaration,
) {
    let Some(declaration) = declaration.as_ref() else {
        return;
    };
    let Some(kind) = UnusedSymbolKind::declaration_from_u8(declaration.kind) else {
        return;
    };
    let Some(name) = ffi_str(&declaration.name).filter(|value| !value.is_empty()) else {
        return;
    };
    let scope_name = ffi_str(&declaration.scope_name).filter(|value| !value.is_empty());
    let incoming_file = ffi_str(&declaration.file).filter(|value| !value.is_empty());
    let key = unused_symbol_key(kind.clone(), scope_name.as_deref(), &name);
    let display_name = unused_display_name(&kind, scope_name.as_deref(), &name);

    let mut state = STATE.lock().expect("gameshark state lock poisoned");
    let Some(state) = state.as_mut() else {
        return;
    };
    if state.unused_run_id.is_none() {
        return;
    }

    let (file, start_line, end_line) = {
        let previous = state.unused_declarations.get(&key);
        (
            incoming_file.or_else(|| previous.and_then(|value| value.file.clone())),
            if declaration.start_line == 0 {
                previous.map_or(0, |value| value.start_line)
            } else {
                declaration.start_line
            },
            if declaration.end_line == 0 {
                previous.map_or(0, |value| value.end_line)
            } else {
                declaration.end_line
            },
        )
    };

    state.unused_declarations.insert(
        key.clone(),
        UnusedDeclaration {
            key,
            display_name,
            scope_name,
            name,
            file,
            start_line,
            end_line,
            flags: declaration.flags,
        },
    );
}

#[no_mangle]
pub unsafe extern "C" fn gameshark_core_record_unused_access(
    access: *const GamesharkCoreUnusedAccess,
) {
    let Some(access) = access.as_ref() else {
        return;
    };
    let Some(access_kind) = UnusedAccessKind::from_u8(access.kind) else {
        return;
    };
    let Some(name) = ffi_str(&access.name).filter(|value| !value.is_empty()) else {
        return;
    };
    let scope_name = ffi_str(&access.scope_name).filter(|value| !value.is_empty());
    let file = ffi_str(&access.file).filter(|value| !value.is_empty());
    let symbol_kind = access_kind.symbol_kind();
    let key = unused_symbol_key(symbol_kind.clone(), scope_name.as_deref(), &name);
    let map_key = (key.clone(), access_kind);
    let display_name = unused_display_name(&symbol_kind, scope_name.as_deref(), &name);

    let mut state = STATE.lock().expect("gameshark state lock poisoned");
    let Some(state) = state.as_mut() else {
        return;
    };
    if state.unused_run_id.is_none() {
        return;
    }

    state
        .unused_accesses
        .entry(map_key)
        .and_modify(|existing| existing.count += 1)
        .or_insert(UnusedAccess {
            key,
            access_kind,
            display_name,
            scope_name,
            name,
            file,
            start_line: access.start_line,
            end_line: access.end_line,
            count: 1,
        });
}

#[no_mangle]
pub extern "C" fn gameshark_core_record_unused_included_file(file: *const c_char) {
    let Some(file) = c_string(file).filter(|value| !value.is_empty()) else {
        return;
    };

    let mut state = STATE.lock().expect("gameshark state lock poisoned");
    let Some(state) = state.as_mut() else {
        return;
    };
    if state.unused_run_id.is_none() {
        return;
    }

    state
        .unused_included_files
        .entry(file)
        .and_modify(|count| *count += 1)
        .or_insert(1);
}

#[no_mangle]
pub extern "C" fn gameshark_core_record_unused_caveat(caveat: *const c_char) {
    let Some(caveat) = c_string(caveat).filter(|value| !value.is_empty()) else {
        return;
    };

    let mut state = STATE.lock().expect("gameshark state lock poisoned");
    let Some(state) = state.as_mut() else {
        return;
    };
    if state.unused_run_id.is_some() {
        state.unused_caveats.insert(caveat);
    }
}

#[no_mangle]
pub extern "C" fn gameshark_core_request_finish() {
    let state = {
        let mut guard = STATE.lock().expect("gameshark state lock poisoned");
        guard.take()
    };

    if let Some(state) = state {
        let _ = flush_state(state);
    }
}

#[no_mangle]
pub extern "C" fn gameshark_core_compare_json(
    storage_config: *const GamesharkCoreStorageConfig,
) -> *mut c_char {
    storage_report_json(storage_config, compare_json_for_storage, || {
        serde_json::json!({
            "summary": {
                "left_total_calls": 0,
                "right_total_calls": 0,
                "left_function_count": 0,
                "right_function_count": 0,
                "changed_function_count": 0
            },
            "left_only": [],
            "right_only": [],
            "changed": [],
            "same": []
        })
    })
}

#[no_mangle]
pub extern "C" fn gameshark_core_compare_text(
    storage_config: *const GamesharkCoreStorageConfig,
    color: c_int,
) -> *mut c_char {
    storage_report_text(storage_config, |target, capture| {
        let report = compare_report_for_storage(target, capture)?;
        Ok(render_compare_text(&report, color != 0))
    })
}

#[no_mangle]
pub extern "C" fn gameshark_core_trace_report_json(
    storage_config: *const GamesharkCoreStorageConfig,
) -> *mut c_char {
    storage_report_json(storage_config, trace_report_json_for_storage, || {
        serde_json::json!({
            "summary": {
                "run_count": 0,
                "event_count": 0,
                "transformed_value_count": 0
            },
            "runs": []
        })
    })
}

#[no_mangle]
pub extern "C" fn gameshark_core_trace_report_text(
    storage_config: *const GamesharkCoreStorageConfig,
    color: c_int,
) -> *mut c_char {
    storage_report_text(storage_config, |target, capture| {
        let report = trace_report_for_storage(target, capture)?;
        Ok(render_trace_text(&report, color != 0))
    })
}

#[no_mangle]
pub extern "C" fn gameshark_core_unused_report_json(
    storage_config: *const GamesharkCoreStorageConfig,
    run_id: i64,
) -> *mut c_char {
    storage_report_json(
        storage_config,
        |target, capture| unused_report_json_for_storage(target, capture, run_id),
        empty_unused_report_json,
    )
}

#[no_mangle]
pub extern "C" fn gameshark_core_unused_report_text(
    storage_config: *const GamesharkCoreStorageConfig,
    color: c_int,
    run_id: i64,
) -> *mut c_char {
    storage_report_text(storage_config, |target, capture| {
        let report = unused_report_for_storage(target, capture, run_id)?;
        Ok(render_unused_text(&report, color != 0))
    })
}

#[no_mangle]
pub extern "C" fn gameshark_core_unused_aggregate_report_json(
    storage_config: *const GamesharkCoreStorageConfig,
    capture: *const c_char,
    since_run_id: i64,
    until_run_id: i64,
) -> *mut c_char {
    storage_report_json(
        storage_config,
        |target, parsed_capture| {
            let capture = c_string(capture)
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| parsed_capture.to_string());
            let report =
                unused_aggregate_report_for_storage(target, &capture, since_run_id, until_run_id)?;
            serde_json::to_string(&report).map_err(|error| {
                storage_error(
                    target.backend_name(),
                    "report_encode_failed",
                    error.to_string(),
                    None,
                )
            })
        },
        empty_unused_report_json,
    )
}

#[no_mangle]
pub extern "C" fn gameshark_core_unused_aggregate_report_text(
    storage_config: *const GamesharkCoreStorageConfig,
    color: c_int,
    capture: *const c_char,
    since_run_id: i64,
    until_run_id: i64,
) -> *mut c_char {
    storage_report_text(storage_config, |target, parsed_capture| {
        let capture = c_string(capture)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| parsed_capture.to_string());
        let report =
            unused_aggregate_report_for_storage(target, &capture, since_run_id, until_run_id)?;
        Ok(render_unused_text(&report, color != 0))
    })
}

#[no_mangle]
pub unsafe extern "C" fn gameshark_core_string_free(ptr: *mut c_char) {
    if !ptr.is_null() {
        drop(CString::from_raw(ptr));
    }
}

fn storage_report_json<F, E>(
    storage_config: *const GamesharkCoreStorageConfig,
    build: F,
    empty: E,
) -> *mut c_char
where
    F: FnOnce(&StorageTarget, &str) -> Result<String, StorageError>,
    E: FnOnce() -> serde_json::Value,
{
    let result = storage_target_for_report(storage_config)
        .and_then(|(target, capture)| build(&target, &capture));
    let json = match result {
        Ok(json) => json,
        Err(error) => {
            let mut value = empty();
            if let serde_json::Value::Object(ref mut object) = value {
                object.insert(
                    "error".to_string(),
                    serde_json::Value::String(error.message.clone()),
                );
                object.insert(
                    "error_code".to_string(),
                    serde_json::Value::String(error.code.clone()),
                );
                object.insert(
                    "error_backend".to_string(),
                    error
                        .backend
                        .as_ref()
                        .map(|value| serde_json::Value::String(value.clone()))
                        .unwrap_or(serde_json::Value::Null),
                );
                object.insert(
                    "error_hint".to_string(),
                    error
                        .hint
                        .as_ref()
                        .map(|value| serde_json::Value::String(value.clone()))
                        .unwrap_or(serde_json::Value::Null),
                );
            }
            value.to_string()
        }
    };

    c_string_from_string(json)
}

fn storage_report_text<F>(
    storage_config: *const GamesharkCoreStorageConfig,
    build: F,
) -> *mut c_char
where
    F: FnOnce(&StorageTarget, &str) -> Result<String, StorageError>,
{
    let text = storage_target_for_report(storage_config)
        .and_then(|(target, capture)| build(&target, &capture))
        .unwrap_or_else(render_storage_error_text);

    c_string_from_string(text)
}

fn empty_unused_report_json() -> serde_json::Value {
    serde_json::json!({
        "summary": {
            "run_count": 0,
            "run_id": null,
            "declaration_count": 0,
            "access_count": 0,
            "uncalled_function_count": 0,
            "uncalled_concrete_method_count": 0,
            "class_without_new_count": 0,
            "global_constant_without_value_access_count": 0,
            "class_constant_without_value_access_count": 0,
            "global_constant_without_read_count": 0,
            "class_constant_without_read_count": 0,
            "included_file_count": 0,
            "included_file_with_no_accessed_declaration_count": 0,
            "included_file_without_declaration_count": 0
        },
        "run": null,
        "uncalled_functions": [],
        "uncalled_concrete_methods": [],
        "classes_with_no_new_opcode_observed": [],
        "global_constants_without_value_access_observed": [],
        "class_constants_without_value_access_observed": [],
        "global_constants_without_read_observed": [],
        "class_constants_without_read_observed": [],
        "included_files_with_no_accessed_declarations": [],
        "included_files_without_declarations": []
    })
}

fn storage_target_for_report(
    storage_config: *const GamesharkCoreStorageConfig,
) -> Result<(StorageTarget, String), StorageError> {
    let parsed = parse_storage_config(storage_config);
    if let Some(error) = parsed.error {
        return Err(error);
    }
    let Some(target) = parsed.target else {
        return Err(StorageError {
            code: "config_missing_storage".to_string(),
            backend: None,
            message: "Gameshark storage is not configured".to_string(),
            hint: Some(
                "Set gameshark.db/GAMESHARK_DB or gameshark.storage plus connection settings."
                    .to_string(),
            ),
        });
    };
    Ok((target, parsed.capture))
}

fn render_storage_error_text(error: StorageError) -> String {
    format!(
        "Gameshark report error\ncode: {}\nbackend: {}\nmessage: {}\nhint: {}\n",
        error.code,
        error.backend.as_deref().unwrap_or("none"),
        error.message,
        error.hint.as_deref().unwrap_or("")
    )
}

fn storage_error(backend: &str, code: &str, message: String, hint: Option<&str>) -> StorageError {
    StorageError {
        code: code.to_string(),
        backend: Some(backend.to_string()),
        message,
        hint: hint.map(str::to_string),
    }
}

fn c_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let string = unsafe { CStr::from_ptr(ptr) };
    Some(string.to_string_lossy().into_owned())
}

fn set_last_storage_error(error: Option<StorageError>) {
    let mut guard = LAST_STORAGE_ERROR
        .lock()
        .expect("gameshark storage error lock poisoned");
    *guard = error;
}

#[no_mangle]
pub extern "C" fn gameshark_core_last_error_json() -> *mut c_char {
    let error = LAST_STORAGE_ERROR
        .lock()
        .expect("gameshark storage error lock poisoned")
        .clone();
    let json = match error {
        Some(error) => serde_json::to_string(&error).unwrap_or_else(|_| "{}".to_string()),
        None => "null".to_string(),
    };
    c_string_from_string(json)
}

#[no_mangle]
pub extern "C" fn gameshark_core_storage_status_json(
    storage_config: *const GamesharkCoreStorageConfig,
) -> *mut c_char {
    let parsed = parse_storage_config(storage_config);
    let target = parsed.target.as_ref();
    let backend = target.map(StorageTarget::backend_name);
    let schema_mode = target.and_then(StorageTarget::schema_mode_name);
    let schema_status = if parsed.error.is_some() {
        "invalid"
    } else if target.is_some() {
        "not_checked"
    } else {
        "unknown"
    };
    let value = serde_json::json!({
        "configured": target.is_some() || parsed.error.is_some(),
        "active": STATE.lock().expect("gameshark state lock poisoned").is_some(),
        "backend": backend,
        "capture": parsed.capture,
        "compiled_backends": {
            "sqlite": true,
            "mysql": cfg!(feature = "backend-mysql"),
            "redis": cfg!(feature = "backend-redis")
        },
        "target": target.map(StorageTarget::status_target),
        "sources": {
            "storage": parsed.sources.storage,
            "dsn": parsed.sources.dsn,
            "legacy_db": parsed.sources.legacy_db,
            "capture": parsed.sources.capture,
            "credentials": parsed.sources.credentials,
            "schema_mode": parsed.sources.schema_mode,
            "timeouts": parsed.sources.timeouts
        },
        "schema": {
            "mode": schema_mode,
            "status": schema_status,
            "version": null,
            "required_version": 1,
            "checked_at": null
        },
        "ignored_legacy_sqlite_path": parsed.ignored_legacy_sqlite_path,
        "last_error": parsed.error
    });
    c_string_from_string(value.to_string())
}

#[no_mangle]
pub extern "C" fn gameshark_core_storage_db_path(
    storage_config: *const GamesharkCoreStorageConfig,
) -> *mut c_char {
    let parsed = parse_storage_config(storage_config);
    let Some(StorageTarget::Sqlite { path }) = parsed.target else {
        return std::ptr::null_mut();
    };
    c_string_from_string(path)
}

fn c_string_from_string(value: String) -> *mut c_char {
    CString::new(value)
        .unwrap_or_else(|error| {
            let text = error.into_vec();
            CString::new(String::from_utf8_lossy(&text).replace('\0', "\\0")).unwrap()
        })
        .into_raw()
}

fn parse_storage_config(config: *const GamesharkCoreStorageConfig) -> ParsedStorageConfig {
    let raw = unsafe { config.as_ref() };
    let (storage, storage_source) =
        select_config(raw.map(|c| c.storage_ini), raw.map(|c| c.storage_env));
    let (dsn, dsn_source) = select_config(raw.map(|c| c.dsn_ini), raw.map(|c| c.dsn_env));
    let (legacy_db, legacy_source) =
        select_config(raw.map(|c| c.legacy_db_ini), raw.map(|c| c.legacy_db_env));
    let (capture_value, capture_source) =
        select_config(raw.map(|c| c.capture_ini), raw.map(|c| c.capture_env));
    let capture = capture_value.unwrap_or_else(|| "default".to_string());

    let mut sources = StorageSources {
        storage: storage_source.to_string(),
        dsn: dsn_source.to_string(),
        legacy_db: legacy_source.to_string(),
        capture: if capture_source == "unset" {
            "default".to_string()
        } else {
            capture_source.to_string()
        },
        credentials: "none".to_string(),
        schema_mode: "not_applicable".to_string(),
        timeouts: "default".to_string(),
    };

    if !valid_capture(&capture) {
        return parsed_error(
            capture,
            sources,
            "config_invalid_capture",
            None,
            "gameshark capture must match [A-Za-z0-9_.:-]{1,128}",
            Some("Set gameshark.capture or GAMESHARK_CAPTURE to a short stable sample name."),
        );
    }

    let storage_normalized = storage
        .as_deref()
        .map(|value| value.trim().to_ascii_lowercase());
    let dsn_backend = dsn.as_deref().and_then(dsn_backend);
    if dsn.is_some() && dsn_backend.is_none() {
        return parsed_error(
            capture,
            sources,
            "config_invalid_dsn",
            None,
            "gameshark.dsn must start with sqlite:, mysql://, redis://, or rediss://",
            None,
        );
    }
    if let (Some(storage), Some(dsn_backend)) = (storage_normalized.as_deref(), dsn_backend) {
        if storage != dsn_backend {
            return parsed_error(
                capture,
                sources,
                "config_conflict",
                Some(storage),
                "explicit gameshark.storage does not match gameshark.dsn scheme",
                Some("Use matching storage and DSN scheme, or omit gameshark.storage."),
            );
        }
    }

    let mysql_split = raw.is_some_and(any_mysql_split_config);
    let redis_split = raw.is_some_and(any_redis_split_config);
    if storage_normalized.is_none() && dsn_backend.is_none() && mysql_split && redis_split {
        return parsed_error(
            capture,
            sources,
            "config_conflict",
            None,
            "both MySQL and Redis split configuration values were provided",
            Some("Set gameshark.storage explicitly when more than one backend has split settings."),
        );
    }

    let selected_backend = if let Some(storage) = storage_normalized.as_deref() {
        match storage {
            "sqlite" | "mysql" | "redis" => Some(storage.to_string()),
            _ => {
                return parsed_error(
                    capture,
                    sources,
                    "config_invalid_storage",
                    None,
                    "gameshark.storage must be sqlite, mysql, or redis",
                    None,
                )
            }
        }
    } else if let Some(backend) = dsn_backend {
        sources.storage = "dsn".to_string();
        Some(backend.to_string())
    } else if mysql_split {
        sources.storage = "default".to_string();
        Some("mysql".to_string())
    } else if redis_split {
        sources.storage = "default".to_string();
        Some("redis".to_string())
    } else if legacy_db.is_some() {
        sources.storage = "legacy".to_string();
        Some("sqlite".to_string())
    } else {
        None
    };

    let ignored_legacy_sqlite_path = if legacy_db.is_some()
        && selected_backend
            .as_deref()
            .is_some_and(|backend| backend != "sqlite")
    {
        sources.legacy_db = "ignored".to_string();
        legacy_db.as_deref().map(redact_path)
    } else {
        None
    };

    let Some(backend) = selected_backend.as_deref() else {
        return ParsedStorageConfig {
            target: None,
            capture,
            sources,
            ignored_legacy_sqlite_path: None,
            error: None,
        };
    };

    let target = match backend {
        "sqlite" => parse_sqlite_target(dsn.as_deref(), legacy_db.as_deref()),
        "mysql" => parse_mysql_target(raw, dsn.as_deref(), &mut sources),
        "redis" => parse_redis_target(raw, dsn.as_deref(), &mut sources),
        _ => unreachable!(),
    };

    match target {
        Ok(target) => ParsedStorageConfig {
            target: Some(target),
            capture,
            sources,
            ignored_legacy_sqlite_path,
            error: None,
        },
        Err(mut error) => {
            if error.backend.is_none() {
                error.backend = Some(backend.to_string());
            }
            ParsedStorageConfig {
                target: None,
                capture,
                sources,
                ignored_legacy_sqlite_path,
                error: Some(error),
            }
        }
    }
}

fn select_config(
    ini: Option<*const c_char>,
    env: Option<*const c_char>,
) -> (Option<String>, &'static str) {
    let ini = ini
        .and_then(c_string)
        .filter(|value| config_has_value(value));
    if ini.is_some() {
        return (ini, "ini");
    }
    let env = env
        .and_then(c_string)
        .filter(|value| config_has_value(value));
    if env.is_some() {
        return (env, "env");
    }
    (None, "unset")
}

fn config_has_value(value: &str) -> bool {
    value.chars().any(|ch| !ch.is_whitespace())
}

fn valid_capture(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-'))
}

fn parsed_error(
    capture: String,
    sources: StorageSources,
    code: &str,
    backend: Option<&str>,
    message: &str,
    hint: Option<&str>,
) -> ParsedStorageConfig {
    ParsedStorageConfig {
        target: None,
        capture,
        sources,
        ignored_legacy_sqlite_path: None,
        error: Some(StorageError {
            code: code.to_string(),
            backend: backend.map(str::to_string),
            message: message.to_string(),
            hint: hint.map(str::to_string),
        }),
    }
}

fn dsn_backend(dsn: &str) -> Option<&'static str> {
    if dsn.starts_with("sqlite:") {
        Some("sqlite")
    } else if dsn.starts_with("mysql://") {
        Some("mysql")
    } else if dsn.starts_with("redis://") || dsn.starts_with("rediss://") {
        Some("redis")
    } else {
        None
    }
}

fn parse_sqlite_target(
    dsn: Option<&str>,
    legacy_db: Option<&str>,
) -> Result<StorageTarget, StorageError> {
    let path = if let Some(dsn) = dsn {
        parse_sqlite_dsn_path(dsn)?
    } else {
        legacy_db
            .filter(|value| config_has_value(value))
            .map(str::to_string)
            .ok_or_else(|| StorageError {
                code: "config_missing_sqlite_path".to_string(),
                backend: Some("sqlite".to_string()),
                message: "SQLite storage requires gameshark.db, GAMESHARK_DB, or sqlite: DSN"
                    .to_string(),
                hint: Some("Set gameshark.db=/path/to/gameshark.sqlite.".to_string()),
            })?
    };
    Ok(StorageTarget::Sqlite { path })
}

fn parse_sqlite_dsn_path(dsn: &str) -> Result<String, StorageError> {
    if !dsn.starts_with("sqlite:") {
        return Err(StorageError {
            code: "config_invalid_dsn".to_string(),
            backend: Some("sqlite".to_string()),
            message: "SQLite DSN must start with sqlite:".to_string(),
            hint: None,
        });
    }
    let rest = &dsn["sqlite:".len()..];
    if let Some(after_slashes) = rest.strip_prefix("//") {
        if !after_slashes.starts_with('/') {
            return Err(StorageError {
                code: "config_invalid_dsn".to_string(),
                backend: Some("sqlite".to_string()),
                message: "sqlite://host/path DSNs are not supported".to_string(),
                hint: Some(
                    "Use sqlite:/absolute/path, sqlite:///absolute/path, or sqlite:relative/path."
                        .to_string(),
                ),
            });
        }
        return Ok(percent_decode_once(after_slashes));
    }
    if rest.is_empty() {
        return Err(StorageError {
            code: "config_missing_sqlite_path".to_string(),
            backend: Some("sqlite".to_string()),
            message: "SQLite DSN did not include a database path".to_string(),
            hint: None,
        });
    }
    Ok(percent_decode_once(rest))
}

fn parse_mysql_target(
    raw: Option<&GamesharkCoreStorageConfig>,
    dsn: Option<&str>,
    sources: &mut StorageSources,
) -> Result<StorageTarget, StorageError> {
    if !cfg!(feature = "backend-mysql") {
        return Err(StorageError {
            code: "backend_not_compiled".to_string(),
            backend: Some("mysql".to_string()),
            message: "Gameshark was built without MySQL/MariaDB backend support".to_string(),
            hint: Some("Rebuild with GAMESHARK_BACKENDS=all.".to_string()),
        });
    }
    let schema_mode = select_config(
        raw.map(|c| c.mysql_schema_mode_ini),
        raw.map(|c| c.mysql_schema_mode_env),
    );
    sources.schema_mode = if schema_mode.1 == "unset" {
        "default".to_string()
    } else {
        schema_mode.1.to_string()
    };
    let schema_mode = parse_schema_mode(schema_mode.0.as_deref(), SchemaMode::Validate)?;
    let (connect_timeout_ms, operation_timeout_ms, report_timeout_ms, timeout_source) =
        mysql_timeouts(raw)?;
    sources.timeouts = timeout_source;

    if let Some(dsn) = dsn {
        if !dsn.starts_with("mysql://") {
            return Err(StorageError {
                code: "config_invalid_dsn".to_string(),
                backend: Some("mysql".to_string()),
                message: "MySQL/MariaDB DSN must start with mysql://".to_string(),
                hint: None,
            });
        }
        sources.credentials = if dsn_contains_credentials(dsn) {
            "dsn"
        } else {
            "none"
        }
        .to_string();
        return Ok(StorageTarget::Mysql {
            dsn: strip_gameshark_query_params(dsn),
            display: redact_dsn(dsn),
            host: dsn_host(dsn),
            port: dsn_port(dsn),
            database: dsn_database(dsn),
            socket: dsn_query_param(dsn, "socket"),
            schema_mode,
            connect_timeout_ms,
            operation_timeout_ms,
            report_timeout_ms,
        });
    }

    let host = select_config(raw.map(|c| c.mysql_host_ini), raw.map(|c| c.mysql_host_env)).0;
    let port = select_config(raw.map(|c| c.mysql_port_ini), raw.map(|c| c.mysql_port_env))
        .0
        .as_deref()
        .map(parse_u16)
        .transpose()?
        .unwrap_or(3306);
    let database = select_config(
        raw.map(|c| c.mysql_database_ini),
        raw.map(|c| c.mysql_database_env),
    )
    .0
    .ok_or_else(|| StorageError {
        code: "config_missing_database".to_string(),
        backend: Some("mysql".to_string()),
        message: "MySQL/MariaDB storage requires gameshark.mysql.database".to_string(),
        hint: None,
    })?;
    let username = select_config(
        raw.map(|c| c.mysql_username_ini),
        raw.map(|c| c.mysql_username_env),
    )
    .0;
    let password = selected_password(
        raw.map(|c| c.mysql_password_ini),
        raw.map(|c| c.mysql_password_env),
        raw.map(|c| c.mysql_password_file_ini),
        raw.map(|c| c.mysql_password_file_env),
        &mut sources.credentials,
        "mysql",
    )?;
    let socket = select_config(
        raw.map(|c| c.mysql_socket_ini),
        raw.map(|c| c.mysql_socket_env),
    )
    .0;
    if sources.credentials == "none" && (username.is_some() || password.is_some()) {
        sources.credentials = "split".to_string();
    }
    let dsn = build_mysql_dsn(
        host.as_deref().unwrap_or("127.0.0.1"),
        port,
        &database,
        username.as_deref(),
        password.as_deref(),
        socket.as_deref(),
    );
    let display = if let Some(socket) = socket.as_deref() {
        format!(
            "mysql://{}/{}?socket={}",
            host.as_deref().unwrap_or("localhost"),
            database,
            socket
        )
    } else {
        format!(
            "mysql://{}:{}/{}",
            host.as_deref().unwrap_or("127.0.0.1"),
            port,
            database
        )
    };
    Ok(StorageTarget::Mysql {
        dsn,
        display,
        host,
        port: Some(port),
        database: Some(database),
        socket,
        schema_mode,
        connect_timeout_ms,
        operation_timeout_ms,
        report_timeout_ms,
    })
}

fn parse_redis_target(
    raw: Option<&GamesharkCoreStorageConfig>,
    dsn: Option<&str>,
    sources: &mut StorageSources,
) -> Result<StorageTarget, StorageError> {
    if !cfg!(feature = "backend-redis") {
        return Err(StorageError {
            code: "backend_not_compiled".to_string(),
            backend: Some("redis".to_string()),
            message: "Gameshark was built without Redis backend support".to_string(),
            hint: Some("Rebuild with GAMESHARK_BACKENDS=all.".to_string()),
        });
    }
    sources.schema_mode = "not_applicable".to_string();
    let (connect_timeout_ms, operation_timeout_ms, report_timeout_ms, timeout_source) =
        redis_timeouts(raw)?;
    sources.timeouts = timeout_source;
    let key_prefix = select_config(
        raw.map(|c| c.redis_key_prefix_ini),
        raw.map(|c| c.redis_key_prefix_env),
    )
    .0
    .or_else(|| dsn.and_then(|value| dsn_query_param(value, "key_prefix")))
    .unwrap_or_else(|| "gameshark".to_string());
    if !valid_redis_key_prefix(&key_prefix) {
        return Err(StorageError {
            code: "config_invalid_key_prefix".to_string(),
            backend: Some("redis".to_string()),
            message: "Redis key prefix must match [A-Za-z0-9:_-]{1,80}".to_string(),
            hint: None,
        });
    }
    let ttl_value = select_config(raw.map(|c| c.redis_ttl_ini), raw.map(|c| c.redis_ttl_env))
        .0
        .or_else(|| dsn.and_then(|value| dsn_query_param(value, "ttl")));
    let ttl = match ttl_value.as_deref() {
        Some(value) => parse_u64(value).map_err(|message| StorageError {
            code: "config_invalid_ttl".to_string(),
            backend: Some("redis".to_string()),
            message,
            hint: None,
        })?,
        None => 3600,
    };
    if ttl < 60 {
        return Err(StorageError {
            code: "config_invalid_ttl".to_string(),
            backend: Some("redis".to_string()),
            message: "Redis ttl must be at least 60 seconds".to_string(),
            hint: None,
        });
    }

    if let Some(dsn) = dsn {
        if dsn.starts_with("rediss://") && !cfg!(feature = "backend-redis") {
            return Err(StorageError {
                code: "backend_not_compiled".to_string(),
                backend: Some("redis".to_string()),
                message: "rediss:// requires Redis TLS support".to_string(),
                hint: Some("Rebuild with GAMESHARK_BACKENDS=all.".to_string()),
            });
        }
        if !dsn.starts_with("redis://") && !dsn.starts_with("rediss://") {
            return Err(StorageError {
                code: "config_invalid_dsn".to_string(),
                backend: Some("redis".to_string()),
                message: "Redis DSN must start with redis:// or rediss://".to_string(),
                hint: None,
            });
        }
        sources.credentials = if dsn_contains_credentials(dsn) {
            "dsn"
        } else {
            "none"
        }
        .to_string();
        return Ok(StorageTarget::Redis {
            dsn: strip_gameshark_query_params(dsn),
            display: redact_dsn(dsn),
            host: dsn_host(dsn),
            port: dsn_port(dsn),
            database: dsn_database(dsn).and_then(|value| value.parse::<i64>().ok()),
            key_prefix,
            ttl,
            connect_timeout_ms,
            operation_timeout_ms,
            report_timeout_ms,
        });
    }

    let host = select_config(raw.map(|c| c.redis_host_ini), raw.map(|c| c.redis_host_env))
        .0
        .unwrap_or_else(|| "127.0.0.1".to_string());
    let port = select_config(raw.map(|c| c.redis_port_ini), raw.map(|c| c.redis_port_env))
        .0
        .as_deref()
        .map(parse_u16)
        .transpose()?
        .unwrap_or(6379);
    let database = select_config(
        raw.map(|c| c.redis_database_ini),
        raw.map(|c| c.redis_database_env),
    )
    .0
    .as_deref()
    .map(parse_i64)
    .transpose()?
    .unwrap_or(0);
    let username = select_config(
        raw.map(|c| c.redis_username_ini),
        raw.map(|c| c.redis_username_env),
    )
    .0;
    let password = selected_password(
        raw.map(|c| c.redis_password_ini),
        raw.map(|c| c.redis_password_env),
        raw.map(|c| c.redis_password_file_ini),
        raw.map(|c| c.redis_password_file_env),
        &mut sources.credentials,
        "redis",
    )?;
    if sources.credentials == "none" && (username.is_some() || password.is_some()) {
        sources.credentials = "split".to_string();
    }
    let dsn = build_redis_dsn(
        &host,
        port,
        database,
        username.as_deref(),
        password.as_deref(),
    );
    Ok(StorageTarget::Redis {
        dsn,
        display: format!("redis://{}:{}/{}", host, port, database),
        host: Some(host),
        port: Some(port),
        database: Some(database),
        key_prefix,
        ttl,
        connect_timeout_ms,
        operation_timeout_ms,
        report_timeout_ms,
    })
}

fn any_mysql_split_config(config: &GamesharkCoreStorageConfig) -> bool {
    [
        config.mysql_host_ini,
        config.mysql_host_env,
        config.mysql_port_ini,
        config.mysql_port_env,
        config.mysql_database_ini,
        config.mysql_database_env,
        config.mysql_username_ini,
        config.mysql_username_env,
        config.mysql_password_ini,
        config.mysql_password_env,
        config.mysql_password_file_ini,
        config.mysql_password_file_env,
        config.mysql_socket_ini,
        config.mysql_socket_env,
    ]
    .into_iter()
    .filter_map(c_string)
    .any(|value| config_has_value(&value))
}

fn any_redis_split_config(config: &GamesharkCoreStorageConfig) -> bool {
    [
        config.redis_host_ini,
        config.redis_host_env,
        config.redis_port_ini,
        config.redis_port_env,
        config.redis_database_ini,
        config.redis_database_env,
        config.redis_username_ini,
        config.redis_username_env,
        config.redis_password_ini,
        config.redis_password_env,
        config.redis_password_file_ini,
        config.redis_password_file_env,
        config.redis_key_prefix_ini,
        config.redis_key_prefix_env,
        config.redis_ttl_ini,
        config.redis_ttl_env,
    ]
    .into_iter()
    .filter_map(c_string)
    .any(|value| config_has_value(&value))
}

fn parse_schema_mode(value: Option<&str>, default: SchemaMode) -> Result<SchemaMode, StorageError> {
    match value.map(|value| value.trim().to_ascii_lowercase()) {
        None => Ok(default),
        Some(value) if value == "auto" => Ok(SchemaMode::Auto),
        Some(value) if value == "validate" => Ok(SchemaMode::Validate),
        _ => Err(StorageError {
            code: "config_invalid_schema_mode".to_string(),
            backend: Some("mysql".to_string()),
            message: "MySQL schema mode must be auto or validate".to_string(),
            hint: None,
        }),
    }
}

fn selected_password(
    password_ini: Option<*const c_char>,
    password_env: Option<*const c_char>,
    password_file_ini: Option<*const c_char>,
    password_file_env: Option<*const c_char>,
    credential_source: &mut String,
    backend: &str,
) -> Result<Option<String>, StorageError> {
    let (password_file, password_file_source) = select_config(password_file_ini, password_file_env);
    if let Some(path) = password_file {
        *credential_source = "password_file".to_string();
        return read_password_file(&path, backend)
            .map(Some)
            .map_err(|message| StorageError {
                code: "config_secret_error".to_string(),
                backend: Some(backend.to_string()),
                message,
                hint: Some(
                    "Fix gameshark.*.password_file permissions/content, or remove it.".to_string(),
                ),
            });
    }
    let (password, password_source) = select_config(password_ini, password_env);
    if password.is_some() {
        *credential_source = if password_source == "unset" {
            "none".to_string()
        } else {
            "split".to_string()
        };
    } else if password_file_source != "unset" {
        *credential_source = "password_file".to_string();
    }
    Ok(password)
}

fn read_password_file(path: &str, backend: &str) -> Result<String, String> {
    let metadata = std::fs::metadata(path).map_err(|_| {
        format!(
            "{backend} password_file could not be read: {}",
            redact_path(path)
        )
    })?;
    if metadata.len() > 65_536 {
        return Err(format!(
            "{backend} password_file is too large: {}",
            redact_path(path)
        ));
    }
    let mut value = std::fs::read_to_string(path).map_err(|_| {
        format!(
            "{backend} password_file could not be read: {}",
            redact_path(path)
        )
    })?;
    if value.ends_with("\r\n") {
        value.truncate(value.len() - 2);
    } else if value.ends_with('\n') || value.ends_with('\r') {
        value.truncate(value.len() - 1);
    }
    if value.is_empty() {
        return Err(format!(
            "{backend} password_file is empty: {}",
            redact_path(path)
        ));
    }
    Ok(value)
}

fn mysql_timeouts(
    raw: Option<&GamesharkCoreStorageConfig>,
) -> Result<(u64, u64, u64, String), StorageError> {
    parse_timeouts(
        raw.map(|c| c.mysql_connect_timeout_ms_ini),
        raw.map(|c| c.mysql_connect_timeout_ms_env),
        raw.map(|c| c.mysql_operation_timeout_ms_ini),
        raw.map(|c| c.mysql_operation_timeout_ms_env),
        raw.map(|c| c.mysql_report_timeout_ms_ini),
        raw.map(|c| c.mysql_report_timeout_ms_env),
        (1000, 5000, 10000),
        "mysql",
    )
}

fn redis_timeouts(
    raw: Option<&GamesharkCoreStorageConfig>,
) -> Result<(u64, u64, u64, String), StorageError> {
    parse_timeouts(
        raw.map(|c| c.redis_connect_timeout_ms_ini),
        raw.map(|c| c.redis_connect_timeout_ms_env),
        raw.map(|c| c.redis_operation_timeout_ms_ini),
        raw.map(|c| c.redis_operation_timeout_ms_env),
        raw.map(|c| c.redis_report_timeout_ms_ini),
        raw.map(|c| c.redis_report_timeout_ms_env),
        (500, 2000, 10000),
        "redis",
    )
}

fn parse_timeouts(
    connect_ini: Option<*const c_char>,
    connect_env: Option<*const c_char>,
    operation_ini: Option<*const c_char>,
    operation_env: Option<*const c_char>,
    report_ini: Option<*const c_char>,
    report_env: Option<*const c_char>,
    defaults: (u64, u64, u64),
    backend: &str,
) -> Result<(u64, u64, u64, String), StorageError> {
    let (connect, connect_source) = select_config(connect_ini, connect_env);
    let (operation, operation_source) = select_config(operation_ini, operation_env);
    let (report, report_source) = select_config(report_ini, report_env);
    let connect = connect
        .as_deref()
        .map(parse_timeout_ms)
        .transpose()
        .map_err(|message| timeout_error(backend, message))?
        .unwrap_or(defaults.0);
    let operation = operation
        .as_deref()
        .map(parse_timeout_ms)
        .transpose()
        .map_err(|message| timeout_error(backend, message))?
        .unwrap_or(defaults.1);
    let report = report
        .as_deref()
        .map(parse_timeout_ms)
        .transpose()
        .map_err(|message| timeout_error(backend, message))?
        .unwrap_or(defaults.2);
    let mut sources = HashSet::new();
    for source in [connect_source, operation_source, report_source] {
        sources.insert(if source == "unset" { "default" } else { source });
    }
    let source = if sources.len() == 1 {
        sources.into_iter().next().unwrap().to_string()
    } else {
        "mixed".to_string()
    };
    Ok((connect, operation, report, source))
}

fn timeout_error(backend: &str, message: String) -> StorageError {
    StorageError {
        code: "config_invalid_timeout".to_string(),
        backend: Some(backend.to_string()),
        message,
        hint: Some("Timeouts must be integer milliseconds from 1 to 60000.".to_string()),
    }
}

fn parse_timeout_ms(value: &str) -> Result<u64, String> {
    let parsed = parse_u64(value)?;
    if (1..=60_000).contains(&parsed) {
        Ok(parsed)
    } else {
        Err(format!(
            "timeout {parsed} is outside the supported range 1..60000"
        ))
    }
}

fn parse_u16(value: &str) -> Result<u16, StorageError> {
    value.trim().parse::<u16>().map_err(|_| StorageError {
        code: "config_invalid_port".to_string(),
        backend: None,
        message: format!("invalid port value {value:?}"),
        hint: None,
    })
}

fn parse_i64(value: &str) -> Result<i64, StorageError> {
    value.trim().parse::<i64>().map_err(|_| StorageError {
        code: "config_invalid_database".to_string(),
        backend: None,
        message: format!("invalid database value {value:?}"),
        hint: None,
    })
}

fn parse_u64(value: &str) -> Result<u64, String> {
    value
        .trim()
        .parse::<u64>()
        .map_err(|_| format!("invalid unsigned integer value {value:?}"))
}

fn valid_redis_key_prefix(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'_' | b'-'))
}

fn build_mysql_dsn(
    host: &str,
    port: u16,
    database: &str,
    username: Option<&str>,
    password: Option<&str>,
    socket: Option<&str>,
) -> String {
    let auth = match (username, password) {
        (Some(user), Some(password)) => {
            format!("{}:{}@", percent_encode(user), percent_encode(password))
        }
        (Some(user), None) => format!("{}@", percent_encode(user)),
        _ => String::new(),
    };
    let mut dsn = format!(
        "mysql://{}{}:{}/{}",
        auth,
        host,
        port,
        percent_encode(database)
    );
    if let Some(socket) = socket {
        dsn.push_str("?socket=");
        dsn.push_str(&percent_encode(socket));
    }
    dsn
}

fn build_redis_dsn(
    host: &str,
    port: u16,
    database: i64,
    username: Option<&str>,
    password: Option<&str>,
) -> String {
    let auth = match (username, password) {
        (Some(user), Some(password)) => {
            format!("{}:{}@", percent_encode(user), percent_encode(password))
        }
        (None, Some(password)) => format!(":{}@", percent_encode(password)),
        (Some(user), None) => format!("{}@", percent_encode(user)),
        _ => String::new(),
    };
    format!("redis://{}{}:{}/{}", auth, host, port, database)
}

fn percent_encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

fn percent_decode_once(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(high), Some(low)) = (hex_value(bytes[i + 1]), hex_value(bytes[i + 2])) {
                out.push((high << 4) | low);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn dsn_contains_credentials(dsn: &str) -> bool {
    dsn.find("://")
        .and_then(|start| dsn[start + 3..].find('@'))
        .is_some()
}

fn redact_dsn(dsn: &str) -> String {
    let Some(scheme_end) = dsn.find("://") else {
        return dsn.to_string();
    };
    let authority_start = scheme_end + 3;
    let Some(at_offset) = dsn[authority_start..].find('@') else {
        return dsn.to_string();
    };
    let at = authority_start + at_offset;
    format!(
        "{}<credentials>@{}",
        &dsn[..authority_start],
        &dsn[at + 1..]
    )
}

fn redact_path(path: &str) -> String {
    path.to_string()
}

fn strip_gameshark_query_params(dsn: &str) -> String {
    let Some(query_start) = dsn.find('?') else {
        return dsn.to_string();
    };
    let base = &dsn[..query_start];
    let query = &dsn[query_start + 1..];
    let kept: Vec<_> = query
        .split('&')
        .filter(|part| {
            let key = part.split_once('=').map(|(key, _)| key).unwrap_or(part);
            !matches!(key, "ssl_mode" | "key_prefix" | "ttl")
        })
        .collect();
    if kept.is_empty() {
        base.to_string()
    } else {
        format!("{base}?{}", kept.join("&"))
    }
}

fn dsn_query_param(dsn: &str, name: &str) -> Option<String> {
    let query = dsn.split_once('?')?.1;
    for part in query.split('&') {
        let (key, value) = part.split_once('=').unwrap_or((part, ""));
        if key == name {
            return Some(percent_decode_once(value));
        }
    }
    None
}

fn dsn_host(dsn: &str) -> Option<String> {
    let rest = dsn.split_once("://")?.1;
    let authority = rest.split(['/', '?']).next().unwrap_or_default();
    let host_port = authority
        .rsplit_once('@')
        .map(|(_, host)| host)
        .unwrap_or(authority);
    let host = host_port.split(':').next().unwrap_or_default();
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

fn dsn_port(dsn: &str) -> Option<u16> {
    let rest = dsn.split_once("://")?.1;
    let authority = rest.split(['/', '?']).next().unwrap_or_default();
    let host_port = authority
        .rsplit_once('@')
        .map(|(_, host)| host)
        .unwrap_or(authority);
    host_port.rsplit_once(':')?.1.parse::<u16>().ok()
}

fn dsn_database(dsn: &str) -> Option<String> {
    let rest = dsn.split_once("://")?.1;
    let path = rest.split_once('/')?.1;
    let database = path.split('?').next().unwrap_or_default();
    if database.is_empty() {
        None
    } else {
        Some(percent_decode_once(database))
    }
}

impl StorageTarget {
    fn backend_name(&self) -> &'static str {
        match self {
            Self::Sqlite { .. } => "sqlite",
            Self::Mysql { .. } => "mysql",
            Self::Redis { .. } => "redis",
        }
    }

    fn schema_mode_name(&self) -> Option<&'static str> {
        match self {
            Self::Mysql { schema_mode, .. } => Some(schema_mode.as_str()),
            _ => None,
        }
    }

    fn status_target(&self) -> serde_json::Value {
        match self {
            Self::Sqlite { path } => serde_json::json!({
                "backend": "sqlite",
                "display": path,
                "path": path,
                "host": null,
                "port": null,
                "database": null,
                "socket": null,
                "key_prefix": null
            }),
            Self::Mysql {
                display,
                host,
                port,
                database,
                socket,
                ..
            } => serde_json::json!({
                "backend": "mysql",
                "display": display,
                "path": null,
                "host": host,
                "port": port,
                "database": database,
                "socket": socket,
                "key_prefix": null
            }),
            Self::Redis {
                display,
                host,
                port,
                database,
                key_prefix,
                ..
            } => serde_json::json!({
                "backend": "redis",
                "display": display,
                "path": null,
                "host": host,
                "port": port,
                "database": database,
                "socket": null,
                "key_prefix": key_prefix
            }),
        }
    }
}

unsafe fn ffi_str(value: &GamesharkCoreStr) -> Option<String> {
    if value.ptr.is_null() {
        return None;
    }
    let bytes = slice::from_raw_parts(value.ptr as *const u8, value.len);
    Some(String::from_utf8_lossy(bytes).into_owned())
}

unsafe fn function_key_from_meta(meta: *const GamesharkCoreFunctionMeta) -> Option<FunctionKey> {
    let meta = meta.as_ref()?;
    let function_name = ffi_str(&meta.function_name)?;
    if function_name.is_empty() {
        return None;
    }

    Some(FunctionKey {
        kind: FunctionKind::from_u8(meta.kind),
        scope_name: ffi_str(&meta.scope_name).filter(|value| !value.is_empty()),
        function_name,
        file: ffi_str(&meta.file).filter(|value| !value.is_empty()),
        start_line: meta.start_line,
        end_line: meta.end_line,
    })
}

fn open_db(db_path: &str) -> Result<Connection, String> {
    let connection = Connection::open(db_path).map_err(|error| error.to_string())?;
    connection
        .busy_timeout(std::time::Duration::from_millis(5000))
        .map_err(|error| error.to_string())?;
    Ok(connection)
}

fn build_trace_filter(pattern: Option<&str>) -> TraceFilter {
    let pattern = pattern.map(str::trim).filter(|value| !value.is_empty());
    let Some(pattern) = pattern else {
        return TraceFilter {
            mode: "none".to_string(),
            allow_pattern: None,
            allow_pattern_hash: None,
            allow_pattern_valid: true,
            allow_pattern_error: None,
            regex: None,
            counters: TraceFilterCounters::default(),
        };
    };

    match Regex::new(pattern) {
        Ok(regex) => TraceFilter {
            mode: "rust_regex_v1".to_string(),
            allow_pattern: Some(pattern.to_string()),
            allow_pattern_hash: Some(fnv1a64_hex(pattern.as_bytes())),
            allow_pattern_valid: true,
            allow_pattern_error: None,
            regex: Some(regex),
            counters: TraceFilterCounters::default(),
        },
        Err(error) => TraceFilter {
            mode: "rust_regex_v1".to_string(),
            allow_pattern: Some(pattern.to_string()),
            allow_pattern_hash: Some(fnv1a64_hex(pattern.as_bytes())),
            allow_pattern_valid: false,
            allow_pattern_error: Some(error.to_string()),
            regex: None,
            counters: TraceFilterCounters::default(),
        },
    }
}

fn initialize_schema(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "
            PRAGMA foreign_keys = ON;
            CREATE TABLE IF NOT EXISTS meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS slots (
                side TEXT PRIMARY KEY CHECK (side IN ('left', 'right')),
                started_at INTEGER NOT NULL,
                finished_at INTEGER,
                status TEXT NOT NULL,
                php_version TEXT,
                sapi TEXT,
                pid INTEGER,
                script_filename TEXT
            );
            CREATE TABLE IF NOT EXISTS functions (
                function_id INTEGER PRIMARY KEY,
                identity_hash TEXT NOT NULL UNIQUE,
                kind TEXT NOT NULL,
                display_name TEXT NOT NULL,
                scope_name TEXT,
                function_name TEXT NOT NULL,
                file TEXT,
                start_line INTEGER,
                end_line INTEGER
            );
            CREATE TABLE IF NOT EXISTS function_counts (
                side TEXT NOT NULL CHECK (side IN ('left', 'right')),
                function_id INTEGER NOT NULL,
                call_count INTEGER NOT NULL,
                PRIMARY KEY (side, function_id),
                FOREIGN KEY (function_id) REFERENCES functions(function_id)
            );
            CREATE TABLE IF NOT EXISTS trace_runs (
                run_id INTEGER PRIMARY KEY,
                started_at INTEGER NOT NULL,
                finished_at INTEGER,
                status TEXT NOT NULL,
                trace_value TEXT NOT NULL,
                trace_value_kind TEXT NOT NULL,
                php_version TEXT,
                sapi TEXT,
                pid INTEGER,
                script_filename TEXT,
                trace_filter_mode TEXT NOT NULL DEFAULT 'none',
                trace_allow_pattern TEXT,
                trace_allow_pattern_hash TEXT,
                trace_allow_pattern_valid INTEGER NOT NULL DEFAULT 1,
                trace_allow_pattern_error TEXT,
                trace_filter_calls_seen INTEGER NOT NULL DEFAULT 0,
                trace_filter_calls_allowed INTEGER NOT NULL DEFAULT 0,
                trace_filter_calls_filtered_before_args INTEGER NOT NULL DEFAULT 0,
                trace_filter_args_inspected INTEGER NOT NULL DEFAULT 0,
                trace_filter_calls_with_value_matches INTEGER NOT NULL DEFAULT 0,
                trace_filter_transform_frames_started INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS trace_events (
                event_id INTEGER PRIMARY KEY,
                run_id INTEGER NOT NULL,
                event_index INTEGER NOT NULL,
                elapsed_ns INTEGER NOT NULL,
                function_id INTEGER NOT NULL,
                argument_path TEXT NOT NULL,
                zval_type TEXT NOT NULL,
                matched_value_id INTEGER NOT NULL DEFAULT 1,
                match_kind TEXT NOT NULL,
                matched_value TEXT NOT NULL,
                preview TEXT NOT NULL,
                observed_value TEXT,
                stack TEXT NOT NULL,
                stack_json TEXT NOT NULL DEFAULT '[]',
                UNIQUE(run_id, event_index),
                FOREIGN KEY (run_id) REFERENCES trace_runs(run_id),
                FOREIGN KEY (function_id) REFERENCES functions(function_id)
            );
            CREATE TABLE IF NOT EXISTS trace_transformed_values (
                transformed_id INTEGER PRIMARY KEY,
                run_id INTEGER NOT NULL,
                value_id INTEGER NOT NULL,
                parent_value_id INTEGER NOT NULL,
                elapsed_ns INTEGER NOT NULL,
                function_id INTEGER NOT NULL,
                transform_kind TEXT NOT NULL,
                value TEXT NOT NULL,
                preview TEXT NOT NULL,
                UNIQUE(run_id, value_id),
                FOREIGN KEY (run_id) REFERENCES trace_runs(run_id),
                FOREIGN KEY (function_id) REFERENCES functions(function_id)
            );
            CREATE TABLE IF NOT EXISTS unused_runs (
                run_id INTEGER PRIMARY KEY,
                started_at INTEGER NOT NULL,
                finished_at INTEGER,
                status TEXT NOT NULL,
                php_version TEXT,
                sapi TEXT,
                pid INTEGER,
                script_filename TEXT,
                request_path TEXT,
                request_uri_full TEXT,
                query_string TEXT,
                new_opcode_handler_active INTEGER NOT NULL DEFAULT 0,
                constant_opcode_handler_active INTEGER NOT NULL DEFAULT 0,
                class_constant_opcode_handler_active INTEGER NOT NULL DEFAULT 0,
                caveats_json TEXT NOT NULL DEFAULT '[]'
            );
            CREATE TABLE IF NOT EXISTS unused_declarations (
                run_id INTEGER NOT NULL,
                identity_hash TEXT NOT NULL,
                kind TEXT NOT NULL,
                display_name TEXT NOT NULL,
                scope_name TEXT,
                name TEXT NOT NULL,
                file TEXT,
                start_line INTEGER,
                end_line INTEGER,
                flags INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (run_id, identity_hash),
                FOREIGN KEY (run_id) REFERENCES unused_runs(run_id)
            );
            CREATE TABLE IF NOT EXISTS unused_accesses (
                run_id INTEGER NOT NULL,
                identity_hash TEXT NOT NULL,
                access_kind TEXT NOT NULL,
                symbol_kind TEXT NOT NULL,
                display_name TEXT NOT NULL,
                scope_name TEXT,
                name TEXT NOT NULL,
                file TEXT,
                start_line INTEGER,
                end_line INTEGER,
                access_count INTEGER NOT NULL,
                PRIMARY KEY (run_id, identity_hash, access_kind),
                FOREIGN KEY (run_id) REFERENCES unused_runs(run_id)
            );
            CREATE TABLE IF NOT EXISTS unused_included_files (
                run_id INTEGER NOT NULL,
                file TEXT NOT NULL,
                include_count INTEGER NOT NULL,
                PRIMARY KEY (run_id, file),
                FOREIGN KEY (run_id) REFERENCES unused_runs(run_id)
            );
            ",
        )
        .map_err(|error| error.to_string())?;

    if !column_exists(connection, "trace_events", "stack_json")? {
        connection
            .execute(
                "ALTER TABLE trace_events ADD COLUMN stack_json TEXT NOT NULL DEFAULT '[]'",
                [],
            )
            .map_err(|error| error.to_string())?;
    }
    if !column_exists(connection, "trace_events", "observed_value")? {
        connection
            .execute(
                "ALTER TABLE trace_events ADD COLUMN observed_value TEXT",
                [],
            )
            .map_err(|error| error.to_string())?;
    }
    if !column_exists(connection, "trace_events", "matched_value_id")? {
        connection
            .execute(
                "ALTER TABLE trace_events ADD COLUMN matched_value_id INTEGER NOT NULL DEFAULT 1",
                [],
            )
            .map_err(|error| error.to_string())?;
    }
    add_column_if_missing(
        connection,
        "trace_runs",
        "trace_filter_mode",
        "ALTER TABLE trace_runs ADD COLUMN trace_filter_mode TEXT NOT NULL DEFAULT 'none'",
    )?;
    add_column_if_missing(
        connection,
        "trace_runs",
        "trace_allow_pattern",
        "ALTER TABLE trace_runs ADD COLUMN trace_allow_pattern TEXT",
    )?;
    add_column_if_missing(
        connection,
        "trace_runs",
        "trace_allow_pattern_hash",
        "ALTER TABLE trace_runs ADD COLUMN trace_allow_pattern_hash TEXT",
    )?;
    add_column_if_missing(
        connection,
        "trace_runs",
        "trace_allow_pattern_valid",
        "ALTER TABLE trace_runs ADD COLUMN trace_allow_pattern_valid INTEGER NOT NULL DEFAULT 1",
    )?;
    add_column_if_missing(
        connection,
        "trace_runs",
        "trace_allow_pattern_error",
        "ALTER TABLE trace_runs ADD COLUMN trace_allow_pattern_error TEXT",
    )?;
    add_column_if_missing(
        connection,
        "trace_runs",
        "trace_filter_calls_seen",
        "ALTER TABLE trace_runs ADD COLUMN trace_filter_calls_seen INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(
        connection,
        "trace_runs",
        "trace_filter_calls_allowed",
        "ALTER TABLE trace_runs ADD COLUMN trace_filter_calls_allowed INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(
        connection,
        "trace_runs",
        "trace_filter_calls_filtered_before_args",
        "ALTER TABLE trace_runs ADD COLUMN trace_filter_calls_filtered_before_args INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(
        connection,
        "trace_runs",
        "trace_filter_args_inspected",
        "ALTER TABLE trace_runs ADD COLUMN trace_filter_args_inspected INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(
        connection,
        "trace_runs",
        "trace_filter_calls_with_value_matches",
        "ALTER TABLE trace_runs ADD COLUMN trace_filter_calls_with_value_matches INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(
        connection,
        "trace_runs",
        "trace_filter_transform_frames_started",
        "ALTER TABLE trace_runs ADD COLUMN trace_filter_transform_frames_started INTEGER NOT NULL DEFAULT 0",
    )?;

    Ok(())
}

fn add_column_if_missing(
    connection: &Connection,
    table: &str,
    column: &str,
    statement: &str,
) -> Result<(), String> {
    if !column_exists(connection, table, column)? {
        connection
            .execute(statement, [])
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn column_exists(connection: &Connection, table: &str, column: &str) -> Result<bool, String> {
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| error.to_string())?;

    for row in rows {
        if row.map_err(|error| error.to_string())? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn initialize_side(
    db_path: &str,
    side: &str,
    started_at: i64,
    php_version: &str,
    sapi_name: &str,
    pid: u32,
    script_filename: Option<&str>,
) -> Result<(), String> {
    let mut connection = open_db(db_path)?;
    initialize_schema(&connection)?;
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    transaction
        .execute("DELETE FROM function_counts WHERE side = ?", params![side])
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "
            INSERT INTO slots (side, started_at, finished_at, status, php_version, sapi, pid, script_filename)
            VALUES (?, ?, NULL, 'running', ?, ?, ?, ?)
            ON CONFLICT(side) DO UPDATE SET
                started_at = excluded.started_at,
                finished_at = NULL,
                status = 'running',
                php_version = excluded.php_version,
                sapi = excluded.sapi,
                pid = excluded.pid,
                script_filename = excluded.script_filename
            ",
            params![
                side,
                started_at,
                php_version,
                sapi_name,
                pid,
                script_filename
            ],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())
}

fn initialize_trace_run(
    db_path: &str,
    started_at: i64,
    trace_value: &str,
    trace_value_kind: &str,
    trace_filter: &TraceFilter,
    php_version: &str,
    sapi_name: &str,
    pid: u32,
    script_filename: Option<&str>,
) -> Result<i64, String> {
    let connection = open_db(db_path)?;
    initialize_schema(&connection)?;
    connection
        .execute(
            "
            INSERT INTO trace_runs (
                started_at, finished_at, status, trace_value, trace_value_kind,
                php_version, sapi, pid, script_filename,
                trace_filter_mode, trace_allow_pattern, trace_allow_pattern_hash,
                trace_allow_pattern_valid, trace_allow_pattern_error
            )
            VALUES (?, NULL, 'running', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ",
            params![
                started_at,
                trace_value,
                trace_value_kind,
                php_version,
                sapi_name,
                pid,
                script_filename,
                trace_filter.mode.as_str(),
                trace_filter.allow_pattern.as_deref(),
                trace_filter.allow_pattern_hash.as_deref(),
                trace_filter.allow_pattern_valid,
                trace_filter.allow_pattern_error.as_deref(),
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(connection.last_insert_rowid())
}

fn initialize_unused_run(
    db_path: &str,
    started_at: i64,
    php_version: &str,
    sapi_name: &str,
    pid: u32,
    script_filename: Option<&str>,
    request_path: Option<&str>,
    request_uri_full: Option<&str>,
    query_string: Option<&str>,
    new_opcode_handler_active: bool,
    constant_opcode_handler_active: bool,
    class_constant_opcode_handler_active: bool,
) -> Result<i64, String> {
    let connection = open_db(db_path)?;
    initialize_schema(&connection)?;
    let mut caveats = Vec::new();
    if !new_opcode_handler_active {
        caveats.push(
            "ZEND_NEW opcode handler unavailable because another extension already registered one",
        );
    }
    if !constant_opcode_handler_active {
        caveats.push("ZEND_FETCH_CONSTANT opcode handler unavailable because another extension already registered one");
    }
    if !class_constant_opcode_handler_active {
        caveats.push("ZEND_FETCH_CLASS_CONSTANT opcode handler unavailable because another extension already registered one");
    }
    let caveats_json = serde_json::to_string(&caveats).map_err(|error| error.to_string())?;
    connection
        .execute(
            "
            INSERT INTO unused_runs (
                started_at, finished_at, status, php_version, sapi, pid, script_filename,
                request_path, request_uri_full, query_string,
                new_opcode_handler_active, constant_opcode_handler_active,
                class_constant_opcode_handler_active, caveats_json
            )
            VALUES (?, NULL, 'running', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ",
            params![
                started_at,
                php_version,
                sapi_name,
                pid,
                script_filename,
                request_path,
                request_uri_full,
                query_string,
                new_opcode_handler_active,
                constant_opcode_handler_active,
                class_constant_opcode_handler_active,
                caveats_json,
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(connection.last_insert_rowid())
}

fn flush_state(state: State) -> Result<(), String> {
    match &state.storage {
        StorageTarget::Sqlite { path } => flush_state_sqlite(&state, path),
        StorageTarget::Mysql { .. } => flush_state_mysql(&state),
        StorageTarget::Redis { .. } => flush_state_redis(&state),
    }
}

fn flush_state_sqlite(state: &State, db_path: &str) -> Result<(), String> {
    let mut connection = open_db(db_path)?;
    initialize_schema(&connection)?;
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;

    if let Some(side) = state.side.as_deref() {
        flush_counts(&transaction, side, &state.counters)?;
        transaction
            .execute(
                "
                INSERT INTO slots (side, started_at, finished_at, status, php_version, sapi, pid, script_filename)
                VALUES (?, ?, ?, 'complete', ?, ?, ?, ?)
                ON CONFLICT(side) DO UPDATE SET
                    finished_at = excluded.finished_at,
                    status = 'complete',
                    php_version = excluded.php_version,
                    sapi = excluded.sapi,
                    pid = excluded.pid,
                    script_filename = excluded.script_filename
                ",
                params![
                    side,
                    state.started_at,
                    now(),
                    state.php_version.as_str(),
                    state.sapi_name.as_str(),
                    state.pid,
                    state.script_filename.as_deref()
                ],
            )
            .map_err(|error| error.to_string())?;
    }

    if let Some(run_id) = state.trace_run_id {
        flush_trace_events(&transaction, run_id, &state.trace_events)?;
        flush_transformed_values(&transaction, run_id, &state.transformed_values)?;
        transaction
            .execute(
                "
                UPDATE trace_runs
                SET finished_at = ?,
                    status = 'complete',
                    php_version = ?,
                    sapi = ?,
                    pid = ?,
                    script_filename = ?,
                    trace_filter_mode = ?,
                    trace_allow_pattern = ?,
                    trace_allow_pattern_hash = ?,
                    trace_allow_pattern_valid = ?,
                    trace_allow_pattern_error = ?,
                    trace_filter_calls_seen = ?,
                    trace_filter_calls_allowed = ?,
                    trace_filter_calls_filtered_before_args = ?,
                    trace_filter_args_inspected = ?,
                    trace_filter_calls_with_value_matches = ?,
                    trace_filter_transform_frames_started = ?
                WHERE run_id = ?
                ",
                params![
                    now_ns(),
                    state.php_version.as_str(),
                    state.sapi_name.as_str(),
                    state.pid,
                    state.script_filename.as_deref(),
                    state.trace_filter.mode.as_str(),
                    state.trace_filter.allow_pattern.as_deref(),
                    state.trace_filter.allow_pattern_hash.as_deref(),
                    state.trace_filter.allow_pattern_valid,
                    state.trace_filter.allow_pattern_error.as_deref(),
                    state.trace_filter.counters.calls_seen,
                    state.trace_filter.counters.calls_allowed,
                    state.trace_filter.counters.calls_filtered_before_args,
                    state.trace_filter.counters.args_inspected,
                    state.trace_filter.counters.calls_with_value_matches,
                    state.trace_filter.counters.transform_frames_started,
                    run_id
                ],
            )
            .map_err(|error| error.to_string())?;
    }

    if let Some(run_id) = state.unused_run_id {
        flush_unused_declarations(&transaction, run_id, &state.unused_declarations)?;
        flush_unused_accesses(&transaction, run_id, &state.unused_accesses)?;
        flush_unused_included_files(&transaction, run_id, &state.unused_included_files)?;
        let mut caveats: Vec<_> = state.unused_caveats.iter().cloned().collect();
        caveats.sort();
        let caveats_json = serde_json::to_string(&caveats).map_err(|error| error.to_string())?;
        transaction
            .execute(
                "
                UPDATE unused_runs
                SET finished_at = ?,
                    status = 'complete',
                    php_version = ?,
                    sapi = ?,
                    pid = ?,
                    script_filename = ?,
                    caveats_json = ?
                WHERE run_id = ?
                ",
                params![
                    now_ns(),
                    state.php_version.as_str(),
                    state.sapi_name.as_str(),
                    state.pid,
                    state.script_filename.as_deref(),
                    caveats_json,
                    run_id
                ],
            )
            .map_err(|error| error.to_string())?;
    }

    transaction.commit().map_err(|error| error.to_string())
}

fn diff_payload_from_state(state: &State, side: &str, finished_at: i64) -> DiffPayload {
    let mut functions: Vec<_> = state
        .counters
        .iter()
        .map(|(function, call_count)| StoredFunctionCount {
            function: function.clone(),
            call_count: *call_count,
        })
        .collect();
    functions.sort_by(|left, right| {
        display_name(&left.function)
            .cmp(&display_name(&right.function))
            .then_with(|| left.function.file.cmp(&right.function.file))
            .then_with(|| left.function.start_line.cmp(&right.function.start_line))
    });
    DiffPayload {
        run_id: finished_at,
        capture: state.capture.clone(),
        side: side.to_string(),
        started_at: state.started_at,
        finished_at,
        php_version: state.php_version.clone(),
        sapi: state.sapi_name.clone(),
        pid: state.pid,
        script_filename: state.script_filename.clone(),
        functions,
    }
}

fn trace_run_report_from_state(state: &State, run_id: i64, finished_at: i64) -> TraceRunReport {
    let transformed_values: Vec<_> = state
        .transformed_values
        .iter()
        .map(|value| TransformedValueReport {
            value_id: value.value_id,
            parent_value_id: value.parent_value_id,
            elapsed_ns: value.elapsed_ns,
            transform_kind: value.transform_kind.clone(),
            producer: display_name(&value.function),
            scope_name: value.function.scope_name.clone(),
            function_name: value.function.function_name.clone(),
            file: value.function.file.clone(),
            start_line: value.function.start_line,
            end_line: value.function.end_line,
            value: value.value.clone(),
            preview: value.preview.clone(),
        })
        .collect();
    let events: Vec<_> = state
        .trace_events
        .iter()
        .map(|event| TraceEventReport {
            event_index: event.event_index,
            elapsed_ns: event.elapsed_ns,
            kind: event.function.kind.as_str().to_string(),
            display_name: display_name(&event.function),
            scope_name: event.function.scope_name.clone(),
            function_name: event.function.function_name.clone(),
            file: event.function.file.clone(),
            start_line: event.function.start_line,
            end_line: event.function.end_line,
            argument_path: event.argument_path.clone(),
            zval_type: event.zval_type.clone(),
            matched_value_id: event.matched_value_id,
            match_kind: event.match_kind.clone(),
            matched_value: event.matched_value.clone(),
            preview: event.preview.clone(),
            observed_value: event.observed_value.clone(),
            stack: event
                .stack
                .lines()
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect(),
            stack_frames: serde_json::from_str(&event.stack_json)
                .unwrap_or_else(|_| serde_json::Value::Array(Vec::new())),
        })
        .collect();
    TraceRunReport {
        run_id,
        started_at: state.started_at,
        finished_at: Some(finished_at),
        status: "complete".to_string(),
        trace_value: state.trace_value.clone().unwrap_or_default(),
        trace_value_kind: state
            .trace_value_kind
            .clone()
            .unwrap_or_else(|| "string".to_string()),
        php_version: state.php_version.clone(),
        sapi: state.sapi_name.clone(),
        pid: state.pid,
        script_filename: state.script_filename.clone(),
        trace_filter: TraceFilterReport {
            mode: state.trace_filter.mode.clone(),
            allow_pattern: state.trace_filter.allow_pattern.clone(),
            allow_pattern_hash: state.trace_filter.allow_pattern_hash.clone(),
            allow_pattern_valid: state.trace_filter.allow_pattern_valid,
            allow_pattern_error: state.trace_filter.allow_pattern_error.clone(),
            calls_seen: state.trace_filter.counters.calls_seen,
            calls_allowed: state.trace_filter.counters.calls_allowed,
            calls_filtered_before_args: state.trace_filter.counters.calls_filtered_before_args,
            args_inspected: state.trace_filter.counters.args_inspected,
            calls_with_value_matches: state.trace_filter.counters.calls_with_value_matches,
            transform_frames_started: state.trace_filter.counters.transform_frames_started,
        },
        event_count: events.len(),
        transformed_value_count: transformed_values.len(),
        transformed_values,
        events,
    }
}

fn unused_snapshot_from_state(state: &State, run_id: i64, finished_at: i64) -> UnusedSnapshot {
    let mut caveats: Vec<_> = state.unused_caveats.iter().cloned().collect();
    caveats.sort();
    let mut declarations: Vec<_> = state.unused_declarations.values().cloned().collect();
    declarations.sort_by(|left, right| {
        left.display_name
            .cmp(&right.display_name)
            .then_with(|| left.file.cmp(&right.file))
            .then_with(|| left.start_line.cmp(&right.start_line))
    });
    let mut accesses: Vec<_> = state.unused_accesses.values().cloned().collect();
    accesses.sort_by(|left, right| {
        left.display_name
            .cmp(&right.display_name)
            .then_with(|| left.access_kind.as_str().cmp(right.access_kind.as_str()))
    });
    let mut included_files: Vec<_> = state
        .unused_included_files
        .iter()
        .map(|(file, include_count)| UnusedIncludedFile {
            file: file.clone(),
            include_count: *include_count,
        })
        .collect();
    included_files.sort_by(|left, right| left.file.cmp(&right.file));
    UnusedSnapshot {
        run: UnusedRunReport {
            run_id,
            started_at: state.started_at,
            finished_at: Some(finished_at),
            status: "complete".to_string(),
            php_version: state.php_version.clone(),
            sapi: state.sapi_name.clone(),
            pid: state.pid,
            script_filename: state.script_filename.clone(),
            request_path: state.request_path.clone(),
            request_uri_full: state.request_uri_full.clone(),
            query_string: state.query_string.clone(),
            new_opcode_handler_active: state.new_opcode_handler_active,
            constant_opcode_handler_active: state.constant_opcode_handler_active,
            class_constant_opcode_handler_active: state.class_constant_opcode_handler_active,
            caveats,
        },
        declarations,
        accesses,
        included_files,
    }
}

fn validate_storage_for_start(target: &StorageTarget) -> Result<(), StorageError> {
    match target {
        StorageTarget::Sqlite { .. } => Ok(()),
        StorageTarget::Mysql { .. } => mysql_ensure_schema(target),
        StorageTarget::Redis { .. } => redis_ping(target),
    }
}

fn flush_state_mysql(state: &State) -> Result<(), String> {
    mysql_flush_state(state).map_err(|error| error.message)
}

fn flush_state_redis(state: &State) -> Result<(), String> {
    redis_flush_state(state).map_err(|error| error.message)
}

#[cfg(feature = "backend-mysql")]
fn mysql_conn(target: &StorageTarget, report: bool) -> Result<mysql::PooledConn, StorageError> {
    let StorageTarget::Mysql {
        dsn,
        connect_timeout_ms,
        operation_timeout_ms,
        report_timeout_ms,
        ..
    } = target
    else {
        unreachable!();
    };
    let opts = mysql::Opts::from_url(dsn).map_err(|error| {
        storage_error(
            "mysql",
            "config_invalid_dsn",
            error.to_string(),
            Some("Check gameshark.dsn or gameshark.mysql.* settings."),
        )
    })?;
    let io_timeout = if report {
        *report_timeout_ms
    } else {
        *operation_timeout_ms
    };
    let builder = mysql::OptsBuilder::from_opts(opts)
        .tcp_connect_timeout(Some(Duration::from_millis(*connect_timeout_ms)))
        .read_timeout(Some(Duration::from_millis(io_timeout)))
        .write_timeout(Some(Duration::from_millis(io_timeout)));
    let pool = mysql::Pool::new(builder).map_err(|error| {
        storage_error(
            "mysql",
            "connection_failed",
            error.to_string(),
            Some("Verify the MySQL/MariaDB host, database, and credentials."),
        )
    })?;
    pool.get_conn().map_err(|error| {
        storage_error(
            "mysql",
            "connection_failed",
            error.to_string(),
            Some("Verify the MySQL/MariaDB host, database, and credentials."),
        )
    })
}

#[cfg(feature = "backend-mysql")]
fn mysql_ensure_schema(target: &StorageTarget) -> Result<(), StorageError> {
    let StorageTarget::Mysql { schema_mode, .. } = target else {
        unreachable!();
    };
    let mut conn = mysql_conn(target, false)?;
    if *schema_mode == SchemaMode::Auto {
        mysql_create_schema(&mut conn)?;
        return Ok(());
    }
    let version: Result<Option<String>, mysql::Error> =
        conn.query_first("SELECT value FROM schema_meta WHERE `key` = 'schema_version'");
    match version {
        Ok(Some(version)) if version == "1" => Ok(()),
        Ok(_) => Err(storage_error(
            "mysql",
            "schema_invalid",
            "MySQL/MariaDB Gameshark schema is missing or has an unsupported version".to_string(),
            Some("Run the documented DDL, or set gameshark.mysql.schema_mode=auto for a disposable database."),
        )),
        Err(error) => Err(storage_error(
            "mysql",
            "schema_invalid",
            error.to_string(),
            Some("Run the documented DDL, or set gameshark.mysql.schema_mode=auto for a disposable database."),
        )),
    }
}

#[cfg(not(feature = "backend-mysql"))]
fn mysql_ensure_schema(_target: &StorageTarget) -> Result<(), StorageError> {
    Err(storage_error(
        "mysql",
        "backend_not_compiled",
        "Gameshark was built without MySQL/MariaDB backend support".to_string(),
        Some("Rebuild with GAMESHARK_BACKENDS=all."),
    ))
}

#[cfg(feature = "backend-mysql")]
fn mysql_create_schema(conn: &mut mysql::PooledConn) -> Result<(), StorageError> {
    let statements = [
        "CREATE TABLE IF NOT EXISTS schema_meta (`key` VARCHAR(191) CHARACTER SET ascii COLLATE ascii_bin NOT NULL PRIMARY KEY, `value` VARCHAR(191) CHARACTER SET ascii COLLATE ascii_bin NOT NULL) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin",
        "CREATE TABLE IF NOT EXISTS diff_runs (run_id BIGINT NOT NULL PRIMARY KEY, capture VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL, side VARCHAR(5) CHARACTER SET ascii COLLATE ascii_bin NOT NULL, status VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL, started_at BIGINT NOT NULL, finished_at BIGINT NULL, php_version VARCHAR(64), sapi VARCHAR(64), pid BIGINT, script_filename TEXT, payload_json LONGTEXT NOT NULL, KEY diff_capture_side_finished (capture, side, status, finished_at, run_id)) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin",
        "CREATE TABLE IF NOT EXISTS trace_runs (run_id BIGINT NOT NULL PRIMARY KEY, capture VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL, status VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL, started_at BIGINT NOT NULL, finished_at BIGINT NULL, trace_value LONGTEXT, trace_value_kind VARCHAR(32), php_version VARCHAR(64), sapi VARCHAR(64), pid BIGINT, script_filename TEXT, payload_json LONGTEXT NOT NULL, KEY trace_capture_started (capture, status, started_at, run_id)) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin",
        "CREATE TABLE IF NOT EXISTS unused_runs (run_id BIGINT NOT NULL PRIMARY KEY, capture VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL, status VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL, started_at BIGINT NOT NULL, finished_at BIGINT NULL, php_version VARCHAR(64), sapi VARCHAR(64), pid BIGINT, script_filename TEXT, request_path TEXT, payload_json LONGTEXT NOT NULL, KEY unused_capture_finished (capture, status, finished_at, run_id)) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin",
    ];
    for statement in statements {
        conn.query_drop(statement).map_err(|error| {
            storage_error("mysql", "schema_create_failed", error.to_string(), None)
        })?;
    }
    conn.exec_drop(
        "INSERT INTO schema_meta (`key`, `value`) VALUES ('schema_version', '1') ON DUPLICATE KEY UPDATE `value` = VALUES(`value`)",
        (),
    )
    .map_err(|error| storage_error("mysql", "schema_create_failed", error.to_string(), None))
}

#[cfg(feature = "backend-mysql")]
fn mysql_flush_state(state: &State) -> Result<(), StorageError> {
    mysql_ensure_schema(&state.storage)?;
    let mut conn = mysql_conn(&state.storage, false)?;
    let finished_at = now_ns();
    let mut tx = conn
        .start_transaction(mysql::TxOpts::default())
        .map_err(|error| storage_error("mysql", "transaction_failed", error.to_string(), None))?;
    if let Some(side) = state.side.as_deref() {
        let payload = diff_payload_from_state(state, side, finished_at);
        let payload_json = serde_json::to_string(&payload).map_err(|error| {
            storage_error("mysql", "payload_encode_failed", error.to_string(), None)
        })?;
        tx.exec_drop(
            "INSERT INTO diff_runs (run_id, capture, side, status, started_at, finished_at, php_version, sapi, pid, script_filename, payload_json) VALUES (?, ?, ?, 'complete', ?, ?, ?, ?, ?, ?, ?) ON DUPLICATE KEY UPDATE status='complete', finished_at=VALUES(finished_at), payload_json=VALUES(payload_json)",
            (
                payload.run_id,
                payload.capture.as_str(),
                payload.side.as_str(),
                payload.started_at,
                payload.finished_at,
                payload.php_version.as_str(),
                payload.sapi.as_str(),
                payload.pid,
                payload.script_filename.as_deref(),
                payload_json.as_str(),
            ),
        )
        .map_err(|error| storage_error("mysql", "write_failed", error.to_string(), None))?;
    }
    if let Some(run_id) = state.trace_run_id {
        let run = trace_run_report_from_state(state, run_id, finished_at);
        let payload_json = serde_json::to_string(&run).map_err(|error| {
            storage_error("mysql", "payload_encode_failed", error.to_string(), None)
        })?;
        tx.exec_drop(
            "INSERT INTO trace_runs (run_id, capture, status, started_at, finished_at, trace_value, trace_value_kind, php_version, sapi, pid, script_filename, payload_json) VALUES (?, ?, 'complete', ?, ?, ?, ?, ?, ?, ?, ?, ?) ON DUPLICATE KEY UPDATE status='complete', finished_at=VALUES(finished_at), payload_json=VALUES(payload_json)",
            (
                run.run_id,
                state.capture.as_str(),
                run.started_at,
                run.finished_at,
                run.trace_value.as_str(),
                run.trace_value_kind.as_str(),
                run.php_version.as_str(),
                run.sapi.as_str(),
                run.pid,
                run.script_filename.as_deref(),
                payload_json.as_str(),
            ),
        )
        .map_err(|error| storage_error("mysql", "write_failed", error.to_string(), None))?;
    }
    if let Some(run_id) = state.unused_run_id {
        let snapshot = unused_snapshot_from_state(state, run_id, finished_at);
        let payload_json = serde_json::to_string(&snapshot).map_err(|error| {
            storage_error("mysql", "payload_encode_failed", error.to_string(), None)
        })?;
        tx.exec_drop(
            "INSERT INTO unused_runs (run_id, capture, status, started_at, finished_at, php_version, sapi, pid, script_filename, request_path, payload_json) VALUES (?, ?, 'complete', ?, ?, ?, ?, ?, ?, ?, ?) ON DUPLICATE KEY UPDATE status='complete', finished_at=VALUES(finished_at), payload_json=VALUES(payload_json)",
            (
                snapshot.run.run_id,
                state.capture.as_str(),
                snapshot.run.started_at,
                snapshot.run.finished_at,
                snapshot.run.php_version.as_str(),
                snapshot.run.sapi.as_str(),
                snapshot.run.pid,
                snapshot.run.script_filename.as_deref(),
                snapshot.run.request_path.as_deref(),
                payload_json.as_str(),
            ),
        )
        .map_err(|error| storage_error("mysql", "write_failed", error.to_string(), None))?;
    }
    tx.commit()
        .map_err(|error| storage_error("mysql", "transaction_failed", error.to_string(), None))
}

#[cfg(not(feature = "backend-mysql"))]
fn mysql_flush_state(_state: &State) -> Result<(), StorageError> {
    Err(storage_error(
        "mysql",
        "backend_not_compiled",
        "Gameshark was built without MySQL/MariaDB backend support".to_string(),
        Some("Rebuild with GAMESHARK_BACKENDS=all."),
    ))
}

#[cfg(feature = "backend-mysql")]
fn mysql_compare_report(
    target: &StorageTarget,
    capture: &str,
) -> Result<CompareReport, StorageError> {
    mysql_ensure_schema(target)?;
    let mut conn = mysql_conn(target, true)?;
    let left = mysql_latest_diff_payload(&mut conn, capture, "left")?;
    let right = mysql_latest_diff_payload(&mut conn, capture, "right")?;
    Ok(compare_report_from_payloads(left.as_ref(), right.as_ref()))
}

#[cfg(not(feature = "backend-mysql"))]
fn mysql_compare_report(
    _target: &StorageTarget,
    _capture: &str,
) -> Result<CompareReport, StorageError> {
    Err(storage_error(
        "mysql",
        "backend_not_compiled",
        "Gameshark was built without MySQL/MariaDB backend support".to_string(),
        Some("Rebuild with GAMESHARK_BACKENDS=all."),
    ))
}

#[cfg(feature = "backend-mysql")]
fn mysql_trace_report(target: &StorageTarget, capture: &str) -> Result<TraceReport, StorageError> {
    mysql_ensure_schema(target)?;
    let mut conn = mysql_conn(target, true)?;
    let rows: Vec<String> = conn
        .exec_map(
            "SELECT payload_json FROM trace_runs WHERE capture = ? AND status = 'complete' ORDER BY started_at, run_id",
            (capture,),
            |payload_json: String| payload_json,
        )
        .map_err(|error| storage_error("mysql", "report_failed", error.to_string(), None))?;
    let mut runs = Vec::new();
    for row in rows {
        runs.push(
            serde_json::from_str::<TraceRunReport>(&row).map_err(|error| {
                storage_error("mysql", "payload_decode_failed", error.to_string(), None)
            })?,
        );
    }
    Ok(trace_report_from_runs(runs))
}

#[cfg(not(feature = "backend-mysql"))]
fn mysql_trace_report(
    _target: &StorageTarget,
    _capture: &str,
) -> Result<TraceReport, StorageError> {
    Err(storage_error(
        "mysql",
        "backend_not_compiled",
        "Gameshark was built without MySQL/MariaDB backend support".to_string(),
        Some("Rebuild with GAMESHARK_BACKENDS=all."),
    ))
}

#[cfg(feature = "backend-mysql")]
fn mysql_unused_report(
    target: &StorageTarget,
    capture: &str,
    requested_run_id: i64,
) -> Result<UnusedReport, StorageError> {
    mysql_ensure_schema(target)?;
    let mut conn = mysql_conn(target, true)?;
    let run_count: u64 = conn
        .exec_first(
            "SELECT COUNT(*) FROM unused_runs WHERE capture = ?",
            (capture,),
        )
        .map_err(|error| storage_error("mysql", "report_failed", error.to_string(), None))?
        .unwrap_or(0);
    let payload_json: Option<String> = if requested_run_id >= 0 {
        conn.exec_first(
            "SELECT payload_json FROM unused_runs WHERE capture = ? AND run_id = ?",
            (capture, requested_run_id),
        )
    } else {
        conn.exec_first(
            "SELECT payload_json FROM unused_runs WHERE capture = ? AND status = 'complete' ORDER BY finished_at DESC, run_id DESC LIMIT 1",
            (capture,),
        )
    }
    .map_err(|error| storage_error("mysql", "report_failed", error.to_string(), None))?;
    let Some(payload_json) = payload_json else {
        return Err(storage_error(
            "mysql",
            "no_complete_run",
            "no completed unused runs recorded for capture".to_string(),
            None,
        ));
    };
    let snapshot: UnusedSnapshot = serde_json::from_str(&payload_json).map_err(|error| {
        storage_error("mysql", "payload_decode_failed", error.to_string(), None)
    })?;
    build_unused_report(
        run_count as usize,
        snapshot.run,
        snapshot.declarations,
        snapshot.accesses,
        snapshot.included_files,
    )
    .map_err(|message| storage_error("mysql", "report_failed", message, None))
}

#[cfg(feature = "backend-mysql")]
fn mysql_unused_aggregate_report(
    target: &StorageTarget,
    capture: &str,
    since_run_id: i64,
    until_run_id: i64,
) -> Result<UnusedReport, StorageError> {
    mysql_ensure_schema(target)?;
    let mut conn = mysql_conn(target, true)?;
    let mut query =
        "SELECT payload_json FROM unused_runs WHERE capture = ? AND status = 'complete'"
            .to_string();
    if since_run_id >= 0 {
        query.push_str(" AND run_id >= ");
        query.push_str(&since_run_id.to_string());
    }
    if until_run_id >= 0 {
        query.push_str(" AND run_id <= ");
        query.push_str(&until_run_id.to_string());
    }
    query.push_str(" ORDER BY run_id");
    let rows: Vec<String> = conn
        .exec_map(query, (capture,), |payload_json: String| payload_json)
        .map_err(|error| storage_error("mysql", "report_failed", error.to_string(), None))?;
    let mut snapshots = Vec::new();
    for row in rows {
        snapshots.push(
            serde_json::from_str::<UnusedSnapshot>(&row).map_err(|error| {
                storage_error("mysql", "payload_decode_failed", error.to_string(), None)
            })?,
        );
    }
    aggregate_unused_snapshots(snapshots)
        .map_err(|message| storage_error("mysql", "report_failed", message, None))
}

#[cfg(not(feature = "backend-mysql"))]
fn mysql_unused_aggregate_report(
    _target: &StorageTarget,
    _capture: &str,
    _since_run_id: i64,
    _until_run_id: i64,
) -> Result<UnusedReport, StorageError> {
    Err(storage_error(
        "mysql",
        "backend_not_compiled",
        "Gameshark was built without MySQL/MariaDB backend support".to_string(),
        Some("Rebuild with GAMESHARK_BACKENDS=all."),
    ))
}

#[cfg(not(feature = "backend-mysql"))]
fn mysql_unused_report(
    _target: &StorageTarget,
    _capture: &str,
    _requested_run_id: i64,
) -> Result<UnusedReport, StorageError> {
    Err(storage_error(
        "mysql",
        "backend_not_compiled",
        "Gameshark was built without MySQL/MariaDB backend support".to_string(),
        Some("Rebuild with GAMESHARK_BACKENDS=all."),
    ))
}

#[cfg(feature = "backend-mysql")]
fn mysql_latest_diff_payload(
    conn: &mut mysql::PooledConn,
    capture: &str,
    side: &str,
) -> Result<Option<DiffPayload>, StorageError> {
    let payload_json: Option<String> = conn
        .exec_first(
            "SELECT payload_json FROM diff_runs WHERE capture = ? AND side = ? AND status = 'complete' ORDER BY finished_at DESC, run_id DESC LIMIT 1",
            (capture, side),
        )
        .map_err(|error| storage_error("mysql", "report_failed", error.to_string(), None))?;
    payload_json
        .map(|json| {
            serde_json::from_str(&json).map_err(|error| {
                storage_error("mysql", "payload_decode_failed", error.to_string(), None)
            })
        })
        .transpose()
}

#[cfg(feature = "backend-redis")]
fn redis_conn(target: &StorageTarget) -> Result<redis::Connection, StorageError> {
    let StorageTarget::Redis {
        dsn,
        connect_timeout_ms: _,
        ..
    } = target
    else {
        unreachable!();
    };
    let client = redis::Client::open(dsn.as_str()).map_err(|error| {
        storage_error(
            "redis",
            "config_invalid_dsn",
            error.to_string(),
            Some("Check gameshark.dsn or gameshark.redis.* settings."),
        )
    })?;
    client.get_connection().map_err(|error| {
        storage_error(
            "redis",
            "connection_failed",
            error.to_string(),
            Some("Verify the Redis host, database, and credentials."),
        )
    })
}

#[cfg(feature = "backend-redis")]
fn redis_ping(target: &StorageTarget) -> Result<(), StorageError> {
    let mut conn = redis_conn(target)?;
    redis::cmd("PING")
        .query::<String>(&mut conn)
        .map(|_| ())
        .map_err(|error| storage_error("redis", "connection_failed", error.to_string(), None))
}

#[cfg(not(feature = "backend-redis"))]
fn redis_ping(_target: &StorageTarget) -> Result<(), StorageError> {
    Err(storage_error(
        "redis",
        "backend_not_compiled",
        "Gameshark was built without Redis backend support".to_string(),
        Some("Rebuild with GAMESHARK_BACKENDS=all."),
    ))
}

#[cfg(feature = "backend-redis")]
fn redis_flush_state(state: &State) -> Result<(), StorageError> {
    let StorageTarget::Redis {
        key_prefix, ttl, ..
    } = &state.storage
    else {
        unreachable!();
    };
    let mut conn = redis_conn(&state.storage)?;
    let finished_at = now_ns();
    if let Some(side) = state.side.as_deref() {
        let payload = diff_payload_from_state(state, side, finished_at);
        let payload_json = serde_json::to_string(&payload).map_err(|error| {
            storage_error("redis", "payload_encode_failed", error.to_string(), None)
        })?;
        let key = redis_diff_key(key_prefix, &state.capture, side);
        conn.set_ex::<_, _, ()>(key, payload_json, *ttl)
            .map_err(|error| storage_error("redis", "write_failed", error.to_string(), None))?;
    }
    if let Some(run_id) = state.trace_run_id {
        let run = trace_run_report_from_state(state, run_id, finished_at);
        let payload_json = serde_json::to_string(&run).map_err(|error| {
            storage_error("redis", "payload_encode_failed", error.to_string(), None)
        })?;
        let key = redis_trace_key(key_prefix, &state.capture, run_id);
        let index = redis_trace_index_key(key_prefix, &state.capture);
        conn.set_ex::<_, _, ()>(&key, payload_json, *ttl)
            .map_err(|error| storage_error("redis", "write_failed", error.to_string(), None))?;
        redis::cmd("ZADD")
            .arg(&index)
            .arg(run.started_at)
            .arg(run_id)
            .query::<()>(&mut conn)
            .map_err(|error| storage_error("redis", "write_failed", error.to_string(), None))?;
        conn.expire::<_, ()>(&index, *ttl as i64)
            .map_err(|error| storage_error("redis", "write_failed", error.to_string(), None))?;
    }
    if let Some(run_id) = state.unused_run_id {
        let snapshot = unused_snapshot_from_state(state, run_id, finished_at);
        let payload_json = serde_json::to_string(&snapshot).map_err(|error| {
            storage_error("redis", "payload_encode_failed", error.to_string(), None)
        })?;
        let key = redis_unused_key(key_prefix, &state.capture, run_id);
        let index = redis_unused_index_key(key_prefix, &state.capture);
        conn.set_ex::<_, _, ()>(&key, payload_json, *ttl)
            .map_err(|error| storage_error("redis", "write_failed", error.to_string(), None))?;
        redis::cmd("ZADD")
            .arg(&index)
            .arg(finished_at)
            .arg(run_id)
            .query::<()>(&mut conn)
            .map_err(|error| storage_error("redis", "write_failed", error.to_string(), None))?;
        conn.expire::<_, ()>(&index, *ttl as i64)
            .map_err(|error| storage_error("redis", "write_failed", error.to_string(), None))?;
    }
    Ok(())
}

#[cfg(not(feature = "backend-redis"))]
fn redis_flush_state(_state: &State) -> Result<(), StorageError> {
    Err(storage_error(
        "redis",
        "backend_not_compiled",
        "Gameshark was built without Redis backend support".to_string(),
        Some("Rebuild with GAMESHARK_BACKENDS=all."),
    ))
}

#[cfg(feature = "backend-redis")]
fn redis_compare_report(
    target: &StorageTarget,
    capture: &str,
) -> Result<CompareReport, StorageError> {
    let StorageTarget::Redis { key_prefix, .. } = target else {
        unreachable!();
    };
    let mut conn = redis_conn(target)?;
    let left =
        redis_get_json::<DiffPayload>(&mut conn, &redis_diff_key(key_prefix, capture, "left"))?;
    let right =
        redis_get_json::<DiffPayload>(&mut conn, &redis_diff_key(key_prefix, capture, "right"))?;
    Ok(compare_report_from_payloads(left.as_ref(), right.as_ref()))
}

#[cfg(not(feature = "backend-redis"))]
fn redis_compare_report(
    _target: &StorageTarget,
    _capture: &str,
) -> Result<CompareReport, StorageError> {
    Err(storage_error(
        "redis",
        "backend_not_compiled",
        "Gameshark was built without Redis backend support".to_string(),
        Some("Rebuild with GAMESHARK_BACKENDS=all."),
    ))
}

#[cfg(feature = "backend-redis")]
fn redis_trace_report(target: &StorageTarget, capture: &str) -> Result<TraceReport, StorageError> {
    let StorageTarget::Redis { key_prefix, .. } = target else {
        unreachable!();
    };
    let mut conn = redis_conn(target)?;
    let ids: Vec<i64> = conn
        .zrange(redis_trace_index_key(key_prefix, capture), 0, -1)
        .map_err(|error| storage_error("redis", "report_failed", error.to_string(), None))?;
    let mut runs = Vec::new();
    for run_id in ids {
        if let Some(run) = redis_get_json::<TraceRunReport>(
            &mut conn,
            &redis_trace_key(key_prefix, capture, run_id),
        )? {
            runs.push(run);
        }
    }
    Ok(trace_report_from_runs(runs))
}

#[cfg(not(feature = "backend-redis"))]
fn redis_trace_report(
    _target: &StorageTarget,
    _capture: &str,
) -> Result<TraceReport, StorageError> {
    Err(storage_error(
        "redis",
        "backend_not_compiled",
        "Gameshark was built without Redis backend support".to_string(),
        Some("Rebuild with GAMESHARK_BACKENDS=all."),
    ))
}

#[cfg(feature = "backend-redis")]
fn redis_unused_report(
    target: &StorageTarget,
    capture: &str,
    requested_run_id: i64,
) -> Result<UnusedReport, StorageError> {
    let StorageTarget::Redis { key_prefix, .. } = target else {
        unreachable!();
    };
    let mut conn = redis_conn(target)?;
    let ids: Vec<i64> = conn
        .zrange(redis_unused_index_key(key_prefix, capture), 0, -1)
        .map_err(|error| storage_error("redis", "report_failed", error.to_string(), None))?;
    let selected = if requested_run_id >= 0 {
        requested_run_id
    } else {
        *ids.last().ok_or_else(|| {
            storage_error(
                "redis",
                "no_complete_run",
                "no completed unused runs recorded for capture".to_string(),
                None,
            )
        })?
    };
    let key = redis_unused_key(key_prefix, capture, selected);
    let snapshot = redis_get_json::<UnusedSnapshot>(&mut conn, &key)?.ok_or_else(|| {
        storage_error(
            "redis",
            "redis_data_expired",
            "selected unused run is no longer present in Redis".to_string(),
            Some("Increase gameshark.redis.ttl or use MySQL/MariaDB for durable collection."),
        )
    })?;
    build_unused_report(
        ids.len(),
        snapshot.run,
        snapshot.declarations,
        snapshot.accesses,
        snapshot.included_files,
    )
    .map_err(|message| storage_error("redis", "report_failed", message, None))
}

#[cfg(feature = "backend-redis")]
fn redis_unused_aggregate_report(
    target: &StorageTarget,
    capture: &str,
    since_run_id: i64,
    until_run_id: i64,
) -> Result<UnusedReport, StorageError> {
    let StorageTarget::Redis { key_prefix, .. } = target else {
        unreachable!();
    };
    let mut conn = redis_conn(target)?;
    let mut ids: Vec<i64> = conn
        .zrange(redis_unused_index_key(key_prefix, capture), 0, -1)
        .map_err(|error| storage_error("redis", "report_failed", error.to_string(), None))?;
    ids.retain(|run_id| {
        (since_run_id < 0 || *run_id >= since_run_id)
            && (until_run_id < 0 || *run_id <= until_run_id)
    });
    let mut snapshots = Vec::new();
    for run_id in ids {
        if let Some(snapshot) = redis_get_json::<UnusedSnapshot>(
            &mut conn,
            &redis_unused_key(key_prefix, capture, run_id),
        )? {
            snapshots.push(snapshot);
        }
    }
    aggregate_unused_snapshots(snapshots)
        .map_err(|message| storage_error("redis", "report_failed", message, None))
}

#[cfg(not(feature = "backend-redis"))]
fn redis_unused_aggregate_report(
    _target: &StorageTarget,
    _capture: &str,
    _since_run_id: i64,
    _until_run_id: i64,
) -> Result<UnusedReport, StorageError> {
    Err(storage_error(
        "redis",
        "backend_not_compiled",
        "Gameshark was built without Redis backend support".to_string(),
        Some("Rebuild with GAMESHARK_BACKENDS=all."),
    ))
}

#[cfg(not(feature = "backend-redis"))]
fn redis_unused_report(
    _target: &StorageTarget,
    _capture: &str,
    _requested_run_id: i64,
) -> Result<UnusedReport, StorageError> {
    Err(storage_error(
        "redis",
        "backend_not_compiled",
        "Gameshark was built without Redis backend support".to_string(),
        Some("Rebuild with GAMESHARK_BACKENDS=all."),
    ))
}

#[cfg(feature = "backend-redis")]
fn redis_get_json<T: for<'de> Deserialize<'de>>(
    conn: &mut redis::Connection,
    key: &str,
) -> Result<Option<T>, StorageError> {
    let value: Option<String> = conn
        .get(key)
        .map_err(|error| storage_error("redis", "report_failed", error.to_string(), None))?;
    value
        .map(|json| {
            serde_json::from_str(&json).map_err(|error| {
                storage_error("redis", "payload_decode_failed", error.to_string(), None)
            })
        })
        .transpose()
}

fn compare_report_from_payloads(
    left: Option<&DiffPayload>,
    right: Option<&DiffPayload>,
) -> CompareReport {
    let mut rows: HashMap<FunctionKey, (u64, u64)> = HashMap::new();
    if let Some(left) = left {
        for function in &left.functions {
            rows.entry(function.function.clone()).or_default().0 = function.call_count;
        }
    }
    if let Some(right) = right {
        for function in &right.functions {
            rows.entry(function.function.clone()).or_default().1 = function.call_count;
        }
    }
    let mut report = CompareReport {
        summary: CompareSummary {
            left_total_calls: 0,
            right_total_calls: 0,
            left_function_count: 0,
            right_function_count: 0,
            changed_function_count: 0,
        },
        left_only: Vec::new(),
        right_only: Vec::new(),
        changed: Vec::new(),
        same: Vec::new(),
    };
    let mut keys: Vec<_> = rows.into_iter().collect();
    keys.sort_by(|(left, _), (right, _)| {
        display_name(left)
            .cmp(&display_name(right))
            .then_with(|| left.file.cmp(&right.file))
            .then_with(|| left.start_line.cmp(&right.start_line))
    });
    for (key, (left_count, right_count)) in keys {
        report.summary.left_total_calls += left_count;
        report.summary.right_total_calls += right_count;
        if left_count > 0 {
            report.summary.left_function_count += 1;
        }
        if right_count > 0 {
            report.summary.right_function_count += 1;
        }
        let status = if left_count > 0 && right_count == 0 {
            "left_only"
        } else if right_count > 0 && left_count == 0 {
            "right_only"
        } else if left_count != right_count {
            "changed"
        } else {
            "same"
        };
        let row = CompareRow {
            status,
            kind: key.kind.as_str().to_string(),
            display_name: display_name(&key),
            scope_name: key.scope_name,
            function_name: key.function_name,
            file: key.file,
            start_line: key.start_line,
            end_line: key.end_line,
            left_count,
            right_count,
            delta: right_count as i64 - left_count as i64,
        };
        match status {
            "left_only" => report.left_only.push(row),
            "right_only" => report.right_only.push(row),
            "changed" => {
                report.summary.changed_function_count += 1;
                report.changed.push(row);
            }
            _ => report.same.push(row),
        }
    }
    report
}

fn trace_report_from_runs(runs: Vec<TraceRunReport>) -> TraceReport {
    let event_count = runs.iter().map(|run| run.event_count).sum();
    let transformed_value_count = runs.iter().map(|run| run.transformed_value_count).sum();
    TraceReport {
        summary: TraceSummary {
            run_count: runs.len(),
            event_count,
            transformed_value_count,
        },
        runs,
    }
}

fn redis_diff_key(prefix: &str, capture: &str, side: &str) -> String {
    format!("{prefix}:v1:diff:{capture}:{side}")
}

fn redis_trace_index_key(prefix: &str, capture: &str) -> String {
    format!("{prefix}:v1:trace-index:{capture}")
}

fn redis_trace_key(prefix: &str, capture: &str, run_id: i64) -> String {
    format!("{prefix}:v1:trace:{capture}:{run_id}")
}

fn redis_unused_index_key(prefix: &str, capture: &str) -> String {
    format!("{prefix}:v1:unused-index:{capture}")
}

fn redis_unused_key(prefix: &str, capture: &str, run_id: i64) -> String {
    format!("{prefix}:v1:unused:{capture}:{run_id}")
}

fn flush_counts(
    transaction: &Transaction<'_>,
    side: &str,
    counters: &HashMap<FunctionKey, u64>,
) -> Result<(), String> {
    for (key, count) in counters {
        let function_id = upsert_function(transaction, key)?;
        transaction
            .execute(
                "
                INSERT INTO function_counts (side, function_id, call_count)
                VALUES (?, ?, ?)
                ON CONFLICT(side, function_id) DO UPDATE SET call_count = excluded.call_count
                ",
                params![side, function_id, count],
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn flush_trace_events(
    transaction: &Transaction<'_>,
    run_id: i64,
    trace_events: &[TraceEvent],
) -> Result<(), String> {
    for event in trace_events {
        let function_id = upsert_function(transaction, &event.function)?;
        transaction
            .execute(
                "
                INSERT INTO trace_events (
                    run_id, event_index, elapsed_ns, function_id, argument_path, zval_type,
                    matched_value_id, match_kind, matched_value, preview, observed_value, stack, stack_json
                )
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                ",
                params![
                    run_id,
                    event.event_index,
                    event.elapsed_ns,
                    function_id,
                    event.argument_path,
                    event.zval_type,
                    event.matched_value_id,
                    event.match_kind,
                    event.matched_value,
                    event.preview,
                    event.observed_value,
                    event.stack,
                    event.stack_json
                ],
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn flush_transformed_values(
    transaction: &Transaction<'_>,
    run_id: i64,
    transformed_values: &[TransformedValue],
) -> Result<(), String> {
    for value in transformed_values {
        let function_id = upsert_function(transaction, &value.function)?;
        transaction
            .execute(
                "
                INSERT INTO trace_transformed_values (
                    run_id, value_id, parent_value_id, elapsed_ns, function_id,
                    transform_kind, value, preview
                )
                VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(run_id, value_id) DO UPDATE SET
                    parent_value_id = excluded.parent_value_id,
                    elapsed_ns = excluded.elapsed_ns,
                    function_id = excluded.function_id,
                    transform_kind = excluded.transform_kind,
                    value = excluded.value,
                    preview = excluded.preview
                ",
                params![
                    run_id,
                    value.value_id,
                    value.parent_value_id,
                    value.elapsed_ns,
                    function_id,
                    value.transform_kind,
                    value.value,
                    value.preview
                ],
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn flush_unused_declarations(
    transaction: &Transaction<'_>,
    run_id: i64,
    declarations: &HashMap<UnusedSymbolKey, UnusedDeclaration>,
) -> Result<(), String> {
    for declaration in declarations.values() {
        let identity_hash = unused_identity_hash(&declaration.key);
        transaction
            .execute(
                "
                INSERT INTO unused_declarations (
                    run_id, identity_hash, kind, display_name, scope_name, name,
                    file, start_line, end_line, flags
                )
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(run_id, identity_hash) DO UPDATE SET
                    kind = excluded.kind,
                    display_name = excluded.display_name,
                    scope_name = excluded.scope_name,
                    name = excluded.name,
                    file = excluded.file,
                    start_line = excluded.start_line,
                    end_line = excluded.end_line,
                    flags = excluded.flags
                ",
                params![
                    run_id,
                    identity_hash,
                    declaration.key.kind.as_str(),
                    declaration.display_name,
                    declaration.scope_name,
                    declaration.name,
                    declaration.file,
                    declaration.start_line,
                    declaration.end_line,
                    declaration.flags
                ],
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn flush_unused_included_files(
    transaction: &Transaction<'_>,
    run_id: i64,
    included_files: &HashMap<String, u64>,
) -> Result<(), String> {
    for (file, include_count) in included_files {
        transaction
            .execute(
                "
                INSERT INTO unused_included_files (run_id, file, include_count)
                VALUES (?, ?, ?)
                ON CONFLICT(run_id, file) DO UPDATE SET
                    include_count = excluded.include_count
                ",
                params![run_id, file, include_count],
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn flush_unused_accesses(
    transaction: &Transaction<'_>,
    run_id: i64,
    accesses: &HashMap<(UnusedSymbolKey, UnusedAccessKind), UnusedAccess>,
) -> Result<(), String> {
    for access in accesses.values() {
        let identity_hash = unused_identity_hash(&access.key);
        transaction
            .execute(
                "
                INSERT INTO unused_accesses (
                    run_id, identity_hash, access_kind, symbol_kind, display_name,
                    scope_name, name, file, start_line, end_line, access_count
                )
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(run_id, identity_hash, access_kind) DO UPDATE SET
                    symbol_kind = excluded.symbol_kind,
                    display_name = excluded.display_name,
                    scope_name = excluded.scope_name,
                    name = excluded.name,
                    file = excluded.file,
                    start_line = excluded.start_line,
                    end_line = excluded.end_line,
                    access_count = excluded.access_count
                ",
                params![
                    run_id,
                    identity_hash,
                    access.access_kind.as_str(),
                    access.key.kind.as_str(),
                    access.display_name,
                    access.scope_name,
                    access.name,
                    access.file,
                    access.start_line,
                    access.end_line,
                    access.count
                ],
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn upsert_function(transaction: &Transaction<'_>, key: &FunctionKey) -> Result<i64, String> {
    let identity = identity_string(key);
    let identity_hash = fnv1a64_hex(identity.as_bytes());
    transaction
        .execute(
            "
            INSERT INTO functions (identity_hash, kind, display_name, scope_name, function_name, file, start_line, end_line)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(identity_hash) DO UPDATE SET
                kind = excluded.kind,
                display_name = excluded.display_name,
                scope_name = excluded.scope_name,
                function_name = excluded.function_name,
                file = excluded.file,
                start_line = excluded.start_line,
                end_line = excluded.end_line
            ",
            params![
                identity_hash,
                key.kind.as_str(),
                display_name(key),
                key.scope_name,
                key.function_name,
                key.file,
                key.start_line,
                key.end_line
            ],
        )
        .map_err(|error| error.to_string())?;
    transaction
        .query_row(
            "SELECT function_id FROM functions WHERE identity_hash = ?",
            params![identity_hash],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
}

fn compare_json(db_path: &str) -> Result<String, String> {
    serde_json::to_string(&compare_report(db_path)?).map_err(|error| error.to_string())
}

fn compare_json_for_storage(target: &StorageTarget, capture: &str) -> Result<String, StorageError> {
    serde_json::to_string(&compare_report_for_storage(target, capture)?).map_err(|error| {
        storage_error(
            target.backend_name(),
            "report_encode_failed",
            error.to_string(),
            None,
        )
    })
}

fn compare_report_for_storage(
    target: &StorageTarget,
    capture: &str,
) -> Result<CompareReport, StorageError> {
    match target {
        StorageTarget::Sqlite { path } => compare_report(path)
            .map_err(|message| storage_error("sqlite", "report_failed", message, None)),
        StorageTarget::Mysql { .. } => mysql_compare_report(target, capture),
        StorageTarget::Redis { .. } => redis_compare_report(target, capture),
    }
}

fn compare_report(db_path: &str) -> Result<CompareReport, String> {
    let connection = open_db(db_path)?;
    initialize_schema(&connection)?;
    let mut statement = connection
        .prepare(
            "
            SELECT
                f.kind,
                f.display_name,
                f.scope_name,
                f.function_name,
                f.file,
                COALESCE(f.start_line, 0),
                COALESCE(f.end_line, 0),
                COALESCE(left_counts.call_count, 0),
                COALESCE(right_counts.call_count, 0)
            FROM functions f
            LEFT JOIN function_counts left_counts
                ON left_counts.function_id = f.function_id AND left_counts.side = 'left'
            LEFT JOIN function_counts right_counts
                ON right_counts.function_id = f.function_id AND right_counts.side = 'right'
            WHERE left_counts.call_count IS NOT NULL OR right_counts.call_count IS NOT NULL
            ORDER BY f.display_name, f.file, f.start_line
            ",
        )
        .map_err(|error| error.to_string())?;

    let rows = statement
        .query_map([], |row| {
            let left_count: u64 = row.get(7)?;
            let right_count: u64 = row.get(8)?;
            let status = if left_count > 0 && right_count == 0 {
                "left_only"
            } else if right_count > 0 && left_count == 0 {
                "right_only"
            } else if left_count != right_count {
                "changed"
            } else {
                "same"
            };
            Ok(CompareRow {
                status,
                kind: row.get(0)?,
                display_name: row.get(1)?,
                scope_name: row.get(2)?,
                function_name: row.get(3)?,
                file: row.get(4)?,
                start_line: row.get(5)?,
                end_line: row.get(6)?,
                left_count,
                right_count,
                delta: right_count as i64 - left_count as i64,
            })
        })
        .map_err(|error| error.to_string())?;

    let mut report = CompareReport {
        summary: CompareSummary {
            left_total_calls: 0,
            right_total_calls: 0,
            left_function_count: 0,
            right_function_count: 0,
            changed_function_count: 0,
        },
        left_only: Vec::new(),
        right_only: Vec::new(),
        changed: Vec::new(),
        same: Vec::new(),
    };

    for row in rows {
        let row = row.map_err(|error| error.to_string())?;
        report.summary.left_total_calls += row.left_count;
        report.summary.right_total_calls += row.right_count;
        if row.left_count > 0 {
            report.summary.left_function_count += 1;
        }
        if row.right_count > 0 {
            report.summary.right_function_count += 1;
        }
        match row.status {
            "left_only" => report.left_only.push(row),
            "right_only" => report.right_only.push(row),
            "changed" => {
                report.summary.changed_function_count += 1;
                report.changed.push(row);
            }
            _ => report.same.push(row),
        }
    }

    Ok(report)
}

fn trace_report_json(db_path: &str) -> Result<String, String> {
    serde_json::to_string(&trace_report(db_path)?).map_err(|error| error.to_string())
}

fn trace_report_json_for_storage(
    target: &StorageTarget,
    capture: &str,
) -> Result<String, StorageError> {
    serde_json::to_string(&trace_report_for_storage(target, capture)?).map_err(|error| {
        storage_error(
            target.backend_name(),
            "report_encode_failed",
            error.to_string(),
            None,
        )
    })
}

fn trace_report_for_storage(
    target: &StorageTarget,
    capture: &str,
) -> Result<TraceReport, StorageError> {
    match target {
        StorageTarget::Sqlite { path } => trace_report(path)
            .map_err(|message| storage_error("sqlite", "report_failed", message, None)),
        StorageTarget::Mysql { .. } => mysql_trace_report(target, capture),
        StorageTarget::Redis { .. } => redis_trace_report(target, capture),
    }
}

fn trace_report(db_path: &str) -> Result<TraceReport, String> {
    let connection = open_db(db_path)?;
    initialize_schema(&connection)?;
    let mut run_statement = connection
        .prepare(
            "
            SELECT run_id, started_at, finished_at, status, trace_value, trace_value_kind,
                   COALESCE(php_version, ''), COALESCE(sapi, ''), COALESCE(pid, 0), script_filename,
                   COALESCE(trace_filter_mode, 'none'), trace_allow_pattern, trace_allow_pattern_hash,
                   COALESCE(trace_allow_pattern_valid, 1), trace_allow_pattern_error,
                   COALESCE(trace_filter_calls_seen, 0),
                   COALESCE(trace_filter_calls_allowed, 0),
                   COALESCE(trace_filter_calls_filtered_before_args, 0),
                   COALESCE(trace_filter_args_inspected, 0),
                   COALESCE(trace_filter_calls_with_value_matches, 0),
                   COALESCE(trace_filter_transform_frames_started, 0)
            FROM trace_runs
            ORDER BY started_at, run_id
            ",
        )
        .map_err(|error| error.to_string())?;

    let run_rows = run_statement
        .query_map([], |row| {
            Ok(TraceRunReport {
                run_id: row.get(0)?,
                started_at: row.get(1)?,
                finished_at: row.get(2)?,
                status: row.get(3)?,
                trace_value: row.get(4)?,
                trace_value_kind: row.get(5)?,
                php_version: row.get(6)?,
                sapi: row.get(7)?,
                pid: row.get(8)?,
                script_filename: row.get(9)?,
                trace_filter: TraceFilterReport {
                    mode: row.get(10)?,
                    allow_pattern: row.get(11)?,
                    allow_pattern_hash: row.get(12)?,
                    allow_pattern_valid: row.get(13)?,
                    allow_pattern_error: row.get(14)?,
                    calls_seen: row.get(15)?,
                    calls_allowed: row.get(16)?,
                    calls_filtered_before_args: row.get(17)?,
                    args_inspected: row.get(18)?,
                    calls_with_value_matches: row.get(19)?,
                    transform_frames_started: row.get(20)?,
                },
                event_count: 0,
                transformed_value_count: 0,
                transformed_values: Vec::new(),
                events: Vec::new(),
            })
        })
        .map_err(|error| error.to_string())?;

    let mut runs = Vec::new();
    let mut total_events = 0;
    let mut total_transformed_values = 0;
    for run in run_rows {
        let mut run = run.map_err(|error| error.to_string())?;
        run.transformed_values = transformed_values_for_run(&connection, run.run_id)?;
        run.transformed_value_count = run.transformed_values.len();
        run.events = trace_events_for_run(&connection, run.run_id)?;
        run.event_count = run.events.len();
        total_events += run.event_count;
        total_transformed_values += run.transformed_value_count;
        runs.push(run);
    }

    Ok(TraceReport {
        summary: TraceSummary {
            run_count: runs.len(),
            event_count: total_events,
            transformed_value_count: total_transformed_values,
        },
        runs,
    })
}

fn unused_report_json(db_path: &str, requested_run_id: i64) -> Result<String, String> {
    serde_json::to_string(&unused_report(db_path, requested_run_id)?)
        .map_err(|error| error.to_string())
}

fn unused_report_json_for_storage(
    target: &StorageTarget,
    capture: &str,
    requested_run_id: i64,
) -> Result<String, StorageError> {
    let report = unused_report_for_storage(target, capture, requested_run_id)?;
    serde_json::to_string(&report).map_err(|error| {
        storage_error(
            target.backend_name(),
            "report_encode_failed",
            error.to_string(),
            None,
        )
    })
}

fn unused_report_for_storage(
    target: &StorageTarget,
    capture: &str,
    requested_run_id: i64,
) -> Result<UnusedReport, StorageError> {
    match target {
        StorageTarget::Sqlite { path } => unused_report(path, requested_run_id)
            .map_err(|message| storage_error("sqlite", "report_failed", message, None)),
        StorageTarget::Mysql { .. } => mysql_unused_report(target, capture, requested_run_id),
        StorageTarget::Redis { .. } => redis_unused_report(target, capture, requested_run_id),
    }
}

fn unused_aggregate_report_for_storage(
    target: &StorageTarget,
    capture: &str,
    since_run_id: i64,
    until_run_id: i64,
) -> Result<UnusedReport, StorageError> {
    if since_run_id >= 0 && until_run_id >= 0 && since_run_id > until_run_id {
        return Err(storage_error(
            target.backend_name(),
            "invalid_range",
            "since_run_id must be less than or equal to until_run_id".to_string(),
            None,
        ));
    }
    match target {
        StorageTarget::Sqlite { path } => {
            sqlite_unused_aggregate_report(path, since_run_id, until_run_id)
                .map_err(|message| storage_error("sqlite", "report_failed", message, None))
        }
        StorageTarget::Mysql { .. } => {
            mysql_unused_aggregate_report(target, capture, since_run_id, until_run_id)
        }
        StorageTarget::Redis { .. } => {
            redis_unused_aggregate_report(target, capture, since_run_id, until_run_id)
        }
    }
}

fn sqlite_unused_aggregate_report(
    db_path: &str,
    since_run_id: i64,
    until_run_id: i64,
) -> Result<UnusedReport, String> {
    let connection = open_db(db_path)?;
    initialize_schema(&connection)?;
    let run_ids = sqlite_completed_unused_run_ids(&connection, since_run_id, until_run_id)?;
    let mut snapshots = Vec::new();
    for run_id in run_ids {
        snapshots.push(UnusedSnapshot {
            run: unused_run_for_id(&connection, run_id)?,
            declarations: unused_declarations_for_run(&connection, run_id)?,
            accesses: unused_accesses_for_run(&connection, run_id)?,
            included_files: unused_included_files_for_run(&connection, run_id)?,
        });
    }
    aggregate_unused_snapshots(snapshots)
}

fn sqlite_completed_unused_run_ids(
    connection: &Connection,
    since_run_id: i64,
    until_run_id: i64,
) -> Result<Vec<i64>, String> {
    let mut sql = "SELECT run_id FROM unused_runs WHERE status = 'complete'".to_string();
    if since_run_id >= 0 {
        sql.push_str(" AND run_id >= ");
        sql.push_str(&since_run_id.to_string());
    }
    if until_run_id >= 0 {
        sql.push_str(" AND run_id <= ");
        sql.push_str(&until_run_id.to_string());
    }
    sql.push_str(" ORDER BY run_id");
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| row.get::<_, i64>(0))
        .map_err(|error| error.to_string())?;
    let mut run_ids = Vec::new();
    for row in rows {
        run_ids.push(row.map_err(|error| error.to_string())?);
    }
    Ok(run_ids)
}

fn aggregate_unused_snapshots(snapshots: Vec<UnusedSnapshot>) -> Result<UnusedReport, String> {
    let run_count = snapshots.len();
    let mut declarations: HashMap<UnusedSymbolKey, UnusedDeclaration> = HashMap::new();
    let mut accesses: HashMap<(UnusedSymbolKey, UnusedAccessKind), UnusedAccess> = HashMap::new();
    let mut included_files: HashMap<String, u64> = HashMap::new();

    for snapshot in snapshots {
        for declaration in snapshot.declarations {
            declarations
                .entry(declaration.key.clone())
                .or_insert(declaration);
        }
        for access in snapshot.accesses {
            let key = (access.key.clone(), access.access_kind);
            accesses
                .entry(key)
                .and_modify(|existing| existing.count = existing.count.saturating_add(access.count))
                .or_insert(access);
        }
        for included_file in snapshot.included_files {
            *included_files.entry(included_file.file).or_insert(0) += included_file.include_count;
        }
    }

    let mut declarations: Vec<_> = declarations.into_values().collect();
    declarations.sort_by(|left, right| {
        left.display_name
            .cmp(&right.display_name)
            .then_with(|| left.file.cmp(&right.file))
            .then_with(|| left.start_line.cmp(&right.start_line))
    });
    let mut accesses: Vec<_> = accesses.into_values().collect();
    accesses.sort_by(|left, right| {
        left.display_name
            .cmp(&right.display_name)
            .then_with(|| left.access_kind.as_str().cmp(right.access_kind.as_str()))
    });
    let mut included_files: Vec<_> = included_files
        .into_iter()
        .map(|(file, include_count)| UnusedIncludedFile {
            file,
            include_count,
        })
        .collect();
    included_files.sort_by(|left, right| left.file.cmp(&right.file));

    build_unused_report_from_parts(run_count, None, declarations, accesses, included_files)
}

fn unused_report(db_path: &str, requested_run_id: i64) -> Result<UnusedReport, String> {
    let connection = open_db(db_path)?;
    initialize_schema(&connection)?;
    let run_count: usize = connection
        .query_row("SELECT COUNT(*) FROM unused_runs", [], |row| {
            row.get::<_, u64>(0)
        })
        .map_err(|error| error.to_string())? as usize;
    let run_id = select_unused_run_id(&connection, requested_run_id)?;
    let run = unused_run_for_id(&connection, run_id)?;
    let declarations = unused_declarations_for_run(&connection, run_id)?;
    let accesses = unused_accesses_for_run(&connection, run_id)?;
    let included_files = unused_included_files_for_run(&connection, run_id)?;
    build_unused_report(run_count, run, declarations, accesses, included_files)
}

fn build_unused_report(
    run_count: usize,
    run: UnusedRunReport,
    declarations: Vec<UnusedDeclaration>,
    accesses: Vec<UnusedAccess>,
    included_files: Vec<UnusedIncludedFile>,
) -> Result<UnusedReport, String> {
    build_unused_report_from_parts(run_count, Some(run), declarations, accesses, included_files)
}

fn build_unused_report_from_parts(
    run_count: usize,
    run: Option<UnusedRunReport>,
    declarations: Vec<UnusedDeclaration>,
    accesses: Vec<UnusedAccess>,
    included_files: Vec<UnusedIncludedFile>,
) -> Result<UnusedReport, String> {
    let run_id = run.as_ref().map(|run| run.run_id);
    let mut declaration_by_key = HashMap::new();
    let mut class_flags_by_name = HashMap::new();
    for declaration in &declarations {
        declaration_by_key.insert(declaration.key.clone(), declaration);
        if declaration.key.kind == UnusedSymbolKind::Class {
            class_flags_by_name.insert(declaration.key.name.clone(), declaration.flags);
        }
    }

    let mut active_files = HashSet::new();
    for access in &accesses {
        if let Some(file) = access.file.as_deref() {
            active_files.insert(file.to_string());
        }
        if let Some(declaration) = declaration_by_key.get(&access.key) {
            if let Some(file) = declaration.file.as_deref() {
                active_files.insert(file.to_string());
            }
        }
    }

    let mut access_counts: HashMap<(UnusedSymbolKey, UnusedAccessKind), u64> = HashMap::new();
    for access in &accesses {
        access_counts.insert((access.key.clone(), access.access_kind), access.count);
    }

    let mut uncalled_functions = Vec::new();
    let mut uncalled_concrete_methods = Vec::new();
    let mut classes_with_no_new_opcode_observed = Vec::new();
    let mut global_constants_without_value_access_observed = Vec::new();
    let mut class_constants_without_value_access_observed = Vec::new();

    for declaration in &declarations {
        match declaration.key.kind {
            UnusedSymbolKind::Function => {
                let call_count = access_count(
                    &access_counts,
                    &declaration.key,
                    UnusedAccessKind::FunctionCall,
                );
                if call_count == 0 {
                    uncalled_functions.push(unused_row(declaration, &access_counts, &active_files));
                }
            }
            UnusedSymbolKind::Method => {
                let call_count = access_count(
                    &access_counts,
                    &declaration.key,
                    UnusedAccessKind::MethodCall,
                );
                let owning_class_flags = declaration
                    .key
                    .scope_name
                    .as_ref()
                    .and_then(|scope| class_flags_by_name.get(scope))
                    .copied()
                    .unwrap_or(0);
                if call_count == 0
                    && declaration.flags & ZEND_ACC_ABSTRACT_FLAG == 0
                    && owning_class_flags & ZEND_ACC_UNINSTANTIABLE_FLAGS == 0
                {
                    uncalled_concrete_methods.push(unused_row(
                        declaration,
                        &access_counts,
                        &active_files,
                    ));
                }
            }
            UnusedSymbolKind::Class => {
                let new_count = access_count(
                    &access_counts,
                    &declaration.key,
                    UnusedAccessKind::NewOpcodeObserved,
                );
                if new_count == 0 && declaration.flags & ZEND_ACC_UNINSTANTIABLE_FLAGS == 0 {
                    classes_with_no_new_opcode_observed.push(unused_row(
                        declaration,
                        &access_counts,
                        &active_files,
                    ));
                }
            }
            UnusedSymbolKind::GlobalConstant => {
                if constant_value_access_count(&access_counts, &declaration.key) == 0 {
                    global_constants_without_value_access_observed.push(unused_row(
                        declaration,
                        &access_counts,
                        &active_files,
                    ));
                }
            }
            UnusedSymbolKind::ClassConstant => {
                if constant_value_access_count(&access_counts, &declaration.key) == 0 {
                    class_constants_without_value_access_observed.push(unused_row(
                        declaration,
                        &access_counts,
                        &active_files,
                    ));
                }
            }
            UnusedSymbolKind::Closure => {}
        }
    }

    sort_unused_rows(&mut uncalled_functions);
    sort_unused_rows(&mut uncalled_concrete_methods);
    sort_unused_rows(&mut classes_with_no_new_opcode_observed);
    sort_unused_rows(&mut global_constants_without_value_access_observed);
    sort_unused_rows(&mut class_constants_without_value_access_observed);

    let (included_files_with_no_accessed_declarations, included_files_without_declarations) =
        included_file_reports(&included_files, &declarations, &access_counts);
    let global_constants_without_read_observed =
        global_constants_without_value_access_observed.clone();
    let class_constants_without_read_observed =
        class_constants_without_value_access_observed.clone();

    Ok(UnusedReport {
        summary: UnusedSummary {
            run_count,
            run_id,
            declaration_count: declarations.len(),
            access_count: accesses.len(),
            uncalled_function_count: uncalled_functions.len(),
            uncalled_concrete_method_count: uncalled_concrete_methods.len(),
            class_without_new_count: classes_with_no_new_opcode_observed.len(),
            global_constant_without_value_access_count:
                global_constants_without_value_access_observed.len(),
            class_constant_without_value_access_count:
                class_constants_without_value_access_observed.len(),
            global_constant_without_read_count: global_constants_without_read_observed.len(),
            class_constant_without_read_count: class_constants_without_read_observed.len(),
            included_file_count: included_files.len(),
            included_file_with_no_accessed_declaration_count:
                included_files_with_no_accessed_declarations.len(),
            included_file_without_declaration_count: included_files_without_declarations.len(),
        },
        run,
        uncalled_functions,
        uncalled_concrete_methods,
        classes_with_no_new_opcode_observed,
        global_constants_without_value_access_observed,
        class_constants_without_value_access_observed,
        global_constants_without_read_observed,
        class_constants_without_read_observed,
        included_files_with_no_accessed_declarations,
        included_files_without_declarations,
    })
}

const ZEND_ACC_ABSTRACT_FLAG: u32 = 1 << 6;
const ZEND_ACC_INTERFACE_FLAG: u32 = 1 << 0;
const ZEND_ACC_TRAIT_FLAG: u32 = 1 << 1;
const ZEND_ACC_IMPLICIT_ABSTRACT_CLASS_FLAG: u32 = 1 << 4;
const ZEND_ACC_EXPLICIT_ABSTRACT_CLASS_FLAG: u32 = 1 << 6;
const ZEND_ACC_ENUM_FLAG: u32 = 1 << 28;
const ZEND_ACC_UNINSTANTIABLE_FLAGS: u32 = ZEND_ACC_INTERFACE_FLAG
    | ZEND_ACC_TRAIT_FLAG
    | ZEND_ACC_IMPLICIT_ABSTRACT_CLASS_FLAG
    | ZEND_ACC_EXPLICIT_ABSTRACT_CLASS_FLAG
    | ZEND_ACC_ENUM_FLAG;

fn select_unused_run_id(connection: &Connection, requested_run_id: i64) -> Result<i64, String> {
    if requested_run_id >= 0 {
        let found = connection
            .query_row(
                "SELECT run_id FROM unused_runs WHERE run_id = ?",
                params![requested_run_id],
                |row| row.get(0),
            )
            .map_err(|error| {
                if matches!(error, rusqlite::Error::QueryReturnedNoRows) {
                    format!("unused run {requested_run_id} was not found")
                } else {
                    error.to_string()
                }
            })?;
        return Ok(found);
    }

    connection
        .query_row(
            "
            SELECT run_id
            FROM unused_runs
            WHERE status = 'complete'
            ORDER BY COALESCE(finished_at, started_at) DESC, run_id DESC
            LIMIT 1
            ",
            [],
            |row| row.get(0),
        )
        .map_err(|error| {
            if matches!(error, rusqlite::Error::QueryReturnedNoRows) {
                "no completed unused runs recorded".to_string()
            } else {
                error.to_string()
            }
        })
}

fn unused_run_for_id(connection: &Connection, run_id: i64) -> Result<UnusedRunReport, String> {
    connection
        .query_row(
            "
            SELECT run_id, started_at, finished_at, status,
                   COALESCE(php_version, ''), COALESCE(sapi, ''), COALESCE(pid, 0),
                   script_filename, request_path, request_uri_full, query_string,
                   COALESCE(new_opcode_handler_active, 0),
                   COALESCE(constant_opcode_handler_active, 0),
                   COALESCE(class_constant_opcode_handler_active, 0),
                   COALESCE(caveats_json, '[]')
            FROM unused_runs
            WHERE run_id = ?
            ",
            params![run_id],
            |row| {
                let caveats_json: String = row.get(14)?;
                let caveats = serde_json::from_str(&caveats_json).unwrap_or_default();
                Ok(UnusedRunReport {
                    run_id: row.get(0)?,
                    started_at: row.get(1)?,
                    finished_at: row.get(2)?,
                    status: row.get(3)?,
                    php_version: row.get(4)?,
                    sapi: row.get(5)?,
                    pid: row.get(6)?,
                    script_filename: row.get(7)?,
                    request_path: row.get(8)?,
                    request_uri_full: row.get(9)?,
                    query_string: row.get(10)?,
                    new_opcode_handler_active: row.get::<_, bool>(11)?,
                    constant_opcode_handler_active: row.get::<_, bool>(12)?,
                    class_constant_opcode_handler_active: row.get::<_, bool>(13)?,
                    caveats,
                })
            },
        )
        .map_err(|error| error.to_string())
}

fn unused_declarations_for_run(
    connection: &Connection,
    run_id: i64,
) -> Result<Vec<UnusedDeclaration>, String> {
    let mut statement = connection
        .prepare(
            "
            SELECT kind, display_name, scope_name, name, file,
                   COALESCE(start_line, 0), COALESCE(end_line, 0), COALESCE(flags, 0)
            FROM unused_declarations
            WHERE run_id = ?
            ORDER BY display_name, file, start_line
            ",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![run_id], |row| {
            let kind_string: String = row.get(0)?;
            let kind =
                unused_symbol_kind_from_str(&kind_string).unwrap_or(UnusedSymbolKind::Function);
            let scope_name: Option<String> = row.get(2)?;
            let name: String = row.get(3)?;
            let key = unused_symbol_key(kind.clone(), scope_name.as_deref(), &name);
            Ok(UnusedDeclaration {
                key,
                display_name: row.get(1)?,
                scope_name,
                name,
                file: row.get(4)?,
                start_line: row.get(5)?,
                end_line: row.get(6)?,
                flags: row.get(7)?,
            })
        })
        .map_err(|error| error.to_string())?;
    let mut declarations = Vec::new();
    for row in rows {
        declarations.push(row.map_err(|error| error.to_string())?);
    }
    Ok(declarations)
}

fn unused_accesses_for_run(
    connection: &Connection,
    run_id: i64,
) -> Result<Vec<UnusedAccess>, String> {
    let mut statement = connection
        .prepare(
            "
            SELECT access_kind, symbol_kind, display_name, scope_name, name, file,
                   COALESCE(start_line, 0), COALESCE(end_line, 0), access_count
            FROM unused_accesses
            WHERE run_id = ?
            ORDER BY display_name, access_kind
            ",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![run_id], |row| {
            let access_kind_string: String = row.get(0)?;
            let symbol_kind_string: String = row.get(1)?;
            let access_kind = unused_access_kind_from_str(&access_kind_string)
                .unwrap_or(UnusedAccessKind::FunctionCall);
            let symbol_kind = unused_symbol_kind_from_str(&symbol_kind_string)
                .unwrap_or_else(|| access_kind.symbol_kind());
            let scope_name: Option<String> = row.get(3)?;
            let name: String = row.get(4)?;
            let key = unused_symbol_key(symbol_kind, scope_name.as_deref(), &name);
            Ok(UnusedAccess {
                key,
                access_kind,
                display_name: row.get(2)?,
                scope_name,
                name,
                file: row.get(5)?,
                start_line: row.get(6)?,
                end_line: row.get(7)?,
                count: row.get(8)?,
            })
        })
        .map_err(|error| error.to_string())?;
    let mut accesses = Vec::new();
    for row in rows {
        accesses.push(row.map_err(|error| error.to_string())?);
    }
    Ok(accesses)
}

fn unused_included_files_for_run(
    connection: &Connection,
    run_id: i64,
) -> Result<Vec<UnusedIncludedFile>, String> {
    let mut statement = connection
        .prepare(
            "
            SELECT file, include_count
            FROM unused_included_files
            WHERE run_id = ?
            ORDER BY file
            ",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![run_id], |row| {
            Ok(UnusedIncludedFile {
                file: row.get(0)?,
                include_count: row.get(1)?,
            })
        })
        .map_err(|error| error.to_string())?;
    let mut included_files = Vec::new();
    for row in rows {
        included_files.push(row.map_err(|error| error.to_string())?);
    }
    Ok(included_files)
}

fn unused_symbol_kind_from_str(value: &str) -> Option<UnusedSymbolKind> {
    match value {
        "function" => Some(UnusedSymbolKind::Function),
        "method" => Some(UnusedSymbolKind::Method),
        "closure" => Some(UnusedSymbolKind::Closure),
        "class" => Some(UnusedSymbolKind::Class),
        "global_constant" => Some(UnusedSymbolKind::GlobalConstant),
        "class_constant" => Some(UnusedSymbolKind::ClassConstant),
        _ => None,
    }
}

fn unused_access_kind_from_str(value: &str) -> Option<UnusedAccessKind> {
    match value {
        "function_call" => Some(UnusedAccessKind::FunctionCall),
        "method_call" => Some(UnusedAccessKind::MethodCall),
        "closure_call" => Some(UnusedAccessKind::ClosureCall),
        "new_opcode_observed" => Some(UnusedAccessKind::NewOpcodeObserved),
        "global_constant_fetch_observed" => Some(UnusedAccessKind::GlobalConstantFetchObserved),
        "class_constant_fetch_observed" => Some(UnusedAccessKind::ClassConstantFetchObserved),
        "global_constant_read" => Some(UnusedAccessKind::GlobalConstantRead),
        "class_constant_read" => Some(UnusedAccessKind::ClassConstantRead),
        "global_constant_probe" => Some(UnusedAccessKind::GlobalConstantProbe),
        "class_constant_probe" => Some(UnusedAccessKind::ClassConstantProbe),
        _ => None,
    }
}

fn access_count(
    access_counts: &HashMap<(UnusedSymbolKey, UnusedAccessKind), u64>,
    key: &UnusedSymbolKey,
    access_kind: UnusedAccessKind,
) -> u64 {
    access_counts
        .get(&(key.clone(), access_kind))
        .copied()
        .unwrap_or(0)
}

fn constant_value_access_count(
    access_counts: &HashMap<(UnusedSymbolKey, UnusedAccessKind), u64>,
    key: &UnusedSymbolKey,
) -> u64 {
    match key.kind {
        UnusedSymbolKind::GlobalConstant => {
            access_count(
                access_counts,
                key,
                UnusedAccessKind::GlobalConstantFetchObserved,
            ) + access_count(access_counts, key, UnusedAccessKind::GlobalConstantRead)
        }
        UnusedSymbolKind::ClassConstant => {
            access_count(
                access_counts,
                key,
                UnusedAccessKind::ClassConstantFetchObserved,
            ) + access_count(access_counts, key, UnusedAccessKind::ClassConstantRead)
        }
        _ => 0,
    }
}

fn declaration_access_count(
    access_counts: &HashMap<(UnusedSymbolKey, UnusedAccessKind), u64>,
    declaration: &UnusedDeclaration,
) -> u64 {
    match declaration.key.kind {
        UnusedSymbolKind::Function => access_count(
            access_counts,
            &declaration.key,
            UnusedAccessKind::FunctionCall,
        ),
        UnusedSymbolKind::Method => access_count(
            access_counts,
            &declaration.key,
            UnusedAccessKind::MethodCall,
        ),
        UnusedSymbolKind::Class => access_count(
            access_counts,
            &declaration.key,
            UnusedAccessKind::NewOpcodeObserved,
        ),
        UnusedSymbolKind::GlobalConstant | UnusedSymbolKind::ClassConstant => {
            constant_value_access_count(access_counts, &declaration.key)
        }
        UnusedSymbolKind::Closure => 0,
    }
}

fn included_file_reports(
    included_files: &[UnusedIncludedFile],
    declarations: &[UnusedDeclaration],
    access_counts: &HashMap<(UnusedSymbolKey, UnusedAccessKind), u64>,
) -> (Vec<UnusedIncludedFileReport>, Vec<UnusedIncludedFileReport>) {
    let mut declarations_by_file: HashMap<&str, Vec<&UnusedDeclaration>> = HashMap::new();
    for declaration in declarations {
        if let Some(file) = declaration.file.as_deref() {
            declarations_by_file
                .entry(file)
                .or_default()
                .push(declaration);
        }
    }

    let mut no_accessed_declarations = Vec::new();
    let mut without_declarations = Vec::new();
    for included_file in included_files {
        let file_declarations = declarations_by_file
            .get(included_file.file.as_str())
            .map(|items| items.as_slice())
            .unwrap_or(&[]);
        let mut report = UnusedIncludedFileReport {
            file: included_file.file.clone(),
            include_count: included_file.include_count,
            declaration_count: file_declarations.len(),
            accessed_declaration_count: 0,
            function_declaration_count: 0,
            method_declaration_count: 0,
            class_declaration_count: 0,
            global_constant_declaration_count: 0,
            class_constant_declaration_count: 0,
        };

        for declaration in file_declarations {
            match declaration.key.kind {
                UnusedSymbolKind::Function => report.function_declaration_count += 1,
                UnusedSymbolKind::Method => report.method_declaration_count += 1,
                UnusedSymbolKind::Class => report.class_declaration_count += 1,
                UnusedSymbolKind::GlobalConstant => report.global_constant_declaration_count += 1,
                UnusedSymbolKind::ClassConstant => report.class_constant_declaration_count += 1,
                UnusedSymbolKind::Closure => {}
            }
            if declaration_access_count(access_counts, declaration) > 0 {
                report.accessed_declaration_count += 1;
            }
        }

        if report.declaration_count == 0 {
            without_declarations.push(report);
        } else if report.accessed_declaration_count == 0 {
            no_accessed_declarations.push(report);
        }
    }

    sort_included_file_rows(&mut no_accessed_declarations);
    sort_included_file_rows(&mut without_declarations);
    (no_accessed_declarations, without_declarations)
}

fn unused_row(
    declaration: &UnusedDeclaration,
    access_counts: &HashMap<(UnusedSymbolKey, UnusedAccessKind), u64>,
    active_files: &HashSet<String>,
) -> UnusedReportRow {
    let file_had_any_access = declaration
        .file
        .as_deref()
        .map(|file| active_files.contains(file));
    UnusedReportRow {
        kind: declaration.key.kind.as_str().to_string(),
        display_name: declaration.display_name.clone(),
        scope_name: declaration.scope_name.clone(),
        name: declaration.name.clone(),
        file: declaration.file.clone(),
        start_line: declaration.start_line,
        end_line: declaration.end_line,
        flags: declaration.flags,
        call_count: access_count(
            access_counts,
            &declaration.key,
            UnusedAccessKind::FunctionCall,
        ) + access_count(
            access_counts,
            &declaration.key,
            UnusedAccessKind::MethodCall,
        ),
        new_opcode_observed_count: access_count(
            access_counts,
            &declaration.key,
            UnusedAccessKind::NewOpcodeObserved,
        ),
        fetch_observed_count: access_count(
            access_counts,
            &declaration.key,
            UnusedAccessKind::GlobalConstantFetchObserved,
        ) + access_count(
            access_counts,
            &declaration.key,
            UnusedAccessKind::ClassConstantFetchObserved,
        ),
        read_observed_count: access_count(
            access_counts,
            &declaration.key,
            UnusedAccessKind::GlobalConstantRead,
        ) + access_count(
            access_counts,
            &declaration.key,
            UnusedAccessKind::ClassConstantRead,
        ),
        defined_probe_count: access_count(
            access_counts,
            &declaration.key,
            UnusedAccessKind::GlobalConstantProbe,
        ) + access_count(
            access_counts,
            &declaration.key,
            UnusedAccessKind::ClassConstantProbe,
        ),
        file_had_any_access,
    }
}

fn sort_unused_rows(rows: &mut [UnusedReportRow]) {
    rows.sort_by(|left, right| {
        left.display_name
            .cmp(&right.display_name)
            .then_with(|| left.file.cmp(&right.file))
            .then_with(|| left.start_line.cmp(&right.start_line))
    });
}

fn sort_included_file_rows(rows: &mut [UnusedIncludedFileReport]) {
    rows.sort_by(|left, right| left.file.cmp(&right.file));
}

fn transformed_values_for_run(
    connection: &Connection,
    run_id: i64,
) -> Result<Vec<TransformedValueReport>, String> {
    let mut statement = connection
        .prepare(
            "
            SELECT
                t.value_id,
                t.parent_value_id,
                t.elapsed_ns,
                t.transform_kind,
                f.display_name,
                f.scope_name,
                f.function_name,
                f.file,
                COALESCE(f.start_line, 0),
                COALESCE(f.end_line, 0),
                t.value,
                t.preview
            FROM trace_transformed_values t
            JOIN functions f ON f.function_id = t.function_id
            WHERE t.run_id = ?
            ORDER BY t.value_id
            ",
        )
        .map_err(|error| error.to_string())?;

    let rows = statement
        .query_map(params![run_id], |row| {
            Ok(TransformedValueReport {
                value_id: row.get(0)?,
                parent_value_id: row.get(1)?,
                elapsed_ns: row.get(2)?,
                transform_kind: row.get(3)?,
                producer: row.get(4)?,
                scope_name: row.get(5)?,
                function_name: row.get(6)?,
                file: row.get(7)?,
                start_line: row.get(8)?,
                end_line: row.get(9)?,
                value: row.get(10)?,
                preview: row.get(11)?,
            })
        })
        .map_err(|error| error.to_string())?;

    let mut values = Vec::new();
    for row in rows {
        values.push(row.map_err(|error| error.to_string())?);
    }
    Ok(values)
}

fn trace_events_for_run(
    connection: &Connection,
    run_id: i64,
) -> Result<Vec<TraceEventReport>, String> {
    let mut statement = connection
        .prepare(
            "
            SELECT
                e.event_index,
                e.elapsed_ns,
                f.kind,
                f.display_name,
                f.scope_name,
                f.function_name,
                f.file,
                COALESCE(f.start_line, 0),
                COALESCE(f.end_line, 0),
                e.argument_path,
                e.zval_type,
                e.matched_value_id,
                e.match_kind,
                e.matched_value,
                e.preview,
                COALESCE(e.observed_value, e.preview),
                e.stack,
                e.stack_json
            FROM trace_events e
            JOIN functions f ON f.function_id = e.function_id
            WHERE e.run_id = ?
            ORDER BY e.event_index
            ",
        )
        .map_err(|error| error.to_string())?;

    let rows = statement
        .query_map(params![run_id], |row| {
            let stack: String = row.get(16)?;
            let stack_json: String = row.get(17)?;
            let stack_frames = serde_json::from_str(&stack_json)
                .unwrap_or_else(|_| serde_json::Value::Array(Vec::new()));
            Ok(TraceEventReport {
                event_index: row.get(0)?,
                elapsed_ns: row.get(1)?,
                kind: row.get(2)?,
                display_name: row.get(3)?,
                scope_name: row.get(4)?,
                function_name: row.get(5)?,
                file: row.get(6)?,
                start_line: row.get(7)?,
                end_line: row.get(8)?,
                argument_path: row.get(9)?,
                zval_type: row.get(10)?,
                matched_value_id: row.get(11)?,
                match_kind: row.get(12)?,
                matched_value: row.get(13)?,
                preview: row.get(14)?,
                observed_value: row.get(15)?,
                stack: stack
                    .lines()
                    .filter(|line| !line.is_empty())
                    .map(str::to_string)
                    .collect(),
                stack_frames,
            })
        })
        .map_err(|error| error.to_string())?;

    let mut events = Vec::new();
    for row in rows {
        events.push(row.map_err(|error| error.to_string())?);
    }
    Ok(events)
}

fn render_compare_text(report: &CompareReport, color: bool) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{}", ansi(color, "1", "Gameshark compare report"));
    let _ = writeln!(
        out,
        "left calls: {} | right calls: {} | left functions: {} | right functions: {} | changed: {}",
        report.summary.left_total_calls,
        report.summary.right_total_calls,
        report.summary.left_function_count,
        report.summary.right_function_count,
        report.summary.changed_function_count
    );
    let _ = writeln!(
        out,
        "left-only: {} | right-only: {} | same: {}",
        report.left_only.len(),
        report.right_only.len(),
        report.same.len()
    );

    if report.left_only.is_empty() && report.right_only.is_empty() && report.changed.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "No differential function calls recorded.");
        return out;
    }

    render_compare_section(&mut out, "Left only", &report.left_only, color, "31");
    render_compare_section(&mut out, "Right only", &report.right_only, color, "32");
    render_compare_section(&mut out, "Changed", &report.changed, color, "33");
    out
}

fn render_compare_section(
    out: &mut String,
    title: &str,
    rows: &[CompareRow],
    color: bool,
    title_color: &str,
) {
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "{}",
        ansi(color, title_color, &format!("{title} ({})", rows.len()))
    );

    if rows.is_empty() {
        let _ = writeln!(out, "  none");
        return;
    }

    let shown = rows.len().min(50);
    for row in rows.iter().take(shown) {
        let location = format_location(row.file.as_deref(), row.start_line);
        let _ = writeln!(
            out,
            "  {} left={} right={} delta={} {}",
            ansi(color, "36", &row.display_name),
            row.left_count,
            row.right_count,
            format_delta(row.delta, color),
            location
        );
    }
    if rows.len() > shown {
        let _ = writeln!(out, "  ... {} more", rows.len() - shown);
    }
}

fn render_trace_text(report: &TraceReport, color: bool) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{}", ansi(color, "1", "Gameshark trace report"));
    let _ = writeln!(
        out,
        "runs: {} | events: {} | transformed values: {}",
        report.summary.run_count,
        report.summary.event_count,
        report.summary.transformed_value_count
    );

    if report.runs.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "No trace runs recorded.");
        return out;
    }

    for run in &report.runs {
        let path_base = trace_path_base(run);
        let script = run
            .script_filename
            .as_deref()
            .map(|path| display_path(path, path_base.as_deref()))
            .unwrap_or_else(|| "[unknown]".to_string());
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "{} trace={} status={} events={} script={}",
            ansi(color, "1", &format!("Run #{}", run.run_id)),
            highlight_value(&run.trace_value, &run.trace_value, 160, color),
            color_status(&run.status, color),
            run.event_count,
            script
        );
        if let Some(path_base) = path_base.as_deref() {
            let _ = writeln!(out, "  base={}", ansi(color, "2", path_base));
        }
        if run.trace_filter.mode != "none" {
            let pattern = run
                .trace_filter
                .allow_pattern
                .as_deref()
                .unwrap_or("[none]");
            let validity = if run.trace_filter.allow_pattern_valid {
                ansi(color, "32", "valid")
            } else {
                ansi(color, "31", "invalid")
            };
            let _ = writeln!(
                out,
                "  filter={} pattern={} {} seen={} allowed={} filtered={} inspected={} matches={} transforms={}",
                ansi(color, "33", &run.trace_filter.mode),
                ansi(color, "36", pattern),
                validity,
                run.trace_filter.calls_seen,
                run.trace_filter.calls_allowed,
                run.trace_filter.calls_filtered_before_args,
                run.trace_filter.args_inspected,
                run.trace_filter.calls_with_value_matches,
                run.trace_filter.transform_frames_started
            );
            if let Some(error) = run.trace_filter.allow_pattern_error.as_deref() {
                let _ = writeln!(out, "  filter_error={}", ansi(color, "31", error));
            }
        }

        if !run.transformed_values.is_empty() {
            let _ = writeln!(
                out,
                "  Followed transformed values ({})",
                run.transformed_values.len()
            );
            for value in &run.transformed_values {
                let elapsed_ms = value.elapsed_ns as f64 / 1_000_000.0;
                let _ = writeln!(
                    out,
                    "    #{} <- #{} +{elapsed_ms:.3}ms {} {} via {}",
                    value.value_id,
                    value.parent_value_id,
                    ansi(color, "33", &value.transform_kind),
                    highlight_value(&value.value, &value.value, 160, color),
                    color_function_name(&value.producer, color)
                );
            }
        }

        if run.events.is_empty() {
            let _ = writeln!(out, "  no trace events");
            continue;
        }

        let shown = run.events.len().min(50);
        let _ = writeln!(out, "  Events (showing {shown} of {})", run.events.len());
        for event in run.events.iter().take(shown) {
            render_trace_event(
                &mut out,
                event,
                &run.trace_value,
                path_base.as_deref(),
                color,
            );
        }
        if run.events.len() > shown {
            let _ = writeln!(out, "  ... {} more events", run.events.len() - shown);
        }
    }

    out
}

fn render_unused_text(report: &UnusedReport, color: bool) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{}",
        ansi(color, "1", "Gameshark unused coverage report")
    );
    let _ = writeln!(
        out,
        "runs: {} | selected: {} | declarations: {} | access rows: {}",
        report.summary.run_count,
        report
            .summary
            .run_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "[none]".to_string()),
        report.summary.declaration_count,
        report.summary.access_count
    );
    let _ = writeln!(
        out,
        "uncalled functions: {} | uncalled concrete methods: {} | classes without new: {} | constants without value access: {}/{} | included files: {}",
        report.summary.uncalled_function_count,
        report.summary.uncalled_concrete_method_count,
        report.summary.class_without_new_count,
        report.summary.global_constant_without_value_access_count,
        report.summary.class_constant_without_value_access_count,
        report.summary.included_file_count
    );
    let _ = writeln!(
        out,
        "{}",
        ansi(
            color,
            "33",
            "Caveat: no observed access in one runtime run is a coverage signal, not proof of dead code."
        )
    );

    if let Some(run) = &report.run {
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "{} status={} sapi={} script={}",
            ansi(color, "1", &format!("Run #{}", run.run_id)),
            color_status(&run.status, color),
            run.sapi,
            run.script_filename.as_deref().unwrap_or("[unknown]")
        );
        if let Some(path) = run.request_path.as_deref() {
            let _ = writeln!(out, "  request_path={}", ansi(color, "36", path));
        }
        if !run.caveats.is_empty() {
            let _ = writeln!(out, "  caveats:");
            for caveat in &run.caveats {
                let _ = writeln!(out, "    {}", ansi(color, "33", caveat));
            }
        }
    }

    render_unused_section(
        &mut out,
        "Uncalled functions",
        &report.uncalled_functions,
        color,
    );
    render_unused_section(
        &mut out,
        "Uncalled concrete methods",
        &report.uncalled_concrete_methods,
        color,
    );
    render_unused_section(
        &mut out,
        "Classes with no new opcode observed",
        &report.classes_with_no_new_opcode_observed,
        color,
    );
    render_unused_section(
        &mut out,
        "Global constants without value access observed",
        &report.global_constants_without_value_access_observed,
        color,
    );
    render_unused_section(
        &mut out,
        "Class constants without value access observed",
        &report.class_constants_without_value_access_observed,
        color,
    );
    render_included_file_section(
        &mut out,
        "Included files with no accessed declarations",
        &report.included_files_with_no_accessed_declarations,
        color,
    );
    render_included_file_section(
        &mut out,
        "Included files without declarations",
        &report.included_files_without_declarations,
        color,
    );

    out
}

fn render_unused_section(out: &mut String, title: &str, rows: &[UnusedReportRow], color: bool) {
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "{}",
        ansi(color, "36", &format!("{title} ({})", rows.len()))
    );
    if rows.is_empty() {
        let _ = writeln!(out, "  none");
        return;
    }

    let shown = rows.len().min(50);
    for row in rows.iter().take(shown) {
        let location = format_location(row.file.as_deref(), row.start_line);
        let peer = match row.file_had_any_access {
            Some(true) => ansi(color, "32", "file-active"),
            Some(false) => ansi(color, "2", "file-inactive"),
            None => ansi(color, "2", "file-unknown"),
        };
        let _ = writeln!(
            out,
            "  {} {} calls={} new={} fetch={} read={} probes={} {}",
            color_function_name(&row.display_name, color),
            location,
            row.call_count,
            row.new_opcode_observed_count,
            row.fetch_observed_count,
            row.read_observed_count,
            row.defined_probe_count,
            peer
        );
    }
    if rows.len() > shown {
        let _ = writeln!(out, "  ... {} more", rows.len() - shown);
    }
}

fn render_included_file_section(
    out: &mut String,
    title: &str,
    rows: &[UnusedIncludedFileReport],
    color: bool,
) {
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "{}",
        ansi(color, "36", &format!("{title} ({})", rows.len()))
    );
    if rows.is_empty() {
        let _ = writeln!(out, "  none");
        return;
    }

    let shown = rows.len().min(50);
    for row in rows.iter().take(shown) {
        let _ = writeln!(
            out,
            "  {} includes={} declarations={} accessed={} functions={} methods={} classes={} constants={}/{}",
            ansi(color, "35", &row.file),
            row.include_count,
            row.declaration_count,
            row.accessed_declaration_count,
            row.function_declaration_count,
            row.method_declaration_count,
            row.class_declaration_count,
            row.global_constant_declaration_count,
            row.class_constant_declaration_count
        );
    }
    if rows.len() > shown {
        let _ = writeln!(out, "  ... {} more", rows.len() - shown);
    }
}

fn render_trace_event(
    out: &mut String,
    event: &TraceEventReport,
    _trace_value: &str,
    path_base: Option<&str>,
    color: bool,
) {
    let elapsed_ms = event.elapsed_ns as f64 / 1_000_000.0;
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "  [{:>4} +{elapsed_ms:.3}ms] {} {} {}",
        event.event_index,
        color_function_name(&event.display_name, color),
        color_match_path(&event.argument_path, color),
        event.match_kind
    );

    if let Some(frames) = event.stack_frames.as_array() {
        if let Some(frame) = frames.first() {
            render_immediate_frame(
                out,
                frame,
                &event.display_name,
                &event.matched_value,
                path_base,
                color,
            );
            render_caller_frames(out, &frames[1..], &event.matched_value, path_base, color);
            return;
        }
    }

    render_trace_event_fallback(out, event, &event.matched_value, path_base, color);
}

fn render_immediate_frame(
    out: &mut String,
    frame: &Value,
    fallback_name: &str,
    trace_value: &str,
    path_base: Option<&str>,
    color: bool,
) {
    let display_name = frame_display_name(frame).unwrap_or(fallback_name);
    let _ = writeln!(out, "    {}", ansi(color, "1", "call:"));
    render_multiline_call(out, frame, display_name, trace_value, color);

    let location = frame_location(frame, path_base);
    if !location.is_empty() {
        let _ = writeln!(out, "      {} {location}", ansi(color, "2", "at"));
    }
}

fn render_caller_frames(
    out: &mut String,
    frames: &[Value],
    trace_value: &str,
    path_base: Option<&str>,
    color: bool,
) {
    if frames.is_empty() {
        return;
    }

    let _ = writeln!(out, "    {}", ansi(color, "1", "called from:"));
    let shown = frames.len().min(4);
    for frame in frames.iter().take(shown) {
        let index = frame
            .get("index")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        let display_name = frame_display_name(frame).unwrap_or("{unknown}");
        let location = frame_location(frame, path_base);
        let location = if location.is_empty() {
            String::new()
        } else {
            format!(" {}", ansi(color, "2", &location))
        };
        let _ = writeln!(
            out,
            "      {} {}{}",
            ansi(color, "2", &format!("#{index}")),
            format_compact_call(frame, display_name, trace_value, color),
            location
        );
    }
    if frames.len() > shown {
        let _ = writeln!(
            out,
            "      {}",
            ansi(
                color,
                "2",
                &format!("... {} more caller frames", frames.len() - shown)
            )
        );
    }
}

fn render_trace_event_fallback(
    out: &mut String,
    event: &TraceEventReport,
    trace_value: &str,
    path_base: Option<&str>,
    color: bool,
) {
    let _ = writeln!(
        out,
        "    match: {}",
        highlight_value(&event.preview, trace_value, 220, color)
    );
    if event.observed_value != event.preview && event.observed_value.chars().count() <= 240 {
        let _ = writeln!(
            out,
            "    observed: {}",
            highlight_value(&event.observed_value, trace_value, 240, color)
        );
    }
    let location = format_location_with_base(event.file.as_deref(), event.start_line, path_base);
    if !location.is_empty() {
        let _ = writeln!(out, "    at: {location}");
    }
    for line in event.stack.iter().take(3) {
        let _ = writeln!(
            out,
            "    {}",
            highlight_value(line, trace_value, 260, color)
        );
    }
    if event.stack.len() > 3 {
        let _ = writeln!(out, "    ... {} more stack frames", event.stack.len() - 3);
    }
}

fn render_multiline_call(
    out: &mut String,
    frame: &Value,
    display_name: &str,
    trace_value: &str,
    color: bool,
) {
    let args = frame_args(frame);
    if args.is_empty() {
        let _ = writeln!(
            out,
            "      {}{}",
            color_function_name(display_name, color),
            color_syntax("()", color)
        );
        return;
    }

    let _ = writeln!(
        out,
        "      {}{}",
        color_function_name(display_name, color),
        color_syntax("(", color)
    );
    for (position, arg) in args.iter().enumerate() {
        let comma = if position + 1 == args.len() { "" } else { "," };
        let _ = writeln!(
            out,
            "        {}{}",
            format_immediate_arg(arg, trace_value, color),
            color_syntax(comma, color)
        );
    }
    let _ = writeln!(out, "      {}", color_syntax(")", color));
}

fn format_compact_call(
    frame: &Value,
    display_name: &str,
    trace_value: &str,
    color: bool,
) -> String {
    let args = frame_args(frame);
    let mut parts = Vec::new();
    for arg in args {
        if arg_contains_trace(arg) {
            parts.push(format_caller_arg(arg, trace_value, color));
        }
    }

    if !args.is_empty() && parts.len() < args.len() {
        parts.push(ansi(color, "2", "..."));
    }

    format!(
        "{}{}{}{}",
        color_caller_function_name(display_name, color),
        color_syntax("(", color),
        parts.join(&color_syntax(", ", color)),
        color_syntax(")", color)
    )
}

fn format_immediate_arg(arg: &Value, trace_value: &str, color: bool) -> String {
    let label = arg_label(arg, color);
    let zval_type = arg.get("type").and_then(Value::as_str).unwrap_or("unknown");
    let preview = arg.get("preview").and_then(Value::as_str).unwrap_or("");
    let value = format_preview_literal(preview, zval_type, trace_value, 180, color);
    let matches = format_match_details(arg, trace_value, 96, color);

    format!("{}{} {}{}", label, color_syntax(":", color), value, matches)
}

fn format_caller_arg(arg: &Value, trace_value: &str, color: bool) -> String {
    let label = arg_label(arg, color);
    let matches = arg_matches(arg);
    let Some(first_match) = matches.first() else {
        return label;
    };

    let path = first_match
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let preview = first_match
        .get("preview")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let raw_label = arg_raw_label(arg);

    let mut out = if path == raw_label {
        format!(
            "{}{}{}",
            label,
            color_syntax("=", color),
            format_preview_literal(preview, "string", trace_value, 80, color)
        )
    } else {
        format!(
            "{} {} {}",
            label,
            color_muted_label("matches", color),
            color_match_path(path, color)
        )
    };

    if matches.len() > 1 {
        out.push(' ');
        out.push_str(&ansi(color, "2", &format!("+{} more", matches.len() - 1)));
    }

    out
}

fn format_match_details(arg: &Value, trace_value: &str, max_chars: usize, color: bool) -> String {
    let matches = arg_matches(arg);
    if matches.is_empty() {
        return String::new();
    }

    let raw_label = arg_raw_label(arg);
    let arg_preview = arg
        .get("preview")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut parts = Vec::new();
    let mut skipped_direct_match = false;
    for matched in matches {
        let path = matched
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let preview = matched
            .get("preview")
            .and_then(Value::as_str)
            .unwrap_or_default();

        if path == raw_label && preview == arg_preview {
            skipped_direct_match = true;
            continue;
        }

        if parts.len() == 2 {
            break;
        }

        parts.push(format!(
            "{}{}{}",
            color_match_path(path, color),
            color_syntax("=", color),
            format_preview_literal(preview, "string", trace_value, max_chars, color)
        ));
    }

    if parts.is_empty() {
        if skipped_direct_match {
            return format!(" {}", color_muted_label("match", color));
        }
        return String::new();
    }

    if matches.len() > parts.len() {
        parts.push(ansi(
            color,
            "2",
            &format!("+{} more", matches.len() - parts.len()),
        ));
    }

    format!(
        " {} {}",
        color_muted_label("matches", color),
        parts.join(&color_syntax(", ", color))
    )
}

fn frame_display_name(frame: &Value) -> Option<&str> {
    frame
        .get("display_name")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

fn frame_args(frame: &Value) -> &[Value] {
    frame
        .get("args")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn arg_matches(arg: &Value) -> &[Value] {
    arg.get("matches")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn arg_contains_trace(arg: &Value) -> bool {
    arg.get("contains_trace_value")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn arg_raw_label(arg: &Value) -> String {
    let index = arg.get("index").and_then(Value::as_u64).unwrap_or_default();
    format!("arg{index}")
}

fn arg_label(arg: &Value, color: bool) -> String {
    color_arg_label(&arg_raw_label(arg), color)
}

fn frame_location(frame: &Value, path_base: Option<&str>) -> String {
    let file = frame.get("file").and_then(Value::as_str);
    let line = frame
        .get("line")
        .and_then(Value::as_u64)
        .and_then(|line| u32::try_from(line).ok())
        .unwrap_or_default();
    format_location_with_base(file, line, path_base)
}

fn format_preview_literal(
    value: &str,
    zval_type: &str,
    trace_value: &str,
    max_chars: usize,
    color: bool,
) -> String {
    let value = highlight_value(value, trace_value, max_chars, color);
    if zval_type == "string" {
        format!(
            "{}{}{}",
            color_syntax("\"", color),
            value,
            color_syntax("\"", color)
        )
    } else {
        value
    }
}

fn color_function_name(value: &str, color: bool) -> String {
    ansi(color, "1;36", value)
}

fn color_caller_function_name(value: &str, color: bool) -> String {
    let _ = color;
    value.to_string()
}

fn color_arg_label(value: &str, color: bool) -> String {
    ansi(color, "34", value)
}

fn color_match_path(value: &str, color: bool) -> String {
    ansi(color, "35", value)
}

fn color_syntax(value: &str, color: bool) -> String {
    ansi(color, "2", value)
}

fn color_muted_label(value: &str, color: bool) -> String {
    let _ = color;
    value.to_string()
}

fn color_status(status: &str, color: bool) -> String {
    match status {
        "complete" => ansi(color, "32", status),
        "started" => ansi(color, "33", status),
        _ => ansi(color, "31", status),
    }
}

fn format_delta(delta: i64, color: bool) -> String {
    let value = if delta > 0 {
        format!("+{delta}")
    } else {
        delta.to_string()
    };
    if delta > 0 {
        ansi(color, "32", &value)
    } else if delta < 0 {
        ansi(color, "31", &value)
    } else {
        ansi(color, "2", &value)
    }
}

fn format_location(file: Option<&str>, line: u32) -> String {
    format_location_with_base(file, line, None)
}

fn format_location_with_base(file: Option<&str>, line: u32, path_base: Option<&str>) -> String {
    match (file, line) {
        (Some(file), line) if line > 0 => format!("{}:{line}", display_path(file, path_base)),
        (Some(file), _) => display_path(file, path_base),
        _ => String::new(),
    }
}

fn trace_path_base(run: &TraceRunReport) -> Option<String> {
    let mut directories = Vec::new();
    if let Some(script) = run.script_filename.as_deref().and_then(path_directory) {
        directories.push(script);
    }

    for event in &run.events {
        if let Some(directory) = event.file.as_deref().and_then(path_directory) {
            directories.push(directory);
        }
        if let Some(frames) = event.stack_frames.as_array() {
            for frame in frames {
                if let Some(directory) = frame
                    .get("file")
                    .and_then(Value::as_str)
                    .and_then(path_directory)
                {
                    directories.push(directory);
                }
            }
        }
    }

    let mut directories = directories.into_iter();
    let mut prefix = directories.next()?;
    for directory in directories {
        while !directory.starts_with(&prefix) {
            prefix = parent_directory_prefix(&prefix)?;
        }
    }

    if prefix.len() > 1 {
        Some(prefix)
    } else {
        None
    }
}

fn path_directory(path: &str) -> Option<String> {
    let index = path.rfind('/')?;
    Some(path[..=index].to_string())
}

fn parent_directory_prefix(path: &str) -> Option<String> {
    let trimmed = path.trim_end_matches('/');
    let index = trimmed.rfind('/')?;
    Some(trimmed[..=index].to_string())
}

fn display_path(path: &str, path_base: Option<&str>) -> String {
    path_base
        .and_then(|base| path.strip_prefix(base))
        .filter(|relative| !relative.is_empty())
        .unwrap_or(path)
        .to_string()
}

fn highlight_value(value: &str, needle: &str, max_chars: usize, color: bool) -> String {
    let escaped = truncate_chars(&escape_text(value), max_chars);
    if !color {
        return escaped;
    }

    let escaped_needle = escape_text(needle);
    if escaped_needle.is_empty() {
        return escaped;
    }

    let mut out = String::new();
    let mut rest = escaped.as_str();
    while let Some(index) = rest.find(&escaped_needle) {
        out.push_str(&rest[..index]);
        out.push_str("\x1b[1;33m");
        out.push_str(&rest[index..index + escaped_needle.len()]);
        out.push_str("\x1b[0m");
        rest = &rest[index + escaped_needle.len()..];
    }
    out.push_str(rest);
    out
}

fn escape_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\0"),
            ch if ch.is_control() => out.push('?'),
            ch => out.push(ch),
        }
    }
    out
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut out: String = value.chars().take(max_chars).collect();
    out.push_str("...");
    out
}

fn ansi(enabled: bool, code: &str, value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    if enabled {
        format!("\x1b[{code}m{value}\x1b[0m")
    } else {
        value.to_string()
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn now_ns() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

fn trace_value_kind(value: &str) -> String {
    if value.parse::<i64>().is_ok() || value.parse::<f64>().is_ok_and(|value| value.is_finite()) {
        "number".to_string()
    } else {
        "string".to_string()
    }
}

fn match_kind_from_u8(value: u8) -> &'static str {
    match value {
        2 => "number_equals",
        3 => "numeric_string_contains",
        _ => "string_contains",
    }
}

fn display_name(key: &FunctionKey) -> String {
    match (&key.kind, &key.scope_name) {
        (FunctionKind::Method | FunctionKind::InternalMethod, Some(scope)) => {
            format!("{scope}::{}", key.function_name)
        }
        (FunctionKind::Closure, _) => format!(
            "{{closure}}@{}:{}",
            key.file.as_deref().unwrap_or("[unknown]"),
            key.start_line
        ),
        _ => key.function_name.clone(),
    }
}

fn identity_string(key: &FunctionKey) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}",
        key.kind.as_str(),
        key.scope_name.as_deref().unwrap_or(""),
        key.function_name,
        key.file.as_deref().unwrap_or(""),
        key.start_line,
        key.end_line
    )
}

fn unused_symbol_key(
    kind: UnusedSymbolKind,
    scope_name: Option<&str>,
    name: &str,
) -> UnusedSymbolKey {
    let scope_name = scope_name
        .map(normalize_class_like_name)
        .filter(|value| !value.is_empty());
    let name = match kind {
        UnusedSymbolKind::Function
        | UnusedSymbolKind::Method
        | UnusedSymbolKind::Class
        | UnusedSymbolKind::Closure => normalize_class_like_name(name),
        UnusedSymbolKind::GlobalConstant | UnusedSymbolKind::ClassConstant => {
            name.trim_start_matches('\\').to_string()
        }
    };
    UnusedSymbolKey {
        kind,
        scope_name,
        name,
    }
}

fn normalize_class_like_name(value: &str) -> String {
    value.trim_start_matches('\\').to_ascii_lowercase()
}

fn unused_display_name(kind: &UnusedSymbolKind, scope_name: Option<&str>, name: &str) -> String {
    match (kind, scope_name) {
        (UnusedSymbolKind::Method | UnusedSymbolKind::ClassConstant, Some(scope)) => {
            format!(
                "{}::{}",
                scope.trim_start_matches('\\'),
                name.trim_start_matches('\\')
            )
        }
        _ => name.trim_start_matches('\\').to_string(),
    }
}

fn unused_identity_hash(key: &UnusedSymbolKey) -> String {
    let identity = format!(
        "{}|{}|{}",
        key.kind.as_str(),
        key.scope_name.as_deref().unwrap_or(""),
        key.name
    );
    fnv1a64_hex(identity.as_bytes())
}

fn fnv1a64_hex(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
