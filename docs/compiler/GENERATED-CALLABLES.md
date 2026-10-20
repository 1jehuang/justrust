# Generated callables and genuine invocation

Status: implementation in progress. This is a compiler prerequisite for the source Iterator library, not a claim of complete Iterator or Rust support.

## Contract

- A postfix invocation retains its actual callee expression and actual argument expressions. The callee is evaluated once, before arguments, in source order.
- The parser supplies no callable declaration identity. Resolution routes the invocation through the canonical callable trait member and its ordinary provider, mode and carrier proofs.
- Arguments use the canonical tuple convention, including the existing unit shape for zero arguments. A leaked unresolved invocation is rejected explicitly by checking.
- A completed blocklike statement cannot consume the next parenthesized expression as its callee.
- Function pointer types remain explicitly unsupported until their type, ABI and invocation implementation is available.
- Expanded lifetime and type parameter occurrences retain their original token spans, context and invocation admission. Textual binder lookup under an admitted single semi-transparent mark does not manufacture a root occurrence or grant foreign expansion authority. Expanded labels remain explicitly unsupported.
- Every function emitted by an item expansion has a distinct source scope and product seal. Generic and associated functions are not translated into unrelated free functions.
- A generated associated product additionally carries an immutable parser-issued source certificate for its exact container, ordered type arguments, binders, bounds, associated assignments and declarations, complete callable inventory, receiver mode and signature.
- Source admission rejects omitted, extra, duplicated, transplanted, foreign-module and foreign-owner products before planning grants callable authority. Canonical resolution can publish a resolved certificate only after validating the original parser certificate.
- Header certificates contain no bodies. Every original body still travels through ordinary graph publication and checking, including unused invalid methods.

## Representation

`Expr::Invoke` is a parser/resolver marker. Ordinary canonical `Expr::MethodCall` represents the resolved operation. There is no new whole-program compiler node and no alternate evaluation path.

`generated_callables` inventories complete emitted products. `generated_owners` retains immutable source header certificates inside the source program fact. Selected callable replay reads its exact indexed owner rather than joining unrelated bodies.

## Acceptance

`function_invocation` covers local, field and computed callee expressions, ordered effects and invalid unused calls. The full positive corpus matches stock at workers 1 and 8 in the composed snapshot with canonical inline closure preparation. The publication authority unit demands real graph-produced signatures and resolution facts, then rejects forged receiver and tuple operands, arity changes, foreign publications and foreign source occurrences.

`expanded_parameters` covers stock-matching definition/invocation binders and unrelated expansions. The scope authority unit exercises rejection of a valid same-session token carrying an unrelated expansion mark.

`generated_callables` covers generic functions, associated functions and provided methods plus invalid unused bodies. `generated_callables_authority` mutates real parsed declarations at the public planning boundary, including insertion of an ordinary donor method into a generated container. Associated scalar result hints reuse ordinary method-argument inference. Explicit UFCS method arguments and inferred required-method arguments retain separate ordered owner and method binder domains before canonical selection. The parser retains method actuals independently of trait actuals, and bounded name, depth, clone, recipe and resolution walkers visit them. `trait_method_arguments` matches stock at workers 1 and 8 for explicit and inferred calls, zero-argument expected-result inference, and rejects wrong owner, ordering, arity and bounds even in unused bodies. Its publication unit rejects omitted, reordered and ill-typed method actuals using real graph-produced signatures and canonical source facts.

The unchanged full Iterator library and its consumer tests remain separate end-to-end acceptance. Passing helper tests or source algorithm wrappers is not sufficient to claim native Iterator activation.
