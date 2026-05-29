--TEST--
gameshark unused aggregate report merges completed request runs
--SKIPIF--
<?php
if (!getenv('TEST_PHP_EXECUTABLE_ESCAPED')) {
    die('skip TEST_PHP_EXECUTABLE_ESCAPED is unavailable');
}
if (!file_exists(__DIR__ . '/../modules/gameshark.so')) {
    die('skip gameshark module is not built');
}
$disabled = array_map('trim', explode(',', ini_get('disable_functions')));
foreach (['exec', 'passthru'] as $function) {
    if (in_array($function, $disabled, true)) {
        die("skip $function is disabled");
    }
}
?>
--FILE--
<?php
$db = __DIR__ . '/gameshark_unused_aggregate.sqlite';
@unlink($db);
@unlink($db . '-shm');
@unlink($db . '-wal');

$php = getenv('TEST_PHP_EXECUTABLE_ESCAPED');
$ext = realpath(__DIR__ . '/../modules/gameshark.so');

foreach ([
    <<<'PHP'
function aggregate_used_a() {}
function aggregate_later_used() {}
function aggregate_never_a() {}
aggregate_used_a();
PHP,
    <<<'PHP'
function aggregate_later_used() {}
function aggregate_never_b() {}
aggregate_later_used();
PHP
] as $code) {
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

$cmd = sprintf(
    'GAMESHARK_DB=%s %s -n -d extension=%s -r %s',
    escapeshellarg($db),
    $php,
    escapeshellarg($ext),
    escapeshellarg(<<<'PHP'
$report = gameshark_unused_aggregate_report('array');
$functions = array_column($report['uncalled_functions'], 'display_name');
sort($functions);
echo $report['summary']['run_count'], "\n";
var_dump(in_array('aggregate_never_a', $functions, true));
var_dump(in_array('aggregate_never_b', $functions, true));
var_dump(!in_array('aggregate_later_used', $functions, true));
var_dump(str_contains(gameshark_unused_aggregate_report(), 'Gameshark unused coverage report'));
$json = gameshark_unused_aggregate_report('json');
var_dump(str_contains($json, '"run_count":2'));
PHP)
);
passthru($cmd);
?>
--CLEAN--
<?php
$db = __DIR__ . '/gameshark_unused_aggregate.sqlite';
@unlink($db);
@unlink($db . '-shm');
@unlink($db . '-wal');
?>
--EXPECT--
2
bool(true)
bool(true)
bool(true)
bool(true)
bool(true)
