# Native declarative macro match/transcribe cache

`-Zjr-cache-declarative-macros=yes` is tracked and defaults to **false**.
It requires incremental compilation and ordinary span hashing. This is a
native-correctness-tested candidate, not a measured speedup or a default promotion.

## Deliberately bounded admission

Only function-like declarative macros whose **entire rule set** consists of
literal token/delimiter matchers and literal token/delimiter transcribers are
eligible. Metavariables, fragment parsing, repetitions, metavariable expressions,
attribute and derive declarative rules all stay on the original path. An earlier
ineligible rule cannot have its parser side effects skipped by a later eligible
arm. No arbitrary procedural macro execution or IO is cached by this feature.

The token codec was inspected before admission: `TokenStream` and ordinary
`TokenTree`/`TokenKind` variants implement the cache encoder/decoder and stable
hash traits. `LazyAttrTokenStream` explicitly panics on encoding, decoding and
stable hashing. The boundary therefore rejects invisible groups, interpolated
identifier/lifetime tokens, invisible delimiter tokens, doc comments and EOF,
recursively in definition, invocation and generated tokens. No AST nonterminal
or lazy-attribute representation is admitted. Literal-only matching cannot call
the fragment parser, so it cannot register gated spans or produce fragment
parser diagnostics that a cache hit would skip.

## Key and per-invocation semantics

The native incremental query key contains the `LocalExpnId`, complete definition
tokens including spans/contexts/spacing, complete invocation input tokens,
definition span and packed definition edition/transparency. Invocation identity
and token syntax contexts use the compiler's stable hygiene hashing. Invocation
edition and feature options remain compiler session dependencies. The cache is
not entered with `-Zincremental-ignore-spans`, which would erase essential key
information. There is deliberately no `crate_hash(LOCAL_CRATE)` dependency.

Only `(matched rule index, transcribed tokens)` is reusable. `ParserAnyMacro`
parses those tokens anew, performs trailing-semicolon lints and complete-parse
checks, and `resolver.record_macro_rule_usage` runs for each successful local
invocation, including hits. `trace_macros` invocation/result notes are generated
outside the query. Failures conservatively return to the original matching,
diagnostic and recovery path. Unexpected transcription errors are cancelled
inside the speculative computation and reissued by the original path.

The callback is an `AtomicRef` with a fail-open direct-compute default. The query
provider uses scoped TLS around a synchronous computation reference. It never
retains an expansion context or a callback pointer after the query returns.
The incremental compiler-version identity now appends `-jr-query-schema-5`
because adding a query kind changes private serialized query discriminants,
even in development builds that share a compiler version string. The header
layout stays at version 0. This private payload identity is not the independent
query-admission telemetry schema and does not change its capability marker.

## Self-profile evidence

With the flag enabled, `generic-activity` events record
`jr_declarative_macro_eligible` and `jr_declarative_macro_ineligible` for caller
admission, `jr_declarative_macro_cache_lookup` for query lookup,
`jr_declarative_macro_cache_provider` for actual recomputation and
`jr_declarative_macro_cache_success` for accepted result use. These do not add
stderr output. Collect `-Zself-profile=<unique-directory>` with
`-Zself-profile-events=default,args`, then run
`summarize summarize --json <profile.mm_profdata>` and inspect its generated
JSON. Require positive eligible and successful-result counts on an isolated
literal macro fixture, plus native cache-hit evidence for
`declarative_macro_expansion` and reduced/absent provider executions on reuse.
Provider absence by itself is not proof: negative query results can also reuse.
The existing `cache-proc-macros` help now explicitly warns that derive/attribute
macro IO and environment dependencies are not tracked. This feature neither
enables nor makes that unrelated unsafe experiment sound.

## Validation boundaries

Added lexical-boundary unit coverage in `rustc_expand::declarative_cache::tests`
and a three-revision incremental fixture at
`tests/incremental/jr-declarative-macro-cache.rs`. These are source fixtures,
not evidence of native execution until the coordinator runs the compiler gate.
Native bootstrap/build and runtime measurements are reserved for root
coordination. Gate should compare flag-off/on cold and incremental runs, traces,
rule-use and parse diagnostics, changed definitions/inputs/edition/hygiene,
fragment-parser fallback and query cache hits before any promotion.

## Native acceptance on October 30, 2026

The corrected stage1 compiler and matching standard library built successfully.
The first build failed because stable hashing supports tuples only up to arity
four. The key now nests `(definition span, mode)` as its fourth component.
The launcher SHA256 is
`570faeb27338cea8625c4a4a7eaeef71fc75470508657ff489c3da96ce5f9144`,
and the actual shared compiler driver SHA256 is
`5e7f545149e25586183b72fa3e1f101012f412732123852ba9eaec8c3705e04a`.
Both identities were unchanged after acceptance. Private freezing excludes
both `lib/rustlib/src` and `lib/rustlib/rustc-src` source-tree links.

The first native gate exposed an independent fixture syntax bug with caching
disabled: a const block substituted before `+` needs parentheses. Both edition
fixtures now use `($e) + private`. The 2021 case remains an expected error and
the 2024 case remains an expected successful runtime. No expectations were
weakened. The original build failure and failed fixture log are retained.

The rerun passed **32 revisions across seven modes, 224 compilations**, with
exact status, diagnostic bytes and runtime output comparisons and
`-Zincremental-verify-ich`. A separate five-process profile proof calibrated one
eligible lookup, provider execution and successful result on cold compilation.
Unchanged and same-width body-edit runs each had one eligible successful lookup
and zero provider executions. Flag-off controls had no cache markers.
This proves reuse of the bounded match/transcribe operation, not a compiler
latency improvement or general macro-expansion reuse.

The complete report, five raw profiles, compiler identities and build logs are
under `bench/results/declarative-macro-cache-20261030/`. Run its standalone
`validate.py` to verify the manifest, exact comparisons and positive reuse proof.
The prospective runtime harness supports `--declarative-macro-cache`, which adds
one isolated stack with other reuse experiments and procedural macro caching
disabled. Performance acceptance remains separate and pending.
