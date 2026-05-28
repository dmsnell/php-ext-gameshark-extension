--TEST--
gameshark unused mode selects latest run by default and explicit run ids on request
--SKIPIF--
<?php
if (!getenv('TEST_PHP_EXECUTABLE_ESCAPED')) {
    die('skip TEST_PHP_EXECUTABLE_ESCAPED is unavailable');
}
if (!file_exists(__DIR__ . '/../modules/gameshark.so')) {
    die('skip gameshark module is not built');
}
$disabled = array_map('trim', explode(',', ini_get('disable_functions')));
foreach (['exec', 'shell_exec'] as $function) {
    if (in_array($function, $disabled, true)) {
        die("skip $function is disabled");
    }
}
?>
--FILE--
<?php
$db = __DIR__ . '/gameshark_unused_selection.sqlite';
@unlink($db);
@unlink($db . '-shm');
@unlink($db . '-wal');

$php = getenv('TEST_PHP_EXECUTABLE_ESCAPED');
$ext = realpath(__DIR__ . '/../modules/gameshark.so');

function run_unused_code(string $db, string $code): void {
    global $php, $ext;
    $cmd = sprintf(
        'GAMESHARK_DB=%s GAMESHARK_UNUSED=1 %s -n -d extension=%s -r %s 2>&1',
        escapeshellarg($db),
        $php,
        escapeshellarg($ext),
        escapeshellarg($code)
    );
    exec($cmd, $output, $status);
    if ($status !== 0 || $output) {
        echo implode("\n", $output), "\n";
        echo "status=$status\n";
    }
}

function unused_report(string $db, ?int $runId = null): array {
    global $php, $ext;
    $reportCode = $runId === null
        ? 'echo serialize(gameshark_unused_report("array"));'
        : 'echo serialize(gameshark_unused_report("array", ' . $runId . '));';
    $cmd = sprintf(
        'GAMESHARK_DB=%s %s -n -d extension=%s -r %s',
        escapeshellarg($db),
        $php,
        escapeshellarg($ext),
        escapeshellarg($reportCode)
    );
    return unserialize(shell_exec($cmd));
}

function report_names(array $report): array {
    $names = array_column($report['uncalled_functions'], 'display_name');
    sort($names);
    return $names;
}

run_unused_code($db, <<<'PHP'
function first_called() {}
function first_uncalled() {}
first_called();
PHP);
$first = unused_report($db);
$firstRunId = $first['summary']['run_id'];

run_unused_code($db, <<<'PHP'
function second_called() {}
function second_uncalled() {}
second_called();
PHP);
$latest = unused_report($db);
$secondRunId = $latest['summary']['run_id'];
$selectedFirst = unused_report($db, $firstRunId);

$latestNames = report_names($latest);
$firstNames = report_names($selectedFirst);

var_dump($secondRunId > $firstRunId);
var_dump(in_array('second_uncalled', $latestNames, true));
var_dump(!in_array('first_uncalled', $latestNames, true));
var_dump(in_array('first_uncalled', $firstNames, true));
var_dump(!in_array('second_uncalled', $firstNames, true));
?>
--CLEAN--
<?php
$db = __DIR__ . '/gameshark_unused_selection.sqlite';
@unlink($db);
@unlink($db . '-shm');
@unlink($db . '-wal');
?>
--EXPECT--
bool(true)
bool(true)
bool(true)
bool(true)
bool(true)
