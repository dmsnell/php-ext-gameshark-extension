use regex::Regex;
use rusqlite::{params, Connection, Transaction};
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, CString};
use std::fmt::Write as _;
use std::os::raw::{c_char, c_int};
use std::slice;
use std::sync::{LazyLock, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

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

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct FunctionKey {
    kind: FunctionKind,
    scope_name: Option<String>,
    function_name: String,
    file: Option<String>,
    start_line: u32,
    end_line: u32,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
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

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
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

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
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

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct UnusedSymbolKey {
    kind: UnusedSymbolKind,
    scope_name: Option<String>,
    name: String,
}

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

struct State {
    db_path: String,
    side: Option<String>,
    started_at: i64,
    started_monotonic: Instant,
    last_elapsed_ns: u64,
    trace_run_id: Option<i64>,
    php_version: String,
    sapi_name: String,
    pid: u32,
    script_filename: Option<String>,
    trace_filter: TraceFilter,
    counters: HashMap<FunctionKey, u64>,
    trace_events: Vec<TraceEvent>,
    transformed_values: Vec<TransformedValue>,
    unused_run_id: Option<i64>,
    unused_declarations: HashMap<UnusedSymbolKey, UnusedDeclaration>,
    unused_accesses: HashMap<(UnusedSymbolKey, UnusedAccessKind), UnusedAccess>,
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

#[derive(Serialize)]
struct TraceReport {
    summary: TraceSummary,
    runs: Vec<TraceRunReport>,
}

#[derive(Serialize)]
struct TraceSummary {
    run_count: usize,
    event_count: usize,
    transformed_value_count: usize,
}

#[derive(Serialize)]
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

#[derive(Serialize)]
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

#[derive(Serialize)]
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

#[derive(Serialize)]
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

#[derive(Serialize)]
struct UnusedReport {
    summary: UnusedSummary,
    run: Option<UnusedRunReport>,
    uncalled_functions: Vec<UnusedReportRow>,
    uncalled_concrete_methods: Vec<UnusedReportRow>,
    classes_with_no_new_opcode_observed: Vec<UnusedReportRow>,
    global_constants_without_read_observed: Vec<UnusedReportRow>,
    class_constants_without_read_observed: Vec<UnusedReportRow>,
}

#[derive(Serialize)]
struct UnusedSummary {
    run_count: usize,
    run_id: Option<i64>,
    declaration_count: usize,
    access_count: usize,
    uncalled_function_count: usize,
    uncalled_concrete_method_count: usize,
    class_without_new_count: usize,
    global_constant_without_read_count: usize,
    class_constant_without_read_count: usize,
}

#[derive(Serialize)]
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

#[derive(Serialize)]
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

static STATE: LazyLock<Mutex<Option<State>>> = LazyLock::new(|| Mutex::new(None));

#[no_mangle]
pub extern "C" fn gameshark_core_request_start(
    db_path: *const c_char,
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
    let Some(db_path) = c_string(db_path) else {
        return 0;
    };

    let side = c_string(side).filter(|value| !value.is_empty());
    if let Some(side) = side.as_deref() {
        if side != "left" && side != "right" {
            return 0;
        }
    }

    let trace_value = c_string(trace_value).filter(|value| !value.is_empty());
    let unused_enabled = unused_enabled != 0;
    if side.is_none() && trace_value.is_none() && !unused_enabled {
        return 0;
    }

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

    if let Some(side) = side.as_deref() {
        if initialize_side(
            &db_path,
            side,
            started_at,
            &php_version,
            &sapi_name,
            pid,
            script_filename.as_deref(),
        )
        .is_err()
        {
            return 0;
        }
    }

    let trace_run_id = if let Some(trace_value) = trace_value.as_deref() {
        match initialize_trace_run(
            &db_path,
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
            Err(_) => return 0,
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
        match initialize_unused_run(
            &db_path,
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
            Err(_) => return 0,
        }
    } else {
        None
    };

    let mut state = STATE.lock().expect("gameshark state lock poisoned");
    *state = Some(State {
        db_path,
        side,
        started_at,
        started_monotonic: Instant::now(),
        last_elapsed_ns: 0,
        trace_run_id,
        php_version,
        sapi_name,
        pid,
        script_filename,
        trace_filter,
        counters: HashMap::new(),
        trace_events: Vec::new(),
        transformed_values: Vec::new(),
        unused_run_id,
        unused_declarations: HashMap::new(),
        unused_accesses: HashMap::new(),
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
    let file = ffi_str(&declaration.file).filter(|value| !value.is_empty());
    let key = unused_symbol_key(kind.clone(), scope_name.as_deref(), &name);
    let display_name = unused_display_name(&kind, scope_name.as_deref(), &name);

    let mut state = STATE.lock().expect("gameshark state lock poisoned");
    let Some(state) = state.as_mut() else {
        return;
    };
    if state.unused_run_id.is_none() {
        return;
    }

    state.unused_declarations.insert(
        key.clone(),
        UnusedDeclaration {
            key,
            display_name,
            scope_name,
            name,
            file,
            start_line: declaration.start_line,
            end_line: declaration.end_line,
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
pub extern "C" fn gameshark_core_compare_json(db_path: *const c_char) -> *mut c_char {
    report_json(db_path, compare_json, || {
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
pub extern "C" fn gameshark_core_compare_text(db_path: *const c_char, color: c_int) -> *mut c_char {
    report_text(db_path, |db_path| {
        let report = compare_report(db_path)?;
        Ok(render_compare_text(&report, color != 0))
    })
}

#[no_mangle]
pub extern "C" fn gameshark_core_trace_report_json(db_path: *const c_char) -> *mut c_char {
    report_json(db_path, trace_report_json, || {
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
    db_path: *const c_char,
    color: c_int,
) -> *mut c_char {
    report_text(db_path, |db_path| {
        let report = trace_report(db_path)?;
        Ok(render_trace_text(&report, color != 0))
    })
}

#[no_mangle]
pub extern "C" fn gameshark_core_unused_report_json(
    db_path: *const c_char,
    run_id: i64,
) -> *mut c_char {
    report_json(
        db_path,
        |db_path| unused_report_json(db_path, run_id),
        || {
            serde_json::json!({
                "summary": {
                    "run_count": 0,
                    "run_id": null,
                    "declaration_count": 0,
                    "access_count": 0,
                    "uncalled_function_count": 0,
                    "uncalled_concrete_method_count": 0,
                    "class_without_new_count": 0,
                    "global_constant_without_read_count": 0,
                    "class_constant_without_read_count": 0
                },
                "run": null,
                "uncalled_functions": [],
                "uncalled_concrete_methods": [],
                "classes_with_no_new_opcode_observed": [],
                "global_constants_without_read_observed": [],
                "class_constants_without_read_observed": []
            })
        },
    )
}

#[no_mangle]
pub extern "C" fn gameshark_core_unused_report_text(
    db_path: *const c_char,
    color: c_int,
    run_id: i64,
) -> *mut c_char {
    report_text(db_path, |db_path| {
        let report = unused_report(db_path, run_id)?;
        Ok(render_unused_text(&report, color != 0))
    })
}

#[no_mangle]
pub unsafe extern "C" fn gameshark_core_string_free(ptr: *mut c_char) {
    if !ptr.is_null() {
        drop(CString::from_raw(ptr));
    }
}

fn report_json<F, E>(db_path: *const c_char, build: F, empty: E) -> *mut c_char
where
    F: FnOnce(&str) -> Result<String, String>,
    E: FnOnce() -> serde_json::Value,
{
    let result = c_string(db_path)
        .ok_or_else(|| "GAMESHARK_DB is not set".to_string())
        .and_then(|db_path| build(&db_path));
    let json = match result {
        Ok(json) => json,
        Err(error) => {
            let mut value = empty();
            if let serde_json::Value::Object(ref mut object) = value {
                object.insert("error".to_string(), serde_json::Value::String(error));
            }
            value.to_string()
        }
    };

    CString::new(json)
        .unwrap_or_else(|_| CString::new("{\"error\":\"invalid json\"}").unwrap())
        .into_raw()
}

fn report_text<F>(db_path: *const c_char, build: F) -> *mut c_char
where
    F: FnOnce(&str) -> Result<String, String>,
{
    let text = c_string(db_path)
        .ok_or_else(|| "GAMESHARK_DB is not set".to_string())
        .and_then(|db_path| build(&db_path))
        .unwrap_or_else(|error| format!("Gameshark report error: {error}\n"));

    CString::new(text)
        .unwrap_or_else(|error| {
            let text = error.into_vec();
            CString::new(String::from_utf8_lossy(&text).replace('\0', "\\0")).unwrap()
        })
        .into_raw()
}

fn c_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let string = unsafe { CStr::from_ptr(ptr) };
    Some(string.to_string_lossy().into_owned())
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
    let mut connection = open_db(&state.db_path)?;
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
                    state.php_version,
                    state.sapi_name,
                    state.pid,
                    state.script_filename
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
                    state.php_version,
                    state.sapi_name,
                    state.pid,
                    state.script_filename,
                    state.trace_filter.mode,
                    state.trace_filter.allow_pattern,
                    state.trace_filter.allow_pattern_hash,
                    state.trace_filter.allow_pattern_valid,
                    state.trace_filter.allow_pattern_error,
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
        let mut caveats: Vec<_> = state.unused_caveats.into_iter().collect();
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
                    state.php_version,
                    state.sapi_name,
                    state.pid,
                    state.script_filename,
                    caveats_json,
                    run_id
                ],
            )
            .map_err(|error| error.to_string())?;
    }

    transaction.commit().map_err(|error| error.to_string())
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
    let mut global_constants_without_read_observed = Vec::new();
    let mut class_constants_without_read_observed = Vec::new();

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
                let read_count = access_count(
                    &access_counts,
                    &declaration.key,
                    UnusedAccessKind::GlobalConstantRead,
                );
                if read_count == 0 {
                    global_constants_without_read_observed.push(unused_row(
                        declaration,
                        &access_counts,
                        &active_files,
                    ));
                }
            }
            UnusedSymbolKind::ClassConstant => {
                let read_count = access_count(
                    &access_counts,
                    &declaration.key,
                    UnusedAccessKind::ClassConstantRead,
                );
                if read_count == 0 {
                    class_constants_without_read_observed.push(unused_row(
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
    sort_unused_rows(&mut global_constants_without_read_observed);
    sort_unused_rows(&mut class_constants_without_read_observed);

    Ok(UnusedReport {
        summary: UnusedSummary {
            run_count,
            run_id: Some(run_id),
            declaration_count: declarations.len(),
            access_count: accesses.len(),
            uncalled_function_count: uncalled_functions.len(),
            uncalled_concrete_method_count: uncalled_concrete_methods.len(),
            class_without_new_count: classes_with_no_new_opcode_observed.len(),
            global_constant_without_read_count: global_constants_without_read_observed.len(),
            class_constant_without_read_count: class_constants_without_read_observed.len(),
        },
        run: Some(run),
        uncalled_functions,
        uncalled_concrete_methods,
        classes_with_no_new_opcode_observed,
        global_constants_without_read_observed,
        class_constants_without_read_observed,
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
        "uncalled functions: {} | uncalled concrete methods: {} | classes without new: {} | unread constants: {}/{}",
        report.summary.uncalled_function_count,
        report.summary.uncalled_concrete_method_count,
        report.summary.class_without_new_count,
        report.summary.global_constant_without_read_count,
        report.summary.class_constant_without_read_count
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
        "Global constants without read observed",
        &report.global_constants_without_read_observed,
        color,
    );
    render_unused_section(
        &mut out,
        "Class constants without read observed",
        &report.class_constants_without_read_observed,
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
