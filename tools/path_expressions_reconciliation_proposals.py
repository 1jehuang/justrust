"""Pinned path-expression proposals, UNREVIEWED and source-only.

Scratch authoring only. No semantic review, executable evidence or completion credit.
"""
SELECTED = ('src/expressions/path-expr.md',)
SOURCE_COMMIT = '86635e30bf861a038dc197d7e16fd09e7e514e7a'
PAGE_SHA256 = {'src/expressions/path-expr.md': 'e5a1ee1c83c4ee6bc7b39d7442e2107ea7147941eba0d8871fb8960940a9caf3'}
ALIASES = {
    'path': 'reference-expressions-path-expr-value-paths',
    'forms': 'reference-paths-path-forms',
    'resolve': 'reference-names-name-resolution-resolution-context',
    'privacy': 'reference-visibility-and-privacy-privacy-reachability',
    'borrow': 'reference-expressions-operator-expr-borrow-deref',
    'static': 'reference-items-static-items-static-identity',
    'associated': 'reference-items-associated-items-associated-constants',
    'block': 'reference-expressions-block-expr-block-values',
}
CLASS_NAMES = {'N': 'normative-semantic', 'G': 'grammar', 'I': 'index', 'X': 'cross-reference', 'E': 'edition-context'}
END_TO_END = (
    'end-to-end: Missing unchanged-source pinned-stock stable1.98.1 eligibility classification '
    'and native CHECK/BUILD at workers1/8 in edition2024, with defined runs comparing values, '
    'effects and exit status. Source identity is not stable eligibility or compiler support. '
    'Missing unused invalid controls, stage-specific rejection and forged-fact controls. '
    'CHECK must not demand codegen or publish an image; rejected input must not release an executable. '
    'Distinguish invalid Rust, unsupported valid source, resource limits and infrastructure failures. '
    'No semantic review, execution or completion credit.')
LIMITATIONS = (
    'All six labels on the complete pinned path-expression page have explicit UNREVIEWED proposals. Static source/schema checks establish neither interpretation correctness nor implementation or executable coverage.',
    'Existing inventory aliases are partial routing only, not semantic equivalence or evidence. Preserve the complete 2965-label universe and every missing positive, invalid, interaction and end-to-end need.',
    'Grammar alternatives, name resolution, privacy and qualified-path rules remain separate linked contracts. Resolving a path does not independently authorize access or establish stable eligibility.',
    'Paths resolving to locals or statics are places; other resolved paths are values. Category is distinct from operand context, ownership, mutability and borrowing legality.',
    'Mutable-static use requires an unsafe block. An unsafe block is not blanket safety authorization and does not waive aliasing, provenance, synchronization or other applicable rules. Undefined behavior is not a valid output oracle or necessarily a required static rejection.',
    'Associated-constant evaluation follows the linked const-block contract. This is not a requirement to evaluate every unused associated constant. Resolve evaluation obligations and deferred generic instantiation under the linked contracts before selecting invalid controls.',
    'Edition2024 and Linux x86-64 are the intended scope. Pinned source identity and example syntax do not establish stable1.98.1 eligibility. Headings and examples earn no standalone executable credit.',
)
ROWS = r'''
expr.path|I|Path-expression heading routes its child contracts without standalone executable credit.|path|Missing attribution review of all path-expression child labels.|Missing applicability review rather than heading-derived invalid programs.|Missing separate grammar, resolution, privacy, qualified-path and stable-eligibility review without heading credit.
expr.path.syntax|G X|PathExpression permits PathInExpression or QualifiedPathInExpression under their separate linked grammar contracts.|path forms|Missing ordinary and qualified path forms including eligible generic arguments and associated-item paths.|Missing malformed ordinary and qualified path syntax, classified separately from unresolved or inaccessible names in unused bodies.|Missing linked path grammar, qualification, resolution and privacy checks; grammar acceptance does not establish stable eligibility or authorize access.
expr.path.intro|N X|A path in expression context denotes a local variable or an item, subject to linked resolution and privacy contracts.|path resolve privacy|Missing local-variable and item expression paths, constructors and associated-function values with defined observations.|Missing unresolved names, wrong-namespace uses and inaccessible items in unused bodies under their linked rules.|Missing shadowing, module paths, generic arguments and qualified associated paths without conflating name resolution, privacy, grammar or stable eligibility.
expr.path.place|N X|Paths resolving to local or static variables are place expressions; other resolved paths are value expressions.|path borrow static associated|Missing local and static place reads and borrows contrasted with constant, constructor and function-item value paths.|Missing illegal moves, borrows or assignments under linked category, ownership and mutability rules, including unused invalid bodies.|Missing qualified associated paths, place versus operand context, Copy versus move and static access restrictions without classifying every path as a place.
expr.path.safety|N X|Using a mutable static requires an unsafe block; this requirement is not blanket safety authorization.|static path borrow|Missing defined mutable-static use inside unsafe blocks and ordinary local and immutable-static controls.|Missing mutable-static use outside required unsafe blocks in unused bodies, with other independently invalid operations classified by their own rules.|Missing unsafe-block scope, borrowing, aliasing and synchronization obligations without treating unsafe as sufficient permission or using undefined behavior as a differential oracle.
expr.path.const|N X|Associated-constant evaluation is handled like const blocks under the linked evaluation contract, not mandatory evaluation of every unused constant.|associated block path|Missing associated-constant values and const-block comparison cases where the linked contract requires evaluation, including eligible generic instantiations.|Missing failures where evaluation is required and independent declaration/type errors even in unused items; do not demand evaluation merely because an unused constant exists.|Missing linked const-block evaluation timing, reached versus unreached uses and generic instantiation, separate from associated-item resolution, privacy and stable eligibility.
'''
