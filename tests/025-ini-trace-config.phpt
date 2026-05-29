--TEST--
gameshark trace mode accepts db and trace value INI entries
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
$db = __DIR__ . '/gameshark_ini_trace_config.sqlite';
@unlink($db);
@unlink($db . '-shm');
@unlink($db . '-wal');

$php = getenv('TEST_PHP_EXECUTABLE_ESCAPED');
$ext = realpath(__DIR__ . '/../modules/gameshark.so');

$code = <<<'PHP'
function ini_trace_target($value) {
    return "prefix " . $value;
}
ini_trace_target("test");
PHP;

$cmd = sprintf(
    '%s -n -d extension=%s -d gameshark.db=%s -d gameshark.trace_value=%s -r %s 2>&1',
    $php,
    escapeshellarg($ext),
    escapeshellarg($db),
    escapeshellarg('test'),
    escapeshellarg($code)
);
exec($cmd, $output, $status);
if ($status !== 0 || $output) {
    echo implode("\n", $output), "\n";
    echo "status=$status\n";
}

$cmd = sprintf(
    '%s -n -d extension=%s -d gameshark.db=%s -r %s',
    $php,
    escapeshellarg($ext),
    escapeshellarg($db),
    escapeshellarg(<<<'PHP'
$report = gameshark_trace_report("array");
echo $report['summary']['run_count'], ':', $report['summary']['event_count'], "\n";
foreach ($report['runs'][0]['events'] as $event) {
    if ($event['display_name'] === 'ini_trace_target') {
        echo $event['display_name'], '|', $event['argument_path'], '|', $event['match_kind'], '|', $event['preview'], "\n";
    }
}
echo basename(gameshark_db_path()), "\n";
PHP)
);
passthru($cmd);
?>
--CLEAN--
<?php
$db = __DIR__ . '/gameshark_ini_trace_config.sqlite';
@unlink($db);
@unlink($db . '-shm');
@unlink($db . '-wal');
?>
--EXPECT--
1:1
ini_trace_target|arg0|string_contains|test
gameshark_ini_trace_config.sqlite
