--TEST--
gameshark unused mode reports declarations without observed runtime access
--SKIPIF--
<?php
if (!getenv('TEST_PHP_EXECUTABLE_ESCAPED')) {
    die('skip TEST_PHP_EXECUTABLE_ESCAPED is unavailable');
}
if (!file_exists(__DIR__ . '/../modules/gameshark.so')) {
    die('skip gameshark module is not built');
}
$disabled = array_map('trim', explode(',', ini_get('disable_functions')));
foreach (['exec', 'passthru'] as $function) {
    if (in_array($function, $disabled, true)) {
        die("skip $function is disabled");
    }
}
?>
--FILE--
<?php
$db = __DIR__ . '/gameshark_unused_basic.sqlite';
@unlink($db);
@unlink($db . '-shm');
@unlink($db . '-wal');

$php = getenv('TEST_PHP_EXECUTABLE_ESCAPED');
$ext = realpath(__DIR__ . '/../modules/gameshark.so');

$cmd = sprintf(
    'GAMESHARK_DB=%s GAMESHARK_UNUSED=1 %s -n -d extension=%s -r %s 2>&1',
    escapeshellarg($db),
    $php,
    escapeshellarg($ext),
    escapeshellarg(<<<'PHP'
function used_fn() {}
function unused_fn() {}
function dynamic_fn() {}

class UsedClass {
    public const HIT = 'hit';
    public const MISSED = 'missed';

    public function usedMethod() {}
    public function unusedMethod() {}
}

class NeverNew {}
abstract class AbstractBase { public function inherited() {} }
interface ExampleInterface { public function ifaceMethod(); }
trait ExampleTrait { public function traitMethod() {} }
enum ExampleEnum { case CaseA; }

define('USED_GLOBAL', 'used');
define('PROBED_GLOBAL', 'probed');

used_fn();
$dynamic = 'dynamic_fn';
$dynamic();
$object = new UsedClass();
$object->usedMethod();
$hit = constant('UsedClass::HIT');
$used = USED_GLOBAL;
$usedAgain = constant('USED_GLOBAL');
defined('PROBED_GLOBAL');
PHP)
);
exec($cmd, $output, $status);
if ($status !== 0 || $output) {
    echo implode("\n", $output), "\n";
    echo "status=$status\n";
}

$cmd = sprintf(
    'GAMESHARK_DB=%s %s -n -d extension=%s -r %s',
    escapeshellarg($db),
    $php,
    escapeshellarg($ext),
    escapeshellarg(<<<'PHP'
$report = gameshark_unused_report("array");

$functions = array_column($report['uncalled_functions'], 'display_name');
$methods = array_column($report['uncalled_concrete_methods'], 'display_name');
$classes = array_column($report['classes_with_no_new_opcode_observed'], 'display_name');
$globals = [];
foreach ($report['global_constants_without_read_observed'] as $row) {
    $globals[$row['display_name']] = $row;
}
$classConstants = array_column($report['class_constants_without_read_observed'], 'display_name');

sort($functions);
sort($methods);
sort($classes);
sort($classConstants);

var_dump(in_array('unused_fn', $functions, true));
var_dump(!in_array('used_fn', $functions, true));
var_dump(!in_array('dynamic_fn', $functions, true));
var_dump(in_array('UsedClass::unusedMethod', $methods, true));
var_dump(!in_array('UsedClass::usedMethod', $methods, true));
var_dump(in_array('NeverNew', $classes, true));
var_dump(!in_array('UsedClass', $classes, true));
var_dump(!in_array('AbstractBase', $classes, true));
var_dump(!in_array('ExampleInterface', $classes, true));
var_dump(!in_array('ExampleEnum', $classes, true));
var_dump(isset($globals['PROBED_GLOBAL']));
var_dump($globals['PROBED_GLOBAL']['defined_probe_count'] === 1);
var_dump(!isset($globals['USED_GLOBAL']));
var_dump(in_array('UsedClass::MISSED', $classConstants, true));
var_dump(!in_array('UsedClass::HIT', $classConstants, true));
var_dump(is_string(gameshark_unused_report("json")));
var_dump(str_contains(gameshark_unused_report(), 'Gameshark unused coverage report'));
PHP)
);
passthru($cmd);
?>
--CLEAN--
<?php
$db = __DIR__ . '/gameshark_unused_basic.sqlite';
@unlink($db);
@unlink($db . '-shm');
@unlink($db . '-wal');
?>
--EXPECT--
bool(true)
bool(true)
bool(true)
bool(true)
bool(true)
bool(true)
bool(true)
bool(true)
bool(true)
bool(true)
bool(true)
bool(true)
bool(true)
bool(true)
bool(true)
bool(true)
bool(true)
