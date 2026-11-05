--TEST--
gameshark invariant post hooks get their own call's arguments after many calls which threw
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
$config = __DIR__ . '/gameshark_invariants_frames_after_exceptions_config.php';
file_put_contents($config, <<<'PHP'
<?php
return [
    [
        'id' => 'middle-post',
        'target' => 'middle',
        'when' => 'post',
        'hook' => static function ($return, array $args): void {
            echo 'post middle:', json_encode($args), "\n";
        },
    ],
    [
        'id' => 'fine-post',
        'target' => 'fine',
        'when' => 'post',
        'hook' => static function ($return, array $args): void {
            echo 'post fine:', json_encode($args), "\n";
        },
    ],
];
PHP);

$php = getenv('TEST_PHP_EXECUTABLE_ESCAPED');
$ext = realpath(__DIR__ . '/../modules/gameshark.so');
$code = <<<'PHP'
class Held {
    public function __destruct() {
        echo "destructed\n";
    }
}
function thrower() {
    throw new Exception('thrown');
}
function middle($value) {
    return thrower();
}
function fine($value) {
    return $value;
}
function call($callback, $value) {
    try {
        $callback($value);
    } catch (Exception $e) {
    }
}
// Calls which throw leave no frame behind, which a later call at the same address would be given.
for ($i = 0; $i < 300; $i++) {
    call('middle', 'stale');
}
call('fine', 'right');
// Nor do they keep their arguments alive.
call('middle', new Held());
echo "after\n";
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
@unlink(__DIR__ . '/gameshark_invariants_frames_after_exceptions_config.php');
?>
--EXPECT--
post fine:["right"]
destructed
after
