--TEST--
gameshark invariant mode hooks built-in functions and methods
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
$config = __DIR__ . '/gameshark_invariants_builtins_basic_config.php';
file_put_contents($config, <<<'PHP'
<?php
return [
    [
        'id' => 'preg-pre',
        'target' => 'preg_match',
        'when' => 'pre',
        'hook' => static function ($pattern, $subject, $matches = null): void {
            echo 'preg-pre:', $pattern, ':', $subject, ':', get_debug_type($matches), "\n";
            if ($subject === 'block') {
                throw new RuntimeException('blocked preg');
            }
        },
    ],
    [
        'id' => 'preg-post',
        'target' => 'preg_match',
        'when' => 'post',
        'hook' => static function ($return, array $args): void {
            echo 'preg-post:', $return, ':', $args[0], ':', $args[1], ':', get_debug_type($args[2] ?? null), "\n";
        },
    ],
    [
        'id' => 'format-pre',
        'target' => 'DateTime::format',
        'when' => 'pre',
        'hook' => static function (DateTime $date, string $format): void {
            echo 'format-pre:', get_class($date), ':', $format, "\n";
        },
    ],
    [
        'id' => 'format-post',
        'target' => 'DateTime::format',
        'when' => 'post',
        'hook' => static function (DateTime $date, $return, array $args): void {
            echo 'format-post:', get_class($date), ':', $return, ':', $args[0], "\n";
        },
    ],
    [
        'id' => 'static-pre',
        'target' => 'DateTime::createFromFormat',
        'when' => 'pre',
        'hook' => static function (string $format, string $value, ?DateTimeZone $timezone = null): void {
            echo 'static-pre:', $format, ':', $value, ':', get_debug_type($timezone), "\n";
        },
    ],
    [
        'id' => 'str-replace-named',
        'target' => 'str_replace',
        'when' => 'pre',
        'hook' => static function ($search, $replace, $subject): void {
            echo 'str-replace-pre:', $search, ':', $replace, ':', $subject, "\n";
        },
    ],
    [
        'id' => 'array-merge-variadic',
        'target' => 'array_merge',
        'when' => 'pre',
        'hook' => static function (array ...$arrays): void {
            echo 'array-merge-pre:', count($arrays), ':', implode(',', $arrays[0]), ':', implode(',', $arrays[1]), "\n";
        },
    ],
];
PHP);

$php = getenv('TEST_PHP_EXECUTABLE_ESCAPED');
$ext = realpath(__DIR__ . '/../modules/gameshark.so');
$code = <<<'PHP'
$matches = null;
var_dump(preg_match('/a/', 'abc', $matches));
echo 'matches:', $matches[0], "\n";

$matches = ['keep'];
try {
    preg_match('/block/', 'block', $matches);
} catch (Throwable $e) {
    echo 'blocked:', $e->getMessage(), "\n";
}
echo 'blocked-matches:', implode(',', $matches), "\n";

echo str_replace(subject: 'abc', replace: 'b', search: 'a'), "\n";
$left = ['x'];
$right = ['y'];
echo implode(',', array_merge($left, $right)), "\n";

$date = new DateTime('2024-01-02 03:04:05', new DateTimeZone('UTC'));
echo $date->format('Y-m-d'), "\n";
$made = DateTime::createFromFormat('!Y-m-d', '2024-01-03', new DateTimeZone('UTC'));
echo $made->format('Y-m-d'), "\n";

$status = gameshark_invariants_status();
echo 'status:', $status['spec_count'], ':', $status['matched_count'], ':', $status['unmatched_count'], ':', $status['internal_hook_count'], ':', $status['internal_matched_count'], "\n";
echo 'internal-invocations:', $status['internal_pre_invocations'], ':', $status['internal_post_invocations'], "\n";
foreach ($status['specs'] as $spec) {
    echo $spec['id'], ':', $spec['resolved_kind'], ':', $spec['executions'], "\n";
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

$traceDb = __DIR__ . '/gameshark_invariants_builtins_trace.sqlite';
@unlink($traceDb);
$traceCode = <<<'PHP'
preg_match('/a/', 'abc');
PHP;
$traceCmd = sprintf(
    'GAMESHARK_DB=%s GAMESHARK_TRACE_VALUE=%s %s -n -d extension=%s -d gameshark.invariants=1 -d gameshark.invariants_file=%s -r %s 2>&1',
    escapeshellarg($traceDb),
    escapeshellarg('abc'),
    $php,
    escapeshellarg($ext),
    escapeshellarg($config),
    escapeshellarg($traceCode)
);
exec($traceCmd, $traceOutput, $traceStatus);
$traceText = implode("\n", $traceOutput);
echo 'trace-process:', $traceStatus === 0 ? 'ok' : 'failed', "\n";
echo 'trace-post-count:', substr_count($traceText, 'preg-post:1:/a/:abc:null'), "\n";
?>
--CLEAN--
<?php
@unlink(__DIR__ . '/gameshark_invariants_builtins_basic_config.php');
@unlink(__DIR__ . '/gameshark_invariants_builtins_trace.sqlite');
@unlink(__DIR__ . '/gameshark_invariants_builtins_trace.sqlite-shm');
@unlink(__DIR__ . '/gameshark_invariants_builtins_trace.sqlite-wal');
?>
--EXPECT--
process:ok
warning-count:1
preg-pre:/a/:abc:null
preg-post:1:/a/:abc:array
int(1)
matches:a
preg-pre:/block/:block:array
blocked:blocked preg
blocked-matches:keep
str-replace-pre:a:b:abc
bbc
array-merge-pre:2:x:y
x,y
format-pre:DateTime:Y-m-d
format-post:DateTime:2024-01-02:Y-m-d
2024-01-02
static-pre:!Y-m-d:2024-01-03:DateTimeZone
format-pre:DateTime:Y-m-d
format-post:DateTime:2024-01-03:Y-m-d
2024-01-03
status:7:7:0:7:7
internal-invocations:7:3
preg-pre:internal_function:2
preg-post:internal_function:1
format-pre:internal_method:2
format-post:internal_method:2
static-pre:internal_method:1
str-replace-named:internal_function:1
array-merge-variadic:internal_function:1
trace-process:ok
trace-post-count:1
