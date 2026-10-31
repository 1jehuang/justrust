# Native reload correctness evidence, 2026-10-31

**Default correctness only. Whole-generation reuse is UNIMPLEMENTED. No speed claim,
no default promotion and no general performance-goal completion.** This source-only
bundle retains the coordinator's already completed native work for source fix
`69669f0c7` on `stable`. Collection and validation execute no native jobs. The
licensed Blaze-derived resident engine is not the clean-room compiler.

## Immutable inputs and actual outcomes

| Existing experiment | Acceptance | Actual native values |
| --- | --- | --- |
| native-link-reload-v56-original | true, false, true | 11, 11, 11 |
| native-link-reload-v56-candidate | true, true, false | 11, 22, 22 |
| native-link-reload-v58-candidate | true, true, true | 11, 22, 11 |
| native-file-reload-v58-original | true, false, true | 11, 11, 11 |
| native-file-reload-v58-prerequisite | true, true, false | 11, 33, 33 |
| native-file-reload-v58-candidate | true, true, true | 11, 33, 11 |

The environment probe requests A/B/A, expecting 11/22/11. The file probe keeps
its environment and link directives constant, changes `a.c` 11/33/11, and
records source hashes at each call. Both exact Python probes and fixture
Rust/C/build-script/manifest/lock bytes are retained. Actual original A/B
archive disassembly is retained under `runtime/native-link-reload-v56-original`.
`native-artifact-identities.json` records read-only, post-run hashes of objects
and archives still present in preserved stores, not copies of binaries and not
a new disassembly or native run. Its paths identify the corresponding A/B
artifacts. No binaries, targets, private homes or caches are published.

`manifest-cfg-reload-v58` has nine accepted valid/error/recovery rows across
original/OFF/ON. The ON request does **not** implement generation reuse.
`native-reload-v58-seven-state` has seven accepted cold/noop/manifest/type-error/
recovery/assert-failure/recovery states with `RA_JIT_WHOLE_RELOAD=0`. Exact raw
runtime nonces, compiler errors, assertions, test results and status/worker/store
receipts are retained. Passing these controls establishes scoped default native
reload correctness, not same-worker changed-generation retention.

## Source, builds, and failure history

* `frozen/v55`, `v56`, `v57`, `v58`, `v58-final-doc` preserve full original and
  candidate bytes, patches, original SHA inventories and release decisions.
  The standalone validator applies every unified patch in memory using only
  Python stdlib, checking exact original context and candidate output. Unpatched
  candidate files must equal their originals. No `git` or `patch` executable is
  required.
* `gates` retains existing build and unit logs, exit receipts, 3,757-file source
  hash inventories and root `Cargo.lock` identities. `provenance` retains the
  exact root build launchers. `journals` retains the private justrust runs'
  metadata, complete output journals and summaries with environment sanitization.
* Both v56 and v57 check/build and 53-codegen-test gates passed. Each full resident
  test gate failed with **361 passed, 1 failed**, exit 101. v56 failed
  `native_reload_cfg_only_rechecks_unreachable_errors_and_recovers`; v57 failed
  `resident_diagnostic_skeleton_cache_matches_fresh_errors_after_each_edit`.
  Those failures are retained, not rewritten as acceptance. v58 passed **53
  codegen and 364 resident tests**, exit 0, with full source/root-lock restoration.
* v58 binary identity is
  `005c6198fddf1a7a77b6fda6d0be27601a0a48a7f2279deac460ebf5d4898edd`.
  The exact `engine-version.txt` identifies build
  `resident-v1-c404975eaca237bc7dcbcf336de7f22d1374639ccd096f0c07bf7dadcb38b1e8`.
  The historical v56 link candidate and v57 file prerequisite have distinct
  binary identities, both separately bound by the validator.
* `provenance/v58-bench-regression.log` retains the existing **733-test PASS**.
  This is not a performance benchmark. Cleanup script and completed stop receipt
  are retained under `provenance` and `cleanup`; `stores_deleted=false`.

## Privacy and reproducibility boundary

`privacy.json` binds each private original SHA/byte count to its published
SHA/byte count and records recursive environment-map entry/omission counts.
There are **424 retained files, 90 environment maps and 24 omitted entries**.
The allowlist is explicit and enumerated, never a permissive prefix match.
Omitted names and values are not published. Every private original was read-only
and rehashed after collection. Complete original bytes remain at their original
private locations. Public environment-bearing JSON is a **sanitized derivative**,
not a claim to retain raw ambient environment. Non-environment metadata, source,
raw stdout/stderr, launcher bytes, diagnostics and identities remain bound.

`validate.py` checks semantics as well as the public `SHA256.json` inventory.
A fixed independent historical evidence digest binds exact retained nonces,
raw output, errors, status, source/tool/toolchain identities, failure history,
privacy accounting and artifact identities. Re-signing the mutable outer
inventory or derivative hashes alone cannot silently replace history. This is
an integrity commitment rooted in the committed validator, not a cryptographic
signature from a third-party witness and not proof that omitted private bytes
can be reconstructed publicly.

```sh
python3 -I -B bench/results/resident-native-reload-20261031/validate.py --audit
python3 -B bench/results/resident-native-reload-20261031/test_validate.py
```

Python 3.9+ stdlib is sufficient. The bundle may be copied anywhere, without the
checkout, original scratch directory, toolchain, native engine, network or build
artifacts. The validator reads only its own bundle. Its audit hook rejects
external reads/directory access, writes, native process execution and network
activity during validation, after Python/stdlib startup. Mutation tests use
Python subprocesses only to exercise the audited validator. They relocate a
copy with empty HOME/PATH and test deny rules with synthetic native/network audit
events, without actually running a native command or opening a network socket.
Set `JCODE_SCRATCH_DIR` to an existing scratch directory if needed for test copies.

The test suite checks coherent mutations after repairing raw SHA/length receipts,
published privacy SHA/lengths, and the outer inventory. It checks wrong native
values, coherently replaced nonces/source/TC identities, diagnostics, failure
history, exact PIDs, status, patch/candidate replay, full journals, launcher,
root lock, omissions, unsafe environment entries, regression count and cleanup.
`checks` contains the retained offline test and audited relocation receipts.
`collect.py` is a historical collection script for the original machine only,
not required for portable validation. Do not execute the archived native probes,
launchers or cleanup scripts to validate this bundle.
