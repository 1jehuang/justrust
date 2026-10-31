# Watched manifest and lock bytes: historical v76 evidence

This archive binds root-executed October 31, 2026 evidence to immutable accepted
source `0aa1d0727` and original v74 base `de8533b93`. Export and replay do not run
the native engine. The implementation is licensed Blaze-derived resident code,
not a clean-room compiler. Existing MIT/Apache-2.0 attribution is retained in
`lineage/`.

## Accepted, bounded experiment

The distinct `watched-manifest-retouch` policy fixes
`RA_JIT_MANIFEST_CONTENT_GENERATION=1` in BOTH candidate arms and varies only
`RA_JIT_WATCHED_MANIFEST_BYTES=0/1`. Reference has both flags zero. The original
v74 schema is not interchangeable. One frozen native ELF, full inventories
(native source 3757, toolchain 4020, pinned source 63), isolated states, raw
runtime nonces, exact source recipes, byte/mtime receipts and process identity
proofs bind the experiment. Both watcher arms retain PID/startticks/executable
and private socket identity across manifest AND lock retouches.

Completed `wm76b` has 54 calls, 20 measurements, four warmups and five successful
stops. All four segment-first samples remain in the estimator. Recomputed OFF
median is 2.457933 seconds, ON 0.054515 seconds, descriptively -97.78%. This is
combined WATCHED manifest+lock retouch only, with unknown noise. It is neither an
independent manifest/lock attribution nor a general speed, causal, comprehensive
workload or default-promotion claim. Default remains OFF. The historical root
compare was separately reported at 15:19:17.896Z, not saved as stdout. The archive
labels its posthoc witness and independently reconstructs arithmetic from actual
raw samples rather than fabricating a historical transcript.

Root correctness evidence comprises named native 53/373 tests, all four new
fixture names, cfg9, native environment/file ABA6, strict28 and Python regression
761/51.106s. Strict replay extracts hash-bound original tuple oracles, never a
JSON-list-weakened substitute. Full source/lock before-after journals and exact
frozen source patches join originals, candidates, final documentation regression
and root licensed-lineage clarification. Original v74 harness SHA remains
`1508a424fde23d7093ae004bc476abcf95d7681e36ed61d9d5c923de9f9cb2df`.

### Supplemental actual manifest-as-script-input gate

Root task `979857g6da` passed actual native 11/22/11 in watcher OFF and ON. The
real build script explicitly watches Cargo.toml and invokes cc/ar. Six fresh
runtime nonces, unchanged source/manifest bytes, controlled future timestamp
guards, append-only actual script invocation records, identities and successful
stops are replayed. This is correctness only, not a new timing estimator or
performance claim. No source fix was needed. Only the executed v77-future
candidate is accepted. Its earlier script is preserved solely as a patch
antecedent, not evidence of an earlier executed proposal.

### Incomplete run retained separately

`wm76a` reached 52/54 rows, only 51 completed, before the combined root job
`973809gnbi` outer 600-second timeout (exit124). It is INCOMPLETE, not a native
correctness failure and never merged into accepted samples. Separate cleanup
proves successful stop and restored pristine pinned63. Original backup SHA/size
receipts are retained without modifying originals or deleting private stores.

## Offline replay and adversarial checks

```sh
python3 -B bench/results/resident-watched-manifest-bytes-20261031/validate.py --manifest
python3 -B bench/results/resident-watched-manifest-bytes-20261031/test_offline.py
```

58 Python tests pass, including coherent resealed negatives for exact flags,
mode/schema mixing, source/nonce/gates, PID/startticks/executable/socket,
bytes/mtime, all first touches, medians, script appends/future-clock guards,
patch/source joins and incomplete-run rejection. Relocation runs with empty
HOME/PATH under an audit hook denying external reads/listings, all writes,
process/native-library execution and network. Actual denied attempts are tested.
The replay loads only bundled hash-bound pure oracle definitions. Archived
drivers are text, never executed or imported. Native correctness comes from
root records, not Python mocks.

`SHA256.json` seals every package file. A fixed observation commitment in the
reviewed validator distinguishes historical authenticity from self-consistent
resealing. Trust requires the reviewed Git commit and validator, not a mutable
checksum file alone. `export_private.py` is a separate archival authoring tool
which uses read-only Git and private input files, not the offline replay.

## Privacy and preservation

Export uses a closed environment allowlist (backend/optimization/profile controls
and the two exact factor flags). It retains selected raw stdout, raw-byte hashes
and sizes, selected diagnostics and normalized process/source/socket roles, never
ambient environment, raw private proc command lines or private absolute paths.
Private originals have SHA256/size receipts and are checked unchanged after
export. Public pinned source and immutable licensed source blobs remain exact,
including any public fixture literals. Selected records are explicit projections,
not claims that entire private logs were published.
