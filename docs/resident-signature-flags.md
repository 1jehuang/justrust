# Resident signature comptime flag experiment

## Decision: evidence only, not a performance win

The native signature-flag candidate passed the final semantic gates. The matched scoped runtime pair **RSB04 → RSA10** recorded engine medians **0.116184 s → 0.105814 s**, a raw **−8.93%** difference, below the requested 10% floor. The reference-normalized engine/reference ratio moved **0.904701 → 0.924668**, **+2.21% worse**. Noise is unknown. This is **not a performance win**, significance result, comprehensive workload result, or default-promotion gate.

The retained archive is `bench/results/resident-signature-flags-20261030/`. Its portable validator replays original source patches and raw native and runtime receipts without git, native tools, original machine paths, or network access. Historical absolute paths inside receipts are provenance only. No binaries, caches, target directories, fabricated old sources, or reconstructed original DM files are retained. Existing per-version exact-byte source-ready manifests and real snapshots are preserved as the ready receipts.

No live source is restored or committed by this evidence-only change. The root coordinator is evaluating a separate coarse-root candidate on the same uncommitted experimental signature base. That current source state is not evidence of promotion, and the archive does not claim restoration occurred.

## Mechanism and inspected scope

The bounded native resident candidate uses bit 15 of the existing `FnFlags: u16` as `RUSTC_COMPTIME`. `FunctionSignature::with_source_map` computes it using the unchanged original predicate `AttrFlags::has_word(db, id.into(), "rustc_comptime")`. `compile_time_only` reads `FunctionSignature::of(db, func).flags`. There is no new tracked query, tracked wrapper, custom AST scan, main.rs production change in this candidate, or clean-room change.

The inspected flag consumers test individual flags rather than treating the complete bitset as a closed enumeration. Existing signature equality includes the comptime bit. The u16 representation is unchanged. The direct attribute predicate runs inside the existing tracked signature query, preserving its original source and cfg dependencies there rather than creating a new query node. Attribute edits must invalidate the result even when the compact general `AttrFlags` query itself backdates unchanged.

`AttrFlags::has_word` accepts a matching single-segment path meta after cfg_attr expansion. Token-list, key-value and qualified-path forms do not qualify. The implementation calls that exact predicate rather than approximating its syntax or trusting general attribute flags.

## Failed gates retained, final native semantics passed

| Version | Native result | Consequence |
| --- | --- | --- |
| v13 | Production check and build passed. `-p hir-def -- resident_signature_comptime` failed, exit 101: hir-def is an excluded dependency whose dev-dependencies are not available as a resident workspace member. | No semantic pass claimed. Original failure, raw recorder journal, source-ready snapshot and frozen build sources retained. Build exit 0 is distinct from subsequent orchestration exit 101. |
| v14 | Integrated fixture moved into main.rs existing native tests. Check failed E0502 because its baseline held database-lifetime IDs/signatures across mutable `db.set_file_text` revisions. | Failure log and raw journal retained. Runtime gate exit 2 remains blocked, not passed. |
| v15 | Test-only lifetime repair stores owned keys and complete signature snapshots. Native check/build, signature fixture 1, declaration graph fixture 1, listed-root fixture 1, and telemetry tests 3 passed. Full native suite passed **347/347**, zero ignored or filtered, **16.8 s** recorded wall time. | Frozen source hashes remained intact. All 347 actual passing test names are in the raw recorder output and enumerated by the offline audit, not inferred from a summary count. Runtime controls passed, but the timing pair did not establish a win. |

The integrated test `resident_signature_flags_match_direct_attributes_across_revisions` uses `RootDatabase::with_single_file` with native DefMap and trait/impl associated-item APIs. It asserts **12 distinct functions in each of 8 revisions, 96 comparisons**. Every revision reads the cached signature bit and real codegen helper before an independent ORIGINAL DIRECT `AttrFlags::has_word` oracle invocation. Initial true/false expectations guard fixture meaning in addition to the differential comparison.

Coverage includes absent and bare attributes, enabled/disabled/nested cfg_attr, trait declarations/default bodies, inherent and trait impl methods, token-list/key-value/qualified forms, attribute add/remove, cfg_attr toggles, form-to-word changes, restoration, and body-only edits. Both the signature bit and consumer must equal the independent direct oracle.

For the body-only edit check, v15 retains only owned `(container Debug, function name)` keys and complete derived `FunctionSignature` Debug strings, produced while attached to the database. This compares all fields exposed by the signature's derived Debug implementation within the same database across the body-only edit. It is not a portable signature serialization, a cross-database equality guarantee, or a replacement for the per-revision attribute oracle.

Eleven supplied slim exports are retained bundle-locally and bound by exact IDs to the original raw `meta.json`, `output.jsonl`, `summary.json` and unit receipts. The source-ready v13-v15 receipts and genuine root before-build snapshots are retained verbatim. No unsupported assertion of earlier full source availability is made. Native and runtime checks are the actual recorded gates, not proof of every supported Rust language behavior or diagnostic.

## Potential cost and limits

There is no assumed speedup. A warm classification may reuse an already-needed signature instead of repeatedly collecting attributes. Conversely, a cold classification that previously only inspected attributes can force complete signature lowering, including generic parameters, parameter/return types, ABI and source maps. Every lowered function also pays an additional direct attribute collection, even if codegen never classifies it. Attribute or cfg changes can cause additional invalidations. An unchanged u16 size does not establish a memory or runtime win.

RSB04 uses the archived native v4 baseline engine and RSA10 uses the exact v15 candidate executable. Both sessions retain original workload source, all top-level raw outputs, source/toolchain/executable identities, type-error and assertion-failure controls, recoveries, and five edit samples per stack. Remote-off/proc-macro-zero settings are checked in original session metadata. The historical per-command environment allowlist omits those variables, so no independent per-command attestation of them is claimed. Recorded per-command backend/profile fields are checked directly. The installed reference is explicitly separate and not used as a source-built parity claim. The comparison is a filtered libtest diagnostic, not the comprehensive session-weighted synthetic workload. No general speed, clean-room, shipping, or default-promotion claim follows.
