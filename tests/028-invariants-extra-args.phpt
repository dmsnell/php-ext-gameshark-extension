--TEST--
gameshark invariant hooks receive arguments beyond those a function declares
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
$config = __DIR__ . '/gameshark_invariants_extra_args_config.php';
file_put_contents($config, <<<'PHP'
<?php
return [
    [
        'id' => 'variadic-pre',
        'target' => 'variadic_target',
        'when' => 'pre',
        'hook' => static function (...$args): void {
            echo 'pre:', json_encode($args), "\n";
        },
    ],
    [
        'id' => 'variadic-post',
        'target' => 'variadic_target',
        'when' => 'post',
        'hook' => static function ($return, array $args): void {
            echo 'post:', json_encode($args), "\n";
        },
    ],
    [
        'id' => 'undeclared-pre',
        'target' => 'undeclared_target',
        'when' => 'pre',
        'hook' => static function (...$args): void {
            echo 'undeclared:', json_encode($args), "\n";
        },
    ],
];
PHP);

$php = getenv('TEST_PHP_EXECUTABLE_ESCAPED');
$ext = realpath(__DIR__ . '/../modules/gameshark.so');
$code = <<<'PHP'
function variadic_target($first, ...$rest) {
    $local = 'local';
    $other = [$first, $rest, $local];
    return count($rest);
}
function undeclared_target() {
    $local = 'local';
    return func_get_args();
}
variadic_target(1, 'two', [3]);
undeclared_target('a', 'b');
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
@unlink(__DIR__ . '/gameshark_invariants_extra_args_config.php');
?>
--EXPECT--
pre:[1,"two",[3]]
post:[1,"two",[3]]
undeclared:["a","b"]
