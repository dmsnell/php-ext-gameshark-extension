--TEST--
gameshark extension loads
--EXTENSIONS--
gameshark
--FILE--
<?php
var_dump(extension_loaded('gameshark'));
var_dump(gameshark_loaded());
?>
--EXPECT--
bool(true)
bool(true)
