# Reusable operation inputs and global cache authority

This contract separates three questions that must not be conflated:

1. Does a compiler operation have an owned, complete input encoding for its supported shapes?
2. Does the consumer validate its current graph's semantic authority before using the result?
3. Is the artifact's producer authorized and its provenance authenticated?

A source observation receipt, runtime image digest, successful verifier, or local cache hit does not by itself answer all three questions. Global artifact sharing remains disabled in the clean-room compiler.

## Owned operation boundary

The opt-in local cache stores authority-free intermediate values and output bytes, not graph capabilities. Its producers remain inside their ordinary graph nodes. Current input encoders are deliberately narrower than the language implementation:

| Operation | Input boundary | Reuse restriction |
| --- | --- | --- |
| Native ELF link | Ordered function names and object code, export/call/string relocation offsets and payloads, panic entry mode, implementation identity | Vtables are ineligible. External linkers and C ABI inputs do not use this cache. |
| Stack-emitter machine object | Explicit encoding of supported CFG values, types, places, operations, signatures and resolved symbols | Proof-dependent or otherwise unsupported shapes reach the normal backend instead of finding a cache entry. |
| SSA lowering | Explicit supported CFG encoding and current signatures | Current source/body admission still runs. Stored values cannot restore loan, world, or declaration authority. |
| Local/post-inline optimization and SSA machine object | Exact SSA value encoding, relevant current callee signatures/symbols, implementation identity and distinct operation domains | Decoded SSA must pass current structural verification and resource admission. Callee and proof capabilities are reconstructed from the current graph. |

The implementation lives in `src/compiler/link_cache.rs`, `native_cache_key.rs`, `cache.rs`, `ssa/lowering_cache.rs`, and the bounded object/SSA codecs. These are operation-specific contracts, not evidence that arbitrary compiler processes are hermetic.

Changing an operation's semantic inputs requires updating its encoding and version domain. Unsupported forms must remain ineligible until encoded explicitly. Raw process-local IDs, pointer identity, Debug formatting and noncryptographic summaries cannot stand in for stable semantic inputs. Storage placement and paid accounting receipts are not semantic inputs or transferable authority.

Compiler runtime evidence identifies executable/mapped code and bounded process configuration. Linux/x86-64 floating control admission rejects unsupported environments; floating literal conversion also checks the actual executing worker. This does not seal arbitrary mutable memory, CPU observations, external processes, build scripts or procedural macros.

## Local integrity is not producer authentication

`LocalCache` assumes a trusted local directory. Its checksums detect corruption and its bounds prevent oversized decoding. On Linux/x86-64, entry reads reject final symlinks and nonregular files without waiting for a FIFO writer. These properties do not make a directory supplied by another tenant trustworthy.

A well-formed SSA program can still compute the wrong answer for a requested input. Structural verification cannot prove that an untrusted party ran the specified optimizer. Likewise, a checksum-valid machine object is not authenticated code-generation evidence. An attacker who can replace both a payload and its checksum is outside the local cache's integrity guarantee.

Do not implement a global cache by mounting a shared directory as `--cache-dir`, copying account-produced records into a shared namespace, or accepting a caller's runtime digest as publication authority.

## Required global producer and consumer boundary

Multitenant policy belongs in `justrust-cloud`. The existing invariant remains:

- Shared writes require the trusted prebuild fleet's `CACHE_PUBLISH_TOKEN`.
- Account build hosts and user processes never receive that token.
- Account IDs come from backend authentication, and private reads/writes remain under that account's namespace.
- Missing required bindings or secrets fail closed.

A future producer must actually compute the selected supported operation under a trusted, pinned implementation and bind its input key, implementation/operation version and output digest. Relabeling an account-produced artifact, checking only its checksum, or verifying only its SSA structure is insufficient. Untrusted code must not execute with publisher credentials.

A future consumer must retain normal current-graph admission, enforce per-operation decode/resource bounds, and authenticate the producer before interpreting a global result. Transport framing must preserve the complete operation key and output integrity identifiers; the local format does not automatically define a compatible remote protocol.

