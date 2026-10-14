# Whole-result sharing follow-through, 2026-10-14

This closes the final requirement-level follow-through on the accepted installed source `059674bf76fa218ba7e475279a86b4b6caf32aac`, executable SHA256 `0a19acdd5be8fd2963cda9bb551db508fe9a7875db69575c09a8fe5bb294b56d`. It does not attribute a changing global symlink or unrelated peer source to that binary.

The adjacent JSON maps **18 explicit requirements** individually to checks, observed behavior and scope. It includes 71 named actual sharing/native test outcomes from the exact frozen project's complete 410-pass/3-ignored run, not merely its aggregate count. The retained raw output SHA256 is `bd70ea176517bb21b6003731fcb8e50e49694911e1b3fc1f87d689544a802893`.

## Fresh actual public-interface rerun

At 05:36:01–05:36:07 UTC, the unchanged immutable client ran ordinary `check --local --locked` and `test --local --locked` in four independently empty Cargo/target/dependency-cache roots, nightly/full-JR × default/debug=0. All eight commands exited zero; each actual memchr compiler unit was restored from `prebuilt`, with zero dependency misses. Four test commands passed two runtime tests each. This exercises the real client, Cargo, published dependency bytes, compiler selection, linking and test execution, not stubbed compilation.

Actual run IDs:

| Compiler/profile | Check | Test |
|---|---|---|
| nightly/default |20261013-223601560-1850759|20261013-223602544-1851044|
| nightly/debug0 |20261013-223603222-1851203|20261013-223603940-1851304|
| full JR/default |20261013-223604528-1851451|20261013-223605395-1851551|
| full JR/debug0 |20261013-223606062-1851743|20261013-223606896-1851837|

Public fetching was forced for correctness acceptance, not a default latency policy or speed measurement. Account/peer/shadow reuse was disabled. The tiny public memchr2.8.3 application is the committed matched consumer fixture, not a copied compiler implementation or a mock.

## Native safety through ordinary Cargo, not only a plan harness

At 05:39:21–05:40:04 UTC, **20 ordinary Cargo test workflows** used the real nightly compiler and the same public application. They covered all eight valid native CPU flag forms, a real `@file` containing native flags, and generic explicit x86-64 CPU controls. Every command passed two runtime tests, for 40 runtime test results.

Two distinct target directories per case forced compilation instead of Cargo's unchanged-artifact freshness shortcut. The cache stores were shared only within each task-private case. Actual compiler unit records showed:

- 16 native-flag workflows and two response-file workflows: no dependency or local artifact-cache reuse.
- Two generic workflows: dependency miss then hit, preserving ordinary reuse.
- The actual shim's earlier 40 invocation regression remains separately mapped: 36 native/@ bypasses, four generic controls, 38 compiler calls and two restores.
- Real bundled C code with unchanged Rust metadata has a separate miss/output1, miss/output2, hit/output2 compiler/link/runtime regression. The retained old CLI failure is not rewritten as success.
- Real proc-macro native helper1/2 expansion and full executable hashing are independently mapped to named frozen-project tests.

Actual ordinary native test run IDs and per-case flags, unit cache observations and raw output hashes are in the adjacent JSON. The guard did not rewrite compiler arguments or replace the compiler. These are native-flag safety results, not proof of cross-machine hermetic native build-script reuse or a demonstrated cross-CPU stale executable.

## Actual failure and recovery

`20261013-224400547-1905506` deliberately compiled a task-private type error with native flags. It returned **101** with real **E0308**, retained in the export rather than hidden. Restoring the original public fixture bytes from the unchanged source copy produced `20261013-224401789-1906049`, exit0 and two passing tests. Both actual unit paths bypassed caches. No user repository source was modified by this error fixture.

The first native acceptance attempt returned client exit0/two passed (`20261013-223803007-1860524`) but the acceptance script returned1 because it expected raw libtest semicolon output instead of justrust's compact comma report. The guard was corrected and new independent v2 roots were used. A separate public attribution guard first assumed `name`; the actual unit schema uses `crate_name`. Both acceptance-only mistakes are preserved in the JSON and are not relabeled as production failures.

## Package, edge and integration checks

- Two actual `justrust toolchain install <existing-id>` CLI reuse calls passed. Prefix marker inode and SHA remained unchanged, and the JR `installed_at` marker remained zero.
- Twelve real compiler/helper binaries (rustc, cargo, rustdoc, rustfmt, clippy-driver and jr-cargo for both prefixes) executed `--version` successfully with ambient loader overrides cleared. Their executable and output hashes are recorded. The previous actual Ubuntu broken-helper127 observation and corrected full immutable package remain distinct.
- A fresh complete sharing-specific cloud boundary rerun passed **91 named cases**. The JSON lists each case, rather than treating the total as a requirement map: private/shared authorization and sizes, billing refusals, no-launch failures, quota/reclaim races, bootstrap fixed-origin/no-JR-activation, trusted publisher ordering, registry tampering, rootfs nonauthorization, shell matrix exact pins, sanitized status failure behavior and template invariants. These offline fixtures are supporting edge evidence, not live hostile cross-account probes.
- The exact installed full project already passed hosted check, all410 tests/3 ignored, strict all-target lint and recheck after all shipped source changes. Its 71 relevant named outcomes were re-audited from actual raw events now. The final Desktop FPS edit/repeat/backup-copy restoration remains actual end-user workload acceptance. No new hosted call was made during the peer's exclusive graph-capture lease.

All **31 newly cited run records**, including the genuine101 and the first acceptance-script/client0 result, are committed in `data/runs/sharing-final-whole-20261014/2026-10.jsonl`. Raw logs remain task-private, with hashes in the JSON. Earlier evidence stays untouched.

## Honest remaining gates

The future east fleet version3 configuration and unchanged non-UserData infrastructure were actually verified; its next runtime status receipt still awaits the natural scheduled producer. The read-only observation is scheduled for 10:45 UTC, without launching new machines.

Genuine first-new-tenant bootstrap remains unobserved. Existing-host fresh-prefix acceptance is not substituted for it, and no accounts were fabricated or existing host volumes destroyed to manufacture that event.

Native build-script public authorization remains **false** until the trusted mounted native closure, kernel/CPU/environment determinants and ABI/runtime acceptance exist. Rootfs candidate filesystem tests and synthetic namespace probes do not authorize public replay. Registry eager seeding remains off because its complete measured network cost was slower for this workload (1.697s versus direct1.057s). No performance improvement is claimed by these functional checks.
