# Scoped native parsed-AST retention

This is an explicit, default-off experiment inside the native serial compiler
owner. It is not full frontend residency, a production Cargo default, or evidence
of a general build speedup.

## Ownership and admission

`JUSTRUST_RUSTC_RESIDENT_AST=1` enables root-crate parsing retention for the
explicit owned worker. Missing or `0` disables it. Other values are rejected.
The cache drops before its worker-owned `SessionGlobals` universe, including on
unwind. Compiler retirement and RSS limits remain unchanged.

Every request still loads its root file, lexes it and constructs a native parser.
Only `parse_crate_mod` can be replaced by a clone of an admitted pre-expansion
`ast::Crate`. Command-line attributes are injected afterwards. Expansion,
resolution, typechecking, code generation and unloaded module parsing remain
fresh. AST publication does not depend on later typechecking success.

The key uses exact owning `Arc<SourceFile>` and `SourceMap` identities, edition
and token-stripping mode. Equal restored file contents do not authorize reuse
when a new source-file identity was allocated. Recovery can therefore correctly
miss, while unchanged-root and unrelated-helper-edit acceptance requires hits.

Admission is bounded and syntax-only: ordinary primitive functions, immutable
primitive arguments, tail-only literal/path/arithmetic expressions and unloaded
modules. Attributes, macros, lazy captured tokens, recovered trees and other
unsupported syntax use the ordinary parser. The source preflight excludes
attribute spelling and side-effectful lexical forms. Parser errors, buffered
lints, identifier side tables, gated spans and symbol-gallery changes prevent
publication. The gallery is populated by fresh lexing, not manually replayed.

Retention is limited to 32 entries and 8 MiB estimated bytes, with bounded syntax
inspection, source size and gallery size. Estimates are not allocator accounting.
Native worker RSS retirement remains an independent bound.

## Native correctness evidence

The full native parser suite passed 87 tests, including 12 AST admission/cache
tests. The interface check, driver check and 15 resident driver tests passed.
A complete matching compiler and library sysroot was frozen without installing
it over the published client.

`bench/stable-resident-ast-20261117-0326-native` passed 13 transitions per arm:
unchanged and unrelated-module edits, changed roots, parser errors, type errors,
macro/attribute fallbacks and recovery. Enabled mode ended with 4 hits, 9 misses,
4 rejected parses and 5 retained ASTs. Disabled AST counters and other cache
counters stayed zero. Fresh Rust consumers verified ordinary rlib behavior,
exact diagnostics and unique runtime nonces.

AST-enabled safety guards and shipping-module dogfood, three repetitions, passed:

- `bench/stable-resident-ast-20261117-0326-guard`
- `bench/stable-resident-ast-20261117-0326-dogfood`

The shipping module exercises correct fallback, not proof of real-project AST
hits. All three originals have authenticated exact JSON readback verification.
Earlier failed recovery-contract and incomplete-sysroot receipts were preserved
and archived, not rewritten into passing evidence.

## Focused timing evidence, not promotion

`bench/suite.py stable-native-ast` measures ordinary producer rlib invocations.
Fresh consumer compilation/execution, startup, source generation and two warmup
requests are outside the measured interval. Fresh process, owned process-only
control and AST retention arms use identical producer arguments. Other caches
and incremental compilation stay disabled.

Two CPU-3-pinned, counterbalanced captures used the same compiler, boot, affinity,
parent environment, 32 root functions, 6 unrelated-helper edits and 3 repetitions.
Each arm retained 18 measured observations per capture:

| Record suffix | Fresh median | Process-only median | AST median | AST/control |
| --- | ---: | ---: | ---: | ---: |
| `0328-a` | 18.401 ms | 8.539 ms | 8.509 ms | 0.9965x |
| `0328-b` | 18.650 ms | 8.598 ms | 8.510 ms | 0.9898x |

Full IDs are `bench/stable-resident-ast-suite-20261117-0328-{a,b}`. Both originals
were stored and verified by exact authenticated JSON readback. The strict
`bench/suite.py compare` passed. These repeated captures show no meaningful added
AST-retention advantage on this scope. Process reuse must not be misattributed to
AST caching. No compiler optimization before/after change was measured here.

The comprehensive session-weighted synthetic gate, full-project Cargo routing,
general changed-source Typeck/HIR residency, external-type reuse and default
promotion are not established by this experiment.

## Combined lexer and AST diagnostic

