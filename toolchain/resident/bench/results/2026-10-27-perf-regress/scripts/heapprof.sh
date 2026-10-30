# heapprof.sh BRANCH: release ra-jit of BRANCH (from fix.bundle) with the heap-profile feature into bin/heapprof,
# then a cold ripgrep build with RA_JIT_MEMORY and jemalloc profiling; folds the worker's build.prof with jeprof.
# Out: /root/perf/heap/.
export PATH=/nix/var/nix/profiles/default/bin:/run/current-system/sw/bin:$PATH
b=$1
cd /root/perf/blaze && git fetch -q -f ../fix.bundle "$b:$b"
wt=/root/perf/wt-build
cd $wt && git checkout -q --force --detach "$b"
tc=$(cat harness/toolchain)
export CARGO_TARGET_DIR=/root/perf/target-heap CARGO_INCREMENTAL=0
nix develop --command bash -c "export RUSTUP_TOOLCHAIN=$tc PATH=/nix/var/nix/profiles/default/bin:\$PATH; cargo build -j 8 -p ra-jit --release --locked --features heap-profile" 2>&1 | tail -2
bin=/root/perf/bin/heapprof; rm -rf $bin; mkdir -p $bin
for x in ra-jit cargo-ra-jit cargo-blaze blaze ra-jit-rustc ra-jit-rustc-first; do cp $CARGO_TARGET_DIR/release/$x $bin/ 2>/dev/null; done
out=/root/perf/heap; rm -rf $out; mkdir -p $out/tables
p=/root/perf/heap-ws; rm -rf $p; mkdir -p $p
cp -r /root/ra-jit-projects/ripgrep $p/rg; rm -rf $p/rg/target $p/rg/.git
(
  export RA_JIT_HOME=$p/home RA_JIT_CACHE=$p/store RA_JIT_BACKEND=cranelift RUSTUP_TOOLCHAIN=$tc
  export RA_JIT_STD_FROM=/root/perf/work/stdkey-cranelift/std-cranelift CARGO_TERM_COLOR=never
  export RA_JIT_MEMORY=$out/tables _RJEM_MALLOC_CONF=prof:true,lg_prof_sample:19
  export PATH=$bin:$PATH
  cd $p/rg
  taskset -c 0-3 cargo ra-jit build > $out/cold.out 2>&1; echo "cold exit $?"
  for i in $(seq 600); do cargo ra-jit status 2>&1 | grep -q "$p/rg: ready" && break; sleep 0.5; done
  for w in $RA_JIT_HOME/workers/*/; do cp $w/log $out/worker-$(basename $w).log; done
  cargo ra-jit stop >/dev/null 2>&1
)
ls $out/tables
jeprof=$(nix build --no-link --print-out-paths nixpkgs#jemalloc.bin 2>/dev/null || nix build --no-link --print-out-paths nixpkgs#jemalloc 2>/dev/null)
echo "jeprof from $jeprof"
for prof in $out/tables/*.prof; do
  nix shell nixpkgs#perl nixpkgs#binutils -c perl $(ls $jeprof/bin/jeprof | head -1) --collapsed $bin/ra-jit $prof > ${prof%.prof}.folded 2> ${prof%.prof}.jeprof.err
  echo "$prof folded: $(wc -l < ${prof%.prof}.folded) stacks"
done
echo HEAP DONE
