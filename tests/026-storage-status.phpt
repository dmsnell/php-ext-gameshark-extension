--TEST--
gameshark reports storage configuration status and backend errors
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
$db = __DIR__ . '/gameshark_storage_status.sqlite';
@unlink($db);
@unlink($db . '-shm');
@unlink($db . '-wal');

$php = getenv('TEST_PHP_EXECUTABLE_ESCAPED');
$ext = realpath(__DIR__ . '/../modules/gameshark.so');

$cmd = sprintf(
    '%s -n -d extension=%s -r %s',
    $php,
    escapeshellarg($ext),
    escapeshellarg(<<<'PHP'
$status = gameshark_storage_status();
var_dump($status['configured']);
var_dump($status['backend']);
var_dump($status['capture']);
var_dump($status['compiled_backends']['sqlite']);
PHP)
);
passthru($cmd);

$cmd = sprintf(
    '%s -n -d extension=%s -d gameshark.dsn=%s -d gameshark.capture=%s -r %s',
    $php,
    escapeshellarg($ext),
    escapeshellarg('sqlite:' . $db),
    escapeshellarg('sample-prod'),
    escapeshellarg(<<<'PHP'
$status = gameshark_storage_status();
var_dump($status['configured']);
var_dump($status['backend']);
var_dump($status['capture']);
echo basename(gameshark_db_path()), "\n";
PHP)
);
passthru($cmd);

$cmd = sprintf(
    '%s -n -d extension=%s -d gameshark.storage=mysql -d gameshark.mysql.database=gameshark -d gameshark.db=%s -r %s',
    $php,
    escapeshellarg($ext),
    escapeshellarg($db),
    escapeshellarg(<<<'PHP'
$status = gameshark_storage_status();
echo $status['last_error']['code'], "\n";
echo $status['last_error']['backend'], "\n";
echo $status['sources']['legacy_db'], "\n";
var_dump(gameshark_db_path());
$report = gameshark_compare('array');
echo $report['error_code'], "\n";
PHP)
);
passthru($cmd);
?>
--CLEAN--
<?php
$db = __DIR__ . '/gameshark_storage_status.sqlite';
@unlink($db);
@unlink($db . '-shm');
@unlink($db . '-wal');
?>
--EXPECT--
bool(false)
NULL
string(7) "default"
bool(true)
bool(true)
string(6) "sqlite"
string(11) "sample-prod"
gameshark_storage_status.sqlite
backend_not_compiled
mysql
ignored
NULL
backend_not_compiled
