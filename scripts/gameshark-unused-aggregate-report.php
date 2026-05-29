<?php

if ( $argc < 2 || $argc > 5 ) {
	fwrite( STDERR, "usage: php -d extension=gameshark.so -d gameshark.db=/path/to/db.sqlite gameshark-unused-aggregate-report.php <text|json|array> [capture] [since_run_id] [until_run_id]\n" );
	exit( 2 );
}

if ( ! function_exists( 'gameshark_unused_aggregate_report' ) ) {
	fwrite( STDERR, "gameshark extension is not loaded\n" );
	exit( 1 );
}

$format = $argv[1];
$capture = $argv[2] ?? null;
$since = isset( $argv[3] ) && '' !== $argv[3] ? (int) $argv[3] : null;
$until = isset( $argv[4] ) && '' !== $argv[4] ? (int) $argv[4] : null;

$report = gameshark_unused_aggregate_report( $format, $capture, $since, $until );
if ( is_array( $report ) ) {
	echo json_encode( $report, JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES ), "\n";
} else {
	echo $report;
}
