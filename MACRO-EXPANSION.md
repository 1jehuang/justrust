# Native token trees and declarative expansion

Date: 2026-10-17.
Status: proposed next implementation contract, not implemented capability.
Dependency: publish the current identity checkpoint and release its source/frontend
freeze before integration. This document does not expand that checkpoint.

## 1. Outcome and non-negotiable boundaries

The outcome remains full Rust under RUST-COMPATIBILITY.md and IMPLEMENTATION.md.
This is a reusable native expansion foundation and dependency-ordered program,
not another source-selected probe, text shim, or claim that macro_rules, core,
CTFE, proc macros, or full Rust already work. ARCHITECTURE.md governs graph,
resource, provenance and correctness requirements. G1/G5 remain open.

Implement fresh Rust code using the ordinary toolchain as bootstrap tooling.
Do not embed or delegate native compilation internals to rustc, Cargo, LLVM,
the legacy compiler, stock expansion services, or stock library artifacts.
A separate stock process is a verification oracle only.

Preserve SOURCE-PREPARATION.md: original UTF-8 spans, configured source handling,
syntax-versus-unsupported-versus-limit distinctions, inactive syntax checks,
cfg-before-child-I/O, text-input filesystem prohibition, and original metadata
capabilities. In particular, a copied span/attribute or SourceProbeRecord cannot
create VerifiedIdentityContext or validated semantic publication authority.

## 2. Current seams and limits of the delivered implementation

At inspection, source_map.rs retains FileId and SourceSpan {file, lo, hi}.
OriginSegment maps prepared transport to original bytes or anchored synthetic
punctuation. It is not an expansion/hygiene representation. frontend.rs Token
owns text, a string flag, line/column and prepared byte ranges. lex fuses some
punctuation and rejects several valid token classes, including dollar tokens.
parse_prepared checks original mappings before parsing, but the semantic AST
largely stores names as strings without general occurrence provenance.

source.rs applies source_attrs configuration before Loader::expand resolves
external module candidates. The source reader is private. Its observations are
not separate persistent per-file cache facts. compiler/mod.rs SOURCE, PARSE and
RESOLVE publish immutable snapshot slots. engine::Key and ir::DeclId are
snapshot-local, not stable cross-edit or persistent cache identities. Early
engine outputs can survive a producer's later failure, so one early publication
cannot certify a complete expanded unit.

CORE-BRINGUP.md's original-source-span limitations are historical where superseded
by SOURCE-PREPARATION.md. Expansion and general semantic spans remain open.
CORE-IDENTITY.md describes a selected original attributed function plus authored
client, not an independently compiled library crate or a general metadata policy.

## 3. Assignable files and ownership

The initial files are NEW files. No shared-file edits occur before API freeze.

| Lane | Files and responsibility |
| --- | --- |
| Token/provenance owner | NEW src/compiler/token.rs, token_tree.rs, expansion_span.rs, hygiene.rs, expansion_budget.rs and isolated tests |
| Pure macro owner | NEW src/compiler/macro_rules.rs and matcher/transcriber helpers using the frozen foundation |
| Single frontend/source integration owner | frontend.rs, source.rs, source_attrs.rs, source_map.rs, source_cfg.rs, source_probe.rs only as needed, and module registration |
| Graph owner | compiler/mod.rs plus NEW expansion.rs coordinator and fact tables |
| Independent conformance owner | New stock/native fixtures, recording-reader checks and receipts, without semantic implementation authority |

The single integration owner coordinates source changes with the current identity
owner and provenance-to-DeclId mapping with source-replay/type owners. Do not split
concurrent edits inside frontend.rs between matcher and source workers. No backend
change is a prerequisite for token trees. engine.rs changes need an identified
runtime requirement, not incidental module work. Existing source/IR publication
capabilities remain opaque to the expansion owner.

## 4. Data contract

All proposed visibility starts pub(super), not a stable external library API.
Names below are interface targets, not declarations already present in the tree.

### 4.1 Session and immutable storage

TokenSession owns the symbol, span, expansion and hygiene interners and one
ExpansionBudget authority. A TokenSessionId qualifies all arena handles. Either
handles carry that owner or opaque stream/session APIs enforce it, including at
public decoding/test boundaries. Bare u32 IDs must never accidentally resolve in
another session. FileId additionally belongs to its retained SourceMap snapshot.

