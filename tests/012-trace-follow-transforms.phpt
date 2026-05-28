--TEST--
gameshark trace follow transforms records escaped return values and traces them later
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
$db = __DIR__ . '/gameshark_trace_follow_transforms.sqlite';
@unlink($db);
@unlink($db . '-shm');
@unlink($db . '-wal');

$php = getenv('TEST_PHP_EXECUTABLE_ESCAPED');
$ext = realpath(__DIR__ . '/../modules/gameshark.so');

$cmd = sprintf(
    'GAMESHARK_DB=%s GAMESHARK_TRACE_VALUE=%s GAMESHARK_TRACE_FOLLOW_TRANSFORMS=1 %s -n -d extension=%s -r %s 2>&1',
    escapeshellarg($db),
    escapeshellarg("O'Reilly"),
    $php,
    escapeshellarg($ext),
    escapeshellarg(<<<'PHP'
function escape_slashes($value) {
    return addslashes($value);
}
function escape_sql($value) {
    return "WHERE title = '" . str_replace("'", "''", $value) . "'";
}
function sink($value) {}

$source = "O'Reilly";
$slash = escape_slashes($source);
$sql = escape_sql($source);
sink($slash);
sink($sql);
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
$run = $report['runs'][0];

echo $report['summary']['run_count'], ':', $report['summary']['transformed_value_count'], "\n";

$values = [];
foreach ($run['transformed_values'] as $value) {
    $values[$value['transform_kind']] = $value['value'];
}
var_dump($values['addslashes'] ?? null);
var_dump($values['sql_quote_doubling'] ?? null);

$sinkMatches = [];
foreach ($run['events'] as $event) {
    if ($event['display_name'] === 'sink') {
        $sinkMatches[] = $event['matched_value'];
    }
}
sort($sinkMatches);
echo implode("\n", $sinkMatches), "\n";
PHP)
);
passthru($cmd);
?>
--CLEAN--
<?php
$db = __DIR__ . '/gameshark_trace_follow_transforms.sqlite';
@unlink($db);
@unlink($db . '-shm');
@unlink($db . '-wal');
?>
--EXPECT--
1:2
string(9) "O\'Reilly"
string(9) "O''Reilly"
O''Reilly
O\'Reilly
