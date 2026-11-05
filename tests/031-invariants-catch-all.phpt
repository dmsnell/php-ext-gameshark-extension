--TEST--
gameshark catch-all invariant hooks trap every named user function in the files they are given
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
$dir = __DIR__ . '/gameshark_catch_all';
@mkdir($dir . '/lib', 0777, true);
file_put_contents($dir . '/lib/lib.php', <<<'PHP'
<?php
function lib_fn($a, ...$rest) {
    return count($rest) + 1;
}
function helper_in_lib() {
    return 'helper';
}
function lib_closure() {
    return static function ($x) {
        return $x;
    };
}
function lib_throws() {
    throw new RuntimeException('thrown');
}
class Lib_C {
    public function m($x) {
        return $x * 2;
    }
    public static function s() {
        return 's';
    }
}
PHP);

$config = $dir . '/config.php';
file_put_contents($config, sprintf(<<<'PHP'
<?php
return [
    [
        'id' => 'specific',
        'target' => 'lib_fn',
        'when' => 'pre',
        'hook' => static function (...$args): void {
            echo "specific\n";
        },
    ],
    [
        'id' => 'all-pre',
        'target' => '*',
        'when' => 'pre',
        'files' => [%s],
        'hook' => static function (string $function, array $args, ?object $self): void {
            echo 'pre:', $function, ':', json_encode($args), ':', $self ? get_class($self) : 'null', "\n";
            helper_in_lib();
        },
    ],
    [
        'id' => 'all-post',
        'target' => '*',
        'when' => 'post',
        'files' => [%s],
        'hook' => static function (string $function, $return, array $args, ?object $self): void {
            echo 'post:', $function, ':', json_encode($return), ':', json_encode($args), "\n";
        },
    ],
];
PHP, var_export($dir . '/lib/', true), var_export($dir . '/lib/', true)));

$php = getenv('TEST_PHP_EXECUTABLE_ESCAPED');
$ext = realpath(__DIR__ . '/../modules/gameshark.so');
$code = sprintf(<<<'PHP'
require %s;
function main_fn($x) {
    return $x;
}
lib_fn(1, 'a');
(new Lib_C())->m(5);
Lib_C::s();
$closure = lib_closure();
$closure(1);
main_fn(1);
strlen('x');
try {
    lib_throws();
} catch (RuntimeException $e) {
    echo "caught\n";
}
$status = gameshark_invariants_status();
foreach ($status['specs'] as $spec) {
    echo $spec['id'], ':', $spec['target_kind'], ':', $spec['resolved_kind'], ':', $spec['executions'], ':', count($spec['files'] ?? []), "\n";
}
PHP, var_export($dir . '/lib/lib.php', true));

$cmd = sprintf(
    '%s -n -d extension=%s -d gameshark.invariants=1 -d gameshark.invariants_file=%s -r %s',
    $php,
    escapeshellarg($ext),
    escapeshellarg($config),
    escapeshellarg($code)
);
passthru($cmd);

file_put_contents($config, <<<'PHP'
<?php
return [
    [
        'id' => 'named-files',
        'target' => 'lib_fn',
        'when' => 'pre',
        'files' => ['/tmp/'],
        'hook' => static function (): void {},
    ],
];
PHP);
$cmd = sprintf(
    '%s -n -d extension=%s -d gameshark.invariants=1 -d gameshark.invariants_file=%s -r %s',
    $php,
    escapeshellarg($ext),
    escapeshellarg($config),
    escapeshellarg('echo "loaded\n";')
);
$output = shell_exec($cmd . ' 2>&1');
echo str_contains($output, 'spec 0 files must be a list of non-empty path prefixes, for the target "*"') ? 'rejected' : $output, "\n";
?>
--CLEAN--
<?php
$dir = __DIR__ . '/gameshark_catch_all';
@unlink($dir . '/lib/lib.php');
@unlink($dir . '/config.php');
@rmdir($dir . '/lib');
@rmdir($dir);
?>
--EXPECT--
specific
pre:lib_fn:[1,"a"]:null
post:lib_fn:2:[1,"a"]
pre:Lib_C::m:[5]:Lib_C
post:Lib_C::m:10:[5]
pre:Lib_C::s:[]:null
post:Lib_C::s:"s":[]
pre:lib_closure:[]:null
post:lib_closure:{}:[]
pre:lib_throws:[]:null
caught
specific:function:user_function:1:0
all-pre:all:all_user_functions:5:1
all-post:all:all_user_functions:4:1
rejected