`0924dad7b` adds explicit `--with-lexer` composition to the focused suite.
AST-only defaults remain unchanged. Combined mode requires the exact expected
lexer hit/miss deltas as well as AST hits, bounds both caches independently,
preserves original telemetry, and rejects comparisons with AST-only captures.
Its 44 pure tests and the native `0359-diagnostic` capture passed.

Two CPU-3-pinned captures used the same frozen compiler and parent environment,
32 functions, 6 helper edits and 3 repetitions, with 18 measured rows per arm:

| Record suffix | Fresh median | Process-only median | AST + lexer median |
| --- | ---: | ---: | ---: |
| `0400-a` | 19.081 ms | 8.916 ms | 8.812 ms |
| `0400-b` | 21.805 ms | 10.431 ms | 10.131 ms |

Full record IDs are
`bench/stable-resident-composed-suite-20261117-0400-{a,b}`.
The `bench/suite.py compare` validated provenance but reported repeat drift
above its 10% descriptive floor in fresh and control timings, at +14.28% and
+16.99%. Combined/cache-control ratios were 0.9884x and 0.9713x. This small added
cache effect amid larger repeat drift does not establish a reproducible speedup.
All three untouched generated receipts passed exact authenticated JSON readback.
No compiler code, production default or published client was changed.

The larger unchanged admission-boundary diagnostic used 128 functions, 8 helper
edits and 6 repetitions, with 48 measured observations per arm. Local original
evidence directories are
`~/.jcode/scratch/stable-resident-composed-suite-20261117-0401-large-{a,b}`.
The process-only/combined medians were 27.463/27.872 ms and 27.208/27.814 ms,
respectively. Combined retention was 1.0149x and 1.0223x the process-only time.
The repeat comparison validated identical provenance and reported fresh +6.85%
and control -0.93%. This larger diagnostic also provides no added cache win.
Consumer correctness, exact per-cache telemetry and cache bounds passed.

Both original receipts and raw evidence remain local and unchanged. Automatic
direct-record upload failed with HTTP 413. Their sizes are 3,762,220 and
3,762,206 bytes, exceeding the bounded record endpoint. Original SHA256 values:

- `a`: `1dde304801b00ce93ed108647dbd4a31b8c009a45f77da8d7b5e2c5eda3e043a`
- `b`: `d1d5f5602d01f4c90ed08354ade8745299cfb4af7405ed6d0ba290bae863f43a`

The size blocker was resolved using `bench/record_archive.py`, not a larger
backend limit. Distinct account-private records contain schema-versioned
bounded gzip/base64 envelopes:

- `bench/stable-resident-composed-suite-20261117-0401-large-a-lossless`
- `bench/stable-resident-composed-suite-20261117-0401-large-b-lossless`

Authenticated retrieval reconstructed **every original byte** and matched the
independently pinned SHA256 values above. Both original files were checked
unchanged after readback. These are lossless archive records, not direct receipt
JSON records. The helper rejects malformed schemas, oversized input/output,
corruption, trailing or concatenated gzip members, changed source files and
existing output paths. Its 16 tests passed. No backend cap, credentials,
production client or compiler defaults changed.

To reconstruct an original, supply the corresponding pinned hash:

```sh
python3 bench/record_archive.py --justrust /path/to/published/justrust \
  get stable-resident-composed-suite-20261117-0401-large-a-lossless \
  /new/private/receipt.json \
  --sha256 1dde304801b00ce93ed108647dbd4a31b8c009a45f77da8d7b5e2c5eda3e043a
```

## Hosted correctness acceptance

The same immutable compiler and complete matching library sysroot passed native
AST transitions, AST-enabled safety guards and three shipping-module dogfood
repetitions on the authenticated hosted guest on November 17, 2026:

- `bench/stable-resident-ast-host-20261117-0344-native`
- `bench/stable-resident-ast-host-20261117-0344-guard`
- `bench/stable-resident-ast-host-20261117-0344-dogfood`

The launcher SHA256 was
`8cb4b7089e2e70cbd4832fdf53a24ef2e27db676dcc3632c2501134e6f3dd567`
and the driver SHA256 was
`5d1f35080f0c2c0e0d5d6c6039318a200e96b7b6e8f29f6b57fc6260084588bb`,
matching the local frozen candidate. Original guest evidence directories,
including raw streams, were preserved locally. All three generated receipts
passed authenticated exact JSON readback, without float tolerance or rewriting.

This low-priority guest run establishes correctness only. It is not hosted
performance evidence, Cargo-path acceptance, proof of general typed-query reuse,
or a reason to enable AST retention by default. The published client and
production defaults were not replaced.
