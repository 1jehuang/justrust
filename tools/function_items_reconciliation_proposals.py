"""Bounded pinned function-item proposals, UNREVIEWED and source-only.

No semantic review, executable evidence or completion credit.
"""
SELECTED = ('src/types/function-item.md',)
SOURCE_COMMIT = '86635e30bf861a038dc197d7e16fd09e7e514e7a'
PAGE_SHA256 = {'src/types/function-item.md': 'bdcabb9164f4eb341ab0277cd6cf6a6fa95af3ce0e148f3d3991611c39975754'}
ALIASES = {'function-item': 'reference-types-function-item-fn-item-identity'}
CLASS_NAMES = {'N': 'normative-semantic', 'G': 'grammar', 'I': 'index', 'X': 'cross-reference', 'E': 'edition-context'}
END_TO_END = (
    'end-to-end: Missing unchanged-source pinned-stock stable1.98.1 eligibility classification '
    'and native CHECK/BUILD at workers1/8 in edition2024 on Linux x86-64, with defined runs comparing '
    'values, effects and exit status. Source identity is not stable eligibility or compiler support. '
    'Missing unused invalid controls, stage-specific rejection and actual-changing forged-fact controls. '
    'CHECK must not demand codegen or publish an image; rejected input must not release an executable. '
    'Distinguish invalid Rust, unsupported valid source, resource limits and infrastructure failures. '
    'Never execute undefined behavior, invalid function calls or unverified target-feature calls as deterministic tests. '
    'Context-only rows require source review and child attribution, not standalone runtime credit. '
    'No semantic review, execution or completion credit.')
LIMITATIONS = (
    'All six labels on the complete pinned src/types/function-item.md page (54 lines) receive literal UNREVIEWED source-only proposals. This is not a complete generic, lifetime, coercion, callable-trait, ABI or code-generation contract.',
    'The existing fn-item-identity alias is partial routing, not semantic equivalence or exact executable evidence. Old source pin identity does not establish stable1.98.1 eligibility. Linked pages are context only, not imported complete proofs.',
    'Function items and tuple-like struct or enum-variant constructors yield zero-sized function-item values. This does not generalize constructor claims to unit or braced forms or invent alignment, pointer representation or observable optimizer-output requirements.',
    'Identity includes the function name, type arguments and early-bound lifetime arguments, but not late-bound lifetime arguments assigned at call time. Resolved declaration identity is not interchangeable with display names or signature equality.',
    'There is no syntax directly naming a function-item type; diagnostic fn(signature) {fn_name} notation is illustrative, not a parseable type syntax or an exact diagnostic-byte contract. Distinct item or generic-instantiation types can still coerce to compatible function pointers in the stated contexts.',
    'Same-signature function-item coercion applies when a function pointer is expected and when different function-item types meet in if or match arms. This is not unconditional equality of function-item types or a full coercion lattice.',
    'Copy, Clone, Send and Sync are unconditional on this page. Fn, FnMut and FnOnce have all three explicit exclusions: unsafe qualifier, target_feature attribute, or ABI other than Rust. Extern spelling alone is not the stated ABI test; no CPU-feature safety, universal calling convention or call permission follows.',
)
ROWS = r'''
type.fn-item|I|Function-item type heading indexes the six source labels, not standalone runtime behavior.|function-item|Missing child-contract review and attribution.|Missing applicability review, not a heading-derived invalid example.|Missing distinction between item identity, values, coercions and trait conditions without heading credit.
type.fn-item.intro|N X|Referring to a function item or the constructor of a tuple-like struct or enum variant yields a zero-sized value of its function-item type.|function-item|Missing independently eligible unchanged references to function items and both tuple-like constructor kinds with zero-sized item values.|Missing forged nonzero item payloads or inappropriate constructor-kind assumptions without executing invalid calls.|Missing item value versus function pointer or constructed value and tuple-like scope, without imported layout or alignment guarantees.
type.fn-item.unique|N|The item type identifies the function name, type arguments and early-bound lifetime arguments, not late-bound lifetime arguments assigned only when called; no actual function pointer payload or call indirection is needed.|function-item|Missing independently eligible distinct generic instantiations and early versus late lifetime identity witnesses.|Missing forged conflation of different resolved declarations or ordered type and early lifetime arguments, without treating late-bound call arguments as item identity.|Missing canonical resolved identity versus display names or same signature, and the no-indirection-needed explanation versus a mandatory emitted instruction shape.
type.fn-item.name|N|No syntax directly refers to a function-item type; diagnostics display illustrative fn(signature) {fn_name} notation. Different functions or the same item with different generics have distinct item types, and the shown mutable assignment creates E0308.|function-item|Missing source review of diagnostic-only notation and eligible distinct item-type observations without naming the type directly.|Missing unchanged shown generic-instantiation assignment mismatch in used and unused bodies, with independently verified stock stage and diagnostic classification.|Missing distinct item types versus possible function-pointer coercion, not an exact diagnostic-byte promise or a ban on every mixed-item expression.
type.fn-item.coercion|N X|Function items coerce to same-signature function pointers when directly expected and when different same-signature item types meet in arms of the same if or match.|function-item|Missing independently eligible explicit function-pointer expectations and if and match arm joins across distinct generic item types with matching signatures.|Missing incompatible signature or forged coercion facts, without assuming all different function-item combinations must be invalid.|Missing distinct source item identities versus common pointer target and separately reviewed arm and coercion contexts, not imported complete coercion proofs.
type.fn-item.traits|N X|All function items implement Copy, Clone, Send and Sync. Fn, FnMut and FnOnce are implemented unless the function is unsafe, has target_feature, or has an ABI other than Rust.|function-item|Missing independently eligible unconditional four-trait witnesses and callable-trait witnesses without any of the three exclusions.|Missing forged callable-trait permission for each separate exclusion, including unused invalid bodies, without unsafe or unsupported CPU-feature execution.|Missing unconditional versus conditional trait sets, all three exclusions, and ABI identity rather than extern spelling, without importing complete trait, ABI or target-feature safety contracts.
'''
