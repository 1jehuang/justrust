# Source-backed primitive constants

Status: implemented and accepted for the i32::MIN-only slice below.

## Recorded acceptance

On pinned nightly-2026-10-08, the genuine source declaration and alias-qualified
use pass stock-differential CHECK/BUILD/run at workers 1 and 8. User primitive
impls reject with E0390 and an unused invalid body rejects with E0308. Two
transport controls, the canonical primitive issuer control and the real CHECK
graph control pass. Mandatory graph integrity passes 14/14 without filtered
tests or stack, budget or ratchet changes. CHECK demands the ordinary associated
constant header and no BUILD, MONO_PLAN or OBJECT operation.

Validation used explicit LOCAL placement during the verified hosted disk outage.
The native consumer was tested on tree 3d2631f69b073d0c20e247d0b3d5cfcc3beade00;
its sole later production change narrows the primitive receipt field visibility.
The final authority and graph tree is 98f929659bc1c3ab6bb78cb41145d6f39a9e317c,
on parent 4b7ed44fc1a8e4876f03c3d7bf15ffd40fab60e0. Six authentic exported
receipts are retained in data/runs/xps14/2026-10-19-primitive-min.jsonl. Existing
unrelated CLI, resolver and checked-function dead-code warnings remain; the new
private-interface warning is fixed without suppression. No broad full-suite or
corpus acceptance is claimed.

The source loader seals configured text/origins and semantic transport metadata
(children and routes, module/ancestry, attributes and tests, no-std policy).
The final assembler separately seals the complete parser-visible stream, original
file identities and issuer attachments. Import replays that transport before
minting source-snapshot-qualified file witnesses. Public transport mutation or
copied text cannot issue new primitive authority. Receipt copies are byte/work
admitted and retain only immutable accounting receipts, never a budget ledger.
Original error diagnostic text is not permission and is not certified by this
transport: a unit with a terminal error cannot complete source assembly.

## First supported slice

Only the canonical primitive `i32::MIN` is added. Its declared type is i32 and
its value is exactly -2147483648. The initializer is ordinary typed source,
`-2147483647i32 - 1i32`, evaluated by existing associated-constant CTFE.
Type aliases resolve to the same primitive owner and the same declaring member.
No trait-bound fallback, spelling-to-value table, magic literal replacement,
or invented caller predicate is permitted. Other primitive members remain
outside this feature until separately declared and validated.

## Issuance and admission

Primitive inherent implementations are reserved. User `impl i32` is rejected
with E0390 even when unused. A module spelling, virtual path, copied embedded
text, or existing `prelude_module` ancestry never grants this permission.

The embedded source loader issues a private immutable capability only when it
actually reads its embedded source table. Descendant loading must propagate
that issuer, not reconstruct it from `Path::starts_with`. Source assembly binds
it to the exact original file and prepared assembly identity. Parsing binds an
original, non-generated impl occurrence to the authenticated file and owned
source snapshot. Registration binds that occurrence to its canonical ImplDeclId
and resolved primitive owner. Each consumer checks this full correspondence.
User/raw parsing defaults to no capability. Public raw constructors, counters,
global registries, lazy caches and mutable publication slots are forbidden.

A transplanted source token, file seal, assembly attachment or impl occurrence
cannot authorize another file, snapshot, declaration or primitive owner. The
new capability payload contains immutable identities and original syntax only,
never a compiler world, facts, CFG, budget or schema context. Existing unrelated
prelude/alignment authorization is not retrofitted by this slice.

## Source and graph shape

No extra program traversal or parallel primitive inventory is introduced: the
existing parser impl arm attaches its own finite original-source receipt, the
existing resolver impl arm replays it, and the existing canonical registration
self-type arm binds the resolved declaration and primitive owner before admission.

A new embedded `libraries/prelude/primitive_constants.rs` contains the source
inherent declaration, loaded through narrow entries in the existing library root
and embedded source table. It is indexed through ordinary associated-constant
registration. Existing operations 134 (header), 135 (original use selection),
and 136 (closed typed value) carry the declaration, selection and value.
There is no new whole-program operation or side inventory. `uses.rs` selection
semantics remain unchanged. CHECK validates unused bodies and declarations and
does not request code generation.

## Acceptance gates

- Authentic source `i32::MIN` and an alias-qualified use: stock-differential
  CHECK/BUILD/run at workers 1 and 8 with exact signed-32 output.
- Ordinary required-argument and unused-invalid-body rejection remains active.
- User primitive impls, copied source under spoofed paths/modules, missing issuer,
  foreign snapshot/assembly/file/impl occurrence and changed primitive owner reject.
- Associated-constant selection/value facts reject foreign member/owner and
  altered signed width/value authority rather than trusting a member spelling.
- Graph tests show existing per-member fact edges and CHECK without codegen.
- Mandatory graph integrity passes without stack, budget or ratchet changes.

No corpus-11 acceptance, full primitive-constant coverage, performance improvement
or full Rust compatibility follows from this first slice.