TokenStream owns immutable Arc-backed chunks, an arena identity, and a checked
root range. Flat node and child-edge tables avoid recursive owned Vec trees.
TreeRange indexes a child-edge table, not an assumed contiguous subtree layout.
Clones share payload, but newly emitted occurrences still count against output
and provenance budgets. Allocation, traversal, validation and destruction must
all be safe on ordinary stacks. No lock or mutable arena guard crosses an await.

For parallel operations, freeze immutable input chunks and return a bounded
operation-local output batch for publication. Do not share a whole-session mutable
borrow among workers. Local IDs are remapped when batches enter a session arena.
A lock-based initial interner is permitted only without hidden semantic state or
schedule-dependent persistent identities. Measure layout/locking before promoting
performance claims.

### 4.2 token.rs and token_tree.rs

Proposed public types:

- SymbolId, Edition, Ident { symbol, raw, context: SyntaxContextId }.
- LiteralKind and Literal { kind, exact_spelling }, retaining prefixes, suffixes
  and lexical spelling. Decoded values are separate downstream products.
- Spacing::Alone | Joint.
- TokenKind::Ident(Ident) | Literal(Literal) | Punct { ch, spacing }.
- Token { kind, span: SpanId }.
- Delimiter::Parenthesis | Brace | Bracket.
- TokenTree::Leaf(Token) | Delimited { kind, open, children, close }
  | Opaque(FragmentId).
- TokenCursor, CursorPosition, FragmentId and immutable FragmentPayload handles.

Lexing must not conflate valid-but-unimplemented expression semantics with invalid
lexical input. Valid raw/byte/character/C literal tokens may be transported when
appropriate to the pinned edition even if active semantic clients reject them.
Do not use inactive-source placeholder normalization in the general token stream.
Retain punctuation adjacency and source byte ranges. Lower documentation comments
to attributed token structures with provenance. Other comments need not survive.

TokenCursor is a borrowed immutable view with peek, bump, checkpoint and restore.
Parser-only glue/split views must retain the underlying leaves and exact subspans.
A checkpoint includes pending split pieces, group position and opaque-fragment
state, not merely an integer token index. CursorMark contains stream/session/view
identity, event position and punctuation offset. Restore/adopt rejects foreign or
out-of-range marks. Cursor input borrows independently owned immutable chunks,
not mutable session vectors. Checkpoint restoration never refunds budget work.
EOF has an explicit valid anchor for the current range or no location, never a
fabricated file span. Logical glued operators retain every constituent span.
If punctuation originates in different captures/files, diagnostics must not invent
one contiguous physical span or shift a synthetic anchor arithmetically.
Printed expansions are debug views only.
They are never relexed as the authoritative parse or hygiene transport.

### 4.3 expansion_span.rs

Keep current SourceSpan unchanged. Add:

- SpanId, ExpansionId, SyntaxId, MacroDefinitionId and CrateIdentity.
- SpanData { origin, context, expansion }.
- origin = Original(SourceSpan) | Synthetic { anchor: SpanId, reason }.
- ExpansionRecord { definition: SpanId, call: SpanId, parent,
  defining_crate, definition_id, edition }.

Each output token occurrence has a span tied to its original definition or
captured argument and its current expansion record. A repeated capture retains
its original location but gets distinct output occurrence provenance. Captured
identifiers preserve their incoming hygiene context. Definition-body identifiers
receive the appropriate expansion mark. Group opens and closes have separate
spans. A construct across origins has a primary label and related origin labels,
not an invented contiguous SourceSpan. Validate span chains and bound rendering.

ParsedUnit retains SyntaxProvenance mapping syntax occurrences to spans and later
validated declarations. ir::DeclId remains the authority for declarations, not a
span hash or syntax index. A diagnostic-only side table is insufficient for
hygiene: semantic identifier occurrences must actually retain their context.

### 4.4 hygiene.rs

SyntaxContextId indexes an interned chain of parent, expansion and transparency.
Resolution accepts hygienic identifiers and an explicit lookup category. Mixed
site lookup cannot be implemented by changing printable identifier strings.
Expose separate context selection for local/label versus item/path lookup.
Represent $crate as a dedicated defining-crate reference. No name such as core
or identity grants crate identity, privilege or semantic implementation.

The initial single-crate case still requires an explicit native crate identity.
Cross-crate imports/exports wait for source-owned crate facts, not string aliases.
Hygiene must compose with existing namespace, privacy and declaration identity
checks rather than bypass them.

## 5. API targets for freeze

These signatures describe ownership and outcomes. Exact Rust lifetimes and
module-qualified names must be agreed by the token and integration owners.
Avoid defining two incompatible Token/Span/Fragment types in separate lanes.

