--TEST--
gameshark trace reports can return raw JSON with untruncated values
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
$db = __DIR__ . '/gameshark_json_report.sqlite';
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
class TraceBox {
    public string $prop;
}
function traced_sink($value, $array, $object) {
    call_user_func("strlen", $value);
}
$long = str_repeat("A", 220) . "needle" . str_repeat("B", 220);
$box = new TraceBox();
$box->prop = $long;
traced_sink($long, ["copy" => $long], $box);
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
$long = str_repeat("A", 220) . "needle" . str_repeat("B", 220);

$textReport = gameshark_trace_report();
var_dump(is_string($textReport));
var_dump(str_contains($textReport, 'Gameshark trace report'));
var_dump(is_array(gameshark_trace_report("array")));

$json = gameshark_trace_report("json");
var_dump(is_string($json));
$report = json_decode($json, true);
var_dump(json_last_error() === JSON_ERROR_NONE);

$strlen = null;
foreach ($report['runs'][0]['events'] as $event) {
    if ($event['display_name'] === 'strlen') {
        $strlen = $event;
        break;
    }
}

var_dump($strlen !== null);
var_dump($strlen['observed_value'] === $long);
var_dump($strlen['preview'] !== $long);
var_dump($strlen['stack_frames'][0]['args'][0]['value'] === $long);
var_dump($strlen['stack_frames'][1]['args'][0]['value'] === $long);
var_dump($strlen['stack_frames'][1]['args'][1]['matches'][0]['value'] === $long);
var_dump($strlen['stack_frames'][1]['args'][2]['matches'][0]['value'] === $long);

$compare = json_decode(gameshark_compare("json"), true);
var_dump(isset($compare['summary']['left_total_calls']));

try {
    gameshark_trace_report("xml");
} catch (ValueError $error) {
    echo $error->getMessage(), "\n";
}
PHP)
);
passthru($cmd);
?>
--CLEAN--
<?php
$db = __DIR__ . '/gameshark_json_report.sqlite';
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
bool(true)
bool(true)
bool(true)
gameshark report format must be "text", "array", or "json"
