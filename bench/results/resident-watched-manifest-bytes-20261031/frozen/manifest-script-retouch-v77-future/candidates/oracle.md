# v77 controlled-future manifest-as-script-input oracle

STOPEDITS release replaces the unexecuted fixed-2023 proposal for execution.
The earlier manifest-script-retouch-v77 freeze remains intact as proposal history.
Only private ROOT/probe-manifest-script-retouch-v77.py and private freeze files
are authored. No checkout source/documentation changes or native jobs.

## Root execution and strict contract

After verifying SHA256SUMS, root may execute:

```sh
python3 ROOT/probe-manifest-script-retouch-v77.py \
  --engine /absolute/path/to/exact-tested-v76-ELF \
  --output /absolute/new/private-evidence-directory \
  --private /absolute/new/disjoint-private-cache-directory
```

ROOT=<EXPERIMENT>. Both private roots
must be new and disjoint. The existing pinned nightly toolchain is inspected
and linked, not installed. Each engine run timeout defaults to 180 seconds.
Six runs total, plus status/stop, use separate fresh private OFF and ON arms,
the same ELF, whole-generation flag 1, and watched-manifest flag 0/1.

The root-authorized setup computes exactly:

```python
base = ((time.time_ns() // 1_000_000_000) // 1000 + 2) * 1000
seconds = (base + 11, base + 22, base + 11)
```

The setup clock, base, exact nanosecond stat receipts, and per-call observed
clocks are retained. Each call must begin while its input mtime is greater than
the observed current clock. Expiration aborts as failure, without recomputing
or relaxing the expected oracle. These future times are explicit experimental
setup, not production semantics or performance evidence. They keep the manifest
newer than newly written Cargo/script fingerprints without editing fingerprints.
The earlier fixed-2023 case was never executed by this worker and is not to run:
it is ambiguous because historical inputs can predate the last script run.

Source manifest, lock, Rust and build.rs bytes stay identical. Only the manifest
mtime changes A/B/A. build.rs explicitly emits rerun-if-changed=Cargo.toml,
reads modified().duration_since(UNIX_EPOCH).as_secs()%1000, generates C in OUT_DIR,
and invokes resolved real cc/ar to produce a fixed native archive. Raw linker
directives stay stable. A constant per-arm external proof log appends actual
seconds/value/build-script PID after successful native compilation. Each call
must add an invocation for its expected timestamp. It is not an env-change oracle.

Rust uses runtime std::env::var("NATIVE_NONCE"), never env!, and calls the real
native function. Sole exact runtime stdout markers must encode fresh nonce and
11,22,11 respectively in BOTH arms. Status must be zero with no timeout, launch
or capture error. No expected native result or source-byte assertion is weakened.
A failed OFF control is failure, not evidence uniquely attributable to v76.

## Evidence, cleanup, limitations

Per-arm state.json retains actual launch argv/cwd/full environment, source full
bytes/base64/hashes and nanosecond stat identities before/after every call, raw
stdout/stderr paths/hashes, strict markers, appended script proof, status output,
worker/daemon/PID logs and private-home-scoped /proc PID/stat/starttime/exe hash
and cmdline observations. PID continuity is recorded for root interpretation,
not inferred from flag values. Available CARGO_TARGET_DIR fingerprints, invoked
timestamps, build outputs, C and archive hashes are retained. Internal engine
artifact paths may instead need locating from worker logs. The same ELF, cc/ar
and complete pinned toolchain identities are checked before/after.

Finally restores initial exact source bytes and mtimes including manifest A,
records restoration and stop receipt, and retains all caches. Stop must exit 0
without timeout/error. No unrelated process is killed. External hard termination
can prevent finally, so root retains outer cleanup responsibility. Private full
environment receipts may contain inherited credentials and must not be published.

Validation is Python AST parse/compile and static assertions only. No fixture or
helper imports/execution by the worker, no builds/probes/native commands, installs,
checkout changes or commits. Root owns actual execution and acceptance. This is
a correctness fixture, no performance/default-promotion claim. Existing helper
interfaces retain licensed Blaze-derived resident harness lineage, not clean room.
