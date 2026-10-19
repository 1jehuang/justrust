# Input-scaled standalone object-layout validation receipts

Source checkpoint: `b06efac54615409f21bcab6574f6384774693b2c`, private and
unregistered, rebased onto `fd2712074`. The source change touches only
`src/compiler/object_layout.rs` and `tests/object_layout.rs`. Original fixed-cap
checkpoints `5633a107b` and `a1ea60f25` remain preserved; their six receipts in
`data/runs/object-layout/` are unchanged and are not evidence for this replacement.

## Exact commands and placement

All commands used the installed stable justrust client from the private
worktree root. The local C-only slot was explicitly authorized by the coordinator
on October 19, 2026 at 06:43 UTC. No local C run preceded that authorization.

| Run ID | Command | Verified placement | Verdict |
| --- | --- | --- | --- |
| `20261018-232514461-2882537` | `justrust check --remote --test object_layout` | forced hosted | exit 0 |
| `20261018-232537063-2890870` | `justrust test --remote --test object_layout -j 2 -- --test-threads=1 --skip c_scalar_and_array_object_layout_controls` | forced hosted | 24 passed, 1 C test filtered |
| `20261018-232608120-2905251` | `justrust clippy --remote --test object_layout -- -D warnings` | forced hosted | exit 0, no warnings |
| `20261018-232615593-2905230` | `justrust test --remote --test graph_integrity -j 2 -- --test-threads=1` | forced hosted | 10 passed, unchanged ratchets |
| `20261018-234402059-3266586` | `justrust test --local --test object_layout -j2 -- c_scalar_and_array_object_layout_controls --exact --test-threads=1` | explicitly authorized local C-only | 1 passed, 24 filtered, exit 0 |

Exact-ID `justrust show` confirmed `remote on hosted` for all four hosted
receipts and `local (JUSTRUST_REMOTE=off)` for the C-only receipt. Hosted used
`nightly-2026-10-08`; local used `jr-ac20de5e7921`. Unit labels such as `local`
inside a hosted compilation report mean workspace crates, not local placement.
The shared latest-run pointer was not used as placement evidence.

The four hosted gates ran on the same two tracked modified source files before
committing them as `b06efac54`. The C-only run verified clean, unchanged HEAD
`b06efac54615409f21bcab6574f6384774693b2c` before execution; its exact-ID show
records that checkpoint with zero modified files. Source hashes were checked
before the hosted gates, before the source commit and before the C-only run:

- `src/compiler/object_layout.rs`: SHA-256 `f4bb393ecce0cdced7e407bf7349f6f500182fb465eb77817fd15b4ceeb6fd3c`
- `tests/object_layout.rs`: SHA-256 `6c170b2f3d73bfaf7372d955c45385ebd9cab50cff618fcd573211ba3df1d659`

## What this verifies

The replacement removes independent fixed key-word and frame-depth constants.
Complete private grammar validation, including pointer suffixes, uses the
existing caller PlanBudget depth/work admission. That budget's existing default
and constructor depth clamp remain 128; no higher-depth acceptance is claimed.
Only the root by-value array frontier requires layout frames. Pointer layout
does not calculate or demand its pointee layout.

The key owns its input-sized buffer and immutable accounting receipts. Scratch
owns task-local leases and destroys buffers before releasing them. Controls cover
actual capacity/header accounting, zero-prefix and zero-sized scratch wrappers,
checked storage arithmetic, synthetic allocator failure and surplus rejection,
full old-plus-new overlap preflight, drop-before-refund behavior, sticky resource
failure, moved rather than copied keys, exact authority/type/target/receipt checks,
forged shape/child/validity rejection and borrowed-input independence.

Requested allocation capacity is prepaid. Allocator-returned excess is admitted
immediately before filling or publication, with fail-closed discard on rejection.
This is the expressly accepted standalone bootstrap policy, not proof that an
allocator-rounded transient physical peak was fully prepaid. Surplus/failure
injection controls are synthetic tests, not allocator/RSS measurements.

Bootstrap Rust controls compare admitted scalar, unit, thin-pointer and fixed
array size/alignment/stride. The C-only test compiles and executes a C11 oracle
for 17 scalar/pointer storage shapes and their three-element arrays. C uint32_t
checks Rust char storage, not Unicode validity; C has no standard unit/ZST oracle.
These are not native compiler addressed reads/writes, bidirectional FFI calls or
platform calling-convention acceptance.

Hare independently reviewed the exact source hashes for production allocation,
receipt authority, drop order, moved-key ownership and pointer-suffix boundaries,
with no new concrete blocker found. This was read-only inspection, not independent
execution of these gates.

The existing graph integrity gate passed without ratchet changes. Because this
module is unregistered, that pass does not establish a new graph fact's source
admission, dependency shape or reachable-state integration. Accounting receipts
are not canonical semantic/source/invocation authority. No source intrinsic,
nominal layout, addressed backend storage, pointee cleanup, Box, full-Rust, full
suite or speedup claim follows from these controls. No source or backend
registration was added, and no full build/test suite or push was performed.

## Export provenance and exact scope

Stable-client `justrust export` ran from the worktree root into an isolated
scratch output root, with machine label `object-layout-dynamic`. Exactly the five
IDs above, all belonging to this lane's agent session, were selected from that
path-redacted export. Only those five original exported records appear in
`2026-10.jsonl`; no unrelated history or raw private paths were copied into this
checkout. Export redaction is preserved, and the command/placement annotations
above supply the separately verified scoped context. No results were synthesized.

This evidence addendum changes only this directory's JSONL and README. It does
not alter the two source files, the original six prototype receipts or
HEAP-LAYOUT.md.
