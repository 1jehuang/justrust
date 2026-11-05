"""Pinned grouped-expression proposals, UNREVIEWED and without executable credit."""
SELECTED = ('src/expressions/grouped-expr.md',)
SOURCE_COMMIT = '86635e30bf861a038dc197d7e16fd09e7e514e7a'
PAGE_SHA256 = {'src/expressions/grouped-expr.md': '5f068eea2424954be26b8a0c8e74bea57105a07a8c38820f06f3224cfefca96d'}
ALIASES = {'group': 'reference-expressions-grouped-expr-grouping'}
CLASS_NAMES = {'N': 'normative-semantic', 'G': 'grammar', 'I': 'index', 'X': 'cross-reference', 'E': 'edition-context'}
END_TO_END = (
    'end-to-end: Missing unchanged-source pinned-stock classification and native CHECK/BUILD '
    'at workers1/8 in edition2024, with successful defined runs comparing values, effects and '
    'exit status. Missing unused invalid controls, stage-specific rejection and forged-fact '
    'controls. CHECK must not demand codegen or publish an image; rejected input must not '
    'release an executable. Distinguish invalid Rust, unsupported valid source, resource '
    'limits and infrastructure failures. No semantic review, execution or completion credit.')
LIMITATIONS = (
    'All six labels on the complete pinned grouped-expression page have explicit unreviewed proposals. Structural source and schema validation does not establish semantic interpretation, compiler support or executable coverage.',
    'The existing grouping candidate is partial routing only. Preserve the complete 2965-label universe and all missing positive, invalid, interaction and end-to-end needs.',
    'Grouping contains one expression. Unit and tuple construction, including the trailing comma distinction, are governed by linked tuple rules, not inferred from this page.',
    'Place/value preservation is not an independent authorization to borrow, move or mutate. Ownership, mutability, temporary lifetimes, evaluation order and destruction require their linked contracts.',
    'The callable-field example distinguishes grouped field calls from same-name method calls. It does not define general method lookup, closure trait eligibility or unsafe call admission.',
    'Edition2024 is the scope. Pinned source identity does not independently establish release stability. Headings earn no standalone executable credit.',
)
ROWS = r'''
expr.paren|I|Chapter heading routes grouping syntax, evaluation and place classification without a standalone semantic obligation.|group|Missing attribution review of all child grouping rules.|Missing applicability review rather than a heading-derived invalid source.|Missing routing among tuples, precedence and callable fields without heading coverage credit.
expr.paren.syntax|G X|GroupedExpression encloses one Expression between opening and closing parentheses.|group|Missing a grouped literal, grouped block and nested grouping with their values observed.|Missing unmatched delimiters and non-expression operands, separating syntax rejection from semantic rejection.|Missing unit and one-element tuple syntax as separately governed alternatives, including trailing comma and nested delimiters.
expr.paren.intro|N G X|A parenthesized expression wraps a single enclosed operand and evaluates to that expression.|group|Missing a side-effecting enclosed operand with its result and exactly-once effects observed.|Missing a syntactically valid grouped unresolved name or wrong-typed operand in an unused item under linked resolution and typing rules.|Missing nested grouping and grouped control-flow operands without treating grouping as a tuple constructor or an extra evaluation.
expr.paren.evaluation|N X|Parenthesized expressions evaluate to the value of the enclosed operand.|group|Missing grouped arithmetic and a grouped side-effecting function call with result and effect count observed.|Missing a grouped operand whose type cannot satisfy its context, including unused invalid declarations.|Missing short-circuit and operand-order contexts under linked operator rules, and owned values whose grouping must not introduce an extra clone or move permission.
expr.paren.place-or-value|N X|Grouping preserves whether its enclosed operand is a place expression or a value expression.|group|Missing assignment and mutable borrowing through a grouped mutable field place, plus a grouped computed value.|Missing mutation through a grouped immutable or shared-reference place, and attempts to use grouping to evade ownership restrictions.|Missing nested projections, disjoint field borrowing and temporary value lifetimes under linked rules without claiming that parentheses extend every temporary lifetime.
expr.paren.override-precedence|N X|Parentheses explicitly modify precedence among subexpressions; grouping a callable field distinguishes its call from a same-name method call.|group|Missing observable 2+3*4 versus (2+3)*4 and distinguishable holder.f() versus (holder.f)() results.|Missing a grouped non-callable field invoked as a function and grouped operands with invalid operator types.|Missing generic callable fields, precedence with overloaded operators and side effects under linked call and operator contracts, without importing method lookup behavior from grouping alone.
'''
