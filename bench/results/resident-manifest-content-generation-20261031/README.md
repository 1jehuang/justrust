# v74 manifest content generation: portable historical evidence

Accepted **only as a default-OFF, scoped combined same-byte manifest+lock retouch
option**, implemented in historical source commit `de8533b93`. This is not a
general speedup, default promotion, causal attribution, or an independently
measured manifest-versus-lock claim. Later live Rust/Python changes are outside
this evidence package. No compiler or native runtime was invoked while packaging.

## Replay without the private experiment or repository

Copy this directory anywhere, then use a standard Python interpreter:

```sh
python3 validate.py --manifest
python3 test_offline.py
```

`validate.py` uses only stdlib and package files. It does not import runnable
benchmark modules. It hash-checks archived original sources, extracts only a
specific pure-function AST closure, and executes the **original tuple-valued
parser/contract/oracle**, without converting the oracle to accept JSON lists.
It independently derives strict mutation inputs and the reachable runtime nonce
overlay from archived original source, validates complete native named results,
replays every native/harness/doc patch in memory, and recomputes timings from
all measured call receipts rather than trusting arm summaries.

`test_offline.py` includes coherent raw+parsed, raw+nonce, raw+PID, native-result,
source, timing+summary, and promotion tamper cases. Its relocated child uses
`HOME` set to an empty directory and `PATH` empty, Python `-I -S -B`, preloaded
stdlib, and a Python audit hook that then forbids external file/directory reads,
writes, subprocess/native execution, dynamic native loading, and networking.
Separate negative payloads prove that external reads, native child execution,
sockets, dynamic loading, and writes are denied, not merely unused by the replay.
The test driver itself launches only the same Python interpreter for these
checks, never an engine, compiler, shell, Cargo, git, or network job.

### Trust boundary

Keep `validate.py` from the reviewed Git commit. Its literal historical
`OBSERVATIONS` SHA binds the complete allowlisted observation file, which binds
all archived historical derivatives. A resealed `SHA256.json` is only an integrity
inventory, not authenticity: arbitrary coherent changes to timings, runtime
nonces, process identities, native named results, and parsed receipts must still
fail the trusted historical commitment. Semantic checks execute before that
commitment, including exact original named-test and runtime oracles. Rewriting
both the trusted validator and all evidence is outside this threat model.
This package cannot independently prove that an original machine clock or kernel
receipt was honest. It independently recomputes the conclusions recorded from
those historically committed receipts, not a new native measurement.

## Actual correctness evidence

- Actual native journals: **53 codegen + 369 resident tests**, all named outcomes,
  including all five real temporary-file manifest generation helper tests.
- Original source3757 checksum inventory and matching after-checks, unchanged
  compiler lock, exact built executable SHA, pinned nightly **4020 files**.
- **30 targeted runtime calls**: original OFF plus samecandidate OFF/ON/ON/OFF,
  six phases each. Separate identical manifest/lock rewrites and final no-op have
  fresh runtime nonces. These targeted calls are diagnostic, not the suite claim.
- **9 changed-manifest calls**: original/OFF/ON valid, unused-function `E0308`,
  recovery. Rust source stays identical, manifest feature selection activates the
  unused error, failures have no successful runtime marker, recoveries have a
  fresh nonce.
- **6 native build-script guard calls**: environment choice A/B/A produces
  **11/22/11**, and C-file content A/B/A produces **11/33/11**. The replay derives
  expected native values from archived C sources, not the row's `expected` field.
  Build script, native sources, linkage consumer, per-call file identities, and
  runtime output are archived.
- **28 strict state calls**, OFF/ON/ON/OFF: cold preparation, no-op, actual
  manifest feature addition, type error/recovery, assertion failure/recovery.
  Full source63 identities are independently reconstructed with the original
  instrumentation and mutation recipes. The exact original oracle is retained.
- Full historical Python regression: **746 tests**, `41.936s`, `OK`. This is the
  original completed journal receipt, not a claim that evidence-only tests
  re-executed that regression.

## Pinned scoped comparison, including the slower first ON calls

Pinned public source `f97fcc2`, all **63 source blobs** archived. One candidate
ELF for both arms, private unseeded cold stores per segment, local native direct
execution, Cranelift backend, opt level 0, fixed dev/test profiles, both proc-macro
server overrides `0` as a compatibility control, both remote overrides `off`.
Host receipt: Intel Core Ultra X9 388H, x86_64, 16 logical CPUs. This is a
licensed Blaze-derived resident engine, not the shipping justrust client.

