--TEST--
gameshark invariant mode exceptions break execution at pre and post hooks
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
$config = __DIR__ . '/gameshark_invariants_exception_config.php';
file_put_contents($config, <<<'PHP'
<?php
return [
    [
        'id' => 'pre-blocks-negative',
        'target' => 'pre_target',
        'when' => 'pre',
        'hook' => static function (int $value): void {
            if ($value < 0) {
                throw new RuntimeException('negative blocked');
            }
        },
    ],
    [
        'id' => 'post-replaces-return',
        'target' => 'post_target',
        'when' => 'post',
        'hook' => static function ($return, array $args): void {
            throw new RuntimeException('bad return ' . $return . ' from ' . $args[0]);
        },
    ],
];
PHP);

$php = getenv('TEST_PHP_EXECUTABLE_ESCAPED');
$ext = realpath(__DIR__ . '/../modules/gameshark.so');
$code = <<<'PHP'
$pre_ran = false;

function pre_target(int $value): void {
    global $pre_ran;
    $pre_ran = true;
    echo "pre body\n";
}

function post_target(string $value): string {
    echo "post body\n";
    return 'ret:' . $value;
}

try {
    pre_target(-1);
} catch (Throwable $e) {
    echo 'pre caught:', $e->getMessage(), "\n";
}
var_dump($pre_ran);

try {
    echo post_target('x'), "\n";
} catch (Throwable $e) {
    echo 'post caught:', $e->getMessage(), "\n";
}
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
@unlink(__DIR__ . '/gameshark_invariants_exception_config.php');
?>
--EXPECT--
pre caught:negative blocked
bool(false)
post body
post caught:bad return ret:x from x