```text
lex_file(file: FileId, sources: &SourceMap, edition: Edition,
         session: &mut TokenSession) -> Result<LexedTokens, FrontendDiagnostic>

group(input: LexedTokens, session: &mut TokenSession)
    -> Result<TokenStream, FrontendDiagnostic>

configure_items(input: ItemEnvelopeStream, cfg: &CfgSet,
                policy: &AttributePolicy, budget: &ExpansionBudget)
    -> Result<ConfiguredItems, FrontendDiagnostic>

module_candidates(context: &ModuleContext, declaration: &ModuleDecl)
    -> Result<ModuleCandidates, FrontendDiagnostic>

parse_fragment(cursor: &TokenCursor, kind: FragmentKind,
               defining_edition: Edition, output: &mut FragmentOutput)
    -> FragmentOutcome

compile(definition: &MacroDefinitionInput, session: &mut TokenSession)
    -> Result<MacroRulesDef, FrontendDiagnostic>

expand(definition: &MacroRulesDef, call: &MacroCall,
       fragments: &dyn FragmentParser, session: &mut TokenSession)
    -> Result<ExpansionOutput, FrontendDiagnostic>

parse_tokens(input: &ExpandedUnit, budget: &ExpansionBudget)
    -> Result<ParsedUnit, FrontendDiagnostic>

drive_unit(cx: Context, unit: UnitId) -> TaskFuture
```

FragmentOutput owns an operation-local output arena and the shared budget handle.
It avoids requesting a mutable borrow of the same session from which cursor
borrows its input. FragmentOutcome is Matched { fragment, consumed_position },
NoMatch, CommittedError(diagnostic), Unsupported(diagnostic), or Limit(diagnostic).
consumed_position is a checked CursorMark, including punctuation offset and view
identity, not a plain integer. FIRST-set probing, parser commitment and follow-set
validation have explicit boundaries: do not convert every legacy ParseError into
NoMatch. Parse errors carry SpanId/origin handles before expanded input is admitted,
never expansion offsets interpreted as PreparedSource byte offsets.
Infrastructure faults and internal invariant failures remain separate errors.
A successful match consumes a specified cursor extent, not a printed text range.
NoMatch is legal only for a genuine parser/matcher non-match. An unsupported
potentially matching fragment cannot be skipped to select another arm.

ConfiguredItems contains active item envelopes, inactive syntax-only validation
requests, attribute records and source provenance. It performs no child I/O.
ModuleContext contains containing crate, lexical module occurrence and directory,
plus expansion provenance for diagnostics. It is not a path guessed from the
macro definition's source location. Module candidate construction is pure;
observed lookup/read remains source-owned and engine-visible.

Keep existing prepare_file, prepare_text, parse_prepared and syntax-only adapters
until replacement consumers pass the delivered regression corpus. An adapter may
supply original token provenance. It must not become a permanent text-expansion
shim, erase hygiene, or quietly run the old prepared-source path for macros.

The cursor-only migration is root-context/no-macro admission until semantic
identifier migration passes. Existing Function names/parameters, path segments,
Expr::Var/Call, imports and resolver String keys cannot consume contextual names
via ident()->String. Such an adapter must reject nonroot-context identifiers,
opaque fragments it cannot preserve and session-mismatched input, rather than
silently downgrade them. SyntaxContextId/SpanId also cannot grant OriginalBodyId,
canonical source replay, checked-body or VerifiedIdentityContext authority.
Root context alone does not admit every token transported by the new lexer.
Keyword tests respect TokenKind, raw-identifier flag and defining edition:
`Ident { raw: true, symbol: "fn" }` must never satisfy a grammar `fn` keyword
check. Retain explicit Unsupported for valid Unicode identifiers or literal
classes not yet admitted by the active parser. Literal spelling cannot masquerade
as a keyword/operator. Rich lexical transport is not richer semantic acceptance.

## 6. Macro invocation, scope and fragment program

### 6.1 Semantic anchor and oracle obligations

The pinned Rust Reference is normative for the compatibility matrix. Briefly:
macro arms are ordered; a matched arm's failed transcription does not retry later
arms. Local ambiguity is an error, not permission for arbitrary lookahead.
Forwarded parsed fragments are opaque, except specified ident/lifetime/tt cases.
Outer matcher delimiters are flexible. Repetition structure and capture counts
must agree. Fragment grammar depends on the definition edition. Unqualified
lookup checks textual scope before path scope; qualified lookup uses path scope.
Textual visibility begins after the definition and follows lexical nesting,
including module source order. macro_rules hygiene is mixed-site: locals/labels
use definition context, other symbols use invocation context. Substitutions retain
incoming context. $crate denotes the defining crate without bypassing visibility.
These rules require focused stock cases, not inferred compatibility from examples.

