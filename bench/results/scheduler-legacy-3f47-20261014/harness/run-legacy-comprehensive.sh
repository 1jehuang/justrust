#!/usr/bin/env bash
set -euo pipefail
umask 077
trial="$HOME/bench-scheduler-legacy-3f47-20261014"
[[ -d "$trial/harness/bench" && -d "$trial/home/toolchains/nightly-2026-10-08" ]]
[[ ! -e "$trial/results/0-exec.jsonl" ]]
export JUSTRUST_HOME="$trial/home" JUSTRUST="$trial/bin/justrust"
export JUSTRUST_BENCH_WORK="$trial/home/bench/work" JUSTRUST_VERSUS_WORK="$trial/home/bench/versus"
export JUSTRUST_SYNTH_WORK="$trial/home/bench/synth" JUSTRUST_BENCH_SRC="$trial/src"
export JUSTRUST_BENCH_REV=059674bf76fa218ba7e475279a86b4b6caf32aac JUSTRUST_BENCH_DIRTY=0
export JUSTRUST_BENCH_REPO_REV=3f47eadb35019943dd506d8d2f6ab69ea4cadd96
export JUSTRUST_REMOTE=0 JUSTRUST_NO_UPDATE=1 JUSTRUST_NO_TELEMETRY=1 JUSTRUST_TOOLCHAIN_AUTO_INSTALL=0
export JUSTRUST_CARGO_FORK=1 JUSTRUST_CARGO_DAEMON=0 JUSTRUST_SHADOW=0 CARGO_BUILD_JOBS=24
export JUSTRUST_CARGO_CRITICAL_PATH_COSTS="$trial/costs.json" EXPERIMENT_RESULTS="$trial/results"
export PYTHONDONTWRITEBYTECODE=1 PYTHONUNBUFFERED=1
[[ -x /usr/bin/cargo ]] || export VERSUS_STOCK_CARGO="$HOME/.cargo/bin/cargo"
unset RUSTFLAGS RUSTC RUSTC_WRAPPER JUSTRUST_RUN_ID_OVERRIDE JUSTRUST_DISABLE JCODE_API_KEY JUSTRUST_API_KEY JUSTRUST_BENCH_REMOTE
cd "$trial/harness"
( while sleep 60; do date +%s | sudo -n tee /var/lib/justrust/last-active >/dev/null; done ) &
keeper=$!
trap 'kill "$keeper" 2>/dev/null || true' EXIT
sha256sum "$JUSTRUST" "$JUSTRUST_HOME/toolchains/nightly-2026-10-08/bin/rustc" "$JUSTRUST_HOME/toolchains/nightly-2026-10-08/bin/jr-cargo" "$trial/costs.json" bench/suite.py bench/versus.py bench/synth/gen.py bench/synth/weights.json | tee "$trial/results/identity-start.txt"
date -u
getconf _NPROCESSORS_ONLN
df -B1 "$trial"
python3 -m unittest discover -s bench -p 'test*.py'
for mode in 0 1; do
  export JUSTRUST_CARGO_CRITICAL_PATH=$mode
  printf 'JCODE_CHECKPOINT {"message":"Begin fresh same-host seed1/40-point synthetic epoch mode=%s"}\n' "$mode"
  remaining=$((1791975540 - $(date +%s)))
  [[ $remaining -gt 60 ]] || { echo 'Deadline gate, preserve partial outputs'; exit 124; }
  timeout --signal=TERM --kill-after=15s "$remaining" python3 "$trial/observer.py" 2>&1 | tee "$trial/results/$mode-output.log"
done
python3 bench/suite.py compare "$trial/results/0-suite.json" "$trial/results/1-suite.json" | tee "$trial/results/compare.txt"
sha256sum "$JUSTRUST" "$JUSTRUST_HOME/toolchains/nightly-2026-10-08/bin/rustc" "$JUSTRUST_HOME/toolchains/nightly-2026-10-08/bin/jr-cargo" "$trial/costs.json" | tee "$trial/results/identity-end.txt"
printf 'JCODE_CHECKPOINT {"message":"Both full synthetic epochs finished, audit noise and provenance before any claim"}\n'
