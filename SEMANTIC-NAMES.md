# Owned semantic-name checkpoint

This historical checkpoint implements reusable checked name/path storage and the first
actual AST integration: loop labels. It does not complete the name-bearing AST,
root resolver migration, mixed-site hygiene, Unicode semantic identifiers, or
native macro execution. The later actual root-context AST/resolver/checker/replay
migration is accepted under `MODULE-IDENTITY.md`. A later bounded function-local
source macro client and checked mixed-site lexical bindings are accepted under
`SOURCE-MACROS.md`. General hygiene, Unicode semantic identifiers and broader
source macro execution remain explicit work in `IMPLEMENTATION.md`.

## One owner, real occurrences

`NameStoreBuilder` consumes the same `TokenSession` that issued the admitted
native tokens. The parser borrows an independently owned immutable logical-token
buffer plus this mutable builder. It does not borrow mutable session storage
through the token buffer. Freezing the builder produces one immutable
`Arc<NameStore>` retained by the parsed/resolved program and planning snapshot.

`AstIdent` is a compact private owner/index occurrence handle, not a printable
name or a globally meaningful integer. Its checked record retains the normalized
atom, exact spelling, syntax context, original occurrence span, raw flag, and
token-origin edition. This root-only stage admits ASCII, non-raw,
non-expanded identifiers. Valid lexical forms outside that semantic boundary
remain Unsupported rather than being reconstructed as root identifiers.

`NameKey` is explicitly selected for a lookup class/namespace. Equality uses the
owner-qualified atom and projected context, not occurrence coordinates, raw
spelling syntax, or display text. Different original occurrences can name the
same binding. Foreign stores cannot contribute interchangeable identifiers even
when their text and allocation order happen to match.

The public fixture builder creates a real bounded in-memory source snapshot and
issues identifiers through its own checked store. It is not a source-parser
fallback. No sentinel owner, detached `From<String>`, or same-spelling authority
repair is provided.

## Structured paths

`AstPath` retains a typed relative/crate/self/super/Self anchor and immutable
checked identifier segments. Shared backing retains its actual-capacity lease.
Empty roots retain owner identity even without a first segment. Join, parent,
slice, and append operate structurally, not by printing and splitting resolver
keys. The explicit fixture text factory is separate from source parsing.

`PathKey` shares prevalidated segment keys without a new allocation per lookup.
Its pure equality/hash operations include owner, anchor, final namespace, and
semantic segment keys. Resolver integrations must charge the corresponding
segment traversal before map operations. There is no new global 128-segment
restriction: parser source limits and canonicalized module paths are different
boundaries.

Diagnostic `PathDisplay` owns both its string and actual-capacity storage lease
until drop. It is not an authoritative lookup key or an unaccounted escaping
string. Diagnostics retain their separate cumulative byte/work limits.

## Actual frontend integration delivered in this checkpoint

Loop-label declarations and uses now retain `AstIdent`. The checker uses
`LookupClass::Label` keys, so different spans alone do not distinguish labels.
Program publication validates label handles, including unused bodies, before
registry use. The names-enabled universal traversal uses a budgeted iterative
temporary stack, charges work before bulk child processing, and preserves typed
transport failures rather than formatting a budget cause as source syntax.

Plan-backed callable typing requires the supplied program and immutable planning
snapshot to retain the same name-store `Arc`, before either recipe or source
body dispatch. A clone sharing that store is valid. An independently parsed
same-spelling program or a replaced store is not repaired by name equality.
Original-body and source-replay authority remain separate from token/name IDs.

Other name-bearing AST fields and resolver/checker maps still use the prior
representation at this checkpoint. They are not silently declared hygienic.
The next vertical step must migrate actual paths, local/declaration/type/region/
member identities and replay bindings together, without a String fallback or
mixed authoritative key system. Contextual admission requires subsequent
namespace-specific definition/invocation scope anchors and real expansion-driver
acceptance.

## Evidence

Hosted external name/path tests passed 8/8 in
`20261017-071349820-722641`; focused semantic tests passed 20/20 in
`20261017-071356932-722904`. The initial frontend checkpoint passed 190/190 in
`20261017-071407341-723675`, including the unchanged 65,537-use and 4,096-parameter
resource fixtures. After the validator and retained-plan repairs and a separately
recorded graph publication-event fix, the complete coordinated suite passed
1,951 tests across 98 binaries without retry output in
`20261017-072549738-775993`. Warning-free all-target check
`20261017-072603261-776840`, strict all-target lint
`20261017-072603747-776966`, formatting, and whitespace checks passed.
Exact observed failed and successful checkpoint records are retained in
`data/runs/xps14/2026-10-17-semantic-macro.jsonl`.
