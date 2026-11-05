--TEST--
gameshark invariant hooks see __call() for a call routed through a trampoline, and named arguments with their defaults
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
$config = __DIR__ . '/gameshark_invariants_trampolines_config.php';
file_put_contents($config, <<<'PHP'
<?php
return [
    [
        'id' => 'all-pre',
        'target' => '*',
        'when' => 'pre',
        'hook' => static function (string $function, array $args): void {
            if (!str_contains($function . json_encode($args), 'quiet')) {
                echo 'pre:', $function, ':', json_encode($args), "\n";
            }
        },
    ],
    [
        'id' => 'all-post',
        'target' => '*',
        'when' => 'post',
        'hook' => static function (string $function, $return, array $args): void {
            if (!str_contains($function . json_encode($args), 'quiet')) {
                echo 'post:', $function, ':', json_encode($return), "\n";
            }
        },
    ],
    [
        'id' => 'named-pre',
        'target' => 'named',
        'when' => 'pre',
        'hook' => static function (...$args): void {
            echo 'named pre:', json_encode($args), "\n";
        },
    ],
];
PHP);

$php = getenv('TEST_PHP_EXECUTABLE_ESCAPED');
$ext = realpath(__DIR__ . '/../modules/gameshark.so');
$code = <<<'PHP'
class Magic {
    public function __call($name, $arguments) {
        return $name . ':' . count($arguments);
    }
    protected function hidden($value) {
        return $value;
    }
}
function named($a, $b = 'b', $c = 'c') {
    return $a . $b . $c;
}
function quiet_check() {
    return 'checked';
}
$magic = new Magic();
$magic->hidden('x');
$magic->undefined(1, 2);
named('a', c: 'z');
// Trampolines leave nothing behind for later calls.
for ($i = 0; $i < 200; $i++) {
    $magic->quiet($i);
}
echo quiet_check(), "\n";
foreach (gameshark_invariants_status()['specs'] as $spec) {
    if ('all-post' === $spec['id']) {
        echo 'posts:', $spec['executions'], "\n";
    }
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
@unlink(__DIR__ . '/gameshark_invariants_trampolines_config.php');
?>
--EXPECT--
pre:Magic::__call:["hidden",["x"]]
post:Magic::__call:"hidden:1"
pre:Magic::__call:["undefined",[1,2]]
post:Magic::__call:"undefined:2"
named pre:["a","b","z"]
pre:named:["a","b","z"]
post:named:"abz"
checked
posts:204
