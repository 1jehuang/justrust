#!/bin/sh
echo used >> "$(dirname "$0")/linker.used"
exec cc "$@"
