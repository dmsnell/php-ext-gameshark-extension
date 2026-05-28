--TEST--
gameshark invariant mode runs function, instance method, and static method hooks
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
$config = __DIR__ . '/gameshark_invariants_basic_config.php';
file_put_contents($config, <<<'PHP'
<?php
function gs_invariant_record(string $message): void {
    echo $message, "\n";
}

return [
    [
        'id' => 'function-pre',
        'target' => 'target_function',
        'when' => 'pre',
        'hook' => static function (int $value): void {
            gs_invariant_record('pre:' . $GLOBALS['prefix'] . ':' . $value);
        },
    ],
    [
        'id' => 'method-post',
        'target' => 'Widget::save',
        'when' => 'post',
        'hook' => static function (Widget $widget, $return, array $args): void {
            gs_invariant_record('post:' . $widget->name . ':' . $args[0]);
            if ($return !== $widget->name . ':' . $args[0]) {
                throw new RuntimeException('bad return');
            }
        },
    ],
    [
        'id' => 'static-pre',
        'target' => 'Widget::check',
        'when' => 'pre',
        'hook' => static function (string $value): void {
            gs_invariant_record('static:' . $value);
        },
    ],
];
PHP);

$php = getenv('TEST_PHP_EXECUTABLE_ESCAPED');
$ext = realpath(__DIR__ . '/../modules/gameshark.so');
$code = <<<'PHP'
$GLOBALS['prefix'] = 'main';

function target_function(int $value): int {
    echo "function body\n";
    return $value;
}

class Widget {
    public function __construct(public string $name) {}
    public function save(string $value): string {
        echo "method body\n";
        return $this->name . ':' . $value;
    }
    public static function check(string $value): string {
        echo "static body\n";
        return strtoupper($value);
    }
}

target_function(5);
(new Widget('ok'))->save('item');
echo Widget::check('abc'), "\n";

$status = gameshark_invariants_status();
echo 'status:', $status['spec_count'], ':', $status['matched_count'], ':', $status['unmatched_count'], "\n";
foreach ($status['specs'] as $spec) {
    echo $spec['id'], ':', $spec['matched'] ? 'yes' : 'no', ':', $spec['executions'], "\n";
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
@unlink(__DIR__ . '/gameshark_invariants_basic_config.php');
?>
--EXPECT--
pre:main:5
function body
method body
post:ok:item
static:abc
static body
ABC
status:3:3:0
function-pre:yes:1
method-post:yes:1
static-pre:yes:1
