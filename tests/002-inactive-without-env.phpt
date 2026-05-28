--TEST--
gameshark stays inactive without recording env
--EXTENSIONS--
gameshark
--ENV--
GAMESHARK_DB=
GAMESHARK_SIDE=
--FILE--
<?php
var_dump(gameshark_loaded());
var_dump(gameshark_side());
var_dump(gameshark_db_path());
$compare = gameshark_compare("array");
var_dump(isset($compare['error']));
?>
--EXPECT--
bool(true)
NULL
NULL
bool(true)
