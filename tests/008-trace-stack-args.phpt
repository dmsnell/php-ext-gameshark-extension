--TEST--
gameshark trace stack frames include argument previews and structured matches
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
$db = __DIR__ . '/gameshark_trace_stack_args.sqlite';
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
    'GAMESHARK_DB=%s %s -n -d extension=%s -r %s',
    escapeshellarg($db),
    $php,
    escapeshellarg($ext),
    escapeshellarg(<<<'PHP'
$report = gameshark_trace_report("array");
$strlen = null;
foreach ($report['runs'][0]['events'] as $event) {
    if ($event['display_name'] === 'strlen') {
        $strlen = $event;
        break;
    }
}
var_dump($strlen !== null);
var_dump(str_contains($strlen['stack'][0], 'arg0="prefix needle suffix"'));
var_dump(str_contains($strlen['stack'][1], 'arg1=array(1) matches arg1["copy"]="needle"'));
echo $strlen['stack_frames'][0]['args'][0]['preview'], "\n";
echo implode(',', $strlen['stack_frames'][1]['args'][1]['matched_paths']), "\n";
echo $strlen['stack_frames'][1]['args'][1]['matches'][0]['preview'], "\n";
var_dump($strlen['stack_frames'][1]['args'][1]['contains_trace_value']);
PHP)
);
passthru($cmd);
?>
--CLEAN--
<?php
$db = __DIR__ . '/gameshark_trace_stack_args.sqlite';
@unlink($db);
@unlink($db . '-shm');
@unlink($db . '-wal');
?>
--EXPECT--
bool(true)
bool(true)
bool(true)
prefix needle suffix
arg1["copy"]
needle
bool(true)
