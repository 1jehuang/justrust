#!/usr/bin/env bash
# cold.sh REV: a cold `cargo ra-jit build` of ripgrep as bench.py's (empty store, std fetched from
# what `ra-jit std` made), with a time on every line the worker and daemon log and on every
# process the client starts. Out: /root/perf/cold/REV/.
set -uo pipefail
export PATH=/nix/var/nix/profiles/default/bin:/run/current-system/sw/bin:$PATH
rev=$(git -C /root/perf/blaze rev-parse --short=10 "$1")
bin=/root/perf/bin/$rev
out=/root/perf/cold/$rev; rm -rf "$out"; mkdir -p "$out"
p=/root/perf/cold-ws/$rev; rm -rf "$p"; mkdir -p "$p"
cp -r /root/ra-jit-projects/ripgrep "$p/rg"; rm -rf "$p/rg/target" "$p/rg/.git"
wt=/root/perf/wt-prof
[ -d $wt ] || git -C /root/perf/blaze worktree add -q --detach $wt "$rev"
git -C $wt checkout -q --force --detach "$rev"
tc=$(cat $wt/harness/toolchain)
rustup toolchain install "$tc" --profile minimal -c rust-src >/dev/null 2>&1
export RUSTUP_TOOLCHAIN=$tc RA_JIT_BACKEND=cranelift RUST_BACKTRACE=0 CARGO_TERM_COLOR=never
export RA_JIT_HOME=$p/home-std
s=$(date +%s.%N)
$bin/ra-jit std $p/std "$(rustc --print sysroot)" > $out/std.out 2>&1
echo "std made in $(echo "$(date +%s.%N) - $s" | bc)s" | tee $out/times.txt
export RA_JIT_HOME=$p/home RA_JIT_CACHE=$p/store RA_JIT_STD_FROM=$p/std RA_JIT_STATS=1
export PATH=$bin:$PATH
stamp() { while IFS= read -r l; do printf '%s %s\n' "$(date +%s.%N)" "$l"; done; }
( for i in $(seq 1200); do ls $RA_JIT_HOME/workers/*/log >/dev/null 2>&1 && break; sleep 0.05; done
  tail -n +1 -F $RA_JIT_HOME/workers/*/log 2>/dev/null | stamp > $out/worker.ts ) &
tp=$!
( for i in $(seq 1200); do [ -e $RA_JIT_HOME/daemon.log ] && break; sleep 0.05; done
  tail -n +1 -F $RA_JIT_HOME/daemon.log 2>/dev/null | stamp > $out/daemon.ts ) &
dp=$!
cd $p/rg
s=$(date +%s.%N); echo "$s start" > $out/start.ts
taskset -c 0-3 strace -f --seccomp-bpf -tt -s 200 -e trace=execve,exit_group -o $out/client.strace cargo ra-jit build > $out/cold.out 2> >(stamp > $out/cold.err.ts)
echo "cold exit $? in $(echo "$(date +%s.%N) - $s" | bc)s" | tee -a $out/times.txt
for i in $(seq 600); do cargo ra-jit status 2>&1 | grep -q "$p/rg: ready" && break; sleep 0.5; done
echo "ready at $(echo "$(date +%s.%N) - $s" | bc)s" | tee -a $out/times.txt
sleep 1; kill $tp $dp 2>/dev/null; pkill -f "tail -n \+1 -F $RA_JIT_HOME" 2>/dev/null
cargo ra-jit stop >/dev/null 2>&1
echo done