Private-derived data and metadata require an explicit sharing policy as well as correct keys. Do not put source text, environment values, credentials, tenant identifiers, or private artifacts into public catalog/status metadata. Knowledge of a cache key alone is not proof of source ownership. Lookup timing and hit reporting must be included in the privacy threat model, not silently assumed harmless.

## Offline computed-operation export v1

The standalone compiler accepts `--computed-operation-export NEW_DIR` alongside
`--cache-dir EXISTING_TRUSTED_DIR` for builds, not CHECK. Defaults are unchanged. The output directory must not exist and must be outside captured
source roots. It is private (0700, files 0600). The compiler has no publisher
credentials, transport, or global-cache permission.

This first exporter supports **native-link v1 only**, after ordinary graph reads
and CHECK admission. Only a newly computed eligible link miss writes a receipt.
A warm hit or ineligible operation produces an empty operations list, never a
COMPUTED record. The implementation digest comes from the actual admitted
compiler image and mapped code, not an argument or caller-provided digest.
Each payload is at most 16 MiB, narrower than the local cache's 64 MiB limit.
Explicit export I/O, bounds, provenance or verification failures fail the CLI.
Ordinary non-export local cache failures remain best-effort misses.

On successful compilation the compiler seals `manifest.json`, at most 1024
bytes, as compact UTF-8 JSON with sorted keys and one trailing LF. Format fields
are `format: "justrust-computed-operations"`, `hash: "blake3-256"`,
`version: 1` and `operations`, an array of zero or one records. A record has:

- `operation: "justrust/native-link/x86_64-linux"` and `operation_version: 1`.
- `implementation_digest`, `operation_key` and `payload_digest`: complete
  BLAKE3-256 values encoded as exactly 64 lowercase hexadecimal characters.
- `payload_bytes`: exact bounded byte length, and `payload_path`: exactly the
  operation key, a single relative filename containing the raw payload.

The directory also contains `computed-native-link-v1` for a computed record:
a fixed 104-byte internal receipt (implementation digest, key, payload digest,
then little-endian u64 length). It has no independent pointer authority. Sealing
checks this fixed shape, the key-derived payload path, regular non-symlink files,
length, digest and directory file count. No manifest is sealed on failed source
checks or failed export. Consumers must require successful compiler exit as well
as the sealed manifest, reject unexpected fields/files and verify all bounds,
keys, paths and output digests before transport or interpretation.

**This is evidence for a trusted producer, not authentication of an arbitrary
local directory.** Trusted fleet orchestration must start with an empty private
cache, run an operator-pinned compiler in an isolated credential-free environment
on explicitly authorized public inputs, wait for the entire isolated process tree
to exit, and verify the resulting manifest against that trusted implementation
pin before publication. Never ingest caller-provided caches or export directories.
A link computed using warm intermediate objects is not evidence that those
objects came from trusted computation. The compiler does not attest that a
caller-selected directory was initially empty or grant sharing permission.
Private inputs remain unpublishable without a separate explicit sharing policy.

## Acceptance before global promotion

Global promotion requires real producer/consumer workflows in addition to unit fixtures:

- Mutating each semantic determinant causes a miss or changes the operation key.
- Unsupported shapes cannot acquire reuse eligibility.
- Warm entries cannot hide invalid unused source, changed signatures, missing inputs, or failed current proof/resource checks.
- Corrupted, malformed, truncated and oversized payloads fail open to computation or fail closed to an explicit resource error, never restored success.
- Unauthorized publication and cross-account private reads fail closed.
- A trusted publication is consumed through the actual authenticated path, with its key/version/output binding checked.
- Performance claims use the representative before/after workload, not a cache-hit counter or a focused synthetic fixture.

Frozen source namespaces and bounded captured-tool execution remain useful independent foundations. Repeated acquisition does not prove an atomic live host snapshot, and a few reproducible tool launches do not establish closure over arbitrary kernel, CPU or process state. Full-machine execution experiments are also nonauthorizing until their complete machine/input/provenance contracts and measured costs are established.
