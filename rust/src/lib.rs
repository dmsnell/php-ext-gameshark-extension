use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::slice;
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

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
}

impl FunctionKind {
    fn from_u8(value: u8) -> Self {
        match value {
            2 => Self::Method,
            3 => Self::Closure,
            _ => Self::Function,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Method => "method",
            Self::Closure => "closure",
        }
    }
}

struct State {
    db_path: String,
    side: String,
    started_at: i64,
    php_version: String,
    sapi_name: String,
    pid: u32,
    script_filename: Option<String>,
    counters: HashMap<FunctionKey, u64>,
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

static STATE: LazyLock<Mutex<Option<State>>> = LazyLock::new(|| Mutex::new(None));

#[no_mangle]
pub extern "C" fn gameshark_core_request_start(
    db_path: *const c_char,
    side: *const c_char,
    php_version: *const c_char,
    sapi_name: *const c_char,
    pid: u32,
    script_filename: *const c_char,
) -> c_int {
    let Some(db_path) = c_string(db_path) else {
        return 0;
    };
    let Some(side) = c_string(side) else {
        return 0;
    };
    if side != "left" && side != "right" {
        return 0;
    }

    let php_version = c_string(php_version).unwrap_or_default();
    let sapi_name = c_string(sapi_name).unwrap_or_default();
    let script_filename = c_string(script_filename).filter(|value| !value.is_empty());
    let started_at = now();

    if initialize_side(
        &db_path,
        &side,
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

    let mut state = STATE.lock().expect("gameshark state lock poisoned");
    *state = Some(State {
        db_path,
        side,
        started_at,
        php_version,
        sapi_name,
        pid,
        script_filename,
        counters: HashMap::new(),
    });
    1
}

#[no_mangle]
pub unsafe extern "C" fn gameshark_core_record_call(meta: *const GamesharkCoreFunctionMeta) {
    let Some(meta) = meta.as_ref() else {
        return;
    };
    let Some(function_name) = ffi_str(&meta.function_name) else {
        return;
    };
    if function_name.is_empty() {
        return;
    }

    let key = FunctionKey {
        kind: FunctionKind::from_u8(meta.kind),
        scope_name: ffi_str(&meta.scope_name).filter(|value| !value.is_empty()),
        function_name,
        file: ffi_str(&meta.file).filter(|value| !value.is_empty()),
        start_line: meta.start_line,
        end_line: meta.end_line,
    };

    let mut state = STATE.lock().expect("gameshark state lock poisoned");
    let Some(state) = state.as_mut() else {
        return;
    };
    *state.counters.entry(key).or_insert(0) += 1;
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
    let result = c_string(db_path)
        .ok_or_else(|| "GAMESHARK_DB is not set".to_string())
        .and_then(|db_path| compare_json(&db_path));
    let json = match result {
        Ok(json) => json,
        Err(error) => {
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
                "same": [],
                "error": error
            })
            .to_string()
        }
    };

    CString::new(json)
        .unwrap_or_else(|_| CString::new("{\"error\":\"invalid json\"}").unwrap())
        .into_raw()
}

#[no_mangle]
pub unsafe extern "C" fn gameshark_core_string_free(ptr: *mut c_char) {
    if !ptr.is_null() {
        drop(CString::from_raw(ptr));
    }
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

fn open_db(db_path: &str) -> Result<Connection, String> {
    let connection = Connection::open(db_path).map_err(|error| error.to_string())?;
    connection
        .busy_timeout(std::time::Duration::from_millis(5000))
        .map_err(|error| error.to_string())?;
    Ok(connection)
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
            ",
        )
        .map_err(|error| error.to_string())
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
    let transaction = connection.transaction().map_err(|error| error.to_string())?;
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

fn flush_state(state: State) -> Result<(), String> {
    let mut connection = open_db(&state.db_path)?;
    initialize_schema(&connection)?;
    let transaction = connection.transaction().map_err(|error| error.to_string())?;

    for (key, count) in state.counters {
        let identity = identity_string(&key);
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
                    display_name(&key),
                    key.scope_name,
                    key.function_name,
                    key.file,
                    key.start_line,
                    key.end_line
                ],
            )
            .map_err(|error| error.to_string())?;
        let function_id: i64 = transaction
            .query_row(
                "SELECT function_id FROM functions WHERE identity_hash = ?",
                params![identity_hash],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "
                INSERT INTO function_counts (side, function_id, call_count)
                VALUES (?, ?, ?)
                ON CONFLICT(side, function_id) DO UPDATE SET call_count = excluded.call_count
                ",
                params![state.side, function_id, count],
            )
            .map_err(|error| error.to_string())?;
    }

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
                state.side,
                state.started_at,
                now(),
                state.php_version,
                state.sapi_name,
                state.pid,
                state.script_filename
            ],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())
}

fn compare_json(db_path: &str) -> Result<String, String> {
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

    serde_json::to_string(&report).map_err(|error| error.to_string())
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn display_name(key: &FunctionKey) -> String {
    match (&key.kind, &key.scope_name) {
        (FunctionKind::Method, Some(scope)) => format!("{scope}::{}", key.function_name),
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

fn fnv1a64_hex(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
