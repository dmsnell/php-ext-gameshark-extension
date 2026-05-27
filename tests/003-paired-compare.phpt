--TEST--
gameshark compares left and right slots
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
$db = __DIR__ . '/gameshark_pair.sqlite';
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

run_side('left', 'function common_fn(){} function left_only_fn(){} common_fn(); left_only_fn(); left_only_fn();');
run_side('right', 'function common_fn(){} function right_only_fn(){} common_fn(); common_fn(); right_only_fn();');

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
foreach ($compare['changed'] as $row) {
    if ($row['display_name'] === 'common_fn') {
        echo $row['display_name'], ':', $row['left_count'], ':', $row['right_count'], ':', $row['delta'], "\n";
    }
}
PHP)
);
passthru($cmd);
?>
--CLEAN--
<?php
$db = __DIR__ . '/gameshark_pair.sqlite';
@unlink($db);
@unlink($db . '-shm');
@unlink($db . '-wal');
?>
--EXPECT--
left_only_fn
right_only_fn
common_fn:1:2:1
