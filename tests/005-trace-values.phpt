--TEST--
gameshark traces string values through user, builtin, array, and object arguments
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
$db = __DIR__ . '/gameshark_trace_values.sqlite';
@unlink($db);
@unlink($db . '-shm');
@unlink($db . '-wal');

$php = getenv('TEST_PHP_EXECUTABLE_ESCAPED');
$ext = realpath(__DIR__ . '/../modules/gameshark.so');

$code = <<<'PHP'
class MagicBox {
    public string $visible = "object needle value";
    public function __get(string $name) {
        echo "__get called\n";
        return "needle";
    }
}
function accepts($value) {}
accepts("before needle after");
array_search("needle", ["needle"]);
accepts(["post" => ["title" => "nested needle value"]]);
accepts(new MagicBox());
PHP;

$cmd = sprintf(
    'GAMESHARK_DB=%s GAMESHARK_TRACE_VALUE=%s %s -n -d extension=%s -r %s 2>&1',
    escapeshellarg($db),
    escapeshellarg('needle'),
    $php,
    escapeshellarg($ext),
    escapeshellarg($code)
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
echo $report['summary']['run_count'], ':', $report['summary']['event_count'], "\n";
foreach ($report['runs'][0]['events'] as $event) {
    echo $event['display_name'], '|', $event['argument_path'], '|', $event['match_kind'], '|', $event['preview'], '|', count($event['stack']), "\n";
}
PHP)
);
passthru($cmd);
?>
--CLEAN--
<?php
$db = __DIR__ . '/gameshark_trace_values.sqlite';
@unlink($db);
@unlink($db . '-shm');
@unlink($db . '-wal');
?>
--EXPECT--
1:5
accepts|arg0|string_contains|before needle after|2
array_search|arg0|string_contains|needle|2
array_search|arg1[0]|string_contains|needle|2
accepts|arg0["post"]["title"]|string_contains|nested needle value|2
accepts|arg0->visible|string_contains|object needle value|2
