#!/usr/bin/env bash
# Nonpublishing acceptance gate for the two static Linux release artifacts.
set -euo pipefail
export LC_ALL=C
binary=${1:?binary path required}
target=${2:?target triple required}
version=${3:?expected package version required}
case "$target" in
  x86_64-unknown-linux-musl) machine='Advanced Micro Devices X86-64'; arch=x86_64 ;;
  aarch64-unknown-linux-musl) machine=AArch64; arch=aarch64 ;;
  *) echo "unsupported static release target: $target" >&2; exit 1 ;;
esac
[[ -x "$binary" ]] || { echo "missing executable: $binary" >&2; exit 1; }
header=$(readelf -h "$binary")
grep -Eq 'Class:[[:space:]]+ELF64' <<<"$header"
grep -Fq "$machine" <<<"$header"
programs=$(readelf -l "$binary")
if grep -Eq '(^|[[:space:]])INTERP([[:space:]]|$)' <<<"$programs"; then
  echo 'release binary must not have a dynamic interpreter' >&2
  exit 1
fi
dynamic=$(readelf -d "$binary")
if grep -Fq '(NEEDED)' <<<"$dynamic"; then
  echo 'release binary must not need shared libraries' >&2
  exit 1
fi
runner=()
if [[ $(uname -m) != "$arch" ]]; then
  runner=("qemu-$arch")
  command -v "${runner[0]}" >/dev/null || {
    echo "${runner[0]} is required to smoke-test $target" >&2
    exit 1
  }
fi
actual=$(CI=1 JUSTRUST_NO_UPDATE=1 "${runner[@]}" "$binary" --version)
[[ "$actual" == "justrust $version" ]] || {
  printf 'wrong release metadata: expected justrust %s, got %s\n' "$version" "$actual" >&2
  exit 1
}
CI=1 JUSTRUST_NO_UPDATE=1 "${runner[@]}" "$binary" --help >/dev/null
printf 'verified %s: static ELF64, %s, version/help smoke passed\n' "$target" "$actual"