Reference reviewed for planning: Rust Reference, Macros By Example, at
`https://doc.rust-lang.org/reference/macros-by-example.html`.
Freeze an exact revision before acceptance. Online documentation is not itself a
pinned oracle receipt.

### 6.2 Definition and invocation records

MacroDefinitionInput contains immutable matcher/transcriber trees, defining
edition/crate, definition syntax occurrence, definition scope/frontier and span.
MacroRulesDef stores compiled arms, binder/repetition schema and dependencies.
MacroCall contains token input, invocation span/occurrence, caller scope/frontier,
crate, edition context and expected output fragment position. Definition edition
and caller parse context must not be collapsed into a single global edition.

FragmentKind reserves block, expr, expr_2021, ident, item, lifetime, literal, meta,
pat, pat_param, path, stmt, tt, ty and vis. FragmentPayload stores syntax handle,
kind, provenance and retained tokens as needed for permitted operations. Preserve
opaque grammar identity when forwarding into another invocation. The parser
adapter must not flatten every FragmentPayload back into literal token matching.

Expected output contexts reserve items, statements, expression, pattern, type and
other independently admitted positions. Parse generated output in that context
and require complete consumption. Calls in unsupported positions receive explicit
Unsupported, not accidental parse success. Empty and semicolon-sensitive outputs
need dedicated oracle cases.

### 6.3 Matching and transcription

Compile definitions once into immutable matcher/transcriber instructions with
explicit branch, group, capture, repetition and accept states. Validate binder
names, fragment/follow restrictions and structural repetition obligations. Do not
postpone malformed unused definitions until invocation. Preserve definition-site
and invocation-site diagnostics separately.

CaptureTree indexes captures by repetition path/depth and preserves zero matches.
Do not eagerly duplicate complete captured subtrees. Count every output token
occurrence even when its payload is shared. Validate transcriber cardinality,
repetition nesting, missing binders and nonprogress/epsilon behavior. Charge state
transitions, fragment parse work and capture handling. Resource exhaustion is
never NoMatch and cannot authorize an alternate arm.

Use bounded matcher states with explicit local ambiguity handling. A generic
exponential backtracking search that happens to find a success is not acceptable.
A fragment parser error at a committed point remains a source error. After a
successful matcher, generated parsing failure is final for that invocation.
Unused generated declarations still enter ordinary universal/type/ownership and
name/privacy validation. Expansion cannot directly construct trusted checked IR.

### 6.4 Scope and mixed expansion/name resolution

ScopeId identifies a lexical scope occurrence. ScopeFrontier identifies the
immutable prefix and unresolved discoveries relevant to lookup. Distinguish
Pending { dependencies }, Resolved { definition }, NotFound { completeness }
and Ambiguous { candidates }. Never publish NotFound while a relevant expansion
could still introduce the binding. Nor should unrelated later textual definitions
be made visible by eagerly registering all definitions in a module.

Interleave definition bindings, invocation expansion, modules and item discovery
in source order. Generated definitions have explicit discovery occurrences and
scope transitions. Block-local macro/item scopes are a later explicit slice if
not admitted initially, not silently treated as module scope. Path-based macro
resolution must integrate import/namespace/crate completeness facts when enabled.
A bounded convergence protocol handles genuine recursive lookup/discovery;
unresolved cycles must not hang workers or manufacture a successful empty unit.

## 7. Source order: configure before module I/O

For original AND generated item streams, the same source-owned pipeline applies:

1. Lex and group with lexical/delimiter validation and retained original spans.
2. Identify the next item envelope without interpreting macro argument bodies as
   module declarations. Validate required outer syntax/attribute envelopes.
3. Process effective cfg/cfg_attr and admitted attribute policy in existing
   source order. Preserve inactive-payload and predicate short-circuit behavior.
4. Inactive items receive the required syntax-only validation, but no name/type
   publication, macro resolution/execution, candidate probes or child reads.
5. For an active macro definition, validate/compile and publish its lexical binding.
   Do not discover modules inside its matcher/transcriber trees.
6. For an active invocation, resolve its macro at the correct frontier, expand,
   and reenter this pipeline with the generated item stream. The caller's
   already-effective attributes must not be lost. Attribute attachment and
   expansion interactions outside the frozen supported cases reject explicitly.
