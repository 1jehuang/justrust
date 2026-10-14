# Dependency bundle size policies

`JRDC1` wire bytes and cache keys are unchanged. The static public store is not
served by the authenticated cache Worker and must not inherit its request cap.

| Path | Compressed bytes | Expanded stream bytes |
| --- | ---: | ---: |
| Authenticated private/shared Worker, peer unpack | 95 MiB | 380 MiB |
| Static public prebuilt read and publish | 512 MiB | 2 GiB |

Both unpack policies enforce the on-disk gzip size before creating the stage,
stream expansion with an explicit EOF/budget distinction, at most 64 output files
plus `meta.json`, portable flat file names, unique file lists, a 256 KiB metadata
budget, gzip integrity, and existing binary content hashes. Text dep-info retains
its existing normalization semantics. Public publishing preflights the same
expanded size and file count, rejects symlinks/non-files and unsafe/duplicate
names, and bounds compressed writes while packing (it never needs to write an
oversized object before rejecting it).
Public HTTP reads also require curl success, not just a reported HTTP 200.

`justrust-cloud/src/cache.js` continues to enforce `MAX_OBJECT_BYTES = 95 MiB`.
No server deployment, cache migration, key change, or protocol sync is needed.
User-configured static stores use the same bounded policy as the default public
store. Raising the public limit does not relax the authenticated shared tier.

Regression coverage includes exact budgets, compressed/expanded rejection,
record-boundary exhaustion, oversized metadata, gzip CRC/truncation, unsafe paths,
symlinks, binary tampering, and actual static publish/fetch/local restore of a
96 MiB incompressible fixture whose gzip exceeds the Worker cap. That fixture is
not a real compiled dependency or performance benchmark.

Local validation is explicitly approved by the coordinator while the hosted
machine is leased to profiling. Commands use a separate target directory and
`JUSTRUST_PREBUILT=off JUSTRUST_REMOTE_CACHE=0` to avoid public/cache network calls:

```sh
CARGO_TARGET_DIR="$JCODE_SCRATCH_DIR/public-bundle-target" \
  JUSTRUST_PREBUILT=off JUSTRUST_REMOTE_CACHE=0 justrust check --local
CARGO_TARGET_DIR="$JCODE_SCRATCH_DIR/public-bundle-target" \
  JUSTRUST_PREBUILT=off JUSTRUST_REMOTE_CACHE=0 justrust test --local depcache_
CARGO_TARGET_DIR="$JCODE_SCRATCH_DIR/public-bundle-target" \
  JUSTRUST_PREBUILT=off JUSTRUST_REMOTE_CACHE=0 justrust clippy --local --all-targets -- -D warnings
cargo fmt
(cd ~/justrust-cloud && npm test)
```

Reinstallation and real hosted large-artifact acceptance must be coordinated with
the host owner after the exclusive lease ends. Record raw and gzip sizes
separately: a 118 MiB raw bedrock output is not proof of a gzip above 95 MiB.

No speedup is claimed.

## Validation evidence (October 14, 2026 UTC)

- Check passed: `20261013-193756540-689216`.
- Public/remote dependency-cache tests: 17 passed, one opt-in real-artifact test
  ignored in normal runs: `20261013-193948705-707889`. The synthetic artifact was
  100,663,296 raw bytes and 100,756,940 gzip bundle bytes, above the unchanged
  99,614,720-byte authenticated cap. Public publication and empty-cache restore
  succeeded, while authenticated unpack rejected it before creating a stage.
- Opt-in local bedrock rlib publication/restore passed:
  `20261013-193803661-690798`. The existing compiled rlib was copied read-only
  into an isolated test plan. Its 129,090,240 raw bytes packed into a
  25,839,023-byte bundle and restored hash-identically. This real sample does
  **not** cross the old gzip cap and is not evidence of a newly enabled bedrock
  hit. Its original artifact, project, and cache were not modified.
- `cargo fmt` and warning-free all-target clippy passed:
  `20261013-194104060-715892`.
- Unchanged cloud protocol suite: 81 Node tests and 11 Python tests passed.
- Shared-source full suite `20261013-193816435-693360`: 368 passed, four failed,
  three ignored. Failures were in concurrent toolchain bundle/staging work, not
  dependency bundles. Their owner was notified. Consolidated full-suite and
  hosted acceptance are coordinated by the root agent.

The real-artifact check can be repeated without compiling or touching its source
project or cache:

```sh
CARGO_TARGET_DIR="$JCODE_SCRATCH_DIR/public-bundle-target" \
  JUSTRUST_PREBUILT=off JUSTRUST_REMOTE_CACHE=0 \
  JUSTRUST_TEST_PUBLIC_ARTIFACT=/path/to/existing/dependency.rlib \
  justrust test --local public_real_artifact_round_trip -- --ignored --nocapture
```
