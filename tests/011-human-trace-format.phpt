--TEST--
gameshark human trace reports render structured calls with spacing and color
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
$db = __DIR__ . '/gameshark_human_trace_format.sqlite';
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
function outer($value) {
    inner("prefix " . $value . " suffix", ["copy" => $value]);
}
function inner($string, $array) {
    call_user_func("strlen", $string);
}
outer("needle");
PHP)
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
$report = gameshark_trace_report();
var_dump(str_contains($report, "\n\n  ["));
var_dump(str_contains($report, "    call:\n      strlen("));
var_dump(str_contains($report, '        arg0: "prefix needle suffix"'));
var_dump(str_contains($report, "    called from:\n"));
var_dump(str_contains($report, '#1 inner(arg0="prefix needle suffix", arg1 matches arg1["copy"])'));
var_dump(!str_contains($report, '#0 strlen(arg0='));
var_dump(!str_contains($report, "\033["));
PHP)
);
passthru($cmd);

$cmd = sprintf(
    'GAMESHARK_DB=%s GAMESHARK_COLOR=always %s -n -d extension=%s -r %s',
    escapeshellarg($db),
    $php,
    escapeshellarg($ext),
    escapeshellarg(<<<'PHP'
$report = gameshark_trace_report();
var_dump(str_contains($report, "\033[1;36mstrlen"));
var_dump(str_contains($report, "\033[1;33mneedle"));
PHP)
);
passthru($cmd);
?>
--CLEAN--
<?php
$db = __DIR__ . '/gameshark_human_trace_format.sqlite';
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