7. For an active external module declaration, only now request observed module
   candidates and read the selected file. Configure the child before descending
   into its descendants. Retain both failed and successful candidate observations.
8. Publish ordinary declarations and parse bodies in their admitted context;
   complete all required validation before sealing unit completion.

Strings, comments, arbitrary tt captures and inactive generated output cannot
trigger module reads. Generated false-cfg modules are a mandatory acceptance
case using a recording reader. A missing file's mere absence is not proof of
zero I/O. Text input has no filesystem capability even if its name resembles a
path. Active path overrides remain Unsupported until the source owner implements
and oracle-tests their exact module placement/path semantics.

Do not silently weaken current inactive syntax validation to delimiter balance
alone. Full inactive Rust syntax remains an explicit open requirement. Legal
syntax beyond a staged parser remains Unsupported, not falsely invalid Rust.

## 8. Graph dependencies and result identity

New logical operations: FILE_SNAPSHOT, LEX_TT, CONFIGURE_ITEMS, MACRO_DEF,
MACRO_LOOKUP, EXPAND_CALL, MODULE_LOOKUP, PARSE_UNIT and UNIT_COMPLETE. A batch may
compute several facts without a task per token. The first integration can
aggregate these into existing PARSE/RESOLVE consumers. No project-wide mandatory
expand-all/resolve-all barrier is required. Lexical semantic order is still real.

Every dependent fact is requested/read via Context. The synchronous lexer,
fragment parser and matcher perform no hidden engine reads or filesystem effects.
Suspending lookup releases the worker. Immutable published payloads are stored in
snapshot-owned typed tables, following the engine's existing fact discipline.

CHECK demands UNIT_COMPLETE for its entire required source/expansion closure and
all required unused-declaration validation. BUILD additionally demands ordinary
verified lowering/emission. Early syntax or declaration outputs cannot make a
failed or partially expanded unit successful. Test late failure after early
publication explicitly.

Within one graph, intern complete identity tuples and use checked handles in
Key.params with operation/schema versions. Do not truncate a content digest into
an unchecked key or use allocation order as logical identity. Graph identity
supplies the immutable snapshot context in the current prototype.

Separate logical occurrence identity from result fingerprints. A macro definition
or invocation has a source-owned syntax occurrence anchor. A generated occurrence
is qualified by its parent invocation/expansion path and output occurrence. A
result fingerprint includes exact definition/input tokens, semantic contexts,
expected output position, definition edition, scope/frontier and defining crate,
effective cfg/target inputs, operation versions and actual dependency closure.

FileId, SpanId, ExpansionId, ir::DeclId and byte offsets are not cross-run cache
identities. Stable syntax anchor matching across edits is separate work; ambiguous
matching conservatively invalidates the unit. Equal-looking call text in another
scope is not automatically reusable. Location changes must not retain stale
provenance/diagnostics. Splitting semantic payload from diagnostic provenance is
optional only after correct remapping is demonstrated.

MODULE_LOOKUP observes both flat and nested candidates, including nonexistence
and ambiguity. Appearance of a previously missing alternative invalidates lookup.
Pure macro_rules does not observe environment/files. Future builtins/proc macros
must declare observed external inputs or prevent unsafe reuse. In-snapshot sharing
is not a persistent/distributed cache implementation. Future persistence needs
exact inputs, authorization, integrity and atomic publication under G9.

## 9. Resource contract and proposed caps

Numbers below are initial policy proposals, not measured capacity or speed claims.
Preserve every existing source-owned limit and its current comparison/charging
semantics. Do not silently change a boundary by rewriting a greater-than test to
greater-or-equal. New aggregate caps supplement existing per-scan limits.

| Existing source-owner limit | Preserve |
| --- | --- |
| Original source bytes and prepared transport bytes | 8 MiB each under existing accounting |
| Source files | 1,024 |
| External-file depth | Existing MAX_FILE_DEPTH 64 check semantics |
| Inline/module depth | 128 |
| Original transport origin segments | 1,048,576 |
| Attribute scanner tokens | 1,000,000 per existing scanner contract |
| Attribute scanner delimiter stack | 512, preserving existing delimiter diagnostics and ordering |
| Attribute expansion depth/work | 64 / 16,384 under existing scope |
| Meta parser depth/nodes | 64 / 4,096 under existing parse scope |
| User cfg | Existing 128-entry and 64 KiB accounting/validation |
| Reserved native cfg facts | Cannot be overridden |

