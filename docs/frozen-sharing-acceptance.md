# Frozen client acceptance for sharing

## Why this gate exists

The sharing rollout's normal hosted workflows passed, but another session
replaced the mutable global executable during the acceptance epoch. A passing
run without a client identity receipt cannot establish that every step used
one exact executable. This harness closes the attribution gap prospectively,
not retroactively.

`bench/frozen-acceptance.py` invokes only the caller's explicit regular executable,
checks its expected SHA-256 before and after each workflow, disables automatic
updates in that child environment, and never installs a client or supplies a
run-ID override. It runs check, full tests, strict all-target clippy and repeated
check. It rejects local fallback, compiler-pin changes, missing or ambiguous
invocation metadata, foreign-session records, a changed executable and timeout.
It retains full diagnostics and incomplete receipts for failures.

The harness is not an executable source-provenance attestation or an adversarial
filesystem sandbox. Verify the trusted build/snapshot independently. Do not
claim a speedup from this functional sequence. Do not treat an inactive JR
loader test as a fresh tenant bootstrap or public artifact cache hit.

## Run only after explicit lease release

A known accepted artifact from the 2026-10-14 helper repair is the separate
`client-target-v3/release/justrust` build of source `ec42e0f`. Its digest is
`2153a809b3f91826f4baa9acd81ad5ccc4fb7102f17aacb029469853f38b5a53`.
Use the actual verified artifact path, not `command -v justrust` or a symlink to
the mutable global installation. Preserve its source snapshot and packaging
receipt. This is an example of the verified historical client, not a claim
that future global defaults retain this identity.

```sh
python3 bench/frozen-acceptance.py \
  --client "$JCODE_SCRATCH_DIR/sharing/client-target-v3/release/justrust" \
  --sha256 2153a809b3f91826f4baa9acd81ad5ccc4fb7102f17aacb029469853f38b5a53 \
  --repo /path/to/accepted/source-snapshot \
  --toolchain nightly-2026-10-08 \
  --out-dir "$JCODE_SCRATCH_DIR/sharing/frozen-hosted-acceptance"
```

Keep the legitimate account configuration and billing path. Do not invent an
account, override a run ID, or weaken permissions to produce a record. Do not
run while another agent owns the host or compiler window. This invocation may
negotiate a daemon from the explicit executable, so absence of a global install
alone is not permission to disrupt another lease.

The output directory must not exist. Raw logs may contain private paths,
account names or diagnostics. The harness creates its output directory with
mode 0700. Keep logs private and export only path-redacted run records plus a
reviewed receipt. After a timeout, confirm the remote job has ended before
releasing the host window. Every step's recorded `place` must be
`hosted`, `toolchain` must equal the accepted pin, and exactly one new run must
match the current `JCODE_SESSION_ID` and argv. A `complete=false` receipt is
never an acceptance result, even if an individual exit code was zero.

## Observed harness validation, not live acceptance

Fourteen lightweight tests pass without compilers or hosted calls. They cover
exact invocation/order and full logs, digest changes, malformed hashes,
non-executables, symlinks, run-ID/agent overrides, local fallback, wrong compiler,
missing/ambiguous/mismatched records, unrelated and old records, actual parser
help/rejection, timeout diagnostics, failed workflow receipts and no-clobber
output directories. Simulated process/metadata checks are synthetic validation.

## Actual frozen hosted acceptance

At 04:12:42–04:13:16 UTC on 2026-10-14, after explicit host lease release,
the known `2153a809...` client ran all four real workflows against the separate
`ec42e0f` snapshot. All 66 source/test/asset/manifest Git blobs matched the
committed input. Check, full tests (386 passed, three ignored), strict all-target
clippy and repeated check exited zero. Every per-step executable digest guard
and hosted/compiler-pin/own-session metadata guard passed. The reviewed receipt
is `docs/sharing-frozen-acceptance-20261014.json`, and the four exact redacted
records are in `data/runs/sharing-20261014/2026-10.jsonl`. Private raw-log digests
were checked against the receipt. No install or service default change occurred.

This is actual end-user functional acceptance of the guard and frozen snapshot,
not a controlled timing comparison. It does not retroactively attribute the
older mutable-client epoch or attest every remote compiler byte.

Fleet final publication, a fresh public dependency hit, a genuinely new host
consuming bootstrap configuration, and portable native-script closure remain
separate pending rollout gates. The general functional receipt cannot substitute
for those outcomes.
