# stdcheck.sh NAME...: cold `cargo ra-jit build` of ripgrep with bin/NAME and its bench std (work/NAME-cranelift),
# then what the worker log says of the standard library and the store. Out: /root/perf/stdcheck/NAME/.
export PATH=/nix/var/nix/profiles/default/bin:/run/current-system/sw/bin:$PATH
tc=$(cat /root/perf/wt-build/harness/toolchain)
for name in "$@"; do
  out=/root/perf/stdcheck/$name; rm -rf $out; mkdir -p $out
  p=/root/perf/stdcheck-ws/$name; rm -rf $p; mkdir -p $p
  cp -r /root/ra-jit-projects/ripgrep $p/rg; rm -rf $p/rg/target $p/rg/.git
  (
    export RA_JIT_HOME=$p/home RA_JIT_CACHE=$p/store RA_JIT_BACKEND=cranelift RUSTUP_TOOLCHAIN=$tc
    export RA_JIT_STD_FROM=/root/perf/work/$name-cranelift/std-cranelift CARGO_TERM_COLOR=never
    export PATH=/root/perf/bin/$name:$PATH
    cd $p/rg
    s=$(date +%s%N)
    taskset -c 0-3 cargo ra-jit build > $out/cold.out 2>&1
    echo "$name cold exit $? in $(( ($(date +%s%N) - s) / 1000000 ))ms"
    for w in $RA_JIT_HOME/workers/*/; do cp $w/log $out/worker-$(basename $w).log; done
    grep -h -E "standard library|^store|frozen|resolve:" $out/worker-*.log | cut -c1-200
    cargo ra-jit stop >/dev/null 2>&1
  )
  rm -rf $p
done
echo STDCHECK DONE
