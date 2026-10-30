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
one local-path, check-only, non-test leaf library unit with mtime or native
content-checksum dep-info, no
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
the native dep-info reference in mtime mode, and output mtimes changed since native validation.
Content mode instead requires each native dep-info length and checksum to match
current bytes using the recorded algorithm, including Cargo's default BLAKE3.
It records `native_freshness_method`, `native_dep_info_checksum`, and
`native_dep_info_length`. Missing or mismatched native checksums deny support.
A timestamp-only change is not rejected in content mode.
These checks are **not** an atomic filesystem snapshot or content proof tying
source bytes to historical compiler outputs. Concurrent same-timestamp edits,
mutation after capture, or uncovered discovery changes can still race. Hence
`trusted` and replay eligibility remain false regardless of the observations.
Each unit explicitly reports `compiler_read_content_bound_to_outputs: false`,
and the top-level denials include
`compiler-read-content-to-output-binding-not-captured`. A fresh mtime decision
plus newly observed content hashes cannot establish that those bytes produced
an older artifact. The public backdated same-length edit regression exercises
this exact counterexample through actual native Cargo, not a simulated receipt.

A checksum-backed observation must still distinguish checksums derived from bytes
actually loaded by the compiler from checksums obtained by reopening tracked
files when dep-info is written. The separately implemented builtin-include read
binding covers `include_str!` and `include_bytes!` in our compiler, not every
tracked input or arbitrary compiler selected by Cargo. The compiler's `write_out_deps` uses
SourceFile checksums for ordinary source-map files but reopens extra tracked
files. A checksum-looking field alone is therefore not a complete historical
read proof. None of these observations removes the planning/config/discovery/
toolchain or remote-mapping replay denials.

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

The backdated same-length counterexample also passed through actual hosted
native Cargo: check `20261029-215544105-3156861`, five public CLI tests
`20261029-215552085-3156859`, and normal Clippy
`20261029-215712627-3167092` with the same ten inherited warnings. The test
observed `native_fresh: true` while the source digest changed, and verified the
explicit false compiler-read binding and replay denials. This is a rejected
unsafe replay hypothesis, not a new cache hit or speedup.

The hosted endpoint's DNS lookup timed out for the original validation, so justrust
explicitly fell back to local execution. The public tests used the actual final
native Cargo binary, including parentless-sink and non-UTF8-path fail-open
hardening. This validates producer behavior, not a hosted consumer or a no-op
performance improvement.

## Content-checksum prerequisite acceptance, 2026-10-30

The additive content-mode observation is opt-in and does not enable checksum
freshness by default. The actual native CLI test requests `-Zchecksum-freshness`
and `--config 'build.fingerprint="content"'`. It verifies a fresh leaf records
native BLAKE3 checksums, a same-length backdated edit becomes dirty, the next
unchanged invocation becomes supported, and a timestamp-only change remains
supported. All replay/trust flags remain false throughout. The separate mtime
counterexample remains in the suite, so the distinction is tested explicitly.

Hosted validation of the final producer and public executable:

- Library check: `20261029-223459011-3499713`.
- Six actual native CLI tests: `20261029-223502373-3499705`.
- Extended seven-test public CLI suite: `20261029-223850129-3526908`.
  Same-length backdated edits to `include_str!`, `include_bytes!`, and module
  sources each invalidate the native content fingerprint. Subsequent unchanged
  runs record native checksums. Environment absence-to-value and value-to-absence
  transitions also invalidate, with no plaintext value in receipts. This is
  ordinary ordered-edit acceptance, not a concurrent compiler-read proof.
- Direct collector rejection test: `20261029-223703501-3515493`, validating
  SHA-256 acceptance, wrong-length denial, and equal-length changed-content
  denial. This unit test supplements, not replaces, the six actual CLI tests.
- Library Clippy: `20261029-223521567-3502818`, passing with the same ten
  inherited warnings, not a warning-free whole-tree claim.

No client consumer, native toolchain selection, result replay, or performance
promotion is included. A complete replay design still needs authoritative
planning/configuration/discovery and negative-input closure, toolchain identity,
output provenance, mutation barriers, and a consumer that rejects unsupported
commands rather than trusting a current hash inventory.
