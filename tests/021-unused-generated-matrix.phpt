--TEST--
gameshark unused mode handles generated dynamic-call and construct matrices
--SKIPIF--
<?php
if (!getenv('TEST_PHP_EXECUTABLE_ESCAPED')) {
    die('skip TEST_PHP_EXECUTABLE_ESCAPED is unavailable');
}
if (!file_exists(__DIR__ . '/../modules/gameshark.so')) {
    die('skip gameshark module is not built');
}
$disabled = array_map('trim', explode(',', ini_get('disable_functions')));
foreach (['exec', 'shell_exec'] as $function) {
    if (in_array($function, $disabled, true)) {
        die("skip $function is disabled");
    }
}
?>
--FILE--
<?php
$php = getenv('TEST_PHP_EXECUTABLE_ESCAPED');
$ext = realpath(__DIR__ . '/../modules/gameshark.so');
$dbs = [];

function run_unused_matrix(string $name, string $code): array {
    global $php, $ext, $dbs;
    $db = __DIR__ . "/gameshark_unused_matrix_$name.sqlite";
    $dbs[] = $db;
    @unlink($db);
    @unlink($db . '-shm');
    @unlink($db . '-wal');

    $cmd = sprintf(
        'GAMESHARK_DB=%s GAMESHARK_UNUSED=1 %s -n -d extension=%s -r %s 2>&1',
        escapeshellarg($db),
        $php,
        escapeshellarg($ext),
        escapeshellarg($code)
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
        escapeshellarg('echo serialize(gameshark_unused_report("array"));')
    );
    $serialized = shell_exec($cmd);
    return unserialize($serialized);
}

function names(array $rows): array {
    $names = array_column($rows, 'display_name');
    sort($names);
    return $names;
}

function row_by_name(array $rows, string $name): ?array {
    foreach ($rows as $row) {
        if ($row['display_name'] === $name) {
            return $row;
        }
    }
    return null;
}

$dynamic = run_unused_matrix('dynamic', <<<'PHP'
function matrix_live_dynamic() {}
function matrix_cold_dynamic() {}
function matrix_live_call_user_func() {}
function matrix_cold_call_user_func() {}
class MatrixDynamicClass {
    public function liveMethod() {}
    public function coldMethod() {}
    public function liveCallUserFuncMethod() {}
    public function coldCallUserFuncMethod() {}
}
$fn = 'matrix_live_dynamic';
$fn();
call_user_func('matrix_live_call_user_func');
$object = new MatrixDynamicClass();
$method = 'liveMethod';
$object->$method();
call_user_func([$object, 'liveCallUserFuncMethod']);
PHP);

$dynamicFunctions = names($dynamic['uncalled_functions']);
$dynamicMethods = names($dynamic['uncalled_concrete_methods']);
var_dump(in_array('matrix_cold_dynamic', $dynamicFunctions, true));
var_dump(!in_array('matrix_live_dynamic', $dynamicFunctions, true));
var_dump(in_array('matrix_cold_call_user_func', $dynamicFunctions, true));
var_dump(!in_array('matrix_live_call_user_func', $dynamicFunctions, true));
var_dump(in_array('MatrixDynamicClass::coldMethod', $dynamicMethods, true));
var_dump(!in_array('MatrixDynamicClass::liveMethod', $dynamicMethods, true));
var_dump(in_array('MatrixDynamicClass::coldCallUserFuncMethod', $dynamicMethods, true));
var_dump(!in_array('MatrixDynamicClass::liveCallUserFuncMethod', $dynamicMethods, true));

$constants = run_unused_matrix('constants', <<<'PHP'
class MatrixConstClass {
    public const HIT = 'hit';
    public const MISS = 'miss';
}
define('MATRIX_HIT_CONST', 'hit');
define('MATRIX_PROBED_CONST', 'probed');
$classFile = sys_get_temp_dir() . '/gameshark_matrix_direct_const_' . getmypid() . '.php';
file_put_contents($classFile, <<<'PHP_FILE'
<?php
class MatrixDirectConstClass {
    public const HIT = 'hit';
    public const MISS = 'miss';
}
PHP_FILE);
require $classFile;
@unlink($classFile);
constant('MatrixConstClass::HIT');
constant('MATRIX_HIT_CONST');
defined('MATRIX_PROBED_CONST');
defined('MatrixConstClass::MISS');
try {
    $late = MATRIX_LATE_CONST;
} catch (Throwable $e) {
}
define('MATRIX_LATE_CONST', 'late');
defined('MATRIX_LATE_PROBED_CONST');
define('MATRIX_LATE_PROBED_CONST', 'late-probed');
$directHit = MatrixDirectConstClass::HIT;
PHP);

$globalProbe = row_by_name($constants['global_constants_without_read_observed'], 'MATRIX_PROBED_CONST');
$lateFetch = row_by_name($constants['global_constants_without_read_observed'], 'MATRIX_LATE_CONST');
$lateProbe = row_by_name($constants['global_constants_without_read_observed'], 'MATRIX_LATE_PROBED_CONST');
$classProbe = row_by_name($constants['class_constants_without_read_observed'], 'MatrixConstClass::MISS');
$directHit = row_by_name($constants['class_constants_without_read_observed'], 'MatrixDirectConstClass::HIT');
$classConstants = names($constants['class_constants_without_read_observed']);
var_dump($globalProbe !== null);
var_dump($globalProbe['defined_probe_count'] === 1);
var_dump(row_by_name($constants['global_constants_without_read_observed'], 'MATRIX_HIT_CONST') === null);
var_dump($lateFetch !== null);
var_dump($lateFetch['fetch_observed_count'] === 1);
var_dump($lateFetch['read_observed_count'] === 0);
var_dump($lateProbe !== null);
var_dump($lateProbe['defined_probe_count'] === 1);
var_dump($classProbe !== null);
var_dump($classProbe['defined_probe_count'] === 1);
var_dump(!in_array('MatrixConstClass::HIT', $classConstants, true));
var_dump(in_array('MatrixDirectConstClass::MISS', $classConstants, true));
var_dump($directHit !== null);
var_dump($directHit['fetch_observed_count'] === 1);

$constructs = run_unused_matrix('constructs', <<<'PHP'
abstract class MatrixAbstractBase {
    public function inheritedConcrete() {}
    abstract public function requiredMethod();
}
interface MatrixInterface {
    public function interfaceMethod();
}
trait MatrixTrait {
    public function traitOnly() {}
}
enum MatrixEnum {
    case CaseA;
}
class MatrixConcrete {
    use MatrixTrait;
    public function usedMethod() {}
    public function unusedMethod() {}
}
$object = new MatrixConcrete();
$object->usedMethod();
PHP);

$constructClasses = names($constructs['classes_with_no_new_opcode_observed']);
$constructMethods = names($constructs['uncalled_concrete_methods']);
var_dump(!in_array('MatrixAbstractBase', $constructClasses, true));
var_dump(!in_array('MatrixInterface', $constructClasses, true));
var_dump(!in_array('MatrixTrait', $constructClasses, true));
var_dump(!in_array('MatrixEnum', $constructClasses, true));
var_dump(in_array('MatrixConcrete::unusedMethod', $constructMethods, true));
var_dump(!in_array('MatrixConcrete::usedMethod', $constructMethods, true));
?>
--CLEAN--
<?php
foreach (glob(__DIR__ . '/gameshark_unused_matrix_*.sqlite*') as $file) {
    @unlink($file);
}
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
