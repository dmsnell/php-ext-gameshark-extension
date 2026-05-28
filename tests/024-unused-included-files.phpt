--TEST--
gameshark unused mode reports included files with no accessed declarations
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
$db = __DIR__ . '/gameshark_unused_included_files.sqlite';
@unlink($db);
@unlink($db . '-shm');
@unlink($db . '-wal');

$php = getenv('TEST_PHP_EXECUTABLE_ESCAPED');
$ext = realpath(__DIR__ . '/../modules/gameshark.so');
$suffix = getmypid();
$usedFile = __DIR__ . "/gameshark_used_include_$suffix.php";
$unusedFile = __DIR__ . "/gameshark_unused_include_$suffix.php";
$sideEffectFile = __DIR__ . "/gameshark_side_effect_include_$suffix.php";

file_put_contents($usedFile, <<<'PHP'
<?php
function gs_included_used_fn() {}
class GSIncludedUsedClass {
    public const HIT = 'hit';
    public function hit() {}
}
define('GS_INCLUDED_USED_CONST', 'used');
PHP);

file_put_contents($unusedFile, <<<'PHP'
<?php
function gs_included_unused_fn() {}
class GSIncludedUnusedClass {
    public const COLD = 'cold';
    public function cold() {}
}
define('GS_INCLUDED_UNUSED_CONST', 'unused');
PHP);

file_put_contents($sideEffectFile, <<<'PHP'
<?php
$GLOBALS['gs_side_effect_include_ran'] = true;
PHP);

$code = sprintf(
    <<<'PHP'
require %s;
require %s;
require %s;
gs_included_used_fn();
$object = new GSIncludedUsedClass();
$object->hit();
$constant = GS_INCLUDED_USED_CONST;
if (empty($GLOBALS['gs_side_effect_include_ran'])) {
    throw new RuntimeException('side effect include did not run');
}
PHP,
    var_export($usedFile, true),
    var_export($unusedFile, true),
    var_export($sideEffectFile, true)
);

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
$report = unserialize(shell_exec($cmd));

function row_by_file(array $rows, string $file): ?array {
    foreach ($rows as $row) {
        if ($row['file'] === $file) {
            return $row;
        }
    }
    return null;
}

$unusedRow = row_by_file($report['included_files_with_no_accessed_declarations'], realpath($unusedFile));
$sideEffectRow = row_by_file($report['included_files_without_declarations'], realpath($sideEffectFile));

var_dump(isset($report['included_files_with_no_accessed_declarations']));
var_dump(isset($report['included_files_without_declarations']));
var_dump($report['summary']['included_file_count'] === 3);
var_dump(row_by_file($report['included_files_with_no_accessed_declarations'], realpath($usedFile)) === null);
var_dump(row_by_file($report['included_files_with_no_accessed_declarations'], realpath($sideEffectFile)) === null);
var_dump(row_by_file($report['included_files_without_declarations'], realpath($usedFile)) === null);
var_dump($unusedRow !== null);
var_dump($unusedRow['declaration_count'] === 5);
var_dump($unusedRow['accessed_declaration_count'] === 0);
var_dump($unusedRow['function_declaration_count'] === 1);
var_dump($unusedRow['method_declaration_count'] === 1);
var_dump($unusedRow['class_declaration_count'] === 1);
var_dump($unusedRow['global_constant_declaration_count'] === 1);
var_dump($unusedRow['class_constant_declaration_count'] === 1);
var_dump($sideEffectRow !== null);
var_dump($sideEffectRow['declaration_count'] === 0);
var_dump($sideEffectRow['include_count'] === 1);

$primaryDb = __DIR__ . '/gameshark_unused_primary_files.sqlite';
@unlink($primaryDb);
@unlink($primaryDb . '-shm');
@unlink($primaryDb . '-wal');
$primaryFile = __DIR__ . "/gameshark_primary_entry_$suffix.php";
$primaryIncludeFile = __DIR__ . "/gameshark_primary_include_$suffix.php";

file_put_contents($primaryIncludeFile, <<<'PHP'
<?php
function gs_primary_include_unused_fn() {}
PHP);

file_put_contents($primaryFile, '<?php require ' . var_export($primaryIncludeFile, true) . ';');

$cmd = sprintf(
    'GAMESHARK_DB=%s GAMESHARK_UNUSED=1 %s -n -d extension=%s %s 2>&1',
    escapeshellarg($primaryDb),
    $php,
    escapeshellarg($ext),
    escapeshellarg($primaryFile)
);
exec($cmd, $output, $status);
if ($status !== 0 || $output) {
    echo implode("\n", $output), "\n";
    echo "status=$status\n";
}

$cmd = sprintf(
    'GAMESHARK_DB=%s %s -n -d extension=%s -r %s',
    escapeshellarg($primaryDb),
    $php,
    escapeshellarg($ext),
    escapeshellarg('echo serialize(gameshark_unused_report("array"));')
);
$primaryReport = unserialize(shell_exec($cmd));

var_dump($primaryReport['summary']['included_file_count'] === 1);
var_dump(row_by_file($primaryReport['included_files_with_no_accessed_declarations'], realpath($primaryIncludeFile)) !== null);
var_dump(row_by_file($primaryReport['included_files_with_no_accessed_declarations'], realpath($primaryFile)) === null);
var_dump(row_by_file($primaryReport['included_files_without_declarations'], realpath($primaryFile)) === null);
?>
--CLEAN--
<?php
foreach (glob(__DIR__ . '/gameshark_*_include_*.php') as $file) {
    @unlink($file);
}
foreach (glob(__DIR__ . '/gameshark_*_entry_*.php') as $file) {
    @unlink($file);
}
$db = __DIR__ . '/gameshark_unused_included_files.sqlite';
@unlink($db);
@unlink($db . '-shm');
@unlink($db . '-wal');
$db = __DIR__ . '/gameshark_unused_primary_files.sqlite';
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
bool(true)
bool(true)
bool(true)
bool(true)
