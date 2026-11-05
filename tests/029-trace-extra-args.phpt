--TEST--
gameshark trace events and stack frames show arguments beyond those a function declares
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
$db = __DIR__ . '/gameshark_trace_extra_args.sqlite';
@unlink($db);
@unlink($db . '-shm');
@unlink($db . '-wal');

$php = getenv('TEST_PHP_EXECUTABLE_ESCAPED');
$ext = realpath(__DIR__ . '/../modules/gameshark.so');

$cmd = sprintf(
    'GAMESHARK_DB=%s GAMESHARK_TRACE_VALUE=%s %s -n -d extension=%s -r %s 2>&1',
    escapeshellarg($db),
    escapeshellarg('needle'),
    $php,
    escapeshellarg($ext),
    escapeshellarg(<<<'PHP'
function variadic_outer($first, ...$rest) {
    $local = 'local value';
    $other = 'other value';
    inner_target($first . ' needle');
}
function inner_target($value) {}
variadic_outer('plain', 'extra needle', 'more needle');
PHP)
);
exec($cmd, $output, $status);
if ($status !== 0 || $output) {
    echo implode("\n", $output), "\n";
    echo "status=$status\n";
}

$cmd = sprintf(
    'GAMESHARK_DB=%s %s -n -d extension=%s -r %s',
    escapeshellarg($db),
    $php,
    escapeshellarg($ext),
    escapeshellarg(<<<'PHP'
$report = gameshark_trace_report("array");
$inner = null;
foreach ($report['runs'][0]['events'] as $event) {
    if ($event['display_name'] === 'variadic_outer') {
        echo $event['display_name'], '|', $event['argument_path'], '|', $event['preview'], "\n";
    }
    if ($event['display_name'] === 'inner_target') {
        $inner = $event;
    }
}
foreach ($inner['stack_frames'] as $frame) {
    if ($frame['display_name'] === 'variadic_outer') {
        echo implode(',', array_column($frame['args'], 'preview')), "\n";
    }
}
PHP)
);
passthru($cmd);
?>
--CLEAN--
<?php
$db = __DIR__ . '/gameshark_trace_extra_args.sqlite';
@unlink($db);
@unlink($db . '-shm');
@unlink($db . '-wal');
?>
--EXPECT--
variadic_outer|arg1|extra needle
variadic_outer|arg2|more needle
plain,extra needle,more needle
