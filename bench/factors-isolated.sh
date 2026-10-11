#!/usr/bin/env bash
# Run bench/factors.py against a frozen copy of justrust in a private
# JUSTRUST_HOME, so other agents reinstalling justrust (and repointing the
# shared rustc shim) during a multi-hour run cannot mix versions.
# Snapshots, factor target dirs, and toolchains are shared with the normal
# home, so warm state is reused. Usage: bench/factors-isolated.sh run [factor...]
set -euo pipefail
src=$(cd "$(dirname "$0")/.." && pwd)
real=${JUSTRUST_REAL_HOME:-$HOME/.justrust}
iso=$real/bench/home
mkdir -p "$iso/bin" "$real/toolchains"
[ -e "$iso/toolchains" ] || ln -s "$real/toolchains" "$iso/toolchains"
cp "$(command -v justrust)" "$iso/bin/justrust.new"
mv "$iso/bin/justrust.new" "$iso/bin/justrust"
echo "factors-isolated: $("$iso/bin/justrust" -V), home $iso" >&2
export JUSTRUST="$iso/bin/justrust"
export JUSTRUST_HOME="$iso"
export JUSTRUST_BENCH_WORK="$real/bench/work"
export JUSTRUST_FACTORS_WORK="$real/bench/factors"
exec python3 "$src/bench/factors.py" "$@"
