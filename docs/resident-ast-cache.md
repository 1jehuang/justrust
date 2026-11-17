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
promotion are not established by this experiment. Hosted AST correctness is a
separate remaining acceptance step.
