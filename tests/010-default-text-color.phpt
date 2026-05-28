--TEST--
gameshark reports default to text and honor color controls
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
$db = __DIR__ . '/gameshark_text_report.sqlite';
@unlink($db);
@unlink($db . '-shm');
@unlink($db . '-wal');

$php = getenv('TEST_PHP_EXECUTABLE_ESCAPED');
$ext = realpath(__DIR__ . '/../modules/gameshark.so');

$cmd = sprintf(
    'GAMESHARK_DB=%s GAMESHARK_SIDE=%s GAMESHARK_TRACE_VALUE=%s %s -n -d extension=%s -r %s 2>&1',
    escapeshellarg($db),
    escapeshellarg('left'),
    escapeshellarg('needle'),
    $php,
    escapeshellarg($ext),
    escapeshellarg('function only_left($value){} only_left("needle");')
);
exec($cmd, $output, $status);
if ($status !== 0 || $output) {
    echo implode("\n", $output), "\n";
    echo "status=$status\n";
}

$cmd = sprintf(
    'GAMESHARK_DB=%s GAMESHARK_SIDE=%s %s -n -d extension=%s -r %s 2>&1',
    escapeshellarg($db),
    escapeshellarg('right'),
    $php,
    escapeshellarg($ext),
    escapeshellarg('function only_right(){} only_right();')
);
exec($cmd, $output, $status);
if ($status !== 0 || $output) {
    echo implode("\n", $output), "\n";
    echo "status=$status\n";
}

$cmd = sprintf(
    'GAMESHARK_DB=%s GAMESHARK_COLOR=never %s -n -d extension=%s -r %s',
    escapeshellarg($db),
    $php,
    escapeshellarg($ext),
    escapeshellarg(<<<'PHP'
$compare = gameshark_compare();
var_dump(is_string($compare));
var_dump(str_contains($compare, 'Gameshark compare report'));
var_dump(!str_contains($compare, "\033["));
$trace = gameshark_trace_report();
var_dump(is_string($trace));
var_dump(str_contains($trace, 'Gameshark trace report'));
var_dump(!str_contains($trace, "\033["));
var_dump(is_array(gameshark_compare("array")));
var_dump(is_array(gameshark_trace_report("array")));
PHP)
);
passthru($cmd);

$cmd = sprintf(
    'GAMESHARK_DB=%s GAMESHARK_COLOR=always %s -n -d extension=%s -r %s',
    escapeshellarg($db),
    $php,
    escapeshellarg($ext),
    escapeshellarg(<<<'PHP'
var_dump(str_contains(gameshark_compare(), "\033["));
var_dump(str_contains(gameshark_trace_report(), "\033["));
PHP)
);
passthru($cmd);
?>
--CLEAN--
<?php
$db = __DIR__ . '/gameshark_text_report.sqlite';
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
bool(true)
bool(true)
bool(true)
bool(true)
bool(true)