| New expansion-session proposal | Initial ceiling and charging unit |
| --- | --- |
| Lexical input tokens across demanded closure | 4,000,000 leaves, including repeated file occurrences |
| Original-source delimiter group depth | 512 with existing scanner boundary/diagnostic ordering preserved |
| Newly generated delimiter group depth | Proposed 256, separate from original-source and module-depth policies |
| Macro invocation count | 65,536, including nested calls |
| Expansion ancestry depth | 128 |
| Matcher transitions plus fragment work | 16,000,000 aggregate charged work units |
| Simultaneous matcher states | 65,536 per invocation, also charged to bytes/work |
| Capture/repetition elements | 1,000,000 aggregate logical elements |
| Tokens emitted by one invocation | 1,000,000 including delimiter leaves |
| Tokens emitted across session | 4,000,000, repeated occurrences count |
| Retained token/fragment/matcher/provenance storage | 256 MiB shared ceiling, actual owned capacity plus charged overhead |
| Expansion records | 65,536, also byte charged |
| Span records | 8,000,000, also byte charged |
| Hygiene contexts | 1,000,000, also byte charged |
| Scope convergence/discovery transitions | 1,000,000 aggregate |
| Diagnostic text | 1 MiB aggregate expansion diagnostics |
| Rendered expansion backtrace | 64 frames per diagnostic, visibly truncate |

Byte caps may reject before count caps. They do not reserve implied capacity in
advance. Do not impose the proposed generated-group ceiling on original source:
module nesting at 300 must still reach the delivered module-depth 128 diagnostic,
not an earlier generic group-depth 256 rejection. The original-source adapter
retains the scanner's delimiter ceiling 512 and existing envelope/module-check
ordering. A true generic delimiter rejection reports that cause, never pretends
to be module nesting. Generated groups use a separately tagged policy with clear
Limit diagnostics; changing that policy is not a source-compatibility relaxation.
Source-owned original-byte retention remains source-accounted. Charge
new copies and cross-layer ownership exactly once, document accounting boundaries,
and never waive existing limits merely because an Arc is shared. Expansion output
is not required to be flattened into the existing 8 MiB prepared text transport.
Original-source and current adapter limits nevertheless remain intact. Opaque
fragment caches, temporary matcher sets and diagnostic labels are not free.

ExpansionBudget is a shared, poison-on-failure session authority with independent
counters, checked addition/multiplication/ID conversion, fallible allocation and
RAII temporary reservations. Work/produced counts are monotonic even when storage
is released. Match attempts and nested invocations never reset aggregate caps.
No partial result is published as complete after a resource rejection. Limit,
Unsupported, invalid source, infrastructure failure and compiler bug remain
separate outcomes. Reuse PlanBudget reservation/accounting primitives through an
adapter or budget-owner extraction, not unrelated generic-planning numeric caps.

These caps are not replacements for Rust's user recursion_limit or other language
attribute behavior. Staged active control attributes not yet supported must
reject explicitly. Additional policy caps are identified as resource policy,
not a claim that stock rejects the same valid input. Lowered-limit tests must
exercise boundaries deterministically without allocating all maximum capacities.

Add cancellation/admission checkpoints at bounded work-loop boundaries. Current
engine demand cancellation detaches demand but does not stop orphan operations.
Cooperative operation cancellation requires an explicit runtime integration gate;
do not claim it follows from the proposed checkpoints alone.

## 10. Dependency-ordered implementation and staged support

| Slice | Required implementation and acceptance | Explicit remaining boundary |
| --- | --- | --- |
| T0 | Freeze shared token/session/span/fragment/budget types and ownership | No macro language support |
| T0a | Sole frontend owner first encapsulates the current lexer output in an immutable cursor with complete split/checkpoint/source parity | No new token classes, hygienic names or macro admission; replace this backing with T1 tokens at integration rather than retaining two production scanners |
| T1 | New native lexer/group/cursor/provenance/hygiene primitives, malformed/deep/cross-session tests | Standalone utilities do not imply frontend integration |
| T2 | Sole owner integrates token parser and configured envelopes, actual hygienic identifiers and original diagnostics | Retain old public adapters and source contract corpus |
| T3 | Pure local macro_rules compiler/matcher/transcriber: ordered arms, tt/ident/literal, separators and nested repetitions | Full fragment grammar and cross-crate lookup remain open |
| T4 | Opaque expr capture over the currently admitted expression grammar, lexical scope driver, item/expression output and nested expansion | Other fragment/output forms remain individually Unsupported |
| T5 | Graph-visible discoveries, generated cfg-before-I/O, closure completion, unused generated-body validation and public CHECK/BUILD/run at workers 1/8 | This accepts a named local macro_rules slice, not all macros |
| T6 | Complete all fragment grammars/follow sets and block-local scope/output positions with edition matrix | Track interactions individually, no category-wide closure by enumeration |
| T7 | Native crate roots, macro imports/export/$crate/privacy and cross-crate provenance | No stock rlibs or string-qualified identity shortcuts |
| T8 | Builtins, complete attribute/cfg placement, derives/proc macros and observed external inputs | Host/target isolation and real native proc-macro execution remain G5 gates |

