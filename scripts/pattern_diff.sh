#!/usr/bin/env bash
# Differential probe: one Rust item per stdin line, compiled with native jr-rust and
# stock rustc (oracle only). Prints exit statuses side by side plus native stdout.
set -u
J=${J:-$(dirname "$0")/../target/debug/jr-rust}
D=$(mktemp -d -p "${JCODE_SCRATCH_DIR:-/tmp}")
MAIN=${MAIN:-'fn main(){ println!("ran"); }'}
i=0
while IFS= read -r body; do
    i=$((i + 1))
    printf '%s\n%s\n' "$body" "$MAIN" >"$D/e$i.rs"
    "$J" "$D/e$i.rs" -o "$D/n$i" >/dev/null 2>"$D/j$i.txt"
    jn=$?
    rustc --edition 2024 -C panic=abort -A warnings "$D/e$i.rs" -o "$D/s$i" 2>/dev/null
    sn=$?
    out=""
    if [ $jn -eq 0 ] && [ $sn -eq 0 ]; then
        a=$("$D/n$i" 2>&1)
        b=$("$D/s$i" 2>&1)
        if [ "$a" = "$b" ]; then out="same-output"; else out="OUTPUT-DIFF native=[$a] stock=[$b]"; fi
    fi
    flag=""
    if [ $((jn == 0)) -ne $((sn == 0)) ]; then flag="MISMATCH "; fi
    echo "$flag$i native=$jn stock=$sn $out :: $(head -1 "$D/j$i.txt" | cut -c1-160)"
done
