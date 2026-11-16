# Stable rustc resident-state experiment

This experiment is implemented in the shipping `stable` branch's owned rustc.
It does not invoke Blaze, `jr-engine`, rust-analyzer, or the clean-room compiler.
The LLVM backend and compiler source are the same in all comparison modes.
Nothing here enables a new default or replaces the published client.

## Current implementation boundary

This is the first native resident-state slice, **not a resident GlobalCtxt**.
A dedicated process accepts sequential compilations with fixed arguments,
working directory, and environment. Every request creates a fresh compiler
session. A bounded process-owned cache retains an explicit whitelist of owned
boolean query results between those sessions. Arena references, interned types,
DefIds, spans, symbols, MIR, and LLVM contexts are not transferred between
sessions by this cache.

A retained value is considered only after the existing incremental dependency
graph marks the node green and promotes its diagnostics. Lookup additionally
matches the incremental root, tracked options hash, stable dependency node, and
previous result fingerprint. RAM hits still pass incremental hash verification.
Red queries compute normally. Missing or evicted values follow ordinary query
execution. The cache does not change compiler options or incremental formats.

The current bound is 8192 values, with generation-reset eviction. Roots longer
than 4096 bytes are not admitted. Counters and self-profile activities distinguish
live cross-session hits from same-session query-cache hits. These cheap scalar
queries do not establish the benefit of retaining most frontend work. A neutral
or negative result here must not be described as a ceiling on full residency.

## Native request protocol

On supported Unix targets, explicitly launch the candidate compiler as:

```text
/path/to/candidate/rustc --jr-resident
```

The launcher transfers a writable pipe descriptor numbered by
`JUSTRUST_RUSTC_RESIDENT_FD`. This pipe is exclusively owned by the worker and is
made close-on-exec. Compiler stdout and stderr remain separate, unmodified
streams. There is no listener, socket namespace, automatic worker startup, or
cross-tenant service.

Standard input carries newline-delimited JSON requests, at most 1 MiB each:

```json
{"id":1,"args":["fixture.rs","--edition=2024","-Cincremental=cache","-o","fixture"]}
```

Arguments exclude argv[0] and must match the first request exactly. IDs must
increase strictly. Response files and stdin source input are rejected. Flags
cannot silently change process-global backend state. Only the implicit host target
and owned LLVM backend are supported in this first protocol. Proc-macro crates
are explicitly refused before dynamic-library loading or macro execution:
rustc's normal loader intentionally keeps their libraries alive, which does not
provide fresh macro static state in a persistent process. Such compilations must
still use a fresh compiler until per-session library isolation is implemented.
A request cannot change
cwd or environment. EOF ends the worker normally. Invalid protocol terminates
the worker rather than guessing or retrying a compilation.

The response pipe carries one JSON completion per request, including `id`,
`status`, `pid`, `elapsed_seconds`, `resident_hits`, `resident_misses`, and
`resident_entries`. Counters are cumulative. Output is flushed before completion.
A normal compilation error reports status 1 and permits a recovery request.
An internal compiler panic is not treated as a recoverable type error.

`JUSTRUST_RUSTC_RESIDENT_VALUES=1` enables typed value retention. With `0` or
unset, the same persistent process runs without that cache. One-shot compilation
remains the normal compiler entry. Existing one-process phase-file recording and
early-reply settings are rejected in resident mode rather than emitting invalid
per-request telemetry. An inherited native query-admission binding is also
rejected: this experimental driver does not pretend one job ticket authorizes
an unlimited resident worker.

## Acceptance harnesses

Use an explicit, frozen candidate path rather than replacing the installed CLI.
Each output directory must be new. The native fixture tests compiler diagnostics,
executable output, edits, error recovery, process identity, and live cache counters:

```sh
python3 bench/stable_resident.py --compiler /absolute/candidate/bin/rustc \
  --output "$JCODE_SCRATCH_DIR/resident-native" -n 5 --items 256
python3 bench/stable_resident_guard.py --compiler /absolute/candidate/bin/rustc \
  --output "$JCODE_SCRATCH_DIR/resident-guard"
python3 bench/stable_resident_dogfood.py --compiler /absolute/candidate/bin/rustc \
  --source "$PWD/src/remote_sync_queue.rs" \
  --output "$JCODE_SCRATCH_DIR/resident-module-dogfood" -n 3
```

The guard harness requires a fresh-process positive control to execute a macro,
then verifies resident refusal before execution and recovery in the same PID.
The dogfood harness copies a real standard-library-only shipping module, compiles
and executes its tests with fresh runtime nonces, and checks intentional failures
and recovery. It does not compile the whole client or change the supplied source.
A developer bootstrap artifact is not the published optimized compiler merely
because its version string matches. Freeze compiler and driver identities and
keep this distinction in every result.

## Measurement and production gates

Compare three modes using **one frozen compiler binary and driver library**:

1. Fresh compiler process per compilation.
2. Persistent compiler with resident values disabled.
3. Persistent compiler with resident values enabled.

Use separate incremental roots, identical source transitions and flags, rotating
mode order, and no Cargo freshness skip. Compare process startup separately from
value reuse. Check actual hit counters, output/diagnostic equivalence, body and
signature changes, type-error recovery, executable results, and retained memory.
Report first preparation separately from hot edits.

The direct native harness is a local bootstrap/protocol diagnostic, not a managed
hosted end-to-end benchmark. An authenticated guest-shell run is likewise a
host-only experiment, not proof of managed build integration. Production dogfood
requires native request ownership/admission, cancellation, output forwarding,
cache accounting, and cleanup through the ordinary hosted build lifecycle. Do
not enable this driver as a default Cargo compiler merely because a tiny fixture
passes. The installed CLI stays on its published release.

General speedup and default-promotion claims still require matching
`bench/suite.py run --synth 1 -n 1` runs and `bench/suite.py compare`, including
command/change breakdowns and cold/setup cost. This document records the design,
not a successful benchmark result. Actual results belong to automatically
uploaded benchmark records.
