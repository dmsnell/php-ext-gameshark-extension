--TEST--
gameshark invariant mode fails closed on invalid config
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
$config = __DIR__ . '/gameshark_invariants_invalid_config.php';
file_put_contents($config, <<<'PHP'
<?php
return [
    [
        'id' => 'dup',
        'target' => 'a',
        'when' => 'pre',
        'hook' => static function (): void {},
    ],
    [
        'id' => 'dup',
        'target' => 'b',
        'when' => 'pre',
        'hook' => static function (): void {},
    ],
];
PHP);

$php = getenv('TEST_PHP_EXECUTABLE_ESCAPED');
$ext = realpath(__DIR__ . '/../modules/gameshark.so');
$cmd = sprintf(
    '%s -n -d display_errors=1 -d extension=%s -d gameshark.invariants=1 -d gameshark.invariants_file=%s -r %s 2>&1',
    $php,
    escapeshellarg($ext),
    escapeshellarg($config),
    escapeshellarg('echo "should-not-run\n";')
);
exec($cmd, $output, $status);
$text = implode("\n", $output);
echo 'status:', $status === 0 ? 'ok' : 'failed', "\n";
echo str_contains($text, 'duplicate invariant id "dup"') ? "saw duplicate\n" : "missing duplicate\n";
echo str_contains($text, 'should-not-run') ? "ran\n" : "did-not-run\n";
?>
--CLEAN--
<?php
@unlink(__DIR__ . '/gameshark_invariants_invalid_config.php');
?>
--EXPECT--
status:failed
saw duplicate
did-not-run
