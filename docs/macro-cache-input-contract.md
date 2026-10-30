# Native proc-macro input correctness gate

`bench/reuse-macro-input-accept.py` is a differential correctness gate, not a
purity assumption or authorization to promote proc-macro caching. It changes no
compiler sources and does not bootstrap. Its default compiler is the frozen
`/home/jeremy/.jcode/scratch/dead-item-cache-20261030/sysroot/bin/rustc`.

## Running and attestation

Coordinate native profiling, then use a new private directory:

```sh
python3 -m unittest bench/test_reuse_macro_input_accept.py -v
python3 bench/reuse-macro-input-accept.py \
  --work-dir "$JCODE_SCRATCH_DIR/macro-input-fixture" \
  --output "$JCODE_SCRATCH_DIR/macro-input-result.json"
```

The work directory must not exist and is retained, including failures and raw
profiles. The script removes inherited `RUSTC_BOOTSTRAP` and the attribute cache
opt-out, records `rustc -vV`, and hashes both the resolved launcher and sysroot
`lib/librustc_driver*.so` libraries. Missing driver attestation prevents a pass.
Direct rustc is deliberate: the variable under test is the frozen compiler's
native cache, not hosted Cargo or the installed client.

## Families and inputs

Each of **derive**, **attribute**, and **functionlike** families has five cases:
constant, ordinary environment read, tracked environment read, ordinary file
read, and tracked path plus file read. Derive uses `#[proc_macro_derive(Input)]`
on a fixed struct. Attribute uses `#[proc_macro_attribute]` on a fixed struct.
Both emit a `macro_value()` function called from `main`. Functionlike retains
the original `#[proc_macro]` fixture as an explicitly **uncached control**.
The frozen native implementation caches `derive_macro_expansion` and
`attr_macro_expansion`, not functionlike expansion.

Each case warms, edits the actual consuming `main` body, changes external input
with another body edit, then rebuilds the macro crate at the same shared-library
path with a changed expansion and another body edit. The constant case skips
external input changes. Invocation tokens remain fixed throughout.

Each state compares six configurations: separate persistent off and on lanes,
fresh off and on references, and off then on in a shared persistent directory.
Fresh directories are unique per state. Repeated states also exercise on-to-off
switching in the shared lane. Old executables are removed before compiling.
Compile status/stdout/stderr must match exactly across configurations, then
runtime status/stdout/stderr must match. Only the private root is normalized.
Fresh-off must succeed and print independently expected input, macro revision,
and body marker. Equal failures cannot pass. Expectations are never weakened
when a stale observation occurs.

## Actual cache evidence

After each cached-family case, a separate two-process profile probe warms then
edits only the consuming body in its own persistent cache. Raw self-profile
files and `summarize summarize --json` analysis are retained. A cached-family
pass additionally requires a **nonzero incremental_load_time for its exact
named expansion query** in the second process. In-memory cache-hit counts,
elapsed time, and functional equality alone are not evidence of disk reuse.
No named load means `incomplete`, never pass. This proves the case's query can
load across body edits, not that every matrix transition was a cache hit.

Tracked fixtures probe modern `proc_macro::tracked::{env_var,path}` first and
legacy spellings second. Both failed probes are retained as `unsupported`, not
pass. Unsupported APIs or missing hit evidence make the overall gate incomplete
unless a functional failure already makes it fail. Both exit nonzero.

## Native findings, October 30, 2026

The old root report
`/home/jeremy/.jcode/scratch/macro-input-root-20261030.json` is explicitly limited
negative coverage: its PASS covers only uncached functionlike macros. It is not
cached-family safety proof.

The repaired first native report
`/home/jeremy/.jcode/scratch/macro-input-families-20261030.json` retained failures
for **both derive and attribute**, in all four external-input cases, including
supported modern tracked environment and path APIs. At the external-input plus
body transition, persistent-on printed `alpha:0|2` instead of fresh-off's
`beta:0|2`. All functionlike controls passed. Its initial profile collector did
not locate nested profile files, so its constant cases are incomplete, not
passes. No unsafe expectations were changed. A second independently fresh
matrix with corrected profiling is recorded in
`/home/jeremy/.jcode/scratch/macro-input-families-v2-20261030.json`.
The second matrix independently reproduced all eight stale cases, passed the
two cached constant cases and five functionlike controls, and proved named
incremental loads in all ten cached-family profile probes. For example the
constant derive probe loaded `derive_macro_expansion` in 11,444 ns and the
constant attribute probe loaded `attr_macro_expansion` in 11,922 ns. Both named
rows have zero provider invocations. These are load evidence, not speed claims.
The second report SHA-256 is
`a5aa0c43d1d13d7506c11a4005955e3e396167046cac645b49885442ff628dfd`.
Nine harness unit tests passed before the second native run and after the final
missing-profiler error-handling repair.

The frozen driver SHA-256 is
`3527393f3db7e09eeef270a83f51c5aeedb787c77f845ab76a76618f76ae137e`.
These findings reject safety of this frozen experimental cache for the tested
external-input transitions, including tracked input. They are not a statement
about all Rust compilers, all macros, or any later repaired compiler.

Ordinary untracked reads remain observational probes, not an arbitrary-side-
effect contract. This gate does not establish Cargo dep-info integration,
cross-target loading, concurrent processes, deletion/unset input behavior,
failing macro diagnostics, arbitrary side effects, or speedup. Unit tests check
the harness, not compiler correctness. No performance claim follows.
