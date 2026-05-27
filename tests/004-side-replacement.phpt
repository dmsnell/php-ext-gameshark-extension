--TEST--
gameshark replaces only the selected side
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
$db = __DIR__ . '/gameshark_replace.sqlite';
@unlink($db);
@unlink($db . '-shm');
@unlink($db . '-wal');

$php = getenv('TEST_PHP_EXECUTABLE_ESCAPED');
$ext = realpath(__DIR__ . '/../modules/gameshark.so');

function run_side(string $side, string $code): void {
    global $db, $php, $ext;
    $cmd = sprintf(
        'GAMESHARK_DB=%s GAMESHARK_SIDE=%s %s -n -d extension=%s -r %s 2>&1',
        escapeshellarg($db),
        escapeshellarg($side),
        $php,
        escapeshellarg($ext),
        escapeshellarg($code)
    );
    exec($cmd, $output, $status);
    if ($status !== 0) {
        echo implode("\n", $output), "\n";
        echo "status=$status\n";
    }
}

run_side('left', 'function old_left_fn(){} old_left_fn();');
run_side('right', 'function right_stays_fn(){} right_stays_fn();');
run_side('left', 'function new_left_fn(){} new_left_fn();');

$cmd = sprintf(
    'GAMESHARK_DB=%s %s -n -d extension=%s -r %s',
    escapeshellarg($db),
    $php,
    escapeshellarg($ext),
    escapeshellarg(<<<'PHP'
$compare = gameshark_compare();
$left = array_column($compare['left_only'], 'display_name');
$right = array_column($compare['right_only'], 'display_name');
sort($left);
sort($right);
echo implode(',', $left), "\n";
echo implode(',', $right), "\n";
var_dump(in_array('old_left_fn', $left, true));
PHP)
);
passthru($cmd);
?>
--CLEAN--
<?php
$db = __DIR__ . '/gameshark_replace.sqlite';
@unlink($db);
@unlink($db . '-shm');
@unlink($db . '-wal');
?>
--EXPECT--
new_left_fn
right_stays_fn
bool(false)
