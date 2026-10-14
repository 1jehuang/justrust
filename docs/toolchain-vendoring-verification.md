# Final vendoring requirement-to-result verification

Source under test: `6d898a9ea8ab2f2bfb3cac67736b33baf89dce3d`, October 14, 2026.
This verifies the completed migration, not concurrent execution-graph work.
Compiler sources stayed unchanged. Client and Cargo acceptance used clean Git
archives after another session began editing their working-tree sources.

## Requirement-to-check matrix

| Requirement or changed output | Final check and observed behavior | Evidence |
| --- | --- | --- |
| Own the existing compiler and Cargo forks in one repository | Frozen Git tree has 66,660 ordinary tracked toolchain entries, zero gitlinks, and zero nested `.git`. Shared bootstrap Cargo resolves to `../../../cargo`. Actual compiler/std and Cargo binary builds succeeded from the owned source layout. | Final fidelity, native, and Cargo receipts |
| Include compiler std/sysroot source and required nested dependency | Backtrace has 102 ordinary entries, all matching original blobs/modes. Actual stage-1 compiler, Cranelift backend, and optimized std build passed. Allocation, arithmetic, and thread tests executed on both backends. | Final fidelity and native receipts |
| Remove inherited development policies without removing legal attribution | Frozen policy audit found no inherited agent/AI/review policy gates. Root guidance governs surviving technical landing pages. All 80 license/attribution candidates match original blobs and modes. | Final fidelity receipt |
| Preserve technical fixtures and imported source bytes | All 19 original CRLF/mixed entries match original Git blobs, modes, and raw working bytes. Original-source comparisons identify only the documented native/docs/attribute edits. | Final fidelity receipt |
| Preserve original repositories and unrelated work | Original compiler, Cargo, and backtrace HEADs match import provenance and tracked statuses remain clean. Concurrent main-tree edits were not reverted or included in the evidence commit. | Final fidelity receipt and final exact-path commit audit |
| Keep compiler version/cache identity tied to owned source, not outer Git or generated files | All 289 bootstrap tests passed, including nine identity regressions. Fresh HEAD archive and working source matched 5,591 files/symlinks plus 1,075 directories and produced `910e39a6c3ef8b6fe54663a4f513213687b9bf4b`, matching the rebuilt compiler's public `-vV` output. No included untracked or empty-directory inputs. | Final native receipt |
| Keep CI LLVM provenance separate and support default optimized std without a full LLVM checkout | Six compiler-rt selector tests are included in the passing bootstrap suite. Actual compiler/optimized std build and runtime tests passed using CI LLVM compiler-rt. Optional LLVM source path has zero descendants. Compiler reports LLVM 23.1.3. | Final native receipt |
| Keep Cargo compatibility fields and make dirty/archive source changes observable | Frozen archive Cargo binary built and reports upstream base `f3865b2a4d1acc5276f6b3c67d0e057f4dab3928` and owned suffix `3f0793ee26b30ba57130f5662c6936756e27c95d`. Hosted identity tests passed 4/4. A concurrent dirty-source build reported a different suffix and was explicitly excluded as frozen-commit proof. | Final Cargo receipt |
| Preserve Cargo's public version behavior | Frozen archive local `version::` acceptance passed all 31 tests, zero failures, run `20261014-074731859-178866`. Hosted no-Git archive mirroring omitted fixture directories named `target`, so its version-suite compile failed before tests. Local acceptance used the original fixtures unchanged. | Final Cargo receipt, including mirror failure and successful local rerun |
| Migrate installed CLI default without breaking explicit override | `justrust toolchain jr-install --help` displays `~/justrust/toolchain/rustc/build/host/stage2`. Root parser tests exercise both the migrated default and explicit sysroot. No compiler default activation was performed. | Final root receipt |
| Migrate benchmark paths while honoring caller overrides | Fresh source/materializer suite passed 10/10, including execution of `bench/factors.py` with defaults and explicit `JR_RUSTC_DIR`/mode overrides. | Final root receipt |
| Preserve client build, test, and diagnostic/lint behavior | Clean archived client passed formatting, hosted check, 439 tests with 3 ignored and zero failures, and strict all-target clippy. Root crates actually recompiled in the archived workspace. | Final root receipt |
| Make optional source dependencies explicit and safely materializable | Ten source/materializer tests include exact revision import, license preservation, policy stripping, overwrite rejection, traversal handling, and shared-source layout. Bootstrap Python suite passed 23/23. | Final root and native receipts |
| Deliver the integrated result and durable evidence | Source commits `8f0a1aeef` and `6d898a9ea` were pushed. This verification adds evidence only, with exact-path staging/commit and remote HEAD verification recorded at completion. | Git delivery audit and exported run receipts |

## Concrete improvement, and what is not claimed

Before the migration, compiler/Cargo ownership depended on independently edited
fork repositories and pinned gitlinks. The final committed tree contains the
actual source and shared Cargo path. Builds and runtime tests now succeed using
that layout, and a clean source archive reproduces both native source identities.
The generated snapshot journal that previously contaminated compiler identity
is now excluded, with regression coverage and a real rebuilt compiler agreeing
with the clean archive. These are operational ownership/reproducibility results,
not just a documentation change or a claim that process boundaries are faster.

No compile-time speedup, stock-versus-owned advantage, stage-2 release readiness,
full Rust language-suite pass, full Cargo-suite pass, cross-platform acceptance,
or new compiler/Cargo default promotion is claimed. Stage-0, CI LLVM, registry
crates, and optional source-build prerequisites still require external artifacts.

## Scope and honest verifier failures

- The initial root chain used the wrong top-level `jr-install` syntax after its
  build/test/lint gates passed. Correct nested CLI help succeeded separately.
- Git-index-specific structural tests cannot run unchanged inside a no-Git
  archive. They passed on the actual Git tree; frozen fidelity uses Git objects,
  and archived client build/test/lint passed independently.
- Native acceptance commands all passed. Its wrapper initially rejected an
  empty optional LLVM directory. A corrected no-materialized-content and frozen
  native-source postcheck passed without changing compiler source.
- Frozen hosted Cargo identity tests passed. Frozen hosted version compilation
  exposed a no-Git archive mirror limitation: source fixture directories named
  `target` were excluded. Local frozen acceptance is used for this unsupported
  mirror case, with the hosted failure retained rather than relabeled as a pass.
- First archived client upload took 137.4s, versus 4.1s recorded check work;
  upload/fixture mirroring remains a concrete future integration opportunity,
  not a migration speed improvement. The full original language/Cargo suites
  remain outside this verification scope.

## Evidence

- `data/toolchain-vendoring/2026-10-14-final-fidelity.txt`
- `data/toolchain-vendoring/2026-10-14-final-native.txt`
- `data/toolchain-vendoring/2026-10-14-final-root.txt`
- `data/toolchain-vendoring/2026-10-14-final-cargo.txt`
- Final exported run receipts under `data/runs/xps14-toolchain-verification/`.

The detailed migration provenance and commands remain in
`docs/toolchain-vendoring.md`.
