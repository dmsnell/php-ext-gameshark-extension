--TEST--
gameshark invariant mode handles built-in exceptions, reentrancy, and warning suppression
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
$config = __DIR__ . '/gameshark_invariants_builtins_exception_config.php';
file_put_contents($config, <<<'PHP'
<?php
return [
    [
        'id' => 'json-pre',
        'target' => 'json_decode',
        'when' => 'pre',
        'hook' => static function (string $value): void {
            if ($value === '"block"') {
                throw new RuntimeException('blocked json');
            }
        },
    ],
    [
        'id' => 'json-post',
        'target' => 'json_decode',
        'when' => 'post',
        'hook' => static function ($return, array $args): void {
            if ($args[0] === '"post"') {
                throw new RuntimeException('post json ' . $return);
            }
        },
    ],
    [
        'id' => 'intdiv-post',
        'target' => 'intdiv',
        'when' => 'post',
        'hook' => static function ($return, array $args): void {
            echo "intdiv-post\n";
        },
    ],
    [
        'id' => 'contains-pre',
        'target' => 'str_contains',
        'when' => 'pre',
        'hook' => static function (string $haystack, string $needle): void {
            echo 'contains-pre:', $haystack, ':', $needle, "\n";
            str_contains($haystack, $needle);
        },
    ],
];
PHP);

$php = getenv('TEST_PHP_EXECUTABLE_ESCAPED');
$ext = realpath(__DIR__ . '/../modules/gameshark.so');
$code = <<<'PHP'
try {
    $json = '"block"';
    json_decode($json);
    echo "json-block-ran\n";
} catch (Throwable $e) {
    echo 'pre caught:', $e->getMessage(), "\n";
}

try {
    $json = '"post"';
    echo json_decode($json), "\n";
} catch (Throwable $e) {
    echo 'post caught:', $e->getMessage(), "\n";
}

try {
    intdiv(1, 0);
} catch (Throwable $e) {
    echo 'intdiv caught:', get_class($e), "\n";
}

var_dump(str_contains('abc', 'a'));

$status = gameshark_invariants_status();
echo 'reentrant:', $status['reentrancy_suppressed'] > 0 ? 'yes' : 'no', "\n";
echo 'internal-exceptions:', $status['internal_hook_exceptions'], ':', $status['internal_original_exceptions'], "\n";
foreach ($status['specs'] as $spec) {
    echo $spec['id'], ':', $spec['resolved_kind'], ':', $spec['executions'], ':', $spec['hook_exceptions'], "\n";
}
PHP;

$cmd = sprintf(
    '%s -n -d extension=%s -d gameshark.invariants=1 -d gameshark.invariants_file=%s -r %s 2>&1',
    $php,
    escapeshellarg($ext),
    escapeshellarg($config),
    escapeshellarg($code)
);
exec($cmd, $output, $status);
$text = implode("\n", $output);
echo 'process:', $status === 0 ? 'ok' : 'failed', "\n";
echo 'warning-count:', substr_count($text, 'php-gameshark: invariant hooks include built-in PHP targets'), "\n";
foreach ($output as $line) {
    if (!str_contains($line, 'php-gameshark: invariant hooks include built-in PHP targets')) {
        echo $line, "\n";
    }
}

$suppressed_cmd = sprintf(
    '%s -n -d extension=%s -d gameshark.invariants=1 -d gameshark.invariants_warn_builtins=0 -d gameshark.invariants_file=%s -r %s 2>&1',
    $php,
    escapeshellarg($ext),
    escapeshellarg($config),
    escapeshellarg('echo "suppressed-ok\n";')
);
exec($suppressed_cmd, $suppressed_output, $suppressed_status);
$suppressed_text = implode("\n", $suppressed_output);
echo 'suppressed-process:', $suppressed_status === 0 ? 'ok' : 'failed', "\n";
echo 'suppressed-warning-count:', substr_count($suppressed_text, 'php-gameshark: invariant hooks include built-in PHP targets'), "\n";
foreach ($suppressed_output as $line) {
    if (!str_contains($line, 'php-gameshark: invariant hooks include built-in PHP targets')) {
        echo $line, "\n";
    }
}
?>
--CLEAN--
<?php
@unlink(__DIR__ . '/gameshark_invariants_builtins_exception_config.php');
?>
--EXPECT--
process:ok
warning-count:1
pre caught:blocked json
post caught:post json post
intdiv caught:DivisionByZeroError
contains-pre:abc:a
bool(true)
reentrant:yes
internal-exceptions:2:1
json-pre:internal_function:2:1
json-post:internal_function:1:1
intdiv-post:internal_function:0:0
contains-pre:internal_function:1:0
suppressed-process:ok
suppressed-warning-count:0
suppressed-ok
