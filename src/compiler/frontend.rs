//! Native, deliberately restricted Rust frontend. No external compiler is invoked.
//!
//! Syntax diagnostics have one-based line/column positions. The span-free public
//! AST means semantic diagnostics identify functions and names instead of spans.
//! Explicit semicolons discard expression values, while omitted-semicolon
//! blocklike statements require unit. While loops do not establish return flow.
//! Numeric literals support primitive integers through 128 bits.
//! Character literals are Unicode scalars with equality/order, not integers.
//! Explicit character casts admit char to integer and u8 to char only.
//! Inline modules, named nonempty braced structs, nonempty unit/tuple/braced enums, ordinary visibility, and
//! module-level named/grouped imports have separate type/value namespaces. Named
//! structs and enums are nominal and non-Copy. Checked pattern decisions retain
//! ordered scalar/tag observations and initialize bindings only after all tests
//! succeed. Scoped acceptance and remaining pattern boundaries are documented
//! in PATTERN-DECISIONS.md and PATTERN-USEFULNESS.md.
//! Move checking belongs to CFG analysis, not
//! this semantic checker. Restricted visibility names lexical ancestor modules.
//! External modules, glob imports, and block-local items are not
//! supported. An executable requires a declared root main, not an imported alias.
use super::ir;
use super::traits as tr;
use std::collections::HashMap;

mod expanded_adapter;
mod native_adapter;
use super::module_identity::{ModuleArena, ModuleArenaBuilder, ModuleId};
use super::semantic_name::{
    AstIdent, LookupClass, NameKey, NameStore, NameStoreBuilder, Namespace,
};
pub use super::semantic_path::{AstPath, PathAnchor};
use super::semantic_path::{NameAccess, PathKey};
use native_adapter::{NativeParserInput, NativeTokenBuffer};
mod char_literal;
mod name_scope;
mod name_validation;
mod pattern_usefulness;
mod program_symbols;
mod token_cursor;
use char_literal::parse_char_literal;
mod trait_selection;
pub use program_symbols::ProgramSymbols;
mod persistent_path;
pub use persistent_path::{
    PersistentCrate, PersistentDeclItem, PersistentDeclPath, PersistentModule,
};
use token_cursor::{TokenCursor, TokenView};

use super::source_replay as sr;
use tr::FactsError;

pub(super) enum PublishedCallableBody {
    Source(Arc<sr::CanonicalSourcePublication>),
    Recipe(Arc<ir::CheckedFunction>),
}

#[cfg(test)]
mod canonical_source_replay_tests {
    use super::*;
    fn fixture_program(source: &str) -> Program {
        let mut program = parse(source).unwrap();
        if !program.functions.iter().any(|function| {
            function.name.anchor() == PathAnchor::Crate
                && function.name.len() == 1
                && program.names.spelling(function.name.segments()[0]).unwrap() == "main"
        }) {
            program = parse(&format!("{source}\nfn main(){{}}")).unwrap();
        }
        program
    }
    fn published(
        source: &str,
        index: usize,
    ) -> (
        Program,
        PlanCtx,
        ir::PlanBudget,
        Arc<sr::CanonicalSourcePublication>,
    ) {
        let program = fixture_program(source);
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let declaration = plan.function_decl(index).unwrap();
        let publication = match plan.publish_callable_body(declaration, &budget) {
            Ok(PublishedCallableBody::Source(body)) => body,
            Ok(_) => panic!("expected original source"),
            Err(error) => panic!("{error:?}"),
        };
        (program, plan, budget, publication)
    }
    fn kind_error<T>(result: Result<T, FactsError>) -> tr::FactsErrorKind {
        match result {
            Err(error) => error.kind(),
            Ok(_) => panic!("unexpected replay success"),
        }
    }
    #[test]
    fn shared_origins_and_legacy_adapter_boundary() {
        for (source, index) in [
            (
                "fn id<T>(x:T)->T{x}fn f(r:&i32){let y=id(r);}fn main(){}",
                1,
            ),
            (
                "fn id<T>(x:T)->T{x}fn main(){let v=7;let r:&i32=&v;let y=id(r);}",
                1,
            ),
        ] {
            let (program, plan, budget, publication) = published(source, index);
            ir::validate_published_source(&publication, &budget).unwrap();
            let calls = publication
                .source()
                .exprs
                .iter()
                .enumerate()
                .filter_map(|(i, e)| {
                    if let ir::ExprKind::Call { args, .. } = &e.kind {
                        Some((ir::ExprId(i), args))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            assert_eq!(calls.len(), 1);
            let (call, args) = calls[0];
            assert_eq!(args.len(), 1);
            assert!(matches!(
                publication.source().exprs[args[0].0].kind,
                ir::ExprKind::Local(_)
            ));
            assert_eq!(
                publication.replay().expected(call, 0, &budget).unwrap(),
                Some(ir::CallExpectation::NoFirmReference)
            );
            // Public cloned-Program adapters cannot acquire the private canonical capability.
            assert!(
                type_callable_with_plan(
                    &program.clone(),
                    plan.function_decl(index).unwrap(),
                    &plan,
                    &budget
                )
                .is_err()
            );
        }
        let program=parse("fn id<T>(x:T)->T{x}fn forward<U>(u:U)->U{id(u)}fn main(){let x=id::<i32>(7);let y=id(8);}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        for index in [1, 2] {
            type_callable_with_plan(
                &program.clone(),
                plan.function_decl(index).unwrap(),
                &plan,
                &budget,
            )
            .unwrap();
        }
    }
    #[test]
    fn bounded_contexts_shadowing_dead_calls_and_legacy_constructors() {
        for source in [
            "fn id<T>(x:T)->T{x}fn f(r:&i32){let y:&i32=id(r);}",
            "fn id<T>(x:T)->T{x}fn f<'a>(r:&'a i32)->&'a i32{id(r)}",
            "fn id<T>(x:T)->T{x}fn f(){let v=1;let y=id(&v);}",
            "fn id<T>(x:T)->T{x}fn f(r:&mut i32){let y=id(r);}",
            "fn same<T>(x:T,y:T)->T{x}fn f(r:&i32){let y=same(r,r);}",
            "fn id<T>(x:T)->T{x}fn f(r:&i32){let a=r;let y=id(a);}",
            "fn id<T>(x:T)->T{x}fn f(r:&i32){{let a:&i32=r;let y=id(a);}}",
            "fn id<T>(x:T)->T{x}fn f(r:&i32){let y=if true{id(r)}else{r};}",
        ] {
            let program = fixture_program(source);
            let budget = ir::PlanBudget::default();
            let plan = PlanCtx::from_program(&program, &budget).unwrap();
            assert_eq!(
                kind_error(plan.publish_callable_body(plan.function_decl(1).unwrap(), &budget)),
                tr::FactsErrorKind::Unsupported,
                "{source}"
            );
        }
        let (_, _, budget, shadow) = published(
            "fn id<T>(x:T)->T{x}fn main(){let v=1;let w=2;let r:&i32=&v;let r:&i32=&w;let y=id(r);}",
            1,
        );
        let last = shadow
            .source()
            .locals
            .iter()
            .enumerate()
            .rev()
            .find(|(_, local)| local.name == "r")
            .unwrap()
            .0;
        let arg = shadow
            .source()
            .exprs
            .iter()
            .find_map(|expr| {
                if let ir::ExprKind::Call { args, .. } = &expr.kind {
                    Some(args[0])
                } else {
                    None
                }
            })
            .unwrap();
        assert!(
            matches!(shadow.source().exprs[arg.0].kind,ir::ExprKind::Local(local) if local.0==last)
        );
        ir::validate_published_source(&shadow, &budget).unwrap();
        let (_, _, budget, dead) =
            published("fn id<T>(x:T)->T{x}fn f(r:&i32){return;let y=id(r);}", 1);
        ir::validate_published_source(&dead, &budget).unwrap();
        let (_, _, budget, legacy) = published(
            "enum E{A(i32),B}fn main(){let p=E::A(1);let (a,b)=(1,2);let x=3;{let q=4;}}",
            0,
        );
        ir::validate_published_source(&legacy, &budget).unwrap();
        // Associated omitted method inference is still not admitted by this ordinary-call slice.
        let program = fixture_program(
            "struct S{x:i32}impl S{fn id<T>(x:T)->T{x}}fn f(r:&i32){let y=S::id(r);}",
        );
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        assert!(
            plan.publish_callable_body(plan.function_decl(0).unwrap(), &budget)
                .is_err()
        );
    }
    #[test]
    fn root_borrow_identity_inference_keeps_source_and_loan_authority() {
        for source in [
            "fn renamed<T>(x:T)->T{x}fn main(){let value=42i32;let r=renamed(&value);if *r==42{println!(\"ok\");}}",
            "fn renamed<T>(x:T)->T{x}fn main(){let mut value=1i32;let r=renamed(&mut value);*r=42;if value==42{println!(\"ok\");}}",
            "fn renamed<T>(x:T)->T{x}fn caller(value:i32){let r=renamed(&value);let out=*r;}fn main(){caller(42);}",
            "fn renamed<T>(x:T)->T{x}fn main(){let value=true;let r=renamed(&value);if *r{println!(\"ok\");}}",
            "fn renamed<T>(x:T)->T{x}fn main(){let value:i32=1+2;let r=renamed(&value);let out=*r;}",
        ] {
            for workers in [1, 8] {
                for check in [true, false] {
                    super::super::compile(source.into(), workers, check)
                        .unwrap_or_else(|error| panic!("{source}: {error}"));
                }
            }
        }
        for source in [
            "fn renamed<T>(x:T)->T{x}fn main(){let value=1i32;let r=renamed(&mut value);*r=2;}",
            "fn renamed<T>(x:T)->T{x}fn main(){let mut value=1i32;let first=&mut value;let second=renamed(&mut value);*first=*second;}",
            "fn renamed<T>(x:T)->T{x}fn main(){let mut value=1i32;let r=renamed(&value);value=2;let later=*r;}",
            "fn renamed<T>(x:T)->T{x}fn main(){let mut value=1i32;let r=renamed(&mut value);let moved=r;let invalid=*r;}",
        ] {
            for workers in [1, 8] {
                for check in [true, false] {
                    let failure = super::super::compile_observed(source.into(), workers, check)
                        .err()
                        .unwrap();
                    assert!(
                        failure
                            .analyses
                            .iter()
                            .any(|(_, analysis)| !analysis.passed()),
                        "{source}: {}",
                        failure.error
                    );
                }
            }
        }
        let bounded = "fn renamed<T:core::marker::Copy>(x:T)->T{x}fn main(){let mut value=1i32;let r=renamed(&mut value);*r=2;}";
        for workers in [1, 8] {
            for check in [true, false] {
                let failure = super::super::compile_observed(bounded.into(), workers, check)
                    .err()
                    .unwrap();
                assert!(
                    !failure.error.to_string().contains("Unsupported"),
                    "{}",
                    failure.error
                );
                assert!(
                    failure
                        .report
                        .unwrap()
                        .node_evaluations
                        .iter()
                        .all(|(key, _)| key.kind != 103)
                );
            }
        }
        for source in [
            "fn renamed<T>(x:T)->T{x}struct S{x:i32}fn main(){let value=S{x:1};let r=renamed(&value.x);}",
            "fn renamed<T>(x:T)->T{x}fn runtime()->i32{1}fn main(){let value=runtime();let r=renamed(&value);}",
            "fn renamed<T>(x:T)->T{x}fn main(){let value=1i32;let nested=&value;let r=renamed(&nested);}",
        ] {
            let error = super::super::compile(source.into(), 1, true).err().unwrap();
            assert!(
                error.to_string().contains("unsupported")
                    || error.to_string().contains("Unsupported"),
                "{source}: {error}"
            );
        }
    }

    #[test]
    fn root_borrow_replay_rejects_kind_place_capability_and_initializer_mutations() {
        let (_, _, budget, publication) = published(
            "fn renamed<T>(x:T)->T{x}fn main(){let mut left=1i32;let mut right=2i32;let r=renamed(&mut left);*r=3;}",
            1,
        );
        let original = publication.original().source_for_test();
        let source = publication.source();
        replay_source_body(&original, source, publication.facts(), &budget).unwrap();
        let borrow = source
            .exprs
            .iter()
            .position(|e| matches!(e.kind, ir::ExprKind::Borrow { .. }))
            .unwrap();
        let root = source.locals.iter().position(|l| l.name == "left").unwrap();
        let sibling = source
            .locals
            .iter()
            .position(|l| l.name == "right")
            .unwrap();
        let ir::Stmt::Let {
            value: initializer, ..
        } = source.body.statements[0]
        else {
            panic!("root let missing")
        };
        for mutation in 0..7 {
            let mut changed = source.as_ref().clone();
            match mutation {
                0 => {
                    let ir::ExprKind::Borrow { kind, .. } = &mut changed.exprs[borrow].kind else {
                        unreachable!()
                    };
                    *kind = ir::BorrowKind::Shared;
                }
                1 => {
                    let ir::ExprKind::Borrow { place, .. } = &mut changed.exprs[borrow].kind else {
                        unreachable!()
                    };
                    place.local = ir::LocalId(sibling);
                }
                2 => {
                    let ir::ExprKind::Borrow { place, .. } = &mut changed.exprs[borrow].kind else {
                        unreachable!()
                    };
                    place.projection.push(ir::Projection::Deref);
                }
                3 => changed.locals[root].mutable = false,
                4 => {
                    let ir::ExprKind::Int(value) = &mut changed.exprs[initializer.0].kind else {
                        unreachable!()
                    };
                    value.bits = 2;
                }
                5 => changed.exprs[initializer.0].ty = ir::Ty::Int(ir::IntTy::U32),
                _ => changed.exprs[borrow].kind = ir::ExprKind::Local(ir::LocalId(root)),
            }
            assert_eq!(
                kind_error(replay_source_body(
                    &original,
                    &Arc::new(changed),
                    publication.facts(),
                    &budget
                )),
                tr::FactsErrorKind::InvalidMetadata,
                "mutation {mutation}"
            );
        }
        let mut changed = original.clone();
        let function = &mut Arc::make_mut(&mut changed.plan.program).functions[1];
        let Stmt::Let {
            value: Expr::Integer(literal),
            ..
        } = &mut function.body.statements[0]
        else {
            panic!("source literal missing")
        };
        literal.suffix = Some(ir::IntTy::U32);
        assert_eq!(
            kind_error(replay_source_body(
                &changed,
                source,
                publication.facts(),
                &budget
            )),
            tr::FactsErrorKind::InvalidMetadata
        );
        for kind in [Some(ir::BorrowKind::Shared), None] {
            let mut changed = original.clone();
            changed.calls[0].argument_borrow = kind;
            assert_eq!(
                kind_error(replay_source_body(
                    &changed,
                    source,
                    publication.facts(),
                    &budget
                )),
                tr::FactsErrorKind::InvalidMetadata
            );
        }
    }

    #[test]
    fn retained_authority_mutations_and_same_ledger_resource() {
        let (_, _, budget, publication) =
            published("fn f(a:i32,b:i32){let x=1;let y=2;{let z=3;}}", 0);
        let original = publication.original().source_for_test();
        replay_source_body(
            &original,
            publication.source(),
            publication.facts(),
            &budget,
        )
        .unwrap();
        let mut mutated = original.clone();
        let a = mutated.binding_mapping[0].1;
        let b = mutated.binding_mapping[1].1;
        mutated.binding_mapping[0].1 = b;
        mutated.binding_mapping[1].1 = a;
        assert_eq!(
            kind_error(replay_source_body(
                &mutated,
                publication.source(),
                publication.facts(),
                &budget
            )),
            tr::FactsErrorKind::InvalidMetadata
        );
        let roots = original
            .binding_mapping
            .iter()
            .enumerate()
            .filter(|(_, row)| original.bindings[row.0].statement.is_some())
            .map(|(i, _)| i)
            .collect::<Vec<_>>();
        let mut mutated = original.clone();
        let a = mutated.binding_mapping[roots[0]].1;
        let b = mutated.binding_mapping[roots[1]].1;
        mutated.binding_mapping[roots[0]].1 = b;
        mutated.binding_mapping[roots[1]].1 = a;
        assert_eq!(
            kind_error(replay_source_body(
                &mutated,
                publication.source(),
                publication.facts(),
                &budget
            )),
            tr::FactsErrorKind::InvalidMetadata
        );
        for mutation in 0..4 {
            let mut mutated = original.clone();
            match mutation {
                0 => {
                    mutated.binding_mapping.pop();
                }
                1 => mutated.binding_mapping.push(mutated.binding_mapping[0]),
                2 => mutated.binding_mapping[0].0 = usize::MAX,
                _ => mutated.identity = sr::OriginalBodyId::new(&budget).unwrap(),
            }
            assert_eq!(
                kind_error(replay_source_body(
                    &mutated,
                    publication.source(),
                    publication.facts(),
                    &budget
                )),
                tr::FactsErrorKind::InvalidMetadata
            );
        }
        // Independently emitted occurrence positions must not be derived from a mutated local mapping.
        let function = original
            .plan
            .callable_body(&original.plan.program, original.declaration)
            .unwrap();
        let inventory = SourceInventory::build(
            &original.plan.program,
            function,
            original.identity.clone(),
            &budget,
        )
        .unwrap();
        let cap = CanonicalSourceCapability {
            plan: original.plan.clone(),
            declaration: original.declaration,
            facts: original.facts.clone(),
            inventory,
            calls: Default::default(),
            builtin_leading_zeros: Default::default(),
            builtin_lens: Default::default(),
            builtin_intrinsics: Default::default(),
            unsizes: Default::default(),
            bindings: Default::default(),
            narrow: Default::default(),
            emissions: Default::default(),
            error: Default::default(),
        };
        let checked = type_callable_source(
            &original.plan.program,
            original.declaration,
            &original.plan,
            &budget,
            Some(&cap),
        )
        .unwrap();
        let before = canonical_root_positions(&cap, &budget).unwrap();
        {
            let mut mappings = cap.bindings.borrow_mut();
            let a = mappings[roots[0]].1;
            let b = mappings[roots[1]].1;
            mappings[roots[0]].1 = b;
            mappings[roots[1]].1 = a;
        }
        let after = canonical_root_positions(&cap, &budget).unwrap();
        assert_eq!(before, after);
        let mut forged = original.clone();
        forged.binding_mapping = cap.bindings.borrow().clone();
        forged.root_positions = after;
        assert_eq!(
            kind_error(validate_binding_coverage(&forged, &checked, &budget)),
            tr::FactsErrorKind::InvalidMetadata
        );
        // Nested/pattern/deferred origins are inventoried but carry no retained binding authority.
        for (index, site) in original.bindings.iter().enumerate() {
            if !site.required {
                assert!(!original.binding_mapping.iter().any(|row| row.0 == index));
                assert!(original.root_positions[index].is_none());
            }
        }
        let (_, _, shared_budget, shared) =
            published("fn id<T>(x:T)->T{x}fn f(r:&i32,s:&i32){let y=id(r);}", 1);
        let mut payload = shared.source().as_ref().clone();
        let local = payload.params[1];
        let argument = payload
            .exprs
            .iter()
            .find_map(|expr| {
                if let ir::ExprKind::Call { args, .. } = &expr.kind {
                    Some(args[0])
                } else {
                    None
                }
            })
            .unwrap();
        payload.exprs[argument.0].kind = ir::ExprKind::Local(local);
        assert_eq!(
            kind_error(sr::replay_original_calls(
                Arc::new(payload),
                shared.original(),
                shared.facts(),
                &shared_budget
            )),
            tr::FactsErrorKind::InvalidMetadata
        );
        let mut source = shared.original().source_for_test();
        source.calls[0].direct_let = None;
        assert_eq!(
            kind_error(replay_source_body(
                &source,
                shared.source(),
                shared.facts(),
                &shared_budget
            )),
            tr::FactsErrorKind::InvalidMetadata
        );
        let same = budget.clone();
        assert!(same.same_authority(&budget));
        for bit in (0..usize::BITS).rev() {
            let _ = same.charge_work(1usize << bit);
        }
        assert!(same.charge_work(1).is_err());
        assert_eq!(
            kind_error(publication.validate_source(
                publication.source(),
                publication.facts(),
                &same
            )),
            tr::FactsErrorKind::Resource
        );
        let foreign = ir::PlanBudget::default();
        assert_eq!(
            kind_error(publication.validate_source(
                publication.source(),
                publication.facts(),
                &foreign
            )),
            tr::FactsErrorKind::InvalidMetadata
        );
    }
}

#[derive(Clone, Debug)]
struct SourceBindingSite {
    node: sr::SourceNode,
    annotation: Option<Type>,
    required: bool,
    is_declare: bool,
    parameter: Option<usize>,
    statement: Option<usize>,
}
#[derive(Clone, Debug)]
struct SourceCallSite {
    node: sr::SourceNode,
    /// Syntactic `receiver.len()` with no arguments or type arguments. This is
    /// only a candidate: the checker alone decides builtin SliceLen from the
    /// typed receiver, and replay requires typed evidence for every builtin row.
    len_candidate: bool,
    leading_zeros_candidate: bool,
    /// Syntactic `receiver.name(args)` with an admitted primitive method name.
    intrinsic_candidate: bool,
    ordinary: Option<AstPath>,
    application: Option<GenericPath>,
    argument_binding: Option<sr::SourceNode>,
    argument_borrow: Option<ir::BorrowKind>,
    direct_let: Option<sr::SourceNode>,
    args_pointer: usize,
}
// Addresses are only a transient index into the exact owned AST during checking.
// They are stripped before OriginalSource is retained.
struct SourceInventory {
    names: Arc<NameStore>,
    lexical: Option<Arc<super::semantic_scope::ScopeArena>>,
    function_scope: Option<super::semantic_scope::ScopeId>,
    owner: sr::OriginalBodyId,
    calls: Vec<SourceCallSite>,
    bindings: Vec<SourceBindingSite>,
    call_pointers: HashMap<usize, usize>,
    statement_pointers: HashMap<usize, usize>,
    scopes: Vec<HashMap<LocalKey, sr::SourceNode>>,
    root_block_pointer: usize,
}
/// Static local-rooted place syntax for `&place`, or None for value operands.
fn borrow_place(mut expr: &Expr) -> Option<Place> {
    let mut fields = Vec::new();
    loop {
        match expr {
            Expr::Var(path) => {
                let local = if path.anchor() == PathAnchor::SelfModule && path.is_empty() {
                    LocalName::Receiver
                } else if path.anchor() == PathAnchor::Relative {
                    LocalName::Named(path.single()?)
                } else {
                    return None;
                };
                fields.reverse();
                return Some(Place { local, fields });
            }
            Expr::Deref { value } => {
                fields.push(FieldSelector::Deref);
                expr = value;
            }
            Expr::Field { base, field } => {
                fields.push(field.clone());
                expr = base;
            }
            Expr::Index { base, index } => {
                fields.push(FieldSelector::ConstantIndex(*index));
                expr = base;
            }
            _ => return None,
        }
    }
}
fn source_local_name(path: &AstPath, names: &NameStore) -> Result<LocalName, String> {
    path.check(names).map_err(native_adapter::name_diagnostic)?;
    match (path.anchor(), path.segments()) {
        (PathAnchor::Relative, [name]) => Ok(LocalName::Named(*name)),
        (PathAnchor::SelfModule, []) => Ok(LocalName::Receiver),
        _ => Err("expected a local-root path".into()),
    }
}
fn reserve_source_map<K: std::hash::Hash + Eq, V>(
    map: &mut HashMap<K, V>,
    budget: &ir::PlanBudget,
) -> Result<(), FactsError> {
    if map.len() < map.capacity() {
        return Ok(());
    }
    let target = map
        .capacity()
        .checked_mul(2)
        .map(|n| n.max(4))
        .ok_or_else(|| FactsError::resource(budget, "source map capacity overflow"))?;
    let row = std::mem::size_of::<K>()
        .checked_add(std::mem::size_of::<V>())
        .and_then(|n| n.checked_add(32))
        .ok_or_else(|| FactsError::resource(budget, "source map row overflow"))?;
    let slots = map
        .capacity()
        .checked_add(
            target
                .checked_mul(2)
                .ok_or_else(|| FactsError::resource(budget, "source map peak overflow"))?,
        )
        .ok_or_else(|| FactsError::resource(budget, "source map peak overflow"))?;
    sr::work(budget, map.len())?;
    sr::retain(
        budget,
        slots
            .checked_mul(row)
            .and_then(|n| n.checked_add(64))
            .ok_or_else(|| FactsError::resource(budget, "source map allocation overflow"))?,
    )?;
    map.try_reserve(target - map.len())
        .map_err(|_| FactsError::resource(budget, "source map allocation"))?;
    Ok(())
}
fn insert_source_pointer(
    map: &mut HashMap<usize, usize>,
    key: usize,
    index: usize,
    budget: &ir::PlanBudget,
) -> Result<(), FactsError> {
    sr::work(budget, 1)?;
    reserve_source_map(map, budget)?;
    if map.insert(key, index).is_some() {
        return Err(FactsError::invalid(
            budget,
            "duplicate original source pointer",
        ));
    }
    Ok(())
}
fn source_node_equal(
    left: &sr::SourceNode,
    right: &sr::SourceNode,
    budget: &ir::PlanBudget,
) -> Result<bool, FactsError> {
    sr::work(budget, left.path.len())?;
    sr::work(budget, right.path.len())?;
    sr::work(budget, 1)?;
    Ok(left == right)
}
fn source_binding_index(
    bindings: &[SourceBindingSite],
    node: &sr::SourceNode,
    budget: &ir::PlanBudget,
) -> Result<Option<usize>, FactsError> {
    for (index, site) in bindings.iter().enumerate() {
        if source_node_equal(&site.node, node, budget)? {
            return Ok(Some(index));
        }
    }
    Ok(None)
}
fn compare_source_type_work(
    ty: &Type,
    budget: &ir::PlanBudget,
    at: usize,
) -> Result<(), FactsError> {
    sr::depth(budget, at)?;
    sr::work(budget, 1)?;
    match ty {
        Type::Apply(path) => compare_source_path_work(path, budget, at + 1)?,
        Type::Projection(projection) => {
            sr::work(budget, 1)?;
            if let SourceProjection::Qualified {
                self_ty,
                trait_path,
                ..
            } = projection.as_ref()
            {
                compare_source_type_work(self_ty, budget, at + 1)?;
                if let TraitPath::Source(path) = trait_path {
                    compare_source_path_work(path, budget, at + 1)?;
                }
            }
        }
        Type::Param(_) | Type::Named(_) => sr::work(budget, 1)?,
        Type::Ref {
            region, referent, ..
        } => {
            if region.is_some() {
                sr::work(budget, 1)?;
            }
            compare_source_type_work(referent, budget, at + 1)?;
        }
        Type::Array(element, _)
        | Type::Slice(element)
        | Type::RawPtr {
            pointee: element, ..
        } => compare_source_type_work(element, budget, at + 1)?,
        Type::Tuple(fields) => {
            for field in fields {
                compare_source_type_work(field, budget, at + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}
fn compare_source_path_work(
    path: &GenericPath,
    budget: &ir::PlanBudget,
    at: usize,
) -> Result<(), FactsError> {
    sr::depth(budget, at)?;
    sr::work(budget, 1)?;
    for segment in &path.segments {
        sr::work(budget, 1)?;
        for binding in &segment.associated_types {
            sr::work(budget, 1)?;
            compare_source_type_work(&binding.value, budget, at + 1)?;
        }
        for ty in &segment.args {
            compare_source_type_work(ty, budget, at + 1)?;
        }
    }
    Ok(())
}
fn compare_call_site_work(
    site: &SourceCallSite,
    budget: &ir::PlanBudget,
) -> Result<(), FactsError> {
    sr::work(budget, site.node.path.len())?;
    if let Some(node) = &site.argument_binding {
        sr::work(budget, node.path.len())?;
    }
    if let Some(node) = &site.direct_let {
        sr::work(budget, node.path.len())?;
    }
    if let Some(name) = &site.ordinary {
        sr::work(budget, name.len())?;
    }
    if let Some(path) = &site.application {
        compare_source_path_work(path, budget, 0)?;
    }
    Ok(())
}
fn validate_original_callee(
    original: &ir::Callee,
    actual: &ir::Callee,
    budget: &ir::PlanBudget,
) -> Result<(), FactsError> {
    sr::work(budget, 1)?;
    let role = match (original, actual) {
        (ir::Callee::Direct(a), ir::Callee::Direct(b)) => a.declaration == b.declaration,
        (ir::Callee::TraitItem(a), ir::Callee::TraitItem(b)) => {
            a.item == b.item
                && a.trait_ref.declaration == b.trait_ref.declaration
                && a.trait_ref.args.len() == b.trait_ref.args.len()
        }
        _ => false,
    };
    if !role || original.type_arguments().len() != actual.type_arguments().len() {
        return Err(FactsError::invalid(
            budget,
            "original source call changed provider role",
        ));
    }
    let mut left = original.metadata_types();
    let mut right = actual.metadata_types();
    loop {
        match (left.next(), right.next()) {
            (None, None) => break,
            (Some(a), Some(b)) => {
                if !replay_type_equal(a, b, budget)? {
                    return Err(FactsError::invalid(
                        budget,
                        "original solved call actual changed",
                    ));
                }
            }
            _ => {
                return Err(FactsError::invalid(
                    budget,
                    "original call metadata count changed",
                ));
            }
        }
    }
    Ok(())
}
fn copy_source_path(
    path: &GenericPath,
    budget: &ir::PlanBudget,
    at: usize,
) -> Result<GenericPath, FactsError> {
    sr::depth(budget, at)?;
    sr::work(budget, 1)?;
    let mut segments = Vec::new();
    segments
        .try_reserve_exact(path.segments.len())
        .map_err(|_| FactsError::resource(budget, "source path copy allocation"))?;
    for segment in &path.segments {
        let mut args = Vec::new();
        args.try_reserve_exact(segment.args.len())
            .map_err(|_| FactsError::resource(budget, "source path argument allocation"))?;
        for ty in &segment.args {
            args.push(copy_source_type(ty, budget, at + 1)?);
        }
        let mut associated_types = Vec::new();
        associated_types
            .try_reserve_exact(segment.associated_types.len())
            .map_err(|_| FactsError::resource(budget, "source equality allocation"))?;
        for binding in &segment.associated_types {
            sr::work(budget, 1)?;
            associated_types.push(AssociatedTypeBinding {
                name: binding.name,
                value: copy_source_type(&binding.value, budget, at + 1)?,
            });
        }
        segments.push(PathSegment {
            name: segment.name,
            args,
            associated_types,
        });
    }
    Ok(GenericPath {
        anchor: path.anchor,
        segments,
        resolved_use: path.resolved_use.clone(),
    })
}
fn copy_source_type(ty: &Type, budget: &ir::PlanBudget, at: usize) -> Result<Type, FactsError> {
    sr::depth(budget, at)?;
    sr::work(budget, 1)?;
    Ok(match ty {
        Type::Apply(path) => Type::Apply(copy_source_path(path, budget, at + 1)?),
        Type::Projection(projection) => {
            let projection = match projection.as_ref() {
                SourceProjection::Qualified {
                    self_ty,
                    trait_path,
                    item,
                } => SourceProjection::Qualified {
                    self_ty: copy_source_type(self_ty, budget, at + 1)?,
                    trait_path: match trait_path {
                        TraitPath::Source(path) => {
                            TraitPath::Source(copy_source_path(path, budget, at + 1)?)
                        }
                        TraitPath::Builtin(kind) => TraitPath::Builtin(*kind),
                    },
                    item: *item,
                },
                SourceProjection::SelfItem { item } => SourceProjection::SelfItem { item: *item },
            };
            Type::Projection(Box::new(projection))
        }
        Type::Param(name) => Type::Param(*name),
        Type::Named(name) => Type::Named(name.clone()),
        Type::Ref {
            kind,
            region,
            referent,
        } => Type::Ref {
            kind: *kind,
            region: *region,
            referent: Box::new(copy_source_type(referent, budget, at + 1)?),
        },
        Type::Array(element, len) => {
            Type::Array(Box::new(copy_source_type(element, budget, at + 1)?), *len)
        }
        Type::Slice(element) => Type::Slice(Box::new(copy_source_type(element, budget, at + 1)?)),
        Type::RawPtr { kind, pointee } => Type::RawPtr {
            kind: *kind,
            pointee: Box::new(copy_source_type(pointee, budget, at + 1)?),
        },
        Type::Tuple(fields) => {
            let mut result = Vec::new();
            result
                .try_reserve_exact(fields.len())
                .map_err(|_| FactsError::resource(budget, "source tuple type allocation"))?;
            for field in fields {
                result.push(copy_source_type(field, budget, at + 1)?);
            }
            Type::Tuple(result)
        }
        Type::SelfType => Type::SelfType,
        Type::Int(kind) => Type::Int(*kind),
        Type::Float(kind) => Type::Float(*kind),
        Type::Char => Type::Char,
        Type::Str => Type::Str,
        Type::Bool => Type::Bool,
        Type::Unit => Type::Unit,
        Type::Never => Type::Never,
        Type::Infer => Type::Infer,
    })
}
fn retain_source_path(
    path: &GenericPath,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<(), FactsError> {
    sr::depth(budget, depth)?;
    sr::work(budget, 1)?;
    sr::retain(
        budget,
        path.segments
            .len()
            .checked_mul(2 * std::mem::size_of::<PathSegment>())
            .and_then(|n| n.checked_add(32))
            .ok_or_else(|| FactsError::resource(budget, "source path sizing overflow"))?,
    )?;
    for segment in &path.segments {
        sr::work(budget, 1)?;

        sr::retain(
            budget,
            segment
                .associated_types
                .len()
                .checked_mul(2 * std::mem::size_of::<AssociatedTypeBinding>())
                .ok_or_else(|| FactsError::resource(budget, "source equality sizing overflow"))?,
        )?;
        for ty in segment.args.iter().chain(
            segment
                .associated_types
                .iter()
                .map(|binding| &binding.value),
        ) {
            retain_source_type(ty, budget, depth + 1)?;
        }
    }
    Ok(())
}
fn retain_source_type(ty: &Type, budget: &ir::PlanBudget, depth: usize) -> Result<(), FactsError> {
    sr::depth(budget, depth)?;
    sr::work(budget, 1)?;
    sr::retain(budget, 2 * std::mem::size_of::<Type>() + 32)?;
    match ty {
        Type::Apply(path) => retain_source_path(path, budget, depth + 1)?,
        Type::Projection(projection) => {
            sr::retain(budget, 2 * std::mem::size_of::<SourceProjection>() + 32)?;
            if let SourceProjection::Qualified {
                self_ty,
                trait_path,
                ..
            } = projection.as_ref()
            {
                retain_source_type(self_ty, budget, depth + 1)?;
                if let TraitPath::Source(path) = trait_path {
                    retain_source_path(path, budget, depth + 1)?;
                }
            }
        }
        Type::Param(_) | Type::Named(_) => {
            sr::work(budget, 1)?;
        }
        Type::Ref {
            region, referent, ..
        } => {
            if region.is_some() {
                sr::work(budget, 1)?;
            }
            retain_source_type(referent, budget, depth + 1)?;
        }
        Type::Array(element, _)
        | Type::Slice(element)
        | Type::RawPtr {
            pointee: element, ..
        } => retain_source_type(element, budget, depth + 1)?,
        Type::Tuple(fields) => {
            for field in fields {
                retain_source_type(field, budget, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}
impl SourceInventory {
    fn build(
        program: &Program,
        function: &Function,
        owner: sr::OriginalBodyId,
        budget: &ir::PlanBudget,
    ) -> Result<Self, FactsError> {
        sr::retain(budget, 128)?;
        let mut scopes = Vec::new();
        scopes
            .try_reserve_exact(1)
            .map_err(|_| FactsError::resource(budget, "source root scope allocation"))?;
        scopes.push(HashMap::new());
        let mut this = Self {
            names: program.names.clone(),
            lexical: program.scopes.clone(),
            function_scope: function.lexical_scope,
            owner: owner.clone(),
            calls: Vec::new(),
            bindings: Vec::new(),
            call_pointers: HashMap::new(),
            statement_pointers: HashMap::new(),
            scopes,
            root_block_pointer: &function.body as *const Block as usize,
        };
        let root = sr::SourceNode {
            owner,
            path: Arc::from([]),
        };
        for (index, (name, ty)) in function.params.iter().enumerate() {
            let node = root.child(0, index, budget)?;
            this.bind(name, node, Some(ty), Some(index), None, budget)?;
        }
        this.block(&function.body, &root.child(1, 0, budget)?, true, budget, 0)?;
        Ok(this)
    }
    fn bind(
        &mut self,
        name: impl LocalInput,
        node: sr::SourceNode,
        annotation: Option<&Type>,
        parameter: Option<usize>,
        statement: Option<usize>,
        budget: &ir::PlanBudget,
    ) -> Result<(), FactsError> {
        sr::work(budget, 1)?;
        sr::retain(budget, 256)?;
        let key = self.lexical_key(&name, true, budget)?;
        if let Some(ty) = annotation {
            retain_source_type(ty, budget, 0)?;
        }
        let annotation = annotation
            .map(|ty| copy_source_type(ty, budget, 0))
            .transpose()?;
        if let Some(key) = key {
            reserve_source_map(self.scopes.last_mut().unwrap(), budget)?;
            self.scopes.last_mut().unwrap().insert(key, node.clone());
        }
        sr::push(
            &mut self.bindings,
            SourceBindingSite {
                node,
                annotation,
                parameter,
                statement,
                required: parameter.is_some(),
                is_declare: false,
            },
            budget,
        )
    }
    fn lookup(
        &self,
        name: impl LocalInput,
        budget: &ir::PlanBudget,
    ) -> Result<Option<sr::SourceNode>, FactsError> {
        let Some(key) = self.lexical_key(&name, false, budget)? else {
            return Ok(None);
        };
        for scope in self.scopes.iter().rev() {
            sr::work(budget, 1)?;
            if let Some(node) = scope.get(&key) {
                return Ok(Some(node.clone()));
            }
        }
        if matches!(key, LocalKey::Anchored(_)) {
            return Err(FactsError::invalid(
                budget,
                "missing original source node for anchored lexical binding",
            ));
        }
        Ok(None)
    }
    fn lexical_key(
        &self,
        name: &impl LocalInput,
        declaration: bool,
        budget: &ir::PlanBudget,
    ) -> Result<Option<LocalKey>, FactsError> {
        let Some(function) = self.function_scope else {
            return name
                .checked_key(&self.names)
                .map_err(|error| source_name_error(&self.names, budget, &error));
        };
        let scopes = self
            .lexical
            .as_ref()
            .ok_or_else(|| FactsError::invalid(budget, "missing original lexical scope owner"))?;
        scopes
            .check_names(self.names.as_ref())
            .map_err(|error| source_scope_error(budget, error))?;
        scopes
            .check_function(function)
            .map_err(|error| source_scope_error(budget, error))?;
        let Some(ident) = name
            .occurrence(&self.names)
            .map_err(|error| source_name_error(&self.names, budget, &error))?
        else {
            return match name
                .checked_key(&self.names)
                .map_err(|error| source_name_error(&self.names, budget, &error))?
            {
                None if declaration => Ok(None),
                None => Err(FactsError::invalid(
                    budget,
                    "wildcard is not an original local reference",
                )),
                Some(LocalKey::Receiver) => Err(FactsError::unsupported(
                    budget,
                    "unsupported: lexical macro receiver binding",
                )),
                _ => Err(source_scope_error(
                    budget,
                    super::semantic_scope::ScopeError::UnrecordedOccurrence,
                )),
            };
        };
        let binding = if declaration {
            Some(
                scopes
                    .declaration_for(self.names.as_ref(), ident)
                    .map_err(|error| source_scope_error(budget, error))?,
            )
        } else {
            if scopes
                .reference_function(self.names.as_ref(), ident)
                .map_err(|error| source_scope_error(budget, error))?
                != function
            {
                return Err(FactsError::invalid(
                    budget,
                    "original reference has foreign lexical function",
                ));
            }
            scopes
                .reference_for(self.names.as_ref(), ident)
                .map_err(|error| source_scope_error(budget, error))?
        };
        if let Some(binding) = binding {
            if scopes
                .binding_function(binding)
                .map_err(|error| source_scope_error(budget, error))?
                != function
            {
                return Err(FactsError::invalid(
                    budget,
                    "original binding has foreign lexical function",
                ));
            }
            return Ok(Some(LocalKey::Anchored(binding)));
        }
        Ok(None)
    }
    fn shadow_pattern(
        &mut self,
        pattern: &Pattern,
        node: &sr::SourceNode,
        budget: &ir::PlanBudget,
        depth: usize,
    ) -> Result<(), FactsError> {
        sr::depth(budget, depth)?;
        sr::work(budget, 1)?;
        match pattern {
            Pattern::Binding { name, .. } => {
                self.bind(name, node.clone(), None, None, None, budget)?
            }
            Pattern::Path(path) => {
                if let Some(name) = path.single() {
                    self.bind(name, node.clone(), None, None, None, budget)?;
                }
            }
            Pattern::Apply { pattern, .. } | Pattern::Ref { pattern, .. } => {
                self.shadow_pattern(pattern, node, budget, depth + 1)?
            }
            Pattern::Tuple(fields) | Pattern::Array(fields) => {
                for (i, p) in fields.iter().enumerate() {
                    self.shadow_pattern(p, &node.child(8, i, budget)?, budget, depth + 1)?;
                }
            }
            Pattern::TupleRest(fields)
            | Pattern::ArrayRest(fields)
            | Pattern::VariantRest { fields, .. } => {
                for (i, pattern) in fields.prefix.iter().chain(&fields.suffix).enumerate() {
                    self.shadow_pattern(pattern, &node.child(8, i, budget)?, budget, depth + 1)?;
                }
            }
            Pattern::Variant { fields, .. } => {
                for (i, (_, p)) in fields.iter().enumerate() {
                    self.shadow_pattern(p, &node.child(8, i, budget)?, budget, depth + 1)?;
                }
            }
            Pattern::Struct { fields, .. } => {
                for (i, (_, p)) in fields.iter().enumerate() {
                    self.shadow_pattern(p, &node.child(8, i, budget)?, budget, depth + 1)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn block(
        &mut self,
        block: &Block,
        node: &sr::SourceNode,
        top: bool,
        budget: &ir::PlanBudget,
        depth: usize,
    ) -> Result<(), FactsError> {
        sr::depth(budget, depth)?;
        sr::retain(budget, 128)?;
        self.scopes
            .try_reserve(1)
            .map_err(|_| FactsError::resource(budget, "source scope stack allocation"))?;
        self.scopes.push(HashMap::new());
        for (index, statement) in block.statements.iter().enumerate() {
            sr::work(budget, 1)?;
            let here = node.child(2, index, budget)?;
            match statement {
                Stmt::Let {
                    name, ty, value, ..
                } => {
                    let absent =
                        top && matches!(ty, Type::Infer) && matches!(value, Expr::Call { .. });
                    self.expr(
                        value,
                        &here.child(3, 0, budget)?,
                        None,
                        if absent { Some(here.clone()) } else { None },
                        budget,
                        depth + 1,
                    )?;
                    let binding = self.bindings.len();
                    self.bind(
                        name,
                        here.clone(),
                        if matches!(ty, Type::Infer) {
                            None
                        } else {
                            Some(ty)
                        },
                        None,
                        if top { Some(index) } else { None },
                        budget,
                    )?;
                    self.bindings[binding].required = top;
                    insert_source_pointer(
                        &mut self.statement_pointers,
                        statement as *const Stmt as usize,
                        binding,
                        budget,
                    )?;
                }
                Stmt::Declare { name, ty, .. } => {
                    let binding = self.bindings.len();
                    self.bind(
                        name,
                        here.clone(),
                        if matches!(ty, Type::Infer) {
                            None
                        } else {
                            Some(ty)
                        },
                        None,
                        if top { Some(index) } else { None },
                        budget,
                    )?;
                    // Deferred untyped declarations are synthesized later and cannot be shared replay origins.
                    self.bindings[binding].required = top && !matches!(ty, Type::Infer);
                    self.bindings[binding].is_declare = true;
                    insert_source_pointer(
                        &mut self.statement_pointers,
                        statement as *const Stmt as usize,
                        binding,
                        budget,
                    )?;
                }
                Stmt::LetElse {
                    pattern,
                    value,
                    else_block,
                    ..
                } => {
                    self.expr(
                        value,
                        &here.child(23, 0, budget)?,
                        None,
                        None,
                        budget,
                        depth + 1,
                    )?;
                    self.block(
                        else_block,
                        &here.child(23, 1, budget)?,
                        false,
                        budget,
                        depth + 1,
                    )?;
                    // Successful bindings belong to the actual enclosing scope,
                    // not an artificial match arm or recipe root statement.
                    self.shadow_pattern(pattern, &here.child(23, 2, budget)?, budget, depth + 1)?;
                }
                Stmt::LetPattern { pattern, value, .. } => {
                    self.expr(
                        value,
                        &here.child(3, 0, budget)?,
                        None,
                        None,
                        budget,
                        depth + 1,
                    )?;
                    self.shadow_pattern(pattern, &here, budget, depth + 1)?;
                }
                Stmt::AssignDeref { carrier, value } => {
                    self.expr(
                        carrier,
                        &here.child(4, 0, budget)?,
                        None,
                        None,
                        budget,
                        depth + 1,
                    )?;
                    self.expr(
                        value,
                        &here.child(4, 1, budget)?,
                        None,
                        None,
                        budget,
                        depth + 1,
                    )?;
                }
                Stmt::AssignIndex { index, value, .. }
                | Stmt::AssignIndexOp { index, value, .. } => {
                    self.expr(
                        index,
                        &here.child(4, 0, budget)?,
                        None,
                        None,
                        budget,
                        depth + 1,
                    )?;
                    self.expr(
                        value,
                        &here.child(4, 1, budget)?,
                        None,
                        None,
                        budget,
                        depth + 1,
                    )?;
                }
                Stmt::Assign { value, .. }
                | Stmt::AssignOp { value, .. }
                | Stmt::AssignPlace { value, .. }
                | Stmt::Expr(value)
                | Stmt::ImplicitExpr(value)
                | Stmt::Return(Some(value)) => self.expr(
                    value,
                    &here.child(4, 0, budget)?,
                    None,
                    None,
                    budget,
                    depth + 1,
                )?,
                Stmt::WhileLet {
                    pattern,
                    scrutinee,
                    body,
                    ..
                } => {
                    self.expr(
                        scrutinee,
                        &here.child(22, 0, budget)?,
                        None,
                        None,
                        budget,
                        depth + 1,
                    )?;
                    sr::retain(budget, 128)?;
                    self.scopes.try_reserve(1).map_err(|_| {
                        FactsError::resource(budget, "source while-let scope allocation")
                    })?;
                    self.scopes.push(HashMap::new());
                    let body_result = (|| {
                        self.shadow_pattern(
                            pattern,
                            &here.child(22, 1, budget)?,
                            budget,
                            depth + 1,
                        )?;
                        self.block(body, &here.child(22, 2, budget)?, false, budget, depth + 1)
                    })();
                    self.scopes.pop();
                    body_result?;
                }
                Stmt::For {
                    pattern,
                    source,
                    body,
                    ..
                } => {
                    // Inventory tag 24: iterable start/value 0, range end 1,
                    // pattern 2 and body 3. Bindings are body-only.
                    let (first, end) = match &source.iterable {
                        ForIterable::Range { start, end, .. } => (start, Some(end)),
                        ForIterable::Value(value) => (value, None),
                    };
                    self.expr(
                        first,
                        &here.child(24, 0, budget)?,
                        None,
                        None,
                        budget,
                        depth + 1,
                    )?;
                    if let Some(end) = end {
                        self.expr(
                            end,
                            &here.child(24, 1, budget)?,
                            None,
                            None,
                            budget,
                            depth + 1,
                        )?;
                    }
                    sr::retain(budget, 128)?;
                    self.scopes
                        .try_reserve(1)
                        .map_err(|_| FactsError::resource(budget, "source for scope allocation"))?;
                    self.scopes.push(HashMap::new());
                    let body_result = (|| {
                        self.shadow_pattern(
                            pattern,
                            &here.child(24, 2, budget)?,
                            budget,
                            depth + 1,
                        )?;
                        self.block(body, &here.child(24, 3, budget)?, false, budget, depth + 1)
                    })();
                    self.scopes.pop();
                    body_result?;
                }
                Stmt::While {
                    condition, body, ..
                } => {
                    self.expr(
                        condition,
                        &here.child(4, 0, budget)?,
                        None,
                        None,
                        budget,
                        depth + 1,
                    )?;
                    self.block(body, &here.child(5, 0, budget)?, false, budget, depth + 1)?;
                }
                Stmt::Return(None) => {}
            }
        }
        if let Some(tail) = &block.tail {
            self.expr(
                tail,
                &node.child(6, 0, budget)?,
                None,
                None,
                budget,
                depth + 1,
            )?;
        }
        self.scopes.pop();
        Ok(())
    }
    fn expr(
        &mut self,
        expr: &Expr,
        node: &sr::SourceNode,
        application: Option<&GenericPath>,
        direct_let: Option<sr::SourceNode>,
        budget: &ir::PlanBudget,
        depth: usize,
    ) -> Result<(), FactsError> {
        sr::depth(budget, depth)?;
        sr::work(budget, 1)?;
        match expr {
            Expr::UnexpandedMacro(_) => {
                return Err(source_error(
                    budget,
                    "unexpanded macro syntax cannot enter active source inventory",
                ));
            }
            Expr::Output(output) => validate_output_site(output, &self.names)
                .map_err(|message| source_error(budget, &message))?,
            Expr::Format { arguments, .. } => {
                for (index, argument) in arguments.iter().enumerate() {
                    self.expr(
                        argument,
                        &node.child(21, index, budget)?,
                        None,
                        None,
                        budget,
                        depth + 1,
                    )?;
                }
            }
            Expr::Panic(panic) => validate_panic_site(panic, &self.names)
                .map_err(|message| source_error(budget, &message))?,
            Expr::Apply { path, value } => {
                return self.expr(
                    value,
                    &node.child(7, 0, budget)?,
                    Some(path),
                    None,
                    budget,
                    depth + 1,
                );
            }
            Expr::Call { name, args } => {
                sr::work(budget, 1)?;
                sr::retain(
                    budget,
                    std::mem::size_of_val(name)
                        .checked_add(128)
                        .ok_or_else(|| FactsError::resource(budget, "source call name overflow"))?,
                )?;
                if let Some(path) = application {
                    retain_source_path(path, budget, 0)?;
                }
                let (binding, argument_borrow) = match args.as_slice() {
                    [Expr::Var(name)] => (self.lookup(name, budget)?, None),
                    [Expr::Borrow { kind, place }] if place.fields.is_empty() => {
                        (self.lookup(place.local, budget)?, Some(*kind))
                    }
                    _ => (None, None),
                };
                let index = self.calls.len();
                sr::push(
                    &mut self.calls,
                    SourceCallSite {
                        node: node.clone(),
                        len_candidate: false,
                        leading_zeros_candidate: false,
                        intrinsic_candidate: false,
                        ordinary: Some(name.clone()),
                        application: application
                            .map(|path| copy_source_path(path, budget, 0))
                            .transpose()?,
                        argument_binding: binding,
                        argument_borrow,
                        direct_let,
                        args_pointer: args.as_ptr() as usize,
                    },
                    budget,
                )?;
                insert_source_pointer(
                    &mut self.call_pointers,
                    expr as *const Expr as usize,
                    index,
                    budget,
                )?;
                for (i, arg) in args.iter().enumerate() {
                    self.expr(
                        arg,
                        &node.child(9, i, budget)?,
                        None,
                        None,
                        budget,
                        depth + 1,
                    )?;
                }
            }
            Expr::MethodCall { args, .. }
            | Expr::AssociatedCall { args, .. }
            | Expr::TraitCall { args, .. } => {
                let index = self.calls.len();
                sr::push(
                    &mut self.calls,
                    SourceCallSite {
                        node: node.clone(),
                        leading_zeros_candidate: leading_zeros_candidate(expr, &self.names)
                            .map_err(|e| source_error(budget, &e))?,
                        len_candidate: slice_len_candidate(expr, &self.names)
                            .map_err(|e| source_error(budget, &e))?,
                        intrinsic_candidate: intrinsic_candidate(expr, &self.names)
                            .map_err(|e| source_error(budget, &e))?,
                        ordinary: None,
                        application: None,
                        argument_binding: None,
                        argument_borrow: None,
                        direct_let: None,
                        args_pointer: args.as_ptr() as usize,
                    },
                    budget,
                )?;
                insert_source_pointer(
                    &mut self.call_pointers,
                    expr as *const Expr as usize,
                    index,
                    budget,
                )?;
                if let Expr::MethodCall { receiver, .. } = expr {
                    self.expr(
                        receiver,
                        &node.child(10, 0, budget)?,
                        None,
                        None,
                        budget,
                        depth + 1,
                    )?;
                }
                for (i, arg) in args.iter().enumerate() {
                    self.expr(
                        arg,
                        &node.child(9, i, budget)?,
                        None,
                        None,
                        budget,
                        depth + 1,
                    )?;
                }
            }
            Expr::Cast { value, .. }
            | Expr::Deref { value }
            | Expr::BorrowValue { value, .. }
            | Expr::Repeat { value, .. }
            | Expr::Unary { value, .. } => self.expr(
                value,
                &node.child(11, 0, budget)?,
                None,
                None,
                budget,
                depth + 1,
            )?,
            Expr::Array(fields) | Expr::Tuple(fields) | Expr::TupleStruct { fields, .. } => {
                let tag = if matches!(expr, Expr::TupleStruct { .. }) {
                    14
                } else {
                    12
                };
                for (i, e) in fields.iter().enumerate() {
                    self.expr(
                        e,
                        &node.child(tag, i, budget)?,
                        None,
                        None,
                        budget,
                        depth + 1,
                    )?;
                }
            }
            Expr::DynamicIndex { base, index }
            | Expr::Binary {
                left: base,
                right: index,
                ..
            } => {
                self.expr(
                    base,
                    &node.child(13, 0, budget)?,
                    None,
                    None,
                    budget,
                    depth + 1,
                )?;
                self.expr(
                    index,
                    &node.child(13, 1, budget)?,
                    None,
                    None,
                    budget,
                    depth + 1,
                )?;
            }
            Expr::Index { base, .. } | Expr::Field { base, .. } => self.expr(
                base,
                &node.child(13, 0, budget)?,
                None,
                None,
                budget,
                depth + 1,
            )?,
            Expr::EnumVariant { fields, .. } => {
                for (i, (_, e)) in fields.iter().enumerate() {
                    self.expr(
                        e,
                        &node.child(14, i, budget)?,
                        None,
                        None,
                        budget,
                        depth + 1,
                    )?;
                }
            }
            Expr::Struct { fields, .. } => {
                for (i, (_, e)) in fields.iter().enumerate() {
                    self.expr(
                        e,
                        &node.child(14, i, budget)?,
                        None,
                        None,
                        budget,
                        depth + 1,
                    )?;
                }
            }
            Expr::Match { scrutinee, arms } => {
                self.expr(
                    scrutinee,
                    &node.child(15, 0, budget)?,
                    None,
                    None,
                    budget,
                    depth + 1,
                )?;
                for (i, arm) in arms.iter().enumerate() {
                    sr::retain(budget, 128)?;
                    self.scopes
                        .try_reserve(1)
                        .map_err(|_| FactsError::resource(budget, "source arm scope allocation"))?;
                    self.scopes.push(HashMap::new());
                    let here = node.child(16, i, budget)?;
                    self.shadow_pattern(&arm.pattern, &here, budget, depth + 1)?;
                    self.expr(
                        &arm.body,
                        &here.child(17, 0, budget)?,
                        None,
                        None,
                        budget,
                        depth + 1,
                    )?;
                    self.scopes.pop();
                }
            }
            Expr::IfLet {
                pattern,
                scrutinee,
                then_block,
                else_branch,
            } => {
                self.expr(
                    scrutinee,
                    &node.child(21, 0, budget)?,
                    None,
                    None,
                    budget,
                    depth + 1,
                )?;
                sr::retain(budget, 128)?;
                self.scopes
                    .try_reserve(1)
                    .map_err(|_| FactsError::resource(budget, "source if-let scope allocation"))?;
                self.scopes.push(HashMap::new());
                let then_result = (|| {
                    self.shadow_pattern(pattern, &node.child(21, 1, budget)?, budget, depth + 1)?;
                    self.block(
                        then_block,
                        &node.child(21, 2, budget)?,
                        false,
                        budget,
                        depth + 1,
                    )
                })();
                self.scopes.pop();
                then_result?;
                if let Some(branch) = else_branch {
                    self.expr(
                        branch,
                        &node.child(21, 3, budget)?,
                        None,
                        None,
                        budget,
                        depth + 1,
                    )?;
                }
            }
            Expr::If {
                condition,
                then_block,
                else_block,
            } => {
                self.expr(
                    condition,
                    &node.child(18, 0, budget)?,
                    None,
                    None,
                    budget,
                    depth + 1,
                )?;
                self.block(
                    then_block,
                    &node.child(18, 1, budget)?,
                    false,
                    budget,
                    depth + 1,
                )?;
                self.block(
                    else_block,
                    &node.child(18, 2, budget)?,
                    false,
                    budget,
                    depth + 1,
                )?;
            }
            Expr::Block(block)
            | Expr::Unsafe(block)
            | Expr::Loop { body: block, .. }
            | Expr::LabeledBlock { body: block, .. } => {
                self.block(block, &node.child(19, 0, budget)?, false, budget, depth + 1)?
            }
            Expr::Break {
                value: Some(value), ..
            } => self.expr(
                value,
                &node.child(20, 0, budget)?,
                None,
                None,
                budget,
                depth + 1,
            )?,
            Expr::Borrow { .. }
            | Expr::Integer(_)
            | Expr::Float(_)
            | Expr::Char(_)
            | Expr::Str(_)
            | Expr::Bool(_)
            | Expr::Var(_)
            | Expr::Continue { .. }
            | Expr::Break { value: None, .. }
            | Expr::Println(_) => {}
        }
        Ok(())
    }
}

/// Syntactic shape of a builtin shared-slice `.len()` occurrence.
fn slice_len_candidate(expr: &Expr, names: &NameStore) -> Result<bool, String> {
    // This lexical candidate never grants builtin authority. The retained
    // checker proof still establishes the actual SliceLen operation.
    if let Expr::MethodCall {
        name,
        type_args,
        args,
        ..
    } = expr
    {
        Ok(names
            .spelling(*name)
            .map_err(native_adapter::name_diagnostic)?
            == "len"
            && type_args.is_empty()
            && args.is_empty())
    } else {
        Ok(false)
    }
}

/// Syntax alone never grants primitive-method authority: the checker resolves
/// the actual operation from the typed receiver.
fn intrinsic_candidate(expr: &Expr, names: &NameStore) -> Result<bool, String> {
    if let Expr::MethodCall {
        name, type_args, ..
    } = expr
    {
        Ok(type_args.is_empty()
            && ir::IntrinsicOp::candidate_name(
                names
                    .spelling(*name)
                    .map_err(native_adapter::name_diagnostic)?,
            ))
    } else {
        Ok(false)
    }
}

/// Typed primitive-method occurrence: source call index, the checked
/// Intrinsic expression, and its exact checked receiver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct BuiltinIntrinsicOccurrence {
    pub(super) source_index: usize,
    pub(super) expr: ir::ExprId,
    pub(super) receiver: ir::ExprId,
}

/// Stock E0658: `X..` directly inside a slice or array pattern is feature-gated.
fn reject_slice_half_open(pattern: &Pattern) -> Result<(), &'static str> {
    if let Pattern::Range {
        lo: Some(_),
        hi: None,
        ..
    } = pattern
    {
        return Err("`X..` patterns in slices are experimental");
    }
    Ok(())
}

/// Syntax alone never grants integer-method authority.
fn leading_zeros_candidate(expr: &Expr, names: &NameStore) -> Result<bool, String> {
    if let Expr::MethodCall {
        name,
        type_args,
        args,
        ..
    } = expr
    {
        Ok(names
            .spelling(*name)
            .map_err(native_adapter::name_diagnostic)?
            == "leading_zeros"
            && type_args.is_empty()
            && args.is_empty())
    } else {
        Ok(false)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct BuiltinLeadingZerosOccurrence {
    pub(super) source_index: usize,
    pub(super) expr: ir::ExprId,
    pub(super) receiver: ir::ExprId,
}

/// Typed builtin `.len()` occurrence: original source call index, the checked
/// SliceLen expression, and its exact checked receiver expression.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct BuiltinLenOccurrence {
    pub(super) source_index: usize,
    pub(super) expr: ir::ExprId,
    pub(super) receiver: ir::ExprId,
}
/// Checked ArrayToSlice wrapper and its exact wrapped operand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct UnsizeOccurrence {
    pub(super) wrapper: ir::ExprId,
    pub(super) reference: ir::ExprId,
}

#[derive(Clone)]
struct RawSharedOccurrence {
    source_index: usize,
    binding_index: usize,
    local: ir::LocalId,
    trace: Arc<sr::SharedIdentityTrace>,
}
pub(super) struct CanonicalSourceCapability {
    plan: PlanCtx,
    declaration: ir::DeclId,
    facts: Arc<tr::ValidatedFacts>,
    inventory: SourceInventory,
    calls: std::cell::RefCell<Vec<(usize, ir::ExprId, ir::Callee)>>,
    builtin_leading_zeros: std::cell::RefCell<Vec<BuiltinLeadingZerosOccurrence>>,
    builtin_lens: std::cell::RefCell<Vec<BuiltinLenOccurrence>>,
    builtin_intrinsics: std::cell::RefCell<Vec<BuiltinIntrinsicOccurrence>>,
    unsizes: std::cell::RefCell<Vec<UnsizeOccurrence>>,
    bindings: std::cell::RefCell<Vec<(usize, ir::LocalId)>>,
    narrow: std::cell::RefCell<Vec<RawSharedOccurrence>>,
    emissions: std::cell::RefCell<Vec<(usize, usize)>>,
    error: std::cell::RefCell<Option<FactsError>>,
}
#[cfg_attr(test, derive(Clone))]
pub(super) struct OriginalSource {
    plan: PlanCtx,
    declaration: ir::DeclId,
    facts: Arc<tr::ValidatedFacts>,
    identity: sr::OriginalBodyId,
    calls: Vec<SourceCallSite>,
    bindings: Vec<SourceBindingSite>,
    call_mapping: Vec<(usize, ir::ExprId, ir::Callee)>,
    builtin_leading_zeros_mapping: Vec<BuiltinLeadingZerosOccurrence>,
    builtin_len_mapping: Vec<BuiltinLenOccurrence>,
    builtin_intrinsic_mapping: Vec<BuiltinIntrinsicOccurrence>,
    unsize_mapping: Vec<UnsizeOccurrence>,
    binding_mapping: Vec<(usize, ir::LocalId)>,
    narrow: Vec<RawSharedOccurrence>,
    root_positions: Vec<Option<usize>>,
}
impl CanonicalSourceCapability {
    fn render(&self, error: FactsError) -> String {
        let diagnostic = error.clone().into_legacy_diagnostic(&self.plan.budget);
        *self.error.borrow_mut() = Some(if self.plan.budget.has_resource_failure() {
            FactsError::resource(&self.plan.budget, "source diagnostic budget exhausted")
        } else {
            error
        });
        diagnostic
    }
    pub(super) fn validate_original(
        &self,
        original: &OriginalSource,
        budget: &ir::PlanBudget,
    ) -> Result<(), FactsError> {
        validate_original_context(original, &self.facts, budget)?;
        if original.identity != self.inventory.owner
            || original.declaration != self.declaration
            || !Arc::ptr_eq(&original.plan.program, &self.plan.program)
        {
            return Err(FactsError::invalid(
                budget,
                "original body changed canonical input",
            ));
        }
        Ok(())
    }
    fn record_call(&self, expr: &Expr, id: ir::ExprId, kind: &ir::ExprKind) -> Result<(), String> {
        let budget = &self.plan.budget;
        sr::work(budget, 1).map_err(|e| self.render(e))?;
        let index = self
            .inventory
            .call_pointers
            .get(&(expr as *const Expr as usize))
            .copied()
            .ok_or_else(|| {
                self.render(FactsError::invalid(
                    budget,
                    "source call has no original occurrence",
                ))
            })?;
        let ir::ExprKind::Call { target, .. } = kind else {
            return Err(self.render(FactsError::invalid(
                budget,
                "original source call did not produce a call",
            )));
        };
        sr::retain(budget, std::mem::size_of::<ir::Callee>()).map_err(|e| self.render(e))?;
        sr::push(
            &mut self.calls.borrow_mut(),
            (index, id, target.clone()),
            budget,
        )
        .map_err(|e| self.render(e))
    }
    /// Record a typed primitive method at its exact original source occurrence.
    fn record_builtin_intrinsic(
        &self,
        expr: &Expr,
        id: ir::ExprId,
        checked: &[ir::TypedExpr],
    ) -> Result<(), String> {
        let budget = &self.plan.budget;
        sr::work(budget, 1).map_err(|e| self.render(e))?;
        let index = self
            .inventory
            .call_pointers
            .get(&(expr as *const Expr as usize))
            .copied()
            .ok_or_else(|| {
                self.render(FactsError::invalid(
                    budget,
                    "primitive method has no original occurrence",
                ))
            })?;
        let Some(ir::TypedExpr {
            kind: ir::ExprKind::Intrinsic { intrinsic, args },
            ty,
            ..
        }) = checked.get(id.0)
        else {
            return Err(self.render(FactsError::invalid(
                budget,
                "primitive method occurrence did not produce Intrinsic",
            )));
        };
        let receiver = *args.first().ok_or_else(|| {
            self.render(FactsError::invalid(
                budget,
                "primitive method lost receiver",
            ))
        })?;
        if !self.inventory.calls[index].intrinsic_candidate
            || intrinsic.signature().map(|s| s.result).as_ref() != Some(ty)
            || !checked
                .get(receiver.0)
                .is_some_and(|r| r.ty == intrinsic.receiver.ty() || r.ty == ir::Ty::Never)
        {
            return Err(self.render(FactsError::invalid(
                budget,
                "primitive method occurrence has wrong typed shape",
            )));
        }
        sr::push(
            &mut self.builtin_intrinsics.borrow_mut(),
            BuiltinIntrinsicOccurrence {
                source_index: index,
                expr: id,
                receiver,
            },
            budget,
        )
        .map_err(|e| self.render(e))
    }
    /// Record a typed builtin `.len()` at its exact original source occurrence.
    /// Only a syntactic candidate whose checked result is exactly SliceLen over a
    /// shared slice qualifies; nothing is inferred from spelling alone.
    fn record_builtin_len(
        &self,
        expr: &Expr,
        id: ir::ExprId,
        checked: &[ir::TypedExpr],
    ) -> Result<(), String> {
        let budget = &self.plan.budget;
        sr::work(budget, 1).map_err(|e| self.render(e))?;
        let index = self
            .inventory
            .call_pointers
            .get(&(expr as *const Expr as usize))
            .copied()
            .ok_or_else(|| {
                self.render(FactsError::invalid(
                    budget,
                    "builtin length has no original occurrence",
                ))
            })?;
        let site = &self.inventory.calls[index];
        let typed = checked.get(id.0).map(|e| (&e.kind, &e.ty));
        let Some((ir::ExprKind::SliceLen { slice }, ty)) = typed else {
            return Err(self.render(FactsError::invalid(
                budget,
                "builtin length occurrence did not produce SliceLen",
            )));
        };
        if !site.len_candidate
            || *ty != ir::Ty::Int(ir::IntTy::Usize)
            || !checked
                .get(slice.0)
                .is_some_and(|receiver| receiver.ty.shared_descriptor_element().is_some())
        {
            return Err(self.render(FactsError::invalid(
                budget,
                "builtin length occurrence has wrong typed shape",
            )));
        }
        sr::push(
            &mut self.builtin_lens.borrow_mut(),
            BuiltinLenOccurrence {
                source_index: index,
                expr: id,
                receiver: *slice,
            },
            budget,
        )
        .map_err(|e| self.render(e))
    }
    fn record_builtin_leading_zeros(
        &self,
        expr: &Expr,
        id: ir::ExprId,
        checked: &[ir::TypedExpr],
    ) -> Result<(), String> {
        let budget = &self.plan.budget;
        sr::work(budget, 1).map_err(|e| self.render(e))?;
        let index = self
            .inventory
            .call_pointers
            .get(&(expr as *const Expr as usize))
            .copied()
            .ok_or_else(|| {
                self.render(FactsError::invalid(
                    budget,
                    "leading zeros has no original occurrence",
                ))
            })?;
        let Some(result) = checked.get(id.0) else {
            return Err(self.render(FactsError::invalid(budget, "leading zeros outside arena")));
        };
        let ir::ExprKind::LeadingZeros { value } = result.kind else {
            return Err(self.render(FactsError::invalid(budget, "expected LeadingZeros")));
        };
        if !self.inventory.calls[index].leading_zeros_candidate
            || result.ty != ir::Ty::Int(ir::IntTy::U32)
            || !checked.get(value.0).is_some_and(|receiver| {
                matches!(receiver.ty, ir::Ty::Int(_)) && receiver.completion == result.completion
            })
        {
            return Err(self.render(FactsError::invalid(
                budget,
                "leading zeros typed shape changed",
            )));
        }
        sr::push(
            &mut self.builtin_leading_zeros.borrow_mut(),
            BuiltinLeadingZerosOccurrence {
                source_index: index,
                expr: id,
                receiver: value,
            },
            budget,
        )
        .map_err(|e| self.render(e))
    }

    fn record_unsize(&self, wrapper: ir::ExprId, reference: ir::ExprId) -> Result<(), String> {
        let budget = &self.plan.budget;
        sr::work(budget, 1).map_err(|e| self.render(e))?;
        sr::push(
            &mut self.unsizes.borrow_mut(),
            UnsizeOccurrence { wrapper, reference },
            budget,
        )
        .map_err(|e| self.render(e))
    }
    fn record_emission(&self, statement: &Stmt, position: usize) -> Result<(), String> {
        let budget = &self.plan.budget;
        sr::work(budget, 1).map_err(|e| self.render(e))?;
        if let Some(index) = self
            .inventory
            .statement_pointers
            .get(&(statement as *const Stmt as usize))
        {
            let site = &self.inventory.bindings[*index];
            if site.required && site.statement.is_some() {
                sr::push(&mut self.emissions.borrow_mut(), (*index, position), budget)
                    .map_err(|e| self.render(e))?;
            }
        }
        Ok(())
    }
    fn shift_root_emissions(&self, block: &Block, position: usize) -> Result<(), String> {
        if block as *const Block as usize != self.inventory.root_block_pointer {
            return Ok(());
        }
        let budget = &self.plan.budget;
        sr::work(budget, self.emissions.borrow().len()).map_err(|e| self.render(e))?;
        for (_, actual) in self.emissions.borrow_mut().iter_mut() {
            if *actual >= position {
                *actual = actual.checked_add(1).ok_or_else(|| {
                    self.render(FactsError::resource(
                        budget,
                        "root insertion position overflow",
                    ))
                })?;
            }
        }
        Ok(())
    }
    fn record_binding(&self, statement: &Stmt, local: ir::LocalId) -> Result<(), String> {
        let budget = &self.plan.budget;
        sr::work(budget, 1).map_err(|e| self.render(e))?;
        if let Some(index) = self
            .inventory
            .statement_pointers
            .get(&(statement as *const Stmt as usize))
        {
            if !self.inventory.bindings[*index].required {
                return Ok(());
            }
            sr::push(&mut self.bindings.borrow_mut(), (*index, local), budget)
                .map_err(|e| self.render(e))?;
        }
        Ok(())
    }
    fn shared_actual(
        &self,
        declaration: ir::DeclId,
        args: &[Expr],
        expected: Option<&ir::Ty>,
        checker: &Checker<'_>,
    ) -> Result<Option<ir::Ty>, String> {
        let budget = &self.plan.budget;
        if expected.is_some() {
            return Ok(None);
        }
        sr::work(
            budget,
            self.inventory
                .calls
                .len()
                .checked_add(self.inventory.bindings.len())
                .ok_or_else(|| {
                    self.render(FactsError::resource(budget, "source lookup work overflow"))
                })?,
        )
        .map_err(|e| self.render(e))?;
        let Some((index, site)) = self.inventory.calls.iter().enumerate().find(|(_, site)| {
            site.args_pointer == args.as_ptr() as usize && site.ordinary.is_some()
        }) else {
            return Ok(None);
        };
        if site.application.is_some() || site.direct_let.is_none() {
            return Ok(None);
        }
        let (name, fresh_borrow) = match args {
            [Expr::Var(name)] => (source_local_name(name, &self.plan.program.names)?, None),
            [Expr::Borrow { kind, place }] if place.fields.is_empty() => (place.local, Some(*kind)),
            _ => return Ok(None),
        };
        if fresh_borrow != site.argument_borrow {
            return Err(self.render(FactsError::invalid(
                budget,
                "original borrow argument classification changed",
            )));
        }
        if fresh_borrow.is_some() {
            // This extension owns only the original one-slot forwarding schema.
            // Other borrowed argument shapes must retain their existing inference path.
            sr::work(budget, 1).map_err(|e| self.render(e))?;
            let view = self
                .facts
                .original_callable_schema(tr::OriginalCallableSubject::Direct(declaration))
                .map_err(|e| self.render(e))?;
            let scopes = view.generic_env().scopes();
            if scopes.len() != 1 || scopes[0].arity() != 1 || view.signature().params.len() != 1 {
                return Ok(None);
            }
            let parameter = ir::Ty::Param(ir::ParamId {
                owner: scopes[0].binder(),
                index: 0,
            });
            if view.signature().params[0] != parameter || view.signature().result != parameter {
                return Ok(None);
            }
        }
        let local = checker.lookup(name)?;
        let Some(binding) = &site.argument_binding else {
            return Ok(None);
        };
        let Some(binding_index) = source_binding_index(&self.inventory.bindings, binding, budget)
            .map_err(|e| self.render(e))?
        else {
            return Ok(None);
        };
        let origin = &self.inventory.bindings[binding_index];
        let (kind, referent) = if let Some(kind) = fresh_borrow {
            let function = self
                .plan
                .callable_body(&self.plan.program, self.declaration)?;
            let source = root_borrow_origin(function, origin, checker.signatures, budget)
                .map_err(|e| self.render(e))?;
            let checked = &checker.locals[local.0];
            if checked.mutable != source.mutable
                || !replay_type_equal(&checked.ty, &source.ty, budget)
                    .map_err(|e| self.render(e))?
            {
                return Err(self.render(FactsError::invalid(
                    budget,
                    "borrow root type or mutable capability changed",
                )));
            }
            (kind, source.ty)
        } else {
            let ir::Ty::Ref {
                kind: ir::BorrowKind::Shared,
                referent,
            } = &checker.locals[local.0].ty
            else {
                return Ok(None);
            };
            if origin.parameter.is_none()
                && (origin.annotation.is_none() || origin.statement.is_none())
            {
                return Ok(None);
            }
            (ir::BorrowKind::Shared, referent.as_ref().clone())
        };
        require_ordinary_free(&self.plan, declaration, budget).map_err(|e| self.render(e))?;
        let view = self
            .facts
            .original_callable_schema(tr::OriginalCallableSubject::Direct(declaration))
            .map_err(|e| self.render(e))?;
        require_reference_free(&referent, budget, 0).map_err(|e| self.render(e))?;
        let mut materialize = |decl, args: &[ir::Ty]| {
            self.plan
                .intern_nominal(decl, args, ir::FactsCtx::Validated(&self.facts), budget)
                .map_err(|e| source_error(budget, &e))
        };
        let trace = if kind == ir::BorrowKind::Shared {
            sr::infer_shared_identity(&view, &referent, budget, &mut materialize)
        } else {
            sr::infer_reference_identity(&view, kind, &referent, budget, &mut materialize)
        }
        .map_err(|e| self.render(e))?;
        let actual = trace.actual().clone();
        sr::push(
            &mut self.narrow.borrow_mut(),
            RawSharedOccurrence {
                source_index: index,
                binding_index,
                local,
                trace,
            },
            budget,
        )
        .map_err(|e| self.render(e))?;
        Ok(Some(actual))
    }
    fn identity_arguments(&self, args: &[Expr]) -> Result<bool, String> {
        let budget = &self.plan.budget;
        sr::work(budget, self.inventory.calls.len()).map_err(|e| self.render(e))?;
        let Some(index) = self
            .inventory
            .calls
            .iter()
            .position(|site| site.args_pointer == args.as_ptr() as usize)
        else {
            return Ok(false);
        };
        sr::work(budget, self.narrow.borrow().len()).map_err(|e| self.render(e))?;
        Ok(self
            .narrow
            .borrow()
            .iter()
            .any(|row| row.source_index == index))
    }
}

pub(super) fn replay_type_equal(
    left: &ir::Ty,
    right: &ir::Ty,
    budget: &ir::PlanBudget,
) -> Result<bool, FactsError> {
    metered_type_equal(left, right, budget, 0).map_err(|e| source_error(budget, &e))
}
fn source_error(budget: &ir::PlanBudget, message: &str) -> FactsError {
    if budget.has_resource_failure() {
        FactsError::resource(budget, message)
    } else {
        FactsError::invalid(budget, message)
    }
}
fn source_scope_error(
    budget: &ir::PlanBudget,
    error: super::semantic_scope::ScopeError,
) -> FactsError {
    let diagnostic = error.source_error();
    match diagnostic.kind {
        super::source::SourceErrorKind::Limit | super::source::SourceErrorKind::Infrastructure => {
            FactsError::resource(budget, &diagnostic.to_string())
        }
        super::source::SourceErrorKind::Unsupported => {
            FactsError::unsupported(budget, &diagnostic.to_string())
        }
        _ => FactsError::invalid(budget, &diagnostic.to_string()),
    }
}
fn source_name_error(names: &NameStore, budget: &ir::PlanBudget, message: &str) -> FactsError {
    if let Some(failure) = names.budget().snapshot().failure {
        return source_scope_error(
            budget,
            super::semantic_scope::ScopeError::Token(super::token_session::TokenError::Budget(
                failure,
            )),
        );
    }
    source_error(budget, message)
}
fn require_ordinary_free(
    plan: &PlanCtx,
    declaration: ir::DeclId,
    budget: &ir::PlanBudget,
) -> Result<(), FactsError> {
    let info = plan
        .declaration(declaration)
        .map_err(|e| source_error(budget, &e))?;
    let index = plan
        .callable_index(declaration)
        .map_err(|e| source_error(budget, &e))?;
    if info.kind != ir::DeclKind::Function
        || !matches!(
            plan.registry.callable_sources.get(index),
            Some(CallableSource::Free(_))
        )
    {
        return Err(FactsError::unsupported(
            budget,
            "new reference inference requires an ordinary free source function",
        ));
    }
    Ok(())
}
fn require_reference_free(
    ty: &ir::Ty,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<(), FactsError> {
    sr::depth(budget, depth)?;
    sr::work(budget, 1)?;
    match ty {
        ir::Ty::Ref { .. } | ir::Ty::Param(_) => {
            return Err(FactsError::unsupported(
                budget,
                "shared identity referent must be reference-free and Param-free",
            ));
        }
        ir::Ty::Tuple(fields) => {
            for ty in fields.iter() {
                require_reference_free(ty, budget, depth + 1)?;
            }
        }
        ir::Ty::Array { element, .. } => require_reference_free(element, budget, depth + 1)?,
        ir::Ty::Struct(t) => {
            for ty in t.def().type_args.iter() {
                require_reference_free(ty, budget, depth + 1)?;
            }
            for f in t.def().fields.iter() {
                require_reference_free(&f.ty, budget, depth + 1)?;
            }
        }
        ir::Ty::Enum(t) => {
            for ty in t.def().type_args.iter() {
                require_reference_free(ty, budget, depth + 1)?;
            }
            for v in t.def().variants.iter() {
                for f in v.fields.iter() {
                    require_reference_free(&f.ty, budget, depth + 1)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn validate_original_occurrence(
    original: &OriginalSource,
    node: &sr::SourceNode,
    budget: &ir::PlanBudget,
) -> Result<(), FactsError> {
    sr::work(budget, 1)?;
    if node.owner != original.identity {
        return Err(FactsError::invalid(
            budget,
            "retained replay changed source identity",
        ));
    }
    Ok(())
}
pub(super) fn validate_original_context(
    original: &OriginalSource,
    facts: &tr::ValidatedFacts,
    budget: &ir::PlanBudget,
) -> Result<(), FactsError> {
    sr::work(budget, 1)?;
    if !std::ptr::eq(original.facts.as_ref(), facts)
        || !facts.budget().same_authority(budget)
        || !original.plan.budget.same_authority(budget)
        || facts.declaration() != original.declaration
    {
        return Err(FactsError::invalid(
            budget,
            "original source changed caller context",
        ));
    }
    let view = original
        .facts
        .original_callable_schema(tr::OriginalCallableSubject::Direct(original.declaration))?;
    view.validate_for(facts)?;
    if !Arc::ptr_eq(
        facts.world(),
        &original
            .plan
            .trait_world()
            .map_err(|e| source_error(budget, &e))?,
    ) {
        return Err(FactsError::invalid(budget, "original source changed world"));
    }
    Ok(())
}

fn contains_source_ref(
    ty: &ir::Ty,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<bool, FactsError> {
    sr::depth(budget, depth)?;
    sr::work(budget, 1)?;
    Ok(match ty {
        ir::Ty::Ref { .. } => true,
        ir::Ty::Tuple(fields) => {
            let mut any = false;
            for t in fields.iter() {
                any |= contains_source_ref(t, budget, depth + 1)?;
            }
            any
        }
        ir::Ty::Array { element, .. } => contains_source_ref(element, budget, depth + 1)?,
        ir::Ty::Struct(t) => {
            let mut any = false;
            for t in t.def().type_args.iter() {
                any |= contains_source_ref(t, budget, depth + 1)?;
            }
            for f in t.def().fields.iter() {
                any |= contains_source_ref(&f.ty, budget, depth + 1)?;
            }
            any
        }
        ir::Ty::Enum(t) => {
            let mut any = false;
            for t in t.def().type_args.iter() {
                any |= contains_source_ref(t, budget, depth + 1)?;
            }
            for v in t.def().variants.iter() {
                for f in v.fields.iter() {
                    any |= contains_source_ref(&f.ty, budget, depth + 1)?;
                }
            }
            any
        }
        _ => false,
    })
}
fn canonical_root_positions(
    cap: &CanonicalSourceCapability,
    budget: &ir::PlanBudget,
) -> Result<Vec<Option<usize>>, FactsError> {
    let count = cap.inventory.bindings.len();
    sr::work(budget, count)?;
    sr::retain(
        budget,
        count
            .checked_mul(std::mem::size_of::<Option<usize>>())
            .and_then(|n| n.checked_add(32))
            .ok_or_else(|| FactsError::resource(budget, "root occurrence storage overflow"))?,
    )?;
    let mut positions = Vec::new();
    positions
        .try_reserve_exact(count)
        .map_err(|_| FactsError::resource(budget, "root occurrence allocation"))?;
    positions.resize(count, None);
    for (index, position) in cap.emissions.borrow().iter() {
        sr::work(budget, 1)?;
        let site = cap
            .inventory
            .bindings
            .get(*index)
            .ok_or_else(|| FactsError::invalid(budget, "orphan source emission occurrence"))?;
        if !site.required || site.statement.is_none() {
            return Err(FactsError::invalid(
                budget,
                "nonroot source emission retained",
            ));
        }
        if positions[*index].replace(*position).is_some() {
            return Err(FactsError::invalid(
                budget,
                "duplicate source emission occurrence",
            ));
        }
    }
    Ok(positions)
}
fn validate_binding_coverage(
    original: &OriginalSource,
    checked: &ir::CheckedFunction,
    budget: &ir::PlanBudget,
) -> Result<(), FactsError> {
    let count = original
        .bindings
        .len()
        .checked_add(checked.locals.len())
        .ok_or_else(|| FactsError::resource(budget, "binding coverage count overflow"))?;
    sr::retain(
        budget,
        count
            .checked_mul(2)
            .and_then(|n| n.checked_add(64))
            .ok_or_else(|| FactsError::resource(budget, "binding coverage storage overflow"))?,
    )?;
    sr::work(budget, count)?;
    if original.root_positions.len() != original.bindings.len() {
        return Err(FactsError::invalid(
            budget,
            "root source position table length changed",
        ));
    }
    let context = original
        .plan
        .callable_context(original.declaration)
        .map_err(|e| source_error(budget, &e))?;
    let offset = usize::from(context.receiver != ReceiverKind::None);
    let mut seen = Vec::new();
    seen.try_reserve_exact(original.bindings.len())
        .map_err(|_| FactsError::resource(budget, "source binding coverage allocation"))?;
    seen.resize(original.bindings.len(), false);
    let mut locals = Vec::new();
    locals
        .try_reserve_exact(checked.locals.len())
        .map_err(|_| FactsError::resource(budget, "checked binding coverage allocation"))?;
    locals.resize(checked.locals.len(), false);
    for (index, local) in &original.binding_mapping {
        sr::work(budget, 1)?;
        let source = original
            .bindings
            .get(*index)
            .ok_or_else(|| FactsError::invalid(budget, "orphan source binding index"))?;
        if !source.required || std::mem::replace(&mut seen[*index], true) {
            return Err(FactsError::invalid(
                budget,
                "duplicate or unclassified source binding mapping",
            ));
        }
        if let Some(parameter) = source.parameter {
            if checked.params.get(
                parameter
                    .checked_add(offset)
                    .ok_or_else(|| FactsError::resource(budget, "parameter position overflow"))?,
            ) != Some(local)
                || original.root_positions[*index].is_some()
            {
                return Err(FactsError::invalid(
                    budget,
                    "original parameter changed checked position",
                ));
            }
        } else {
            if source.statement.is_none() {
                return Err(FactsError::invalid(
                    budget,
                    "nonroot binding retained authority",
                ));
            }
            let position = original.root_positions[*index]
                .ok_or_else(|| FactsError::invalid(budget, "root binding lost checked position"))?;
            let matched = match checked.body.statements.get(position) {
                Some(ir::Stmt::Let { local: actual, .. }) => !source.is_declare && actual == local,
                Some(ir::Stmt::Declare { local: actual }) => source.is_declare && actual == local,
                _ => false,
            };
            if !matched {
                return Err(FactsError::invalid(
                    budget,
                    "original root statement changed checked local",
                ));
            }
        }
        let slot = locals.get_mut(local.0).ok_or_else(|| {
            FactsError::invalid(budget, "source binding maps outside checked locals")
        })?;
        if std::mem::replace(slot, true) {
            return Err(FactsError::invalid(
                budget,
                "two source bindings map to one checked local",
            ));
        }
    }
    for (index, (site, present)) in original.bindings.iter().zip(seen).enumerate() {
        if !site.required && original.root_positions[index].is_some() {
            return Err(FactsError::invalid(
                budget,
                "nonorigin root position retained",
            ));
        }
        sr::work(budget, 1)?;
        if site.required != present {
            return Err(FactsError::invalid(
                budget,
                "dead or live original binding lacks correspondence",
            ));
        }
    }
    Ok(())
}
fn mapped_source_binding(
    original: &OriginalSource,
    index: usize,
    budget: &ir::PlanBudget,
) -> Result<ir::LocalId, FactsError> {
    sr::work(budget, original.binding_mapping.len())?;
    let mut found = None;
    for (source, local) in &original.binding_mapping {
        if *source == index && found.replace(*local).is_some() {
            return Err(FactsError::invalid(
                budget,
                "duplicate source binding correspondence",
            ));
        }
    }
    found.ok_or_else(|| FactsError::invalid(budget, "missing source binding correspondence"))
}
fn source_binding_type(
    original: &OriginalSource,
    index: usize,
    checked: &ir::CheckedFunction,
    signatures: &Signatures,
    budget: &ir::PlanBudget,
) -> Result<(ir::LocalId, ir::Ty), FactsError> {
    let site = original
        .bindings
        .get(index)
        .ok_or_else(|| FactsError::invalid(budget, "source binding index outside original body"))?;
    let local = mapped_source_binding(original, index, budget)?;
    let caller = original
        .facts
        .original_callable_schema(tr::OriginalCallableSubject::Direct(original.declaration))?;
    let ty = if let Some(parameter) = site.parameter {
        let context = original
            .plan
            .callable_context(original.declaration)
            .map_err(|e| source_error(budget, &e))?;
        let position = parameter + usize::from(context.receiver != ReceiverKind::None);
        if checked.params.get(position) != Some(&local) {
            return Err(FactsError::invalid(
                budget,
                "source parameter changed checked position",
            ));
        }
        caller
            .signature()
            .params
            .get(position)
            .ok_or_else(|| FactsError::invalid(budget, "source parameter outside caller schema"))?
            .clone()
    } else {
        if site.statement.is_none() {
            return Err(FactsError::unsupported(
                budget,
                "shared binding requires a direct annotated local",
            ));
        }
        let statement = original.root_positions[index].ok_or_else(|| {
            FactsError::invalid(budget, "shared source lost root checked position")
        })?;
        if site.annotation.is_none() {
            return Err(FactsError::unsupported(
                budget,
                "inferred-local source binding is outside shared replay",
            ));
        }
        let Some(ir::Stmt::Let { local: actual, .. }) = checked.body.statements.get(statement)
        else {
            return Err(FactsError::unsupported(
                budget,
                "shared local requires direct source statement correspondence",
            ));
        };
        if *actual != local {
            return Err(FactsError::invalid(
                budget,
                "source annotation mapped to sibling local",
            ));
        }
        checked_local_ty(site.annotation.as_ref().unwrap(), signatures)
            .map_err(|e| source_error(budget, &e))?
    };
    let actual = checked
        .locals
        .get(local.0)
        .ok_or_else(|| FactsError::invalid(budget, "source binding outside checked locals"))?;
    if !metered_type_equal(&actual.ty, &ty, budget, 0)
        .map_err(|e| FactsError::resource(budget, &e))?
    {
        return Err(FactsError::invalid(
            budget,
            "source binding type changed after checking",
        ));
    }
    Ok((local, ty))
}

enum RootBorrowLiteral {
    Int(ir::IntValue),
    Bool(bool),
    Char(char),
}
struct RootBorrowOrigin {
    ty: ir::Ty,
    mutable: bool,
    literal: Option<RootBorrowLiteral>,
}
/// Fresh-borrow inference is deliberately limited to scalar root bindings whose
/// type is independently recoverable from the immutable source, not the checked
/// local arena. Nested references, projections and general initializer inference
/// stay outside this frontier.
fn root_borrow_origin(
    function: &Function,
    site: &SourceBindingSite,
    signatures: &Signatures,
    budget: &ir::PlanBudget,
) -> Result<RootBorrowOrigin, FactsError> {
    sr::work(budget, 1)?;
    let origin = if let Some(parameter) = site.parameter {
        let index = u32::try_from(parameter)
            .map_err(|_| FactsError::resource(budget, "borrow parameter index overflow"))?;
        if site.node.path.as_ref() != [0, index] {
            return Err(FactsError::invalid(
                budget,
                "borrow parameter occurrence changed",
            ));
        }
        let (_, ty) = function
            .params
            .get(parameter)
            .ok_or_else(|| FactsError::invalid(budget, "borrow parameter outside source"))?;
        RootBorrowOrigin {
            ty: checked_local_ty(ty, signatures).map_err(|e| source_error(budget, &e))?,
            mutable: false,
            literal: None,
        }
    } else {
        let statement = site.statement.ok_or_else(|| {
            FactsError::unsupported(budget, "fresh borrow requires a root source binding")
        })?;
        let index = u32::try_from(statement)
            .map_err(|_| FactsError::resource(budget, "borrow statement index overflow"))?;
        if site.node.path.as_ref() != [1, 0, 2, index] {
            return Err(FactsError::invalid(
                budget,
                "borrow root statement occurrence changed",
            ));
        }
        let Some(Stmt::Let {
            ty, mutable, value, ..
        }) = function.body.statements.get(statement)
        else {
            return Err(FactsError::unsupported(
                budget,
                "fresh borrow requires a direct source let",
            ));
        };
        let (ty, literal) = if !matches!(ty, Type::Infer) {
            (
                checked_local_ty(ty, signatures).map_err(|e| source_error(budget, &e))?,
                None,
            )
        } else {
            match value {
                Expr::Integer(literal) if literal.suffix.is_some() => {
                    let value = resolve_integer(literal, false, None)
                        .map_err(|e| source_error(budget, &e))?;
                    (ir::Ty::Int(value.ty), Some(RootBorrowLiteral::Int(value)))
                }
                Expr::Bool(value) => (ir::Ty::Bool, Some(RootBorrowLiteral::Bool(*value))),
                Expr::Char(value) => (ir::Ty::Char, Some(RootBorrowLiteral::Char(*value))),
                _ => {
                    return Err(FactsError::unsupported(
                        budget,
                        "fresh borrow root requires an annotation or suffixed integer/Bool/char literal",
                    ));
                }
            }
        };
        RootBorrowOrigin {
            ty,
            mutable: *mutable,
            literal,
        }
    };
    if !matches!(origin.ty, ir::Ty::Int(_) | ir::Ty::Bool | ir::Ty::Char) {
        return Err(FactsError::unsupported(
            budget,
            "fresh borrow inference requires an integer, Bool or char root",
        ));
    }
    Ok(origin)
}

fn source_root_borrow_type(
    original: &OriginalSource,
    index: usize,
    checked: &ir::CheckedFunction,
    signatures: &Signatures,
    budget: &ir::PlanBudget,
) -> Result<(ir::LocalId, ir::Ty), FactsError> {
    let site = original
        .bindings
        .get(index)
        .ok_or_else(|| FactsError::invalid(budget, "borrow binding outside source"))?;
    let function = original
        .plan
        .callable_body(&original.plan.program, original.declaration)
        .map_err(|e| source_error(budget, &e))?;
    let origin = root_borrow_origin(function, site, signatures, budget)?;
    let local = mapped_source_binding(original, index, budget)?;
    let actual = checked
        .locals
        .get(local.0)
        .ok_or_else(|| FactsError::invalid(budget, "borrow root outside checked locals"))?;
    if actual.mutable != origin.mutable || !replay_type_equal(&actual.ty, &origin.ty, budget)? {
        return Err(FactsError::invalid(
            budget,
            "borrow root type or mutable capability changed",
        ));
    }
    if let Some(literal) = origin.literal {
        let position = original.root_positions[index]
            .ok_or_else(|| FactsError::invalid(budget, "borrow literal lost root statement"))?;
        let Some(ir::Stmt::Let {
            local: bound,
            value,
        }) = checked.body.statements.get(position)
        else {
            return Err(FactsError::invalid(
                budget,
                "borrow literal changed source statement",
            ));
        };
        if *bound != local {
            return Err(FactsError::invalid(
                budget,
                "borrow literal mapped to sibling local",
            ));
        }
        let value = checked.exprs.get(value.0).ok_or_else(|| {
            FactsError::invalid(budget, "borrow literal outside checked expressions")
        })?;
        let same = match (literal, &value.kind) {
            (RootBorrowLiteral::Int(expected), ir::ExprKind::Int(actual)) => expected == *actual,
            (RootBorrowLiteral::Bool(expected), ir::ExprKind::Bool(actual)) => expected == *actual,
            (RootBorrowLiteral::Char(expected), ir::ExprKind::Char(actual)) => {
                expected as u32 == actual.value
            }
            _ => false,
        };
        if !same || !replay_type_equal(&value.ty, &origin.ty, budget)? {
            return Err(FactsError::invalid(
                budget,
                "borrow source literal initializer changed",
            ));
        }
    }
    Ok((local, origin.ty))
}

/// Replay every integer intrinsic, including unused arena rows, against its
/// separately retained source candidate and exact typed receiver ID.
/// This correspondence helper does not reconstruct arbitrary receiver ASTs.
/// CanonicalSourcePublication seals the immutable checked-body Arc: its
/// validate_source boundary rejects replaced bodies, including copy-on-write
/// edits to a receiver's contents that preserve its ID and type.
fn replay_builtin_leading_zeros(
    original: &OriginalSource,
    checked: &ir::CheckedFunction,
    ordinary: &[Option<usize>],
    seen: &mut [bool],
    budget: &ir::PlanBudget,
) -> Result<Vec<bool>, FactsError> {
    sr::work(budget, original.calls.len())?;
    sr::retain(budget, original.calls.len())?;
    let mut sources = Vec::new();
    sources
        .try_reserve_exact(original.calls.len())
        .map_err(|_| FactsError::resource(budget, "leading zeros occurrence allocation"))?;
    sources.resize(original.calls.len(), false);
    let mut previous: Option<(usize, ir::ExprId)> = None;
    for row in &original.builtin_leading_zeros_mapping {
        sr::work(budget, 1)?;
        let site = original
            .calls
            .get(row.source_index)
            .ok_or_else(|| FactsError::invalid(budget, "orphan leading zeros occurrence"))?;
        if !site.leading_zeros_candidate || site.ordinary.is_some() || site.application.is_some() {
            return Err(FactsError::invalid(
                budget,
                "leading zeros mapped from non-candidate",
            ));
        }
        if std::mem::replace(&mut sources[row.source_index], true)
            || ordinary.get(row.source_index).copied().flatten().is_some()
        {
            return Err(FactsError::invalid(
                budget,
                "duplicate leading zeros correspondence",
            ));
        }
        if let Some((index, expr)) = previous {
            let prior = &original.calls[index].node.path;
            let current = &site.node.path;
            sr::work(budget, prior.len().saturating_add(current.len()))?;
            // Inventory is preorder, emission is postorder: a receiver call
            // precedes its enclosing method, but independent sites stay ordered.
            let encloses_prior = prior.len() > current.len() && prior.starts_with(current);
            let prior_encloses = current.len() > prior.len() && current.starts_with(prior);
            if row.expr.0 <= expr.0
                || prior_encloses
                || (row.source_index <= index && !encloses_prior)
            {
                return Err(FactsError::invalid(
                    budget,
                    "leading zeros source order changed",
                ));
            }
        }
        previous = Some((row.source_index, row.expr));
        let result = checked
            .exprs
            .get(row.expr.0)
            .ok_or_else(|| FactsError::invalid(budget, "leading zeros outside checked arena"))?;
        let ir::ExprKind::LeadingZeros { value } = result.kind else {
            return Err(FactsError::invalid(
                budget,
                "leading zeros mapped to different operation",
            ));
        };
        if value != row.receiver
            || result.ty != ir::Ty::Int(ir::IntTy::U32)
            || !checked.exprs.get(value.0).is_some_and(|receiver| {
                matches!(receiver.ty, ir::Ty::Int(_)) && receiver.completion == result.completion
            })
        {
            return Err(FactsError::invalid(
                budget,
                "leading zeros receiver, type or completion changed",
            ));
        }
        if std::mem::replace(&mut seen[row.expr.0], true) {
            return Err(FactsError::invalid(
                budget,
                "leading zeros mapped from two sources",
            ));
        }
    }
    sr::work(budget, checked.exprs.len())?;
    for (index, expr) in checked.exprs.iter().enumerate() {
        if matches!(expr.kind, ir::ExprKind::LeadingZeros { .. }) && !seen[index] {
            return Err(FactsError::invalid(budget, "orphan checked leading zeros"));
        }
    }
    Ok(sources)
}

/// Exact replay of typed primitive-method occurrences, including unused rows.
fn replay_builtin_intrinsics(
    original: &OriginalSource,
    checked: &ir::CheckedFunction,
    ordinary: &[Option<usize>],
    seen: &mut [bool],
    budget: &ir::PlanBudget,
) -> Result<Vec<bool>, FactsError> {
    sr::work(budget, original.calls.len())?;
    sr::retain(budget, original.calls.len())?;
    let mut sources = Vec::new();
    sources
        .try_reserve_exact(original.calls.len())
        .map_err(|_| FactsError::resource(budget, "primitive method occurrence allocation"))?;
    sources.resize(original.calls.len(), false);
    let mut previous: Option<(usize, ir::ExprId)> = None;
    for row in &original.builtin_intrinsic_mapping {
        sr::work(budget, 1)?;
        let site = original
            .calls
            .get(row.source_index)
            .ok_or_else(|| FactsError::invalid(budget, "orphan primitive method occurrence"))?;
        if !site.intrinsic_candidate || site.ordinary.is_some() || site.application.is_some() {
            return Err(FactsError::invalid(
                budget,
                "primitive method mapped from a non-candidate source call",
            ));
        }
        if std::mem::replace(&mut sources[row.source_index], true)
            || ordinary.get(row.source_index).copied().flatten().is_some()
        {
            return Err(FactsError::invalid(
                budget,
                "duplicate primitive method correspondence",
            ));
        }
        if let Some((index, expr)) = previous {
            let prior = &original.calls[index].node.path;
            let current = &site.node.path;
            sr::work(budget, prior.len().saturating_add(current.len()))?;
            // Inventory is preorder, emission is postorder: receiver and
            // argument calls precede their enclosing method.
            let encloses_prior = prior.len() > current.len() && prior.starts_with(current);
            let prior_encloses = current.len() > prior.len() && current.starts_with(prior);
            if row.expr.0 <= expr.0
                || prior_encloses
                || (row.source_index <= index && !encloses_prior)
            {
                return Err(FactsError::invalid(
                    budget,
                    "primitive method source order changed",
                ));
            }
        }
        previous = Some((row.source_index, row.expr));
        let result = checked
            .exprs
            .get(row.expr.0)
            .ok_or_else(|| FactsError::invalid(budget, "primitive method outside checked arena"))?;
        let ir::ExprKind::Intrinsic { intrinsic, args } = &result.kind else {
            return Err(FactsError::invalid(
                budget,
                "primitive method mapped to a different operation",
            ));
        };
        if args.first() != Some(&row.receiver)
            || intrinsic.signature().map(|s| s.result).as_ref() != Some(&result.ty)
        {
            return Err(FactsError::invalid(
                budget,
                "primitive method receiver or type changed",
            ));
        }
        if std::mem::replace(&mut seen[row.expr.0], true) {
            return Err(FactsError::invalid(
                budget,
                "checked primitive method mapped from two source occurrences",
            ));
        }
    }
    sr::work(budget, checked.exprs.len())?;
    for (index, expr) in checked.exprs.iter().enumerate() {
        if matches!(expr.kind, ir::ExprKind::Intrinsic { .. }) && !seen[index] {
            return Err(FactsError::invalid(
                budget,
                "orphan checked primitive method lacks original occurrence",
            ));
        }
    }
    Ok(sources)
}

/// Exact replay of typed builtin `.len()` occurrences and their receivers.
fn replay_builtin_lens(
    original: &OriginalSource,
    checked: &ir::CheckedFunction,
    ordinary: &[Option<usize>],
    seen: &mut [bool],
    budget: &ir::PlanBudget,
) -> Result<Vec<bool>, FactsError> {
    sr::work(budget, original.calls.len())?;
    sr::retain(budget, original.calls.len())?;
    let mut sources = Vec::new();
    sources
        .try_reserve_exact(original.calls.len())
        .map_err(|_| FactsError::resource(budget, "builtin occurrence allocation"))?;
    sources.resize(original.calls.len(), false);
    let mut previous: Option<(usize, ir::ExprId)> = None;
    for row in &original.builtin_len_mapping {
        sr::work(budget, 1)?;
        let site = original
            .calls
            .get(row.source_index)
            .ok_or_else(|| FactsError::invalid(budget, "orphan builtin length occurrence"))?;
        if !site.len_candidate || site.ordinary.is_some() || site.application.is_some() {
            return Err(FactsError::invalid(
                budget,
                "builtin length mapped from a non-candidate source call",
            ));
        }
        if std::mem::replace(&mut sources[row.source_index], true)
            || ordinary.get(row.source_index).copied().flatten().is_some()
        {
            return Err(FactsError::invalid(
                budget,
                "duplicate builtin length correspondence",
            ));
        }
        if let Some((index, expr)) = previous
            && (row.source_index <= index || row.expr.0 <= expr.0)
        {
            return Err(FactsError::invalid(
                budget,
                "builtin length occurrences changed source order",
            ));
        }
        previous = Some((row.source_index, row.expr));
        let value = checked
            .exprs
            .get(row.expr.0)
            .ok_or_else(|| FactsError::invalid(budget, "builtin length outside checked arena"))?;
        let ir::ExprKind::SliceLen { slice } = value.kind else {
            return Err(FactsError::invalid(
                budget,
                "builtin length mapped to a different operation",
            ));
        };
        if slice != row.receiver
            || value.ty != ir::Ty::Int(ir::IntTy::Usize)
            || !checked
                .exprs
                .get(slice.0)
                .is_some_and(|receiver| receiver.ty.shared_descriptor_element().is_some())
        {
            return Err(FactsError::invalid(
                budget,
                "builtin length receiver or type changed",
            ));
        }
        if std::mem::replace(&mut seen[row.expr.0], true) {
            return Err(FactsError::invalid(
                budget,
                "checked builtin length mapped from two source occurrences",
            ));
        }
    }
    sr::work(budget, checked.exprs.len())?;
    for (index, expr) in checked.exprs.iter().enumerate() {
        if matches!(expr.kind, ir::ExprKind::SliceLen { .. }) && !seen[index] {
            return Err(FactsError::invalid(
                budget,
                "orphan checked builtin length lacks original occurrence",
            ));
        }
    }
    Ok(sources)
}

/// Exact replay of ArrayToSlice wrappers: each checked wrapper is recorded once
/// with its exact operand, so a replaced wrapped call or operand is rejected.
fn replay_unsizes(
    original: &OriginalSource,
    checked: &ir::CheckedFunction,
    budget: &ir::PlanBudget,
) -> Result<(), FactsError> {
    sr::work(budget, checked.exprs.len())?;
    sr::retain(budget, checked.exprs.len())?;
    let mut covered = Vec::new();
    covered
        .try_reserve_exact(checked.exprs.len())
        .map_err(|_| FactsError::resource(budget, "unsize coverage allocation"))?;
    covered.resize(checked.exprs.len(), false);
    for row in &original.unsize_mapping {
        sr::work(budget, 1)?;
        let value = checked
            .exprs
            .get(row.wrapper.0)
            .ok_or_else(|| FactsError::invalid(budget, "unsize wrapper outside checked arena"))?;
        if !matches!(value.kind, ir::ExprKind::ArrayToSlice { reference } if reference == row.reference)
        {
            return Err(FactsError::invalid(
                budget,
                "array-to-slice wrapper changed its wrapped operand",
            ));
        }
        if std::mem::replace(&mut covered[row.wrapper.0], true) {
            return Err(FactsError::invalid(
                budget,
                "duplicate array-to-slice wrapper correspondence",
            ));
        }
    }
    for (index, expr) in checked.exprs.iter().enumerate() {
        if matches!(expr.kind, ir::ExprKind::ArrayToSlice { .. }) && !covered[index] {
            return Err(FactsError::invalid(budget, "orphan array-to-slice wrapper"));
        }
    }
    Ok(())
}

fn optional_source_path_equal(
    left: &Option<AstPath>,
    right: &Option<AstPath>,
    names: &NameStore,
) -> Result<bool, String> {
    let left = left
        .as_ref()
        .map(|path| name_scope::path_key(names, path, Namespace::Value))
        .transpose()?;
    let right = right
        .as_ref()
        .map(|path| name_scope::path_key(names, path, Namespace::Value))
        .transpose()?;
    Ok(left == right)
}
fn optional_source_type_equal(
    left: &Option<Type>,
    right: &Option<Type>,
    names: &NameStore,
) -> Result<bool, String> {
    for ty in left.iter().chain(right.iter()) {
        ty.semantic_eq(ty, names)?;
    }
    match (left, right) {
        (Some(a), Some(b)) => a.semantic_eq(b, names),
        (None, None) => Ok(true),
        _ => Ok(false),
    }
}
fn optional_application_equal(
    left: &Option<GenericPath>,
    right: &Option<GenericPath>,
    names: &NameStore,
) -> Result<bool, String> {
    for path in left.iter().chain(right.iter()) {
        path.checked_path(names)
            .map_err(native_adapter::name_diagnostic)?;
        for segment in &path.segments {
            for binding in &segment.associated_types {
                names
                    .key(binding.name, LookupClass::Member)
                    .map_err(native_adapter::name_diagnostic)?;
            }
            for ty in segment.args.iter().chain(
                segment
                    .associated_types
                    .iter()
                    .map(|binding| &binding.value),
            ) {
                ty.semantic_eq(ty, names)?;
            }
        }
    }
    let (Some(left), Some(right)) = (left, right) else {
        return Ok(left.is_none() && right.is_none());
    };
    if left.anchor != right.anchor || left.segments.len() != right.segments.len() {
        return Ok(false);
    }
    for (a, b) in left.segments.iter().zip(&right.segments) {
        if !name_scope::ident_eq(names, a.name, b.name, LookupClass::Item(Namespace::Type))?
            || a.args.len() != b.args.len()
            || a.associated_types.len() != b.associated_types.len()
        {
            return Ok(false);
        }
        for (a, b) in a.associated_types.iter().zip(&b.associated_types) {
            if !name_scope::ident_eq(names, a.name, b.name, LookupClass::Member)?
                || !a.value.semantic_eq(&b.value, names)?
            {
                return Ok(false);
            }
        }
        for (a, b) in a.args.iter().zip(&b.args) {
            if !a.semantic_eq(b, names)? {
                return Ok(false);
            }
        }
    }
    Ok(true)
}
/// Validate retained source correspondences, not arbitrary AST equivalence.
/// The private publication factory checks its own canonical checked body and
/// seals that immutable Arc. Consumers must also use the publication's
/// validate_source boundary, rather than treating this helper as permission to
/// replace an expression's contents in an independently supplied checked body.
pub(super) fn replay_source_body(
    original: &OriginalSource,
    checked: &Arc<ir::CheckedFunction>,
    facts: &Arc<tr::ValidatedFacts>,
    budget: &ir::PlanBudget,
) -> Result<Vec<sr::NarrowValidatedCall>, FactsError> {
    validate_original_context(original, facts, budget)?;
    if checked.body_substitution.is_some() {
        return Err(FactsError::invalid(
            budget,
            "original source is a substituted payload",
        ));
    }
    let function = original
        .plan
        .callable_body(&original.plan.program, original.declaration)
        .map_err(|e| source_error(budget, &e))?;
    let fresh = SourceInventory::build(
        &original.plan.program,
        function,
        original.identity.clone(),
        budget,
    )?;
    if fresh.calls.len() != original.calls.len() || fresh.bindings.len() != original.bindings.len()
    {
        return Err(FactsError::invalid(
            budget,
            "source occurrence inventory changed",
        ));
    }
    for (fresh, retained) in fresh.calls.iter().zip(&original.calls) {
        compare_call_site_work(fresh, budget)?;
        compare_call_site_work(retained, budget)?;
        sr::work(budget, 1)?;
        if fresh.node != retained.node
            || fresh.leading_zeros_candidate != retained.leading_zeros_candidate
            || fresh.len_candidate != retained.len_candidate
            || fresh.intrinsic_candidate != retained.intrinsic_candidate
            || !optional_source_path_equal(
                &fresh.ordinary,
                &retained.ordinary,
                &original.plan.program.names,
            )
            .map_err(|e| source_error(budget, &e))?
            || !optional_application_equal(
                &fresh.application,
                &retained.application,
                &original.plan.program.names,
            )
            .map_err(|e| source_error(budget, &e))?
            || fresh.argument_binding != retained.argument_binding
            || fresh.argument_borrow != retained.argument_borrow
            || fresh.direct_let != retained.direct_let
            || retained.args_pointer != 0
        {
            return Err(FactsError::invalid(
                budget,
                "retained source call or parent context changed",
            ));
        }
    }
    for (fresh, retained) in fresh.bindings.iter().zip(&original.bindings) {
        sr::work(budget, fresh.node.path.len())?;
        sr::work(budget, retained.node.path.len())?;
        sr::work(budget, 1)?;
        if let Some(ty) = &fresh.annotation {
            compare_source_type_work(ty, budget, 0)?;
        }
        if let Some(ty) = &retained.annotation {
            compare_source_type_work(ty, budget, 0)?;
        }
        if fresh.node != retained.node
            || !optional_source_type_equal(
                &fresh.annotation,
                &retained.annotation,
                &original.plan.program.names,
            )
            .map_err(|e| source_error(budget, &e))?
            || fresh.parameter != retained.parameter
            || fresh.statement != retained.statement
            || fresh.required != retained.required
            || fresh.is_declare != retained.is_declare
        {
            return Err(FactsError::invalid(
                budget,
                "retained source binding changed",
            ));
        }
    }
    sr::retain(
        budget,
        checked
            .exprs
            .len()
            .checked_mul(2)
            .ok_or_else(|| FactsError::resource(budget, "source call coverage overflow"))?,
    )?;
    let mut seen = Vec::new();
    seen.try_reserve_exact(checked.exprs.len())
        .map_err(|_| FactsError::resource(budget, "source call coverage allocation"))?;
    seen.resize(checked.exprs.len(), false);
    validate_binding_coverage(original, checked, budget)?;
    let mut decisions = Vec::new();
    let mut signatures = original
        .plan
        .environment(Some(original.declaration))
        .map_err(|e| source_error(budget, &e))?;
    signatures.facts = Some(facts.clone());
    let caller_context = original
        .plan
        .callable_context(original.declaration)
        .map_err(|e| source_error(budget, &e))?;
    sr::retain(
        budget,
        original
            .calls
            .len()
            .checked_mul(std::mem::size_of::<Option<usize>>())
            .ok_or_else(|| FactsError::resource(budget, "source call position storage overflow"))?,
    )?;
    sr::work(budget, original.calls.len())?;
    let mut positions = Vec::new();
    positions
        .try_reserve_exact(original.calls.len())
        .map_err(|_| FactsError::resource(budget, "source call position allocation"))?;
    positions.resize(original.calls.len(), None);
    for (position, (index, _, _)) in original.call_mapping.iter().enumerate() {
        sr::work(budget, 1)?;
        let slot = positions
            .get_mut(*index)
            .ok_or_else(|| FactsError::invalid(budget, "orphan original call index"))?;
        if slot.replace(position).is_some() {
            return Err(FactsError::invalid(
                budget,
                "duplicate original call correspondence",
            ));
        }
    }
    sr::retain(
        budget,
        checked
            .exprs
            .len()
            .checked_mul(std::mem::size_of::<Option<usize>>())
            .ok_or_else(|| FactsError::resource(budget, "source evidence index overflow"))?,
    )?;
    sr::work(budget, checked.exprs.len())?;
    let mut evidence_positions = Vec::new();
    evidence_positions
        .try_reserve_exact(checked.exprs.len())
        .map_err(|_| FactsError::resource(budget, "source evidence index allocation"))?;
    evidence_positions.resize(checked.exprs.len(), None);
    for (position, row) in checked.call_evidence.iter().enumerate() {
        sr::work(budget, 1)?;
        let slot = evidence_positions
            .get_mut(row.call.0)
            .ok_or_else(|| FactsError::invalid(budget, "source evidence outside checked arena"))?;
        if slot.replace(position).is_some() {
            return Err(FactsError::invalid(
                budget,
                "duplicate source call evidence",
            ));
        }
    }
    let leading_zero_sources =
        replay_builtin_leading_zeros(original, checked, &positions, &mut seen, budget)?;
    let builtin_sources = replay_builtin_lens(original, checked, &positions, &mut seen, budget)?;
    let intrinsic_sources =
        replay_builtin_intrinsics(original, checked, &positions, &mut seen, budget)?;
    replay_unsizes(original, checked, budget)?;
    for (index, site) in original.calls.iter().enumerate() {
        sr::work(budget, 1)?;
        if builtin_sources[index] || leading_zero_sources[index] || intrinsic_sources[index] {
            continue;
        }
        let position = positions[index].ok_or_else(|| {
            FactsError::invalid(budget, "dead or live original call lacks correspondence")
        })?;
        let (_, id, original_target) = &original.call_mapping[position];
        let id = *id;
        let slot = seen
            .get_mut(id.0)
            .ok_or_else(|| FactsError::invalid(budget, "original call outside checked arena"))?;
        if std::mem::replace(slot, true) {
            return Err(FactsError::invalid(
                budget,
                "checked call mapped from two source occurrences",
            ));
        }
        let ir::ExprKind::Call {
            target,
            args,
            signature,
            ..
        } = &checked.exprs[id.0].kind
        else {
            return Err(FactsError::invalid(
                budget,
                "original call mapped to non-call",
            ));
        };
        validate_original_callee(original_target, target, budget)?;
        let Some(name) = &site.ordinary else {
            // Source provider/selection role is frozen before publication; no ordinary omitted-call exemption.
            ir::validate_callee(target, signature, ir::FactsCtx::Validated(facts))
                .map_err(|e| source_error(budget, &e))?;
            if let ir::Callee::TraitItem(call) = target
                && facts.uses_equality_required_call(call)?
            {
                // This is the exact required declaration in the actual caller
                // environment, not a selected implementation's source schema.
                let view =
                    facts.original_callable_schema(tr::OriginalCallableSubject::TraitItem(call))?;
                view.validate_for(facts)?;
                let required = view.signature();
                let mut equal = required.params.len() == signature.params.len();
                for (required, actual) in required.params.iter().zip(&signature.params) {
                    equal &= replay_type_equal(required, actual, budget)?;
                }
                equal &= replay_type_equal(&required.result, &signature.result, budget)?;
                sr::work(
                    budget,
                    required
                        .ref_contract
                        .validation_work(&required.params, &required.result)
                        .map_err(|error| source_error(budget, &error))?,
                )?;
                sr::work(
                    budget,
                    signature
                        .ref_contract
                        .validation_work(&signature.params, &signature.result)
                        .map_err(|error| source_error(budget, &error))?,
                )?;
                equal &= required.ref_contract == signature.ref_contract;
                if !equal {
                    return Err(FactsError::invalid(
                        budget,
                        "original required call changed its equality-normalized source signature",
                    ));
                }
            }
            continue;
        };
        let expected_decl = *original
            .plan
            .registry
            .functions
            .get(
                &name_scope::path_key(original.plan.program.names.as_ref(), name, Namespace::Value)
                    .map_err(|e| source_error(budget, &e))?,
            )
            .ok_or_else(|| FactsError::invalid(budget, "original call name outside registry"))?;
        let ir::Callee::Direct(target) = target else {
            return Err(FactsError::invalid(
                budget,
                "ordinary source call changed canonical role",
            ));
        };
        if target.declaration != expected_decl {
            return Err(FactsError::invalid(
                budget,
                "ordinary source call changed declaration",
            ));
        }
        let view =
            facts.original_callable_schema(tr::OriginalCallableSubject::Direct(expected_decl))?;
        view.validate_for(facts)?;
        if let Some(path) = &site.application {
            let checker = Checker::new(&signatures, ir::Ty::Unit, caller_context.module);
            checker
                .push_application(path, Namespace::Value)
                .map_err(|e| source_error(budget, &e))?;
            let application = checker.applications.borrow_mut().pop().ok_or_else(|| {
                FactsError::invalid(budget, "explicit source application vanished")
            })?;
            if application.1.len() != target.type_args.len() {
                return Err(FactsError::invalid(
                    budget,
                    "explicit source actual count changed",
                ));
            }
            for (original, actual) in application.1.iter().zip(target.type_args.iter()) {
                if !metered_type_equal(original, actual, budget, 0)
                    .map_err(|e| FactsError::resource(budget, &e))?
                {
                    return Err(FactsError::invalid(
                        budget,
                        "explicit source actual term changed",
                    ));
                }
            }
            let evidence = &checked.call_evidence[evidence_positions[id.0].ok_or_else(|| {
                FactsError::invalid(budget, "explicit source call missing old trace")
            })?];
            let mut rows = Vec::new();
            for row in application.2.iter() {
                sr::push(&mut rows, row.clone(), budget)?;
            }
            for (argument, ty) in application.1.iter().enumerate() {
                let locations =
                    ir::carrier_locations(ty, budget).map_err(|e| source_error(budget, &e))?;
                for location in locations.iter() {
                    if !matches!(location.channel, ir::CarrierChannel::Fixed(_)) {
                        continue;
                    }
                    let mut present = false;
                    for row in &rows {
                        sr::work(budget, row.location.path.0.len())?;
                        sr::work(budget, location.path.0.len())?;
                        if row.argument == argument && row.location == *location {
                            present = true;
                            break;
                        }
                    }
                    if !present {
                        sr::push(
                            &mut rows,
                            ir::TypeActualRegion {
                                argument,
                                location: location.clone(),
                                source: ir::ActualRegion::Elided,
                            },
                            budget,
                        )?;
                    }
                }
            }
            if rows.len() != evidence.type_actual_regions.len() {
                return Err(FactsError::invalid(
                    budget,
                    "explicit source region count changed",
                ));
            }
            for (row, actual) in rows.iter().zip(evidence.type_actual_regions.iter()) {
                sr::work(budget, row.location.path.0.len())?;
                sr::work(budget, actual.location.path.0.len())?;
                sr::work(budget, 1)?;
                if actual != row {
                    return Err(FactsError::invalid(
                        budget,
                        "explicit source region occurrence changed",
                    ));
                }
            }
            continue;
        }
        let ir::Callee::Direct(original_target) = original_target else {
            return Err(FactsError::invalid(
                budget,
                "ordinary original target lost Direct role",
            ));
        };
        let mut new_reference = false;
        for actual in original_target.type_args.iter() {
            new_reference |= contains_source_ref(actual, budget, 0)?;
        }
        if !new_reference {
            continue;
        }
        require_ordinary_free(&original.plan, expected_decl, budget)?;
        let parent = site.direct_let.as_ref().ok_or_else(|| {
            FactsError::unsupported(
                budget,
                "inferred reference requires direct unannotated let context",
            )
        })?;
        let binding = site.argument_binding.as_ref().ok_or_else(|| {
            FactsError::unsupported(
                budget,
                "inferred reference requires original shared local Var",
            )
        })?;
        let binding_index = source_binding_index(&original.bindings, binding, budget)?
            .ok_or_else(|| FactsError::invalid(budget, "original argument binding missing"))?;
        let (local, kind, referent, ty) = if let Some(kind) = site.argument_borrow {
            let (local, referent) =
                source_root_borrow_type(original, binding_index, checked, &signatures, budget)?;
            sr::retain(budget, std::mem::size_of::<ir::Ty>() + 32)?;
            let ty = ir::Ty::Ref {
                kind,
                referent: Arc::new(referent.clone()),
            };
            (local, kind, referent, ty)
        } else {
            let (local, ty) =
                source_binding_type(original, binding_index, checked, &signatures, budget)?;
            let ir::Ty::Ref {
                kind: ir::BorrowKind::Shared,
                referent,
            } = &ty
            else {
                return Err(FactsError::unsupported(
                    budget,
                    "mutable inferred reference Var remains unsupported",
                ));
            };
            (local, ir::BorrowKind::Shared, referent.as_ref().clone(), ty)
        };
        require_reference_free(&referent, budget, 0)?;
        let mut materialize = |decl, args: &[ir::Ty]| {
            original
                .plan
                .intern_nominal(decl, args, ir::FactsCtx::Validated(facts), budget)
                .map_err(|e| source_error(budget, &e))
        };
        let trace = if kind == ir::BorrowKind::Shared {
            sr::infer_shared_identity(&view, &referent, budget, &mut materialize)
        } else {
            sr::infer_reference_identity(&view, kind, &referent, budget, &mut materialize)
        }?;
        sr::work(budget, original.narrow.len())?;
        let mut retained = None;
        for raw in &original.narrow {
            if raw.source_index == index && retained.replace(raw).is_some() {
                return Err(FactsError::invalid(
                    budget,
                    "duplicate original shared frontier",
                ));
            }
        }
        let retained = retained.ok_or_else(|| {
            FactsError::invalid(budget, "new reference lacks original shared frontier")
        })?;
        if retained.binding_index != binding_index || retained.local != local {
            return Err(FactsError::invalid(
                budget,
                "shared frontier changed original binding identity",
            ));
        }
        retained.trace.validate_replayed(&trace, budget)?;
        let actual = trace.actual();
        if target.type_args.len() != 1 || !replay_type_equal(&target.type_args[0], actual, budget)?
        {
            return Err(FactsError::invalid(
                budget,
                "replayed actual differs from canonical target",
            ));
        }
        if args.len() != 1 {
            return Err(FactsError::invalid(
                budget,
                "shared replay argument count changed",
            ));
        }
        let value = checked
            .exprs
            .get(args[0].0)
            .ok_or_else(|| FactsError::invalid(budget, "shared replay argument outside arena"))?;
        let exact_argument = if let Some(expected) = site.argument_borrow {
            matches!(&value.kind, ir::ExprKind::Borrow { kind, place, timing: ir::BorrowTiming::Immediate }
                if *kind == expected && place.local == local && place.projection.is_empty())
        } else {
            matches!(value.kind, ir::ExprKind::Local(actual) if actual == local)
        };
        if !exact_argument || !replay_type_equal(&value.ty, &ty, budget)? {
            return Err(FactsError::invalid(
                budget,
                "reference identity argument changed original borrow kind, place or Var",
            ));
        }
        let output_index = source_binding_index(&original.bindings, parent, budget)?
            .ok_or_else(|| FactsError::invalid(budget, "original initializer parent missing"))?;
        let output = mapped_source_binding(original, output_index, budget)?;
        let statement = original.root_positions[output_index].ok_or_else(|| {
            FactsError::invalid(
                budget,
                "shared initializer parent is not a direct statement",
            )
        })?;
        if original.bindings[output_index].annotation.is_some() {
            return Err(FactsError::invalid(
                budget,
                "shared absent context forged from annotation",
            ));
        }
        if !matches!(checked.body.statements.get(statement),Some(ir::Stmt::Let{local,value}) if *local==output && *value==id)
        {
            return Err(FactsError::invalid(
                budget,
                "original initializer changed checked let occurrence",
            ));
        }
        if signature.params.len() != 1
            || !replay_type_equal(&signature.params[0], actual, budget)?
            || !replay_type_equal(&signature.result, actual, budget)?
        {
            return Err(FactsError::invalid(
                budget,
                "shared canonical signature changed",
            ));
        }
        let evidence = &checked.call_evidence[evidence_positions[id.0]
            .ok_or_else(|| FactsError::invalid(budget, "new shared call missing trace"))?];
        sr::work(budget, evidence.type_actual_regions.len())?;
        sr::work(budget, 1)?;
        if evidence.arguments.len() != 1
            || evidence.arguments[0].argument != 0
            || evidence.arguments[0].value != args[0]
            || evidence.arguments[0].expectation != ir::CallExpectation::NoFirmReference
            || evidence
                .type_actual_regions
                .iter()
                .any(|r| r.source != ir::ActualRegion::Elided)
        {
            return Err(FactsError::invalid(
                budget,
                "shared original trace or fresh elision changed",
            ));
        }
        ir::FactsCtx::Validated(facts)
            .validate_ty_with_budget(actual, budget)
            .map_err(|e| source_error(budget, &e))?;
        validate_argument(&referent, ir::FactsCtx::Validated(facts), budget, 0)
            .map_err(|e| source_error(budget, &e))?;
        sr::push(
            &mut decisions,
            sr::NarrowValidatedCall {
                call: id,
                source: site.node.clone(),
                trace,
            },
            budget,
        )?;
    }
    if original.narrow.len() != decisions.len() {
        return Err(FactsError::invalid(
            budget,
            "unused or missing original shared frontier",
        ));
    }
    if original
        .call_mapping
        .len()
        .checked_add(original.builtin_len_mapping.len())
        .and_then(|n| n.checked_add(original.builtin_intrinsic_mapping.len()))
        .and_then(|n| n.checked_add(original.builtin_leading_zeros_mapping.len()))
        != Some(original.calls.len())
    {
        return Err(FactsError::invalid(budget, "orphan original call mapping"));
    }
    for (index, expr) in checked.exprs.iter().enumerate() {
        sr::work(budget, 1)?;
        if matches!(expr.kind, ir::ExprKind::Call { .. }) && !seen[index] {
            return Err(FactsError::invalid(
                budget,
                "orphan checked call lacks original occurrence",
            ));
        }
    }
    Ok(decisions)
}

impl PlanCtx {
    pub(super) fn publish_callable_body(
        &self,
        declaration: ir::DeclId,
        budget: &ir::PlanBudget,
    ) -> Result<PublishedCallableBody, FactsError> {
        self.check_budget(budget)
            .map_err(|e| source_error(budget, &e))?;
        let env = self
            .callable_env(declaration)
            .map_err(|e| source_error(budget, &e))?;
        match self
            .callable_provider(declaration)
            .map_err(|e| source_error(budget, &e))?
        {
            CallableProvider::Recipe { .. } => {
                let count = env.total_arity();
                budget
                    .charge_nodes(count)
                    .map_err(|e| FactsError::resource(budget, &e))?;
                sr::work(budget, count)?;
                sr::retain(
                    budget,
                    count
                        .checked_mul(2 * std::mem::size_of::<ir::Ty>())
                        .and_then(|n| n.checked_add(32))
                        .ok_or_else(|| {
                            FactsError::resource(budget, "recipe symbolic target overflow")
                        })?,
                )?;
                let mut actuals = Vec::new();
                actuals.try_reserve_exact(count).map_err(|_| {
                    FactsError::resource(budget, "recipe symbolic target allocation")
                })?;
                actuals.extend(env.ordered_params().map(ir::Ty::Param));
                sr::work(budget, count)?;
                let target = ir::CallTarget {
                    declaration,
                    type_args: actuals.into(),
                };
                let facts = self
                    .callable_facts(declaration)
                    .map_err(|e| source_error(budget, &e))?;
                let checked = self
                    .checked_body(&target, &facts, budget)
                    .map_err(|e| source_error(budget, &e))?;
                sr::retain(budget, std::mem::size_of::<ir::CheckedFunction>() + 32)?;
                Ok(PublishedCallableBody::Recipe(Arc::new(checked)))
            }
            CallableProvider::UnavailableProvided { .. } => Err(FactsError::unsupported(
                budget,
                "unavailable canonical body",
            )),
            CallableProvider::Source { .. } => {
                let facts = if env.is_generic() {
                    self.callable_facts(declaration)
                } else {
                    self.instance_facts(&ir::CallTarget {
                        declaration,
                        type_args: Arc::from([]),
                    })
                }
                .map_err(|e| source_error(budget, &e))?;
                let function = self
                    .callable_body(&self.program, declaration)
                    .map_err(|e| source_error(budget, &e))?;
                let identity = sr::OriginalBodyId::new(budget)?;
                let inventory =
                    SourceInventory::build(&self.program, function, identity.clone(), budget)?;
                let mut cap = CanonicalSourceCapability {
                    plan: self.clone(),
                    declaration,
                    facts: facts.clone(),
                    inventory,
                    calls: Default::default(),
                    builtin_leading_zeros: Default::default(),
                    builtin_lens: Default::default(),
                    builtin_intrinsics: Default::default(),
                    unsizes: Default::default(),
                    bindings: Default::default(),
                    narrow: Default::default(),
                    emissions: Default::default(),
                    error: Default::default(),
                };
                let checked = match type_callable_source(
                    &self.program,
                    declaration,
                    self,
                    budget,
                    Some(&cap),
                ) {
                    Ok(checked) => checked,
                    Err(message) => {
                        return Err(if budget.has_resource_failure() {
                            source_error(budget, &message)
                        } else {
                            cap.error
                                .borrow_mut()
                                .take()
                                .unwrap_or_else(|| source_error(budget, &message))
                        });
                    }
                };
                let root_positions = canonical_root_positions(&cap, budget)?;
                // Move precharged source-only records. Never clone the AST or output arena.
                let mut calls = std::mem::take(&mut cap.inventory.calls);
                for call in &mut calls {
                    call.args_pointer = 0;
                }
                let source = OriginalSource {
                    plan: self.clone(),
                    declaration,
                    facts: facts.clone(),
                    identity,
                    calls,
                    bindings: std::mem::take(&mut cap.inventory.bindings),
                    call_mapping: cap.calls.take(),
                    builtin_leading_zeros_mapping: cap.builtin_leading_zeros.take(),
                    builtin_len_mapping: cap.builtin_lens.take(),
                    builtin_intrinsic_mapping: cap.builtin_intrinsics.take(),
                    unsize_mapping: cap.unsizes.take(),
                    binding_mapping: cap.bindings.take(),
                    narrow: cap.narrow.take(),
                    root_positions,
                };
                let original = sr::OriginalBody::from_source(&cap, source, budget)?;
                let mut cx = self.clone();
                let final_payload =
                    ir::canonicalize_source_calls(&checked, &facts, &mut cx, &cap, budget)?;
                sr::retain(budget, std::mem::size_of::<ir::CheckedFunction>() + 32)?;
                let source = Arc::new(final_payload);
                let publication =
                    sr::publish_canonical_source(cap, source, original, facts, budget)?;
                Ok(PublishedCallableBody::Source(publication))
            }
        }
    }
}

#[cfg(test)]
mod associated_bounds;
#[cfg(test)]
mod leading_zero_tests;
#[cfg(test)]
mod name_tests;
mod recipes;
#[cfg(test)]
mod slice_tests;
pub use tr::CallableProvider;

#[derive(Clone, Debug)]
pub struct Program {
    pub names: Arc<NameStore>,
    pub modules: Arc<ModuleArena>,
    pub symbols: Arc<ProgramSymbols>,
    pub scopes: Option<Arc<super::semantic_scope::ScopeArena>>,
    pub functions: Vec<Function>,
    pub structs: Vec<StructDecl>,
    pub enums: Vec<EnumDecl>,
    pub impls: Vec<ImplDecl>,
    pub traits: Vec<TraitDecl>,
    pub trait_imports: Vec<(ModuleId, TraitPath)>,
    // Resolver-issued source authority. Shared unchanged by public AST clones.
    associated_bound_sources: Arc<[AssociatedBoundSource]>,
}
#[derive(Clone, Debug)]
struct AssociatedBoundSource {
    trait_index: usize,
    member_index: usize,
    module: ModuleId,
    trait_name: AstPath,
    member_name: AstIdent,
    bounds: Arc<[(GenericPath, BuiltinTrait)]>,
}
/// Syntax-only result. Resolve this once before publishing callable signatures.
#[derive(Clone, Debug)]
pub struct ParsedProgram {
    pub program: Program,
    items: HashMap<PathKey, Vec<Item>>,
    empty_use_prefixes: Vec<(ModuleId, AstPath)>,
}
/// Parser construction state has no detached or placeholder name store. The
/// final program is published only after its one mutable owner is frozen.
#[derive(Default)]
struct ProgramDeclarations {
    functions: Vec<Function>,
    structs: Vec<StructDecl>,
    enums: Vec<EnumDecl>,
    impls: Vec<ImplDecl>,
    traits: Vec<TraitDecl>,
    trait_imports: Vec<(ModuleId, TraitPath)>,
}
struct ParsedDeclarations {
    program: ProgramDeclarations,
    items: HashMap<PathKey, Vec<Item>>,
    empty_use_prefixes: Vec<(ModuleId, AstPath)>,
}
impl ProgramDeclarations {
    fn publish(
        self,
        names: Arc<NameStore>,
        modules: Arc<ModuleArena>,
        symbols: Arc<ProgramSymbols>,
        scopes: Option<Arc<super::semantic_scope::ScopeArena>>,
    ) -> Program {
        Program {
            names,
            modules,
            symbols,
            scopes,
            functions: self.functions,
            structs: self.structs,
            enums: self.enums,
            impls: self.impls,
            traits: self.traits,
            trait_imports: self.trait_imports,
            associated_bound_sources: Arc::from([]),
        }
    }
}
/// Explicit structural construction for fixtures and native AST producers. All
/// nodes must use identifiers issued by this one owner before publication.
pub struct ProgramBuilder {
    names: NameStoreBuilder,
    modules: ModuleArenaBuilder,
    symbols: Arc<ProgramSymbols>,
    fixture_anchor: super::expansion_span::SpanId,
    pub functions: Vec<Function>,
    pub structs: Vec<StructDecl>,
    pub enums: Vec<EnumDecl>,
    pub impls: Vec<ImplDecl>,
    pub traits: Vec<TraitDecl>,
    pub trait_imports: Vec<(ModuleId, TraitPath)>,
}
impl ProgramBuilder {
    pub fn new(
        name: &std::path::Path,
        source: &str,
        budget: &super::expansion_budget::ExpansionBudget,
    ) -> Result<Self, String> {
        let input = super::source::PreparedLexInput::identity(name, source, budget)
            .map_err(|error| error.to_string())?;
        let mut session =
            super::token_session::TokenSession::new(input.snapshot().clone(), budget.clone())
                .map_err(|error| error.to_string())?;
        let fixture_anchor = session
            .original_span(input.eof().map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
        let mut names = NameStoreBuilder::new(session, super::token::Edition::E2024)
            .map_err(|error| error.to_string())?;
        let symbols = ProgramSymbols::new(&mut names).map_err(native_adapter::name_diagnostic)?;
        let modules = ModuleArenaBuilder::for_source(&names).map_err(module_diagnostic)?;
        Ok(Self {
            names,
            modules,
            symbols,
            fixture_anchor,
            functions: Vec::new(),
            structs: Vec::new(),
            enums: Vec::new(),
            impls: Vec::new(),
            traits: Vec::new(),
            trait_imports: Vec::new(),
        })
    }
    pub fn names(&self) -> &NameStoreBuilder {
        &self.names
    }
    pub fn modules(&self) -> &ModuleArenaBuilder {
        &self.modules
    }
    pub fn module_root(&self) -> Result<ModuleId, String> {
        self.modules.source_root().map_err(module_diagnostic)
    }
    pub fn module_child(&mut self, parent: ModuleId, name: AstIdent) -> Result<ModuleId, String> {
        self.modules
            .intern_child(&self.names, parent, name)
            .map_err(module_diagnostic)
    }
    pub fn ident(&mut self, text: &str) -> Result<AstIdent, String> {
        self.names
            .intern_root(text, self.fixture_anchor)
            .map_err(native_adapter::name_diagnostic)
    }
    /// Fixture grammar only. The source parser never prints/relexes identifiers
    /// or uses this structural text factory to recover semantic authority.
    pub fn path(&mut self, text: &str) -> Result<AstPath, String> {
        let mut parts = text.split("::").peekable();
        let mut anchor = PathAnchor::Relative;
        match parts.peek().copied() {
            Some("crate") => {
                anchor = PathAnchor::Crate;
                parts.next();
            }
            Some("self") => {
                anchor = PathAnchor::SelfModule;
                parts.next();
            }
            Some("Self") => {
                anchor = PathAnchor::SelfType;
                parts.next();
            }
            Some("super") => {
                let mut levels = 0u16;
                while parts.peek() == Some(&"super") {
                    parts.next();
                    levels = levels
                        .checked_add(1)
                        .ok_or("fixture path ancestor overflow")?;
                }
                anchor = PathAnchor::Super(levels);
            }
            _ => {}
        }
        let budget = self.names.budget().clone();
        let mut segments = super::token_session::BudgetVec::new(
            &budget,
            super::expansion_budget::Storage::Temporary,
        )
        .map_err(|error| error.to_string())?;
        for part in parts {
            segments
                .push(self.ident(part)?)
                .map_err(|error| error.to_string())?;
        }
        AstPath::new(&self.names, anchor, segments.as_slice()).map_err(|error| error.to_string())
    }
    pub fn finish(self) -> Result<Program, String> {
        self.modules
            .check_names(&self.names)
            .map_err(module_diagnostic)?;
        let modules = self.modules.freeze().map_err(module_diagnostic)?;
        let names = self
            .names
            .freeze()
            .map_err(native_adapter::name_diagnostic)?;
        let program = ProgramDeclarations {
            functions: self.functions,
            structs: self.structs,
            enums: self.enums,
            impls: self.impls,
            traits: self.traits,
            trait_imports: self.trait_imports,
        }
        .publish(names, modules, self.symbols, None);
        program.validate_name_handles()?;
        Ok(program)
    }
}
impl Program {
    fn validate_name_handles(&self) -> Result<(), String> {
        name_validation::validate_program_names(self)
    }
    fn validate_associated_bound_source(
        &self,
        source: &AssociatedBoundSource,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        budget.charge_work(1)?;
        self.modules
            .check(source.module)
            .map_err(module_diagnostic)?;
        let owner = self
            .traits
            .get(source.trait_index)
            .ok_or("associated bound source owner was removed")?;
        let member = owner
            .associated_types
            .get(source.member_index)
            .ok_or("associated bound source member was removed")?;
        let mut equal = owner.module == source.module;
        equal &= exact_source_path(&self.names, &owner.name, &source.trait_name, budget)?;
        equal &= exact_source_ident(&self.names, member.name, source.member_name)?;
        equal &= member.bounds.len() == source.bounds.len();
        budget.charge_work(member.bounds.len().saturating_add(source.bounds.len()))?;
        for (path, (original, _)) in member.bounds.iter().zip(source.bounds.iter()) {
            equal &= match path {
                TraitPath::Source(path) => {
                    exact_source_generic_path(&self.names, path, original, budget, 0)?
                }
                TraitPath::Builtin(_) => false,
            };
        }
        if !equal {
            return Err(
                "associated declaration bounds differ from their exact retained source occurrences"
                    .into(),
            );
        }
        Ok(())
    }
    fn validate_associated_bound_sources(&self, budget: &ir::PlanBudget) -> Result<(), String> {
        let mut count = 0usize;
        let mut has_bounds = false;
        for owner in &self.traits {
            budget.charge_work(owner.associated_types.len().saturating_add(1))?;
            count = count
                .checked_add(owner.associated_types.len())
                .ok_or("associated bound source count overflow")?;
            for member in &owner.associated_types {
                has_bounds |= !member.bounds.is_empty();
            }
        }
        // Structural no-bound fixtures do not need resolver certificates.
        if self.associated_bound_sources.is_empty() && !has_bounds {
            return Ok(());
        }
        for source in self.associated_bound_sources.iter() {
            self.validate_associated_bound_source(source, budget)?;
        }
        if count != self.associated_bound_sources.len() {
            return Err("associated declaration source inventory changed".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
struct Item {
    path: AstPath,
    module: ModuleId,
    visibility: ir::Visibility,
    kind: ItemKind,
}
#[derive(Clone, Debug)]
enum ItemKind {
    TypeAlias(TypeAliasDecl),
    Module(ModuleId),
    Function,
    Struct,
    Enum,
    Trait,
    Variant(ir::VariantShape),
    Import(AstPath, bool), // target path, type-namespace-only (`prefix::{self}`)
}

#[derive(Clone, Debug)]
struct TypeAliasDecl {
    params: Vec<AstIdent>,
    target: Type,
}

/// Declaration qualification only. `Const` does not imply an available CTFE engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Constness {
    Runtime,
    Const,
}
/// Wildcards never enter a lexical binding map or acquire a semantic name key.
#[derive(Clone, Copy, Debug)]
pub enum BindingName {
    Named(AstIdent),
    Wildcard,
}
/// Builtin lifetime syntax is distinct from a user-declared lifetime binder.
#[derive(Clone, Copy, Debug)]
pub enum RegionName {
    Named(AstIdent),
    Static,
    Infer,
}
/// Tuple positions are structural indices, not decimal identifier spellings.
#[derive(Clone, Copy, Debug)]
pub enum FieldName {
    Named(AstIdent),
    Tuple(usize),
}
#[derive(Clone, Debug)]
pub struct Function {
    pub lexical_scope: Option<super::semantic_scope::ScopeId>,
    pub module: ModuleId,
    pub constness: Constness,
    pub visibility: ir::Visibility,
    pub bounds: Vec<TypeBound>,
    pub name: AstPath,
    pub regions: RegionDecls,
    pub type_params: Vec<AstIdent>,
    pub params: Vec<(BindingName, Type)>,
    /// Per-parameter `mut` binding mode, parallel to `params`. Empty means all
    /// parameters are immutable (synthetic and test-built functions).
    pub param_mut: Vec<bool>,
    pub result: Type,
    /// Trailing `type_params` minted for argument-position `impl Trait`.
    /// They are never nameable by explicit generic arguments (E0107/E0632).
    pub impl_params: usize,
    pub body: Block,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiverKind {
    None,
    Value { mutable: bool },
    Borrowed { kind: ir::BorrowKind },
}
#[derive(Clone, Debug)]
pub struct ImplDecl {
    pub trait_path: Option<TraitPath>,
    pub bounds: Vec<TypeBound>,
    pub module: ModuleId,
    pub type_params: Vec<AstIdent>,
    pub self_ty: Type,
    pub items: Vec<AssociatedFunction>,
    pub associated_types: Vec<AssociatedTypeValue>,
}
/// A written trait-impl assignment, not a callable or a new generic binder.
#[derive(Clone, Debug)]
pub struct AssociatedTypeValue {
    pub name: AstIdent,
    pub value: Type,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuiltinTrait {
    Sized,
    Copy,
    Clone,
    Drop,
}
#[derive(Clone, Debug)]
pub enum TraitPath {
    Source(GenericPath),
    Builtin(BuiltinTrait),
}
#[derive(Clone, Debug)]
pub struct TypeBound {
    pub parameter: AstIdent,
    pub traits: Vec<TraitPath>,
}
#[derive(Clone, Debug)]
pub struct TraitDecl {
    pub module: ModuleId,
    pub type_params: Vec<AstIdent>,
    pub bounds: Vec<TypeBound>,
    pub supertraits: Vec<TraitPath>,
    pub name: AstPath,
    pub visibility: ir::Visibility,
    pub items: Vec<TraitItemDecl>,
    pub associated_types: Vec<AssociatedTypeDecl>,
}
/// A genuine type member owned by its declaring trait.
#[derive(Clone, Debug)]
pub struct AssociatedTypeDecl {
    pub name: AstIdent,
    pub bounds: Vec<TraitPath>,
}
#[derive(Clone, Debug)]
pub struct TraitItemDecl {
    pub signature: FunctionHead,
    pub receiver: ReceiverKind,
    /// Provided (default) body. It is a separate callable declaration owned by
    /// the trait, with the trait Self binder as its first scope.
    pub default: Option<Function>,
}
#[derive(Clone, Debug)]
pub struct FunctionHead {
    pub constness: Constness,
    pub name: AstIdent,
    pub regions: RegionDecls,
    pub type_params: Vec<AstIdent>,
    pub bounds: Vec<TypeBound>,
    pub params: Vec<(BindingName, Type)>,
    pub param_mut: Vec<bool>,
    pub result: Type,
    /// Trailing `type_params` minted for argument-position `impl Trait`.
    pub impl_params: usize,
}
#[derive(Clone, Debug)]
pub struct AssociatedFunction {
    pub function: Function,
    pub receiver: ReceiverKind,
    pub visibility: ir::Visibility,
}
#[derive(Clone, Debug)]
pub struct CallableContext {
    pub module: ModuleId,
    pub self_ty: Option<ir::Ty>,
    pub associated_owner: Option<ir::DeclId>,
    pub receiver: ReceiverKind,
}
#[derive(Clone, Debug)]
pub struct AssociatedSemanticOwner {
    pub trait_decl: Option<ir::DeclId>,
    pub trait_item: Option<ir::DeclId>,
    pub nominal: ir::DeclId,
    pub impl_decl: ir::DeclId,
    pub self_pattern: ir::Ty,
    pub receiver: ReceiverKind,
}
#[derive(Clone, Debug, Default)]
pub struct RegionDecls {
    pub receiver: Option<RegionName>,
    pub params: Vec<AstIdent>,
    pub outlives: Vec<(RegionName, RegionName)>,
}
#[derive(Clone, Debug)]
pub struct StructDecl {
    pub shape: StructShape,
    pub module: ModuleId,
    pub type_params: Vec<AstIdent>,
    pub name: AstPath,
    pub visibility: ir::Visibility,
    pub fields: Vec<FieldDecl>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StructShape {
    Record,
    Unit,
    Tuple,
}
#[derive(Clone, Debug)]
pub struct EnumDecl {
    pub module: ModuleId,
    pub type_params: Vec<AstIdent>,
    pub name: AstPath,
    pub visibility: ir::Visibility,
    pub variants: Vec<VariantDecl>,
}
#[derive(Clone, Debug)]
pub struct VariantDecl {
    pub name: AstIdent,
    pub shape: ir::VariantShape,
    pub fields: Vec<FieldDecl>,
}
#[derive(Clone, Debug)]
pub struct FieldDecl {
    pub name: FieldName,
    pub ty: Type,
    pub visibility: ir::Visibility,
}
#[derive(Clone, Debug)]
pub enum FieldSelector {
    Deref,
    ConstantIndex(usize),
    Tuple(usize),
    Named(AstIdent),
}
/// Structured source path. Arguments belong to their declaration segment.
#[derive(Clone, Debug)]
pub struct GenericPath {
    pub anchor: PathAnchor,
    pub segments: Vec<PathSegment>,
    /// Certified canonical target, including its checked original-use provenance.
    pub(crate) resolved_use: Option<AstPath>,
}
#[derive(Clone, Debug)]
pub struct PathSegment {
    pub name: AstIdent,
    pub args: Vec<Type>,
    pub associated_types: Vec<AssociatedTypeBinding>,
}
/// A genuine member occurrence and its written equality RHS, not a positional actual.
#[derive(Clone, Debug)]
pub struct AssociatedTypeBinding {
    pub name: AstIdent,
    pub value: Type,
}
impl GenericPath {
    fn path(&self, names: &impl NameAccess) -> Result<AstPath, String> {
        self.checked_path(names)
            .map_err(native_adapter::name_diagnostic)
    }
    fn checked_path(
        &self,
        names: &impl NameAccess,
    ) -> Result<AstPath, super::semantic_name::NameError> {
        let mut segments = super::token_session::BudgetVec::new(
            names.budget(),
            super::expansion_budget::Storage::Temporary,
        )?;
        for segment in &self.segments {
            segments.push(segment.name)?;
        }
        let target = AstPath::new(names, self.anchor, segments.as_slice())?;
        match &self.resolved_use {
            Some(certificate) => {
                certificate.check_target(names, &target)?;
                Ok(certificate.clone())
            }
            None => Ok(target),
        }
    }
    fn application(&self) -> Result<(AstIdent, &[Type]), String> {
        let mut applications = self.segments.iter().filter(|s| !s.args.is_empty());
        let segment = applications
            .next()
            .ok_or("missing explicit generic arguments")?;
        if applications.next().is_some() {
            return Err("multiple generic declaration segments in a path are unsupported".into());
        }
        Ok((segment.name, &segment.args))
    }
}
#[derive(Clone, Debug)]
pub enum Type {
    SelfType,
    Apply(GenericPath),
    /// Exact written projection syntax, never a fabricated parameter or alias.
    Projection(Box<SourceProjection>),
    Param(AstIdent),
    Ref {
        kind: ir::BorrowKind,
        region: Option<RegionName>,
        referent: Box<Type>,
    },
    Array(Box<Type>, usize),
    /// Unsized `[T]`. Admitted values require a checked shared reference.
    Slice(Box<Type>),
    /// `*const T` (Shared) or `*mut T` (Mut).
    RawPtr {
        kind: ir::BorrowKind,
        pointee: Box<Type>,
    },
    Int(ir::IntTy),
    Float(ir::FloatTy),
    Char,
    /// Unsized `str`; admitted only behind a shared reference.
    Str,
    Bool,
    Unit,
    Tuple(Vec<Type>),
    Named(AstPath),
    /// Semantic-only never type, not an admitted source type annotation.
    Never,
    /// Unannotated locals only, resolved independently during function checking.
    Infer,
}
/// Boxed as a whole by `Type`, keeping ordinary expression stack frames small.
#[derive(Clone, Debug)]
pub enum SourceProjection {
    Qualified {
        self_ty: Type,
        trait_path: TraitPath,
        item: AstIdent,
    },
    /// The exact owning trait comes from the retained declaration context.
    /// No source trait path is fabricated for this syntax.
    SelfItem { item: AstIdent },
}
#[derive(Clone, Debug)]
pub struct Block {
    pub statements: Vec<Stmt>,
    pub tail: Option<Box<Expr>>,
}
#[derive(Clone, Copy, Debug)]
pub enum LocalName {
    Named(AstIdent),
    Receiver,
}
#[derive(Clone, Debug)]
pub struct Place {
    pub local: LocalName,
    pub fields: Vec<FieldSelector>,
}
#[derive(Clone, Debug)]
pub enum Pattern {
    Apply {
        path: GenericPath,
        pattern: Box<Pattern>,
    },
    Variant {
        name: AstPath,
        shape: ir::VariantShape,
        fields: Vec<(FieldSelector, Pattern)>,
        rest: bool,
    },
    Path(AstPath),
    Bool(bool),
    Int {
        literal: IntegerLiteral,
        negative: bool,
    },
    Char(char),
    /// Scalar range pattern. Missing bounds are the domain extreme; `inclusive`
    /// applies to the upper bound only and requires one when set.
    Range {
        lo: Option<RangeBound>,
        hi: Option<RangeBound>,
        inclusive: bool,
    },
    Wildcard,
    Binding {
        name: AstIdent,
        mutable: bool,
        /// Explicit `ref`/`ref mut` binding mode. Default binding modes
        /// through references are inferred by the checker, not the parser.
        by_ref: Option<ir::BorrowKind>,
    },
    /// Explicit `&pat` / `&mut pat` reference pattern.
    Ref {
        kind: ir::BorrowKind,
        pattern: Box<Pattern>,
    },
    Tuple(Vec<Pattern>),
    TupleRest(PositionalRest),
    Array(Vec<Pattern>),
    ArrayRest(PositionalRest),
    VariantRest {
        name: AstPath,
        fields: PositionalRest,
    },
    Struct {
        name: AstPath,
        fields: Vec<(AstIdent, Pattern)>,
        rest: bool,
    },
}
/// One literal endpoint of a range pattern.
#[derive(Clone, Debug)]
pub enum RangeBound {
    Int {
        literal: IntegerLiteral,
        negative: bool,
    },
    Char(char),
    /// `<integer type>::MIN` or `::MAX`, written as a path endpoint.
    IntLimit {
        ty: ir::IntTy,
        max: bool,
    },
}
/// Only patterns actually written around one positional `..` occurrence.
/// Omitted fields have no syntax nodes, identifiers, or lexical bindings.
#[derive(Clone, Debug)]
pub struct PositionalRest {
    pub prefix: Vec<Pattern>,
    pub suffix: Vec<Pattern>,
}
pub(super) struct PositionalProjection<'a> {
    prefix: &'a [Pattern],
    suffix: &'a [Pattern],
    suffix_start: usize,
}
impl PositionalRest {
    /// Positional arithmetic only. Callers must validate actual type/schema
    /// arity and precharge traversal work before using these borrowed children.
    pub(super) fn project(&self, arity: usize) -> Result<PositionalProjection<'_>, String> {
        let suffix_start = arity
            .checked_sub(self.suffix.len())
            .ok_or("positional rest pattern has too many fields")?;
        if self.prefix.len() > suffix_start {
            return Err("positional rest pattern has too many fields".into());
        }
        Ok(PositionalProjection {
            prefix: &self.prefix,
            suffix: &self.suffix,
            suffix_start,
        })
    }
}
impl<'a> PositionalProjection<'a> {
    pub(super) fn iter(&self) -> impl Iterator<Item = (usize, &'a Pattern)> + '_ {
        self.prefix.iter().enumerate().chain(
            self.suffix
                .iter()
                .enumerate()
                .map(|(index, pattern)| (self.suffix_start + index, pattern)),
        )
    }
}
#[derive(Clone, Debug)]
pub enum Stmt {
    Declare {
        name: BindingName,
        ty: Type,
        mutable: bool,
    },
    Let {
        name: BindingName,
        mutable: bool,
        ty: Type,
        value: Expr,
    },
    LetPattern {
        pattern: Pattern,
        ty: Type,
        value: Expr,
    },
    Assign {
        name: LocalName,
        value: Expr,
    },
    AssignDeref {
        carrier: Expr,
        value: Expr,
    },
    AssignIndex {
        array: Place,
        index: Expr,
        value: Expr,
    },
    AssignOp {
        place: Place,
        op: BinaryOp,
        value: Expr,
    },
    AssignIndexOp {
        array: Place,
        index: Expr,
        op: BinaryOp,
        value: Expr,
    },
    AssignPlace {
        place: Place,
        value: Expr,
    },
    Expr(Expr),
    /// A blocklike statement without an explicit value-discarding semicolon.
    ImplicitExpr(Expr),
    Return(Option<Expr>),
    While {
        label: Option<AstIdent>,
        condition: Expr,
        body: Block,
    },
    WhileLet {
        label: Option<AstIdent>,
        pattern: Box<Pattern>,
        scrutinee: Box<Expr>,
        body: Block,
        /// The actual optional token following the body, not inferred liveness.
        has_semicolon: bool,
    },
    /// Genuine `for PAT in ITER BODY`. Range syntax and zero-argument
    /// iterator adapters are retained as written structure, never as
    /// fabricated method calls.
    For {
        label: Option<AstIdent>,
        pattern: Box<Pattern>,
        source: Box<ForSource>,
        body: Block,
        has_semicolon: bool,
    },
    LetElse {
        pattern: Box<Pattern>,
        ty: Type,
        value: Box<Expr>,
        else_block: Block,
    },
}
/// The written iterable of a for loop.
#[derive(Clone, Debug)]
pub enum ForIterable {
    /// `start..end` or `start..=end`, evaluated once, start first.
    Range {
        start: Expr,
        end: Expr,
        inclusive: bool,
    },
    /// Any other expression; its type selects the iteration plan.
    Value(Expr),
}
/// Zero-argument adapters written after the iterable, in source order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForAdapter {
    Iter,
    IntoIter,
    Rev,
    Enumerate,
}
#[derive(Clone, Debug)]
pub struct ForSource {
    pub iterable: ForIterable,
    pub adapters: Vec<ForAdapter>,
}
/// Source literal magnitude, before contextual signedness and range checking.
#[derive(Clone, Debug)]
pub struct IntegerLiteral {
    pub magnitude: u128,
    pub suffix: Option<ir::IntTy>,
}

fn integer_type(name: &str) -> Option<ir::IntTy> {
    use ir::IntTy::*;
    Some(match name {
        "i8" => I8,
        "i16" => I16,
        "i32" => I32,
        "i128" => I128,
        "i64" => I64,
        "isize" => Isize,
        "u8" => U8,
        "u16" => U16,
        "u32" => U32,
        "u128" => U128,
        "u64" => U64,
        "usize" => Usize,
        _ => return None,
    })
}

fn float_type(name: &str) -> Option<ir::FloatTy> {
    match name {
        "f32" => Some(ir::FloatTy::F32),
        "f64" => Some(ir::FloatTy::F64),
        _ => None,
    }
}

/// A decimal float literal, or an integer-looking literal with a float suffix.
/// The decimal text (underscores removed) is retained so rounding happens
/// exactly once, directly to the resolved width. Rounding to f64 first and
/// then to f32 would double-round some literals.
#[derive(Clone, Debug, PartialEq)]
pub struct FloatLiteral {
    pub digits: String,
    pub suffix: Option<ir::FloatTy>,
}

impl FloatLiteral {
    pub fn value(&self, ty: ir::FloatTy, negative: bool) -> Result<ir::FloatValue, String> {
        let mut value = match ty {
            ir::FloatTy::F64 => ir::FloatValue::f64(
                self.digits
                    .parse::<f64>()
                    .map_err(|_| "invalid float literal")?,
            ),
            ir::FloatTy::F32 => ir::FloatValue::f32(
                self.digits
                    .parse::<f32>()
                    .map_err(|_| "invalid float literal")?,
            ),
        };
        let finite = match ty {
            ir::FloatTy::F32 => f32::from_bits(value.bits as u32).is_finite(),
            ir::FloatTy::F64 => f64::from_bits(value.bits).is_finite(),
        };
        if !finite {
            return Err("float literal out of range for its type".into());
        }
        if negative {
            value.bits ^= 1 << (ty.bits() - 1);
        }
        value.validate()?;
        Ok(value)
    }
}

/// `Ok(None)` means the token is an integer literal, not a float.
fn parse_float_literal(text: &str) -> Result<Option<FloatLiteral>, String> {
    if text.starts_with("0x") || text.starts_with("0o") || text.starts_with("0b") {
        return Ok(None);
    }
    let bytes = text.as_bytes();
    let mut at = 0;
    let mut digits = String::new();
    let mut float = false;
    let take_digits = |at: &mut usize, digits: &mut String| {
        let start = digits.len();
        while let Some(&b) = bytes.get(*at) {
            if b == b'_' {
            } else if b.is_ascii_digit() {
                digits.push(b as char);
            } else {
                break;
            }
            *at += 1;
        }
        digits.len() > start
    };
    take_digits(&mut at, &mut digits);
    if bytes.get(at) == Some(&b'.') {
        float = true;
        digits.push('.');
        at += 1;
        take_digits(&mut at, &mut digits);
    }
    if matches!(bytes.get(at), Some(b'e' | b'E')) {
        float = true;
        digits.push('e');
        at += 1;
        if let Some(&sign @ (b'+' | b'-')) = bytes.get(at) {
            digits.push(sign as char);
            at += 1;
        }
        if !take_digits(&mut at, &mut digits) {
            return Err("float literal exponent requires a digit".into());
        }
    }
    let suffix = &text[at..];
    let suffix = if suffix.is_empty() {
        None
    } else if let Some(ty) = float_type(suffix) {
        Some(ty)
    } else if float {
        return Err("unsupported or invalid float literal suffix".into());
    } else {
        return Ok(None);
    };
    if !float && suffix.is_none() {
        return Ok(None);
    }
    Ok(Some(FloatLiteral { digits, suffix }))
}

fn parse_integer_literal(text: &str) -> Result<IntegerLiteral, String> {
    let (radix, start) = if text.starts_with("0x") {
        (16, 2)
    } else if text.starts_with("0o") {
        (8, 2)
    } else if text.starts_with("0b") {
        (2, 2)
    } else {
        (10, 0)
    };
    let bytes = text.as_bytes();
    let mut at = start;
    let mut magnitude = 0u128;
    let mut digit_seen = false;
    while let Some(&b) = bytes.get(at) {
        if b == b'_' {
            at += 1;
            continue;
        }
        let Some(digit) = (b as char).to_digit(radix) else {
            break;
        };
        magnitude = magnitude
            .checked_mul(u128::from(radix))
            .and_then(|n| n.checked_add(u128::from(digit)))
            .ok_or("integer literal exceeds u128")?;
        digit_seen = true;
        at += 1;
    }
    if !digit_seen {
        return Err("integer literal requires a digit".into());
    }
    let suffix = if at == text.len() {
        None
    } else {
        Some(integer_type(&text[at..]).ok_or("unsupported or invalid integer literal suffix")?)
    };
    Ok(IntegerLiteral { magnitude, suffix })
}

fn resolve_integer(
    literal: &IntegerLiteral,
    negative: bool,
    expected: Option<&ir::Ty>,
) -> Result<ir::IntValue, String> {
    let contextual = match expected {
        Some(ir::Ty::Int(ty)) => Some(*ty),
        _ => None,
    };
    let ty = literal.suffix.or(contextual).unwrap_or(ir::IntTy::I32);
    if literal.suffix.is_some() && contextual.is_some_and(|t| t != ty) {
        return Err("integer literal suffix does not match expected integer type".into());
    }
    if negative && !ty.is_signed() {
        return Err("unsigned integer negation is invalid".into());
    }
    let limit = if ty.is_signed() {
        let sign = 1u128 << (ty.bits() - 1);
        if negative { sign } else { sign - 1 }
    } else {
        ty.mask()
    };
    if literal.magnitude > limit {
        let inference = if literal.suffix.is_none() && contextual.is_none() {
            "; backward numeric inference is unsupported"
        } else {
            ""
        };
        return Err(format!(
            "integer literal out of range for {}{inference}",
            ty.name()
        ));
    }
    let bits = if negative {
        0u128.wrapping_sub(literal.magnitude) & ty.mask()
    } else {
        literal.magnitude
    };
    let value = ir::IntValue { ty, bits };
    value.validate()?;
    Ok(value)
}

impl Expr {
    /// Construct a source signed literal for hand-built frontend fixtures.
    pub fn i32(value: i32) -> Self {
        let literal = Self::Integer(IntegerLiteral {
            magnitude: u128::from(value.unsigned_abs()),
            suffix: Some(ir::IntTy::I32),
        });
        if value < 0 {
            Self::Unary {
                op: UnaryOp::Neg,
                value: Box::new(literal),
            }
        } else {
            literal
        }
    }
}

/// A syntax-only invocation retained by the inactive-source validator. There is
/// deliberately no public constructor or conversion to an expanded expression.
pub struct UnexpandedMacroSyntax {
    _path: AstPath,
    _args: super::macro_cursor::TokenSlice,
    span: super::expansion_span::SpanId,
    _lease: super::expansion_budget::ByteLease,
}
impl std::fmt::Debug for UnexpandedMacroSyntax {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnexpandedMacroSyntax")
            .field("span", &self.span)
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Debug)]
pub struct SourceOutput {
    pub output: ir::LiteralOutput,
    pub site: super::expansion_span::SpanId,
}

#[derive(Clone, Debug)]
pub struct SourcePanic {
    pub message: String,
    pub site: super::expansion_span::SpanId,
}

fn panic_callsite(
    session: &super::token_session::TokenSession,
    mut site: super::expansion_span::SpanId,
) -> Result<super::expansion_span::SpanId, super::token_session::TokenError> {
    for _ in 0..512 {
        let span = session.span(site)?;
        match span.expansion {
            Some(expansion) => site = session.expansion(expansion)?.call,
            None => return Ok(site),
        }
    }
    Err(super::token_session::TokenError::TraceTruncated)
}

fn validate_panic_site(panic: &SourcePanic, names: &NameStore) -> Result<(), String> {
    validate_source_site(panic.site, names)
}

fn validate_output_site(output: &SourceOutput, names: &NameStore) -> Result<(), String> {
    validate_source_site(output.site, names)
}

fn validate_source_site(
    site: super::expansion_span::SpanId,
    names: &NameStore,
) -> Result<(), String> {
    let site = panic_callsite(names.session(), site)
        .map_err(|error| super::source::SourceError::from_token(error).to_string())?;
    let original = names
        .session()
        .original(site, 512)
        .map_err(|error| super::source::SourceError::from_token(error).to_string())?;
    names
        .session()
        .sources()
        .location(original)
        .map_err(|error| super::source::SourceError::from_token(error).to_string())?;
    Ok(())
}

#[derive(Clone, Debug)]
pub enum Expr {
    UnexpandedMacro(Arc<UnexpandedMacroSyntax>),
    TraitCall {
        owner: Type,
        trait_path: Box<TraitPath>,
        name: AstIdent,
        args: Vec<Expr>,
    },
    MethodCall {
        receiver: Box<Expr>,
        name: AstIdent,
        type_args: Vec<Type>,
        args: Vec<Expr>,
    },
    AssociatedCall {
        owner: Type,
        name: AstIdent,
        type_args: Vec<Type>,
        args: Vec<Expr>,
    },
    Apply {
        path: GenericPath,
        value: Box<Expr>,
    },
    Cast {
        value: Box<Expr>,
        target: Type,
    },
    Borrow {
        kind: ir::BorrowKind,
        place: Place,
    },
    /// Borrow of a value expression that is not a static place, such as
    /// `&P { .. }`, `&f()` or `&a[i]`. The operand is evaluated into a
    /// temporary whose scope follows Rust's temporary lifetime rules.
    BorrowValue {
        kind: ir::BorrowKind,
        value: Box<Expr>,
    },
    Deref {
        value: Box<Expr>,
    },
    Integer(IntegerLiteral),
    Float(FloatLiteral),
    /// A validated Unicode scalar, never an arbitrary integer.
    Char(char),
    /// Decoded UTF-8 contents of a string literal, typed `&'static str`.
    Str(String),
    DynamicIndex {
        base: Box<Expr>,
        index: Box<Expr>,
    },
    Array(Vec<Expr>),
    Repeat {
        value: Box<Expr>,
        len: usize,
    },
    Index {
        base: Box<Expr>,
        index: usize,
    },
    EnumVariant {
        name: AstPath,
        shape: ir::VariantShape,
        fields: Vec<(FieldSelector, Expr)>,
    },
    Match {
        scrutinee: Box<Expr>,
        arms: Vec<MatchArm>,
    },
    Tuple(Vec<Expr>),
    Struct {
        name: AstPath,
        fields: Vec<(AstIdent, Expr)>,
    },
    TupleStruct {
        name: AstPath,
        fields: Vec<Expr>,
    },
    Field {
        base: Box<Expr>,
        field: FieldSelector,
    },

    Bool(bool),
    Var(AstPath),
    Unary {
        op: UnaryOp,
        value: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Call {
        name: AstPath,
        args: Vec<Expr>,
    },
    If {
        condition: Box<Expr>,
        then_block: Block,
        else_block: Block,
    },
    IfLet {
        pattern: Box<Pattern>,
        scrutinee: Box<Expr>,
        then_block: Block,
        else_branch: Option<Box<Expr>>,
    },
    Block(Block),
    /// `unsafe { ... }`: an ordinary block whose body may perform unsafe
    /// operations (raw pointer dereference, unsafe calls).
    Unsafe(Block),
    /// `'label: { ... }`. Only labeled breaks may target it (E0695/E0696).
    LabeledBlock {
        label: AstIdent,
        body: Block,
    },
    Loop {
        label: Option<AstIdent>,
        body: Block,
    },
    Break {
        label: Option<AstIdent>,
        value: Option<Box<Expr>>,
    },
    Continue {
        label: Option<AstIdent>,
    },
    Println(String),
    Format {
        pieces: Vec<String>,
        arguments: Vec<Expr>,
        stream: ir::OutputStream,
        newline: bool,
    },
    Output(Box<SourceOutput>),
    Panic(Box<SourcePanic>),
}
#[derive(Clone, Debug)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub body: Expr,
    /// A builtin matches! guard lowered into the body's If condition.
    /// Retained so type checking can enforce the bounded guard binding rules.
    pub guarded: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

#[derive(Debug)]
pub(super) struct ParseError {
    pub(super) kind: super::source::SourceErrorKind,
    // True only when bytes is a real prepared-source coordinate available for
    // a mapping fallback. Expanded diagnostics carry checked primary provenance.
    pub(super) has_location: bool,
    pub(super) bytes: std::ops::Range<usize>,
    primary: Option<super::source::SourceSpan>,
    line: usize,
    column: usize,
    pub(super) message: String,
    _diagnostic_lease: Option<super::expansion_budget::ByteLease>,
}
impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind {
            super::source::SourceErrorKind::Contract => write!(f, "contract: {}", self.message),
            super::source::SourceErrorKind::Infrastructure => {
                write!(f, "infrastructure: {}", self.message)
            }
            _ => write!(f, "{}:{}: {}", self.line, self.column, self.message),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ParserTokenKind {
    Ident,
    Number,
    Char,
    String,
    Punct,
    Lifetime,
    OpaqueLiteral,
    DeferredLiteral,
    DeferredUnsupported,
    Eof,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TokenProvenance {
    Prepared,
    Expanded,
}
#[derive(Clone, Debug)]
struct TokenPart {
    span: super::expansion_span::SpanId,
    lo: u32,
    hi: u32,
}
impl TokenPart {
    // Prepared source is bounded far below 2 GiB. Reuse the high bit of the
    // existing hi word instead of growing every ordinary token and suffix.
    // Expanded words hold a checked diagnostic line/column, never source bytes.
    const EXPANDED: u32 = 1 << 31;
    fn new(
        span: super::expansion_span::SpanId,
        bytes: std::ops::Range<usize>,
    ) -> Result<Self, ParseError> {
        if bytes.start > bytes.end || bytes.end >= Self::EXPANDED as usize {
            return Err(native_adapter::transport_error(
                super::token_session::TokenError::InvalidRange,
            ));
        }
        let invalid =
            |_| native_adapter::transport_error(super::token_session::TokenError::InvalidRange);
        Ok(Self {
            span,
            lo: u32::try_from(bytes.start).map_err(invalid)?,
            hi: u32::try_from(bytes.end).map_err(invalid)?,
        })
    }
    fn expanded(
        span: super::expansion_span::SpanId,
        line: usize,
        column: usize,
    ) -> Result<Self, ParseError> {
        let invalid =
            || native_adapter::transport_error(super::token_session::TokenError::InvalidRange);
        if line == 0 || column == 0 || column >= Self::EXPANDED as usize {
            return Err(invalid());
        }
        let lo = u32::try_from(line).map_err(|_| invalid())?;
        let hi = u32::try_from(column).map_err(|_| invalid())? | Self::EXPANDED;
        Ok(Self { span, lo, hi })
    }
    fn provenance(&self) -> TokenProvenance {
        if self.hi & Self::EXPANDED == 0 {
            TokenProvenance::Prepared
        } else {
            TokenProvenance::Expanded
        }
    }
    fn bytes(&self) -> std::ops::Range<usize> {
        match self.provenance() {
            TokenProvenance::Prepared => self.lo as usize..self.hi as usize,
            // Explicit absence of a prepared coordinate, not a source anchor.
            TokenProvenance::Expanded => 0..0,
        }
    }
    fn position(&self, line: usize, column: usize, prefix: usize) -> (usize, usize) {
        match self.provenance() {
            TokenProvenance::Prepared => (line, column + prefix),
            TokenProvenance::Expanded => (self.lo as usize, (self.hi & !Self::EXPANDED) as usize),
        }
    }
}
#[derive(Debug)]
struct Token {
    text: String,
    line: usize,
    column: usize,
    string: bool,
    kind: ParserTokenKind,
    whole: TokenPart,
    constituents: [Option<super::expansion_span::SpanId>; 3],
    suffixes: [Option<TokenPart>; 2],
    name: Option<std::num::NonZeroU32>,
}
fn compound_operator(token: TokenView<'_>) -> Option<BinaryOp> {
    if token.string {
        return None;
    }
    Some(match token.text {
        "+=" => BinaryOp::Add,
        "-=" => BinaryOp::Sub,
        "*=" => BinaryOp::Mul,
        "/=" => BinaryOp::Div,
        "%=" => BinaryOp::Rem,
        "&=" => BinaryOp::BitAnd,
        "|=" => BinaryOp::BitOr,
        "^=" => BinaryOp::BitXor,
        "<<=" => BinaryOp::Shl,
        ">>=" => BinaryOp::Shr,
        _ => return None,
    })
}
fn lex(source: &str) -> Result<NativeParserInput, ParseError> {
    let budget = super::expansion_budget::ExpansionBudget::default();
    let projection = super::source::PreparedLexInput::identity(
        std::path::Path::new("<syntax>"),
        source,
        &budget,
    )
    .map_err(native_adapter::source_error)?;
    native_input(&projection, budget)
}
fn native_input(
    projection: &super::source::PreparedLexInput<'_>,
    budget: super::expansion_budget::ExpansionBudget,
) -> Result<NativeParserInput, ParseError> {
    let mut session =
        super::token_session::TokenSession::new(projection.snapshot().clone(), budget)
            .map_err(native_adapter::transport_error)?;
    let lexed = super::lexer::lex_prepared(projection, super::token::Edition::E2024, &mut session)
        .map_err(native_adapter::lex_error)?;
    NativeParserInput::new(lexed, session, projection)
}
fn path_text_len(
    names: &impl NameAccess,
    path: &AstPath,
) -> Result<usize, super::semantic_name::NameError> {
    path.check(names)?;
    let anchor = match path.anchor() {
        PathAnchor::Relative => 0,
        PathAnchor::Crate => 5,
        PathAnchor::SelfModule | PathAnchor::SelfType => 4,
        PathAnchor::Super(count) => usize::from(count) * 7 - 2,
    };
    let mut bytes = anchor;
    for (index, name) in path.segments().iter().enumerate() {
        names
            .budget()
            .charge(super::expansion_budget::Counter::Work, 1)?;
        if index != 0 || anchor != 0 {
            bytes += 2;
        }
        bytes += names.spelling(*name)?.len();
    }
    Ok(bytes)
}
fn canonical_path_text_len(
    names: &impl NameAccess,
    path: &AstPath,
) -> Result<usize, super::semantic_name::NameError> {
    let size = path_text_len(names, path)?;
    Ok(if path.anchor() == PathAnchor::Crate {
        size - if path.is_empty() { 5 } else { 7 }
    } else {
        size
    })
}
#[cfg(test)]
mod local_macro_parser_tests {
    use super::*;
    fn syntax_with_budget(
        source: &str,
        budget: &super::super::expansion_budget::ExpansionBudget,
    ) -> Result<ParsedProgram, ParseError> {
        let input = super::super::source::PreparedLexInput::identity(
            std::path::Path::new("nested.rs"),
            source,
            budget,
        )
        .map_err(native_adapter::source_error)?;
        parse_syntax_tokens(native_input(&input, budget.clone())?)
    }
    #[test]
    fn nested_source_depth_is_live_through_parse_and_restored_for_siblings() {
        use super::super::expansion_budget::{
            BudgetResource, Depth, ExpansionBudget, ExpansionLimits,
        };
        let mut limits = ExpansionLimits::default();
        limits.depths[Depth::Expansion as usize] = 2;
        let budget = ExpansionBudget::new(limits);
        let result = syntax_with_budget("fn main(){macro_rules! leaf{()=>{1};}macro_rules! pass{()=>{leaf!()};}let a:i32=pass!();let b:i32=pass!();}", &budget).unwrap();
        assert!(budget.snapshot().failure.is_none());
        drop(result);
        assert_eq!(budget.snapshot().retained_bytes, 0);
        assert_eq!(budget.snapshot().temporary_bytes, 0);

        let budget = ExpansionBudget::new(limits);
        let error = syntax_with_budget("fn main(){macro_rules! leaf{()=>{1};}macro_rules! pass{()=>{leaf!()};}macro_rules! outer{()=>{pass!()};}let a:i32=outer!();}", &budget).unwrap_err();
        assert_eq!(error.kind, super::super::source::SourceErrorKind::Limit);
        assert_eq!(
            budget.snapshot().failure.unwrap().resource,
            BudgetResource::Depth(Depth::Expansion)
        );
        drop(error);
        assert_eq!(budget.snapshot().retained_bytes, 0);
        assert_eq!(budget.snapshot().temporary_bytes, 0);
    }
    #[test]
    fn parser_root_owner_preflights_bytes_and_releases_after_borrowed_children() {
        use super::super::expansion_budget::{ExpansionBudget, Storage};
        let budget = ExpansionBudget::default();
        let mut owner = ParserState::owned(7u64, &budget).unwrap();
        let reserved = budget.snapshot().temporary_bytes;
        assert!(reserved >= std::mem::size_of::<u64>() as u64);
        {
            let mut child = ParserState::Borrowed(&mut *owner);
            *child = 11;
        }
        assert_eq!(budget.snapshot().temporary_bytes, reserved);
        assert_eq!(owner.into_owned().unwrap(), 11);
        assert_eq!(budget.snapshot().temporary_bytes, 0);
        let blocker = budget
            .reserve(Storage::Temporary, budget.limits().bytes - 7)
            .unwrap();
        let error = match ParserState::owned(7u64, &budget) {
            Ok(_) => panic!("root owner allocated without admission"),
            Err(error) => error,
        };
        assert_eq!(error.kind, super::super::source::SourceErrorKind::Limit);
        assert_eq!(budget.snapshot().temporary_bytes, blocker.bytes());
        drop(error);
        drop(blocker);
        assert_eq!(budget.snapshot().temporary_bytes, 0);
    }
    #[test]
    fn println_owned_format_denies_bytes_before_allocating_output() {
        use super::super::expansion_budget::{BudgetResource, ExpansionBudget, Storage};
        let budget = ExpansionBudget::default();
        let mut builder =
            ProgramBuilder::new(std::path::Path::new("format.rs"), "", &budget).unwrap();
        let before = budget.snapshot();
        let blocker = budget
            .reserve(
                Storage::Temporary,
                budget.limits().bytes - before.retained_bytes - before.temporary_bytes - 2,
            )
            .unwrap();
        let error = literal_format_owned("{{}}", &mut builder.names).unwrap_err();
        assert_eq!(error.kind, super::super::source::SourceErrorKind::Limit);
        let denied = budget.snapshot();
        assert_eq!(denied.retained_bytes, before.retained_bytes);
        let failure = denied.failure.unwrap();
        assert_eq!(failure.resource, BudgetResource::Bytes);
        assert_eq!(failure.requested, 3);
        drop(error);
        drop(builder);
        drop(blocker);
        assert_eq!(budget.snapshot().retained_bytes, 0);
        assert_eq!(budget.snapshot().temporary_bytes, 0);
    }
    #[test]
    fn println_owned_format_charge_survives_until_last_name_owner() {
        let budget = super::super::expansion_budget::ExpansionBudget::default();
        let mut builder =
            ProgramBuilder::new(std::path::Path::new("format.rs"), "", &budget).unwrap();
        let before = budget.snapshot().retained_bytes;
        let text = literal_format_owned("é{{ok}}", &mut builder.names).unwrap();
        assert_eq!(text, "é{ok}");
        let charged = budget.snapshot().retained_bytes - before;
        assert!(charged >= text.capacity() as u64);
        drop(text);
        assert_eq!(budget.snapshot().retained_bytes - before, charged);
        let program = builder.finish().unwrap();
        let owner = program.names.clone();
        drop(program);
        assert!(budget.snapshot().retained_bytes >= charged);
        drop(owner);
        assert_eq!(budget.snapshot().retained_bytes, 0);
        assert_eq!(budget.snapshot().temporary_bytes, 0);
    }
    #[test]
    fn lexical_replay_transport_preserves_resource_and_unsupported_causes() {
        use super::super::expansion_budget::{
            BudgetFailure, BudgetFailureKind, BudgetResource, Counter,
        };
        use super::super::semantic_scope::ScopeError;
        use super::super::token_session::TokenError;
        let budget = ir::PlanBudget::default();
        for kind in [
            BudgetFailureKind::Limit,
            BudgetFailureKind::Allocation,
            BudgetFailureKind::Infrastructure,
        ] {
            let cause = BudgetFailure {
                kind,
                resource: BudgetResource::Counter(Counter::Work),
                used: 1,
                requested: 1,
                limit: 1,
            };
            let error = source_scope_error(&budget, ScopeError::Token(TokenError::Budget(cause)));
            assert_eq!(error.kind(), tr::FactsErrorKind::Resource);
            assert!(!budget.has_resource_failure());
        }
        assert_eq!(
            source_scope_error(&budget, ScopeError::UnsupportedProjection).kind(),
            tr::FactsErrorKind::Unsupported
        );
        assert_eq!(
            source_scope_error(&budget, ScopeError::UnrecordedOccurrence).kind(),
            tr::FactsErrorKind::InvalidMetadata
        );
    }
    #[test]
    fn source_macro_uses_definition_and_capture_frontiers() {
        let program = parse("fn helper()->i32{2}fn main(){let x:i32=40;macro_rules! add{($arg:ident)=>{x+helper()+$arg};}{let x:i32=1;let y:i32=3;let result:i32=add!(y);if result==45{println!(\"ok\");}else{println!(\"bad\");}}}").unwrap();
        assert!(program.scopes.is_some());
        assert!(program.functions[0].lexical_scope.is_none());
        assert!(program.functions[1].lexical_scope.is_some());
        check(&program).unwrap();
    }
    #[test]
    fn checked_occurrence_expansion_exhaustion_remains_replay_resource() {
        let program = parse("fn main(){}").unwrap();
        let ident = program.functions[0].name.segments()[0];
        let budget = ir::PlanBudget::default();
        assert!(
            program
                .names
                .budget()
                .charge(super::super::expansion_budget::Counter::Work, u64::MAX)
                .is_err()
        );
        let message = ident.occurrence(&program.names).unwrap_err();
        let error = source_name_error(&program.names, &budget, &message);
        assert_eq!(error.kind(), tr::FactsErrorKind::Resource);
        assert!(!budget.has_resource_failure());
    }
    #[test]
    fn definition_cannot_see_later_binding_or_escape_block() {
        for source in [
            "fn main(){macro_rules! get{()=>{x};}let x:i32=1;let result:i32=get!();}",
            "fn main(){{macro_rules! get{()=>{1};}}let result:i32=get!();}",
            "fn main(){let result:i32=get!();macro_rules! get{()=>{1};}}",
        ] {
            assert!(
                parse(source).and_then(|program| check(&program)).is_err(),
                "{source}"
            );
        }
    }
    #[test]
    fn macro_argument_strings_are_not_cursor_delimiters_or_eof() {
        for literal in ["\"(\"", "\")\"", "\"}\"", "\"\""] {
            let source = format!(
                "fn main(){{macro_rules! one{{($value:tt)=>{{1}};}}let result:i32=one!({literal});}}"
            );
            let program = parse(&source).unwrap();
            check(&program).unwrap();
        }
    }
    #[test]
    fn literal_fragment_unused_discarded_and_consumed_are_admitted() {
        for source in [
            "fn main(){macro_rules! one{($value:literal)=>{1};}}",
            "fn main(){macro_rules! one{($value:literal)=>{1};}let value:i32=one!(2);}",
            "fn main(){macro_rules! one{($value:literal)=>{$value};}let value:i32=one!(2);}",
        ] {
            let program = parse(source).unwrap();
            check(&program).unwrap();
        }
    }
    #[test]
    fn opaque_signed_minimum_retains_existing_direct_unary_shape() {
        for (ty, literal) in [
            ("i32", "-2147483648i32"),
            ("i128", "-170141183460469231731687303715884105728i128"),
        ] {
            let source = format!(
                "fn main(){{macro_rules! one{{($value:literal)=>{{$value}};}}let value:{ty}=one!({literal});}}"
            );
            let program = parse(&source).unwrap();
            let Stmt::Let {
                value:
                    Expr::Unary {
                        op: UnaryOp::Neg,
                        value,
                    },
                ..
            } = &program.functions[0].body.statements[0]
            else {
                panic!("opaque literal changed unary shape");
            };
            assert!(matches!(value.as_ref(), Expr::Integer(_)));
            check(&program).unwrap();
        }
    }
    #[test]
    fn opaque_minus_is_atomic_before_outer_cast() {
        let program = parse("fn main(){macro_rules! one{($value:literal)=>{$value as i64};}let value:i64=one!(-1i32);}").unwrap();
        let Stmt::Let {
            value: Expr::Cast { value, .. },
            ..
        } = &program.functions[0].body.statements[0]
        else {
            panic!("outer cast no longer wraps atom");
        };
        assert!(matches!(
            value.as_ref(),
            Expr::Unary {
                op: UnaryOp::Neg,
                ..
            }
        ));
        check(&program).unwrap();
    }
    #[test]
    fn opaque_bool_numeric_and_contextual_string_use_real_source_forwarding() {
        let program = parse("fn forward(n:i32)->i32{n}fn main(){macro_rules! emit{($n:literal,$flag:literal,$text:literal)=>{if $flag{if forward($n)==-45{println!($text);}}};}emit!(-45,true,\"ok\");}").unwrap();
        check(&program).unwrap();
        // A forwarded string literal is an ordinary `&'static str` value.
        let program = parse(
            "fn main(){macro_rules! one{($value:literal)=>{$value};}let value=one!(\"text\");}",
        )
        .unwrap();
        check(&program).unwrap();
        let program = parse(
            "fn main(){macro_rules! one{($value:literal)=>{$value};}let value:bool=one!(-true);}",
        )
        .unwrap();
        assert!(check(&program).is_err());
    }
    #[test]
    fn compact_provenance_tags_never_alias_prepared_coordinates() {
        let (buffer, _session) = lex("x").unwrap().into_parts();
        let token = TokenCursor::new(&buffer).current();
        let prepared = TokenPart::new(token.span, 1..7).unwrap();
        assert_eq!(prepared.provenance(), TokenProvenance::Prepared);
        assert_eq!(prepared.bytes(), 1..7);
        assert!(TokenPart::new(token.span, 0..TokenPart::EXPANDED as usize).is_err());
        let expanded = TokenPart::expanded(token.span, 8, 13).unwrap();
        assert_eq!(expanded.provenance(), TokenProvenance::Expanded);
        assert_eq!(expanded.bytes(), 0..0);
        assert_eq!(expanded.position(1, 1, 2), (8, 13));
        assert!(TokenPart::expanded(token.span, 0, 1).is_err());
        assert!(TokenPart::expanded(token.span, 1, TokenPart::EXPANDED as usize).is_err());
        assert_eq!(
            std::mem::size_of::<TokenPart>(),
            std::mem::size_of::<(super::super::expansion_span::SpanId, u32, u32)>()
        );
    }
}
struct ParserOwned<T> {
    values: Vec<T>,
    _lease: super::expansion_budget::ByteLease,
}
enum ParserState<'a, T> {
    Owned(ParserOwned<T>),
    Borrowed(&'a mut T),
}
impl<T> std::ops::Deref for ParserState<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        match self {
            Self::Owned(value) => &value.values[0],
            Self::Borrowed(value) => value,
        }
    }
}
impl<T> std::ops::DerefMut for ParserState<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        match self {
            Self::Owned(value) => &mut value.values[0],
            Self::Borrowed(value) => value,
        }
    }
}
impl<T> ParserState<'_, T> {
    fn owned(
        value: T,
        budget: &super::expansion_budget::ExpansionBudget,
    ) -> Result<Self, ParseError> {
        budget
            .charge(super::expansion_budget::Counter::Work, 1)
            .map_err(|error| native_adapter::transport_error(error.into()))?;
        let bytes = u64::try_from(std::mem::size_of::<T>()).map_err(|_| {
            native_adapter::transport_error(super::token_session::TokenError::IdExhausted)
        })?;
        let mut lease = budget
            .reserve(super::expansion_budget::Storage::Temporary, bytes)
            .map_err(|error| native_adapter::transport_error(error.into()))?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(1)
            .map_err(|_| native_adapter::transport_error(budget.poison_allocation().into()))?;
        let actual = (values.capacity() as u64)
            .checked_mul(bytes)
            .ok_or_else(|| {
                native_adapter::transport_error(super::token_session::TokenError::IdExhausted)
            })?;
        lease
            .resize(actual)
            .map_err(|error| native_adapter::transport_error(error.into()))?;
        values.push(value);
        Ok(Self::Owned(ParserOwned {
            values,
            _lease: lease,
        }))
    }
    fn into_owned(self) -> Result<T, ParseError> {
        match self {
            Self::Owned(mut value) => value.values.pop().ok_or_else(|| {
                native_adapter::transport_error(super::token_session::TokenError::InvalidHandle)
            }),
            Self::Borrowed(_) => Err(native_adapter::transport_error(
                super::token_session::TokenError::InvalidInput("missing parser lexical owner"),
            )),
        }
    }
}
#[derive(Default)]
struct ParserLexicalState {
    scopes: Option<super::semantic_scope::ScopeArenaBuilder>,
    function: Option<FunctionMacroState>,
    item_macros: Option<super::source_item_macro::SourceItemMacroDriver>,
    inactive_items: Option<super::token_session::BudgetVec<Arc<UnexpandedMacroSyntax>>>,
}
struct FunctionMacroState {
    frontier: super::semantic_scope::FrontierId,
    driver: Option<super::source_macro::LocalMacroDriver>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum ParserPurpose {
    Active,
    InactiveSyntax,
}
fn scope_parse_error(error: super::semantic_scope::ScopeError) -> ParseError {
    let error = error.source_error();
    ParseError {
        _diagnostic_lease: None,
        primary: error.primary,
        kind: error.kind,
        has_location: false,
        bytes: 0..0,
        line: 1,
        column: 1,
        message: error.message,
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum ConditionPurpose {
    Ordinary,
    Conditional,
}

struct Parser<'input, 'names> {
    debug_assertions: bool,
    input: &'input NativeTokenBuffer,
    names: &'names mut NameStoreBuilder,
    modules: ParserState<'names, ModuleArenaBuilder>,
    lexical: ParserState<'names, ParserLexicalState>,
    expansion: Option<super::semantic_scope::ExpansionAdmission>,
    expansion_depth: u32,
    purpose: ParserPurpose,
    item_expansion: Option<super::semantic_scope::ItemExpansionAdmission>,
    module_frontier: Option<super::semantic_scope::ModuleFrontierId>,
    module: ModuleId,
    alias_target: bool,
    pending_bounds: Vec<TypeBound>,
    /// Anonymous parameters minted for argument-position `impl Trait` while a
    /// function parameter list is parsed. `None` forbids `impl Trait` (E0562).
    impl_trait_params: Option<Vec<AstIdent>>,
    cursor: TokenCursor<'input>,
    nesting: usize,
    allow_struct: bool,
    condition_purpose: ConditionPurpose,
    type_params: Vec<AstIdent>,
    use_expansions: usize,
    use_expansion_bytes: usize,
}
impl<'input, 'names> Parser<'input, 'names> {
    fn inactive_invocation(
        &mut self,
        path: AstPath,
        span: super::expansion_span::SpanId,
    ) -> Result<Arc<UnexpandedMacroSyntax>, ParseError> {
        if self.purpose != ParserPurpose::InactiveSyntax {
            return Err(self.unsupported("unexpanded macro syntax cannot enter active parsing"));
        }
        path.check(self.names).map_err(native_adapter::name_error)?;
        let args = self.macro_group()?;
        let bytes = std::mem::size_of::<UnexpandedMacroSyntax>() + 2 * std::mem::size_of::<usize>();
        self.names
            .budget()
            .charge(super::expansion_budget::Counter::Work, 1)
            .map_err(|error| native_adapter::transport_error(error.into()))?;
        let lease = self
            .names
            .budget()
            .reserve(super::expansion_budget::Storage::Retained, bytes as u64)
            .map_err(|error| native_adapter::transport_error(error.into()))?;
        Ok(Arc::new(UnexpandedMacroSyntax {
            _path: path,
            _args: args,
            span,
            _lease: lease,
        }))
    }
    fn module_macro_definition(&mut self) -> Result<(), ParseError> {
        if self.item_expansion.is_some() || self.expansion.is_some() {
            return Err(self.unsupported("generated macro definitions are unsupported"));
        }
        let (name, body, site) = self.take_macro_definition()?;
        if self.purpose == ParserPurpose::InactiveSyntax {
            self.names
                .compile_local_rules(body, site)
                .map_err(|error| self.driver_error(error.into()))?;
            return Ok(());
        }
        let point = self.module_frontier.ok_or_else(|| {
            native_adapter::transport_error(super::token_session::TokenError::InvalidHandle)
        })?;
        let driver = self.lexical.item_macros.as_mut().ok_or_else(|| {
            native_adapter::transport_error(super::token_session::TokenError::InvalidHandle)
        })?;
        let result = driver.define(name, body, site, point, self.names);
        result.map_err(|error| self.driver_error(error))
    }
    fn item_macro_invocation(
        &mut self,
        module: ModuleId,
        module_path: &AstPath,
        parsed: &mut ParsedDeclarations,
    ) -> Result<(), ParseError> {
        let span = self.token().span;
        let name = self.ident()?;
        self.expect("!")?;
        if self.purpose == ParserPurpose::InactiveSyntax {
            let path = AstPath::new(self.names, PathAnchor::Relative, &[name])
                .map_err(native_adapter::name_error)?;
            let braced = self.is("{");
            let syntax = self.inactive_invocation(path, span)?;
            if braced {
                self.eat(";");
            } else {
                self.expect(";")?;
            }
            self.lexical
                .inactive_items
                .as_mut()
                .ok_or_else(|| {
                    native_adapter::transport_error(super::token_session::TokenError::InvalidHandle)
                })?
                .push(syntax)
                .map_err(native_adapter::transport_error)?;
            return Ok(());
        }
        if self.item_expansion.is_some() {
            return Err(self.unsupported("nested item macro products are unsupported"));
        }
        if self.module_frontier.is_none() || self.lexical.item_macros.is_none() {
            return Err(native_adapter::expanded_error(
                self.names.session(),
                span,
                super::source::SourceErrorKind::Syntax,
                "unknown module macro at this lexical point",
            ));
        }
        let braced = self.is("{");
        let args = self.macro_group()?;
        if braced {
            self.eat(";");
        } else {
            self.expect(";")?;
        }
        let call = self.module_frontier.ok_or_else(|| {
            native_adapter::transport_error(super::token_session::TokenError::InvalidHandle)
        })?;
        let scopes = self.lexical.scopes.as_ref().ok_or_else(|| {
            native_adapter::transport_error(super::token_session::TokenError::InvalidHandle)
        })?;
        let driver = self
            .lexical
            .item_macros
            .as_ref()
            .ok_or_else(|| self.error("unknown module macro"))?;
        let selected = driver
            .select(name, call, scopes, self.names)
            .map_err(|error| self.driver_error(error))?;
        let expanded = driver
            .expand(
                selected,
                args,
                super::macro_rules::CallSite { span, parent: None },
                self.names,
            )
            .map_err(|error| self.driver_error(error))?;
        let expansion = expanded
            .expansion(self.names.session())
            .map_err(|error| self.driver_error(error.into()))?;
        let tokens = expanded
            .tokens(self.names.session())
            .map_err(|error| self.driver_error(error.into()))?;
        let scopes = self.lexical.scopes.as_mut().ok_or_else(|| {
            native_adapter::transport_error(super::token_session::TokenError::InvalidHandle)
        })?;
        let proof = scopes
            .anchor_item_expansion(
                self.names,
                &*self.modules,
                expansion,
                selected.definition(),
                selected.call(),
            )
            .map_err(scope_parse_error)?;
        let eof = self
            .names
            .item_expansion_eof(&proof)
            .map_err(scope_parse_error)?;
        let buffer = expanded_adapter::lower_item(
            tokens,
            self.names,
            &*self.modules,
            self.names.edition(),
            eof,
            &proof,
        )?;
        let mut child = Parser {
            debug_assertions: self.debug_assertions,
            input: &buffer,
            names: &mut *self.names,
            modules: ParserState::Borrowed(&mut *self.modules),
            lexical: ParserState::Borrowed(&mut *self.lexical),
            expansion: None,
            expansion_depth: 0,
            purpose: ParserPurpose::Active,
            item_expansion: Some(proof),
            module_frontier: self.module_frontier,
            module,
            alias_target: false,
            pending_bounds: Vec::new(),
            impl_trait_params: None,
            cursor: TokenCursor::new(&buffer),
            nesting: self.nesting,
            allow_struct: true,
            condition_purpose: ConditionPurpose::Ordinary,
            type_params: Vec::new(),
            use_expansions: self.use_expansions,
            use_expansion_bytes: self.use_expansion_bytes,
        };
        let mut product = ParsedDeclarations {
            program: ProgramDeclarations::default(),
            items: HashMap::new(),
            empty_use_prefixes: Vec::new(),
        };
        child.reserve_item_publication(&mut product)?;
        let (visibility, explicit_visibility) = child.visibility(module)?;
        if !child.is("fn") {
            return Err(child.unsupported("item macro output must be exactly one nongeneric free function; generated attributes and other items are unsupported"));
        }
        child.nonmodule_item(
            module,
            module_path,
            &mut product,
            visibility,
            explicit_visibility,
        )?;
        if !child.is("") || product.program.functions.len() != 1 {
            return Err(
                child.unsupported("item macro output must be exactly one complete free function")
            );
        }
        drop(child);
        let generated = &product.program.functions[0];
        let declaration = *generated.name.segments().last().ok_or_else(|| {
            native_adapter::transport_error(super::token_session::TokenError::InvalidHandle)
        })?;
        let function = generated.lexical_scope.ok_or_else(|| {
            native_adapter::transport_error(super::token_session::TokenError::InvalidHandle)
        })?;
        self.lexical
            .scopes
            .as_mut()
            .ok_or_else(|| {
                native_adapter::transport_error(super::token_session::TokenError::InvalidHandle)
            })?
            .seal_item_product(self.names, &*self.modules, &proof, function, declaration)
            .map_err(scope_parse_error)?;
        self.reserve_item_publication(parsed)?;
        for (key, item) in product.items {
            if parsed.items.contains_key(&key) {
                return Err(self.error("duplicate item name"));
            }
            parsed.items.insert(key, item);
        }
        parsed.program.functions.extend(product.program.functions);
        Ok(())
    }
    fn reserve_item_publication(
        &mut self,
        parsed: &mut ParsedDeclarations,
    ) -> Result<(), ParseError> {
        use super::expansion_budget::Counter;
        use super::token_session::TokenError;
        let overflow = || native_adapter::transport_error(TokenError::IdExhausted);
        if parsed.program.functions.len() == parsed.program.functions.capacity() {
            let planned = parsed
                .program
                .functions
                .len()
                .checked_add(1)
                .and_then(|n| n.checked_mul(std::mem::size_of::<Function>()))
                .ok_or_else(overflow)?;
            self.names
                .retain_parser_storage(planned)
                .map_err(native_adapter::name_error)?;
            parsed.program.functions.try_reserve_exact(1).map_err(|_| {
                native_adapter::transport_error(self.names.budget().poison_allocation().into())
            })?;
            let actual = parsed
                .program
                .functions
                .capacity()
                .checked_mul(std::mem::size_of::<Function>())
                .ok_or_else(overflow)?;
            self.names
                .retain_parser_storage(actual.saturating_sub(planned))
                .map_err(native_adapter::name_error)?;
        }
        // Hashing traverses the immutable path segments. Precharge all old
        // keys before any possible collision walk or growth-driven rehash.
        self.names
            .budget()
            .charge(Counter::Work, parsed.items.len() as u64)
            .map_err(|error| native_adapter::transport_error(error.into()))?;
        for items in parsed.items.values() {
            let item = items
                .first()
                .ok_or_else(|| native_adapter::transport_error(TokenError::InvalidHandle))?;
            let work = item
                .path
                .len()
                .checked_add(1)
                .and_then(|n| n.checked_mul(3))
                .ok_or_else(overflow)?;
            self.names
                .budget()
                .charge(Counter::Work, work as u64)
                .map_err(|error| native_adapter::transport_error(error.into()))?;
        }
        if parsed.items.len() == parsed.items.capacity() {
            let entries = parsed.items.len().checked_add(1).ok_or_else(overflow)?;
            let buckets = entries
                .checked_mul(8)
                .and_then(|n| n.checked_add(6))
                .map(|n| n / 7)
                .and_then(|n| n.max(4).checked_next_power_of_two())
                .ok_or_else(overflow)?;
            let entry_bytes = std::mem::size_of::<(PathKey, Vec<Item>)>()
                .checked_add(1)
                .ok_or_else(overflow)?;
            let planned = buckets
                .checked_mul(entry_bytes)
                .and_then(|n| n.checked_add(16))
                .ok_or_else(overflow)?;
            self.names
                .retain_parser_storage(planned)
                .map_err(native_adapter::name_error)?;
            parsed.items.try_reserve(1).map_err(|_| {
                native_adapter::transport_error(self.names.budget().poison_allocation().into())
            })?;
            let actual = parsed
                .items
                .capacity()
                .max(4)
                .checked_next_power_of_two()
                .and_then(|n| n.checked_mul(entry_bytes))
                .and_then(|n| n.checked_add(16))
                .ok_or_else(overflow)?;
            self.names
                .retain_parser_storage(actual.saturating_sub(planned))
                .map_err(native_adapter::name_error)?;
        }
        Ok(())
    }
    fn driver_error(&self, error: super::source_macro::DriverError) -> ParseError {
        use super::macro_rules::MacroError;
        use super::source_macro::DriverError;
        match error {
            DriverError::Token(error) => native_adapter::transport_error(error),
            DriverError::Name(error) => native_adapter::name_error(error),
            DriverError::Scope(error) => scope_parse_error(error),
            DriverError::Unsupported { span, message } => native_adapter::expanded_error(
                self.names.session(),
                span,
                super::source::SourceErrorKind::Unsupported,
                message,
            ),
            DriverError::Macro(MacroError::Resource(error)) => {
                native_adapter::transport_error(super::token_session::TokenError::Budget(error))
            }
            DriverError::Macro(MacroError::Contract(error)) => {
                native_adapter::transport_error(error)
            }
            DriverError::Macro(error) => {
                let span = match &error {
                    MacroError::Syntax { span, .. } | MacroError::Ambiguous { span } => *span,
                    MacroError::Unsupported { span, .. } => Some(*span),
                    _ => None,
                };
                let kind = if matches!(error, MacroError::Unsupported { .. }) {
                    super::source::SourceErrorKind::Unsupported
                } else {
                    super::source::SourceErrorKind::Syntax
                };
                if let Some(span) = span {
                    native_adapter::expanded_error(
                        self.names.session(),
                        span,
                        kind,
                        &error.to_string(),
                    )
                } else {
                    let mut diagnostic = self.error(&error.to_string());
                    diagnostic.kind = kind;
                    diagnostic
                }
            }
        }
    }
    fn frontier(&self) -> Option<super::semantic_scope::FrontierId> {
        self.lexical
            .function
            .as_ref()
            .map(|function| function.frontier)
    }
    fn macro_group(&mut self) -> Result<super::macro_cursor::TokenSlice, ParseError> {
        if !self.is("(") && !self.is("[") && !self.is("{") {
            return Err(self.error("expected macro delimiter"));
        }
        let open = self.token().span;
        let body =
            if self.purpose == ParserPurpose::InactiveSyntax || self.lexical.function.is_none() {
                let source = self
                    .input
                    .macro_source()
                    .cloned()
                    .ok_or_else(|| self.error("macro source inventory is unavailable"))?;
                let source = super::macro_cursor::TokenSlice::roots(self.names.session(), source)
                    .map_err(native_adapter::transport_error)?;
                super::source_item_macro::group_body(&source, open, self.names)
                    .map_err(native_adapter::transport_error)?
            } else {
                let function = self
                    .lexical
                    .function
                    .as_ref()
                    .ok_or_else(|| self.unsupported("function-local macro context required"))?;
                let driver = function.driver.as_ref().ok_or_else(|| {
                    self.unsupported("user macros in generated functions are unsupported")
                })?;
                if let Some(source) = self.input.current_macro_slice() {
                    driver.group_body_in(source, open, self.names)
                } else if self.expansion.is_some() {
                    return Err(native_adapter::transport_error(
                        super::token_session::TokenError::InvalidHandle,
                    ));
                } else {
                    driver.group_body(open, self.names)
                }
                .map_err(|error| self.driver_error(error))?
            };
        let mut stack = super::token_session::BudgetVec::new(
            self.names.budget(),
            super::expansion_budget::Storage::Temporary,
        )
        .map_err(native_adapter::transport_error)?;
        loop {
            self.names
                .budget()
                .charge(super::expansion_budget::Counter::Work, 1)
                .map_err(|error| {
                    native_adapter::transport_error(super::token_session::TokenError::Budget(error))
                })?;
            let token = self.token();
            if token.matches("(") {
                stack.push(")").map_err(native_adapter::transport_error)?;
            } else if token.matches("[") {
                stack.push("]").map_err(native_adapter::transport_error)?;
            } else if token.matches("{") {
                stack.push("}").map_err(native_adapter::transport_error)?;
            } else if token.matches(")") || token.matches("]") || token.matches("}") {
                if stack.pop() != Some(token.text) {
                    return Err(self.error("mismatched macro delimiter"));
                }
            } else if token.matches("") {
                return Err(self.error("unterminated macro delimiter"));
            }
            self.cursor.bump();
            if stack.as_slice().is_empty() {
                break;
            }
        }
        Ok(body)
    }
    fn local_macro_definition(&mut self) -> Result<(), ParseError> {
        if self.expansion.is_some() || self.item_expansion.is_some() {
            return Err(self.unsupported("nested or generated macro definitions are unsupported"));
        }
        let (name, body, site) = self.take_macro_definition()?;
        if self.purpose == ParserPurpose::InactiveSyntax {
            self.names
                .compile_local_rules(body, site)
                .map_err(|error| self.driver_error(error.into()))?;
            return Ok(());
        }
        let function = self.lexical.function.as_mut().ok_or_else(|| {
            native_adapter::transport_error(super::token_session::TokenError::InvalidHandle)
        })?;
        let driver = function.driver.as_mut().ok_or_else(|| {
            native_adapter::transport_error(super::token_session::TokenError::InvalidHandle)
        })?;
        let result = driver.define(name, body, site, function.frontier, self.names);
        result.map_err(|error| self.driver_error(error))
    }
    fn take_macro_definition(
        &mut self,
    ) -> Result<
        (
            AstIdent,
            super::macro_cursor::TokenSlice,
            super::macro_rules::DefinitionSite,
        ),
        ParseError,
    > {
        self.expect("macro_rules")?;
        self.expect("!")?;
        let span = self.token().span;
        let name = self.ident()?;
        let braced = self.is("{");
        let body = self.macro_group()?;
        if braced {
            self.eat(";");
        } else {
            self.expect(";")?;
        }
        let original = self
            .names
            .session()
            .original(span, 64)
            .map_err(native_adapter::transport_error)?;
        let rendered = self
            .names
            .session()
            .sources()
            .render_span(original)
            .map_err(native_adapter::transport_error)?;
        let file = self
            .names
            .session()
            .sources()
            .file(rendered.file.0 as usize)
            .map_err(native_adapter::transport_error)?;
        let defining_crate = self
            .names
            .register_crate_root(file)
            .map_err(native_adapter::transport_error)?;
        let site = super::macro_rules::DefinitionSite {
            span,
            edition: self.names.edition(),
            defining_crate,
        };
        Ok((name, body, site))
    }
    fn local_macro_expression(&mut self) -> Result<Expr, ParseError> {
        if self.purpose == ParserPurpose::InactiveSyntax {
            let span = self.token().span;
            let path = self.path()?;
            self.expect("!")?;
            return Ok(Expr::UnexpandedMacro(self.inactive_invocation(path, span)?));
        }
        for spelling in ["print", "eprint", "eprintln"] {
            if self.builtin_macro_ahead(spelling)? {
                return self.output_expression(spelling);
            }
        }
        for spelling in ["panic", "unreachable", "todo", "unimplemented"] {
            if self.builtin_macro_ahead(spelling)? {
                return self.panic_expression(spelling);
            }
        }
        if self.builtin_macro_ahead("matches")? {
            return self.matches_expression();
        }
        if self.builtin_macro_ahead("assert")? {
            return self.assert_expression();
        }
        if self.builtin_macro_ahead("assert_eq")? {
            return self.assertion_expression(Some(BinaryOp::Eq));
        }
        if self.builtin_macro_ahead("assert_ne")? {
            return self.assertion_expression(Some(BinaryOp::Ne));
        }
        for (spelling, comparison) in [
            ("debug_assert", None),
            ("debug_assert_eq", Some(BinaryOp::Eq)),
            ("debug_assert_ne", Some(BinaryOp::Ne)),
        ] {
            if self.builtin_macro_ahead(spelling)? {
                return self.debug_assertion_expression(comparison);
            }
        }
        if self.item_expansion.is_some() {
            return Err(self.unsupported(
                "user macro expressions in item-generated functions are unsupported",
            ));
        }
        let (buffer, admission, depth) = self.prepare_local_macro()?;
        self.isolated_expression_at(&buffer, Some(admission), depth)
    }
    /// Select the builtin only after checked lexical and module lookup miss.
    fn builtin_macro_ahead(&mut self, spelling: &str) -> Result<bool, ParseError> {
        let token = self.token();
        let name = self.input.mint_name(&token, self.names)?;
        let key = self
            .names
            .key(name, LookupClass::Item(Namespace::Macro))
            .map_err(native_adapter::name_error)?;
        let builtin_site = panic_callsite(self.names.session(), token.span)
            .map_err(native_adapter::transport_error)?;
        let builtin = self
            .names
            .intern_root(spelling, builtin_site)
            .map_err(native_adapter::name_error)?;
        let builtin = self
            .names
            .key(builtin, LookupClass::Item(Namespace::Macro))
            .map_err(native_adapter::name_error)?;
        if key != builtin {
            return Ok(false);
        }
        if let (Some(function), Some(scopes)) = (&self.lexical.function, &self.lexical.scopes)
            && let Some(driver) = &function.driver
            && driver
                .lookup(name, function.frontier, scopes, self.names)
                .map_err(|error| self.driver_error(error))?
                .is_some()
        {
            return Ok(false);
        }
        Ok(!self.known_module_macro(name)?)
    }
    /// Lower through the existing bounded pattern grammar and source Match.
    fn matches_expression(&mut self) -> Result<Expr, ParseError> {
        let site = self.token().span;
        let original = self
            .names
            .session()
            .original(
                panic_callsite(self.names.session(), site)
                    .map_err(native_adapter::transport_error)?,
                512,
            )
            .map_err(native_adapter::transport_error)?;
        self.names
            .session()
            .sources()
            .location(original)
            .map_err(native_adapter::transport_error)?;
        self.ident()?;
        self.expect("!")?;
        let close = if self.eat("(") {
            ")"
        } else if self.eat("[") {
            "]"
        } else if self.eat("{") {
            "}"
        } else {
            return Err(self.error("expected a matches! delimiter"));
        };
        let scrutinee = self.nested_expr()?;
        self.expect(",")?;
        let pattern = self.pattern()?;
        let guarded = self.eat("if");
        let body = if guarded {
            let frontier = self.enter_lexical()?;
            let guard = (|| {
                self.bind_pattern_lexical(&pattern)?;
                self.nested_expr()
            })();
            self.restore_frontier(frontier);
            let guard = guard?;
            self.names
                .budget()
                .charge(super::expansion_budget::Counter::Work, 3)
                .map_err(|error| native_adapter::transport_error(error.into()))?;
            Expr::If {
                condition: self
                    .names
                    .try_parser_box(guard)
                    .map_err(native_adapter::name_error)?,
                then_block: Block {
                    statements: Vec::new(),
                    tail: Some(
                        self.names
                            .try_parser_box(Expr::Bool(true))
                            .map_err(native_adapter::name_error)?,
                    ),
                },
                else_block: Block {
                    statements: Vec::new(),
                    tail: Some(
                        self.names
                            .try_parser_box(Expr::Bool(false))
                            .map_err(native_adapter::name_error)?,
                    ),
                },
            }
        } else {
            Expr::Bool(true)
        };
        self.eat(",");
        self.expect(close)?;
        let scrutinee = self
            .names
            .try_parser_box(scrutinee)
            .map_err(native_adapter::name_error)?;
        let capacity = 2 * std::mem::size_of::<MatchArm>();
        self.names
            .budget()
            .charge(super::expansion_budget::Counter::Work, 2)
            .map_err(|error| native_adapter::transport_error(error.into()))?;
        self.names
            .retain_parser_text(capacity)
            .map_err(native_adapter::name_error)?;
        let mut arms = Vec::new();
        arms.try_reserve_exact(2).map_err(|_| {
            native_adapter::transport_error(self.names.budget().poison_allocation().into())
        })?;
        let extra = (arms.capacity() - 2) * std::mem::size_of::<MatchArm>();
        self.names
            .retain_parser_text(extra)
            .map_err(native_adapter::name_error)?;
        arms.push(MatchArm {
            pattern,
            body,
            guarded,
        });
        arms.push(MatchArm {
            pattern: Pattern::Wildcard,
            body: Expr::Bool(false),
            guarded: false,
        });
        let expression = Expr::Match { scrutinee, arms };
        validate_depth(DepthNode::Expr(&expression)).map_err(|e| self.depth_error(&e))?;
        Ok(expression)
    }
    fn output_expression(&mut self, spelling: &'static str) -> Result<Expr, ParseError> {
        let site = self.token().span;
        let original = self
            .names
            .session()
            .original(
                panic_callsite(self.names.session(), site)
                    .map_err(native_adapter::transport_error)?,
                512,
            )
            .map_err(native_adapter::transport_error)?;
        self.names
            .session()
            .sources()
            .location(original)
            .map_err(native_adapter::transport_error)?;
        self.ident()?;
        self.expect("!")?;
        let close = if self.eat("(") {
            ")"
        } else if self.eat("[") {
            "]"
        } else if self.eat("{") {
            "}"
        } else {
            return Err(self.error("expected an output macro delimiter"));
        };
        if !self.is(close) {
            return self.scalar_output(close, spelling, Some(site));
        }
        let (nonliteral, placeholders, arguments) = match spelling {
            "print" => (
                "print! nonliteral arguments are unsupported",
                "print! formatting placeholders are unsupported",
                "print! formatting arguments are unsupported",
            ),
            "eprint" => (
                "eprint! nonliteral arguments are unsupported",
                "eprint! formatting placeholders are unsupported",
                "eprint! formatting arguments are unsupported",
            ),
            "eprintln" => (
                "eprintln! nonliteral arguments are unsupported",
                "eprintln! formatting placeholders are unsupported",
                "eprintln! formatting arguments are unsupported",
            ),
            _ => unreachable!("checked output builtin"),
        };
        let text = if self.is(close) && spelling == "eprintln" {
            String::new()
        } else {
            let text = self.literal_message(nonliteral, placeholders)?;
            self.eat(",");
            if !self.is(close) {
                return Err(self.unsupported(arguments));
            }
            text
        };
        self.expect(close)?;
        let payload = self
            .names
            .try_parser_box(SourceOutput {
                output: ir::LiteralOutput {
                    text,
                    stream: if spelling == "print" {
                        ir::OutputStream::Stdout
                    } else {
                        ir::OutputStream::Stderr
                    },
                    newline: spelling == "eprintln",
                },
                site,
            })
            .map_err(native_adapter::name_error)?;
        Ok(Expr::Output(payload))
    }
    fn scalar_output(
        &mut self,
        close: &str,
        spelling: &str,
        site: Option<super::expansion_span::SpanId>,
    ) -> Result<Expr, ParseError> {
        let buffer;
        let token = if self.token().is_opaque_literal() {
            buffer = self
                .input
                .opaque_literal_buffer(&self.token(), self.names)?;
            TokenCursor::new(&buffer).current()
        } else {
            self.token()
        };
        if !token.string {
            if spelling == "println" {
                return Err(self.error("println! requires one literal string"));
            }
            return Err(
                self.unsupported(&format!("{spelling}! nonliteral arguments are unsupported"))
            );
        }
        let text = self.string_literal_value(token.text)?;
        self.cursor.bump();
        let mut pieces = Vec::new();
        let mut piece = String::new();
        let mut chars = text.chars();
        while let Some(c) = chars.next() {
            match c {
                '{' => match chars.next() {
                    Some('{') => piece.push('{'),
                    Some('}') => {
                        pieces.push(std::mem::take(&mut piece));
                    }
                    None => return Err(self.error("unmatched opening brace in format string")),
                    _ => {
                        return Err(
                            self.unsupported("only bare {} formatting placeholders are supported")
                        );
                    }
                },
                '}' => {
                    if chars.next() == Some('}') {
                        piece.push('}');
                    } else {
                        return Err(self.error("unmatched closing brace in format string"));
                    }
                }
                _ => piece.push(c),
            }
        }
        pieces.push(piece);
        let mut arguments = Vec::new();
        while self.eat(",") && !self.is(close) {
            arguments.push(self.nested_expr()?);
        }
        self.expect(close)?;
        if pieces.len() != arguments.len() + 1 {
            return Err(self.error("format argument count mismatch"));
        }
        let stream = if matches!(spelling, "print" | "println") {
            ir::OutputStream::Stdout
        } else {
            ir::OutputStream::Stderr
        };
        let newline = matches!(spelling, "println" | "eprintln");
        if arguments.is_empty() {
            let text = pieces.pop().unwrap();
            return if let Some(site) = site {
                Ok(Expr::Output(
                    self.names
                        .try_parser_box(SourceOutput {
                            output: ir::LiteralOutput {
                                text,
                                stream,
                                newline,
                            },
                            site,
                        })
                        .map_err(native_adapter::name_error)?,
                ))
            } else {
                Ok(Expr::Println(text))
            };
        }
        self.names
            .retain_parser_text(
                pieces.len() * std::mem::size_of::<String>()
                    + arguments.len() * std::mem::size_of::<Expr>(),
            )
            .map_err(native_adapter::name_error)?;
        Ok(Expr::Format {
            pieces,
            arguments,
            stream,
            newline,
        })
    }
    fn panic_expression(&mut self, spelling: &'static str) -> Result<Expr, ParseError> {
        let (default, delimiter, nonliteral, placeholders, arguments) = match spelling {
            "panic" => (
                "explicit panic",
                "expected a panic! delimiter",
                "panic! nonliteral arguments are unsupported",
                "panic! formatting placeholders are unsupported",
                "panic! formatting arguments are unsupported",
            ),
            "unreachable" => (
                "internal error: entered unreachable code",
                "expected an unreachable! delimiter",
                "unreachable! nonliteral arguments are unsupported",
                "unreachable! formatting placeholders are unsupported",
                "unreachable! formatting arguments are unsupported",
            ),
            "todo" => (
                "not yet implemented",
                "expected a todo! delimiter",
                "todo! nonliteral arguments are unsupported",
                "todo! formatting placeholders are unsupported",
                "todo! formatting arguments are unsupported",
            ),
            "unimplemented" => (
                "not implemented",
                "expected an unimplemented! delimiter",
                "unimplemented! nonliteral arguments are unsupported",
                "unimplemented! formatting placeholders are unsupported",
                "unimplemented! formatting arguments are unsupported",
            ),
            _ => unreachable!("checked panic builtin"),
        };
        let site = self.token().span;
        let original = self
            .names
            .session()
            .original(
                panic_callsite(self.names.session(), site)
                    .map_err(native_adapter::transport_error)?,
                512,
            )
            .map_err(native_adapter::transport_error)?;
        self.names
            .session()
            .sources()
            .location(original)
            .map_err(native_adapter::transport_error)?;
        self.ident()?;
        self.expect("!")?;
        let close = if self.eat("(") {
            ")"
        } else if self.eat("[") {
            "]"
        } else if self.eat("{") {
            "}"
        } else {
            return Err(self.error(delimiter));
        };
        let message = if self.is(close) {
            self.prefixed_panic_message(default, None)?
        } else {
            let text = self.literal_message(nonliteral, placeholders)?;
            self.eat(",");
            if !self.is(close) {
                return Err(self.unsupported(arguments));
            }
            if spelling == "panic" {
                text
            } else {
                self.prefixed_panic_message(default, Some(&text))?
            }
        };
        self.expect(close)?;
        let payload = self
            .names
            .try_parser_box(SourcePanic { message, site })
            .map_err(native_adapter::name_error)?;
        Ok(Expr::Panic(payload))
    }
    /// Lower before source inventory: the condition remains one real source
    /// occurrence and the failure uses the checked assertion invocation site.
    #[inline(never)]
    fn assert_expression(&mut self) -> Result<Expr, ParseError> {
        self.assertion_expression(None)
    }
    /// Keep the ordinary assertion available for semantic checking even when
    /// disabled. The false source conditional evaluates no original argument.
    fn debug_assertion_expression(
        &mut self,
        comparison: Option<BinaryOp>,
    ) -> Result<Expr, ParseError> {
        let assertion = self.assertion_expression(comparison)?;
        if self.debug_assertions {
            return Ok(assertion);
        }
        self.names
            .budget()
            .charge(super::expansion_budget::Counter::Work, 4)
            .map_err(|error| native_adapter::transport_error(error.into()))?;
        let condition = self
            .names
            .try_parser_box(Expr::Bool(false))
            .map_err(native_adapter::name_error)?;
        let assertion = self
            .names
            .try_parser_box(assertion)
            .map_err(native_adapter::name_error)?;
        let expression = Expr::If {
            condition,
            then_block: Block {
                statements: Vec::new(),
                tail: Some(assertion),
            },
            else_block: Block {
                statements: Vec::new(),
                tail: None,
            },
        };
        validate_depth(DepthNode::Expr(&expression)).map_err(|e| self.depth_error(&e))?;
        Ok(expression)
    }
    fn assertion_expression(&mut self, comparison: Option<BinaryOp>) -> Result<Expr, ParseError> {
        let debug = self.token().matches("debug_assert")
            || self.token().matches("debug_assert_eq")
            || self.token().matches("debug_assert_ne");
        let (delimiter, operand, right_operand, separator) = match (comparison, debug) {
            (Some(BinaryOp::Eq), true) => (
                "expected a debug_assert_eq! delimiter",
                "debug_assert_eq! requires a left operand",
                "debug_assert_eq! requires a right operand",
                "expected comma or closing debug_assert_eq! delimiter",
            ),
            (Some(BinaryOp::Ne), true) => (
                "expected a debug_assert_ne! delimiter",
                "debug_assert_ne! requires a left operand",
                "debug_assert_ne! requires a right operand",
                "expected comma or closing debug_assert_ne! delimiter",
            ),
            (_, true) => (
                "expected a debug_assert! delimiter",
                "debug_assert! requires a condition",
                "debug_assert! requires a condition",
                "expected comma or closing debug_assert! delimiter",
            ),
            (Some(BinaryOp::Eq), false) => (
                "expected an assert_eq! delimiter",
                "assert_eq! requires a left operand",
                "assert_eq! requires a right operand",
                "expected comma or closing assert_eq! delimiter",
            ),
            (Some(BinaryOp::Ne), false) => (
                "expected an assert_ne! delimiter",
                "assert_ne! requires a left operand",
                "assert_ne! requires a right operand",
                "expected comma or closing assert_ne! delimiter",
            ),
            _ => (
                "expected an assert! delimiter",
                "assert! requires a condition",
                "assert! requires a condition",
                "expected comma or closing assert! delimiter",
            ),
        };
        let site = self.token().span;
        let original = self
            .names
            .session()
            .original(
                panic_callsite(self.names.session(), site)
                    .map_err(native_adapter::transport_error)?,
                512,
            )
            .map_err(native_adapter::transport_error)?;
        self.names
            .session()
            .sources()
            .location(original)
            .map_err(native_adapter::transport_error)?;
        self.ident()?;
        self.expect("!")?;
        let close = if self.eat("(") {
            ")"
        } else if self.eat("[") {
            "]"
        } else if self.eat("{") {
            "}"
        } else {
            return Err(self.error(delimiter));
        };
        if self.is(close) || self.is(",") {
            return Err(self.error(operand));
        }
        // Macro arguments are ordinary expressions, even inside an outer if
        // condition. In particular, do not inherit its struct-literal fence.
        let condition = self.nested_expr()?;
        let condition = if let Some(op) = comparison {
            self.expect(",")?;
            if self.is(close) || self.is(",") {
                return Err(self.error(right_operand));
            }
            let right = self.nested_expr()?;
            self.names
                .budget()
                .charge(super::expansion_budget::Counter::Work, 1)
                .map_err(|error| native_adapter::transport_error(error.into()))?;
            Expr::Binary {
                op,
                left: self
                    .names
                    .try_parser_box(condition)
                    .map_err(native_adapter::name_error)?,
                right: self
                    .names
                    .try_parser_box(right)
                    .map_err(native_adapter::name_error)?,
            }
        } else {
            condition
        };
        let (nonliteral, placeholders, arguments) = match (comparison, debug) {
            (Some(BinaryOp::Eq), true) => (
                "debug_assert_eq! nonliteral messages are unsupported",
                "debug_assert_eq! formatting placeholders are unsupported",
                "debug_assert_eq! formatting arguments are unsupported",
            ),
            (Some(BinaryOp::Ne), true) => (
                "debug_assert_ne! nonliteral messages are unsupported",
                "debug_assert_ne! formatting placeholders are unsupported",
                "debug_assert_ne! formatting arguments are unsupported",
            ),
            (_, true) => (
                "debug_assert! nonliteral messages are unsupported",
                "debug_assert! formatting placeholders are unsupported",
                "debug_assert! formatting arguments are unsupported",
            ),
            (Some(BinaryOp::Eq), false) => (
                "assert_eq! nonliteral messages are unsupported",
                "assert_eq! formatting placeholders are unsupported",
                "assert_eq! formatting arguments are unsupported",
            ),
            (Some(BinaryOp::Ne), false) => (
                "assert_ne! nonliteral messages are unsupported",
                "assert_ne! formatting placeholders are unsupported",
                "assert_ne! formatting arguments are unsupported",
            ),
            _ => (
                "assert! nonliteral messages are unsupported",
                "assert! formatting placeholders are unsupported",
                "assert! formatting arguments are unsupported",
            ),
        };
        let message = if self.eat(",") && !self.is(close) {
            if !self.token().string && !self.token().is_opaque_literal() {
                return Err(self.unsupported(nonliteral));
            }
            let message = self.literal_message(nonliteral, placeholders)?;
            self.eat(",");
            if !self.is(close) {
                return Err(self.unsupported(arguments));
            }
            message
        } else {
            if !self.is(close) {
                return Err(self.error(separator));
            }
            let message = match comparison {
                Some(BinaryOp::Eq) => "assertion failed: left == right",
                Some(BinaryOp::Ne) => "assertion failed: left != right",
                _ => "assertion failed",
            };
            self.names
                .retain_parser_text(message.len())
                .map_err(native_adapter::name_error)?;
            let mut retained = String::new();
            retained.try_reserve_exact(message.len()).map_err(|_| {
                native_adapter::transport_error(self.names.budget().poison_allocation().into())
            })?;
            if retained.capacity() > message.len() {
                self.names
                    .retain_parser_text(retained.capacity() - message.len())
                    .map_err(native_adapter::name_error)?;
            }
            retained.push_str(message);
            retained
        };
        self.expect(close)?;
        self.names
            .budget()
            .charge(super::expansion_budget::Counter::Work, 3)
            .map_err(|error| native_adapter::transport_error(error.into()))?;
        let condition = self
            .names
            .try_parser_box(condition)
            .map_err(native_adapter::name_error)?;
        let panic = self
            .names
            .try_parser_box(SourcePanic { message, site })
            .map_err(native_adapter::name_error)?;
        let failure = self
            .names
            .try_parser_box(Expr::Panic(panic))
            .map_err(native_adapter::name_error)?;
        let expression = Expr::If {
            condition,
            then_block: Block {
                statements: Vec::new(),
                tail: None,
            },
            else_block: Block {
                statements: Vec::new(),
                tail: Some(failure),
            },
        };
        validate_depth(DepthNode::Expr(&expression)).map_err(|e| self.depth_error(&e))?;
        Ok(expression)
    }
    fn known_module_macro(&self, name: AstIdent) -> Result<bool, ParseError> {
        let (Some(driver), Some(call), Some(scopes)) = (
            &self.lexical.item_macros,
            self.module_frontier,
            &self.lexical.scopes,
        ) else {
            return Ok(false);
        };
        driver
            .known(name, call, scopes, self.names)
            .map_err(|error| self.driver_error(error))
    }
    // Selection/lowering scratch is gone before recursively parsing the child.
    #[inline(never)]
    fn prepare_local_macro(
        &mut self,
    ) -> Result<
        (
            NativeTokenBuffer,
            super::semantic_scope::ExpansionAdmission,
            u32,
        ),
        ParseError,
    > {
        let span = self.token().span;
        if self.lexical.function.is_none() {
            let name = self.ident()?;
            if self.known_module_macro(name)? {
                return Err(
                    self.unsupported("module macros in expression position are unsupported")
                );
            }
            return Err(self.error("unknown function-local macro"));
        }
        let parent = if let Some(proof) = &self.expansion {
            proof
                .validate_span(self.names.session(), span)
                .map_err(scope_parse_error)?;
            Some(proof.expansion(self.names).map_err(scope_parse_error)?)
        } else {
            None
        };
        let depth = self.expansion_depth.checked_add(1).ok_or_else(|| {
            native_adapter::transport_error(super::token_session::TokenError::IdExhausted)
        })?;
        self.names
            .budget()
            .check_depth(super::expansion_budget::Depth::Expansion, depth)
            .map_err(|error| native_adapter::transport_error(error.into()))?;
        let name = self.ident()?;
        self.expect("!")?;
        let args = self.macro_group()?;
        let function = self
            .lexical
            .function
            .as_ref()
            .ok_or_else(|| self.unsupported("function-local macro context required"))?;
        let call = function.frontier;
        let scopes = self.lexical.scopes.as_ref().ok_or_else(|| {
            native_adapter::transport_error(super::token_session::TokenError::InvalidHandle)
        })?;
        let driver = function.driver.as_ref().ok_or_else(|| {
            self.unsupported("user macros in generated functions are unsupported")
        })?;
        let selected = match driver
            .lookup(name, call, scopes, self.names)
            .map_err(|error| self.driver_error(error))?
        {
            Some(selected) => selected,
            None => {
                if self.known_module_macro(name)? {
                    return Err(
                        self.unsupported("module macros in expression position are unsupported")
                    );
                }
                return Err(self.error("unknown function-local macro at this lexical point"));
            }
        };
        let expanded = driver
            .expand(
                selected,
                args,
                super::macro_rules::CallSite { span, parent },
                self.names,
            )
            .map_err(|error| self.driver_error(error))?;
        let expansion = expanded
            .expansion(self.names.session())
            .map_err(|error| self.driver_error(super::source_macro::DriverError::Macro(error)))?;
        let slice = expanded
            .tokens(self.names.session())
            .map_err(|error| self.driver_error(super::source_macro::DriverError::Macro(error)))?;
        let scopes = self.lexical.scopes.as_mut().ok_or_else(|| {
            native_adapter::transport_error(super::token_session::TokenError::InvalidHandle)
        })?;
        let admission = if let Some(parent) = &self.expansion {
            scopes.anchor_nested_expansion(
                self.names,
                expansion,
                selected.definition(),
                selected.call(),
                parent,
            )
        } else {
            scopes.anchor_expansion(
                self.names,
                expansion,
                selected.definition(),
                selected.call(),
            )
        }
        .map_err(scope_parse_error)?;
        let eof = self
            .names
            .expansion_eof(&admission)
            .map_err(scope_parse_error)?;
        let buffer =
            expanded_adapter::lower(slice, self.names, self.names.edition(), eof, &admission)?;
        Ok((buffer, admission, depth))
    }
    fn isolated_expression(&mut self, buffer: &NativeTokenBuffer) -> Result<Expr, ParseError> {
        self.isolated_expression_at(buffer, self.expansion, self.expansion_depth)
    }
    fn isolated_expression_at(
        &mut self,
        buffer: &NativeTokenBuffer,
        expansion: Option<super::semantic_scope::ExpansionAdmission>,
        expansion_depth: u32,
    ) -> Result<Expr, ParseError> {
        let mut child = Parser {
            debug_assertions: self.debug_assertions,
            input: buffer,
            names: &mut *self.names,
            modules: ParserState::Borrowed(&mut *self.modules),
            lexical: ParserState::Borrowed(&mut *self.lexical),
            expansion,
            expansion_depth,
            purpose: self.purpose,
            item_expansion: self.item_expansion,
            module_frontier: self.module_frontier,
            module: self.module,
            alias_target: false,
            pending_bounds: Vec::new(),
            impl_trait_params: None,
            cursor: TokenCursor::new(buffer),
            nesting: self.nesting,
            allow_struct: self.allow_struct,
            condition_purpose: self.condition_purpose,
            type_params: std::mem::take(&mut self.type_params),
            use_expansions: self.use_expansions,
            use_expansion_bytes: self.use_expansion_bytes,
        };
        let result = child.expr(0).and_then(|value| {
            if !child.is("") {
                Err(child.unsupported("macro output must be exactly one complete expression"))
            } else {
                Ok(value)
            }
        });
        self.type_params = std::mem::take(&mut child.type_params);
        result
    }
    fn opaque_literal_expression(&mut self) -> Result<Expr, ParseError> {
        let buffer = self
            .input
            .opaque_literal_buffer(&self.token(), self.names)?;
        let value = self.isolated_expression(&buffer)?;
        self.cursor.bump();
        Ok(value)
    }
    /// Charge the complete new allocation, including any allocator rounding.
    /// The decoded literal remains separately charged by `literal_message`.
    fn prefixed_panic_message(
        &mut self,
        prefix: &str,
        custom: Option<&str>,
    ) -> Result<String, ParseError> {
        let capacity = custom
            .map_or(Some(prefix.len()), |text| {
                prefix.len().checked_add(2)?.checked_add(text.len())
            })
            .ok_or_else(|| {
                native_adapter::transport_error(self.names.budget().poison_allocation().into())
            })?;
        self.names
            .budget()
            .charge(super::expansion_budget::Counter::Work, capacity as u64)
            .map_err(|error| native_adapter::transport_error(error.into()))?;
        self.names
            .retain_parser_text(capacity)
            .map_err(native_adapter::name_error)?;
        let mut message = String::new();
        message.try_reserve_exact(capacity).map_err(|_| {
            native_adapter::transport_error(self.names.budget().poison_allocation().into())
        })?;
        if message.capacity() > capacity {
            self.names
                .retain_parser_text(message.capacity() - capacity)
                .map_err(native_adapter::name_error)?;
        }
        message.push_str(prefix);
        if let Some(text) = custom {
            message.push_str(": ");
            message.push_str(text);
        }
        Ok(message)
    }
    /// Copy a decoded string literal into charged parser storage.
    fn string_literal_value(&mut self, text: &str) -> Result<String, ParseError> {
        self.names
            .budget()
            .charge(super::expansion_budget::Counter::Work, text.len() as u64)
            .map_err(|error| native_adapter::transport_error(error.into()))?;
        self.names
            .retain_parser_text(text.len())
            .map_err(native_adapter::name_error)?;
        Ok(text.to_owned())
    }
    fn literal_message(
        &mut self,
        nonliteral: &'static str,
        formatting: &'static str,
    ) -> Result<String, ParseError> {
        let buffer;
        let token = if self.token().is_opaque_literal() {
            buffer = self
                .input
                .opaque_literal_buffer(&self.token(), self.names)?;
            TokenCursor::new(&buffer).current()
        } else {
            self.token()
        };
        if !token.string {
            return Err(self.unsupported(nonliteral));
        }
        let text = literal_format_owned(token.text, self.names).map_err(|error| {
            if error.kind == super::source::SourceErrorKind::Syntax {
                self.unsupported(formatting)
            } else {
                error
            }
        })?;
        self.cursor.bump();
        Ok(text)
    }
    fn enter_lexical(&mut self) -> Result<Option<super::semantic_scope::FrontierId>, ParseError> {
        let previous = self.frontier();
        if let Some(frontier) = previous {
            let lexical = &mut *self.lexical;
            let next = lexical
                .scopes
                .as_mut()
                .ok_or_else(|| {
                    native_adapter::transport_error(super::token_session::TokenError::InvalidInput(
                        "missing parser lexical owner",
                    ))
                })?
                .enter(self.names, frontier)
                .map_err(scope_parse_error)?;
            lexical
                .function
                .as_mut()
                .ok_or_else(|| {
                    native_adapter::transport_error(super::token_session::TokenError::InvalidInput(
                        "missing parser lexical owner",
                    ))
                })?
                .frontier = next;
        }
        Ok(previous)
    }
    fn restore_frontier(&mut self, previous: Option<super::semantic_scope::FrontierId>) {
        if let (Some(function), Some(previous)) = (self.lexical.function.as_mut(), previous) {
            function.frontier = previous;
        }
    }
    fn bind_lexical(&mut self, ident: AstIdent) -> Result<(), ParseError> {
        if let Some(frontier) = self.frontier() {
            let lexical = &mut *self.lexical;
            let (next, _) = lexical
                .scopes
                .as_mut()
                .ok_or_else(|| {
                    native_adapter::transport_error(super::token_session::TokenError::InvalidInput(
                        "missing parser lexical owner",
                    ))
                })?
                .bind(self.names, frontier, ident)
                .map_err(scope_parse_error)?;
            lexical
                .function
                .as_mut()
                .ok_or_else(|| {
                    native_adapter::transport_error(super::token_session::TokenError::InvalidInput(
                        "missing parser lexical owner",
                    ))
                })?
                .frontier = next;
        }
        Ok(())
    }
    fn reference_lexical(&mut self, ident: AstIdent) -> Result<(), ParseError> {
        if let Some(frontier) = self.frontier() {
            self.lexical
                .scopes
                .as_mut()
                .ok_or_else(|| {
                    native_adapter::transport_error(super::token_session::TokenError::InvalidInput(
                        "missing parser lexical owner",
                    ))
                })?
                .resolve_reference(self.names, frontier, ident)
                .map_err(scope_parse_error)?;
        }
        Ok(())
    }
    fn bind_pattern_lexical(&mut self, pattern: &Pattern) -> Result<(), ParseError> {
        if self.frontier().is_none() {
            return Ok(());
        }
        let mut pending = super::token_session::BudgetVec::new(
            self.names.budget(),
            super::expansion_budget::Storage::Temporary,
        )
        .map_err(native_adapter::transport_error)?;
        pending
            .push(pattern)
            .map_err(native_adapter::transport_error)?;
        while let Some(pattern) = pending.pop() {
            self.names
                .budget()
                .charge(super::expansion_budget::Counter::Work, 1)
                .map_err(|error| {
                    native_adapter::transport_error(super::token_session::TokenError::Budget(error))
                })?;
            match pattern {
                Pattern::Binding { name, .. } => self.bind_lexical(*name)?,
                Pattern::Tuple(fields) => {
                    for field in fields {
                        pending
                            .push(field)
                            .map_err(native_adapter::transport_error)?;
                    }
                }
                Pattern::Array(fields) => {
                    for field in fields.iter().rev() {
                        pending
                            .push(field)
                            .map_err(native_adapter::transport_error)?;
                    }
                }
                Pattern::TupleRest(fields)
                | Pattern::ArrayRest(fields)
                | Pattern::VariantRest { fields, .. } => {
                    // LIFO insertion retains written prefix-then-suffix order.
                    for field in fields.prefix.iter().chain(&fields.suffix).rev() {
                        pending
                            .push(field)
                            .map_err(native_adapter::transport_error)?;
                    }
                }
                Pattern::Struct { fields, .. } => {
                    for (_, field) in fields {
                        pending
                            .push(field)
                            .map_err(native_adapter::transport_error)?;
                    }
                }
                Pattern::Variant { fields, .. } => {
                    for (_, field) in fields {
                        pending
                            .push(field)
                            .map_err(native_adapter::transport_error)?;
                    }
                }
                Pattern::Apply { pattern, .. } | Pattern::Ref { pattern, .. } => pending
                    .push(pattern)
                    .map_err(native_adapter::transport_error)?,
                _ => {}
            }
        }
        Ok(())
    }
    fn function_body(
        &mut self,
        params: &[(BindingName, Type)],
    ) -> Result<(Block, Option<super::semantic_scope::ScopeId>), ParseError> {
        if self.purpose == ParserPurpose::InactiveSyntax {
            return Ok((self.block()?, None));
        }
        let Some(source) = self.input.macro_source().cloned() else {
            return Ok((self.block()?, None));
        };
        let mut probe = TokenCursor::new(self.input);
        if !probe.restore(self.cursor.checkpoint()) {
            return Err(native_adapter::transport_error(
                super::token_session::TokenError::InvalidHandle,
            ));
        }
        let mut depth = 0usize;
        let mut found = false;
        loop {
            self.names
                .budget()
                .charge(super::expansion_budget::Counter::Work, 1)
                .map_err(|error| {
                    native_adapter::transport_error(super::token_session::TokenError::Budget(error))
                })?;
            let token = probe.current();
            if token.matches("") {
                break;
            }
            if token.matches("macro_rules") && probe.peek(1).is_some_and(|token| token.matches("!"))
            {
                found = true;
                break;
            }
            if token.matches("{") {
                depth += 1;
            }
            if token.matches("}") {
                if depth <= 1 {
                    break;
                }
                depth -= 1;
            }
            probe.bump();
        }
        if !found {
            return Ok((self.block()?, None));
        }
        if self.expansion.is_some() {
            return Err(self.unsupported("nested source macro definitions are unsupported"));
        }
        if self.lexical.scopes.is_none() {
            self.lexical.scopes = Some(
                super::semantic_scope::ScopeArenaBuilder::for_names(self.names)
                    .map_err(scope_parse_error)?,
            );
        }
        let scopes = self.lexical.scopes.as_mut().ok_or_else(|| {
            native_adapter::transport_error(super::token_session::TokenError::InvalidInput(
                "missing parser lexical owner",
            ))
        })?;
        let frontier = scopes
            .function_root(self.names)
            .map_err(scope_parse_error)?;
        let owner = scopes.function(frontier).map_err(scope_parse_error)?;
        let driver = super::source_macro::LocalMacroDriver::new(source, owner, self.names)
            .map_err(|error| self.driver_error(error))?;
        let previous = self.lexical.function.replace(FunctionMacroState {
            frontier,
            driver: Some(driver),
        });
        for (name, _) in params {
            if let BindingName::Named(name) = name {
                self.bind_lexical(*name)?;
            }
        }
        let body = self.block();
        self.lexical.function = previous;
        Ok((body?, Some(owner)))
    }
    fn new(
        tokens: &'input NativeTokenBuffer,
        names: &'names mut NameStoreBuilder,
    ) -> Result<Self, ParseError> {
        Self::with_purpose(tokens, names, ParserPurpose::Active)
    }
    fn with_purpose(
        tokens: &'input NativeTokenBuffer,
        names: &'names mut NameStoreBuilder,
        purpose: ParserPurpose,
    ) -> Result<Self, ParseError> {
        let modules = ModuleArenaBuilder::for_source(names).map_err(module_parse_error)?;
        let module = modules.source_root().map_err(module_parse_error)?;
        let modules = ParserState::owned(modules, names.budget())?;
        let mut state = ParserLexicalState::default();
        if purpose == ParserPurpose::InactiveSyntax {
            state.inactive_items = Some(
                super::token_session::BudgetVec::new(
                    names.budget(),
                    super::expansion_budget::Storage::Retained,
                )
                .map_err(native_adapter::transport_error)?,
            );
        }
        let lexical = ParserState::owned(state, names.budget())?;
        Ok(Self {
            debug_assertions: super::source::CfgSet::DEFAULT_DEBUG_ASSERTIONS,
            input: tokens,
            names,
            modules,
            lexical,
            expansion: None,
            expansion_depth: 0,
            purpose,
            item_expansion: None,
            module_frontier: None,
            module,
            alias_target: false,
            pending_bounds: Vec::new(),
            impl_trait_params: None,
            cursor: TokenCursor::new(tokens),
            nesting: 0,
            allow_struct: true,
            condition_purpose: ConditionPurpose::Ordinary,
            type_params: Vec::new(),
            use_expansions: 0,
            use_expansion_bytes: 0,
        })
    }
    fn token(&self) -> TokenView<'input> {
        self.cursor.current()
    }
    fn is(&self, s: &str) -> bool {
        self.token().matches(s)
    }
    fn eat(&mut self, s: &str) -> bool {
        if self.is(s) {
            self.cursor.bump();
            true
        } else {
            false
        }
    }
    fn error(&self, message: &str) -> ParseError {
        self.error_at(&self.token(), message)
    }
    fn error_at(&self, token: &TokenView<'_>, message: &str) -> ParseError {
        let kind = if token.is_deferred_unsupported() {
            super::source::SourceErrorKind::Unsupported
        } else {
            super::source::SourceErrorKind::Syntax
        };
        if token.provenance == TokenProvenance::Expanded {
            return native_adapter::expanded_error(
                self.names.session(),
                token.span,
                kind,
                &format!("{message}, found {:?}", token.text),
            );
        }
        let primary = match native_adapter::original(self.names.session(), token.span) {
            Ok(primary) => Some(primary),
            Err(error) => return error,
        };
        ParseError {
            _diagnostic_lease: None,
            primary,
            kind,
            has_location: true,
            bytes: token.bytes.clone(),
            line: token.line,
            column: token.column,
            message: format!("{message}, found {:?}", token.text),
        }
    }
    fn limit(&self, message: &str) -> ParseError {
        let mut error = self.error(message);
        if matches!(
            error.kind,
            super::source::SourceErrorKind::Syntax | super::source::SourceErrorKind::Unsupported
        ) {
            error.kind = super::source::SourceErrorKind::Limit;
        }
        error
    }
    fn depth_error(&self, message: &str) -> ParseError {
        let mut error = self.limit(message);
        error.has_location = false;
        error.primary = None;
        error
    }
    fn unsupported(&self, message: &str) -> ParseError {
        let mut error = self.error(message);
        if error.kind == super::source::SourceErrorKind::Syntax {
            error.kind = super::source::SourceErrorKind::Unsupported;
        }
        error
    }
    fn expect(&mut self, s: &str) -> Result<(), ParseError> {
        if self.eat(s) {
            Ok(())
        } else {
            Err(self.error(&format!("expected {s:?}")))
        }
    }
    #[inline(never)]
    fn is_identifier(&self) -> bool {
        let t = self.token();
        let reserved = [
            "fn", "let", "mut", "if", "else", "while", "return", "true", "false", "pub", "const",
            "static", "loop", "break", "continue", "for", "in", "match", "struct", "enum", "use",
            "mod", "impl", "trait", "self", "Self", "as", "async", "await", "move", "unsafe",
            "where", "type", "ref", "extern", "crate", "super", "dyn", "abstract", "become", "box",
            "do", "final", "macro", "override", "priv", "typeof", "unsized", "virtual", "yield",
            "try", "gen",
        ];
        t.is_ident()
            && t.text != "_"
            && t.text
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && !reserved.contains(&t.text)
    }
    fn ident(&mut self) -> Result<AstIdent, ParseError> {
        let t = self.token();
        if self.is_identifier() {
            let name = self.input.mint_name(&t, self.names)?;
            self.cursor.bump();
            Ok(name)
        } else {
            Err(self.error("expected ASCII identifier"))
        }
    }
    fn binding(&mut self) -> Result<BindingName, ParseError> {
        if self.eat("_") {
            Ok(BindingName::Wildcard)
        } else {
            self.ident().map(BindingName::Named)
        }
    }
    fn name_key(&self, name: AstIdent, class: LookupClass) -> Result<NameKey, ParseError> {
        self.names
            .key(name, class)
            .map_err(native_adapter::name_error)
    }
    fn contains_name(
        &self,
        list: &[AstIdent],
        name: AstIdent,
        class: LookupClass,
    ) -> Result<bool, ParseError> {
        let key = self.name_key(name, class)?;
        for existing in list {
            if self.name_key(*existing, class)? == key {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn current_type_parameter(&mut self) -> Result<Option<AstIdent>, ParseError> {
        if !self.token().is_ident() || self.token().text == "_" || self.type_params.is_empty() {
            return Ok(None);
        }
        let name = self.input.mint_name(&self.token(), self.names)?;
        Ok(self
            .contains_name(&self.type_params, name, LookupClass::TypeParameter)?
            .then_some(name))
    }
    fn ty(&mut self) -> Result<Type, ParseError> {
        if self.nesting >= 128 {
            return Err(self.limit("type nesting exceeds limit 128"));
        }
        self.nesting += 1;
        let result = self.ty_inner();
        self.nesting -= 1;
        result
    }
    fn ty_inner(&mut self) -> Result<Type, ParseError> {
        if self.token().string {
            return Err(self.error("quoted tokens are not types"));
        }
        if self.cursor.consume_type_amp_prefix() {
            return Ok(Type::Ref {
                kind: ir::BorrowKind::Shared,
                region: None,
                referent: Box::new(self.ty()?),
            });
        }
        if self.alias_target && self.is("_") {
            return Err(self.error("E0121: placeholder _ is not allowed in type alias signatures"));
        }
        if self.is("_") || self.is("!") {
            return Err(
                self.unsupported("inferred and never source type annotations are unsupported")
            );
        }
        if self.eat("&") {
            let region = if self.token().is_lifetime()
                && self.token().text == "'static"
                && self
                    .cursor
                    .peek(1)
                    .is_some_and(|token| token.matches("str"))
            {
                // Only `&'static str` (program data) is admitted. Other
                // `'static` references keep the explicit parser boundary.
                self.cursor.bump();
                Some(RegionName::Static)
            } else if self.token().is_lifetime() {
                Some(self.lifetime()?)
            } else {
                None
            };
            let kind = if self.eat("mut") {
                ir::BorrowKind::Mut
            } else {
                ir::BorrowKind::Shared
            };
            return Ok(Type::Ref {
                kind,
                region,
                referent: Box::new(self.ty()?),
            });
        }
        if !self.token().string && self.is("*") {
            self.cursor.bump();
            let kind = if self.eat("mut") {
                ir::BorrowKind::Mut
            } else if self.eat("const") {
                ir::BorrowKind::Shared
            } else {
                return Err(self.error("expected mut or const after * in raw pointer type"));
            };
            return Ok(Type::RawPtr {
                kind,
                pointee: Box::new(self.ty()?),
            });
        }
        if self.is("<") || self.is("<<") || self.is("<<=") {
            return self.qualified_type_projection();
        }
        if self.is("impl") {
            return self.impl_trait_param();
        }
        if self.eat("[") {
            let element = self.ty()?;
            if self.eat("]") {
                return Ok(Type::Slice(Box::new(element)));
            }
            self.expect(";")?;
            let len = self.array_number("array length")?;
            self.expect("]")?;
            Ok(Type::Array(Box::new(element), len))
        } else if self.eat("Self") {
            if self.eat("::") {
                let item = self.ident()?;
                return self.self_type_projection(item);
            }
            Ok(Type::SelfType)
        } else if let Some(name) = self.current_type_parameter()? {
            self.cursor.bump();
            if self.is("::") {
                return Err(self.unsupported("associated type projections are unsupported"));
            }
            Ok(Type::Param(name))
        } else if let Some(ty) = integer_type(self.token().text) {
            self.cursor.bump();
            Ok(Type::Int(ty))
        } else if let Some(ty) = float_type(self.token().text) {
            self.cursor.bump();
            Ok(Type::Float(ty))
        } else if self.eat("char") {
            Ok(Type::Char)
        } else if self.eat("str") {
            Ok(Type::Str)
        } else if self.eat("bool") {
            Ok(Type::Bool)
        } else if self.eat("(") {
            if self.eat(")") {
                return Ok(Type::Unit);
            }
            let first = self.ty()?;
            if !self.eat(",") {
                self.expect(")")?;
                return Ok(first);
            }
            let mut fields = vec![first];
            while !self.is(")") {
                fields.push(self.ty()?);
                if !self.eat(",") {
                    break;
                }
            }
            self.expect(")")?;
            Ok(Type::Tuple(fields))
        } else {
            let path = self.generic_path(true)?;
            if path
                .segments
                .iter()
                .any(|s| !s.args.is_empty() || !s.associated_types.is_empty())
            {
                Ok(Type::Apply(path))
            } else {
                Ok(Type::Named(
                    path.checked_path(self.names)
                        .map_err(native_adapter::name_error)?,
                ))
            }
        }
    }
    /// Argument-position `impl Bound + ...`: an anonymous, unnameable callable
    /// type parameter carrying the written bounds (stock semantics). Every other
    /// position is rejected like stock E0562.
    #[inline(never)]
    fn impl_trait_param(&mut self) -> Result<Type, ParseError> {
        let token = self.token();
        let Some(mut minted) = self.impl_trait_params.take() else {
            return Err(self.error(
                "E0562: `impl Trait` is only allowed in function parameter and return types",
            ));
        };
        if minted.len() >= 32 {
            self.impl_trait_params = Some(minted);
            return Err(self.limit("generic parameter resource limit 32"));
        }
        // The keyword occurrence's own span anchors the anonymous parameter.
        // Its spelling cannot be written by source as an identifier.
        let site = panic_callsite(self.names.session(), token.span)
            .map_err(native_adapter::transport_error)?;
        let spelling = format!("impl_{}", minted.len());
        let name = self
            .names
            .intern_root(&spelling, site)
            .map_err(native_adapter::name_error)?;
        self.cursor.bump();
        let mut traits = Vec::new();
        let parsed = (|| {
            loop {
                if traits.len() >= 256 {
                    return Err(self.limit("predicate limit exceeded"));
                }
                if self.token().is_lifetime() {
                    return Err(self.unsupported("lifetime bounds on `impl Trait` are unsupported"));
                }
                let path = self.trait_path()?;
                self.reserve_parser_items(&mut traits, 1)?;
                traits.push(path);
                if !self.eat("+") {
                    break;
                }
            }
            Ok(())
        })();
        if let Err(error) = parsed {
            self.impl_trait_params = Some(minted);
            return Err(error);
        }
        self.reserve_parser_items(&mut minted, 1)?;
        minted.push(name);
        self.impl_trait_params = Some(minted);
        let mut bounds = std::mem::take(&mut self.pending_bounds);
        self.reserve_parser_items(&mut bounds, 1)?;
        bounds.push(TypeBound {
            parameter: name,
            traits,
        });
        self.pending_bounds = bounds;
        Ok(Type::Param(name))
    }
    #[inline(never)]
    fn self_type_projection(&mut self, item: AstIdent) -> Result<Type, ParseError> {
        self.box_type_projection(SourceProjection::SelfItem { item })
    }
    #[inline(never)]
    fn box_type_projection(&mut self, projection: SourceProjection) -> Result<Type, ParseError> {
        self.names
            .retain_parser_storage(std::mem::size_of::<SourceProjection>())
            .map_err(native_adapter::name_error)?;
        Ok(Type::Projection(Box::new(projection)))
    }
    #[inline(never)]
    fn qualified_type_projection(&mut self) -> Result<Type, ParseError> {
        if !self.cursor.consume_type_lt() {
            return Err(self.error("expected qualified type opening <"));
        }
        let self_ty = self.ty()?;
        self.expect("as")?;
        let trait_path = self.trait_path()?;
        self.close_angle()?;
        self.expect("::")?;
        let item = self.ident()?;
        self.box_type_projection(SourceProjection::Qualified {
            self_ty,
            trait_path,
            item,
        })
    }
    /// Split closing angle tokens only while parsing types, never expressions.
    fn close_angle(&mut self) -> Result<(), ParseError> {
        if self.cursor.consume_type_gt() {
            Ok(())
        } else {
            Err(self.error("expected generic closing >"))
        }
    }

    #[inline(never)]
    fn type_arguments(&mut self) -> Result<Vec<Type>, ParseError> {
        let (args, associated_types) = self.path_arguments()?;
        if !associated_types.is_empty() {
            return Err(
                self.unsupported("associated equalities are unsupported in method type arguments")
            );
        }
        Ok(args)
    }
    #[inline(never)]
    fn path_arguments(&mut self) -> Result<(Vec<Type>, Vec<AssociatedTypeBinding>), ParseError> {
        self.expect("<")?;
        let mut args = Vec::new();
        let mut associated_types = Vec::new();
        loop {
            if args.len() + associated_types.len() >= 32 {
                return Err(self.limit("generic argument resource limit 32"));
            }
            if self.token().is_ident()
                && self.cursor.peek(1).is_some_and(|token| token.matches("="))
            {
                let name = self.ident()?;
                self.expect("=")?;
                let value = self.ty()?;
                self.reserve_parser_items(&mut associated_types, 1)?;
                associated_types.push(AssociatedTypeBinding { name, value });
            } else {
                if !associated_types.is_empty() {
                    return Err(
                        self.error("positional type arguments must precede associated equalities")
                    );
                }
                let value = self.ty()?;
                self.reserve_parser_items(&mut args, 1)?;
                args.push(value);
            }
            if !self.eat(",") || self.token().text.starts_with('>') {
                break;
            }
        }
        self.close_angle()?;
        Ok((args, associated_types))
    }
    #[inline(never)]
    fn generic_path(&mut self, type_context: bool) -> Result<GenericPath, ParseError> {
        let anchor = self.path_anchor()?;
        let mut segments = Vec::new();
        let mut count = match anchor {
            PathAnchor::Relative => 0,
            PathAnchor::Super(n) => usize::from(n),
            _ => 1,
        };
        if anchor != PathAnchor::Relative && !self.eat("::") {
            return Ok(GenericPath {
                anchor,
                segments,
                resolved_use: None,
            });
        }
        loop {
            if count >= 128 {
                return Err(self.limit("path depth resource limit 128"));
            }
            count += 1;
            let name = self.ident()?;
            let (args, associated_types) = if type_context && self.is("<") {
                self.path_arguments()?
            } else {
                (Vec::new(), Vec::new())
            };
            self.reserve_parser_items(&mut segments, 1)?;
            segments.push(PathSegment {
                name,
                args,
                associated_types,
            });
            if !self.eat("::") {
                break;
            }
            if self.is("<") {
                let (args, associated_types) = self.path_arguments()?;
                if !segments.last().unwrap().args.is_empty()
                    || !segments.last().unwrap().associated_types.is_empty()
                {
                    return Err(self.error("duplicate generic arguments"));
                }
                segments.last_mut().unwrap().args = args;
                segments.last_mut().unwrap().associated_types = associated_types;
                if !self.eat("::") {
                    break;
                }
            }
        }
        Ok(GenericPath {
            anchor,
            segments,
            resolved_use: None,
        })
    }
    #[inline(never)]
    fn type_parameters(&mut self) -> Result<Vec<AstIdent>, ParseError> {
        self.pending_bounds.clear();
        let declarations = self.predeclare_type_parameters()?;
        if !self.eat("<") {
            return Ok(Vec::new());
        }
        self.pending_bounds.clear();
        let mut params = Vec::new();
        loop {
            if params.len() >= 32 {
                return Err(self.limit("generic parameter resource limit 32"));
            }
            if self.is("const") {
                return Err(self.unsupported("const generic parameters are unsupported"));
            }
            if self.token().is_lifetime() {
                self.lifetime()?;
                return Err(self.error(
                    "lifetime parameters on nominal and impl declarations are unsupported",
                ));
            }
            let name = self.predeclared_parameter(&declarations, params.len())?;
            if self.contains_name(&params, name, LookupClass::TypeParameter)? {
                return Err(self.error("duplicate type parameter"));
            }
            self.reserve_parser_items(&mut params, 1)?;
            params.push(name);
            if self.eat(":") {
                let mut traits = Vec::new();
                loop {
                    let path = self.trait_path()?;
                    self.reserve_parser_items(&mut traits, 1)?;
                    traits.push(path);
                    if traits.len() > 256 {
                        return Err(self.limit("predicate limit exceeded"));
                    }
                    if !self.eat("+") {
                        break;
                    }
                }
                let mut bounds = std::mem::take(&mut self.pending_bounds);
                self.reserve_parser_items(&mut bounds, 1)?;
                bounds.push(TypeBound {
                    parameter: name,
                    traits,
                });
                self.pending_bounds = bounds;
            }
            if self.is("=") {
                return Err(self.unsupported("generic defaults are unsupported"));
            }
            if !self.eat(",") || self.token().text.starts_with('>') {
                break;
            }
        }
        self.close_angle()?;
        Ok(params)
    }
    #[inline(never)]
    fn trait_path(&mut self) -> Result<TraitPath, ParseError> {
        let path = self.generic_path(true)?;
        self.check_trait_path(&path)?;
        Ok(TraitPath::Source(path))
    }
    fn check_trait_path(&self, path: &GenericPath) -> Result<(), ParseError> {
        if path.segments.is_empty() {
            return Err(self.error("expected a trait declaration path"));
        }
        if path.segments[..path.segments.len() - 1]
            .iter()
            .any(|segment| !segment.args.is_empty())
        {
            return Err(self.unsupported("trait arguments must belong to the final trait segment"));
        }
        Ok(())
    }
    fn unapplied_trait_path(&mut self, path: &AstPath) -> Result<GenericPath, ParseError> {
        let mut segments = Vec::new();
        self.reserve_parser_items(&mut segments, path.segments().len())?;
        for name in path.segments() {
            self.names
                .budget()
                .charge(super::expansion_budget::Counter::Work, 1)
                .map_err(|error| native_adapter::transport_error(error.into()))?;
            segments.push(PathSegment {
                name: *name,
                args: Vec::new(),
                associated_types: Vec::new(),
            });
        }
        Ok(GenericPath {
            anchor: path.anchor(),
            segments,
            resolved_use: Some(path.clone()),
        })
    }
    fn reserve_parser_items<T>(
        &mut self,
        items: &mut Vec<T>,
        additional: usize,
    ) -> Result<(), ParseError> {
        let needed = items
            .len()
            .checked_add(additional)
            .ok_or_else(|| self.limit("parser item count overflow"))?;
        if needed <= items.capacity() {
            return Ok(());
        }
        let old = items.capacity();
        let capacity = old
            .max(4)
            .checked_mul(2)
            .map(|n| n.max(needed))
            .ok_or_else(|| self.limit("parser item capacity overflow"))?;
        let planned = (capacity - old)
            .checked_mul(std::mem::size_of::<T>())
            .ok_or_else(|| self.limit("parser item storage overflow"))?;
        self.names
            .retain_parser_storage(planned)
            .map_err(native_adapter::name_error)?;
        items
            .try_reserve_exact(capacity - items.len())
            .map_err(|_| {
                native_adapter::transport_error(self.names.budget().poison_allocation().into())
            })?;
        let extra = items
            .capacity()
            .saturating_sub(capacity)
            .checked_mul(std::mem::size_of::<T>())
            .ok_or_else(|| self.limit("parser item storage overflow"))?;
        self.names
            .retain_parser_storage(extra)
            .map_err(native_adapter::name_error)
    }
    /// Predeclare only real generic-name tokens. The full parser subsequently
    /// consumes those exact occurrences, including forward references in bounds.
    /// This is a cursor pass over the same immutable tokens, not a second lexer.
    fn predeclare_type_parameters(&mut self) -> Result<Vec<AstIdent>, ParseError> {
        let mut declarations = Vec::new();
        if !self.is("<") {
            return Ok(declarations);
        }
        let mut scan = TokenCursor::new(self.input);
        if !scan.restore(self.cursor.checkpoint()) {
            return Err(native_adapter::transport_error(
                super::token_session::TokenError::InvalidHandle,
            ));
        }
        scan.bump();
        let mut angles = 1usize;
        let mut groups = 0usize;
        let mut at_parameter = true;
        loop {
            self.names
                .budget()
                .charge(super::expansion_budget::Counter::Work, 1)
                .map_err(|error| native_adapter::transport_error(error.into()))?;
            let token = scan.current();
            if token.matches("") {
                break;
            }
            if at_parameter {
                at_parameter = false;
                if token.is_ident() && !token.matches("const") {
                    if declarations.len() >= 32 {
                        return Err(self.limit("generic parameter resource limit 32"));
                    }
                    let name = self.input.mint_name(&token, self.names)?;
                    self.reserve_parser_items(&mut declarations, 1)?;
                    declarations.push(name);
                }
            }
            if groups == 0 && token.matches("<") {
                angles = angles
                    .checked_add(1)
                    .ok_or_else(|| self.limit("generic nesting overflow"))?;
                if angles > 128 {
                    return Err(self.limit("type nesting exceeds limit 128"));
                }
            } else if groups == 0 && scan.consume_type_gt() {
                angles -= 1;
                if angles == 0 {
                    break;
                }
                continue;
            } else if angles == 1 && groups == 0 && token.matches(",") {
                at_parameter = true;
            } else if token.matches("(") || token.matches("[") || token.matches("{") {
                groups = groups
                    .checked_add(1)
                    .ok_or_else(|| self.limit("generic group nesting overflow"))?;
            } else if token.matches(")") || token.matches("]") || token.matches("}") {
                groups = groups.saturating_sub(1);
            }
            scan.bump();
        }
        let mut visible = std::mem::take(&mut self.type_params);
        self.reserve_parser_items(&mut visible, declarations.len())?;
        self.names
            .budget()
            .charge(
                super::expansion_budget::Counter::Work,
                declarations.len() as u64,
            )
            .map_err(|error| native_adapter::transport_error(error.into()))?;
        visible.extend_from_slice(&declarations);
        self.type_params = visible;
        Ok(declarations)
    }
    fn predeclared_parameter(
        &mut self,
        declarations: &[AstIdent],
        index: usize,
    ) -> Result<AstIdent, ParseError> {
        if !self.is_identifier() {
            return Err(self.error("expected an ASCII identifier"));
        }
        let name = *declarations.get(index).ok_or_else(|| {
            native_adapter::transport_error(super::token_session::TokenError::InvalidHandle)
        })?;
        if self.names.span(name).map_err(native_adapter::name_error)? != self.token().span {
            return Err(native_adapter::transport_error(
                super::token_session::TokenError::InvalidInput(
                    "generic declaration cursor differs from its original token",
                ),
            ));
        }
        self.cursor.bump();
        Ok(name)
    }
    fn path_anchor(&mut self) -> Result<PathAnchor, ParseError> {
        if self.eat("crate") {
            return Ok(PathAnchor::Crate);
        }
        if self.eat("self") {
            return Ok(PathAnchor::SelfModule);
        }
        if self.eat("Self") {
            return Ok(PathAnchor::SelfType);
        }
        if self.eat("super") {
            let mut count = 1u16;
            while self.is("::") && self.cursor.peek(1).is_some_and(|t| t.matches("super")) {
                self.cursor.bump();
                self.cursor.bump();
                count = count
                    .checked_add(1)
                    .ok_or_else(|| self.limit("path ancestor count overflow"))?;
            }
            return Ok(PathAnchor::Super(count));
        }
        Ok(PathAnchor::Relative)
    }
    fn path(&mut self) -> Result<AstPath, ParseError> {
        let anchor = self.path_anchor()?;
        let mut parts = super::token_session::BudgetVec::new(
            self.names.budget(),
            super::expansion_budget::Storage::Temporary,
        )
        .map_err(native_adapter::transport_error)?;
        if anchor == PathAnchor::Relative {
            parts
                .push(self.ident()?)
                .map_err(native_adapter::transport_error)?;
        }
        while self.eat("::") {
            if self.is("*") || self.is("{") {
                return Err(self.unsupported("glob and grouped paths are unsupported"));
            }
            parts
                .push(self.ident()?)
                .map_err(native_adapter::transport_error)?;
        }
        AstPath::new(self.names, anchor, parts.as_slice()).map_err(native_adapter::name_error)
    }
    /// Flatten syntax only. Canonical identities, privacy, cycles and namespace
    /// collisions are still decided by the ordinary import resolver.
    fn use_tree(
        &mut self,
        prefix: &AstPath,
        depth: usize,
        work: &mut usize,
        bytes: &mut usize,
        imports: &mut Vec<(Option<AstIdent>, AstPath, bool)>,
    ) -> Result<(), ParseError> {
        if depth >= 128 {
            return Err(self.limit("use tree nesting exceeds limit 128"));
        }
        *work += 1;
        if *work > 4096 {
            return Err(self.limit("use tree work exceeds limit 4096"));
        }
        if self.is("::") {
            return Err(self.unsupported("absolute extern crate use paths are unsupported"));
        }
        if self.is("*") {
            return Err(self.unsupported("glob imports are unsupported"));
        }
        let mut path = prefix.clone();
        let prefix_bytes = path_text_len(self.names, prefix).map_err(native_adapter::name_error)?;
        *bytes += if prefix_bytes == 0 {
            0
        } else {
            prefix_bytes + 2
        };
        if *bytes > 1024 * 1024 {
            return Err(self.limit("use tree allocation exceeds limit 1048576"));
        }
        let mut grouped_self = false;
        let mut group_allowed = self.is("{");
        if !self.is("{") {
            loop {
                *work += 1;
                if *work > 4096 {
                    return Err(self.limit("use tree work exceeds limit 4096"));
                }
                let keyword = self.is("self") || self.is("super") || self.is("crate");
                if self.is("self")
                    && depth > 0
                    && (path.anchor() != PathAnchor::Relative || !path.is_empty())
                    && !self.cursor.peek(1).is_some_and(|t| t.matches("::"))
                {
                    self.cursor.bump();
                    grouped_self = true;
                    *bytes += 6;
                } else if keyword {
                    if path.anchor() != PathAnchor::Relative || !path.is_empty() {
                        return Err(self.error("module keyword must begin a use path"));
                    }
                    let anchor = self.path_anchor()?;
                    if let PathAnchor::Super(count) = anchor {
                        *work += usize::from(count) - 1;
                        if *work > 4096 {
                            return Err(self.limit("use tree work exceeds limit 4096"));
                        }
                    }
                    *bytes += match anchor {
                        PathAnchor::Super(n) => usize::from(n) * 7,
                        PathAnchor::Crate => 7,
                        _ => 6,
                    };
                    path = path.with_anchor(anchor);
                } else {
                    let name = self.ident()?;
                    *bytes += self
                        .names
                        .spelling(name)
                        .map_err(native_adapter::name_error)?
                        .len()
                        + 2;
                    path = path
                        .push(self.names, name)
                        .map_err(native_adapter::name_error)?;
                }
                if *bytes > 1024 * 1024 {
                    return Err(self.limit("use tree allocation exceeds limit 1048576"));
                }
                if !self.eat("::") {
                    break;
                }
                if self.is("*") {
                    return Err(self.unsupported("glob imports are unsupported"));
                }
                if self.is("{") {
                    group_allowed = true;
                    break;
                }
            }
        }
        if self.is("{") && !group_allowed {
            return Err(self.error("expected :: before use group"));
        }
        if self.eat("{") {
            if self.is("}") && (path.anchor() != PathAnchor::Relative || !path.is_empty()) {
                imports.push((None, path.clone(), false));
            }
            while !self.eat("}") {
                self.use_tree(&path, depth + 1, work, bytes, imports)?;
                if !self.eat(",") {
                    self.expect("}")?;
                    break;
                }
            }
            return Ok(());
        }
        let name = if self.eat("as") {
            if self.is("_") {
                return Err(self.unsupported("anonymous underscore imports are unsupported"));
            }
            self.ident()?
        } else {
            *path
                .segments()
                .last()
                .ok_or_else(|| self.error("module keyword imports require an explicit alias"))?
        };
        *bytes += path_text_len(self.names, &path).map_err(native_adapter::name_error)?
            + self
                .names
                .spelling(name)
                .map_err(native_adapter::name_error)?
                .len();
        if *bytes > 1024 * 1024 {
            return Err(self.limit("use tree allocation exceeds limit 1048576"));
        }
        imports.push((Some(name), path, grouped_self));
        Ok(())
    }
    fn array_number(&mut self, context: &str) -> Result<usize, ParseError> {
        let start = self.cursor.checkpoint();
        let token = self.token();
        if token.string {
            return Err(self.error(&format!("{context} requires an integer literal")));
        }
        if !token.text.starts_with(|c: char| c.is_ascii_digit()) {
            return self.unsupported_array_expression(context);
        }
        let literal = parse_integer_literal(token.text).map_err(|e| self.error(&e))?;
        let value = resolve_integer(&literal, false, Some(&ir::Ty::Int(ir::IntTy::Usize)))
            .map_err(|e| self.error(&e))?;
        if value.bits > ir::MAX_ARRAY_LEN as u128 {
            return Err(self.limit("array length/index exceeds supported limit"));
        }
        let number = value.bits as usize;
        self.cursor.bump();
        if !self.is("]") {
            assert!(
                self.cursor.restore(start),
                "parser checkpoint belongs to this input"
            );
            return self.unsupported_array_expression(context);
        }
        Ok(number)
    }
    fn unsupported_array_expression(&mut self, context: &str) -> Result<usize, ParseError> {
        // Validate the bounded expression grammar before classifying the
        // constant-evaluation boundary. Missing expressions, malformed literals
        // and missing closing brackets remain syntax errors, not fake CTFE.
        self.nested_expr()?;
        if !self.is("]") {
            return Err(self.error("expected closing bracket after array count expression"));
        }
        Err(self.unsupported(&format!(
            "const-expression {context} is unsupported; use an integer literal"
        )))
    }
    /// Visibility paths are lexical ancestors, never general name-resolution
    /// paths. Checking the canonical spelling here prevents aliases from
    /// masquerading as ancestor modules, even before those aliases resolve.
    fn visibility(&mut self, module: ModuleId) -> Result<(ir::Visibility, bool), ParseError> {
        self.modules.check(module).map_err(module_parse_error)?;
        if !self.eat("pub") {
            return Ok((ir::Visibility::Restricted(module), false));
        }
        if !self.eat("(") {
            return Ok((ir::Visibility::Public, true));
        }
        let scope = if self.eat("in") {
            let path = self.path()?;
            let mut scope = match path.anchor() {
                PathAnchor::Crate => self.modules.source_root().map_err(module_parse_error)?,
                PathAnchor::SelfModule => module,
                PathAnchor::Super(count) => {
                    let mut scope = module;
                    for _ in 0..count {
                        scope = self
                            .modules
                            .parent(scope)
                            .map_err(module_parse_error)?
                            .ok_or_else(|| self.error("visibility path escapes crate root"))?;
                    }
                    scope
                }
                _ => return Err(self.error("pub(in) paths must start with crate, self or super")),
            };
            let anchor_len = match path.anchor() {
                PathAnchor::Super(n) => usize::from(n),
                _ => 1,
            };
            if path.len().saturating_add(anchor_len) > 129 {
                return Err(self.limit("visibility path nesting exceeds limit 129"));
            }
            for name in path.segments() {
                scope = self
                    .modules
                    .lookup_child(self.names, scope, *name)
                    .map_err(module_parse_error)?
                    .ok_or_else(|| {
                        self.error("visibility path must name a lexical ancestor module")
                    })?;
            }
            if !self
                .modules
                .is_descendant(module, scope)
                .map_err(module_parse_error)?
            {
                return Err(self.error("visibility path must name a lexical ancestor module"));
            }
            scope
        } else if self.eat("crate") {
            self.modules.source_root().map_err(module_parse_error)?
        } else if self.eat("self") {
            module
        } else if self.eat("super") {
            self.modules
                .parent(module)
                .map_err(module_parse_error)?
                .ok_or_else(|| self.error("visibility path escapes crate root"))?
        } else {
            return Err(self.error("expected crate, self, super or in in restricted visibility"));
        };
        self.expect(")")?;
        Ok((ir::Visibility::Restricted(scope), true))
    }
    fn items(
        &mut self,
        module: ModuleId,
        module_path: &AstPath,
        parsed: &mut ParsedDeclarations,
    ) -> Result<(), ParseError> {
        if self.nesting > 128 {
            return Err(self.limit("module nesting exceeds limit 128"));
        }
        self.nesting += 1;
        let previous_module = self.module;
        let previous_frontier = self.module_frontier;
        self.module = module;
        if self.purpose == ParserPurpose::Active && self.input.macro_source().is_some() {
            if self.lexical.scopes.is_none() {
                self.lexical.scopes = Some(
                    super::semantic_scope::ScopeArenaBuilder::for_names(self.names)
                        .map_err(scope_parse_error)?,
                );
            }
            if self.lexical.item_macros.is_none() {
                let source = self.input.macro_source().cloned().ok_or_else(|| {
                    native_adapter::transport_error(super::token_session::TokenError::InvalidHandle)
                })?;
                self.lexical.item_macros = Some(
                    super::source_item_macro::SourceItemMacroDriver::new(source, self.names)
                        .map_err(|error| self.driver_error(error))?,
                );
            }
            let scopes = self.lexical.scopes.as_mut().ok_or_else(|| {
                native_adapter::transport_error(super::token_session::TokenError::InvalidHandle)
            })?;
            self.module_frontier = Some(
                scopes
                    .module_root(self.names, &*self.modules, module, previous_frontier)
                    .map_err(scope_parse_error)?,
            );
        }
        while !self.is("") && !self.is("}") {
            if let Some(before) = self.module_frontier {
                let scopes = self.lexical.scopes.as_mut().ok_or_else(|| {
                    native_adapter::transport_error(super::token_session::TokenError::InvalidHandle)
                })?;
                self.module_frontier = Some(
                    scopes
                        .module_step(self.names, &*self.modules, before)
                        .map_err(scope_parse_error)?,
                );
            }
            let (visibility, explicit_visibility) = self.visibility(module)?;
            if self.is("macro_rules") && self.cursor.peek(1).is_some_and(|token| token.matches("!"))
            {
                if explicit_visibility {
                    return Err(
                        self.unsupported("visibility-qualified macro definitions are unsupported")
                    );
                }
                self.module_macro_definition()?;
                continue;
            }
            if self.token().is_ident()
                && self.cursor.peek(1).is_some_and(|token| token.matches("!"))
            {
                if explicit_visibility {
                    return Err(self.error("macro invocations cannot have visibility"));
                }
                self.item_macro_invocation(module, module_path, parsed)?;
                continue;
            }
            if self.eat("mod") {
                let name = self.ident()?;
                let full = module_path
                    .push(self.names, name)
                    .map_err(native_adapter::name_error)?;
                let child = self
                    .modules
                    .intern_child(self.names, module, name)
                    .map_err(module_parse_error)?;
                if self.eat(";") {
                    return Err(
                        self.unsupported("external module files are unsupported; use inline mod")
                    );
                }
                self.expect("{")?;
                // Register before parsing descendants, including empty modules.
                self.insert_item(parsed, module, &full, visibility, ItemKind::Module(child))?;
                self.items(child, &full, parsed)?;
                self.expect("}")?;
                continue;
            }
            self.nonmodule_item(module, module_path, parsed, visibility, explicit_visibility)?;
        }
        self.module = previous_module;
        self.module_frontier = previous_frontier;
        self.nesting -= 1;
        Ok(())
    }
    // Keep the large nonrecursive declaration parser frame off the recursive
    // inline-module path, including on ordinary 2 MiB test-thread stacks.
    #[inline(never)]
    fn nonmodule_item(
        &mut self,
        module: ModuleId,
        module_path: &AstPath,
        parsed: &mut ParsedDeclarations,
        visibility: ir::Visibility,
        explicit_visibility: bool,
    ) -> Result<(), ParseError> {
        if !self.token().string
            && matches!(
                self.token().text,
                "async" | "unsafe" | "extern" | "static" | "union" | "macro_rules"
            )
        {
            return Err(self.unsupported("item syntax is unsupported"));
        }
        if self.is("const") && !self.cursor.peek(1).is_some_and(|t| t.text == "fn") {
            return Err(self.unsupported("const items and constant evaluation are unsupported"));
        }
        let (name, kind) = if self.eat("use") {
            let mut imports = Vec::new();
            let mut work = 0;
            let mut bytes = 0;
            let prefix = AstPath::new(self.names, PathAnchor::Relative, &[])
                .map_err(native_adapter::name_error)?;
            self.use_tree(&prefix, 0, &mut work, &mut bytes, &mut imports)?;
            self.expect(";")?;
            for (name, path, type_only) in imports {
                // Bound logical retained expansion across the entire source,
                // not just each tree. This is not an allocator-byte charge.
                self.use_expansions += 1;
                self.use_expansion_bytes += path_text_len(self.names, &path)
                    .map_err(native_adapter::name_error)?
                    + canonical_path_text_len(self.names, module_path)
                        .map_err(native_adapter::name_error)?
                    + name
                        .map(|name| self.names.spelling(name).map(str::len))
                        .transpose()
                        .map_err(native_adapter::name_error)?
                        .unwrap_or(0)
                    + 2;
                if self.use_expansions > 65536 {
                    return Err(self.limit("source use expansion count exceeds limit 65536"));
                }
                if self.use_expansion_bytes > 16 * 1024 * 1024 {
                    return Err(self.limit("source use logical expansion exceeds limit 16777216"));
                }
                let Some(name) = name else {
                    parsed.empty_use_prefixes.push((module, path));
                    continue;
                };
                self.insert_item(
                    parsed,
                    module,
                    &module_path
                        .push(self.names, name)
                        .map_err(native_adapter::name_error)?,
                    visibility.clone(),
                    ItemKind::Import(path, type_only),
                )?;
            }
            return Ok(());
        } else if self.eat("trait") {
            let name = self.ident()?;
            let type_params = self.type_parameters()?;
            if type_params.len() >= 32 {
                return Err(self.limit("trait binder resource limit 32 includes Self"));
            }
            let mut bounds = std::mem::take(&mut self.pending_bounds);
            let mut supertraits = Vec::new();
            if self.eat(":") {
                loop {
                    if supertraits.len() >= 256 {
                        return Err(self.limit("supertrait limit exceeded"));
                    }
                    let path = self.trait_path()?;
                    self.reserve_parser_items(&mut supertraits, 1)?;
                    supertraits.push(path);
                    if !self.eat("+") {
                        break;
                    }
                }
            }
            if self.eat("where") {
                loop {
                    self.where_type_predicate(&mut bounds)?;
                    if !self.eat(",") || self.is("{") {
                        break;
                    }
                }
            }
            self.expect("{")?;
            let mut items = Vec::new();
            let mut associated_types = Vec::new();
            let mut item_names = std::collections::HashSet::new();
            while !self.is("}") {
                if items.len().saturating_add(associated_types.len()) >= 4096 {
                    return Err(self.limit("trait item limit exceeded"));
                }
                if self.is("pub") {
                    return Err(self.error("trait items cannot declare visibility"));
                }
                if self.eat("type") {
                    let name = self.ident()?;
                    if self.is("<") || self.is("where") {
                        return Err(self.unsupported("generic associated types are unsupported"));
                    }
                    let mut associated_bounds = Vec::new();
                    if self.eat(":") {
                        loop {
                            if associated_bounds.len() >= 256 {
                                return Err(self.limit("associated type bound limit exceeded"));
                            }
                            let path = self.trait_path()?;
                            self.reserve_parser_items(&mut associated_bounds, 1)?;
                            associated_bounds.push(path);
                            if !self.eat("+") {
                                break;
                            }
                        }
                    }
                    if self.is("=") {
                        return Err(
                            self.unsupported("associated type bounds and defaults are unsupported")
                        );
                    }
                    if self.is("where") {
                        return Err(self.unsupported("generic associated types are unsupported"));
                    }
                    self.expect(";")?;
                    self.reserve_parser_items(&mut associated_types, 1)?;
                    associated_types.push(AssociatedTypeDecl {
                        name,
                        bounds: associated_bounds,
                    });
                    continue;
                }
                let (signature, receiver) = self.function_head(&type_params, true)?;
                if !item_names.insert(self.name_key(signature.name, LookupClass::Member)?) {
                    return Err(self.error("duplicate required trait item"));
                }
                if !signature.type_params.is_empty() {
                    return Err(self.unsupported("generic trait methods are unsupported"));
                }
                let default = if self.is("{") {
                    let (body, lexical_scope) = self.function_body(&signature.params)?;
                    Some(Function {
                        lexical_scope,
                        module,
                        constness: signature.constness,
                        visibility: visibility.clone(),
                        bounds: signature.bounds.clone(),
                        name: AstPath::new(self.names, PathAnchor::Relative, &[signature.name])
                            .map_err(native_adapter::name_error)?,
                        regions: signature.regions.clone(),
                        type_params: signature.type_params.clone(),
                        params: signature.params.clone(),
                        param_mut: signature.param_mut.clone(),
                        result: signature.result.clone(),
                        impl_params: signature.impl_params,
                        body,
                    })
                } else {
                    self.expect(";")?;
                    None
                };
                self.reserve_parser_items(&mut items, 1)?;
                items.push(TraitItemDecl {
                    signature,
                    receiver,
                    default,
                });
            }
            self.expect("}")?;
            parsed.program.traits.push(TraitDecl {
                module,
                type_params,
                bounds,
                supertraits,
                name: module_path
                    .push(self.names, name)
                    .map_err(native_adapter::name_error)?,
                visibility: visibility.clone(),
                items,
                associated_types,
            });
            (name, ItemKind::Trait)
        } else if self.eat("enum") {
            let name = self.ident()?;
            let type_params = self.type_parameters()?;
            if !self.pending_bounds.is_empty() {
                return Err(self.unsupported("bounds on nominal declarations are unsupported"));
            }
            self.type_params = type_params.clone();
            self.expect("{")?;
            let mut variants: Vec<VariantDecl> = Vec::new();
            while !self.is("}") {
                let variant = self.ident()?;
                let mut fields = Vec::new();
                let shape =
                    if self.eat("(") {
                        while !self.is(")") {
                            if fields.len() >= 4096 {
                                return Err(self.limit("variant field count exceeds limit 4096"));
                            }
                            if self.is("pub") {
                                return Err(self
                                    .error("enum variant fields cannot have explicit visibility"));
                            }
                            fields.push(FieldDecl {
                                name: FieldName::Tuple(fields.len()),
                                ty: self.ty()?,
                                visibility: ir::Visibility::Public,
                            });
                            if !self.eat(",") {
                                break;
                            }
                        }
                        self.expect(")")?;
                        ir::VariantShape::Tuple
                    } else if self.eat("{") {
                        while !self.is("}") {
                            if fields.len() >= 4096 {
                                return Err(self.limit("variant field count exceeds limit 4096"));
                            }
                            if self.is("pub") {
                                return Err(self
                                    .error("enum variant fields cannot have explicit visibility"));
                            }
                            let name = self.ident()?;
                            self.expect(":")?;
                            fields.push(FieldDecl {
                                name: FieldName::Named(name),
                                ty: self.ty()?,
                                visibility: ir::Visibility::Public,
                            });
                            if !self.eat(",") {
                                break;
                            }
                        }
                        self.expect("}")?;
                        ir::VariantShape::Struct
                    } else {
                        ir::VariantShape::Unit
                    };
                if self.is("=") {
                    return Err(self.unsupported("explicit enum discriminants are unsupported"));
                }
                if self.contains_name(
                    &variants.iter().map(|v| v.name).collect::<Vec<_>>(),
                    variant,
                    LookupClass::Item(Namespace::Value),
                )? {
                    return Err(self.error("duplicate enum variant"));
                }
                if variants.len() >= 4096 {
                    return Err(self.limit("enum variant count exceeds limit 4096"));
                }
                self.insert_item(
                    parsed,
                    module,
                    &module_path
                        .push(self.names, name)
                        .and_then(|path| path.push(self.names, variant))
                        .map_err(native_adapter::name_error)?,
                    ir::Visibility::Public,
                    ItemKind::Variant(shape),
                )?;
                variants.push(VariantDecl {
                    name: variant,
                    shape,
                    fields,
                });
                if !self.eat(",") {
                    break;
                }
            }
            self.expect("}")?;
            if variants.is_empty() {
                return Err(self.unsupported("empty enums are unsupported"));
            }
            parsed.program.enums.push(EnumDecl {
                module,
                type_params,
                name: module_path
                    .push(self.names, name)
                    .map_err(native_adapter::name_error)?,
                visibility: visibility.clone(),
                variants,
            });
            (name, ItemKind::Enum)
        } else if self.eat("struct") {
            let name = self.ident()?;
            let type_params = self.type_parameters()?;
            if !self.pending_bounds.is_empty() {
                return Err(self.unsupported("bounds on nominal declarations are unsupported"));
            }
            self.type_params = type_params.clone();
            let mut fields = Vec::new();
            let shape = if self.eat(";") {
                StructShape::Unit
            } else if self.eat("(") {
                if self.is(")") {
                    return Err(self.unsupported("empty tuple structs are unsupported"));
                }
                loop {
                    let (visibility, _) = self.visibility(module)?;
                    let ty = self.ty()?;
                    fields.push(FieldDecl {
                        name: FieldName::Tuple(fields.len()),
                        ty,
                        visibility,
                    });
                    if !self.eat(",") || self.is(")") {
                        break;
                    }
                }
                self.expect(")")?;
                self.expect(";")?;
                StructShape::Tuple
            } else {
                self.expect("{")?;
                while !self.is("}") {
                    let (visibility, _) = self.visibility(module)?;
                    let name = self.ident()?;
                    self.expect(":")?;
                    fields.push(FieldDecl {
                        name: FieldName::Named(name),
                        ty: self.ty()?,
                        visibility,
                    });
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect("}")?;
                if fields.is_empty() {
                    return Err(self.unsupported("empty structs are unsupported"));
                }
                StructShape::Record
            };
            parsed.program.structs.push(StructDecl {
                shape,
                module,
                type_params,
                name: module_path
                    .push(self.names, name)
                    .map_err(native_adapter::name_error)?,
                visibility: visibility.clone(),
                fields,
            });
            (name, ItemKind::Struct)
        } else if self.eat("impl") {
            if explicit_visibility {
                return Err(self.error("pub impl is invalid"));
            }
            let type_params = self.type_parameters()?;
            self.type_params = type_params.clone();
            let mut bounds = std::mem::take(&mut self.pending_bounds);
            let first_ty = self.ty()?;
            let (trait_path, self_ty) = if self.eat("for") {
                let path = match first_ty {
                    Type::Named(name) => self.unapplied_trait_path(&name)?,
                    Type::Apply(path) => path,
                    _ => return Err(self.error("expected a trait declaration before for")),
                };
                self.check_trait_path(&path)?;
                (Some(TraitPath::Source(path)), self.ty()?)
            } else {
                (None, first_ty)
            };
            if self.eat("where") {
                loop {
                    if self.token().is_lifetime() {
                        return Err(
                            self.unsupported("lifetime where predicates on impls are unsupported")
                        );
                    }
                    self.where_type_predicate(&mut bounds)?;
                    if !self.eat(",") || self.is("{") {
                        break;
                    }
                }
            }
            self.expect("{")?;
            let mut items = Vec::new();
            let mut associated_types = Vec::new();
            while !self.is("}") {
                if items.len().saturating_add(associated_types.len()) >= 4096 {
                    return Err(self.limit("associated item limit exceeded"));
                }
                let (visibility, explicit_visibility) = self.visibility(module)?;
                if explicit_visibility && trait_path.is_some() {
                    return Err(self.error("trait impl items cannot declare pub"));
                }
                if self.eat("type") {
                    if trait_path.is_none() {
                        return Err(self.unsupported("inherent associated types are unsupported"));
                    }
                    let name = self.ident()?;
                    if self.is("<") || self.is("where") || self.is(":") {
                        return Err(self.unsupported(
                            "generic associated types and associated bounds are unsupported",
                        ));
                    }
                    self.expect("=")?;
                    // A preceding generic method must not leak its binder into
                    // this assignment's exact impl-only environment.
                    let mut visible = std::mem::take(&mut self.type_params);
                    visible.clear();
                    self.reserve_parser_items(&mut visible, type_params.len())?;
                    visible.extend_from_slice(&type_params);
                    self.type_params = visible;
                    let value = self.ty()?;
                    self.expect(";")?;
                    self.reserve_parser_items(&mut associated_types, 1)?;
                    associated_types.push(AssociatedTypeValue { name, value });
                    continue;
                }
                let (mut function, receiver) = self.function_scoped(&type_params, true)?;
                function.visibility = visibility.clone();
                self.reserve_parser_items(&mut items, 1)?;
                items.push(AssociatedFunction {
                    function,
                    receiver,
                    visibility,
                });
            }
            self.expect("}")?;
            parsed.program.impls.push(ImplDecl {
                trait_path,
                bounds,
                module,
                type_params,
                self_ty,
                items,
                associated_types,
            });
            self.type_params.clear();
            return Ok(());
        } else if self.is("fn") || self.is("const") {
            let mut function = self.function()?;
            function.visibility = visibility.clone();
            let name = function
                .name
                .single()
                .ok_or_else(|| self.error("function declaration must have a single name"))?;
            function.name = module_path
                .push(self.names, name)
                .map_err(native_adapter::name_error)?;
            parsed.program.functions.push(function);
            (name, ItemKind::Function)
        } else if self.eat("type") {
            let name = self.ident()?;
            if self.is("<") && self.cursor.peek(1).is_some_and(|t| t.text == "const") {
                return Err(self.unsupported("const type alias parameters are unsupported"));
            }
            let params = self.type_parameters()?;
            if !self.pending_bounds.is_empty() || self.is("where") || self.is(":") {
                return Err(self.unsupported("type alias bounds and where clauses are unsupported"));
            }
            self.type_params = params.clone();
            self.expect("=")?;
            self.alias_target = true;
            let target = self.ty()?;
            self.alias_target = false;
            validate_depth(DepthNode::Type(&target)).map_err(|e| self.depth_error(&e))?;
            if self.is("where") {
                return Err(self.unsupported("type alias where clauses are unsupported"));
            }
            self.expect(";")?;
            (name, ItemKind::TypeAlias(TypeAliasDecl { params, target }))
        } else {
            return Err(self.error("expected fn, inline mod or simple use item"));
        };
        self.type_params.clear();
        self.insert_item(
            parsed,
            module,
            &module_path
                .push(self.names, name)
                .map_err(native_adapter::name_error)?,
            visibility,
            kind,
        )?;
        Ok(())
    }
    fn insert_item(
        &mut self,
        parsed: &mut ParsedDeclarations,
        module: ModuleId,
        path: &AstPath,
        visibility: ir::Visibility,
        kind: ItemKind,
    ) -> Result<(), ParseError> {
        path.check(self.names).map_err(native_adapter::name_error)?;
        self.names
            .budget()
            .charge(
                super::expansion_budget::Counter::Work,
                path.len() as u64 + 1,
            )
            .map_err(|error| native_adapter::transport_error(error.into()))?;
        let key = path.key(Namespace::Type);
        let bindings = parsed.items.entry(key).or_default();
        if self.item_expansion.is_some() && bindings.len() == bindings.capacity() {
            let planned = bindings
                .len()
                .checked_add(1)
                .and_then(|n| n.checked_mul(std::mem::size_of::<Item>()))
                .ok_or_else(|| {
                    native_adapter::transport_error(super::token_session::TokenError::IdExhausted)
                })?;
            self.names
                .retain_parser_storage(planned)
                .map_err(native_adapter::name_error)?;
            bindings.try_reserve_exact(1).map_err(|_| {
                native_adapter::transport_error(self.names.budget().poison_allocation().into())
            })?;
            let actual = bindings
                .capacity()
                .checked_mul(std::mem::size_of::<Item>())
                .ok_or_else(|| {
                    native_adapter::transport_error(super::token_session::TokenError::IdExhausted)
                })?;
            self.names
                .retain_parser_storage(actual.saturating_sub(planned))
                .map_err(native_adapter::name_error)?;
        }
        if bindings.iter().any(|item| {
            if matches!(kind, ItemKind::Enum)
                && matches!(
                    item.kind,
                    ItemKind::Module(_) | ItemKind::Struct | ItemKind::Enum
                )
                || matches!(item.kind, ItemKind::Enum)
                    && matches!(kind, ItemKind::Module(_) | ItemKind::Struct)
            {
                return true;
            }
            matches!(
                (&item.kind, &kind),
                (ItemKind::Module(_), ItemKind::Module(_))
                    | (ItemKind::Module(_), ItemKind::Struct)
                    | (ItemKind::Struct, ItemKind::Module(_))
                    | (ItemKind::Struct, ItemKind::Struct)
                    | (ItemKind::Function, ItemKind::Function)
            )
        }) {
            return Err(self.error(&format!("duplicate item {path:?}")));
        }
        bindings.push(Item {
            path: path.clone(),
            module,
            visibility,
            kind,
        });
        Ok(())
    }
    fn lifetime(&mut self) -> Result<RegionName, ParseError> {
        if self.token().string {
            return Err(self.error("expected named lifetime, not a string"));
        }
        let text = self.token().text.to_owned();
        let name = text
            .strip_prefix('\'')
            .ok_or_else(|| self.error("expected named lifetime"))?;
        if name == "static" || name == "_" {
            return Err(self.unsupported("static and inferred lifetimes are unsupported"));
        }
        if [
            "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn",
            "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
            "return", "self", "Self", "struct", "super", "trait", "true", "type", "unsafe", "use",
            "where", "while", "async", "await", "dyn", "abstract", "become", "box", "do", "final",
            "macro", "override", "priv", "typeof", "unsized", "virtual", "yield", "try", "gen",
        ]
        .contains(&name)
        {
            return Err(self.unsupported("keyword lifetime names are unsupported"));
        }
        if !name
            .as_bytes()
            .first()
            .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
        {
            return Err(self.error("expected named ASCII lifetime"));
        }
        let ident = self.input.mint_name(&self.token(), self.names)?;
        self.cursor.bump();
        Ok(RegionName::Named(ident))
    }
    fn region_bounds(
        &mut self,
        long: RegionName,
        regions: &mut RegionDecls,
    ) -> Result<(), ParseError> {
        if self.eat(":") {
            loop {
                let short = self.lifetime()?;
                if regions.outlives.len() >= 4096 {
                    return Err(self.limit("lifetime bounds exceed supported limit 4096"));
                }
                regions.outlives.push((long, short));
                if !self.eat("+") {
                    break;
                }
            }
        }
        Ok(())
    }
    /// One `T: Trait + Other` where predicate whose subject is a visible type
    /// parameter. Bounds join the same predicate rows as inline bounds, so
    /// the trait world sees one authoritative environment for both spellings.
    fn where_type_predicate(&mut self, bounds: &mut Vec<TypeBound>) -> Result<(), ParseError> {
        if self.is("for") {
            return Err(self.unsupported("higher-ranked where predicates are unsupported"));
        }
        if self.is("Self") {
            return Err(self.unsupported("Self where predicates are unsupported"));
        }
        let parameter = self.ident()?;
        if !self.contains_name(&self.type_params, parameter, LookupClass::TypeParameter)? {
            return Err(
                self.error("where predicates are supported only on visible type parameters")
            );
        }
        if !self.is(":") {
            return Err(
                self.error("where predicates are supported only on visible type parameters")
            );
        }
        self.expect(":")?;
        let mut traits = Vec::new();
        while !(self.is(",") || self.is("{") || self.is(";")) {
            if self.is("?") {
                return Err(self.unsupported("relaxed ?Sized bounds are unsupported"));
            }
            if self.token().is_lifetime() {
                return Err(self.unsupported("type outlives bounds are unsupported"));
            }
            if traits.len() >= 256 {
                return Err(self.limit("predicate limit exceeded"));
            }
            let path = self.trait_path()?;
            self.reserve_parser_items(&mut traits, 1)?;
            traits.push(path);
            if !self.eat("+") {
                break;
            }
        }
        if bounds.len() >= 4096 {
            return Err(self.limit("predicate limit exceeded"));
        }
        self.reserve_parser_items(bounds, 1)?;
        bounds.push(TypeBound { parameter, traits });
        Ok(())
    }
    fn function(&mut self) -> Result<Function, ParseError> {
        self.function_scoped(&[], false).map(|(f, _)| f)
    }
    fn function_scoped(
        &mut self,
        inherited: &[AstIdent],
        associated: bool,
    ) -> Result<(Function, ReceiverKind), ParseError> {
        let (head, receiver) = self.function_head(inherited, associated)?;
        let (body, lexical_scope) = if let Some(proof) = self.item_expansion {
            if associated
                || !inherited.is_empty()
                || !head.type_params.is_empty()
                || !head.regions.params.is_empty()
            {
                return Err(
                    self.unsupported("item macro products must be nongeneric free functions")
                );
            }
            let scopes = self.lexical.scopes.as_mut().ok_or_else(|| {
                native_adapter::transport_error(super::token_session::TokenError::InvalidHandle)
            })?;
            let frontier = scopes
                .generated_function_root(self.names, &*self.modules, &proof, head.name)
                .map_err(scope_parse_error)?;
            let owner = scopes.function(frontier).map_err(scope_parse_error)?;
            let previous = self.lexical.function.replace(FunctionMacroState {
                frontier,
                driver: None,
            });
            let result = (|| {
                for (name, _) in &head.params {
                    if let BindingName::Named(name) = name {
                        self.bind_lexical(*name)?;
                    }
                }
                self.block()
            })();
            self.lexical.function = previous;
            (result?, Some(owner))
        } else {
            self.function_body(&head.params)?
        };
        Ok((
            Function {
                lexical_scope,
                module: self.module,
                constness: head.constness,
                visibility: ir::Visibility::Public,
                name: AstPath::new(self.names, PathAnchor::Relative, &[head.name])
                    .map_err(native_adapter::name_error)?,
                regions: head.regions,
                type_params: head.type_params,
                bounds: head.bounds,
                params: head.params,
                param_mut: head.param_mut,
                result: head.result,
                impl_params: head.impl_params,
                body,
            },
            receiver,
        ))
    }
    fn callable_generics(
        &mut self,
        regions: &mut RegionDecls,
    ) -> Result<Vec<AstIdent>, ParseError> {
        self.pending_bounds.clear();
        let declarations = self.predeclare_type_parameters()?;
        let mut params = Vec::new();
        if !self.eat("<") {
            return Ok(params);
        }
        let mut seen_type = false;
        loop {
            if self.eat("const") {
                self.ident()?;
                self.expect(":")?;
                self.ty()?;
                if !self.is(">") && !self.is(",") && !self.is("=") {
                    return Err(self.error("expected const generic parameter delimiter"));
                }
                return Err(self.unsupported("const generic parameters are unsupported"));
            }
            if self.token().is_lifetime() {
                if seen_type {
                    return Err(self.error("lifetime parameters must precede type parameters"));
                }
                if regions.params.len() >= 64 {
                    return Err(self.limit("declared lifetime limit64"));
                }
                let name = self.lifetime()?;
                let RegionName::Named(ident) = name else {
                    return Err(self.error("expected declared lifetime"));
                };
                regions.params.push(ident);
                self.region_bounds(name, regions)?;
            } else {
                seen_type = true;
                if params.len() >= 32 {
                    return Err(self.limit("generic parameter resource limit32"));
                }
                let name = self.predeclared_parameter(&declarations, params.len())?;
                if self.contains_name(&params, name, LookupClass::TypeParameter)? {
                    return Err(self.error("duplicate type parameter"));
                }
                self.reserve_parser_items(&mut params, 1)?;
                params.push(name);
                if self.eat(":") {
                    let mut traits = Vec::new();
                    loop {
                        if traits.len() >= 256 {
                            return Err(self.limit("predicate limit exceeded"));
                        }
                        let path = self.trait_path()?;
                        self.reserve_parser_items(&mut traits, 1)?;
                        traits.push(path);
                        if !self.eat("+") {
                            break;
                        }
                    }
                    let mut bounds = std::mem::take(&mut self.pending_bounds);
                    self.reserve_parser_items(&mut bounds, 1)?;
                    bounds.push(TypeBound {
                        parameter: name,
                        traits,
                    });
                    self.pending_bounds = bounds;
                }
                if self.is("=") {
                    return Err(self.unsupported("generic defaults are unsupported"));
                }
            }
            if !self.eat(",") || self.is(">") {
                break;
            }
        }
        self.close_angle()?;
        Ok(params)
    }
    fn function_head(
        &mut self,
        inherited: &[AstIdent],
        associated: bool,
    ) -> Result<(FunctionHead, ReceiverKind), ParseError> {
        let constness = if self.eat("const") {
            if associated {
                return Err(self.unsupported("const associated functions are unsupported"));
            }
            Constness::Const
        } else {
            Constness::Runtime
        };
        self.expect("fn")?;
        let name = self.ident()?;
        let mut regions = RegionDecls::default();
        self.type_params.clear();
        let mut visible = std::mem::take(&mut self.type_params);
        self.reserve_parser_items(&mut visible, inherited.len())?;
        visible.extend_from_slice(inherited);
        self.type_params = visible;
        let type_params = self.callable_generics(&mut regions)?;
        let mut bounds = std::mem::take(&mut self.pending_bounds);
        let mut visible = std::mem::take(&mut self.type_params);
        visible.clear();
        self.reserve_parser_items(&mut visible, inherited.len())?;
        visible.extend_from_slice(inherited);
        self.type_params = visible;
        for name in &type_params {
            if self.contains_name(&self.type_params, *name, LookupClass::TypeParameter)? {
                return Err(self.error("duplicate visible generic parameter"));
            }
            let mut visible = std::mem::take(&mut self.type_params);
            self.reserve_parser_items(&mut visible, 1)?;
            visible.push(*name);
            self.type_params = visible;
        }
        self.expect("(")?;
        let mut params = Vec::new();
        let mut param_mut = Vec::new();
        let mut receiver = ReceiverKind::None;
        let outer_impl_params = self.impl_trait_params.replace(Vec::new());
        let outer_pending = std::mem::take(&mut self.pending_bounds);
        if associated && self.eat("&") {
            regions.receiver = if self.token().is_lifetime() {
                Some(self.lifetime()?)
            } else {
                None
            };
            let kind = if self.eat("mut") {
                ir::BorrowKind::Mut
            } else {
                ir::BorrowKind::Shared
            };
            self.expect("self")?;
            receiver = ReceiverKind::Borrowed { kind };
            if !self.is(")") {
                self.expect(",")?;
            }
        } else if associated && (self.is("self") || self.is("mut")) {
            let mutable = self.eat("mut");
            self.expect("self")?;
            if self.is(":") {
                return Err(self.unsupported("typed receivers are unsupported"));
            }
            receiver = ReceiverKind::Value { mutable };
            if !self.is(")") {
                self.expect(",")?;
            }
        }
        if !self.is(")") {
            loop {
                if self.is("[") {
                    return Err(self.unsupported("array parameter patterns are unsupported"));
                }
                let mutable = self.eat("mut");
                if mutable && self.is("mut") {
                    return Err(self.error("`mut` must be followed by a binding name"));
                }
                if mutable && (self.is("(") || self.is("[") || self.is("_")) {
                    return Err(self.unsupported("`mut` parameter patterns are unsupported"));
                }
                let n = self.binding()?;
                if self.is("(") || self.is("{") || self.is("::") {
                    return Err(self.unsupported("constructor parameter patterns are unsupported"));
                }
                self.expect(":")?;
                let ty = self.ty();
                let ty = match ty {
                    Ok(ty) => ty,
                    Err(error) => {
                        self.impl_trait_params = outer_impl_params;
                        self.pending_bounds = outer_pending;
                        return Err(error);
                    }
                };
                params.push((n, ty));
                param_mut.push(mutable);
                if !self.eat(",") || self.is(")") {
                    break;
                }
            }
        }
        let minted =
            std::mem::replace(&mut self.impl_trait_params, outer_impl_params).unwrap_or_default();
        let impl_bounds = std::mem::replace(&mut self.pending_bounds, outer_pending);
        let mut type_params = type_params;
        for name in &minted {
            if self.contains_name(&type_params, *name, LookupClass::TypeParameter)?
                || self.contains_name(&self.type_params, *name, LookupClass::TypeParameter)?
            {
                return Err(self.unsupported(
                    "a generic parameter spelling collides with an anonymous `impl Trait` parameter",
                ));
            }
        }
        if type_params.len() + minted.len() > 32 {
            return Err(self.limit("generic parameter resource limit 32"));
        }
        self.reserve_parser_items(&mut type_params, minted.len())?;
        type_params.extend_from_slice(&minted);
        let impl_params = minted.len();
        self.reserve_parser_items(&mut bounds, impl_bounds.len())?;
        bounds.extend(impl_bounds);
        self.expect(")")?;
        let result = if self.eat("->") {
            self.ty()?
        } else {
            Type::Unit
        };
        if self.eat("where") {
            loop {
                if self.token().is_lifetime() {
                    let long = self.lifetime()?;
                    if !self.is(":") {
                        return Err(self.error("only lifetime outlives where bounds are supported"));
                    }
                    self.region_bounds(long, &mut regions)?;
                } else {
                    self.where_type_predicate(&mut bounds)?;
                }
                if !self.eat(",") || self.is("{") || self.is(";") {
                    break;
                }
            }
        }
        Ok((
            FunctionHead {
                constness,
                name,
                regions,
                type_params,
                bounds,
                params,
                param_mut,
                result,
                impl_params,
            },
            receiver,
        ))
    }
    fn block(&mut self) -> Result<Block, ParseError> {
        if self.nesting >= 128 {
            return Err(self.limit("syntax nesting exceeds limit 128"));
        }
        self.nesting += 1;
        let previous = self.allow_struct;
        let previous_purpose = self.condition_purpose;
        self.allow_struct = true;
        self.condition_purpose = ConditionPurpose::Ordinary;
        let frontier = self.enter_lexical()?;
        let result = self.block_inner();
        self.restore_frontier(frontier);
        self.allow_struct = previous;
        self.condition_purpose = previous_purpose;
        self.nesting -= 1;
        result
    }
    /// The operand of a reference pattern binds tighter than `|` and `@`.
    fn pattern_no_alternatives(&mut self) -> Result<Pattern, ParseError> {
        if self.nesting >= 128 {
            return Err(self.limit("pattern nesting exceeds limit 128"));
        }
        self.nesting += 1;
        let result = self.pattern_inner();
        self.nesting -= 1;
        result
    }
    fn pattern(&mut self) -> Result<Pattern, ParseError> {
        if self.nesting >= 128 {
            return Err(self.limit("pattern nesting exceeds limit 128"));
        }
        self.nesting += 1;
        let result = self.pattern_inner();
        let result = result.and_then(|pattern| {
            if self.is("|") || self.is("@") {
                Err(self.unsupported("or-patterns and @ patterns are unsupported"))
            } else if self.is("..=") || self.is("..") {
                // Literal endpoints were consumed by pattern_inner already.
                Err(self.unsupported("path and constant range pattern bounds are unsupported"))
            } else {
                Ok(pattern)
            }
        });
        self.nesting -= 1;
        result
    }
    fn reserve_rest_patterns(
        &mut self,
        fields: &mut Vec<Pattern>,
        additional: usize,
    ) -> Result<(), ParseError> {
        let overflow =
            || native_adapter::transport_error(super::token_session::TokenError::IdExhausted);
        let needed = fields.len().checked_add(additional).ok_or_else(overflow)?;
        if needed <= fields.capacity() {
            return Ok(());
        }
        let capacity = needed
            .max(fields.capacity().checked_mul(2).ok_or_else(overflow)?)
            .max(4);
        let planned = capacity
            .checked_sub(fields.capacity())
            .and_then(|count| count.checked_mul(std::mem::size_of::<Pattern>()))
            .ok_or_else(overflow)?;
        self.names
            .retain_parser_storage(planned)
            .map_err(native_adapter::name_error)?;
        fields
            .try_reserve_exact(capacity - fields.len())
            .map_err(|_| {
                native_adapter::transport_error(self.names.budget().poison_allocation().into())
            })?;
        let extra = fields
            .capacity()
            .checked_sub(capacity)
            .and_then(|count| count.checked_mul(std::mem::size_of::<Pattern>()))
            .ok_or_else(overflow)?;
        self.names
            .retain_parser_storage(extra)
            .map_err(native_adapter::name_error)
    }
    /// Called after the sole `..`; its suffix remains explicit and unexpanded.
    fn positional_rest_suffix(&mut self, close: &str) -> Result<Vec<Pattern>, ParseError> {
        let mut suffix = Vec::new();
        if self.eat(",") {
            while !self.is(close) {
                if self.is("..") {
                    return Err(self.error("duplicate positional rest pattern"));
                }
                self.names
                    .budget()
                    .charge(super::expansion_budget::Counter::Work, 1)
                    .map_err(|error| native_adapter::transport_error(error.into()))?;
                let pattern = self.pattern()?;
                if self.is("@") {
                    return Err(self.unsupported("@ positional rest patterns are unsupported"));
                }
                if close == "]" {
                    reject_slice_half_open(&pattern).map_err(|e| self.error(e))?;
                }
                self.reserve_rest_patterns(&mut suffix, 1)?;
                suffix.push(pattern);
                if !self.eat(",") {
                    break;
                }
            }
        }
        self.expect(close)?;
        Ok(suffix)
    }
    #[inline(never)]
    fn array_pattern(&mut self) -> Result<Pattern, ParseError> {
        let mut prefix = Vec::new();
        while !self.is("]") {
            if self.eat("..") {
                let suffix = self.positional_rest_suffix("]")?;
                return Ok(Pattern::ArrayRest(PositionalRest { prefix, suffix }));
            }
            self.names
                .budget()
                .charge(super::expansion_budget::Counter::Work, 1)
                .map_err(|error| native_adapter::transport_error(error.into()))?;
            let pattern = self.pattern()?;
            if self.is("@") {
                return Err(self.unsupported("@ array rest patterns are unsupported"));
            }
            reject_slice_half_open(&pattern).map_err(|e| self.error(e))?;
            self.reserve_rest_patterns(&mut prefix, 1)?;
            prefix.push(pattern);
            if !self.eat(",") {
                break;
            }
        }
        self.expect("]")?;
        Ok(Pattern::Array(prefix))
    }
    fn range_bound_start(&self) -> bool {
        !self.token().string
            && (self.token().is_char()
                || self.is("-")
                || self.token().text.starts_with(|c: char| c.is_ascii_digit())
                || self.int_limit_start())
    }
    /// `i32::MAX` style endpoint: a primitive integer name, `::`, MIN or MAX.
    fn int_limit_start(&self) -> bool {
        self.token().is_ident()
            && integer_type(self.token().text).is_some()
            && self.cursor.peek(1).is_some_and(|t| t.matches("::"))
            && self
                .cursor
                .peek(2)
                .is_some_and(|t| t.matches("MAX") || t.matches("MIN"))
    }
    /// One literal range or scalar pattern endpoint: char, or optionally negated integer.
    fn range_bound(&mut self) -> Result<RangeBound, ParseError> {
        if self.token().is_char() {
            self.names
                .budget()
                .charge(
                    super::expansion_budget::Counter::Work,
                    self.token().text.len() as u64,
                )
                .map_err(|error| native_adapter::transport_error(error.into()))?;
            let value = parse_char_literal(self.token().text).map_err(|e| self.error(e))?;
            self.cursor.bump();
            return Ok(RangeBound::Char(value));
        }
        if self.int_limit_start() {
            let ty = integer_type(self.token().text).ok_or_else(|| self.error("integer type"))?;
            self.cursor.bump();
            self.expect("::")?;
            let max = self.is("MAX");
            self.cursor.bump();
            return Ok(RangeBound::IntLimit { ty, max });
        }
        let negative = self.eat("-");
        if self.token().string || !self.token().text.starts_with(|c: char| c.is_ascii_digit()) {
            return Err(self.error("expected an integer literal after minus"));
        }
        let literal = parse_integer_literal(self.token().text).map_err(|e| self.error(&e))?;
        self.cursor.bump();
        Ok(RangeBound::Int { literal, negative })
    }
    /// Literal and range patterns, kept out of the recursive pattern frame.
    #[inline(never)]
    fn scalar_pattern(&mut self) -> Result<Pattern, ParseError> {
        if self.is("..=") || self.is("..") {
            let inclusive = self.is("..=");
            self.cursor.bump();
            if !self.range_bound_start() {
                return Err(if self.token().is_ident() {
                    self.unsupported("path and constant range pattern bounds are unsupported")
                } else {
                    self.error("expected a range pattern upper bound")
                });
            }
            let hi = self.range_bound()?;
            return Ok(Pattern::Range {
                lo: None,
                hi: Some(hi),
                inclusive,
            });
        }
        if self.range_bound_start() {
            if self.token().string {
                return Err(self.unsupported("string literal patterns are unsupported"));
            }
            let lo = self.range_bound()?;
            if self.is("..=") || self.is("..") {
                let inclusive = self.is("..=");
                self.cursor.bump();
                let hi = if self.range_bound_start() {
                    Some(self.range_bound()?)
                } else if self.token().is_ident() {
                    return Err(
                        self.unsupported("path and constant range pattern bounds are unsupported")
                    );
                } else if inclusive {
                    return Err(self.error("inclusive range pattern requires an upper bound"));
                } else {
                    None
                };
                return Ok(Pattern::Range {
                    lo: Some(lo),
                    hi,
                    inclusive,
                });
            }
            return match lo {
                RangeBound::Int { literal, negative } => Ok(Pattern::Int { literal, negative }),
                RangeBound::Char(value) => Ok(Pattern::Char(value)),
                RangeBound::IntLimit { ty, max } => Ok(Pattern::Range {
                    lo: Some(RangeBound::IntLimit { ty, max }),
                    hi: Some(RangeBound::IntLimit { ty, max }),
                    inclusive: true,
                }),
            };
        }
        Err(self.error("expected a literal or range pattern"))
    }
    /// `ref`/`ref mut` bindings and `&pat`/`&mut pat`, kept out of
    /// `pattern_inner` so the recursive parser frame stays small.
    #[inline(never)]
    fn reference_pattern(&mut self) -> Result<Pattern, ParseError> {
        if self.eat("ref") {
            let kind = if self.eat("mut") {
                ir::BorrowKind::Mut
            } else {
                ir::BorrowKind::Shared
            };
            return Ok(Pattern::Binding {
                name: self.ident()?,
                mutable: false,
                by_ref: Some(kind),
            });
        }
        let double = self.is("&&");
        self.cursor.bump();
        let kind = if self.eat("mut") {
            ir::BorrowKind::Mut
        } else {
            ir::BorrowKind::Shared
        };
        let inner = self.pattern_no_alternatives()?;
        let pattern = Pattern::Ref {
            kind,
            pattern: Box::new(inner),
        };
        Ok(if double {
            Pattern::Ref {
                kind: ir::BorrowKind::Shared,
                pattern: Box::new(pattern),
            }
        } else {
            pattern
        })
    }
    fn pattern_inner(&mut self) -> Result<Pattern, ParseError> {
        if self.is("..=") || self.is("..") || self.range_bound_start() {
            return self.scalar_pattern();
        }
        if self.token().string {
            return Err(self.unsupported("string literal patterns are unsupported"));
        }
        if self.is("ref") || self.is("&") || self.is("&&") {
            return self.reference_pattern();
        }
        if self.eat("true") {
            return Ok(Pattern::Bool(true));
        }
        if self.eat("false") {
            return Ok(Pattern::Bool(false));
        }
        if self.eat("_") {
            return Ok(Pattern::Wildcard);
        }
        if self.eat("[") {
            return self.array_pattern();
        }
        if self.eat("(") {
            if self.eat(")") {
                return Ok(Pattern::Tuple(Vec::new()));
            }
            if self.eat("..") {
                return Ok(Pattern::TupleRest(PositionalRest {
                    prefix: Vec::new(),
                    suffix: self.positional_rest_suffix(")")?,
                }));
            }
            let first = self.pattern()?;
            if self.is("@") {
                return Err(self.unsupported("@ positional rest patterns are unsupported"));
            }
            if !self.eat(",") {
                self.expect(")")?;
                return Ok(first);
            }
            let mut fields = vec![first];
            while !self.is(")") {
                if self.eat("..") {
                    return Ok(Pattern::TupleRest(PositionalRest {
                        prefix: fields,
                        suffix: self.positional_rest_suffix(")")?,
                    }));
                }
                fields.push(self.pattern()?);
                if self.is("@") {
                    return Err(self.unsupported("@ positional rest patterns are unsupported"));
                }
                if !self.eat(",") {
                    break;
                }
            }
            self.expect(")")?;
            return Ok(Pattern::Tuple(fields));
        }
        if self.eat("mut") {
            return Ok(Pattern::Binding {
                name: self.ident()?,
                mutable: true,
                by_ref: None,
            });
        }
        let generic_path = self.generic_path(false)?;
        let name = generic_path
            .checked_path(self.names)
            .map_err(native_adapter::name_error)?;
        let wrap = |pattern| {
            if generic_path
                .segments
                .iter()
                .any(|s| !s.args.is_empty() || !s.associated_types.is_empty())
            {
                Pattern::Apply {
                    path: generic_path.clone(),
                    pattern: Box::new(pattern),
                }
            } else {
                pattern
            }
        };
        if self.eat("(") {
            let mut fields = Vec::new();
            while !self.is(")") {
                if self.eat("..") {
                    let mut prefix = Vec::new();
                    self.names
                        .budget()
                        .charge(super::expansion_budget::Counter::Work, fields.len() as u64)
                        .map_err(|error| native_adapter::transport_error(error.into()))?;
                    self.reserve_rest_patterns(&mut prefix, fields.len())?;
                    prefix.extend(fields.into_iter().map(|(_, pattern)| pattern));
                    let suffix = self.positional_rest_suffix(")")?;
                    return Ok(wrap(Pattern::VariantRest {
                        name,
                        fields: PositionalRest { prefix, suffix },
                    }));
                }
                fields.push((FieldSelector::Tuple(fields.len()), self.pattern()?));
                if self.is("@") {
                    return Err(self.unsupported("@ positional rest patterns are unsupported"));
                }
                if !self.eat(",") {
                    break;
                }
            }
            self.expect(")")?;
            return Ok(wrap(Pattern::Variant {
                name,
                shape: ir::VariantShape::Tuple,
                fields,
                rest: false,
            }));
        }
        if !self.eat("{") {
            if name.single().is_none() {
                return Ok(wrap(Pattern::Path(name)));
            }
            return Ok(Pattern::Binding {
                name: name
                    .single()
                    .ok_or_else(|| self.error("expected local binding name"))?,
                mutable: false,
                by_ref: None,
            });
        }
        let mut fields = Vec::new();
        let mut rest = false;
        while !self.is("}") {
            if self.eat("..") {
                rest = true;
                if !self.is("}") {
                    return Err(self.error("struct rest pattern must be last"));
                }
                break;
            }
            let by_ref = if self.eat("ref") {
                Some(if self.eat("mut") {
                    ir::BorrowKind::Mut
                } else {
                    ir::BorrowKind::Shared
                })
            } else {
                None
            };
            let mutable = by_ref.is_none() && self.eat("mut");
            let token = self.token();
            if token.matches(token.text) && token.text.starts_with(|c: char| c.is_ascii_digit()) {
                return Err(self.unsupported("numeric braced constructor patterns are unsupported"));
            }
            let field = self.ident()?;
            let pattern = if self.eat(":") {
                if mutable || by_ref.is_some() {
                    return Err(
                        self.error("mut or ref must apply to a binding, not a field selector")
                    );
                }
                self.pattern()?
            } else {
                Pattern::Binding {
                    name: field,
                    mutable,
                    by_ref,
                }
            };
            fields.push((field, pattern));
            if !self.eat(",") {
                break;
            }
        }
        self.expect("}")?;
        Ok(wrap(Pattern::Struct { name, fields, rest }))
    }
    fn compound_statement(&mut self, target: Expr, op: BinaryOp) -> Result<Stmt, ParseError> {
        let value = self.expr(0)?;
        self.expect(";")?;
        if let Expr::DynamicIndex { base, index } = target {
            return Ok(Stmt::AssignIndexOp {
                array: self.assignment_place(*base)?,
                index: *index,
                op,
                value,
            });
        }
        Ok(Stmt::AssignOp {
            place: self.assignment_place(target)?,
            op,
            value,
        })
    }
    #[inline(never)]
    fn validate_let_else_initializer(
        &self,
        start: token_cursor::CursorMark<'input>,
    ) -> Result<(), ParseError> {
        let invalid_range = || {
            native_adapter::transport_error(super::token_session::TokenError::InvalidInput(
                "invalid let-else initializer token range",
            ))
        };
        let end = self.cursor.checkpoint();
        let mut scan = TokenCursor::new(self.input);
        if !scan.restore(start) {
            return Err(invalid_range());
        }
        let mut depth = 0usize;
        let mut last = None;
        while !scan.is_at(end) {
            self.names
                .budget()
                .charge(super::expansion_budget::Counter::Work, 1)
                .map_err(|error| native_adapter::transport_error(error.into()))?;
            let token = scan.bump();
            if token.matches("") {
                return Err(invalid_range());
            }
            // Parentheses and macro argument groups remain actual source
            // tokens. Never infer these restrictions from an expanded Expr:
            // m!() may legitimately expand to an unparenthesized lazy boolean.
            if depth == 0 && (token.matches("&&") || token.matches("||")) {
                return Err(self.error_at(
                    &token,
                    "let-else lazy boolean initializer requires parentheses",
                ));
            }
            if token.matches("(") || token.matches("[") || token.matches("{") {
                depth = depth.checked_add(1).ok_or_else(invalid_range)?;
            } else if token.matches(")") || token.matches("]") || token.matches("}") {
                depth = depth.checked_sub(1).ok_or_else(invalid_range)?;
            }
            last = Some(token);
        }
        let last = last.ok_or_else(invalid_range)?;
        if depth != 0 {
            return Err(invalid_range());
        }
        if last.matches("}") {
            return Err(self.error_at(
                &last,
                "let-else initializer ending in a brace requires parentheses",
            ));
        }
        Ok(())
    }
    fn block_inner(&mut self) -> Result<Block, ParseError> {
        self.expect("{")?;
        let mut statements = Vec::new();
        let mut tail = None;
        while !self.is("}") {
            if self.is("") {
                return Err(self.error("unterminated block"));
            }
            if self.eat(";") {
                continue;
            }
            if self.is("macro_rules") && self.cursor.peek(1).is_some_and(|token| token.matches("!"))
            {
                self.local_macro_definition()?;
                continue;
            }
            if self.is("mod") || self.is("use") || self.is("fn") || self.is("pub") {
                return Err(self.unsupported("block-local items are unsupported"));
            }
            if self.try_for_statement(&mut statements)? {
                continue;
            }
            if self.eat("let") {
                let pattern = self.pattern()?;
                // Local inference is performed by each independent function checker.
                let annotation = if self.eat(":") {
                    Some(self.ty()?)
                } else {
                    None
                };
                if self.eat(";") {
                    if matches!(
                        pattern,
                        Pattern::Bool(_)
                            | Pattern::Int { .. }
                            | Pattern::Char(_)
                            | Pattern::Range { .. }
                    ) {
                        return Err(self.error("refutable literal pattern in let binding"));
                    }
                    let Pattern::Binding {
                        name,
                        mutable,
                        by_ref: None,
                    } = pattern
                    else {
                        return Err(
                            self.unsupported("deferred pattern declarations are unsupported")
                        );
                    };
                    let ty = annotation.unwrap_or(Type::Infer);
                    self.bind_lexical(name)?;
                    statements.push(Stmt::Declare {
                        name: BindingName::Named(name),
                        ty,
                        mutable,
                    });
                    continue;
                }
                self.expect("=")?;
                let initializer = self.cursor.checkpoint();
                let value = self.expr(0)?;
                if self.is("else") {
                    self.validate_let_else_initializer(initializer)?;
                    self.cursor.bump();
                    // Neither this pattern nor its declarations exist in the
                    // initializer or the genuine diverging else block.
                    let else_block = self.block()?;
                    self.expect(";")?;
                    let pattern = self
                        .names
                        .try_parser_box(pattern)
                        .map_err(native_adapter::name_error)?;
                    let value = self
                        .names
                        .try_parser_box(value)
                        .map_err(native_adapter::name_error)?;
                    self.bind_pattern_lexical(&pattern)?;
                    statements.push(Stmt::LetElse {
                        pattern,
                        ty: annotation.unwrap_or(Type::Infer),
                        value,
                        else_block,
                    });
                    continue;
                }
                if matches!(
                    pattern,
                    Pattern::Bool(_)
                        | Pattern::Int { .. }
                        | Pattern::Char(_)
                        | Pattern::Range { .. }
                ) {
                    return Err(self.error("refutable literal pattern in let binding"));
                }
                self.expect(";")?;
                self.bind_pattern_lexical(&pattern)?;
                let ty = annotation.unwrap_or(Type::Infer);
                if let Pattern::Binding {
                    name,
                    mutable,
                    by_ref: None,
                } = pattern
                {
                    statements.push(Stmt::Let {
                        name: BindingName::Named(name),
                        mutable,
                        ty,
                        value,
                    });
                } else {
                    statements.push(Stmt::LetPattern { pattern, ty, value });
                }
            } else if self.eat("return") {
                let value = if self.is(";") || self.is("}") {
                    None
                } else {
                    Some(self.expr(0)?)
                };
                if !self.eat(";") && !self.is("}") {
                    return Err(self.error("expected semicolon or block end after return"));
                }
                statements.push(Stmt::Return(value));
            } else if self.is("while")
                || (self.token().is_lifetime()
                    && self.cursor.peek(1).is_some_and(|t| t.text == ":")
                    && self.cursor.peek(2).is_some_and(|t| t.text == "while"))
            {
                let label = if self.is("while") {
                    None
                } else {
                    let label = self.input.mint_name(&self.token(), self.names)?;
                    self.cursor.bump();
                    self.expect(":")?;
                    Some(label)
                };
                self.expect("while")?;
                if self.eat("let") {
                    statements.push(self.while_let_statement(label)?);
                    continue;
                }
                let condition = self.condition_with(0, ConditionPurpose::Conditional)?;
                let body = self.block()?;
                statements.push(Stmt::While {
                    label,
                    condition,
                    body,
                });
            } else {
                // Statement eligibility comes from written syntax, not a
                // lowered macro's AST shape (assert! lowers to Expr::If).
                let source_blocklike =
                    matches!(self.token().text, "if" | "match" | "loop" | "{" | "unsafe")
                        || self.token().is_lifetime();
                let brace_macro = self.is_identifier()
                    && self.cursor.peek(1).is_some_and(|token| token.matches("!"))
                    && self.cursor.peek(2).is_some_and(|token| token.matches("{"));
                let expr = self.statement_expr()?;
                if let Some(op) = compound_operator(self.token()) {
                    self.cursor.bump();
                    statements.push(self.compound_statement(expr, op)?);
                } else if self.eat("=") {
                    let value = self.expr(0)?;
                    self.expect(";")?;
                    if let Expr::DynamicIndex { base, index } = expr {
                        let array = self.assignment_place(*base)?;
                        statements.push(Stmt::AssignIndex {
                            array,
                            index: *index,
                            value,
                        });
                        continue;
                    }
                    if let Expr::Deref { value: carrier } = &expr
                        && genuine_call(carrier)
                    {
                        let Expr::Deref { value: carrier } = expr else {
                            unreachable!()
                        };
                        statements.push(Stmt::AssignDeref {
                            carrier: *carrier,
                            value,
                        });
                        continue;
                    }
                    let place = self.assignment_place(expr)?;
                    if place.fields.is_empty() {
                        statements.push(Stmt::Assign {
                            name: place.local,
                            value,
                        });
                    } else {
                        statements.push(Stmt::AssignPlace { place, value });
                    }
                } else if self.eat(";") {
                    statements.push(Stmt::Expr(expr));
                } else if self.is("}") {
                    tail = Some(Box::new(expr));
                    break;
                } else if (brace_macro && matches!(expr, Expr::Panic(_) | Expr::Output(_)))
                    || ((source_blocklike || brace_macro)
                        && matches!(
                            expr,
                            Expr::If { .. }
                                | Expr::IfLet { .. }
                                | Expr::Block(_)
                                | Expr::Unsafe(_)
                                | Expr::LabeledBlock { .. }
                                | Expr::Loop { .. }
                                | Expr::Match { .. }
                        ))
                {
                    statements.push(Stmt::ImplicitExpr(expr));
                } else {
                    return Err(self.error("expected semicolon or block end"));
                }
            }
        }
        self.expect("}")?;
        Ok(Block { statements, tail })
    }
    fn assignment_place(&self, mut expr: Expr) -> Result<Place, ParseError> {
        let mut fields = Vec::new();
        loop {
            match expr {
                Expr::Var(path) => {
                    let local = if path.anchor() == PathAnchor::SelfModule && path.is_empty() { LocalName::Receiver } else { LocalName::Named(path.single().ok_or_else(|| self.unsupported("assignment requires a local-rooted place; destructuring and temporary assignment are unsupported"))?) };
                    fields.reverse(); return Ok(Place { local, fields });
                }
                Expr::Deref {value}=>{fields.push(FieldSelector::Deref);expr = *value;}
                Expr::Field { base, field } => { fields.push(field); expr = *base; }
                Expr::Index { base,index } => {fields.push(FieldSelector::ConstantIndex(index));expr = *base;}
                _ => return Err(self.unsupported("assignment requires a local-rooted place; destructuring and temporary assignment are unsupported")),
            }
        }
    }
    fn nested_expr(&mut self) -> Result<Expr, ParseError> {
        let previous = self.allow_struct;
        let previous_purpose = self.condition_purpose;
        self.allow_struct = true;
        self.condition_purpose = ConditionPurpose::Ordinary;
        let result = self.expr(0);
        self.allow_struct = previous;
        self.condition_purpose = previous_purpose;
        result
    }
    fn condition(&mut self) -> Result<Expr, ParseError> {
        self.condition_with(0, ConditionPurpose::Ordinary)
    }
    fn condition_with(&mut self, min: u8, purpose: ConditionPurpose) -> Result<Expr, ParseError> {
        let previous = self.allow_struct;
        let previous_purpose = self.condition_purpose;
        self.allow_struct = false;
        self.condition_purpose = purpose;
        let result = self.expr(min);
        self.allow_struct = previous;
        self.condition_purpose = previous_purpose;
        result
    }
    fn expr(&mut self, min: u8) -> Result<Expr, ParseError> {
        if self.nesting >= 128 {
            return Err(self.limit("syntax nesting exceeds limit 128"));
        }
        self.nesting += 1;
        let result = self.expr_inner_mode(min, false);
        self.nesting -= 1;
        result
    }
    fn statement_expr(&mut self) -> Result<Expr, ParseError> {
        if self.nesting >= 128 {
            return Err(self.limit("syntax nesting exceeds limit128"));
        }
        self.nesting += 1;
        let result = self.expr_inner_mode(0, true);
        self.nesting -= 1;
        result
    }
    fn cast_after_as(&mut self, value: Expr) -> Result<Expr, ParseError> {
        self.expect("as")?;
        if self.is("(") {
            return Err(self.unsupported("parenthesized cast target types are unsupported"));
        }
        let target = self.ty()?;
        if self.is("<") || self.is("<<") {
            return Err(self.error("parenthesize the cast before a less-than comparison; otherwise '<' starts generic arguments"));
        }
        if self.is(".") || self.is("[") || self.is("(") {
            return Err(self.error(
                "a cast cannot be followed directly by a postfix operation; parenthesize the cast",
            ));
        }
        let expr = Expr::Cast {
            value: Box::new(value),
            target,
        };
        validate_depth(DepthNode::Expr(&expr)).map_err(|e| self.depth_error(&e))?;
        Ok(expr)
    }
    fn named_expr(&mut self) -> Result<Expr, ParseError> {
        let println_syntax =
            self.is("println") && self.cursor.peek(1).is_some_and(|token| token.matches("!"));
        if !println_syntax && self.cursor.peek(1).is_some_and(|token| token.matches("!")) {
            return self.local_macro_expression();
        }
        self.ordinary_named_expr(println_syntax)
    }
    #[inline(never)]
    fn ordinary_named_expr(&mut self, println_syntax: bool) -> Result<Expr, ParseError> {
        let generic_path = self.generic_path(false)?;
        let name = generic_path
            .checked_path(self.names)
            .map_err(native_adapter::name_error)?;
        if let Some(head) = name.segments().first() {
            self.reference_lexical(*head)?;
        }
        let value = if self.eat("!") {
            if !println_syntax || name.single().is_none() {
                return Err(self.error("only println! is supported"));
            }
            self.expect("(")?;
            if self.is(")") {
                self.expect(")")?;
                Expr::Println(String::new())
            } else {
                return self.scalar_output(")", "println", None);
            }
        } else if self.eat("(") {
            let mut args = Vec::new();
            if !self.is(")") {
                loop {
                    args.push(self.nested_expr()?);
                    if !self.eat(",") || self.is(")") {
                        break;
                    }
                }
            }
            self.expect(")")?;
            Expr::Call { name, args }
        } else if self.allow_struct && self.eat("{") {
            let mut fields = Vec::new();
            while !self.is("}") {
                let token = self.token();
                if token.matches(token.text) && token.text.starts_with(|c: char| c.is_ascii_digit())
                {
                    return Err(
                        self.unsupported("numeric braced constructor fields are unsupported")
                    );
                }
                let field = self.ident()?;
                let value = if self.eat(":") {
                    self.expr(0)?
                } else {
                    self.reference_lexical(field)?;
                    Expr::Var(
                        AstPath::new(self.names, PathAnchor::Relative, &[field])
                            .map_err(native_adapter::name_error)?,
                    )
                };
                fields.push((field, value));
                if !self.eat(",") {
                    break;
                }
            }
            self.expect("}")?;
            Expr::Struct { name, fields }
        } else {
            Expr::Var(name)
        };
        Ok(
            if generic_path
                .segments
                .iter()
                .any(|s| !s.args.is_empty() || !s.associated_types.is_empty())
            {
                Expr::Apply {
                    path: generic_path,
                    value: Box::new(value),
                }
            } else {
                value
            },
        )
    }
    fn field_or_method_after_dot(&mut self, left: Expr) -> Result<Expr, ParseError> {
        Ok({
            let field = if self.token().text.starts_with(|c: char| c.is_ascii_digit()) {
                let token = self.token();
                if token.string
                    || !token.text.bytes().all(|c| c.is_ascii_digit())
                    || (token.text.len() > 1 && token.text.starts_with('0'))
                {
                    return Err(self
                        .error("tuple field index must be a canonical unsigned decimal integer"));
                }
                let index = token
                    .text
                    .parse::<usize>()
                    .map_err(|_| self.error("tuple field index is out of range"))?;
                self.cursor.bump();
                FieldSelector::Tuple(index)
            } else {
                FieldSelector::Named(self.ident()?)
            };
            let type_args = if self.is("::") && self.cursor.peek(1).is_some_and(|t| t.text == "<") {
                self.expect("::")?;
                self.type_arguments()?
            } else {
                Vec::new()
            };
            if self.eat("(") {
                let FieldSelector::Named(name) = field else {
                    return Err(self.error("tuple fields are not methods"));
                };
                let mut args = Vec::new();
                while !self.is(")") {
                    args.push(self.nested_expr()?);
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect(")")?;
                Expr::MethodCall {
                    receiver: Box::new(left),
                    name,
                    type_args,
                    args,
                }
            } else {
                if !type_args.is_empty() {
                    return Err(self.error("method type arguments require a call"));
                }
                Expr::Field {
                    base: Box::new(left),
                    field,
                }
            }
        })
    }
    fn expr_inner_mode(&mut self, min: u8, statement: bool) -> Result<Expr, ParseError> {
        if self.token().is_lifetime() {
            return self.labeled_expression(min, statement);
        }
        let statement_blocklike =
            statement && matches!(self.token().text, "if" | "match" | "loop" | "{" | "unsafe");
        if self.is("&&") {
            return Err(self.unsupported("nested reference borrows are unsupported in B1"));
        }
        let left = self.expr_prefix(statement)?;
        self.expr_continuation(left, min, statement_blocklike)
    }
    #[inline(never)]
    fn labeled_expression(&mut self, min: u8, statement: bool) -> Result<Expr, ParseError> {
        let label = self.input.mint_name(&self.token(), self.names)?;
        self.cursor.bump();
        self.expect(":")?;
        let left = if self.eat("loop") {
            Expr::Loop {
                label: Some(label),
                body: self.block()?,
            }
        } else if self.is("{") {
            Expr::LabeledBlock {
                label,
                body: self.block()?,
            }
        } else if self.is("while")
            && self
                .cursor
                .peek(1)
                .is_some_and(|token| token.matches("let"))
        {
            return Err(self.unsupported("while let expressions are unsupported"));
        } else {
            return Err(self.error("a label must precede a loop, while, or block"));
        };
        self.expr_continuation(left, min, statement)
    }
    #[inline(never)]
    fn trait_call_prefix(&mut self) -> Result<Expr, ParseError> {
        let owner = self.ty()?;
        self.expect("as")?;
        let trait_path = self.trait_path()?;
        let trait_path = self
            .names
            .try_parser_box(trait_path)
            .map_err(native_adapter::name_error)?;
        self.expect(">")?;
        self.expect("::")?;
        let name = self.ident()?;
        if self.is("::") {
            return Err(self.unsupported("generic trait methods are unsupported"));
        }
        self.expect("(")?;
        let mut args = Vec::new();
        while !self.is(")") {
            if args.len() >= 4096 {
                return Err(self.limit("call argument limit exceeded"));
            }
            args.push(self.nested_expr()?);
            if !self.eat(",") {
                break;
            }
        }
        self.expect(")")?;
        Ok(Expr::TraitCall {
            owner,
            trait_path,
            name,
            args,
        })
    }
    #[inline(never)]
    fn expr_prefix(&mut self, statement: bool) -> Result<Expr, ParseError> {
        if self.token().is_opaque_literal() {
            return self.opaque_literal_expression();
        }
        if self.is_identifier()
            && !self.is("println")
            && self.cursor.peek(1).is_some_and(|token| token.matches("!"))
        {
            return self.local_macro_expression();
        }
        // These recursive forms must not retain the unrelated aggregate,
        // borrow, break-value and trait-call temporaries from the ordinary
        // prefix frame for every admitted syntax nesting level.
        if self.eat("(") {
            return self.group_prefix();
        }
        if self.is("{") {
            return Ok(Expr::Block(self.block()?));
        }
        if !self.token().string
            && self.is("unsafe")
            && self.cursor.peek(1).is_some_and(|token| token.matches("{"))
        {
            self.cursor.bump();
            return Ok(Expr::Unsafe(self.block()?));
        }
        self.ordinary_expr_prefix(statement)
    }
    #[inline(never)]
    fn ordinary_expr_prefix(&mut self, statement: bool) -> Result<Expr, ParseError> {
        if self.is("while")
            && self
                .cursor
                .peek(1)
                .is_some_and(|token| token.matches("let"))
        {
            return Err(self.unsupported("while let expressions are unsupported"));
        }
        if self.is("const") {
            return Err(
                self.unsupported("const expressions and constant evaluation are unsupported")
            );
        }
        let left = if self.eat("<") {
            self.trait_call_prefix()?
        } else if self.eat("&") {
            let kind = if self.eat("mut") {
                ir::BorrowKind::Mut
            } else {
                ir::BorrowKind::Shared
            };
            let value = self.expr(11)?;
            match borrow_place(&value) {
                Some(place) => Expr::Borrow { kind, place },
                None => Expr::BorrowValue {
                    kind,
                    value: Box::new(value),
                },
            }
        } else if self.eat("*") {
            Expr::Deref {
                value: Box::new(self.expr(11)?),
            }
        } else if self.eat("-") {
            Expr::Unary {
                op: UnaryOp::Neg,
                value: Box::new(self.expr(11)?),
            }
        } else if self.eat("!") {
            Expr::Unary {
                op: UnaryOp::Not,
                value: Box::new(self.expr(11)?),
            }
        } else if self.token().string {
            let text = self.string_literal_value(self.token().text)?;
            self.cursor.bump();
            Expr::Str(text)
        } else if self.token().is_char() {
            self.names
                .budget()
                .charge(
                    super::expansion_budget::Counter::Work,
                    self.token().text.len() as u64,
                )
                .map_err(|error| native_adapter::transport_error(error.into()))?;
            let value = parse_char_literal(self.token().text).map_err(|e| self.error(e))?;
            self.cursor.bump();
            Expr::Char(value)
        } else if self.eat("true") {
            Expr::Bool(true)
        } else if self.eat("false") {
            Expr::Bool(false)
        } else if self.eat("loop") {
            Expr::Loop {
                label: None,
                body: self.block()?,
            }
        } else if self.eat("break") {
            let label = if self.token().is_lifetime() {
                let label = self.input.mint_name(&self.token(), self.names)?;
                self.cursor.bump();
                Some(label)
            } else {
                None
            };
            let value = if matches!(self.token().text, ";" | "}" | ")" | "]" | "," | "") {
                None
            } else {
                Some(Box::new(self.expr(0)?))
            };
            Expr::Break { label, value }
        } else if self.eat("continue") {
            let label = if self.token().is_lifetime() {
                let label = self.input.mint_name(&self.token(), self.names)?;
                self.cursor.bump();
                Some(label)
            } else {
                None
            };
            Expr::Continue { label }
        } else if self.eat("match") {
            return self.match_prefix();
        } else if self.eat("if") {
            return self.if_prefix(statement);
        } else if self.eat("[") {
            return self.array_prefix();
        } else if !self.token().string
            && self.token().text.starts_with(|c: char| c.is_ascii_digit())
        {
            return self.number_prefix();
        } else {
            self.named_expr()?
        };
        Ok(left)
    }
    // Keep unrelated syntax temporaries out of every recursive prefix frame.
    // In particular u128's alignment must not multiply match-arm and literal
    // scratch storage across all 128 admitted syntax levels.
    #[inline(never)]
    fn number_prefix(&mut self) -> Result<Expr, ParseError> {
        let text = self.token().text.to_owned();
        let expression =
            if let Some(literal) = parse_float_literal(&text).map_err(|e| self.error(&e))? {
                Expr::Float(literal)
            } else {
                Expr::Integer(parse_integer_literal(&text).map_err(|e| self.error(&e))?)
            };
        self.cursor.bump();
        Ok(expression)
    }
    #[inline(never)]
    fn group_prefix(&mut self) -> Result<Expr, ParseError> {
        if self.eat(")") {
            return Ok(Expr::Block(Block {
                statements: Vec::new(),
                tail: None,
            }));
        }
        let first = self.nested_expr()?;
        if !self.eat(",") {
            self.expect(")")?;
            return Ok(first);
        }
        let mut fields = vec![first];
        while !self.is(")") {
            fields.push(self.nested_expr()?);
            if !self.eat(",") {
                break;
            }
        }
        self.expect(")")?;
        Ok(Expr::Tuple(fields))
    }
    #[inline(never)]
    fn array_prefix(&mut self) -> Result<Expr, ParseError> {
        if self.eat("]") {
            return Ok(Expr::Array(Vec::new()));
        }
        let first = self.nested_expr()?;
        if self.eat(";") {
            let len = self.array_number("array repeat length")?;
            self.expect("]")?;
            return Ok(Expr::Repeat {
                value: Box::new(first),
                len,
            });
        }
        let mut values = vec![first];
        while self.eat(",") && !self.is("]") {
            if values.len() >= ir::MAX_ARRAY_LEN {
                return Err(self.limit("array literal exceeds supported length"));
            }
            values.push(self.nested_expr()?);
        }
        self.expect("]")?;
        Ok(Expr::Array(values))
    }
    #[inline(never)]
    fn if_prefix(&mut self, statement: bool) -> Result<Expr, ParseError> {
        if self.eat("let") {
            return self.if_let_prefix(statement);
        }
        let condition = Box::new(self.condition_with(0, ConditionPurpose::Conditional)?);
        let then_block = self.block()?;
        let else_block = if self.eat("else") {
            if self.is("if") {
                Block {
                    statements: Vec::new(),
                    tail: Some(Box::new(if statement {
                        self.statement_expr()?
                    } else {
                        self.expr(0)?
                    })),
                }
            } else {
                self.block()?
            }
        } else {
            Block {
                statements: Vec::new(),
                tail: None,
            }
        };
        Ok(Expr::If {
            condition,
            then_block,
            else_block,
        })
    }
    #[inline(never)]
    fn if_let_prefix(&mut self, statement: bool) -> Result<Expr, ParseError> {
        let pattern = self.pattern()?;
        if self.is("|") || self.is("@") || self.is("if") {
            return Err(
                self.unsupported("if let guards, or-patterns and @ patterns are unsupported")
            );
        }
        self.expect("=")?;
        // Do not consume a let-chain into the scrutinee or publish its bindings.
        let scrutinee = self.condition_with(3, ConditionPurpose::Conditional)?;
        if self.is("&&") || self.is("||") {
            return Err(self.unsupported("if let chains are unsupported"));
        }
        let pattern = self
            .names
            .try_parser_box(pattern)
            .map_err(native_adapter::name_error)?;
        let scrutinee = self
            .names
            .try_parser_box(scrutinee)
            .map_err(native_adapter::name_error)?;
        let frontier = self.enter_lexical()?;
        let then_result = (|| {
            self.bind_pattern_lexical(&pattern)?;
            self.block()
        })();
        // A failed then parse must not leave a usable binding/macro frontier.
        self.restore_frontier(frontier);
        let then_block = then_result?;
        let else_branch = if self.eat("else") {
            let branch = if self.is("if") {
                if statement {
                    self.statement_expr()?
                } else {
                    self.expr(0)?
                }
            } else {
                Expr::Block(self.block()?)
            };
            Some(
                self.names
                    .try_parser_box(branch)
                    .map_err(native_adapter::name_error)?,
            )
        } else {
            None
        };
        Ok(Expr::IfLet {
            pattern,
            scrutinee,
            then_block,
            else_branch,
        })
    }
    /// Keep the new statement's large parse temporaries off recursive block frames.
    #[inline(never)]
    fn try_for_statement(&mut self, statements: &mut Vec<Stmt>) -> Result<bool, ParseError> {
        if !self.is("for")
            && !(self.token().is_lifetime()
                && self.cursor.peek(1).is_some_and(|t| t.text == ":")
                && self.cursor.peek(2).is_some_and(|t| t.text == "for"))
        {
            return Ok(false);
        }
        let label = if self.is("for") {
            None
        } else {
            let label = self.input.mint_name(&self.token(), self.names)?;
            self.cursor.bump();
            self.expect(":")?;
            Some(label)
        };
        self.expect("for")?;
        statements.push(self.for_statement(label)?);
        Ok(true)
    }
    #[inline(never)]
    fn for_statement(&mut self, label: Option<AstIdent>) -> Result<Stmt, ParseError> {
        let pattern = self.pattern()?;
        self.expect("in")?;
        let mut adapters = Vec::new();
        let iterable = if self.is("(") {
            let mark = self.cursor.checkpoint();
            self.cursor.bump();
            let start = self.nested_expr()?;
            if self.is("..") || self.is("..=") {
                let inclusive = self.eat("..=");
                if !inclusive {
                    self.expect("..")?;
                }
                let end = self.nested_expr()?;
                self.expect(")")?;
                while self.eat(".") {
                    let name = self.token().text;
                    let adapter = match name {
                        "rev" => ForAdapter::Rev,
                        "enumerate" => ForAdapter::Enumerate,
                        "into_iter" => ForAdapter::IntoIter,
                        "iter" => ForAdapter::Iter,
                        _ => {
                            return Err(self.unsupported(
                                "unsupported: for-loop range adapter (supported: rev, enumerate, into_iter)",
                            ));
                        }
                    };
                    self.cursor.bump();
                    self.expect("(")?;
                    self.expect(")")?;
                    if adapters.len() >= 16 {
                        return Err(self.limit("for-loop adapter count exceeds limit 16"));
                    }
                    adapters.push(adapter);
                }
                ForIterable::Range {
                    start,
                    end,
                    inclusive,
                }
            } else {
                if !self.cursor.restore(mark) {
                    return Err(self.error("for-loop iterable checkpoint lost"));
                }
                self.for_value_iterable(&mut adapters)?
            }
        } else if self.is("..") || self.is("..=") {
            return Err(
                self.unsupported("unsupported: unbounded or half-open-start for-loop ranges")
            );
        } else {
            let start = self.condition()?;
            if self.is("..") || self.is("..=") {
                let inclusive = self.eat("..=");
                if !inclusive {
                    self.expect("..")?;
                }
                if self.is("{") {
                    return Err(self.unsupported("unsupported: unbounded for-loop ranges"));
                }
                let end = self.condition()?;
                ForIterable::Range {
                    start,
                    end,
                    inclusive,
                }
            } else {
                self.peel_for_adapters(start, &mut adapters)?
            }
        };
        let pattern = self
            .names
            .try_parser_box(pattern)
            .map_err(native_adapter::name_error)?;
        let source = self
            .names
            .try_parser_box(ForSource { iterable, adapters })
            .map_err(native_adapter::name_error)?;
        let frontier = self.enter_lexical()?;
        let body_result = (|| {
            self.bind_pattern_lexical(&pattern)?;
            self.block()
        })();
        self.restore_frontier(frontier);
        let body = body_result?;
        let has_semicolon = self.eat(";");
        Ok(Stmt::For {
            label,
            pattern,
            source,
            body,
            has_semicolon,
        })
    }
    fn for_value_iterable(
        &mut self,
        adapters: &mut Vec<ForAdapter>,
    ) -> Result<ForIterable, ParseError> {
        let value = self.condition()?;
        if self.is("..") || self.is("..=") {
            return Err(self.unsupported("unsupported: parenthesized range start in for loop"));
        }
        self.peel_for_adapters(value, adapters)
    }
    /// Retain trailing zero-argument iterator adapters as written structure.
    fn peel_for_adapters(
        &mut self,
        mut value: Expr,
        adapters: &mut Vec<ForAdapter>,
    ) -> Result<ForIterable, ParseError> {
        while let Expr::MethodCall {
            name,
            type_args,
            args,
            ..
        } = &value
        {
            if !type_args.is_empty() || !args.is_empty() {
                break;
            }
            let adapter = match self
                .names
                .spelling(*name)
                .map_err(native_adapter::name_error)?
            {
                "iter" => ForAdapter::Iter,
                "into_iter" => ForAdapter::IntoIter,
                "rev" => ForAdapter::Rev,
                "enumerate" => ForAdapter::Enumerate,
                _ => break,
            };
            if adapters.len() >= 16 {
                return Err(self.limit("for-loop adapter count exceeds limit 16"));
            }
            adapters.push(adapter);
            let Expr::MethodCall { receiver, .. } = value else {
                unreachable!()
            };
            value = *receiver;
        }
        adapters.reverse();
        Ok(ForIterable::Value(value))
    }
    #[inline(never)]
    fn while_let_statement(&mut self, label: Option<AstIdent>) -> Result<Stmt, ParseError> {
        let pattern = self.pattern()?;
        if self.is("|") || self.is("@") || self.is("if") {
            return Err(
                self.unsupported("while let guards, or-patterns and @ patterns are unsupported")
            );
        }
        self.expect("=")?;
        let scrutinee = self.condition_with(3, ConditionPurpose::Conditional)?;
        if self.is("&&") || self.is("||") {
            return Err(self.unsupported("while let chains are unsupported"));
        }
        let pattern = self
            .names
            .try_parser_box(pattern)
            .map_err(native_adapter::name_error)?;
        let scrutinee = self
            .names
            .try_parser_box(scrutinee)
            .map_err(native_adapter::name_error)?;
        let frontier = self.enter_lexical()?;
        let body_result = (|| {
            self.bind_pattern_lexical(&pattern)?;
            self.block()
        })();
        self.restore_frontier(frontier);
        let body = body_result?;
        let has_semicolon = self.eat(";");
        Ok(Stmt::WhileLet {
            label,
            pattern,
            scrutinee,
            body,
            has_semicolon,
        })
    }
    #[inline(never)]
    fn match_prefix(&mut self) -> Result<Expr, ParseError> {
        let scrutinee = Box::new(self.condition()?);
        self.expect("{")?;
        let mut arms = Vec::new();
        while !self.is("}") {
            if arms.len() >= 4096 {
                return Err(self.limit("match arm count exceeds limit 4096"));
            }
            let pattern = self.pattern()?;
            if self.is("if") || self.is("|") || self.is("@") {
                return Err(
                    self.unsupported("match guards, or-patterns and @ patterns are unsupported")
                );
            }
            self.expect("=>")?;
            let frontier = self.enter_lexical()?;
            self.bind_pattern_lexical(&pattern)?;
            let direct_return = self.eat("return");
            let source_blocklike =
                matches!(self.token().text, "if" | "match" | "loop" | "{" | "unsafe")
                    || self.token().is_lifetime();
            let brace_macro = self.is_identifier()
                && self.cursor.peek(1).is_some_and(|token| token.matches("!"))
                && self.cursor.peek(2).is_some_and(|token| token.matches("{"));
            let body = if direct_return {
                let value = if self.is(",") || self.is("}") {
                    None
                } else {
                    Some(self.nested_expr()?)
                };
                Expr::Block(Block {
                    statements: vec![Stmt::Return(value)],
                    tail: None,
                })
            } else {
                self.nested_expr()?
            };
            let blocklike = !direct_return
                && (source_blocklike || brace_macro)
                && matches!(
                    body,
                    Expr::Block(_)
                        | Expr::Unsafe(_)
                        | Expr::If { .. }
                        | Expr::IfLet { .. }
                        | Expr::Loop { .. }
                        | Expr::Match { .. }
                );
            arms.push(MatchArm {
                pattern,
                body,
                guarded: false,
            });
            self.restore_frontier(frontier);
            if !self.eat(",") && !self.is("}") && !blocklike {
                return Err(self.error("expected comma after match arm"));
            }
        }
        self.expect("}")?;
        Ok(Expr::Match { scrutinee, arms })
    }
    fn expr_continuation(
        &mut self,
        mut left: Expr,
        min: u8,
        statement_blocklike: bool,
    ) -> Result<Expr, ParseError> {
        let mut compared = false;
        loop {
            if statement_blocklike
                && matches!(
                    left,
                    Expr::If { .. }
                        | Expr::IfLet { .. }
                        | Expr::Match { .. }
                        | Expr::Loop { .. }
                        | Expr::LabeledBlock { .. }
                        | Expr::Block(_)
                        | Expr::Unsafe(_)
                )
                && self.is("[")
            {
                break;
            }
            if self.eat("[") {
                let literal = self.token().text.to_owned();
                let constant = (!self.token().string
                    && self.cursor.peek(1).is_some_and(|t| t.text == "]"))
                .then(|| parse_integer_literal(&literal).ok())
                .flatten()
                .filter(|literal| literal.suffix.is_none());
                if let Some(literal) = constant {
                    let value =
                        resolve_integer(&literal, false, Some(&ir::Ty::Int(ir::IntTy::Usize)))
                            .map_err(|e| self.error(&e))?;
                    if value.bits > ir::MAX_ARRAY_LEN as u128 {
                        return Err(self.limit("array index exceeds supported limit"));
                    }
                    let index = value.bits as usize;
                    self.cursor.bump();
                    self.expect("]")?;
                    left = Expr::Index {
                        base: Box::new(left),
                        index,
                    };
                } else {
                    if self.is("..") {
                        return Err(self.unsupported("unsupported: range indexing and subslicing"));
                    }
                    let index = self.nested_expr()?;
                    if self.is("..") || self.is("..=") {
                        return Err(self.unsupported("unsupported: range indexing and subslicing"));
                    }
                    self.expect("]")?;
                    left = Expr::DynamicIndex {
                        base: Box::new(left),
                        index: Box::new(index),
                    };
                }
                validate_depth(DepthNode::Expr(&left)).map_err(|e| self.depth_error(&e))?;
                continue;
            }
            if self.eat(".") {
                left = self.field_or_method_after_dot(left)?;
                validate_depth(DepthNode::Expr(&left)).map_err(|e| self.depth_error(&e))?;
                continue;
            }
            if statement_blocklike
                && matches!(
                    left,
                    Expr::If { .. }
                        | Expr::IfLet { .. }
                        | Expr::Match { .. }
                        | Expr::Loop { .. }
                        | Expr::LabeledBlock { .. }
                        | Expr::Block(_)
                        | Expr::Unsafe(_)
                )
            {
                break;
            }
            if self.is("as") {
                if min > 10 {
                    break;
                }
                left = self.cast_after_as(left)?;
                continue;
            }
            let (precedence, op) = match self.token().text {
                "||" => (1, BinaryOp::Or),
                "&&" => (2, BinaryOp::And),
                "==" => (3, BinaryOp::Eq),
                "!=" => (3, BinaryOp::Ne),
                "<" => (3, BinaryOp::Lt),
                "<=" => (3, BinaryOp::Le),
                ">" => (3, BinaryOp::Gt),
                ">=" => (3, BinaryOp::Ge),
                "|" => (4, BinaryOp::BitOr),
                "^" => (5, BinaryOp::BitXor),
                "&" => (6, BinaryOp::BitAnd),
                "<<" => (7, BinaryOp::Shl),
                ">>" => (7, BinaryOp::Shr),
                "+" => (8, BinaryOp::Add),
                "-" => (8, BinaryOp::Sub),
                "*" => (9, BinaryOp::Mul),
                "/" => (9, BinaryOp::Div),
                "%" => (9, BinaryOp::Rem),
                _ => break,
            };
            if self.token().string || precedence < min {
                break;
            }
            if self.condition_purpose == ConditionPurpose::Conditional
                && self.is("&&")
                && self
                    .cursor
                    .peek(1)
                    .is_some_and(|token| token.matches("let"))
            {
                return Err(self.unsupported("if let chains are unsupported"));
            }
            if matches!(
                op,
                BinaryOp::BitAnd
                    | BinaryOp::BitOr
                    | BinaryOp::BitXor
                    | BinaryOp::Shl
                    | BinaryOp::Shr
            ) && self
                .cursor
                .peek(1)
                .is_some_and(|token| !token.string && token.text == "=")
            {
                return Err(self.unsupported("compound bitwise/shift assignments are unsupported"));
            }
            if precedence == 3 {
                if compared {
                    return Err(self.error("chained comparisons require parentheses"));
                }
                compared = true;
            } else if precedence < 3 {
                compared = false;
            }
            self.cursor.bump();
            let right = self.expr(precedence + 1)?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
            validate_depth(DepthNode::Expr(&left)).map_err(|e| self.depth_error(&e))?;
        }
        Ok(left)
    }
}
fn literal_format_owned(text: &str, names: &mut NameStoreBuilder) -> Result<String, ParseError> {
    use super::expansion_budget::Counter;
    let budget = names.budget().clone();
    budget
        .charge(Counter::Work, text.len() as u64)
        .map_err(|error| native_adapter::transport_error(error.into()))?;
    let mut chars = text.chars();
    let mut planned = 0usize;
    while let Some(c) = chars.next() {
        if (c == '{' || c == '}') && chars.next() != Some(c) {
            return Err(native_adapter::source_error(
                super::source::SourceError::syntax(
                    None,
                    "println! formatting placeholders are unsupported",
                ),
            ));
        }
        planned += c.len_utf8();
    }
    let capacity = planned.checked_add(1).ok_or_else(|| {
        native_adapter::transport_error(super::token_session::TokenError::InvalidRange)
    })?;
    budget
        .charge(Counter::Work, capacity as u64)
        .map_err(|error| native_adapter::transport_error(error.into()))?;
    names
        .retain_parser_text(capacity)
        .map_err(native_adapter::name_error)?;
    let mut out = String::new();
    out.try_reserve_exact(capacity)
        .map_err(|_| native_adapter::transport_error(budget.poison_allocation().into()))?;
    if out.capacity() > capacity {
        names
            .retain_parser_text(out.capacity() - capacity)
            .map_err(native_adapter::name_error)?;
    }
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '{' || c == '}' {
            chars.next();
        }
        out.push(c);
    }
    Ok(out)
}

fn module_parse_error(error: super::module_identity::ModuleError) -> ParseError {
    use super::module_identity::ModuleError;
    match error {
        ModuleError::Token(error) => native_adapter::transport_error(error),
        ModuleError::Name(error) => native_adapter::name_error(error),
        other => ParseError {
            _diagnostic_lease: None,
            kind: if matches!(other, ModuleError::SourceUnavailable) {
                super::source::SourceErrorKind::Unsupported
            } else {
                super::source::SourceErrorKind::Contract
            },
            has_location: false,
            primary: None,
            bytes: 0..0,
            line: 1,
            column: 1,
            message: other.to_string(),
        },
    }
}
fn module_diagnostic(error: super::module_identity::ModuleError) -> String {
    let error = module_parse_error(error);
    let mut diagnostic = super::source::SourceError::syntax(error.primary, error.message);
    diagnostic.kind = error.kind;
    diagnostic._diagnostic_lease = error._diagnostic_lease;
    diagnostic.to_string()
}
struct ParsedSyntaxUnit {
    parsed: ParsedDeclarations,
    names: Arc<NameStore>,
    modules: Arc<super::module_identity::ModuleArena>,
    symbols: Arc<ProgramSymbols>,
    scopes: Option<Arc<super::semantic_scope::ScopeArena>>,
    _inactive_items: Option<super::token_session::BudgetVec<Arc<UnexpandedMacroSyntax>>>,
}
/// No conversion to Program or ParsedProgram is exposed for this certificate.
pub(super) struct InactiveSyntaxCertificate {
    _unit: ParsedSyntaxUnit,
}
impl std::fmt::Debug for InactiveSyntaxCertificate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InactiveSyntaxCertificate")
            .finish_non_exhaustive()
    }
}
/// Parse syntax only, retaining imports and visibility for the signatures phase.
#[cfg(test)]
fn parse_syntax_tokens(tokens: NativeParserInput) -> Result<ParsedProgram, ParseError> {
    parse_syntax_tokens_configured(tokens, super::source::CfgSet::DEFAULT_DEBUG_ASSERTIONS)
}
fn parse_syntax_tokens_configured(
    tokens: NativeParserInput,
    debug_assertions: bool,
) -> Result<ParsedProgram, ParseError> {
    let unit = parse_syntax_unit(tokens, ParserPurpose::Active, debug_assertions)?;
    Ok(ParsedProgram {
        program: unit
            .parsed
            .program
            .publish(unit.names, unit.modules, unit.symbols, unit.scopes),
        items: unit.parsed.items,
        empty_use_prefixes: unit.parsed.empty_use_prefixes,
    })
}
fn parse_syntax_unit(
    tokens: NativeParserInput,
    purpose: ParserPurpose,
    debug_assertions: bool,
) -> Result<ParsedSyntaxUnit, ParseError> {
    let (tokens, session) = tokens.into_parts();
    let mut names = NameStoreBuilder::new(session, super::token::Edition::E2024)
        .map_err(native_adapter::name_error)?;
    let symbols = ProgramSymbols::new(&mut names).map_err(native_adapter::name_error)?;
    let mut parser = match purpose {
        ParserPurpose::Active => Parser::new(&tokens, &mut names)?,
        ParserPurpose::InactiveSyntax => Parser::with_purpose(&tokens, &mut names, purpose)?,
    };
    parser.debug_assertions = debug_assertions;
    let mut parsed = ParsedDeclarations {
        program: ProgramDeclarations::default(),
        items: HashMap::new(),
        empty_use_prefixes: Vec::new(),
    };
    let root = parser.modules.source_root().map_err(module_parse_error)?;
    let root_path =
        AstPath::new(parser.names, PathAnchor::Crate, &[]).map_err(native_adapter::name_error)?;
    parser.items(root, &root_path, &mut parsed)?;
    if !parser.is("") {
        return Err(parser.error("unexpected module closing brace"));
    }
    for function in &parsed.program.functions {
        let function_name = function
            .name
            .display(parser.names)
            .map_err(native_adapter::name_error)?;
        validate_depth(DepthNode::Block(&function.body)).map_err(|e| ParseError {
            _diagnostic_lease: None,
            kind: super::source::SourceErrorKind::Limit,
            has_location: false,
            primary: None,
            bytes: 0..0,
            line: 1,
            column: 1,
            message: format!("in function {function_name}: {e}"),
        })?;
    }
    for declaration in &parsed.program.traits {
        for item in &declaration.items {
            if let Some(default) = &item.default {
                let declaration_name = declaration
                    .name
                    .display(parser.names)
                    .map_err(native_adapter::name_error)?;
                let default_name = default
                    .name
                    .display(parser.names)
                    .map_err(native_adapter::name_error)?;
                validate_depth(DepthNode::Block(&default.body)).map_err(|e| ParseError {
                    _diagnostic_lease: None,
                    kind: super::source::SourceErrorKind::Limit,
                    has_location: false,
                    primary: None,
                    bytes: 0..0,
                    line: 1,
                    column: 1,
                    message: format!(
                        "in trait default {}::{}: {e}",
                        declaration_name, default_name
                    ),
                })?;
            }
        }
    }
    let lexical = parser.lexical.into_owned()?;
    let scopes = lexical
        .scopes
        .map(|scopes| scopes.freeze())
        .transpose()
        .map_err(scope_parse_error)?;
    let modules = parser
        .modules
        .into_owned()?
        .freeze()
        .map_err(module_parse_error)?;
    let names = names.freeze().map_err(native_adapter::name_error)?;
    Ok(ParsedSyntaxUnit {
        parsed,
        names,
        modules,
        symbols,
        scopes,
        _inactive_items: lexical.inactive_items,
    })
}
/// Syntax-only preparation seam. Does not process attributes, resolve names, or
/// access files. The source preparation owner supplies attribute-masked text.
#[cfg(test)]
pub(super) fn parse_syntax_unprepared(source: &str) -> Result<ParsedProgram, ParseError> {
    parse_syntax_tokens(lex(source)?)
}
pub(super) fn validate_inactive_syntax(
    source: &str,
) -> Result<InactiveSyntaxCertificate, ParseError> {
    let budget = super::expansion_budget::ExpansionBudget::default();
    let projection = super::source::PreparedLexInput::identity(
        std::path::Path::new("<inactive>"),
        source,
        &budget,
    )
    .map_err(native_adapter::source_error)?;
    let mut session =
        super::token_session::TokenSession::new(projection.snapshot().clone(), budget)
            .map_err(native_adapter::transport_error)?;
    let lexed = super::lexer::lex_prepared(&projection, super::token::Edition::E2024, &mut session)
        .map_err(native_adapter::lex_error)?;
    let input = NativeParserInput::new_inactive(lexed, session, &projection)?;
    Ok(InactiveSyntaxCertificate {
        _unit: parse_syntax_unit(
            input,
            ParserPurpose::InactiveSyntax,
            super::source::CfgSet::DEFAULT_DEBUG_ASSERTIONS,
        )?,
    })
}

#[cfg(test)]
mod inactive_item_syntax_tests {
    use super::*;

    #[test]
    fn keyword_prefix_negation_is_not_an_identifier_macro_invocation() {
        for source in [
            "fn main(){if !false{println!(\"ok\");}}",
            "fn main(){let x:bool=match !false{true=>true,false=>false};if x{println!(\"ok\");}}",
            "fn main(){let x:bool=loop{break !false;};if x{println!(\"ok\");}}",
            "fn main(){macro_rules! yes{()=>{true}}if !yes!(){println!(\"bad\");}}",
        ] {
            let program = parse(source).unwrap();
            check(&program).unwrap();
        }
    }

    #[test]
    fn active_unknown_item_macro_is_syntax_at_invocation_without_inventory() {
        for source in [
            "undefined!();fn main(){}",
            "#[cfg(any())] macro_rules! undefined {()=>{fn skipped(){}}} undefined!();fn main(){}",
        ] {
            let prepared = super::super::source::prepare_text(
                "unknown.rs",
                source,
                &super::super::source::SourceOptions::default(),
            )
            .unwrap();
            let error = parse_prepared(&prepared).unwrap_err();
            assert_eq!(
                error.kind,
                super::super::source::SourceErrorKind::Syntax,
                "{error}"
            );
            assert!(error.message.contains("unknown module macro"), "{error}");
            assert_eq!(
                error.primary.unwrap().lo as usize,
                source.rfind("undefined!").unwrap()
            );
        }
    }

    #[test]
    fn unit_struct_syntax_retains_nominal_shape_and_alias_constructor_rejection() {
        let parsed = parse_unresolved("struct S;fn main(){}").unwrap();
        assert_eq!(parsed.program.structs[0].shape, StructShape::Unit);
        assert!(parsed.program.structs[0].fields.is_empty());
        let program = resolve(&parsed).unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let key = name_scope::path_key(
            program.names.as_ref(),
            &program.structs[0].name,
            Namespace::Type,
        )
        .unwrap();
        let declaration = plan.registry.nominals[&key];
        let (_, ty) = plan.nominal_schema(declaration, &budget).unwrap();
        assert!(matches!(ty, ir::Ty::Struct(_)), "{ty:?}");
        assert_eq!(nominal_parts(&ty).unwrap().0, declaration);
        assert_eq!(budget.usage_snapshot().applications, 1);
        assert_eq!(budget.usage_snapshot().obligations, 0);
        let error = parse("struct S;type A=S;fn main(){let x=A;}").unwrap_err();
        assert!(!error.contains("unsupported"), "{error}");
        assert!(
            error.contains("type alias is not a value constructor"),
            "{error}"
        );
        assert!(
            parse_unresolved("struct S{}fn main(){}")
                .unwrap_err()
                .contains("empty structs are unsupported")
        );
    }

    #[test]
    fn unit_struct_values_patterns_and_calls_keep_exact_nominal_types() {
        for source in [
            "struct U;fn make()->U{U}fn pass(x:U)->U{x}fn main(){let U=pass(make());println!(\"ok\");}",
            "struct U;fn main(){match U{U=>println!(\"ok\")}}",
            "struct U;type A=U;fn main(){let U{}=A{};println!(\"ok\");}",
            "mod m{pub struct U;}use m::U as V;fn main(){let m::U=V;println!(\"ok\");}",
            "struct U;fn main(){macro_rules! value{()=>{42};}let crate::U=U;let n=value!();println!(\"ok\");}",
        ] {
            let program = parse(source).unwrap_or_else(|error| panic!("{source}: {error}"));
            check(&program).unwrap_or_else(|error| panic!("{source}: {error}"));
        }
    }

    #[test]
    fn unit_structs_do_not_become_copy_or_interchangeable_empty_values() {
        for source in [
            "struct U;fn take(x:U){}fn main(){let u=U;take(u);take(u);}",
            "struct U;struct V;fn take(x:U){}fn main(){take(V);}",
        ] {
            let program = parse(source).unwrap();
            assert!(check(&program).is_err(), "{source}");
        }
    }

    #[test]
    fn unit_struct_bare_scoped_patterns_reject_before_ghost_binding_publication() {
        for source in [
            "struct U;fn main(){macro_rules! value{()=>{42};}let a=U;let U=a;println!(\"ok\");}",
            "struct U;fn main(){macro_rules! value{()=>{42};}match U{U=>println!(\"ok\")}}",
        ] {
            let error = parse(source).unwrap_err();
            assert!(error.contains("unsupported"), "{source}: {error}");
            assert!(!error.contains("contract violation"), "{source}: {error}");
        }
    }

    #[test]
    fn generic_unit_declarations_remain_prepublication_unsupported() {
        let program = parse("struct U<T>;fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let error = PlanCtx::from_program(&program, &budget).unwrap_err();
        assert!(
            error.contains("unsupported: generic unit structs"),
            "{error}"
        );
        assert_eq!(budget.usage_snapshot().applications, 0);
        assert_eq!(budget.usage_snapshot().obligations, 0);
    }

    #[test]
    fn tuple_struct_parser_retains_indexed_fields_and_visibility() {
        let parsed = parse_unresolved("pub struct Pair(pub i32,bool,);fn main(){}").unwrap();
        let declaration = &parsed.program.structs[0];
        assert_eq!(declaration.shape, StructShape::Tuple);
        assert_eq!(declaration.fields.len(), 2);
        for (index, field) in declaration.fields.iter().enumerate() {
            assert!(matches!(field.name, FieldName::Tuple(actual) if actual == index));
        }
        assert!(matches!(
            declaration.fields[0].visibility,
            ir::Visibility::Public
        ));
        assert!(matches!(declaration.fields[1].visibility,
            ir::Visibility::Restricted(module) if module == declaration.module));
    }

    #[test]
    fn generic_tuple_struct_parser_retains_actual_type_parameter_identity() {
        let program = parse("struct Generic<T>(T);fn main(){}").unwrap();
        let declaration = &program.structs[0];
        assert_eq!(declaration.shape, StructShape::Tuple);
        assert_eq!(declaration.type_params.len(), 1);
        assert_eq!(declaration.fields.len(), 1);
        assert!(matches!(declaration.fields[0].name, FieldName::Tuple(0)));
        let Type::Param(parameter) = declaration.fields[0].ty else {
            panic!("generic tuple field lost its actual source parameter");
        };
        assert_eq!(
            program
                .names
                .key(parameter, LookupClass::TypeParameter)
                .unwrap(),
            program
                .names
                .key(declaration.type_params[0], LookupClass::TypeParameter)
                .unwrap()
        );
        check(&program).unwrap();
    }

    #[test]
    fn tuple_struct_parser_preserves_explicit_unadmitted_syntax_boundaries() {
        for source in [
            "struct Empty();fn main(){}",
            "struct Pair(i32);fn main(){let p=Pair{0:42};}",
            "struct Pair(i32);fn main(){let Pair{0:x}=Pair(42);}",
        ] {
            let error = parse_unresolved(source).unwrap_err();
            assert!(error.contains("unsupported"), "{source}: {error}");
            assert!(!error.contains("contract violation"), "{source}: {error}");
        }
        let source = "struct Pair(i32) fn main(){}";
        let error = parse_syntax_unprepared(source).unwrap_err();
        assert_eq!(error.kind, super::super::source::SourceErrorKind::Syntax);
        assert_eq!(
            error.bytes,
            source.find("fn").unwrap()..source.find("fn").unwrap() + 2
        );
        assert_eq!(&source[error.bytes.clone()], "fn");
        let primary = error.primary.unwrap();
        assert_eq!(primary.lo as usize, error.bytes.start);
        assert_eq!(primary.hi as usize, error.bytes.end);
    }

    #[test]
    fn aggregate_ref_binding_parses_as_a_borrowing_binding() {
        let source = "struct Pair(i32,i32);fn main(){match Pair(40,2){Pair(ref a,b)=>*a+b};}";
        parse_syntax_unprepared(source).unwrap();
    }

    #[test]
    fn tuple_struct_inventory_retains_child_calls_but_not_constructor_calls() {
        let program =
            parse("struct Pair(i32,i32);fn value()->i32{42}fn main(){let p=Pair(value(),1);}")
                .unwrap();
        let budget = ir::PlanBudget::default();
        let identity = sr::OriginalBodyId::new(&budget).unwrap();
        let inventory = SourceInventory::build(
            &program,
            program.functions.last().unwrap(),
            identity,
            &budget,
        )
        .unwrap();
        assert_eq!(inventory.calls.len(), 1);
        let actual = inventory.calls[0].ordinary.as_ref().unwrap();
        let expected = &program.functions[0].name;
        assert_eq!(
            name_scope::path_key(program.names.as_ref(), actual, Namespace::Value).unwrap(),
            name_scope::path_key(program.names.as_ref(), expected, Namespace::Value).unwrap()
        );
    }

    #[test]
    fn positional_rest_projects_only_written_children_without_cloning_them() {
        for source in [
            "fn main(){let (a,..,z)=(1,2,3);}",
            "struct P(i32,i32,i32);fn main(){let P(a,..,z)=P(1,2,3);}",
        ] {
            let parsed = parse_unresolved(source).unwrap();
            let Stmt::LetPattern { pattern, .. } = &parsed.program.functions[0].body.statements[0]
            else {
                panic!("expected real pattern");
            };
            let fields = match pattern {
                Pattern::TupleRest(fields) | Pattern::VariantRest { fields, .. } => fields,
                _ => panic!("expected explicit positional rest"),
            };
            assert_eq!(fields.prefix.len(), 1);
            assert_eq!(fields.suffix.len(), 1);
            let projection = fields.project(3).unwrap();
            let mut projected = projection.iter();
            let (index, first) = projected.next().unwrap();
            assert_eq!(index, 0);
            assert!(std::ptr::eq(first, &fields.prefix[0]));
            let (index, last) = projected.next().unwrap();
            assert_eq!(index, 2);
            assert!(std::ptr::eq(last, &fields.suffix[0]));
            assert!(projected.next().is_none());
            assert!(fields.project(1).is_err());
            assert!(fields.project(0).is_err());
            assert_eq!(
                fields
                    .project(2)
                    .unwrap()
                    .iter()
                    .map(|(i, _)| i)
                    .collect::<Vec<_>>(),
                [0, 1]
            );
        }
    }

    #[test]
    fn duplicate_positional_rest_is_syntax_at_the_second_written_rest() {
        for source in [
            "fn main(){let (..,..)=(1,2);}",
            "struct P(i32,i32);fn main(){let P(a,..,..)=P(1,2);}",
        ] {
            let error = parse_syntax_unprepared(source).unwrap_err();
            assert_eq!(error.kind, super::super::source::SourceErrorKind::Syntax);
            let at = source.rfind("..").unwrap();
            assert_eq!(error.bytes, at..at + 2);
            assert!(
                error
                    .message
                    .starts_with("duplicate positional rest pattern")
            );
        }
    }

    #[test]
    fn positional_rest_scope_inventory_contains_no_omitted_bindings() {
        let program = parse("struct P(i32,i32,i32);fn main(){macro_rules! v{()=>{42}}let P(a,..,z)=P(40,7,2);let n=v!();}").unwrap();
        let function = &program.functions[0];
        assert!(function.lexical_scope.is_some());
        let budget = ir::PlanBudget::default();
        let identity = sr::OriginalBodyId::new(&budget).unwrap();
        let inventory = SourceInventory::build(&program, function, identity, &budget).unwrap();
        assert_eq!(inventory.bindings.len(), 3);
        assert!(inventory.calls.is_empty());
        assert_ne!(inventory.bindings[0].node, inventory.bindings[1].node);
    }

    #[test]
    fn array_pattern_syntax_retains_only_written_children_and_real_empty_shape() {
        let parsed = parse_unresolved(
            "fn main(){let [a,b]=[1,2];let [x,..,z]=[1,2,3];let []=[];let (p,..,q)=(1,2,3);}",
        )
        .unwrap();
        let statements = &parsed.program.functions[0].body.statements;
        let Stmt::LetPattern {
            pattern: Pattern::Array(fields),
            ..
        } = &statements[0]
        else {
            panic!("expected written array pattern");
        };
        assert_eq!(fields.len(), 2);
        let Stmt::LetPattern {
            pattern: Pattern::ArrayRest(fields),
            ..
        } = &statements[1]
        else {
            panic!("expected explicit array rest");
        };
        assert_eq!(fields.prefix.len(), 1);
        assert_eq!(fields.suffix.len(), 1);
        assert_eq!(
            fields
                .project(3)
                .unwrap()
                .iter()
                .map(|(i, _)| i)
                .collect::<Vec<_>>(),
            [0, 2]
        );
        assert!(fields.project(1).is_err());
        assert!(
            matches!(&statements[2], Stmt::LetPattern { pattern: Pattern::Array(fields), .. } if fields.is_empty())
        );
        assert!(matches!(
            &statements[3],
            Stmt::LetPattern {
                pattern: Pattern::TupleRest(_),
                ..
            }
        ));
    }

    #[test]
    fn array_pattern_duplicate_rest_and_parameter_boundaries_keep_exact_spans() {
        for source in [
            "fn main(){let [..,..]=[1,2];}",
            "fn main(){let [a,..,..]=[1,2];}",
        ] {
            let error = parse_syntax_unprepared(source).unwrap_err();
            assert_eq!(error.kind, super::super::source::SourceErrorKind::Syntax);
            let at = source.rfind("..").unwrap();
            assert_eq!(error.bytes, at..at + 2);
            assert!(
                error
                    .message
                    .starts_with("duplicate positional rest pattern")
            );
        }
        let source = "fn f([a,b]:[i32;2]){}fn main(){}";
        let error = parse_syntax_unprepared(source).unwrap_err();
        assert_eq!(
            error.kind,
            super::super::source::SourceErrorKind::Unsupported
        );
        let at = source.find('[').unwrap();
        assert_eq!(error.bytes, at..at + 1);
        assert!(
            error
                .message
                .starts_with("array parameter patterns are unsupported")
        );
    }

    #[test]
    fn array_rest_scope_inventory_contains_only_actual_bindings() {
        let program =
            parse("fn main(){macro_rules! v{()=>{42}}let [a,..,z]=[40,7,2];let n=v!();}").unwrap();
        let function = &program.functions[0];
        assert!(function.lexical_scope.is_some());
        let budget = ir::PlanBudget::default();
        let identity = sr::OriginalBodyId::new(&budget).unwrap();
        let inventory = SourceInventory::build(&program, function, identity, &budget).unwrap();
        assert_eq!(inventory.bindings.len(), 3);
        assert!(inventory.calls.is_empty());
        assert_ne!(inventory.bindings[0].node, inventory.bindings[1].node);
    }

    #[test]
    fn if_let_keeps_absent_else_and_real_else_expression_shapes() {
        let parsed = parse_unresolved("fn main(){if let true=true {} if let true=false {} else if let false=false {} else {}}")
            .unwrap();
        let body = &parsed.program.functions[0].body;
        assert!(matches!(
            &body.statements[0],
            Stmt::ImplicitExpr(Expr::IfLet {
                else_branch: None,
                ..
            })
        ));
        let Expr::IfLet {
            else_branch: Some(branch),
            ..
        } = body.tail.as_deref().unwrap()
        else {
            panic!("expected actual if-let tail");
        };
        let Expr::IfLet {
            else_branch: Some(branch),
            ..
        } = branch.as_ref()
        else {
            panic!("expected actual else-if-let, not a wrapping block");
        };
        assert!(matches!(branch.as_ref(), Expr::Block(_)));
    }

    #[test]
    fn if_let_chain_boundaries_are_typed_at_written_tokens() {
        for (source, token) in [
            ("fn main(){if let true=true && true {}}", "&&"),
            ("fn main(){if true && let true=true {}}", "&&"),
            ("fn main(){if let true=true || false {}}", "||"),
        ] {
            let error = parse_syntax_unprepared(source).unwrap_err();
            assert_eq!(
                error.kind,
                super::super::source::SourceErrorKind::Unsupported
            );
            let at = source.find(token).unwrap();
            assert_eq!(error.bytes, at..at + token.len());
        }
        parse_unresolved("fn main(){if let true=(true && false) {} else {} if true && false {}}")
            .unwrap();
    }

    #[test]
    fn while_let_preserves_real_statement_label_and_promoted_source() {
        // Exact source promoted from the former explicit unsupported boundary.
        let parsed = parse_unresolved("fn main(){while let true=true {}}").unwrap();
        assert!(matches!(&parsed.program.functions[0].body.statements[0],
            Stmt::WhileLet { label: None, pattern, .. } if matches!(pattern.as_ref(), Pattern::Bool(true))));
        let parsed =
            parse_unresolved("fn main(){'outer: while let true=false {continue 'outer;}}").unwrap();
        let Stmt::WhileLet {
            label: Some(label),
            body,
            ..
        } = &parsed.program.functions[0].body.statements[0]
        else {
            panic!("real labeled while-let")
        };
        let Stmt::Expr(Expr::Continue {
            label: Some(target),
        }) = &body.statements[0]
        else {
            panic!("real continue")
        };
        assert_eq!(
            parsed
                .program
                .names
                .key(*label, LookupClass::Label)
                .unwrap(),
            parsed
                .program
                .names
                .key(*target, LookupClass::Label)
                .unwrap()
        );
    }

    #[test]
    fn while_let_chains_and_expression_positions_remain_explicit() {
        for (source, token) in [
            ("fn main(){while let true=true && true {}}", "&&"),
            ("fn main(){while true && let true=true {}}", "&&"),
            ("fn main(){while let true=true || false {}}", "||"),
            ("fn main(){let value=while let true=false {};}", "while"),
            (
                "fn main(){let value='outer: while let true=false {};}",
                "while",
            ),
        ] {
            let error = parse_syntax_unprepared(source).unwrap_err();
            assert_eq!(
                error.kind,
                super::super::source::SourceErrorKind::Unsupported
            );
            let at = source.find(token).unwrap();
            assert_eq!(error.bytes, at..at + token.len());
        }
        parse_unresolved("fn main(){while let true=(true && false) {break;}}").unwrap();
    }

    #[test]
    fn while_let_scope_inventory_keeps_outer_scrutinee_and_one_real_callsite() {
        let program = parse("fn probe()->bool{false}fn main(){macro_rules! marker{()=>{42}}let x=40;while let x=x{let y=x;break;}let z=x;while let true=probe(){break;}}").unwrap();
        let function = &program.functions[1];
        let scopes = program.scopes.as_ref().unwrap();
        let Stmt::Let {
            name: BindingName::Named(outer),
            ..
        } = &function.body.statements[0]
        else {
            panic!("outer")
        };
        let Stmt::WhileLet {
            pattern,
            scrutinee,
            body,
            ..
        } = &function.body.statements[1]
        else {
            panic!("while-let")
        };
        let Pattern::Binding { name: inner, .. } = pattern.as_ref() else {
            panic!("pattern")
        };
        let outer_id = scopes
            .declaration_for(program.names.as_ref(), *outer)
            .unwrap();
        let inner_id = scopes
            .declaration_for(program.names.as_ref(), *inner)
            .unwrap();
        assert_ne!(outer_id, inner_id);
        let reference = |value: &Expr| {
            let Expr::Var(path) = value else {
                panic!("reference")
            };
            scopes
                .reference_for(program.names.as_ref(), path.single().unwrap())
                .unwrap()
        };
        assert_eq!(reference(scrutinee), Some(outer_id));
        let Stmt::Let { value, .. } = &body.statements[0] else {
            panic!("body binding")
        };
        assert_eq!(reference(value), Some(inner_id));
        let Stmt::Let { value, .. } = &function.body.statements[2] else {
            panic!("after binding")
        };
        assert_eq!(reference(value), Some(outer_id));
        let budget = ir::PlanBudget::default();
        let identity = sr::OriginalBodyId::new(&budget).unwrap();
        let inventory = SourceInventory::build(&program, function, identity, &budget).unwrap();
        assert_eq!(inventory.bindings.len(), 4);
        assert_eq!(inventory.calls.len(), 1);
        assert!(inventory.calls[0].ordinary.is_some());
        assert_ne!(inventory.bindings[0].node, inventory.bindings[1].node);
    }

    #[test]
    fn let_else_keeps_actual_annotation_and_mandatory_semicolon() {
        let parsed = parse_unresolved("fn main(){let x:i32=42 else{return;};let y=x;}").unwrap();
        let Stmt::LetElse {
            pattern,
            ty,
            value,
            else_block,
        } = &parsed.program.functions[0].body.statements[0]
        else {
            panic!("genuine let-else")
        };
        assert!(matches!(pattern.as_ref(), Pattern::Binding { .. }));
        assert!(matches!(ty, Type::Int(ir::IntTy::I32)));
        assert!(matches!(value.as_ref(), Expr::Integer(_)));
        assert!(matches!(&else_block.statements[0], Stmt::Return(None)));
        let source = "fn main(){let x=42 else{return;}}";
        let error = parse_syntax_unprepared(source).unwrap_err();
        assert_eq!(error.kind, super::super::source::SourceErrorKind::Syntax);
        let at = source.rfind('}').unwrap();
        assert_eq!(error.bytes, at..at + 1);
        assert!(parse_syntax_unprepared("fn main(){let true=true;}").is_err());
    }

    #[test]
    fn let_else_initializer_grammar_uses_actual_tokens_not_expanded_ast() {
        for (source, token) in [
            ("fn main(){let _={1}else{return;};}", "}else"),
            ("fn main(){let true=true&&false else{return;};}", "&&"),
            ("fn main(){let true=true||false else{return;};}", "||"),
            (
                "fn main(){macro_rules! v{()=>{true}}let true=v!{}else{return;};}",
                "}else",
            ),
        ] {
            let error = parse_syntax_unprepared(source).unwrap_err();
            assert_eq!(error.kind, super::super::source::SourceErrorKind::Syntax);
            let at = source.find(token).unwrap();
            let len = if token == "}else" { 1 } else { token.len() };
            assert_eq!(error.bytes, at..at + len);
        }
        for source in [
            "fn main(){let _=({1})else{return;};}",
            "fn main(){let true=(true&&false)else{return;};}",
            "fn main(){let _=println!(\"}} && ||\")else{return;};}",
            "fn main(){macro_rules! v{()=>{true}}let true=(v!{})else{return;};}",
            "fn main(){macro_rules! v{()=>{true&&false}}let true=v!()else{return;};}",
        ] {
            parse_unresolved(source).unwrap_or_else(|error| panic!("{source}: {error}"));
        }
        let source = "enum E{A(i32),B(i32)}fn main(){let (E::A(x)|E::B(x))=E::A(42)else{return;};}";
        let error = parse_syntax_unprepared(source).unwrap_err();
        assert_eq!(
            error.kind,
            super::super::source::SourceErrorKind::Unsupported
        );
        let at = source.find('|').unwrap();
        assert_eq!(error.bytes, at..at + 1);
    }

    #[test]
    fn let_else_binding_escapes_only_after_old_frontier_else_and_initializer() {
        let program = parse("fn probe()->bool{false}fn main(){macro_rules! marker{()=>{42}}let x=40;let x=x else{let y=x;return;};let z=x;let true=probe()else{return;};}").unwrap();
        let function = &program.functions[1];
        assert!(function.lexical_scope.is_some());
        let scopes = program.scopes.as_ref().unwrap();
        let Stmt::Let {
            name: BindingName::Named(outer),
            ..
        } = &function.body.statements[0]
        else {
            panic!("outer")
        };
        let Stmt::LetElse {
            pattern,
            value,
            else_block,
            ..
        } = &function.body.statements[1]
        else {
            panic!("let-else")
        };
        let Pattern::Binding { name: inner, .. } = pattern.as_ref() else {
            panic!("pattern")
        };
        let outer_id = scopes
            .declaration_for(program.names.as_ref(), *outer)
            .unwrap();
        let inner_id = scopes
            .declaration_for(program.names.as_ref(), *inner)
            .unwrap();
        assert_ne!(outer_id, inner_id);
        let reference = |value: &Expr| {
            let Expr::Var(path) = value else {
                panic!("reference")
            };
            scopes
                .reference_for(program.names.as_ref(), path.single().unwrap())
                .unwrap()
        };
        assert_eq!(reference(value), Some(outer_id));
        let Stmt::Let { value, .. } = &else_block.statements[0] else {
            panic!("else binding")
        };
        assert_eq!(reference(value), Some(outer_id));
        let Stmt::Let { value, .. } = &function.body.statements[2] else {
            panic!("after binding")
        };
        assert_eq!(reference(value), Some(inner_id));
        let budget = ir::PlanBudget::default();
        let identity = sr::OriginalBodyId::new(&budget).unwrap();
        let inventory = SourceInventory::build(&program, function, identity, &budget).unwrap();
        assert_eq!(inventory.bindings.len(), 4);
        assert_eq!(inventory.calls.len(), 1);
        assert!(inventory.calls[0].ordinary.is_some());
        assert_ne!(inventory.bindings[0].node, inventory.bindings[2].node);
    }

    #[test]
    fn if_let_scrutinee_and_else_keep_outer_binding_and_inventory_identity() {
        let program = parse("fn main(){macro_rules! marker{()=>{42}}let x=40;if let x=x {let y=x;} else {let z=x;}let w=x;}").unwrap();
        let function = &program.functions[0];
        let scope = function.lexical_scope.unwrap();
        let scopes = program.scopes.as_ref().unwrap();
        let Stmt::Let {
            name: BindingName::Named(outer),
            ..
        } = &function.body.statements[0]
        else {
            panic!("outer binding")
        };
        let Stmt::ImplicitExpr(Expr::IfLet {
            pattern,
            scrutinee,
            then_block,
            else_branch,
        }) = &function.body.statements[1]
        else {
            panic!("if-let statement")
        };
        let Pattern::Binding { name: inner, .. } = pattern.as_ref() else {
            panic!("pattern binding")
        };
        let outer_id = scopes
            .declaration_for(program.names.as_ref(), *outer)
            .unwrap();
        let inner_id = scopes
            .declaration_for(program.names.as_ref(), *inner)
            .unwrap();
        assert_ne!(outer_id, inner_id);
        assert_eq!(scopes.binding_function(inner_id).unwrap(), scope);
        let reference = |value: &Expr| {
            let Expr::Var(path) = value else {
                panic!("reference")
            };
            scopes
                .reference_for(program.names.as_ref(), path.single().unwrap())
                .unwrap()
        };
        assert_eq!(reference(scrutinee), Some(outer_id));
        let Stmt::Let { value, .. } = &then_block.statements[0] else {
            panic!("then binding")
        };
        assert_eq!(reference(value), Some(inner_id));
        let Expr::Block(block) = else_branch.as_deref().unwrap() else {
            panic!("else block")
        };
        let Stmt::Let { value, .. } = &block.statements[0] else {
            panic!("else binding")
        };
        assert_eq!(reference(value), Some(outer_id));
        let Stmt::Let { value, .. } = &function.body.statements[2] else {
            panic!("after binding")
        };
        assert_eq!(reference(value), Some(outer_id));
        let budget = ir::PlanBudget::default();
        let identity = sr::OriginalBodyId::new(&budget).unwrap();
        let inventory = SourceInventory::build(&program, function, identity, &budget).unwrap();
        assert_eq!(inventory.bindings.len(), 5);
        assert!(inventory.calls.is_empty());
        assert_ne!(inventory.bindings[0].node, inventory.bindings[1].node);
    }

    #[test]
    fn item_publication_denies_before_container_allocation_and_releases() {
        use super::super::expansion_budget::{BudgetResource, Storage};
        let (tokens, session) = lex("fn main(){}").unwrap().into_parts();
        let mut names =
            NameStoreBuilder::new(session, super::super::token::Edition::E2024).unwrap();
        let budget = names.budget().clone();
        let mut parser = Parser::new(&tokens, &mut names).unwrap();
        let mut parsed = ParsedDeclarations {
            program: ProgramDeclarations::default(),
            items: HashMap::new(),
            empty_use_prefixes: Vec::new(),
        };
        let snapshot = budget.snapshot();
        let remaining = budget.limits().bytes - snapshot.retained_bytes - snapshot.temporary_bytes;
        let blocker = budget.reserve(Storage::Temporary, remaining).unwrap();
        assert_eq!(
            parser
                .reserve_item_publication(&mut parsed)
                .unwrap_err()
                .kind,
            super::super::source::SourceErrorKind::Limit
        );
        assert_eq!(parsed.program.functions.capacity(), 0);
        assert_eq!(parsed.items.capacity(), 0);
        assert_eq!(
            budget.snapshot().failure.unwrap().resource,
            BudgetResource::Bytes
        );
        drop(blocker);
        drop(parser);
        drop(names);
        drop(tokens);
        assert_eq!(budget.snapshot().retained_bytes, 0);
        assert_eq!(budget.snapshot().temporary_bytes, 0);
    }

    #[test]
    fn inactive_unknown_macro_retains_nonplannable_syntax_without_expansion() {
        let certificate =
            validate_inactive_syntax("undefined!(); fn unused(){ undefined!(); } fn main(){}")
                .unwrap();
        let items = certificate._unit._inactive_items.as_ref().unwrap();
        assert_eq!(items.len(), 1);
        assert!(certificate._unit.scopes.is_none());
        assert!(matches!(
            certificate._unit.parsed.program.functions[0]
                .body
                .statements[0],
            Stmt::Expr(Expr::UnexpandedMacro(_))
        ));
        let unit = certificate._unit;
        let program =
            unit.parsed
                .program
                .publish(unit.names, unit.modules, unit.symbols, unit.scopes);
        assert!(
            PlanCtx::from_program(&program, &ir::PlanBudget::default())
                .unwrap_err()
                .contains("unexpanded macro")
        );
    }

    #[test]
    fn inactive_macro_syntax_does_not_hide_invalid_statements_or_delimiters() {
        for source in [
            "fn unused(){let =;}",
            "undefined!([);",
            "macro_rules! bad { ($x:ident) => }",
        ] {
            assert_eq!(
                validate_inactive_syntax(source).unwrap_err().kind,
                super::super::source::SourceErrorKind::Syntax
            );
        }
    }
}

/// Parse mapped preparation output without reprocessing attributes or configuration.
pub fn parse_prepared(
    source: &super::source::PreparedSource,
) -> Result<ParsedProgram, super::source::SourceError> {
    let mapped = |error: ParseError| {
        let mut diagnostic = super::source::SourceError::syntax(
            error.primary.or_else(|| {
                if error.has_location {
                    source.map_range(error.bytes)
                } else {
                    None
                }
            }),
            error.message,
        );
        diagnostic.kind = error.kind;
        diagnostic._diagnostic_lease = error._diagnostic_lease;
        diagnostic.with_sources(&source.sources)
    };
    let budget = super::expansion_budget::ExpansionBudget::default();
    let projection = super::source::PreparedLexInput::prepared(source, &budget)?;
    let tokens = native_input(&projection, budget).map_err(mapped)?;
    parse_syntax_tokens_configured(tokens, source.debug_assertions()).map_err(mapped)
}

/// Parse syntax with the same preparation policy as file inputs, retaining the
/// legacy line/column String diagnostic format for existing direct callers.
pub fn parse_unresolved(source: &str) -> Result<ParsedProgram, String> {
    let prepared =
        super::source::prepare_text("<text>", source, &super::source::SourceOptions::default())
            .map_err(|e| e.to_string())?;
    let tokens = lex(&prepared.text).map_err(|e| e.to_string())?;
    parse_syntax_tokens_configured(tokens, prepared.debug_assertions()).map_err(|e| e.to_string())
}

/// Convenience frontend for callers without separate parse/signatures phases.
/// Performs name resolution, but not function body type checks.
pub fn parse(source: &str) -> Result<Program, String> {
    resolve(&parse_unresolved(source)?)
}

mod root_resolution;
pub use root_resolution::resolve;
#[cfg(test)]
mod generic_trait_source_tests {
    use super::*;

    fn parameter(program: &Program, ty: &Type, expected: AstIdent) {
        let Type::Param(actual) = ty else {
            panic!("trait actual did not retain its lexical type parameter: {ty:?}");
        };
        assert_eq!(
            program
                .names
                .key(*actual, LookupClass::TypeParameter)
                .unwrap(),
            program
                .names
                .key(expected, LookupClass::TypeParameter)
                .unwrap()
        );
    }
    fn actuals(path: &TraitPath) -> &[Type] {
        let TraitPath::Source(path) = path else {
            panic!("source trait became a builtin")
        };
        &path.segments.last().unwrap().args
    }

    #[test]
    fn associated_equality_rows_keep_source_handles_and_positional_arity() {
        let program = parse("trait Source<A>{type Out;} fn f<T:Source<i32,Out=(i32,bool),Out=(i32,bool)>>(){ } fn main(){}").unwrap();
        let TraitPath::Source(path) = &program.functions[0].bounds[0].traits[0] else {
            panic!("lost source bound")
        };
        let segment = path.segments.last().unwrap();
        assert_eq!(segment.args.len(), 1);
        assert_eq!(segment.associated_types.len(), 2);
        assert!(
            matches!(&segment.associated_types[0].value, Type::Tuple(fields) if fields.len() == 2)
        );
        assert!(
            name_scope::ident_eq(
                program.names.as_ref(),
                segment.associated_types[0].name,
                segment.associated_types[1].name,
                LookupClass::Member
            )
            .unwrap()
        );
        assert!(
            !exact_source_ident(
                program.names.as_ref(),
                segment.associated_types[0].name,
                segment.associated_types[1].name
            )
            .unwrap()
        );
        let budget = ir::PlanBudget::default();
        let copied = copy_source_path(path, &budget, 0).unwrap();
        assert!(
            exact_source_generic_path(program.names.as_ref(), path, &copied, &budget, 0).unwrap()
        );
    }

    #[test]
    fn associated_equality_conflicts_are_not_parser_or_resolver_invalidity() {
        for source in [
            "trait Source{type Out;}fn f<T:Source<Out=i32,Out=bool>>(){}fn main(){}",
            "trait Source{type Out;}fn f<T:Source<Out=i32>+Source<Out=bool>>(){}fn main(){}",
            "trait Source{type Out;}fn f<T>() where T:Source<Out=i32>{}fn main(){}",
            "trait Source{type Out;}struct S<T>{value:T}impl<T:Source<Out=i32>> S<T>{fn f(self){}}fn main(){}",
        ] {
            parse(source).unwrap_or_else(|error| panic!("{source}: {error}"));
        }
    }

    #[test]
    fn associated_equality_nonbound_sites_are_explicit_boundaries() {
        for source in [
            "trait Source{type Out;}trait Child:Source<Out=i32>{}fn main(){}",
            "trait Source{type Out;}trait Child<T:Source<Out=i32>>{}fn main(){}",
            "trait Source{type Out;}struct S;impl Source<Out=i32> for S{type Out=i32;}fn main(){}",
            "struct S<T>{value:T}fn f(x:S<Out=i32>){}fn main(){}",
            "trait Source{type Out;fn get(self)->Self::Out;}struct S;fn f(x:S)->i32{<S as Source<Out=i32>>::get(x)}fn main(){}",
        ] {
            let error = parse(source).unwrap_err();
            assert!(error.contains("unsupported:"), "{source}: {error}");
        }
        let error =
            parse_syntax_unprepared("fn f(x:i32){x.method::<Out=i32>();}fn main(){}").unwrap_err();
        assert_eq!(
            error.kind,
            super::super::source::SourceErrorKind::Unsupported
        );
    }

    #[test]
    fn associated_equality_validates_late_members_before_capability() {
        let error =
            parse("trait Source{type Out;}trait Child:Source<Out=&i32,Missing=i32>{}fn main(){}")
                .unwrap_err();
        assert!(!error.starts_with("unsupported:"), "{error}");
        assert!(error.contains("no associated type"), "{error}");
        let error = parse_syntax_unprepared(
            "trait Source<A>{type Out;}fn f<T:Source<Out=i32,bool>>(){}fn main(){}",
        )
        .unwrap_err();
        assert_eq!(error.kind, super::super::source::SourceErrorKind::Syntax);
    }

    #[test]
    fn source_trait_parameters_bind_forward_actuals_and_inherited_method_types() {
        let parsed = parse_unresolved("trait Base<T>{}trait Derived<T:Base<U>,U>:Base<T> where U:Base<T>{fn choose(self,x:U)->T;fn keep(self,x:T)->T{x}}fn forward<T:Base<U>,U>(x:T,y:U){}fn shadow<T:Base<i32>,i32>(x:T,y:i32){}fn main(){}").unwrap();
        let program = &parsed.program;
        let declaration = &program.traits[1];
        assert_eq!(declaration.type_params.len(), 2);
        assert_eq!(declaration.bounds.len(), 2);
        parameter(
            program,
            &actuals(&declaration.bounds[0].traits[0])[0],
            declaration.type_params[1],
        );
        parameter(
            program,
            &actuals(&declaration.bounds[1].traits[0])[0],
            declaration.type_params[0],
        );
        parameter(
            program,
            &actuals(&declaration.supertraits[0])[0],
            declaration.type_params[0],
        );
        let required = &declaration.items[0].signature;
        assert!(required.type_params.is_empty());
        parameter(program, &required.params[0].1, declaration.type_params[1]);
        parameter(program, &required.result, declaration.type_params[0]);
        let default = declaration.items[1].default.as_ref().unwrap();
        assert!(default.type_params.is_empty());
        parameter(program, &default.params[0].1, declaration.type_params[0]);
        parameter(program, &default.result, declaration.type_params[0]);
        for function in &program.functions[..2] {
            parameter(
                program,
                &actuals(&function.bounds[0].traits[0])[0],
                function.type_params[1],
            );
        }
        parameter(
            program,
            &program.functions[1].params[1].1,
            program.functions[1].type_params[1],
        );
    }

    #[test]
    fn source_trait_impl_and_ufcs_keep_actual_types_without_extra_calls() {
        let parsed = parse_unresolved("trait Use<T>{fn use_it(self,x:T)->T;}struct S;impl Use<(i32,bool)> for S{fn use_it(self,x:(i32,bool))->(i32,bool){x}}impl Use<i32> for S{fn use_it(self,x:i32)->i32{x}}fn invoke(s:S)->i32{<S as Use<i32>>::use_it(s,42)}fn main(){}").unwrap();
        let program = &parsed.program;
        assert!(
            matches!(actuals(program.impls[0].trait_path.as_ref().unwrap()), [Type::Tuple(fields)] if fields.len()==2)
        );
        let function = &program.functions[0];
        let Expr::TraitCall {
            trait_path, args, ..
        } = function.body.tail.as_deref().unwrap()
        else {
            panic!("UFCS source call lost its actual syntax");
        };
        assert!(matches!(actuals(trait_path), [Type::Int(ir::IntTy::I32)]));
        assert_eq!(args.len(), 2);
        let budget = ir::PlanBudget::default();
        let body = sr::OriginalBodyId::new(&budget).unwrap();
        let inventory = SourceInventory::build(program, function, body, &budget).unwrap();
        assert_eq!(inventory.calls.len(), 1);
        assert!(inventory.calls[0].ordinary.is_none());
    }

    #[test]
    fn source_trait_argument_placement_and_method_generic_boundaries_stay_closed() {
        for source in [
            "trait T{fn value<U>(x:U);}fn main(){}",
            "trait T<const N:usize>{}fn main(){}",
            "trait T<U=i32>{}fn main(){}",
            "fn f<T:module<i32>::Trait>(){}fn main(){}",
        ] {
            let error = parse_syntax_unprepared(source).unwrap_err();
            assert_eq!(
                error.kind,
                super::super::source::SourceErrorKind::Unsupported,
                "{source}: {error}"
            );
        }
        let params = (0..32)
            .map(|i| format!("T{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let error =
            parse_syntax_unprepared(&format!("trait T<{params}>{{}}fn main(){{}}")).unwrap_err();
        assert_eq!(error.kind, super::super::source::SourceErrorKind::Limit);
    }

    #[test]
    fn generic_parameter_predeclaration_checks_work_before_scanning_and_releases() {
        use super::super::expansion_budget::{BudgetResource, Counter};
        let (tokens, session) = lex("<T:Bound<U>,U>").unwrap().into_parts();
        let mut names =
            NameStoreBuilder::new(session, super::super::token::Edition::E2024).unwrap();
        let budget = names.budget().clone();
        let mut parser = Parser::new(&tokens, &mut names).unwrap();
        let remaining = budget.limits().counters[Counter::Work as usize]
            - budget.snapshot().counters[Counter::Work as usize];
        budget.charge(Counter::Work, remaining).unwrap();
        let error = parser.predeclare_type_parameters().unwrap_err();
        assert_eq!(error.kind, super::super::source::SourceErrorKind::Limit);
        assert_eq!(
            budget.snapshot().failure.unwrap().resource,
            BudgetResource::Counter(Counter::Work)
        );
        assert!(parser.type_params.is_empty());
        drop(parser);
        drop(names);
        drop(tokens);
        assert_eq!(budget.snapshot().retained_bytes, 0);
        assert_eq!(budget.snapshot().temporary_bytes, 0);
    }
}
enum DepthNode<'a> {
    Type(&'a Type),
    Pattern(&'a Pattern),
    Expr(&'a Expr),
    Block(&'a Block),
}
fn validate_depth(root: DepthNode<'_>) -> Result<(), String> {
    walk_depth(root, None)
}
fn walk_depth(root: DepthNode<'_>, budget: Option<&ir::PlanBudget>) -> Result<(), String> {
    walk_depth_checked(root, budget, None)
}
fn name_walk_work(names: Option<&NameStore>, amount: u64) -> Result<(), String> {
    if let Some(names) = names {
        names
            .budget()
            .charge(super::expansion_budget::Counter::Work, amount)
            .map_err(|error| {
                super::source::SourceError::from_token(super::token_session::TokenError::Budget(
                    error,
                ))
                .to_string()
            })?;
    }
    Ok(())
}
fn walk_depth_checked(
    root: DepthNode<'_>,
    budget: Option<&ir::PlanBudget>,
    names: Option<&NameStore>,
) -> Result<(), String> {
    walk_depth_inspecting(root, budget, names, None)
}
type SourceNodeInspector<'a> = dyn FnMut(&DepthNode<'_>, usize) -> Result<(), String> + 'a;
fn walk_depth_inspecting(
    root: DepthNode<'_>,
    budget: Option<&ir::PlanBudget>,
    names: Option<&NameStore>,
    mut inspect: Option<&mut SourceNodeInspector<'_>>,
) -> Result<(), String> {
    let mut stack = Vec::new();
    let mut checked_stack = names
        .map(|names| {
            super::token_session::BudgetVec::new(
                names.budget(),
                super::expansion_budget::Storage::Temporary,
            )
        })
        .transpose()
        .map_err(|error| super::source::SourceError::from_token(error).to_string())?;
    name_walk_work(names, 1)?;
    if let Some(stack) = &mut checked_stack {
        stack
            .push((root, 0usize))
            .map_err(|error| super::source::SourceError::from_token(error).to_string())?;
    } else {
        stack.push((root, 0usize));
    }
    let mut pattern_nodes = 0usize;
    while let Some((node, depth)) = if let Some(stack) = &mut checked_stack {
        stack.pop()
    } else {
        stack.pop()
    } {
        if let Some(inspect) = inspect.as_deref_mut() {
            inspect(&node, depth)?;
        }
        if let Some(names) = names {
            name_walk_work(Some(names), 1)?;
            let check =
                |label: AstIdent| names.check(label).map_err(native_adapter::name_diagnostic);
            match &node {
                DepthNode::Expr(
                    Expr::Loop { label, .. } | Expr::Break { label, .. } | Expr::Continue { label },
                ) => {
                    if let Some(label) = label {
                        check(*label)?;
                    }
                }
                DepthNode::Expr(Expr::Output(output)) => validate_output_site(output, names)?,
                DepthNode::Expr(Expr::Panic(panic)) => validate_panic_site(panic, names)?,
                DepthNode::Expr(Expr::LabeledBlock { label, .. }) => check(*label)?,
                _ => {}
            }
        }
        if let Some(budget) = budget {
            preflight_clone_node(&node, budget)?;
        }
        if depth > 128 {
            return Err("AST nesting exceeds limit 128".into());
        }
        let mut push = |node| -> Result<(), String> {
            name_walk_work(names, 1)?;
            if let Some(stack) = &mut checked_stack {
                stack
                    .push((node, depth + 1))
                    .map_err(|error| super::source::SourceError::from_token(error).to_string())?;
            } else {
                stack.push((node, depth + 1));
            }
            Ok(())
        };
        match node {
            DepthNode::Type(Type::Projection(projection)) => {
                if let Some(names) = names {
                    let item = match projection.as_ref() {
                        SourceProjection::Qualified { item, .. }
                        | SourceProjection::SelfItem { item } => *item,
                    };
                    names.check(item).map_err(native_adapter::name_diagnostic)?;
                }
                if let SourceProjection::Qualified {
                    self_ty,
                    trait_path,
                    ..
                } = projection.as_ref()
                {
                    push(DepthNode::Type(self_ty))?;
                    if let TraitPath::Source(path) = trait_path {
                        for segment in &path.segments {
                            name_walk_work(names, 1)?;
                            for actual in segment.args.iter().chain(
                                segment
                                    .associated_types
                                    .iter()
                                    .map(|binding| &binding.value),
                            ) {
                                push(DepthNode::Type(actual))?;
                            }
                        }
                    }
                }
            }
            DepthNode::Type(Type::Ref { referent, .. }) => push(DepthNode::Type(referent))?,
            DepthNode::Type(
                Type::Array(element, _)
                | Type::Slice(element)
                | Type::RawPtr {
                    pointee: element, ..
                },
            ) => push(DepthNode::Type(element))?,
            DepthNode::Type(Type::Tuple(fields)) => {
                for field in fields {
                    push(DepthNode::Type(field))?;
                }
            }
            DepthNode::Type(Type::Apply(path)) => {
                for segment in &path.segments {
                    name_walk_work(names, 1)?;
                    for arg in segment.args.iter().chain(
                        segment
                            .associated_types
                            .iter()
                            .map(|binding| &binding.value),
                    ) {
                        push(DepthNode::Type(arg))?;
                    }
                }
            }
            DepthNode::Type(_) => {}
            DepthNode::Pattern(pattern) => {
                pattern_nodes += 1;
                if pattern_nodes > ir::MAX_PATTERN_PROJECTION_WORK {
                    return Err("pattern node count exceeds limit".into());
                }
                match pattern {
                    Pattern::Apply { path, pattern } => {
                        push(DepthNode::Pattern(pattern))?;
                        for segment in &path.segments {
                            name_walk_work(names, 1)?;
                            for arg in segment.args.iter().chain(
                                segment
                                    .associated_types
                                    .iter()
                                    .map(|binding| &binding.value),
                            ) {
                                push(DepthNode::Type(arg))?;
                            }
                        }
                    }
                    Pattern::Variant { fields, .. } => {
                        for (_, p) in fields {
                            push(DepthNode::Pattern(p))?;
                        }
                    }
                    Pattern::Tuple(fields) | Pattern::Array(fields) => {
                        for pattern in fields {
                            push(DepthNode::Pattern(pattern))?;
                        }
                    }
                    Pattern::TupleRest(fields)
                    | Pattern::ArrayRest(fields)
                    | Pattern::VariantRest { fields, .. } => {
                        for pattern in fields.prefix.iter().chain(&fields.suffix) {
                            push(DepthNode::Pattern(pattern))?;
                        }
                    }
                    Pattern::Struct { fields, .. } => {
                        for (_, pattern) in fields {
                            push(DepthNode::Pattern(pattern))?;
                        }
                    }
                    Pattern::Ref { pattern, .. } => push(DepthNode::Pattern(pattern))?,
                    _ => {}
                }
            }
            DepthNode::Expr(expr) => match expr {
                Expr::MethodCall {
                    receiver,
                    type_args,
                    args,
                    ..
                } => {
                    push(DepthNode::Expr(receiver))?;
                    for ty in type_args {
                        push(DepthNode::Type(ty))?;
                    }
                    for arg in args {
                        push(DepthNode::Expr(arg))?;
                    }
                }
                Expr::TraitCall {
                    owner,
                    trait_path,
                    args,
                    ..
                } => {
                    push(DepthNode::Type(owner))?;
                    if let TraitPath::Source(path) = trait_path.as_ref() {
                        for segment in &path.segments {
                            name_walk_work(names, 1)?;
                            for arg in segment.args.iter().chain(
                                segment
                                    .associated_types
                                    .iter()
                                    .map(|binding| &binding.value),
                            ) {
                                push(DepthNode::Type(arg))?;
                            }
                        }
                    }
                    for arg in args {
                        push(DepthNode::Expr(arg))?;
                    }
                }
                Expr::AssociatedCall {
                    owner,
                    type_args,
                    args,
                    ..
                } => {
                    push(DepthNode::Type(owner))?;
                    for ty in type_args {
                        push(DepthNode::Type(ty))?;
                    }
                    for arg in args {
                        push(DepthNode::Expr(arg))?;
                    }
                }
                Expr::Apply { path, value } => {
                    push(DepthNode::Expr(value))?;
                    for segment in &path.segments {
                        name_walk_work(names, 1)?;
                        for arg in segment.args.iter().chain(
                            segment
                                .associated_types
                                .iter()
                                .map(|binding| &binding.value),
                        ) {
                            push(DepthNode::Type(arg))?;
                        }
                    }
                }
                Expr::Cast { value, target } => {
                    push(DepthNode::Expr(value))?;
                    push(DepthNode::Type(target))?;
                }
                Expr::Array(elements) => {
                    for e in elements {
                        push(DepthNode::Expr(e))?;
                    }
                }
                Expr::Deref { value } | Expr::BorrowValue { value, .. } => {
                    push(DepthNode::Expr(value))?
                }
                Expr::Repeat { value, .. } => push(DepthNode::Expr(value))?,
                Expr::DynamicIndex { base, index } => {
                    push(DepthNode::Expr(base))?;
                    push(DepthNode::Expr(index))?;
                }
                Expr::Index { base, .. } => push(DepthNode::Expr(base))?,
                Expr::EnumVariant { fields, .. } => {
                    for (_, e) in fields {
                        push(DepthNode::Expr(e))?;
                    }
                }
                Expr::Match { scrutinee, arms } => {
                    push(DepthNode::Expr(scrutinee))?;
                    for arm in arms {
                        push(DepthNode::Pattern(&arm.pattern))?;
                        push(DepthNode::Expr(&arm.body))?;
                    }
                }
                Expr::Tuple(fields) | Expr::TupleStruct { fields, .. } => {
                    for field in fields {
                        push(DepthNode::Expr(field))?;
                    }
                }
                Expr::Struct { fields, .. } => {
                    for (_, value) in fields {
                        push(DepthNode::Expr(value))?;
                    }
                }
                Expr::Field { base, .. } => push(DepthNode::Expr(base))?,
                Expr::Unary { value, .. } => push(DepthNode::Expr(value))?,
                Expr::Binary { left, right, .. } => {
                    push(DepthNode::Expr(left))?;
                    push(DepthNode::Expr(right))?;
                }
                Expr::Call { args, .. }
                | Expr::Format {
                    arguments: args, ..
                } => {
                    for arg in args {
                        push(DepthNode::Expr(arg))?;
                    }
                }
                Expr::IfLet {
                    pattern,
                    scrutinee,
                    then_block,
                    else_branch,
                } => {
                    push(DepthNode::Pattern(pattern))?;
                    push(DepthNode::Expr(scrutinee))?;
                    push(DepthNode::Block(then_block))?;
                    if let Some(branch) = else_branch {
                        push(DepthNode::Expr(branch))?;
                    }
                }
                Expr::If {
                    condition,
                    then_block,
                    else_block,
                } => {
                    push(DepthNode::Expr(condition))?;
                    push(DepthNode::Block(then_block))?;
                    push(DepthNode::Block(else_block))?;
                }
                Expr::Block(block)
                | Expr::Unsafe(block)
                | Expr::Loop { body: block, .. }
                | Expr::LabeledBlock { body: block, .. } => push(DepthNode::Block(block))?,
                Expr::Break {
                    value: Some(value), ..
                } => push(DepthNode::Expr(value))?,
                _ => {}
            },
            DepthNode::Block(block) => {
                for stmt in &block.statements {
                    name_walk_work(names, 1)?;
                    if let (
                        Some(names),
                        Stmt::While {
                            label: Some(label), ..
                        }
                        | Stmt::WhileLet {
                            label: Some(label), ..
                        }
                        | Stmt::For {
                            label: Some(label), ..
                        },
                    ) = (names, stmt)
                    {
                        names
                            .check(*label)
                            .map_err(native_adapter::name_diagnostic)?;
                    }
                    match stmt {
                        Stmt::Declare { ty, .. } => push(DepthNode::Type(ty))?,
                        Stmt::Let { value, ty, .. } => {
                            push(DepthNode::Expr(value))?;
                            push(DepthNode::Type(ty))?;
                        }
                        Stmt::LetElse {
                            pattern,
                            ty,
                            value,
                            else_block,
                        } => {
                            push(DepthNode::Pattern(pattern))?;
                            push(DepthNode::Type(ty))?;
                            push(DepthNode::Expr(value))?;
                            push(DepthNode::Block(else_block))?;
                        }
                        Stmt::LetPattern { pattern, ty, value } => {
                            push(DepthNode::Pattern(pattern))?;
                            push(DepthNode::Type(ty))?;
                            push(DepthNode::Expr(value))?;
                        }
                        Stmt::AssignDeref { carrier, value } => {
                            push(DepthNode::Expr(value))?;
                            push(DepthNode::Expr(carrier))?;
                        }
                        Stmt::AssignIndex { index, value, .. }
                        | Stmt::AssignIndexOp { index, value, .. } => {
                            push(DepthNode::Expr(value))?;
                            push(DepthNode::Expr(index))?;
                        }
                        Stmt::Assign { value, .. }
                        | Stmt::AssignOp { value, .. }
                        | Stmt::AssignPlace { value, .. }
                        | Stmt::Expr(value)
                        | Stmt::ImplicitExpr(value)
                        | Stmt::Return(Some(value)) => push(DepthNode::Expr(value))?,
                        Stmt::WhileLet {
                            pattern,
                            scrutinee,
                            body,
                            ..
                        } => {
                            push(DepthNode::Pattern(pattern))?;
                            push(DepthNode::Expr(scrutinee))?;
                            push(DepthNode::Block(body))?;
                        }
                        Stmt::For {
                            pattern,
                            source,
                            body,
                            ..
                        } => {
                            push(DepthNode::Pattern(pattern))?;
                            match &source.iterable {
                                ForIterable::Range { start, end, .. } => {
                                    push(DepthNode::Expr(start))?;
                                    push(DepthNode::Expr(end))?;
                                }
                                ForIterable::Value(value) => push(DepthNode::Expr(value))?,
                            }
                            push(DepthNode::Block(body))?;
                        }
                        Stmt::While {
                            condition, body, ..
                        } => {
                            push(DepthNode::Expr(condition))?;
                            push(DepthNode::Block(body))?;
                        }
                        Stmt::Return(None) => {}
                    }
                }
                if let Some(tail) = &block.tail {
                    push(DepthNode::Expr(tail))?;
                }
            }
        }
    }
    Ok(())
}
fn exact_source_ident(names: &NameStore, a: AstIdent, b: AstIdent) -> Result<bool, String> {
    use super::semantic_name::occurrence_index;
    Ok(
        occurrence_index(names, a).map_err(native_adapter::name_diagnostic)?
            == occurrence_index(names, b).map_err(native_adapter::name_diagnostic)?,
    )
}
fn exact_source_path(
    names: &NameStore,
    a: &AstPath,
    b: &AstPath,
    budget: &ir::PlanBudget,
) -> Result<bool, String> {
    let mut equal = true;
    for (a, b) in [
        (a.clone(), b.clone()),
        (
            a.original_use(names)
                .map_err(native_adapter::name_diagnostic)?,
            b.original_use(names)
                .map_err(native_adapter::name_diagnostic)?,
        ),
    ] {
        a.check(names).map_err(native_adapter::name_diagnostic)?;
        b.check(names).map_err(native_adapter::name_diagnostic)?;
        budget.charge_work(
            a.len()
                .checked_add(b.len())
                .ok_or("source path work overflow")?,
        )?;
        equal &= a.anchor() == b.anchor() && a.len() == b.len();
        for (a, b) in a.segments().iter().zip(b.segments()) {
            equal &= exact_source_ident(names, *a, *b)?;
        }
    }
    Ok(equal)
}
fn exact_source_generic_path(
    names: &NameStore,
    a: &GenericPath,
    b: &GenericPath,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<bool, String> {
    budget.check_depth(depth)?;
    budget.charge_work(
        a.segments
            .len()
            .checked_add(b.segments.len())
            .ok_or("source generic path work overflow")?,
    )?;
    let mut equal = exact_source_path(names, &a.path(names)?, &b.path(names)?, budget)?;
    equal &= a.anchor == b.anchor && a.segments.len() == b.segments.len();
    for (a, b) in a.segments.iter().zip(&b.segments) {
        equal &= exact_source_ident(names, a.name, b.name)?;
        equal &= a.args.len() == b.args.len();
        equal &= a.associated_types.len() == b.associated_types.len();
        for (a, b) in a.associated_types.iter().zip(&b.associated_types) {
            equal &= exact_source_ident(names, a.name, b.name)?;
            equal &= exact_source_type(names, &a.value, &b.value, budget, depth + 1)?;
        }
        for (a, b) in a.args.iter().zip(&b.args) {
            equal &= exact_source_type(names, a, b, budget, depth + 1)?;
        }
    }
    Ok(equal)
}
fn exact_source_type(
    names: &NameStore,
    a: &Type,
    b: &Type,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<bool, String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    Ok(match (a, b) {
        (Type::Projection(a), Type::Projection(b)) => {
            exact_source_projection(names, a, b, budget, depth + 1)?
        }
        (Type::Param(a), Type::Param(b)) => exact_source_ident(names, *a, *b)?,
        (Type::Named(a), Type::Named(b)) => exact_source_path(names, a, b, budget)?,
        (Type::Apply(a), Type::Apply(b)) => {
            exact_source_generic_path(names, a, b, budget, depth + 1)?
        }
        (
            Type::Ref {
                kind: ak,
                region: ar,
                referent: a,
            },
            Type::Ref {
                kind: bk,
                region: br,
                referent: b,
            },
        ) => {
            let region = match (ar, br) {
                (None, None)
                | (Some(RegionName::Static), Some(RegionName::Static))
                | (Some(RegionName::Infer), Some(RegionName::Infer)) => true,
                (Some(RegionName::Named(a)), Some(RegionName::Named(b))) => {
                    exact_source_ident(names, *a, *b)?
                }
                _ => false,
            };
            let referent = exact_source_type(names, a, b, budget, depth + 1)?;
            ak == bk && region && referent
        }
        (Type::Array(a, al), Type::Array(b, bl)) => {
            exact_source_type(names, a, b, budget, depth + 1)? && al == bl
        }
        (Type::Slice(a), Type::Slice(b)) => exact_source_type(names, a, b, budget, depth + 1)?,
        (
            Type::RawPtr {
                kind: ak,
                pointee: a,
            },
            Type::RawPtr {
                kind: bk,
                pointee: b,
            },
        ) => exact_source_type(names, a, b, budget, depth + 1)? && ak == bk,
        (Type::Tuple(a), Type::Tuple(b)) => {
            let mut equal = a.len() == b.len();
            for (a, b) in a.iter().zip(b) {
                equal &= exact_source_type(names, a, b, budget, depth + 1)?;
            }
            equal
        }
        (Type::Int(a), Type::Int(b)) => a == b,
        (Type::Float(a), Type::Float(b)) => a == b,
        (Type::SelfType, Type::SelfType)
        | (Type::Char, Type::Char)
        | (Type::Str, Type::Str)
        | (Type::Bool, Type::Bool)
        | (Type::Unit, Type::Unit)
        | (Type::Never, Type::Never)
        | (Type::Infer, Type::Infer) => true,
        _ => false,
    })
}
fn exact_source_projection(
    names: &NameStore,
    a: &SourceProjection,
    b: &SourceProjection,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<bool, String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    Ok(match (a, b) {
        (SourceProjection::SelfItem { item: a }, SourceProjection::SelfItem { item: b }) => {
            exact_source_ident(names, *a, *b)?
        }
        (
            SourceProjection::Qualified {
                self_ty: a,
                trait_path: at,
                item: ai,
            },
            SourceProjection::Qualified {
                self_ty: b,
                trait_path: bt,
                item: bi,
            },
        ) => {
            let mut equal = exact_source_ident(names, *ai, *bi)?;
            equal &= exact_source_type(names, a, b, budget, depth + 1)?;
            equal &= match (at, bt) {
                (TraitPath::Source(a), TraitPath::Source(b)) => {
                    exact_source_generic_path(names, a, b, budget, depth + 1)?
                }
                (TraitPath::Builtin(a), TraitPath::Builtin(b)) => a == b,
                _ => false,
            };
            equal
        }
        _ => false,
    })
}
fn retained_node_projection(
    names: &NameStore,
    root: DepthNode<'_>,
    projection: &SourceProjection,
    budget: &ir::PlanBudget,
) -> Result<bool, String> {
    // Check the complete candidate before an unequal retained site can discard
    // any child. Equality here is exact source occurrence, not NameKey equality.
    exact_source_projection(names, projection, projection, budget, 0)?;
    let mut found = false;
    let mut inspect = |node: &DepthNode<'_>, depth| {
        budget.check_depth(depth)?;
        budget.charge_work(1)?;
        if let DepthNode::Type(Type::Projection(retained)) = node {
            found |= exact_source_projection(names, retained, projection, budget, 0)?;
        }
        Ok(())
    };
    // The checked worklist owns temporary storage. This is inspection, not a
    // clone, so it must not retain clone reservations for every visited node.
    walk_depth_inspecting(root, None, Some(names), Some(&mut inspect))?;
    Ok(found)
}
fn retained_type_projection(
    names: &NameStore,
    ty: &Type,
    projection: &SourceProjection,
    budget: &ir::PlanBudget,
) -> Result<bool, String> {
    retained_node_projection(names, DepthNode::Type(ty), projection, budget)
}
fn retained_function_projection(
    names: &NameStore,
    function: &Function,
    projection: &SourceProjection,
    budget: &ir::PlanBudget,
) -> Result<bool, String> {
    let mut found = retained_type_projection(names, &function.result, projection, budget)?;
    for (_, ty) in &function.params {
        budget.charge_work(1)?;
        found |= retained_type_projection(names, ty, projection, budget)?;
    }
    for bound in &function.bounds {
        for path in &bound.traits {
            budget.charge_work(1)?;
            if let TraitPath::Source(path) = path {
                for segment in &path.segments {
                    for actual in segment.args.iter().chain(
                        segment
                            .associated_types
                            .iter()
                            .map(|binding| &binding.value),
                    ) {
                        found |= retained_type_projection(names, actual, projection, budget)?;
                    }
                }
            }
        }
    }
    found |= retained_node_projection(names, DepthNode::Block(&function.body), projection, budget)?;
    Ok(found)
}
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug)]
pub enum DeclItemName {
    Source(AstIdent),
    Impl,
    Catalog(tr::CatalogItem),
}
#[derive(Clone, Debug)]
pub struct DeclSemanticPath {
    pub module: ModuleId,
    pub item: DeclItemName,
    pub kind: ir::DeclKind,
    pub associated: Option<AssociatedSemanticOwner>,
}
#[derive(Clone, Debug)]
pub struct DeclInfo {
    pub receiver: ReceiverKind,
    pub owner: Option<ir::DeclId>,
    pub id: ir::DeclId,
    pub kind: ir::DeclKind,
    pub name: String,
    pub path: DeclSemanticPath,
    pub source_index: usize,
    pub param_env: Arc<ir::ParamEnv>,
    pub type_params: Arc<[AstIdent]>,
    pub env: Arc<ir::GenericEnv>,
    pub visibility: ir::Visibility,
}
#[derive(Debug)]
pub struct DeclRegistry {
    program: Arc<Program>,
    names: Arc<NameStore>,
    modules: Arc<ModuleArena>,
    symbols: Arc<ProgramSymbols>,
    declarations: Vec<DeclInfo>,
    functions: HashMap<PathKey, ir::DeclId>,
    nominals: HashMap<PathKey, ir::DeclId>,
    function_ids: Vec<ir::DeclId>,
    callable_sources: Vec<CallableSource>,
    impl_ids: Vec<ir::DeclId>,
    associated_ids: Vec<Vec<ir::DeclId>>,
    traits: HashMap<TraitLookupKey, ir::DeclId>,
    trait_ids: Vec<ir::DeclId>,
    trait_items: Vec<Vec<ir::DeclId>>,
    trait_associated_types: Vec<Vec<ir::DeclId>>,
    associated_type_bounds: HashMap<ir::DeclId, FrozenAssociatedBounds>,
    trait_imports: Vec<(ModuleId, TraitPath)>,
    /// First position of each callable in `function_ids`, built once when the
    /// registry is sealed. Replaces O(callables) scans per lookup.
    function_positions: HashMap<ir::DeclId, usize>,
    function_positions_sealed: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum TraitLookupKey {
    Source(PathKey),
    Builtin(BuiltinTrait),
}
#[derive(Debug)]
struct FrozenAssociatedBounds {
    ids: Arc<[ir::DeclId]>,
    source_index: Option<usize>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum GenericBindingKey {
    Param(NameKey),
    SelfType,
}
#[derive(Clone, Copy, Debug)]
enum CallableSource {
    Free(usize),
    Associated { implementation: usize, item: usize },
    TraitDefault { trait_index: usize, item: usize },
}
impl DeclRegistry {
    /// First `function_ids` position for `decl`. Uses the sealed index; during
    /// registry construction (before the seal) it falls back to a scan.
    fn function_position(&self, decl: ir::DeclId) -> Option<usize> {
        if self.function_positions_sealed {
            return self.function_positions.get(&decl).copied();
        }
        self.function_ids.iter().position(|id| *id == decl)
    }
    /// Retained source callable for a function declaration, if any.
    pub(super) fn source_callable(&self, decl: ir::DeclId) -> Option<&Function> {
        let source = self
            .function_position(decl)
            .and_then(|index| self.callable_sources.get(index).copied())?;
        match source {
            CallableSource::Free(index) => self.program.functions.get(index),
            CallableSource::Associated {
                implementation,
                item,
            } => self
                .program
                .impls
                .get(implementation)
                .and_then(|block| block.items.get(item))
                .map(|item| &item.function),
            CallableSource::TraitDefault { trait_index, item } => self
                .program
                .traits
                .get(trait_index)
                .and_then(|owner| owner.items.get(item))
                .and_then(|item| item.default.as_ref()),
        }
    }
    /// Explicit generic arguments may name only the written parameters, never
    /// anonymous argument-position `impl Trait` parameters (stock E0107/E0632).
    pub(super) fn check_explicit_impl_params(
        &self,
        decl: ir::DeclId,
        written: usize,
    ) -> Result<(), String> {
        let Some(function) = self.source_callable(decl) else {
            return Ok(());
        };
        if function.impl_params == 0 {
            return Ok(());
        }
        let explicit = function.type_params.len() - function.impl_params;
        if written > explicit {
            return Err(format!(
                "E0107: function takes {explicit} generic arguments but {written} were supplied"
            ));
        }
        // Stock accepts explicit arguments for the written parameters and infers
        // the anonymous ones; that needs partial inference, not yet native.
        Err("unsupported: explicit generic arguments on a function with argument-position `impl Trait` (partial inference)".into())
    }
    fn source_type_context(&self, declaration: ir::DeclId) -> Result<SourceTypeContext, String> {
        let info = self.declaration(declaration)?;
        self.modules
            .check(info.path.module)
            .map_err(module_diagnostic)?;
        let owner = match info.kind {
            ir::DeclKind::Trait | ir::DeclKind::TraitImpl => Some(info.id),
            ir::DeclKind::TraitItem | ir::DeclKind::AssociatedType => info.owner,
            ir::DeclKind::AssociatedFunction => info
                .owner
                .or_else(|| info.path.associated.as_ref().map(|owner| owner.impl_decl)),
            _ => None,
        };
        if let Some(owner) = owner {
            let retained = self.declaration(owner)?;
            let module = match retained.kind {
                ir::DeclKind::Trait => self
                    .program
                    .traits
                    .get(retained.source_index)
                    .map(|item| item.module),
                ir::DeclKind::TraitImpl => self
                    .program
                    .impls
                    .get(retained.source_index)
                    .map(|item| item.module),
                ir::DeclKind::Impl => {
                    return Ok(SourceTypeContext {
                        declaration,
                        module: info.path.module,
                        owner: None,
                    });
                }
                _ => return Err("invalid retained source projection owner".into()),
            }
            .ok_or("missing retained source projection owner")?;
            if module != info.path.module || module != retained.path.module {
                return Err("source projection owner module mismatch".into());
            }
        }
        Ok(SourceTypeContext {
            declaration,
            module: info.path.module,
            owner,
        })
    }
    fn validate_source_projection(
        &self,
        context: SourceTypeContext,
        projection: &SourceProjection,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        let expected = self.source_type_context(context.declaration)?;
        if context.module != expected.module || context.owner != expected.owner {
            return Err("source projection changed retained declaration owner or module".into());
        }
        let info = self.declaration(context.declaration)?;
        let names = self.names.as_ref();
        let mut found = match info.kind {
            ir::DeclKind::TraitImpl => {
                let block = self
                    .program
                    .impls
                    .get(info.source_index)
                    .ok_or("missing retained projection impl")?;
                let mut found = false;
                for row in &block.associated_types {
                    budget.charge_work(1)?;
                    found |= retained_type_projection(names, &row.value, projection, budget)?;
                }
                found
            }
            ir::DeclKind::TraitItem => {
                let owner =
                    self.declaration(info.owner.ok_or("trait source item missing owner")?)?;
                let head = &self
                    .program
                    .traits
                    .get(owner.source_index)
                    .ok_or("missing retained trait projection source")?
                    .items
                    .get(info.source_index)
                    .ok_or("missing retained trait projection item")?
                    .signature;
                let mut found = retained_type_projection(names, &head.result, projection, budget)?;
                for (_, ty) in &head.params {
                    budget.charge_work(1)?;
                    found |= retained_type_projection(names, ty, projection, budget)?;
                }
                found
            }
            ir::DeclKind::Function | ir::DeclKind::AssociatedFunction => {
                budget.charge_work(1)?;
                let source = self
                    .function_position(info.id)
                    .and_then(|index| self.callable_sources.get(index).copied());
                let function = match source.ok_or("missing retained projection callable")? {
                    CallableSource::Free(index) => self.program.functions.get(index),
                    CallableSource::Associated {
                        implementation,
                        item,
                    } => self
                        .program
                        .impls
                        .get(implementation)
                        .and_then(|block| block.items.get(item))
                        .map(|item| &item.function),
                    CallableSource::TraitDefault { trait_index, item } => self
                        .program
                        .traits
                        .get(trait_index)
                        .and_then(|owner| owner.items.get(item))
                        .and_then(|item| item.default.as_ref()),
                }
                .ok_or("missing retained projection function")?;
                if function.module != context.module {
                    return Err("projection callable retained module changed".into());
                }
                retained_function_projection(names, function, projection, budget)?
            }
            _ => false,
        };
        // Equality RHS projections retain their original bound occurrence,
        // including requirements inherited by an implementation callable.
        let mut inspect_bounds = |bounds: &[TypeBound]| -> Result<(), String> {
            for bound in bounds {
                for path in &bound.traits {
                    if let TraitPath::Source(path) = path {
                        for segment in &path.segments {
                            for ty in segment.args.iter().chain(
                                segment
                                    .associated_types
                                    .iter()
                                    .map(|binding| &binding.value),
                            ) {
                                found |= retained_type_projection(names, ty, projection, budget)?;
                            }
                        }
                    }
                }
            }
            Ok(())
        };
        match info.kind {
            ir::DeclKind::Trait => inspect_bounds(&self.program.traits[info.source_index].bounds)?,
            ir::DeclKind::TraitImpl => {
                inspect_bounds(&self.program.impls[info.source_index].bounds)?
            }
            ir::DeclKind::AssociatedFunction => {
                if let Some(owner) = info
                    .owner
                    .or_else(|| info.path.associated.as_ref().map(|owner| owner.impl_decl))
                {
                    let owner = self.declaration(owner)?;
                    if owner.kind == ir::DeclKind::TraitImpl {
                        inspect_bounds(&self.program.impls[owner.source_index].bounds)?;
                    }
                }
            }
            _ => {}
        }
        if !found {
            return Err(
                "projection member does not match its original retained source occurrence".into(),
            );
        }
        Ok(())
    }
    fn validate_source_bound_path(
        &self,
        context: SourceTypeContext,
        path: &TraitPath,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        let TraitPath::Source(candidate) = path else {
            return Ok(());
        };
        let expected = self.source_type_context(context.declaration)?;
        if context.module != expected.module || context.owner != expected.owner {
            return Err("associated equality changed source owner".into());
        }
        let mut found = false;
        let mut inspect = |bounds: &[TypeBound]| -> Result<(), String> {
            for bound in bounds {
                for retained in &bound.traits {
                    budget.charge_work(1)?;
                    if let TraitPath::Source(retained) = retained {
                        found |= exact_source_generic_path(
                            self.names.as_ref(),
                            candidate,
                            retained,
                            budget,
                            0,
                        )?;
                    }
                }
            }
            Ok(())
        };
        let info = self.declaration(context.declaration)?;
        match info.kind {
            ir::DeclKind::Trait => inspect(&self.program.traits[info.source_index].bounds)?,
            ir::DeclKind::TraitImpl => inspect(&self.program.impls[info.source_index].bounds)?,
            ir::DeclKind::Function | ir::DeclKind::AssociatedFunction => {
                for (index, id) in self.function_ids.iter().enumerate() {
                    budget.charge_work(1)?;
                    if *id != context.declaration {
                        continue;
                    }
                    match self.callable_sources[index] {
                        CallableSource::Free(index) => {
                            inspect(&self.program.functions[index].bounds)?
                        }
                        CallableSource::Associated {
                            implementation,
                            item,
                        } => {
                            let block = &self.program.impls[implementation];
                            inspect(&block.bounds)?;
                            inspect(&block.items[item].function.bounds)?;
                        }
                        CallableSource::TraitDefault { trait_index, item } => {
                            let declaration = &self.program.traits[trait_index];
                            inspect(&declaration.bounds)?;
                            if let Some(function) = &declaration.items[item].default {
                                inspect(&function.bounds)?;
                            }
                        }
                    }
                }
            }
            ir::DeclKind::TraitItem => {
                let owner = self.declaration(info.owner.ok_or("trait item has no owner")?)?;
                let declaration = &self.program.traits[owner.source_index];
                inspect(&declaration.bounds)?;
                inspect(&declaration.items[info.source_index].signature.bounds)?;
            }
            _ => {}
        }
        if !found {
            return Err(
                "associated equality does not match its original retained bound occurrence".into(),
            );
        }
        Ok(())
    }
    fn associated_type_item(
        &self,
        owner: ir::DeclId,
        name: AstIdent,
        budget: &ir::PlanBudget,
    ) -> Result<ir::DeclId, String> {
        self.names
            .check(name)
            .map_err(native_adapter::name_diagnostic)?;
        let index = self
            .trait_ids
            .iter()
            .position(|id| *id == owner)
            .ok_or("unknown associated type trait owner")?;
        let mut found = None;
        for item in &self.trait_associated_types[index] {
            budget.charge_work(1)?;
            if self.source_member_equal(self.declaration(*item)?.path.item, name)?
                && found.replace(*item).is_some()
            {
                return Err("duplicate trait associated type declaration".into());
            }
        }
        found.ok_or_else(|| "extra or unknown associated type item".into())
    }
    fn source_member_equal(&self, left: DeclItemName, right: AstIdent) -> Result<bool, String> {
        self.names
            .check(right)
            .map_err(native_adapter::name_diagnostic)?;
        match left {
            DeclItemName::Source(left) => {
                name_scope::ident_eq(self.names.as_ref(), left, right, LookupClass::Member)
            }
            DeclItemName::Catalog(item) => {
                use super::semantic_name::BuiltinAtom;
                let atom = match item {
                    tr::CatalogItem::CloneItem => BuiltinAtom::CloneMethod,
                    tr::CatalogItem::CloneFromItem | tr::CatalogItem::CloneFromDefault => {
                        BuiltinAtom::CloneFromMethod
                    }
                    tr::CatalogItem::DropItem => BuiltinAtom::DropMethod,
                    _ => return Ok(false),
                };
                self.symbols.matches(self.names.as_ref(), right, atom)
            }
            DeclItemName::Impl => Ok(false),
        }
    }
    fn source_items_equal(&self, left: DeclItemName, right: DeclItemName) -> Result<bool, String> {
        match (left, right) {
            (DeclItemName::Source(left), DeclItemName::Source(right)) => {
                name_scope::ident_eq(self.names.as_ref(), left, right, LookupClass::Member)
            }
            (DeclItemName::Source(name), _) | (_, DeclItemName::Source(name)) => {
                self.names
                    .check(name)
                    .map_err(native_adapter::name_diagnostic)?;
                Ok(false)
            }
            (DeclItemName::Catalog(left), DeclItemName::Catalog(right)) => Ok(left == right),
            (DeclItemName::Impl, DeclItemName::Impl) => Ok(true),
            _ => Ok(false),
        }
    }
    fn trait_key(&self, path: &TraitPath) -> Result<TraitLookupKey, String> {
        Ok(match path {
            TraitPath::Source(path) => TraitLookupKey::Source(name_scope::path_key(
                self.names.as_ref(),
                &path.path(self.names.as_ref())?,
                Namespace::Type,
            )?),
            TraitPath::Builtin(kind) => TraitLookupKey::Builtin(*kind),
        })
    }
    fn resolve_trait_path(&self, path: &TraitPath) -> Result<ir::DeclId, String> {
        self.traits
            .get(&self.trait_key(path)?)
            .copied()
            .ok_or_else(|| "unknown trait".into())
    }
    fn parameter_key(&self, ident: AstIdent) -> Result<GenericBindingKey, String> {
        Ok(GenericBindingKey::Param(
            self.names
                .key(ident, LookupClass::TypeParameter)
                .map_err(native_adapter::name_diagnostic)?,
        ))
    }
    pub fn declaration(&self, id: ir::DeclId) -> Result<&DeclInfo, String> {
        self.declarations
            .get(id.index as usize)
            .filter(|d| d.id == id)
            .ok_or_else(|| "foreign generic declaration identity".into())
    }
}
#[derive(Clone, Debug)]
enum NominalSchema {
    Struct(StructDecl),
    Enum(EnumDecl),
}
#[derive(Debug, Default)]
pub struct NominalInterner {
    applications: HashMap<(ir::DeclId, Vec<ir::Ty>), ir::Ty>,
    ordered: Vec<ir::Ty>,
    active: Vec<ir::DeclId>,
}
#[derive(Clone, Copy, Debug)]
struct SourceTypeContext {
    declaration: ir::DeclId,
    module: ModuleId,
    owner: Option<ir::DeclId>,
}
#[derive(Debug)]
struct PlanState {
    source_context: Option<SourceTypeContext>,
    associated_type_rows: Vec<tr::AssociatedTypeValue>,
    associated_type_rhs: HashMap<(ir::DeclId, ir::DeclId), ir::Ty>,
    schemas: HashMap<ir::DeclId, Arc<NominalSchema>>,
    functions: HashMap<ir::DeclId, tr::GenericSchema>,
    region_modes: HashMap<ir::DeclId, tr::RegionContractMode>,
    source_regions: HashMap<ir::DeclId, Arc<ir::SourceRegionEnv>>,
    nominals: NominalInterner,
    contexts: HashMap<ir::DeclId, CallableContext>,
    reservations: Vec<ir::PlanReservation>,
}
#[derive(Clone, Debug)]
pub struct PlanCtx {
    program: Arc<Program>,
    catalog: Arc<tr::CoreCatalog>,
    world: Option<Arc<tr::ValidatedTraitWorld>>,
    validated: Arc<HashMap<ir::DeclId, ir::GenericSignature>>,
    registry: Arc<DeclRegistry>,
    state: Arc<Mutex<PlanState>>,
    /// Immutable whole-program signature and nominal lookup maps, built once
    /// when the trait world is published and shared by every body check.
    /// Previously each body copied every signature, which made checking
    /// quadratic in program size in both time and retained bytes.
    shared_env: Option<Arc<SharedTypeEnv>>,
    /// Callable indices of nongeneric Source callables in callable order:
    /// exactly the root slots `instances::build` enumerates first.
    closed_roots: Arc<[usize]>,
    pub budget: ir::PlanBudget,
}
/// Shared read-only part of a [`TypeEnv`].
#[derive(Debug)]
pub(super) struct SharedTypeEnv {
    structs: Arc<HashMap<PathKey, ir::StructTy>>,
    enums: Arc<HashMap<PathKey, ir::EnumTy>>,
    functions: Arc<HashMap<PathKey, ir::Signature>>,
}
fn source_type_storage(ty: &Type, budget: &ir::PlanBudget, depth: usize) -> Result<usize, String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    let mut bytes = std::mem::size_of::<Type>();
    let mut add = |n: usize| -> Result<(), String> {
        bytes = bytes
            .checked_add(n)
            .ok_or("generic schema allocation overflow")?;
        Ok(())
    };
    match ty {
        Type::Projection(projection) => {
            add(std::mem::size_of::<SourceProjection>())?;
            match projection.as_ref() {
                SourceProjection::Qualified {
                    self_ty,
                    trait_path,
                    ..
                } => {
                    add(source_type_storage(self_ty, budget, depth + 1)?)?;
                    if let TraitPath::Source(path) = trait_path {
                        for segment in &path.segments {
                            budget.charge_work(1)?;
                            add(std::mem::size_of::<PathSegment>())?;
                            for argument in &segment.args {
                                add(source_type_storage(argument, budget, depth + 1)?)?;
                            }
                            for binding in &segment.associated_types {
                                add(std::mem::size_of::<AssociatedTypeBinding>())?;
                                add(source_type_storage(&binding.value, budget, depth + 1)?)?;
                            }
                        }
                    }
                }
                SourceProjection::SelfItem { .. } => {}
            }
        }
        Type::Named(_) | Type::Param(_) => {}
        Type::Apply(path) => {
            budget.charge_work(path.segments.len())?;
            for segment in &path.segments {
                add(std::mem::size_of::<PathSegment>())?;
                for arg in &segment.args {
                    add(source_type_storage(arg, budget, depth + 1)?)?;
                }
                for binding in &segment.associated_types {
                    add(std::mem::size_of::<AssociatedTypeBinding>())?;
                    add(source_type_storage(&binding.value, budget, depth + 1)?)?;
                }
            }
        }
        Type::Tuple(fields) => {
            for field in fields {
                add(source_type_storage(field, budget, depth + 1)?)?;
            }
        }
        Type::Array(element, _)
        | Type::Slice(element)
        | Type::RawPtr {
            pointee: element, ..
        }
        | Type::Ref {
            referent: element, ..
        } => add(source_type_storage(element, budget, depth + 1)?)?,
        _ => {}
    }
    Ok(bytes)
}
fn registry_diagnostic_copy(text: &str, budget: &ir::PlanBudget) -> Result<String, String> {
    budget.charge_work(text.len())?;
    budget.retain_bytes(text.len())?;
    let mut value = String::new();
    value
        .try_reserve_exact(text.len())
        .map_err(|_| "infrastructure: registry diagnostic allocation failed")?;
    budget.retain_bytes(value.capacity().saturating_sub(text.len()))?;
    value.push_str(text);
    Ok(value)
}
fn reserve_registry_map<K: Eq + std::hash::Hash, V>(
    table: &mut HashMap<K, V>,
    entries: usize,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    if entries == 0 {
        return Ok(());
    }
    if !table.is_empty() {
        return Err("contract: registry capacity must be preflighted before insertion".into());
    }
    budget.charge_work(entries)?;
    let buckets = entries
        .checked_mul(8)
        .and_then(|n| n.checked_add(6))
        .map(|n| n / 7)
        .and_then(|n| n.max(4).checked_next_power_of_two())
        .ok_or("registry map allocation overflow")?;
    let entry_bytes = std::mem::size_of::<(K, V)>()
        .checked_add(1)
        .ok_or("registry map allocation overflow")?;
    let bytes = buckets
        .checked_mul(entry_bytes)
        .and_then(|n| n.checked_add(16))
        .ok_or("registry map allocation overflow")?;
    budget.retain_bytes(bytes)?;
    table
        .try_reserve(entries)
        .map_err(|_| "infrastructure: registry map allocation failed")?;
    let actual_buckets = table
        .capacity()
        .max(4)
        .checked_next_power_of_two()
        .ok_or("registry map allocation overflow")?;
    let actual_bytes = actual_buckets
        .checked_mul(entry_bytes)
        .and_then(|n| n.checked_add(16))
        .ok_or("registry map allocation overflow")?;
    budget.retain_bytes(actual_bytes.saturating_sub(bytes))?;
    Ok(())
}
fn schema_fields_storage(fields: &[FieldDecl], budget: &ir::PlanBudget) -> Result<usize, String> {
    let mut bytes = fields
        .len()
        .checked_mul(std::mem::size_of::<FieldDecl>() + 64)
        .ok_or("generic schema allocation overflow")?;
    for field in fields {
        let field_bytes = source_type_storage(&field.ty, budget, 0)?;
        bytes = bytes
            .checked_add(field_bytes)
            .ok_or("generic schema allocation overflow")?;
    }
    Ok(bytes)
}
/// A deliberately narrow const-valid body envelope, not constant evaluation.
/// The ordinary type, region and universal move proofs remain mandatory. Moving
/// the sole argument directly into the result has no discarded value to drop,
/// called function to qualify, or executed effect to pretend is const-valid.
fn validate_const_forwarding(
    names: &NameStore,
    function: &Function,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    budget.charge_work(1)?;
    if function.constness == Constness::Runtime {
        return Ok(());
    }
    let [(parameter, ty)] = function.params.as_slice() else {
        return Err("unsupported: const fn requires exactly one forwarded parameter".into());
    };
    budget.charge_work(1)?;
    let forwarded = match (parameter, function.body.tail.as_deref()) {
        (BindingName::Named(parameter), Some(Expr::Var(path)))
            if path.anchor() == PathAnchor::Relative =>
        {
            path.check(names).map_err(native_adapter::name_diagnostic)?;
            if let Some(name) = path.single() {
                name_scope::ident_eq(names, *parameter, name, LookupClass::Local)?
            } else {
                false
            }
        }
        _ => false,
    };
    if !forwarded || !function.body.statements.is_empty() {
        return Err(
            "unsupported: const fn body must only forward its parameter as the tail expression"
                .into(),
        );
    }
    // Charge and depth-check both source types before their structural equality
    // comparison, including callers constructing public ASTs without parsing.
    budget.charge_work(source_type_storage(ty, budget, 0)?)?;
    budget.charge_work(source_type_storage(&function.result, budget, 0)?)?;
    if !ty.semantic_eq(&function.result, names)? {
        return Err("unsupported: const fn forwarding result must match its parameter type".into());
    }
    Ok(())
}
impl PlanCtx {
    pub(super) fn validate_source_bound_path(
        &self,
        declaration: ir::DeclId,
        path: &TraitPath,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        self.check_budget(budget)?;
        self.registry.validate_source_bound_path(
            self.registry.source_type_context(declaration)?,
            path,
            budget,
        )
    }
    pub fn resolve_trait_path(&self, path: &TraitPath) -> Result<ir::DeclId, String> {
        self.registry.resolve_trait_path(path)
    }
    pub fn resolve_trait_use(
        &self,
        owner: &ir::Ty,
        path: &TraitPath,
        env: &TypeEnv,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<ir::TraitRef, String> {
        self.check_budget(budget)?;
        let structural = match caller {
            ir::FactsCtx::Validated(facts) => {
                if !facts.budget().same_authority(budget)
                    || !Arc::ptr_eq(facts.world(), &self.trait_world()?)
                {
                    return Err("trait use changed validated caller authority".into());
                }
                ir::FactsCtx::Universal(facts.env().as_ref())
            }
            ir::FactsCtx::Universal(scope) => ir::FactsCtx::Universal(scope),
            ir::FactsCtx::Concrete => ir::FactsCtx::Concrete,
        };
        // World validation may call SchemaAdapter::validate_nominal, which
        // needs this same state mutex. Only structural checks with the exact
        // caller binder may execute while source actuals are being interned.
        caller.validate_ty_with_budget(owner, budget)?;
        let application = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "generic plan context poisoned")?;
            state
                .source_trait_application(
                    &self.registry,
                    path,
                    owner,
                    &env.params,
                    structural,
                    budget,
                )?
                .reject_equalities()?
        };
        caller.validate_ty_with_budget(&application.self_ty, budget)?;
        for actual in application.args.iter() {
            budget.charge_work(1)?;
            caller.validate_ty_with_budget(actual, budget)?;
        }
        Ok(application)
    }
    pub fn from_program(program: &Program, budget: &ir::PlanBudget) -> Result<Self, String> {
        preflight_program_clone(program, budget)?;
        Self::from_shared_program(Arc::new(program.clone()), budget)
    }
    pub fn from_shared_program(
        shared: Arc<Program>,
        budget: &ir::PlanBudget,
    ) -> Result<Self, String> {
        let program = shared.as_ref();
        program.validate_name_handles()?;
        program.validate_associated_bound_sources(budget)?;
        // Source constructors retain nominal identity. Generic unit declarations
        // remain outside source-shape admission.
        budget.charge_work(program.structs.len())?;
        for declaration in &program.structs {
            if !declaration.type_params.is_empty() {
                match declaration.shape {
                    StructShape::Unit => {
                        return Err("unsupported: generic unit structs are unsupported".into());
                    }
                    StructShape::Record | StructShape::Tuple => {}
                }
            }
        }
        // Signature construction sees all source declarations, not just demanded
        // instances. Unsupported unused const bodies cannot escape publication.
        for function in &program.functions {
            validate_const_forwarding(program.names.as_ref(), function, budget)?;
        }
        for implementation in &program.impls {
            for item in &implementation.items {
                budget.charge_work(1)?;
                if item.function.constness != Constness::Runtime {
                    return Err("unsupported: const associated functions are unsupported".into());
                }
            }
        }
        for declaration in &program.traits {
            for item in &declaration.items {
                budget.charge_work(1)?;
                if item.signature.constness != Constness::Runtime
                    || item
                        .default
                        .as_ref()
                        .is_some_and(|f| f.constness != Constness::Runtime)
                {
                    return Err("unsupported: const associated functions are unsupported".into());
                }
            }
        }
        let snapshot = ir::SnapshotId::allocate()?;
        let catalog_builder =
            tr::CoreCatalogBuilder::new(snapshot, program.modules.clone(), budget)
                .map_err(|e| e.into_legacy_diagnostic(budget))?;
        let mut import_bytes = 128usize;
        for (module, path) in &program.trait_imports {
            budget.charge_work(1)?;
            import_bytes = import_bytes
                .checked_add(std::mem::size_of_val(module))
                .and_then(|n| n.checked_add(std::mem::size_of_val(path)))
                .and_then(|n| n.checked_add(128))
                .ok_or("trait import allocation overflow")?;
        }
        budget.retain_bytes(import_bytes)?;
        let mut registry = DeclRegistry {
            program: shared.clone(),
            names: program.names.clone(),
            modules: program.modules.clone(),
            symbols: program.symbols.clone(),
            declarations: Vec::new(),
            functions: HashMap::new(),
            nominals: HashMap::new(),
            function_ids: Vec::new(),
            callable_sources: Vec::new(),
            impl_ids: Vec::new(),
            associated_ids: Vec::new(),
            traits: HashMap::new(),
            trait_ids: Vec::new(),
            trait_items: Vec::new(),
            trait_associated_types: Vec::new(),
            associated_type_bounds: HashMap::new(),
            trait_imports: program.trait_imports.clone(),
            function_positions: HashMap::new(),
            function_positions_sealed: false,
        };
        reserve_registry_map(&mut registry.functions, program.functions.len(), budget)?;
        let nominal_count = program
            .structs
            .len()
            .checked_add(program.enums.len())
            .ok_or("registry inventory overflow")?;
        reserve_registry_map(&mut registry.nominals, nominal_count, budget)?;
        reserve_registry_map(
            &mut registry.traits,
            program
                .traits
                .len()
                .checked_add(4)
                .ok_or("registry inventory overflow")?,
            budget,
        )?;
        let mut state = PlanState {
            source_context: None,
            associated_type_rows: Vec::new(),
            associated_type_rhs: HashMap::new(),
            schemas: HashMap::new(),
            region_modes: HashMap::new(),
            source_regions: HashMap::new(),
            functions: HashMap::new(),
            nominals: NominalInterner::default(),
            contexts: HashMap::new(),
            reservations: Vec::new(),
        };
        let assignment_count = program.impls.iter().try_fold(0usize, |n, block| {
            n.checked_add(block.associated_types.len())
                .ok_or("associated assignment inventory overflow")
        })?;
        reserve_registry_map(&mut state.associated_type_rhs, assignment_count, budget)?;
        for entry in catalog_builder.base_declarations() {
            let info = &entry.info;
            budget.charge_work(1)?;
            budget.retain_bytes(1024 + entry.name.len() * 4)?;
            if entry.declaration.index as usize != registry.declarations.len() {
                return Err("catalog prefix identity mismatch".into());
            }
            registry
                .declarations
                .push(catalog_decl_info(entry, budget)?);
            if let Some(schema) = &entry.schema {
                state.functions.insert(entry.declaration, schema.clone());
            }
            state
                .region_modes
                .insert(entry.declaration, info.region_contract_mode);
            if info.kind == ir::DeclKind::Trait {
                let canonical = match entry.origin {
                    tr::DeclOrigin::CoreCatalog {
                        item: tr::CatalogItem::Sized,
                        ..
                    } => BuiltinTrait::Sized,
                    tr::DeclOrigin::CoreCatalog {
                        item: tr::CatalogItem::Clone,
                        ..
                    } => BuiltinTrait::Clone,
                    tr::DeclOrigin::CoreCatalog {
                        item: tr::CatalogItem::Copy,
                        ..
                    } => BuiltinTrait::Copy,
                    tr::DeclOrigin::CoreCatalog {
                        item: tr::CatalogItem::Drop,
                        ..
                    } => BuiltinTrait::Drop,
                    _ => return Err("catalog trait origin mismatch".into()),
                };
                registry
                    .traits
                    .insert(TraitLookupKey::Builtin(canonical), entry.declaration);
            }
        }
        let declarations = program
            .functions
            .len()
            .checked_add(program.structs.len())
            .and_then(|n| n.checked_add(program.enums.len()))
            .and_then(|n| {
                program.traits.iter().try_fold(n, |n, t| {
                    n.checked_add(
                        1 + t.items.len()
                            + t.associated_types.len()
                            + t.items.iter().filter(|i| i.default.is_some()).count(),
                    )
                })
            })
            .and_then(|n| {
                program
                    .impls
                    .iter()
                    .try_fold(n, |n, b| n.checked_add(1 + b.items.len()))
            })
            .ok_or("generic declaration resource overflow")?;
        budget.charge_nodes(declarations)?;
        state.reservations.push(
            budget.reserve_bytes(
                declarations
                    .checked_mul(512)
                    .ok_or("generic declaration resource overflow")?,
            )?,
        );
        let mut source_storage = 0usize;
        for d in &program.structs {
            source_storage = source_storage
                .checked_add(schema_fields_storage(&d.fields, budget)?)
                .ok_or("generic schema allocation overflow")?;
        }
        for d in &program.enums {
            source_storage = source_storage
                .checked_add(
                    d.variants
                        .len()
                        .checked_mul(std::mem::size_of::<VariantDecl>() + 64)
                        .ok_or("generic schema allocation overflow")?,
                )
                .ok_or("generic schema allocation overflow")?;
            for variant in &d.variants {
                let fields_storage = schema_fields_storage(&variant.fields, budget)?;
                source_storage = source_storage
                    .checked_add(std::mem::size_of_val(&variant.name))
                    .and_then(|n| n.checked_add(fields_storage))
                    .ok_or("generic schema allocation overflow")?;
            }
        }
        state.retain(budget, source_storage)?;
        let mut insert = |path: Option<&AstPath>,
                          module: ModuleId,
                          item: DeclItemName,
                          params: &[AstIdent],
                          kind: ir::DeclKind,
                          source_index: usize|
         -> Result<ir::DeclId, String> {
            program.modules.check(module).map_err(module_diagnostic)?;
            let name = if let Some(path) = path {
                registry_diagnostic_copy(
                    path.display(program.names.as_ref())
                        .map_err(native_adapter::name_diagnostic)?
                        .as_str(),
                    budget,
                )?
            } else {
                match item {
                    DeclItemName::Source(name) => registry_diagnostic_copy(
                        program
                            .names
                            .spelling(name)
                            .map_err(native_adapter::name_diagnostic)?,
                        budget,
                    )?,
                    DeclItemName::Impl => registry_diagnostic_copy("impl", budget)?,
                    DeclItemName::Catalog(_) => {
                        return Err("contract: source catalog declaration".into());
                    }
                }
            };
            budget.charge_work(1 + name.len() + params.len())?;
            if params.len() > budget.limits().params {
                return Err("generic parameter resource limit exceeded".into());
            }
            budget.retain_bytes(
                name.len()
                    .checked_mul(2)
                    .and_then(|n| n.checked_add(std::mem::size_of_val(params) + params.len() * 64))
                    .ok_or("generic declaration allocation overflow")?,
            )?;
            let mut seen = std::collections::HashSet::new();
            for parameter in params {
                let key = program
                    .names
                    .key(*parameter, LookupClass::TypeParameter)
                    .map_err(native_adapter::name_diagnostic)?;
                if !seen.insert(key) {
                    return Err("duplicate generic parameter".into());
                }
            }
            let id = ir::DeclId {
                snapshot,
                index: u32::try_from(registry.declarations.len())
                    .map_err(|_| "generic declaration count overflow")?,
            };
            let arity = params
                .len()
                .checked_add(usize::from(kind == ir::DeclKind::Trait))
                .ok_or("generic parameter resource overflow")?;
            if arity > budget.limits().params {
                return Err("generic parameter resource limit exceeded".into());
            }
            let param_env = Arc::new(ir::ParamEnv::new(ir::BinderId { declaration: id }, arity)?);
            if matches!(
                kind,
                ir::DeclKind::Function | ir::DeclKind::Struct | ir::DeclKind::Enum
            ) {
                let path = path.ok_or("contract: missing declaration path")?;
                let namespace = if kind == ir::DeclKind::Function {
                    Namespace::Value
                } else {
                    Namespace::Type
                };
                let key = name_scope::path_key(program.names.as_ref(), path, namespace)?;
                let previous = if kind == ir::DeclKind::Function {
                    registry.functions.insert(key, id)
                } else {
                    registry.nominals.insert(key, id)
                };
                if previous.is_some() {
                    return Err(format!("duplicate declaration {name}"));
                }
            }
            let env = Arc::new(ir::GenericEnv::new(
                std::slice::from_ref(&param_env),
                budget,
            )?);
            registry.declarations.push(DeclInfo {
                receiver: ReceiverKind::None,
                owner: None,
                id,
                kind,
                name,
                path: DeclSemanticPath {
                    module,
                    item,
                    kind,
                    associated: None,
                },
                source_index,
                param_env,
                env,
                visibility: ir::Visibility::Public,
                type_params: params.to_vec().into(),
            });
            Ok(id)
        };
        for (index, d) in program.structs.iter().enumerate() {
            let item = *d
                .name
                .segments()
                .last()
                .ok_or("contract: empty struct path")?;
            let id = insert(
                Some(&d.name),
                d.module,
                DeclItemName::Source(item),
                &d.type_params,
                ir::DeclKind::Struct,
                index,
            )?;
            clone_visibility(&d.visibility, budget)?;
            state
                .schemas
                .insert(id, Arc::new(NominalSchema::Struct(d.clone())));
        }
        for (index, d) in program.enums.iter().enumerate() {
            let item = *d
                .name
                .segments()
                .last()
                .ok_or("contract: empty enum path")?;
            let id = insert(
                Some(&d.name),
                d.module,
                DeclItemName::Source(item),
                &d.type_params,
                ir::DeclKind::Enum,
                index,
            )?;
            clone_visibility(&d.visibility, budget)?;
            state
                .schemas
                .insert(id, Arc::new(NominalSchema::Enum(d.clone())));
        }
        for (index, f) in program.functions.iter().enumerate() {
            let item = *f
                .name
                .segments()
                .last()
                .ok_or("contract: empty function path")?;
            let id = insert(
                Some(&f.name),
                f.module,
                DeclItemName::Source(item),
                &f.type_params,
                ir::DeclKind::Function,
                index,
            )?;
            registry.function_ids.push(id);
            registry.callable_sources.push(CallableSource::Free(index));
        }
        for (index, declaration) in program.traits.iter().enumerate() {
            let item = *declaration
                .name
                .segments()
                .last()
                .ok_or("contract: empty trait path")?;
            let id = insert(
                Some(&declaration.name),
                declaration.module,
                DeclItemName::Source(item),
                &declaration.type_params,
                ir::DeclKind::Trait,
                index,
            )?;
            registry.traits.insert(
                TraitLookupKey::Source(name_scope::path_key(
                    program.names.as_ref(),
                    &declaration.name,
                    Namespace::Type,
                )?),
                id,
            );
            registry.trait_ids.push(id);
            let mut items = Vec::new();
            for (item, method) in declaration.items.iter().enumerate() {
                items.push(insert(
                    None,
                    declaration.module,
                    DeclItemName::Source(method.signature.name),
                    &[],
                    ir::DeclKind::TraitItem,
                    item,
                )?);
                if method.default.is_some() {
                    let id = insert(
                        None,
                        declaration.module,
                        DeclItemName::Source(method.signature.name),
                        &[],
                        ir::DeclKind::AssociatedFunction,
                        item,
                    )?;
                    registry.function_ids.push(id);
                    registry
                        .callable_sources
                        .push(CallableSource::TraitDefault {
                            trait_index: index,
                            item,
                        });
                }
            }
            let mut associated_types = Vec::new();
            budget.charge_work(declaration.associated_types.len())?;
            budget.retain_bytes(
                declaration
                    .associated_types
                    .len()
                    .checked_mul(std::mem::size_of::<ir::DeclId>())
                    .ok_or("associated declaration storage overflow")?,
            )?;
            associated_types
                .try_reserve_exact(declaration.associated_types.len())
                .map_err(|_| "associated declaration allocation failed")?;
            for (item, associated) in declaration.associated_types.iter().enumerate() {
                for previous in &declaration.associated_types[..item] {
                    budget.charge_work(1)?;
                    if name_scope::ident_eq(
                        program.names.as_ref(),
                        previous.name,
                        associated.name,
                        LookupClass::Member,
                    )? {
                        return Err("duplicate trait associated type declaration".into());
                    }
                }
                associated_types.push(insert(
                    None,
                    declaration.module,
                    DeclItemName::Source(associated.name),
                    &[],
                    ir::DeclKind::AssociatedType,
                    item,
                )?);
            }
            registry.trait_associated_types.push(associated_types);
            registry.trait_items.push(items);
        }
        for (implementation, block) in program.impls.iter().enumerate() {
            let kind = if block.trait_path.is_some() {
                ir::DeclKind::TraitImpl
            } else {
                ir::DeclKind::Impl
            };
            let impl_id = insert(
                None,
                block.module,
                DeclItemName::Impl,
                &block.type_params,
                kind,
                implementation,
            )?;
            registry.impl_ids.push(impl_id);
            let mut ids = Vec::new();
            for (item, method) in block.items.iter().enumerate() {
                let name = method
                    .function
                    .name
                    .single()
                    .ok_or("contract: associated member path")?;
                let id = insert(
                    None,
                    block.module,
                    DeclItemName::Source(name),
                    &method.function.type_params,
                    ir::DeclKind::AssociatedFunction,
                    item,
                )?;
                registry.function_ids.push(id);
                registry.callable_sources.push(CallableSource::Associated {
                    implementation,
                    item,
                });
                ids.push(id);
            }
            registry.associated_ids.push(ids);
        }
        for entry in catalog_builder.base_declarations() {
            if entry.info.kind != ir::DeclKind::Trait {
                continue;
            }
            registry.trait_ids.push(entry.declaration);
            registry.trait_associated_types.push(Vec::new());
            registry.trait_items.push(
                catalog_builder
                    .base_declarations()
                    .iter()
                    .filter(|item| {
                        item.info.owner == Some(entry.declaration)
                            && item.info.kind == ir::DeclKind::TraitItem
                    })
                    .map(|item| item.declaration)
                    .collect(),
            );
        }
        for declaration in &program.structs {
            clone_visibility(&declaration.visibility, budget)?;
            let id = registry.nominals[&name_scope::path_key(
                program.names.as_ref(),
                &declaration.name,
                Namespace::Type,
            )?];
            registry.declarations[id.index as usize].visibility = declaration.visibility.clone();
        }
        for declaration in &program.enums {
            clone_visibility(&declaration.visibility, budget)?;
            let id = registry.nominals[&name_scope::path_key(
                program.names.as_ref(),
                &declaration.name,
                Namespace::Type,
            )?];
            registry.declarations[id.index as usize].visibility = declaration.visibility.clone();
        }
        for (index, function) in program.functions.iter().enumerate() {
            clone_visibility(&function.visibility, budget)?;
            let id = registry.function_ids[index];
            registry.declarations[id.index as usize].visibility = function.visibility.clone();
        }
        for (index, declaration) in program.traits.iter().enumerate() {
            let id = registry.trait_ids[index];
            clone_visibility(&declaration.visibility, budget)?;
            registry.declarations[id.index as usize].visibility = declaration.visibility.clone();
            let trait_env = registry.declarations[id.index as usize].param_env.clone();
            let inherited_env = registry.declarations[id.index as usize].env.clone();
            for item in &registry.trait_associated_types[index] {
                clone_visibility(&declaration.visibility, budget)?;
                let info = &mut registry.declarations[item.index as usize];
                info.owner = Some(id);
                info.visibility = declaration.visibility.clone();
                info.env = inherited_env.clone();
            }
            for (ii, item) in registry.trait_items[index].iter().enumerate() {
                clone_visibility(&declaration.visibility, budget)?;
                let info = &mut registry.declarations[item.index as usize];
                info.receiver = declaration.items[ii].receiver;
                info.owner = Some(id);
                info.visibility = declaration.visibility.clone();
                info.env = Arc::new(ir::GenericEnv::new(
                    &[trait_env.clone(), info.param_env.clone()],
                    budget,
                )?);
            }
        }
        for (implementation, block) in program.impls.iter().enumerate() {
            let id = registry.impl_ids[implementation];
            registry.declarations[id.index as usize].path.module = block.module;
        }
        for (index, source) in registry.callable_sources.iter().enumerate() {
            let CallableSource::TraitDefault { trait_index, item } = *source else {
                continue;
            };
            let owner = registry.trait_ids[trait_index];
            let trait_env = registry.declarations[owner.index as usize]
                .param_env
                .clone();
            let module = registry.declarations[owner.index as usize].path.module;
            let id = registry.function_ids[index];
            let info = &mut registry.declarations[id.index as usize];
            info.owner = Some(owner);
            info.receiver = program.traits[trait_index].items[item].receiver;
            clone_visibility(&program.traits[trait_index].visibility, budget)?;
            info.visibility = program.traits[trait_index].visibility.clone();
            info.env = Arc::new(ir::GenericEnv::new(
                &[trait_env.clone(), info.param_env.clone()],
                budget,
            )?);
            state.contexts.insert(
                id,
                CallableContext {
                    module,
                    self_ty: Some(ir::Ty::Param(ir::ParamId {
                        owner: trait_env.binder(),
                        index: 0,
                    })),
                    associated_owner: None,
                    receiver: info.receiver,
                },
            );
        }
        for (implementation, ids) in registry.associated_ids.iter().enumerate() {
            let impl_env = registry.declarations[registry.impl_ids[implementation].index as usize]
                .param_env
                .clone();
            for id in ids {
                let info = &mut registry.declarations[id.index as usize];
                info.env = Arc::new(ir::GenericEnv::new(
                    &[impl_env.clone(), info.param_env.clone()],
                    budget,
                )?);
                info.receiver = program.impls[implementation].items[info.source_index].receiver;
                info.owner = Some(registry.impl_ids[implementation]);
                clone_visibility(
                    &program.impls[implementation].items[info.source_index].visibility,
                    budget,
                )?;
                info.visibility = program.impls[implementation].items[info.source_index]
                    .visibility
                    .clone();
            }
        }
        let bound_count =
            registry
                .trait_associated_types
                .iter()
                .try_fold(0usize, |count, members| {
                    count
                        .checked_add(members.len())
                        .ok_or("associated bound inventory overflow")
                })?;
        reserve_registry_map(&mut registry.associated_type_bounds, bound_count, budget)?;
        let mut source_index = 0usize;
        for (trait_index, owner) in program.traits.iter().enumerate() {
            for (member_index, member) in owner.associated_types.iter().enumerate() {
                let declaration = registry.trait_associated_types[trait_index][member_index];
                budget.charge_work(member.bounds.len().saturating_add(1))?;
                let mut ids = Vec::new();
                budget.retain_bytes(
                    member
                        .bounds
                        .len()
                        .checked_mul(2 * std::mem::size_of::<ir::DeclId>())
                        .and_then(|bytes| bytes.checked_add(128))
                        .ok_or("associated bound storage overflow")?,
                )?;
                ids.try_reserve_exact(member.bounds.len())
                    .map_err(|_| "associated bound allocation failed")?;
                if !program.associated_bound_sources.is_empty() {
                    let source = program
                        .associated_bound_sources
                        .get(source_index)
                        .ok_or("missing associated bound source")?;
                    if source.trait_index != trait_index || source.member_index != member_index {
                        return Err("associated bound source order changed".into());
                    }
                    for (_, kind) in source.bounds.iter() {
                        let id = registry
                            .traits
                            .get(&TraitLookupKey::Builtin(*kind))
                            .copied()
                            .ok_or("missing associated bound catalog identity")?;
                        ids.push(id);
                    }
                }
                registry.associated_type_bounds.insert(
                    declaration,
                    FrozenAssociatedBounds {
                        ids: ids.into(),
                        source_index: (!program.associated_bound_sources.is_empty())
                            .then_some(source_index),
                    },
                );
                source_index += 1;
            }
        }
        let mut registry = Arc::new(registry);
        // Every schema, including unused generic declarations, gets authoritative symbolic arguments.
        for info in &registry.declarations {
            if !matches!(info.kind, ir::DeclKind::Struct | ir::DeclKind::Enum) {
                continue;
            }
            let args = symbolic_args(&info.param_env, budget)?;
            let caller = if args.is_empty() {
                ir::FactsCtx::Concrete
            } else {
                ir::FactsCtx::Universal(&info.env)
            };
            state.intern(&registry, info.id, &args, caller, budget, 0)?;
            let schema = state
                .schemas
                .get(&info.id)
                .ok_or("missing nominal schema")?;
            let mut used = std::collections::HashSet::new();
            let fields: Vec<_> = match schema.as_ref() {
                NominalSchema::Struct(d) => d.fields.iter().collect(),
                NominalSchema::Enum(d) => d.variants.iter().flat_map(|v| v.fields.iter()).collect(),
            };
            for field in fields {
                collect_source_params(program.names.as_ref(), &field.ty, &mut used, budget, 0)?;
            }
            if info
                .type_params
                .iter()
                .map(|p| {
                    program
                        .names
                        .key(*p, LookupClass::TypeParameter)
                        .map_err(native_adapter::name_diagnostic)
                })
                .collect::<Result<Vec<_>, _>>()?
                .iter()
                .any(|p| !used.contains(p))
            {
                return Err(format!("unused nominal type parameter in {}", info.name));
            }
        }
        for (implementation, block) in program.impls.iter().enumerate() {
            let impl_id = registry.impl_ids[implementation];
            let info = registry.declaration(impl_id)?;
            let mut bindings = generic_bindings(&registry, &info.env, budget)?;
            let caller = if info.env.is_generic() {
                ir::FactsCtx::Universal(&info.env)
            } else {
                ir::FactsCtx::Concrete
            };
            let self_ty = state.source_ty(
                &registry,
                &block.self_ty,
                &bindings,
                caller,
                budget,
                0,
                false,
            )?;
            if raw_type_has_projection(&self_ty, budget, 0)? {
                return Err("unsupported: projection-bearing impl heads".into());
            }
            let nominal = match &self_ty {
                ir::Ty::Struct(t) => t.def().source_decl,
                ir::Ty::Enum(t) => t.def().source_decl,
                _ => return Err("inherent impl requires a local nominal self type".into()),
            };
            let mut used = std::collections::HashSet::new();
            collect_source_params(program.names.as_ref(), &block.self_ty, &mut used, budget, 0)?;
            if let Some(path) = &block.trait_path {
                bindings.insert(GenericBindingKey::SelfType, self_ty.clone());
                state
                    .source_trait_application(&registry, path, &self_ty, &bindings, caller, budget)?
                    .reject_equalities()?;
                if let TraitPath::Source(path) = path {
                    for segment in &path.segments {
                        for actual in &segment.args {
                            collect_source_params(
                                program.names.as_ref(),
                                actual,
                                &mut used,
                                budget,
                                0,
                            )?;
                        }
                    }
                }
            }
            if block
                .type_params
                .iter()
                .map(|p| {
                    program
                        .names
                        .key(*p, LookupClass::TypeParameter)
                        .map_err(native_adapter::name_diagnostic)
                })
                .collect::<Result<Vec<_>, _>>()?
                .iter()
                .any(|p| !used.contains(p))
            {
                return Err("unconstrained impl parameter".into());
            }
            state.source_context = Some(registry.source_type_context(impl_id)?);
            bindings.insert(GenericBindingKey::SelfType, self_ty.clone());
            for assignment in &block.associated_types {
                budget.charge_work(source_type_storage(&assignment.value, budget, 0)?)?;
                let rhs = state.source_ty(
                    &registry,
                    &assignment.value,
                    &bindings,
                    caller,
                    budget,
                    0,
                    false,
                )?;
                let trait_decl = block
                    .trait_path
                    .as_ref()
                    .ok_or_else(|| "unsupported: inherent associated types".to_owned())
                    .and_then(|path| registry.resolve_trait_path(path))?;
                let associated =
                    registry.associated_type_item(trait_decl, assignment.name, budget)?;
                budget.retain_bytes(std::mem::size_of::<tr::AssociatedTypeValue>() + 256)?;
                state
                    .associated_type_rows
                    .try_reserve_exact(1)
                    .map_err(|_| "associated assignment allocation failed")?;
                state.associated_type_rows.push(tr::AssociatedTypeValue {
                    implementation: impl_id,
                    associated,
                    rhs: rhs.clone(),
                });
                // Keep every source row. A duplicate must never overwrite the raw callback authority.
                state
                    .associated_type_rhs
                    .entry((impl_id, associated))
                    .or_insert(rhs);
            }
            state.source_context = None;
            let ids = registry.associated_ids[implementation].clone();
            for (item, id) in ids.iter().enumerate() {
                let method = &block.items[item];
                let owner = AssociatedSemanticOwner {
                    trait_decl: block
                        .trait_path
                        .as_ref()
                        .map(|p| registry.resolve_trait_path(p))
                        .transpose()?,
                    trait_item: if let Some(path) = &block.trait_path {
                        let tid = registry.resolve_trait_path(path)?;
                        let mut required = None;
                        for (ti, trait_id) in registry.trait_ids.iter().enumerate() {
                            budget.charge_work(1)?;
                            if *trait_id == tid {
                                for item_id in &registry.trait_items[ti] {
                                    let item_name =
                                        &registry.declarations[item_id.index as usize].path.item;
                                    budget.charge_work(1)?;
                                    let method_name = method
                                        .function
                                        .name
                                        .single()
                                        .ok_or("contract: associated member path")?;
                                    if registry.source_member_equal(*item_name, method_name)? {
                                        required = Some(*item_id);
                                        break;
                                    }
                                }
                                break;
                            }
                        }
                        Some(required.ok_or("extra trait impl item")?)
                    } else {
                        None
                    },
                    nominal,
                    impl_decl: impl_id,
                    self_pattern: self_ty.clone(),
                    receiver: method.receiver,
                };
                Arc::get_mut(&mut registry)
                    .ok_or("registry unexpectedly shared before seal")?
                    .declarations[id.index as usize]
                    .path
                    .associated = Some(owner);
                budget.retain_bytes(512)?;
                state.contexts.insert(
                    *id,
                    CallableContext {
                        module: block.module,
                        self_ty: Some(self_ty.clone()),
                        associated_owner: Some(impl_id),
                        receiver: method.receiver,
                    },
                );
            }
        }
        for (index, id) in registry.function_ids.iter().copied().enumerate() {
            let f = match registry.callable_sources[index] {
                CallableSource::Free(i) => &program.functions[i],
                CallableSource::Associated {
                    implementation,
                    item,
                } => &program.impls[implementation].items[item].function,
                CallableSource::TraitDefault { trait_index, item } => program.traits[trait_index]
                    .items[item]
                    .default
                    .as_ref()
                    .ok_or("missing source default body")?,
            };
            let info = registry.declaration(id)?;
            state.source_context = Some(registry.source_type_context(id)?);
            let mut bindings = generic_bindings(&registry, &info.env, budget)?;
            let context = state
                .contexts
                .entry(id)
                .or_insert_with(|| CallableContext {
                    module: info.path.module,
                    self_ty: None,
                    associated_owner: None,
                    receiver: ReceiverKind::None,
                })
                .clone();
            if let Some(ty) = &context.self_ty {
                bindings.insert(GenericBindingKey::SelfType, ty.clone());
            }
            let caller = if !info.env.is_generic() {
                ir::FactsCtx::Concrete
            } else {
                ir::FactsCtx::Universal(&info.env)
            };
            let mut names = std::collections::HashSet::new();
            for (name, _) in &f.params {
                if let Some(key) = name.checked_key(program.names.as_ref())?
                    && !names.insert(key)
                {
                    return Err("duplicate parameter".into());
                }
            }
            let mut params = f
                .params
                .iter()
                .map(|(_, ty)| state.source_ty(&registry, ty, &bindings, caller, budget, 0, true))
                .collect::<Result<Vec<_>, _>>()?;
            if context.receiver != ReceiverKind::None {
                params.insert(
                    0,
                    receiver_ty(
                        context.receiver,
                        context
                            .self_ty
                            .clone()
                            .ok_or("receiver without Self type")?,
                    ),
                );
            }
            let result =
                state.source_ty(&registry, &f.result, &bindings, caller, budget, 0, true)?;
            reject_slice_result(&result)?;
            if info.env.is_generic() {
                for ty in params.iter().chain(std::iter::once(&result)) {
                    if !raw_type_has_projection(ty, budget, 0)? {
                        validate_root_carrier(ty, caller, budget)?;
                    }
                }
            }
            let region_mode = source_region_mode(
                &f.regions,
                &f.params,
                &f.result,
                matches!(registry.callable_sources[index], CallableSource::Free(_)),
                budget,
            )?;
            state.retain(budget, 128)?;
            state.region_modes.insert(id, region_mode);
            let signature = ir::Signature {
                ref_contract: canonical_ref_contract(
                    (program.names.as_ref(), budget),
                    &f.regions,
                    &f.params,
                    &f.result,
                    (context.receiver, region_mode),
                    &params,
                    &result,
                )?,
                params,
                result,
            };
            state.source_regions.insert(
                id,
                original_source_regions(
                    (program.names.as_ref(), budget),
                    id,
                    &f.regions,
                    &f.params,
                    &f.result,
                    &signature.ref_contract,
                    context.receiver,
                )?,
            );
            let mut rows = Vec::new();
            append_implicit_sized(&registry, &info.env, &mut rows, budget)?;
            if let CallableSource::Associated { implementation, .. } =
                registry.callable_sources[index]
            {
                append_source_bounds(
                    &mut state,
                    &registry,
                    &bindings,
                    &program.impls[implementation].bounds,
                    &mut rows,
                    caller,
                    budget,
                )?;
                if let Some(path) = &program.impls[implementation].trait_path {
                    let trait_ref = state
                        .source_trait_application(
                            &registry,
                            path,
                            context
                                .self_ty
                                .as_ref()
                                .ok_or("trait impl item without Self")?,
                            &bindings,
                            caller,
                            budget,
                        )?
                        .reject_equalities()?;
                    if rows.len() >= tr::MAX_PREDICATES {
                        return Err("predicate environment limit exceeded".into());
                    }
                    budget.retain_bytes(std::mem::size_of::<ir::Predicate>())?;
                    let old_capacity = rows.capacity();
                    rows.try_reserve_exact(1).map_err(|_| {
                        super::source::SourceError::from_token(
                            super::token_session::TokenError::Budget(
                                registry.names.budget().poison_allocation(),
                            ),
                        )
                        .to_string()
                    })?;
                    budget.retain_bytes(
                        rows.capacity()
                            .saturating_sub(old_capacity)
                            .saturating_sub(1)
                            .checked_mul(std::mem::size_of::<ir::Predicate>())
                            .ok_or("trait impl item predicate capacity overflow")?,
                    )?;
                    rows.push(ir::Predicate::Implements(trait_ref));
                }
            }
            append_source_bounds(
                &mut state, &registry, &bindings, &f.bounds, &mut rows, caller, budget,
            )?;
            if let CallableSource::TraitDefault { trait_index, .. } =
                registry.callable_sources[index]
            {
                let declaration = registry.trait_ids[trait_index];
                let owner = registry.declaration(declaration)?;
                append_source_bounds(
                    &mut state,
                    &registry,
                    &bindings,
                    &program.traits[trait_index].bounds,
                    &mut rows,
                    caller,
                    budget,
                )?;
                let mut args = symbolic_args(&owner.param_env, budget)?;
                args.remove(0);
                budget.retain_bytes(256 + args.capacity() * std::mem::size_of::<ir::Ty>())?;
                rows.push(ir::Predicate::Implements(ir::TraitRef {
                    declaration,
                    self_ty: context.self_ty.clone().ok_or("default without Self")?,
                    args: args.into(),
                }));
            }
            state.functions.insert(
                id,
                tr::GenericSchema {
                    env: info.env.clone(),
                    signature,
                    predicates: rows.into(),
                },
            );
        }
        for (ti, t) in program.traits.iter().enumerate() {
            let tid = registry.trait_ids[ti];
            let self_param = ir::ParamId {
                owner: registry.declaration(tid)?.param_env.binder(),
                index: 0,
            };
            for (ii, item) in t.items.iter().enumerate() {
                let id = registry.trait_items[ti][ii];
                let info = registry.declaration(id)?;
                state.source_context = Some(registry.source_type_context(id)?);
                let mut bindings = generic_bindings(&registry, &info.env, budget)?;
                bindings.insert(GenericBindingKey::SelfType, ir::Ty::Param(self_param));
                let caller = ir::FactsCtx::Universal(&info.env);
                let h = &item.signature;
                budget.retain_bytes(
                    h.params
                        .len()
                        .checked_add(1)
                        .and_then(|n| n.checked_mul(128))
                        .ok_or("trait signature allocation overflow")?,
                )?;
                let mut params = h
                    .params
                    .iter()
                    .map(|(_, ty)| {
                        state.source_ty(&registry, ty, &bindings, caller, budget, 0, true)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if item.receiver != ReceiverKind::None {
                    params.insert(0, receiver_ty(item.receiver, ir::Ty::Param(self_param)));
                }
                let result =
                    state.source_ty(&registry, &h.result, &bindings, caller, budget, 0, true)?;
                reject_slice_result(&result)?;
                let ref_contract = canonical_ref_contract(
                    (program.names.as_ref(), budget),
                    &h.regions,
                    &h.params,
                    &h.result,
                    (item.receiver, tr::RegionContractMode::Positioned),
                    &params,
                    &result,
                )?;
                state.source_regions.insert(
                    id,
                    original_source_regions(
                        (program.names.as_ref(), budget),
                        id,
                        &h.regions,
                        &h.params,
                        &h.result,
                        &ref_contract,
                        item.receiver,
                    )?,
                );
                let mut rows = Vec::new();
                append_implicit_sized(&registry, &info.env, &mut rows, budget)?;
                append_source_bounds(
                    &mut state, &registry, &bindings, &t.bounds, &mut rows, caller, budget,
                )?;
                append_source_bounds(
                    &mut state, &registry, &bindings, &h.bounds, &mut rows, caller, budget,
                )?;
                let mut args = symbolic_args(&registry.declaration(tid)?.param_env, budget)?;
                args.remove(0);
                budget.retain_bytes(256 + args.capacity() * std::mem::size_of::<ir::Ty>())?;
                rows.push(ir::Predicate::Implements(ir::TraitRef {
                    declaration: tid,
                    self_ty: ir::Ty::Param(self_param),
                    args: args.into(),
                }));
                state.functions.insert(
                    id,
                    tr::GenericSchema {
                        env: info.env.clone(),
                        signature: ir::Signature {
                            ref_contract,
                            params,
                            result,
                        },
                        predicates: rows.into(),
                    },
                );
            }
        }
        state.source_context = None;
        let root = program.modules.source_root().map_err(module_diagnostic)?;
        let mut has_main = false;
        for f in &program.functions {
            if f.module == root
                && f.name.segments().len() == 1
                && f.type_params.is_empty()
                && f.params.is_empty()
                && matches!(f.result, Type::Unit)
            {
                has_main |= program.symbols.matches(
                    program.names.as_ref(),
                    f.name.segments()[0],
                    super::semantic_name::BuiltinAtom::EntryMain,
                )?;
            }
        }
        if !has_main {
            return Err("main must exist, take no arguments and return ()".into());
        }
        let source_end = u32::try_from(registry.declarations.len())
            .map_err(|_| "source declaration count overflow")?;
        let inventory = recipes::inventory(program, budget)?;
        let catalog = catalog_builder
            .finish(source_end, inventory, budget)
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
        let registry_mut =
            Arc::get_mut(&mut registry).ok_or("registry shared before catalog seal")?;
        for entry in catalog.declarations() {
            if entry.declaration.index < source_end {
                continue;
            }
            budget.charge_work(1)?;
            budget.retain_bytes(1024 + entry.name.len() * 4)?;
            if entry.declaration.index as usize != registry_mut.declarations.len() {
                return Err("recipe suffix identity mismatch".into());
            }
            registry_mut
                .declarations
                .push(catalog_decl_info(entry, budget)?);
            if let Some(schema) = &entry.schema {
                state.functions.insert(entry.declaration, schema.clone());
            }
            state
                .region_modes
                .insert(entry.declaration, entry.info.region_contract_mode);
            if matches!(
                entry.callable_provider,
                Some(CallableProvider::Recipe { .. })
            ) {
                registry_mut.function_ids.push(entry.declaration);
            }
        }
        // Seal the callable position index after the final recipe suffix.
        let mut positions = HashMap::new();
        positions
            .try_reserve(registry_mut.function_ids.len())
            .map_err(|_| "callable position index allocation limit exceeded")?;
        budget.charge_work(registry_mut.function_ids.len())?;
        for (index, id) in registry_mut.function_ids.iter().enumerate() {
            positions.entry(*id).or_insert(index);
        }
        registry_mut.function_positions = positions;
        registry_mut.function_positions_sealed = true;
        let mut plan = Self {
            program: shared.clone(),
            catalog,
            world: None,
            validated: Arc::new(HashMap::new()),
            registry,
            state: Arc::new(Mutex::new(state)),
            shared_env: None,
            closed_roots: Arc::from([]),
            budget: budget.clone(),
        };
        plan.validate_inherent_overlaps(budget)?;
        plan.publish_trait_world(program)?;
        plan.closed_roots = plan.compute_closed_roots()?;
        Ok(plan)
    }
    fn compute_closed_roots(&self) -> Result<Arc<[usize]>, String> {
        let mut roots = Vec::new();
        for index in 0..self.callable_count() {
            self.budget.charge_work(1)?;
            let declaration = self.callable_decl(index)?;
            if matches!(
                self.callable_provider(declaration)?,
                CallableProvider::Source { .. }
            ) && !self.callable_env(declaration)?.is_generic()
            {
                roots.push(index);
            }
        }
        Ok(roots.into())
    }
    /// Nongeneric Source callable indices in callable order (plan roots).
    pub fn closed_roots(&self) -> &[usize] {
        &self.closed_roots
    }
    fn publish_trait_world(&mut self, program: &Program) -> Result<(), String> {
        let budget = &self.budget;
        budget.retain_bytes(
            self.registry
                .declarations
                .len()
                .checked_mul(1024)
                .ok_or("trait headers allocation overflow")?,
        )?;
        let mut headers = tr::HeaderSet::default();
        let mut traits = self.catalog.headers().traits.to_vec();
        let mut items = self.catalog.headers().items.to_vec();
        let mut associated_types = self.catalog.headers().associated_types.to_vec();
        let mut impls = self.catalog.headers().impls.to_vec();
        let mut callables = self.catalog.headers().callables.to_vec();
        for (ti, t) in program.traits.iter().enumerate() {
            let id = self.registry.trait_ids[ti];
            let info = self.declaration(id)?;
            let bindings = generic_bindings(&self.registry, &info.env, budget)?;
            let caller = ir::FactsCtx::Universal(&info.env);
            let self_ty = ir::Ty::Param(ir::ParamId {
                owner: info.param_env.binder(),
                index: 0,
            });
            let supertraits = {
                let mut state = self
                    .state
                    .lock()
                    .map_err(|_| "generic plan context poisoned")?;
                state.source_context = Some(self.registry.source_type_context(id)?);
                let mut rows = Vec::new();
                append_implicit_sized(&self.registry, &info.env, &mut rows, budget)?;
                append_source_bounds(
                    &mut state,
                    &self.registry,
                    &bindings,
                    &t.bounds,
                    &mut rows,
                    caller,
                    budget,
                )?;
                for path in &t.supertraits {
                    if rows.len() >= tr::MAX_PREDICATES {
                        return Err("predicate environment limit exceeded".into());
                    }
                    let application = state
                        .source_trait_application(
                            &self.registry,
                            path,
                            &self_ty,
                            &bindings,
                            caller,
                            budget,
                        )?
                        .reject_equalities()?;
                    budget.retain_bytes(256)?;
                    rows.try_reserve_exact(1).map_err(|_| {
                        super::source::SourceError::from_token(
                            super::token_session::TokenError::Budget(
                                self.registry.names.budget().poison_allocation(),
                            ),
                        )
                        .to_string()
                    })?;
                    rows.push(ir::Predicate::Implements(application));
                }
                budget.charge_work(rows.len())?;
                budget.retain_bytes(
                    rows.len()
                        .checked_mul(2 * std::mem::size_of::<ir::TraitRef>())
                        .and_then(|bytes| bytes.checked_add(2 * std::mem::size_of::<usize>()))
                        .ok_or("trait implication storage overflow")?,
                )?;
                let mut applications = Vec::new();
                applications.try_reserve_exact(rows.len()).map_err(|_| {
                    super::source::SourceError::from_token(
                        super::token_session::TokenError::Budget(
                            self.registry.names.budget().poison_allocation(),
                        ),
                    )
                    .to_string()
                })?;
                budget.retain_bytes(
                    applications
                        .capacity()
                        .saturating_sub(rows.len())
                        .checked_mul(std::mem::size_of::<ir::TraitRef>())
                        .ok_or("trait implication capacity overflow")?,
                )?;
                for row in rows {
                    match row {
                        ir::Predicate::Implements(application) => applications.push(application),
                        ir::Predicate::ProjectionEq { .. } => {
                            return Err(
                                "unsupported: associated equality in trait implications".into()
                            );
                        }
                    }
                }
                applications
            };
            traits.push(tr::TraitHeader {
                declaration: id,
                self_param: ir::ParamId {
                    owner: info.param_env.binder(),
                    index: 0,
                },
                env: info.env.clone(),
                items: self.registry.trait_items[ti].clone().into(),
                associated_types: self.registry.trait_associated_types[ti].clone().into(),
                supertraits: supertraits.into(),
            });
            for declaration in &self.registry.trait_associated_types[ti] {
                let info = self.declaration(*declaration)?;
                associated_types.push(tr::AssociatedTypeHeader {
                    declaration: *declaration,
                    trait_decl: id,
                    env: info.env.clone(),
                    bounds: self
                        .registry
                        .associated_type_bounds
                        .get(declaration)
                        .map(|bounds| bounds.ids.clone())
                        .ok_or("missing authoritative associated declaration bounds")?,
                });
            }
            for (ii, item) in t.items.iter().enumerate() {
                let declaration = self.registry.trait_items[ti][ii];
                budget.charge_work(self.registry.callable_sources.len())?;
                items.push(tr::TraitItemHeader {
                    declaration,
                    trait_decl: id,
                    receiver: receiver_mode(item.receiver),
                    schema: self.raw_schema(declaration)?,
                    default: self.registry.callable_sources.iter().enumerate().find_map(
                        |(index, source)| match source {
                            CallableSource::TraitDefault { trait_index, item }
                                if *trait_index == ti && *item == ii =>
                            {
                                Some(self.registry.function_ids[index])
                            }
                            _ => None,
                        },
                    ),
                });
            }
        }
        for (bi, block) in program.impls.iter().enumerate() {
            let Some(path) = &block.trait_path else {
                continue;
            };
            let declaration = self.registry.impl_ids[bi];
            let info = self.declaration(declaration)?;
            let mut bindings = generic_bindings(&self.registry, &info.env, budget)?;
            let caller = if info.env.is_generic() {
                ir::FactsCtx::Universal(&info.env)
            } else {
                ir::FactsCtx::Concrete
            };
            let self_ty = self.source_ty(&block.self_ty, &bindings, caller, budget)?;
            bindings.insert(GenericBindingKey::SelfType, self_ty.clone());
            let mut predicates = Vec::new();
            append_implicit_sized(&self.registry, &info.env, &mut predicates, budget)?;
            let trait_ref = {
                let mut state = self
                    .state
                    .lock()
                    .map_err(|_| "generic plan context poisoned")?;
                state.source_context = Some(self.registry.source_type_context(declaration)?);
                append_source_bounds(
                    &mut state,
                    &self.registry,
                    &bindings,
                    &block.bounds,
                    &mut predicates,
                    caller,
                    budget,
                )?;
                state
                    .source_trait_application(
                        &self.registry,
                        path,
                        &self_ty,
                        &bindings,
                        caller,
                        budget,
                    )?
                    .reject_equalities()?
            };
            let mut pairs = Vec::new();
            for id in &self.registry.associated_ids[bi] {
                budget.charge_work(1)?;
                let owner = self
                    .declaration(*id)?
                    .path
                    .associated
                    .as_ref()
                    .ok_or("missing associated owner")?;
                pairs.push((owner.trait_item.ok_or("missing required trait item")?, *id));
            }
            impls.push(tr::ImplHeader {
                declaration,
                trait_ref,
                schema_env: info.env.clone(),
                predicates: predicates.into(),
                items: pairs.into(),
                associated_types: {
                    let state = self
                        .state
                        .lock()
                        .map_err(|_| "generic plan context poisoned")?;
                    let mut rows = Vec::new();
                    for row in &state.associated_type_rows {
                        budget.charge_work(1)?;
                        if row.implementation == declaration {
                            budget.retain_bytes(std::mem::size_of::<tr::AssociatedTypeValue>())?;
                            rows.try_reserve_exact(1)
                                .map_err(|_| "associated header allocation failed")?;
                            rows.push(row.clone());
                        }
                    }
                    rows.into()
                },
            });
        }
        for id in &self.registry.function_ids {
            budget.charge_work(1)?;
            if self.catalog.declaration(*id).is_some() {
                continue;
            }
            callables.push(tr::CallableHeader {
                declaration: *id,
                schema: self.raw_schema(*id)?,
            });
        }
        headers.traits = traits.into();
        headers.items = items.into();
        headers.associated_types = associated_types.into();
        headers.impls = impls.into();
        headers.callables = callables.into();
        let adapter = Arc::new(SchemaAdapter {
            program: self.program.clone(),
            catalog: self.catalog.clone(),
            registry: self.registry.clone(),
            state: self.state.clone(),
            budget: budget.clone(),
        });
        let world = tr::ValidatedTraitWorld::build(headers, adapter, budget)
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
        let mut validated = HashMap::new();
        for id in &self.registry.function_ids {
            budget.charge_work(1)?;
            let raw = world
                .normalized_callable_schema(*id)
                .map_err(|e| e.into_legacy_diagnostic(budget))?;
            let facts = world
                .callable_facts(*id)
                .map_err(|e| e.into_legacy_diagnostic(budget))?;
            validated.insert(
                *id,
                ir::GenericSignature {
                    env: raw.env,
                    signature: raw.signature,
                    predicates: facts.predicates().clone(),
                },
            );
        }
        self.validated = Arc::new(validated);
        self.world = Some(world);
        self.shared_env = Some(Arc::new(self.build_shared_env()?));
        Ok(())
    }
    pub fn trait_world(&self) -> Result<Arc<tr::ValidatedTraitWorld>, String> {
        self.world
            .clone()
            .ok_or_else(|| "trait world has not been published".into())
    }
    pub fn callable_facts(&self, decl: ir::DeclId) -> Result<Arc<tr::ValidatedFacts>, String> {
        self.trait_world()?
            .callable_facts(decl)
            .map_err(|e| e.into_legacy_diagnostic(&self.budget))
    }
    pub fn instance_facts(
        &self,
        target: &ir::CallTarget,
    ) -> Result<Arc<tr::ValidatedFacts>, String> {
        self.trait_world()?
            .instance_facts(target)
            .map_err(|e| e.into_legacy_diagnostic(&self.budget))
    }
    fn check_budget(&self, budget: &ir::PlanBudget) -> Result<(), String> {
        if !self.budget.same_authority(budget) {
            return Err("generic planning budget authority mismatch".into());
        }
        Ok(())
    }
    pub fn declaration_origin(&self, id: ir::DeclId) -> Result<tr::DeclOrigin, String> {
        self.declaration(id)?;
        Ok(self
            .catalog
            .declaration(id)
            .map_or(tr::DeclOrigin::Source, |d| d.origin))
    }
    pub fn callable_provider(&self, id: ir::DeclId) -> Result<CallableProvider, String> {
        self.declaration(id)?;
        if let Some(d) = self.catalog.declaration(id) {
            return d
                .callable_provider
                .ok_or_else(|| "declaration has no executable provider".into());
        }
        Ok(CallableProvider::Source {
            source_index: self.callable_index(id)?,
        })
    }
    pub fn checked_body(
        &self,
        target: &ir::CallTarget,
        facts: &tr::ValidatedFacts,
        budget: &ir::PlanBudget,
    ) -> Result<ir::CheckedFunction, String> {
        self.check_budget(budget)?;
        if facts.phase() != tr::FactsPhase::Universal
            || facts.declaration() != target.declaration
            || !budget.same_authority(facts.budget())
            || !Arc::ptr_eq(facts.world(), &self.trait_world()?)
        {
            return Err("checked body requires exact universal declaration facts".into());
        }
        let env = self.callable_env(target.declaration)?;
        budget.retain_bytes(env.total_arity() * std::mem::size_of::<ir::Ty>() + 128)?;
        budget.charge_work(env.total_arity())?;
        let ordered = env.ordered_params().map(ir::Ty::Param).collect::<Vec<_>>();
        for (a, b) in ordered.iter().zip(target.type_args.iter()) {
            if !metered_type_equal(a, b, budget, 0)? {
                return Err("checked body requires rigid arguments".into());
            }
        }
        if ordered.len() != target.type_args.len() {
            return Err("checked body rigid arity mismatch".into());
        }
        match self.callable_provider(target.declaration)? {
            CallableProvider::Source { .. } => {
                type_callable_with_plan(&self.program, target.declaration, self, budget)
            }
            CallableProvider::Recipe { descriptor } => {
                recipes::checked_body(self, target, facts, descriptor, budget)
            }
            CallableProvider::UnavailableProvided { .. } => {
                Err("unsupported: unavailable provided body".into())
            }
        }
    }
    pub fn declaration(&self, id: ir::DeclId) -> Result<&DeclInfo, String> {
        self.registry.declaration(id)
    }
    pub fn callable_count(&self) -> usize {
        self.registry.function_ids.len()
    }
    pub fn callable_decl(&self, index: usize) -> Result<ir::DeclId, String> {
        self.function_decl(index)
    }
    pub fn callable_index(&self, decl: ir::DeclId) -> Result<usize, String> {
        self.declaration(decl)?;
        self.budget.charge_work(1)?;
        self.registry
            .function_position(decl)
            .ok_or_else(|| "declaration is not callable".into())
    }
    pub fn callable_body<'p>(
        &self,
        program: &'p Program,
        decl: ir::DeclId,
    ) -> Result<&'p Function, String> {
        if !Arc::ptr_eq(&program.names, &self.program.names)
            || !Arc::ptr_eq(&program.modules, &self.program.modules)
        {
            return Err("contract: callable source owner mismatch".into());
        }
        let index = self.callable_index(decl)?;
        match *self
            .registry
            .callable_sources
            .get(index)
            .ok_or("recipe has no source AST body")?
        {
            CallableSource::Free(i) => program.functions.get(i),
            CallableSource::TraitDefault { trait_index, item } => program
                .traits
                .get(trait_index)
                .and_then(|t| t.items.get(item))
                .and_then(|i| i.default.as_ref()),
            CallableSource::Associated {
                implementation,
                item,
            } => program
                .impls
                .get(implementation)
                .and_then(|b| b.items.get(item))
                .map(|m| &m.function),
        }
        .ok_or_else(|| "callable source does not belong to program".into())
    }
    fn validate_callable_associated_bounds(
        &self,
        program: &Program,
        declaration: ir::DeclId,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        self.check_budget(budget)?;
        let owner_index = match self.registry.callable_sources[self.callable_index(declaration)?] {
            CallableSource::Free(_) => return Ok(()),
            CallableSource::TraitDefault { trait_index, .. } => trait_index,
            CallableSource::Associated { implementation, .. } => {
                let Some(path) = &self.program.impls[implementation].trait_path else {
                    return Ok(());
                };
                let owner = self.registry.resolve_trait_path(path)?;
                if self.catalog.declaration(owner).is_some() {
                    return Ok(());
                }
                self.registry.declaration(owner)?.source_index
            }
        };
        let supplied = program
            .traits
            .get(owner_index)
            .ok_or("callable associated-bound owner was removed")?;
        name_validation::validate_associated_bound_names(program, supplied)?;
        let original = self
            .program
            .traits
            .get(owner_index)
            .ok_or("missing retained callable associated-bound owner")?;
        let ids = self
            .registry
            .trait_associated_types
            .get(owner_index)
            .ok_or("missing retained associated-bound member identities")?;
        budget.charge_work(ids.len().saturating_add(1))?;
        let mut equal = supplied.associated_types.len() == original.associated_types.len();
        for (index, id) in ids.iter().enumerate() {
            let frozen = self
                .registry
                .associated_type_bounds
                .get(id)
                .ok_or("missing frozen associated bound metadata")?;
            if let Some(source_index) = frozen.source_index {
                let source = self
                    .program
                    .associated_bound_sources
                    .get(source_index)
                    .ok_or("missing retained associated bound occurrence")?;
                program.validate_associated_bound_source(source, budget)?;
            } else {
                let candidate = supplied
                    .associated_types
                    .get(index)
                    .ok_or("callable associated member was removed")?;
                equal &= exact_source_ident(
                    &program.names,
                    candidate.name,
                    original.associated_types[index].name,
                )?;
                equal &= candidate.bounds.is_empty();
            }
        }
        if !equal {
            return Err("callable associated declaration bounds changed".into());
        }
        Ok(())
    }
    pub fn callable_env(&self, decl: ir::DeclId) -> Result<Arc<ir::GenericEnv>, String> {
        Ok(self.declaration(decl)?.env.clone())
    }
    pub fn callable_context(&self, decl: ir::DeclId) -> Result<CallableContext, String> {
        self.state
            .lock()
            .map_err(|_| "generic plan context poisoned")?
            .contexts
            .get(&decl)
            .cloned()
            .ok_or_else(|| "missing callable context".into())
    }
    pub fn function_decl(&self, index: usize) -> Result<ir::DeclId, String> {
        self.registry
            .function_ids
            .get(index)
            .copied()
            .ok_or_else(|| "invalid function declaration index".into())
    }
    pub fn semantic_path(&self, id: ir::DeclId) -> Result<&DeclSemanticPath, String> {
        Ok(&self.declaration(id)?.path)
    }
    pub fn param_env(&self, id: ir::DeclId) -> Result<Arc<ir::ParamEnv>, String> {
        Ok(self.declaration(id)?.param_env.clone())
    }
    pub fn function_schema(&self, id: ir::DeclId) -> Result<ir::GenericSignature, String> {
        let schema = self
            .validated
            .get(&id)
            .ok_or("missing validated callable schema")?;
        self.budget.charge_work(schema.signature.params.len() + 1)?;
        self.budget.retain_bytes(
            std::mem::size_of_val(schema.signature.params.as_slice())
                .checked_mul(4)
                .and_then(|n| n.checked_add(128))
                .ok_or("validated signature allocation overflow")?,
        )?;
        Ok(schema.clone())
    }
    fn raw_schema(&self, id: ir::DeclId) -> Result<tr::GenericSchema, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "generic plan context poisoned")?;
        let schema = state
            .functions
            .get(&id)
            .ok_or("unknown function declaration")?;
        self.budget.charge_work(schema.signature.params.len() + 1)?;
        self.budget.retain_bytes(
            schema
                .signature
                .params
                .len()
                .checked_mul(4 * std::mem::size_of::<ir::Ty>() + 64)
                .ok_or("signature allocation overflow")?,
        )?;
        Ok(schema.clone())
    }
    pub fn source_region_env(&self, id: ir::DeclId) -> Result<Arc<ir::SourceRegionEnv>, String> {
        self.budget.charge_work(1)?;
        self.declaration(id)?;
        let state = self
            .state
            .lock()
            .map_err(|_| "source region registry poisoned")?;
        if let Some(env) = state.source_regions.get(&id) {
            return Ok(env.clone());
        }
        if state.functions.contains_key(&id) && self.catalog.declaration(id).is_none() {
            return Err("source callable has no original lifetime environment".into());
        }
        // Catalog recipes and noncallable declaration headers declare no source
        // lifetime names. Their fixed contracts remain separately authoritative.
        self.budget
            .retain_bytes(std::mem::size_of::<ir::SourceRegionEnv>() + 64)?;
        Ok(Arc::new(ir::SourceRegionEnv {
            declaration: id,
            region_count: 0,
            outlives: Arc::from([]),
            normalized: Arc::from([]),
        }))
    }
    pub fn nominal_schema(
        &self,
        decl: ir::DeclId,
        budget: &ir::PlanBudget,
    ) -> Result<(Arc<ir::ParamEnv>, ir::Ty), String> {
        self.check_budget(budget)?;
        let info = self.declaration(decl)?;
        if info.kind == ir::DeclKind::Function {
            return Err("function is not a nominal schema".into());
        }
        let _scratch = budget.reserve_bytes(
            info.param_env
                .arity()
                .checked_mul(std::mem::size_of::<ir::Ty>())
                .and_then(|n| n.checked_add(128))
                .ok_or("inference scratch size overflow")?,
        )?;
        let args = symbolic_args(&info.param_env, budget)?;
        budget.charge_work(1 + args.len())?;
        let state = self
            .state
            .lock()
            .map_err(|_| "generic plan context poisoned")?;
        let ty = state
            .nominals
            .applications
            .get(&(decl, args))
            .cloned()
            .ok_or("missing canonical nominal schema")?;
        Ok((info.param_env.clone(), ty))
    }
    pub fn signature(&self, id: ir::DeclId) -> Result<ir::GenericSignature, String> {
        self.function_schema(id)
    }
    pub fn binder(&self, id: ir::DeclId) -> Result<Arc<ir::ParamEnv>, String> {
        self.param_env(id)
    }
    fn source_ty(
        &self,
        ty: &Type,
        bindings: &HashMap<GenericBindingKey, ir::Ty>,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<ir::Ty, String> {
        self.check_budget(budget)?;
        let structural = match caller {
            ir::FactsCtx::Validated(facts) => {
                if !facts.budget().same_authority(budget)
                    || !Arc::ptr_eq(facts.world(), &self.trait_world()?)
                {
                    return Err("source type changed validated caller authority".into());
                }
                ir::FactsCtx::Universal(facts.env().as_ref())
            }
            other => other,
        };
        // Validated nominal facts may re-enter SchemaAdapter and this mutex.
        // Intern structurally under the exact binder, then check the original
        // caller's complete authority after the lexical state guard is gone.
        let out = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "generic plan context poisoned")?;
            let context = match caller {
                ir::FactsCtx::Validated(facts) => {
                    Some(self.registry.source_type_context(facts.declaration())?)
                }
                _ => None,
            };
            let previous = state.source_context;
            state.source_context = context;
            let result = state.source_ty(&self.registry, ty, bindings, structural, budget, 0, true);
            state.source_context = previous;
            result?
        };
        let out = if let ir::FactsCtx::Validated(facts) = caller {
            facts
                .world()
                .normalize_ty(&out, facts)
                .map_err(|e| e.into_legacy_diagnostic(budget))?
        } else {
            out
        };
        caller.validate_ty_with_budget(&out, budget)?;
        if let Some(element) = out.shared_slice_element() {
            ir::validate_shared_slice_element(element, caller, budget)?;
        }
        Ok(out)
    }
    fn application(
        &self,
        path: &GenericPath,
        namespace: Namespace,
        bindings: &HashMap<GenericBindingKey, ir::Ty>,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<(PathKey, Vec<ir::Ty>), String> {
        let (at, segment) = path
            .segments
            .iter()
            .enumerate()
            .find(|(_, s)| !s.args.is_empty())
            .ok_or("missing generic arguments")?;
        if path.segments[at + 1..].iter().any(|s| !s.args.is_empty()) {
            return Err(
                "multiple generic declaration applications in one path are unsupported".into(),
            );
        }
        let full = path.path(self.program.names.as_ref())?;
        let base = full
            .semantic_slice(self.program.names.as_ref(), 0..at + 1, full.anchor())
            .map_err(native_adapter::name_diagnostic)?;
        let name = name_scope::path_key(self.program.names.as_ref(), &base, namespace)?;
        let args = segment
            .args
            .iter()
            .map(|t| self.source_ty(t, bindings, caller, budget))
            .collect::<Result<Vec<_>, _>>()?;
        Ok((name, args))
    }
    pub fn intern_nominal(
        &self,
        decl: ir::DeclId,
        args: &[ir::Ty],
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<ir::Ty, String> {
        self.check_budget(budget)?;
        self.state
            .lock()
            .map_err(|_| "generic plan context poisoned")?
            .intern(&self.registry, decl, args, caller, budget, 0)
    }
    pub fn instantiate_signature(
        &self,
        target: &ir::CallTarget,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<ir::Signature, String> {
        self.check_budget(budget)?;
        if let ir::FactsCtx::Validated(facts) = caller {
            if !facts.budget().same_authority(budget) {
                return Err("call signature changed planning budget authority".into());
            }
            // Executable call signatures include declaration-owned latent
            // carrier transport. Structural substitution alone cannot mint
            // the reference rows introduced by an admitted reference actual.
            let proof = facts
                .service()
                .discharge_direct(target, facts)
                .map_err(|error| error.into_legacy_diagnostic(budget))?;
            let signature = proof.signature();
            budget.charge_work(signature.params.len() + 1)?;
            budget.retain_bytes(
                signature
                    .params
                    .len()
                    .checked_mul(std::mem::size_of::<ir::Ty>())
                    .and_then(|bytes| bytes.checked_add(std::mem::size_of::<ir::Signature>()))
                    .ok_or("generic planning byte limit exceeded")?,
            )?;
            return Ok(signature.clone());
        }
        let info = self.declaration(target.declaration)?;
        let schema = if let Some(world) = &self.world {
            world
                .normalized_callable_schema(target.declaration)
                .map_err(|e| e.into_legacy_diagnostic(budget))?
        } else {
            self.raw_schema(target.declaration)?
        };
        if info.env.total_arity() != target.type_args.len() {
            return Err("wrong callable generic arity".into());
        }
        for arg in target.type_args.iter() {
            caller.validate_ty_with_budget(arg, budget)?;
            ir::validate_root_reference_actual(arg, budget)?;
        }
        let mut cx = self.clone();
        let params = schema
            .signature
            .params
            .iter()
            .map(|ty| ir::substitute_ty(ty, &info.env, &target.type_args, caller, &mut cx, budget))
            .collect::<Result<Vec<_>, _>>()?;
        let result = ir::substitute_ty(
            &schema.signature.result,
            &info.env,
            &target.type_args,
            caller,
            &mut cx,
            budget,
        )?;
        Ok(ir::Signature {
            params,
            result,
            ref_contract: schema.signature.ref_contract.clone(),
        })
    }
    fn environment(&self, decl: Option<ir::DeclId>) -> Result<TypeEnv, String> {
        match &self.shared_env {
            Some(shared) => self.finish_environment(shared, decl),
            // Before the trait world is published there is no shared fact.
            None => self.finish_environment(&self.build_shared_env()?, decl),
        }
    }
    #[expect(
        clippy::mutable_key_type,
        reason = "PathKey hashes immutable owner/anchor/namespace/name keys only; its shared resource ledger is not semantic identity"
    )]
    fn build_shared_env(&self) -> Result<SharedTypeEnv, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "generic plan context poisoned")?;
        let mut environment_bytes = 0usize;
        for info in &self.registry.declarations {
            self.budget.charge_work(1)?;
            let params = state
                .functions
                .get(&info.id)
                .map_or(0, |s| s.signature.params.len());
            let bytes = info
                .name
                .len()
                .checked_mul(3)
                .and_then(|n| n.checked_add(512))
                .and_then(|n| {
                    n.checked_add(params.checked_mul(4 * std::mem::size_of::<ir::Ty>() + 64)?)
                })
                .ok_or("body environment allocation overflow")?;
            environment_bytes = environment_bytes
                .checked_add(bytes)
                .ok_or("body environment allocation overflow")?;
        }
        self.budget.retain_bytes(environment_bytes)?;
        let mut functions = HashMap::new();
        let mut structs = HashMap::new();
        let mut enums = HashMap::new();
        for (key, id) in &self.registry.functions {
            self.budget.charge_work(1)?;
            functions.insert(
                key.clone(),
                self.validated
                    .get(id)
                    .ok_or("missing validated function schema")?
                    .signature
                    .clone(),
            );
        }
        for (key, id) in &self.registry.nominals {
            self.budget.charge_work(1)?;
            let info = self.declaration(*id)?;
            if info.param_env.arity() != 0 {
                continue;
            }
            match state.nominals.applications.get(&(*id, Vec::new())) {
                Some(ir::Ty::Struct(ty)) => {
                    structs.insert(key.clone(), ty.clone());
                }
                Some(ir::Ty::Enum(ty)) => {
                    enums.insert(key.clone(), ty.clone());
                }
                _ => return Err("missing nominal application".into()),
            }
        }
        Ok(SharedTypeEnv {
            structs: Arc::new(structs),
            enums: Arc::new(enums),
            functions: Arc::new(functions),
        })
    }
    fn finish_environment(
        &self,
        shared: &SharedTypeEnv,
        decl: Option<ir::DeclId>,
    ) -> Result<TypeEnv, String> {
        // Per-body state only: the shared maps are reference counted.
        self.budget.charge_work(1)?;
        self.budget.retain_bytes(512)?;
        let mut env = TypeEnv {
            structs: shared.structs.clone(),
            enums: shared.enums.clone(),
            functions: shared.functions.clone(),
            plan: Some(self.clone()),
            params: HashMap::new(),
            param_env: None,
            self_ty: None,
            facts: None,
        };
        let state = self
            .state
            .lock()
            .map_err(|_| "generic plan context poisoned")?;
        if let Some(decl) = decl {
            let info = self.declaration(decl)?;
            env.params = generic_bindings(&self.registry, &info.env, &self.budget)?;
            env.self_ty = state.contexts.get(&decl).and_then(|c| c.self_ty.clone());
            if let Some(ty) = &env.self_ty {
                env.params.insert(GenericBindingKey::SelfType, ty.clone());
            }
            if info.env.is_generic() {
                env.param_env = Some(info.env.clone());
            }
        }
        drop(state);
        if let Some(decl) = decl {
            env.facts = Some(self.callable_facts(decl)?);
        }
        Ok(env)
    }
}
impl ir::SubstitutionCx for PlanCtx {
    fn nominal_application(&self, id: ir::NominalAppId) -> Result<ir::NominalAppInfo, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "generic plan context poisoned")?;
        let ty = state
            .nominals
            .ordered
            .get(id.index)
            .ok_or("unknown nominal application")?;
        match ty {
            ir::Ty::Struct(t) if t.def().id == id => Ok(ir::NominalAppInfo {
                source_decl: t.def().source_decl,
                type_args: t.def().type_args.clone(),
            }),
            ir::Ty::Enum(t) if t.def().id == id => Ok(ir::NominalAppInfo {
                source_decl: t.def().source_decl,
                type_args: t.def().type_args.clone(),
            }),
            _ => Err("foreign nominal application identity".into()),
        }
    }
    fn intern_nominal(
        &mut self,
        decl: ir::DeclId,
        args: &[ir::Ty],
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<ir::Ty, String> {
        PlanCtx::intern_nominal(self, decl, args, caller, budget)
    }
    fn intern_substituted_nominal(
        &mut self,
        request: &ir::NominalSubstitutionRequest<'_>,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<ir::Ty, String> {
        self.check_budget(budget)?;
        budget.charge_work(1)?;
        for arg in request.arguments() {
            caller.validate_ty_with_budget(arg, budget)?;
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| "generic plan context poisoned")?;
        let original = state
            .nominals
            .ordered
            .get(request.original().index)
            .ok_or("substitution request names unknown nominal")?;
        let declaration = match original {
            ir::Ty::Struct(ty) if ty.def().id == request.original() => ty.def().source_decl,
            ir::Ty::Enum(ty) if ty.def().id == request.original() => ty.def().source_decl,
            _ => return Err("substitution request names foreign nominal".into()),
        };
        state.intern_with_transport(
            &self.registry,
            declaration,
            request.arguments(),
            caller,
            budget,
            0,
            Some(request),
        )
    }
    fn instantiate_signature(
        &mut self,
        target: &ir::CallTarget,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<ir::Signature, String> {
        PlanCtx::instantiate_signature(self, target, caller, budget)
    }
}

fn receiver_mode(receiver: ReceiverKind) -> ir::ReceiverMode {
    match receiver {
        ReceiverKind::None => ir::ReceiverMode::None,
        ReceiverKind::Value { .. } => ir::ReceiverMode::Value,
        ReceiverKind::Borrowed { kind } => ir::ReceiverMode::Borrowed { kind },
    }
}
struct LoweredTraitApplication {
    trait_ref: ir::TraitRef,
    equalities: Vec<LoweredAssociatedEquality>,
}
struct LoweredAssociatedEquality {
    projection: ir::AssociatedTypeProjection,
    ty: ir::Ty,
}
impl LoweredTraitApplication {
    fn reject_equalities(self) -> Result<ir::TraitRef, String> {
        if !self.equalities.is_empty() {
            return Err("unsupported: associated equalities are only admitted in callable and implementation bounds".into());
        }
        Ok(self.trait_ref)
    }
}
impl PlanState {
    // Metadata-only pass. Capability checks must not hide malformed later
    // children behind an earlier unsupported reference or equality position.
    fn validate_source_type_metadata(
        &self,
        registry: &DeclRegistry,
        ty: &Type,
        bindings: &HashMap<GenericBindingKey, ir::Ty>,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        let mut inspect = |node: &DepthNode<'_>, depth| {
            budget.check_depth(depth)?;
            budget.charge_work(1)?;
            if let DepthNode::Type(ty) = node {
                match ty {
                    Type::Param(parameter) => {
                        let ty = bindings
                            .get(&registry.parameter_key(*parameter)?)
                            .ok_or("unknown generic parameter")?;
                        caller.validate_ty_metadata_with_budget(ty, budget)?;
                    }
                    Type::SelfType => {
                        let ty = bindings
                            .get(&GenericBindingKey::SelfType)
                            .ok_or("Self is not in an impl scope")?;
                        caller.validate_ty_metadata_with_budget(ty, budget)?;
                    }
                    Type::Named(path) => {
                        let key =
                            name_scope::path_key(registry.names.as_ref(), path, Namespace::Type)?;
                        let id = registry.nominals.get(&key).ok_or("unknown nominal type")?;
                        if registry.declaration(*id)?.param_env.arity() != 0 {
                            return Err("nominal type argument arity mismatch".into());
                        }
                    }
                    Type::Apply(path) => {
                        let key = name_scope::path_key(
                            registry.names.as_ref(),
                            &path.path(registry.names.as_ref())?,
                            Namespace::Type,
                        )?;
                        let id = registry.nominals.get(&key).ok_or("unknown nominal type")?;
                        let count = path.segments.iter().try_fold(0usize, |count, segment| {
                            count
                                .checked_add(segment.args.len())
                                .ok_or("nominal argument count overflow")
                        })?;
                        if registry.declaration(*id)?.param_env.arity() != count {
                            return Err("nominal type argument arity mismatch".into());
                        }
                    }
                    Type::Projection(projection) => {
                        registry.validate_source_projection(
                            self.source_context
                                .ok_or("projection without retained source declaration context")?,
                            projection,
                            budget,
                        )?;
                        if let SourceProjection::Qualified {
                            trait_path, item, ..
                        } = projection.as_ref()
                        {
                            let owner = registry.resolve_trait_path(trait_path)?;
                            registry.associated_type_item(owner, *item, budget)?;
                        }
                    }
                    _ => {}
                }
            }
            Ok(())
        };
        walk_depth_inspecting(
            DepthNode::Type(ty),
            None,
            Some(registry.names.as_ref()),
            Some(&mut inspect),
        )
    }
    fn source_trait_application(
        &mut self,
        registry: &DeclRegistry,
        path: &TraitPath,
        owner: &ir::Ty,
        bindings: &HashMap<GenericBindingKey, ir::Ty>,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<LoweredTraitApplication, String> {
        budget.charge_work(1)?;
        caller.validate_ty_metadata_with_budget(owner, budget)?;
        let declaration = registry.resolve_trait_path(path)?;
        if let TraitPath::Source(path) = path {
            path.path(registry.names.as_ref())?;
            // Visit all supplied children before lowering can issue a capability error.
            for segment in &path.segments {
                for actual in &segment.args {
                    self.validate_source_type_metadata(registry, actual, bindings, caller, budget)?;
                }
                for binding in &segment.associated_types {
                    registry.associated_type_item(declaration, binding.name, budget)?;
                    self.validate_source_type_metadata(
                        registry,
                        &binding.value,
                        bindings,
                        caller,
                        budget,
                    )?;
                }
            }
        }
        let mut args = Vec::new();
        if let TraitPath::Source(path) = path {
            // Check the exact original-use certificate, never synthesize a new
            // path from the final printable segment.
            path.path(registry.names.as_ref())?;
            let count = path.segments.iter().try_fold(0usize, |count, segment| {
                count
                    .checked_add(segment.args.len())
                    .ok_or("trait actual count overflow")
            })?;
            budget.charge_work(
                count
                    .checked_add(path.segments.len())
                    .ok_or("trait actual work overflow")?,
            )?;
            let bytes = count
                .checked_mul(std::mem::size_of::<ir::Ty>())
                .ok_or("trait actual storage overflow")?;
            budget.retain_bytes(bytes)?;
            args.try_reserve_exact(count).map_err(|_| {
                super::source::SourceError::from_token(super::token_session::TokenError::Budget(
                    registry.names.budget().poison_allocation(),
                ))
                .to_string()
            })?;
            budget.retain_bytes(
                args.capacity()
                    .saturating_sub(count)
                    .checked_mul(std::mem::size_of::<ir::Ty>())
                    .ok_or("trait actual storage overflow")?,
            )?;
            // Every written actual is checked before shape/arity rejection.
            for segment in &path.segments {
                for actual in &segment.args {
                    let ty =
                        self.source_ty(registry, actual, bindings, caller, budget, 0, false)?;
                    caller.validate_ty_metadata_with_budget(&ty, budget)?;
                    args.push(ty);
                }
            }
            if path
                .segments
                .iter()
                .take(path.segments.len().saturating_sub(1))
                .any(|segment| !segment.args.is_empty())
            {
                return Err("unsupported: only final trait segment may carry arguments".into());
            }
        }
        let expected = registry
            .declaration(declaration)?
            .param_env
            .arity()
            .checked_sub(1)
            .ok_or("contract: trait missing Self binder")?;
        if args.len() != expected {
            return Err("trait type argument arity mismatch".into());
        }
        budget.retain_bytes(
            args.len()
                .checked_mul(std::mem::size_of::<ir::Ty>())
                .and_then(|bytes| bytes.checked_add(2 * std::mem::size_of::<usize>()))
                .ok_or("trait actual storage overflow")?,
        )?;
        let trait_ref = ir::TraitRef {
            declaration,
            self_ty: owner.clone(),
            args: args.into(),
        };
        let mut equalities = Vec::new();
        if let TraitPath::Source(path) = path {
            for segment in &path.segments {
                for binding in &segment.associated_types {
                    budget.charge_work(1)?;
                    let item = registry.associated_type_item(declaration, binding.name, budget)?;
                    let ty = self.source_ty(
                        registry,
                        &binding.value,
                        bindings,
                        caller,
                        budget,
                        0,
                        true,
                    )?;
                    caller.validate_ty_metadata_with_budget(&ty, budget)?;
                    budget.retain_bytes(std::mem::size_of::<LoweredAssociatedEquality>())?;
                    let old_capacity = equalities.capacity();
                    equalities.try_reserve_exact(1).map_err(|_| {
                        super::source::SourceError::from_token(
                            super::token_session::TokenError::Budget(
                                registry.names.budget().poison_allocation(),
                            ),
                        )
                        .to_string()
                    })?;
                    budget.retain_bytes(
                        equalities
                            .capacity()
                            .saturating_sub(old_capacity)
                            .saturating_sub(1)
                            .checked_mul(std::mem::size_of::<LoweredAssociatedEquality>())
                            .ok_or("associated equality capacity overflow")?,
                    )?;
                    equalities.push(LoweredAssociatedEquality {
                        projection: ir::AssociatedTypeProjection {
                            trait_ref: trait_ref.clone(),
                            item,
                        },
                        ty,
                    });
                }
            }
            if path
                .segments
                .iter()
                .take(path.segments.len().saturating_sub(1))
                .any(|segment| !segment.associated_types.is_empty())
            {
                return Err(
                    "unsupported: only final trait segment may carry associated equalities".into(),
                );
            }
        }
        for arg in trait_ref.args.iter() {
            ir::reject_references(arg, budget)?;
        }
        Ok(LoweredTraitApplication {
            trait_ref,
            equalities,
        })
    }
}
fn append_source_bounds(
    state: &mut PlanState,
    registry: &DeclRegistry,
    bindings: &HashMap<GenericBindingKey, ir::Ty>,
    bounds: &[TypeBound],
    out: &mut Vec<ir::Predicate>,
    caller: ir::FactsCtx<'_>,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    for row in bounds {
        budget.charge_work(1)?;
        let ty = bindings
            .get(&registry.parameter_key(row.parameter)?)
            .ok_or("bound parameter outside declaration")?;
        for path in &row.traits {
            budget.charge_work(1)?;
            if out.len() >= tr::MAX_PREDICATES {
                return Err("predicate environment limit exceeded".into());
            }
            budget.retain_bytes(256)?;
            let application =
                state.source_trait_application(registry, path, ty, bindings, caller, budget)?;
            if !application.equalities.is_empty() {
                registry.validate_source_bound_path(
                    state
                        .source_context
                        .ok_or("associated equality without retained source context")?,
                    path,
                    budget,
                )?;
            }
            let count = application
                .equalities
                .len()
                .checked_add(1)
                .ok_or("predicate count overflow")?;
            if out
                .len()
                .checked_add(count)
                .ok_or("predicate count overflow")?
                > tr::MAX_PREDICATES
            {
                return Err("predicate environment limit exceeded".into());
            }
            budget.retain_bytes(
                count
                    .checked_mul(std::mem::size_of::<ir::Predicate>())
                    .ok_or("predicate storage overflow")?,
            )?;
            out.try_reserve_exact(count).map_err(|_| {
                super::source::SourceError::from_token(super::token_session::TokenError::Budget(
                    registry.names.budget().poison_allocation(),
                ))
                .to_string()
            })?;
            out.push(ir::Predicate::Implements(application.trait_ref));
            for equality in application.equalities {
                out.push(ir::Predicate::ProjectionEq {
                    projection: equality.projection,
                    ty: equality.ty,
                });
            }
        }
    }
    Ok(())
}
#[derive(Debug)]
struct SchemaAdapter {
    program: Arc<Program>,
    catalog: Arc<tr::CoreCatalog>,
    registry: Arc<DeclRegistry>,
    state: Arc<Mutex<PlanState>>,
    budget: ir::PlanBudget,
}
impl SchemaAdapter {
    fn raw_plan(&self) -> Result<PlanCtx, String> {
        self.budget.retain_bytes(128)?;
        Ok(PlanCtx {
            program: self.program.clone(),
            catalog: self.catalog.clone(),
            registry: self.registry.clone(),
            state: self.state.clone(),
            budget: self.budget.clone(),
            world: None,
            validated: Arc::new(HashMap::new()),
            shared_env: None,
            closed_roots: Arc::from([]),
        })
    }
    fn error(&self, e: String) -> tr::FactsError {
        tr::FactsError::from_message(&self.budget, &e)
    }
}
impl tr::SchemaCx for SchemaAdapter {
    fn core_catalog(&self) -> &Arc<tr::CoreCatalog> {
        &self.catalog
    }
    fn budget(&self) -> &ir::PlanBudget {
        &self.budget
    }
    fn registry_snapshot(&self) -> ir::SnapshotId {
        self.registry.declarations[0].id.snapshot
    }
    fn declaration_info(&self, id: ir::DeclId) -> Result<tr::SchemaDecl, tr::FactsError> {
        self.budget.charge_work(1).map_err(|e| self.error(e))?;
        let d = self.registry.declaration(id).map_err(|e| self.error(e))?;
        clone_visibility(&d.visibility, &self.budget).map_err(|e| self.error(e))?;
        let receiver = receiver_mode(d.receiver);
        Ok(tr::SchemaDecl {
            declaration: id,
            kind: d.kind,
            own_env: d.param_env.clone(),
            env: d.env.clone(),
            owner: d.owner,
            module: d.path.module,
            visibility: d.visibility.clone(),
            receiver,
            region_contract_mode: self
                .state
                .lock()
                .map_err(|_| self.error("schema state lock poisoned".into()))?
                .region_modes
                .get(&id)
                .copied()
                .unwrap_or(tr::RegionContractMode::Positioned),
        })
    }
    fn declaration_schema(&self, id: ir::DeclId) -> Result<tr::GenericSchema, tr::FactsError> {
        self.registry.declaration(id).map_err(|e| self.error(e))?;
        self.raw_plan()
            .and_then(|plan| plan.raw_schema(id))
            .map_err(|e| self.error(e))
    }
    fn associated_type_rhs(
        &self,
        implementation: ir::DeclId,
        associated: ir::DeclId,
    ) -> Result<ir::Ty, tr::FactsError> {
        self.budget.charge_work(1).map_err(|e| self.error(e))?;
        let implementation_info = self
            .registry
            .declaration(implementation)
            .map_err(|e| self.error(e))?;
        let associated_info = self
            .registry
            .declaration(associated)
            .map_err(|e| self.error(e))?;
        if implementation_info.kind != ir::DeclKind::TraitImpl
            || associated_info.kind != ir::DeclKind::AssociatedType
        {
            return Err(self.error("invalid associated assignment pair kinds".into()));
        }
        let block = self
            .program
            .impls
            .get(implementation_info.source_index)
            .ok_or_else(|| self.error("missing associated implementation source".into()))?;
        let trait_decl = block
            .trait_path
            .as_ref()
            .ok_or_else(|| self.error("associated assignment has no trait".into()))
            .and_then(|path| {
                self.registry
                    .resolve_trait_path(path)
                    .map_err(|e| self.error(e))
            })?;
        if associated_info.owner != Some(trait_decl) {
            return Err(self.error("associated assignment pair owner mismatch".into()));
        }
        self.state
            .lock()
            .map_err(|_| self.error("schema state lock poisoned".into()))?
            .associated_type_rhs
            .get(&(implementation, associated))
            .cloned()
            .ok_or_else(|| self.error("missing authoritative associated assignment".into()))
    }
    fn associated_type_bounds(
        &self,
        associated: ir::DeclId,
    ) -> Result<Arc<[ir::DeclId]>, tr::FactsError> {
        self.budget.charge_work(1).map_err(|e| self.error(e))?;
        let info = self
            .registry
            .declaration(associated)
            .map_err(|e| self.error(e))?;
        if info.kind != ir::DeclKind::AssociatedType {
            return Err(self.error("associated bounds requested for a nonmember".into()));
        }
        let owner = self
            .registry
            .declaration(
                info.owner
                    .ok_or_else(|| self.error("associated bounds have no owner".into()))?,
            )
            .map_err(|e| self.error(e))?;
        if owner.kind != ir::DeclKind::Trait || owner.path.module != info.path.module {
            return Err(self.error("associated bound owner mismatch".into()));
        }
        self.registry
            .associated_type_bounds
            .get(&associated)
            .map(|bounds| bounds.ids.clone())
            .ok_or_else(|| self.error("missing authoritative associated declaration bounds".into()))
    }
    fn source_region_env(
        &self,
        id: ir::DeclId,
    ) -> Result<Arc<ir::SourceRegionEnv>, tr::FactsError> {
        self.raw_plan()
            .and_then(|plan| plan.source_region_env(id))
            .map_err(|e| self.error(e))
    }
    fn instantiate_schema(
        &self,
        schema: &tr::GenericSchema,
        args: &[ir::Ty],
        caller: &ir::GenericEnv,
        budget: &ir::PlanBudget,
    ) -> Result<tr::InstantiatedSchema, tr::FactsError> {
        if !budget.same_authority(&self.budget) {
            return Err(tr::FactsError::invalid(
                budget,
                "schema budget authority mismatch",
            ));
        }
        budget
            .retain_bytes(256 + schema.signature.params.len() * 128 + schema.predicates.len() * 256)
            .map_err(|e| self.error(e))?;
        let mut cx = self.raw_plan().map_err(|e| self.error(e))?;
        let facts = ir::FactsCtx::Universal(caller);
        let mut schema_only_item = false;
        for scope in schema.env.scopes() {
            budget.charge_work(1).map_err(|e| self.error(e))?;
            let declaration = self
                .registry
                .declaration(scope.binder().declaration)
                .map_err(|e| self.error(e))?;
            schema_only_item |= declaration.kind == ir::DeclKind::TraitItem;
        }
        let mut sub = |ty: &ir::Ty| {
            if schema_only_item {
                ir::substitute_schema_ty(ty, &schema.env, args, facts, &mut cx, self, budget)
            } else {
                ir::substitute_ty(ty, &schema.env, args, facts, &mut cx, budget)
            }
            .map_err(|e| self.error(e))
        };
        let params = schema
            .signature
            .params
            .iter()
            .map(&mut sub)
            .collect::<Result<Vec<_>, _>>()?;
        let result = sub(&schema.signature.result)?;
        let mut predicates = Vec::new();
        for row in schema.predicates.iter() {
            let r = row.trait_ref();
            let self_ty = sub(&r.self_ty)?;
            let args = r.args.iter().map(&mut sub).collect::<Result<Vec<_>, _>>()?;
            let trait_ref = ir::TraitRef {
                declaration: r.declaration,
                self_ty,
                args: args.into(),
            };
            predicates.push(match row {
                ir::Predicate::Implements(_) => ir::Predicate::Implements(trait_ref),
                ir::Predicate::ProjectionEq { projection, ty } => ir::Predicate::ProjectionEq {
                    projection: ir::AssociatedTypeProjection {
                        trait_ref,
                        item: projection.item,
                    },
                    ty: sub(ty)?,
                },
            });
        }
        Ok(tr::InstantiatedSchema {
            signature: ir::Signature {
                params,
                result,
                ref_contract: schema.signature.ref_contract.clone(),
            },
            predicates: predicates.into(),
        })
    }
    fn validate_nominal(
        &self,
        ty: &ir::Ty,
        env: &ir::GenericEnv,
        budget: &ir::PlanBudget,
    ) -> Result<(), tr::FactsError> {
        if !budget.same_authority(&self.budget) {
            return Err(tr::FactsError::invalid(
                budget,
                "schema budget authority mismatch",
            ));
        }
        fn walk(
            cx: &SchemaAdapter,
            ty: &ir::Ty,
            env: &ir::GenericEnv,
            b: &ir::PlanBudget,
            depth: usize,
            memo: &mut HashMap<ir::NominalAppId, usize>,
        ) -> Result<usize, String> {
            b.check_depth(depth)?;
            b.charge_work(1)?;
            let nominal = match ty {
                ir::Ty::Struct(t) => Some((t.def().id, t.def() as *const _ as usize)),
                ir::Ty::Enum(t) => Some((t.def().id, t.def() as *const _ as usize)),
                _ => None,
            };
            if let Some((id, pointer)) = nominal {
                let state = cx.state.lock().map_err(|_| "schema interner poisoned")?;
                let canonical = match state.nominals.ordered.get(id.index) {
                    Some(ir::Ty::Struct(t)) if matches!(ty, ir::Ty::Struct(_)) => {
                        t.def() as *const _ as usize
                    }
                    Some(ir::Ty::Enum(t)) if matches!(ty, ir::Ty::Enum(_)) => {
                        t.def() as *const _ as usize
                    }
                    _ => return Err("noncanonical nominal application".into()),
                };
                if canonical != pointer || id.snapshot != cx.registry.declarations[0].id.snapshot {
                    return Err("noncanonical nominal application".into());
                }
                drop(state);
                if let Some(height) = memo.get(&id) {
                    b.check_depth(depth + height)?;
                    return Ok(*height);
                }
            }
            let mut height = 0usize;
            let mut child = |t: &ir::Ty| -> Result<(), String> {
                height = height.max(walk(cx, t, env, b, depth + 1, memo)? + 1);
                Ok(())
            };
            match ty {
                ir::Ty::AssociatedType(projection) => {
                    let trait_info = cx.registry.declaration(projection.trait_ref.declaration)?;
                    let item_info = cx.registry.declaration(projection.item)?;
                    if trait_info.kind != ir::DeclKind::Trait
                        || item_info.kind != ir::DeclKind::AssociatedType
                        || item_info.owner != Some(trait_info.id)
                        || projection.trait_ref.args.len() != trait_info.type_params.len()
                    {
                        return Err("invalid raw associated projection owner or arity".into());
                    }
                    child(&projection.trait_ref.self_ty)?;
                    for argument in projection.trait_ref.args.iter() {
                        child(argument)?;
                    }
                }
                ir::Ty::Param(p) => {
                    b.charge_work(env.scopes().len())?;
                    if !env.contains(*p) {
                        return Err("parameter outside schema environment".into());
                    }
                }
                ir::Ty::Struct(t) => {
                    for a in t.def().type_args.iter() {
                        child(a)?;
                    }
                    for f in t.def().fields.iter() {
                        child(&f.ty)?;
                    }
                }
                ir::Ty::Enum(t) => {
                    for a in t.def().type_args.iter() {
                        child(a)?;
                    }
                    for v in t.def().variants.iter() {
                        b.charge_work(1)?;
                        for f in v.fields.iter() {
                            child(&f.ty)?;
                        }
                    }
                }
                ir::Ty::Tuple(v) => {
                    if v.is_empty() {
                        return Err("empty tuple must be Unit".into());
                    }
                    for t in v.iter() {
                        child(t)?;
                    }
                }
                ir::Ty::Array { element, len } => {
                    if *len > ir::MAX_ARRAY_LEN {
                        return Err("array length limit exceeded".into());
                    }
                    child(element)?;
                }
                ir::Ty::Ref { referent, .. } => child(referent)?,
                _ => {}
            }
            if let Some((id, _)) = nominal {
                b.retain_bytes(128)?;
                memo.insert(id, height);
            }
            Ok(height)
        }
        let layers = ir::root_reference_layers(ty).map_err(|e| self.error(e))?;
        let mut inner = ty;
        for _ in 0..layers {
            if let ir::Ty::Ref { referent, .. } = inner {
                inner = referent;
            }
        }
        walk(self, inner, env, budget, layers, &mut HashMap::new())
            .map(|_| ())
            .map_err(|e| self.error(e))
    }
}
fn raw_type_has_projection(
    ty: &ir::Ty,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<bool, String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    match ty {
        ir::Ty::AssociatedType(_) => Ok(true),
        ir::Ty::Tuple(fields) => {
            for field in fields.iter() {
                if raw_type_has_projection(field, budget, depth + 1)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        ir::Ty::Array { element, .. }
        | ir::Ty::Slice { element }
        | ir::Ty::Ref {
            referent: element, ..
        } => raw_type_has_projection(element, budget, depth + 1),
        ir::Ty::Struct(ty) => {
            for argument in ty
                .def()
                .type_args
                .iter()
                .chain(ty.def().fields.iter().map(|field| &field.ty))
            {
                if raw_type_has_projection(argument, budget, depth + 1)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        ir::Ty::Enum(ty) => {
            for argument in ty.def().type_args.iter().chain(
                ty.def()
                    .variants
                    .iter()
                    .flat_map(|variant| variant.fields.iter().map(|field| &field.ty)),
            ) {
                if raw_type_has_projection(argument, budget, depth + 1)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        _ => Ok(false),
    }
}
fn nominal_parts(ty: &ir::Ty) -> Option<(ir::DeclId, &[ir::Ty])> {
    match ty {
        ir::Ty::Struct(t) => Some((t.def().source_decl, &t.def().type_args)),
        ir::Ty::Enum(t) => Some((t.def().source_decl, &t.def().type_args)),
        _ => None,
    }
}
fn metered_type_equal(
    a: &ir::Ty,
    b: &ir::Ty,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<bool, String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    match (a, b) {
        (ir::Ty::AssociatedType(a), ir::Ty::AssociatedType(b)) => {
            if a.item != b.item
                || a.trait_ref.declaration != b.trait_ref.declaration
                || a.trait_ref.args.len() != b.trait_ref.args.len()
            {
                return Ok(false);
            }
            if !metered_type_equal(
                &a.trait_ref.self_ty,
                &b.trait_ref.self_ty,
                budget,
                depth + 1,
            )? {
                return Ok(false);
            }
            for (a, b) in a.trait_ref.args.iter().zip(b.trait_ref.args.iter()) {
                if !metered_type_equal(a, b, budget, depth + 1)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (ir::Ty::AssociatedType(_), _) | (_, ir::Ty::AssociatedType(_)) => Ok(false),
        (ir::Ty::Tuple(a), ir::Ty::Tuple(b)) => {
            if a.len() != b.len() {
                return Ok(false);
            }
            for (a, b) in a.iter().zip(b.iter()) {
                if !metered_type_equal(a, b, budget, depth + 1)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (ir::Ty::Array { element: a, len: n }, ir::Ty::Array { element: b, len: m }) => {
            Ok(n == m && metered_type_equal(a, b, budget, depth + 1)?)
        }
        (ir::Ty::Slice { element: a }, ir::Ty::Slice { element: b }) => {
            metered_type_equal(a, b, budget, depth + 1)
        }
        (
            ir::Ty::Ref {
                kind: a,
                referent: x,
            },
            ir::Ty::Ref {
                kind: b,
                referent: y,
            },
        ) => Ok(a == b && metered_type_equal(x, y, budget, depth + 1)?),
        (
            ir::Ty::RawPtr {
                kind: a,
                pointee: x,
            },
            ir::Ty::RawPtr {
                kind: b,
                pointee: y,
            },
        ) => Ok(a == b && metered_type_equal(x, y, budget, depth + 1)?),
        (ir::Ty::Struct(a), ir::Ty::Struct(b)) => Ok(a.def().id == b.def().id),
        (ir::Ty::Enum(a), ir::Ty::Enum(b)) => Ok(a.def().id == b.def().id),
        (
            ir::Ty::Tuple(_)
            | ir::Ty::Array { .. }
            | ir::Ty::Slice { .. }
            | ir::Ty::Ref { .. }
            | ir::Ty::Struct(_)
            | ir::Ty::Enum(_),
            _,
        )
        | (
            _,
            ir::Ty::Tuple(_)
            | ir::Ty::Array { .. }
            | ir::Ty::Slice { .. }
            | ir::Ty::Ref { .. }
            | ir::Ty::Struct(_)
            | ir::Ty::Enum(_),
        ) => Ok(false),
        _ => Ok(a == b),
    }
}
fn match_impl_type(
    pattern: &ir::Ty,
    actual: &ir::Ty,
    env: &ir::ParamEnv,
    bindings: &mut HashMap<ir::ParamId, ir::Ty>,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<bool, String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    if raw_type_has_projection(pattern, budget, depth)?
        || raw_type_has_projection(actual, budget, depth)?
    {
        return Err("unsupported: residual projection in impl matching".into());
    }
    if let ir::Ty::Param(param) = pattern
        && env.contains(*param)
    {
        if let Some(old) = bindings.get(param) {
            return metered_type_equal(old, actual, budget, depth + 1);
        }
        budget.retain_bytes(256)?;
        bindings.insert(*param, actual.clone());
        return Ok(true);
    }
    match (pattern, actual) {
        (ir::Ty::Tuple(a), ir::Ty::Tuple(b)) if a.len() == b.len() => {
            for (a, b) in a.iter().zip(b.iter()) {
                if !match_impl_type(a, b, env, bindings, budget, depth + 1)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (ir::Ty::Array { element: a, len: n }, ir::Ty::Array { element: b, len: m }) if n == m => {
            match_impl_type(a, b, env, bindings, budget, depth + 1)
        }
        _ => {
            if let (Some((a, aa)), Some((b, bb))) = (nominal_parts(pattern), nominal_parts(actual))
            {
                if a != b || aa.len() != bb.len() {
                    return Ok(false);
                }
                for (a, b) in aa.iter().zip(bb) {
                    if !match_impl_type(a, b, env, bindings, budget, depth + 1)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            } else {
                metered_type_equal(pattern, actual, budget, depth + 1)
            }
        }
    }
}
fn overlap_resolve(
    ty: &ir::Ty,
    bindings: &HashMap<ir::ParamId, ir::Ty>,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<ir::Ty, String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    if let ir::Ty::Param(p) = ty
        && let Some(value) = bindings.get(p)
    {
        return overlap_resolve(value, bindings, budget, depth + 1);
    }
    Ok(ty.clone())
}
fn overlap_occurs(
    param: ir::ParamId,
    ty: &ir::Ty,
    bindings: &HashMap<ir::ParamId, ir::Ty>,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<bool, String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    let ty = overlap_resolve(ty, bindings, budget, depth)?;
    match &ty {
        ir::Ty::AssociatedType(_) => {
            Err("unsupported: residual projection in overlap checking".into())
        }
        ir::Ty::Param(p) => Ok(*p == param),
        ir::Ty::Tuple(fields) => {
            for field in fields.iter() {
                if overlap_occurs(param, field, bindings, budget, depth + 1)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        ir::Ty::Array { element, .. } => {
            overlap_occurs(param, element, bindings, budget, depth + 1)
        }
        _ => {
            if let Some((_, args)) = nominal_parts(&ty) {
                for arg in args {
                    if overlap_occurs(param, arg, bindings, budget, depth + 1)? {
                        return Ok(true);
                    }
                }
            }
            Ok(false)
        }
    }
}
fn overlap_types(
    a: &ir::Ty,
    b: &ir::Ty,
    bindings: &mut HashMap<ir::ParamId, ir::Ty>,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<bool, String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    let a = overlap_resolve(a, bindings, budget, depth)?;
    let b = overlap_resolve(b, bindings, budget, depth)?;
    if raw_type_has_projection(&a, budget, depth)? || raw_type_has_projection(&b, budget, depth)? {
        return Err("unsupported: residual projection in overlap checking".into());
    }
    if metered_type_equal(&a, &b, budget, depth + 1)? {
        return Ok(true);
    }
    if let ir::Ty::Param(p) = a {
        if overlap_occurs(p, &b, bindings, budget, depth + 1)? {
            return Ok(false);
        }
        budget.retain_bytes(256)?;
        bindings.insert(p, b);
        return Ok(true);
    }
    if let ir::Ty::Param(p) = b {
        if overlap_occurs(p, &a, bindings, budget, depth + 1)? {
            return Ok(false);
        }
        budget.retain_bytes(256)?;
        bindings.insert(p, a);
        return Ok(true);
    }
    match (&a, &b) {
        (ir::Ty::Tuple(a), ir::Ty::Tuple(b)) if a.len() == b.len() => {
            for (a, b) in a.iter().zip(b.iter()) {
                if !overlap_types(a, b, bindings, budget, depth + 1)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (ir::Ty::Array { element: a, len: n }, ir::Ty::Array { element: b, len: m }) if n == m => {
            overlap_types(a, b, bindings, budget, depth + 1)
        }
        _ => {
            if let (Some((ad, aa)), Some((bd, bb))) = (nominal_parts(&a), nominal_parts(&b)) {
                if ad != bd || aa.len() != bb.len() {
                    return Ok(false);
                }
                for (a, b) in aa.iter().zip(bb) {
                    if !overlap_types(a, b, bindings, budget, depth + 1)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            } else {
                Ok(false)
            }
        }
    }
}
impl PlanCtx {
    fn validate_inherent_overlaps(&self, budget: &ir::PlanBudget) -> Result<(), String> {
        self.check_budget(budget)?;
        for (index, a) in self.registry.function_ids.iter().enumerate() {
            budget.charge_work(1)?;
            let a = self.declaration(*a)?;
            let Some(ao) = &a.path.associated else {
                continue;
            };
            if ao.trait_decl.is_some() {
                continue;
            }
            for b in &self.registry.function_ids[index + 1..] {
                budget.charge_work(1)?;
                let b = self.declaration(*b)?;
                let Some(bo) = &b.path.associated else {
                    continue;
                };
                if bo.trait_decl.is_some() {
                    continue;
                }
                budget.charge_work(1)?;
                if !self.registry.source_items_equal(a.path.item, b.path.item)?
                    || ao.nominal != bo.nominal
                {
                    continue;
                }
                let _scratch = budget.reserve_bytes(32 * 512 + 128)?;
                if overlap_types(
                    &ao.self_pattern,
                    &bo.self_pattern,
                    &mut HashMap::new(),
                    budget,
                    0,
                )? {
                    return Err(format!("duplicate overlapping associated item {}", a.name));
                }
            }
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    fn select_associated(
        &self,
        owner: &ir::Ty,
        name: &AstIdent,
        args: &[ir::Ty],
        caller: ir::FactsCtx<'_>,
        module: &ModuleId,
        dot: bool,
        budget: &ir::PlanBudget,
    ) -> Result<(ir::Callee, ir::Signature, ReceiverKind), String> {
        for traits_only in [false, true] {
            if let Some(found) = self.lookup_associated(
                owner,
                name,
                args,
                caller,
                module,
                dot,
                None,
                traits_only,
                budget,
            )? {
                return Ok(found);
            }
        }
        Err(format!(
            "no applicable associated item {}",
            self.program
                .names
                .spelling(*name)
                .map_err(native_adapter::name_diagnostic)?
        ))
    }
    fn select_method(
        &self,
        receiver: &ir::Ty,
        name: &AstIdent,
        args: &[ir::Ty],
        caller: ir::FactsCtx<'_>,
        module: &ModuleId,
        budget: &ir::PlanBudget,
    ) -> Result<MethodSelection, String> {
        let mut root = receiver;
        for depth in 0..=2 {
            budget.charge_work(1)?;
            for adjustment in [
                ReceiverAdjustment::Identity,
                ReceiverAdjustment::SharedAutoref,
                ReceiverAdjustment::MutAutoref,
            ] {
                budget.retain_bytes(128)?;
                let candidate = match adjustment {
                    ReceiverAdjustment::Identity => root.clone(),
                    ReceiverAdjustment::SharedAutoref => ir::Ty::Ref {
                        kind: ir::BorrowKind::Shared,
                        referent: Arc::new(root.clone()),
                    },
                    ReceiverAdjustment::MutAutoref => ir::Ty::Ref {
                        kind: ir::BorrowKind::Mut,
                        referent: Arc::new(root.clone()),
                    },
                };
                for traits_only in [false, true] {
                    let borrowed = match &candidate {
                        ir::Ty::Ref { kind, referent } => {
                            Some((ReceiverKind::Borrowed { kind: *kind }, referent.as_ref()))
                        }
                        _ => None,
                    };
                    for (mode, owner) in
                        std::iter::once((ReceiverKind::Value { mutable: false }, &candidate))
                            .chain(borrowed)
                    {
                        if let Some((target, signature, mode)) = self.lookup_associated(
                            owner,
                            name,
                            args,
                            caller,
                            module,
                            true,
                            Some(mode),
                            traits_only,
                            budget,
                        )? {
                            if !signature
                                .params
                                .first()
                                .map(|formal| metered_type_equal(formal, &candidate, budget, 0))
                                .transpose()?
                                .unwrap_or(false)
                            {
                                return Err("canonical receiver formal mismatch".into());
                            }
                            ir::root_reference_layers(&candidate)?;
                            return Ok(MethodSelection {
                                target,
                                signature,
                                mode,
                                depth,
                                adjustment,
                            });
                        }
                    }
                }
            }
            match root {
                ir::Ty::Ref { referent, .. } => root = referent,
                _ => break,
            }
        }
        Err(format!(
            "no applicable method {}",
            self.program
                .names
                .spelling(*name)
                .map_err(native_adapter::name_diagnostic)?
        ))
    }
}
fn generic_bindings(
    registry: &DeclRegistry,
    env: &ir::GenericEnv,
    budget: &ir::PlanBudget,
) -> Result<HashMap<GenericBindingKey, ir::Ty>, String> {
    budget.retain_bytes(env.total_arity() * 256 + 128)?;
    let mut bindings = HashMap::new();
    for scope in env.scopes() {
        let info = registry.declaration(scope.binder().declaration)?;
        if info.kind == ir::DeclKind::Trait {
            let values = symbolic_args(scope, budget)?;
            let (self_ty, actuals) = values
                .split_first()
                .ok_or("contract: trait Self binder arity")?;
            if actuals.len() != info.type_params.len() {
                return Err("contract: trait parameter binder arity".into());
            }
            bindings.insert(GenericBindingKey::SelfType, self_ty.clone());
            for (name, ty) in info.type_params.iter().zip(actuals) {
                budget.charge_work(1)?;
                if bindings
                    .insert(registry.parameter_key(*name)?, ty.clone())
                    .is_some()
                {
                    return Err("duplicate visible generic parameter".into());
                }
            }
            continue;
        }
        for (name, ty) in info.type_params.iter().zip(symbolic_args(scope, budget)?) {
            budget.charge_work(1)?;
            if bindings
                .insert(registry.parameter_key(*name)?, ty)
                .is_some()
            {
                return Err("duplicate visible generic parameter".into());
            }
        }
    }
    Ok(bindings)
}
fn append_implicit_sized(
    registry: &DeclRegistry,
    env: &ir::GenericEnv,
    rows: &mut Vec<ir::Predicate>,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    let sized = *registry
        .traits
        .get(&TraitLookupKey::Builtin(BuiltinTrait::Sized))
        .ok_or("missing canonical Sized identity")?;
    for scope in env.scopes() {
        let declaration = registry.declaration(scope.binder().declaration)?;
        let first = usize::from(declaration.kind == ir::DeclKind::Trait);
        for index in first..scope.arity() {
            budget.charge_work(1)?;
            if rows.len() >= 256 {
                return Err("predicate limit exceeded".into());
            }
            budget.retain_bytes(256)?;
            rows.push(ir::Predicate::Implements(ir::TraitRef {
                declaration: sized,
                self_ty: ir::Ty::Param(ir::ParamId {
                    owner: scope.binder(),
                    index: index as u16,
                }),
                args: Arc::from([]),
            }));
        }
    }
    Ok(())
}
fn symbolic_args(env: &ir::ParamEnv, budget: &ir::PlanBudget) -> Result<Vec<ir::Ty>, String> {
    budget.charge_work(env.arity())?;
    budget.retain_bytes(
        env.arity()
            .checked_mul(std::mem::size_of::<ir::Ty>())
            .ok_or("symbolic argument storage overflow")?,
    )?;
    let mut args = Vec::new();
    args.try_reserve_exact(env.arity()).map_err(|_| {
        // Force the shared planning ledger into its sticky resource state.
        let _ = budget.retain_bytes(usize::MAX);
        "resource: symbolic argument allocation failed".to_owned()
    })?;
    budget.retain_bytes(
        args.capacity()
            .saturating_sub(env.arity())
            .checked_mul(std::mem::size_of::<ir::Ty>())
            .ok_or("symbolic argument capacity overflow")?,
    )?;
    for index in 0..env.arity() {
        args.push(ir::Ty::Param(ir::ParamId {
            owner: env.binder(),
            index: index as u16,
        }));
    }
    Ok(args)
}
fn param_bindings(
    registry: &DeclRegistry,
    info: &DeclInfo,
    args: &[ir::Ty],
) -> Result<HashMap<GenericBindingKey, ir::Ty>, String> {
    if info.type_params.len() != args.len() {
        return Err(format!(
            "wrong generic arity for {}; omitted generic inference is unsupported",
            info.name
        ));
    }
    info.type_params
        .iter()
        .zip(args)
        .map(|(name, ty)| Ok((registry.parameter_key(*name)?, ty.clone())))
        .collect()
}

fn receiver_ty(receiver: ReceiverKind, self_ty: ir::Ty) -> ir::Ty {
    match receiver {
        ReceiverKind::Borrowed { kind } => ir::Ty::Ref {
            kind,
            referent: Arc::new(self_ty),
        },
        _ => self_ty,
    }
}
fn validate_root_carrier(
    ty: &ir::Ty,
    caller: ir::FactsCtx<'_>,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    caller.validate_ty_with_budget(ty, budget)?;
    ir::carrier_locations(ty, budget).map(|_| ())
}
fn validate_argument(
    ty: &ir::Ty,
    caller: ir::FactsCtx<'_>,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<(), String> {
    let _scratch = budget.reserve_bytes(4096 * (std::mem::size_of::<ir::NominalAppId>() + 128))?;
    let mut memo = HashMap::new();
    argument_height(ty, caller, budget, depth, &mut memo).map(|_| ())
}
fn argument_height(
    ty: &ir::Ty,
    caller: ir::FactsCtx<'_>,
    budget: &ir::PlanBudget,
    depth: usize,
    memo: &mut HashMap<ir::NominalAppId, usize>,
) -> Result<usize, String> {
    budget
        .check_depth(depth)
        .map_err(|e| format!("generic type nesting resource limit: {e}"))?;
    budget.charge_work(1)?;
    let id = match ty {
        ir::Ty::Struct(t) => Some(t.def().id),
        ir::Ty::Enum(t) => Some(t.def().id),
        _ => None,
    };
    if let Some(height) = id.and_then(|id| memo.get(&id).copied()) {
        budget
            .check_depth(depth + height)
            .map_err(|e| format!("generic type nesting resource limit: {e}"))?;
        return Ok(height);
    }
    let mut height = 0;
    let mut child = |ty: &ir::Ty| -> Result<(), String> {
        height = height.max(argument_height(ty, caller, budget, depth + 1, memo)? + 1);
        Ok(())
    };
    match ty {
        ir::Ty::AssociatedType(projection) => {
            child(&projection.trait_ref.self_ty)?;
            for argument in projection.trait_ref.args.iter() {
                child(argument)?;
            }
        }
        ir::Ty::Param(p) => match caller {
            ir::FactsCtx::Universal(env) if env.contains(*p) => {}
            ir::FactsCtx::Validated(facts) if facts.env().contains(*p) => {}
            _ => return Err("foreign or unresolved generic parameter".into()),
        },
        // `&'static str` is an origin-free leaf.
        ir::Ty::Ref { .. } if ty.is_shared_str() => {}
        ir::Ty::Ref { .. } => {
            return Err("reference-containing generic arguments are unsupported".into());
        }
        ir::Ty::Slice { .. } => {
            return Err("unsupported: slice generic arguments".into());
        }
        ir::Ty::Tuple(fields) => {
            for field in fields.iter() {
                child(field)?;
            }
        }
        ir::Ty::Array { element, .. } => child(element)?,
        ir::Ty::Struct(t) => {
            for arg in t.def().type_args.iter() {
                child(arg)?;
            }
            for field in t.def().fields.iter() {
                child(&field.ty)?;
            }
        }
        ir::Ty::Enum(t) => {
            for arg in t.def().type_args.iter() {
                child(arg)?;
            }
            for variant in t.def().variants.iter() {
                for field in variant.fields.iter() {
                    child(&field.ty)?;
                }
            }
        }
        _ => {}
    }
    if let Some(id) = id {
        if memo.len() >= 4096 {
            return Err("generic argument memo resource limit exceeded".into());
        }
        memo.insert(id, height);
    }
    Ok(height)
}
fn collect_source_params(
    names: &NameStore,
    ty: &Type,
    used: &mut std::collections::HashSet<NameKey>,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<(), String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    match ty {
        Type::Projection(projection) => match projection.as_ref() {
            SourceProjection::Qualified {
                self_ty,
                trait_path,
                ..
            } => {
                collect_source_params(names, self_ty, used, budget, depth + 1)?;
                if let TraitPath::Source(path) = trait_path {
                    for segment in &path.segments {
                        budget.charge_work(1)?;
                        for argument in &segment.args {
                            collect_source_params(names, argument, used, budget, depth + 1)?;
                        }
                        for binding in &segment.associated_types {
                            names
                                .check(binding.name)
                                .map_err(native_adapter::name_diagnostic)?;
                            collect_source_params(names, &binding.value, used, budget, depth + 1)?;
                        }
                    }
                }
            }
            SourceProjection::SelfItem { .. } => {}
        },
        Type::Param(p) => {
            used.insert(
                names
                    .key(*p, LookupClass::TypeParameter)
                    .map_err(native_adapter::name_diagnostic)?,
            );
        }
        Type::Apply(path) => {
            for s in &path.segments {
                for a in &s.args {
                    collect_source_params(names, a, used, budget, depth + 1)?;
                }
                for binding in &s.associated_types {
                    names
                        .check(binding.name)
                        .map_err(native_adapter::name_diagnostic)?;
                    collect_source_params(names, &binding.value, used, budget, depth + 1)?;
                }
            }
        }
        Type::Tuple(fields) => {
            for f in fields {
                collect_source_params(names, f, used, budget, depth + 1)?;
            }
        }
        Type::Array(t, _)
        | Type::Slice(t)
        | Type::RawPtr { pointee: t, .. }
        | Type::Ref { referent: t, .. } => {
            collect_source_params(names, t, used, budget, depth + 1)?
        }
        _ => {}
    }
    Ok(())
}
mod nominal_schema;
/// One immutable per-body view with a shared canonical application service.
mod type_check;
use type_check::*;
pub use type_check::{
    Signatures, TypeEnv, check, check_function, check_function_with_signatures, check_signatures,
    signature_table, type_callable_with_plan, type_env, type_function_with_env,
    type_function_with_plan, type_function_with_signatures,
};

#[cfg(test)]
mod registry_resource_tests {
    use super::*;

    #[test]
    fn empty_argument_type_segments_are_charged_before_traversal() {
        let mut builder = ProgramBuilder::new(
            std::path::Path::new("resource.rs"),
            "",
            &super::super::expansion_budget::ExpansionBudget::default(),
        )
        .unwrap();
        let name = builder.ident("Item").unwrap();
        let ty = Type::Apply(GenericPath {
            anchor: PathAnchor::Relative,
            resolved_use: None,
            segments: (0..128)
                .map(|_| PathSegment {
                    name,
                    args: Vec::new(),
                    associated_types: Vec::new(),
                })
                .collect(),
        });
        let budget = ir::PlanBudget::new(ir::PlanLimits {
            work: 1,
            ..Default::default()
        });
        assert!(source_type_storage(&ty, &budget, 0).is_err());
        let failure = budget.first_resource_failure().unwrap();
        assert_eq!(failure.kind, ir::ResourceBudgetKind::Work);
        assert_eq!(failure.used, 1);
        assert_eq!(failure.requested, 128);
        assert_eq!(failure.limit, 1);
        assert_eq!(budget.usage_snapshot().work, 1);
    }

    #[test]
    fn registry_maps_reserve_once_and_deny_before_allocating() {
        let denied = ir::PlanBudget::new(ir::PlanLimits {
            bytes: 0,
            ..Default::default()
        });
        let mut table = HashMap::<usize, usize>::new();
        assert!(reserve_registry_map(&mut table, 64, &denied).is_err());
        assert_eq!(table.capacity(), 0);
        let budget = ir::PlanBudget::default();
        reserve_registry_map(&mut table, 64, &budget).unwrap();
        let capacity = table.capacity();
        for key in 0..64 {
            table.insert(key, key);
        }
        assert_eq!(table.capacity(), capacity);
        assert!(budget.usage_snapshot().bytes >= capacity * std::mem::size_of::<(usize, usize)>());
        assert!(
            reserve_registry_map(&mut table, 1, &budget)
                .unwrap_err()
                .starts_with("contract:")
        );
    }

    #[test]
    fn diagnostic_storage_is_preflighted_before_copying() {
        let denied = ir::PlanBudget::new(ir::PlanLimits {
            bytes: 0,
            ..Default::default()
        });
        assert!(registry_diagnostic_copy("unchanged", &denied).is_err());
        let budget = ir::PlanBudget::default();
        let text = registry_diagnostic_copy("unchanged", &budget).unwrap();
        assert_eq!(text, "unchanged");
        assert_eq!(budget.usage_snapshot().bytes, text.capacity());
    }
}

#[cfg(test)]
mod associated_registry_tests {
    use super::*;

    fn source_plan() -> PlanCtx {
        let program = parse("trait A { type Out; } trait B { type Out; } struct S; impl A for S { type Out = u64; } impl B for S { type Out = u8; } fn use_a(x:<S as A>::Out)-><S as A>::Out{x} fn main(){}").unwrap();
        PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap()
    }

    #[test]
    fn associated_members_have_distinct_owned_persistent_identities_and_raw_schemas() {
        let plan = source_plan();
        let a = plan.registry.trait_associated_types[0][0];
        let b = plan.registry.trait_associated_types[1][0];
        assert_ne!(a, b);
        for (member, owner) in [
            (a, plan.registry.trait_ids[0]),
            (b, plan.registry.trait_ids[1]),
        ] {
            let info = plan.declaration(member).unwrap();
            assert_eq!(info.kind, ir::DeclKind::AssociatedType);
            assert_eq!(info.owner, Some(owner));
            assert!(Arc::ptr_eq(
                &info.env,
                &plan.declaration(owner).unwrap().env
            ));
            assert_eq!(info.param_env.arity(), 0);
        }
        match (
            plan.persistent_decl_path(a).unwrap().item(),
            plan.persistent_decl_path(b).unwrap().item(),
        ) {
            (
                PersistentDeclItem::AssociatedType {
                    trait_name: a,
                    name: x,
                },
                PersistentDeclItem::AssociatedType {
                    trait_name: b,
                    name: y,
                },
            ) => {
                assert_ne!(a, b);
                assert_eq!(x, y);
            }
            _ => panic!("missing associated persistent owner component"),
        }
        let function = plan.function_decl(0).unwrap();
        assert!(matches!(
            plan.raw_schema(function).unwrap().signature.result,
            ir::Ty::AssociatedType(_)
        ));
        assert_eq!(
            plan.function_schema(function).unwrap().signature.result,
            ir::Ty::Int(ir::IntTy::U64)
        );
    }

    #[test]
    fn associated_rhs_callback_is_raw_pair_owned_and_foreign_safe() {
        use tr::SchemaCx;
        let plan = source_plan();
        let adapter = SchemaAdapter {
            program: plan.program.clone(),
            catalog: plan.catalog.clone(),
            registry: plan.registry.clone(),
            state: plan.state.clone(),
            budget: plan.budget.clone(),
        };
        let implementation = plan.registry.impl_ids[0];
        let a = plan.registry.trait_associated_types[0][0];
        let b = plan.registry.trait_associated_types[1][0];
        assert_eq!(
            adapter.associated_type_rhs(implementation, a).unwrap(),
            ir::Ty::Int(ir::IntTy::U64)
        );
        assert!(adapter.associated_type_rhs(implementation, b).is_err());
        let foreign = source_plan();
        assert!(
            adapter
                .associated_type_rhs(foreign.registry.impl_ids[0], a)
                .is_err()
        );
        assert_eq!(plan.state.lock().unwrap().associated_type_rows.len(), 2);
    }

    #[test]
    fn normalized_publication_never_rewrites_authoritative_raw_assignment_rhs() {
        use tr::SchemaCx;
        let program = parse("trait A{type Out;}trait B{type Out;}struct S;impl A for S{type Out=<S as B>::Out;}impl B for S{type Out=u64;}fn f(x:<S as A>::Out)-><S as A>::Out{x}fn main(){}").unwrap();
        let plan = PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap();
        let adapter = SchemaAdapter {
            program: plan.program.clone(),
            catalog: plan.catalog.clone(),
            registry: plan.registry.clone(),
            state: plan.state.clone(),
            budget: plan.budget.clone(),
        };
        let raw = adapter
            .associated_type_rhs(
                plan.registry.impl_ids[0],
                plan.registry.trait_associated_types[0][0],
            )
            .unwrap();
        assert!(matches!(raw, ir::Ty::AssociatedType(_)));
        assert_eq!(
            plan.function_schema(plan.function_decl(0).unwrap())
                .unwrap()
                .signature
                .result,
            ir::Ty::Int(ir::IntTy::U64)
        );
        let after = adapter
            .associated_type_rhs(
                plan.registry.impl_ids[0],
                plan.registry.trait_associated_types[0][0],
            )
            .unwrap();
        assert!(metered_type_equal(&raw, &after, &plan.budget, 0).unwrap());
    }

    #[test]
    fn same_world_same_owner_member_transplant_is_not_original_source_authority() {
        let plan = source_plan();
        let function = plan.function_decl(0).unwrap();
        let original = &plan.program.functions[0];
        let Type::Projection(parameter) = &original.params[0].1 else {
            panic!("missing parameter projection")
        };
        let SourceProjection::Qualified {
            item: other_item, ..
        } = parameter.as_ref()
        else {
            panic!("missing qualified parameter projection")
        };
        let mut transplanted = original.result.clone();
        let Type::Projection(projection) = &mut transplanted else {
            panic!("missing result projection")
        };
        let SourceProjection::Qualified { item, .. } = projection.as_mut() else {
            panic!("missing qualified result projection")
        };
        *item = *other_item;
        let context = plan.registry.source_type_context(function).unwrap();
        assert!(
            plan.registry
                .validate_source_projection(context, projection, &plan.budget)
                .is_err()
        );
        let facts = plan.callable_facts(function).unwrap();
        assert!(
            plan.source_ty(
                &transplanted,
                &HashMap::new(),
                ir::FactsCtx::Validated(&facts),
                &plan.budget
            )
            .is_err()
        );
    }

    #[test]
    fn projection_alias_retains_definition_site_occurrence_in_signature_and_local() {
        let program = parse("trait A{type Out;}struct S;impl A for S{type Out=u64;}type Alias=<S as A>::Out;fn f(x:Alias)->Alias{let y:Alias=x;y}fn main(){}").unwrap();
        let plan = PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap();
        let function = plan.function_decl(0).unwrap();
        assert_eq!(
            plan.function_schema(function).unwrap().signature.result,
            ir::Ty::Int(ir::IntTy::U64)
        );
        let Type::Projection(projection) = &plan.program.functions[0].result else {
            panic!("alias did not retain projection")
        };
        let context = plan.registry.source_type_context(function).unwrap();
        plan.registry
            .validate_source_projection(context, projection, &plan.budget)
            .unwrap();
    }

    #[test]
    fn unused_source_assignments_and_duplicate_declarations_cannot_disappear() {
        for source in [
            "trait A{type Out;type Out;}fn main(){}",
            "trait A{type Out;}struct S;impl A for S{type Out=u64;type Out=u8;}fn main(){}",
            "trait A{type Out;}struct S;impl A for S{}fn main(){}",
            "trait A{type Out;}struct S;impl A for S{type Out=u64;type Extra=u8;}fn main(){}",
        ] {
            let program = parse(source).unwrap();
            assert!(
                PlanCtx::from_program(&program, &ir::PlanBudget::default()).is_err(),
                "{source}"
            );
        }
    }
}

#[cfg(test)]
mod equality_registry_tests {
    use super::*;

    fn equality_plan() -> PlanCtx {
        let program = parse("trait Source<A>{type Out;type Other;}fn f<T:Source<u8,Out=u64,Out=u64>>(x:T)->T{x}fn refs(x:&u64){}fn main(){}").unwrap();
        PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap()
    }

    #[test]
    fn equality_normalization_keeps_raw_callable_projection_and_requirement() {
        let program = parse("trait Source{type Out;}fn f<T:Source<Out=u64>>(x:T)-><T as Source>::Out{0u64}fn main(){}").unwrap();
        let plan = PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap();
        let declaration = plan.function_decl(0).unwrap();
        let raw = plan.raw_schema(declaration).unwrap();
        assert!(matches!(raw.signature.result, ir::Ty::AssociatedType(_)));
        assert!(
            raw.predicates
                .iter()
                .any(|row| matches!(row, ir::Predicate::ProjectionEq { .. }))
        );
        assert_eq!(
            plan.function_schema(declaration).unwrap().signature.result,
            ir::Ty::Int(ir::IntTy::U64)
        );
        assert!(matches!(
            plan.raw_schema(declaration).unwrap().signature.result,
            ir::Ty::AssociatedType(_)
        ));
    }

    #[test]
    fn repeated_equalities_retain_full_application_in_raw_rows() {
        let plan = equality_plan();
        let schema = plan.raw_schema(plan.function_decl(0).unwrap()).unwrap();
        let equalities = schema
            .predicates
            .iter()
            .filter_map(|row| match row {
                ir::Predicate::ProjectionEq { projection, ty } => Some((projection, ty)),
                ir::Predicate::Implements(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(equalities.len(), 2);
        for (projection, ty) in equalities {
            assert_eq!(projection.trait_ref.declaration, plan.registry.trait_ids[0]);
            assert_eq!(
                projection.trait_ref.args.as_ref(),
                &[ir::Ty::Int(ir::IntTy::U8)]
            );
            assert!(matches!(projection.trait_ref.self_ty, ir::Ty::Param(_)));
            assert_eq!(projection.item, plan.registry.trait_associated_types[0][0]);
            assert_eq!(*ty, ir::Ty::Int(ir::IntTy::U64));
        }
    }

    #[test]
    fn same_owner_identical_binding_transplant_is_not_source_authority() {
        let plan = equality_plan();
        let declaration = plan.function_decl(0).unwrap();
        let original = &plan.program.functions[0].bounds[0].traits[0];
        plan.validate_source_bound_path(declaration, original, &plan.budget)
            .unwrap();
        let mut changed = original.clone();
        let TraitPath::Source(path) = &mut changed else {
            panic!("source bound")
        };
        let bindings = &mut path.segments.last_mut().unwrap().associated_types;
        bindings[0] = bindings[1].clone();
        assert!(
            plan.validate_source_bound_path(declaration, &changed, &plan.budget)
                .is_err()
        );
        let TraitPath::Source(path) = &mut changed else {
            unreachable!()
        };
        path.segments.last_mut().unwrap().associated_types.clear();
        assert!(
            plan.validate_source_bound_path(declaration, &changed, &plan.budget)
                .is_err()
        );
    }

    #[test]
    fn nonbound_application_cannot_discard_equalities() {
        let plan = equality_plan();
        let declaration = plan.function_decl(0).unwrap();
        let info = plan.declaration(declaration).unwrap();
        let bindings = generic_bindings(&plan.registry, &info.env, &plan.budget).unwrap();
        let bound = &plan.program.functions[0].bounds[0];
        let owner = bindings
            .get(&plan.registry.parameter_key(bound.parameter).unwrap())
            .unwrap();
        let application = plan
            .state
            .lock()
            .unwrap()
            .source_trait_application(
                &plan.registry,
                &bound.traits[0],
                owner,
                &bindings,
                ir::FactsCtx::Universal(&info.env),
                &plan.budget,
            )
            .unwrap();
        assert!(
            application
                .reject_equalities()
                .unwrap_err()
                .starts_with("unsupported:")
        );
    }

    #[test]
    fn later_unknown_binding_precedes_unsupported_reference_argument() {
        let plan = equality_plan();
        let declaration = plan.function_decl(0).unwrap();
        let info = plan.declaration(declaration).unwrap();
        let bindings = generic_bindings(&plan.registry, &info.env, &plan.budget).unwrap();
        let bound = &plan.program.functions[0].bounds[0];
        let owner = bindings
            .get(&plan.registry.parameter_key(bound.parameter).unwrap())
            .unwrap();
        let mut path = bound.traits[0].clone();
        let TraitPath::Source(source) = &mut path else {
            panic!("source bound")
        };
        let segment = source.segments.last_mut().unwrap();
        segment.args[0] = plan.program.functions[1].params[0].1.clone();
        segment.associated_types[1].name = bound.parameter;
        let error = plan
            .state
            .lock()
            .unwrap()
            .source_trait_application(
                &plan.registry,
                &path,
                owner,
                &bindings,
                ir::FactsCtx::Universal(&info.env),
                &plan.budget,
            )
            .err()
            .expect("unknown member must fail");
        assert!(error.contains("unknown associated type item"), "{error}");
        assert!(!error.starts_with("unsupported:"));
    }
}

#[cfg(test)]
mod const_declaration_tests {
    use super::*;

    #[test]
    fn const_declaration_qualifier_survives_resolution_and_runtime_publication() {
        let source = "mod m{pub const fn forward<T>(value:T)->T{value}}fn main(){let a=m::forward::<u64>(7u64);if a==7u64{println!(\"forwarded\");}}";
        let unresolved = parse_unresolved(source).unwrap();
        assert_eq!(unresolved.program.functions[0].constness, Constness::Const);
        let program = resolve(&unresolved).unwrap();
        assert_eq!(program.functions[0].name.anchor(), PathAnchor::Crate);
        assert_eq!(
            program.functions[0]
                .name
                .display(program.names.as_ref())
                .unwrap()
                .as_str(),
            "crate::m::forward"
        );
        assert_eq!(program.functions[0].constness, Constness::Const);
        assert_eq!(program.functions[1].constness, Constness::Runtime);
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let declaration = plan.function_decl(0).unwrap();
        assert!(matches!(
            plan.callable_provider(declaration).unwrap(),
            CallableProvider::Source { .. }
        ));
        assert!(matches!(
            plan.publish_callable_body(declaration, &budget).unwrap(),
            PublishedCallableBody::Source(_)
        ));
        let runtime = source.replacen("const fn", "fn", 1);
        let baseline = super::super::compile(runtime, 1, false).unwrap();
        for workers in [1, 8] {
            for check_only in [true, false] {
                let compiled = super::super::compile(source.into(), workers, check_only).unwrap();
                assert_eq!(compiled.executable.is_none(), check_only);
                if !check_only {
                    assert_eq!(compiled.executable, baseline.executable);
                }
            }
        }
    }

    #[test]
    fn unused_const_bodies_cannot_launder_effects_calls_shadowing_or_destruction() {
        for declaration in [
            "const fn f<T>(x:T){}",
            "const fn f<T>(x:T)->(T,T){(x,x)}",
            "const fn f(x:u8)->u8{other(x)}",
            "const fn f(x:u8)->u8{let x=other(x);x}",
            "const fn f(x:u8)->u8{{x}}",
            "const fn f(x:u8)->u8{return x;}",
            "const fn f(x:u8)->u8{println!(\"effect\");x}",
            "const fn f(x:u8,y:u8)->u8{x}",
            "const fn f(x:u8)->u64{x}",
        ] {
            let source = format!("fn other(x:u8)->u8{{x}}{declaration}fn main(){{}}");
            let program = parse(&source).unwrap();
            let error = PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap_err();
            assert!(
                error.starts_with("unsupported: const fn"),
                "{declaration}: {error}"
            );
            for workers in [1, 8] {
                for check_only in [true, false] {
                    assert!(super::super::compile(source.clone(), workers, check_only).is_err());
                }
            }
        }
        // The const gate must not narrow ordinary runtime function bodies.
        check(&parse("fn f(x:u8)->u8{let y=x;y}fn main(){}").unwrap()).unwrap();
    }

    #[test]
    fn const_required_contexts_and_associated_qualifiers_remain_unsupported() {
        for source in [
            "const VALUE:u8=1;fn main(){}",
            "fn main(){let x=const {1};}",
            "const fn id(x:u8)->u8{x}fn main(){let x=[0u8;id(1u8)];}",
            "const fn id(x:u8)->u8{x}fn main(){let x:[u8;id(1u8)]=[0u8];}",
            "fn f<const N:usize>(){}fn main(){}",
            "struct S{x:u8}impl S{const fn f(x:u8)->u8{x}}fn main(){}",
            "trait T{const fn f(x:u8)->u8;}fn main(){}",
        ] {
            let error = parse(source).unwrap_err();
            assert!(error.contains("unsupported"), "{source}: {error}");
        }
        // The original private attributes still belong to the source-policy
        // owner. Const declaration syntax must not silently enable them.
        assert!(parse("#[rustc_diagnostic_item=\"convert_identity\"]pub const fn identity<T>(x:T)->T{x}fn main(){}").unwrap_err().contains("unsupported"));
    }

    #[test]
    fn array_const_expression_boundary_preserves_malformed_literal_syntax() {
        use super::super::source::SourceErrorKind;
        for source in [
            "fn main(){let x=[0u8;id(1u8)];}",
            "fn main(){let x:[u8;id(1u8)]=[0u8];}",
            "fn main(){let x=[0u8;1+2];}",
            "fn main(){let x:[u8;1+2]=[0u8];}",
        ] {
            assert_eq!(
                parse_syntax_unprepared(source).unwrap_err().kind,
                SourceErrorKind::Unsupported,
                "{source}"
            );
        }
        for source in [
            "fn main(){let x=[0u8;0x];}",
            "fn main(){let x:[u8;0x]=[0u8];}",
            "fn main(){let x=[0u8;id(];}",
            "fn main(){let x=[0u8;1+];}",
            "fn main(){let x=[0u8;];}",
            "fn f<const>(){}fn main(){}",
            "fn f<const N>(){}fn main(){}",
            "fn f<const N:>(){}fn main(){}",
            "fn f<const N:usize extra>(){}fn main(){}",
        ] {
            assert_eq!(
                parse_syntax_unprepared(source).unwrap_err().kind,
                SourceErrorKind::Syntax,
                "{source}"
            );
        }
    }

    #[test]
    fn constness_changes_cannot_reuse_an_existing_planning_declaration() {
        for source in [
            "const fn f<T>(x:T)->T{x}fn main(){}",
            "fn f<T>(x:T)->T{x}fn main(){}",
        ] {
            let mut program = parse(source).unwrap();
            let budget = ir::PlanBudget::default();
            let plan = PlanCtx::from_program(&program, &budget).unwrap();
            program.functions[0].constness = match program.functions[0].constness {
                Constness::Runtime => Constness::Const,
                Constness::Const => Constness::Runtime,
            };
            let error =
                type_callable_with_plan(&program, plan.function_decl(0).unwrap(), &plan, &budget)
                    .unwrap_err();
            assert_eq!(
                error,
                "source callable signature differs from its immutable planning declaration"
            );
        }
        let mut program = parse("trait T{fn f(x:u8)->u8{x}}fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let declaration = plan
            .registry
            .function_ids
            .iter()
            .zip(&plan.registry.callable_sources)
            .find_map(|(id, source)| {
                matches!(source, CallableSource::TraitDefault { .. }).then_some(*id)
            })
            .unwrap();
        program.traits[0].items[0].signature.constness = Constness::Const;
        assert!(
            type_callable_with_plan(&program, declaration, &plan, &budget)
                .unwrap_err()
                .contains("signature constness differs")
        );
    }

    #[test]
    fn public_ast_cannot_bypass_associated_const_policy_or_generic_moves() {
        let mut program =
            parse("struct S{x:u8}impl S{fn f(x:u8)->u8{x}}trait T{fn f(x:u8)->u8{x}}fn main(){}")
                .unwrap();
        assert_eq!(
            program.traits[0].items[0].signature.constness,
            Constness::Runtime
        );
        program.impls[0].items[0].function.constness = Constness::Const;
        assert!(
            PlanCtx::from_program(&program, &ir::PlanBudget::default())
                .unwrap_err()
                .contains("const associated")
        );
        program.impls[0].items[0].function.constness = Constness::Runtime;
        program.traits[0].items[0]
            .default
            .as_mut()
            .unwrap()
            .constness = Constness::Const;
        assert!(
            PlanCtx::from_program(&program, &ir::PlanBudget::default())
                .unwrap_err()
                .contains("const associated")
        );
        program.traits[0].items[0]
            .default
            .as_mut()
            .unwrap()
            .constness = Constness::Runtime;
        program.traits[0].items[0].signature.constness = Constness::Const;
        assert!(
            PlanCtx::from_program(&program, &ir::PlanBudget::default())
                .unwrap_err()
                .contains("const associated")
        );
        let source = "struct S{x:u8}const fn f<T>(x:T)->T{x}fn main(){let s=S{x:1u8};let moved=f::<S>(s);let invalid=s.x;}";
        assert!(super::super::compile(source.into(), 1, true).is_err());
    }

    #[test]
    fn syntax_only_const_admission_does_not_claim_body_validity_or_ctfe() {
        let forwarding = "const fn unused(value:i32)->i32{value}";
        let syntax = parse_syntax_unprepared(forwarding).unwrap();
        assert_eq!(syntax.program.functions[0].constness, Constness::Const);
        let effectful = "fn runtime(value:i32)->i32{value}const fn unused(value:i32)->i32{runtime(value)}fn main(){}";
        // Raw syntax checking must not acquire semantic obligations: inactive
        // items use this seam too. Active universal checking remains separate.
        assert!(parse_syntax_unprepared(effectful).is_ok());
        assert!(
            PlanCtx::from_program(&parse(effectful).unwrap(), &ir::PlanBudget::default())
                .unwrap_err()
                .starts_with("unsupported: const fn")
        );
        for source in [
            format!("#[cfg(any())] {forwarding}fn main(){{}}"),
            "#[cfg(any())] const fn unused(value:i32)->i32{unknown(value)}fn main(){}".into(),
        ] {
            for workers in [1, 8] {
                for check_only in [true, false] {
                    assert!(super::super::compile(source.clone(), workers, check_only).is_ok());
                }
            }
        }
    }
}

#[cfg(test)]
mod typed_tests;

#[cfg(test)]
mod prepared_location_tests {
    use super::*;

    fn parser_parts(tokens: NativeParserInput) -> (NativeTokenBuffer, NameStoreBuilder) {
        let (tokens, session) = tokens.into_parts();
        let names = NameStoreBuilder::new(session, super::super::token::Edition::E2024).unwrap();
        (tokens, names)
    }

    #[test]
    fn token_bytes_preserve_unicode_comments_strings_and_crlf() {
        let text = "/* λ */\r\n\"é\r\n水\" >>=";
        let tokens = lex(text).unwrap();
        let string_start = text.find('"').unwrap();
        let angle_start = text.find(">>=").unwrap();
        assert_eq!(tokens[0].whole.bytes(), string_start..angle_start - 1);
        assert_eq!(tokens[0].text, "é\n水");
        assert_eq!((tokens[0].line, tokens[0].column), (2, 1));
        assert_eq!(tokens[1].whole.bytes(), angle_start..angle_start + 3);
        assert_eq!((tokens[1].line, tokens[1].column), (3, 4));
        assert_eq!(tokens[2].whole.bytes(), text.len()..text.len());
    }

    #[test]
    fn typed_lexical_errors_cover_complete_utf8_scalars_and_eof() {
        let text = "/* 水 */\r\né";
        let error = lex(text).unwrap_err();
        assert_eq!(error.bytes, text.len() - 2..text.len());
        assert_eq!((error.line, error.column), (2, 1));
        let text = "\"水\\";
        let error = lex(text).unwrap_err();
        assert_eq!(error.bytes, text.len()..text.len());
        assert!(error.message.contains("unsupported string escape"));
    }

    #[test]
    fn split_angles_and_shared_references_keep_exact_remaining_bytes() {
        for (text, expected) in [(">>", ">"), (">=", "="), (">>=", ">=")] {
            let tokens = lex(text).unwrap();
            let (tokens, mut names) = parser_parts(tokens);
            let mut parser = Parser::new(&tokens, &mut names).unwrap();
            parser.close_angle().unwrap();
            assert_eq!(parser.token().text, expected);
            assert_eq!(parser.token().bytes, 1..text.len());
            if text == ">>=" {
                parser.close_angle().unwrap();
                assert_eq!(parser.token().text, "=");
                assert_eq!(parser.token().bytes, 2..3);
            }
        }
        let tokens = lex("&&&&").unwrap();
        let (tokens, mut names) = parser_parts(tokens);
        let mut parser = Parser::new(&tokens, &mut names).unwrap();
        let error = parser.ty().unwrap_err();
        assert_eq!(tokens[0].whole.bytes(), 0..2);
        assert_eq!(tokens[1].whole.bytes(), 2..4);
        assert_eq!(error.bytes, 4..4);
    }

    #[test]
    fn cursor_array_count_rewind_preserves_categories_spans_and_counters() {
        use super::super::source::SourceErrorKind;
        for (text, kind, bytes) in [
            ("1+2]", SourceErrorKind::Unsupported, 3..4),
            ("1+]", SourceErrorKind::Syntax, 2..3),
            ("1+2", SourceErrorKind::Syntax, 3..3),
        ] {
            let tokens = lex(text).unwrap();
            let (tokens, mut names) = parser_parts(tokens);
            let mut parser = Parser::new(&tokens, &mut names).unwrap();
            parser.use_expansions = 7;
            parser.use_expansion_bytes = 19;
            let error = parser.array_number("array length").unwrap_err();
            assert_eq!(error.kind, kind, "{text}");
            assert_eq!(error.bytes, bytes, "{text}");
            assert_eq!(parser.use_expansions, 7);
            assert_eq!(parser.use_expansion_bytes, 19);
        }
    }

    #[test]
    fn cursor_nested_reference_and_angle_parsing_preserves_original_tokens() {
        for text in ["&&i64", "& &i64", "&&&&i64", "Outer<Inner<i64>>="] {
            let tokens = lex(text).unwrap();
            let original = format!("{tokens:?}");
            let (tokens, mut names) = parser_parts(tokens);
            let mut parser = Parser::new(&tokens, &mut names).unwrap();
            let ty = parser.ty().unwrap();
            if text.starts_with("Outer") {
                assert!(matches!(ty, Type::Apply(_)));
                assert_eq!(parser.token().text, "=");
                assert_eq!(parser.token().bytes, text.len() - 1..text.len());
            } else {
                let mut referent = &ty;
                let mut references = 0;
                while let Type::Ref {
                    referent: inner, ..
                } = referent
                {
                    references += 1;
                    referent = inner;
                }
                assert_eq!(references, text.bytes().filter(|&b| b == b'&').count());
                assert!(parser.is(""));
            }
            assert_eq!(format!("{tokens:?}"), original);
        }
    }

    #[test]
    fn prepared_parser_rejects_a_token_crossing_origin_segments() {
        let text = "fn main() {}";
        let mut prepared = super::super::source::prepare_text(
            "crossing.rs",
            text,
            &super::super::source::SourceOptions::default(),
        )
        .unwrap();
        let mut first = prepared.origins[0].clone();
        let mut second = first.clone();
        first.prepared = 0..1;
        first.origin.hi = first.origin.lo + 1;
        second.prepared = 1..text.len();
        second.origin.lo += 1;
        prepared.origins = vec![first, second];
        let error = parse_prepared(&prepared).err().unwrap();
        assert!(error.primary.is_none());
        assert_eq!(error.kind, super::super::source::SourceErrorKind::Contract);
        assert!(error.message.contains("segment boundary"));
    }

    #[test]
    fn syntax_only_seam_never_prepares_or_resolves() {
        assert!(parse_syntax_unprepared("fn f() { unknown(); }").is_ok());
        assert!(parse_syntax_unprepared("#[cfg(any())] fn f() {}").is_err());
        let error = parse_syntax_unprepared("fn f() { let = 1; }")
            .err()
            .unwrap();
        assert_eq!(error.bytes, 13..14);
        assert_eq!(
            error.to_string(),
            "1:14: expected ASCII identifier, found \"=\""
        );
        assert_eq!(
            parse_unresolved("fn f() { let = 1; }").err().unwrap(),
            error.to_string(),
        );
    }

    #[test]
    fn prepared_diagnostics_preserve_syntax_unsupported_and_limit_categories() {
        use super::super::source::{SourceErrorKind, SourceOptions, prepare_text};
        let syntax = parse_syntax_unprepared("fn f() { let = 1; }")
            .err()
            .unwrap();
        assert_eq!(syntax.kind, SourceErrorKind::Syntax);
        let unsupported = parse_syntax_unprepared("fn f(x: !) {}").err().unwrap();
        assert_eq!(unsupported.kind, SourceErrorKind::Unsupported);
        for text in [
            "async fn unused() {}",
            "fn unused() { let value = r\"raw\"; }",
        ] {
            assert_eq!(
                parse_syntax_unprepared(text).err().unwrap().kind,
                SourceErrorKind::Unsupported,
                "{text}",
            );
        }
        let invalid = lex("`").unwrap_err();
        assert_eq!(invalid.kind, SourceErrorKind::Syntax);
        assert!(!invalid.message.contains("unsupported"));
        let expression = std::iter::repeat_n("1", 140).collect::<Vec<_>>().join("+");
        let text = format!("fn f() -> i64 {{ {expression} }}");
        let prepared = prepare_text("depth.rs", &text, &SourceOptions::default()).unwrap();
        let limit = parse_prepared(&prepared).err().unwrap();
        assert_eq!(limit.kind, SourceErrorKind::Limit);
        assert!(
            limit.primary.is_none(),
            "depth validation has no precise syntax location"
        );
    }

    #[test]
    fn prepared_syntax_errors_map_original_unicode_crlf_positions() {
        let text = "/* 水 */\r\nfn bad() { let = 1; }";
        let prepared = super::super::source::prepare_text(
            "unicode.rs",
            text,
            &super::super::source::SourceOptions::default(),
        )
        .unwrap();
        let error = parse_prepared(&prepared).err().unwrap();
        let primary = error.primary.unwrap();
        assert_eq!(primary.lo as usize, text.find('=').unwrap());
        assert_eq!(primary.hi, primary.lo + 1);
        assert!(error.message.contains("expected ASCII identifier"));
    }
}

#[cfg(test)]
mod char_tests;
#[cfg(test)]
mod parser_tests;

fn catalog_decl_info(entry: &tr::CatalogDecl, budget: &ir::PlanBudget) -> Result<DeclInfo, String> {
    let info = &entry.info;
    clone_visibility(&info.visibility, budget)?;
    let receiver = match info.receiver {
        ir::ReceiverMode::None => ReceiverKind::None,
        ir::ReceiverMode::Value => ReceiverKind::Value { mutable: false },
        ir::ReceiverMode::Borrowed { kind } => ReceiverKind::Borrowed { kind },
    };
    Ok(DeclInfo {
        receiver,
        owner: info.owner,
        id: entry.declaration,
        kind: info.kind,
        name: entry.name.into(),
        path: DeclSemanticPath {
            module: info.module,
            item: match entry.origin {
                tr::DeclOrigin::CoreCatalog { item, .. } => DeclItemName::Catalog(item),
                _ => return Err("contract: catalog source origin".into()),
            },
            kind: info.kind,
            associated: None,
        },
        source_index: usize::MAX,
        param_env: info.own_env.clone(),
        type_params: Arc::from([]),
        env: info.env.clone(),
        visibility: info.visibility.clone(),
    })
}
mod source_clone;
use source_clone::*;

#[derive(Clone, Copy)]
enum ReceiverAdjustment {
    Identity,
    SharedAutoref,
    MutAutoref,
}
struct MethodSelection {
    target: ir::Callee,
    signature: ir::Signature,
    mode: ReceiverKind,
    depth: usize,
    adjustment: ReceiverAdjustment,
}
