#!/bin/bash
# cratesweep.sh <out-dir> <name-version>...: vendors each crate and runs its own tests against
# ra-jit, JOBS crates at a time, under each backend of BACKENDS (`cranelift` unless set; `llvm`
# is run at RA_JIT_OPT=2 RA_JIT_EMIT=fast). Every crate is run under one backend before any is
# run under the next, so two backends never build in one checkout at once. One log, one json
# and one status line per crate in <out-dir>/<backend>. A crate that already has a status there
# is not run again, so a stopped sweep picks up where it was; delete
# `<out-dir>/<backend>/<name>.status` to run a crate again.
# `harness/sweep1000.txt` is the list SWEEP1000.md was measured on.
here=$(cd "$(dirname "$0")" && pwd)
out=$1; shift; mkdir -p "$out"
one() {
    name=$(printf %s "$1" | sed -E 's/-[0-9]+\.[0-9]+\.[0-9]+.*$//')
    dir=$out/$RA_JIT_BACKEND
    [ -e "$dir/$name.status" ] && return
    start=$(date +%s)
    rm -f "$dir/$name.json"
    "$here/vendor.sh" "$1" > "$dir/$name.log" 2>&1 &&
        timeout -k 20 "${TIMEOUT:-1800}" python3 "$here/cratetest.py" "$here/vendor/$name" --json "$dir/$name.json" >> "$dir/$name.log" 2>&1
    code=$?
    result=$(grep -aE '^(MATCH|MISMATCH|UNTESTED|UNSTABLE): ' "$dir/$name.log" | tail -1)
    [ "$code" = 124 ] || [ "$code" = 137 ] && result="TIMEOUT after ${TIMEOUT:-1800}s"
    [ -z "$result" ] && result="FAILED $(tail -1 "$dir/$name.log" | cut -c1-150)"
    echo "$name ($RA_JIT_BACKEND): $result"
    printf '%s\t%s\t%s\n' "$1" "$(($(date +%s) - start))" "$result" > "$dir/$name.status"
    # CLEAN=1: a sweep over many crates fills the disk with their target directories.
    [ -n "$CLEAN" ] && rm -rf "$here/vendor/$name/target"
}
export -f one; export here out
for backend in ${BACKENDS:-cranelift}; do
    mkdir -p "$out/$backend"
    export RA_JIT_BACKEND=$backend
    unset RA_JIT_OPT RA_JIT_EMIT
    [ "$backend" = llvm ] && export RA_JIT_OPT=2 RA_JIT_EMIT=fast
    printf '%s\n' "$@" | xargs -P "${JOBS:-2}" -I{} bash -c 'one {}'
done
