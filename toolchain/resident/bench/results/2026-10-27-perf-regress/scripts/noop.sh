#!/usr/bin/env bash
# noop.sh REV: warm daemon on ripgrep, then where a no-op `cargo ra-jit build` spends its time:
# timed runs, strace -f of the client (execve/connect/openat/wait, with times). Out: /root/perf/noop/REV/.
set -uo pipefail
export PATH=/nix/var/nix/profiles/default/bin:/run/current-system/sw/bin:$PATH
rev=$(git -C /root/perf/blaze rev-parse --short=10 "$1")
bin=/root/perf/bin/$rev
out=/root/perf/noop/$rev; rm -rf "$out"; mkdir -p "$out"
p=/root/perf/noop-ws/$rev; rm -rf "$p"; mkdir -p "$p"
cp -r /root/ra-jit-projects/ripgrep "$p/rg"; rm -rf "$p/rg/target" "$p/rg/.git"
wt=/root/perf/wt-prof
[ -d $wt ] || git -C /root/perf/blaze worktree add -q --detach $wt "$rev"
git -C $wt checkout -q --force --detach "$rev"
tc=$(cat $wt/harness/toolchain)
rustup toolchain install "$tc" --profile minimal -c rust-src >/dev/null 2>&1
std=$(ls -d /root/perf/work/$rev-cranelift/std-cranelift 2>/dev/null | head -1)
export RA_JIT_HOME=$p/home RA_JIT_CACHE=$p/store RA_JIT_BACKEND=cranelift RUSTUP_TOOLCHAIN=$tc
export RA_JIT_STD_FROM=$std RUST_BACKTRACE=0 CARGO_TERM_COLOR=never
export PATH=$bin:$PATH
cd $p/rg
t() { local s=$(date +%s%N); "$@"; local e=$(date +%s%N); echo $(( (e - s) / 1000000 ))ms; }
echo "== cold"; taskset -c 0-3 cargo ra-jit build > $out/cold.out 2> $out/cold.err; echo "exit $?"
for i in $(seq 600); do cargo ra-jit status 2>&1 | grep -q "$p/rg: ready" && break; sleep 0.5; done
for i in 1 2 3 4 5; do echo "noop $(t taskset -c 0-3 cargo ra-jit build 2>/dev/null)"; done | tee $out/times.txt
cargo_bin=$(rustup which cargo)
for i in 1 2 3; do echo "cargo-ra-jit direct $(t taskset -c 0-3 $bin/cargo-ra-jit ra-jit build 2>/dev/null)"; done | tee -a $out/times.txt
for i in 1 2 3; do echo "rustup cargo --version $(t sh -c "cargo --version >/dev/null")"; done | tee -a $out/times.txt
for i in 1 2 3; do echo "plain cargo metadata $(t $cargo_bin metadata --format-version 1 --offline >/dev/null 2>&1)"; done | tee -a $out/times.txt
strace -f -tt -s 300 -e trace=execve,connect,openat,wait4,exit_group -o $out/noop.strace taskset -c 0-3 cargo ra-jit build > $out/strace.out 2>&1
strace -f -c -o $out/noop.count taskset -c 0-3 cargo ra-jit build >/dev/null 2>&1
RA_JIT_STATS=1 taskset -c 0-3 cargo ra-jit build > $out/stats.out 2>&1
for w in $RA_JIT_HOME/workers/*/; do tail -60 $w/log > $out/worker-$(basename $w).log 2>/dev/null; done
cp $RA_JIT_HOME/*.log $out/ 2>/dev/null
cargo ra-jit stop >/dev/null 2>&1
echo done
