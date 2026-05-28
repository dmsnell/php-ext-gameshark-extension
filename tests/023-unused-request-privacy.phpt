--TEST--
gameshark unused mode stores request query strings only when explicitly enabled
--SKIPIF--
<?php
if (!getenv('TEST_PHP_EXECUTABLE_ESCAPED')) {
    die('skip TEST_PHP_EXECUTABLE_ESCAPED is unavailable');
}
if (!file_exists(__DIR__ . '/../modules/gameshark.so')) {
    die('skip gameshark module is not built');
}
$include = __DIR__ . '/../../php-src/sapi/cgi/tests/include.inc';
if (!file_exists($include)) {
    die('skip php-cgi helper is unavailable');
}
require $include;
if (!get_cgi_path()) {
    die('skip php-cgi is unavailable');
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
require __DIR__ . '/../../php-src/sapi/cgi/tests/include.inc';

$php = getenv('TEST_PHP_EXECUTABLE_ESCAPED');
$cgi = escapeshellarg(get_cgi_path());
$ext = realpath(__DIR__ . '/../modules/gameshark.so');
$script = __DIR__ . '/gameshark_unused_privacy_target.php';
$secret = 'secret-token-15648';
$query = 'token=' . $secret . '&mode=unused';
$requestUri = '/gameshark-unused-privacy.php?' . $query;

file_put_contents($script, <<<'PHP'
<?php
function privacy_uncalled_function() {}
echo "ok\n";
PHP);

function cleanup_db(string $db): void {
    @unlink($db);
    @unlink($db . '-shm');
    @unlink($db . '-wal');
}

function run_cgi_unused(string $db, bool $capture): void {
    global $cgi, $ext, $script, $requestUri, $query;
    $env = [
        'GAMESHARK_DB' => $db,
        'GAMESHARK_UNUSED' => '1',
        'REQUEST_METHOD' => 'GET',
        'SCRIPT_FILENAME' => $script,
        'PATH_TRANSLATED' => $script,
        'SCRIPT_NAME' => '/gameshark-unused-privacy.php',
        'REQUEST_URI' => $requestUri,
        'QUERY_STRING' => $query,
        'SERVER_PROTOCOL' => 'HTTP/1.1',
        'REDIRECT_STATUS' => '1',
    ];
    if ($capture) {
        $env['GAMESHARK_UNUSED_CAPTURE_QUERY'] = '1';
    }
    $prefix = '';
    foreach ($env as $name => $value) {
        $prefix .= $name . '=' . escapeshellarg($value) . ' ';
    }
    $cmd = $prefix . $cgi . ' -n -d extension=' . escapeshellarg($ext) . ' 2>&1';
    exec($cmd, $output, $status);
    if ($status !== 0) {
        echo implode("\n", $output), "\n";
        echo "status=$status\n";
    }
}

function report_for_db(string $db): array {
    global $php, $ext;
    $cmd = sprintf(
        'GAMESHARK_DB=%s %s -n -d extension=%s -r %s',
        escapeshellarg($db),
        $php,
        escapeshellarg($ext),
        escapeshellarg('echo serialize(gameshark_unused_report("array"));')
    );
    return unserialize(shell_exec($cmd));
}

$defaultDb = __DIR__ . '/gameshark_unused_privacy_default.sqlite';
$captureDb = __DIR__ . '/gameshark_unused_privacy_capture.sqlite';
cleanup_db($defaultDb);
cleanup_db($captureDb);

run_cgi_unused($defaultDb, false);
$defaultReport = report_for_db($defaultDb);
$defaultJson = json_encode($defaultReport);

run_cgi_unused($captureDb, true);
$captureReport = report_for_db($captureDb);
$captureJson = json_encode($captureReport);

var_dump($defaultReport['run']['request_path'] === '/gameshark-unused-privacy.php');
var_dump($defaultReport['run']['request_uri_full'] === null);
var_dump($defaultReport['run']['query_string'] === null);
var_dump(!str_contains($defaultJson, $secret));
var_dump($captureReport['run']['request_path'] === '/gameshark-unused-privacy.php');
var_dump($captureReport['run']['request_uri_full'] === $requestUri);
var_dump($captureReport['run']['query_string'] === $query);
var_dump(str_contains($captureJson, $secret));
?>
--CLEAN--
<?php
@unlink(__DIR__ . '/gameshark_unused_privacy_target.php');
foreach ([
    __DIR__ . '/gameshark_unused_privacy_default.sqlite',
    __DIR__ . '/gameshark_unused_privacy_capture.sqlite',
] as $db) {
    @unlink($db);
    @unlink($db . '-shm');
    @unlink($db . '-wal');
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
