--TEST--
gameshark trace value mode can limit inspected calls with a native regex allow pattern
--SKIPIF--
<?php
if (!getenv('TEST_PHP_EXECUTABLE_ESCAPED')) {
    die('skip TEST_PHP_EXECUTABLE_ESCAPED is unavailable');
}
if (!file_exists(__DIR__ . '/../modules/gameshark.so')) {
    die('skip gameshark module is not built');
}
?>
--FILE--
<?php
$php = getenv('TEST_PHP_EXECUTABLE_ESCAPED');
$ext = realpath(__DIR__ . '/../modules/gameshark.so');

function reset_db(string $db): void {
    @unlink($db);
    @unlink($db . '-shm');
    @unlink($db . '-wal');
}

function run_trace(string $db, string $value, string $pattern, string $code, bool $follow = false): array {
    global $php, $ext;
    $cmd = sprintf(
        'GAMESHARK_DB=%s GAMESHARK_TRACE_VALUE=%s GAMESHARK_TRACE_ALLOW_PATTERN=%s %s %s -n -d extension=%s -r %s 2>&1',
        escapeshellarg($db),
        escapeshellarg($value),
        escapeshellarg($pattern),
        $follow ? 'GAMESHARK_TRACE_FOLLOW_TRANSFORMS=1' : '',
        $php,
        escapeshellarg($ext),
        escapeshellarg($code)
    );
    exec($cmd, $output, $status);
    return [$status, implode("\n", $output)];
}

function run_trace_ini(string $db, string $value, string $pattern, string $code): array {
    global $php, $ext;
    $cmd = sprintf(
        'GAMESHARK_DB=%s GAMESHARK_TRACE_VALUE=%s %s -n -d extension=%s -d gameshark.trace_allow_pattern=%s -r %s 2>&1',
        escapeshellarg($db),
        escapeshellarg($value),
        $php,
        escapeshellarg($ext),
        escapeshellarg($pattern),
        escapeshellarg($code)
    );
    exec($cmd, $output, $status);
    return [$status, implode("\n", $output)];
}

function trace_report(string $db): array {
    global $php, $ext;
    $cmd = sprintf(
        'GAMESHARK_DB=%s %s -n -d extension=%s -r %s',
        escapeshellarg($db),
        $php,
        escapeshellarg($ext),
        escapeshellarg('$report = gameshark_trace_report("array"); echo serialize($report);')
    );
    exec($cmd, $output, $status);
    if ($status !== 0) {
        return ['status' => $status, 'output' => $output];
    }
    return unserialize(implode("\n", $output));
}

$db = __DIR__ . '/gameshark_trace_allow_pattern.sqlite';
reset_db($db);
[$status, $text] = run_trace($db, 'needle', '^(?:allowed|sample::method)$', <<<'PHP'
class Box {
    public string $value = 'nested needle that must not be inspected';
}
class Sample {
    public function method($value) {}
}
function blocked($value) {}
function allowed($value) {}
blocked(new Box());
allowed('direct needle');
(new Sample())->method('method needle');
PHP);
echo 'regex-process:', $status === 0 ? 'ok' : 'failed', "\n";
echo 'regex-output:', $text === '' ? 'clean' : 'dirty', "\n";
$report = trace_report($db);
$run = $report['runs'][0];
$names = array_map(static fn($event) => $event['display_name'], $run['events']);
sort($names);
echo 'regex-events:', $run['event_count'], "\n";
echo 'regex-names:', implode(',', $names), "\n";
echo 'regex-mode:', $run['trace_filter']['mode'], "\n";
echo 'regex-valid:', $run['trace_filter']['allow_pattern_valid'] ? 'yes' : 'no', "\n";
echo 'regex-seen:', $run['trace_filter']['calls_seen'], "\n";
echo 'regex-allowed:', $run['trace_filter']['calls_allowed'], "\n";
echo 'regex-filtered:', $run['trace_filter']['calls_filtered_before_args'], "\n";
echo 'regex-inspected:', $run['trace_filter']['args_inspected'], "\n";
echo 'regex-matches:', $run['trace_filter']['calls_with_value_matches'], "\n";

