#!/usr/bin/env bash
# Usage: bench_edit.sh <label> <file> <anchor> <replacement>
# Applies an edit, times `cargo test -p jcode-desktop-ui --lib --no-run`, restores the file.
set -u
cd /home/jeremy/jcode-desktop
OUTDIR=/home/jeremy/.jcode/scratch/buildtime/runs
mkdir -p "$OUTDIR"
label="$1"; file="$2"; old="$3"; new="$4"
backup="$OUTDIR/$label.orig"
cp "$file" "$backup"
OLD="$old" NEW="$new" FILE="$file" python3 -c '
import os
p=os.environ["FILE"]; s=open(p).read(); o=os.environ["OLD"]
assert o in s, "anchor not found"
open(p,"w").write(s.replace(o, os.environ["NEW"], 1))'
start=$(date +%s.%N)
cargo test -p jcode-desktop-ui --lib --no-run --timings > "$OUTDIR/$label.log" 2>&1
rc=$?
end=$(date +%s.%N)
cp "$backup" "$file"
echo "$label rc=$rc wall=$(python3 -c "print(round($end - $start, 2))")"
python3 - <<'EOF'
import re,json,glob,os
f=max(glob.glob("target/cargo-timings/cargo-timing-*.html"),key=os.path.getmtime)
s=open(f).read()
m=re.search(r"const UNIT_DATA = (\[.*?\]);",s,re.S)
for u in json.loads(m.group(1)):
    print(f"   {u['name']:28s} {u.get('target',''):12s} total={u['duration']:.1f}s rmeta={u.get('rmeta_time') or 0:.1f}s")
EOF
