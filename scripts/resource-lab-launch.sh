#!/usr/bin/env bash
# Open the explicitly isolated resource-sharing experiment, never switch branches.
set -euo pipefail
repo=${JUSTRUST_RESOURCE_LAB_DIR:-/home/jeremy/justrust-resource-lab}
expected=experiment/resource-sharing
if [[ ! -d "$repo/.git" ]]; then
  printf 'Resource Lab checkout is missing: %s\n' "$repo" >&2
  exit 1
fi
branch=$(git -C "$repo" symbolic-ref --quiet --short HEAD)
if [[ "$branch" != "$expected" ]]; then
  printf 'Refusing to open Resource Lab on branch %s (expected %s).\n' "$branch" "$expected" >&2
  exit 1
fi
cd "$repo"
printf 'Justrust Resource Lab | %s\n' "$branch"
printf 'Client: %s\nCloud: /home/jeremy/justrust-cloud-resource-lab\n' "$repo"
printf 'Start with docs/resource-lab.md. Production routing is NOT changed by this launcher.\n\n'
exec "${JCODE_BIN:-/home/jeremy/.local/bin/jcode}" "$@"
