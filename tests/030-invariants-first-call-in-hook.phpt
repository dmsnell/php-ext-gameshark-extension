--TEST--
gameshark invariant post hooks run for a function whose first call was made from a hook
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
$config = __DIR__ . '/gameshark_invariants_first_call_in_hook_config.php';
file_put_contents($config, <<<'PHP'
<?php
function helper($value) {
    return $value;
}
return [
    [
        'id' => 'outer-pre',
        'target' => 'outer',
        'when' => 'pre',
        'hook' => static function ($value): void {
            helper('from a hook');
        },
    ],
    [
        'id' => 'helper-pre',
        'target' => 'helper',
        'when' => 'pre',
        'hook' => static function ($value): void {
            echo 'pre:', $value, "\n";
        },
    ],
    [
        'id' => 'helper-post',
        'target' => 'helper',
        'when' => 'post',
        'hook' => static function ($return, array $args): void {
            echo 'post:', $return, "\n";
        },
    ],
];
PHP);

$php = getenv('TEST_PHP_EXECUTABLE_ESCAPED');
$ext = realpath(__DIR__ . '/../modules/gameshark.so');
$code = <<<'PHP'
function outer($value) {
    return $value;
}
outer(1);
helper('direct');
helper('again');
$status = gameshark_invariants_status();
echo 'reentrant:', $status['reentrancy_suppressed'], "\n";
PHP;

$cmd = sprintf(
    '%s -n -d extension=%s -d gameshark.invariants=1 -d gameshark.invariants_file=%s -r %s',
    $php,
    escapeshellarg($ext),
    escapeshellarg($config),
    escapeshellarg($code)
);
passthru($cmd);
?>
--CLEAN--
<?php
@unlink(__DIR__ . '/gameshark_invariants_first_call_in_hook_config.php');
?>
--EXPECT--
pre:direct
post:direct
pre:again
post:again
reentrant:1
