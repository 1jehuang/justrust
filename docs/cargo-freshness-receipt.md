# Native Cargo freshness-input prerequisite, schema 1

This is an **opt-in producer prerequisite**, not a complete action cache or a
source-only no-op certificate. Native Cargo still resolves, plans, validates,
and executes normally. No receipt authorizes skipping Cargo, rustc, tests, or
hosted synchronization. This change makes no latency/speedup claim.

## Public native interface

Set `JUSTRUST_CARGO_FRESHNESS_RECEIPT` to an explicit local output file and
`JUSTRUST_CARGO_FRESHNESS_NONCE` to a unique, nonsecret caller correlation ID.
Both are read through Cargo's tracked `GlobalContext` environment API. Unset or
empty output path means no capture. A missing/empty nonce denies prerequisite
support. A consumer must match its fresh nonce, schema, scope, and sealed native
result. An old file after manifest/config parsing failure or process termination
must not be accepted. The nonce is not an authentication token.

Do not forward an arbitrary local output path to hosted builds. This interface
has no hosted consumer or remote external-path mapping. It is not automatically
installed, selected, or enabled by justrust.

## Native authority and bounded support

The observation hook runs at `prepare_target` after native
`compare_old_fingerprint`. It records Cargo's actual fresh/dirty decision, not a
separate guess from a source directory scan. The supported prerequisite slice is
one local-path, check-only, non-test leaf library unit with mtime dep-info, no
dependencies, build script, proc macro, compiler wrappers, extra compiler flags,
or cross/custom target. Cold, dirty, failed, and unsupported invocations produce
explicit denials. Successful compilation does not retroactively turn a dirty
unit into a supported fresh observation. The next real fresh invocation can.

For a supported leaf, `files` come from the decoded Cargo dep-info and include
all compiler-reported source/include/module inputs, native dep-info, native
fingerprint file, and manifest context. `environment` comes from the actual
rustc dep-info environment dependencies. Output paths come from Cargo's native
fingerprint output set. Compiler version/native fingerprint/config hash fields
are context, not proof of the toolchain or planning closure.

Every receipt has these non-negotiable fields:

```json
{
  "schema": 1,
  "scope": "unit-freshness-input-prerequisite",
  "trusted": false,
  "command_replay_eligible": false,
  "content_snapshot_authorizes_freshness": false
}
```

`prerequisite_supported: true` means only that this bounded native unit-input
observation completed after a real fresh comparison. It never changes the
fields above. `replay_denials` always enumerate missing planning/resolution,
config-file/environment, directory discovery/negative-input, compiler/sysroot/
native-library, and remote external-path-mapping authorities. Canonicalized
native dep-info is not a record of symlink topology or all directory probes.

## Races, privacy, and failure behavior

File records contain length, mtime, and SHA-256. Before/after metadata and Unix
identity/ctime checks reject files changed during hashing. The producer also
rejects a native fingerprint changed around capture, source mtimes newer than
the native dep-info reference, and output mtimes changed since native validation.
These checks are **not** an atomic filesystem snapshot or content proof tying
source bytes to historical compiler outputs. Concurrent same-timestamp edits,
mutation after capture, or uncovered discovery changes can still race. Hence
`trusted` and replay eligibility remain false regardless of the observations.

Environment values are represented by presence and SHA-256, never copied in
plaintext. File paths, content hashes, variable names, and compiler identity are
still private data. Output uses a private tempfile (0600 on Unix) and atomic
replacement without following an existing sink symlink. It does not create
missing sink directories. Publication errors are fail-open debug logs, never a
change to Cargo's exit status or stdout. An in-progress receipt is published at
capture start and sealed succeeded/failed on normal returns.

## Validation

Public native CLI integration tests launch the actual built Cargo executable,
not a duplicated collector or mocked parser:

```sh
cd toolchain/cargo
justrust check -p cargo --lib
justrust test -p cargo --test jr_freshness_receipt
```

Coverage includes cold versus fresh, module/include inputs, environment absence
and changes without plaintext disclosure, output deletion, missing nonce,
default-off behavior, unwritable/missing output parent, private atomic sink
replacement, failed builds, and explicit script/proc-macro/dependency/flag/
external-input denials. All checks keep whole-command replay denied.

Observed validation runs:

- Native library check passed: `20261029-211335909-2705512`.
- Final public CLI suite: four tests passed, `20261029-211619753-2745232`.
- Native library plus public test Clippy passed:
  `20261029-211621066-2745624`, with the ten inherited cutoff/daemon warnings
  already documented by the subtree-cache work. No warning-free whole-tree
  claim is made.

The hosted endpoint's DNS lookup timed out for this validation, so justrust
explicitly fell back to local execution. The public tests used the actual final
native Cargo binary, including parentless-sink and non-UTF8-path fail-open
hardening. This validates producer behavior, not a hosted consumer or a no-op
performance improvement.
