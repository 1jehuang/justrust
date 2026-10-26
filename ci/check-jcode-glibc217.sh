#!/usr/bin/env bash
# Acceptance gate for hosted Jcode's portable Linux x86-64 release artifact.
# Run with Docker access (or invoke this script with sudo -n). This executes
# the supplied binary, so use only trusted, locally verified build outputs.
set -euo pipefail
if [[ $# != 1 ]]; then
  echo "usage: $0 /path/to/jcode" >&2
  exit 2
fi
binary="$(realpath -- "$1")"
if [[ ! -f "$binary" || ! -x "$binary" ]]; then
  echo "error: expected an executable Jcode binary: $binary" >&2
  exit 2
fi
image='quay.io/pypa/manylinux2014_x86_64@sha256:2bd80fa8fca6b775a913b7e42fa410c5f2246fb6f7f8f3280b5d562e5ca5d63f'
# This checks imported versioned symbols, not strings embedded in the program.
# The native artifact reader separately verifies ELF architecture and tables.
versions="$(objdump -T -- "$binary" | awk '/\*UND\*/ { for (i = 1; i <= NF; i++) if ($i ~ /^\(GLIBC_[0-9.]+\)$/) { v = $i; gsub(/[()]/, "", v); sub(/^GLIBC_/, "", v); print v } }' | sort -Vu)"
if [[ -z "$versions" ]]; then
  echo "error: no imported GLIBC symbol versions found" >&2
  exit 1
fi
newest="$(printf '%s\n' "$versions" | tail -n 1)"
if [[ "$(printf '%s\n' 2.17 "$newest" | sort -V | tail -n 1)" != 2.17 ]]; then
  echo "error: binary requires GLIBC_$newest, exceeding GLIBC_2.17" >&2
  exit 1
fi
printf 'highest imported GLIBC version: %s\n' "$newest"
sha256sum -- "$binary"
docker run --rm --network none --read-only --cap-drop ALL \
  --security-opt no-new-privileges --user 65534:65534 \
  --mount "type=bind,source=$binary,target=/jcode,readonly" \
  --entrypoint /bin/bash "$image" -c '
set -euo pipefail
[[ "$(getconf GNU_LIBC_VERSION)" == "glibc 2.17" ]]
getconf GNU_LIBC_VERSION
ldd /jcode
/jcode --no-update --no-selfdev --version
for command in receive credentials router-credentials cancel-return; do
  /jcode --no-update --no-selfdev cloud "$command" --help >/dev/null
done
printf "glibc 2.17 launch and cloud handoff smoke checks passed\n"
'