T3 can be developed against frozen pure APIs while T2 proceeds, without editing
shared files. T4 native acceptance depends on T2 and T3. T5 depends on source and
graph correctness as well as the matcher. Cross-crate API design and oracle
classification can start before T7. T8 is not a prerequisite for every local
macro, but is a real full-Rust/core dependency. CTFE remains a separate G5 program.

Unsupported staged behavior fails closed: recognize known forms and return an
honest Unsupported rather than stripping attributes, treating $crate as text,
flattening opaque fragments, ignoring invalid unused declarations, skipping an
unsupported matching arm, or interpreting block-local scope as module scope.
Malformed syntax remains a source error. A valid staged form must not be called
invalid Rust merely because its implementation is missing. Unknown/malformed
forms require a pinned oracle classification, not an optimistic bucket.

## 11. Stock/native matrix and receipts

Freeze actual rustc -Vv, target, edition, profile, cfg/features and exact fixture
bytes before running cells. Record source revision/hash separately when using
library source. Existing documentation mentions different local/hosted oracle
revisions; none is automatically the next matrix pin. Native workers 1 and 8 must
produce equivalent outcomes. Full edition support is not inferred from one cell.

Each row has: requirement ID, fixture/hash, stock compile/run outcome, supported /
invalid / legal-unsupported / resource-policy classification, native CHECK,
native BUILD/run when meaningful, primary/related provenance, observed I/O,
graph dependencies/completion and retained receipt. A successful CHECK never
demands OBJECT. Match behavior/runtime outcomes, not exact rustc diagnostic prose.

| Family | Required positive, invalid and legal-unsupported cells |
| --- | --- |
| Lexical/tree | All delimiter kinds; mismatch/unterminated groups; raw identifiers; Unicode; raw/byte/char/C literals; suffixes; doc comments; dollar/hash; lifetimes; joint punctuation; parser generic-angle/reference splits |
| Arms/fragments | Empty inputs; first-arm selection; fragment follow restrictions; local ambiguity; committed fragment errors; output failure after matched arm; opaque forwarding and permitted exceptions; complete FragmentKind inventory |
| Editions | Definition versus invocation edition; expr/expr_2021 and pat/pat_param boundary cases, including unsupported native edition rows |
| Repetition | Zero/one/many; nested and separated forms; optional/trailing forms; cardinality mismatch; missing binders; malformed unused definitions; zero-width/nonprogress captures |
| Output positions | Item/expression first; statements/semicolon behavior, types/patterns and other reserved positions as admitted or legal-unsupported |
| Textual scope | Call before/after definition; redefinition/shadowing; nested modules; source order across external module closure; generated definitions; block-local scope boundaries |
| Hygiene | Introduced local/label versus caller capture; definition locals; call-site item lookup; nested expansion; raw identifiers; context-bearing binding/path diagnostics |
| Crate/path scope | Qualified/imported/exported macro; $crate; rename/reexport; privacy; two crates with identical names; explicit unsupported until native crate facts exist |
| Configuration/I/O | Inactive definition/call/unknown macro; generated disabled module; nested cfg_attr; no mod discovery inside tt/string; malformed inactive envelopes; missing/both candidate files; negative candidate becoming present |
| Provenance | Definition and captured argument failures across files; repeated capture occurrences; nested expansion labels; UTF-8/CRLF; synthetic group anchors; changed source location; wrong-session handles |
| Existing semantics | Generated invalid unused function/import; non-Copy moves and rejected escape/aliasing; existing privacy/namespace checks; attempted identity metadata spoof; unsupported const contexts |
| Limits | Deep/wide recursion; explosive repetition; state growth; lowered count/byte/work caps; allocation/overflow injection; ordinary-stack traversal/drop; bounded diagnostic truncation |
| Graph | Shared invocation demand; independent files; ordered frontier; CHECK without codegen; late failure after early publication; source closure complete; worker parity; cancellation honestly classified |

