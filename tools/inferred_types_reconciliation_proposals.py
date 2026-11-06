"""Bounded pinned inferred-type proposals, UNREVIEWED and source-only.

No semantic review, executable evidence or completion credit.
"""
SELECTED = ('src/types/inferred.md',)
SOURCE_COMMIT = '86635e30bf861a038dc197d7e16fd09e7e514e7a'
PAGE_SHA256 = {'src/types/inferred.md': 'cb764e3df2fd66b1d1968a6fd700259763a1b9b4ea4fa1e047b57ee692cadb58'}
ALIASES = {'inferred': 'reference-types-inferred-placeholder-types'}
CLASS_NAMES = {'N': 'normative-semantic', 'G': 'grammar', 'I': 'index', 'X': 'cross-reference', 'E': 'edition-context'}
END_TO_END = (
    'end-to-end: Missing unchanged-source pinned-stock stable1.98.1 eligibility classification '
    'and native CHECK/BUILD at workers1/8 in edition2024 on Linux x86-64 with defined runs comparing '
    'values, effects and exit status. Source identity is not stable eligibility or compiler support. '
    'Missing unused invalid controls, stage-specific rejection and actual-changing forged-fact controls. '
    'CHECK must not demand codegen or publish an image; rejected input must not release an executable. '
    'Distinguish invalid Rust, unsupported valid source, resource limits and infrastructure failures. '
    'Context-only rows require source review and child attribution, not standalone runtime credit. '
    'No semantic review, execution or completion credit.')
LIMITATIONS = (
    'All four labels on the complete pinned src/types/inferred.md page (26 lines) receive UNREVIEWED source-only proposals. The bounded chapter explicitly leaves a broader inference discussion absent and is not a complete inference algorithm, unification, numeric defaulting or trait-solving specification.',
    'The existing placeholder-types alias is partial routing only, not semantic equivalence, review or executable evidence. Old source pin identity does not establish stable1.98.1 eligibility.',
    'The underscore grammar is an inferred type in type context, not a wildcard pattern, inferred const argument, expression placeholder or a requirement that every underscore use succeed.',
    'Surrounding information permits inference if possible; ambiguity and incompatible constraints need independent stock classification. The illustrative Vec collect example is not a complete Iterator, collection, range or standard-library contract.',
    'The pinned item-signature restriction requires independent version and position applicability review. Do not promote the old wording into an unqualified current-Rust rule, conflate item signatures with local annotations or alter a stock-valid fixture into invalid input.',
)
ROWS = r'''
type.inferred|I|Inferred type heading indexes grammar, contextual inference and item-signature restrictions, not independent executable behavior.|inferred|Missing review and attribution of each child label.|Missing applicability review rather than a heading-derived compile error.|Missing separation of grammar, contextual information and signature-position eligibility without heading credit.
type.inferred.syntax|G|InferredType grammar consists of an underscore token in a type position.|inferred|Missing independently eligible underscore type positions, including partial local generic annotations.|Missing malformed type syntax and forbidden positions after pinned-stock classification.|Missing separation of type underscores from wildcard patterns, const arguments and expression placeholders; grammar alone is not inference success.
type.inferred.intro|N|An inferred type asks the compiler to infer a type if possible from surrounding available information; the generic-argument example illustrates context, not universal success.|inferred|Missing compatible surrounding constraints resolving partial annotations and independently eligible generic arguments.|Missing ambiguous or conflicting constraints with genuine semantic-stage rejection and unchanged unused invalid controls.|Missing caller, local and generic context interactions without importing a complete inference algorithm, numeric defaulting, trait solver or Vec/Iterator contract.
type.inferred.constraint|N|The pinned chapter states that inferred types cannot be used in item signatures; version-specific applicability must be classified before assigning a rejection expectation.|inferred|Missing independently eligible local annotations contrasted with separately reviewed signature positions.|Missing unchanged signature-position controls classified by pinned stock, preserving unsupported-valid and version-ineligible outcomes rather than manufacturing invalidity.|Missing item versus local and nested generic-position distinctions, stable eligibility and source-certified rejection without treating old prose as an unconditional current-Rust rule.
'''