$iniDb = __DIR__ . '/gameshark_trace_allow_pattern_ini.sqlite';
reset_db($iniDb);
[$iniStatus, $iniText] = run_trace_ini($iniDb, 'needle', '^ini_allowed$', <<<'PHP'
function ini_blocked($value) {}
function ini_allowed($value) {}
ini_blocked('direct needle');
ini_allowed('direct needle');
PHP);
echo 'ini-process:', $iniStatus === 0 ? 'ok' : 'failed', "\n";
echo 'ini-output:', $iniText === '' ? 'clean' : 'dirty', "\n";
$iniReport = trace_report($iniDb);
$iniRun = $iniReport['runs'][0];
echo 'ini-events:', $iniRun['event_count'], "\n";
echo 'ini-name:', $iniRun['events'][0]['display_name'] ?? 'missing', "\n";

$invalidDb = __DIR__ . '/gameshark_trace_allow_pattern_invalid.sqlite';
reset_db($invalidDb);
[$invalidStatus, $invalidText] = run_trace($invalidDb, 'needle', '(', <<<'PHP'
function allowed($value) {}
allowed('direct needle');
PHP);
echo 'invalid-process:', $invalidStatus === 0 ? 'ok' : 'failed', "\n";
echo 'invalid-warning:', str_contains($invalidText, 'Gameshark trace allow pattern error') ? 'yes' : 'no', "\n";
$invalidReport = trace_report($invalidDb);
$invalidRun = $invalidReport['runs'][0];
echo 'invalid-events:', $invalidRun['event_count'], "\n";
echo 'invalid-valid:', $invalidRun['trace_filter']['allow_pattern_valid'] ? 'yes' : 'no', "\n";
echo 'invalid-error:', $invalidRun['trace_filter']['allow_pattern_error'] !== null ? 'yes' : 'no', "\n";

$followDb = __DIR__ . '/gameshark_trace_allow_pattern_follow.sqlite';
reset_db($followDb);
[$followStatus, $followText] = run_trace($followDb, "O'Reilly", '^sink$', <<<'PHP'
function blocked_make($value) {
    return addslashes($value);
}
function sink($value) {}
$escaped = blocked_make("O'Reilly");
sink($escaped);
PHP, true);
echo 'follow-process:', $followStatus === 0 ? 'ok' : 'failed', "\n";
echo 'follow-output:', $followText === '' ? 'clean' : 'dirty', "\n";
$followReport = trace_report($followDb);
$followRun = $followReport['runs'][0];
echo 'follow-events:', $followRun['event_count'], "\n";
echo 'follow-transforms:', $followRun['transformed_value_count'], "\n";
echo 'follow-transform-frames:', $followRun['trace_filter']['transform_frames_started'], "\n";
?>
--CLEAN--
<?php
foreach ([
    __DIR__ . '/gameshark_trace_allow_pattern.sqlite',
    __DIR__ . '/gameshark_trace_allow_pattern_ini.sqlite',
    __DIR__ . '/gameshark_trace_allow_pattern_invalid.sqlite',
    __DIR__ . '/gameshark_trace_allow_pattern_follow.sqlite',
] as $db) {
    @unlink($db);
    @unlink($db . '-shm');
    @unlink($db . '-wal');
}
?>
--EXPECT--
regex-process:ok
regex-output:clean
regex-events:2
regex-names:Sample::method,allowed
regex-mode:rust_regex_v1
regex-valid:yes
regex-seen:3
regex-allowed:2
regex-filtered:1
regex-inspected:2
regex-matches:2
ini-process:ok
ini-output:clean
ini-events:1
ini-name:ini_allowed
invalid-process:ok
invalid-warning:yes
invalid-events:0
invalid-valid:no
invalid-error:yes
follow-process:ok
follow-output:clean
follow-events:0
follow-transforms:0
follow-transform-frames:0
