#!/bin/bash
# usage: split-before-after.sh <repo> <file-before> <file-after> <pattern> <replacement-fmt with %d>
# Checks out HEAD~1 then HEAD, makes the same body-only edit 3 times each,
# and runs `justrust check --workspace` after each. Restores the tree.
set -u
cd "$1"
run() {
  local file="$1"
  justrust check --workspace >/dev/null 2>&1
  for i in 1 2 3; do
    cp "$file" "$file.bak"
    sed -i "s/$PAT/$(printf "$REP" "$i")/" "$file"
    out=$(justrust check --workspace 2>&1)
    echo "  run $i: $(echo "$out" | grep -E '^justrust:' | head -1)  ($(echo "$out" | grep -oE 'justrust show [0-9-]+' | head -1 | awk '{print $3}'))"
    mv "$file.bak" "$file"
  done
}
PAT="$4"; REP="$5"
git checkout -q HEAD~1
echo "before split ($(git rev-parse --short HEAD)):"; run "$2"
git checkout -q -
echo "after split ($(git rev-parse --short HEAD)):"; run "$3"
git status --short | head -3
