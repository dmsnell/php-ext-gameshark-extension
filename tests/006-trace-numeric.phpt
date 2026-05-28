--TEST--
gameshark traces numeric values as numbers and string contents without a side
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
$db = __DIR__ . '/gameshark_trace_numeric.sqlite';
@unlink($db);
@unlink($db . '-shm');
@unlink($db . '-wal');

$php = getenv('TEST_PHP_EXECUTABLE_ESCAPED');
$ext = realpath(__DIR__ . '/../modules/gameshark.so');

$cmd = sprintf(
    'GAMESHARK_DB=%s GAMESHARK_TRACE_VALUE=%s %s -n -d extension=%s -r %s 2>&1',
    escapeshellarg($db),
    escapeshellarg('42'),
    $php,
    escapeshellarg($ext),
    escapeshellarg(<<<'PHP'
function takes($value) {}
var_dump(gameshark_side());
var_dump(gameshark_db_path() !== null);
takes(42);
takes("id=42 and more");
takes(43);
PHP)
);
passthru($cmd);

$cmd = sprintf(
    'GAMESHARK_DB=%s %s -n -d extension=%s -r %s',
    escapeshellarg($db),
    $php,
    escapeshellarg($ext),
    escapeshellarg(<<<'PHP'
$report = gameshark_trace_report("array");
echo $report['summary']['run_count'], ':', $report['summary']['event_count'], "\n";
foreach ($report['runs'][0]['events'] as $event) {
    echo $event['display_name'], '|', $event['argument_path'], '|', $event['zval_type'], '|', $event['match_kind'], '|', $event['preview'], "\n";
}
PHP)
);
passthru($cmd);
?>
--CLEAN--
<?php
$db = __DIR__ . '/gameshark_trace_numeric.sqlite';
@unlink($db);
@unlink($db . '-shm');
@unlink($db . '-wal');
?>
--EXPECT--
NULL
bool(true)
1:2
takes|arg0|int|number_equals|42
takes|arg0|string|numeric_string_contains|id=42 and more
