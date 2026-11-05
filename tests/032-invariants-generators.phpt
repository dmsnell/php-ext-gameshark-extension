--TEST--
gameshark invariant hooks run once for a call to a generator, and not when it is resumed
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
$config = __DIR__ . '/gameshark_invariants_generators_config.php';
file_put_contents($config, <<<'PHP'
<?php
return [
    [
        'id' => 'gen-pre',
        'target' => 'gen',
        'when' => 'pre',
        'hook' => static function (...$args): void {
            echo 'pre:', json_encode($args), "\n";
        },
    ],
    [
        'id' => 'gen-post',
        'target' => 'gen',
        'when' => 'post',
        'hook' => static function ($return, array $args): void {
            echo 'post:', json_encode($return), "\n";
        },
    ],
    [
        'id' => 'all-pre',
        'target' => '*',
        'when' => 'pre',
        'hook' => static function (string $function, array $args): void {
            echo 'all:', $function, "\n";
        },
    ],
    [
        'id' => 'fine-post',
        'target' => 'fine',
        'when' => 'post',
        'hook' => static function ($return, array $args): void {
            echo 'post fine:', $return, "\n";
        },
    ],
];
PHP);

$php = getenv('TEST_PHP_EXECUTABLE_ESCAPED');
$ext = realpath(__DIR__ . '/../modules/gameshark.so');
$code = <<<'PHP'
function gen($n) {
    for ($i = 0; $i < $n; $i++) {
        yield $i;
    }
    return 'done';
}
function fine() {
    return 'fine';
}
$generator = gen(2);
foreach ($generator as $value) {
    echo 'got:', $value, "\n";
}
// Generators which are never finished leave nothing behind for later calls.
for ($i = 0; $i < 200; $i++) {
    gen(1);
}
fine();
PHP;

$cmd = sprintf(
    '%s -n -d extension=%s -d gameshark.invariants=1 -d gameshark.invariants_file=%s -r %s',
    $php,
    escapeshellarg($ext),
    escapeshellarg($config),
    escapeshellarg($code)
);
$output = shell_exec($cmd);
// The 200 calls print the same two lines each.
echo preg_replace('/(pre:\[1\]\nall:gen\n)+/', "(pre and all for each of the 200 calls)\n", $output);
?>
--CLEAN--
<?php
@unlink(__DIR__ . '/gameshark_invariants_generators_config.php');
?>
--EXPECT--
pre:[2]
all:gen
got:0
got:1
(pre and all for each of the 200 calls)
all:fine
post fine:fine
