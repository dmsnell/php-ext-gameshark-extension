--TEST--
gameshark can trace values and collect differential counts in one invocation
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
$db = __DIR__ . '/gameshark_trace_combined.sqlite';
@unlink($db);
@unlink($db . '-shm');
@unlink($db . '-wal');

$php = getenv('TEST_PHP_EXECUTABLE_ESCAPED');
$ext = realpath(__DIR__ . '/../modules/gameshark.so');

$cmd = sprintf(
    'GAMESHARK_DB=%s GAMESHARK_SIDE=%s GAMESHARK_TRACE_VALUE=%s %s -n -d extension=%s -r %s',
    escapeshellarg($db),
    escapeshellarg('left'),
    escapeshellarg('needle'),
    $php,
    escapeshellarg($ext),
    escapeshellarg(<<<'PHP'
function counted($value) {}
echo gameshark_side(), "\n";
counted("needle");
PHP)
);
passthru($cmd);

$cmd = sprintf(
    'GAMESHARK_DB=%s %s -n -d extension=%s -r %s',
    escapeshellarg($db),
    $php,
    escapeshellarg($ext),
    escapeshellarg(<<<'PHP'
$compare = gameshark_compare("array");
$left = array_column($compare['left_only'], 'display_name');
sort($left);
echo implode(',', $left), "\n";
$trace = gameshark_trace_report("array");
echo $trace['summary']['run_count'], ':', $trace['summary']['event_count'], "\n";
foreach ($trace['runs'][0]['events'] as $event) {
    echo $event['display_name'], '|', $event['argument_path'], '|', $event['preview'], "\n";
}
PHP)
);
passthru($cmd);
?>
--CLEAN--
<?php
$db = __DIR__ . '/gameshark_trace_combined.sqlite';
@unlink($db);
@unlink($db . '-shm');
@unlink($db . '-wal');
?>
--EXPECT--
left
counted
1:1
counted|arg0|needle
