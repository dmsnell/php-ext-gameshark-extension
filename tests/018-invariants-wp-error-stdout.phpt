--TEST--
gameshark invariant mode logs WP_Error returns from get_post with ANSI color
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
$config = __DIR__ . '/gameshark_invariants_wp_error_stdout_config.php';
file_put_contents($config, <<<'PHP'
<?php
return [
    [
        'id' => 'wp-rest-get-post-wp-error',
        'target' => 'WP_REST_Posts_Controller::get_post',
        'when' => 'post',
        'hook' => static function ($controller, $return, array $args): void {
            if (!($return instanceof WP_Error)) {
                return;
            }

            $user = function_exists('wp_get_current_user') ? wp_get_current_user() : null;
            $display_name = is_object($user) && isset($user->display_name) ? trim((string) $user->display_name) : '';
            $user_login = is_object($user) && isset($user->user_login) ? (string) $user->user_login : '';
            $user_label = $display_name !== '' ? $display_name : ($user_login !== '' ? $user_login : 'anonymous');
            $user_label = addcslashes($user_label, "\\\"\n\r\t");
            $data = $return->get_error_data();
            $status = is_array($data) && isset($data['status']) ? (string) $data['status'] : 'unknown';
            $id = isset($args[0]) ? (string) $args[0] : 'unknown';
            $stdout = fopen('php://stdout', 'wb');
            fwrite(
                $stdout,
                "\033[1;31mWP_Error\033[0m " .
                "\033[34muser\033[0m=\"{$user_label}\" " .
                "\033[36mtarget\033[0m=WP_REST_Posts_Controller::get_post " .
                "\033[33mid\033[0m={$id} " .
                "\033[35mcode\033[0m={$return->get_error_code()} " .
                "\033[32mstatus\033[0m={$status} " .
                "\033[37mmessage\033[0m=\"" . $return->get_error_message() . "\"\n"
            );
            fclose($stdout);
        },
    ],
];
PHP);

$php = getenv('TEST_PHP_EXECUTABLE_ESCAPED');
$ext = realpath(__DIR__ . '/../modules/gameshark.so');
$code = <<<'PHP'
class WP_Error {
    private string $code;
    private string $message;
    private array $data;

    public function __construct(string $code, string $message, array $data = []) {
        $this->code = $code;
        $this->message = $message;
        $this->data = $data;
    }

    public function get_error_code(): string {
        return $this->code;
    }

    public function get_error_message(): string {
        return $this->message;
    }

    public function get_error_data(): array {
        return $this->data;
    }
}

function wp_get_current_user() {
    return $GLOBALS['current_wp_user'] ?? null;
}

class WP_REST_Posts_Controller {
    public function dispatch(int $id) {
        return $this->get_post($id);
    }

    protected function get_post(int $id) {
        return new WP_Error('rest_post_invalid_id', 'Invalid post ID.', ['status' => 404]);
    }
}

$controller = new WP_REST_Posts_Controller();
$GLOBALS['current_wp_user'] = (object) ['display_name' => 'Ada Editor', 'user_login' => 'ada'];
$result = $controller->dispatch(15648);
echo $result instanceof WP_Error ? "returned-error\n" : "missing-error\n";
$GLOBALS['current_wp_user'] = (object) ['display_name' => '   ', 'user_login' => 'admin'];
$fallback = $controller->dispatch(15649);
echo $fallback instanceof WP_Error ? "returned-fallback-error\n" : "missing-fallback-error\n";

$status = gameshark_invariants_status();
foreach ($status['specs'] as $spec) {
    echo $spec['id'], ':', $spec['resolved_kind'], ':', $spec['executions'], "\n";
}
PHP;

$cmd = sprintf(
    '%s -n -d extension=%s -d gameshark.invariants=1 -d gameshark.invariants_file=%s -r %s',
    $php,
    escapeshellarg($ext),
    escapeshellarg($config),
    escapeshellarg($code)
);
exec($cmd, $output, $status);
$text = implode("\n", $output);
$plain = preg_replace('/\033\[[0-9;]*m/', '', $text);
echo 'process:', $status === 0 ? 'ok' : 'failed', "\n";
echo 'has-color:', str_contains($text, "\033[1;31mWP_Error\033[0m") ? 'yes' : 'no', "\n";
echo 'has-target:', str_contains($text, 'target') && str_contains($text, 'WP_REST_Posts_Controller::get_post') ? 'yes' : 'no', "\n";
echo 'has-id:', str_contains($text, 'id') && str_contains($text, '15648') ? 'yes' : 'no', "\n";
echo 'has-code:', str_contains($text, 'rest_post_invalid_id') ? 'yes' : 'no', "\n";
echo 'has-message:', str_contains($text, 'Invalid post ID.') ? 'yes' : 'no', "\n";
echo 'has-status:', str_contains($text, '404') ? 'yes' : 'no', "\n";
echo 'has-display-user:', str_contains($plain, 'user="Ada Editor"') ? 'yes' : 'no', "\n";
echo 'has-login-fallback:', str_contains($plain, 'user="admin"') ? 'yes' : 'no', "\n";
foreach ($output as $line) {
    echo preg_replace('/\033\[[0-9;]*m/', '', $line), "\n";
}
?>
--CLEAN--
<?php
@unlink(__DIR__ . '/gameshark_invariants_wp_error_stdout_config.php');
?>
--EXPECT--
process:ok
has-color:yes
has-target:yes
has-id:yes
has-code:yes
has-message:yes
has-status:yes
has-display-user:yes
has-login-fallback:yes
WP_Error user="Ada Editor" target=WP_REST_Posts_Controller::get_post id=15648 code=rest_post_invalid_id status=404 message="Invalid post ID."
returned-error
WP_Error user="admin" target=WP_REST_Posts_Controller::get_post id=15649 code=rest_post_invalid_id status=404 message="Invalid post ID."
returned-fallback-error
wp-rest-get-post-wp-error:user_method:2
