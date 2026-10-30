#!/usr/bin/env bash
# noop.sh REV: warm daemon on ripgrep, then where a no-op `cargo ra-jit build` spends its time:
# timed runs, strace -f of the client (execve/connect/openat/wait, with times). Out: /root/perf/noop/REV/.
set -uo pipefail
export PATH=/nix/var/nix/profiles/default/bin:/run/current-system/sw/bin:$PATH
rev=$(git -C /root/perf/blaze rev-parse --short=10 "$1")
bin=/root/perf/bin/$rev
out=/root/perf/noopperf/$rev; rm -rf "$out"; mkdir -p "$out"
p=/root/perf/noopperf-ws/$rev; rm -rf "$p"; mkdir -p "$p"
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
for i in 1 2 3; do taskset -c 0-3 cargo ra-jit build >/dev/null 2>&1; done
s=$(date +%s%N); for i in $(seq 30); do taskset -c 0-3 cargo ra-jit build >/dev/null 2>&1; done; echo "30 noops $(( ($(date +%s%N) - s) / 1000000 ))ms" | tee $out/times.txt
perf record -a -C 0-3 -g -F 4999 -o $out/perf.data -- bash -c 'for i in $(seq 30); do taskset -c 0-3 cargo ra-jit build >/dev/null 2>&1; done' 2> $out/perf-record.err
perf report -i $out/perf.data --no-children --sort comm,dso --stdio 2>/dev/null | head -60 > $out/by-dso.txt
perf report -i $out/perf.data --no-children --comm ra-jit --sort sym --stdio -g none 2>/dev/null | head -80 > $out/ra-jit-syms.txt
perf report -i $out/perf.data --children --comm ra-jit --sort sym --stdio -g caller,0.5,callee --percent-limit 3 2>/dev/null | head -400 > $out/ra-jit-children.txt
file $bin/ra-jit > $out/file.txt
cargo ra-jit stop >/dev/null 2>&1
echo done