Use a recording filesystem adapter to assert the sequence and absence of probes,
not only missing files or timing. Prove complete mapping of generated diagnostics
and unchanged original source behavior. Cross-edit invalidation claims require an
actual persistent/edit-session implementation and changed-input tests. Until then,
verify dependency identity and fresh-snapshot outcomes without claiming reuse.

The exact attributed identity item can fail stock compilation due to compiler
internal attributes or duplicate diagnostic identity. Keep those classifications
separate from installed-core behavioral references. Do not remove original
attributes to manufacture an unchanged-source oracle success.

Acceptance ladder: isolated primitives -> pure matcher -> public native source
CHECK -> BUILD and executable behavior -> graph/I/O/provenance checks -> full
regression/lint/format gates through the stable justrust client. Retain source and
run receipts. No builds or acceptance runs are evidence supplied by this design.
No speed or full-Rust claim follows from passing synthetic token fixtures.

### 11.1 Exact frontend cursor acceptance cases

Before permitting macro-bearing input, add these focused cursor regressions:

1. Immutable original `>>=`: mark start, consume a type-closing `>` to view
   `>=` at original bytes 1..3, mark middle, consume another `>` to view `=`
   at 2..3, then restore middle/start and reproduce both views. Base leaves
   remain unchanged. For `&&`, one borrow-prefix consumption views `&` at 1..2
   and eventual EOF at 2..2.
2. Ordinary expression `x>>=1`, `x>>y`, `x>=y`, `x&&y` never type-splits.
   `> > =` and `>/*gap*/>=` do not glue across non-Joint boundaries.
3. Checkpoints cover lookahead 0..2, multi-event advance, group entry/exit,
   split suffixes and per-range EOF. Reject foreign stream/view/session marks.
4. Unicode comments, CRLF, decoded versus raw literal spelling, root/child EOF,
   synthetic braces and literal text containing `>> &&` retain correct origins.
   Preserve the existing numeric after-dot exception: `t.0.1` remains tuple
   projections, distinct from `0.1`, `1.`, `1e-3`, `2.5E+7f32` and `1..2`.
   Retain exact numeric suffixes and separate negative-sign tokens. Punctuation
   splitting alone is not evidence of lexical parity. Add root-context raw-keyword
   identifiers, Unicode identifiers and keyword/operator-looking literal contents:
   verify kind/raw/edition-aware classification and explicit Unsupported where
   active parser support is still absent.
5. Existing array-count fallback rewinds its cursor: `[i32;1+2]` remains legal
   unsupported while `[i32;1+]` remains syntax-invalid. No rewind refunds fuel.
6. Logical multi-leaf operators from different files/expansions retain constituent
   origins without fabricated physical union spans or shifted synthetic anchors.
7. Retain entire frontend/source regressions, including module depth 128/129/300,
   const qualifier publication, source replay, cfg-before-child-I/O and file/text
   parity. Run through stable justrust only after implementation freeze release.
8. Reject illicit opaque-fragment inspection, foreign handles and nonroot-context
   adapter input before the semantic hygiene gate. Cursor tests alone are not
   macro_rules or hygiene acceptance.

## 12. Blockers, choices and freeze checklist

Real blockers for first native macro acceptance: context-carrying tokens AND
semantic identifiers; fragment opacity/parser seam; lexical scope/frontier
semantics; generated configuration before I/O; complete validation closure;
bounded deterministic matching/output/provenance; current source regression
preservation. Cross-crate roots block export/$crate/library acceptance but not
standalone token primitives. Full fragments, proc macros, builtins, CTFE and core
semantic/runtime requirements remain genuine full-Rust gates.

Optional policy/layout choices: exact arena chunking, Unicode utility dependency,
lock strategy, default cap tuning, cross-edit anchor matching algorithm,
distributed persistence, batching and a debug expansion printer. These must not
be used to delay the essential APIs or justify embedding another compiler.

Before integration release, coordinator and owners agree:

- One definition of tokens, session identity, spans, contexts and fragments.
- No aliasing conflict between immutable parser input and mutable output arenas.
- Resource accounting ownership and explicit cap/version policy.
- Exact initial syntax/fragment/output/attribute boundaries and oracle cases.
- One frontend/source integration owner and a separate graph coordinator owner.
- Current identity/source publication completed independently and freeze released.
- Native expansion is the supported path, not a new selected-source shortcut.
- Full macro_rules and full Rust remain open until their complete acceptance gates.