Reference six semantic calls, then four candidate segments OFF/ON/ON/OFF, each
with six original semantic calls, one fixed-overlay warmup, and five measured
combined-retouch calls: **54 calls, 20 measured samples, 24 fresh reachable runtime
nonces, five successful native stops**, pristine original source restored.
Each measured call rewrites both original `Cargo.toml` and `Cargo.lock` with
unchanged bytes. SHA/size equality and strictly increasing observed nanosecond
mtimes are retained. Native status PID plus `/proc` PID/start ticks, executable
SHA, and private worker socket roles prove actual same-generation retention ON
and fresh generation OFF. Entry marker counts are **not** reuse proof.

Exact suite compare:

```text
Native resident combined same-byte manifest+lock retouch scoped diagnostic.
Same candidate ELF, OFF/ON/ON/OFF; 5 samples per segment, 10 per arm.
OFF median: 9.459865s -> ON median: 2.475840s, delta -6.984025s (-73.83%).
Absolute median delta exceeds requested 10% floor.
Descriptive/uncontrolled, noise unknown. No significance or general speedup verdict.
Combined retouch only, not separate manifest/lock attribution or default promotion.
```

Unrounded medians: OFF `9.45986455900129s`, ON `2.4758396840043133s`.
**First ON retouch in each segment costs `11.96186254301574s` and
`12.040021672000876s`, slower than EVERY OFF sample (`9.233493571984582s` through
`10.609115069994004s`).** These two measured costs remain in both ten-sample ON
statistics and full receipts. They are not warmups or discarded outliers.
Remaining eight ON samples range `2.3167891800112557s` to `2.656919036002364s`.
The first-touch cause is not established and ambient/daemon/OS-cache noise remains
**unknown**. No broad session-weighted benchmark or default promotion is claimed.

## What is archived and what is intentionally excluded

- `observations.json`: fixed-field receipts, full stdout subject to exact line
  grammars, selected full-line diagnostics, exact flags/profile controls,
  raw-original SHA256/size, nonces, all measured seconds, normalized process and
  private socket roles. No arbitrary ambient environment or raw proc argv dump.
- `inventories/`: full source63, native-source3757 and toolchain4020 inventories.
  `pinned-source/`: all63 immutable public source blobs.
- `native/`: all selected native named test outcomes and complete result counts.
- `frozen/`: full original/candidate `whole.rs`, both harness files, source-only
  and final acceptance documents, every patch, static check receipts, releases,
  and original historical SHA files. Rust and harness Python source bytes are
  exact. Historical docs/scripts/static helper text containing known private
  paths is a normalized derivative. The original SHA/size is separately recorded.
  Historical SHA files describe originals, while `SHA256.json` describes these
  exported files. Do not mistake the former for the export integrity manifest.
- `strict/`, `targeted/`, `changed-manifest/`, `native-aba/`, `pinned/`: original
  real fixture sources and historical backups used for independent derivation.
- `oracles/`: immutable original pure-oracle sources and delivered v74 harness.
  `drivers/`: allowlisted historical driver/validator scripts as non-executed text.
  `analysis/`: original root validator outputs, including their original historical
  pending-measurement wording, not rewritten after acceptance.
- `lineage/`: full Apache-2.0/MIT licenses and imported resident README/Cargo
  attribution. Blaze origin `5d4f78f7577cf7170993b74f735099dd03f5c6ca`, tag
  `v0.20261027.0`. No clean-room claim or license removal.

`export_private.py` is an explicit historical export tool, not needed for replay.
It reads only assigned frozen inputs and read-only immutable Git public blobs,
records original SHA/size, checks that private originals did not change, and
exports known fields/line grammars and known argv path roles. Unknown stdout or
worker argument structure aborts. Original private files are preserved. Binaries,
private caches/targets, ambient environment, arbitrary daemon logs, raw private
metadata, credentials and raw private proc argv are excluded. Original private
raw streams are represented by their SHA/size plus the allowlisted derivative,
not falsely described as byte-for-byte full stderr exports.

## Contract and limitations

The flag is exact `RA_JIT_MANIFEST_CONTENT_GENERATION=1`, process-first-use,
default OFF. Only the same original manifest+lock fileset is content-keyed,
framed with canonical native path/root identity, explicit missing lock state,
version/domain and deterministic sorted set semantics. Content tags use bit127,
disjoint from original timestamp tags. IO uncertainty retains the **exact original
maxmtime fallback**, including its existing repeated-unknown limitations. It does
not silently turn unknown IO into a known content digest or promise stronger
freshness for repeated unknown states. Actual changed program inputs still use
the original reload behavior. `kept: false`, native ABI/config/script/worker-key
and fallback guards remain the external contract. Content identity alone is
**not a full toolchain ABI safety proof**. This does not reactivate rejected v64
retention or infer same-PID generation reuse after changed content.
