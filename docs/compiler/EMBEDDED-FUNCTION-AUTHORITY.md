# Embedded function occurrence authority

Status: prerequisite candidate, not yet published. Test compilation, six focused authority filters and the mandatory graph gates have passed across the frozen checkpoints documented below. The full graph attempt passed 13 gates and exposed one forbidden owner mint in a new test fixture; the fixture was repaired using a real graph-issued task owner, then its affected authority control and the failed scanner both passed. This is scoped retained evidence, not a claim that the full suite was rerun on the final tree. This does not activate packed-byte reserve, change any type-query admission, or establish native acceptance.

## Frozen scope

Only nongeneric free functions whose complete original syntax fits a finite recipe can receive a receipt. The grammar covers integer/unit/inferred types, exact ordered named/wildcard parameters, immutable/mutable lets, explicit and implicit expression statements, returns, integer literals with suffixes, variables, ordinary calls with ordered arguments, the existing typed layout expression, binary operations with original sites, both branches of `if`, exact unsafe blocks and syntax-only native syscall nodes with exactly seven ordered arguments. The syscall syntax marker grants no operation privilege or native/allocator authority.

Unsupported syntax receives no privileged receipt. Ordinary source acceptance is unchanged. Synthetic functions start without receipts.

## Issuance and transport

The existing parser function item arm mints only after final visibility, module, and name adjustment. Its token range begins before item attributes/visibility and ends at the consumed closing brace, never the next token. Every token must be prepared original source in the same owned file with ordered nonoverlapping ranges. A loader-issued file capability must authorize the complete range. Original header tokens additionally admit only optional supported visibility, `const`, `unsafe`, and `fn`. Attributes, extern ABI qualifiers, async, and unsupported visibility/qualifiers receive no receipt rather than inferring default semantics from the lossy Function AST.

A flat immutable event recipe binds the full supported header/body, module and lexical scope, original identifier occurrences, current and authored path channels, literal widths, binary operators/sites, block boundaries, and argument/statement order. Count/work and retained bytes are admitted before allocation. The receipt retains only IDs, immutable syntax rows, source/issuer capability, and a pure retained-byte receipt, not Program, NameStore, CFG, World, handles, or a budget ledger.

At the existing per-function resolve entry, the original recipe is replayed. Only ordinary call paths may change, through the existing resolver's canonical Function lookup and original-use correspondence. Expected resolved rows are derived from original rows, never recaptured from the transformed body. Other legitimate rewrites drop privilege rather than broadening equality or rejecting ordinary input.

The existing canonical callable-registration loop binds the receipt to the exact declaration ID, source index, free-function identity and nongeneric binder/environment. Admission borrows current Program/registry/context again and replays source, issuer, complete resolved syntax and canonical identity. No new scan, producer, or whole-program join is introduced.

## Executed checkpoints and authority controls

Production source is frozen at `b26956ff`; subsequent `7bf458f7`, `e8e21830` and `31b0b7e4` changes repair test fixtures only. The current-parent carry preserves the tested code except for the independent parent-added Ord test module declaration in `frontend.rs`.

Authentic slim exported receipts are retained in `data/runs/embedded-function-authority/2026-10.jsonl`. Successful stages:

| Checkpoint | Stage | Run ID suffix (prefix `20261019-`) |
| --- | --- | --- |
| 089b26da | Genuine called helper and resolved mutations | 225416881-2625008 |
| 7bf458f7 | Check all test targets | 231021560-2694426 |
| 7bf458f7 | Lossy qualifier syntax rejection | 231026119-2694666 |
| 7bf458f7 | Actual loader/resolver/registration | 231104088-2696536 |
| 7bf458f7 | Raw/copied/spoofed/foreign issuer | 231105409-2696631 |
| e8e21830 | Header/body/site/canonical identity and lexical scope | 231322453-2706914 |
| e8e21830 | Ordered calls and arguments | 231338580-2707914 |
| e8e21830 | Full graph attempt: 13 passed, owner-root scanner failed | 231338729-2707976 |
| 31b0b7e4 | Repaired raw/token-range authority control | 232202296-2733072 |
| 31b0b7e4 | Repaired owner-root scanner, one passed/13 filtered | 232223695-2734278 |

The graph failure was real and was not waived. The token-range test now obtains its TokenSession owner from an actual Engine/Provider task, uses the same task budget, and publishes only a scalar completion value. No scanner allowance, ceiling or ratchet was changed. Earlier fixture-only failures (boxed prepared-source return type and an assumed distinct shipped lexical scope) are retained in the receipt file as failures, not authority verdicts. The replacement scope negative obtains a genuine scope from separately parsed ordinary macro source without an embedded receipt and transplants only that scope into the genuine function.

Covered controls:

- Real loader/parser/resolver/registration positive using the shipped `string::utf8_width` body.
- Raw copied text, spoofed prelude module, missing/foreign issuer and foreign snapshot rejection.
- Changed header, original body, operator/site, canonical declaration and environment rejection.
- Same-source ordered call/argument syntax negatives, retained as syntax-only controls.
- Genuine unchanged `mem::raw_realloc` loader/parser/resolver/registration receipt assertion, then same authored call redirected to another canonical function, original-use corruption, call-order change, and syscall argument/order/arity rejection. These assertions passed on 089b; later fixture hardening does not alter that production path.
- Unsupported ABI/attribute/qualifier token-header predicate controls, labeled syntax-only. Shipped loader-backed `align_of<T>` supplies a genuine unsupported generic-header no-receipt control. No shipped free function in this source set has an unsupported ABI/attribute qualifier, so no such genuine-loader positive is fabricated.
- Genuine prepared `utf8_width` tokens supply exact consumed-close range inclusion, missing outer close despite a remaining inner close, following-token/extra-close inclusion, cross-file original token insertion and mixed prepared/expanded provenance controls. The private original-range predicate is factored for these controls, without issuing a receipt. It now explicitly balances body braces and rejects tokens after the outer close, strengthening rather than widening issuance.
- Original lexical-scope mutation is rejected independently of the unchanged body and identifier recipe.

Bound receipt type reachability matches its existing compiler-visible containing field. Fields and factories remain private. Inactive admission methods still produce dead-code warnings; warning-free status is not claimed.

Both recipe traversals and the complete correspondence scan, including no-call rows, admit work explicitly. Correspondence retained-byte sizing and scan offsets use checked arithmetic. No generated token provenance is admitted to obtain a positive fixture.

Actual packed-helper call-path integration and operation-specific query-kind/queried-type authorization remain separate gates. No public size, native memory compatibility, String, Vec, allocation, or performance claim follows from this prerequisite.
