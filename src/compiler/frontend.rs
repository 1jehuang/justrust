//! Native, deliberately restricted Rust frontend. No external compiler is invoked.
//!
//! Syntax diagnostics have one-based line/column positions. The span-free public
//! AST means semantic diagnostics identify functions and names instead of spans.
//! Explicit semicolons discard expression values, while omitted-semicolon
//! blocklike statements require unit. While loops do not establish return flow.
//! Numeric literals support primitive integers through 128 bits.
//! Inline modules, named nonempty braced structs, nonempty unit/tuple/braced enums, ordinary visibility, and
//! module-level named/grouped imports have separate type/value namespaces. Named
//! structs and enums are nominal and non-Copy. Match admits scalar literals,
//! top-level variants with irrefutable payload patterns and whole-value
//! bindings/wildcards with finite exhaustiveness. Nested enum patterns require
//! a single-variant enum. Tuple-variant rest patterns remain unsupported.
//! Move checking belongs to CFG analysis, not
//! this semantic checker. Restricted visibility names lexical ancestor modules.
//! External modules, glob imports, and block-local items are not
//! supported. An executable requires a declared root main, not an imported alias.
use super::ir;
use super::traits as tr;
use std::collections::HashMap;

mod token_cursor;
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
        if !program
            .functions
            .iter()
            .any(|function| function.name == "main")
        {
            program
                .functions
                .extend(parse("fn main(){}").unwrap().functions);
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
        let inventory =
            SourceInventory::build(function, original.identity.clone(), &budget).unwrap();
        let cap = CanonicalSourceCapability {
            plan: original.plan.clone(),
            declaration: original.declaration,
            facts: original.facts.clone(),
            inventory,
            calls: Default::default(),
            builtin_lens: Default::default(),
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
    ordinary: Option<String>,
    application: Option<GenericPath>,
    argument_binding: Option<sr::SourceNode>,
    argument_borrow: Option<ir::BorrowKind>,
    direct_let: Option<sr::SourceNode>,
    args_pointer: usize,
}
// Addresses are only a transient index into the exact owned AST during checking.
// They are stripped before OriginalSource is retained.
struct SourceInventory {
    owner: sr::OriginalBodyId,
    calls: Vec<SourceCallSite>,
    bindings: Vec<SourceBindingSite>,
    call_pointers: HashMap<usize, usize>,
    statement_pointers: HashMap<usize, usize>,
    scopes: Vec<HashMap<String, sr::SourceNode>>,
    root_block_pointer: usize,
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
        Type::Param(name) | Type::Named(name) => sr::work(budget, name.len())?,
        Type::Ref {
            region, referent, ..
        } => {
            if let Some(name) = region {
                sr::work(budget, name.len())?;
            }
            compare_source_type_work(referent, budget, at + 1)?;
        }
        Type::Array(element, _) | Type::Slice(element) => {
            compare_source_type_work(element, budget, at + 1)?
        }
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
        sr::work(budget, segment.name.len())?;
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
fn copy_source_string(value: &str, budget: &ir::PlanBudget) -> Result<String, FactsError> {
    sr::work(budget, value.len())?;
    let mut result = String::new();
    result
        .try_reserve_exact(value.len())
        .map_err(|_| FactsError::resource(budget, "source string allocation"))?;
    result.push_str(value);
    Ok(result)
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
        segments.push(PathSegment {
            name: copy_source_string(&segment.name, budget)?,
            args,
        });
    }
    Ok(GenericPath { segments })
}
fn copy_source_type(ty: &Type, budget: &ir::PlanBudget, at: usize) -> Result<Type, FactsError> {
    sr::depth(budget, at)?;
    sr::work(budget, 1)?;
    Ok(match ty {
        Type::Apply(path) => Type::Apply(copy_source_path(path, budget, at + 1)?),
        Type::Param(name) => Type::Param(copy_source_string(name, budget)?),
        Type::Named(name) => Type::Named(copy_source_string(name, budget)?),
        Type::Ref {
            kind,
            region,
            referent,
        } => Type::Ref {
            kind: *kind,
            region: region
                .as_ref()
                .map(|r| copy_source_string(r, budget))
                .transpose()?,
            referent: Box::new(copy_source_type(referent, budget, at + 1)?),
        },
        Type::Array(element, len) => {
            Type::Array(Box::new(copy_source_type(element, budget, at + 1)?), *len)
        }
        Type::Slice(element) => Type::Slice(Box::new(copy_source_type(element, budget, at + 1)?)),
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
        sr::work(budget, segment.name.len())?;
        sr::retain(budget, segment.name.len())?;
        for ty in &segment.args {
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
        Type::Param(name) | Type::Named(name) => {
            sr::work(budget, name.len())?;
            sr::retain(budget, name.len())?;
        }
        Type::Ref {
            region, referent, ..
        } => {
            if let Some(name) = region {
                sr::work(budget, name.len())?;
                sr::retain(budget, name.len())?;
            }
            retain_source_type(referent, budget, depth + 1)?;
        }
        Type::Array(element, _) | Type::Slice(element) => {
            retain_source_type(element, budget, depth + 1)?
        }
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
        name: &str,
        node: sr::SourceNode,
        annotation: Option<&Type>,
        parameter: Option<usize>,
        statement: Option<usize>,
        budget: &ir::PlanBudget,
    ) -> Result<(), FactsError> {
        sr::work(budget, name.len())?;
        sr::retain(
            budget,
            name.len()
                .checked_add(256)
                .ok_or_else(|| FactsError::resource(budget, "source binding storage overflow"))?,
        )?;
        if let Some(ty) = annotation {
            retain_source_type(ty, budget, 0)?;
        }
        let annotation = annotation
            .map(|ty| copy_source_type(ty, budget, 0))
            .transpose()?;
        reserve_source_map(self.scopes.last_mut().unwrap(), budget)?;
        self.scopes
            .last_mut()
            .unwrap()
            .insert(copy_source_string(name, budget)?, node.clone());
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
        name: &str,
        budget: &ir::PlanBudget,
    ) -> Result<Option<sr::SourceNode>, FactsError> {
        for scope in self.scopes.iter().rev() {
            sr::work(budget, name.len())?;
            sr::work(budget, 1)?;
            if let Some(node) = scope.get(name) {
                return Ok(Some(node.clone()));
            }
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
            Pattern::Binding { name, .. } | Pattern::Path(name) => {
                self.bind(name, node.clone(), None, None, None, budget)?
            }
            Pattern::Apply { pattern, .. } => {
                self.shadow_pattern(pattern, node, budget, depth + 1)?
            }
            Pattern::Tuple(fields) => {
                for (i, p) in fields.iter().enumerate() {
                    self.shadow_pattern(p, &node.child(8, i, budget)?, budget, depth + 1)?;
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
                    let absent = top && *ty == Type::Infer && matches!(value, Expr::Call { .. });
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
                        if *ty == Type::Infer { None } else { Some(ty) },
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
                        if *ty == Type::Infer { None } else { Some(ty) },
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
                sr::work(budget, name.len())?;
                sr::retain(
                    budget,
                    name.len()
                        .checked_add(128)
                        .ok_or_else(|| FactsError::resource(budget, "source call name overflow"))?,
                )?;
                if let Some(path) = application {
                    retain_source_path(path, budget, 0)?;
                }
                let (binding, argument_borrow) = match args.as_slice() {
                    [Expr::Var(name)] => (self.lookup(name, budget)?, None),
                    [Expr::Borrow { kind, place }] if place.fields.is_empty() => {
                        (self.lookup(&place.local, budget)?, Some(*kind))
                    }
                    _ => (None, None),
                };
                let index = self.calls.len();
                sr::push(
                    &mut self.calls,
                    SourceCallSite {
                        node: node.clone(),
                        len_candidate: false,
                        ordinary: Some(copy_source_string(name, budget)?),
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
                        len_candidate: slice_len_candidate(expr),
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
            | Expr::Repeat { value, .. }
            | Expr::Unary { value, .. } => self.expr(
                value,
                &node.child(11, 0, budget)?,
                None,
                None,
                budget,
                depth + 1,
            )?,
            Expr::Array(fields) | Expr::Tuple(fields) => {
                for (i, e) in fields.iter().enumerate() {
                    self.expr(
                        e,
                        &node.child(12, i, budget)?,
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
            | Expr::Bool(_)
            | Expr::Var(_)
            | Expr::Continue { .. }
            | Expr::Break { value: None, .. }
            | Expr::Println(_) => {}
        }
        Ok(())
    }
}

/// Syntactic shape of a builtin shared byte-slice `.len()` occurrence.
fn slice_len_candidate(expr: &Expr) -> bool {
    matches!(expr, Expr::MethodCall { name, type_args, args, .. }
        if name == "len" && type_args.is_empty() && args.is_empty())
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
    builtin_lens: std::cell::RefCell<Vec<BuiltinLenOccurrence>>,
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
    builtin_len_mapping: Vec<BuiltinLenOccurrence>,
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
    /// Record a typed builtin `.len()` at its exact original source occurrence.
    /// Only a syntactic candidate whose checked result is exactly SliceLen over a
    /// shared byte slice qualifies; nothing is inferred from spelling alone.
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
                .is_some_and(|receiver| receiver.ty.shared_byte_slice())
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
            [Expr::Var(name)] => (name, None),
            [Expr::Borrow { kind, place }] if place.fields.is_empty() => {
                (&place.local, Some(*kind))
            }
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
        let (ty, literal) = if *ty != Type::Infer {
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
                _ => {
                    return Err(FactsError::unsupported(
                        budget,
                        "fresh borrow root requires an annotation or suffixed integer/Bool literal",
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
    if !matches!(origin.ty, ir::Ty::Int(_) | ir::Ty::Bool) {
        return Err(FactsError::unsupported(
            budget,
            "fresh borrow inference requires an integer or Bool root",
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

/// Exact replay of typed builtin `.len()` occurrences. Every row names a distinct
/// syntactic candidate with no ordinary call correspondence, maps to exactly one
/// checked SliceLen over its retained receiver, rows follow source order, and
/// every SliceLen in the checked arena is covered. Returns the builtin sources.
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
                .is_some_and(|receiver| receiver.ty.shared_byte_slice())
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
    let fresh = SourceInventory::build(function, original.identity.clone(), budget)?;
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
            || fresh.len_candidate != retained.len_candidate
            || fresh.ordinary != retained.ordinary
            || fresh.application != retained.application
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
            || fresh.annotation != retained.annotation
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
    let builtin_sources = replay_builtin_lens(original, checked, &positions, &mut seen, budget)?;
    replay_unsizes(original, checked, budget)?;
    for (index, site) in original.calls.iter().enumerate() {
        sr::work(budget, 1)?;
        if builtin_sources[index] {
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
            continue;
        };
        let expected_decl =
            *original.plan.registry.functions.get(name).ok_or_else(|| {
                FactsError::invalid(budget, "original call name outside registry")
            })?;
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
            let checker = Checker::new(&signatures, ir::Ty::Unit, caller_context.module.join("::"));
            checker
                .push_application(path)
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
                let inventory = SourceInventory::build(function, identity.clone(), budget)?;
                let mut cap = CanonicalSourceCapability {
                    plan: self.clone(),
                    declaration,
                    facts: facts.clone(),
                    inventory,
                    calls: Default::default(),
                    builtin_lens: Default::default(),
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
                    builtin_len_mapping: cap.builtin_lens.take(),
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

mod recipes;
#[cfg(test)]
mod slice_tests;
pub use tr::CallableProvider;

#[derive(Clone, Debug)]
pub struct Program {
    pub functions: Vec<Function>,
    pub structs: Vec<StructDecl>,
    pub enums: Vec<EnumDecl>,
    pub impls: Vec<ImplDecl>,
    pub traits: Vec<TraitDecl>,
    pub trait_imports: Vec<(String, String)>,
}
/// Syntax-only result. Resolve this once before publishing callable signatures.
#[derive(Clone, Debug)]
pub struct ParsedProgram {
    pub program: Program,
    items: HashMap<String, Vec<Item>>,
    empty_use_prefixes: Vec<(String, String)>,
}
#[derive(Clone, Debug)]
struct Item {
    visibility: ir::Visibility,
    kind: ItemKind,
}
#[derive(Clone, Debug)]
enum ItemKind {
    TypeAlias(TypeAliasDecl),
    Module,
    Function,
    Struct,
    Enum,
    Trait,
    Variant(ir::VariantShape),
    Import(String, bool), // target path, type-namespace-only (`prefix::{self}`)
}

#[derive(Clone, Debug)]
struct TypeAliasDecl {
    params: Vec<String>,
    target: Type,
}

fn qualify(module: &str, name: &str) -> String {
    if module.is_empty() {
        name.to_owned()
    } else {
        format!("{module}::{name}")
    }
}
fn parent(path: &str) -> &str {
    path.rsplit_once("::").map_or("", |(p, _)| p)
}
/// Declaration qualification only. `Const` does not imply an available CTFE engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Constness {
    Runtime,
    Const,
}
#[derive(Clone, Debug)]
pub struct Function {
    pub constness: Constness,
    pub visibility: ir::Visibility,
    pub bounds: Vec<TypeBound>,
    pub name: String,
    pub regions: RegionDecls,
    pub type_params: Vec<String>,
    pub params: Vec<(String, Type)>,
    pub result: Type,
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
    pub trait_path: Option<String>,
    pub bounds: Vec<TypeBound>,
    pub module: String,
    pub type_params: Vec<String>,
    pub self_ty: Type,
    pub items: Vec<AssociatedFunction>,
}
#[derive(Clone, Debug)]
pub struct TypeBound {
    pub parameter: String,
    pub traits: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct TraitDecl {
    pub supertraits: Vec<String>,
    pub name: String,
    pub visibility: ir::Visibility,
    pub items: Vec<TraitItemDecl>,
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
    pub name: String,
    pub regions: RegionDecls,
    pub type_params: Vec<String>,
    pub bounds: Vec<TypeBound>,
    pub params: Vec<(String, Type)>,
    pub result: Type,
}
#[derive(Clone, Debug)]
pub struct AssociatedFunction {
    pub function: Function,
    pub receiver: ReceiverKind,
    pub visibility: ir::Visibility,
}
#[derive(Clone, Debug)]
pub struct CallableContext {
    pub module: Arc<[String]>,
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
    pub receiver: Option<String>,
    pub params: Vec<String>,
    pub outlives: Vec<(String, String)>,
}
#[derive(Clone, Debug)]
pub struct StructDecl {
    pub type_params: Vec<String>,
    pub name: String,
    pub visibility: ir::Visibility,
    pub fields: Vec<FieldDecl>,
}
#[derive(Clone, Debug)]
pub struct EnumDecl {
    pub type_params: Vec<String>,
    pub name: String,
    pub visibility: ir::Visibility,
    pub variants: Vec<VariantDecl>,
}
#[derive(Clone, Debug)]
pub struct VariantDecl {
    pub name: String,
    pub shape: ir::VariantShape,
    pub fields: Vec<FieldDecl>,
}
#[derive(Clone, Debug)]
pub struct FieldDecl {
    pub name: String,
    pub ty: Type,
    pub visibility: ir::Visibility,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldSelector {
    Deref,
    ConstantIndex(usize),
    Tuple(usize),
    Named(String),
}
/// Structured source path. Arguments belong to their declaration segment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenericPath {
    pub segments: Vec<PathSegment>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathSegment {
    pub name: String,
    pub args: Vec<Type>,
}
impl GenericPath {
    fn spelling(&self) -> String {
        self.segments
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join("::")
    }
    fn application(&self) -> Result<(&str, &[Type]), String> {
        let mut applications = self.segments.iter().filter(|s| !s.args.is_empty());
        let segment = applications
            .next()
            .ok_or("missing explicit generic arguments")?;
        if applications.next().is_some() {
            return Err("multiple generic declaration segments in a path are unsupported".into());
        }
        Ok((&segment.name, &segment.args))
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    SelfType,
    Apply(GenericPath),
    Param(String),
    Ref {
        kind: ir::BorrowKind,
        region: Option<String>,
        referent: Box<Type>,
    },
    Array(Box<Type>, usize),
    /// Unsized `[T]`. Only admitted behind a shared reference with `u8` elements.
    Slice(Box<Type>),
    Int(ir::IntTy),
    Float(ir::FloatTy),
    Bool,
    Unit,
    Tuple(Vec<Type>),
    Named(String),
    /// Semantic-only never type, not an admitted source type annotation.
    Never,
    /// Unannotated locals only, resolved independently during function checking.
    Infer,
}
#[derive(Clone, Debug)]
pub struct Block {
    pub statements: Vec<Stmt>,
    pub tail: Option<Box<Expr>>,
}
#[derive(Clone, Debug)]
pub struct Place {
    pub local: String,
    pub fields: Vec<FieldSelector>,
}
#[derive(Clone, Debug)]
pub enum Pattern {
    Apply {
        path: GenericPath,
        pattern: Box<Pattern>,
    },
    Variant {
        name: String,
        shape: ir::VariantShape,
        fields: Vec<(FieldSelector, Pattern)>,
        rest: bool,
    },
    Path(String),
    Bool(bool),
    Int {
        literal: IntegerLiteral,
        negative: bool,
    },
    Wildcard,
    Binding {
        name: String,
        mutable: bool,
    },
    Tuple(Vec<Pattern>),
    Struct {
        name: String,
        fields: Vec<(String, Pattern)>,
        rest: bool,
    },
}
#[derive(Clone, Debug)]
pub enum Stmt {
    Declare {
        name: String,
        ty: Type,
        mutable: bool,
    },
    Let {
        name: String,
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
        name: String,
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
        label: Option<String>,
        condition: Expr,
        body: Block,
    },
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

#[derive(Clone, Debug)]
pub enum Expr {
    TraitCall {
        owner: Type,
        trait_path: String,
        name: String,
        args: Vec<Expr>,
    },
    MethodCall {
        receiver: Box<Expr>,
        name: String,
        type_args: Vec<Type>,
        args: Vec<Expr>,
    },
    AssociatedCall {
        owner: Type,
        name: String,
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
    Deref {
        value: Box<Expr>,
    },
    Integer(IntegerLiteral),
    Float(FloatLiteral),
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
        name: String,
        shape: ir::VariantShape,
        fields: Vec<(FieldSelector, Expr)>,
    },
    Match {
        scrutinee: Box<Expr>,
        arms: Vec<MatchArm>,
    },
    Tuple(Vec<Expr>),
    Struct {
        name: String,
        fields: Vec<(String, Expr)>,
    },
    Field {
        base: Box<Expr>,
        field: FieldSelector,
    },

    Bool(bool),
    Var(String),
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
        name: String,
        args: Vec<Expr>,
    },
    If {
        condition: Box<Expr>,
        then_block: Block,
        else_block: Block,
    },
    Block(Block),
    /// `'label: { ... }`. Only labeled breaks may target it (E0695/E0696).
    LabeledBlock {
        label: String,
        body: Block,
    },
    Loop {
        label: Option<String>,
        body: Block,
    },
    Break {
        label: Option<String>,
        value: Option<Box<Expr>>,
    },
    Continue {
        label: Option<String>,
    },
    Println(String),
}
#[derive(Clone, Debug)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub body: Expr,
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

#[derive(Clone, Debug)]
pub(super) struct ParseError {
    pub(super) kind: super::source::SourceErrorKind,
    pub(super) has_location: bool,
    pub(super) bytes: std::ops::Range<usize>,
    line: usize,
    column: usize,
    pub(super) message: String,
}
impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.column, self.message)
    }
}
#[derive(Clone, Debug)]
struct Token {
    text: String,
    line: usize,
    column: usize,
    string: bool,
    bytes: std::ops::Range<usize>,
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
fn lex(source: &str) -> Result<Vec<Token>, ParseError> {
    let chars: Vec<char> = source.chars().collect();
    let offsets: Vec<usize> = source
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(source.len()))
        .collect();
    let error_at = |index: usize, line, column, message| ParseError {
        kind: super::source::SourceErrorKind::Syntax,
        has_location: true,
        bytes: offsets[index]..offsets.get(index + 1).copied().unwrap_or(source.len()),
        line,
        column,
        message,
    };
    let unsupported_at = |index, line, column, message| {
        let mut error = error_at(index, line, column, message);
        error.kind = super::source::SourceErrorKind::Unsupported;
        error
    };
    let (mut i, mut line, mut column) = (0, 1, 1);
    let mut tokens = Vec::new();
    fn advance(chars: &[char], i: &mut usize, line: &mut usize, column: &mut usize) {
        if chars[*i] == '\n' {
            *line += 1;
            *column = 1;
        } else {
            *column += 1;
        }
        *i += 1;
    }
    while i < chars.len() {
        let c = chars[i];
        if matches!(
            c,
            ' ' | '\t'
                | '\n'
                | '\r'
                | '\u{000b}'
                | '\u{000c}'
                | '\u{0085}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{2028}'
                | '\u{2029}'
        ) {
            advance(&chars, &mut i, &mut line, &mut column);
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                advance(&chars, &mut i, &mut line, &mut column);
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            let start = (line, column);
            let begin = i;
            let mut depth = 1;
            advance(&chars, &mut i, &mut line, &mut column);
            advance(&chars, &mut i, &mut line, &mut column);
            while i < chars.len() && depth > 0 {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    advance(&chars, &mut i, &mut line, &mut column);
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    advance(&chars, &mut i, &mut line, &mut column);
                }
                advance(&chars, &mut i, &mut line, &mut column);
            }
            if depth != 0 {
                return Err(error_at(
                    begin,
                    start.0,
                    start.1,
                    "unterminated comment".into(),
                ));
            }
            continue;
        }
        let (sl, sc) = (line, column);
        let begin = i;
        let mut text = String::new();
        if (c == 'r' && matches!(chars.get(i + 1), Some('"' | '#')))
            || (c == 'b' && matches!(chars.get(i + 1), Some('"' | '\'')))
            || (c == 'b'
                && chars.get(i + 1) == Some(&'r')
                && matches!(chars.get(i + 2), Some('"' | '#')))
        {
            return Err(unsupported_at(
                begin,
                sl,
                sc,
                "raw and byte literals are unsupported".into(),
            ));
        }
        let string = c == '"';
        if string {
            advance(&chars, &mut i, &mut line, &mut column);
            while i < chars.len() && chars[i] != '"' {
                let mut ch = chars[i];
                if ch == '\r' {
                    if chars.get(i + 1) != Some(&'\n') {
                        return Err(error_at(
                            i,
                            line,
                            column,
                            "bare carriage return in string".into(),
                        ));
                    }
                    advance(&chars, &mut i, &mut line, &mut column);
                    ch = '\n';
                }
                if ch == '\\' {
                    advance(&chars, &mut i, &mut line, &mut column);
                    ch = match chars.get(i) {
                        Some('n') => '\n',
                        Some('r') => '\r',
                        Some('t') => '\t',
                        Some('0') => '\0',
                        Some('"') => '"',
                        Some('\'') => '\'',
                        Some('\\') => '\\',
                        Some('x') => {
                            advance(&chars, &mut i, &mut line, &mut column);
                            let hi =
                                chars.get(i).and_then(|c| c.to_digit(16)).ok_or_else(|| {
                                    error_at(i, line, column, "expected two hex digits".into())
                                })?;
                            advance(&chars, &mut i, &mut line, &mut column);
                            let lo =
                                chars.get(i).and_then(|c| c.to_digit(16)).ok_or_else(|| {
                                    error_at(i, line, column, "expected two hex digits".into())
                                })?;
                            let value = hi * 16 + lo;
                            if value > 127 {
                                return Err(error_at(
                                    i,
                                    line,
                                    column,
                                    "hex string escape must be ASCII".into(),
                                ));
                            }
                            char::from_u32(value).unwrap()
                        }
                        Some('u') => {
                            advance(&chars, &mut i, &mut line, &mut column);
                            if chars.get(i) != Some(&'{') {
                                return Err(error_at(
                                    i,
                                    line,
                                    column,
                                    "expected unicode escape brace".into(),
                                ));
                            }
                            advance(&chars, &mut i, &mut line, &mut column);
                            let mut value = 0u32;
                            let mut digits = 0;
                            while let Some(&c) = chars.get(i) {
                                if c == '}' {
                                    break;
                                }
                                if c == '_' && digits > 0 {
                                    advance(&chars, &mut i, &mut line, &mut column);
                                    continue;
                                }
                                let digit = c.to_digit(16).ok_or_else(|| {
                                    error_at(i, line, column, "invalid unicode escape".into())
                                })?;
                                digits += 1;
                                if digits > 6 {
                                    return Err(error_at(
                                        i,
                                        line,
                                        column,
                                        "unicode escape exceeds six digits".into(),
                                    ));
                                }
                                value = value * 16 + digit;
                                advance(&chars, &mut i, &mut line, &mut column);
                            }
                            if digits == 0 || chars.get(i) != Some(&'}') {
                                return Err(error_at(
                                    i,
                                    line,
                                    column,
                                    "unterminated unicode escape".into(),
                                ));
                            }
                            char::from_u32(value).ok_or_else(|| {
                                error_at(i, line, column, "invalid unicode scalar".into())
                            })?
                        }
                        _ => {
                            return Err(error_at(
                                i,
                                line,
                                column,
                                "unsupported string escape".into(),
                            ));
                        }
                    };
                }
                text.push(ch);
                advance(&chars, &mut i, &mut line, &mut column);
            }
            if i == chars.len() {
                return Err(error_at(begin, sl, sc, "unterminated string".into()));
            }
            advance(&chars, &mut i, &mut line, &mut column);
        } else if c == '\'' {
            text.push(c);
            advance(&chars, &mut i, &mut line, &mut column);
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                text.push(chars[i]);
                advance(&chars, &mut i, &mut line, &mut column);
            }
            if text.len() == 1 || chars.get(i) == Some(&'\'') {
                return Err(unsupported_at(
                    begin,
                    sl,
                    sc,
                    "character literals are unsupported".into(),
                ));
            }
        } else if c.is_ascii_alphabetic() || c == '_' || c.is_ascii_digit() {
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                text.push(chars[i]);
                advance(&chars, &mut i, &mut line, &mut column);
            }
            // Decimal float literals: `1.5`, `1.`, `1e-3`, `2.5E+7f32`. A digit
            // run directly after `.` is a tuple field (`t.0.1`), never a float.
            let after_dot = tokens
                .last()
                .is_some_and(|t: &Token| !t.string && t.text == ".");
            let decimal = c.is_ascii_digit()
                && !after_dot
                && !(text.len() > 1 && matches!(&text[..2], "0x" | "0o" | "0b"));
            if decimal
                && chars.get(i) == Some(&'.')
                && !text.contains(['e', 'E'])
                && text.bytes().all(|b| b.is_ascii_digit() || b == b'_')
            {
                let next = chars.get(i + 1).copied();
                if next.is_some_and(|n| n.is_ascii_digit()) {
                    text.push('.');
                    advance(&chars, &mut i, &mut line, &mut column);
                    while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                        text.push(chars[i]);
                        advance(&chars, &mut i, &mut line, &mut column);
                    }
                } else if !next.is_some_and(|n| n == '.' || n == '_' || n.is_alphabetic()) {
                    text.push('.');
                    advance(&chars, &mut i, &mut line, &mut column);
                }
            }
            if decimal
                && text.ends_with(['e', 'E'])
                && matches!(chars.get(i), Some('+' | '-'))
                && chars.get(i + 1).is_some_and(|n| n.is_ascii_digit())
            {
                text.push(chars[i]);
                advance(&chars, &mut i, &mut line, &mut column);
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    text.push(chars[i]);
                    advance(&chars, &mut i, &mut line, &mut column);
                }
            }
        } else {
            if !c.is_ascii() {
                return Err(unsupported_at(
                    i,
                    line,
                    column,
                    "non-ASCII identifiers are unsupported".into(),
                ));
            }
            text.push(c);
            advance(&chars, &mut i, &mut line, &mut column);
            if let Some(&next) = chars.get(i)
                && matches!(
                    (c, next),
                    ('-', '>')
                        | (':', ':')
                        | ('=', '=')
                        | ('!', '=')
                        | ('<', '=')
                        | ('>', '=')
                        | ('<', '<')
                        | ('>', '>')
                        | ('&', '&')
                        | ('|', '|')
                        | ('=', '>')
                        | ('%', '=')
                        | ('+', '=')
                        | ('-', '=')
                        | ('*', '=')
                        | ('/', '=')
                        | ('&', '=')
                        | ('|', '=')
                        | ('^', '=')
                        | ('.', '.')
                )
            {
                text.push(next);
                advance(&chars, &mut i, &mut line, &mut column);
            }
            if matches!(text.as_str(), "<<" | ">>") && chars.get(i) == Some(&'=') {
                text.push('=');
                advance(&chars, &mut i, &mut line, &mut column);
            }
            if !matches!(
                text.as_str(),
                "(" | ")"
                    | "{"
                    | "}"
                    | "["
                    | "]"
                    | ":"
                    | "::"
                    | ";"
                    | ","
                    | "."
                    | ".."
                    | "->"
                    | "=>"
                    | "+"
                    | "-"
                    | "*"
                    | "/"
                    | "%"
                    | "="
                    | "!"
                    | "=="
                    | "!="
                    | "<"
                    | "<="
                    | ">"
                    | ">="
                    | "<<"
                    | ">>"
                    | "+="
                    | "-="
                    | "*="
                    | "/="
                    | "%="
                    | "&="
                    | "|="
                    | "^="
                    | "<<="
                    | ">>="
                    | "&"
                    | "|"
                    | "^"
                    | "&&"
                    | "||"
            ) {
                return Err(if matches!(text.as_str(), "#" | "@" | "?" | "$" | "~") {
                    unsupported_at(begin, sl, sc, format!("unsupported token {text:?}"))
                } else {
                    error_at(begin, sl, sc, format!("invalid token {text:?}"))
                });
            }
        }
        tokens.push(Token {
            text,
            line: sl,
            column: sc,
            string,
            bytes: offsets[begin]..offsets[i],
        });
    }
    tokens.push(Token {
        text: String::new(),
        line,
        column,
        string: false,
        bytes: source.len()..source.len(),
    });
    Ok(tokens)
}
struct Parser<'input> {
    alias_target: bool,
    pending_bounds: Vec<TypeBound>,
    cursor: TokenCursor<'input>,
    nesting: usize,
    allow_struct: bool,
    type_params: Vec<String>,
    use_expansions: usize,
    use_expansion_bytes: usize,
}
impl<'input> Parser<'input> {
    fn new(tokens: &'input [Token]) -> Self {
        Self {
            alias_target: false,
            pending_bounds: Vec::new(),
            cursor: TokenCursor::new(tokens),
            nesting: 0,
            allow_struct: true,
            type_params: Vec::new(),
            use_expansions: 0,
            use_expansion_bytes: 0,
        }
    }
    fn token(&self) -> TokenView<'input> {
        self.cursor.current()
    }
    fn is(&self, s: &str) -> bool {
        !self.token().string && self.token().text == s
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
        ParseError {
            kind: super::source::SourceErrorKind::Syntax,
            has_location: true,
            bytes: self.token().bytes.clone(),
            line: self.token().line,
            column: self.token().column,
            message: format!("{message}, found {:?}", self.token().text),
        }
    }
    fn limit(&self, message: &str) -> ParseError {
        let mut error = self.error(message);
        error.kind = super::source::SourceErrorKind::Limit;
        error
    }
    fn depth_error(&self, message: &str) -> ParseError {
        let mut error = self.limit(message);
        error.has_location = false;
        error
    }
    fn unsupported(&self, message: &str) -> ParseError {
        let mut error = self.error(message);
        error.kind = super::source::SourceErrorKind::Unsupported;
        error
    }
    fn expect(&mut self, s: &str) -> Result<(), ParseError> {
        if self.eat(s) {
            Ok(())
        } else {
            Err(self.error(&format!("expected {s:?}")))
        }
    }
    fn ident(&mut self) -> Result<String, ParseError> {
        let t = self.token();
        let reserved = [
            "fn", "let", "mut", "if", "else", "while", "return", "true", "false", "pub", "const",
            "static", "loop", "break", "continue", "for", "in", "match", "struct", "enum", "use",
            "mod", "impl", "trait", "self", "Self", "as", "async", "await", "move", "unsafe",
            "where", "type", "ref", "extern", "crate", "super", "dyn", "abstract", "become", "box",
            "do", "final", "macro", "override", "priv", "typeof", "unsized", "virtual", "yield",
            "try", "gen",
        ];
        if !t.string
            && t.text != "_"
            && t.text
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && !reserved.contains(&t.text)
        {
            let name = t.text.to_owned();
            self.cursor.bump();
            Ok(name)
        } else {
            Err(self.error("expected ASCII identifier"))
        }
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
            let region = if self.token().text.starts_with('\'') {
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
            if self.is("::") {
                return Err(self.unsupported("associated type projections are unsupported"));
            }
            Ok(Type::SelfType)
        } else if self
            .type_params
            .iter()
            .any(|name| name == self.token().text)
        {
            let name = self.ident()?;
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
            if path.segments.iter().any(|s| !s.args.is_empty()) {
                Ok(Type::Apply(path))
            } else {
                Ok(Type::Named(path.spelling()))
            }
        }
    }
    /// Split closing angle tokens only while parsing types, never expressions.
    fn close_angle(&mut self) -> Result<(), ParseError> {
        if self.cursor.consume_type_gt() {
            Ok(())
        } else {
            Err(self.error("expected generic closing >"))
        }
    }

    fn type_arguments(&mut self) -> Result<Vec<Type>, ParseError> {
        self.expect("<")?;
        let mut args = Vec::new();
        loop {
            if args.len() >= 32 {
                return Err(self.limit("generic argument resource limit 32"));
            }
            args.push(self.ty()?);
            if !self.eat(",") || self.token().text.starts_with('>') {
                break;
            }
        }
        self.close_angle()?;
        Ok(args)
    }
    fn generic_path(&mut self, type_context: bool) -> Result<GenericPath, ParseError> {
        let mut segments = Vec::new();
        loop {
            if segments.len() >= 128 {
                return Err(self.limit("path depth resource limit 128"));
            }
            let name = if matches!(self.token().text, "crate" | "self" | "super" | "Self")
                && !self.token().string
            {
                let name = self.token().text.to_owned();
                self.cursor.bump();
                name
            } else {
                self.ident()?
            };
            let args = if type_context && self.is("<") {
                self.type_arguments()?
            } else {
                Vec::new()
            };
            segments.push(PathSegment { name, args });
            if !self.eat("::") {
                break;
            }
            if self.is("<") {
                let args = self.type_arguments()?;
                if !segments.last().unwrap().args.is_empty() {
                    return Err(self.error("duplicate generic arguments"));
                }
                segments.last_mut().unwrap().args = args;
                if !self.eat("::") {
                    break;
                }
            }
        }
        Ok(GenericPath { segments })
    }
    fn type_parameters(&mut self) -> Result<Vec<String>, ParseError> {
        self.pending_bounds.clear();
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
            if !self.token().string && self.token().text.starts_with('\'') {
                self.lifetime()?;
                return Err(self.error(
                    "lifetime parameters on nominal and impl declarations are unsupported",
                ));
            }
            let name = self.ident()?;
            if params.contains(&name) {
                return Err(self.error("duplicate type parameter"));
            }
            params.push(name.clone());
            if self.eat(":") {
                let mut traits = Vec::new();
                loop {
                    traits.push(self.path()?);
                    if traits.len() > 256 {
                        return Err(self.limit("predicate limit exceeded"));
                    }
                    if !self.eat("+") {
                        break;
                    }
                }
                self.pending_bounds.push(TypeBound {
                    parameter: name,
                    traits,
                });
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
    fn path(&mut self) -> Result<String, ParseError> {
        let mut parts = Vec::new();
        if self.is("crate") || self.is("self") || self.is("super") {
            let first = self.token().text.to_owned();
            self.cursor.bump();
            parts.push(first.clone());
            if first == "super" {
                while self.is("::")
                    && self
                        .cursor
                        .peek(1)
                        .is_some_and(|t| !t.string && t.text == "super")
                {
                    self.cursor.bump();
                    self.cursor.bump();
                    parts.push("super".into());
                }
            }
        } else if self.eat("Self") {
            parts.push("Self".into());
        } else {
            parts.push(self.ident()?);
        }
        while self.eat("::") {
            if self.is("*") || self.is("{") {
                return Err(self.unsupported("glob and grouped paths are unsupported"));
            }
            parts.push(self.ident()?);
        }
        Ok(parts.join("::"))
    }
    /// Flatten syntax only. Canonical identities, privacy, cycles and namespace
    /// collisions are still decided by the ordinary import resolver.
    fn use_tree(
        &mut self,
        prefix: &[String],
        depth: usize,
        work: &mut usize,
        bytes: &mut usize,
        imports: &mut Vec<(Option<String>, String, bool)>,
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
        let prefix_bytes: usize = prefix.iter().map(|s| s.len() + 2).sum();
        *bytes += prefix_bytes;
        if *bytes > 1024 * 1024 {
            return Err(self.limit("use tree allocation exceeds limit 1048576"));
        }
        let mut path = prefix.to_vec();
        let mut grouped_self = false;
        let mut group_allowed = self.is("{");
        if !self.is("{") {
            loop {
                *work += 1;
                if *work > 4096 {
                    return Err(self.limit("use tree work exceeds limit 4096"));
                }
                let keyword = self.is("self") || self.is("super") || self.is("crate");
                let segment = if keyword {
                    let segment = self.token().text.to_owned();
                    self.cursor.bump();
                    segment
                } else {
                    self.ident()?
                };
                *bytes += segment.len() + 2;
                if *bytes > 1024 * 1024 {
                    return Err(self.limit("use tree allocation exceeds limit 1048576"));
                }
                // A terminal self within a group names the prefix itself,
                // rather than an item literally named `self` inside it.
                if segment == "self" && depth > 0 && !path.is_empty() && !self.is("::") {
                    grouped_self = true;
                } else {
                    if keyword
                        && !path.is_empty()
                        && !(segment == "super" && path.iter().all(|s| s == "super"))
                    {
                        return Err(self.error("module keyword must begin a use path"));
                    }
                    path.push(segment);
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
            if self.is("}") && !path.is_empty() {
                imports.push((None, path.join("::"), false));
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
            let name = path.last().ok_or_else(|| self.error("empty use path"))?;
            if matches!(name.as_str(), "crate" | "self" | "super") {
                return Err(self.error("module keyword imports require an explicit alias"));
            }
            name.clone()
        };
        let target = path.join("::");
        *bytes += target.len() + name.len();
        if *bytes > 1024 * 1024 {
            return Err(self.limit("use tree allocation exceeds limit 1048576"));
        }
        imports.push((Some(name), target, grouped_self));
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
    fn visibility(&mut self, module: &str) -> Result<(ir::Visibility, bool), ParseError> {
        if !self.eat("pub") {
            return Ok((ir::Visibility::Restricted(module.into()), false));
        }
        if !self.eat("(") {
            return Ok((ir::Visibility::Public, true));
        }
        let scope = if self.eat("in") {
            let path = self.path()?;
            let parts: Vec<_> = path.split("::").collect();
            if parts.len() > 129 {
                return Err(self.limit("visibility path nesting exceeds limit 129"));
            }
            let (mut scope, mut at) = match parts[0] {
                "crate" => (String::new(), 1),
                "self" => (module.to_owned(), 1),
                "super" => (module.to_owned(), 0),
                _ => return Err(self.error("pub(in) paths must start with crate, self or super")),
            };
            while parts[0] == "super" && parts.get(at) == Some(&"super") {
                if scope.is_empty() {
                    return Err(self.error("visibility path escapes crate root"));
                }
                scope = parent(&scope).into();
                at += 1;
            }
            for name in &parts[at..] {
                if matches!(*name, "crate" | "self" | "super") {
                    return Err(self.error("invalid visibility path segment"));
                }
                scope = qualify(&scope, name);
            }
            if !visible_from(&scope, module) {
                return Err(self.error("visibility path must name a lexical ancestor module"));
            }
            scope
        } else if self.eat("crate") {
            String::new()
        } else if self.eat("self") {
            module.to_owned()
        } else if self.eat("super") {
            if module.is_empty() {
                return Err(self.error("visibility path escapes crate root"));
            }
            parent(module).to_owned()
        } else {
            return Err(self.error("expected crate, self, super or in in restricted visibility"));
        };
        self.expect(")")?;
        Ok((ir::Visibility::Restricted(scope), true))
    }
    fn items(&mut self, module: &str, parsed: &mut ParsedProgram) -> Result<(), ParseError> {
        if self.nesting > 128 {
            return Err(self.limit("module nesting exceeds limit 128"));
        }
        self.nesting += 1;
        while !self.is("") && !self.is("}") {
            let (visibility, explicit_visibility) = self.visibility(module)?;
            if self.eat("mod") {
                let name = self.ident()?;
                let full = qualify(module, &name);
                if self.eat(";") {
                    return Err(
                        self.unsupported("external module files are unsupported; use inline mod")
                    );
                }
                self.expect("{")?;
                // Register before parsing descendants, including empty modules.
                self.insert_item(parsed, &full, visibility, ItemKind::Module)?;
                self.items(&full, parsed)?;
                self.expect("}")?;
                continue;
            }
            self.nonmodule_item(module, parsed, visibility, explicit_visibility)?;
        }
        self.nesting -= 1;
        Ok(())
    }
    // Keep the large nonrecursive declaration parser frame off the recursive
    // inline-module path, including on ordinary 2 MiB test-thread stacks.
    #[inline(never)]
    fn nonmodule_item(
        &mut self,
        module: &str,
        parsed: &mut ParsedProgram,
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
            self.use_tree(&[], 0, &mut work, &mut bytes, &mut imports)?;
            self.expect(";")?;
            for (name, path, type_only) in imports {
                // Bound logical retained expansion across the entire source,
                // not just each tree. This is not an allocator-byte charge.
                self.use_expansions += 1;
                self.use_expansion_bytes +=
                    path.len() + module.len() + name.as_ref().map_or(0, String::len) + 2;
                if self.use_expansions > 65536 {
                    return Err(self.limit("source use expansion count exceeds limit 65536"));
                }
                if self.use_expansion_bytes > 16 * 1024 * 1024 {
                    return Err(self.limit("source use logical expansion exceeds limit 16777216"));
                }
                let Some(name) = name else {
                    parsed.empty_use_prefixes.push((module.into(), path));
                    continue;
                };
                self.insert_item(
                    parsed,
                    &qualify(module, &name),
                    visibility.clone(),
                    ItemKind::Import(path, type_only),
                )?;
            }
            return Ok(());
        } else if self.eat("trait") {
            let name = self.ident()?;
            if self.is("<") {
                return Err(self.unsupported("trait generics are unsupported"));
            }
            let mut supertraits = Vec::new();
            if self.eat(":") {
                loop {
                    if supertraits.len() >= 256 {
                        return Err(self.limit("supertrait limit exceeded"));
                    }
                    supertraits.push(self.path()?);
                    if !self.eat("+") {
                        break;
                    }
                }
            }
            self.expect("{")?;
            let mut items = Vec::new();
            let mut item_names = std::collections::HashSet::new();
            while !self.is("}") {
                if items.len() >= 4096 {
                    return Err(self.limit("trait item limit exceeded"));
                }
                if self.is("pub") {
                    return Err(self.error("trait items cannot declare visibility"));
                }
                if self.is("type") {
                    return Err(self.unsupported("associated types are unsupported"));
                }
                let (signature, receiver) = self.function_head(&[], true)?;
                if !item_names.insert(signature.name.clone()) {
                    return Err(self.error("duplicate required trait item"));
                }
                if !signature.type_params.is_empty() {
                    return Err(self.unsupported("generic trait methods are unsupported"));
                }
                let default = if self.is("{") {
                    Some(Function {
                        constness: signature.constness,
                        visibility: visibility.clone(),
                        bounds: signature.bounds.clone(),
                        name: signature.name.clone(),
                        regions: signature.regions.clone(),
                        type_params: signature.type_params.clone(),
                        params: signature.params.clone(),
                        result: signature.result.clone(),
                        body: self.block()?,
                    })
                } else {
                    self.expect(";")?;
                    None
                };
                items.push(TraitItemDecl {
                    signature,
                    receiver,
                    default,
                });
            }
            self.expect("}")?;
            parsed.program.traits.push(TraitDecl {
                supertraits,
                name: qualify(module, &name),
                visibility: visibility.clone(),
                items,
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
                                name: fields.len().to_string(),
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
                                name,
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
                if variants.iter().any(|v| v.name == variant) {
                    return Err(self.error("duplicate enum variant"));
                }
                if variants.len() >= 4096 {
                    return Err(self.limit("enum variant count exceeds limit 4096"));
                }
                self.insert_item(
                    parsed,
                    &qualify(&qualify(module, &name), &variant),
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
                type_params,
                name: qualify(module, &name),
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
            self.expect("{")?;
            let mut fields = Vec::new();
            while !self.is("}") {
                let (visibility, _) = self.visibility(module)?;
                let name = self.ident()?;
                self.expect(":")?;
                fields.push(FieldDecl {
                    name,
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
            parsed.program.structs.push(StructDecl {
                type_params,
                name: qualify(module, &name),
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
                let Type::Named(name) = first_ty else {
                    return Err(self.unsupported("trait arguments are unsupported"));
                };
                (Some(name), self.ty()?)
            } else {
                (None, first_ty)
            };
            if self.eat("where") {
                loop {
                    if !self.token().string && self.token().text.starts_with('\'') {
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
            while !self.is("}") {
                if items.len() >= 4096 {
                    return Err(self.limit("associated item limit exceeded"));
                }
                let (visibility, explicit_visibility) = self.visibility(module)?;
                if explicit_visibility && trait_path.is_some() {
                    return Err(self.error("trait impl items cannot declare pub"));
                }
                if self.is("type") {
                    return Err(self.unsupported("associated types are unsupported"));
                }
                let (mut function, receiver) = self.function_scoped(&type_params, true)?;
                function.visibility = visibility.clone();
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
                module: module.into(),
                type_params,
                self_ty,
                items,
            });
            self.type_params.clear();
            return Ok(());
        } else if self.is("fn") || self.is("const") {
            let mut function = self.function()?;
            function.visibility = visibility.clone();
            let name = function.name.clone();
            function.name = qualify(module, &name);
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
        self.insert_item(parsed, &qualify(module, &name), visibility, kind)?;
        Ok(())
    }
    fn insert_item(
        &self,
        parsed: &mut ParsedProgram,
        path: &str,
        visibility: ir::Visibility,
        kind: ItemKind,
    ) -> Result<(), ParseError> {
        let bindings = parsed.items.entry(path.to_owned()).or_default();
        if bindings.iter().any(|item| {
            if matches!(kind, ItemKind::Enum)
                && matches!(
                    item.kind,
                    ItemKind::Module | ItemKind::Struct | ItemKind::Enum
                )
                || matches!(item.kind, ItemKind::Enum)
                    && matches!(kind, ItemKind::Module | ItemKind::Struct)
            {
                return true;
            }
            matches!(
                (&item.kind, &kind),
                (ItemKind::Module, ItemKind::Module)
                    | (ItemKind::Module, ItemKind::Struct)
                    | (ItemKind::Struct, ItemKind::Module)
                    | (ItemKind::Struct, ItemKind::Struct)
                    | (ItemKind::Function, ItemKind::Function)
            )
        }) {
            return Err(self.error(&format!("duplicate item {path}")));
        }
        bindings.push(Item { visibility, kind });
        Ok(())
    }
    fn lifetime(&mut self) -> Result<String, ParseError> {
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
        self.cursor.bump();
        Ok(text)
    }
    fn region_bounds(&mut self, long: &str, regions: &mut RegionDecls) -> Result<(), ParseError> {
        if self.eat(":") {
            loop {
                let short = self.lifetime()?;
                if regions.outlives.len() >= 4096 {
                    return Err(self.limit("lifetime bounds exceed supported limit 4096"));
                }
                regions.outlives.push((long.to_owned(), short));
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
        if !self.type_params.contains(&parameter) {
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
            if !self.token().string && self.token().text.starts_with('\'') {
                return Err(self.unsupported("type outlives bounds are unsupported"));
            }
            if traits.len() >= 256 {
                return Err(self.limit("predicate limit exceeded"));
            }
            traits.push(self.path()?);
            if !self.eat("+") {
                break;
            }
        }
        if bounds.len() >= 4096 {
            return Err(self.limit("predicate limit exceeded"));
        }
        bounds.push(TypeBound { parameter, traits });
        Ok(())
    }
    fn function(&mut self) -> Result<Function, ParseError> {
        self.function_scoped(&[], false).map(|(f, _)| f)
    }
    fn function_scoped(
        &mut self,
        inherited: &[String],
        associated: bool,
    ) -> Result<(Function, ReceiverKind), ParseError> {
        let (head, receiver) = self.function_head(inherited, associated)?;
        Ok((
            Function {
                constness: head.constness,
                visibility: ir::Visibility::Public,
                name: head.name,
                regions: head.regions,
                type_params: head.type_params,
                bounds: head.bounds,
                params: head.params,
                result: head.result,
                body: self.block()?,
            },
            receiver,
        ))
    }
    fn callable_generics(&mut self, regions: &mut RegionDecls) -> Result<Vec<String>, ParseError> {
        self.pending_bounds.clear();
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
            if self.token().text.starts_with('\'') {
                if seen_type {
                    return Err(self.error("lifetime parameters must precede type parameters"));
                }
                if regions.params.len() >= 64 {
                    return Err(self.limit("declared lifetime limit64"));
                }
                let name = self.lifetime()?;
                regions.params.push(name.clone());
                self.region_bounds(&name, regions)?;
            } else {
                seen_type = true;
                if params.len() >= 32 {
                    return Err(self.limit("generic parameter resource limit32"));
                }
                let name = self.ident()?;
                if params.contains(&name) {
                    return Err(self.error("duplicate type parameter"));
                }
                params.push(name.clone());
                if self.eat(":") {
                    let mut traits = Vec::new();
                    loop {
                        if traits.len() >= 256 {
                            return Err(self.limit("predicate limit exceeded"));
                        }
                        traits.push(self.path()?);
                        if !self.eat("+") {
                            break;
                        }
                    }
                    self.pending_bounds.push(TypeBound {
                        parameter: name,
                        traits,
                    });
                }
                if self.is("=") {
                    return Err(self.unsupported("generic defaults are unsupported"));
                }
            }
            if !self.eat(",") || self.is(">") {
                break;
            }
        }
        self.expect(">")?;
        Ok(params)
    }
    fn function_head(
        &mut self,
        inherited: &[String],
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
        let type_params = self.callable_generics(&mut regions)?;
        let mut bounds = std::mem::take(&mut self.pending_bounds);
        self.type_params = inherited.to_vec();
        for name in &type_params {
            if self.type_params.contains(name) {
                return Err(self.error("duplicate visible generic parameter"));
            }
            self.type_params.push(name.clone());
        }
        self.expect("(")?;
        let mut params = Vec::new();
        let mut receiver = ReceiverKind::None;
        if associated && self.eat("&") {
            regions.receiver = if self.token().text.starts_with('\'') {
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
                let n = self.ident()?;
                self.expect(":")?;
                params.push((n, self.ty()?));
                if !self.eat(",") || self.is(")") {
                    break;
                }
            }
        }
        self.expect(")")?;
        let result = if self.eat("->") {
            self.ty()?
        } else {
            Type::Unit
        };
        if self.eat("where") {
            loop {
                if !self.token().string && self.token().text.starts_with('\'') {
                    let long = self.lifetime()?;
                    if !self.is(":") {
                        return Err(self.error("only lifetime outlives where bounds are supported"));
                    }
                    self.region_bounds(&long, &mut regions)?;
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
                result,
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
        self.allow_struct = true;
        let result = self.block_inner();
        self.allow_struct = previous;
        self.nesting -= 1;
        result
    }
    fn pattern(&mut self) -> Result<Pattern, ParseError> {
        if self.nesting >= 128 {
            return Err(self.limit("pattern nesting exceeds limit 128"));
        }
        self.nesting += 1;
        let result = self.pattern_inner();
        self.nesting -= 1;
        result
    }
    fn pattern_inner(&mut self) -> Result<Pattern, ParseError> {
        if self.token().string {
            return Err(self.unsupported("string literal patterns are unsupported"));
        }
        if self.is("&") || self.is("&&") {
            return Err(
                self.unsupported("reference patterns and match ergonomics are unsupported in B1")
            );
        }
        if self.eat("true") {
            return Ok(Pattern::Bool(true));
        }
        if self.eat("false") {
            return Ok(Pattern::Bool(false));
        }
        if self.is("-") || self.token().text.starts_with(|c: char| c.is_ascii_digit()) {
            let negative = self.eat("-");
            if self.token().string {
                return Err(self.error("expected an integer literal after minus"));
            }
            let literal = parse_integer_literal(self.token().text).map_err(|e| self.error(&e))?;
            self.cursor.bump();
            return Ok(Pattern::Int { literal, negative });
        }
        if self.eat("_") {
            return Ok(Pattern::Wildcard);
        }
        if self.eat("(") {
            if self.eat(")") {
                return Ok(Pattern::Tuple(Vec::new()));
            }
            let first = self.pattern()?;
            if !self.eat(",") {
                self.expect(")")?;
                return Ok(first);
            }
            let mut fields = vec![first];
            while !self.is(")") {
                if self.is("..") {
                    return Err(self.unsupported("tuple rest patterns are unsupported"));
                }
                fields.push(self.pattern()?);
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
            });
        }
        let generic_path = self.generic_path(false)?;
        let name = generic_path.spelling();
        let wrap = |pattern| {
            if generic_path.segments.iter().any(|s| !s.args.is_empty()) {
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
                if self.is("..") {
                    return Err(self.unsupported("tuple variant rest patterns are unsupported"));
                }
                fields.push((FieldSelector::Tuple(fields.len()), self.pattern()?));
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
            if name.contains("::") || matches!(name.as_str(), "self" | "super" | "crate") {
                return Ok(wrap(Pattern::Path(name)));
            }
            return Ok(Pattern::Binding {
                name,
                mutable: false,
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
            let mutable = self.eat("mut");
            let field = self.ident()?;
            let pattern = if self.eat(":") {
                if mutable {
                    return Err(self.error("mut must apply to a binding, not a field selector"));
                }
                self.pattern()?
            } else {
                Pattern::Binding {
                    name: field.clone(),
                    mutable,
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
            if self.is("mod") || self.is("use") || self.is("fn") || self.is("pub") {
                return Err(self.unsupported("block-local items are unsupported"));
            }
            if self.eat("let") {
                let pattern = self.pattern()?;
                if matches!(pattern, Pattern::Bool(_) | Pattern::Int { .. }) {
                    return Err(self.error("refutable literal pattern in let binding"));
                }
                // Local inference is performed by each independent function checker.
                let annotation = if self.eat(":") {
                    Some(self.ty()?)
                } else {
                    None
                };
                if self.eat(";") {
                    let Pattern::Binding { name, mutable } = pattern else {
                        return Err(
                            self.unsupported("deferred pattern declarations are unsupported")
                        );
                    };
                    let ty = annotation.unwrap_or(Type::Infer);
                    statements.push(Stmt::Declare { name, ty, mutable });
                    continue;
                }
                self.expect("=")?;
                let value = self.expr(0)?;
                self.expect(";")?;
                let ty = annotation.unwrap_or(Type::Infer);
                if let Pattern::Binding { name, mutable } = pattern {
                    statements.push(Stmt::Let {
                        name,
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
                || (self.token().text.starts_with('\'')
                    && self.cursor.peek(1).is_some_and(|t| t.text == ":")
                    && self.cursor.peek(2).is_some_and(|t| t.text == "while"))
            {
                let label = if self.is("while") {
                    None
                } else {
                    let label = self.token().text.trim_start_matches('\'').to_owned();
                    self.cursor.bump();
                    self.expect(":")?;
                    Some(label)
                };
                self.expect("while")?;
                let condition = self.condition()?;
                let body = self.block()?;
                statements.push(Stmt::While {
                    label,
                    condition,
                    body,
                });
            } else if self.is("for") {
                return Err(self.error(
                    "unsupported: for loops and iterators (including slice iteration); use while with an index",
                ));
            } else {
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
                } else if matches!(
                    expr,
                    Expr::If { .. }
                        | Expr::Block(_)
                        | Expr::LabeledBlock { .. }
                        | Expr::Loop { .. }
                        | Expr::Match { .. }
                ) {
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
                Expr::Var(local) => { fields.reverse(); return Ok(Place { local, fields }); }
                Expr::Deref {value}=>{fields.push(FieldSelector::Deref);expr = *value;}
                Expr::Field { base, field } => { fields.push(field); expr = *base; }
                Expr::Index { base,index } => {fields.push(FieldSelector::ConstantIndex(index));expr = *base;}
                _ => return Err(self.unsupported("assignment requires a local-rooted place; destructuring and temporary assignment are unsupported")),
            }
        }
    }
    fn nested_expr(&mut self) -> Result<Expr, ParseError> {
        let previous = self.allow_struct;
        self.allow_struct = true;
        let result = self.expr(0);
        self.allow_struct = previous;
        result
    }
    fn condition(&mut self) -> Result<Expr, ParseError> {
        let previous = self.allow_struct;
        self.allow_struct = false;
        let result = self.expr(0);
        self.allow_struct = previous;
        result
    }
    fn expr(&mut self, min: u8) -> Result<Expr, ParseError> {
        if self.nesting >= 128 {
            return Err(self.limit("syntax nesting exceeds limit 128"));
        }
        self.nesting += 1;
        let result = self.expr_inner(min);
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
        let generic_path = self.generic_path(false)?;
        let name = generic_path.spelling();
        let value = if self.eat("!") {
            if name != "println" {
                return Err(self.error("only println! is supported"));
            }
            self.expect("(")?;
            if !self.token().string {
                return Err(self.error("println! requires one literal string"));
            }
            let text = self.token().text.to_owned();
            self.cursor.bump();
            let text = literal_format(&text).map_err(|e| self.error(&e))?;
            self.eat(",");
            self.expect(")")?;
            Expr::Println(text)
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
                let field = self.ident()?;
                let value = if self.eat(":") {
                    self.expr(0)?
                } else {
                    Expr::Var(field.clone())
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
            if generic_path.segments.iter().any(|s| !s.args.is_empty()) {
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
    fn expr_inner(&mut self, min: u8) -> Result<Expr, ParseError> {
        self.expr_inner_mode(min, false)
    }
    fn expr_inner_mode(&mut self, min: u8, statement: bool) -> Result<Expr, ParseError> {
        if !self.token().string && self.token().text.starts_with('\'') {
            return self.labeled_expression(min, statement);
        }
        let statement_blocklike =
            statement && matches!(self.token().text, "if" | "match" | "loop" | "{");
        if self.is("&&") {
            return Err(self.unsupported("nested reference borrows are unsupported in B1"));
        }
        let left = self.expr_prefix(statement)?;
        self.expr_continuation(left, min, statement_blocklike)
    }
    #[inline(never)]
    fn labeled_expression(&mut self, min: u8, statement: bool) -> Result<Expr, ParseError> {
        let label = self.token().text.trim_start_matches('\'').to_owned();
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
        } else {
            return Err(self.error("a label must precede a loop, while, or block"));
        };
        self.expr_continuation(left, min, statement)
    }
    fn trait_call_prefix(&mut self) -> Result<Expr, ParseError> {
        let owner = self.ty()?;
        self.expect("as")?;
        let trait_path = self.path()?;
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
            let place=self.assignment_place(value).map_err(|_|self.unsupported("borrowing requires a local-root static place; rvalues and runtime indexed borrows are unsupported"))?;
            Expr::Borrow { kind, place }
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
            let label = if self.token().text.starts_with('\'') {
                let label = self.token().text.trim_start_matches('\'').to_owned();
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
            let label = if self.token().text.starts_with('\'') {
                let label = self.token().text.trim_start_matches('\'').to_owned();
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
        } else if self.is("{") {
            Expr::Block(self.block()?)
        } else if self.eat("[") {
            return self.array_prefix();
        } else if self.eat("(") {
            return self.group_prefix();
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
        let condition = Box::new(self.condition()?);
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
            let direct_return = self.eat("return");
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
                && matches!(
                    body,
                    Expr::Block(_) | Expr::If { .. } | Expr::Loop { .. } | Expr::Match { .. }
                );
            arms.push(MatchArm { pattern, body });
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
                        | Expr::Match { .. }
                        | Expr::Loop { .. }
                        | Expr::LabeledBlock { .. }
                        | Expr::Block(_)
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
                        | Expr::Match { .. }
                        | Expr::Loop { .. }
                        | Expr::LabeledBlock { .. }
                        | Expr::Block(_)
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
fn literal_format(text: &str) -> Result<String, String> {
    let mut chars = text.chars();
    let mut out = String::new();
    while let Some(c) = chars.next() {
        if (c == '{' || c == '}') && chars.next() != Some(c) {
            return Err("println! formatting placeholders are unsupported".into());
        }
        out.push(c);
    }
    Ok(out)
}

/// Parse syntax only, retaining imports and visibility for the signatures phase.
fn parse_syntax_tokens(tokens: Vec<Token>) -> Result<ParsedProgram, ParseError> {
    let mut parser = Parser::new(&tokens);
    let mut parsed = ParsedProgram {
        program: Program {
            functions: Vec::new(),
            structs: Vec::new(),
            enums: Vec::new(),
            impls: Vec::new(),
            traits: Vec::new(),
            trait_imports: Vec::new(),
        },
        items: HashMap::new(),
        empty_use_prefixes: Vec::new(),
    };
    parser.items("", &mut parsed)?;
    if !parser.is("") {
        return Err(parser.error("unexpected module closing brace"));
    }
    for function in &parsed.program.functions {
        validate_depth(DepthNode::Block(&function.body)).map_err(|e| ParseError {
            kind: super::source::SourceErrorKind::Limit,
            has_location: false,
            bytes: 0..0,
            line: 1,
            column: 1,
            message: format!("in function {}: {e}", function.name),
        })?;
    }
    for declaration in &parsed.program.traits {
        for item in &declaration.items {
            if let Some(default) = &item.default {
                validate_depth(DepthNode::Block(&default.body)).map_err(|e| ParseError {
                    kind: super::source::SourceErrorKind::Limit,
                    has_location: false,
                    bytes: 0..0,
                    line: 1,
                    column: 1,
                    message: format!(
                        "in trait default {}::{}: {e}",
                        declaration.name, default.name
                    ),
                })?;
            }
        }
    }
    Ok(parsed)
}
/// Syntax-only preparation seam. Does not process attributes, resolve names, or
/// access files. The source preparation owner supplies attribute-masked text.
pub(super) fn parse_syntax_unprepared(source: &str) -> Result<ParsedProgram, ParseError> {
    parse_syntax_tokens(lex(source)?)
}

/// Parse mapped preparation output without reprocessing attributes or configuration.
pub fn parse_prepared(
    source: &super::source::PreparedSource,
) -> Result<ParsedProgram, super::source::SourceError> {
    let mapped = |error: ParseError| {
        let mut diagnostic = super::source::SourceError::syntax(
            if error.has_location {
                source.map_range(error.bytes)
            } else {
                None
            },
            error.message,
        );
        diagnostic.kind = error.kind;
        diagnostic.with_sources(&source.sources)
    };
    let tokens = lex(&source.text).map_err(mapped)?;
    for token in &tokens {
        if source.map_range(token.bytes.clone()).is_none() {
            return Err(super::source::SourceError::syntax(
                None,
                "prepared token crosses or lacks an original source mapping",
            )
            .with_sources(&source.sources));
        }
    }
    parse_syntax_tokens(tokens).map_err(mapped)
}

/// Parse syntax with the same preparation policy as file inputs, retaining the
/// legacy line/column String diagnostic format for existing direct callers.
pub fn parse_unresolved(source: &str) -> Result<ParsedProgram, String> {
    let prepared =
        super::source::prepare_text("<text>", source, &super::source::SourceOptions::default())
            .map_err(|e| e.to_string())?;
    parse_syntax_unprepared(&prepared.text).map_err(|e| e.to_string())
}

/// Convenience frontend for callers without separate parse/signatures phases.
/// Performs name resolution, but not function body type checks.
pub fn parse(source: &str) -> Result<Program, String> {
    resolve(&parse_unresolved(source)?)
}

#[derive(Clone, Debug)]
struct ResolvedItem {
    // Publicly exportable original and final alias binding. Containing module
    // visibility is deliberately excluded. Variants inherit their enum's export
    // eligibility, independently of their ordinary in-crate accessibility.
    visibility: ir::Visibility,
    path: String,
    module: bool,
    kind: ResolvedKind,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResolvedKind {
    TypeAlias,
    Trait,
    Module,
    Function,
    Struct,
    Enum,
    Variant,
}
struct Resolver<'a> {
    type_depth: usize,
    alias_cache: HashMap<String, Type>,
    alias_active: std::collections::HashSet<String>,
    alias_budget: ir::PlanBudget,
    nominal_arity: HashMap<String, usize>,
    self_nominal: Option<String>,
    items: &'a HashMap<String, Vec<Item>>,
    imports: HashMap<(String, usize, bool), Option<ResolvedItem>>,
    active: std::collections::HashSet<(String, usize, bool)>,
    type_params: Vec<String>,
}
impl Resolver<'_> {
    fn alias_source_path(&self, name: &str) -> Result<GenericPath, String> {
        clone_text(name, &self.alias_budget)?;
        self.alias_budget.retain_bytes(
            name.len()
                .checked_add(1)
                .and_then(|n| n.checked_mul(std::mem::size_of::<PathSegment>()))
                .ok_or("alias source path storage overflow")?,
        )?;
        Ok(GenericPath {
            segments: name
                .split("::")
                .map(|name| PathSegment {
                    name: name.into(),
                    args: Vec::new(),
                })
                .collect(),
        })
    }
    fn alias_decl(&self, name: &str) -> Result<&TypeAliasDecl, String> {
        self.items
            .get(name)
            .and_then(|items| {
                items.iter().find_map(|item| {
                    if let ItemKind::TypeAlias(alias) = &item.kind {
                        Some(alias)
                    } else {
                        None
                    }
                })
            })
            .ok_or_else(|| format!("missing type alias {name}"))
    }
    fn resolve_aliases(&mut self) -> Result<(), String> {
        self.alias_budget.retain_bytes(
            self.items
                .len()
                .checked_mul(2 * std::mem::size_of::<&str>())
                .and_then(|n| n.checked_add(128))
                .ok_or("alias root storage overflow")?,
        )?;
        let mut roots = self
            .items
            .iter()
            .filter(|(_, items)| {
                items
                    .iter()
                    .any(|item| matches!(item.kind, ItemKind::TypeAlias(_)))
            })
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>();
        roots.sort();
        let mut active = std::collections::HashSet::new();
        for root in roots {
            clone_text(root, &self.alias_budget)?;
            self.alias_budget.retain_bytes(128)?;
            let mut stack = vec![(root.to_owned(), false)];
            while let Some((name, finish)) = stack.pop() {
                self.alias_budget.charge_work(1)?;
                if self.alias_cache.contains_key(&name) {
                    continue;
                }
                if finish {
                    self.alias_template(&name)?;
                    active.remove(&name);
                    continue;
                }
                clone_text(&name, &self.alias_budget)?;
                self.alias_budget.retain_bytes(128)?;
                if !active.insert(name.clone()) {
                    return Err(format!("E0391: cycle in type alias {name}"));
                }
                walk_depth(
                    DepthNode::Type(&self.alias_decl(&name)?.target),
                    Some(&self.alias_budget),
                )?;
                let mut spellings = Vec::new();
                self.alias_budget.retain_bytes(128)?;
                let mut pending = vec![&self.alias_decl(&name)?.target];
                while let Some(ty) = pending.pop() {
                    self.alias_budget.charge_work(1)?;
                    let children = match ty {
                        Type::Tuple(fields) => fields.len(),
                        Type::Apply(path) => path
                            .segments
                            .iter()
                            .try_fold(0usize, |n, s| n.checked_add(s.args.len()))
                            .ok_or("alias dependency child count overflow")?,
                        Type::Ref { .. } | Type::Array(..) | Type::Slice(_) => 1,
                        _ => 0,
                    };
                    self.alias_budget.retain_bytes(
                        children
                            .checked_mul(2 * std::mem::size_of::<&Type>())
                            .and_then(|n| n.checked_add(128))
                            .ok_or("alias dependency worklist storage overflow")?,
                    )?;
                    match ty {
                        Type::Named(path) => {
                            clone_text(path, &self.alias_budget)?;
                            spellings.push(path.clone());
                        }
                        Type::Apply(path) => {
                            clone_path(path, &self.alias_budget)?;
                            spellings.push(path.spelling());
                            for segment in &path.segments {
                                pending.extend(&segment.args);
                            }
                        }
                        Type::Tuple(fields) => pending.extend(fields),
                        Type::Ref { referent, .. }
                        | Type::Array(referent, _)
                        | Type::Slice(referent) => pending.push(referent),
                        _ => {}
                    }
                }
                let mut dependencies = Vec::new();
                for spelling in spellings {
                    if let Some(item) = self.path(&spelling, parent(&name), true, false)?
                        && item.kind == ResolvedKind::TypeAlias
                    {
                        self.alias_budget.retain_bytes(128)?;
                        dependencies.push(item.path);
                    }
                }
                self.alias_budget.retain_bytes(
                    (dependencies.len() + 1)
                        .checked_mul(std::mem::size_of::<(String, bool)>() + 64)
                        .ok_or("alias dependency storage overflow")?,
                )?;
                stack.push((name, true));
                for dependency in dependencies.into_iter().rev() {
                    stack.push((dependency, false));
                }
            }
        }
        Ok(())
    }
    /// Resolve in the definition's binder before substituting caller arguments.
    /// Templates contain canonical nominal paths, never caller-relative names.
    fn alias_template(&mut self, name: &str) -> Result<Type, String> {
        self.alias_budget.charge_work(1)?;
        if let Some(target) = self.alias_cache.get(name) {
            walk_depth(DepthNode::Type(target), Some(&self.alias_budget))?;
            return Ok(target.clone());
        }
        if self.alias_active.contains(name) {
            return Err(format!("E0391: cycle in type alias {name}"));
        }
        if self.alias_active.len() >= 8 {
            return Err("type alias resolution nesting exceeds resource limit 8".into());
        }
        let alias = self.alias_decl(name)?;
        clone_names(&alias.params, &self.alias_budget)?;
        walk_depth(DepthNode::Type(&alias.target), Some(&self.alias_budget))?;
        let params = alias.params.clone();
        let mut target = alias.target.clone();
        fn check_target(ty: &Type) -> Result<(), String> {
            match ty {
                Type::SelfType => return Err("Self outside impl in type alias".into()),
                Type::Ref { region: None, .. } => {
                    return Err("E0106: missing lifetime specifier in type alias".into());
                }
                Type::Ref { .. } => {
                    return Err("explicit lifetime type aliases are unsupported".into());
                }
                Type::Slice(_) => return Err("unsized type aliases are unsupported".into()),
                Type::Array(t, _) => check_target(t)?,
                Type::Tuple(fields) => {
                    for field in fields {
                        check_target(field)?;
                    }
                }
                Type::Apply(path) => {
                    for segment in &path.segments {
                        for arg in &segment.args {
                            check_target(arg)?;
                        }
                    }
                }
                _ => {}
            }
            Ok(())
        }
        check_target(&target)?;
        clone_text(name, &self.alias_budget)?;
        self.alias_active.insert(name.into());
        let saved_params = std::mem::replace(&mut self.type_params, params);
        let saved_self = self.self_nominal.take();
        let result = self.ty(&mut target, parent(name));
        let params = std::mem::replace(&mut self.type_params, saved_params);
        self.self_nominal = saved_self;
        self.alias_active.remove(name);
        result?;
        validate_depth(DepthNode::Type(&target))?;
        let mut used = std::collections::HashSet::new();
        collect_source_params(&target, &mut used, &self.alias_budget, 0)?;
        for param in params {
            if !used.contains(&param) {
                return Err(format!(
                    "E0091: type parameter {param} is never used in type alias {name}"
                ));
            }
        }
        walk_depth(DepthNode::Type(&target), Some(&self.alias_budget))?;
        clone_text(name, &self.alias_budget)?;
        self.alias_cache.insert(name.into(), target.clone());
        Ok(target)
    }
    fn expand_alias(&mut self, name: &str, args: &[Type]) -> Result<Type, String> {
        let params = &self.alias_decl(name)?.params;
        if params.len() != args.len() {
            return Err(format!(
                "E0107: type alias {name} expects {} type arguments, got {}",
                params.len(),
                args.len()
            ));
        }
        clone_names(params, &self.alias_budget)?;
        let params = params.clone();
        let mut target = self.alias_template(name)?;
        fn substitute(
            ty: &mut Type,
            params: &[String],
            args: &[Type],
            budget: &ir::PlanBudget,
            depth: usize,
        ) -> Result<(), String> {
            budget.check_depth(depth)?;
            budget.charge_work(1)?;
            match ty {
                Type::Param(name) => {
                    if let Some(index) = params.iter().position(|p| p == name) {
                        walk_depth(DepthNode::Type(&args[index]), Some(budget))?;
                        *ty = args[index].clone();
                    }
                }
                Type::Apply(path) => {
                    for segment in &mut path.segments {
                        for arg in &mut segment.args {
                            substitute(arg, params, args, budget, depth + 1)?;
                        }
                    }
                }
                Type::Tuple(fields) => {
                    for field in fields {
                        substitute(field, params, args, budget, depth + 1)?;
                    }
                }
                Type::Ref { referent, .. } | Type::Array(referent, _) | Type::Slice(referent) => {
                    substitute(referent, params, args, budget, depth + 1)?
                }
                _ => {}
            }
            Ok(())
        }
        substitute(&mut target, &params, args, &self.alias_budget, 0)?;
        validate_depth(DepthNode::Type(&target))?;
        Ok(target)
    }
    /// A source alias can qualify enum variants and named-field constructors,
    /// but is not itself a value constructor or a traversable import module.
    fn alias_constructor_path(
        &mut self,
        path: &GenericPath,
        module: &str,
        named_constructor: bool,
    ) -> Result<Option<GenericPath>, String> {
        for end in 1..=path.segments.len() {
            if end == path.segments.len() && !named_constructor {
                return Ok(None);
            }
            let spelling = path.segments[..end]
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>()
                .join("::");
            let Some(item) = self.path(&spelling, module, true, false)? else {
                continue;
            };
            if item.kind != ResolvedKind::TypeAlias {
                if item.kind == ResolvedKind::Module && !path.segments[end - 1].args.is_empty() {
                    return Err("generic arguments are not allowed on modules".into());
                }
                if item.kind != ResolvedKind::Module {
                    return Ok(None);
                }
                continue;
            }
            if end == path.segments.len() && !named_constructor {
                return Ok(None);
            }
            if path.segments[end - 1].args.is_empty()
                && !self.alias_decl(&item.path)?.params.is_empty()
            {
                return Err("inferred generic type alias constructors and patterns are unsupported; supply explicit type arguments".into());
            }
            clone_vector(&path.segments[end - 1].args, &self.alias_budget)?;
            for arg in &path.segments[end - 1].args {
                walk_depth(DepthNode::Type(arg), Some(&self.alias_budget))?;
            }
            let mut args = path.segments[end - 1].args.clone();
            for arg in &mut args {
                self.ty(arg, module)?;
            }
            let target = self.expand_alias(&item.path, &args)?;
            let mut expanded = match target {
                Type::Named(name) => self.alias_source_path(&name)?,
                Type::Apply(path) => path,
                _ => return Ok(None),
            };
            for segment in &path.segments[end..] {
                clone_text(&segment.name, &self.alias_budget)?;
                clone_vector(&segment.args, &self.alias_budget)?;
                for arg in &segment.args {
                    walk_depth(DepthNode::Type(arg), Some(&self.alias_budget))?;
                }
            }
            expanded.segments.extend_from_slice(&path.segments[end..]);
            return Ok(Some(expanded));
        }
        Ok(None)
    }
    fn trait_path(&mut self, path: &mut String, module: &str) -> Result<(), String> {
        let item = match self.path(path, module, true, false)? {
            Some(item) => item,
            None => {
                if let Some(canonical) = canonical_trait_spelling(path) {
                    *path = canonical.into();
                    return Ok(());
                }
                return Err(format!("unresolved trait {path}"));
            }
        };
        if item.kind != ResolvedKind::Trait {
            return Err(format!("{} is not a trait", item.path));
        }
        *path = item.path;
        Ok(())
    }
    fn bounds(&mut self, bounds: &mut [TypeBound], module: &str) -> Result<(), String> {
        for bound in bounds {
            if !self.type_params.contains(&bound.parameter) {
                return Err("bound parameter is not in scope".into());
            }
            for path in &mut bound.traits {
                self.trait_path(path, module)?;
            }
        }
        Ok(())
    }
    fn item(
        &mut self,
        path: &str,
        requester: &str,
        module: bool,
        visible_only: bool,
    ) -> Result<Option<ResolvedItem>, String> {
        let Some(bindings) = self.items.get(path) else {
            if module {
                if let Some(canonical) = canonical_trait_spelling(path)
                    && path.contains("::")
                {
                    return Ok(Some(ResolvedItem {
                        visibility: ir::Visibility::Public,
                        path: canonical.into(),
                        module: true,
                        kind: ResolvedKind::Trait,
                    }));
                }
                if matches!(
                    path,
                    "core"
                        | "std"
                        | "core::marker"
                        | "std::marker"
                        | "core::clone"
                        | "std::clone"
                        | "core::ops"
                        | "std::ops"
                ) {
                    return Ok(Some(ResolvedItem {
                        visibility: ir::Visibility::Public,
                        path: path.into(),
                        module: true,
                        kind: ResolvedKind::Module,
                    }));
                }
            }
            return Ok(None);
        };
        let mut result = None;
        for (index, item) in bindings.iter().enumerate() {
            let resolved = match &item.kind {
                ItemKind::TypeAlias(_) if module => Some(ResolvedItem {
                    visibility: ir::Visibility::Public,
                    path: path.into(),
                    module: true,
                    kind: ResolvedKind::TypeAlias,
                }),
                ItemKind::Module if module => Some(ResolvedItem {
                    visibility: ir::Visibility::Public,
                    path: path.into(),
                    module: true,
                    kind: ResolvedKind::Module,
                }),
                ItemKind::Function if !module => Some(ResolvedItem {
                    visibility: ir::Visibility::Public,
                    path: path.into(),
                    module: false,
                    kind: ResolvedKind::Function,
                }),
                ItemKind::Trait if module => Some(ResolvedItem {
                    visibility: ir::Visibility::Public,
                    path: path.into(),
                    module: true,
                    kind: ResolvedKind::Trait,
                }),
                ItemKind::Struct if module => Some(ResolvedItem {
                    visibility: ir::Visibility::Public,
                    path: path.into(),
                    module: true,
                    kind: ResolvedKind::Struct,
                }),
                ItemKind::Enum if module => Some(ResolvedItem {
                    visibility: ir::Visibility::Public,
                    path: path.into(),
                    module: true,
                    kind: ResolvedKind::Enum,
                }),
                ItemKind::Variant(shape) if module || *shape != ir::VariantShape::Struct => {
                    Some(ResolvedItem {
                        visibility: self
                            .items
                            .get(parent(path))
                            .and_then(|items| {
                                items
                                    .iter()
                                    .find(|item| matches!(item.kind, ItemKind::Enum))
                            })
                            .map_or_else(
                                || ir::Visibility::Restricted(parent(parent(path)).into()),
                                |item| item.visibility.clone(),
                            ),
                        path: path.into(),
                        module,
                        kind: ResolvedKind::Variant,
                    })
                }
                ItemKind::Import(_, _) => self.import(path, index, module)?,
                _ => None,
            };
            if let Some(mut resolved) = resolved {
                // A mixed-namespace import may legally widen one namespace
                // while retaining a narrower binding in the other namespace.
                // Never erase that original scope when publishing the alias.
                if resolved.visibility.allows_reexport(&item.visibility) {
                    resolved.visibility = item.visibility.clone();
                }
                if !resolved.visibility.accessible_from(requester) {
                    if visible_only {
                        continue;
                    }
                    return Err(format!(
                        "private item {path} is inaccessible from module {requester}"
                    ));
                }
                if result.replace(resolved).is_some() {
                    return Err(format!(
                        "duplicate item {path} in {} namespace",
                        if module { "type" } else { "value" }
                    ));
                }
            }
        }
        Ok(result)
    }
    fn import(
        &mut self,
        path: &str,
        index: usize,
        module: bool,
    ) -> Result<Option<ResolvedItem>, String> {
        let key = (path.to_owned(), index, module);
        if let Some(resolved) = self.imports.get(&key) {
            return Ok(resolved.clone());
        }
        if self.active.len() >= 128 {
            return Err("import resolution nesting exceeds limit 128".into());
        }
        if !self.active.insert(key.clone()) {
            // This import may bind only the other namespace. Treat an active
            // edge as absent until its concrete namespace is established. A
            // cycle without a concrete item remains unresolved and is rejected.
            return Ok(None);
        }
        let ItemKind::Import(target, type_only) = &self.items[path][index].kind else {
            unreachable!()
        };
        let resolved = if *type_only && !module {
            None
        } else {
            self.path(&target.clone(), parent(path), module, true)?
        };
        if *type_only
            && resolved.as_ref().is_some_and(|item| {
                !matches!(
                    item.kind,
                    ResolvedKind::Module | ResolvedKind::Enum | ResolvedKind::Trait
                )
            })
        {
            return Err(format!("{target} is not a module in grouped self import"));
        }
        self.active.remove(&key);
        self.imports.insert(key, resolved.clone());
        Ok(resolved)
    }
    fn path(
        &mut self,
        path: &str,
        from: &str,
        module: bool,
        importing: bool,
    ) -> Result<Option<ResolvedItem>, String> {
        let parts: Vec<_> = path.split("::").collect();
        let mut at = 0;
        let mut current = match parts[0] {
            "crate" => {
                at = 1;
                ResolvedItem {
                    visibility: ir::Visibility::Public,
                    path: String::new(),
                    module: true,
                    kind: ResolvedKind::Module,
                }
            }
            "self" => {
                at = 1;
                ResolvedItem {
                    visibility: self
                        .items
                        .get(from)
                        .and_then(|items| {
                            items
                                .iter()
                                .find(|item| matches!(item.kind, ItemKind::Module))
                        })
                        .map_or(ir::Visibility::Public, |item| item.visibility.clone()),
                    path: from.into(),
                    module: true,
                    kind: ResolvedKind::Module,
                }
            }
            "super" => {
                let mut module = from;
                while parts.get(at) == Some(&"super") {
                    if module.is_empty() {
                        return Err(format!("super path {path} escapes crate root"));
                    }
                    module = parent(module);
                    at += 1;
                }
                ResolvedItem {
                    visibility: self
                        .items
                        .get(module)
                        .and_then(|items| {
                            items
                                .iter()
                                .find(|item| matches!(item.kind, ItemKind::Module))
                        })
                        .map_or(ir::Visibility::Public, |item| item.visibility.clone()),
                    path: module.into(),
                    module: true,
                    kind: ResolvedKind::Module,
                }
            }
            name => {
                at += 1;
                let Some(item) = self.item(
                    &qualify(from, name),
                    from,
                    if parts.len() == 1 { module } else { true },
                    importing && parts.len() == 1,
                )?
                else {
                    return Ok(None);
                };
                item
            }
        };
        while let Some(name) = parts.get(at) {
            if !matches!(current.kind, ResolvedKind::Module | ResolvedKind::Enum) {
                return Err(format!("{} is not a module in path {path}", current.path));
            }
            // Enum variants reserve type bindings too, but are neither usable
            // source types nor traversable nested module scopes.
            if current.kind == ResolvedKind::Enum && at + 1 != parts.len() {
                return Ok(None);
            }
            let Some(item) = self.item(
                &qualify(&current.path, name),
                from,
                if at + 1 == parts.len() { module } else { true },
                importing && at + 1 == parts.len(),
            )?
            else {
                return Ok(None);
            };
            current = item;
            at += 1;
        }
        Ok((current.module == module).then_some(current))
    }
    fn generic_path(&mut self, path: &mut GenericPath, module: &str) -> Result<(), String> {
        if path.segments.first().is_some_and(|s| s.name == "Self") {
            if self.self_nominal.is_none() {
                return Err("Self outside impl".into());
            }
            for segment in &mut path.segments {
                for arg in &mut segment.args {
                    self.ty(arg, module)?;
                }
            }
            return Ok(());
        }
        let (at, _) = path
            .segments
            .iter()
            .enumerate()
            .find(|(_, s)| !s.args.is_empty())
            .ok_or("missing explicit type arguments")?;
        let spelling = path.segments[..=at]
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join("::");
        let item = self
            .path(&spelling, module, true, false)?
            .or(self.path(&spelling, module, false, false)?)
            .ok_or_else(|| format!("unresolved generic declaration {spelling}"))?;
        if item.kind == ResolvedKind::Module {
            return Err("generic arguments are not allowed on modules".into());
        }
        let mut args = std::mem::take(&mut path.segments[at].args);
        for arg in &mut args {
            self.ty(arg, module)?;
        }
        let suffix = path.segments[at + 1..].to_vec();
        path.segments = item
            .path
            .split("::")
            .map(|name| PathSegment {
                name: name.into(),
                args: Vec::new(),
            })
            .collect();
        path.segments.last_mut().unwrap().args = args;
        path.segments.extend(suffix);
        Ok(())
    }
    fn associated_path(
        &mut self,
        path: &GenericPath,
        module: &str,
    ) -> Result<Option<(Type, String, Vec<Type>)>, String> {
        if path.segments.len() < 2 {
            return Ok(None);
        }
        let mut prefix = GenericPath {
            segments: path.segments[..path.segments.len() - 1].to_vec(),
        };
        let item = path.segments.last().unwrap();
        let owner_name = prefix.spelling();
        let mut owner = if self.type_params.contains(&owner_name) {
            Type::Param(owner_name.clone())
        } else if owner_name == "Self" {
            if let Some(nominal) = &self.self_nominal
                && self
                    .items
                    .get(&qualify(nominal, &item.name))
                    .is_some_and(|items| {
                        items.iter().any(|i| matches!(i.kind, ItemKind::Variant(_)))
                    })
            {
                return Ok(None);
            }
            Type::SelfType
        } else {
            let Some(resolved) = self.path(&owner_name, module, true, false)? else {
                return Ok(None);
            };
            if !matches!(resolved.kind, ResolvedKind::Struct | ResolvedKind::Enum) {
                return Ok(None);
            }
            if resolved.kind == ResolvedKind::Enum
                && self
                    .item(&qualify(&resolved.path, &item.name), module, false, false)?
                    .is_some_and(|i| i.kind == ResolvedKind::Variant)
            {
                return Ok(None);
            }
            if prefix.segments.iter().any(|s| !s.args.is_empty()) {
                self.generic_path(&mut prefix, module)?;
                Type::Apply(prefix)
            } else {
                Type::Named(resolved.path)
            }
        };
        self.ty(&mut owner, module)?;
        let mut args = item.args.clone();
        for arg in &mut args {
            self.ty(arg, module)?;
        }
        Ok(Some((owner, item.name.clone(), args)))
    }
    fn ty(&mut self, ty: &mut Type, module: &str) -> Result<(), String> {
        if self.type_depth >= 128 {
            return Err("type resolution nesting exceeds resource limit 128".into());
        }
        self.type_depth += 1;
        let result = self.ty_inner(ty, module);
        self.type_depth -= 1;
        result
    }
    fn ty_inner(&mut self, ty: &mut Type, module: &str) -> Result<(), String> {
        match ty {
            Type::Apply(path) => {
                self.generic_path(path, module)?;
                let name = path.spelling();
                let (_, args) = path.application()?;
                if self.items.get(&name).is_some_and(|items| {
                    items
                        .iter()
                        .any(|item| matches!(item.kind, ItemKind::TypeAlias(_)))
                }) {
                    *ty = self.expand_alias(&name, args)?;
                } else if let Some(expected) = self.nominal_arity.get(&name)
                    && *expected != args.len()
                {
                    return Err(format!(
                        "E0107: {name} expects {expected} type arguments, got {}",
                        args.len()
                    ));
                } else if !self.nominal_arity.contains_key(&name) && name != "Self" {
                    return Err(format!("{name} is not a nominal type"));
                }
            }
            Type::Param(name) => {
                if !self.type_params.contains(name) {
                    return Err(format!("unknown type parameter {name}"));
                }
            }
            Type::Ref { referent, .. } => self.ty(referent, module)?,
            Type::Array(element, _) | Type::Slice(element) => self.ty(element, module)?,
            Type::Named(name) => {
                let item = self
                    .path(name, module, true, false)?
                    .ok_or_else(|| format!("unresolved type {name}"))?;
                if item.kind == ResolvedKind::TypeAlias {
                    *ty = self.expand_alias(&item.path, &[])?;
                    return Ok(());
                }
                if !matches!(item.kind, ResolvedKind::Struct | ResolvedKind::Enum) {
                    return Err(format!("{} is a module, not a nominal type", item.path));
                }
                *name = item.path;
                if self
                    .nominal_arity
                    .get(name)
                    .is_some_and(|arity| *arity != 0)
                {
                    return Err(format!("E0107: missing type arguments for {name}"));
                }
            }
            Type::Tuple(fields) => {
                for ty in fields {
                    self.ty(ty, module)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn pattern(
        &mut self,
        pattern: &mut Pattern,
        module: &str,
        names: &mut std::collections::HashSet<String>,
    ) -> Result<(), String> {
        let source_path = match pattern {
            Pattern::Apply { path, .. } => {
                clone_path(path, &self.alias_budget)?;
                for segment in &path.segments {
                    for arg in &segment.args {
                        walk_depth(DepthNode::Type(arg), Some(&self.alias_budget))?;
                    }
                }
                Some(path.clone())
            }
            Pattern::Struct { name, .. } | Pattern::Variant { name, .. } | Pattern::Path(name) => {
                Some(self.alias_source_path(name)?)
            }
            _ => None,
        };
        if let Some(source_path) = source_path
            && let Some(expanded) = self.alias_constructor_path(&source_path, module, matches!(pattern, Pattern::Struct { .. }) || matches!(pattern, Pattern::Apply { pattern, .. } if matches!(pattern.as_ref(), Pattern::Struct { .. })))?
        {
            let canonical = expanded.spelling();
            let inner = if let Pattern::Apply { pattern, .. } = pattern { pattern.as_mut() } else { &mut *pattern };
            let variant = self.items.get(&canonical).and_then(|items| items.iter().find_map(|item| if let ItemKind::Variant(shape) = item.kind { Some(shape) } else { None }));
            match inner {
                Pattern::Struct { name, fields, rest } if variant.is_some() => {
                    let fields = std::mem::take(fields).into_iter().map(|(name,p)| (FieldSelector::Named(name),p)).collect();
                    *inner = Pattern::Variant { name: canonical, shape: variant.unwrap(), fields, rest: *rest };
                }
                Pattern::Struct { name, .. } | Pattern::Variant { name, .. } | Pattern::Path(name) => *name = canonical,
                _ => return Err("unsupported alias constructor pattern".into()),
            }
            match inner {
                Pattern::Struct { fields, .. } => for (_, field) in fields { self.pattern(field, module, names)?; },
                Pattern::Variant { fields, .. } => for (_, field) in fields { self.pattern(field, module, names)?; },
                _ => {}
            }
            if expanded.segments.iter().any(|s| !s.args.is_empty()) {
                if let Pattern::Apply { path, .. } = pattern { *path = expanded; }
                else { let inner = std::mem::replace(pattern, Pattern::Wildcard); *pattern = Pattern::Apply { path: expanded, pattern: Box::new(inner) }; }
            } else if let Pattern::Apply { pattern: inner, .. } = pattern { *pattern = *std::mem::replace(inner, Box::new(Pattern::Wildcard)); }
            return Ok(());
        }
        match pattern {
            Pattern::Apply { path, pattern } => {
                self.generic_path(path, module)?;
                self.pattern(pattern, module, names)?;
            }
            Pattern::Variant { name, fields, .. } if name.starts_with("Self::") => {
                if self.self_nominal.is_none() {
                    return Err("Self outside impl".into());
                }
                for (_, p) in fields {
                    self.pattern(p, module, names)?;
                }
            }
            Pattern::Variant { name, fields, .. } => {
                let item = self
                    .path(name, module, false, false)?
                    .ok_or_else(|| format!("unresolved enum variant {name}"))?;
                if item.kind != ResolvedKind::Variant {
                    return Err(format!("{} is not a variant", item.path));
                }
                *name = item.path;
                for (_, pattern) in fields {
                    self.pattern(pattern, module, names)?;
                }
            }
            Pattern::Wildcard => {}
            Pattern::Bool(_) | Pattern::Int { .. } => {}
            Pattern::Path(name) if name.starts_with("Self::") && self.self_nominal.is_some() => {}
            Pattern::Path(name) => {
                let item = self
                    .path(name, module, false, false)?
                    .ok_or_else(|| format!("unresolved variant pattern {name}"))?;
                if item.kind != ResolvedKind::Variant {
                    return Err(format!("{} is not an enum variant pattern", item.path));
                }
                *name = item.path;
            }
            Pattern::Binding { name, mutable } => {
                if let Some(item) = self.path(name, module, false, false)?
                    && item.kind == ResolvedKind::Variant
                {
                    if *mutable {
                        return Err(format!("binding {name} cannot shadow an enum variant"));
                    }
                    *pattern = Pattern::Path(item.path);
                    return Ok(());
                }
                if !names.insert(name.clone()) {
                    return Err(format!("duplicate pattern binding {name}"));
                }
            }
            Pattern::Tuple(fields) => {
                for pattern in fields {
                    self.pattern(pattern, module, names)?;
                }
            }
            Pattern::Struct { name, fields, rest } => {
                if let Some(item) = self.path(name, module, true, false)?
                    && item.kind == ResolvedKind::Variant
                {
                    for (_, pattern) in fields.iter_mut() {
                        self.pattern(pattern, module, names)?;
                    }
                    *pattern = Pattern::Variant {
                        name: item.path,
                        shape: ir::VariantShape::Struct,
                        fields: fields
                            .iter()
                            .map(|(n, p)| (FieldSelector::Named(n.clone()), p.clone()))
                            .collect(),
                        rest: *rest,
                    };
                    return Ok(());
                }
                let item = self
                    .path(name, module, true, false)?
                    .ok_or_else(|| format!("unresolved struct pattern {name}"))?;
                if item.kind != ResolvedKind::Struct {
                    return Err(format!("{} is not a struct pattern", item.path));
                }
                *name = item.path;
                for (_, pattern) in fields {
                    self.pattern(pattern, module, names)?;
                }
            }
        }
        Ok(())
    }
    fn block(
        &mut self,
        block: &mut Block,
        module: &str,
        locals: &mut Vec<std::collections::HashSet<String>>,
    ) -> Result<(), String> {
        locals.push(std::collections::HashSet::new());
        for stmt in &mut block.statements {
            if let Stmt::Let {
                name,
                mutable,
                ty,
                value,
            } = stmt
                && let Some(item) = self.path(name, module, false, false)?
                && item.kind == ResolvedKind::Variant
            {
                if *mutable {
                    return Err(format!("binding {name} cannot shadow an enum variant"));
                }
                *stmt = Stmt::LetPattern {
                    pattern: Pattern::Path(item.path),
                    ty: ty.clone(),
                    value: value.clone(),
                };
            }
            match stmt {
                Stmt::Declare { name, ty, .. } => {
                    self.ty(ty, module)?;
                    if let Some(item) = self.path(name, module, false, false)?
                        && item.kind == ResolvedKind::Variant
                    {
                        return Err(format!(
                            "enum variant patterns in deferred declarations are unsupported: {name}"
                        ));
                    }
                    locals.last_mut().unwrap().insert(name.clone());
                }
                Stmt::Let {
                    name, ty, value, ..
                } => {
                    self.ty(ty, module)?;
                    self.expr(value, module, locals)?;
                    locals.last_mut().unwrap().insert(name.clone());
                }
                Stmt::LetPattern { pattern, ty, value } => {
                    self.ty(ty, module)?;
                    self.expr(value, module, locals)?;
                    let mut names = std::collections::HashSet::new();
                    self.pattern(pattern, module, &mut names)?;
                    locals.last_mut().unwrap().extend(names);
                }
                Stmt::AssignDeref { carrier, value } => {
                    self.expr(value, module, locals)?;
                    self.expr(carrier, module, locals)?;
                }
                Stmt::AssignIndex { index, value, .. }
                | Stmt::AssignIndexOp { index, value, .. } => {
                    self.expr(value, module, locals)?;
                    self.expr(index, module, locals)?;
                }
                Stmt::Assign { value, .. }
                | Stmt::AssignOp { value, .. }
                | Stmt::AssignPlace { value, .. }
                | Stmt::Expr(value)
                | Stmt::ImplicitExpr(value)
                | Stmt::Return(Some(value)) => {
                    self.expr(value, module, locals)?;
                }
                Stmt::While {
                    condition, body, ..
                } => {
                    self.expr(condition, module, locals)?;
                    self.block(body, module, locals)?;
                }
                Stmt::Return(None) => {}
            }
        }
        if let Some(tail) = &mut block.tail {
            self.expr(tail, module, locals)?;
        }
        locals.pop();
        Ok(())
    }
    fn expr(
        &mut self,
        expr: &mut Expr,
        module: &str,
        locals: &mut Vec<std::collections::HashSet<String>>,
    ) -> Result<(), String> {
        if let Expr::Var(name) | Expr::Call { name, .. } = expr
            && !name.contains("::")
            && locals.iter().rev().any(|scope| scope.contains(name))
        {
            return if matches!(expr, Expr::Call { .. }) {
                Err("local binding is not callable".into())
            } else {
                Ok(())
            };
        }
        // Retain explicit applications on the erased nominal owner, not the
        // alias binder. Inner constructor lookup uses a root-qualified path so
        // canonical definition-context names cannot be captured at the use site.
        let source_path = match expr {
            Expr::Apply { path, .. } => {
                clone_path(path, &self.alias_budget)?;
                for segment in &path.segments {
                    for arg in &segment.args {
                        walk_depth(DepthNode::Type(arg), Some(&self.alias_budget))?;
                    }
                }
                Some(path.clone())
            }
            Expr::Struct { name, .. } | Expr::Call { name, .. } | Expr::Var(name) => {
                Some(self.alias_source_path(name)?)
            }
            _ => None,
        };
        if let Some(source_path) = source_path
            && let Some(expanded) = self.alias_constructor_path(&source_path, module, matches!(expr, Expr::Struct { .. }) || matches!(expr, Expr::Apply { value, .. } if matches!(value.as_ref(), Expr::Struct { .. })))?
        {
            let canonical = expanded.spelling();
            let value = if let Expr::Apply { value, .. } = expr {
                value.as_mut()
            } else {
                &mut *expr
            };
            let variant = self.items.get(&canonical).and_then(|items| items.iter().find_map(|item| if let ItemKind::Variant(shape) = item.kind { Some(shape) } else { None }));
            if let Some(shape) = variant {
                let fields = match value {
                    Expr::Var(_) if shape == ir::VariantShape::Unit => Vec::new(),
                    Expr::Call { args, .. } if shape == ir::VariantShape::Tuple => std::mem::take(args).into_iter().enumerate().map(|(i,e)| (FieldSelector::Tuple(i),e)).collect(),
                    Expr::Struct { fields, .. } if shape == ir::VariantShape::Struct => std::mem::take(fields).into_iter().map(|(n,e)| (FieldSelector::Named(n),e)).collect(),
                    _ => return Err("enum alias variant constructor shape mismatch".into()),
                };
                *value = Expr::EnumVariant { name: canonical, shape, fields };
                if let Expr::EnumVariant { fields, .. } = value { for (_, field) in fields { self.expr(field, module, locals)?; } }
            } else if self.items.get(&canonical).is_some_and(|items| items.iter().any(|i| matches!(i.kind, ItemKind::Struct))) {
                let Expr::Struct { name, fields } = value else { return Err("type alias is not a value constructor".into()); };
                *name = canonical;
                for (_, field) in fields { self.expr(field, module, locals)?; }
            } else {
                // Methods use the erased owner, with arguments in caller scope.
                let Expr::Call { args, .. } = value else { return Err("unresolved alias associated item".into()); };
                let mut owner_path = expanded;
                let method = owner_path.segments.pop().ok_or("missing associated item")?;
                let owner = if owner_path.segments.iter().any(|s| !s.args.is_empty()) { Type::Apply(owner_path) } else { Type::Named(owner_path.spelling()) };
                let mut type_args = method.args;
                for ty in &mut type_args { self.ty(ty, module)?; }
                for arg in args.iter_mut() { self.expr(arg, module, locals)?; }
                let args = std::mem::take(args);
                *expr = Expr::AssociatedCall { owner, name: method.name, type_args, args };
                return Ok(());
            }
            if expanded.segments.iter().any(|s| !s.args.is_empty()) {
                if let Expr::Apply { path, .. } = expr {
                    *path = expanded;
                } else {
                    let value = std::mem::replace(expr, Expr::Var(String::new()));
                    *expr = Expr::Apply {
                        path: expanded,
                        value: Box::new(value),
                    };
                }
            } else if let Expr::Apply { value, .. } = expr {
                *expr = *std::mem::replace(value, Box::new(Expr::Var(String::new())));
            }
            return Ok(());
        }
        match expr {
            Expr::Apply { path, value } => {
                if matches!(value.as_ref(), Expr::Call { .. })
                    && let Some((owner, name, type_args)) = self.associated_path(path, module)?
                {
                    let Expr::Call { args, .. } = value.as_mut() else {
                        unreachable!()
                    };
                    *expr = Expr::AssociatedCall {
                        owner,
                        name,
                        type_args,
                        args: std::mem::take(args),
                    };
                    return self.expr(expr, module, locals);
                }
                self.generic_path(path, module)?;
                self.expr(value, module, locals)?;
            }
            Expr::MethodCall {
                receiver,
                type_args,
                args,
                ..
            } => {
                self.expr(receiver, module, locals)?;
                for ty in type_args {
                    self.ty(ty, module)?;
                }
                for arg in args {
                    self.expr(arg, module, locals)?;
                }
            }
            Expr::AssociatedCall {
                owner,
                type_args,
                args,
                ..
            } => {
                self.ty(owner, module)?;
                for ty in type_args {
                    self.ty(ty, module)?;
                }
                for arg in args {
                    self.expr(arg, module, locals)?;
                }
            }
            Expr::Array(elements) => {
                for e in elements {
                    self.expr(e, module, locals)?;
                }
            }
            Expr::Cast { value, target } => {
                self.expr(value, module, locals)?;
                self.ty(target, module)?;
            }
            Expr::Deref { value } => self.expr(value, module, locals)?,
            Expr::Borrow { .. } => {}
            Expr::Repeat { value, .. } => self.expr(value, module, locals)?,
            Expr::DynamicIndex { base, index } => {
                self.expr(base, module, locals)?;
                self.expr(index, module, locals)?;
            }
            Expr::Index { base, .. } => self.expr(base, module, locals)?,
            Expr::Var(name) => {
                if !name.contains("::") && locals.iter().rev().any(|scope| scope.contains(name)) {
                    return Ok(());
                }
                if let Some(item) = self.path(name, module, false, false)?
                    && item.kind == ResolvedKind::Variant
                {
                    *expr = Expr::EnumVariant {
                        name: item.path,
                        shape: ir::VariantShape::Unit,
                        fields: Vec::new(),
                    };
                }
            }
            Expr::Match { scrutinee, arms } => {
                self.expr(scrutinee, module, locals)?;
                for arm in arms {
                    let mut names = std::collections::HashSet::new();
                    self.pattern(&mut arm.pattern, module, &mut names)?;
                    locals.push(names);
                    self.expr(&mut arm.body, module, locals)?;
                    locals.pop();
                }
            }
            Expr::Call { name, args } => {
                let path = GenericPath {
                    segments: name
                        .split("::")
                        .map(|s| PathSegment {
                            name: s.into(),
                            args: Vec::new(),
                        })
                        .collect(),
                };
                if let Some((owner, item, type_args)) = self.associated_path(&path, module)? {
                    let args = std::mem::take(args);
                    *expr = Expr::AssociatedCall {
                        owner,
                        name: item,
                        type_args,
                        args,
                    };
                    return self.expr(expr, module, locals);
                }
                if name.starts_with("Self::") && self.self_nominal.is_some() {
                    let variant = name.clone();
                    for arg in args.iter_mut() {
                        self.expr(arg, module, locals)?;
                    }
                    *expr = Expr::EnumVariant {
                        name: variant,
                        shape: ir::VariantShape::Tuple,
                        fields: std::mem::take(args)
                            .into_iter()
                            .enumerate()
                            .map(|(i, e)| (FieldSelector::Tuple(i), e))
                            .collect(),
                    };
                    return Ok(());
                }
                // Check the source spelling, not its eventual canonical name:
                // `let f=1; crate::f()` must not be shadowed by the local f.
                if !name.contains("::") && locals.iter().rev().any(|s| s.contains(name)) {
                    return Err(format!("local {name} is not callable"));
                }
                let resolved = self
                    .path(name, module, false, false)?
                    .ok_or_else(|| format!("unresolved function {name}"))?;
                if resolved.kind == ResolvedKind::Variant {
                    for arg in args.iter_mut() {
                        self.expr(arg, module, locals)?;
                    }
                    *expr = Expr::EnumVariant {
                        name: resolved.path,
                        shape: ir::VariantShape::Tuple,
                        fields: args
                            .iter()
                            .enumerate()
                            .map(|(i, e)| (FieldSelector::Tuple(i), e.clone()))
                            .collect(),
                    };
                    return Ok(());
                }
                if resolved.kind != ResolvedKind::Function {
                    return Err(format!("{} is not callable", resolved.path));
                }
                *name = resolved.path;
                for arg in args {
                    self.expr(arg, module, locals)?;
                }
            }
            Expr::Tuple(fields) => {
                for field in fields {
                    self.expr(field, module, locals)?;
                }
            }
            Expr::Struct { name, fields } if name == "Self" => {
                if self.self_nominal.is_none() {
                    return Err("Self outside impl".into());
                }
                for (_, value) in fields {
                    self.expr(value, module, locals)?;
                }
            }
            Expr::Struct { name, fields } => {
                if let Some(item) = self.path(name, module, true, false)?
                    && item.kind == ResolvedKind::Variant
                {
                    for (_, value) in fields.iter_mut() {
                        self.expr(value, module, locals)?;
                    }
                    *expr = Expr::EnumVariant {
                        name: item.path,
                        shape: ir::VariantShape::Struct,
                        fields: fields
                            .iter()
                            .map(|(n, e)| (FieldSelector::Named(n.clone()), e.clone()))
                            .collect(),
                    };
                    return Ok(());
                }
                let item = self
                    .path(name, module, true, false)?
                    .ok_or_else(|| format!("unresolved struct {name}"))?;
                if item.kind != ResolvedKind::Struct {
                    return Err(format!("{} is not a struct", item.path));
                }
                *name = item.path;
                for (_, value) in fields {
                    self.expr(value, module, locals)?;
                }
            }
            Expr::TraitCall {
                owner,
                trait_path,
                args,
                ..
            } => {
                self.ty(owner, module)?;
                self.trait_path(trait_path, module)?;
                for arg in args {
                    self.expr(arg, module, locals)?;
                }
            }
            Expr::Field { base, .. } => self.expr(base, module, locals)?,
            Expr::Unary { value, .. } => self.expr(value, module, locals)?,
            Expr::Binary { left, right, .. } => {
                self.expr(left, module, locals)?;
                self.expr(right, module, locals)?;
            }
            Expr::If {
                condition,
                then_block,
                else_block,
            } => {
                self.expr(condition, module, locals)?;
                self.block(then_block, module, locals)?;
                self.block(else_block, module, locals)?;
            }
            Expr::Block(block)
            | Expr::Loop { body: block, .. }
            | Expr::LabeledBlock { body: block, .. } => self.block(block, module, locals)?,
            Expr::Break {
                value: Some(value), ..
            } => self.expr(value, module, locals)?,
            _ => {}
        }
        Ok(())
    }
}

/// Resolve imports, visibility and all call paths once in the name-resolution
/// graph operation, before publishing signatures.
/// The result is backend-ready: flattened canonical function names, with root
/// `main` unchanged. Public AST construction remains supported as resolved IR.
/// Unqualified names belong to the current module only, not parent modules.
pub fn resolve(parsed: &ParsedProgram) -> Result<Program, String> {
    let mut resolver = Resolver {
        type_depth: 0,
        alias_cache: HashMap::new(),
        alias_active: std::collections::HashSet::new(),
        alias_budget: ir::PlanBudget::default(),
        nominal_arity: parsed
            .program
            .structs
            .iter()
            .map(|d| (d.name.clone(), d.type_params.len()))
            .chain(
                parsed
                    .program
                    .enums
                    .iter()
                    .map(|d| (d.name.clone(), d.type_params.len())),
            )
            .collect(),
        items: &parsed.items,
        self_nominal: None,
        imports: HashMap::new(),
        active: std::collections::HashSet::new(),
        type_params: Vec::new(),
    };
    for (module, path) in &parsed.empty_use_prefixes {
        let target = resolver
            .path(path, module, true, false)?
            .ok_or_else(|| format!("unresolved empty use group prefix {path}"))?;
        if !matches!(
            target.kind,
            ResolvedKind::Module | ResolvedKind::Enum | ResolvedKind::Trait
        ) {
            return Err(format!(
                "{} is not a module in empty use group",
                target.path
            ));
        }
    }
    // Validate unused imports too, deterministically, including cycles/privacy.
    let mut paths: Vec<_> = parsed.items.keys().cloned().collect();
    paths.sort();
    for path in paths {
        for (index, item) in parsed.items[&path].iter().enumerate() {
            if matches!(item.kind, ItemKind::Import(_, _)) {
                let m = resolver.import(&path, index, true)?;
                let f = resolver.import(&path, index, false)?;
                if m.is_none() && f.is_none() {
                    return Err(format!("unresolved or cyclic import {path}"));
                }
                // A named import can bind both namespaces. Rust permits a
                // public alias when at least one binding allows it, but its
                // private namespace must not become visible outside the owner.
                if !m
                    .as_ref()
                    .is_some_and(|target| target.visibility.allows_reexport(&item.visibility))
                    && !f
                        .as_ref()
                        .is_some_and(|target| target.visibility.allows_reexport(&item.visibility))
                {
                    return Err(format!(
                        "private item or import cannot be publicly reexported as {path}"
                    ));
                }
            }
        }
        resolver.item(&path, parent(&path), true, false)?;
        resolver.item(&path, parent(&path), false, false)?;
    }
    resolver.resolve_aliases()?;
    if !parsed.program.functions.iter().any(|f| f.name == "main")
        && let Some(target) = resolver.item("main", "", false, false)?
        && target.kind == ResolvedKind::Function
        && parsed.program.functions.iter().any(|f| {
            f.name == target.path
                && f.type_params.is_empty()
                && f.params.is_empty()
                && f.result == Type::Unit
        })
    {
        return Err(
            "imported main entry points are unsupported; declare a root main function".into(),
        );
    }
    let mut program = parsed.program.clone();
    for (path, items) in &parsed.items {
        for (index, item) in items.iter().enumerate() {
            if matches!(item.kind, ItemKind::Import(_, _))
                && let Some(target) = resolver.import(path, index, true)?
                && target.kind == ResolvedKind::Trait
            {
                program
                    .trait_imports
                    .push((parent(path).into(), target.path));
            }
        }
    }
    for declaration in &mut program.traits {
        for path in &mut declaration.supertraits {
            resolver.trait_path(path, parent(&declaration.name))?;
        }
        resolver.type_params.clear();
        resolver.self_nominal = Some("Self".into());
        for item in &mut declaration.items {
            resolver.bounds(&mut item.signature.bounds, parent(&declaration.name))?;
            resolver.ty(&mut item.signature.result, parent(&declaration.name))?;
            for (_, ty) in &mut item.signature.params {
                resolver.ty(ty, parent(&declaration.name))?;
            }
            if let Some(f) = &mut item.default {
                resolver.bounds(&mut f.bounds, parent(&declaration.name))?;
                resolver.ty(&mut f.result, parent(&declaration.name))?;
                for (_, ty) in &mut f.params {
                    resolver.ty(ty, parent(&declaration.name))?;
                }
                let mut names = f
                    .params
                    .iter()
                    .map(|(n, _)| n.clone())
                    .collect::<std::collections::HashSet<_>>();
                if item.receiver != ReceiverKind::None {
                    names.insert("self".into());
                }
                validate_depth(DepthNode::Block(&f.body))?;
                resolver.block(&mut f.body, parent(&declaration.name), &mut vec![names])?;
            }
        }
        resolver.self_nominal = None;
    }
    for enumeration in &mut program.enums {
        resolver.type_params = enumeration.type_params.clone();
        for variant in &mut enumeration.variants {
            for field in &mut variant.fields {
                validate_depth(DepthNode::Type(&field.ty))?;
                resolver.ty(&mut field.ty, parent(&enumeration.name))?;
            }
        }
    }
    for structure in &mut program.structs {
        resolver.type_params = structure.type_params.clone();
        for field in &mut structure.fields {
            validate_depth(DepthNode::Type(&field.ty))?;
            resolver.ty(&mut field.ty, parent(&structure.name))?;
        }
    }
    for function in &mut program.functions {
        resolver.type_params = function.type_params.clone();
        resolver.bounds(&mut function.bounds, parent(&function.name))?;
        resolver.ty(&mut function.result, parent(&function.name))?;
        for (name, ty) in &mut function.params {
            resolver.ty(ty, parent(&function.name))?;
            if let Some(item) = resolver.path(name, parent(&function.name), false, false)?
                && item.kind == ResolvedKind::Variant
            {
                return Err(format!(
                    "enum variant patterns in function parameters are unsupported: {name}"
                ));
            }
        }
        validate_depth(DepthNode::Block(&function.body))?;
        let mut locals = vec![function.params.iter().map(|(n, _)| n.clone()).collect()];
        resolver
            .block(&mut function.body, parent(&function.name), &mut locals)
            .map_err(|e| format!("in function {}: {e}", function.name))?;
    }
    for block in &mut program.impls {
        resolver.type_params = block.type_params.clone();
        resolver.bounds(&mut block.bounds, &block.module)?;
        if let Some(path) = &mut block.trait_path {
            resolver.trait_path(path, &block.module)?;
        }
        resolver.ty(&mut block.self_ty, &block.module)?;
        resolver.self_nominal = Some(match &block.self_ty {
            Type::Named(n) => n.clone(),
            Type::Apply(p) => p.spelling(),
            _ => return Err("impl requires a nominal self pattern".into()),
        });
        for item in &mut block.items {
            let f = &mut item.function;
            resolver.type_params = block.type_params.clone();
            for name in &f.type_params {
                if resolver.type_params.contains(name) {
                    return Err("duplicate visible generic parameter".into());
                }
                resolver.type_params.push(name.clone());
            }
            resolver.bounds(&mut f.bounds, &block.module)?;
            resolver.ty(&mut f.result, &block.module)?;
            for (_, ty) in &mut f.params {
                resolver.ty(ty, &block.module)?;
            }
            let mut names = f
                .params
                .iter()
                .map(|(n, _)| n.clone())
                .collect::<std::collections::HashSet<_>>();
            if item.receiver != ReceiverKind::None {
                names.insert("self".into());
            }
            validate_depth(DepthNode::Block(&f.body))?;
            resolver.block(&mut f.body, &block.module, &mut vec![names])?;
        }
        resolver.self_nominal = None;
    }
    Ok(program)
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
    let mut stack = vec![(root, 0usize)];
    let mut pattern_nodes = 0usize;
    while let Some((node, depth)) = stack.pop() {
        if let Some(budget) = budget {
            preflight_clone_node(&node, budget)?;
        }
        if depth > 128 {
            return Err("AST nesting exceeds limit 128".into());
        }
        let mut push = |node| stack.push((node, depth + 1));
        match node {
            DepthNode::Type(Type::Ref { referent, .. }) => push(DepthNode::Type(referent)),
            DepthNode::Type(Type::Array(element, _) | Type::Slice(element)) => {
                push(DepthNode::Type(element))
            }
            DepthNode::Type(Type::Tuple(fields)) => {
                for field in fields {
                    push(DepthNode::Type(field));
                }
            }
            DepthNode::Type(Type::Apply(path)) => {
                for segment in &path.segments {
                    for arg in &segment.args {
                        push(DepthNode::Type(arg));
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
                        push(DepthNode::Pattern(pattern));
                        for segment in &path.segments {
                            for arg in &segment.args {
                                push(DepthNode::Type(arg));
                            }
                        }
                    }
                    Pattern::Variant { fields, .. } => {
                        for (_, p) in fields {
                            push(DepthNode::Pattern(p));
                        }
                    }
                    Pattern::Tuple(fields) => {
                        for pattern in fields {
                            push(DepthNode::Pattern(pattern));
                        }
                    }
                    Pattern::Struct { fields, .. } => {
                        for (_, pattern) in fields {
                            push(DepthNode::Pattern(pattern));
                        }
                    }
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
                    push(DepthNode::Expr(receiver));
                    for ty in type_args {
                        push(DepthNode::Type(ty));
                    }
                    for arg in args {
                        push(DepthNode::Expr(arg));
                    }
                }
                Expr::TraitCall { owner, args, .. } => {
                    push(DepthNode::Type(owner));
                    for arg in args {
                        push(DepthNode::Expr(arg));
                    }
                }
                Expr::AssociatedCall {
                    owner,
                    type_args,
                    args,
                    ..
                } => {
                    push(DepthNode::Type(owner));
                    for ty in type_args {
                        push(DepthNode::Type(ty));
                    }
                    for arg in args {
                        push(DepthNode::Expr(arg));
                    }
                }
                Expr::Apply { path, value } => {
                    push(DepthNode::Expr(value));
                    for segment in &path.segments {
                        for arg in &segment.args {
                            push(DepthNode::Type(arg));
                        }
                    }
                }
                Expr::Cast { value, target } => {
                    push(DepthNode::Expr(value));
                    push(DepthNode::Type(target));
                }
                Expr::Array(elements) => {
                    for e in elements {
                        push(DepthNode::Expr(e));
                    }
                }
                Expr::Deref { value } => push(DepthNode::Expr(value)),
                Expr::Repeat { value, .. } => push(DepthNode::Expr(value)),
                Expr::DynamicIndex { base, index } => {
                    push(DepthNode::Expr(base));
                    push(DepthNode::Expr(index));
                }
                Expr::Index { base, .. } => push(DepthNode::Expr(base)),
                Expr::EnumVariant { fields, .. } => {
                    for (_, e) in fields {
                        push(DepthNode::Expr(e));
                    }
                }
                Expr::Match { scrutinee, arms } => {
                    push(DepthNode::Expr(scrutinee));
                    for arm in arms {
                        push(DepthNode::Pattern(&arm.pattern));
                        push(DepthNode::Expr(&arm.body));
                    }
                }
                Expr::Tuple(fields) => {
                    for field in fields {
                        push(DepthNode::Expr(field));
                    }
                }
                Expr::Struct { fields, .. } => {
                    for (_, value) in fields {
                        push(DepthNode::Expr(value));
                    }
                }
                Expr::Field { base, .. } => push(DepthNode::Expr(base)),
                Expr::Unary { value, .. } => push(DepthNode::Expr(value)),
                Expr::Binary { left, right, .. } => {
                    push(DepthNode::Expr(left));
                    push(DepthNode::Expr(right));
                }
                Expr::Call { args, .. } => {
                    for arg in args {
                        push(DepthNode::Expr(arg));
                    }
                }
                Expr::If {
                    condition,
                    then_block,
                    else_block,
                } => {
                    push(DepthNode::Expr(condition));
                    push(DepthNode::Block(then_block));
                    push(DepthNode::Block(else_block));
                }
                Expr::Block(block)
                | Expr::Loop { body: block, .. }
                | Expr::LabeledBlock { body: block, .. } => push(DepthNode::Block(block)),
                Expr::Break {
                    value: Some(value), ..
                } => push(DepthNode::Expr(value)),
                _ => {}
            },
            DepthNode::Block(block) => {
                for stmt in &block.statements {
                    match stmt {
                        Stmt::Declare { ty, .. } => push(DepthNode::Type(ty)),
                        Stmt::Let { value, ty, .. } => {
                            push(DepthNode::Expr(value));
                            push(DepthNode::Type(ty));
                        }
                        Stmt::LetPattern { pattern, ty, value } => {
                            push(DepthNode::Pattern(pattern));
                            push(DepthNode::Type(ty));
                            push(DepthNode::Expr(value));
                        }
                        Stmt::AssignDeref { carrier, value } => {
                            push(DepthNode::Expr(value));
                            push(DepthNode::Expr(carrier));
                        }
                        Stmt::AssignIndex { index, value, .. }
                        | Stmt::AssignIndexOp { index, value, .. } => {
                            push(DepthNode::Expr(value));
                            push(DepthNode::Expr(index));
                        }
                        Stmt::Assign { value, .. }
                        | Stmt::AssignOp { value, .. }
                        | Stmt::AssignPlace { value, .. }
                        | Stmt::Expr(value)
                        | Stmt::ImplicitExpr(value)
                        | Stmt::Return(Some(value)) => push(DepthNode::Expr(value)),
                        Stmt::While {
                            condition, body, ..
                        } => {
                            push(DepthNode::Expr(condition));
                            push(DepthNode::Block(body));
                        }
                        Stmt::Return(None) => {}
                    }
                }
                if let Some(tail) = &block.tail {
                    push(DepthNode::Expr(tail));
                }
            }
        }
    }
    Ok(())
}
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub struct DeclSemanticPath {
    pub module: Arc<[String]>,
    pub item: String,
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
    pub type_params: Arc<[String]>,
    pub env: Arc<ir::GenericEnv>,
    pub visibility: ir::Visibility,
}
#[derive(Debug)]
pub struct DeclRegistry {
    declarations: Vec<DeclInfo>,
    functions: HashMap<String, ir::DeclId>,
    nominals: HashMap<String, ir::DeclId>,
    function_ids: Vec<ir::DeclId>,
    callable_sources: Vec<CallableSource>,
    impl_ids: Vec<ir::DeclId>,
    associated_ids: Vec<Vec<ir::DeclId>>,
    traits: HashMap<String, ir::DeclId>,
    trait_ids: Vec<ir::DeclId>,
    trait_items: Vec<Vec<ir::DeclId>>,
    trait_imports: Vec<(String, String)>,
}
#[derive(Clone, Copy, Debug)]
enum CallableSource {
    Free(usize),
    Associated { implementation: usize, item: usize },
    TraitDefault { trait_index: usize, item: usize },
}
impl DeclRegistry {
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
#[derive(Debug)]
struct PlanState {
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
    pub budget: ir::PlanBudget,
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
        Type::Named(name) | Type::Param(name) => add(name.len())?,
        Type::Apply(path) => {
            for segment in &path.segments {
                add(std::mem::size_of::<PathSegment>() + segment.name.len())?;
                for arg in &segment.args {
                    add(source_type_storage(arg, budget, depth + 1)?)?;
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
        | Type::Ref {
            referent: element, ..
        } => add(source_type_storage(element, budget, depth + 1)?)?,
        _ => {}
    }
    Ok(bytes)
}
fn schema_fields_storage(fields: &[FieldDecl], budget: &ir::PlanBudget) -> Result<usize, String> {
    let mut bytes = fields
        .len()
        .checked_mul(std::mem::size_of::<FieldDecl>() + 64)
        .ok_or("generic schema allocation overflow")?;
    for field in fields {
        bytes = bytes
            .checked_add(field.name.len())
            .and_then(|n| {
                n.checked_add(match &field.visibility {
                    ir::Visibility::Public => 0,
                    ir::Visibility::Restricted(scope) => scope.len(),
                })
            })
            .and_then(|n| n.checked_add(source_type_storage(&field.ty, budget, 0).ok()?))
            .ok_or("generic schema allocation overflow")?;
    }
    Ok(bytes)
}
/// A deliberately narrow const-valid body envelope, not constant evaluation.
/// The ordinary type, region and universal move proofs remain mandatory. Moving
/// the sole argument directly into the result has no discarded value to drop,
/// called function to qualify, or executed effect to pretend is const-valid.
fn validate_const_forwarding(function: &Function, budget: &ir::PlanBudget) -> Result<(), String> {
    budget.charge_work(1)?;
    if function.constness == Constness::Runtime {
        return Ok(());
    }
    let [(parameter, ty)] = function.params.as_slice() else {
        return Err("unsupported: const fn requires exactly one forwarded parameter".into());
    };
    budget.charge_work(parameter.len())?;
    if parameter == "_"
        || !function.body.statements.is_empty()
        || !matches!(function.body.tail.as_deref(), Some(Expr::Var(name)) if name == parameter)
    {
        return Err(
            "unsupported: const fn body must only forward its parameter as the tail expression"
                .into(),
        );
    }
    // Charge and depth-check both source types before their structural equality
    // comparison, including callers constructing public ASTs without parsing.
    budget.charge_work(source_type_storage(ty, budget, 0)?)?;
    budget.charge_work(source_type_storage(&function.result, budget, 0)?)?;
    if *ty != function.result {
        return Err("unsupported: const fn forwarding result must match its parameter type".into());
    }
    Ok(())
}
impl PlanCtx {
    pub fn from_program(program: &Program, budget: &ir::PlanBudget) -> Result<Self, String> {
        preflight_program_clone(program, budget)?;
        Self::from_shared_program(Arc::new(program.clone()), budget)
    }
    pub fn from_shared_program(
        shared: Arc<Program>,
        budget: &ir::PlanBudget,
    ) -> Result<Self, String> {
        let program = shared.as_ref();
        // Signature construction sees all source declarations, not just demanded
        // instances. Unsupported unused const bodies cannot escape publication.
        for function in &program.functions {
            validate_const_forwarding(function, budget)?;
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
        let catalog_builder = tr::CoreCatalogBuilder::new(snapshot, budget)
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
        let mut import_bytes = 128usize;
        for (module, path) in &program.trait_imports {
            budget.charge_work(1)?;
            import_bytes = import_bytes
                .checked_add(module.len())
                .and_then(|n| n.checked_add(path.len()))
                .and_then(|n| n.checked_add(128))
                .ok_or("trait import allocation overflow")?;
        }
        budget.retain_bytes(import_bytes)?;
        let mut registry = DeclRegistry {
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
            trait_imports: program.trait_imports.clone(),
        };
        let mut state = PlanState {
            schemas: HashMap::new(),
            region_modes: HashMap::new(),
            source_regions: HashMap::new(),
            functions: HashMap::new(),
            nominals: NominalInterner::default(),
            contexts: HashMap::new(),
            reservations: Vec::new(),
        };
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
                    } => "core::marker::Sized",
                    tr::DeclOrigin::CoreCatalog {
                        item: tr::CatalogItem::Clone,
                        ..
                    } => "core::clone::Clone",
                    tr::DeclOrigin::CoreCatalog {
                        item: tr::CatalogItem::Copy,
                        ..
                    } => "core::marker::Copy",
                    tr::DeclOrigin::CoreCatalog {
                        item: tr::CatalogItem::Drop,
                        ..
                    } => "core::ops::Drop",
                    _ => return Err("catalog trait origin mismatch".into()),
                };
                registry.traits.insert(canonical.into(), entry.declaration);
                if let Some(rest) = canonical.strip_prefix("core::") {
                    registry
                        .traits
                        .insert(format!("std::{rest}"), entry.declaration);
                }
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
                        1 + t.items.len() + t.items.iter().filter(|i| i.default.is_some()).count(),
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
                source_storage = source_storage
                    .checked_add(variant.name.len())
                    .and_then(|n| {
                        n.checked_add(schema_fields_storage(&variant.fields, budget).ok()?)
                    })
                    .ok_or("generic schema allocation overflow")?;
            }
        }
        state.retain(budget, source_storage)?;
        let mut insert = |name: &str,
                          params: &[String],
                          kind: ir::DeclKind,
                          source_index: usize|
         -> Result<ir::DeclId, String> {
            budget.charge_work(1 + name.len() + params.len())?;
            if params.len() > budget.limits().params {
                return Err("generic parameter resource limit exceeded".into());
            }
            budget.retain_bytes(
                name.len()
                    .checked_mul(5)
                    .and_then(|n| {
                        params
                            .iter()
                            .try_fold(n, |n, p| n.checked_add(p.len().checked_mul(3)? + 64))
                    })
                    .ok_or("generic declaration allocation overflow")?,
            )?;
            let mut names = std::collections::HashSet::new();
            for p in params {
                if !names.insert(p) {
                    return Err(format!("duplicate generic parameter {p}"));
                }
            }
            let id = ir::DeclId {
                snapshot,
                index: u32::try_from(registry.declarations.len())
                    .map_err(|_| "generic declaration count overflow")?,
            };
            let param_env = Arc::new(ir::ParamEnv::new(
                ir::BinderId { declaration: id },
                params.len(),
            )?);
            let path = DeclSemanticPath {
                module: (if matches!(kind, ir::DeclKind::Impl | ir::DeclKind::TraitImpl) {
                    name
                } else {
                    parent(name)
                })
                .split("::")
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
                .into(),
                item: name.rsplit("::").next().unwrap_or(name).into(),
                kind,
                associated: None,
            };
            if matches!(
                kind,
                ir::DeclKind::Function | ir::DeclKind::Struct | ir::DeclKind::Enum
            ) {
                let table = if kind == ir::DeclKind::Function {
                    &mut registry.functions
                } else {
                    &mut registry.nominals
                };
                if table.insert(name.into(), id).is_some() {
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
                name: name.into(),
                path,
                source_index,
                param_env,
                env,
                visibility: ir::Visibility::Public,
                type_params: params.to_vec().into(),
            });
            Ok(id)
        };
        for (index, d) in program.structs.iter().enumerate() {
            let id = insert(&d.name, &d.type_params, ir::DeclKind::Struct, index)?;
            clone_visibility(&d.visibility, budget)?;
            state
                .schemas
                .insert(id, Arc::new(NominalSchema::Struct(d.clone())));
        }
        for (index, d) in program.enums.iter().enumerate() {
            let id = insert(&d.name, &d.type_params, ir::DeclKind::Enum, index)?;
            clone_visibility(&d.visibility, budget)?;
            state
                .schemas
                .insert(id, Arc::new(NominalSchema::Enum(d.clone())));
        }
        for (index, f) in program.functions.iter().enumerate() {
            let id = insert(&f.name, &f.type_params, ir::DeclKind::Function, index)?;
            registry.function_ids.push(id);
            registry.callable_sources.push(CallableSource::Free(index));
        }
        for (index, declaration) in program.traits.iter().enumerate() {
            let id = insert(
                &declaration.name,
                &["Self".into()],
                ir::DeclKind::Trait,
                index,
            )?;
            registry.traits.insert(declaration.name.clone(), id);
            registry.trait_ids.push(id);
            let mut items = Vec::new();
            for (item, method) in declaration.items.iter().enumerate() {
                items.push(insert(
                    &qualify(&declaration.name, &method.signature.name),
                    &[],
                    ir::DeclKind::TraitItem,
                    item,
                )?);
                if method.default.is_some() {
                    let id = insert(
                        &qualify(&declaration.name, &method.signature.name),
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
            registry.trait_items.push(items);
        }
        for (implementation, block) in program.impls.iter().enumerate() {
            let impl_id = insert(
                &block.module,
                &block.type_params,
                if block.trait_path.is_some() {
                    ir::DeclKind::TraitImpl
                } else {
                    ir::DeclKind::Impl
                },
                implementation,
            )?;
            registry.impl_ids.push(impl_id);
            let mut ids = Vec::new();
            for (item, method) in block.items.iter().enumerate() {
                let id = insert(
                    &qualify(&block.module, &method.function.name),
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
            let id = registry.nominals[&declaration.name];
            registry.declarations[id.index as usize].visibility = declaration.visibility.clone();
        }
        for declaration in &program.enums {
            clone_visibility(&declaration.visibility, budget)?;
            let id = registry.nominals[&declaration.name];
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
            budget.retain_bytes(block.module.len() + 128)?;
            registry.declarations[id.index as usize].path.module = block
                .module
                .split("::")
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
                .into();
        }
        for (index, source) in registry.callable_sources.iter().enumerate() {
            let CallableSource::TraitDefault { trait_index, item } = *source else {
                continue;
            };
            let owner = registry.trait_ids[trait_index];
            let trait_env = registry.declarations[owner.index as usize]
                .param_env
                .clone();
            let module = registry.declarations[owner.index as usize]
                .path
                .module
                .clone();
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
                collect_source_params(&field.ty, &mut used, budget, 0)?;
            }
            if info.type_params.iter().any(|p| !used.contains(p)) {
                return Err(format!("unused nominal type parameter in {}", info.name));
            }
        }
        for (implementation, block) in program.impls.iter().enumerate() {
            let impl_id = registry.impl_ids[implementation];
            let info = registry.declaration(impl_id)?;
            let bindings = generic_bindings(&registry, &info.env, budget)?;
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
            let nominal = match &self_ty {
                ir::Ty::Struct(t) => t.def().source_decl,
                ir::Ty::Enum(t) => t.def().source_decl,
                _ => return Err("inherent impl requires a local nominal self type".into()),
            };
            let mut used = std::collections::HashSet::new();
            collect_source_params(&block.self_ty, &mut used, budget, 0)?;
            if block.type_params.iter().any(|p| !used.contains(p)) {
                return Err("unconstrained impl parameter".into());
            }
            let ids = registry.associated_ids[implementation].clone();
            for (item, id) in ids.iter().enumerate() {
                let method = &block.items[item];
                let owner = AssociatedSemanticOwner {
                    trait_decl: block
                        .trait_path
                        .as_ref()
                        .map(|p| {
                            registry
                                .traits
                                .get(p)
                                .copied()
                                .ok_or("unknown trait impl header")
                        })
                        .transpose()?,
                    trait_item: if let Some(path) = &block.trait_path {
                        let tid = *registry.traits.get(path).ok_or("unknown trait impl")?;
                        let mut required = None;
                        for (ti, trait_id) in registry.trait_ids.iter().enumerate() {
                            budget.charge_work(1)?;
                            if *trait_id == tid {
                                for item_id in &registry.trait_items[ti] {
                                    let item_name =
                                        &registry.declarations[item_id.index as usize].path.item;
                                    budget.charge_work(
                                        1 + item_name.len().min(method.function.name.len()),
                                    )?;
                                    if item_name == &method.function.name {
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
                budget.retain_bytes(512 + block.module.len())?;
                state.contexts.insert(
                    *id,
                    CallableContext {
                        module: block
                            .module
                            .split("::")
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned)
                            .collect::<Vec<_>>()
                            .into(),
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
            let mut bindings = generic_bindings(&registry, &info.env, budget)?;
            let context = state
                .contexts
                .entry(id)
                .or_insert_with(|| CallableContext {
                    module: info.path.module.clone(),
                    self_ty: None,
                    associated_owner: None,
                    receiver: ReceiverKind::None,
                })
                .clone();
            if let Some(ty) = &context.self_ty {
                bindings.insert("Self".into(), ty.clone());
            }
            let caller = if !info.env.is_generic() {
                ir::FactsCtx::Concrete
            } else {
                ir::FactsCtx::Universal(&info.env)
            };
            let mut names = std::collections::HashSet::new();
            for (name, _) in &f.params {
                if !names.insert(name) {
                    return Err(format!("duplicate parameter {name}"));
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
                    validate_root_carrier(ty, caller, budget)?;
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
                    &f.regions,
                    &f.params,
                    &f.result,
                    (context.receiver, region_mode),
                    &params,
                    &result,
                    budget,
                )?,
                params,
                result,
            };
            state.source_regions.insert(
                id,
                original_source_regions(
                    id,
                    &f.regions,
                    &f.params,
                    &f.result,
                    &signature.ref_contract,
                    context.receiver,
                    budget,
                )?,
            );
            state.functions.insert(
                id,
                tr::GenericSchema {
                    env: info.env.clone(),
                    signature,
                    predicates: {
                        let mut rows = Vec::new();
                        append_implicit_sized(&registry, &info.env, &mut rows, budget)?;
                        if let CallableSource::Associated { implementation, .. } =
                            registry.callable_sources[index]
                        {
                            append_source_bounds(
                                &registry,
                                &bindings,
                                &program.impls[implementation].bounds,
                                &mut rows,
                                budget,
                            )?;
                        }
                        append_source_bounds(&registry, &bindings, &f.bounds, &mut rows, budget)?;
                        if let CallableSource::TraitDefault { trait_index, .. } =
                            registry.callable_sources[index]
                        {
                            rows.push(ir::Predicate::Implements(ir::TraitRef {
                                declaration: registry.trait_ids[trait_index],
                                self_ty: context.self_ty.clone().ok_or("default without Self")?,
                                args: Arc::from([]),
                            }));
                        }
                        rows.into()
                    },
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
                let mut bindings = generic_bindings(&registry, &info.env, budget)?;
                bindings.insert("Self".into(), ir::Ty::Param(self_param));
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
                    &h.regions,
                    &h.params,
                    &h.result,
                    (item.receiver, tr::RegionContractMode::Positioned),
                    &params,
                    &result,
                    budget,
                )?;
                state.source_regions.insert(
                    id,
                    original_source_regions(
                        id,
                        &h.regions,
                        &h.params,
                        &h.result,
                        &ref_contract,
                        item.receiver,
                        budget,
                    )?,
                );
                state.functions.insert(
                    id,
                    tr::GenericSchema {
                        env: info.env.clone(),
                        signature: ir::Signature {
                            ref_contract,
                            params,
                            result,
                        },
                        predicates: {
                            let mut rows = Vec::new();
                            append_source_bounds(
                                &registry, &bindings, &h.bounds, &mut rows, budget,
                            )?;
                            rows.into()
                        },
                    },
                );
            }
        }
        if !program.functions.iter().any(|f| {
            f.name == "main"
                && f.type_params.is_empty()
                && f.params.is_empty()
                && f.result == Type::Unit
        }) {
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
        let mut plan = Self {
            program: shared.clone(),
            catalog,
            world: None,
            validated: Arc::new(HashMap::new()),
            registry,
            state: Arc::new(Mutex::new(state)),
            budget: budget.clone(),
        };
        plan.validate_inherent_overlaps(budget)?;
        plan.publish_trait_world(program)?;
        Ok(plan)
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
        let mut impls = self.catalog.headers().impls.to_vec();
        let mut callables = self.catalog.headers().callables.to_vec();
        for (ti, t) in program.traits.iter().enumerate() {
            let id = self.registry.trait_ids[ti];
            let info = self.declaration(id)?;
            traits.push(tr::TraitHeader {
                declaration: id,
                self_param: ir::ParamId {
                    owner: info.param_env.binder(),
                    index: 0,
                },
                env: info.env.clone(),
                items: self.registry.trait_items[ti].clone().into(),
                supertraits: t
                    .supertraits
                    .iter()
                    .map(|path| {
                        Ok(ir::TraitRef {
                            declaration: *self
                                .registry
                                .traits
                                .get(path)
                                .ok_or("unknown supertrait")?,
                            self_ty: ir::Ty::Param(ir::ParamId {
                                owner: info.param_env.binder(),
                                index: 0,
                            }),
                            args: Arc::from([]),
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?
                    .into(),
            });
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
            let trait_decl = *self.registry.traits.get(path).ok_or("unknown trait impl")?;
            let bindings = generic_bindings(&self.registry, &info.env, budget)?;
            let caller = if info.env.is_generic() {
                ir::FactsCtx::Universal(&info.env)
            } else {
                ir::FactsCtx::Concrete
            };
            let self_ty = self.source_ty(&block.self_ty, &bindings, caller, budget)?;
            let mut predicates = Vec::new();
            append_implicit_sized(&self.registry, &info.env, &mut predicates, budget)?;
            append_source_bounds(
                &self.registry,
                &bindings,
                &block.bounds,
                &mut predicates,
                budget,
            )?;
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
                trait_ref: ir::TraitRef {
                    declaration: trait_decl,
                    self_ty,
                    args: Arc::from([]),
                },
                schema_env: info.env.clone(),
                predicates: predicates.into(),
                items: pairs.into(),
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
            let raw = self.raw_schema(*id)?;
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
        for (index, id) in self.registry.function_ids.iter().enumerate() {
            self.budget.charge_work(1)?;
            if *id == decl {
                return Ok(index);
            }
        }
        Err("declaration is not callable".into())
    }
    pub fn callable_body<'p>(
        &self,
        program: &'p Program,
        decl: ir::DeclId,
    ) -> Result<&'p Function, String> {
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
        bindings: &HashMap<String, ir::Ty>,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<ir::Ty, String> {
        self.check_budget(budget)?;
        self.state
            .lock()
            .map_err(|_| "generic plan context poisoned")?
            .source_ty(&self.registry, ty, bindings, caller, budget, 0, true)
    }
    fn application(
        &self,
        path: &GenericPath,
        bindings: &HashMap<String, ir::Ty>,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<(String, Vec<ir::Ty>), String> {
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
        let name = path.segments[..=at]
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join("::");
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
        let schema = self.raw_schema(target.declaration)?;
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
        let mut env = TypeEnv {
            structs: HashMap::new(),
            enums: HashMap::new(),
            functions: HashMap::new(),
            plan: Some(self.clone()),
            params: HashMap::new(),
            param_env: None,
            self_ty: None,
            facts: None,
        };
        for info in &self.registry.declarations {
            if info.kind == ir::DeclKind::Function && self.catalog.declaration(info.id).is_none() {
                env.functions.insert(
                    info.name.clone(),
                    state
                        .functions
                        .get(&info.id)
                        .ok_or("missing function schema")?
                        .signature
                        .clone(),
                );
            } else if matches!(info.kind, ir::DeclKind::Struct | ir::DeclKind::Enum)
                && info.param_env.arity() == 0
            {
                match state.nominals.applications.get(&(info.id, Vec::new())) {
                    Some(ir::Ty::Struct(ty)) => {
                        env.structs.insert(info.name.clone(), ty.clone());
                    }
                    Some(ir::Ty::Enum(ty)) => {
                        env.enums.insert(info.name.clone(), ty.clone());
                    }
                    _ => return Err("missing nominal application".into()),
                }
            }
        }
        if let Some(decl) = decl {
            let info = self.declaration(decl)?;
            env.params = generic_bindings(&self.registry, &info.env, &self.budget)?;
            env.self_ty = state.contexts.get(&decl).and_then(|c| c.self_ty.clone());
            if let Some(ty) = &env.self_ty {
                env.params.insert("Self".into(), ty.clone());
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
fn append_source_bounds(
    registry: &DeclRegistry,
    bindings: &HashMap<String, ir::Ty>,
    bounds: &[TypeBound],
    out: &mut Vec<ir::Predicate>,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    for row in bounds {
        budget.charge_work(1 + row.parameter.len())?;
        let ty = bindings
            .get(&row.parameter)
            .ok_or("bound parameter outside declaration")?;
        for path in &row.traits {
            budget.charge_work(1 + path.len())?;
            if out.len() >= tr::MAX_PREDICATES {
                return Err("predicate environment limit exceeded".into());
            }
            budget.retain_bytes(256)?;
            out.push(ir::Predicate::Implements(ir::TraitRef {
                declaration: *registry
                    .traits
                    .get(path)
                    .ok_or("bound does not name a trait")?,
                self_ty: ty.clone(),
                args: Arc::from([]),
            }));
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
            module: d.path.module.clone(),
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
        for ir::Predicate::Implements(r) in schema.predicates.iter() {
            let self_ty = sub(&r.self_ty)?;
            let args = r.args.iter().map(&mut sub).collect::<Result<Vec<_>, _>>()?;
            predicates.push(ir::Predicate::Implements(ir::TraitRef {
                declaration: r.declaration,
                self_ty,
                args: args.into(),
            }));
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
                budget.charge_work(1 + a.path.item.len().min(b.path.item.len()))?;
                if a.path.item != b.path.item || ao.nominal != bo.nominal {
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
                    return Err(format!(
                        "duplicate overlapping associated item {}",
                        a.path.item
                    ));
                }
            }
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    fn select_associated(
        &self,
        owner: &ir::Ty,
        name: &str,
        args: &[ir::Ty],
        caller: ir::FactsCtx<'_>,
        module: &str,
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
        Err(format!("no applicable associated item {name}"))
    }
    fn select_method(
        &self,
        receiver: &ir::Ty,
        name: &str,
        args: &[ir::Ty],
        caller: ir::FactsCtx<'_>,
        module: &str,
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
        Err(format!("no applicable method {name}"))
    }
    #[allow(clippy::too_many_arguments)]
    fn lookup_associated(
        &self,
        owner: &ir::Ty,
        name: &str,
        method_args: &[ir::Ty],
        caller: ir::FactsCtx<'_>,
        module: &str,
        dot: bool,
        filter: Option<ReceiverKind>,
        traits_only: bool,
        budget: &ir::PlanBudget,
    ) -> Result<Option<(ir::Callee, ir::Signature, ReceiverKind)>, String> {
        self.check_budget(budget)?;
        let nominal = nominal_parts(owner).map(|(id, _)| id);
        let mut selected = None;
        for id in &self.registry.function_ids {
            budget.charge_work(1)?;
            let info = self.declaration(*id)?;
            let Some(associated) = &info.path.associated else {
                continue;
            };
            budget.charge_work(1 + info.path.item.len().min(name.len()))?;
            if traits_only
                || filter
                    .is_some_and(|mode| receiver_mode(mode) != receiver_mode(associated.receiver))
                || associated.trait_decl.is_some()
                || Some(associated.nominal) != nominal
                || info.path.item != name
            {
                continue;
            }
            let impl_env = self.param_env(associated.impl_decl)?;
            let _scratch = budget.reserve_bytes(32 * 512 + 128)?;
            let mut bindings = HashMap::new();
            if !match_impl_type(
                &associated.self_pattern,
                owner,
                &impl_env,
                &mut bindings,
                budget,
                0,
            )? {
                continue;
            }
            if selected.is_some() {
                return Err("ambiguous inherent associated item".into());
            }
            if dot && associated.receiver == ReceiverKind::None {
                return Err("associated function without self is not dot-callable".into());
            }
            if info.param_env.arity() != method_args.len() {
                return Err("unsupported omitted or partial method type arguments; use all explicit method arguments".into());
            }
            if !info.visibility.accessible_from(module) {
                return Err(format!("private associated item {name}"));
            }
            budget.retain_bytes(32 * std::mem::size_of::<ir::Ty>() + 128)?;
            let mut args = Vec::with_capacity(impl_env.arity() + method_args.len());
            for index in 0..impl_env.arity() {
                args.push(
                    bindings
                        .get(&ir::ParamId {
                            owner: impl_env.binder(),
                            index: index as u16,
                        })
                        .cloned()
                        .ok_or("unconstrained impl parameter")?,
                );
            }
            args.extend_from_slice(method_args);
            let target = ir::CallTarget {
                declaration: *id,
                type_args: args.into(),
            };
            let signature = self.instantiate_signature(&target, caller, budget)?;
            let ir::FactsCtx::Validated(facts) = caller else {
                return Err("associated call requires validated facts".into());
            };
            let proof = facts
                .service()
                .discharge_direct(&target, facts)
                .map_err(|e| e.into_legacy_diagnostic(facts.budget()))?;
            if !facts
                .world()
                .signatures_equal(proof.signature(), &signature)
                .map_err(|e| e.into_legacy_diagnostic(facts.budget()))?
            {
                return Err("canonical associated signature mismatch".into());
            }
            selected = Some((ir::Callee::Direct(target), signature, associated.receiver));
        }
        if let Some(selected) = selected {
            return Ok(Some(selected));
        }
        if !traits_only {
            return Ok(None);
        }
        let ir::FactsCtx::Validated(facts) = caller else {
            return Err("trait selection requires validated facts".into());
        };
        let mut selected = None;
        for trait_id in &self.registry.trait_ids {
            budget.charge_work(1)?;
            let info = self.declaration(*trait_id)?;
            let path_module = info.path.module.join("::");
            let mut visible =
                path_module == module || self.catalog.declaration(*trait_id).is_some();
            for (import_module, path) in &self.registry.trait_imports {
                budget.charge_work(1 + path.len() + import_module.len())?;
                if import_module == module && self.registry.traits.get(path) == Some(trait_id) {
                    visible = true;
                }
            }
            for ir::Predicate::Implements(bound) in facts.predicates().predicates() {
                budget.charge_work(1)?;
                if bound.declaration == *trait_id
                    && metered_type_equal(&bound.self_ty, owner, budget, 0)?
                {
                    visible = true;
                }
            }
            if !visible {
                continue;
            }
            if let Some(call) = self.select_trait_filtered(
                owner,
                *trait_id,
                name,
                method_args,
                caller,
                dot,
                filter,
                budget,
            )? {
                if selected.is_some() {
                    return Err("ambiguous in-scope trait item".into());
                }
                selected = Some(call);
            }
        }
        Ok(selected)
    }
}
impl PlanCtx {
    #[allow(clippy::too_many_arguments)]
    fn select_trait(
        &self,
        owner: &ir::Ty,
        id: ir::DeclId,
        name: &str,
        args: &[ir::Ty],
        caller: ir::FactsCtx<'_>,
        dot: bool,
        budget: &ir::PlanBudget,
    ) -> Result<Option<(ir::Callee, ir::Signature, ReceiverKind)>, String> {
        self.select_trait_filtered(owner, id, name, args, caller, dot, None, budget)
    }
    #[allow(clippy::too_many_arguments)]
    fn select_trait_filtered(
        &self,
        owner: &ir::Ty,
        trait_id: ir::DeclId,
        name: &str,
        method_args: &[ir::Ty],
        caller: ir::FactsCtx<'_>,
        dot: bool,
        filter: Option<ReceiverKind>,
        budget: &ir::PlanBudget,
    ) -> Result<Option<(ir::Callee, ir::Signature, ReceiverKind)>, String> {
        self.check_budget(budget)?;
        if !method_args.is_empty() {
            return Err("unsupported: trait method type arguments".into());
        }
        let ir::FactsCtx::Validated(facts) = caller else {
            return Err("trait call requires validated facts".into());
        };
        let mut item = None;
        for (ti, id) in self.registry.trait_ids.iter().enumerate() {
            budget.charge_work(1)?;
            if *id == trait_id {
                for iid in &self.registry.trait_items[ti] {
                    let info = self.declaration(*iid)?;
                    budget.charge_work(1 + info.path.item.len().min(name.len()))?;
                    if info.path.item == name {
                        item = Some(*iid);
                        break;
                    }
                }
                break;
            }
        }
        let Some(item) = item else {
            return Ok(None);
        };
        let item_receiver = self.declaration(item)?.receiver;
        if filter.is_some_and(|mode| receiver_mode(mode) != receiver_mode(item_receiver)) {
            return Ok(None);
        }
        let call = ir::TraitCall {
            trait_ref: ir::TraitRef {
                declaration: trait_id,
                self_ty: owner.clone(),
                args: Arc::from([]),
            },
            item,
            method_args: Arc::from([]),
        };
        match facts
            .service()
            .prove_goal(&ir::Predicate::Implements(call.trait_ref.clone()), facts)
            .map_err(|e| e.into_legacy_diagnostic(facts.budget()))?
        {
            tr::ProofOutcome::NotProven => return Ok(None),
            tr::ProofOutcome::Proved(_) => {}
        }
        let signature = facts
            .service()
            .instantiate_trait_signature(&call, facts)
            .map_err(|e| e.into_legacy_diagnostic(facts.budget()))?;
        let receiver = self.declaration(item)?.receiver;
        if dot && receiver == ReceiverKind::None {
            return Err("trait associated function without self is not dot-callable".into());
        }
        Ok(Some((ir::Callee::TraitItem(call), signature, receiver)))
    }
}
fn generic_bindings(
    registry: &DeclRegistry,
    env: &ir::GenericEnv,
    budget: &ir::PlanBudget,
) -> Result<HashMap<String, ir::Ty>, String> {
    budget.retain_bytes(env.total_arity() * 256 + 128)?;
    let mut bindings = HashMap::new();
    for scope in env.scopes() {
        let info = registry.declaration(scope.binder().declaration)?;
        for name in info.type_params.iter() {
            budget.retain_bytes(name.len())?;
        }
        for (name, ty) in info
            .type_params
            .iter()
            .cloned()
            .zip(symbolic_args(scope, budget)?)
        {
            budget.charge_work(1 + name.len())?;
            if bindings.insert(name, ty).is_some() {
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
        .get("core::marker::Sized")
        .ok_or("missing canonical Sized identity")?;
    for scope in env.scopes() {
        let declaration = registry.declaration(scope.binder().declaration)?;
        if declaration.kind == ir::DeclKind::Trait {
            continue;
        }
        for index in 0..scope.arity() {
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
    Ok((0..env.arity())
        .map(|index| {
            ir::Ty::Param(ir::ParamId {
                owner: env.binder(),
                index: index as u16,
            })
        })
        .collect())
}
fn param_bindings(info: &DeclInfo, args: &[ir::Ty]) -> Result<HashMap<String, ir::Ty>, String> {
    if info.type_params.len() != args.len() {
        return Err(format!(
            "wrong generic arity for {}; omitted generic inference is unsupported",
            info.name
        ));
    }
    Ok(info
        .type_params
        .iter()
        .cloned()
        .zip(args.iter().cloned())
        .collect())
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
        ir::Ty::Param(p) => match caller {
            ir::FactsCtx::Universal(env) if env.contains(*p) => {}
            ir::FactsCtx::Validated(facts) if facts.env().contains(*p) => {}
            _ => return Err("foreign or unresolved generic parameter".into()),
        },
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
    ty: &Type,
    used: &mut std::collections::HashSet<String>,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<(), String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    match ty {
        Type::Param(p) => {
            clone_text(p, budget)?;
            used.insert(p.clone());
        }
        Type::Apply(path) => {
            for s in &path.segments {
                for a in &s.args {
                    collect_source_params(a, used, budget, depth + 1)?;
                }
            }
        }
        Type::Tuple(fields) => {
            for f in fields {
                collect_source_params(f, used, budget, depth + 1)?;
            }
        }
        Type::Array(t, _) | Type::Slice(t) | Type::Ref { referent: t, .. } => {
            collect_source_params(t, used, budget, depth + 1)?
        }
        _ => {}
    }
    Ok(())
}
impl PlanState {
    fn retain(&mut self, budget: &ir::PlanBudget, bytes: usize) -> Result<(), String> {
        self.reservations.push(
            budget.reserve_bytes(
                bytes
                    .checked_add(4 * std::mem::size_of::<ir::PlanReservation>() + 64)
                    .ok_or("planning reservation allocation overflow")?,
            )?,
        );
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    fn source_ty(
        &mut self,
        registry: &DeclRegistry,
        ty: &Type,
        bindings: &HashMap<String, ir::Ty>,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
        depth: usize,
        allow_ref: bool,
    ) -> Result<ir::Ty, String> {
        self.source_ty_with_transport(
            registry, ty, bindings, caller, budget, depth, allow_ref, None,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn source_ty_with_transport(
        &mut self,
        registry: &DeclRegistry,
        ty: &Type,
        bindings: &HashMap<String, ir::Ty>,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
        depth: usize,
        allow_ref: bool,
        transport: Option<&ir::NominalSubstitutionRequest<'_>>,
    ) -> Result<ir::Ty, String> {
        budget.check_depth(depth)?;
        budget.charge_work(1)?;
        budget.charge_nodes(1)?;
        Ok(match ty {
            Type::Param(p) => bindings
                .get(p)
                .cloned()
                .ok_or_else(|| format!("unknown generic parameter {p}"))?,
            Type::Apply(path) => {
                let (at, segment) = path
                    .segments
                    .iter()
                    .enumerate()
                    .find(|(_, s)| !s.args.is_empty())
                    .ok_or("missing type arguments")?;
                if at + 1 != path.segments.len() {
                    return Err("generic associated types are unsupported".into());
                }
                let name = path.spelling();
                let decl = *registry
                    .nominals
                    .get(&name)
                    .ok_or_else(|| format!("unknown nominal type {name}"))?;
                let args = segment
                    .args
                    .iter()
                    .map(|arg| {
                        self.source_ty_with_transport(
                            registry,
                            arg,
                            bindings,
                            caller,
                            budget,
                            depth + 1,
                            false,
                            transport,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                self.intern_with_transport(
                    registry,
                    decl,
                    &args,
                    caller,
                    budget,
                    depth + 1,
                    transport,
                )?
            }
            Type::SelfType => bindings
                .get("Self")
                .cloned()
                .ok_or("Self is not in an impl scope")?,
            Type::Named(name) => {
                let decl = *registry
                    .nominals
                    .get(name)
                    .ok_or_else(|| format!("unknown nominal type {name}"))?;
                self.intern_with_transport(
                    registry,
                    decl,
                    &[],
                    caller,
                    budget,
                    depth + 1,
                    transport,
                )?
            }
            Type::Ref {
                kind,
                referent: slice,
                ..
            } if matches!(slice.as_ref(), Type::Slice(_)) => {
                if !allow_ref {
                    return Err(
                        "unsupported: slice references in generic, nested or aggregate types"
                            .into(),
                    );
                }
                admit_shared_byte_slice(*kind, slice)?;
                if matches!(caller, ir::FactsCtx::Universal(_)) {
                    return Err("unsupported: shared byte slices in generic callables".into());
                }
                ir::Ty::shared_byte_slice_ty()
            }
            Type::Ref { kind, referent, .. } => {
                if !allow_ref {
                    return Err("references in generic or aggregate types are unsupported".into());
                }
                let layers = source_reference_layers(ty)?;
                let inner = self.source_ty_with_transport(
                    registry,
                    referent,
                    bindings,
                    caller,
                    budget,
                    depth + 1,
                    layers > 1,
                    transport,
                )?;
                ir::Ty::Ref {
                    kind: *kind,
                    referent: Arc::new(inner),
                }
            }
            Type::Slice(_) => {
                return Err(BARE_SLICE_UNSUPPORTED.into());
            }
            Type::Array(element, len) => {
                if *len > ir::MAX_ARRAY_LEN {
                    return Err("array length exceeds supported limit".into());
                }
                ir::Ty::Array {
                    element: Arc::new(self.source_ty_with_transport(
                        registry,
                        element,
                        bindings,
                        caller,
                        budget,
                        depth + 1,
                        false,
                        transport,
                    )?),
                    len: *len,
                }
            }
            Type::Tuple(fields) => {
                if fields.is_empty() {
                    return Err("empty tuple types must use ()".into());
                }
                ir::Ty::Tuple(
                    fields
                        .iter()
                        .map(|t| {
                            self.source_ty_with_transport(
                                registry,
                                t,
                                bindings,
                                caller,
                                budget,
                                depth + 1,
                                false,
                                transport,
                            )
                        })
                        .collect::<Result<Vec<_>, _>>()?
                        .into(),
                )
            }
            Type::Int(i) => ir::Ty::Int(*i),
            Type::Float(f) => ir::Ty::Float(*f),
            Type::Bool => ir::Ty::Bool,
            Type::Unit => ir::Ty::Unit,
            Type::Never => ir::Ty::Never,
            Type::Infer => {
                return Err(
                    "signature types cannot be inferred; generic argument inference is unsupported"
                        .into(),
                );
            }
        })
    }
    fn intern(
        &mut self,
        registry: &DeclRegistry,
        decl: ir::DeclId,
        args: &[ir::Ty],
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
        depth: usize,
    ) -> Result<ir::Ty, String> {
        self.intern_with_transport(registry, decl, args, caller, budget, depth, None)
    }
    #[allow(clippy::too_many_arguments)]
    fn intern_with_transport(
        &mut self,
        registry: &DeclRegistry,
        decl: ir::DeclId,
        args: &[ir::Ty],
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
        depth: usize,
        transport: Option<&ir::NominalSubstitutionRequest<'_>>,
    ) -> Result<ir::Ty, String> {
        budget.check_depth(depth)?;
        budget.charge_work(1 + args.len())?;
        let info = registry.declaration(decl)?;
        let bindings = param_bindings(info, args)?;
        for arg in args {
            if transport.is_some() {
                // Only IR's unforgeable structural request can reach this path.
                // The produced application is not an executable body proof.
                let structural = match caller {
                    ir::FactsCtx::Validated(facts) => ir::FactsCtx::Universal(facts.env()),
                    facts => facts,
                };
                structural.validate_ty_with_budget(arg, budget)?;
                ir::carrier_locations(arg, budget)?;
            } else {
                validate_argument(arg, caller, budget, depth + 1)?;
            }
        }
        let key_bytes = args
            .len()
            .checked_mul(std::mem::size_of::<ir::Ty>())
            .ok_or("generic key allocation overflow")?;
        let _key_scratch = budget.reserve_bytes(key_bytes)?;
        if let Some(ty) = self.nominals.applications.get(&(decl, args.to_vec())) {
            return Ok(ty.clone());
        }
        if self.nominals.active.contains(&decl) {
            return Err(format!("by-value nominal cycle involving {}", info.name));
        }
        budget.charge_application()?;
        self.retain(budget, 512 + std::mem::size_of_val(args))?;
        let active_len = self.nominals.active.len();
        self.nominals.active.push(decl);
        let result = (|| -> Result<ir::Ty, String> {
            let schema = self
                .schemas
                .get(&decl)
                .cloned()
                .ok_or("declaration is not a nominal schema")?;
            let make_fields =
                |state: &mut Self, fields: &[FieldDecl]| -> Result<Arc<[ir::FieldDef]>, String> {
                    if fields.len() > 4096 {
                        return Err("nominal field resource limit exceeded".into());
                    }
                    state.retain(
                        budget,
                        fields
                            .len()
                            .checked_mul(4 * std::mem::size_of::<ir::FieldDef>() + 128)
                            .ok_or("nominal field allocation overflow")?,
                    )?;
                    let mut names = std::collections::HashSet::new();
                    let mut result = Vec::new();
                    for field in fields {
                        clone_visibility(&field.visibility, budget)?;
                        state.retain(budget, field.name.len())?;
                        if !names.insert(&field.name) {
                            return Err(format!("duplicate field {}", field.name));
                        }
                        let ty = state.source_ty_with_transport(
                            registry,
                            &field.ty,
                            &bindings,
                            caller,
                            budget,
                            depth + 1,
                            false,
                            transport,
                        )?;
                        result.push(ir::FieldDef {
                            name: field.name.clone(),
                            ty,
                            visibility: field.visibility.clone(),
                        });
                    }
                    Ok(result.into())
                };
            // Descendant applications are completed first, so every published Arc is acyclic.
            self.retain(
                budget,
                info.name
                    .len()
                    .checked_mul(2)
                    .ok_or("nominal name allocation overflow")?,
            )?;
            let ty = match schema.as_ref() {
                NominalSchema::Struct(d) => {
                    clone_visibility(&d.visibility, budget)?;
                    if d.fields.is_empty() {
                        return Err("empty structs are unsupported".into());
                    }
                    let fields = make_fields(self, &d.fields)?;
                    let id = ir::NominalAppId {
                        snapshot: decl.snapshot,
                        index: self.nominals.ordered.len(),
                    };
                    ir::Ty::Struct(ir::StructTy::new(ir::StructDef {
                        id,
                        source_decl: decl,
                        type_args: args.to_vec().into(),
                        name: d.name.clone(),
                        module: parent(&d.name).into(),
                        visibility: d.visibility.clone(),
                        fields,
                    }))
                }
                NominalSchema::Enum(d) => {
                    clone_visibility(&d.visibility, budget)?;
                    if d.variants.is_empty() || d.variants.len() > 4096 {
                        return Err("enum variant count exceeds supported limits".into());
                    }
                    self.retain(
                        budget,
                        d.variants
                            .len()
                            .checked_mul(std::mem::size_of::<ir::VariantDef>() * 4 + 128)
                            .ok_or("enum allocation overflow")?,
                    )?;
                    let mut names = std::collections::HashSet::new();
                    let mut variants = Vec::new();
                    for v in &d.variants {
                        self.retain(
                            budget,
                            v.name
                                .len()
                                .checked_mul(2)
                                .ok_or("variant name allocation overflow")?,
                        )?;
                        if !names.insert(v.name.clone()) {
                            return Err("duplicate enum variant".into());
                        }
                        if v.shape == ir::VariantShape::Unit && !v.fields.is_empty() {
                            return Err("unit variant cannot have fields".into());
                        }
                        if v.shape == ir::VariantShape::Tuple
                            && v.fields
                                .iter()
                                .enumerate()
                                .any(|(i, f)| f.name != i.to_string())
                        {
                            return Err("tuple variant fields require positional names".into());
                        }
                        variants.push(ir::VariantDef {
                            name: v.name.clone(),
                            shape: v.shape,
                            fields: make_fields(self, &v.fields)?,
                        });
                    }
                    let id = ir::NominalAppId {
                        snapshot: decl.snapshot,
                        index: self.nominals.ordered.len(),
                    };
                    ir::Ty::Enum(ir::EnumTy::new(ir::EnumDef {
                        id,
                        source_decl: decl,
                        type_args: args.to_vec().into(),
                        name: d.name.clone(),
                        module: parent(&d.name).into(),
                        visibility: d.visibility.clone(),
                        variants: variants.into(),
                    }))
                }
            };
            if transport.is_some() {
                ir::carrier_locations(&ty, budget)?;
            } else {
                validate_argument(&ty, caller, budget, 0)?;
            }
            self.nominals.active.pop();
            self.nominals.ordered.push(ty.clone());
            self.nominals
                .applications
                .insert((decl, args.to_vec()), ty.clone());
            Ok(ty)
        })();
        self.nominals.active.truncate(active_len);
        result
    }
}
/// One immutable per-body view with a shared canonical application service.
#[derive(Clone, Debug)]
pub struct TypeEnv {
    pub structs: HashMap<String, ir::StructTy>,
    pub enums: HashMap<String, ir::EnumTy>,
    pub functions: HashMap<String, ir::Signature>,
    plan: Option<PlanCtx>,
    params: HashMap<String, ir::Ty>,
    param_env: Option<Arc<ir::GenericEnv>>,
    self_ty: Option<ir::Ty>,
    facts: Option<Arc<tr::ValidatedFacts>>,
}
impl TypeEnv {
    pub fn plan(&self) -> Result<&PlanCtx, String> {
        self.plan
            .as_ref()
            .ok_or_else(|| "missing authoritative plan".into())
    }
}
pub type Signatures = TypeEnv;
fn visible_from(owner: &str, requester: &str) -> bool {
    owner.is_empty()
        || requester == owner
        || requester
            .strip_prefix(owner)
            .is_some_and(|s| s.starts_with("::"))
}
pub fn type_env(program: &Program) -> Result<TypeEnv, String> {
    let budget = ir::PlanBudget::default();
    PlanCtx::from_program(program, &budget)?.environment(None)
}
struct LoopFrame {
    id: ir::LoopId,
    label: Option<String>,
    is_block: bool,
    accepts_value: bool,
    checking_condition: bool,
    break_ty: Option<ir::Ty>,
    expected: Option<ir::Ty>,
    can_exit: bool,
}
struct PatternLocal {
    name: String,
    ty: ir::Ty,
    mutable: bool,
    projection: Vec<ir::Projection>,
}
type ResolvedVariantPattern<'p> = (ir::EnumTy, usize, Vec<(usize, &'p Pattern)>);

const COUNT_EVAL_WORK: usize = 1_000_000;
#[derive(Clone, Copy, Debug)]
enum CountValue {
    Known(ir::IntValue),
    Dynamic,
    Unsupported,
}
#[derive(Clone, Copy)]
enum CountMemo {
    Unseen,
    Visiting,
    Done(CountValue),
}
struct CountEvaluator<'a> {
    exprs: &'a [ir::TypedExpr],
    memo: Vec<CountMemo>,
    heights: Vec<usize>,
    work: usize,
}
impl<'a> CountEvaluator<'a> {
    fn new(exprs: &'a [ir::TypedExpr]) -> Result<Self, String> {
        let bytes = exprs
            .len()
            .checked_mul(std::mem::size_of::<CountMemo>() + std::mem::size_of::<usize>())
            .ok_or("shift count memo allocation overflow")?;
        if exprs.len() > COUNT_EVAL_WORK || bytes > 64 * 1024 * 1024 {
            return Err("shift count memo resource limit exceeded".into());
        }
        Ok(Self {
            exprs,
            memo: vec![CountMemo::Unseen; exprs.len()],
            heights: vec![0; exprs.len()],
            work: 0,
        })
    }
    fn charge(&mut self) -> Result<(), String> {
        self.work += 1;
        if self.work > COUNT_EVAL_WORK {
            return Err("shift count evaluation work limit exceeded".into());
        }
        Ok(())
    }
    fn value(&self, id: ir::ExprId) -> Result<CountValue, String> {
        match self.memo.get(id.0) {
            Some(CountMemo::Done(value)) => Ok(*value),
            _ => Err("invalid shift count evaluation dependency".into()),
        }
    }
    fn evaluate(&mut self, root: ir::ExprId) -> Result<CountValue, String> {
        let mut pending = vec![(root, false, 0usize)];
        while let Some((id, ready, depth)) = pending.pop() {
            self.charge()?;
            if depth > 128 {
                return Err("shift count evaluation depth limit exceeded".into());
            }
            let expr = self
                .exprs
                .get(id.0)
                .ok_or("invalid shift count expression ID")?;
            if !ready {
                match self.memo[id.0] {
                    CountMemo::Done(_) => {
                        if depth + self.heights[id.0] > 128 {
                            return Err("shift count evaluation depth limit exceeded".into());
                        }
                        continue;
                    }
                    CountMemo::Visiting => return Err("cycle in shift count expression".into()),
                    CountMemo::Unseen => {}
                }
                // A typed integer that cannot return has no constant value.
                if expr.completion == ir::Completion::Never {
                    self.memo[id.0] = CountMemo::Done(CountValue::Dynamic);
                    continue;
                }
                let children = match &expr.kind {
                    ir::ExprKind::Int(value) => {
                        value.validate()?;
                        if expr.ty != ir::Ty::Int(value.ty) {
                            return Err("shift count constant type mismatch".into());
                        }
                        self.memo[id.0] = CountMemo::Done(CountValue::Known(*value));
                        continue;
                    }
                    ir::ExprKind::Local(_)
                    | ir::ExprKind::Call { .. }
                    | ir::ExprKind::Index { .. }
                    | ir::ExprKind::DynamicIndex { .. }
                    | ir::ExprKind::Deref { .. }
                    | ir::ExprKind::Field { .. } => {
                        self.memo[id.0] = CountMemo::Done(CountValue::Dynamic);
                        continue;
                    }
                    ir::ExprKind::Unary { value, .. } | ir::ExprKind::IntCast { value, .. } => {
                        [Some(*value), None]
                    }
                    ir::ExprKind::Binary { left, right, .. } => [Some(*left), Some(*right)],
                    ir::ExprKind::Block(block) if block.statements.is_empty() => [block.tail, None],
                    ir::ExprKind::Block(_) => {
                        self.memo[id.0] = CountMemo::Done(CountValue::Dynamic);
                        continue;
                    }
                    _ => {
                        self.memo[id.0] = CountMemo::Done(CountValue::Unsupported);
                        continue;
                    }
                };
                self.memo[id.0] = CountMemo::Visiting;
                pending.push((id, true, depth));
                for child in children.into_iter().flatten().rev() {
                    pending.push((child, false, depth + 1));
                }
                continue;
            }
            let result = match &expr.kind {
                ir::ExprKind::Block(block) => block
                    .tail
                    .map(|id| self.value(id))
                    .transpose()?
                    .unwrap_or(CountValue::Unsupported),
                ir::ExprKind::IntCast { value, to } => match self.value(*value)? {
                    CountValue::Known(value) => CountValue::Known(ir::IntValue {
                        ty: *to,
                        bits: count_cast(value, *to),
                    }),
                    other => other,
                },
                ir::ExprKind::Unary { op, value } => match self.value(*value)? {
                    CountValue::Known(value) => {
                        let bits = match op {
                            UnaryOp::Not => !value.bits & value.ty.mask(),
                            UnaryOp::Neg => {
                                if !value.ty.is_signed() {
                                    return Err("unsigned negation in shift count".into());
                                }
                                count_checked_signed(
                                    count_integer(value)
                                        .checked_neg()
                                        .ok_or("arithmetic overflow in static shift count")?,
                                    value.ty,
                                )?
                            }
                        };
                        CountValue::Known(ir::IntValue { ty: value.ty, bits })
                    }
                    other => other,
                },
                ir::ExprKind::Binary { op, left, right } => {
                    let lhs = self.value(*left)?;
                    let rhs = self.value(*right)?;
                    if matches!(op, BinaryOp::Shl | BinaryOp::Shr)
                        && let CountValue::Known(value) = rhs
                    {
                        let ir::Ty::Int(ty) = self.exprs[left.0].ty else {
                            return Err("noninteger nested shift".into());
                        };
                        count_shift_amount(value, ty)?;
                    }
                    match (lhs, rhs) {
                        (CountValue::Known(lhs), CountValue::Known(rhs)) => {
                            count_binary(*op, lhs, rhs)?
                        }
                        (CountValue::Unsupported, _) | (_, CountValue::Unsupported) => {
                            CountValue::Unsupported
                        }
                        _ => CountValue::Dynamic,
                    }
                }
                _ => return Err("invalid shift count evaluation state".into()),
            };
            if let CountValue::Known(value) = result
                && expr.ty != ir::Ty::Int(value.ty)
            {
                return Err("shift count result type mismatch".into());
            }
            let height = match &expr.kind {
                ir::ExprKind::Block(block) => {
                    block.tail.map(|id| self.heights[id.0] + 1).unwrap_or(0)
                }
                ir::ExprKind::IntCast { value, .. } | ir::ExprKind::Unary { value, .. } => {
                    self.heights[value.0] + 1
                }
                ir::ExprKind::Binary { left, right, .. } => {
                    self.heights[left.0].max(self.heights[right.0]) + 1
                }
                _ => 0,
            };
            if depth + height > 128 {
                return Err("shift count evaluation depth limit exceeded".into());
            }
            self.heights[id.0] = height;
            self.memo[id.0] = CountMemo::Done(result);
        }
        self.value(root)
    }
}
fn count_integer(value: ir::IntValue) -> i128 {
    // Sign extension by shifts avoids constructing 2^128 or negating MIN.
    let shift = 128 - value.ty.bits();
    ((value.bits << shift) as i128) >> shift
}
fn count_cast(value: ir::IntValue, to: ir::IntTy) -> u128 {
    let extended = if value.ty.is_signed() {
        count_integer(value) as u128
    } else {
        value.bits
    };
    extended & to.mask()
}
fn count_checked_signed(value: i128, ty: ir::IntTy) -> Result<u128, String> {
    if ty.bits() < 128 {
        let limit = 1i128 << (ty.bits() - 1);
        if value < -limit || value >= limit {
            return Err("arithmetic overflow in static shift count".into());
        }
    }
    Ok(value as u128 & ty.mask())
}
fn count_shift_amount(value: ir::IntValue, lhs: ir::IntTy) -> Result<u32, String> {
    value.validate()?;
    if (value.ty.is_signed() && count_integer(value) < 0) || value.bits >= u128::from(lhs.bits()) {
        let amount = if value.ty.is_signed() {
            count_integer(value).to_string()
        } else {
            value.bits.to_string()
        };
        return Err(format!(
            "shift count {amount} overflows {}-bit integer",
            lhs.bits()
        ));
    }
    Ok(value.bits as u32)
}
fn count_binary(op: BinaryOp, lhs: ir::IntValue, rhs: ir::IntValue) -> Result<CountValue, String> {
    lhs.validate()?;
    rhs.validate()?;
    let ty = lhs.ty;
    let bits = if matches!(op, BinaryOp::Shl | BinaryOp::Shr) {
        let amount = count_shift_amount(rhs, ty)?;
        match op {
            BinaryOp::Shl => (lhs.bits << amount) & ty.mask(),
            _ if ty.is_signed() => (count_integer(lhs) >> amount) as u128 & ty.mask(),
            _ => lhs.bits >> amount,
        }
    } else {
        if lhs.ty != rhs.ty {
            return Err("static shift count integer type mismatch".into());
        }
        match op {
            BinaryOp::BitAnd => lhs.bits & rhs.bits,
            BinaryOp::BitOr => lhs.bits | rhs.bits,
            BinaryOp::BitXor => lhs.bits ^ rhs.bits,
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => {
                if ty.is_signed() {
                    let a = count_integer(lhs);
                    let b = count_integer(rhs);
                    if matches!(op, BinaryOp::Div | BinaryOp::Rem)
                        && (b == 0
                            || (a
                                == count_integer(ir::IntValue {
                                    ty,
                                    bits: 1u128 << (ty.bits() - 1),
                                })
                                && b == -1))
                    {
                        return Err("invalid division in static shift count".into());
                    }
                    let value = match op {
                        BinaryOp::Add => a.checked_add(b),
                        BinaryOp::Sub => a.checked_sub(b),
                        BinaryOp::Mul => a.checked_mul(b),
                        BinaryOp::Div => a.checked_div(b),
                        _ => a.checked_rem(b),
                    }
                    .ok_or("arithmetic overflow in static shift count")?;
                    count_checked_signed(value, ty)?
                } else {
                    let a = lhs.bits;
                    let b = rhs.bits;
                    match op {
                        BinaryOp::Add => a.checked_add(b),
                        BinaryOp::Sub => a.checked_sub(b),
                        BinaryOp::Mul => a.checked_mul(b),
                        BinaryOp::Div => a.checked_div(b),
                        _ => a.checked_rem(b),
                    }
                    .filter(|v| *v <= ty.mask())
                    .ok_or("invalid arithmetic in static shift count")?
                }
            }
            _ => return Ok(CountValue::Unsupported),
        }
    };
    Ok(CountValue::Known(ir::IntValue { ty, bits }))
}
fn validate_shift_counts(
    exprs: &[ir::TypedExpr],
    queries: &[(ir::IntTy, ir::ExprId)],
) -> Result<(), String> {
    if queries.is_empty()
        && !exprs.iter().any(|e| {
            matches!(
                e.kind,
                ir::ExprKind::Binary {
                    op: BinaryOp::Shl | BinaryOp::Shr,
                    ..
                }
            )
        })
    {
        return Ok(());
    }
    let mut evaluator = CountEvaluator::new(exprs)?;
    if queries.len() > COUNT_EVAL_WORK {
        return Err("compound shift query work limit exceeded".into());
    }
    for &(ty, value) in queries {
        evaluator.charge()?;
        match evaluator.evaluate(value)? {
            CountValue::Known(value) => {
                count_shift_amount(value, ty)?;
            }
            CountValue::Dynamic => {}
            CountValue::Unsupported => {
                return Err("unsupported static-shaped compound shift count expression".into());
            }
        }
    }
    for expr in exprs {
        evaluator.charge()?;
        if let ir::ExprKind::Binary {
            op: BinaryOp::Shl | BinaryOp::Shr,
            left,
            right,
        } = expr.kind
        {
            let ir::Ty::Int(lhs) = exprs.get(left.0).ok_or("invalid shift lhs")?.ty else {
                return Err("noninteger shift lhs".into());
            };
            match evaluator.evaluate(right)? {
                CountValue::Known(value) => {
                    count_shift_amount(value, lhs)?;
                }
                CountValue::Dynamic => {}
                CountValue::Unsupported => {
                    return Err("unsupported static-shaped shift count expression".into());
                }
            }
        }
    }
    Ok(())
}
// The legacy adapter shares all constraints with source replay, but keeps its admission fence.
use super::source_replay::{CallInferenceState, InferKind, InferTerm};
enum InferField<'a> {
    Named(&'a str),
    Tuple(usize),
}
struct Inference<'c, 's> {
    checker: &'c Checker<'s>,
    core: CallInferenceState,
}
impl<'c, 's> Inference<'c, 's> {
    fn new(checker: &'c Checker<'s>, env: Arc<ir::ParamEnv>) -> Result<Self, String> {
        let budget = &checker
            .signatures
            .plan
            .as_ref()
            .ok_or("inference requires planning context")?
            .budget;
        let core = CallInferenceState::new(Some(env), budget)
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
        Ok(Self { checker, core })
    }
    fn empty(checker: &'c Checker<'s>) -> Result<Self, String> {
        let budget = &checker
            .signatures
            .plan
            .as_ref()
            .ok_or("inference requires planning context")?
            .budget;
        let core =
            CallInferenceState::new(None, budget).map_err(|e| e.into_legacy_diagnostic(budget))?;
        Ok(Self { checker, core })
    }
    fn schema(&self, ty: &ir::Ty, depth: usize) -> Result<InferTerm, String> {
        self.core.schema(ty, depth).map_err(|e| {
            e.into_legacy_diagnostic(&self.checker.signatures.plan.as_ref().unwrap().budget)
        })
    }
    fn merge(&self, a: &InferTerm, b: &InferTerm, depth: usize) -> Result<InferTerm, String> {
        self.core.merge(a, b, depth).map_err(|e| {
            e.into_legacy_diagnostic(&self.checker.signatures.plan.as_ref().unwrap().budget)
        })
    }
    fn constrain(&mut self, a: &InferTerm, b: &InferTerm, depth: usize) -> Result<(), String> {
        self.core.constrain(a, b, depth).map_err(|e| {
            e.into_legacy_diagnostic(&self.checker.signatures.plan.as_ref().unwrap().budget)
        })
    }
    fn materialize(
        &self,
        declaration: ir::DeclId,
        actuals: &[ir::Ty],
    ) -> Result<ir::Ty, FactsError> {
        let plan = self.checker.signatures.plan.as_ref().unwrap();
        plan.intern_nominal(declaration, actuals, self.checker.facts(), &plan.budget)
            .map_err(|e| source_error(&plan.budget, &e))
    }
    fn resolve(
        &self,
        term: &InferTerm,
        fallback: bool,
        depth: usize,
    ) -> Result<Option<ir::Ty>, String> {
        let budget = &self.checker.signatures.plan.as_ref().unwrap().budget;
        self.core
            .resolve(term, fallback, depth, &mut |decl, args| {
                self.materialize(decl, args)
            })
            .map_err(|e| e.into_legacy_diagnostic(budget))
    }
    fn finish(&self, fallback: bool) -> Result<Option<Vec<ir::Ty>>, String> {
        let plan = self.checker.signatures.plan.as_ref().unwrap();
        let Some(args) = self
            .core
            .finish(fallback, &mut |decl, args| self.materialize(decl, args))
            .map_err(|e| e.into_legacy_diagnostic(&plan.budget))?
        else {
            return Ok(None);
        };
        for ty in &args {
            if let Some(cap) = self.checker.canonical
                && contains_source_ref(ty, &plan.budget, 0).map_err(|e| cap.render(e))?
            {
                return Err(cap.render(FactsError::unsupported(
                    &plan.budget,
                    "new inferred reference is outside direct shared Var replay",
                )));
            }
            if matches!(ty, ir::Ty::Ref { .. }) {
                return Err("unsupported generic inference: newly inferred structural reference actual; use explicit type arguments".into());
            }
            validate_argument(ty, self.checker.facts(), &plan.budget, 0)?;
        }
        Ok(Some(args))
    }
}
impl Checker<'_> {
    fn inference_core(&self) -> Result<CallInferenceState, String> {
        let budget = &self
            .signatures
            .plan
            .as_ref()
            .ok_or("inference requires planning context")?
            .budget;
        CallInferenceState::new(None, budget).map_err(|e| e.into_legacy_diagnostic(budget))
    }
    fn infer_work(&self, depth: usize) -> Result<(), String> {
        self.inference_core()?
            .work(depth)
            .map_err(|e| e.into_legacy_diagnostic(&self.signatures.plan.as_ref().unwrap().budget))
    }
    fn infer_storage(&self, count: usize) -> Result<(), String> {
        self.inference_core()?
            .storage(count)
            .map_err(|e| e.into_legacy_diagnostic(&self.signatures.plan.as_ref().unwrap().budget))
    }
    fn infer_node(&self, kind: InferKind) -> Result<InferTerm, String> {
        self.inference_core()?
            .node(kind)
            .map_err(|e| e.into_legacy_diagnostic(&self.signatures.plan.as_ref().unwrap().budget))
    }
    fn infer_type_term(
        &self,
        ty: &ir::Ty,
        schema: Option<ir::BinderId>,
        depth: usize,
    ) -> Result<InferTerm, String> {
        self.inference_core()?
            .type_term(ty, schema, depth)
            .map_err(|e| e.into_legacy_diagnostic(&self.signatures.plan.as_ref().unwrap().budget))
    }
    fn inference_evidence(&self, expr: &Expr, depth: usize) -> Result<InferTerm, String> {
        self.infer_work(depth)?;
        // Local IDs grow monotonically, including shadowing. Scope depth distinguishes pops.
        let key = (
            expr as *const Expr as usize,
            self.locals.len(),
            self.scopes.len(),
        );
        if let Some(term) = self.inference_evidence_cache.borrow().get(&key) {
            return Ok(term.clone());
        }
        let plan = self
            .signatures
            .plan
            .as_ref()
            .ok_or("inference requires planning context")?;
        let term = match expr {
            Expr::Integer(literal) => self.infer_node(match literal.suffix {
                Some(t) => InferKind::Rigid(ir::Ty::Int(t)),
                None => InferKind::Integer,
            })?,
            Expr::Float(literal) => self.infer_node(match literal.suffix {
                Some(t) => InferKind::Rigid(ir::Ty::Float(t)),
                None => InferKind::Float,
            })?,
            Expr::Bool(_) => self.infer_node(InferKind::Rigid(ir::Ty::Bool))?,
            Expr::Var(name) => match self.lookup(name) {
                Ok(local) => self.infer_type_term(&self.locals[local.0].ty, None, depth + 1)?,
                Err(_) => self.infer_node(InferKind::Unknown(false))?,
            },
            Expr::Borrow { kind, place } => {
                plan.budget.charge_work(place.fields.len() + 1)?;
                self.infer_storage(place.fields.len() + 1)?;
                let (_, referent, _) = self.checked_place(place)?;
                self.validate_borrow_referent(*kind, &referent)?;
                let inner = self.infer_type_term(&referent, None, depth + 1)?;
                self.infer_node(InferKind::Ref(*kind, inner))?
            }
            Expr::Deref { value } => {
                let carrier = self.inference_evidence(value, depth + 1)?;
                match carrier.as_ref() {
                    InferKind::Ref(_, inner) => inner.clone(),
                    _ => self.infer_node(InferKind::Unknown(false))?,
                }
            }
            Expr::Tuple(fields) => {
                self.infer_storage(fields.len())?;
                self.infer_node(InferKind::Tuple(
                    fields
                        .iter()
                        .map(|f| self.inference_evidence(f, depth + 1))
                        .collect::<Result<Vec<_>, _>>()?
                        .into(),
                ))?
            }
            Expr::Array(fields) => {
                let mut elements = None;
                let dummy = Inference::empty(self)?;
                for f in fields {
                    let term = self.inference_evidence(f, depth + 1)?;
                    elements = Some(if let Some(old) = elements {
                        dummy.merge(&old, &term, depth + 1)?
                    } else {
                        term
                    });
                }
                self.infer_node(InferKind::Array(
                    elements.unwrap_or(self.infer_node(InferKind::Unknown(false))?),
                    fields.len(),
                ))?
            }
            Expr::Repeat { value, len } => self.infer_node(InferKind::Array(
                self.inference_evidence(value, depth + 1)?,
                *len,
            ))?,
            Expr::Unary { value, .. }
                if matches!(value.as_ref(), Expr::Integer(_) | Expr::Float(_)) =>
            {
                self.inference_evidence(value, depth + 1)?
            }
            Expr::Block(block) if block.statements.is_empty() && block.tail.is_none() => {
                self.infer_node(InferKind::Rigid(ir::Ty::Unit))?
            }
            Expr::Cast { target, .. } => {
                self.infer_type_term(&checked_local_ty(target, self.signatures)?, None, depth + 1)?
            }
            Expr::Call { name, .. } => {
                let declaration = *plan
                    .registry
                    .functions
                    .get(name)
                    .ok_or("unresolved function declaration in inference")?;
                let schema = plan.function_schema(declaration)?;
                if !schema.env.is_generic() {
                    self.infer_type_term(&schema.signature.result, None, depth + 1)?
                } else {
                    self.infer_node(InferKind::Unknown(false))?
                }
            }
            Expr::Struct { name, .. } if name == "Self" => self.infer_type_term(
                self.signatures
                    .self_ty
                    .as_ref()
                    .ok_or("Self outside impl")?,
                None,
                depth + 1,
            )?,
            Expr::TraitCall { .. } | Expr::AssociatedCall { .. } | Expr::MethodCall { .. } => {
                if let Some(ty) = self.scalar_hint(expr)? {
                    self.infer_type_term(&ty, None, depth + 1)?
                } else {
                    self.infer_node(InferKind::Unknown(false))?
                }
            }
            Expr::Struct { name, .. } | Expr::EnumVariant { name, .. } => {
                let owner = if matches!(expr, Expr::EnumVariant { .. }) {
                    name.rsplit_once("::").ok_or("invalid enum constructor")?.0
                } else {
                    name.as_str()
                };
                let decl = *plan
                    .registry
                    .nominals
                    .get(owner)
                    .ok_or("unresolved nominal declaration in inference")?;
                let (env, ty) = plan.nominal_schema(decl, &plan.budget)?;
                if env.arity() == 0 {
                    self.infer_type_term(&ty, None, depth + 1)?
                } else {
                    self.infer_node(InferKind::Unknown(false))?
                }
            }
            Expr::Apply { path, value } => {
                let (_, args) =
                    plan.application(path, &self.signatures.params, self.facts(), &plan.budget)?;
                let ty = match value.as_ref() {
                    Expr::Call { name, .. } => {
                        let declaration = *plan
                            .registry
                            .functions
                            .get(name)
                            .ok_or("unresolved function declaration in inference")?;
                        plan.instantiate_signature(
                            &ir::CallTarget {
                                declaration,
                                type_args: args.into(),
                            },
                            self.facts(),
                            &plan.budget,
                        )?
                        .result
                    }
                    Expr::Struct { name, .. } => {
                        let decl = *plan
                            .registry
                            .nominals
                            .get(name)
                            .ok_or("unresolved nominal declaration in inference")?;
                        plan.intern_nominal(decl, &args, self.facts(), &plan.budget)?
                    }
                    Expr::EnumVariant { name, .. } => {
                        let (owner, _) =
                            name.rsplit_once("::").ok_or("invalid enum constructor")?;
                        let decl = *plan
                            .registry
                            .nominals
                            .get(owner)
                            .ok_or("unresolved enum declaration in inference")?;
                        plan.intern_nominal(decl, &args, self.facts(), &plan.budget)?
                    }
                    _ => return Err("unsupported explicit generic evidence form".into()),
                };
                self.infer_type_term(&ty, None, depth + 1)?
            }
            Expr::Field { base, field } => {
                let base = self.inference_evidence(base, depth + 1)?;
                let dummy = Inference::empty(self)?;
                if let Some(ty) = dummy.resolve(&base, false, depth + 1)? {
                    let receiver = if let ir::Ty::Ref { referent, .. } = &ty {
                        referent.as_ref()
                    } else {
                        &ty
                    };
                    let (_, ty) = self.field_type(receiver, field)?;
                    self.infer_type_term(&ty, None, depth + 1)?
                } else {
                    self.infer_node(InferKind::Unknown(false))?
                }
            }
            Expr::Index { base, index } => {
                match self.inference_evidence(base, depth + 1)?.as_ref() {
                    InferKind::Array(element, len) if index < len => element.clone(),
                    InferKind::Array(_, _) => return Err("array index out of bounds".into()),
                    _ => self.infer_node(InferKind::Unknown(false))?,
                }
            }
            _ => self.infer_node(InferKind::Unknown(false))?,
        };
        self.infer_storage(1)?;
        self.inference_evidence_cache
            .borrow_mut()
            .insert(key, term.clone());
        Ok(term)
    }
    fn infer_call_args(
        &self,
        declaration: ir::DeclId,
        args: &[Expr],
        expected: Option<&ir::Ty>,
        fallback: bool,
    ) -> Result<Option<Vec<ir::Ty>>, String> {
        let plan = self
            .signatures
            .plan
            .as_ref()
            .ok_or("inference requires planning context")?;
        if let Some(cap) = self.canonical
            && let Some(actual) = cap.shared_actual(declaration, args, expected, self)?
        {
            self.infer_storage(1)?;
            let mut actuals = Vec::new();
            actuals.try_reserve_exact(1).map_err(|_| {
                cap.render(FactsError::resource(
                    &plan.budget,
                    "shared actual allocation",
                ))
            })?;
            actuals.push(actual);
            return Ok(Some(actuals));
        }
        let schema = plan.function_schema(declaration)?;
        if schema.signature.params.len() != args.len() {
            return Err("generic function argument count mismatch".into());
        }
        let mut solver = Inference::new(self, schema.env.scopes()[0].clone())?;
        if let Some(expected) = expected {
            let formal = solver.schema(&schema.signature.result, 0)?;
            let evidence = self.infer_type_term(expected, None, 0)?;
            solver.constrain(&formal, &evidence, 0)?;
        }
        for (formal, expr) in schema.signature.params.iter().zip(args) {
            let formal = solver.schema(formal, 0)?;
            let evidence = self.inference_evidence(expr, 0)?;
            solver.constrain(&formal, &evidence, 0)?;
        }
        solver.finish(fallback)
    }
    fn infer_constructor(
        &self,
        name: &str,
        variant: Option<&str>,
        fields: &[(InferField<'_>, &Expr)],
        expected: Option<&ir::Ty>,
        fallback: bool,
    ) -> Result<Option<ir::Ty>, String> {
        let plan = self
            .signatures
            .plan
            .as_ref()
            .ok_or("inference requires planning context")?;
        let declaration = *plan
            .registry
            .nominals
            .get(name)
            .ok_or("unknown nominal declaration")?;
        let (env, schema) = plan.nominal_schema(declaration, &plan.budget)?;
        let mut solver = Inference::new(self, env)?;
        if let Some(expected) = expected {
            let formal = solver.schema(&schema, 0)?;
            let evidence = self.infer_type_term(expected, None, 0)?;
            solver.constrain(&formal, &evidence, 0)?;
        }
        let definitions = match (&schema, variant) {
            (ir::Ty::Struct(ty), None) => &ty.def().fields,
            (ir::Ty::Enum(ty), Some(name)) => {
                let mut found = None;
                for variant in ty.def().variants.iter() {
                    plan.budget
                        .charge_work(1 + variant.name.len().min(name.len()))?;
                    if variant.name == name {
                        found = Some(variant);
                        break;
                    }
                }
                &found.ok_or("unknown enum variant")?.fields
            }
            _ => return Err("constructor declaration kind mismatch".into()),
        };
        for (selector, expr) in fields {
            let field = match selector {
                InferField::Tuple(index) => definitions.get(*index),
                InferField::Named(name) => {
                    let mut found = None;
                    for field in definitions.iter() {
                        plan.budget
                            .charge_work(1 + field.name.len().min(name.len()))?;
                        if field.name == *name {
                            found = Some(field);
                            break;
                        }
                    }
                    found
                }
            }
            .ok_or("unknown constructor field")?;
            let formal = solver.schema(&field.ty, 0)?;
            let evidence = self.inference_evidence(expr, 0)?;
            solver.constrain(&formal, &evidence, 0)?;
        }
        let Some(args) = solver.finish(fallback)? else {
            return Ok(None);
        };
        Ok(Some(plan.intern_nominal(
            declaration,
            &args,
            self.facts(),
            &plan.budget,
        )?))
    }
}
fn genuine_call(expr: &Expr) -> bool {
    match expr {
        Expr::Call { .. }
        | Expr::AssociatedCall { .. }
        | Expr::TraitCall { .. }
        | Expr::MethodCall { .. } => true,
        Expr::Apply { value, .. } => genuine_call(value),
        _ => false,
    }
}
fn static_source_place(mut expr: &Expr) -> Result<Place, String> {
    let mut fields = Vec::new();
    loop {
        if fields.len() >= 128 {
            return Err("receiver place nesting limit128".into());
        }
        match expr {
            Expr::Var(local) => {
                fields.reverse();
                return Ok(Place {
                    local: local.clone(),
                    fields,
                });
            }
            Expr::Field { base, field } => {
                fields.push(field.clone());
                expr = base;
            }
            Expr::Index { base, index } => {
                fields.push(FieldSelector::ConstantIndex(*index));
                expr = base;
            }
            Expr::Deref { value } => {
                fields.push(FieldSelector::Deref);
                expr = value;
            }
            _ => {
                return Err(
                    "owned rvalue or dynamic borrowed receiver places are unsupported".into(),
                );
            }
        }
    }
}
struct PendingReferenceDecl {
    name: String,
    mutable: bool,
    scope: usize,
    position: usize,
    local: Option<ir::LocalId>,
}
type CheckedApplication = (String, Vec<ir::Ty>, Arc<[ir::TypeActualRegion]>);

struct Checker<'a> {
    signatures: &'a Signatures,
    canonical: Option<&'a CanonicalSourceCapability>,
    scopes: Vec<HashMap<String, ir::LocalId>>,
    pending_references: Vec<PendingReferenceDecl>,
    locals: Vec<ir::LocalDecl>,
    exprs: Vec<ir::TypedExpr>,
    call_evidence: Vec<ir::CallEvidence>,
    declaration: Option<ir::DeclId>,
    loops: Vec<LoopFrame>,
    next_loop: usize,
    reachable: bool,
    result: ir::Ty,
    module: String,
    count_queries: Vec<(ir::IntTy, ir::ExprId)>,
    field_indices: std::cell::RefCell<HashMap<ir::NominalAppId, HashMap<String, usize>>>,
    applications: std::cell::RefCell<Vec<CheckedApplication>>,
    inference_evidence_cache: std::cell::RefCell<HashMap<(usize, usize, usize), InferTerm>>,
}
#[derive(Clone, Copy)]
enum Expectation<'a> {
    None,
    HasType(&'a ir::Ty),
    CastableToInt(ir::IntTy),
}
impl<'a> Expectation<'a> {
    fn hard(self) -> Option<&'a ir::Ty> {
        match self {
            Self::HasType(ty) => Some(ty),
            _ => None,
        }
    }
    fn literal(self, literal: &IntegerLiteral, negative: bool) -> Result<ir::IntValue, String> {
        let cast_hint = match self {
            Self::CastableToInt(to) if literal.suffix.is_none() => Some(ir::Ty::Int(to)),
            _ => None,
        };
        resolve_integer(literal, negative, self.hard().or(cast_hint.as_ref())).map_err(|error| {
            if cast_hint.is_some() && error.starts_with("integer literal out of range") {
                // A cast is a soft hint, not a source type obligation. The
                // default source integer could legally be wider than its cast.
                format!("{error}; backward numeric inference is unsupported")
            } else {
                error
            }
        })
    }
}
impl<'a> From<Option<&'a ir::Ty>> for Expectation<'a> {
    fn from(ty: Option<&'a ir::Ty>) -> Self {
        ty.map_or(Self::None, Self::HasType)
    }
}

// Semantic type and possible normal completion are independent. In particular,
// an unreachable explicit block tail retains its type, unlike a pure never value.
#[derive(Clone)]
struct Flow {
    ty: ir::Ty,
    completes: bool,
}
impl Flow {
    fn value(ty: ir::Ty) -> Self {
        Self {
            ty,
            completes: true,
        }
    }
    fn never() -> Self {
        Self {
            ty: ir::Ty::Never,
            completes: false,
        }
    }
}
fn source_region_mode(
    regions: &RegionDecls,
    params: &[(String, Type)],
    result: &Type,
    free: bool,
    budget: &ir::PlanBudget,
) -> Result<tr::RegionContractMode, String> {
    budget.charge_work(params.len() + 1)?;
    let legacy = free
        && regions.params.is_empty()
        && regions.outlives.is_empty()
        && regions.receiver.is_none()
        && !matches!(result, Type::Ref { .. })
        && params.iter().all(|(_, ty)| match ty {
            Type::Ref {
                region, referent, ..
            } => region.is_none() && !matches!(referent.as_ref(), Type::Ref { .. }),
            _ => true,
        });
    Ok(if legacy {
        tr::RegionContractMode::LegacyUnnamedFreeNoReturn
    } else {
        tr::RegionContractMode::Positioned
    })
}
// Original names are retained by the source registry. Normalization can prune
// unused declarations, but a pruned named lifetime remains a rigid source name.
fn original_source_regions(
    declaration: ir::DeclId,
    regions: &RegionDecls,
    params: &[(String, Type)],
    result: &Type,
    contract: &ir::RefContract,
    receiver: ReceiverKind,
    budget: &ir::PlanBudget,
) -> Result<Arc<ir::SourceRegionEnv>, String> {
    if regions.params.len() > ir::MAX_REGION_PARAMS {
        return Err("declared lifetime resource limit exceeded".into());
    }
    budget.charge_work(regions.params.len() + regions.outlives.len() + params.len() + 1)?;
    budget.retain_bytes(
        regions
            .params
            .len()
            .checked_mul(256)
            .and_then(|n| {
                regions
                    .outlives
                    .len()
                    .checked_mul(64)
                    .and_then(|m| n.checked_add(m))
            })
            .and_then(|n| n.checked_add(512))
            .ok_or("source lifetime allocation overflow")?,
    )?;
    let mut names = HashMap::new();
    for (index, name) in regions.params.iter().enumerate() {
        budget.charge_work(name.len() + 1)?;
        if names.insert(name.as_str(), index).is_some() {
            return Err("duplicate source lifetime".into());
        }
    }
    let lookup = |name: &str| -> Result<ir::SourceRegionId, String> {
        budget.charge_work(name.len() + 1)?;
        Ok(ir::SourceRegionId {
            declaration,
            index: *names.get(name).ok_or("undeclared source lifetime")? as u16,
        })
    };
    let mut normalized = vec![None; regions.params.len()];
    let mut bind = |name: Option<&str>, region: ir::RegionParamId| -> Result<(), String> {
        if let Some(name) = name {
            let source = lookup(name)?;
            let slot = &mut normalized[source.index as usize];
            if slot.is_some_and(|old| old != region) {
                return Err("source lifetime normalization disagrees with canonical schema".into());
            }
            *slot = Some(region);
        }
        Ok(())
    };
    // A receiver is present even when its lifetime is elided. The source params
    // exclude it; canonical input positions retain its actual parameter index.
    let offset = usize::from(receiver != ReceiverKind::None);
    if matches!(receiver, ReceiverKind::Borrowed { .. }) {
        bind(
            regions.receiver.as_deref(),
            contract
                .input_regions
                .first()
                .ok_or("borrowed receiver missing canonical region")?
                .region,
        )?;
    }
    for (index, (_, source)) in params.iter().enumerate() {
        let mut ty = source;
        let mut depth = 0u8;
        while let Type::Ref {
            region, referent, ..
        } = ty
        {
            budget.charge_work(1)?;
            // Canonical row lookup is bounded and explicitly charged.
            let mut found = None;
            for row in contract.input_regions.iter() {
                budget.charge_work(1)?;
                if row.input.param == index + offset && row.input.position.depth == depth {
                    found = Some(row.region);
                    break;
                }
            }
            if let Some(canonical) = found {
                bind(region.as_deref(), canonical)?;
            } else if region.is_some() {
                return Err("named source input missing canonical region".into());
            }
            depth = depth
                .checked_add(1)
                .ok_or("source lifetime nesting overflow")?;
            ty = referent;
        }
    }
    let mut ty = result;
    let mut depth = 0usize;
    while let Type::Ref {
        region, referent, ..
    } = ty
    {
        budget.charge_work(1)?;
        let canonical = contract
            .results
            .get(depth)
            .ok_or("source result missing canonical region")?;
        bind(region.as_deref(), canonical.region)?;
        depth += 1;
        ty = referent;
    }
    let mut outlives = Vec::with_capacity(regions.outlives.len());
    for (longer, shorter) in &regions.outlives {
        outlives.push((lookup(longer)?, lookup(shorter)?));
    }
    Ok(Arc::new(ir::SourceRegionEnv {
        declaration,
        region_count: regions.params.len() as u16,
        outlives: outlives.into(),
        normalized: normalized.into(),
    }))
}
fn canonical_ref_contract(
    regions: &RegionDecls,
    source_params: &[(String, Type)],
    source_result: &Type,
    source: (ReceiverKind, tr::RegionContractMode),
    params: &[ir::Ty],
    result: &ir::Ty,
    budget: &ir::PlanBudget,
) -> Result<Arc<ir::RefContract>, String> {
    let (receiver, mode) = source;
    if regions.params.len() > 64 || regions.outlives.len() > 4096 {
        return Err("declared lifetime resource limit exceeded".into());
    }
    budget.charge_work(regions.params.len() + regions.outlives.len() + source_params.len() + 1)?;
    let bytes = regions
        .params
        .len()
        .checked_mul(256)
        .and_then(|n| {
            source_params
                .len()
                .checked_mul(256)
                .and_then(|m| n.checked_add(m))
        })
        .and_then(|n| {
            regions
                .outlives
                .len()
                .checked_mul(32)
                .and_then(|m| n.checked_add(m))
        })
        .and_then(|n| n.checked_add(4096))
        .ok_or("region allocation overflow")?;
    let _scratch = budget.reserve_bytes(bytes)?;
    let mut ids = HashMap::new();
    for (i, name) in regions.params.iter().enumerate() {
        budget.charge_work(name.len() + 1)?;
        if ids
            .insert(name.as_str(), ir::RegionParamId(i as u16))
            .is_some()
        {
            return Err(format!("duplicate lifetime parameter {name}"));
        }
    }
    let lookup = |name: &str| {
        budget.charge_work(name.len() + 1)?;
        ids.get(name)
            .copied()
            .ok_or_else(|| format!("undeclared lifetime {name}"))
    };
    let mut edges = Vec::with_capacity(regions.outlives.len() + 128);
    for (long, short) in &regions.outlives {
        edges.push((lookup(long)?, lookup(short)?));
    }
    if mode == tr::RegionContractMode::LegacyUnnamedFreeNoReturn {
        return ir::normalize_ref_contract(&ir::RefContract::default(), budget);
    }
    let mut count = ids.len();
    let mut fresh = |name: Option<&str>| -> Result<ir::RegionParamId, String> {
        if let Some(name) = name {
            return lookup(name);
        }
        if count >= ir::MAX_REGION_PARAMS {
            return Err("combined lifetime regions exceed supported limit64".into());
        }
        let id = ir::RegionParamId(count as u16);
        count += 1;
        Ok(id)
    };
    let injected = params
        .len()
        .checked_sub(source_params.len())
        .ok_or("source parameter arity mismatch")?;
    if injected > 1 || (injected == 0 && receiver != ReceiverKind::None) {
        return Err("invalid injected receiver arity".into());
    }
    let mut inputs = Vec::with_capacity(params.len().saturating_mul(2));
    let receiver_region = if matches!(receiver, ReceiverKind::Borrowed { .. }) {
        let region = fresh(regions.receiver.as_deref())?;
        inputs.push(ir::InputRegion {
            input: ir::InputReference {
                param: 0,
                position: ir::ReferencePosition { depth: 0 },
            },
            region,
        });
        Some(region)
    } else {
        None
    };
    for (index, (_, source_ty)) in source_params.iter().enumerate() {
        let mut ty = source_ty;
        let mut previous = None;
        let mut depth = 0;
        while let Type::Ref {
            region, referent, ..
        } = ty
        {
            if depth >= 128 {
                return Err("reference depth exceeds supported limit128".into());
            }
            let region = fresh(region.as_deref())?;
            if let Some(outer) = previous {
                edges.push((region, outer));
            }
            inputs.push(ir::InputRegion {
                input: ir::InputReference {
                    param: index + injected,
                    position: ir::ReferencePosition { depth },
                },
                region,
            });
            previous = Some(region);
            depth += 1;
            ty = referent;
        }
    }
    let mut results = Vec::with_capacity(2);
    let mut ty = source_result;
    let mut previous = None;
    while let Type::Ref {
        region, referent, ..
    } = ty
    {
        if results.len() >= 128 {
            return Err("reference depth exceeds supported limit128".into());
        }
        let region = if let Some(name) = region {
            lookup(name)?
        } else if let Some(region) = receiver_region {
            region
        } else {
            if inputs.len() != 1 {
                return Err("missing lifetime specifier: elided reference output requires exactly one input reference lifetime position".into());
            }
            inputs[0].region
        };
        if let Some(outer) = previous {
            edges.push((region, outer));
        }
        results.push(ir::ResultReference {
            position: ir::ReferencePosition {
                depth: results.len() as u8,
            },
            region,
            returns: ir::ReturnContract::NoReference,
        });
        previous = Some(region);
        ty = referent;
    }
    budget.retain_bytes(
        inputs
            .len()
            .checked_mul(64)
            .and_then(|n| edges.len().checked_mul(32).and_then(|m| n.checked_add(m)))
            .and_then(|n| n.checked_add(512))
            .ok_or("region allocation overflow")?,
    )?;
    let raw = ir::RefContract {
        region_count: count as u16,
        input_regions: inputs.into(),
        results: results.into(),
        outlives: edges.into(),
    };
    let normalized = ir::normalize_ref_contract(&raw, budget)?;
    let results = ir::derive_return_contracts(params, result, &normalized, budget)?;
    budget.retain_bytes(std::mem::size_of::<ir::RefContract>() + 64)?;
    Ok(Arc::new(ir::RefContract {
        results,
        ..(*normalized).clone()
    }))
}

/// Values may carry a slice only as exactly shared `&[u8]`.
fn admit_slice_value(ty: &ir::Ty) -> Result<(), String> {
    if ty.contains_slice() && !ty.shared_byte_slice() {
        return Err(if matches!(ty, ir::Ty::Slice { .. }) {
            BARE_SLICE_UNSUPPORTED.into()
        } else {
            "unsupported: slices nested in aggregate or reference types".into()
        });
    }
    Ok(())
}
fn reject_slice_result(result: &ir::Ty) -> Result<(), String> {
    if result.contains_slice() {
        return Err("unsupported: slice return types".into());
    }
    Ok(())
}
const BARE_SLICE_UNSUPPORTED: &str = "unsupported: bare by-value slice type [T]; only shared &[u8] parameters and locals are admitted";

/// Phase-1 slice admission: exactly a shared reference to `[u8]`.
fn admit_shared_byte_slice(kind: ir::BorrowKind, slice: &Type) -> Result<(), String> {
    let Type::Slice(element) = slice else {
        return Err("internal: slice admission on non-slice type".into());
    };
    if kind != ir::BorrowKind::Shared {
        return Err("unsupported: mutable slice references (&mut [T])".into());
    }
    match element.as_ref() {
        Type::Int(ir::IntTy::U8) => Ok(()),
        Type::Param(_) => Err("unsupported: generic slice element types (&[T])".into()),
        _ => Err("unsupported: non-u8 slice element types; only &[u8] is admitted".into()),
    }
}

fn source_reference_layers(ty: &Type) -> Result<usize, String> {
    let mut inner = ty;
    let mut layers = 0;
    while let Type::Ref { referent, .. } = inner {
        layers += 1;
        if layers > 128 {
            return Err("reference type nesting exceeds supported limit128".into());
        }
        inner = referent;
    }
    Ok(layers)
}
fn checked_local_ty(ty: &Type, env: &TypeEnv) -> Result<ir::Ty, String> {
    let mut inner = ty;
    let mut named = false;
    for _ in 0..source_reference_layers(ty)? {
        if let Type::Ref {
            region, referent, ..
        } = inner
        {
            named |= region.is_some();
            inner = referent;
        }
    }
    if named {
        return Err("named lifetime annotations on locals are unsupported; use an elided local reference annotation".into());
    }
    checked_ty(ty, env)
}
fn checked_ty(ty: &Type, env: &TypeEnv) -> Result<ir::Ty, String> {
    checked_ty_at(ty, env, true)
}
fn checked_ty_at(ty: &Type, env: &TypeEnv, allow_ref: bool) -> Result<ir::Ty, String> {
    if let Some(plan) = &env.plan {
        if !allow_ref && matches!(ty, Type::Ref { .. }) {
            return Err("nested or aggregate references are unsupported".into());
        }
        let caller = env
            .param_env
            .as_deref()
            .map_or(ir::FactsCtx::Concrete, ir::FactsCtx::Universal);
        return plan.source_ty(ty, &env.params, caller, &plan.budget);
    }
    match ty {
        Type::SelfType => env
            .self_ty
            .clone()
            .ok_or_else(|| "Self outside impl".into()),
        Type::Apply(_) | Type::Param(_) => {
            Err("generic types require authoritative declaration environment".into())
        }
        Type::Slice(_) => Err(BARE_SLICE_UNSUPPORTED.into()),
        Type::Ref { referent, .. } if matches!(referent.as_ref(), Type::Slice(_)) => {
            Err("unsupported: shared byte slices require the planning frontend".into())
        }
        Type::Ref { kind, referent, .. } => {
            if !allow_ref || matches!(referent.as_ref(), Type::Never | Type::Infer) {
                return Err("nested or aggregate reference types are unsupported in B1".into());
            }
            Ok(ir::Ty::Ref {
                kind: *kind,
                referent: std::sync::Arc::new(checked_ty_at(
                    referent,
                    env,
                    source_reference_layers(ty)? > 1,
                )?),
            })
        }
        Type::Array(element, len) => {
            if *len > ir::MAX_ARRAY_LEN {
                return Err("array length exceeds supported limit".into());
            }
            Ok(ir::Ty::Array {
                element: std::sync::Arc::new(checked_ty_at(element, env, false)?),
                len: *len,
            })
        }
        Type::Int(ty) => Ok(ir::Ty::Int(*ty)),
        Type::Float(ty) => Ok(ir::Ty::Float(*ty)),
        Type::Bool => Ok(ir::Ty::Bool),
        Type::Unit => Ok(ir::Ty::Unit),
        Type::Never => Ok(ir::Ty::Never),
        Type::Tuple(fields) if !fields.is_empty() => fields
            .iter()
            .map(|ty| checked_ty_at(ty, env, false))
            .collect::<Result<Vec<_>, _>>()
            .map(|fields| ir::Ty::Tuple(fields.into())),
        Type::Tuple(_) => Err("empty tuple types must use ()".into()),
        Type::Named(name) => env
            .structs
            .get(name)
            .cloned()
            .map(ir::Ty::Struct)
            .or_else(|| env.enums.get(name).cloned().map(ir::Ty::Enum))
            .ok_or_else(|| format!("unknown nominal type {name}")),
        Type::Infer => Err("signature types cannot be inferred".into()),
    }
}
fn inhabited(ty: &ir::Ty) -> bool {
    let mut work = vec![ty];
    let mut seen = std::collections::HashSet::new();
    while let Some(ty) = work.pop() {
        match ty {
            ir::Ty::Array { element, len } => {
                if *len > 0 {
                    work.push(element);
                }
            }
            ir::Ty::Never => return false,
            ir::Ty::Tuple(fields) => work.extend(fields.iter()),
            ir::Ty::Struct(ty) if seen.insert(ty.def().id) => {
                work.extend(ty.def().fields.iter().map(|field| &field.ty))
            }
            _ => {}
        }
    }
    true
}
fn coerces(got: &ir::Ty, want: &ir::Ty) -> bool {
    if *got == ir::Ty::Never || got == want {
        return true;
    }
    match (got, want) {
        (
            ir::Ty::Array {
                element: a,
                len: na,
            },
            ir::Ty::Array {
                element: b,
                len: nb,
            },
        ) if na == nb => coerces(a, b),
        (ir::Ty::Tuple(a), ir::Ty::Tuple(b)) if a.len() == b.len() => {
            a.iter().zip(b.iter()).all(|(a, b)| coerces(a, b))
        }
        _ => false,
    }
}
impl<'a> Checker<'a> {
    fn new(signatures: &'a Signatures, result: ir::Ty, module: String) -> Self {
        Self {
            signatures,
            scopes: vec![HashMap::new()],
            pending_references: Vec::new(),
            locals: Vec::new(),
            exprs: Vec::new(),
            call_evidence: Vec::new(),
            declaration: None,
            loops: Vec::new(),
            next_loop: 0,
            reachable: true,
            result,
            module,
            count_queries: Vec::new(),
            field_indices: std::cell::RefCell::new(HashMap::new()),
            applications: std::cell::RefCell::new(Vec::new()),
            inference_evidence_cache: std::cell::RefCell::new(HashMap::new()),
            canonical: None,
        }
    }
    fn charge_node(&self, bytes: usize) -> Result<(), String> {
        if let Some(plan) = &self.signatures.plan {
            plan.budget.charge_nodes(1)?;
            plan.budget.charge_work(1)?;
            plan.state
                .lock()
                .map_err(|_| "generic plan context poisoned")?
                .retain(&plan.budget, bytes)?;
        }
        Ok(())
    }
    fn facts(&self) -> ir::FactsCtx<'_> {
        if let Some(facts) = &self.signatures.facts {
            return ir::FactsCtx::Validated(facts);
        }
        self.signatures
            .param_env
            .as_deref()
            .map_or(ir::FactsCtx::Concrete, ir::FactsCtx::Universal)
    }
    fn push_application(&self, path: &GenericPath) -> Result<(), String> {
        let plan = self
            .signatures
            .plan
            .as_ref()
            .ok_or("generic application requires declaration registry")?;
        let app = plan.application(path, &self.signatures.params, self.facts(), &plan.budget)?;
        let (_, source_args) = path.application()?;
        let regions = self.type_actual_regions(source_args, 0)?;
        self.infer_storage(1)?;
        self.applications.borrow_mut().push((app.0, app.1, regions));
        Ok(())
    }
    fn source_region(&self, name: &str) -> Result<ir::ActualRegion, String> {
        let plan = self
            .signatures
            .plan
            .as_ref()
            .ok_or("source region requires registry")?;
        let declaration = self
            .declaration
            .ok_or("source region requires original callable")?;
        let source = plan.callable_body(&plan.program, declaration)?;
        for (index, declared) in source.regions.params.iter().enumerate() {
            plan.budget
                .charge_work(1 + declared.len().min(name.len()))?;
            if declared == name {
                return Ok(ir::ActualRegion::Named(ir::SourceRegionId {
                    declaration,
                    index: index as u16,
                }));
            }
        }
        Err("undeclared lifetime in explicit type argument".into())
    }
    fn type_actual_regions(
        &self,
        args: &[Type],
        offset: usize,
    ) -> Result<Arc<[ir::TypeActualRegion]>, String> {
        let mut rows = Vec::new();
        for (argument, arg) in args.iter().enumerate() {
            self.infer_work(0)?;
            let mut ty = arg;
            let mut depth = 0u8;
            while let Type::Ref {
                region, referent, ..
            } = ty
            {
                self.infer_work(depth as usize)?;
                self.charge_node(4 * std::mem::size_of::<ir::TypeActualRegion>() + 64)?;
                rows.push(ir::TypeActualRegion {
                    argument: argument
                        .checked_add(offset)
                        .ok_or("type argument index overflow")?,
                    location: ir::CarrierLocation {
                        path: ir::CarrierPath(Arc::from([])),
                        channel: ir::CarrierChannel::Fixed(ir::ReferencePosition { depth }),
                    },
                    source: region
                        .as_deref()
                        .map_or(Ok(ir::ActualRegion::Elided), |name| {
                            self.source_region(name)
                        })?,
                });
                depth = depth
                    .checked_add(1)
                    .ok_or("type argument reference nesting overflow")?;
                ty = referent;
            }
        }
        Ok(rows.into())
    }
    fn checked_type_actual(&self, source: &Type) -> Result<ir::Ty, String> {
        let ty = checked_ty(source, self.signatures)?;
        if ty.contains_slice() {
            return Err("unsupported: slice generic arguments".into());
        }
        let plan = self
            .signatures
            .plan
            .as_ref()
            .ok_or("type actual requires registry")?;
        ir::validate_root_reference_actual(&ty, &plan.budget)?;
        // Validate source lifetime names now, before erasure and argument checking.
        self.type_actual_regions(std::slice::from_ref(source), 0)?;
        Ok(ty)
    }
    fn nominal(&self, name: &str, hint: Option<&ir::Ty>) -> Result<ir::Ty, String> {
        if name == "Self" {
            return self
                .signatures
                .self_ty
                .clone()
                .ok_or_else(|| "Self outside impl".into());
        }
        if let Some(ty) = hint {
            let matches = match ty {
                ir::Ty::Struct(t) => t.def().name == name,
                ir::Ty::Enum(t) => t.def().name == name,
                _ => false,
            };
            if matches {
                return Ok(ty.clone());
            }
        }
        if let Some(plan) = &self.signatures.plan {
            let decl = *plan
                .registry
                .nominals
                .get(name)
                .ok_or_else(|| format!("unknown nominal type {name}"))?;
            let args = if self
                .applications
                .borrow()
                .last()
                .is_some_and(|(owner, _, _)| owner == name)
            {
                self.applications.borrow_mut().pop().unwrap().1
            } else {
                Vec::new()
            };
            return plan.intern_nominal(decl, &args, self.facts(), &plan.budget);
        }
        self.signatures
            .structs
            .get(name)
            .cloned()
            .map(ir::Ty::Struct)
            .or_else(|| self.signatures.enums.get(name).cloned().map(ir::Ty::Enum))
            .ok_or_else(|| format!("unknown nominal type {name}"))
    }
    fn pattern_source<'p>(
        &self,
        pattern: &'p Pattern,
        expected: &ir::Ty,
    ) -> Result<&'p Pattern, String> {
        if let Pattern::Apply { path, pattern } = pattern {
            self.push_application(path)?;
            let result = (|| {
                let (name, _) = path.application()?;
                let _ = name;
                let plan = self.signatures.plan.as_ref().ok_or("missing plan")?;
                let (owner, args) =
                    plan.application(path, &self.signatures.params, self.facts(), &plan.budget)?;
                let decl = *plan
                    .registry
                    .nominals
                    .get(&owner)
                    .ok_or("generic pattern is not nominal")?;
                let actual = plan.intern_nominal(decl, &args, self.facts(), &plan.budget)?;
                if !coerces(expected, &actual) {
                    return Err("generic pattern application does not match scrutinee".into());
                }
                self.pattern_source(pattern, expected)
            })();
            self.applications.borrow_mut().pop();
            result
        } else {
            Ok(pattern)
        }
    }
    fn enter_loop(&mut self, accepts_value: bool) -> ir::LoopId {
        let id = ir::LoopId(self.next_loop);
        self.next_loop += 1;
        self.loops.push(LoopFrame {
            id,
            label: None,
            is_block: false,
            accepts_value,
            checking_condition: !accepts_value,
            break_ty: None,
            expected: None,
            can_exit: false,
        });
        id
    }
    fn loop_target(&self, operation: &str, label: Option<&str>) -> Result<usize, String> {
        let index = if let Some(label) = label {
            self.loops
                .iter()
                .rposition(|f| f.label.as_deref() == Some(label))
                .ok_or_else(|| format!("undeclared label '{label}"))?
        } else {
            self.loops
                .len()
                .checked_sub(1)
                .ok_or_else(|| format!("{operation} outside a loop"))?
        };
        let frame = &self.loops[index];
        if frame.is_block && (label.is_none() || operation == "continue") {
            return Err(format!(
                "{operation} cannot target a labeled block without a matching break label"
            ));
        }
        if self.loops[index..].iter().any(|f| f.checking_condition) {
            return Err(format!(
                "{operation} in a while condition is unsupported (E0590)"
            ));
        }
        Ok(index)
    }
    fn local(&mut self, name: &str, ty: ir::Ty, mutable: bool) -> Result<ir::LocalId, String> {
        admit_slice_value(&ty)?;
        let local = ir::LocalId(self.locals.len());
        self.charge_node(std::mem::size_of::<ir::LocalDecl>() + name.len())?;
        self.locals.push(ir::LocalDecl {
            name: name.to_owned(),
            ty,
            mutable,
        });
        self.scopes
            .last_mut()
            .unwrap()
            .insert(name.to_owned(), local);
        Ok(local)
    }
    fn bind(&mut self, name: &str, ty: ir::Ty) -> Result<ir::LocalId, String> {
        if self.scopes.last().unwrap().contains_key(name) {
            return Err(format!("duplicate parameter {name}"));
        }
        self.local(name, ty, false)
    }
    fn charge_pending_lookup(&self, n: &str) -> Result<(), String> {
        if let Some(plan) = &self.signatures.plan {
            plan.budget.charge_work(self.pending_references.len())?;
            let bytes = self.pending_references.iter().try_fold(0usize, |sum, p| {
                sum.checked_add(p.name.len())
                    .ok_or("pending reference lookup work overflow")
            })?;
            let work = bytes
                .checked_add(
                    self.pending_references
                        .len()
                        .checked_mul(n.len().saturating_add(1))
                        .ok_or("pending reference lookup work overflow")?,
                )
                .and_then(|v| v.checked_mul(self.scopes.len()))
                .ok_or("pending reference lookup work overflow")?;
            plan.budget.charge_work(work)?;
        }
        Ok(())
    }
    fn pending_reference(&self, n: &str) -> Result<Option<usize>, String> {
        self.charge_pending_lookup(n)?;
        for scope in (0..self.scopes.len()).rev() {
            if self.scopes[scope].contains_key(n) {
                return Ok(None);
            }
            if let Some(index) = self
                .pending_references
                .iter()
                .rposition(|p| p.scope == scope && p.name == n && p.local.is_none())
            {
                return Ok(Some(index));
            }
        }
        Ok(None)
    }
    fn lookup(&self, n: &str) -> Result<ir::LocalId, String> {
        self.charge_pending_lookup(n)?;
        for scope in (0..self.scopes.len()).rev() {
            if let Some(local) = self.scopes[scope].get(n) {
                return Ok(*local);
            }
            if self
                .pending_references
                .iter()
                .any(|p| p.scope == scope && p.name == n && p.local.is_none())
            {
                return Err(format!(
                    "deferred local {n} requires an independently typed reference assignment before use"
                ));
            }
        }
        Err(format!("unresolved local {n}"))
    }
    fn flow(&self, id: ir::ExprId) -> Flow {
        let e = &self.exprs[id.0];
        Flow {
            ty: e.ty.clone(),
            completes: e.completion == ir::Completion::Returns,
        }
    }
    fn require(&self, got: &Flow, want: &ir::Ty) -> Result<(), String> {
        if matches!((&got.ty,want),(ir::Ty::Ref{kind:ir::BorrowKind::Mut,referent:a},ir::Ty::Ref{kind:ir::BorrowKind::Shared,referent:b}) if a==b)
        {
            return Err("implicit mutable-to-shared reference coercion/reborrowing is unsupported; use an explicit &*r reborrow".into());
        }
        if !coerces(&got.ty, want) {
            Err(format!("expected {want:?}, got {:?}", got.ty))
        } else {
            Ok(())
        }
    }
    fn join(&self, a: &ir::Ty, b: &ir::Ty, context: &str) -> Result<ir::Ty, String> {
        if *a == ir::Ty::Never {
            return Ok(b.clone());
        }
        if *b == ir::Ty::Never || a == b {
            return Ok(a.clone());
        }
        if let (
            ir::Ty::Array {
                element: a,
                len: na,
            },
            ir::Ty::Array {
                element: b,
                len: nb,
            },
        ) = (a, b)
            && na == nb
        {
            return Ok(ir::Ty::Array {
                element: std::sync::Arc::new(self.join(a, b, context)?),
                len: *na,
            });
        }
        if let (ir::Ty::Tuple(a), ir::Ty::Tuple(b)) = (a, b)
            && a.len() == b.len()
        {
            return a
                .iter()
                .zip(b.iter())
                .map(|(a, b)| self.join(a, b, context))
                .collect::<Result<Vec<_>, _>>()
                .map(|fields| ir::Ty::Tuple(fields.into()));
        }
        Err(format!("{context} differ: {a:?} and {b:?}"))
    }
    fn accessible(&self, ty: &ir::Ty) -> Result<(), String> {
        fn visit(checker: &Checker<'_>, ty: &ir::Ty, depth: usize) -> Result<(), String> {
            if depth > 128 {
                return Err("type visibility nesting exceeds limit128".into());
            }
            if let Some(plan) = &checker.signatures.plan {
                plan.budget.charge_work(1)?;
            }
            match ty {
                ir::Ty::Ref { referent, .. } => visit(checker, referent, depth + 1)?,
                ir::Ty::Array { element, .. } => visit(checker, element, depth + 1)?,
                ir::Ty::Tuple(fields) => {
                    for field in fields.iter() {
                        visit(checker, field, depth + 1)?;
                    }
                }
                ir::Ty::Enum(ty) if !ty.def().visibility.accessible_from(&checker.module) => {
                    return Err(format!(
                        "private type {} is inaccessible from module {}",
                        ty.def().name,
                        checker.module
                    ));
                }
                ir::Ty::Struct(ty) if !ty.def().visibility.accessible_from(&checker.module) => {
                    return Err(format!(
                        "private type {} is inaccessible from module {}",
                        ty.def().name,
                        checker.module
                    ));
                }
                _ => {}
            }
            Ok(())
        }
        visit(self, ty, 0)
    }
    fn checked_place(&self, place: &Place) -> Result<(ir::Place, ir::Ty, bool), String> {
        if place.fields.len() > 128 {
            return Err("place projection nesting exceeds limit128".into());
        }
        let local = self.lookup(&place.local)?;
        let declaration = &self.locals[local.0];
        let mut ty = declaration.ty.clone();
        let mut writable = declaration.mutable;
        let mut derefs = 0;
        let mut projection = Vec::with_capacity(place.fields.len());
        for selector in &place.fields {
            match selector {
                FieldSelector::Deref => {
                    derefs += 1;
                    if derefs > 128 {
                        return Err("reference dereference depth exceeds limit128".into());
                    }
                    let ir::Ty::Ref { kind, referent } = &ty else {
                        return Err("dereference requires a reference".into());
                    };
                    writable = *kind == ir::BorrowKind::Mut;
                    ty = referent.as_ref().clone();
                    projection.push(ir::Projection::Deref);
                }
                FieldSelector::ConstantIndex(index) => {
                    ty = self.index_type(&ty, *index)?;
                    projection.push(ir::Projection::ConstantIndex(*index));
                }
                _ => {
                    if let ir::Ty::Ref { kind, referent } = &ty {
                        if derefs >= 128 {
                            return Err(
                                "multiple dereferences in one field place are unsupported".into()
                            );
                        }
                        if place.fields.len() >= 128 {
                            return Err(
                                "normalized place projection nesting exceeds limit128".into()
                            );
                        }
                        derefs += 1;
                        writable = *kind == ir::BorrowKind::Mut;
                        ty = referent.as_ref().clone();
                        projection.push(ir::Projection::Deref);
                    }
                    let (index, field) = self.field_type(&ty, selector)?;
                    ty = field;
                    projection.push(ir::Projection::Field(index));
                }
            }
        }
        self.accessible(&ty)?;
        Ok((ir::Place { local, projection }, ty, writable))
    }
    fn validate_value_carriers(&self, ty: &ir::Ty) -> Result<(), String> {
        let plan = self
            .signatures
            .plan
            .as_ref()
            .ok_or("carrier validation requires registry")?;
        // Shape alone grants no lifetime, target, Copy or executable-body proof.
        ir::carrier_locations(ty, &plan.budget).map(|_| ())
    }
    fn require_index(&self, flow: &Flow) -> Result<(), String> {
        self.require(flow,&ir::Ty::Int(ir::IntTy::Usize)).map_err(|_|"runtime index requires usize, without implicit i32 conversion; backward numeric inference is unsupported".into())
    }
    fn scalar_hint(&self, e: &Expr) -> Result<Option<ir::Ty>, String> {
        Ok(match e {
            Expr::TraitCall {
                owner,
                trait_path,
                name,
                ..
            } => {
                let plan = self
                    .signatures
                    .plan
                    .as_ref()
                    .ok_or("missing trait registry")?;
                let owner = checked_local_ty(owner, self.signatures)?;
                let tid = *plan
                    .registry
                    .traits
                    .get(trait_path)
                    .ok_or("unknown trait")?;
                Some(
                    plan.select_trait(&owner, tid, name, &[], self.facts(), false, &plan.budget)?
                        .ok_or("trait obligation not proven")?
                        .1
                        .result,
                )
            }
            Expr::Apply { .. } => {
                let evidence = self.inference_evidence(e, 0)?;
                Inference::empty(self)?.resolve(&evidence, false, 0)?
            }
            Expr::AssociatedCall {
                owner,
                name,
                type_args,
                ..
            } => {
                let plan = self
                    .signatures
                    .plan
                    .as_ref()
                    .ok_or("missing associated registry")?;
                self.infer_storage(type_args.len())?;
                let args = type_args
                    .iter()
                    .map(|t| self.checked_type_actual(t))
                    .collect::<Result<Vec<_>, _>>()?;
                let owner = checked_local_ty(owner, self.signatures)?;
                Some(
                    plan.select_associated(
                        &owner,
                        name,
                        &args,
                        self.facts(),
                        &self.module,
                        false,
                        &plan.budget,
                    )?
                    .1
                    .result,
                )
            }
            Expr::MethodCall { receiver, .. }
                if slice_len_candidate(e)
                    && self
                        .scalar_hint(receiver)?
                        .is_some_and(|ty| ty.shared_byte_slice()) =>
            {
                Some(ir::Ty::Int(ir::IntTy::Usize))
            }
            Expr::MethodCall {
                receiver,
                name,
                type_args,
                ..
            } => {
                if let Some(owner) = self.scalar_hint(receiver)? {
                    let plan = self
                        .signatures
                        .plan
                        .as_ref()
                        .ok_or("missing associated registry")?;
                    self.infer_storage(type_args.len())?;
                    let args = type_args
                        .iter()
                        .map(|t| self.checked_type_actual(t))
                        .collect::<Result<Vec<_>, _>>()?;
                    Some(
                        plan.select_method(
                            &owner,
                            name,
                            &args,
                            self.facts(),
                            &self.module,
                            &plan.budget,
                        )?
                        .signature
                        .result,
                    )
                } else {
                    None
                }
            }
            Expr::Struct { .. } | Expr::EnumVariant { .. } => {
                let evidence = self.inference_evidence(e, 0)?;
                Inference::empty(self)?.resolve(&evidence, false, 0)?
            }
            Expr::Cast { target, .. } => Some(checked_local_ty(target, self.signatures)?),
            Expr::Deref { value } => match self.scalar_hint(value)? {
                Some(ir::Ty::Ref { referent, .. }) => Some(referent.as_ref().clone()),
                _ => None,
            },
            Expr::Integer(literal) => literal.suffix.map(ir::Ty::Int),
            Expr::Float(literal) => literal.suffix.map(ir::Ty::Float),
            Expr::Bool(_) => Some(ir::Ty::Bool),
            Expr::Tuple(fields) if fields.is_empty() => Some(ir::Ty::Unit),
            Expr::Var(name) => self
                .lookup(name)
                .ok()
                .map(|id| self.locals[id.0].ty.clone()),
            Expr::Call { name, args } => {
                let plan = self
                    .signatures
                    .plan
                    .as_ref()
                    .ok_or("call hint requires declaration registry")?;
                let declaration = *plan
                    .registry
                    .functions
                    .get(name)
                    .ok_or("unresolved function in hint")?;
                if plan.param_env(declaration)?.arity() == 0 {
                    Some(
                        self.signatures
                            .functions
                            .get(name)
                            .ok_or("missing function signature")?
                            .result
                            .clone(),
                    )
                } else if let Some(type_args) =
                    self.infer_call_args(declaration, args, None, false)?
                {
                    Some(
                        plan.instantiate_signature(
                            &ir::CallTarget {
                                declaration,
                                type_args: type_args.into(),
                            },
                            self.facts(),
                            &plan.budget,
                        )?
                        .result,
                    )
                } else {
                    None
                }
            }
            Expr::Unary { value, .. } => self.scalar_hint(value)?,
            Expr::Binary {
                op:
                    BinaryOp::Eq
                    | BinaryOp::Ne
                    | BinaryOp::Lt
                    | BinaryOp::Le
                    | BinaryOp::Gt
                    | BinaryOp::Ge
                    | BinaryOp::And
                    | BinaryOp::Or,
                ..
            } => Some(ir::Ty::Bool),
            Expr::Binary {
                op: BinaryOp::Shl | BinaryOp::Shr,
                left,
                ..
            } => self.scalar_hint(left)?,
            Expr::Binary { left, right, .. } => match self.scalar_hint(left)? {
                Some(ty) => Some(ty),
                None => self.scalar_hint(right)?,
            },
            Expr::Field { base, field } => {
                if let Some(ty) = self.scalar_hint(base)? {
                    let receiver = if let ir::Ty::Ref { referent, .. } = &ty {
                        referent.as_ref()
                    } else {
                        &ty
                    };
                    Some(self.field_type(receiver, field)?.1)
                } else {
                    None
                }
            }
            Expr::Index { base, .. } | Expr::DynamicIndex { base, .. } => {
                match self.scalar_hint(base)? {
                    Some(ir::Ty::Array { element, .. }) => Some(element.as_ref().clone()),
                    Some(ty) if ty.shared_byte_slice() => Some(ir::Ty::Int(ir::IntTy::U8)),
                    _ => None,
                }
            }
            Expr::Block(block) if block.statements.is_empty() => match &block.tail {
                Some(tail) => self.scalar_hint(tail)?,
                None => Some(ir::Ty::Unit),
            },
            _ => None,
        })
    }

    fn deref_source(&self, mut id: ir::ExprId) -> bool {
        loop {
            match &self.exprs[id.0].kind {
                ir::ExprKind::Deref { .. } => return true,
                ir::ExprKind::Index { base, .. }
                | ir::ExprKind::Field { base, .. }
                | ir::ExprKind::DynamicIndex { base, .. } => id = *base,
                _ => return false,
            }
        }
    }
    fn dynamic_place(e: &Expr) -> bool {
        match e {
            Expr::DynamicIndex { .. } => true,
            Expr::Index { base, .. } | Expr::Field { base, .. } => Self::dynamic_place(base),
            _ => false,
        }
    }
    fn index_type(&self, ty: &ir::Ty, index: usize) -> Result<ir::Ty, String> {
        if matches!(ty, ir::Ty::Ref { .. }) {
            return Err("implicit reference autodereference is unsupported in B1; use an explicit (*r) place".into());
        }
        let ir::Ty::Array { element, len } = ty else {
            return Err("indexing requires an array".into());
        };
        if index >= *len {
            return Err(format!(
                "array index {index} out of bounds for length {len}"
            ));
        }
        self.accessible(element)?;
        Ok(element.as_ref().clone())
    }
    fn field_type(&self, ty: &ir::Ty, field: &FieldSelector) -> Result<(usize, ir::Ty), String> {
        if matches!(ty, ir::Ty::Ref { .. }) {
            return Err("implicit reference autodereference is unsupported in B1; use an explicit (*r) place".into());
        }
        let (index, projected) = match (ty, field) {
            (ir::Ty::Tuple(fields), FieldSelector::Tuple(index)) => fields
                .get(*index)
                .cloned()
                .map(|ty| (*index, ty))
                .ok_or_else(|| {
                    format!(
                        "tuple field {index} is out of bounds for {} fields",
                        fields.len()
                    )
                }),
            (ir::Ty::Struct(ty), FieldSelector::Named(name)) => {
                let index = {
                    let mut cache = self.field_indices.borrow_mut();
                    let indices = cache.entry(ty.def().id).or_insert_with(|| {
                        ty.def()
                            .fields
                            .iter()
                            .enumerate()
                            .map(|(index, field)| (field.name.clone(), index))
                            .collect()
                    });
                    *indices
                        .get(name)
                        .ok_or_else(|| format!("unknown field {name} in {}", ty.def().name))?
                };
                let field = &ty.def().fields[index];
                if !field.visibility.accessible_from(&self.module) {
                    return Err(format!("private field {name} in {}", ty.def().name));
                }
                Ok((index, field.ty.clone()))
            }
            _ => Err(format!("invalid field projection {field:?} on {ty:?}")),
        }?;
        self.accessible(&projected)?;
        Ok((index, projected))
    }
    /// Recover only concrete pattern shape, without a general inference engine.
    fn pattern_ty(&self, pattern: &Pattern, ty: &ir::Ty) -> Result<ir::Ty, String> {
        if matches!(ty, ir::Ty::Ref { .. })
            && !matches!(pattern, Pattern::Wildcard | Pattern::Binding { .. })
        {
            return Err(
                "reference patterns and implicit match ergonomics are unsupported in B1".into(),
            );
        }
        match pattern {
            Pattern::Path(name) | Pattern::Variant { name,.. } => {
                let expected = if let ir::Ty::Enum(enumeration)=ty {
                    if name.rsplit_once("::").is_some_and(|(owner,_)| if owner == "Self" { matches!(self.signatures.self_ty.as_ref(), Some(ir::Ty::Enum(own)) if own == enumeration) } else { owner==enumeration.def().name }) {ty.clone()}else{return Err("enum pattern owner mismatch".into());}
                }else{ir::Ty::Enum(self.enum_variant(name)?.0)};
                if !coerces(ty,&expected) { return Err(format!("enum pattern {name} does not match {ty:?}")); }
                Ok(expected)
            }
            Pattern::Apply {..} => self.pattern_ty(self.pattern_source(pattern,ty)?,ty),
            Pattern::Bool(_) | Pattern::Int { .. } => Err("refutable literal pattern in let binding".into()),
            Pattern::Wildcard | Pattern::Binding { .. } => Ok(ty.clone()),
            Pattern::Tuple(patterns) => match ty {
                ir::Ty::Unit if patterns.is_empty() => Ok(ir::Ty::Unit),
                ir::Ty::Never if patterns.is_empty() => Ok(ir::Ty::Unit),
                ir::Ty::Never => patterns.iter().map(|pattern| {
                    if matches!(pattern, Pattern::Binding { .. } | Pattern::Wildcard) {
                        Err("unannotated never tuple pattern inference is unsupported; add a whole-pattern type annotation".into())
                    } else { self.pattern_ty(pattern, &ir::Ty::Never) }
                }).collect::<Result<Vec<_>, String>>().map(|fields| ir::Ty::Tuple(fields.into())),
                ir::Ty::Tuple(fields) if fields.len() == patterns.len() => patterns.iter().zip(fields.iter()).map(|(pattern, ty)| self.pattern_ty(pattern, ty)).collect::<Result<Vec<_>, _>>().map(|fields| ir::Ty::Tuple(fields.into())),
                _ => Err(format!("tuple pattern with {} fields does not match {ty:?}", patterns.len())),
            },
            Pattern::Struct { name, .. } => {
                let expected = self.nominal(name,Some(ty))?;
                self.accessible(&expected)?;
                if !coerces(ty, &expected) { return Err(format!("struct pattern {name} does not match {ty:?}")); }
                Ok(expected)
            }
        }
    }
    fn pattern_locals(
        &self,
        pattern: &Pattern,
        ty: &ir::Ty,
        path: &mut Vec<ir::Projection>,
        names: &mut std::collections::HashSet<String>,
        pending: &mut Vec<PatternLocal>,
        work: &mut usize,
    ) -> Result<(), String> {
        if path.len() > ir::MAX_PATTERN_PROJECTION_DEPTH {
            return Err("pattern projection depth exceeds limit".into());
        }
        match pattern {
            Pattern::Apply { .. } => {
                return self.pattern_locals(
                    self.pattern_source(pattern, ty)?,
                    ty,
                    path,
                    names,
                    pending,
                    work,
                );
            }
            Pattern::Wildcard => {}
            Pattern::Path(_) | Pattern::Variant { .. } => {
                let (enumeration, variant, fields) = self.variant_pattern(pattern, ty)?;
                if enumeration.def().variants.len() != 1 {
                    return Err(
                        "refutable enum patterns in irrefutable positions are unsupported".into(),
                    );
                }
                for (index, pattern) in fields {
                    path.push(ir::Projection::Downcast(ir::VariantId {
                        owner: enumeration.def().id,
                        index: variant,
                    }));
                    path.push(ir::Projection::Field(index));
                    self.pattern_locals(
                        pattern,
                        &enumeration.def().variants[variant].fields[index].ty,
                        path,
                        names,
                        pending,
                        work,
                    )?;
                    path.pop();
                    path.pop();
                }
            }
            Pattern::Bool(_) | Pattern::Int { .. } => {
                return Err("nested refutable literal patterns are unsupported".into());
            }
            Pattern::Binding { name, mutable } => {
                if name == "_" {
                    return Err("underscore is a wildcard, not a binding name".into());
                }
                if !names.insert(name.clone()) {
                    return Err(format!("duplicate pattern binding {name}"));
                }
                self.accessible(ty)?;
                if pending.len() >= ir::MAX_PATTERN_BINDINGS {
                    return Err("pattern binding count exceeds limit".into());
                }
                *work = work
                    .checked_add(1 + path.len())
                    .ok_or("pattern projection work overflow")?;
                if *work > ir::MAX_PATTERN_PROJECTION_WORK {
                    return Err("pattern projection work exceeds limit".into());
                }
                pending.push(PatternLocal {
                    name: name.clone(),
                    ty: ty.clone(),
                    mutable: *mutable,
                    projection: path.clone(),
                });
            }
            Pattern::Tuple(patterns) => {
                if patterns.is_empty() && *ty == ir::Ty::Unit {
                    return Ok(());
                }
                let ir::Ty::Tuple(fields) = ty else {
                    return Err(format!("tuple pattern does not match {ty:?}"));
                };
                if fields.len() != patterns.len() {
                    return Err("tuple pattern arity mismatch".into());
                }
                for (index, (pattern, ty)) in patterns.iter().zip(fields.iter()).enumerate() {
                    path.push(ir::Projection::Field(index));
                    self.pattern_locals(pattern, ty, path, names, pending, work)?;
                    path.pop();
                }
            }
            Pattern::Struct { name, fields, rest } => {
                let expected = self.pattern_ty(pattern, ty)?;
                if &expected != ty {
                    return Err(
                        "nested never struct pattern requires a concrete annotated shape".into(),
                    );
                }
                let ir::Ty::Struct(structure) = ty else {
                    return Err(format!("expected struct pattern {name}"));
                };
                let mut selected = std::collections::HashSet::new();
                for (field, pattern) in fields {
                    let (index, field_ty) =
                        self.field_type(ty, &FieldSelector::Named(field.clone()))?;
                    if !selected.insert(index) {
                        return Err(format!("duplicate struct pattern field {field}"));
                    }
                    path.push(ir::Projection::Field(index));
                    self.pattern_locals(pattern, &field_ty, path, names, pending, work)?;
                    path.pop();
                }
                if !rest && selected.len() != structure.def().fields.len() {
                    return Err(format!(
                        "missing fields in struct pattern {name}; use .. to omit fields"
                    ));
                }
            }
        }
        Ok(())
    }
    fn expr_expected_when(
        &mut self,
        e: &Expr,
        reachable: bool,
        expected: Option<&ir::Ty>,
    ) -> Result<ir::ExprId, String> {
        let previous = self.reachable;
        self.reachable = reachable;
        let result = self.expr_expected(e, expected);
        self.reachable = previous;
        result
    }
    fn constructor_type(
        &self,
        name: &str,
        variant: Option<&str>,
        fields: &[(InferField<'_>, &Expr)],
        expected: Option<&ir::Ty>,
    ) -> Result<ir::Ty, String> {
        let plan = self
            .signatures
            .plan
            .as_ref()
            .ok_or("constructor requires planning context")?;
        if name == "Self" {
            return self.nominal(name, None);
        }
        let decl = *plan
            .registry
            .nominals
            .get(name)
            .ok_or("unknown nominal declaration")?;
        if plan.param_env(decl)?.arity() == 0
            || self
                .applications
                .borrow()
                .last()
                .is_some_and(|(owner, _, _)| owner == name)
        {
            return self.nominal(name, None);
        }
        self.infer_constructor(name,variant,fields,expected,true)?.ok_or_else(||"unsupported generic inference: insufficient evidence; explicit type arguments required".into())
    }
    fn enum_variant(&self, name: &str) -> Result<(ir::EnumTy, usize), String> {
        let (owner, variant) = name
            .rsplit_once("::")
            .ok_or_else(|| format!("unresolved enum variant {name}"))?;
        let ir::Ty::Enum(ty) = self.nominal(owner, None)? else {
            return Err(format!("unknown enum {owner}"));
        };
        self.accessible(&ir::Ty::Enum(ty.clone()))?;
        let index = ty
            .def()
            .variants
            .iter()
            .position(|definition| definition.name == variant)
            .ok_or_else(|| format!("unknown enum variant {name}"))?;
        Ok((ty, index))
    }
    fn variant_pattern<'p>(
        &self,
        pattern: &'p Pattern,
        expected: &ir::Ty,
    ) -> Result<ResolvedVariantPattern<'p>, String> {
        let (name, shape, fields, rest) = match pattern {
            Pattern::Path(name) => (name, ir::VariantShape::Unit, &[][..], false),
            Pattern::Variant {
                name,
                shape,
                fields,
                rest,
            } => (name, *shape, fields.as_slice(), *rest),
            _ => return Err("expected enum variant pattern".into()),
        };
        let (ty, variant) = if let ir::Ty::Enum(ty) = expected {
            let (owner, variant) = name.rsplit_once("::").ok_or("unresolved variant pattern")?;
            let owner_matches = if owner == "Self" {
                matches!(self.signatures.self_ty.as_ref(), Some(ir::Ty::Enum(own)) if own == ty)
            } else {
                owner == ty.def().name
            };
            if !owner_matches {
                return Err("variant pattern owner mismatch".into());
            }
            let index = ty
                .def()
                .variants
                .iter()
                .position(|v| v.name == variant)
                .ok_or("unknown enum variant")?;
            (ty.clone(), index)
        } else {
            self.enum_variant(name)?
        };
        if *expected != ir::Ty::Enum(ty.clone()) {
            return Err(format!(
                "variant pattern {name} does not match {expected:?}"
            ));
        }
        let definition = &ty.def().variants[variant];
        if shape != definition.shape
            && !(definition.shape == ir::VariantShape::Unit
                && shape == ir::VariantShape::Struct
                && fields.is_empty())
        {
            return Err(format!(
                "variant pattern {name} has wrong constructor shape"
            ));
        }
        let mut seen = std::collections::HashSet::new();
        let mut selected = Vec::new();
        for (selector, pattern) in fields {
            let index = match selector {
                FieldSelector::Tuple(i) if definition.shape == ir::VariantShape::Tuple => *i,
                FieldSelector::Named(n) => definition
                    .fields
                    .iter()
                    .position(|f| &f.name == n)
                    .ok_or_else(|| format!("unknown variant field {n}"))?,
                _ => return Err("invalid variant field selector".into()),
            };
            let field = definition
                .fields
                .get(index)
                .ok_or("variant pattern arity mismatch")?;
            if !seen.insert(index) {
                return Err("duplicate variant pattern field".into());
            }
            self.accessible(&field.ty)?;
            selected.push((index, pattern));
        }
        if !rest && seen.len() != definition.fields.len() {
            return Err("missing variant pattern fields".into());
        }
        Ok((ty, variant, selected))
    }
    fn compound_value(
        &mut self,
        op: BinaryOp,
        ty: &ir::Ty,
        value: &Expr,
    ) -> Result<(ir::ExprId, Flow), String> {
        let shift = matches!(op, BinaryOp::Shl | BinaryOp::Shr);
        let bitwise = matches!(op, BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor);
        if !matches!(
            op,
            BinaryOp::Add
                | BinaryOp::Sub
                | BinaryOp::Mul
                | BinaryOp::Div
                | BinaryOp::Rem
                | BinaryOp::BitAnd
                | BinaryOp::BitOr
                | BinaryOp::BitXor
                | BinaryOp::Shl
                | BinaryOp::Shr
        ) {
            return Err("invalid compound assignment operator".into());
        }
        if !matches!(ty, ir::Ty::Int(_))
            && !(matches!(ty, ir::Ty::Float(_)) && !shift && !bitwise)
            && !(bitwise && *ty == ir::Ty::Bool)
        {
            return Err(
                "compound assignment requires primitive integer or bitwise Bool destination".into(),
            );
        }
        let value = self.expr_expected(value, if shift { None } else { Some(ty) })?;
        let flow = self.flow(value);
        if shift {
            if flow.ty == ir::Ty::Never {
                return Err("compound shift on semantic Never is unsupported".into());
            }
            if !matches!(flow.ty, ir::Ty::Int(_)) {
                return Err("compound shift count must be an integer".into());
            }
            let ir::Ty::Int(ty) = ty else {
                return Err("compound shift destination must be integer".into());
            };
            if self.count_queries.len() >= COUNT_EVAL_WORK {
                return Err("compound shift query work limit exceeded".into());
            }
            self.count_queries.push((*ty, value));
        } else {
            if flow.ty == ir::Ty::Never {
                return Err("compound operator on semantic Never is unsupported".into());
            }
            self.require(&flow, ty)?;
        }
        Ok((value, flow))
    }
    fn deref_assignment(
        &mut self,
        carrier: &Expr,
        value: &Expr,
        statements: &mut Vec<ir::Stmt>,
    ) -> Result<(ir::Stmt, bool), String> {
        let carrier_ty = self
            .scalar_hint(carrier)?
            .ok_or("unresolved reference call assignment carrier")?;
        let ir::Ty::Ref { referent, .. } = &carrier_ty else {
            return Err("call assignment carrier must return a reference".into());
        };
        let referent = referent.as_ref().clone();
        let rhs = self.expr_expected(value, Some(&referent))?;
        let rhs_flow = self.flow(rhs);
        self.require(&rhs_flow, &referent)?;
        let saved = self.local(
            &format!("$rhs{}", self.locals.len()),
            referent.clone(),
            false,
        )?;
        self.charge_node(std::mem::size_of::<ir::Stmt>() * 3)?;
        statements.push(ir::Stmt::Let {
            local: saved,
            value: rhs,
        });
        let pointer =
            self.expr_expected_when(carrier, self.reachable && rhs_flow.completes, None)?;
        let pf = self.flow(pointer);
        self.require(&pf, &carrier_ty)?;
        let handle = self.local(&format!("$carrier{}", self.locals.len()), carrier_ty, false)?;
        statements.push(ir::Stmt::Let {
            local: handle,
            value: pointer,
        });
        let loaded = ir::ExprId(self.exprs.len());
        self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
        self.exprs.push(ir::TypedExpr {
            ty: referent,
            completion: ir::Completion::Returns,
            kind: ir::ExprKind::Local(saved),
        });
        Ok((
            ir::Stmt::AssignPlace {
                place: ir::Place {
                    local: handle,
                    projection: vec![ir::Projection::Deref],
                },
                value: loaded,
            },
            rhs_flow.completes && pf.completes,
        ))
    }
    fn block_when(&mut self, b: &Block, reachable: bool) -> Result<(ir::Block, Flow), String> {
        self.block_expected_when(b, reachable, None)
    }
    fn block_expected_when(
        &mut self,
        b: &Block,
        reachable: bool,
        expected: Option<&ir::Ty>,
    ) -> Result<(ir::Block, Flow), String> {
        let previous = self.reachable;
        self.reachable = reachable;
        let result = self.block_expected(b, expected);
        self.reachable = previous;
        result
    }
    fn block(&mut self, b: &Block) -> Result<(ir::Block, Flow), String> {
        self.block_expected(b, None)
    }
    fn block_expected(
        &mut self,
        b: &Block,
        expected: Option<&ir::Ty>,
    ) -> Result<(ir::Block, Flow), String> {
        self.block_with_expectation(b, expected.into())
    }
    fn block_with_expectation(
        &mut self,
        b: &Block,
        expectation: Expectation<'_>,
    ) -> Result<(ir::Block, Flow), String> {
        let entered_reachable = self.reachable;
        let pending_start = self.pending_references.len();
        self.scopes.push(HashMap::new());
        let mut live = true;
        let mut statements = Vec::with_capacity(b.statements.len());
        for s in &b.statements {
            self.reachable = entered_reachable && live;
            let (statement, completes) = match s {
                Stmt::Declare { name, ty, mutable } => {
                    if matches!(ty, Type::Infer) {
                        self.charge_node(
                            std::mem::size_of::<PendingReferenceDecl>() * 4 + name.len(),
                        )?;
                        self.scopes.last_mut().unwrap().remove(name);
                        self.pending_references.push(PendingReferenceDecl {
                            name: name.clone(),
                            mutable: *mutable,
                            scope: self.scopes.len() - 1,
                            position: statements.len(),
                            local: None,
                        });
                        continue;
                    }
                    if matches!(ty, Type::Never) {
                        return Err(
                            "deferred declarations require a concrete supported source type".into(),
                        );
                    }
                    let ty = checked_local_ty(ty, self.signatures)?;
                    self.accessible(&ty)?;
                    let local = self.local(name, ty, *mutable)?;
                    if let Some(cap) = self.canonical {
                        cap.record_binding(s, local)?;
                    }
                    (ir::Stmt::Declare { local }, true)
                }
                Stmt::Let {
                    name,
                    mutable,
                    ty,
                    value,
                } => {
                    let annotation = if *ty == Type::Infer {
                        None
                    } else {
                        Some(checked_local_ty(ty, self.signatures)?)
                    };
                    let value = self.expr_expected(value, annotation.as_ref())?;
                    let f = self.flow(value);
                    let t = if *ty == Type::Infer {
                        f.ty.clone()
                    } else {
                        let ty = checked_local_ty(ty, self.signatures)?;
                        self.accessible(&ty)?;
                        self.require(&f, &ty)?;
                        ty
                    };
                    let local = self.local(name, t, *mutable)?;
                    if let Some(cap) = self.canonical {
                        cap.record_binding(s, local)?;
                    }
                    (ir::Stmt::Let { local, value }, f.completes)
                }
                Stmt::LetPattern { pattern, ty, value } => {
                    // Resolve/check every initializer against the complete old scope.
                    let annotation = if *ty == Type::Infer {
                        None
                    } else {
                        Some(checked_local_ty(ty, self.signatures)?)
                    };
                    let direct_dynamic = matches!(value, Expr::DynamicIndex { .. });
                    let direct_wildcard = matches!(pattern, Pattern::Wildcard);
                    let value = self.expr_expected(value, annotation.as_ref())?;
                    if self.deref_source(value)
                        && matches!(pattern, Pattern::Path(_) | Pattern::Variant { .. })
                    {
                        return Err(
                            "enum downcasts after a dereference are unsupported in B1".into()
                        );
                    }
                    let flow = self.flow(value);
                    if direct_dynamic && !self.facts().must_copy(&flow.ty)? && !direct_wildcard {
                        return Err("non-Copy runtime indexed pattern extraction is unsupported; only direct let _ inspection is admitted".into());
                    }
                    let seed = if *ty == Type::Infer {
                        flow.ty.clone()
                    } else {
                        let ty = checked_local_ty(ty, self.signatures)?;
                        self.accessible(&ty)?;
                        self.require(&flow, &ty)?;
                        ty
                    };
                    let ty = self.pattern_ty(pattern, &seed)?;
                    self.require(&flow, &ty)?;
                    self.accessible(&ty)?;
                    let mut pending = Vec::new();
                    self.pattern_locals(
                        pattern,
                        &ty,
                        &mut Vec::new(),
                        &mut std::collections::HashSet::new(),
                        &mut pending,
                        &mut 0usize,
                    )?;
                    // No partially installed binder set can affect another binder.
                    let mut bindings = Vec::with_capacity(pending.len());
                    for binding in pending {
                        let local = self.local(&binding.name, binding.ty, binding.mutable)?;
                        bindings.push(ir::PatternBinding {
                            local,
                            projection: binding.projection,
                        });
                    }
                    (
                        ir::Stmt::LetPattern {
                            value,
                            ty,
                            bindings,
                        },
                        flow.completes,
                    )
                }
                Stmt::AssignDeref { carrier, value } => {
                    self.deref_assignment(carrier, value, &mut statements)?
                }
                Stmt::Assign { name, value } => {
                    if let Some(pending) = self.pending_reference(name)? {
                        // Only the already known dereference result supplies a scalar
                        // type here. Later uses never contribute inference evidence.
                        let scalar = if matches!(value, Expr::Deref { .. }) {
                            self.scalar_hint(value)?.filter(|ty| {
                                matches!(
                                    ty,
                                    ir::Ty::Int(_) | ir::Ty::Float(_) | ir::Ty::Bool | ir::Ty::Unit
                                )
                            })
                        } else {
                            None
                        };
                        let value = self.expr(value)?;
                        let flow = self.flow(value);
                        if !matches!(flow.ty, ir::Ty::Ref { .. })
                            && scalar.as_ref() != Some(&flow.ty)
                        {
                            return Err("untyped deferred declarations require an independently known reference or scalar dereference type; general backward inference is unsupported".into());
                        }
                        let local = ir::LocalId(self.locals.len());
                        self.charge_node(
                            std::mem::size_of::<ir::LocalDecl>() + name.len() * 2 + 128,
                        )?;
                        let declaration = &mut self.pending_references[pending];
                        declaration.local = Some(local);
                        self.locals.push(ir::LocalDecl {
                            name: name.clone(),
                            ty: flow.ty,
                            mutable: declaration.mutable,
                        });
                        self.scopes[declaration.scope].insert(name.clone(), local);
                        statements.push(ir::Stmt::Assign { local, value });
                        live &= flow.completes;
                        continue;
                    }
                    let local = self.lookup(name)?;
                    let declaration = &self.locals[local.0];
                    let t = declaration.ty.clone();
                    let value = self.expr_expected(value, Some(&t))?;
                    let f = self.flow(value);
                    self.require(&f, &t)?;
                    (ir::Stmt::Assign { local, value }, f.completes)
                }
                Stmt::AssignIndex {
                    array,
                    index,
                    value,
                } => {
                    let (array, ty, writable) = self.checked_place(array)?;
                    if array.projection.contains(&ir::Projection::Deref) {
                        return Err("runtime indexed access through references is unsupported in B1; snapshot a Copy referent first".into());
                    }
                    if !writable {
                        return Err("runtime index assignment requires a mutable place".into());
                    }
                    let ir::Ty::Array { element, .. } = &ty else {
                        return Err("runtime index assignment requires an array".into());
                    };
                    let value = self.expr_expected(value, Some(element))?;
                    let flow = self.flow(value);
                    self.require(&flow, element)?;
                    let index = self.expr_expected_when(
                        index,
                        self.reachable && flow.completes,
                        Some(&ir::Ty::Int(ir::IntTy::Usize)),
                    )?;
                    let index_flow = self.flow(index);
                    self.require_index(&index_flow)?;
                    (
                        ir::Stmt::AssignIndex {
                            array,
                            index,
                            value,
                        },
                        flow.completes && index_flow.completes,
                    )
                }
                Stmt::AssignOp { place, op, value } => {
                    let (place, ty, _) = self.checked_place(place)?;
                    let (value, flow) = self.compound_value(*op, &ty, value)?;
                    (
                        ir::Stmt::AssignOp {
                            place,
                            op: *op,
                            value,
                        },
                        flow.completes,
                    )
                }
                Stmt::AssignIndexOp {
                    array,
                    index,
                    op,
                    value,
                } => {
                    let (mut array, mut ty, mut writable) = self.checked_place(array)?;
                    if let ir::Ty::Ref { kind, referent } = &ty {
                        if array.projection.len() >= 128 {
                            return Err(
                                "normalized place projection nesting exceeds limit128".into()
                            );
                        }
                        writable = *kind == ir::BorrowKind::Mut;
                        ty = referent.as_ref().clone();
                        array.projection.push(ir::Projection::Deref);
                    }
                    if !writable {
                        return Err("compound index assignment requires a mutable place; shared references cannot write".into());
                    }
                    let ir::Ty::Array { element, .. } = ty else {
                        return Err("compound index assignment requires an array".into());
                    };
                    // Retain the place rather than reading/copying the referent.
                    // Lowering snapshots RHS, then index, then resolves this
                    // current carrier and performs the checked read/write.
                    let (value, flow) = self.compound_value(*op, &element, value)?;
                    let index = self.expr_expected_when(
                        index,
                        self.reachable && flow.completes,
                        Some(&ir::Ty::Int(ir::IntTy::Usize)),
                    )?;
                    let index_flow = self.flow(index);
                    self.require_index(&index_flow)?;
                    (
                        ir::Stmt::AssignIndexOp {
                            array,
                            index,
                            op: *op,
                            value,
                        },
                        flow.completes && index_flow.completes,
                    )
                }
                Stmt::AssignPlace { place, value } => {
                    let (place, ty, writable) = self.checked_place(place)?;
                    if !place.projection.is_empty()
                        && !writable
                        && !place.projection.contains(&ir::Projection::Deref)
                    {
                        return Err(
                            "assignment requires a mutable place; shared references cannot write"
                                .into(),
                        );
                    }
                    let value = self.expr_expected(value, Some(&ty))?;
                    let flow = self.flow(value);
                    self.require(&flow, &ty)?;
                    (ir::Stmt::AssignPlace { place, value }, flow.completes)
                }
                Stmt::Expr(e) | Stmt::ImplicitExpr(e) => {
                    let value = self.expr(e)?;
                    let f = self.flow(value);
                    if matches!(s, Stmt::ImplicitExpr(_)) {
                        self.require(&f, &ir::Ty::Unit)?;
                    }
                    (ir::Stmt::Expr(value), f.completes)
                }
                Stmt::Return(e) => {
                    let result_ty = self.result.clone();
                    let value = e
                        .as_ref()
                        .map(|e| self.expr_expected(e, Some(&result_ty)))
                        .transpose()?;
                    let f = value.map_or(Flow::value(ir::Ty::Unit), |e| self.flow(e));
                    self.require(&f, &self.result)?;
                    (ir::Stmt::Return(value), false)
                }
                Stmt::While {
                    label,
                    condition,
                    body,
                } => {
                    // Never skip a condition's target barrier to an outer loop.
                    let id = self.enter_loop(false);
                    self.loops.last_mut().unwrap().label = label.clone();
                    let condition = self.expr(condition)?;
                    let f = self.flow(condition);
                    self.require(&f, &ir::Ty::Bool)?;
                    self.loops.last_mut().unwrap().checking_condition = false;
                    let (body, bf) = self.block(body)?;
                    self.require(&bf, &ir::Ty::Unit)?;
                    self.loops.pop();
                    (
                        ir::Stmt::While {
                            id,
                            condition,
                            body,
                        },
                        f.completes,
                    )
                }
            };
            if let Some(cap) = self.canonical {
                cap.record_emission(s, statements.len())?;
            }
            statements.push(statement);
            live &= completes;
        }
        self.reachable = entered_reachable && live;
        let tail = b
            .tail
            .as_ref()
            .map(|e| self.expr_with_expectation(e, expectation))
            .transpose()?;
        self.reachable = entered_reachable;
        let mut flow = tail.map_or_else(
            || {
                if live {
                    Flow::value(ir::Ty::Unit)
                } else {
                    Flow::never()
                }
            },
            |e| self.flow(e),
        );
        flow.completes &= live;
        let count = self.pending_references.len() - pending_start;
        if let Some(plan) = &self.signatures.plan {
            plan.budget.charge_work(
                count
                    .checked_mul(statements.len().saturating_add(count))
                    .ok_or("pending declaration insertion work overflow")?,
            )?;
        }
        self.charge_node(count.saturating_mul(std::mem::size_of::<ir::Stmt>() * 4))?;
        for (offset, pending) in self.pending_references.drain(pending_start..).enumerate() {
            let local = pending
                .local
                .ok_or("untyped deferred reference declaration remains underconstrained")?;
            let position = pending
                .position
                .checked_add(offset)
                .ok_or("pending declaration position overflow")?;
            if let Some(cap) = self.canonical {
                cap.shift_root_emissions(b, position)?;
            }
            statements.insert(position, ir::Stmt::Declare { local });
        }
        self.scopes.pop();
        Ok((ir::Block { statements, tail }, flow))
    }
    fn expr(&mut self, e: &Expr) -> Result<ir::ExprId, String> {
        self.expr_expected(e, None)
    }
    fn expr_expected(&mut self, e: &Expr, expected: Option<&ir::Ty>) -> Result<ir::ExprId, String> {
        self.expr_with_expectation(e, expected.into())
    }
    fn expr_with_expectation(
        &mut self,
        e: &Expr,
        expectation: Expectation<'_>,
    ) -> Result<ir::ExprId, String> {
        if let Some(id) = self.builtin_slice_len(e)? {
            if let Some(cap) = self.canonical {
                cap.record_builtin_len(e, id, &self.exprs)?;
            }
            return Ok(id);
        }
        let result = self.expr_with_expectation_inner(e, expectation)?;
        let result = self.record_original_call(e, result)?;
        match expectation.hard() {
            Some(expected)
                if expected.shared_byte_slice()
                    && self.exprs[result.0].ty.shared_byte_array_len().is_some() =>
            {
                self.coerce_shared_byte_array_to_slice(result, expected)
            }
            _ => Ok(result),
        }
    }
    /// Expected-site unsizing of exactly shared `&[u8; N]` to shared `&[u8]`.
    /// The operand keeps its own identity (including any recorded original call);
    /// the wrapper is recorded separately with its exact operand for replay.
    fn coerce_shared_byte_array_to_slice(
        &mut self,
        value: ir::ExprId,
        expected: &ir::Ty,
    ) -> Result<ir::ExprId, String> {
        if !expected.shared_byte_slice() {
            return Err("array-to-slice coercion target must be &[u8]".into());
        }
        let source = &self.exprs[value.0];
        if source.ty.shared_byte_array_len().is_none() {
            return Err("array-to-slice coercion requires a shared &[u8; N] operand".into());
        }
        let completion = source.completion;
        let id = ir::ExprId(self.exprs.len());
        self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
        self.exprs.push(ir::TypedExpr {
            ty: ir::Ty::shared_byte_slice_ty(),
            completion,
            kind: ir::ExprKind::ArrayToSlice { reference: value },
        });
        if let Some(cap) = self.canonical {
            cap.record_unsize(id, value)?;
        }
        Ok(id)
    }
    /// Builtin `.len()` only on an exactly typed shared `&[u8]` receiver with
    /// zero arguments; every other `len` goes through ordinary method resolution.
    fn builtin_slice_len(&mut self, e: &Expr) -> Result<Option<ir::ExprId>, String> {
        let Expr::MethodCall { receiver, .. } = e else {
            return Ok(None);
        };
        if !slice_len_candidate(e)
            || !self
                .scalar_hint(receiver)?
                .is_some_and(|ty| ty.shared_byte_slice())
        {
            return Ok(None);
        }
        let slice = self.expr(receiver)?;
        let flow = self.flow(slice);
        if !flow.ty.shared_byte_slice() {
            return Err("builtin slice length receiver changed type".into());
        }
        let id = ir::ExprId(self.exprs.len());
        self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
        self.exprs.push(ir::TypedExpr {
            ty: ir::Ty::Int(ir::IntTy::Usize),
            completion: if flow.completes {
                ir::Completion::Returns
            } else {
                ir::Completion::Never
            },
            kind: ir::ExprKind::SliceLen { slice },
        });
        Ok(Some(id))
    }
    fn slice_index(&mut self, slice: ir::ExprId, index: &Expr) -> Result<ir::ExprId, String> {
        let source = self.flow(slice);
        let index = self.expr_expected_when(
            index,
            self.reachable && source.completes,
            Some(&ir::Ty::Int(ir::IntTy::Usize)),
        )?;
        self.slice_index_checked(slice, index)
    }
    fn slice_index_checked(
        &mut self,
        slice: ir::ExprId,
        index: ir::ExprId,
    ) -> Result<ir::ExprId, String> {
        let source = self.flow(slice);
        let index_flow = self.flow(index);
        self.require_index(&index_flow)?;
        let id = ir::ExprId(self.exprs.len());
        self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
        self.exprs.push(ir::TypedExpr {
            ty: ir::Ty::Int(ir::IntTy::U8),
            completion: if source.completes && index_flow.completes {
                ir::Completion::Returns
            } else {
                ir::Completion::Never
            },
            kind: ir::ExprKind::SliceIndex { slice, index },
        });
        Ok(id)
    }
    fn record_original_call(&mut self, e: &Expr, result: ir::ExprId) -> Result<ir::ExprId, String> {
        if matches!(
            e,
            Expr::Call { .. }
                | Expr::MethodCall { .. }
                | Expr::AssociatedCall { .. }
                | Expr::TraitCall { .. }
        ) && let Some(cap) = self.canonical
        {
            // Primitive rvalue receivers use the existing single-evaluation block
            // wrapper. The original method occurrence binds its tail call, not
            // the wrapper expression, while the wrapper remains in the IR.
            let call = match (e, &self.exprs[result.0].kind) {
                (Expr::MethodCall { .. }, ir::ExprKind::Block(block)) => block
                    .tail
                    .ok_or("canonical method wrapper lost its tail call")?,
                _ => result,
            };
            cap.record_call(e, call, &self.exprs[call.0].kind)?;
        }
        Ok(result)
    }
    fn expr_with_expectation_inner(
        &mut self,
        e: &Expr,
        expectation: Expectation<'_>,
    ) -> Result<ir::ExprId, String> {
        if let Expr::Apply { path, value } = e {
            let previous = self.applications.borrow().len();
            self.push_application(path)?;
            let result = self.expr_with_expectation(value, expectation);
            self.applications.borrow_mut().truncate(previous);
            return result;
        }
        if let Expr::Cast { value, target } = e {
            return self.cast_expression(value, target);
        }
        if matches!(
            e,
            Expr::TraitCall { .. } | Expr::MethodCall { .. } | Expr::AssociatedCall { .. }
        ) {
            return self.associated_expression(e);
        }
        self.expr_without_cast(e, expectation)
    }
    fn validate_borrow_referent(
        &self,
        _kind: ir::BorrowKind,
        referent: &ir::Ty,
    ) -> Result<(), String> {
        if referent.contains_slice() {
            return Err("unsupported: borrowing or reborrowing slice places".into());
        }
        self.validate_value_carriers(referent)
    }
    fn borrow_expression(
        &mut self,
        kind: ir::BorrowKind,
        place: ir::Place,
    ) -> Result<ir::ExprId, String> {
        let referent = ir::projected_ty(&self.locals[place.local.0].ty, &place.projection)?.clone();
        self.validate_borrow_referent(kind, &referent)?;
        let id = ir::ExprId(self.exprs.len());
        self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
        self.exprs.push(ir::TypedExpr {
            ty: ir::Ty::Ref {
                kind,
                referent: Arc::new(referent),
            },
            completion: ir::Completion::Returns,
            kind: ir::ExprKind::Borrow {
                kind,
                place,
                timing: ir::BorrowTiming::Immediate,
            },
        });
        Ok(id)
    }
    fn call_argument(
        &mut self,
        source: &Expr,
        expected: &ir::Ty,
        reachable: bool,
    ) -> Result<(ir::ExprId, Option<ir::ImplicitBorrowKind>), String> {
        if let (
            Expr::Var(name),
            ir::Ty::Ref {
                kind: ir::BorrowKind::Mut,
                referent,
            },
        ) = (source, expected)
        {
            let local = self.lookup(name)?;
            if let ir::Ty::Ref {
                kind: ir::BorrowKind::Mut,
                referent: actual,
            } = &self.locals[local.0].ty
            {
                let budget = &self
                    .signatures
                    .plan
                    .as_ref()
                    .ok_or("call requires plan")?
                    .budget;
                if metered_type_equal(actual, referent, budget, 0)? {
                    let carrier = self.expr(source)?;
                    let id = self.borrow_expression(
                        ir::BorrowKind::Mut,
                        ir::Place {
                            local,
                            projection: vec![ir::Projection::Deref],
                        },
                    )?;
                    return Ok((
                        id,
                        Some(ir::ImplicitBorrowKind::MutableArgumentReborrow { carrier }),
                    ));
                }
            }
        }
        self.expr_expected_when(source, reachable, Some(expected))
            .map(|id| (id, None))
    }
    fn finish_checked_call(
        &mut self,
        target: ir::Callee,
        name: &str,
        signature: ir::Signature,
        source_args: &[Expr],
        type_actual_regions: Arc<[ir::TypeActualRegion]>,
        receiver: Option<(ir::ExprId, Option<ir::ImplicitBorrowKind>)>,
    ) -> Result<ir::ExprId, String> {
        let plan = self
            .signatures
            .plan
            .as_ref()
            .ok_or("call evidence requires registry")?;
        // Resolve under the original caller facts before recording ordinary
        // actuals. Trait Self is not an ordinary source type argument, but a
        // selected recipe may have genuine ordinary arguments of its own.
        let (target, signature) = if let ir::Callee::TraitItem(call) = &target {
            let facts = self
                .facts()
                .validated()
                .ok_or("trait call evidence requires validated facts")?;
            facts
                .validate_signature(ir::SignatureSubject::Call(&target), &signature)
                .map_err(|e| e.into_legacy_diagnostic(&plan.budget))?;
            let resolved = facts
                .service()
                .resolve_trait_call(call, facts)
                .map_err(|e| e.into_legacy_diagnostic(&plan.budget))?;
            let signature = resolved.signature();
            plan.budget.charge_work(
                signature
                    .params
                    .len()
                    .checked_add(resolved.callee().type_arguments().len())
                    .and_then(|work| work.checked_add(1))
                    .ok_or("resolved call evidence metadata work overflow")?,
            )?;
            self.charge_node(
                signature
                    .params
                    .len()
                    .checked_mul(std::mem::size_of::<ir::Ty>())
                    .and_then(|bytes| bytes.checked_add(std::mem::size_of::<ir::Signature>()))
                    .and_then(|bytes| bytes.checked_add(std::mem::size_of::<ir::Callee>()))
                    .ok_or("resolved call evidence metadata allocation overflow")?,
            )?;
            facts
                .validate_signature(ir::SignatureSubject::Call(resolved.callee()), signature)
                .map_err(|e| e.into_legacy_diagnostic(&plan.budget))?;
            (resolved.callee().clone(), signature.clone())
        } else {
            (target, signature)
        };
        let actuals = match &target {
            ir::Callee::Direct(target) => target.type_args.as_ref(),
            ir::Callee::TraitItem(call) => call.method_args.as_ref(),
        };
        if actuals.iter().any(ir::Ty::contains_slice) {
            return Err("unsupported: slice generic arguments".into());
        }
        plan.budget.charge_work(type_actual_regions.len())?;
        self.charge_node(
            type_actual_regions
                .len()
                .checked_mul(4 * std::mem::size_of::<ir::TypeActualRegion>() + 64)
                .ok_or("call region evidence allocation overflow")?,
        )?;
        let mut region_rows = type_actual_regions.to_vec();
        for (argument, ty) in actuals.iter().enumerate() {
            for location in ir::carrier_locations(ty, &plan.budget)?.iter() {
                if !matches!(location.channel, ir::CarrierChannel::Fixed(_)) {
                    continue;
                }
                plan.budget.charge_work(region_rows.len())?;
                if !region_rows
                    .iter()
                    .any(|row| row.argument == argument && row.location == *location)
                {
                    self.charge_node(4 * std::mem::size_of::<ir::TypeActualRegion>() + 64)?;
                    region_rows.push(ir::TypeActualRegion {
                        argument,
                        location: location.clone(),
                        source: ir::ActualRegion::Elided,
                    });
                }
            }
        }
        let type_actual_regions = region_rows.into();
        let offset = usize::from(receiver.is_some());
        if signature.params.len() != source_args.len() + offset {
            return Err(format!("wrong arity for {name}"));
        }
        self.infer_storage(
            source_args
                .len()
                .checked_mul(4)
                .and_then(|n| n.checked_add(8))
                .ok_or("call allocation overflow")?,
        )?;
        let mut arguments = Vec::with_capacity(signature.params.len());
        let mut pending = Vec::with_capacity(signature.params.len());
        self.charge_node(
            signature
                .params
                .len()
                .checked_mul(4 * std::mem::size_of::<ir::CallArgumentTrace>() + 64)
                .ok_or("call trace allocation overflow")?,
        )?;
        // This signature is the original source-stage signature. Inference may
        // not introduce new structural reference actuals, so a later argument
        // cannot retroactively make an opaque parameter a firm Ref expectation.
        let identity = match self.canonical {
            Some(cap) => cap.identity_arguments(source_args)?,
            None => false,
        };
        if identity && (receiver.is_some() || source_args.len() != 1) {
            return Err("shared replay identity argument shape changed".into());
        }
        let expectations = signature
            .params
            .iter()
            .map(|ty| match ty {
                ir::Ty::Ref { kind, .. } if !identity => {
                    ir::CallExpectation::FirmReference { kind: *kind }
                }
                _ => ir::CallExpectation::NoFirmReference,
            })
            .collect::<Vec<_>>();
        let mut completes = true;
        if let Some((id, reason)) = receiver {
            self.require(&self.flow(id), &signature.params[0])?;
            completes = self.flow(id).completes;
            arguments.push(id);
            if let Some(reason) = reason {
                pending.push((id, 0, reason));
            }
        }
        for (i, (source, ty)) in source_args
            .iter()
            .zip(&signature.params[offset..])
            .enumerate()
        {
            let (id, reason) = if identity {
                (self.expr(source)?, None)
            } else {
                self.call_argument(source, ty, self.reachable && completes)?
            };
            let flow = self.flow(id);
            self.require(&flow, ty)?;
            completes &= flow.completes;
            arguments.push(id);
            if let Some(reason) = reason {
                pending.push((id, i + offset, reason));
            }
        }
        for ty in signature
            .params
            .iter()
            .chain(std::iter::once(&signature.result))
        {
            self.accessible(ty)?;
        }
        let id = ir::ExprId(self.exprs.len());
        for (borrow, argument, reason) in pending {
            let ir::ExprKind::Borrow { timing, .. } = &mut self.exprs[borrow.0].kind else {
                return Err("invalid implicit borrow argument".into());
            };
            *timing = ir::BorrowTiming::TwoPhase {
                activation: id,
                argument,
                reason,
            };
        }
        let traces = arguments
            .iter()
            .enumerate()
            .map(|(argument, value)| ir::CallArgumentTrace {
                argument,
                value: *value,
                expectation: expectations[argument],
            })
            .collect::<Vec<_>>();
        self.charge_node(4 * std::mem::size_of::<ir::CallEvidence>() + 64)?;
        self.call_evidence.push(ir::CallEvidence {
            call: id,
            arguments: traces.into(),
            type_actual_regions,
        });
        self.charge_node(std::mem::size_of::<ir::TypedExpr>() + name.len())?;
        self.exprs.push(ir::TypedExpr {
            ty: signature.result.clone(),
            completion: if completes {
                ir::Completion::Returns
            } else {
                ir::Completion::Never
            },
            kind: ir::ExprKind::Call {
                target,
                name: name.to_owned(),
                args: arguments,
                signature,
            },
        });
        Ok(id)
    }
    fn method_expression(
        &mut self,
        receiver: &Expr,
        name: &str,
        type_args: &[Type],
        args: &[Expr],
    ) -> Result<ir::ExprId, String> {
        let plan = self
            .signatures
            .plan
            .clone()
            .ok_or("method call requires registry")?;
        let ty = self
            .scalar_hint(receiver)?
            .ok_or("unsupported unresolved receiver type")?;
        self.infer_storage(type_args.len())?;
        let actuals = type_args
            .iter()
            .map(|t| self.checked_type_actual(t))
            .collect::<Result<Vec<_>, _>>()?;
        let selected = plan.select_method(
            &ty,
            name,
            &actuals,
            self.facts(),
            &self.module,
            &plan.budget,
        )?;
        let region_offset = match &selected.target {
            ir::Callee::Direct(target) => target.type_args.len().saturating_sub(type_args.len()),
            ir::Callee::TraitItem(_) => 0,
        };
        // Only primitive value temporaries have the approved lexical autoref extension.
        if selected.depth == 0
            && matches!(selected.adjustment, ReceiverAdjustment::SharedAutoref)
            && matches!(
                ty,
                ir::Ty::Int(_) | ir::Ty::Float(_) | ir::Ty::Bool | ir::Ty::Unit
            )
            && static_source_place(receiver).is_err()
        {
            let value = self.expr(receiver)?;
            self.charge_node(std::mem::size_of::<ir::LocalDecl>() + 64)?;
            let local = ir::LocalId(self.locals.len());
            self.locals.push(ir::LocalDecl {
                name: "$primitive_receiver".into(),
                ty,
                mutable: false,
            });
            let borrowed = self.borrow_expression(
                ir::BorrowKind::Shared,
                ir::Place {
                    local,
                    projection: Vec::new(),
                },
            )?;
            let call = self.finish_checked_call(
                selected.target,
                name,
                selected.signature,
                args,
                self.type_actual_regions(type_args, region_offset)?,
                Some((borrowed, None)),
            )?;
            let result = self.exprs[call.0].ty.clone();
            let completion = self.exprs[call.0].completion;
            self.charge_node(std::mem::size_of::<ir::TypedExpr>() + 128)?;
            let id = ir::ExprId(self.exprs.len());
            self.exprs.push(ir::TypedExpr {
                ty: result,
                completion,
                kind: ir::ExprKind::Block(ir::Block {
                    statements: vec![ir::Stmt::Let { local, value }],
                    tail: Some(call),
                }),
            });
            return Ok(id);
        }
        let adjusted = match selected.mode {
            ReceiverKind::None => return Err("method has no receiver".into()),
            ReceiverKind::Value { .. } => {
                let mut id = self.expr(receiver)?;
                for _ in 0..selected.depth {
                    let ir::Ty::Ref { referent, .. } = &self.exprs[id.0].ty else {
                        return Err("invalid receiver dereference".into());
                    };
                    let ty = referent.as_ref().clone();
                    let completion = self.exprs[id.0].completion;
                    self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
                    let next = ir::ExprId(self.exprs.len());
                    self.exprs.push(ir::TypedExpr {
                        kind: ir::ExprKind::Deref { value: id },
                        ty,
                        completion,
                    });
                    id = next;
                }
                (id, None)
            }
            ReceiverKind::Borrowed { kind } => {
                if matches!(selected.adjustment, ReceiverAdjustment::Identity)
                    && (kind == ir::BorrowKind::Shared
                        || (selected.depth == 0 && genuine_call(receiver)))
                {
                    let mut id = self.expr(receiver)?;
                    for _ in 0..selected.depth {
                        let ir::Ty::Ref { referent, .. } = &self.exprs[id.0].ty else {
                            return Err("invalid receiver dereference".into());
                        };
                        let ty = referent.as_ref().clone();
                        let completion = self.exprs[id.0].completion;
                        self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
                        let next = ir::ExprId(self.exprs.len());
                        self.exprs.push(ir::TypedExpr {
                            kind: ir::ExprKind::Deref { value: id },
                            ty,
                            completion,
                        });
                        id = next;
                    }
                    (id, None)
                } else {
                    if genuine_call(receiver) {
                        return Err(
                            "unsupported: autoref of a reference-return call temporary".into()
                        );
                    }
                    let source = static_source_place(receiver)?;
                    let (mut place, _, _) = self.checked_place(&source)?;
                    let derefs = selected.depth
                        + usize::from(matches!(selected.adjustment, ReceiverAdjustment::Identity));
                    self.charge_node(derefs * std::mem::size_of::<ir::Projection>() + 64)?;
                    place
                        .projection
                        .extend(std::iter::repeat_n(ir::Projection::Deref, derefs));
                    let id = self.borrow_expression(kind, place)?;
                    (
                        id,
                        if kind == ir::BorrowKind::Mut {
                            Some(ir::ImplicitBorrowKind::MutableReceiver)
                        } else {
                            None
                        },
                    )
                }
            }
        };
        self.finish_checked_call(
            selected.target,
            name,
            selected.signature,
            args,
            self.type_actual_regions(type_args, region_offset)?,
            Some(adjusted),
        )
    }
    fn associated_expression(&mut self, expression: &Expr) -> Result<ir::ExprId, String> {
        if let Expr::MethodCall {
            receiver,
            name,
            type_args,
            args,
        } = expression
        {
            return self.method_expression(receiver, name, type_args, args);
        }
        let plan = self
            .signatures
            .plan
            .clone()
            .ok_or("associated call requires registry")?;
        let empty = Vec::new();
        let (owner, name, type_args, args, receiver) = match expression {
            Expr::MethodCall {
                receiver,
                name,
                type_args,
                args,
            } => {
                let ty = self
                    .scalar_hint(receiver)?
                    .ok_or("unsupported unresolved receiver type; use a typed local")?;
                let owner = match ty {
                    ir::Ty::Ref { referent, .. } => referent.as_ref().clone(),
                    ty => ty,
                };
                (owner, name, type_args, args, Some(receiver.as_ref()))
            }
            Expr::AssociatedCall {
                owner,
                name,
                type_args,
                args,
            } => (
                checked_local_ty(owner, self.signatures)?,
                name,
                type_args,
                args,
                None,
            ),
            Expr::TraitCall {
                owner, name, args, ..
            } => (
                checked_local_ty(owner, self.signatures)?,
                name,
                &empty,
                args,
                None,
            ),
            _ => return Err("not an associated call".into()),
        };
        self.infer_storage(type_args.len() + args.len() + 1)?;
        let actuals = type_args
            .iter()
            .map(|t| self.checked_type_actual(t))
            .collect::<Result<Vec<_>, _>>()?;
        let (target, signature, mode) = if let Expr::TraitCall { trait_path, .. } = expression {
            let tid = *plan
                .registry
                .traits
                .get(trait_path)
                .ok_or("unresolved UFCS trait")?;
            plan.select_trait(
                &owner,
                tid,
                name,
                &actuals,
                self.facts(),
                false,
                &plan.budget,
            )?
            .ok_or("trait obligation not proven")?
        } else {
            plan.select_associated(
                &owner,
                name,
                &actuals,
                self.facts(),
                &self.module,
                receiver.is_some(),
                &plan.budget,
            )?
        };
        let adjusted = if let Some(receiver) = receiver {
            match mode {
                ReceiverKind::None => return Err("associated function has no receiver".into()),
                ReceiverKind::Value { .. } => Some((self.expr(receiver)?, None)),
                ReceiverKind::Borrowed { kind } => {
                    if genuine_call(receiver) {
                        let id = self.expr(receiver)?;
                        Some((id, None))
                    } else {
                        let source = static_source_place(receiver)?;
                        let (mut place, ty, _) = self.checked_place(&source)?;
                        if matches!(ty, ir::Ty::Ref { .. }) {
                            if place.projection.contains(&ir::Projection::Deref) {
                                return Err(
                                    "multiple receiver autodereferences are unsupported".into()
                                );
                            }
                            place.projection.push(ir::Projection::Deref);
                        }
                        let id = self.borrow_expression(kind, place)?;
                        Some((
                            id,
                            if kind == ir::BorrowKind::Mut {
                                Some(ir::ImplicitBorrowKind::MutableReceiver)
                            } else {
                                None
                            },
                        ))
                    }
                }
            }
        } else {
            None
        };
        let region_offset = match &target {
            ir::Callee::Direct(target) => target.type_args.len().saturating_sub(type_args.len()),
            ir::Callee::TraitItem(_) => 0,
        };
        self.finish_checked_call(
            target,
            name,
            signature,
            args,
            self.type_actual_regions(type_args, region_offset)?,
            adjusted,
        )
    }
    fn cast_expression(&mut self, value: &Expr, target: &Type) -> Result<ir::ExprId, String> {
        let target = checked_local_ty(target, self.signatures)?;
        let numeric = ir::NumTy::of(&target).ok_or("unsupported cast target")?;
        let value = match numeric {
            ir::NumTy::Int(to) => {
                self.expr_with_expectation(value, Expectation::CastableToInt(to))?
            }
            ir::NumTy::Float(_) => self.expr(value)?,
        };
        let source = self.flow(value);
        let from = ir::NumTy::of(&source.ty).ok_or("unsupported cast source")?;
        let kind = match (from, numeric) {
            (ir::NumTy::Int(_), ir::NumTy::Int(to)) => ir::ExprKind::IntCast { value, to },
            _ => ir::ExprKind::FloatCast { value, to: numeric },
        };
        let id = ir::ExprId(self.exprs.len());
        self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
        self.exprs.push(ir::TypedExpr {
            kind,
            ty: target,
            completion: if source.completes {
                ir::Completion::Returns
            } else {
                ir::Completion::Never
            },
        });
        Ok(id)
    }
    fn expr_without_cast(
        &mut self,
        e: &Expr,
        expectation: Expectation<'_>,
    ) -> Result<ir::ExprId, String> {
        // A castable hint is not a hard result constraint or reference coercion site.
        let expected = expectation.hard();
        if let Some(ir::Ty::Ref { kind, referent }) = expected {
            if let Expr::Var(name) = e {
                let local = self.lookup(name)?;
                if let ir::Ty::Ref {
                    kind: ir::BorrowKind::Mut,
                    referent: actual,
                } = &self.locals[local.0].ty
                    && actual == referent
                {
                    // Reborrow the original carrier before any value read can move it.
                    return self.expr_expected(
                        &Expr::Borrow {
                            kind: *kind,
                            place: Place {
                                local: name.clone(),
                                fields: vec![FieldSelector::Deref],
                            },
                        },
                        expected,
                    );
                }
            }
            if matches!(e, Expr::Loop { .. }) {
                return Err("reference expected-context propagation through loop results is unsupported; use an explicit borrow/reborrow".into());
            }
        }
        let (kind, flow) = match e {
            Expr::Cast { .. } => return Err("cast bypassed expression dispatcher".into()),
            Expr::Borrow { kind, place } => {
                let (place, referent, _writable) = self.checked_place(place).map_err(|error|format!("B1 borrow requires an existing local-root static place; borrowed rvalues/non-local targets are unsupported: {error}"))?;
                self.validate_borrow_referent(*kind, &referent)?;
                (
                    ir::ExprKind::Borrow {
                        kind: *kind,
                        place,
                        timing: ir::BorrowTiming::Immediate,
                    },
                    Flow::value(ir::Ty::Ref {
                        kind: *kind,
                        referent: std::sync::Arc::new(referent),
                    }),
                )
            }
            Expr::Deref { value } => {
                let mut root = value.as_ref();
                let mut depth = 1;
                while let Expr::Deref { value } = root {
                    if depth >= 128 {
                        return Err("reference dereference depth exceeds limit128".into());
                    }
                    if let Some(plan) = &self.signatures.plan {
                        plan.budget.charge_work(1)?;
                    }
                    depth += 1;
                    root = value;
                }
                if !matches!(root, Expr::Var(_)) && !genuine_call(root) {
                    // Static component carriers use the same checked projection
                    // authority as field receivers, without an invented temporary.
                    let source = static_source_place(e)?;
                    self.checked_place(&source)?;
                }
                let value = self.expr(value)?;
                let flow = self.flow(value);
                let ir::Ty::Ref { referent, .. } = &flow.ty else {
                    return Err("dereference requires a reference".into());
                };
                let ty = referent.as_ref().clone();
                self.accessible(&ty)?;
                (
                    ir::ExprKind::Deref { value },
                    Flow {
                        ty,
                        completes: flow.completes,
                    },
                )
            }
            Expr::Array(elements) => {
                if elements.len() > ir::MAX_ARRAY_LEN {
                    return Err("array literal exceeds supported length".into());
                }
                let hint = match expected {
                    Some(ir::Ty::Array { element, .. }) => Some(element.as_ref()),
                    _ => None,
                };
                if elements.is_empty() && hint.is_none() {
                    return Err("empty array literal inference is unsupported; provide a concrete expected array type".into());
                }
                let mut element = hint.cloned().unwrap_or(ir::Ty::Never);
                let mut values = Vec::new();
                let mut completes = true;
                for expression in elements {
                    let value =
                        self.expr_expected_when(expression, self.reachable && completes, hint)?;
                    let flow = self.flow(value);
                    completes &= flow.completes;
                    if let Some(expected) = hint {
                        self.require(&flow, expected)?;
                    } else {
                        element = self.join(&element, &flow.ty, "array elements")?;
                    }
                    values.push(value);
                }
                (
                    ir::ExprKind::Array(values),
                    Flow {
                        ty: ir::Ty::Array {
                            element: std::sync::Arc::new(element),
                            len: elements.len(),
                        },
                        completes,
                    },
                )
            }
            Expr::Repeat { value, len } => {
                if *len > ir::MAX_ARRAY_LEN {
                    return Err("array repeat exceeds supported length".into());
                }
                let hint = match expected {
                    Some(ir::Ty::Array { element, .. }) => Some(element.as_ref()),
                    _ => None,
                };
                let value = self.expr_expected(value, hint)?;
                let flow = self.flow(value);
                if let Some(expected) = hint {
                    self.require(&flow, expected)?;
                }
                let element = hint.cloned().unwrap_or(flow.ty);
                if *len > 1 && !self.facts().must_copy(&element)? {
                    return Err("array repeat of more than one element requires Copy".into());
                }
                (
                    ir::ExprKind::Repeat { value, len: *len },
                    Flow {
                        ty: ir::Ty::Array {
                            element: std::sync::Arc::new(element),
                            len: *len,
                        },
                        completes: flow.completes,
                    },
                )
            }
            Expr::DynamicIndex { base, index } => {
                if Self::dynamic_place(base) {
                    return Err("nested/chained runtime indexed place bases are unsupported; snapshot the row first".into());
                }
                let base = self.expr(base)?;
                if self.flow(base).ty.shared_byte_slice() {
                    return self.slice_index(base, index);
                }
                if self.deref_source(base) {
                    return Err("runtime indexed access through references is unsupported in B1; snapshot a Copy referent first".into());
                }
                let source = self.flow(base);
                let ir::Ty::Array { element, .. } = &source.ty else {
                    return Err("runtime indexing requires an array".into());
                };
                let ty = element.as_ref().clone();
                self.accessible(&ty)?;
                let index = self.expr_expected_when(
                    index,
                    self.reachable && source.completes,
                    Some(&ir::Ty::Int(ir::IntTy::Usize)),
                )?;
                let index_flow = self.flow(index);
                self.require_index(&index_flow)?;
                (
                    ir::ExprKind::DynamicIndex { base, index },
                    Flow {
                        ty,
                        completes: source.completes && index_flow.completes,
                    },
                )
            }
            Expr::Index { base, index } => {
                if Self::dynamic_place(base) {
                    return Err("nested/chained runtime indexed place bases are unsupported; snapshot the row first".into());
                }
                let base = self.expr(base)?;
                let flow = self.flow(base);
                if flow.ty.shared_byte_slice() {
                    // Constant slice indexes use the same runtime-checked read.
                    let constant = ir::ExprId(self.exprs.len());
                    self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
                    self.exprs.push(ir::TypedExpr {
                        ty: ir::Ty::Int(ir::IntTy::Usize),
                        completion: ir::Completion::Returns,
                        kind: ir::ExprKind::Int(ir::IntValue::usize(*index as u64)),
                    });
                    return self.slice_index_checked(base, constant);
                }
                let ty = self.index_type(&flow.ty, *index)?;
                (
                    ir::ExprKind::Index {
                        base,
                        index: *index,
                    },
                    Flow {
                        ty,
                        completes: flow.completes,
                    },
                )
            }
            Expr::EnumVariant {
                name,
                shape,
                fields,
            } => {
                let (owner, variant_name) = name
                    .rsplit_once("::")
                    .ok_or("invalid enum constructor path")?;
                self.infer_storage(fields.len())?;
                if fields.iter().any(|(selector, _)| {
                    matches!(
                        selector,
                        FieldSelector::Deref | FieldSelector::ConstantIndex(_)
                    )
                }) {
                    return Err("unsupported constructor inference selector".into());
                }
                let evidence_fields = fields
                    .iter()
                    .map(|(selector, value)| {
                        (
                            match selector {
                                FieldSelector::Named(name) => InferField::Named(name),
                                FieldSelector::Tuple(index) => InferField::Tuple(*index),
                                FieldSelector::Deref | FieldSelector::ConstantIndex(_) => {
                                    unreachable!("constructor fields are named or tuple selectors")
                                }
                            },
                            value,
                        )
                    })
                    .collect::<Vec<_>>();
                let ir::Ty::Enum(ty) =
                    self.constructor_type(owner, Some(variant_name), &evidence_fields, expected)?
                else {
                    return Err("expected enum constructor".into());
                };
                let variant = ty
                    .def()
                    .variants
                    .iter()
                    .position(|v| v.name == variant_name)
                    .ok_or("unknown enum variant")?;
                self.accessible(&ir::Ty::Enum(ty.clone()))?;
                let definition = &ty.def().variants[variant];
                if *shape != definition.shape
                    && !(definition.shape == ir::VariantShape::Unit
                        && *shape == ir::VariantShape::Struct
                        && fields.is_empty())
                {
                    if *shape == ir::VariantShape::Unit
                        && definition.shape == ir::VariantShape::Tuple
                    {
                        return Err("first-class tuple variant constructors are unsupported".into());
                    }
                    return Err(format!("variant constructor {name} has wrong shape"));
                }
                let mut seen = std::collections::HashSet::new();
                let mut values = Vec::new();
                let mut completes = true;
                for (selector, value) in fields {
                    let index = match selector {
                        FieldSelector::Tuple(i) if definition.shape == ir::VariantShape::Tuple => {
                            *i
                        }
                        FieldSelector::Named(n) => definition
                            .fields
                            .iter()
                            .position(|f| &f.name == n)
                            .ok_or_else(|| format!("unknown variant field {n}"))?,
                        _ => return Err("invalid variant constructor selector".into()),
                    };
                    let field = definition
                        .fields
                        .get(index)
                        .ok_or("variant constructor arity mismatch")?;
                    if !seen.insert(index) {
                        return Err("duplicate variant constructor field".into());
                    }
                    self.accessible(&field.ty)?;
                    let value = self.expr_expected_when(
                        value,
                        self.reachable && completes,
                        Some(&field.ty),
                    )?;
                    let flow = self.flow(value);
                    self.require(&flow, &field.ty)?;
                    completes &= flow.completes;
                    values.push((index, value));
                }
                if seen.len() != definition.fields.len() {
                    return Err("missing variant constructor fields".into());
                }
                (
                    ir::ExprKind::EnumVariant {
                        ty: ty.clone(),
                        variant,
                        fields: values,
                    },
                    Flow {
                        ty: ir::Ty::Enum(ty),
                        completes,
                    },
                )
            }
            Expr::Match { scrutinee, arms } => {
                if arms.len() > 4096 {
                    return Err("match arm count exceeds limit 4096".into());
                }
                let scrutinee = self.expr(scrutinee)?;
                if self.deref_source(scrutinee)
                    && arms.iter().any(|arm| {
                        matches!(arm.pattern, Pattern::Path(_) | Pattern::Variant { .. })
                    })
                {
                    return Err("enum downcasts after a dereference are unsupported in B1".into());
                }
                let source = self.flow(scrutinee);
                let mut domain = source.ty.clone();
                // A divergent operand can coerce to the concrete type named by
                // a pattern without inventing a general inference variable.
                if domain == ir::Ty::Never {
                    for arm in arms {
                        let hint = match &arm.pattern {
                            Pattern::Path(name) | Pattern::Variant { name, .. } => {
                                Some(ir::Ty::Enum(self.enum_variant(name)?.0))
                            }
                            Pattern::Bool(_) => Some(ir::Ty::Bool),
                            Pattern::Int { literal, .. } => {
                                Some(ir::Ty::Int(literal.suffix.unwrap_or(ir::IntTy::I32)))
                            }
                            Pattern::Tuple(fields) if fields.is_empty() => Some(ir::Ty::Unit),
                            _ => None,
                        };
                        if let Some(hint) = hint {
                            domain = hint;
                            break;
                        }
                    }
                }
                self.accessible(&domain)?;
                if matches!(domain, ir::Ty::Ref { .. })
                    && arms.iter().any(|arm| {
                        !matches!(arm.pattern, Pattern::Wildcard | Pattern::Binding { .. })
                    })
                {
                    return Err(
                        "reference patterns and implicit match ergonomics are unsupported in B1"
                            .into(),
                    );
                }
                let mut catchall = false;
                let mut covered = std::collections::HashSet::<u128>::new();
                let mut checked = Vec::with_capacity(arms.len());
                let mut result = ir::Ty::Never;
                let mut completes = false;
                for arm in arms {
                    self.scopes.push(HashMap::new());
                    let arm_pattern = self.pattern_source(&arm.pattern, &domain)?;
                    let (pattern, key) = match arm_pattern {
                        Pattern::Wildcard => (ir::MatchPattern::Wildcard, None),
                        Pattern::Binding { name, mutable } => {
                            let local = self.local(name, domain.clone(), *mutable)?;
                            (ir::MatchPattern::Binding(local), None)
                        }
                        Pattern::Bool(value) => {
                            if domain != ir::Ty::Bool {
                                return Err(format!(
                                    "bool match pattern does not match {domain:?}"
                                ));
                            }
                            (ir::MatchPattern::Bool(*value), Some(u128::from(*value)))
                        }
                        Pattern::Int { literal, negative } => {
                            if !matches!(domain, ir::Ty::Int(_)) {
                                return Err(format!(
                                    "integer match pattern does not match {domain:?}"
                                ));
                            }
                            let value = resolve_integer(literal, *negative, Some(&domain))?;
                            (ir::MatchPattern::Int(value), Some(value.bits))
                        }
                        Pattern::Tuple(fields) if fields.is_empty() => {
                            if domain != ir::Ty::Unit {
                                return Err(format!(
                                    "unit match pattern does not match {domain:?}"
                                ));
                            }
                            (ir::MatchPattern::Unit, Some(0))
                        }
                        Pattern::Path(_) | Pattern::Variant { .. } => {
                            let (ty, variant, fields) =
                                self.variant_pattern(arm_pattern, &domain)?;
                            let mut names = std::collections::HashSet::new();
                            let mut pending = Vec::new();
                            let mut work = 0;
                            let mut bindings = Vec::new();
                            for (index, pattern) in fields {
                                let mut path = vec![
                                    ir::Projection::Downcast(ir::VariantId {
                                        owner: ty.def().id,
                                        index: variant,
                                    }),
                                    ir::Projection::Field(index),
                                ];
                                self.pattern_locals(
                                    pattern,
                                    &ty.def().variants[variant].fields[index].ty,
                                    &mut path,
                                    &mut names,
                                    &mut pending,
                                    &mut work,
                                )?;
                            }
                            for binding in pending {
                                let local =
                                    self.local(&binding.name, binding.ty, binding.mutable)?;
                                bindings.push(ir::PatternBinding {
                                    local,
                                    projection: binding.projection,
                                });
                            }
                            (
                                ir::MatchPattern::EnumVariant {
                                    ty,
                                    variant,
                                    bindings,
                                },
                                Some(variant as u128),
                            )
                        }
                        Pattern::Apply { .. } => {
                            return Err("pattern application normalization invariant".into());
                        }
                        Pattern::Tuple(_) | Pattern::Struct { .. } => {
                            return Err(
                                "aggregate destructuring match patterns are unsupported".into()
                            );
                        }
                    };
                    let fully_covered = match &domain {
                        ir::Ty::Bool => covered.len() == 2,
                        ir::Ty::Unit => covered.len() == 1,
                        ir::Ty::Enum(ty) => covered.len() == ty.def().variants.len(),
                        ir::Ty::Int(ty) => {
                            ty.bits() < 64 && covered.len() as u64 == (1u64 << ty.bits())
                        }
                        _ => false,
                    };
                    let useful = !catchall
                        && !fully_covered
                        && key.is_none_or(|key| !covered.contains(&key));
                    let body = self.expr_expected_when(
                        &arm.body,
                        self.reachable && source.completes && useful,
                        expected,
                    )?;
                    self.scopes.pop();
                    let arm_flow = self.flow(body);
                    // Even unreachable arms impose their source type obligation.
                    result = self.join(&result, &arm_flow.ty, "match arms")?;
                    completes |= useful && arm_flow.completes;
                    if let Some(key) = key {
                        covered.insert(key);
                    } else {
                        catchall = true;
                    }
                    checked.push(ir::MatchArm { pattern, body });
                }
                let exhaustive = catchall
                    || match &domain {
                        ir::Ty::Never => true,
                        ir::Ty::Bool => covered.len() == 2,
                        ir::Ty::Unit => covered.len() == 1,
                        ir::Ty::Enum(ty) => covered.len() == ty.def().variants.len(),
                        ir::Ty::Int(ty) => {
                            ty.bits() < 64 && covered.len() as u64 == (1u64 << ty.bits())
                        }
                        _ => false,
                    };
                if !exhaustive {
                    return Err(format!("non-exhaustive match on {domain:?}"));
                }
                (
                    ir::ExprKind::Match {
                        scrutinee,
                        arms: checked,
                    },
                    Flow {
                        ty: result,
                        completes: source.completes && completes,
                    },
                )
            }
            Expr::Tuple(fields) => {
                if fields.is_empty() {
                    return Err("empty tuples must use ()".into());
                }
                let mut values = Vec::with_capacity(fields.len());
                let mut types = Vec::with_capacity(fields.len());
                let mut completes = true;
                for (index, field) in fields.iter().enumerate() {
                    let hint = match expected {
                        Some(ir::Ty::Tuple(types)) => types.get(index),
                        _ => None,
                    };
                    let value =
                        self.expr_expected_when(field, self.reachable && completes, hint)?;
                    let flow = self.flow(value);
                    completes &= flow.completes;
                    types.push(flow.ty);
                    values.push(value);
                }
                (
                    ir::ExprKind::Tuple(values),
                    Flow {
                        ty: ir::Ty::Tuple(types.into()),
                        completes,
                    },
                )
            }
            Expr::TraitCall { .. }
            | Expr::Apply { .. }
            | Expr::MethodCall { .. }
            | Expr::AssociatedCall { .. } => {
                return Err("application dispatcher invariant".into());
            }
            Expr::Struct { name, fields } => {
                self.infer_storage(fields.len())?;
                let evidence_fields = fields
                    .iter()
                    .map(|(name, value)| (InferField::Named(name), value))
                    .collect::<Vec<_>>();
                let ir::Ty::Struct(ty) =
                    self.constructor_type(name, None, &evidence_fields, expected)?
                else {
                    return Err("expected struct constructor".into());
                };
                if !ty.def().visibility.accessible_from(&self.module) {
                    return Err(format!("private struct {name}"));
                }
                let mut seen = std::collections::HashSet::new();
                let mut values = Vec::with_capacity(fields.len());
                let mut completes = true;
                for (name, value) in fields {
                    let (index, expected) = self.field_type(
                        &ir::Ty::Struct(ty.clone()),
                        &FieldSelector::Named(name.clone()),
                    )?;
                    if !seen.insert(index) {
                        return Err(format!("duplicate field {name}"));
                    }
                    let value = self.expr_expected_when(
                        value,
                        self.reachable && completes,
                        Some(&expected),
                    )?;
                    let flow = self.flow(value);
                    self.require(&flow, &expected)?;
                    completes &= flow.completes;
                    values.push((index, value));
                }
                if seen.len() != ty.def().fields.len() {
                    return Err(format!("missing fields in struct {}", ty.def().name));
                }
                (
                    ir::ExprKind::Struct {
                        ty: ty.clone(),
                        fields: values,
                    },
                    Flow {
                        ty: ir::Ty::Struct(ty),
                        completes,
                    },
                )
            }
            Expr::Field { base, field } => {
                let receiver = base.as_ref();
                let mut base = self.expr(base)?;
                let mut flow = self.flow(base);
                if let ir::Ty::Ref { referent, .. } = &flow.ty {
                    if !matches!(receiver, Expr::Var(_)) && !genuine_call(receiver) {
                        // A projected carrier has the same authority as any other
                        // static place. Validate its dereference without creating a
                        // local or borrow, then read the already evaluated base once.
                        let mut source = static_source_place(receiver)?;
                        source.fields.push(FieldSelector::Deref);
                        self.checked_place(&source)?;
                    }
                    let ty = referent.as_ref().clone();
                    self.accessible(&ty)?;
                    let deref = ir::ExprId(self.exprs.len());
                    self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
                    self.exprs.push(ir::TypedExpr {
                        ty: ty.clone(),
                        completion: self.exprs[base.0].completion,
                        kind: ir::ExprKind::Deref { value: base },
                    });
                    base = deref;
                    flow.ty = ty;
                }
                let (index, ty) = self.field_type(&flow.ty, field)?;
                (
                    ir::ExprKind::Field { base, index },
                    Flow {
                        ty,
                        completes: flow.completes,
                    },
                )
            }
            Expr::Integer(literal) => {
                let value = expectation.literal(literal, false)?;
                (ir::ExprKind::Int(value), Flow::value(ir::Ty::Int(value.ty)))
            }
            Expr::Float(literal) => {
                let ty = match (literal.suffix, expected) {
                    (Some(ty), _) => ty,
                    (None, Some(ir::Ty::Float(ty))) => *ty,
                    _ => ir::FloatTy::F64,
                };
                let value = literal.value(ty, false)?;
                (ir::ExprKind::Float(value), Flow::value(ir::Ty::Float(ty)))
            }
            Expr::Bool(value) => (ir::ExprKind::Bool(*value), Flow::value(ir::Ty::Bool)),
            Expr::Var(n) => {
                let local = self.lookup(n)?;
                let ty = self.locals[local.0].ty.clone();
                (
                    ir::ExprKind::Local(local),
                    Flow {
                        completes: inhabited(&ty),
                        ty,
                    },
                )
            }
            Expr::Println(text) => (
                ir::ExprKind::Println(text.clone()),
                Flow::value(ir::Ty::Unit),
            ),
            Expr::Unary { op, value } => {
                if matches!(op, UnaryOp::Neg)
                    && let Expr::Integer(literal) = value.as_ref()
                {
                    let value = expectation.literal(literal, true)?;
                    let id = ir::ExprId(self.exprs.len());
                    self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
                    self.exprs.push(ir::TypedExpr {
                        kind: ir::ExprKind::Int(value),
                        ty: ir::Ty::Int(value.ty),
                        completion: ir::Completion::Returns,
                    });
                    return Ok(id);
                }
                let mut hint = expected
                    .filter(|t| matches!(t, ir::Ty::Int(_) | ir::Ty::Float(_) | ir::Ty::Bool))
                    .cloned();
                if hint.is_none() {
                    hint = self.scalar_hint(value)?;
                }
                let value = if matches!(expectation, Expectation::CastableToInt(_)) {
                    self.expr_with_expectation(value, expectation)?
                } else {
                    self.expr_expected(value, hint.as_ref())?
                };
                let f = self.flow(value);
                let t = match op {
                    UnaryOp::Neg => match &f.ty {
                        ir::Ty::Int(ty) if ty.is_signed() => f.ty.clone(),
                        ir::Ty::Float(_) => f.ty.clone(),
                        ir::Ty::Never => {
                            return Err("unary negation of never is unsupported".into());
                        }
                        _ => return Err("unary negation requires a signed integer".into()),
                    },
                    UnaryOp::Not if matches!(f.ty, ir::Ty::Int(_)) => f.ty.clone(),
                    UnaryOp::Not => ir::Ty::Bool,
                };
                self.require(&f, &t)?;
                if matches!(op, UnaryOp::Neg) && f.ty == ir::Ty::Never {
                    return Err("unary negation of never is unsupported".into());
                }
                let ty = if f.ty == ir::Ty::Never {
                    ir::Ty::Never
                } else {
                    t
                };
                (
                    ir::ExprKind::Unary { op: *op, value },
                    Flow {
                        ty,
                        completes: f.completes,
                    },
                )
            }
            Expr::Binary { op, left, right } => {
                let logical = matches!(op, BinaryOp::And | BinaryOp::Or);
                let shifting = matches!(op, BinaryOp::Shl | BinaryOp::Shr);
                let hint = if logical {
                    Some(ir::Ty::Bool)
                } else {
                    let mut hint = expected
                        .filter(|t| matches!(t, ir::Ty::Int(_) | ir::Ty::Float(_)))
                        .cloned();
                    if hint.is_none() {
                        hint = self.scalar_hint(left)?;
                    }
                    if hint.is_none() && !shifting {
                        hint = self.scalar_hint(right)?;
                    }
                    hint
                };
                let left = self.expr_expected(left, hint.as_ref())?;
                let left_flow = self.flow(left);
                let right_hint = if shifting {
                    None
                } else {
                    hint.or_else(|| {
                        if matches!(
                            left_flow.ty,
                            ir::Ty::Int(_) | ir::Ty::Float(_) | ir::Ty::Bool
                        ) {
                            Some(left_flow.ty.clone())
                        } else {
                            None
                        }
                    })
                };
                let right = self.expr_expected_when(
                    right,
                    self.reachable && left_flow.completes,
                    right_hint.as_ref(),
                )?;
                let l = self.flow(left);
                let r = self.flow(right);
                if matches!(l.ty, ir::Ty::Ref { .. }) || matches!(r.ty, ir::Ty::Ref { .. }) {
                    return Err("reference comparisons and arithmetic are unsupported in B1".into());
                }
                if matches!(l.ty, ir::Ty::Tuple(_) | ir::Ty::Struct(_))
                    || matches!(r.ty, ir::Ty::Tuple(_) | ir::Ty::Struct(_))
                {
                    return Err("aggregate operators and comparisons are unsupported".into());
                }
                let numeric = if matches!(l.ty, ir::Ty::Int(_) | ir::Ty::Float(_)) {
                    l.ty.clone()
                } else if matches!(r.ty, ir::Ty::Int(_) | ir::Ty::Float(_)) {
                    r.ty.clone()
                } else {
                    ir::Ty::Int(ir::IntTy::I32)
                };
                let (operand, result) = match op {
                    BinaryOp::And | BinaryOp::Or => (ir::Ty::Bool, ir::Ty::Bool),
                    BinaryOp::Shl | BinaryOp::Shr => {
                        if l.ty == ir::Ty::Never || r.ty == ir::Ty::Never {
                            return Err("shift operator on semantic Never is unsupported".into());
                        }
                        if !matches!(l.ty, ir::Ty::Int(_)) || !matches!(r.ty, ir::Ty::Int(_)) {
                            return Err("shift operands must both be integers".into());
                        }
                        (l.ty.clone(), l.ty.clone())
                    }
                    BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor => {
                        let operand = self.join(&l.ty, &r.ty, "bitwise operands")?;
                        if !matches!(operand, ir::Ty::Int(_) | ir::Ty::Bool) {
                            return Err(
                                "bitwise operands require identical integer types or Bool".into()
                            );
                        }
                        (operand.clone(), operand)
                    }
                    BinaryOp::Eq | BinaryOp::Ne => {
                        (self.join(&l.ty, &r.ty, "equality operands")?, ir::Ty::Bool)
                    }
                    BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                        (numeric.clone(), ir::Ty::Bool)
                    }
                    _ => (numeric.clone(), numeric),
                };
                self.require(&l, &operand)?;
                if !shifting {
                    self.require(&r, &operand)?;
                }
                // Trait-based scalar operators do not gain a never coercion
                // merely because evaluating an operand cannot complete.
                if !logical && (l.ty == ir::Ty::Never || r.ty == ir::Ty::Never) {
                    return Err("binary operator on never is unsupported".into());
                }
                let completes = l.completes && (logical || r.completes);
                (
                    ir::ExprKind::Binary {
                        op: *op,
                        left,
                        right,
                    },
                    Flow {
                        ty: result,
                        completes,
                    },
                )
            }
            Expr::Call { name, args } => {
                let plan = self
                    .signatures
                    .plan
                    .as_ref()
                    .ok_or("call requires authoritative declaration registry")?;
                let declaration = *plan
                    .registry
                    .functions
                    .get(name)
                    .ok_or_else(|| format!("unresolved function {name}"))?;
                let (type_args, actual_regions) = if self
                    .applications
                    .borrow()
                    .last()
                    .is_some_and(|(owner, _, _)| owner == name)
                {
                    let (_, args, regions) = self.applications.borrow_mut().pop().unwrap();
                    (args, regions)
                } else if plan.param_env(declaration)?.arity() > 0 {
                    (self.infer_call_args(declaration,args,expected,true)?.ok_or("unsupported generic inference: insufficient evidence; explicit type arguments required")?, Arc::from([]))
                } else {
                    (Vec::new(), Arc::from([]))
                };
                let target = ir::CallTarget {
                    declaration,
                    type_args: type_args.into(),
                };
                let signature =
                    if plan.param_env(declaration)?.arity() == 0 && target.type_args.is_empty() {
                        self.signatures
                            .functions
                            .get(name)
                            .cloned()
                            .ok_or("missing function signature")?
                    } else {
                        plan.instantiate_signature(&target, self.facts(), &plan.budget)?
                    };
                let facts = self
                    .signatures
                    .facts
                    .as_deref()
                    .ok_or("direct call requires validated facts")?;
                let proof = facts
                    .service()
                    .discharge_direct(&target, facts)
                    .map_err(|e| e.into_legacy_diagnostic(facts.budget()))?;
                if !facts
                    .world()
                    .signatures_equal(proof.signature(), &signature)
                    .map_err(|e| e.into_legacy_diagnostic(facts.budget()))?
                {
                    return Err("canonical direct signature mismatch".into());
                }
                if self.signatures.param_env.is_some() {
                    for ty in signature
                        .params
                        .iter()
                        .chain(std::iter::once(&signature.result))
                    {
                        validate_root_carrier(ty, self.facts(), &plan.budget)?;
                    }
                }
                for ty in signature
                    .params
                    .iter()
                    .chain(std::iter::once(&signature.result))
                {
                    self.accessible(ty)?;
                }
                if signature.params.len() != args.len() {
                    return Err(format!(
                        "wrong arity for {name}: expected {}, got {}",
                        signature.params.len(),
                        args.len()
                    ));
                }
                return self.finish_checked_call(
                    target.into(),
                    name,
                    signature,
                    args,
                    actual_regions,
                    None,
                );
            }
            Expr::If {
                condition,
                then_block,
                else_block,
            } => {
                let condition = self.expr(condition)?;
                let c = self.flow(condition);
                self.require(&c, &ir::Ty::Bool)?;
                let reachable = self.reachable && c.completes;
                let (then_block, t) = self.block_expected_when(then_block, reachable, expected)?;
                let (else_block, e) = self.block_expected_when(else_block, reachable, expected)?;
                let ty = self.join(&t.ty, &e.ty, "if branches")?;
                let completes = c.completes && (t.completes || e.completes);
                (
                    ir::ExprKind::If {
                        condition,
                        then_block,
                        else_block,
                    },
                    Flow { ty, completes },
                )
            }
            Expr::Loop { label, body } => {
                let id = self.enter_loop(true);
                self.loops.last_mut().unwrap().label = label.clone();
                self.loops.last_mut().unwrap().expected = expected.cloned();
                // A loop has its own intrinsic completion facts even when the
                // enclosing expression is unreachable. Only its own body paths
                // determine whether one of its breaks can complete.
                let (body, body_flow) = self.block_when(body, true)?;
                self.require(&body_flow, &ir::Ty::Unit)?;
                let frame = self.loops.pop().unwrap();
                let flow = Flow {
                    ty: frame.break_ty.unwrap_or(ir::Ty::Never),
                    completes: frame.can_exit,
                };
                (ir::ExprKind::Loop { id, body }, flow)
            }
            Expr::Break { label, value } => {
                let index = self.loop_target("break", label.as_deref())?;
                let target = self.loops[index].id;
                let expected = self.loops[index].expected.clone();
                let value = value
                    .as_ref()
                    .map(|e| self.expr_expected(e, expected.as_ref()))
                    .transpose()?;
                let flow = value.map_or(Flow::value(ir::Ty::Unit), |e| self.flow(e));
                let frame = &self.loops[index];
                let previous_break_ty = frame.break_ty.clone();
                if !frame.accepts_value && value.is_some() {
                    return Err("break with a value is only allowed in loop, not while".into());
                }
                // Check every lexical break and the operand before contributing.
                // A typed but unreachable tail is not the never type.
                if self.reachable && flow.completes {
                    self.loops[index].can_exit = true;
                }
                if flow.ty != ir::Ty::Never {
                    let previous = previous_break_ty.unwrap_or(ir::Ty::Never);
                    let ty = self.join(&previous, &flow.ty, "break values")?;
                    self.loops[index].break_ty = Some(ty);
                }
                (ir::ExprKind::Break { target, value }, Flow::never())
            }
            Expr::Continue { label } => {
                let index = self.loop_target("continue", label.as_deref())?;
                let target = self.loops[index].id;
                (ir::ExprKind::Continue { target }, Flow::never())
            }
            Expr::LabeledBlock { label, body } => {
                let id = self.enter_loop(true);
                let frame = self.loops.last_mut().unwrap();
                frame.label = Some(label.clone());
                frame.is_block = true;
                frame.expected = expected.cloned();
                let (body, body_flow) = self.block_expected_when(body, true, expected)?;
                let frame = self.loops.pop().unwrap();
                let ty = self.join(
                    &body_flow.ty,
                    &frame.break_ty.unwrap_or(ir::Ty::Never),
                    "labeled block breaks",
                )?;
                (
                    ir::ExprKind::LabeledBlock { id, body },
                    Flow {
                        ty,
                        completes: body_flow.completes || frame.can_exit,
                    },
                )
            }
            Expr::Block(b) => {
                let (block, flow) = self.block_with_expectation(b, expectation)?;
                (ir::ExprKind::Block(block), flow)
            }
        };
        self.accessible(&flow.ty)?;
        admit_slice_value(&flow.ty)?;
        if matches!(flow.ty, ir::Ty::Array { .. } | ir::Ty::Tuple(_)) {
            self.validate_value_carriers(&flow.ty)?;
        }
        let id = ir::ExprId(self.exprs.len());
        self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
        self.exprs.push(ir::TypedExpr {
            ty: flow.ty,
            completion: if flow.completes {
                ir::Completion::Returns
            } else {
                ir::Completion::Never
            },
            kind,
        });
        Ok(id)
    }
}
/// Validate all declarations and publish one shared nominal type environment.
pub fn signature_table(program: &Program) -> Result<Signatures, String> {
    type_env(program)
}
pub fn check_signatures(program: &Program) -> Result<(), String> {
    signature_table(program).map(|_| ())
}
pub fn check_function(program: &Program, index: usize) -> Result<(), String> {
    let signatures = signature_table(program)?;
    let plan = signatures
        .plan
        .as_ref()
        .ok_or("source function check requires planning context")?;
    if !matches!(
        plan.callable_provider(plan.callable_decl(index)?)?,
        CallableProvider::Source { .. }
    ) {
        return Err("invalid source function index".into());
    }
    check_function_with_signatures(program, index, &signatures)
}
/// `signatures` must be the validated table for this same immutable program.
/// Direct semantic checking includes CFG initialization/move analysis once.
/// Graph callers use the pure type_function_with_env operation instead.
pub fn check_function_with_signatures(
    program: &Program,
    index: usize,
    signatures: &Signatures,
) -> Result<(), String> {
    let typed = type_function_with_signatures(program, index, signatures)?;
    if let Some(plan) = &signatures.plan
        && (plan.callable_env(plan.callable_decl(index)?)?.is_generic()
            || matches!(
                plan.callable_provider(plan.callable_decl(index)?)?,
                CallableProvider::Recipe { .. }
            ))
    {
        let plan = signatures
            .plan
            .as_ref()
            .ok_or("generic checking requires planning context")?;
        let facts = plan.callable_facts(plan.callable_decl(index)?)?;
        let template = super::cfg::lower_template(Arc::new(typed), facts, &plan.budget)?;
        return super::ownership::verify_template(template)
            .map(|_| ())
            .map_err(|analysis| {
                super::analysis_diagnostic(&plan.budget, "universal callable", index, &analysis)
            });
    }
    let plan = signatures
        .plan
        .as_ref()
        .ok_or("semantic checking requires authoritative world")?;
    let target = ir::CallTarget {
        declaration: plan.callable_decl(index)?,
        type_args: Arc::from([]),
    };
    let facts = plan.instance_facts(&target)?;
    let mut cx = plan.clone();
    let typed = ir::close_checked_calls(&typed, &facts, &mut cx, &plan.budget)
        .map_err(|e| e.into_legacy_diagnostic(&plan.budget))?;
    let lowered = super::cfg::lower_with_facts(&typed, &facts, &plan.budget)?;
    let closed = super::cfg::close(Arc::new(lowered), facts, &plan.budget)?;
    let analysis = match super::ownership::analyze_closed(closed) {
        Ok(_) => return Ok(()),
        Err(analysis) => analysis,
    };
    if !analysis.passed() {
        return Err(super::analysis_diagnostic(
            &plan.budget,
            &typed.name,
            index,
            &analysis,
        ));
    }
    Ok(())
}
/// Check and lower one body using the validated signatures for this program.
/// The returned arena owns all resolved locals, expression types and call facts.
pub fn type_function_with_signatures(
    program: &Program,
    index: usize,
    signatures: &Signatures,
) -> Result<ir::CheckedFunction, String> {
    type_function_with_env(program, index, signatures)
}
pub fn type_callable_with_plan(
    program: &Program,
    declaration: ir::DeclId,
    plan: &PlanCtx,
    budget: &ir::PlanBudget,
) -> Result<ir::CheckedFunction, String> {
    type_callable_source(program, declaration, plan, budget, None)
}
fn type_callable_source(
    program: &Program,
    declaration: ir::DeclId,
    plan: &PlanCtx,
    budget: &ir::PlanBudget,
    canonical: Option<&CanonicalSourceCapability>,
) -> Result<ir::CheckedFunction, String> {
    plan.check_budget(budget)?;
    budget.charge_work(1)?;
    if matches!(
        plan.callable_provider(declaration)?,
        CallableProvider::Recipe { .. }
    ) {
        let env = plan.callable_env(declaration)?;
        budget.retain_bytes(env.total_arity() * std::mem::size_of::<ir::Ty>() + 128)?;
        let target = ir::CallTarget {
            declaration,
            type_args: env.ordered_params().map(ir::Ty::Param).collect(),
        };
        let facts = plan.callable_facts(declaration)?;
        return plan.checked_body(&target, &facts, budget);
    }
    let f = plan.callable_body(program, declaration)?;
    if f.constness != plan.callable_body(&plan.program, declaration)?.constness {
        return Err(
            "source callable constness differs from its immutable planning declaration".into(),
        );
    }
    if let CallableSource::TraitDefault { trait_index, item } =
        plan.registry.callable_sources[plan.callable_index(declaration)?]
    {
        let supplied = &program.traits[trait_index].items[item].signature;
        let original = &plan.program.traits[trait_index].items[item].signature;
        if supplied.constness != original.constness || supplied.constness != f.constness {
            return Err(
                "source trait signature constness differs from its immutable planning declaration"
                    .into(),
            );
        }
    }
    validate_const_forwarding(f, budget)?;
    let mut signatures = plan.environment(Some(declaration))?;
    if let Some(cap) = canonical {
        signatures.facts = Some(cap.facts.clone());
    }
    let signature = plan.function_schema(declaration)?.signature;
    let context = plan.callable_context(declaration)?;
    validate_depth(DepthNode::Block(&f.body))?;
    let module = context.module.join("::");
    let mut c = Checker::new(&signatures, signature.result.clone(), module);
    c.declaration = Some(declaration);
    c.canonical = canonical;
    let mut params = Vec::new();
    let offset = if context.receiver != ReceiverKind::None {
        let local = c.bind("self", signature.params[0].clone())?;
        c.locals[local.0].mutable =
            matches!(context.receiver, ReceiverKind::Value { mutable: true });
        params.push(local);
        1
    } else {
        0
    };
    if signature.params.len() != f.params.len() + offset {
        return Err("callable parameter count mismatch".into());
    }
    for (parameter, ((name, _), ty)) in f.params.iter().zip(&signature.params[offset..]).enumerate()
    {
        let local = c.bind(name, ty.clone())?;
        if let Some(cap) = canonical {
            sr::work(budget, 1).map_err(|e| cap.render(e))?;
            let index = parameter;
            if !cap
                .inventory
                .bindings
                .get(index)
                .is_some_and(|site| site.parameter == Some(parameter))
            {
                return Err(cap.render(FactsError::invalid(
                    budget,
                    "source parameter prefix occurrence changed",
                )));
            }
            sr::push(&mut cap.bindings.borrow_mut(), (index, local), budget)
                .map_err(|e| cap.render(e))?;
        }
        params.push(local);
    }
    let (body, flow) = c
        .block_expected(&f.body, Some(&signature.result))
        .map_err(|e| format!("in callable {}: {e}", f.name))?;
    c.require(&flow, &c.result)?;
    validate_shift_counts(&c.exprs, &c.count_queries)?;
    let checked = ir::CheckedFunction {
        name: f.name.clone(),
        ref_contract: signature.ref_contract.clone(),
        params,
        result: c.result,
        locals: c.locals,
        exprs: c.exprs,
        call_evidence: c.call_evidence.into(),
        body_substitution: None,
        body,
    };
    if canonical.is_some() {
        return Ok(checked);
    }
    let facts = plan.callable_facts(declaration)?;
    ir::validate_call_evidence(&checked, &facts, budget)?;
    let mut cx = plan.clone();
    ir::canonicalize_checked_calls(&checked, &facts, &mut cx, budget)
        .map_err(|e| e.into_legacy_diagnostic(budget))
}
pub fn type_function_with_plan(
    program: &Program,
    index: usize,
    plan: &PlanCtx,
    budget: &ir::PlanBudget,
) -> Result<ir::CheckedFunction, String> {
    plan.check_budget(budget)?;
    budget.charge_work(1)?;
    let decl = plan.function_decl(index)?;
    type_callable_with_plan(program, decl, plan, budget)
}
/// Check one body with the same nominal environment used for all signatures.
pub fn type_function_with_env(
    program: &Program,
    index: usize,
    signatures: &TypeEnv,
) -> Result<ir::CheckedFunction, String> {
    if let Some(plan) = &signatures.plan {
        return type_callable_with_plan(program, plan.callable_decl(index)?, plan, &plan.budget);
    }
    let f = program
        .functions
        .get(index)
        .ok_or_else(|| format!("invalid function index {index}"))?;
    if f.constness == Constness::Const {
        return Err(
            "unsupported: const fn checking requires its authoritative planning context".into(),
        );
    }
    if !f.type_params.is_empty() && signatures.param_env.is_none() {
        let plan = signatures
            .plan
            .as_ref()
            .ok_or("generic body requires authoritative planning context")?;
        return type_function_with_plan(program, index, plan, &plan.budget);
    }
    validate_depth(DepthNode::Block(&f.body))?;
    let signature = signatures
        .functions
        .get(&f.name)
        .ok_or_else(|| format!("missing signature for {}", f.name))?;
    if signature.params.len() != f.params.len() {
        return Err(format!("signature parameter count mismatch for {}", f.name));
    }
    let mut c = Checker::new(signatures, signature.result.clone(), parent(&f.name).into());
    let mut params = Vec::with_capacity(f.params.len());
    for ((n, _), t) in f.params.iter().zip(&signature.params) {
        params.push(
            c.bind(n, t.clone())
                .map_err(|e| format!("in function {}: {e}", f.name))?,
        );
    }
    let (body, flow) = c
        .block_expected(&f.body, Some(&signature.result))
        .map_err(|e| format!("in function {}: {e}", f.name))?;
    c.require(&flow, &c.result)
        .map_err(|e| format!("in function {}: {e}", f.name))?;
    validate_shift_counts(&c.exprs, &c.count_queries)
        .map_err(|e| format!("in function {}: {e}", f.name))?;
    Ok(ir::CheckedFunction {
        name: f.name.clone(),
        ref_contract: signature.ref_contract.clone(),
        params,
        result: c.result,
        locals: c.locals,
        exprs: c.exprs,
        call_evidence: c.call_evidence.into(),
        body_substitution: None,
        body,
    })
}
pub fn check(program: &Program) -> Result<(), String> {
    let signatures = signature_table(program)?;
    let count = signatures
        .plan
        .as_ref()
        .map_or(program.functions.len(), PlanCtx::callable_count);
    for i in 0..count {
        check_function_with_signatures(program, i, &signatures)?;
    }
    Ok(())
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
        assert_eq!(program.functions[0].name, "m::forward");
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
            assert!(error.contains("constness differs"), "{error}");
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
mod typed_tests {
    use super::*;

    #[test]
    fn canonical_copy_identity_and_source_supertraits_authorize_only_real_bounds() {
        let source = "trait Copy{fn fake(&self)->i32;}trait Clone{fn fake_clone(&self)->i32;}struct S{v:i32}impl core::clone::Clone for S{fn clone(&self)->Self{S{v:self.v}}}impl core::marker::Copy for S{}fn twice<T:std::marker::Copy>(x:T)->(T,T){(x,x)}fn main(){let s=S{v:3};let p=twice::<S>(s);let n=p.0.v+p.1.v;}";
        let p = parse(source).unwrap();
        check(&p).unwrap();
        let p = parse("trait Base{fn base(&self)->i32;}trait Child:Base{fn child(&self)->i32;}struct S{v:i32}impl Base for S{fn base(&self)->i32{self.v}}impl Child for S{fn child(&self)->i32{self.v}}fn via<T:Child>(x:&T)->i32{x.base()+x.child()}fn main(){}").unwrap();
        check(&p).unwrap();
        let fake = parse("trait Copy{fn tag(&self)->i32;}struct S{v:i32}impl Copy for S{fn tag(&self)->i32{self.v}}fn main(){let s=S{v:1};let a=s;let b=s;let n=a.v+b.v;}").unwrap();
        assert!(check(&fake).is_err());
    }

    #[test]
    fn canonical_clone_receiver_order_and_primitive_temporaries_use_real_borrows() {
        let p = parse("struct S{v:i32}impl Clone for S{fn clone(&self)->Self{S{v:self.v+1}}}impl S{fn set(&mut self,n:i32){self.v=n;}}fn carrier(x:&mut S)->&mut S{x}struct Opaque{v:i32}fn main(){let s=S{v:12};let r=&s;let value=r.clone();let owner=Opaque{v:13};let shared=&owner;let handle=shared.clone();let n=10i32.clone();let yes=true.clone();let unit=().clone();let used=n+value.v+handle.v;let mut temporary=S{v:1};carrier(&mut temporary).set(2);}").unwrap();
        let signatures = signature_table(&p).unwrap();
        let checked = type_function_with_signatures(&p, 1, &signatures).unwrap();
        assert!(checked.exprs.iter().any(|e| matches!(&e.kind, ir::ExprKind::Block(block) if block.statements.iter().any(|s| matches!(s,ir::Stmt::Let{local,..} if checked.locals[local.0].name=="$primitive_receiver")))));
        assert!(checked.exprs.iter().any(|e|matches!(&e.ty,ir::Ty::Ref{referent,..} if matches!(referent.as_ref(),ir::Ty::Struct(t) if t.def().name=="Opaque"))));
        check(&p).unwrap();
    }

    #[test]
    fn registered_recipe_bodies_share_counting_and_universal_verification() {
        let p = Arc::new(parse("fn main(){let tuple=(1i32,true);let array:[i32;0]=[];}").unwrap());
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_shared_program(p.clone(), &budget).unwrap();
        let mut recipes_seen = 0;
        for index in 0..plan.callable_count() {
            let declaration = plan.callable_decl(index).unwrap();
            if let CallableProvider::Recipe { descriptor } =
                plan.callable_provider(declaration).unwrap()
            {
                recipes_seen += 1;
                assert!(
                    matches!(plan.declaration_origin(declaration).unwrap(),tr::DeclOrigin::CoreCatalog{item:tr::CatalogItem::Recipe(d),..} if d==descriptor)
                );
                let env = plan.callable_env(declaration).unwrap();
                let target = ir::CallTarget {
                    declaration,
                    type_args: env.ordered_params().map(ir::Ty::Param).collect(),
                };
                let facts = plan.callable_facts(declaration).unwrap();
                let checked = plan.checked_body(&target, &facts, &budget).unwrap();
                let template =
                    super::super::cfg::lower_template(Arc::new(checked), facts, &budget).unwrap();
                super::super::ownership::verify_template(template).unwrap();
            }
        }
        assert!(recipes_seen >= 14);
        let counts =
            recipes::counts(tr::RecipeDescriptor::CloneTuple { arity: 33 }, &budget).unwrap();
        assert!(matches!(
            super::super::ownership::preflight_recipe_counts(&counts, &budget).unwrap(),
            super::super::ownership::RecipeAdmission::IntrinsicLimit(_)
        ));
    }

    fn input_region(param: usize, region: u16) -> ir::InputRegion {
        ir::InputRegion {
            input: ir::InputReference {
                param,
                position: ir::ReferencePosition { depth: 0 },
            },
            region: ir::RegionParamId(region),
        }
    }

    #[test]
    fn nested_shared_contracts_preserve_positions_aliases_and_implied_outlives() {
        let p = parse("fn inner<'a,'b>(rr:&'a &'b i32)->&'b i32{*rr}fn renamed<'z,'w>(rr:&'z &'w i32)->&'w i32{*rr}fn id<'a,'b>(rr:&'a &'b i32)->&'a &'b i32{rr}fn main(){}").unwrap();
        let env = signature_table(&p).unwrap();
        let inner = &env.functions["inner"].ref_contract;
        assert_eq!(inner, &env.functions["renamed"].ref_contract);
        assert_eq!(
            inner.input_regions.as_ref(),
            &[
                input_region(0, 0),
                ir::InputRegion {
                    input: ir::InputReference {
                        param: 0,
                        position: ir::ReferencePosition { depth: 1 }
                    },
                    region: ir::RegionParamId(1)
                },
            ]
        );
        assert_eq!(
            inner.outlives.as_ref(),
            &[(ir::RegionParamId(1), ir::RegionParamId(0))]
        );
        assert_eq!(inner.results[0].region, ir::RegionParamId(1));
        assert_eq!(
            inner.results[0].returns,
            ir::ReturnContract::SharedInputPaths(
                vec![ir::InputPath {
                    input: ir::InputReference {
                        param: 0,
                        position: ir::ReferencePosition { depth: 1 }
                    },
                    projection: Arc::from([])
                }]
                .into()
            )
        );
        let id = &env.functions["id"].ref_contract;
        assert_eq!(id.results.len(), 2);
        for depth in 0..2 {
            assert_eq!(id.results[depth].position.depth, depth as u8);
            assert_eq!(
                id.results[depth].returns,
                ir::ReturnContract::SharedInputPaths(
                    vec![ir::InputPath {
                        input: ir::InputReference {
                            param: 0,
                            position: ir::ReferencePosition { depth: depth as u8 }
                        },
                        projection: Arc::from([])
                    }]
                    .into()
                )
            );
        }
        for index in 0..p.functions.len() {
            check_function_with_signatures(&p, index, &env).unwrap();
        }
    }

    #[test]
    fn nested_shared_source_authority_and_closed_boundaries_remain_distinct() {
        let p = parse("trait Read{fn inner<'a,'b>(rr:&'a &'b i32)->&'b i32;}fn legacy(r:&i32){}fn named<'a>(r:&i32){}fn nested<'a,'b>(r:&'a &'b i32){}fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&p, &budget).unwrap();
        let adapter = SchemaAdapter {
            program: plan.program.clone(),
            catalog: plan.catalog.clone(),
            registry: plan.registry.clone(),
            state: plan.state.clone(),
            budget: budget.clone(),
        };
        for (index, mode) in [
            (0, tr::RegionContractMode::LegacyUnnamedFreeNoReturn),
            (1, tr::RegionContractMode::Positioned),
            (2, tr::RegionContractMode::Positioned),
        ] {
            let decl = plan.function_decl(index).unwrap();
            assert_eq!(
                tr::SchemaCx::declaration_info(&adapter, decl)
                    .unwrap()
                    .region_contract_mode,
                mode
            );
            assert_eq!(
                tr::SchemaCx::declaration_schema(&adapter, decl)
                    .unwrap()
                    .signature,
                plan.raw_schema(decl).unwrap().signature
            );
        }
        let item = plan.registry.trait_items[0][0];
        assert_eq!(
            tr::SchemaCx::declaration_info(&adapter, item)
                .unwrap()
                .region_contract_mode,
            tr::RegionContractMode::Positioned
        );
        assert_eq!(
            tr::SchemaCx::declaration_schema(&adapter, item)
                .unwrap()
                .signature
                .ref_contract
                .input_regions
                .len(),
            2
        );
        for source in [
            "fn f(x:&&i32)->&i32{*x}fn main(){}",
            "fn f<'a>(x:&'a &'a i32)->&i32{*x}fn main(){}",
        ] {
            assert!(
                signature_table(&parse(source).unwrap())
                    .unwrap_err()
                    .contains("missing lifetime specifier")
            );
        }
        assert!(
            parse("struct Holder<'a>{r:&'a i32}fn main(){}")
                .unwrap_err()
                .contains("lifetime parameters on nominal and impl declarations are unsupported")
        );
        assert!(
            parse("struct Holder<'fn>{r:i32}fn main(){}")
                .unwrap_err()
                .contains("keyword lifetime names")
        );
        for source in [
            "fn f(x:&&mut i32){}fn main(){}",
            "fn f(x:&mut &i32){}fn main(){}",
            "fn f(x:&&&i32){}fn main(){}",
            "fn f<T>(x:T){}fn main(){let x=1;let r=&x;f::<&i32>(r);}",
            "fn main(){let x=1;let r=&x;let rr=(&r,);}",
        ] {
            parse(source)
                .and_then(|p| check(&p))
                .unwrap_or_else(|error| panic!("{source}: {error}"));
        }
        // Source-declared lifetime-bearing nominal fields remain outside the
        // internally certified carrier-substitution domain.
        let source = "struct S{x:&&i32}fn main(){}";
        assert!(
            parse(source).and_then(|p| check(&p)).is_err(),
            "accepted {source}"
        );
    }

    #[test]
    fn nested_shared_real_call_loads_are_once_only_and_named_locals_stay_excluded() {
        check(&parse("fn nested<'o,'i,T>(x:&'o &'i T)->&'o &'i T{x}fn main(){let x=11;let r=&x;let rr=nested(&r);let n=**rr;}").unwrap()).unwrap();
        let p = parse("fn inner<'a,'b>(rr:&'a &'b i32)->&'b i32{*rr}fn id<'a,'b>(rr:&'a &'b i32)->&'a &'b i32{rr}fn main(){let x=7;let r=&x;let n=*inner(&r);let m=**id(&r);let rr:&&i32=&r;let copied=*rr;let y=*copied;}").unwrap();
        let env = signature_table(&p).unwrap();
        let main = type_function_with_signatures(&p, 2, &env).unwrap();
        assert_eq!(
            main.exprs
                .iter()
                .filter(|e| matches!(e.kind, ir::ExprKind::Call { .. }))
                .count(),
            2
        );
        assert!(main.exprs.iter().any(|e| matches!(e.kind, ir::ExprKind::Deref { value } if matches!(main.exprs[value.0].kind, ir::ExprKind::Deref { .. }))));
        check_function_with_signatures(&p, 2, &env).unwrap();
        let p = parse("fn bad<'a>(x:&'a i32){let r:&&'a i32=&x;}fn main(){}").unwrap();
        let env = signature_table(&p).unwrap();
        assert!(
            type_function_with_signatures(&p, 0, &env)
                .unwrap_err()
                .contains("named lifetime annotations on locals")
        );
    }

    #[test]
    fn generic_carrier_call_traces_preserve_firm_reborrows_and_opaque_moves() {
        let p = parse("fn id<T>(x:T)->T{x}fn forward<T>(x:T)->T{id::<T>(x)}fn firm<'a>(x:&'a mut i32)->&'a mut i32{id::<&'a mut i32>(x)}fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&p, &budget).unwrap();
        let forward =
            type_callable_with_plan(&p, plan.function_decl(1).unwrap(), &plan, &budget).unwrap();
        let firm_decl = plan.function_decl(2).unwrap();
        let firm = type_callable_with_plan(&p, firm_decl, &plan, &budget).unwrap();
        assert_eq!(forward.call_evidence.len(), 1);
        assert_eq!(firm.call_evidence.len(), 1);
        let opaque = &forward.call_evidence[0].arguments[0];
        assert_eq!(opaque.expectation, ir::CallExpectation::NoFirmReference);
        assert!(matches!(
            forward.exprs[opaque.value.0].kind,
            ir::ExprKind::Local(_)
        ));
        let explicit = &firm.call_evidence[0].arguments[0];
        assert_eq!(
            explicit.expectation,
            ir::CallExpectation::FirmReference {
                kind: ir::BorrowKind::Mut
            }
        );
        assert!(matches!(
            firm.exprs[explicit.value.0].kind,
            ir::ExprKind::Borrow {
                timing: ir::BorrowTiming::TwoPhase { .. },
                ..
            }
        ));
        assert_eq!(firm.call_evidence[0].type_actual_regions.len(), 1);
        assert_eq!(
            firm.call_evidence[0].type_actual_regions[0].source,
            ir::ActualRegion::Named(ir::SourceRegionId {
                declaration: firm_decl,
                index: 0
            })
        );
    }
    #[test]
    fn generic_carrier_original_region_env_retains_unused_names_and_required_items() {
        let p = parse("trait Get{fn get<'a,'unused>(&'a self)->&'a Self;}fn keep<'a,'b:'a,'unused,T>(x:&'a T)->&'a T{x}fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&p, &budget).unwrap();
        let declaration = plan.function_decl(0).unwrap();
        let env = plan.source_region_env(declaration).unwrap();
        assert_eq!(env.declaration, declaration);
        assert_eq!(env.region_count, 3);
        assert_eq!(env.normalized.len(), 3);
        assert!(env.normalized[0].is_some());
        assert_eq!(env.normalized[1..], [None, None]);
        assert_eq!(
            env.outlives.as_ref(),
            &[(
                ir::SourceRegionId {
                    declaration,
                    index: 1
                },
                ir::SourceRegionId {
                    declaration,
                    index: 0
                }
            )]
        );
        let item = plan.registry.trait_items[0][0];
        let item_env = plan.source_region_env(item).unwrap();
        assert_eq!(item_env.region_count, 2);
        assert!(item_env.normalized[0].is_some());
        assert_eq!(item_env.normalized[1], None);
        let foreign =
            parse("fn id<T>(x:T)->T{x}fn bad(x:&i32)->&i32{id::<&'missing i32>(x)}fn main(){}")
                .unwrap();
        let foreign_plan = PlanCtx::from_program(&foreign, &ir::PlanBudget::default()).unwrap();
        assert!(
            type_callable_with_plan(
                &foreign,
                foreign_plan.function_decl(1).unwrap(),
                &foreign_plan,
                &foreign_plan.budget
            )
            .unwrap_err()
            .contains("undeclared lifetime")
        );
    }
    #[test]
    fn generic_carrier_internal_nominal_transport_does_not_relax_source_actuals() {
        let p = parse("struct Wrap<T>{value:T}enum E<T>{One{value:T}}fn wrap<T>(x:T)->E<T>{let w=Wrap{value:x};E::One{value:w.value}}fn main(){let x=1;let e=wrap::<&i32>(&x);}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&p, &budget).unwrap();
        let body =
            type_callable_with_plan(&p, plan.function_decl(0).unwrap(), &plan, &budget).unwrap();
        assert!(
            body.locals
                .iter()
                .any(|local| matches!(&local.ty, ir::Ty::Struct(ty) if ty.def().name == "Wrap"))
        );
        let caller =
            type_callable_with_plan(&p, plan.function_decl(1).unwrap(), &plan, &budget).unwrap();
        assert!(caller.exprs.iter().any(|expr| matches!(&expr.ty, ir::Ty::Enum(ty) if matches!(ty.def().variants[0].fields[0].ty, ir::Ty::Ref { .. }))));
        for source in [
            "struct Wrap<T>{value:T}fn main(){let x=1;let w=Wrap::<&i32>{value:&x};}",
            "fn id<T>(x:T)->T{x}fn main(){let x=1;let r=id(&x);}",
        ] {
            assert!(
                parse(source)
                    .and_then(|program| check(&program))
                    .unwrap_err()
                    .contains("unsupported")
            );
        }
    }
    #[test]
    fn generic_reference_declarations_keep_rigid_envelopes_and_canonical_regions() {
        check(&parse("fn id<T>(x:&T)->&T{x}fn wrap(x:&i32)->&i32{id(x)}fn main(){let x=7;let r=wrap(&x);let n=*r;}").unwrap()).unwrap();
        let p=parse("fn first<'a,'b,T,U>(x:&'a T,y:&'b U)->&'a T{let _=y;x}fn renamed<'z,'w,T,U>(x:&'z T,y:&'w U)->&'z T{let _=y;x}fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&p, &budget).unwrap();
        let declaration = plan.function_decl(0).unwrap();
        let schema = plan.signature(declaration).unwrap();
        assert_eq!(
            schema.signature.ref_contract,
            plan.signature(plan.function_decl(1).unwrap())
                .unwrap()
                .signature
                .ref_contract
        );
        assert!(
            matches!(&schema.signature.ref_contract.results[0].returns,ir::ReturnContract::SharedInputEnvelopes(rows) if rows.len()==1&&rows[0].input.param==0)
        );
        let closed = plan
            .instantiate_signature(
                &ir::CallTarget {
                    declaration,
                    type_args: vec![ir::Ty::Int(ir::IntTy::I32); 2].into(),
                },
                ir::FactsCtx::Concrete,
                &budget,
            )
            .unwrap();
        assert_eq!(closed.ref_contract, schema.signature.ref_contract);
        for i in 0..plan.callable_count() {
            let d = plan.callable_decl(i).unwrap();
            type_callable_with_plan(&p, d, &plan, &budget).unwrap();
        }
        for source in [
            "fn id<T>(x:T)->T{x}fn main(){let x=1;let r=id::<&i32>(&x);}",
            "fn bad<T>(x:&&T){}fn main(){}",
            "struct W<T>{x:T}fn main(){let x=1;let w=W::<&i32>{x:&x};}",
        ] {
            let result = parse(source).and_then(|p| check(&p));
            if source != "struct W<T>{x:T}fn main(){let x=1;let w=W::<&i32>{x:&x};}" {
                result.unwrap();
            } else {
                assert!(result.is_err(), "accepted {source}");
            }
        }
    }
    #[test]
    fn borrowed_receiver_and_argument_adjustments_bind_exact_calls_without_owner_moves() {
        let p=parse("struct S{x:i32}impl S{fn get(&self)->i32{self.x}fn set(&mut self,n:i32){self.x=n;}}fn put(r:&mut i32,n:i32){*r=n;}fn main(){let mut s=S{x:1};s.set(s.get()+1);let mut x=1;let r=&mut x;put(r,*r+1);}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&p, &budget).unwrap();
        let main = plan.callable_decl(1).unwrap();
        let typed = type_callable_with_plan(&p, main, &plan, &budget).unwrap();
        let mut receiver = false;
        let mut argument = false;
        for (index, expr) in typed.exprs.iter().enumerate() {
            if let ir::ExprKind::Borrow {
                timing:
                    ir::BorrowTiming::TwoPhase {
                        activation,
                        argument: slot,
                        reason,
                    },
                ..
            } = &expr.kind
            {
                let ir::ExprKind::Call { args, .. } = &typed.exprs[activation.0].kind else {
                    panic!("missing activation");
                };
                assert_eq!(args[*slot], ir::ExprId(index));
                match reason {
                    ir::ImplicitBorrowKind::MutableReceiver => receiver = true,
                    ir::ImplicitBorrowKind::MutableArgumentReborrow { carrier } => {
                        argument = true;
                        assert!(matches!(
                            typed.exprs[carrier.0].kind,
                            ir::ExprKind::Local(_)
                        ));
                    }
                }
            }
        }
        assert!(receiver && argument);
        assert!(typed.exprs.iter().any(|e| matches!(
            e.kind,
            ir::ExprKind::Borrow {
                timing: ir::BorrowTiming::Immediate,
                ..
            }
        )));
        check(&p).unwrap();
    }
    #[test]
    fn receiver_regions_call_carriers_and_explicit_immediate_conflicts_remain_distinct() {
        let p=parse("trait Get{fn get<'a>(&'a self)->&'a i32;}struct S{x:i32}impl Get for S{fn get<'z>(&'z self)->&'z i32{&self.x}}impl S{fn choose(&self,other:&i32)->&i32{let _=other;&self.x}}fn id<T>(x:&mut T)->&mut T{x}fn main(){let mut a=[1];*id::<i32>(&mut a[0])=2;let s=S{x:3};let r;{let other=1;r=s.choose(&other);}let n=*r;let p=<S as Get>::get(&s);}").unwrap();
        check(&p).unwrap();
        for source in [
            "struct S{x:i32}impl S{fn get(&self)->i32{self.x}fn set(&mut self,n:i32){self.x=n;}}fn main(){let mut s=S{x:1};S::set(&mut s,s.get());}",
            "fn put(r:&mut i32,n:i32){*r=n;}fn main(){let mut x=1;let r=&mut x;put(&mut *r,*r);}",
            "fn bad<'a,'b,T,U>(x:&'a T,y:&'b U)->&'a U{let _=x;y}fn main(){}",
        ] {
            assert!(parse(source).and_then(|p| check(&p)).is_err());
        }
    }

    #[test]
    fn trait_default_sources_have_separate_identity_and_rigid_self_facts() {
        let p = parse(
            "mod m{fn helper()->i32{7}pub trait V{fn value(&self)->i32{helper()}}}fn main(){}",
        )
        .unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&p, &budget).unwrap();
        let trait_decl = plan.registry.trait_ids[0];
        let item = plan.registry.trait_items[0][0];
        let default = plan
            .registry
            .callable_sources
            .iter()
            .position(|s| {
                matches!(
                    s,
                    CallableSource::TraitDefault {
                        trait_index: 0,
                        item: 0
                    }
                )
            })
            .map(|i| plan.registry.function_ids[i])
            .unwrap();
        assert_ne!(default, item);
        assert_eq!(plan.declaration(default).unwrap().owner, Some(trait_decl));
        assert_eq!(
            plan.declaration(default).unwrap().path.module.as_ref(),
            &["m".to_owned(), "V".to_owned()]
        );
        assert_eq!(
            plan.callable_context(default).unwrap().module.as_ref(),
            &["m".to_owned()]
        );
        assert_eq!(plan.callable_env(default).unwrap().total_arity(), 1);
        let self_ty = plan.callable_context(default).unwrap().self_ty.unwrap();
        assert!(plan.callable_facts(default).unwrap().predicates().predicates().iter().any(|p| matches!(p, ir::Predicate::Implements(r) if r.declaration == trait_decl && r.self_ty == self_ty)));
        assert!(matches!(
            plan.publish_callable_body(default, &budget).unwrap(),
            PublishedCallableBody::Source(_)
        ));
        check(&p).unwrap();
        for source in [
            "trait V{fn v(&self)->i32{true}}fn main(){}",
            "trait V:Sized{fn v(self)->(Self,Self){(self,self)}}fn main(){}",
            "trait V{fn v(&mut self)->i32{let a=&mut *self;let b=&mut *self;a.v()+b.v()}}fn main(){}",
            "trait V{fn v(&self)->i32{missing()}}fn main(){}",
        ] {
            assert!(parse(source).and_then(|p| check(&p)).is_err(), "{source}");
        }
    }
    #[test]
    fn trait_required_schemas_and_bound_calls_retain_authoritative_identities() {
        let p=parse("struct S{x:i32}trait Value{fn value(self)->i32;}impl Value for S{fn value(mut self)->i32{self.x=self.x+1;self.x}}fn read<T:Value>(x:T)->i32{<T as Value>::value(x)}fn main(){let n=read::<S>(S{x:6});}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&p, &budget).unwrap();
        assert_eq!(
            (0..plan.callable_count())
                .filter(|&i| matches!(
                    plan.callable_provider(plan.callable_decl(i).unwrap())
                        .unwrap(),
                    CallableProvider::Source { .. }
                ))
                .count(),
            3
        );
        let generic = plan.callable_decl(0).unwrap();
        let facts = plan.callable_facts(generic).unwrap();
        assert_eq!(facts.predicates().predicates().len(), 2);
        let typed = type_callable_with_plan(&p, generic, &plan, &budget).unwrap();
        assert!(typed.exprs.iter().any(|e|matches!(&e.kind,ir::ExprKind::Call{target:ir::Callee::TraitItem(call),..} if matches!(call.trait_ref.self_ty,ir::Ty::Param(_)))));
        let trait_decl = plan.registry.trait_ids[0];
        let item = plan.registry.trait_items[0][0];
        assert!(plan.callable_index(item).is_err());
        let schema = plan.trait_world().unwrap().schema_facts(item).unwrap();
        assert_eq!(schema.phase(), tr::FactsPhase::Schema);
        assert!(schema.must_copy(&ir::Ty::Int(ir::IntTy::I32)).is_err());
        assert_eq!(plan.declaration(item).unwrap().owner, Some(trait_decl));
        check(&p).unwrap();
    }
    #[test]
    fn trait_unused_conformance_and_direct_obligations_are_not_body_witnesses() {
        for source in [
            "struct S{x:i32}trait V{fn v(self)->i32;}impl V for S{}fn main(){}",
            "struct S{x:i32}trait V{fn v(self)->i32;}impl V for S{fn v(self)->u8{1u8}}fn main(){}",
            "struct S{x:i32}trait V{fn v(self)->i32;}impl V for S{fn v(self)->i32{1}}impl V for S{fn v(self)->i32{2}}fn main(){}",
            "trait V{fn v(self)->i32;}fn bad<T:V>(x:T)->(T,T){(x,x)}fn main(){}",
            "struct S{x:i32}trait V{fn v(self)->i32;}fn need<T:V>(x:T){}fn main(){if false{need::<S>(S{x:1});}}",
            "trait V{fn v(self)->i32;}fn need<T:V>(x:T){}fn bad<T>(x:T){need::<T>(x)}fn main(){}",
        ] {
            assert!(
                parse(source).and_then(|p| check(&p)).is_err(),
                "accepted {source}"
            );
        }
    }
    #[test]
    fn trait_scope_inherent_precedence_and_predicate_limits_stay_explicit() {
        for source in [
            "mod m{pub struct S{pub x:i32}pub trait V{fn v(self)->i32;}impl V for S{fn v(self)->i32{self.x}}}use m::V;fn main(){let n=m::S{x:1}.v();}",
            "struct S{x:i32}trait A{fn v(self)->i32;}trait B{fn v(self)->i32;}impl A for S{fn v(self)->i32{1}}impl B for S{fn v(self)->i32{2}}impl S{fn v(self)->i32{3}}fn main(){let n=S{x:1}.v();}",
        ] {
            check(&parse(source).unwrap()).unwrap();
        }
        for source in [
            "mod m{pub struct S{pub x:i32}pub trait V{fn v(self)->i32;}impl V for S{fn v(self)->i32{self.x}}}fn main(){let n=m::S{x:1}.v();}",
            "struct S{x:i32}trait A{fn v(self)->i32;}trait B{fn v(self)->i32;}impl A for S{fn v(self)->i32{1}}impl B for S{fn v(self)->i32{2}}fn main(){let n=S{x:1}.v();}",
        ] {
            assert!(parse(source).and_then(|p| check(&p)).is_err());
        }
        let bounds = std::iter::repeat_n("V", 257).collect::<Vec<_>>().join("+");
        assert!(
            parse(&format!(
                "trait V{{fn v(self)->i32;}}fn f<T:{bounds}>(x:T){{}}fn main(){{}}"
            ))
            .is_err()
        );
    }
    #[test]
    fn generic_paths_are_structured_and_binders_are_rigid() {
        let p=parse("struct W<T>{v:T}fn id<T>(x:T)->T{x}fn forward<U>(x:U)->U{id::<U>(x)}fn shadow<i32>(x:i32)->i32{x}fn main(){let w:W<W<u8>>=W::<W<u8>>{v:W::<u8>{v:7}};let n=1+id::<u8>(w.v.v);}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&p, &budget).unwrap();
        let a = plan
            .function_schema(plan.function_decl(0).unwrap())
            .unwrap();
        let b = plan
            .function_schema(plan.function_decl(1).unwrap())
            .unwrap();
        assert_ne!(a.signature.params[0], b.signature.params[0]);
        assert!(matches!(a.signature.params[0], ir::Ty::Param(_)));
        assert!(matches!(p.functions[2].params[0].1, Type::Param(_)));
        for index in 0..p.functions.len() {
            type_function_with_plan(&p, index, &plan, &budget).unwrap();
        }
        check(&p).unwrap();
    }

    #[test]
    fn associated_callable_registry_preserves_impl_method_binders_and_self() {
        let p=parse("struct W<T>{x:T}impl<T> W<T>{fn make(x:T)->Self{Self{x}}fn pair<U>(self,u:U)->(T,U){(self.x,u)}fn forward<U>(self,u:U)->(T,U){self.pair::<U>(u)}}fn main(){let p=W::<i32>::make(3).forward::<u8>(4u8);}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&p, &budget).unwrap();
        assert_eq!(p.functions.len(), 1);
        assert_eq!(
            (0..plan.callable_count())
                .filter(|&i| matches!(
                    plan.callable_provider(plan.callable_decl(i).unwrap())
                        .unwrap(),
                    CallableProvider::Source { .. }
                ))
                .count(),
            4
        );
        for index in 0..plan.callable_count() {
            let decl = plan.callable_decl(index).unwrap();
            assert_eq!(plan.callable_index(decl).unwrap(), index);
            let f = type_callable_with_plan(&p, decl, &plan, &budget).unwrap();
            if !matches!(
                plan.callable_provider(decl).unwrap(),
                CallableProvider::Source { .. }
            ) {
                continue;
            }
            let context = plan.callable_context(decl).unwrap();
            if context.receiver != ReceiverKind::None {
                assert_eq!(
                    f.params.len(),
                    plan.callable_body(&p, decl).unwrap().params.len() + 1
                );
                assert_eq!(f.locals[f.params[0].0].name, "self");
            }
            if plan.callable_body(&p, decl).unwrap().name == "pair" {
                let env = plan.callable_env(decl).unwrap();
                assert_eq!(env.scopes().len(), 2);
                assert_eq!(env.total_arity(), 2);
                assert_ne!(env.scopes()[0].binder(), env.scopes()[1].binder());
            }
        }
        check(&p).unwrap();
        let singleton = parse("enum E{Item(i32)}impl E{fn take(self)->i32{let Self::Item(x)=self;x}}fn main(){let n=E::Item(7).take();}").unwrap();
        check(&singleton).unwrap();
    }

    #[test]
    fn inherent_coherence_and_definition_privacy_do_not_depend_on_demand() {
        for source in [
            "struct W<T>{x:T}impl W<u8>{fn take(self)->u8{self.x}}impl W<u16>{fn take(self)->u16{self.x}}fn main(){let a=W::<u8>{x:1u8}.take();}",
            "struct P<T,U>{a:T,b:U}impl<T>P<T,T>{fn score(self)->i32{1}}impl P<i32,u8>{fn score(self)->i32{2}}fn main(){}",
        ] {
            check(&parse(source).unwrap()).unwrap();
        }
        for source in [
            "struct W<T>{x:T}impl<T> W<T>{fn clash(self){}}impl W<i32>{fn clash(self){}}fn main(){}",
            "struct W<T>{x:T}impl<T> W<T>{fn bad<U>(self,u:U)->(U,U){(u,u)}}fn main(){}",
            "struct S{x:i32}impl<T>S{fn unused(self){}}fn main(){}",
            "mod a{pub struct S{x:i32}}mod b{impl super::a::S{pub fn leak(self)->i32{self.x}}}fn main(){}",
            "mod a{pub struct S{pub x:i32}impl S{fn take(self)->i32{self.x}}}fn main(){let n=a::S{x:1}.take();}",
        ] {
            assert!(
                parse(source).and_then(|p| check(&p)).is_err(),
                "accepted {source}"
            );
        }
    }

    #[test]
    fn associated_receivers_are_checked_once_and_owning_calls_stay_direct() {
        let p=parse("struct S{x:i32}impl S{fn add(mut self,n:i32)->i32{self.x=self.x+n;self.x}}fn make()->S{S{x:3}}fn arg()->i32{4}fn main(){let n=make().add(arg());}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&p, &budget).unwrap();
        let decl = plan.callable_decl(2).unwrap();
        let function = type_callable_with_plan(&p, decl, &plan, &budget).unwrap();
        let calls = function
            .exprs
            .iter()
            .filter_map(|e| {
                if let ir::ExprKind::Call { target, args, .. } = &e.kind {
                    Some((target, args))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(calls.len(), 3);
        assert_eq!(
            calls
                .iter()
                .filter(
                    |(target, _)|matches!(target,ir::Callee::Direct(t) if plan.declaration(t.declaration).unwrap().path.item=="make")
                )
                .count(),
            1
        );
        let (target, args) = calls.last().unwrap();
        assert_eq!(args.len(), 2);
        assert_eq!(
            plan.declaration(match target {
                ir::Callee::Direct(t) => t.declaration,
                _ => panic!("expected direct call"),
            })
            .unwrap()
            .kind,
            ir::DeclKind::AssociatedFunction
        );
        check(&p).unwrap();
        for source in [
            "struct S{x:i32}impl S{fn take(self)->i32{self.x}}fn main(){let s=S{x:1};let n=s.take();let x=s.x;}",
            "struct S{x:i32}impl S{fn choose<U>(self,u:U)->U{u}}fn main(){let n=S{x:1}.choose(2);}",
        ] {
            assert!(parse(source).and_then(|p| check(&p)).is_err());
        }
    }

    #[test]
    fn omitted_generic_inference_uses_all_evidence_before_integer_fallback() {
        let p = parse("struct W<T>{v:T}enum E<T>{A(T),B}fn id<T>(x:T)->T{x}fn same<T>(x:T,y:T)->T{y}fn main(){let a=id(1);let b:u8=id(2);let c=same(3,4u8);let d=id(1)+2u8;let e=1+id(2u8);let p=id((1,2));let z:[u8;0]=id([]);let w=W{v:7u16};let en=E::A(8u8);let explicit=id::<i32>(id(1));let cast=id(256) as u8;}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&p, &budget).unwrap();
        let function = type_function_with_plan(&p, 2, &plan, &budget).unwrap();
        let local = |name: &str| {
            function
                .locals
                .iter()
                .find(|l| l.name == name)
                .unwrap()
                .ty
                .clone()
        };
        assert_eq!(local("a"), ir::Ty::Int(ir::IntTy::I32));
        for name in ["b", "c", "d", "e", "cast"] {
            assert_eq!(local(name), ir::Ty::Int(ir::IntTy::U8));
        }
        assert_eq!(
            local("p"),
            ir::Ty::Tuple(vec![ir::Ty::Int(ir::IntTy::I32); 2].into())
        );
        assert_eq!(
            local("z"),
            ir::Ty::Array {
                element: Arc::new(ir::Ty::Int(ir::IntTy::U8)),
                len: 0
            }
        );
        assert!(
            matches!(local("w"), ir::Ty::Struct(t) if t.def().type_args.as_ref() == [ir::Ty::Int(ir::IntTy::U16)])
        );
        assert!(
            matches!(local("en"), ir::Ty::Enum(t) if t.def().type_args.as_ref() == [ir::Ty::Int(ir::IntTy::U8)])
        );
        check(&p).unwrap();
    }

    #[test]
    fn omitted_generic_inference_keeps_opaque_sites_and_universal_binders_rigid() {
        for source in [
            "fn id<T>(x:T)->T{x}fn main(){let x:u8=id({let a=1;2});}",
            "fn id<T>(x:T)->T{x}fn same<T>(x:T,y:T)->T{y}fn main(){let x=same(id(1),2u8);}",
            "fn f<T>(x:T,n:i32)->T{if n==0{x}else{f(x,n-1)}}fn main(){let x=f(1u8,1);}",
        ] {
            check(&parse(source).unwrap()).unwrap();
        }
        for source in [
            "fn id<T>(x:T)->T{x}fn main(){let x=id({let a=1;2});}",
            "fn id<T>(x:T)->T{x}fn main(){let x=id(id(1));}",
        ] {
            let error = parse(source).and_then(|p| check(&p)).unwrap_err();
            assert!(error.contains("unsupported generic inference"), "{error}");
        }
        for source in [
            "fn same<T>(x:T,y:T)->T{y}fn main(){let x=same(1u8,2u16);}",
            "fn bad<T>(x:T)->(T,T){(x,x)}fn main(){}",
        ] {
            assert!(parse(source).and_then(|p| check(&p)).is_err());
        }
    }

    #[test]
    fn omitted_generic_hints_do_not_commit_numeric_defaults_or_expression_effects() {
        let p = parse("fn id<T>(x:T)->T{x}fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&p, &budget).unwrap();
        let signatures = plan
            .environment(Some(plan.function_decl(1).unwrap()))
            .unwrap();
        let checker = Checker::new(&signatures, ir::Ty::Unit, String::new());
        let call = |suffix| Expr::Call {
            name: "id".into(),
            args: vec![Expr::Integer(IntegerLiteral {
                magnitude: 1,
                suffix,
            })],
        };
        let untyped = call(None);
        let typed = call(Some(ir::IntTy::U8));
        assert_eq!(checker.scalar_hint(&untyped).unwrap(), None);
        assert_eq!(
            checker.scalar_hint(&typed).unwrap(),
            Some(ir::Ty::Int(ir::IntTy::U8))
        );
        assert!(checker.locals.is_empty());
        assert!(checker.exprs.is_empty());
        assert!(checker.reachable);
        assert_eq!(checker.scalar_hint(&untyped).unwrap(), None);
    }

    #[test]
    fn generic_all_body_semantics_do_not_specialize_away_invalid_templates() {
        for source in [
            "fn bad<T>(x:T)->(T,T){(x,x)}fn main(){}",
            "fn bad<T>(x:T)->(T,T){(x,x)}fn main(){let n=bad::<i32>(7);}",
            "fn bad<T>(x:T)->T{x+1}fn main(){}",
            "fn bad<T>(x:T)->T{7i32}fn main(){}",
            "fn id<T>(x:T)->T{x}fn main(){let n=id::<i32,u8>(1);}",
            "struct W<T>{v:T}fn main(){let x=1;let w=W::<&i32>{v:&x};}",
            "fn bad<T>(x:T){return;let r=&x;}fn main(){}",
        ] {
            if source.contains("return;let r=&x;") {
                check(&parse(source).unwrap()).unwrap();
                continue;
            }
            assert!(
                parse(source).and_then(|p| check(&p)).is_err(),
                "accepted {source}"
            );
        }
        check(&parse("struct P<T>{a:T,b:T}fn split<T>(p:P<T>)->(T,T){let a=p.a;let b=p.b;(a,b)}fn main(){let p=split::<u8>(P::<u8>{a:3,b:4});}").unwrap()).unwrap();
    }

    #[test]
    fn generic_nominal_applications_are_canonical_and_share_resource_authority() {
        let p = parse("struct W<T>{v:T}enum E<T>{A(T),B}fn id<T>(x:T)->T{x}fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&p, &budget).unwrap();
        let id = *plan.registry.nominals.get("W").unwrap();
        let a = plan
            .intern_nominal(
                id,
                &[ir::Ty::Int(ir::IntTy::U8)],
                ir::FactsCtx::Concrete,
                &budget,
            )
            .unwrap();
        let b = plan
            .clone()
            .intern_nominal(
                id,
                &[ir::Ty::Int(ir::IntTy::U8)],
                ir::FactsCtx::Concrete,
                &budget.clone(),
            )
            .unwrap();
        let c = plan
            .intern_nominal(
                id,
                &[ir::Ty::Int(ir::IntTy::I32)],
                ir::FactsCtx::Concrete,
                &budget,
            )
            .unwrap();
        assert_eq!(a, b);
        assert!(
            plan.intern_nominal(
                id,
                &[ir::Ty::Int(ir::IntTy::U8)],
                ir::FactsCtx::Concrete,
                &ir::PlanBudget::default()
            )
            .unwrap_err()
            .contains("authority")
        );
        assert_ne!(a, c);
        let foreign = ir::ParamId {
            owner: ir::BinderId {
                declaration: plan.function_decl(0).unwrap(),
            },
            index: 0,
        };
        assert!(
            plan.intern_nominal(
                id,
                &[ir::Ty::Param(foreign)],
                ir::FactsCtx::Concrete,
                &budget
            )
            .is_err()
        );
        let limited = ir::PlanBudget::new(ir::PlanLimits {
            applications: 0,
            ..Default::default()
        });
        assert!(
            PlanCtx::from_program(&p, &limited)
                .unwrap_err()
                .contains("resource")
        );
        for source in [
            "struct Bad<T>{x:i32}fn main(){}",
            "struct Bad<T>{x:Bad<(T,)>}fn main(){}",
        ] {
            assert!(parse(source).and_then(|p| type_env(&p)).is_err());
        }
    }

    #[test]
    fn compound_statement_tokens_types_and_expression_boundaries() {
        for name in [
            "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize",
        ] {
            let source = format!(
                "fn main(){{let mut x:{name}=4;x+=1;x-=1;x*=2;x/=2;x%=3;x&=3;x|=1;x^=2;x<<=1u64;x>>=1i8;}}"
            );
            let p = parse(&source).unwrap();
            check(&p).unwrap();
            let f = type_function_with_signatures(&p, 0, &signature_table(&p).unwrap()).unwrap();
            assert_eq!(
                f.body
                    .statements
                    .iter()
                    .filter(|s| matches!(s, ir::Stmt::AssignOp { .. }))
                    .count(),
                10
            );
        }
        check(&parse("fn main(){let mut b=true;b&=false;b|=true;b^=true;let mut a=[1u8];let i=0usize;a[i]+=2;}").unwrap()).unwrap();
        for source in [
            "fn main(){let mut x=1;let y=(x+=1);}",
            "fn main(){let mut x=1;x \"+=\" 1;}",
            "fn main(){let mut x=1u8;x+=1u16;}",
            "fn main(){let mut x=true;x+=false;}",
            "fn main(){let mut x=1;x<<=true;}",
            "fn main(){let mut x=(1,2);x+=x;}",
        ] {
            assert!(
                parse(source).and_then(|p| check(&p)).is_err(),
                "accepted {source}"
            );
        }
        let mut p = parse("fn main(){let mut x=1;x+=2;}").unwrap();
        let Stmt::AssignOp { op, .. } = &mut p.functions[0].body.statements[1] else {
            panic!("compound root place")
        };
        *op = BinaryOp::Eq;
        assert!(check(&p).unwrap_err().contains("operator"));
    }

    #[test]
    fn compound_current_place_rhs_initialization_and_permission_checks() {
        for source in [
            "fn main(){let mut x:i32;x+={x=10;1};}",
            "fn main(){let mut x=1;let mut y=10;let mut p=&mut x;*p+={p=&mut y;2};}",
            "fn main(){let mut a:[i32;1];let i=0usize;a[i]+={a=[10];2};}",
            "fn main(){let mut a=[1];a[{a=[10];0usize}]+={a=[3];2};}",
            "fn main(){let mut x=1;return;x+=1;}",
        ] {
            check(&parse(source).unwrap()).unwrap();
        }
        for source in [
            "fn main(){let mut x:i32;x+=1;}",
            "fn main(){let x=1;x+=1;}",
            "fn main(){let x=1;let r=&x;*r+=1;}",
            "fn main(){let a=[1];let i=0usize;a[i]+=1;}",
            "fn main(){let mut x=1;let r=&x;x+=1;let y=*r;}",
        ] {
            assert!(
                parse(source).and_then(|p| check(&p)).is_err(),
                "accepted {source}"
            );
        }
        let p =
            parse("fn rhs()->i32{1}fn idx()->usize{0}fn main(){let mut a=[1];a[idx()]+=rhs();}")
                .unwrap();
        let f = type_function_with_signatures(&p, 2, &signature_table(&p).unwrap()).unwrap();
        assert_eq!(
            f.exprs
                .iter()
                .filter(|e| matches!(e.kind, ir::ExprKind::Call { .. }))
                .count(),
            2
        );
        assert!(
            f.body
                .statements
                .iter()
                .any(|s| matches!(s, ir::Stmt::AssignIndexOp { .. }))
        );
    }

    #[test]
    fn indexed_compound_autodereferences_mutable_array_carriers() {
        for source in [
            "fn shift(a:&mut [u8;2],i:usize,n:u128){a[i]<<=n;}fn main(){}",
            "fn shift(a:&mut [u8;2],i:usize,n:u128){(*a)[i]>>=n;}fn main(){}",
            "fn shift(a:&mut ([u8;2],),i:usize,n:u128){a.0[i]<<=n;}fn main(){}",
            "fn shift(a:&mut [u128;2],i:usize,n:u128){a[i]+=n;}fn main(){}",
            "fn main(){let mut a=[1u8,2u8];let r=&mut a;let i=0usize;r[i]<<=1u128;}",
        ] {
            check(&parse(source).unwrap()).unwrap();
        }
        for source in [
            "fn shift(a:&[u8;2],i:usize,n:u128){a[i]<<=n;}fn main(){}",
            "fn shift(mut a:&[u8;2],i:usize,n:u128){a[i]<<=n;}fn main(){}",
            "fn shift(a:&[u8;2],i:usize,n:u128){(*a)[i]<<=n;}fn main(){}",
            "fn main(){let a=[1u8,2u8];let r=&a;let i=0usize;r[i]<<=1u128;}",
            "fn main(){let mut a=[1u8,2u8];let r=&mut a;let s=&*r;let i=0usize;r[i]<<=1u128;let x=*s;}",
        ] {
            assert!(parse(source).and_then(|p| check(&p)).is_err(), "{source}");
        }
        let p = parse("fn rhs()->u128{1}fn idx()->usize{0}fn shift(a:&mut [u8;2]){a[idx()]<<=rhs();}fn main(){}").unwrap();
        let f = type_function_with_signatures(&p, 2, &signature_table(&p).unwrap()).unwrap();
        let ir::Stmt::AssignIndexOp {
            array,
            index,
            value,
            ..
        } = &f.body.statements[0]
        else {
            panic!("expected indexed compound assignment")
        };
        assert_eq!(array.projection, [ir::Projection::Deref]);
        assert!(value.0 < index.0, "RHS must be typed before index");
        assert_eq!(
            f.exprs
                .iter()
                .filter(|e| matches!(e.kind, ir::ExprKind::Call { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn compound_shift_queries_share_count_memo_and_check_dead_statements() {
        for source in [
            "fn main(){let mut x=1u8;x<<=8u64;}",
            "fn main(){let mut x=1u8;x>>=-1i16;}",
            "fn main(){let mut a=[1u8];let i=0usize;a[i]<<=3u16+5u16;}",
            "fn main(){let mut x=1u8;return;x<<=8u64;}",
        ] {
            assert!(
                parse(source).and_then(|p| check(&p)).is_err(),
                "accepted {source}"
            );
        }
        let p = parse("fn main(){let mut x=1u8;let n=8u64;x<<=n;x>>={x=3;1u32};let y=1u8<<1u64;}")
            .unwrap();
        check(&p).unwrap();
        let f = type_function_with_signatures(&p, 0, &signature_table(&p).unwrap()).unwrap();
        let ir::Stmt::AssignOp { value, .. } = f.body.statements[2] else {
            panic!("compound shift")
        };
        assert_eq!(f.exprs[value.0].ty, ir::Ty::Int(ir::IntTy::U64));
        let literal = ir::TypedExpr {
            kind: ir::ExprKind::Int(ir::IntValue::i32(0)),
            ty: ir::Ty::Int(ir::IntTy::I32),
            completion: ir::Completion::Returns,
        };
        let queries = vec![(ir::IntTy::U8, ir::ExprId(0)); COUNT_EVAL_WORK / 2 + 1];
        assert!(
            validate_shift_counts(&[literal], &queries)
                .unwrap_err()
                .contains("work")
        );
    }

    #[test]
    fn shifts_keep_independent_integer_operands_and_parser_precedence() {
        let names = [
            "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize",
        ];
        for lhs in names {
            for rhs in names {
                let source = format!("fn f(x:{lhs},n:{rhs})->{lhs}{{(x<<n)>>n}}fn main(){{}}");
                let p = parse(&source).unwrap();
                check(&p).unwrap();
                let f =
                    type_function_with_signatures(&p, 0, &signature_table(&p).unwrap()).unwrap();
                for expr in &f.exprs {
                    if let ir::ExprKind::Binary {
                        op: BinaryOp::Shl | BinaryOp::Shr,
                        left,
                        right,
                    } = expr.kind
                    {
                        assert_eq!(expr.ty, f.exprs[left.0].ty);
                        assert_eq!(f.exprs[right.0].ty, f.locals[f.params[1].0].ty);
                    }
                }
            }
        }
        let p = parse("fn f()->u8{1<<1u64}fn g(x:u8)->u8{x<<1}fn main(){let x=1<<1u64;let y=(1i16 as u8)<<1;let z=4i32 as u8>>1;let p=1 | 2 & 3 << 4+1*2;}").unwrap();
        check(&p).unwrap();
        let env = signature_table(&p).unwrap();
        let g = type_function_with_signatures(&p, 1, &env).unwrap();
        let ir::ExprKind::Binary { right, .. } = g.exprs[g.body.tail.unwrap().0].kind else {
            panic!("shift")
        };
        assert_eq!(g.exprs[right.0].ty, ir::Ty::Int(ir::IntTy::I32));
        let main = type_function_with_signatures(&p, 2, &env).unwrap();
        assert_eq!(main.locals[0].ty, ir::Ty::Int(ir::IntTy::I32));
        for source in [
            "fn main(){let x=true<<1;}",
            "fn main(){let x=1>>false;}",
            "fn main(){let x=1i32 as u8<<1;}",
            "fn main(){let x=1 \"<<\" 1;}",
        ] {
            assert!(parse(source).and_then(|p| check(&p)).is_err());
        }
        for op in ["<<=", ">>="] {
            check(&parse(&format!("fn main(){{let mut x=1;x{op}1;}} ")).unwrap()).unwrap();
        }
    }

    #[test]
    fn shifts_static_count_grammar_and_runtime_leaves_are_distinct() {
        for count in [
            "8u8",
            "-1i16",
            "(3u16+5u16)",
            "(1u64<<3u8)",
            "(!0u8)",
            "((-1i8) as u64)",
            "{8u32}",
            "(255u8+1u8)",
            "(1u8/0u8)",
            "((-128i8)/(-1i8))",
        ] {
            let source = format!("fn main(){{let x=1u8<<({count});}}");
            assert!(
                parse(&source).and_then(|p| check(&p)).is_err(),
                "accepted {count}"
            );
        }
        check(&parse("fn count()->u64{64}fn f(x:u8,n:u64)->u8{x<<n}fn main(){let n=8u64;let x=1u8<<n;let y=1u64>>count();let z=1u8<<((3u16^1u16) as u64);let a=1u64<<((18446744073709551615u64*0u64) as u8);}").unwrap()).unwrap();
        assert!(
            parse("fn main(){let x=1u8<<(if true{1}else{2});}")
                .and_then(|p| check(&p))
                .unwrap_err()
                .contains("unsupported")
        );
        check(&parse("fn main(){let mut bits=3u8;let snapshot=bits<<{bits=9u8;1u32};}").unwrap())
            .unwrap();
        let p =
            parse("fn dead()->u8{1u8<<({return 7u8;1u32})}fn once(n:u8)->u8{1u8<<n}fn main(){}")
                .unwrap();
        check(&p).unwrap();
        let f = type_function_with_signatures(&p, 0, &signature_table(&p).unwrap()).unwrap();
        assert_eq!(
            f.exprs[f.body.tail.unwrap().0].completion,
            ir::Completion::Never
        );
    }

    #[test]
    fn shift_count_memo_bounds_cycles_and_never_values() {
        let literal = ir::TypedExpr {
            kind: ir::ExprKind::Int(ir::IntValue::i32(0)),
            ty: ir::Ty::Int(ir::IntTy::I32),
            completion: ir::Completion::Returns,
        };
        let mut arena = vec![literal.clone()];
        for index in 0..100 {
            arena.push(ir::TypedExpr {
                kind: ir::ExprKind::Binary {
                    op: BinaryOp::BitOr,
                    left: ir::ExprId(index),
                    right: ir::ExprId(index),
                },
                ..literal.clone()
            });
        }
        let mut eval = CountEvaluator::new(&arena).unwrap();
        assert!(matches!(
            eval.evaluate(ir::ExprId(100)).unwrap(),
            CountValue::Known(_)
        ));
        assert!(eval.work < 1000);
        eval.work = COUNT_EVAL_WORK;
        assert!(eval.evaluate(ir::ExprId(0)).unwrap_err().contains("work"));
        let mut deep = vec![literal.clone()];
        for index in 0..130 {
            deep.push(ir::TypedExpr {
                kind: ir::ExprKind::IntCast {
                    value: ir::ExprId(index),
                    to: ir::IntTy::I32,
                },
                ..literal.clone()
            });
        }
        let mut eval = CountEvaluator::new(&deep).unwrap();
        eval.evaluate(ir::ExprId(100)).unwrap();
        assert!(
            eval.evaluate(ir::ExprId(130))
                .unwrap_err()
                .contains("depth")
        );
        let cyclic = [ir::TypedExpr {
            kind: ir::ExprKind::IntCast {
                value: ir::ExprId(0),
                to: ir::IntTy::I32,
            },
            ..literal.clone()
        }];
        assert!(
            CountEvaluator::new(&cyclic)
                .unwrap()
                .evaluate(ir::ExprId(0))
                .unwrap_err()
                .contains("cycle")
        );
        let never = [ir::TypedExpr {
            completion: ir::Completion::Never,
            ..literal
        }];
        assert!(matches!(
            CountEvaluator::new(&never)
                .unwrap()
                .evaluate(ir::ExprId(0))
                .unwrap(),
            CountValue::Dynamic
        ));
    }

    #[test]
    fn bitwise_exact_integer_and_boolean_types_preserve_operand_hints() {
        for name in [
            "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize",
        ] {
            let source = format!("fn f(x:{name})->{name}{{(x & 3) | (1 ^ x)}}fn main(){{}}");
            let p = parse(&source).unwrap();
            check(&p).unwrap();
            let f = type_function_with_signatures(&p, 0, &signature_table(&p).unwrap()).unwrap();
            for expr in &f.exprs {
                if matches!(
                    expr.kind,
                    ir::ExprKind::Binary {
                        op: BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor,
                        ..
                    }
                ) {
                    assert_eq!(expr.ty, f.result);
                }
            }
        }
        check(&parse("fn f(x:bool)->bool{(x & true) | (false ^ x)}fn main(){let x=(1i8<2i8)&(3i16<4i16);let y=(200+100)|1;let z=((200+100)|1) as u8;}").unwrap()).unwrap();
        for source in [
            "fn main(){let x=1u8 & 2u16;}",
            "fn main(){let x=true | 1;}",
            "fn main(){let x=() ^ ();}",
            "fn main(){let x=(1,2) & (3,4);}",
            "fn main(){let x=1;let r=&x;let z=r|r;}",
            "fn main(){let x:u8=256 | 0;}",
        ] {
            assert!(
                parse(source).and_then(|p| check(&p)).is_err(),
                "accepted {source}"
            );
        }
    }

    #[test]
    fn bitwise_pratt_precedence_and_compound_boundaries() {
        let p = parse("fn main(){let x=1 | 2 ^ 3 & 4 + 5 * 6i16 as i32;}").unwrap();
        let Stmt::Let {
            value:
                Expr::Binary {
                    op: BinaryOp::BitOr,
                    right,
                    ..
                },
            ..
        } = &p.functions[0].body.statements[0]
        else {
            panic!("OR precedence")
        };
        let Expr::Binary {
            op: BinaryOp::BitXor,
            right,
            ..
        } = right.as_ref()
        else {
            panic!("XOR precedence")
        };
        let Expr::Binary {
            op: BinaryOp::BitAnd,
            right,
            ..
        } = right.as_ref()
        else {
            panic!("AND precedence")
        };
        let Expr::Binary {
            op: BinaryOp::Add,
            right,
            ..
        } = right.as_ref()
        else {
            panic!("addition precedence")
        };
        let Expr::Binary {
            op: BinaryOp::Mul,
            right,
            ..
        } = right.as_ref()
        else {
            panic!("multiplication precedence")
        };
        assert!(matches!(right.as_ref(), Expr::Cast { .. }));
        check(&parse("fn main(){let x=1i32;let r=&x;let a=*r & -1 | !0 ^ 3;let b=1|2|3;let c=1|2==3;let d=true || false & true && false;}").unwrap()).unwrap();
        for op in ["&=", "|=", "^="] {
            let source = format!("fn main(){{let mut x=1;x{op}2;}}");
            check(&parse(&source).unwrap()).unwrap();
        }
        for source in [
            "fn main(){let x=1 \"|\" 2;}",
            "fn main(){let x=1 \"^\" 2;}",
            "fn main(){let x=1<2<3;}",
        ] {
            assert!(parse(source).is_err());
        }
    }

    #[test]
    fn bitwise_eager_completion_and_once_only_operands() {
        let p = parse("fn source()->bool{true}fn f()->bool{source() & source()}fn dead()->bool{false & {return true;false}}fn main(){}").unwrap();
        check(&p).unwrap();
        let env = signature_table(&p).unwrap();
        let f = type_function_with_signatures(&p, 1, &env).unwrap();
        assert_eq!(
            f.exprs
                .iter()
                .filter(|e| matches!(e.kind, ir::ExprKind::Call { .. }))
                .count(),
            2
        );
        let dead = type_function_with_signatures(&p, 2, &env).unwrap();
        assert_eq!(
            dead.exprs[dead.body.tail.unwrap().0].completion,
            ir::Completion::Never
        );
        // An eager RHS read is checked even when the left Boolean decides the value.
        assert!(check(&parse("fn main(){let x:bool;let y=false & x;}").unwrap()).is_err());
        assert!(check(&parse("fn main(){let x:bool;let y=true | x;}").unwrap()).is_err());
    }

    #[test]
    fn cast_soft_expectations_preserve_concrete_sources_and_drop_at_operators() {
        use ir::IntTy::*;
        for (body, source_ty) in [
            ("300i32 as u8", I32),
            ("{let x=300;x} as u8", I32),
            ("(200+100) as u8", I32),
            ("(if true{300}else{301}) as u8", I32),
            ("(match true{true=>300,false=>301}) as u8", I32),
            ("(loop{break 300;}) as u8", I32),
            ("!0 as u8", U8),
            ("-128i8 as u8", I8),
        ] {
            // Parenthesize the outer operand when the body begins a naked block.
            let body = if body.starts_with('{') {
                format!("({body})")
            } else {
                body.to_string()
            };
            let source = format!("fn f()->u8{{{body}}}fn main(){{}}");
            let p = parse(&source).unwrap();
            check(&p).unwrap();
            let env = signature_table(&p).unwrap();
            let f = type_function_with_signatures(&p, 0, &env).unwrap();
            let ir::ExprKind::IntCast { value, to } = f.exprs[f.body.tail.unwrap().0].kind else {
                panic!("not a cast: {source}")
            };
            assert_eq!(to, U8);
            assert_eq!(f.exprs[value.0].ty, ir::Ty::Int(source_ty), "{source}");
        }
        for source in [
            "fn main(){let x=300 as u8;}",
            "fn main(){let x=({300}) as u8;}",
            "fn main(){let x=(-1) as u8;}",
            "fn main(){let x=300 as u8 as u16;}",
            "fn main(){let x=(1u8+300i32) as u16;}",
        ] {
            assert!(
                parse(source).and_then(|p| check(&p)).is_err(),
                "accepted {source}"
            );
        }
        check(&parse("fn wide()->u64{18446744073709551615 as u64}fn f()->u16{(-1i8) as u16}fn main(){let x:u16=300i32 as u8 as u16;}").unwrap()).unwrap();
    }

    #[test]
    fn cast_parser_preserves_precedence_statement_boundaries_and_typed_tokens() {
        let p=parse("fn main(){let x=-1i16 as u8*2;let y=1i32 as u8 as u16;let z=1+2i32 as u8;let b=(1i32 as u8)<2;}").unwrap();
        let Stmt::Let {
            value:
                Expr::Binary {
                    op: BinaryOp::Mul,
                    left,
                    ..
                },
            ..
        } = &p.functions[0].body.statements[0]
        else {
            panic!("multiplication precedence")
        };
        assert!(
            matches!(left.as_ref(),Expr::Cast{value,..} if matches!(value.as_ref(),Expr::Unary{op:UnaryOp::Neg,..}))
        );
        let Stmt::Let {
            value: Expr::Cast { value, .. },
            ..
        } = &p.functions[0].body.statements[1]
        else {
            panic!("cast chain")
        };
        assert!(matches!(value.as_ref(), Expr::Cast { .. }));
        let Stmt::Let {
            value:
                Expr::Binary {
                    op: BinaryOp::Add,
                    right,
                    ..
                },
            ..
        } = &p.functions[0].body.statements[2]
        else {
            panic!("addition precedence")
        };
        assert!(matches!(right.as_ref(), Expr::Cast { .. }));
        for source in [
            "fn main(){let x=1 \"as\" u8;}",
            "fn main(){let x=1 as \"u8\";}",
            "fn main(){let x:\"u8\"=1;}",
            "fn main(){let x=1i32 as u8<2;}",
            "fn main(){let x=1i32 as u8.0;}",
            "fn main(){let x=1i32 as u8[0];}",
            "fn main(){let x=1i32 as u8();}",
            "fn f()->u8{{1} as u8}fn main(){}",
            "fn f()->u8{if true{1}else{2} as u8}fn main(){}",
            "fn f()->u8{match true{_=>1} as u8}fn main(){}",
        ] {
            assert!(parse(source).is_err(), "accepted {source}");
        }
        assert!(
            parse("fn main(){let x=1i32 as (u8);}")
                .unwrap_err()
                .contains("unsupported")
        );
        check(&parse("fn f()->u8{({1}) as u8}fn g()->u8{(if true{1}else{2}) as u8}fn main(){let b=(1i32 as u8)<2;}").unwrap()).unwrap();
    }

    #[test]
    fn cast_completion_and_once_only_operands_do_not_create_values_or_origins() {
        let p=parse("fn source()->i64{300}fn once()->u8{source() as u8}fn typed_dead()->u8{({return 1u8;300i32}) as u8}fn read(r:&i16)->u8{*r as u8}fn main(){}").unwrap();
        check(&p).unwrap();
        let env = signature_table(&p).unwrap();
        let once = type_function_with_signatures(&p, 1, &env).unwrap();
        assert_eq!(
            once.exprs
                .iter()
                .filter(|e| matches!(e.kind, ir::ExprKind::Call { .. }))
                .count(),
            1
        );
        let dead = type_function_with_signatures(&p, 2, &env).unwrap();
        let cast = &dead.exprs[dead.body.tail.unwrap().0];
        assert_eq!(cast.ty, ir::Ty::Int(ir::IntTy::U8));
        assert_eq!(cast.completion, ir::Completion::Never);
        let ir::ExprKind::IntCast { value, .. } = cast.kind else {
            panic!("expected typed dead cast")
        };
        assert_eq!(dead.exprs[value.0].ty, ir::Ty::Int(ir::IntTy::I32));
        assert_eq!(dead.exprs[value.0].completion, ir::Completion::Never);
        for source in [
            "fn main(){let x=(loop{}) as u8;}",
            "fn main(){let x=true as u8;}",
            "fn main(){let x=1i32;let r=&x;let n=r as usize;}",
        ] {
            assert!(
                parse(source)
                    .and_then(|p| check(&p))
                    .unwrap_err()
                    .contains("unsupported")
            );
        }
        let within = format!("fn main(){{let x=1i32{};}}", " as u8".repeat(120));
        check(&parse(&within).unwrap()).unwrap();
        let source = format!("fn main(){{let x=1i32{};}}", " as u8".repeat(140));
        assert!(parse(&source).unwrap_err().contains("nesting"));
    }

    #[test]
    fn wide_count_evaluation_preserves_sign_high_limbs_and_checked_bounds() {
        use ir::IntTy::{I8, I128, U8, U64, U128};
        let int = |ty, bits| ir::IntValue { ty, bits };
        let known = |op, left, right| {
            let CountValue::Known(value) = count_binary(op, left, right).unwrap() else {
                panic!("expected exact static value")
            };
            value
        };
        let min = int(I128, 1u128 << 127);
        let max = int(I128, i128::MAX as u128);
        let umax = int(U128, u128::MAX);
        assert_eq!(count_integer(min), i128::MIN);
        assert_eq!(count_integer(max), i128::MAX);
        assert_eq!(count_cast(int(I8, 255), U128), u128::MAX);
        assert_eq!(count_cast(int(U8, 255), I128), 255);
        assert_eq!(count_cast(umax, U64), u64::MAX as u128);
        assert_eq!(count_checked_signed(i128::MIN, I128).unwrap(), min.bits);
        assert_eq!(count_checked_signed(i128::MAX, I128).unwrap(), max.bits);
        assert!(count_checked_signed(128, I8).is_err());
        assert!(count_checked_signed(-129, I8).is_err());
        assert_eq!(known(BinaryOp::Sub, umax, int(U128, 1)).bits, u128::MAX - 1);
        assert_eq!(
            known(BinaryOp::Add, min, int(I128, 1)).bits,
            (1u128 << 127) + 1
        );
        assert_eq!(known(BinaryOp::Shr, min, int(U128, 127)).bits, u128::MAX);
        assert_eq!(
            known(BinaryOp::Shl, int(U128, 1), int(I128, 127)).bits,
            1u128 << 127
        );
        assert_eq!(
            known(BinaryOp::Div, min, int(I128, 2)).bits,
            (i128::MIN / 2) as u128
        );
        assert_eq!(known(BinaryOp::Rem, umax, int(U128, 2)).bits, 1);
        for (op, lhs, rhs) in [
            (BinaryOp::Add, max, int(I128, 1)),
            (BinaryOp::Sub, min, int(I128, 1)),
            (BinaryOp::Mul, min, int(I128, 2)),
            (BinaryOp::Add, umax, int(U128, 1)),
            (BinaryOp::Sub, int(U128, 0), int(U128, 1)),
            (BinaryOp::Mul, umax, int(U128, 2)),
            (BinaryOp::Div, min, int(I128, u128::MAX)),
            (BinaryOp::Rem, min, int(I128, u128::MAX)),
            (BinaryOp::Div, umax, int(U128, 0)),
            (BinaryOp::Rem, umax, int(U128, 0)),
            (BinaryOp::Shl, int(U128, 1), int(U128, 128)),
            (BinaryOp::Shr, min, int(I128, u128::MAX)),
        ] {
            assert!(
                count_binary(op, lhs, rhs).is_err(),
                "{op:?}: {lhs:?} {rhs:?}"
            );
        }
        for value in [umax, min, int(U128, 1u128 << 64), int(I128, u128::MAX)] {
            assert!(count_shift_amount(value, U8).is_err());
        }
        assert_eq!(count_shift_amount(int(U128, 7), U8).unwrap(), 7);
        let arena = vec![
            ir::TypedExpr {
                ty: ir::Ty::Int(I128),
                completion: ir::Completion::Returns,
                kind: ir::ExprKind::Int(min),
            },
            ir::TypedExpr {
                ty: ir::Ty::Int(I128),
                completion: ir::Completion::Returns,
                kind: ir::ExprKind::Unary {
                    op: UnaryOp::Neg,
                    value: ir::ExprId(0),
                },
            },
        ];
        assert!(
            CountEvaluator::new(&arena)
                .unwrap()
                .evaluate(ir::ExprId(1))
                .is_err()
        );
    }

    #[test]
    fn wide_source_casts_and_static_counts_are_exact() {
        let names = [
            "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize",
        ];
        for from in names {
            for to in names {
                let source = format!("fn f(x:{from})->{to}{{x as {to}}}fn main(){{}}");
                check(&parse(&source).unwrap()).unwrap();
            }
        }
        for integer in ["i128", "u128"] {
            for float in ["f32", "f64"] {
                let source = format!(
                    "fn f(x:{integer})->{float}{{x as {float}}}fn g(x:{float})->{integer}{{x as {integer}}}fn main(){{}}"
                );
                check(&parse(&source).unwrap()).unwrap();
            }
        }
        for count in [
            "127u128",
            "(340282366920938463463374607431768211455u128%128u128)",
            "((340282366920938463463374607431768211455u128*0u128) as u8)",
            "((-170141183460469231731687303715884105728i128>>127u128)&127i128)",
            "(((-1i8) as u128)&127u128)",
            "((18446744073709551616u128 as u64) as i128)",
        ] {
            let source = format!("fn main(){{let x=1u128<<{count};let mut y=1i128;y>>={count};}}");
            check(&parse(&source).unwrap()).unwrap();
        }
        for count in [
            "128u128",
            "18446744073709551616u128",
            "340282366920938463463374607431768211455u128",
            "-170141183460469231731687303715884105728i128",
            "(-(-170141183460469231731687303715884105728i128))",
            "(170141183460469231731687303715884105727i128+1i128)",
            "(-170141183460469231731687303715884105728i128-1i128)",
            "(340282366920938463463374607431768211455u128+1u128)",
            "(340282366920938463463374607431768211455u128*2u128)",
            "((-170141183460469231731687303715884105728i128)/(-1i128))",
            "((-170141183460469231731687303715884105728i128)%(-1i128))",
            "(1u128<<128u128)",
        ] {
            for body in [
                format!("let x=1u128<<{count};"),
                format!("let mut x=1u128;return;x<<={count};"),
                format!("let x=1u8<<{count};"),
            ] {
                let source = format!("fn main(){{{body}}}");
                assert!(parse(&source).and_then(|p| check(&p)).is_err(), "{source}");
            }
        }
    }

    #[test]
    fn wide_literals_and_patterns_keep_all_128_bits() {
        for text in [
            "340_282_366_920_938_463_463_374_607_431_768_211_455u128",
            "0xffff_ffff_ffff_ffff_ffff_ffff_ffff_ffffu128",
            "0b11111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111u128",
            "0o3777777777777777777777777777777777777777777u128",
        ] {
            let literal = parse_integer_literal(text).unwrap();
            assert_eq!(literal.magnitude, u128::MAX, "{text}");
            assert_eq!(
                resolve_integer(&literal, false, None).unwrap().bits,
                u128::MAX
            );
        }
        for text in [
            "340282366920938463463374607431768211456u128",
            "0x100000000000000000000000000000000u128",
            "1u128bad",
            "0b102u128",
        ] {
            assert!(parse_integer_literal(text).is_err(), "{text}");
        }
        for source in [
            "fn main(){let x=170141183460469231731687303715884105728i128;}",
            "fn main(){let x=-170141183460469231731687303715884105729i128;}",
            "fn main(){let x=-1u128;}",
            "fn main(){let x:u64=18446744073709551616;}",
            "fn main(){let x:i128=340282366920938463463374607431768211455;}",
            "fn main(){let x=18446744073709551616;}",
        ] {
            assert!(parse(source).and_then(|p| check(&p)).is_err(), "{source}");
        }
        for source in [
            "fn main(){let x=170141183460469231731687303715884105728i128;}",
            "fn main(){let x=-170141183460469231731687303715884105729i128;}",
            "fn main(){let x=0x80000000000000000000000000000000i128;}",
            "fn main(){let x:i128=170141183460469231731687303715884105728;}",
        ] {
            let error = parse(source).and_then(|p| check(&p)).unwrap_err();
            assert!(error.contains("out of range for i128"), "{error}");
            assert!(
                !error.contains("unsupported"),
                "invalid Rust is not an unsupported feature: {error}"
            );
        }
        assert!(
            parse("fn main(){let x=300 as u8;}")
                .and_then(|p| check(&p))
                .unwrap_err()
                .contains("unsupported")
        );
        let source = "fn f(x:u128)->i32{match x{0=>1,18446744073709551616=>2,340282366920938463463374607431768211455=>3,_=>4}}fn g(x:i128)->i32{match x{-170141183460469231731687303715884105728=>1,170141183460469231731687303715884105727=>2,_=>3}}fn main(){}";
        let p = parse(source).unwrap();
        check(&p).unwrap();
        let f = type_function_with_signatures(&p, 0, &signature_table(&p).unwrap()).unwrap();
        let ir::ExprKind::Match { arms, .. } = &f.exprs[f.body.tail.unwrap().0].kind else {
            panic!("expected match")
        };
        let bits = arms
            .iter()
            .filter_map(|arm| match arm.pattern {
                ir::MatchPattern::Int(value) => Some(value.bits),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(bits, [0, 1u128 << 64, u128::MAX]);
    }

    #[test]
    fn integer_literal_bounds_contexts_and_grouped_minima_are_exact() {
        use ir::IntTy::*;
        for ty in [
            I8, I16, I32, I64, I128, Isize, U8, U16, U32, U64, U128, Usize,
        ] {
            let max = if ty.is_signed() {
                (1u128 << (ty.bits() - 1)) - 1
            } else {
                ty.mask()
            };
            let source = format!(
                "fn f()->{}{{{max}}}fn main(){{let x:{}={max};let y={max}{};let z=1+y;let n=!y;}}",
                ty.name(),
                ty.name(),
                ty.name()
            );
            check(&parse(&source).unwrap()).unwrap();
            let value = resolve_integer(
                &IntegerLiteral {
                    magnitude: max,
                    suffix: Some(ty),
                },
                false,
                None,
            )
            .unwrap();
            assert_eq!(value, ir::IntValue { ty, bits: max });
            if max < u128::MAX {
                assert!(
                    resolve_integer(
                        &IntegerLiteral {
                            magnitude: max + 1,
                            suffix: Some(ty)
                        },
                        false,
                        None
                    )
                    .is_err()
                );
            }
            if ty.is_signed() {
                let magnitude = 1u128 << (ty.bits() - 1);
                let source = format!(
                    "fn f()->{}{{-((({magnitude}{})))}}fn main(){{let x:{}=-(({magnitude}));}}",
                    ty.name(),
                    ty.name(),
                    ty.name()
                );
                let program = parse(&source).unwrap();
                check(&program).unwrap();
                let env = signature_table(&program).unwrap();
                let function = type_function_with_signatures(&program, 0, &env).unwrap();
                assert!(
                    matches!(function.exprs[function.body.tail.unwrap().0].kind,ir::ExprKind::Int(ir::IntValue{ty:actual,bits}) if actual==ty&&bits==magnitude)
                );
                assert!(
                    check(
                        &parse(&format!(
                            "fn main(){{let x=-{{{magnitude}{}}};}}",
                            ty.name()
                        ))
                        .unwrap()
                    )
                    .is_err()
                );
                assert!(
                    resolve_integer(
                        &IntegerLiteral {
                            magnitude: magnitude + 1,
                            suffix: Some(ty)
                        },
                        true,
                        None
                    )
                    .is_err()
                );
            } else {
                assert!(
                    resolve_integer(
                        &IntegerLiteral {
                            magnitude: 0,
                            suffix: Some(ty)
                        },
                        true,
                        None
                    )
                    .is_err()
                );
            }
        }
        check(&parse("fn main(){let x:i64=-2147483648;let a:u16=1+2u16;let b=2u16+1;let x:[u8;0x2_usize]=[0b1_u8,0o7_u8];let y=[0xff_u8;0b10];let v=y[0x0];let b=(1i8<2i8)==(3i16<4i16);}").unwrap()).unwrap();
        for literal in [
            "0xff_u8",
            "0o177_i16",
            "0b_1010__u32",
            "18_446_744_073_709_551_615u64",
        ] {
            let literal = parse_integer_literal(literal).unwrap();
            resolve_integer(&literal, false, None).unwrap();
        }
        for literal in [
            "0x_",
            "0b2",
            "0o8",
            "340282366920938463463374607431768211456u128",
            "0x1_0000_0000_0000_0000_0000_0000_0000_0000u128",
        ] {
            assert!(
                parse_integer_literal(literal).is_err(),
                "accepted {literal}"
            );
        }
        for source in [
            "fn main(){let x=18446744073709551616u64;}",
            "fn main(){let x=0x1_0000_0000_0000_0000u64;}",
            "fn main(){let x:u8=1i8;}",
            "fn main(){let x:i64=1isize;}",
            "fn main(){let a=[1];let i=0u64;let x=a[i];}",
        ] {
            assert!(check(&parse(source).unwrap()).is_err(), "accepted {source}");
        }
    }

    #[test]
    fn integer_patterns_preserve_exact_bits_and_bounded_exhaustiveness() {
        check(&parse("fn f(x:i8)->i32{match x{-128i8=>1,127=>2,_=>3}}fn g(x:u64)->i32{match x{18446744073709551615u64=>1,_=>2}}fn main(){}").unwrap()).unwrap();
        for ty in ["u8", "i8"] {
            let range = if ty == "i8" { -128..128 } else { 0..256 };
            let arms = range.map(|n| format!("{n}=>(),")).collect::<String>();
            check(&parse(&format!("fn f(x:{ty}){{match x{{{arms}}};}}fn main(){{}}")).unwrap())
                .unwrap();
        }
        for source in [
            "fn main(){match 1u8{1i8=>(),_=>()};}",
            "fn main(){match 1u8{256=>(),_=>()};}",
            "fn main(){match 1i8{-129=>(),_=>()};}",
            "fn main(){match 1u64{0=>(),1=>()};}",
        ] {
            assert!(check(&parse(source).unwrap()).is_err(), "accepted {source}");
        }
        assert!(parse("fn main(){match 1i8{\"1\"=>(),_=>()};}").is_err());
        let p = parse("fn main(){match 1u8{1=>(),0x01u8=>(),_=>()};}").unwrap();
        check(&p).unwrap();
    }

    #[test]
    fn e3_mixed_contracts_preserve_formal_kinds_and_real_shared_return_reborrows() {
        let p=parse("fn weaken<'a>(r:&'a mut i32)->&'a i32{r}fn mix<'a>(s:&'a i32,m:&'a mut i32,b:bool)->&'a i32{if b{s}else{m}}fn mut_target<'a>(s:&'a i32,m:&'a mut i32)->&'a mut i32{m}fn no_mut_target<'a>(s:&'a i32)->&'a mut i32{loop{}}fn pin<'a>(m:&'a mut bool,s:&'a i32)->&'a i32{s}fn main(){}").unwrap();
        let env = signature_table(&p).unwrap();
        let empty = |param| ir::InputPath {
            input: ir::InputReference {
                param,
                position: ir::ReferencePosition { depth: 0 },
            },
            projection: vec![].into(),
        };
        assert_eq!(
            env.functions["mix"].ref_contract.results[0].returns,
            ir::ReturnContract::SharedInputPaths(vec![empty(0), empty(1)].into())
        );
        assert_eq!(
            env.functions["mut_target"].ref_contract.results[0].returns,
            ir::ReturnContract::MutInputPaths(vec![empty(1)].into())
        );
        assert_eq!(
            env.functions["no_mut_target"].ref_contract.results[0].returns,
            ir::ReturnContract::MutInputPaths(vec![].into())
        );
        assert_eq!(
            env.functions["pin"].ref_contract.results[0].returns,
            ir::ReturnContract::SharedInputPaths(vec![empty(1)].into())
        );
        assert_eq!(
            env.functions["pin"].ref_contract.input_regions.as_ref(),
            &[input_region(0, 0), input_region(1, 0)]
        );
        assert!(matches!(
            env.functions["weaken"].params[0],
            ir::Ty::Ref {
                kind: ir::BorrowKind::Mut,
                ..
            }
        ));
        let weaken = type_function_with_signatures(&p, 0, &env).unwrap();
        assert!(
            matches!(&weaken.exprs[weaken.body.tail.unwrap().0].kind,ir::ExprKind::Borrow{kind:ir::BorrowKind::Shared,place,..} if place.local==weaken.params[0]&&place.projection==vec![ir::Projection::Deref])
        );
        assert!(
            !weaken
                .exprs
                .iter()
                .any(|e| matches!(e.kind,ir::ExprKind::Local(local) if local==weaken.params[0]))
        );
        for index in 0..p.functions.len() {
            if p.functions[index].name == "no_mut_target" {
                assert!(
                    check_function_with_signatures(&p, index, &env)
                        .unwrap_err()
                        .contains("unsupported")
                );
            } else {
                check_function_with_signatures(&p, index, &env).unwrap();
            }
        }
    }

    #[test]
    fn e3_all_input_positions_regions_and_deferred_adjustments_remain_enforced() {
        let p = parse("fn ambiguous<'a>(s:&'a i32,m:&'a mut bool)->&i32{s}fn main(){}").unwrap();
        assert!(
            signature_table(&p)
                .unwrap_err()
                .contains("missing lifetime specifier")
        );
        let p = parse("fn rigid<'a>(s:&bool,m:&mut i32)->&'a i32{&*m}fn main(){}").unwrap();
        let env = signature_table(&p).unwrap();
        assert_eq!(
            env.functions["rigid"].ref_contract.input_regions.as_ref(),
            &[input_region(0, 0), input_region(1, 1)]
        );
        assert_eq!(
            env.functions["rigid"].ref_contract.results[0].region,
            ir::RegionParamId(2)
        );
        assert!(check_function_with_signatures(&p, 0, &env).is_err());
        let names = (0..63)
            .map(|i| format!("'r{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let p = parse(&format!(
            "fn cap<{names}>(other:&bool,r:&mut i32)->&'r0 mut i32{{loop{{}}}}fn main(){{}}"
        ))
        .unwrap();
        assert!(signature_table(&p).unwrap_err().contains("limit64"));
        let p = parse("fn invalid<'a>(s:&'a i32)->&'a mut i32{s}fn main(){}").unwrap();
        let env = signature_table(&p).unwrap();
        assert!(check_function_with_signatures(&p, 0, &env).is_err());
        for source in [
            "fn deferred<'a>(m:&'a mut i32)->&'a i32{&mut *m}fn main(){}",
            "struct S{x:i32}fn deferred<'a>(m:&'a mut S)->&'a i32{&mut m.x}fn main(){}",
        ] {
            let p = parse(source).unwrap();
            let env = signature_table(&p).unwrap();
            assert!(
                type_function_with_signatures(&p, 0, &env)
                    .unwrap_err()
                    .contains("unsupported")
            );
        }
    }

    #[test]
    fn e2_implicit_and_explicit_fields_have_identical_ir_and_current_carrier_places() {
        let p=parse("struct S{x:i32,y:i32}fn implicit(r:&mut S){let n=r.x;r.x=n+1;let q=&mut r.y;*q=4;}fn explicit(r:&mut S){let n=(*r).x;(*r).x=n+1;let q=&mut (*r).y;*q=4;}fn main(){}").unwrap();
        let env = signature_table(&p).unwrap();
        check_function_with_signatures(&p, 0, &env).unwrap();
        check_function_with_signatures(&p, 1, &env).unwrap();
        let implicit = type_function_with_signatures(&p, 0, &env).unwrap();
        let explicit = type_function_with_signatures(&p, 1, &env).unwrap();
        assert_eq!(
            format!("{:?}", implicit.exprs),
            format!("{:?}", explicit.exprs)
        );
        assert_eq!(
            format!("{:?}", implicit.body),
            format!("{:?}", explicit.body)
        );
        let p=parse("struct S{x:i32}fn implicit(){let mut x=S{x:1};let mut y=S{x:2};let mut r=&mut x;r.x={r=&mut y;3};}fn explicit(){let mut x=S{x:1};let mut y=S{x:2};let mut r=&mut x;(*r).x={r=&mut y;3};}fn main(){}").unwrap();
        let env = signature_table(&p).unwrap();
        check_function_with_signatures(&p, 0, &env).unwrap();
        check_function_with_signatures(&p, 1, &env).unwrap();
        let implicit = type_function_with_signatures(&p, 0, &env).unwrap();
        let explicit = type_function_with_signatures(&p, 1, &env).unwrap();
        assert_eq!(
            format!("{:?}", implicit.exprs),
            format!("{:?}", explicit.exprs)
        );
        assert_eq!(
            format!("{:?}", implicit.body),
            format!("{:?}", explicit.body)
        );
        assert!(implicit.body.statements.iter().any(|s|matches!(s,ir::Stmt::AssignPlace{place,..} if place.projection==vec![ir::Projection::Deref,ir::Projection::Field(0)])));
    }

    #[test]
    fn e2_field_reads_use_one_genuine_call_temp_and_usize_peer_hints() {
        let p=parse("struct Wide{u:usize}fn get<'a>(r:&'a Wide)->&'a Wide{r}fn get_mut<'a>(r:&'a mut Wide)->&'a mut Wide{r}fn main(){let mut s=Wide{u:4294967296usize};let r=&s;let sum=1+r.u;let bigger=r.u>1;let x=get(&s).u;let y=get_mut(&mut s).u;let mut tuple=(1,2);let t=&mut tuple;t.0=3;let n=t.1;}").unwrap();
        let env = signature_table(&p).unwrap();
        check_function_with_signatures(&p, 2, &env).unwrap();
        let main = type_function_with_signatures(&p, 2, &env).unwrap();
        assert_eq!(
            main.exprs
                .iter()
                .filter(|e| matches!(e.kind, ir::ExprKind::Call { .. }))
                .count(),
            2
        );
        assert!(
            main.exprs
                .iter()
                .filter(|e| matches!(
                    e.kind,
                    ir::ExprKind::Int(ir::IntValue {
                        ty: ir::IntTy::Usize,
                        bits: 1
                    })
                ))
                .count()
                >= 2
        );
        for expr in &main.exprs {
            if let ir::ExprKind::Field { base, .. } = expr.kind {
                assert!(matches!(
                    main.exprs[base.0].kind,
                    ir::ExprKind::Deref { .. }
                ));
            }
        }
    }

    #[test]
    fn e2_field_normalization_preserves_authority_privacy_and_deferred_boundaries() {
        for source in [
            "struct S{x:i32}fn main(){let mut s=S{x:1};let r=&s;r.x=2;}",
            "struct S{x:i32}fn main(){let s=S{x:1};let r=&s;let q=&mut r.x;}",
            "struct T{x:i32}struct S{child:T}fn main(){let s=S{child:T{x:1}};let r=&s;let moved=r.child;}",
            "mod m{pub struct S{x:i32}pub fn make()->S{S{x:1}}}fn main(){let s=m::make();let r=&s;let n=r.x;}",
        ] {
            assert!(
                parse(source).and_then(|p| check(&p)).is_err(),
                "accepted {source}"
            );
        }
        for source in [
            "struct S{a:[i32;1]}fn main(){let s=S{a:[1]};let r=&s;let i=0usize;let _=r.a[i];}",
            "struct S{a:[i32;1]}fn main(){let mut s=S{a:[1]};let r=&mut s;let i=0usize;r.a[i]=2;}",
            "enum E{A}struct S{e:E}fn main(){let s=S{e:E::A};let r=&s;match r.e{E::A=>()};}",
            "enum E{A}struct S{e:E}fn main(){let s=S{e:E::A};let r=&s;let E::A=r.e;}",
            "struct S{x:i32}fn main(){let s=S{x:1};let r=&s;let n=({r}).x;}",
        ] {
            let p = parse(source).unwrap();
            let env = signature_table(&p).unwrap();
            assert!(
                type_function_with_signatures(&p, p.functions.len() - 1, &env)
                    .unwrap_err()
                    .contains("unsupported"),
                "lost boundary {source}"
            );
        }
        check(&parse("struct S{x:i32}fn main(){let s=S{x:1};let r=&s;return;r.x=2;}").unwrap())
            .unwrap();
    }

    #[test]
    fn e1_elision_matches_explicit_contracts_and_keeps_anonymous_inputs_rigid() {
        let p=parse("fn elided(x:&i32,n:i32)->&i32{x}fn explicit<'a>(x:&'a i32,n:i32)->&'a i32{x}fn mut_elided(x:&mut i32)->&mut i32{x}fn mut_explicit<'a>(x:&'a mut i32)->&'a mut i32{x}fn unused_names<'a,'b>(x:&i32)->&i32{x}fn rigid<'a>(x:&i32,y:&i32)->&'a i32{x}fn repeated<'a>(x:&'a i32,y:&'a bool)->&'a i32{x}fn main(){}").unwrap();
        let env = signature_table(&p).unwrap();
        assert_eq!(
            env.functions["elided"].ref_contract,
            env.functions["explicit"].ref_contract
        );
        assert_eq!(
            env.functions["mut_elided"].ref_contract,
            env.functions["mut_explicit"].ref_contract
        );
        let unused = &env.functions["unused_names"].ref_contract;
        assert_eq!(unused.region_count, 1);
        assert_eq!(unused.input_regions.as_ref(), &[input_region(0, 0)]);
        assert_eq!(unused.results[0].region, ir::RegionParamId(0));
        let rigid = &env.functions["rigid"].ref_contract;
        assert_eq!(
            rigid.input_regions.as_ref(),
            &[input_region(0, 0), input_region(1, 1)]
        );
        assert_eq!(rigid.results[0].region, ir::RegionParamId(2));
        assert!(rigid.outlives.is_empty());
        assert_eq!(
            rigid.results[0].returns,
            ir::ReturnContract::SharedInputPaths(vec![].into())
        );
        type_function_with_signatures(&p, 5, &env).unwrap();
        assert!(check_function_with_signatures(&p, 5, &env).is_err());
        for index in [0, 1, 2, 3, 4, 6] {
            check_function_with_signatures(&p, index, &env).unwrap();
        }
    }

    #[test]
    fn e1_output_elision_counts_reference_positions_not_distinct_region_ids() {
        for source in [
            "fn missing()->&i32{loop{}}fn main(){}",
            "fn ambiguous(x:&i32,y:&i32)->&i32{x}fn main(){}",
            "fn repeated<'a>(x:&'a i32,y:&'a i32)->&i32{x}fn main(){}",
            "fn repeated<'a>(x:&'a mut i32,y:&'a mut i32)->&mut i32{x}fn main(){}",
            "fn different<'a>(x:&'a i32,y:&'a bool)->&i32{x}fn main(){}",
        ] {
            let p = parse(source).unwrap();
            assert!(
                signature_table(&p)
                    .unwrap_err()
                    .contains("missing lifetime specifier"),
                "accepted {source}"
            );
        }
    }

    #[test]
    fn e1_combined_region_cap_does_not_limit_erased_no_reference_parameters() {
        let names = |count: usize| {
            (0..count)
                .map(|i| format!("'r{i}"))
                .collect::<Vec<_>>()
                .join(",")
        };
        let p = parse(&format!(
            "fn at_limit<{}>(x:&i32)->&i32{{x}}fn main(){{}}",
            names(63)
        ))
        .unwrap();
        let env = signature_table(&p).unwrap();
        assert_eq!(env.functions["at_limit"].ref_contract.region_count, 1);
        assert_eq!(
            env.functions["at_limit"].ref_contract.results[0].region,
            ir::RegionParamId(0)
        );
        let p = parse(&format!(
            "fn over_limit<{}>(x:&i32)->&i32{{x}}fn main(){{}}",
            names(64)
        ))
        .unwrap();
        assert!(signature_table(&p).unwrap_err().contains("limit64"));
        let params = (0..4096)
            .map(|i| format!("p{i}:&i32"))
            .collect::<Vec<_>>()
            .join(",");
        let p = parse(&format!("fn erased({params}){{}}fn main(){{}}")).unwrap();
        let env = signature_table(&p).unwrap();
        let contract = &env.functions["erased"].ref_contract;
        assert_eq!(contract.region_count, 0);
        assert!(contract.input_regions.is_empty());
        assert!(contract.results.is_empty());
        assert!(contract.outlives.is_empty());
        assert_eq!(env.functions["erased"].params.len(), 4096);
    }

    #[test]
    fn projected_contract_paths_keep_source_privacy_and_argument_layouts() {
        let p=parse("mod m{pub struct S{nested:([i32;1],)}pub fn get<'a>(p:&'a S)->&'a i32{&(*p).nested.0[0]}pub fn get_mut<'a>(p:&'a mut S)->&'a mut i32{&mut (*p).nested.0[0]}pub fn make()->S{S{nested:([7],)}}}fn choose<'a>(flag:bool,a:&'a(i32,bool),b:&'a(bool,i32))->&'a i32{if flag{&(*a).0}else{&(*b).1}}fn main(){let mut s=m::make();let r=m::get(&s);let n=*r;let q=m::get_mut(&mut s);*q=9;}").unwrap();
        let env = signature_table(&p).unwrap();
        let nested = ir::InputPath {
            input: ir::InputReference {
                param: 0,
                position: ir::ReferencePosition { depth: 0 },
            },
            projection: vec![
                ir::Projection::Field(0),
                ir::Projection::Field(0),
                ir::Projection::ConstantIndex(0),
            ]
            .into(),
        };
        assert_eq!(
            env.functions["m::get"].ref_contract.results[0].returns,
            ir::ReturnContract::SharedInputPaths(vec![nested.clone()].into())
        );
        assert_eq!(
            env.functions["m::get_mut"].ref_contract.results[0].returns,
            ir::ReturnContract::MutInputPaths(vec![nested].into())
        );
        assert_eq!(
            env.functions["choose"].ref_contract.results[0].returns,
            ir::ReturnContract::SharedInputPaths(
                vec![
                    ir::InputPath {
                        input: ir::InputReference {
                            param: 1,
                            position: ir::ReferencePosition { depth: 0 }
                        },
                        projection: vec![ir::Projection::Field(0)].into()
                    },
                    ir::InputPath {
                        input: ir::InputReference {
                            param: 2,
                            position: ir::ReferencePosition { depth: 0 }
                        },
                        projection: vec![ir::Projection::Field(1)].into()
                    },
                ]
                .into()
            )
        );
        for index in 0..p.functions.len() {
            type_function_with_signatures(&p, index, &env).unwrap();
        }
        let invalid=parse("mod m{pub struct S{x:i32}pub fn make()->S{S{x:7}}}fn main(){let s=m::make();let r=&s.x;}").unwrap();
        let env = signature_table(&invalid).unwrap();
        assert!(
            type_function_with_signatures(&invalid, 1, &env)
                .unwrap_err()
                .contains("private")
        );
    }

    #[test]
    fn b3b_mutable_contracts_keep_complete_original_indices_and_return_reborrows() {
        let p=parse("fn pick<'a>(flag:bool,s:&'a mut bool,x:&'a mut i32,y:&'a mut i32)->&'a mut i32{if flag{x}else{y}}fn moved<'a>(r:&'a mut i32)->&'a mut i32{let moved=r;moved}fn main(){}").unwrap();
        let env = signature_table(&p).unwrap();
        assert_eq!(
            env.functions["pick"].ref_contract.results[0].returns,
            ir::ReturnContract::MutInputPaths(
                vec![
                    ir::InputPath {
                        input: ir::InputReference {
                            param: 2,
                            position: ir::ReferencePosition { depth: 0 }
                        },
                        projection: vec![].into()
                    },
                    ir::InputPath {
                        input: ir::InputReference {
                            param: 3,
                            position: ir::ReferencePosition { depth: 0 }
                        },
                        projection: vec![].into()
                    }
                ]
                .into()
            )
        );
        let pick = type_function_with_signatures(&p, 0, &env).unwrap();
        assert_eq!(
            pick.exprs
                .iter()
                .filter(|e| matches!(
                    e.kind,
                    ir::ExprKind::Borrow {
                        kind: ir::BorrowKind::Mut,
                        ..
                    }
                ))
                .count(),
            2
        );
        let moved = type_function_with_signatures(&p, 1, &env).unwrap();
        let ir::Stmt::Let { local, value } = moved.body.statements[0] else {
            panic!("expected genuine untyped handle let")
        };
        assert!(matches!(moved.exprs[value.0].kind,ir::ExprKind::Local(id) if id==moved.params[0]));
        assert!(
            matches!(&moved.exprs[moved.body.tail.unwrap().0].kind,ir::ExprKind::Borrow{kind:ir::BorrowKind::Mut,place,..} if place.local==local && place.projection==vec![ir::Projection::Deref])
        );
        assert!(std::sync::Arc::ptr_eq(
            &moved.ref_contract,
            &env.functions["moved"].ref_contract
        ));
    }

    #[test]
    fn b3b_mutable_call_temp_copy_reads_preserve_computed_place_boundaries() {
        let p=parse("struct S{x:i32}fn id<'a>(r:&'a mut S)->&'a mut S{r}fn main(){let mut s=S{x:7};let n=(*id(&mut s)).x;}").unwrap();
        let env = signature_table(&p).unwrap();
        let main = type_function_with_signatures(&p, 1, &env).unwrap();
        assert_eq!(
            main.exprs
                .iter()
                .filter(|e| matches!(e.kind, ir::ExprKind::Call { .. }))
                .count(),
            1
        );
        assert!(
            main.exprs
                .iter()
                .any(|e| matches!(e.kind, ir::ExprKind::Deref { .. }))
        );
        for source in [
            "fn id<'a>(r:&'a mut i32)->&'a mut i32{r}fn main(){let mut x=1;*id(&mut x)=2;}",
            "fn id<'a>(r:&'a mut i32)->&'a mut i32{r}fn main(){let mut x=1;let q=&mut *id(&mut x);}",
        ] {
            if source.contains("*id(&mut x)=2") {
                check(&parse(source).unwrap()).unwrap();
            } else {
                assert!(parse(source).unwrap_err().contains("unsupported"));
            }
        }
        for source in [
            "fn bad<'a>(x:&'a i32)->&'a mut i32{loop{}}fn main(){}",
            "fn bad<'a>(x:&'a mut i32,y:&'a i32)->&'a mut i32{x}fn main(){}",
            "fn bad<'a>(x:&'a mut i32)->&'a i32{&*x}fn main(){}",
        ] {
            let p = parse(source).unwrap();
            let env = signature_table(&p).unwrap();
            if source.contains("{loop{}}") {
                assert!(
                    check_function_with_signatures(&p, 0, &env)
                        .unwrap_err()
                        .contains("unsupported")
                );
            } else {
                check_function_with_signatures(&p, 0, &env).unwrap();
            }
        }
    }

    #[test]
    fn b3a_contracts_are_alpha_canonical_and_keep_original_parameter_indices() {
        let source = "fn pick<'a>(flag:bool,s:&'a bool,x:&'a i32,y:&'a i32)->&'a i32{if flag{x}else{y}}fn renamed<'z>(flag:bool,s:&'z bool,x:&'z i32,y:&'z i32)->&'z i32{if flag{x}else{y}}fn main(){}";
        let p = parse(source).unwrap();
        let env = signature_table(&p).unwrap();
        let first = &env.functions["pick"];
        assert_eq!(first.ref_contract, env.functions["renamed"].ref_contract);
        assert_eq!(
            first.ref_contract.input_regions.as_ref(),
            &[input_region(1, 0), input_region(2, 0), input_region(3, 0)]
        );
        assert_eq!(
            first.ref_contract.results[0].returns,
            ir::ReturnContract::SharedInputPaths(
                vec![
                    ir::InputPath {
                        input: ir::InputReference {
                            param: 2,
                            position: ir::ReferencePosition { depth: 0 }
                        },
                        projection: vec![].into()
                    },
                    ir::InputPath {
                        input: ir::InputReference {
                            param: 3,
                            position: ir::ReferencePosition { depth: 0 }
                        },
                        projection: vec![].into()
                    }
                ]
                .into()
            )
        );
        let checked = type_function_with_signatures(&p, 0, &env).unwrap();
        assert!(std::sync::Arc::ptr_eq(
            &first.ref_contract,
            &checked.ref_contract
        ));
        assert_eq!(env.functions["main"].ref_contract.input_regions.len(), 0);
    }

    #[test]
    fn b3a_outlives_bounds_and_call_result_dereference_are_structural() {
        let p=parse("struct S{x:i32}fn id<'a>(x:&'a S)->&'a S{x}fn short<'a,'b>(x:&'a i32)->&'b i32 where 'a:'b{x}fn missing<'a,'b>(x:&'a i32)->&'b i32{x}fn main(){let s=S{x:7};let n=(*id(&s)).x;}").unwrap();
        let env = signature_table(&p).unwrap();
        assert_eq!(
            env.functions["short"].ref_contract.results[0].returns,
            ir::ReturnContract::SharedInputPaths(
                vec![ir::InputPath {
                    input: ir::InputReference {
                        param: 0,
                        position: ir::ReferencePosition { depth: 0 }
                    },
                    projection: vec![].into()
                }]
                .into()
            )
        );
        assert_eq!(
            env.functions["missing"].ref_contract.results[0].returns,
            ir::ReturnContract::SharedInputPaths(vec![].into())
        );
        let checked = type_function_with_signatures(&p, 3, &env).unwrap();
        assert_eq!(
            checked
                .exprs
                .iter()
                .filter(|e| matches!(e.kind, ir::ExprKind::Call { .. }))
                .count(),
            1
        );
        assert!(
            checked
                .exprs
                .iter()
                .any(|e| matches!(e.kind, ir::ExprKind::Deref { .. }))
        );
        // Missing universal outlives relations are analyzer obligations, not type inference.
        type_function_with_signatures(&p, 2, &env).unwrap();
    }

    #[test]
    fn b3a_rejects_unbound_duplicate_and_deferred_lifetime_forms() {
        for source in [
            "fn f<'a,'a>(x:&'a i32)->&'a i32{x}fn main(){}",
            "fn f<'a>(x:&'b i32)->&'a i32{x}fn main(){}",
            "fn f<'a:'b>(x:&'a i32)->&'a i32{x}fn main(){}",
            "fn f<'a>(x:&'a i32){}fn main(){}",
            "fn f<'fn>(x:&'fn i32)->&'fn i32{x}fn main(){}",
        ] {
            if source == "fn f<'a>(x:&'a i32){}fn main(){}" {
                check(&parse(source).unwrap()).unwrap();
                continue;
            }
            assert!(
                parse(source).and_then(|p| signature_table(&p)).is_err(),
                "accepted {source}"
            );
        }
        let p = parse("fn f<'a>(x:&'a i32)->&'a i32{let q:&'a i32=x;q}fn main(){}").unwrap();
        let env = signature_table(&p).unwrap();
        assert!(
            type_function_with_signatures(&p, 0, &env)
                .unwrap_err()
                .contains("unsupported")
        );
    }

    #[test]
    fn b2_expected_reference_sites_reborrow_the_carrier_without_a_prior_move() {
        let p=parse("fn read(r:&i32)->i32{*r}fn bump(r:&mut i32){*r=*r+1;}fn main(){let mut x=1;let mut r=&mut x;bump(r);bump(r);read(r);let t:&mut i32=r;*t=3;let mut y=2;let s=&mut y;r=s;let a:&i32={r};let b:&i32=if true{r}else{r};let c:&i32=match true{_=>r};let moved=r;}").unwrap();
        let env = signature_table(&p).unwrap();
        let typed = type_function_with_signatures(&p, 2, &env).unwrap();
        let carrier = typed.locals.iter().position(|l| l.name == "r").unwrap();
        let reborrows=typed.exprs.iter().filter(|e|matches!(&e.kind,ir::ExprKind::Borrow{place,..} if place.local==ir::LocalId(carrier)&&place.projection==vec![ir::Projection::Deref])).count();
        assert!(
            reborrows >= 7,
            "missing call/let/assignment/control-context reborrows: {reborrows}"
        );
        let moved = typed
            .body
            .statements
            .iter()
            .find_map(|s| {
                if let ir::Stmt::Let { local, value } = s
                    && typed.locals[local.0].name == "moved"
                {
                    Some(*value)
                } else {
                    None
                }
            })
            .unwrap();
        assert!(
            matches!(typed.exprs[moved.0].kind,ir::ExprKind::Local(local) if local==ir::LocalId(carrier))
        );
        for i in 0..2 {
            type_function_with_signatures(&p, i, &env).unwrap();
        }
    }

    #[test]
    fn b2_noescape_restrictions_hold_for_unused_declarations_and_explicit_borrows() {
        for source in [
            "fn unused(r:&[&i32;0]){}fn main(){}",
            "fn unused(r:(&i32,)){}fn main(){}",
            "fn unused(r:&&i32){}fn main(){}",
            "fn unused()->&i32{loop{}}fn main(){}",
            "struct S{r:&i32}fn main(){}",
            "fn read(r:&i32){}fn main(){let mut x=1;read(&mut x);}",
            "fn main(){let mut x=1;let r=&mut x;let q:&mut i32=loop{break r;};}",
        ] {
            if source == "fn unused(r:&&i32){}fn main(){}" {
                check(&parse(source).unwrap()).unwrap();
                continue;
            }
            assert!(
                parse(source)
                    .and_then(|p| {
                        let env = signature_table(&p)?;
                        type_function_with_signatures(&p, p.functions.len() - 1, &env).map(|_| ())
                    })
                    .is_err(),
                "accepted {source}"
            );
        }
    }

    #[test]
    fn blocklike_statements_do_not_consume_a_following_deref_as_multiplication() {
        for source in [
            "fn choose(b:bool)->i32{let x=3;let y=7;let r:&i32;if b{r=&x;}else{r=&y;}*r}fn main(){let n=choose(true);}",
            "fn run()->i32{let mut x=3;let s:&mut i32;{let r=&mut x;s=&mut *r;}*s=7;x}fn main(){let n=run();}",
            "fn main(){let x=1;let r=&x;if true{}else if false{}else{} *r;}",
            "fn main(){let x={1}*2;({1})*2;let a=({[1]})[0];}",
        ] {
            check(&parse(source).unwrap()).unwrap();
        }
    }

    #[test]
    fn b1_types_explicit_borrows_reborrows_and_static_deref_projections() {
        let program=parse("struct S{x:i32,y:i32}fn main(){let mut s=S{x:1,y:2};let r:&mut S=&mut s;(*r).x=3;let c=(*r).x;let p=&mut (*r).y;*p=4;let q=&*r;let z=(*q).x;let mut a=[1,2];let ar=&mut a;(*ar)[0]=3;let n=(*ar)[0];let owner=1;let sh=&owner;let copied=sh;let shared_s=S{x:7,y:8};let shared_r=&shared_s;let shared_n=(*shared_r).x;let moved=r;}").unwrap();
        let env = signature_table(&program).unwrap();
        let typed = type_function_with_signatures(&program, 0, &env).unwrap();
        assert!(typed.body.statements.iter().any(|s|matches!(s,ir::Stmt::AssignPlace{place,..} if place.projection==vec![ir::Projection::Deref,ir::Projection::Field(0)])));
        assert!(typed.body.statements.iter().any(|s|matches!(s,ir::Stmt::AssignPlace{place,..} if place.projection==vec![ir::Projection::Deref,ir::Projection::ConstantIndex(0)])));
        assert!(typed.exprs.iter().any(|e|matches!(&e.kind,ir::ExprKind::Borrow{kind:ir::BorrowKind::Shared,place,..} if place.projection==vec![ir::Projection::Deref])));
        for source in [
            "fn main(){let mut x=1;let r=&mut x;let moved=r;let _=*r;match *r{_=>()};}",
            "fn main(){let r:&i32;let _=*r;match *r{_=>()};}",
            "fn main(){let x=1;return;let _=&mut x;}",
            "fn main(){let x=1;let r=&x;return;*r=2;}",
        ] {
            check(&parse(source).unwrap()).unwrap();
        }
    }

    #[test]
    fn b1_rejects_forbidden_reference_locations_authority_and_implicit_coercions() {
        for source in [
            "fn main(){let x=1;let _=&mut x;}",
            "fn main(){let x=1;let r=&x;*r=2;}",
            "fn main(){let mut x=1;let r=&x;let p=&mut *r;}",
        ] {
            let p = parse(source).unwrap();
            let env = signature_table(&p).unwrap();
            type_function_with_signatures(&p, 0, &env).unwrap();
            assert!(check(&p).is_err(), "analysis accepted {source}");
        }
        for source in [
            "fn unused()->&i32{loop{}}fn main(){}",
            "struct S{x:(&i32,)}fn main(){}",
            "enum E{V([&i32;0])}fn main(){}",
            "fn main(){let x=1;let r=&x;let a=[r;0];}",
            "fn main(){let x=1;let r=&x;let t=(r,);}",
            "fn main(){let x=1;let r=&x;let nested=&r;}",
            "fn main(){let r=&1;}",
            "fn main(){let x=1;let a=&x;let b=&x;let eq=a==b;}",
            "fn main(){let a=[1];let i=0usize;let p=&a[i];}",
            "fn main(){let a=[1];let p=&a;let i=0usize;let n=(*p)[i];}",
        ] {
            let result = parse(source).and_then(|p| {
                let env = signature_table(&p)?;
                type_function_with_signatures(&p, p.functions.len() - 1, &env).map(|_| ())
            });
            if matches!(
                source,
                "fn main(){let x=1;let r=&x;let nested=&r;}"
                    | "fn main(){let x=1;let r=&x;let a=[r;0];}"
                    | "fn main(){let x=1;let r=&x;let t=(r,);}"
            ) {
                result.unwrap();
                check(&parse(source).unwrap()).unwrap();
            } else {
                assert!(result.is_err(), "accepted {source}");
            }
        }
    }

    #[test]
    fn usize_retains_full_width_and_uses_forward_and_binary_peer_contexts() {
        let p=parse("fn id(x:usize)->usize{x}fn high()->usize{18446744073709551615}fn main(){let u:usize=4294967296;let s=18446744073709551615usize;let b=1<u;let c=u>1;let r:usize=1+2;let q=1+u;let not=!0usize;let x=id(4294967296);let a:[usize;2]=[1,2];let mut b=[1,2];let i:usize=0;b[i]=3;let n=b[i];let min=-2147483648i32;}").unwrap();
        let env = signature_table(&p).unwrap();
        let typed = type_function_with_signatures(&p, 2, &env).unwrap();
        assert!(typed.exprs.iter().any(|e| matches!(
            e.kind,
            ir::ExprKind::Int(ir::IntValue {
                ty: ir::IntTy::Usize,
                bits: 18446744073709551615
            })
        )));
        assert!(
            typed
                .exprs
                .iter()
                .any(|e| matches!(e.kind, ir::ExprKind::DynamicIndex { .. }))
        );
        assert!(
            typed
                .body
                .statements
                .iter()
                .any(|s| matches!(s, ir::Stmt::AssignIndex { .. }))
        );
        assert_eq!(
            type_function_with_signatures(&p, 1, &env).unwrap().result,
            ir::Ty::Int(ir::IntTy::Usize)
        );
    }

    #[test]
    fn usize_does_not_convert_i32_or_erase_runtime_index_pattern_boundaries() {
        for source in [
            "fn main(){let x=0usize;match x{0=>(),_=>()};}",
            "fn main(){match 0usize{0usize=>(),_=>()};}",
        ] {
            check(&parse(source).unwrap()).unwrap();
        }
        for source in [
            "fn main(){let a=[1];let i:i32=0;let x=a[i];}",
            "fn main(){let x:usize=1i32;}",
            "fn main(){let x:i32=1usize;}",
            "fn main(){let x:usize=-1;}",
            "fn main(){let x=18446744073709551616usize;}",
            "fn main(){let a=[[1]];let i=0usize;let x=a[i][0];}",
            "struct S{x:i32}fn main(){let a=[S{x:1}];let i=0usize;let S{x:_}=a[i];}",
        ] {
            assert!(
                parse(source).and_then(|p| check(&p)).is_err(),
                "accepted {source}"
            );
        }
    }

    #[test]
    fn arrays_contextualize_empty_literals_and_keep_constant_index_projection() {
        let source = "struct S{x:[i32;0]}enum E{V([i32;0])}fn make()->[i32;0]{[]}fn explicit()->[i32;0]{return [];}fn take(x:[i32;0]){}fn main(){let z:[i32;0]=[];take([]);let s=S{x:[]};let e=E::V([]);let nested:[[i32;0];2]=[[],[]];let mut a=[1,2];a[0]=3;let n=a[0];let t:([i32;0],)=([],);let q:[i32;0]=if true{[]}else{[]};let r:[i32;0]=match true{_=>[]};}";
        let program = parse(source).unwrap();
        check(&program).unwrap();
        let function = type_function_with_signatures(
            &program,
            program.functions.len() - 1,
            &signature_table(&program).unwrap(),
        )
        .unwrap();
        assert!(function.body.statements.iter().any(|s|matches!(s,ir::Stmt::AssignPlace{place,..} if place.projection==vec![ir::Projection::ConstantIndex(0)])));
    }

    #[test]
    fn arrays_reject_bounds_types_repeat_moves_and_unsupported_inference() {
        for source in [
            "fn main(){let a=[1,2];let n=a[2];}",
            "fn main(){let a=[1,true];}",
            "fn main(){let a:[i32;1]=[];}",
            "struct S{x:i32}fn main(){let a=[S{x:1};2];}",
            "struct S{x:i32}fn main(){let a=[S{x:1}];let s=a[0];}",
            "fn main(){let a=[1;65536];}",
            "fn main(){let a=[];}",
            "fn main(){let a=[1];let i=0;let n=a[i];}",
        ] {
            assert!(
                parse(source).and_then(|p| check(&p)).is_err(),
                "accepted {source}"
            );
        }
        check(
            &parse("struct S{x:i32}fn main(){let a=[S{x:1};0];let b=[S{x:2};1];let n=b[0].x;}")
                .unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn payload_variants_preserve_constructor_shape_source_order_and_projection_ids() {
        let source = "struct S{x:i32} enum E{Unit,Tuple(S,bool),Named{a:i32,b:S},Zero(),Empty{}} use E::Named as Alias;fn main(){let e=Alias{b:S{x:2},a:1};let n=match e{Alias{b:S{x:n},a:_,..}=>n,E::Tuple(S{x:n},_)=>n,E::Unit=>0,E::Zero()=>0,E::Empty{}=>0};let z=E::Zero();let q=E::Empty{};}";
        let program = parse(source).unwrap();
        let env = type_env(&program).unwrap();
        let checked = type_function_with_env(&program, 0, &env).unwrap();
        checked.validate().unwrap();
        let enumeration = &env.enums["E"];
        assert!(enumeration.has_payload());
        assert_eq!(enumeration.def().variants[3].shape, ir::VariantShape::Tuple);
        assert_eq!(
            enumeration.def().variants[4].shape,
            ir::VariantShape::Struct
        );
        assert!(checked.exprs.iter().any(|e|matches!(&e.kind,ir::ExprKind::EnumVariant{variant:2,fields,..} if fields.iter().map(|(i,_)|*i).collect::<Vec<_>>()==vec![1,0])));
        let arms = checked
            .exprs
            .iter()
            .find_map(|e| match &e.kind {
                ir::ExprKind::Match { arms, .. } => Some(arms),
                _ => None,
            })
            .unwrap();
        let ir::MatchPattern::EnumVariant { bindings, .. } = &arms[0].pattern else {
            panic!("variant pattern")
        };
        assert_eq!(
            bindings[0].projection,
            vec![
                ir::Projection::Downcast(ir::VariantId {
                    owner: enumeration.def().id,
                    index: 2
                }),
                ir::Projection::Field(1),
                ir::Projection::Field(0)
            ]
        );
        check(&program).unwrap();
    }

    #[test]
    fn singleton_payload_let_and_nested_irrefutable_patterns_are_typed() {
        let source = "enum Inner{One(i32)} enum Outer{One{inner:Inner,flag:bool}}fn main(){let Outer::One{inner:Inner::One(mut n),flag:_}=Outer::One{flag:true,inner:Inner::One(3)};n=4;let e=Outer::One{inner:Inner::One(n),flag:false};let n=match e{Outer::One{inner:Inner::One(n),..}=>n};}";
        let program = parse(source).unwrap();
        let env = type_env(&program).unwrap();
        let checked = type_function_with_env(&program, 0, &env).unwrap();
        checked.validate().unwrap();
        check(&program).unwrap();
    }

    #[test]
    fn mixed_nominal_payload_dag_validates_unused_cycles_fields_and_privacy() {
        for source in [
            "struct S{x:E}enum E{A(S),B}fn main(){}",
            "enum A{V(B)}enum B{V(A)}fn main(){}",
            "enum A{V(Missing)}fn main(){}",
            "enum A{V{x:i32,x:bool}}fn main(){}",
            "mod m{struct H{x:i32}}enum E{V(m::H)}fn main(){}",
            "enum E{V(i32)}fn main(){E::V(true);}",
            "enum E{V{x:i32}}fn main(){E::V{};}",
            "enum E{V(i32)}fn main(){match E::V(1){E::V()=>()};}",
            "enum E{V{i:i32}}fn main(){match E::V{i:1}{E::V{i:x,i:y}=>()};}",
        ] {
            assert!(
                parse(source).and_then(|program| check(&program)).is_err(),
                "accepted {source}"
            );
        }
        let program =
            parse("enum E{V(S)}struct S{x:i32}fn main(){let E::V(S{x:n})=E::V(S{x:1});}").unwrap();
        check(&program).unwrap();
    }

    #[test]
    fn variant_imports_reserve_type_namespace_without_becoming_types() {
        for variant in ["V", "V(i32)", "V{x:i32}"] {
            for conflict in ["struct A{x:i32}", "enum A{Unit}", "mod A{}"] {
                let source = format!("{conflict} enum E{{{variant}}}use E::V as A;fn main(){{}}");
                assert!(parse(&source).is_err(), "accepted {source}");
            }
            let source = format!("enum E{{{variant}}}use E::V as A;fn f(x:A){{}}fn main(){{}}");
            assert!(parse(&source).is_err(), "variant used as type: {source}");
        }
        check(&parse("fn A()->i32{1}enum E{V{x:i32}}use E::V as A;fn main(){let n=A();let e=A{x:n};match e{A{x:n}=>{}}}").unwrap()).unwrap();
        check(&parse("struct A{x:i32}fn A()->i32{1}fn main(){let n=A();let a=A{x:n};}").unwrap())
            .unwrap();
        check(&parse("enum E{V{x:i32}}use E::V;fn f(V:i32)->i32{V}fn main(){let V:i32;V=1;let n=f(V);let V=2;let n=V;}").unwrap()).unwrap();
    }

    #[test]
    fn typed_deferred_declarations_publish_fresh_uninitialized_locals() {
        let source = "enum E{A,B} struct S{e:E} fn main(){let x=1;let x:i32;let mut y:(i32,bool);let s:S;x=2;y=(x,true);s=S{e:E::A};let x=true;match s.e{E::A=>(),E::B=>()};}";
        let program = parse(source).unwrap();
        let env = type_env(&program).unwrap();
        let checked = type_function_with_env(&program, 0, &env).unwrap();
        assert_eq!(checked.locals.len(), 5);
        assert_eq!(checked.locals[0].name, "x");
        assert_eq!(checked.locals[1].name, "x");
        assert!(!checked.locals[1].mutable);
        assert!(checked.locals[2].mutable);
        assert!(matches!(
            checked.body.statements[1],
            ir::Stmt::Declare {
                local: ir::LocalId(1)
            }
        ));
        checked.validate().unwrap();
        check(&program).unwrap();
    }

    #[test]
    fn deferred_declarations_keep_flow_checks_outside_pure_typing() {
        for source in [
            "fn main(){let x:i32;x=1;let y=x;}",
            "fn f(b:bool)->i32{let x:i32;if b{x=1;}else{x=2;}x}fn main(){}",
            "fn main(){let mut x:i32;x=1;x=2;}",
            "fn main(){return;let x=1;x=2;}",
            "fn main(){let x=1;return;x=2;}",
        ] {
            check(&parse(source).unwrap()).unwrap();
        }
        for source in [
            "fn main(){let x:i32;let y=x;}",
            "fn main(){let x:i32;x=1;x=2;}",
            "fn main(){let x=1;x=2;}",
            "fn f(b:bool){let x:i32;if b{x=1;}let y=x;}fn main(){}",
            "fn main(){let x:i32;loop{x=1;}}",
        ] {
            let program = parse(source).unwrap();
            let env = type_env(&program).unwrap();
            type_function_with_env(&program, 0, &env).unwrap();
            assert!(
                check_function_with_signatures(&program, 0, &env).is_err(),
                "accepted {source}"
            );
            assert!(check_function(&program, 0).is_err());
            assert!(check(&program).is_err());
        }
        for source in [
            "fn main(){let x;}",
            "fn main(){let mut x;}",
            "fn main(){let (x,y):(i32,i32);}",
            "fn main(){let _:i32;}",
            "fn main(){let x:_;}",
            "fn main(){let x:!;}",
            "enum E{A}use E::A;fn main(){let A:E;}",
        ] {
            if matches!(source, "fn main(){let x;}" | "fn main(){let mut x;}") {
                assert!(check(&parse(source).unwrap()).is_err(), "accepted {source}");
            } else {
                assert!(parse(source).is_err(), "accepted {source}");
            }
        }
    }

    #[test]
    fn enum_match_types_paths_scopes_and_nominal_identity() {
        let source = "mod m{pub enum E{A,B} pub fn make()->E{E::A}} use m::E as Alias; use Alias::A as First; struct S{e:Alias} fn f(x:Alias)->i32{match x{First=>1,Alias::B=>2}} fn main(){let s=S{e:First};let n=f(s.e);match true{true=>{},false=>{}} match (){()=>{}} let n=match n{-2147483648=>0,x=>x};let _=m::make();}";
        let program = parse(source).unwrap();
        let env = type_env(&program).unwrap();
        assert_ne!(env.structs["S"].def().id, env.enums["m::E"].def().id);
        assert!(!ir::Ty::Enum(env.enums["m::E"].clone()).is_copy());
        check(&program).unwrap();
        let checked = type_function_with_env(&program, 1, &env).unwrap();
        assert!(
            checked
                .exprs
                .iter()
                .any(|expr| matches!(&expr.kind,ir::ExprKind::Match{arms,..} if arms.len()==2))
        );
    }

    #[test]
    fn enum_match_exhaustiveness_and_dead_arm_obligations() {
        for source in [
            "enum E{A,B} fn main(){match E::A{E::A=>()};}",
            "fn main(){match true{true=>()};}",
            "fn main(){match 0{0=>()};}",
            "enum E{A} enum F{A} fn main(){match E::A{F::A=>()};}",
            "fn main(){match true{_=>1,true=>false};}",
            "fn main(){match true{_=>(),false=>{let x:bool=1;}};}",
            "fn main(){match 1{x=>x};x;}",
            "enum E{A,B} fn main(){let E::A=E::A;}",
            "fn f()->i32{loop{match true{_=>break 1,true=>break false}}} fn main(){}",
        ] {
            assert!(check(&parse(source).unwrap()).is_err(), "accepted {source}");
        }
        assert!(parse("enum E{A} use E::A; fn main(){match E::A{mut A=>()};}").is_err());
        for source in [
            "enum E{A,B} fn main(){match E::A{E::A=>(),E::A=>(),E::B=>(),_=>()};}",
            "enum E{A} use E::A; fn main(){let A=E::A;match A{A=>()};}",
            "fn f()->i32{match loop{} {}} fn main(){}",
            "enum E{A,B} fn f(e:E)->i32{match e{E::A{}=>return 1,E::B{}=>return 2}} fn main(){let e=E::A{};let n=f(e);}",
            "fn f()->i32{loop {match true{true=>{break 1;},false=>{break 2;},_=>{break 3;}}}} fn main(){}",
        ] {
            check(&parse(source).unwrap()).unwrap_or_else(|error| panic!("{source}: {error}"));
        }
        for source in [
            "enum E{A,B}use E::A;fn f(A:E){}fn main(){}",
            "enum E{A}use E::A;fn f(A:E)->E{A}fn main(){}",
        ] {
            assert!(
                parse(source)
                    .unwrap_err()
                    .contains("parameters are unsupported")
            );
        }
    }

    fn typed(source: &str, index: usize) -> ir::CheckedFunction {
        let program = parse(source).unwrap();
        let signatures = signature_table(&program).unwrap();
        let checked = type_function_with_signatures(&program, index, &signatures).unwrap();
        checked.validate().unwrap();
        checked
    }

    #[test]
    fn tuple_grouping_and_singleton_syntax_are_distinct() {
        let program = parse("fn f(x:((i32)),y:(bool,))->((i32,bool),()) { ((x,y.0),()) } fn main(){let a=(1);let b=(1,);}").unwrap();
        let f = &program.functions[0];
        assert_eq!(f.params[0].1, Type::Int(ir::IntTy::I32));
        assert_eq!(f.params[1].1, Type::Tuple(vec![Type::Bool]));
        assert_eq!(
            f.result,
            Type::Tuple(vec![
                Type::Tuple(vec![Type::Int(ir::IntTy::I32), Type::Bool]),
                Type::Unit
            ])
        );
        check(&program).unwrap();
        let f = typed("fn main(){let a=(1);let b=(1,);let z=((),((),));}", 0);
        assert_eq!(f.locals[0].ty, ir::Ty::Int(ir::IntTy::I32));
        assert_eq!(
            f.locals[1].ty,
            ir::Ty::Tuple(vec![ir::Ty::Int(ir::IntTy::I32)].into())
        );
        assert_eq!(
            f.locals[2].ty,
            ir::Ty::Tuple(vec![ir::Ty::Unit, ir::Ty::Tuple(vec![ir::Ty::Unit].into())].into())
        );
    }

    #[test]
    fn tuple_places_resolve_parenthesized_local_roots_and_shadowing() {
        let f = typed(
            "fn main(){let mut x=((1,true),());((x)).0.1=false;let mut x=(2,);x.0=3;}",
            0,
        );
        let places = f
            .body
            .statements
            .iter()
            .filter_map(|s| match s {
                ir::Stmt::AssignPlace { place, .. } => Some(place),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(places[0].local, ir::LocalId(0));
        assert_eq!(
            places[0].projection,
            vec![ir::Projection::Field(0), ir::Projection::Field(1)]
        );
        assert_eq!(places[1].local, ir::LocalId(1));
        assert_eq!(places[1].projection, vec![ir::Projection::Field(0)]);
        assert_eq!(f.locals[0].name, f.locals[1].name);
    }

    #[test]
    fn tuple_aggregate_flows_preserve_each_actual_type_and_projection_completion() {
        let f = typed("fn f()->i32 {({return 3;},0).1} fn main(){}", 0);
        let projected = &f.exprs[f.body.tail.unwrap().0];
        assert_eq!(projected.ty, ir::Ty::Int(ir::IntTy::I32));
        assert_eq!(projected.completion, ir::Completion::Never);
        let ir::ExprKind::Field { base, .. } = projected.kind else {
            panic!("expected projection")
        };
        assert_eq!(
            f.exprs[base.0].ty,
            ir::Ty::Tuple(vec![ir::Ty::Never, ir::Ty::Int(ir::IntTy::I32)].into())
        );
        let f = typed("fn main(){let x:(bool,i32)=({return;},0);}", 0);
        assert_eq!(
            f.locals[0].ty,
            ir::Ty::Tuple(vec![ir::Ty::Bool, ir::Ty::Int(ir::IntTy::I32)].into())
        );
        let f = typed("fn main(){let x=({return;false},0);}", 0);
        assert_eq!(
            f.locals[0].ty,
            ir::Ty::Tuple(vec![ir::Ty::Bool, ir::Ty::Int(ir::IntTy::I32)].into())
        );
        for source in [
            "fn main(){let x:(i32,i32)=({return;false},0);}",
            "fn main(){let x=if true {({return;false},0)} else {(1,0)};}",
            "fn main(){let x=loop {break ({return;false},0);break (1,0);};}",
        ] {
            assert!(check(&parse(source).unwrap()).is_err(), "{source}");
        }
    }

    #[test]
    fn tuple_calls_and_loop_breaks_keep_structural_signatures() {
        let f = typed(
            "mod m {pub fn f(x:(i32,bool))->(i32,bool){x}} fn main(){let t=loop {break if true {m::f((1,true))} else {(2,false)};};}",
            1,
        );
        let call = f
            .exprs
            .iter()
            .find(|e| matches!(e.kind, ir::ExprKind::Call { .. }))
            .unwrap();
        let ty = ir::Ty::Tuple(vec![ir::Ty::Int(ir::IntTy::I32), ir::Ty::Bool].into());
        assert_eq!(call.ty, ty);
        let ir::ExprKind::Call {
            name, signature, ..
        } = &call.kind
        else {
            unreachable!()
        };
        assert_eq!(name, "m::f");
        assert_eq!(signature.params, vec![ty.clone()]);
        assert_eq!(signature.result, ty);
        assert_eq!(f.locals[0].ty, ty);
    }

    #[test]
    fn tuple_invalid_fields_assignments_and_operators_are_rejected() {
        for source in [
            "fn main(){let x=(1,);x.0=2;}",
            "fn main(){let mut x=(1,);x.0=true;}",
            "fn main(){let mut x=(1,);x.1=2;}",
            "fn main(){let x=(1,);let y=x.0.0;}",
            "fn main(){let x=().0;}",
            "fn main(){let x=(1,)==(1,);}",
            "fn main(){let x=(1,)<(2,);}",
        ] {
            assert!(check(&parse(source).unwrap()).is_err(), "{source}");
        }
        for source in [
            "fn main(){(1,).0=2;}",
            "fn main(){let x=(1,).00;}",
            "fn main(){let x=(1,).0usize;}",
            "fn main(){let x=(1,).-1;}",
            "fn main(){let (x,..)=(1,2);}",
        ] {
            assert!(parse(source).is_err(), "{source}");
        }
    }

    #[test]
    fn tuple_type_nesting_and_nested_inference_are_checked() {
        let nested = format!("{}i32{}", "(".repeat(140), ",)".repeat(140));
        assert!(
            parse(&format!("fn f(x:{nested}){{}}fn main(){{}}"))
                .unwrap_err()
                .contains("nesting")
        );
        let mut program = parse("fn f(x:(i32,)){}fn main(){}").unwrap();
        program.functions[0].params[0].1 = Type::Tuple(vec![Type::Infer]);
        assert!(
            signature_table(&program)
                .unwrap_err()
                .contains("cannot be inferred")
        );
    }

    #[test]
    fn loop_control_has_distinct_resolved_targets_and_scalar_types() {
        let f = typed(
            "fn main(){ let x=loop { while true { continue; break; } loop { break; } break 3; }; let y=loop { break true; }; }",
            0,
        );
        assert_eq!(
            f.locals.iter().map(|l| l.ty.clone()).collect::<Vec<_>>(),
            vec![ir::Ty::Int(ir::IntTy::I32), ir::Ty::Bool]
        );
        let breaks = f
            .exprs
            .iter()
            .filter_map(|e| match e.kind {
                ir::ExprKind::Break { target, .. } => {
                    assert_eq!(e.flow(), None);
                    Some(target)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            breaks,
            vec![ir::LoopId(1), ir::LoopId(2), ir::LoopId(0), ir::LoopId(3)]
        );
        assert!(f.exprs.iter().any(|e| matches!(
            e.kind,
            ir::ExprKind::Continue {
                target: ir::LoopId(1)
            }
        )));
    }

    #[test]
    fn divergent_break_operand_contributes_only_its_nested_breaks() {
        let f = typed(
            "fn f()->i32 { loop { break { break 3; }; } } fn main(){}",
            0,
        );
        let root = &f.exprs[f.body.tail.unwrap().0];
        assert!(matches!(root.kind, ir::ExprKind::Loop { .. }));
        assert_eq!(root.flow(), Some(ir::Ty::Int(ir::IntTy::I32)));
        let f = typed(
            "fn f()->i32 { loop { break { return 3; }; } } fn main(){}",
            0,
        );
        assert_eq!(f.exprs[f.body.tail.unwrap().0].flow(), None);
        let f = typed("fn f()->i32 { loop { continue; break 3; } } fn main(){}", 0);
        assert_eq!(
            f.exprs[f.body.tail.unwrap().0].ty,
            ir::Ty::Int(ir::IntTy::I32)
        );
        assert_eq!(f.exprs[f.body.tail.unwrap().0].flow(), None);
        let f = typed("fn f()->i32 { loop { continue; } } fn main(){}", 0);
        assert_eq!(f.exprs[f.body.tail.unwrap().0].flow(), None);
    }

    #[test]
    fn typed_unreachable_tails_preserve_types_without_creating_loop_exits() {
        let f = typed(
            "fn f()->i32 { loop { break {return 3; 0}; } } fn main(){}",
            0,
        );
        let value = &f.exprs[f.body.tail.unwrap().0];
        assert_eq!(value.ty, ir::Ty::Int(ir::IntTy::I32));
        assert_eq!(value.completion, ir::Completion::Never);
        let f = typed("fn f()->i32 { !{return 3;} } fn main(){}", 0);
        let value = &f.exprs[f.body.tail.unwrap().0];
        assert_eq!(value.ty, ir::Ty::Never);
        assert_eq!(value.completion, ir::Completion::Never);
        let f = typed("fn main(){ let x={return; true}; }", 0);
        assert_eq!(f.locals[0].ty, ir::Ty::Bool);
        for source in [
            "fn f()->i32 {return 3; ()} fn main(){}",
            "fn main(){loop {break 1; false}}",
            "fn main(){let x:bool=loop {break {return; 0};};}",
        ] {
            assert!(check(&parse(source).unwrap()).is_err(), "{source}");
        }
    }

    #[test]
    fn explicit_semicolons_discard_blocklike_values() {
        let program = parse("fn main(){ {1}; if true {1} else {2}; loop {break 3;}; }").unwrap();
        check(&program).unwrap();
        assert!(
            program.functions[0]
                .body
                .statements
                .iter()
                .all(|s| matches!(s, Stmt::Expr(_)))
        );
        let program = parse("fn main(){ loop {break;} let x=1; }").unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            Stmt::ImplicitExpr(Expr::Loop { .. })
        ));
        check(&program).unwrap();
        for source in [
            "fn main(){ {1} let x=1; }",
            "fn main(){ if true {1} else {2} let x=1; }",
            "fn main(){ loop {break 3;} let x=1; }",
        ] {
            assert!(check(&parse(source).unwrap()).is_err(), "{source}");
        }
    }

    #[test]
    fn loop_obligations_reject_unreachable_conflicts_and_while_values() {
        for source in [
            "fn main(){ loop {break 3; break true;} }",
            "fn main(){ loop {break; break 3;} }",
            "fn main(){ loop {continue; break 3; break true;} }",
            "fn main(){ while true {break ();} }",
            "fn main(){ while true {break {return;};} }",
            "fn main(){ break; }",
            "fn main(){ continue; }",
            "fn main(){ loop {1} }",
        ] {
            let p = parse(source).unwrap();
            assert!(check(&p).is_err(), "{source}");
        }
    }

    #[test]
    fn while_conditions_are_control_target_barriers() {
        for source in [
            "fn main(){ while {break;} {} }",
            "fn main(){ while {continue;} {} }",
            "fn main(){ loop {while {break;} {}} }",
            "fn main(){ loop {while {continue;} {}} }",
        ] {
            let error = check(&parse(source).unwrap()).unwrap_err();
            assert!(error.contains("E0590"), "{error}");
        }
        let f = typed("fn main(){ while {loop {break true;}} {break;} }", 0);
        let ir::Stmt::While { id, .. } = f.body.statements[0] else {
            panic!("expected while")
        };
        assert_eq!(id, ir::LoopId(0));
        let breaks = f
            .exprs
            .iter()
            .filter_map(|e| match e.kind {
                ir::ExprKind::Break { target, .. } => Some(target),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(breaks, vec![ir::LoopId(1), ir::LoopId(0)]);
    }

    #[test]
    fn loop_walkers_resolve_break_value_calls_and_bound_depth() {
        let f = typed(
            "mod m {pub fn f()->i32{3}} fn main(){let x=loop {break m::f();};}",
            1,
        );
        assert!(
            f.exprs
                .iter()
                .any(|e| matches!(&e.kind, ir::ExprKind::Call { name, .. } if name == "m::f"))
        );
        let nested_breaks = "break ".repeat(140);
        let source = format!("fn main(){{{nested_breaks}0;}}");
        assert!(parse(&source).unwrap_err().contains("nesting"));
        let labeled = typed("fn main(){'outer: loop {break 'outer;}}", 0);
        assert!(labeled.exprs.iter().any(|e| matches!(
            e.kind,
            ir::ExprKind::Break {
                target: ir::LoopId(0),
                value: None
            }
        )));
    }

    #[test]
    fn inferred_shadowing_allocates_distinct_typed_locals() {
        let f = typed(
            "fn f(x:i32)->i32 { let x=x+1; let x=true; { let x=3; x; }; if x { 4 } else { 5 } } fn main(){}",
            0,
        );
        assert_eq!(f.params, vec![ir::LocalId(0)]);
        assert_eq!(
            f.locals.iter().map(|l| l.ty.clone()).collect::<Vec<_>>(),
            vec![
                ir::Ty::Int(ir::IntTy::I32),
                ir::Ty::Int(ir::IntTy::I32),
                ir::Ty::Bool,
                ir::Ty::Int(ir::IntTy::I32)
            ]
        );
        assert!(f.locals.iter().all(|l| l.name == "x"));
        let reads = f
            .exprs
            .iter()
            .filter_map(|e| match e.kind {
                ir::ExprKind::Local(id) => Some(id),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(reads, vec![ir::LocalId(0), ir::LocalId(3), ir::LocalId(2)]);
    }

    #[test]
    fn assignment_and_initializer_resolve_before_shadowing() {
        let f = typed("fn main(){ let mut x=1; x=x+1; let x=x==2; x; }", 0);
        assert!(f.locals[0].mutable);
        assert!(!f.locals[1].mutable);
        assert_eq!(f.locals[1].ty, ir::Ty::Bool);
        assert!(matches!(
            f.body.statements[1],
            ir::Stmt::Assign {
                local: ir::LocalId(0),
                ..
            }
        ));
        let reads = f
            .exprs
            .iter()
            .filter_map(|e| match e.kind {
                ir::ExprKind::Local(id) => Some(id),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(reads, vec![ir::LocalId(0), ir::LocalId(0), ir::LocalId(1)]);
    }

    #[test]
    fn canonical_calls_keep_validated_signature() {
        let f = typed(
            "mod m { pub fn f(x:bool)->i32 { if x { 1 } else { 2 } } } use m::f as g; fn main(){ let n=g(true); }",
            1,
        );
        let call = f
            .exprs
            .iter()
            .find(|e| matches!(e.kind, ir::ExprKind::Call { .. }))
            .unwrap();
        assert_eq!(call.flow(), Some(ir::Ty::Int(ir::IntTy::I32)));
        let ir::ExprKind::Call {
            name,
            args,
            signature,
            target,
        } = &call.kind
        else {
            unreachable!()
        };
        assert_eq!(name, "m::f");
        assert!(matches!(target,ir::Callee::Direct(t) if t.type_args.is_empty()));
        assert_eq!(signature.params, vec![ir::Ty::Bool]);
        assert_eq!(signature.result, ir::Ty::Int(ir::IntTy::I32));
        assert_eq!(f.exprs[args[0].0].ty, ir::Ty::Bool);
        assert_eq!(f.locals[0].ty, ir::Ty::Int(ir::IntTy::I32));
    }

    #[test]
    fn divergence_and_short_circuit_keep_existing_completion() {
        let f = typed(
            "fn f()->bool { let x=false && { return true; }; let y=true || { return false; }; let z={ return x; }; } fn main(){}",
            0,
        );
        assert_eq!(
            f.locals.iter().map(|l| l.ty.clone()).collect::<Vec<_>>(),
            vec![ir::Ty::Bool, ir::Ty::Bool, ir::Ty::Never]
        );
        let binaries = f
            .exprs
            .iter()
            .filter(|e| matches!(e.kind, ir::ExprKind::Binary { .. }))
            .collect::<Vec<_>>();
        assert!(binaries.iter().all(|e| e.flow() == Some(ir::Ty::Bool)));
        let never = f
            .exprs
            .iter()
            .filter(|e| e.completion == ir::Completion::Never)
            .collect::<Vec<_>>();
        assert_eq!(never.len(), 3);
        assert!(
            never
                .iter()
                .all(|e| e.ty == ir::Ty::Never && e.flow().is_none())
        );
    }

    #[test]
    fn lowering_and_checking_reject_unreachable_errors_identically() {
        for source in [
            "fn main(){ return; let x=true+1; }",
            "fn main(){ return; let x:bool=1; }",
            "fn main(){ if true { 1 } else { 2 } let x=0; }",
            "fn f()->i32 { while true { return 1; } } fn main(){}",
        ] {
            let program = parse(source).unwrap();
            let signatures = signature_table(&program).unwrap();
            let lowered = type_function_with_signatures(&program, 0, &signatures).unwrap_err();
            assert_eq!(
                check_function_with_signatures(&program, 0, &signatures).unwrap_err(),
                lowered
            );
        }
    }
}

#[cfg(test)]
mod prepared_location_tests {
    use super::*;

    fn parser(tokens: &[Token]) -> Parser<'_> {
        Parser::new(tokens)
    }

    #[test]
    fn token_bytes_preserve_unicode_comments_strings_and_crlf() {
        let text = "/* λ */\r\n\"é\r\n水\" >>=";
        let tokens = lex(text).unwrap();
        let string_start = text.find('"').unwrap();
        let angle_start = text.find(">>=").unwrap();
        assert_eq!(tokens[0].bytes, string_start..angle_start - 1);
        assert_eq!(tokens[0].text, "é\n水");
        assert_eq!((tokens[0].line, tokens[0].column), (2, 1));
        assert_eq!(tokens[1].bytes, angle_start..angle_start + 3);
        assert_eq!((tokens[1].line, tokens[1].column), (3, 4));
        assert_eq!(tokens[2].bytes, text.len()..text.len());
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
            let mut parser = parser(&tokens);
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
        let mut parser = parser(&tokens);
        let error = parser.ty().unwrap_err();
        assert_eq!(tokens[0].bytes, 0..2);
        assert_eq!(tokens[1].bytes, 2..4);
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
            let mut parser = parser(&tokens);
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
            let mut parser = parser(&tokens);
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
        assert!(error.message.contains("crosses or lacks"));
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
            "fn unused() { let value = 'x'; }",
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
mod tests {
    use super::*;

    #[test]
    fn tuple_patterns_preserve_old_scope_and_simultaneous_fresh_bindings() {
        let program = parse("fn f()->i32{7} fn main(){let x=1;let ((mut x,),y):((i32,),bool)=((x,),true);x=2;let _=x;let _y=y;let (f,z)=(f(),f());}").unwrap();
        let env = type_env(&program).unwrap();
        let checked = type_function_with_env(&program, 1, &env).unwrap();
        checked.validate().unwrap();
        let ir::Stmt::LetPattern {
            value, bindings, ..
        } = &checked.body.statements[1]
        else {
            panic!("expected tuple pattern")
        };
        assert_eq!(
            bindings
                .iter()
                .map(|binding| binding
                    .projection
                    .iter()
                    .map(|p| match p {
                        ir::Projection::Field(index) => *index,
                        _ => panic!("unexpected downcast"),
                    })
                    .collect::<Vec<_>>())
                .collect::<Vec<_>>(),
            vec![vec![0, 0], vec![1]]
        );
        assert_eq!(
            bindings
                .iter()
                .map(|binding| binding.local)
                .collect::<Vec<_>>(),
            vec![ir::LocalId(1), ir::LocalId(2)]
        );
        assert!(checked.locals[1].mutable);
        assert_eq!(checked.locals[2].ty, ir::Ty::Bool);
        assert!(
            checked.exprs[..value.0]
                .iter()
                .any(|expr| matches!(expr.kind, ir::ExprKind::Local(ir::LocalId(0))))
        );
        let ir::Stmt::LetPattern { bindings, .. } = &checked.body.statements[3] else {
            panic!("expected wildcard")
        };
        assert!(bindings.is_empty());
        assert_eq!(
            checked
                .locals
                .iter()
                .filter(|local| local.name == "x")
                .count(),
            2
        );
        assert!(
            parse("fn f()->i32{1} fn main(){let (f,z)=(f(),2);f();}")
                .unwrap_err()
                .contains("not callable")
        );
        check(
            &parse(
                "fn main(){let(x,y)=(1,2);let (z,)=(x,);let ()=();let (mut grouped)=z;grouped=y;}",
            )
            .unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn struct_patterns_publish_source_order_paths_and_nominal_types() {
        let program = parse("struct Inner{x:i32} struct S{a:Inner,b:bool} use S as Alias; fn main(){let s=S{a:Inner{x:1},b:true};let Alias{b:mut flag,a:Inner{x:n}}:S=s;flag=false;let Alias{..}=S{a:Inner{x:n},b:flag};}").unwrap();
        let env = type_env(&program).unwrap();
        let checked = type_function_with_env(&program, 0, &env).unwrap();
        checked.validate().unwrap();
        let ir::Stmt::LetPattern { ty, bindings, .. } = &checked.body.statements[1] else {
            panic!("expected struct pattern")
        };
        assert_eq!(*ty, ir::Ty::Struct(env.structs["S"].clone()));
        assert_eq!(
            bindings
                .iter()
                .map(|binding| binding
                    .projection
                    .iter()
                    .map(|p| match p {
                        ir::Projection::Field(index) => *index,
                        _ => panic!("unexpected downcast"),
                    })
                    .collect::<Vec<_>>())
                .collect::<Vec<_>>(),
            vec![vec![1], vec![0, 0]]
        );
        assert_eq!(checked.locals[bindings[0].local.0].name, "flag");
        assert!(checked.locals[bindings[0].local.0].mutable);
        assert_eq!(
            checked.locals[bindings[1].local.0].ty,
            ir::Ty::Int(ir::IntTy::I32)
        );
    }

    #[test]
    fn pattern_shape_duplicate_annotation_and_privacy_errors_reject_even_when_dead() {
        for source in [
            "fn main(){let(x,x)=(1,2);}",
            "fn main(){let(x,y)=(1,);}",
            "fn main(){let ()=(1,);}",
            "fn main(){let(x,y):(i32,bool)=(1,2);}",
            "fn main(){let(x,y)=(1,true);x=false;}",
            "struct S{x:i32,y:bool} fn main(){let S{x}=S{x:1,y:true};}",
            "struct S{x:i32} fn main(){let S{x:_,x:_}=S{x:1};}",
            "struct S{x:i32} fn main(){let S{missing:_,..}=S{x:1};}",
            "struct S{x:i32} struct T{x:i32} fn main(){let S{x}=T{x:1};}",
            "mod m{pub struct S{private:i32} pub fn make()->S{S{private:1}}} fn main(){let m::S{private:_,..}=m::make();}",
            "mod m{struct H{x:i32} pub struct S{pub hidden:H} pub fn make()->S{S{hidden:H{x:1}}}} fn main(){let m::S{hidden:_,..}=m::make();}",
            "fn main(){return;let(x,x)=(1,2);}",
        ] {
            assert!(
                parse(source).and_then(|program| check(&program)).is_err(),
                "accepted {source}"
            );
        }
        check(&parse("mod m{struct H{x:i32} pub struct S{pub hidden:H} pub fn make()->S{S{hidden:H{x:1}}}} fn main(){let m::S{..}=m::make();}").unwrap()).unwrap();
    }

    #[test]
    fn never_patterns_use_explicit_shape_and_preserve_dead_value_types() {
        for source in [
            "fn main(){let(x,y):(i32,bool)=loop{};}",
            "struct S{x:i32} fn main(){let S{x}=loop{};}",
            "fn main(){let _=loop{};}",
            "fn main(){let ()=loop{};}",
            "struct S{x:i32} fn main(){let(S{x},())=loop{};}",
            "struct S{x:i32} fn main(){let (S{x},)=({return;},);}",
        ] {
            let program = parse(source).unwrap();
            let env = type_env(&program).unwrap();
            type_function_with_env(&program, 0, &env)
                .unwrap()
                .validate()
                .unwrap();
        }
        let error = check(&parse("fn main(){let(x,y)=loop{};}").unwrap()).unwrap_err();
        assert!(error.contains("inference is unsupported"));
        assert!(
            check(&parse("fn main(){let(x,y):(i32,bool)={return;(false,0)};}").unwrap()).is_err()
        );
    }

    #[test]
    fn unsupported_patterns_and_pattern_nesting_are_explicit() {
        for source in [
            "fn main(){let(x,..)=(1,2);}",
            "fn main(){let ref x=1;}",
            "fn main(){let x @ _=1;}",
            "fn main(){let 1=1;}",
            "fn main(){let(mut _)=1;}",
            "fn main(){let(x,y)=(1,2) else {};}",
            "struct S{x:i32} fn main(){let S{..,x}=S{x:1};}",
            "struct S{x:i32} fn main(){let S{..,}=S{x:1};}",
            "struct S{x:i32} fn main(){let S{x,..,}=S{x:1};}",
            "struct S{x:i32} fn main(){let S{. .}=S{x:1};}",
            "fn f((x,y):(i32,i32)){} fn main(){}",
        ] {
            assert!(parse(source).is_err(), "accepted {source}");
        }
        let source = format!(
            "fn main(){{let {}x{}=1;}}",
            "(".repeat(140),
            ")".repeat(140)
        );
        assert!(parse(&source).unwrap_err().contains("nesting"));
        let mut program = parse("fn main(){let _=1;}").unwrap();
        let mut pattern = Pattern::Wildcard;
        for _ in 0..140 {
            pattern = Pattern::Tuple(vec![pattern]);
        }
        let Stmt::LetPattern {
            pattern: target, ..
        } = &mut program.functions[0].body.statements[0]
        else {
            unreachable!()
        };
        *target = pattern;
        assert!(check(&program).unwrap_err().contains("nesting"));
    }

    #[test]
    fn structs_share_nominal_handles_within_one_environment_only() {
        let program = parse("struct S{x:i32} use S as Alias; fn id(x:Alias)->S{x} fn main(){let x=Alias{x:1};let y=id(x);y.x;}").unwrap();
        let env = type_env(&program).unwrap();
        let ty = ir::Ty::Struct(env.structs["S"].clone());
        assert_eq!(env.functions["id"].params, vec![ty.clone()]);
        assert_eq!(env.functions["id"].result, ty);
        assert!(!ty.is_copy());
        assert!(!ir::Ty::Tuple(vec![ty.clone()].into()).is_copy());
        let separate = type_env(&program).unwrap();
        assert_ne!(env.structs["S"], separate.structs["S"]);
        for index in 0..program.functions.len() {
            type_function_with_env(&program, index, &env).unwrap();
        }
    }

    #[test]
    fn struct_initializers_retain_source_order_and_checked_field_indices() {
        let program =
            parse("struct S{a:i32,b:bool} fn main(){let mut s=S{b:true,a:3};s.a=4;s.b;}").unwrap();
        let env = type_env(&program).unwrap();
        let body = type_function_with_env(&program, 0, &env).unwrap();
        let fields = body
            .exprs
            .iter()
            .find_map(|expr| match &expr.kind {
                ir::ExprKind::Struct { fields, .. } => Some(fields),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            fields.iter().map(|(index, _)| *index).collect::<Vec<_>>(),
            vec![1, 0]
        );
        assert!(
            body.body.statements.iter().any(
                |stmt| matches!(stmt,ir::Stmt::AssignPlace{place,..} if place.projection==vec![ir::Projection::Field(0)])
            )
        );
    }

    #[test]
    fn structs_reject_invalid_unused_declarations_and_nominal_mismatches() {
        for source in [
            "struct A{x:A} fn main(){}",
            "struct A{x:(B,)} struct B{x:A} fn main(){}",
            "struct A{x:Missing} fn main(){}",
            "struct A{x:i32,x:bool} fn main(){}",
            "struct A{} fn main(){}",
            "struct A{x:i32} struct B{x:i32} fn main(){let x:A=B{x:1};}",
            "struct A{x:i32} struct B{x:i32} fn main(){if true{A{x:1}}else{B{x:1}};}",
            "struct A{x:i32} fn main(){A{x:true};}",
            "struct A{x:i32} fn main(){A{};}",
            "struct A{x:i32} fn main(){A{x:1,x:2};}",
            "struct A{x:i32} fn main(){A{x:1,y:2};}",
            "struct A{x:i32} fn main(){A{x:1}==A{x:1};}",
        ] {
            assert!(
                parse(source).and_then(|program| check(&program)).is_err(),
                "accepted {source}"
            );
        }
    }

    #[test]
    fn struct_privacy_and_type_value_namespaces_are_distinct() {
        let legal = "mod m{pub struct S{x:i32} pub fn S()->i32{7} pub fn make()->S{S{x:1}} mod child{fn read(s:super::S)->i32{s.x}}} use m::S; fn main(){let s=m::make();S();}";
        check(&parse(legal).unwrap()).unwrap();
        for source in [
            "mod m{pub struct S{x:i32}} fn main(){m::S{x:1};}",
            "mod m{pub struct S{x:i32} pub fn make()->S{S{x:1}}} fn main(){m::make().x;}",
            "mod m{struct S{pub x:i32}} fn main(){let x:m::S=loop{};}",
            "mod m{} fn main(){let x:m=loop{};}",
            "mod S{} struct S{x:i32} fn main(){}",
        ] {
            assert!(
                parse(source).and_then(|program| check(&program)).is_err(),
                "accepted {source}"
            );
        }
    }

    #[test]
    fn inferred_private_types_are_not_exposed_through_calls_or_public_fields() {
        for source in [
            "mod m{struct S{pub x:i32} pub fn make()->S{S{x:1}}} fn main(){m::make();}",
            "mod m{struct H{pub x:i32} pub struct O{pub inner:H} pub fn make()->O{O{inner:H{x:1}}}} fn main(){m::make().inner;}",
            "mod m{struct H{x:i32} pub fn take(x:H){}} fn main(){m::take(loop{});}",
            "mod m{struct H{pub x:i32} pub struct O{pub inner:H} pub fn make()->O{O{inner:H{x:1}}}} fn main(){let mut s=m::make();s.inner.x=2;}",
            "mod m{struct H{pub x:i32} pub struct O{pub inner:H} pub fn make()->O{O{inner:H{x:1}}}} fn main(){let mut s=m::make();s.inner=loop{};}",
            "mod m{struct H{pub x:i32} pub struct O{pub inner:H}} fn main(){m::O{inner:loop{}};}",
        ] {
            assert!(
                parse(source).and_then(|program| check(&program)).is_err(),
                "accepted {source}"
            );
        }
    }

    #[test]
    fn struct_conditions_allow_parenthesized_literals_and_nested_blocks() {
        check(&parse("struct S{x:bool} fn test(s:S)->bool{s.x} fn main(){if (S{x:true}).x {} while test(S{x:false}) {} if {let s=S{x:true};s.x} {}}").unwrap()).unwrap();
    }

    #[test]
    fn struct_expanded_depth_is_bounded_independent_of_declaration_order() {
        let mut source = "struct S0{x:i32}".to_owned();
        for index in 1..140 {
            source.push_str(&format!("struct S{index}{{x:S{}}}", index - 1));
        }
        source.push_str("fn main(){}");
        let program = parse(&source).unwrap();
        assert!(type_env(&program).unwrap_err().contains("nesting"));
        let mut source = "struct S0{x:i32}".to_owned();
        for index in 1..40 {
            source.push_str(&format!(
                "struct S{index}{{a:S{},b:S{}}}",
                index - 1,
                index - 1
            ));
        }
        source.push_str("fn f(x:S39){x;} fn main(){}");
        let program = parse(&source).unwrap();
        let env = type_env(&program).unwrap();
        // This fixture intentionally has an exponentially large value layout.
        // Its shared semantic DAG is shallow, independent of backend/analysis
        // resource limits when expanding concrete move paths or storage.
        type_function_with_env(&program, 0, &env).unwrap();
    }

    #[test]
    fn struct_dependency_width_is_not_dependency_depth() {
        let fields = (0..140)
            .map(|i| format!("f{i}:Leaf"))
            .collect::<Vec<_>>()
            .join(",");
        let program = parse(&format!(
            "struct Leaf{{x:i32}} struct Wide{{{fields}}} fn main(){{}}"
        ))
        .unwrap();
        let env = type_env(&program).unwrap();
        assert_eq!(env.structs["Wide"].def().fields.len(), 140);
        let values = (0..140)
            .rev()
            .map(|i| format!("f{i}:Leaf{{x:1}}"))
            .collect::<Vec<_>>()
            .join(",");
        let program = parse(&format!(
            "struct Leaf{{x:i32}} struct Wide{{{fields}}} fn main(){{Wide{{{values}}};}}"
        ))
        .unwrap();
        check(&program).unwrap();
    }

    #[test]
    fn module_paths_are_canonical_and_main_stays_root() {
        let p = parse(
            "mod a { fn hidden()->i32{1} pub mod b { pub fn run()->i32 {super::hidden()+super::super::root()} } pub fn run()->i32{self::b::run()} } fn root()->i32{2} fn main(){let x=crate::a::run(); let y=a::b::run();}",
        ).unwrap();
        check(&p).unwrap();
        let names: Vec<_> = p.functions.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["a::hidden", "a::b::run", "a::run", "root", "main"]);
        let Expr::Binary { left, right, .. } = p.functions[1].body.tail.as_deref().unwrap() else {
            panic!()
        };
        assert!(matches!(left.as_ref(), Expr::Call { name, .. } if name == "a::hidden"));
        assert!(matches!(right.as_ref(), Expr::Call { name, .. } if name == "root"));
    }
    #[test]
    fn syntax_and_resolution_are_separate() {
        let parsed = parse_unresolved("use crate::missing as alias; fn main(){nope();}").unwrap();
        assert_eq!(parsed.program.functions[0].name, "main");
        assert!(resolve(&parsed).unwrap_err().contains("unresolved"));
        let parsed = parse_unresolved("mod a{fn f(){}} fn main(){a::f();}").unwrap();
        assert!(resolve(&parsed).unwrap_err().contains("private"));
        // Body type errors remain independent of declaration/name preparation.
        let parsed = parse_unresolved("fn bad()->i32{true} fn main(){}").unwrap();
        let p = resolve(&parsed).unwrap();
        let signatures = signature_table(&p).unwrap();
        assert!(check_function_with_signatures(&p, 0, &signatures).is_err());
        check_function_with_signatures(&p, 1, &signatures).unwrap();
    }
    #[test]
    fn forward_function_and_module_aliases() {
        good(
            "use next as renamed; use crate::a as next; use renamed::f as call; mod a{pub fn f()->i32{4}} fn main(){let x=call(); let y=renamed::f();}",
        );
        good(
            "mod a { fn f(){} mod child {use super::f as call; pub fn run(){call();}} pub fn run(){child::run();}} fn main(){a::run();}",
        );
        good("mod a{use self as me; pub fn f(){}} fn main(){}");
        good("use crate as root; fn f(){} fn main(){root::f();}");
        bad("use other as one; use one as other; fn main(){}");
        bad("use crate::absent; fn main(){}");
        bad("mod a{fn f(){}} use crate::a::f as unused; fn main(){}");
    }
    #[test]
    fn private_items_accessible_only_from_owner_and_descendants() {
        good(
            "mod a{fn f(){} mod inner{pub fn g(){super::f();}} pub fn run(){inner::g();}} fn main(){a::run();}",
        );
        good(
            "mod a{fn f(){} use self::f as alias; mod b{pub fn f(){super::alias();}} pub fn run(){b::f();}} fn main(){a::run();}",
        );
        bad("mod a{fn f(){}} mod b{pub fn run(){crate::a::f();}} fn main(){b::run();}");
        bad("mod a{mod hidden{pub fn f(){}}} fn main(){a::hidden::f();}");
        bad("mod a{pub fn f(){} use self::f as alias;} fn main(){a::alias();}");
        // Component boundaries, not textual prefixes, establish descendants.
        bad("mod a{fn f(){}} mod ab{pub fn g(){crate::a::f();}} fn main(){}");
    }
    #[test]
    fn module_scopes_do_not_inherit_unqualified_parent_items() {
        bad("fn f(){} mod a{pub fn g(){f();}} fn main(){}");
        bad("mod a{pub fn f(){}} mod b{pub fn g(){a::f();}} fn main(){}");
        good("mod a{pub fn f(){}} mod b{use crate::a; pub fn g(){a::f();}} fn main(){b::g();}");
        bad("fn main(){super::f();}");
        bad("mod a{pub fn f(){super::super::g();}} fn main(){}");
        bad("mod a{fn main(){}}");
    }
    #[test]
    fn source_shadowing_is_checked_before_canonicalization() {
        good("fn f(){} fn main(){let f=1;crate::f();self::f();}");
        good("mod a{pub fn f(){}} use a::f as alias; fn main(){let f=1;alias();}");
        good("mod a{pub fn f(){}} fn main(){let a=1;a::f();}");
        good("fn f()->i32{1} fn main(){let f=f();}");
        bad("fn f(){} fn main(){let f=1;f();}");
        bad("mod a{pub fn f(){}} use a::f as alias; fn main(){let alias=1;alias();}");
        bad("mod a{fn f(){} pub fn g(f:i32){f();}} fn main(){}");
        good("fn f(){} fn main(){{let f=1;}f();}");
    }
    #[test]
    fn separate_namespaces_and_import_collisions() {
        good(
            "mod same{pub fn value()->i32{3}} fn same()->i32{7} fn main(){let x=same()+same::value();}",
        );
        good(
            "mod same{pub fn value()->i32{3}} fn same()->i32{7} use crate::same as alias; fn main(){let x=alias()+alias::value();}",
        );
        good("mod a{pub fn f(){}} use a::f as a; fn main(){a();a::f();}");
        good(
            "mod outer{mod a{pub fn f(){}} use a::f as a; pub fn run(){a();a::f();}} fn main(){outer::run();}",
        );
        good(
            "mod a{pub mod same{pub fn f(){}} fn same(){}} use a::same as alias; fn main(){alias::f();}",
        );
        good("mod a{mod same{} pub fn same(){}} use a::same as alias; fn main(){alias();}");
        bad("mod a{mod same{} fn same(){}} use a::same as alias; fn main(){}");
        good("mod a{} fn f(){} use a as x; use f as x; fn main(){x();}");
        bad("fn f(){} use f as x; use f as x; fn main(){}");
        bad("mod a{} use a as x; mod x{} fn main(){}");
        bad("fn f(){} use f as x; fn x(){} fn main(){}");
    }
    #[test]
    fn grouped_use_trees_preserve_paths_and_identity() {
        good(
            "mod api{pub struct S{pub x:i32} pub mod inner{pub fn f()->i32{3}}} use crate::{api::{self as a,S as T,inner::{f as call,},},}; fn id(s:T)->a::S{s} fn main(){let s=id(T{x:call()});let n=s.x;}",
        );
        good(
            "mod a{fn f(){} mod b{use super::{f as call};use crate::{a as root};pub fn run(){call();root::f();}}pub fn run(){b::run();}}fn main(){a::run();}",
        );
        good(
            "mod a{pub fn f(){}} use {a::{self as first,self as second,f},};fn main(){f();first::f();second::f();}",
        );
        good("enum E{A,B}use E::{A as First,B,};fn main(){let n=match First{First=>1,B=>2};}");
        good("mod a{} use {};use a::{};fn main(){}");
        good("mod a{pub mod b{}}use a::{b::self as x};fn main(){}");
        good("trait T{fn f(self);}use T::{};use T::{self as Alias};fn main(){}");
        good(
            "fn f(){}use {self as root};use crate::{self as other};fn main(){root::f();other::f();}",
        );
        good(
            "use {next as renamed};use crate::{a as next};use renamed::{f as call};mod a{pub fn f(){}}fn main(){call();}",
        );
        good(
            "mod a{fn f(){}mod b{mod c{use super::super::{f as call};pub fn run(){call();}}pub fn run(){self::c::run();}}pub fn run(){b::run();}}fn main(){a::run();}",
        );
    }
    #[test]
    fn grouped_self_is_type_namespace_only() {
        good(
            "mod a{pub fn f(){}}fn a(){}use a::{self as alias};fn alias(){}fn main(){alias();alias::f();}",
        );
        bad("fn f(){}use f::{self as alias};fn main(){}");
        bad("mod a{}fn a(){}use a::{self as alias};fn main(){alias();}");
        bad("struct S{x:i32}use S::{self as T};fn id(s:T)->S{s}fn main(){let s=id(T{x:1});}");
    }
    #[test]
    fn grouped_imports_reject_collisions_privacy_and_cycles() {
        for source in [
            "mod a{pub fn f(){}}use a::{f,f};fn main(){}",
            "mod a{pub fn f(){}pub fn g(){}}use a::{f as x,g as x};fn main(){}",
            "mod a{fn f(){}}use a::{f};fn main(){}",
            "mod a{pub mod b{pub fn f(){}}}mod c{use crate::a::{b as alias};}fn main(){c::alias::f();}",
            "use {a as b,b as a};fn main(){}",
            "use missing::{};fn main(){}",
            "mod a{mod hidden{}}use a::hidden::{};fn main(){}",
        ] {
            bad(source);
        }
        good("mod a{}fn f(){}use {a as x,f as x};fn main(){x();}");
    }
    #[test]
    fn grouped_use_syntax_and_resource_fences() {
        for source in ["use a::{*};fn main(){}", "use a::{f,*};fn main(){}"] {
            assert!(parse(source).unwrap_err().contains("glob imports"));
        }
        for source in [
            "use a::{,};fn main(){}",
            "use a::{f,,};fn main(){}",
            "use a::{f g};fn main(){}",
            "use a::{f} as x;fn main(){}",
            "use a{f};fn main(){}",
        ] {
            assert!(parse_unresolved(source).is_err(), "accepted {source}");
        }
        let nested = format!("use {}f{};fn main(){{}}", "{".repeat(129), "}".repeat(129));
        assert!(parse_unresolved(&nested).unwrap_err().contains("nesting"));
        let wide = format!("use {{{}}};fn main(){{}}", "f,".repeat(4097));
        assert!(parse_unresolved(&wide).unwrap_err().contains("work"));
        let long = "a".repeat(1024 * 1024 + 1);
        assert!(
            parse_unresolved(&format!("use {long};fn main(){{}}"))
                .unwrap_err()
                .contains("allocation")
        );
    }
    #[test]
    fn grouped_use_whole_source_expansion_is_bounded() {
        let many = format!("{}fn main(){{}}", "use a::{};".repeat(65537));
        assert!(
            parse_unresolved(&many)
                .unwrap_err()
                .contains("count exceeds")
        );
        let prefix = "a".repeat(40000);
        let expanded = format!(
            "mod {prefix}{{{}}}fn main(){{}}",
            "use crate::{};".repeat(420)
        );
        assert!(
            parse_unresolved(&expanded)
                .unwrap_err()
                .contains("logical expansion")
        );
    }
    #[test]
    fn public_named_reexports_preserve_canonical_identity() {
        good(
            "mod hidden{pub fn f()->i32{7}pub struct S{pub x:i32}pub trait T{fn f(self);}} pub use hidden::{f as call,S as Alias,T};fn id(s:Alias)->hidden::S{s}fn main(){let s=id(Alias{x:call()});}",
        );
        good(
            "pub use next as first;pub use hidden::f as next;mod hidden{pub fn f(){}}fn main(){first();next();}",
        );
        good(
            "mod hidden{pub mod api{pub fn f(){}}}pub use hidden::{api::{self as exported,f as call}};fn main(){exported::f();call();}",
        );
        good(
            "pub mod a{pub fn f(){}}fn a(){}pub use a::{self as alias};fn alias(){}fn main(){alias();alias::f();}",
        );
        good("mod a{}pub fn f(){}pub use {f as x};use a as x;fn main(){x();}");
        good("pub mod a{pub use self as alias;}fn main(){}");
        good("pub use crate::{self as root};fn main(){}");
        good(
            "mod a{pub struct Mixed{pub x:i32}fn Mixed(){}pub use self::Mixed as Alias;pub fn run(){Alias();}}fn main(){a::run();let x=a::Alias{x:1};}",
        );
        good("mod a{pub enum E{A}}pub use a::E::A;fn main(){let x=A;}");
    }
    #[test]
    fn restricted_visibility_normalizes_ast_and_registry_scopes() {
        let program = parse(
            "mod a{pub(crate) struct S{pub(super) x:i32}pub(in crate::a) enum E{A}pub(self) trait T{fn f(self);}mod b{pub(in super::super) fn f(){}}}fn main(){}",
        ).unwrap();
        assert_eq!(
            program.structs[0].visibility,
            ir::Visibility::Restricted(String::new())
        );
        assert_eq!(
            program.structs[0].fields[0].visibility,
            ir::Visibility::Restricted(String::new())
        );
        assert_eq!(
            program.enums[0].visibility,
            ir::Visibility::Restricted("a".into())
        );
        assert_eq!(
            program.traits[0].visibility,
            ir::Visibility::Restricted("a".into())
        );
        assert_eq!(
            program.functions[0].visibility,
            ir::Visibility::Restricted(String::new())
        );
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        for info in &plan.registry.declarations {
            match info.name.as_str() {
                "a::S" | "a::b::f" => {
                    assert_eq!(info.visibility, ir::Visibility::Restricted(String::new()))
                }
                "a::E" | "a::T" => {
                    assert_eq!(info.visibility, ir::Visibility::Restricted("a".into()))
                }
                _ => {}
            }
        }
    }
    #[test]
    fn restricted_visibility_rejects_unused_nonlexical_paths_and_explicit_trait_items() {
        for source in [
            "mod a{}mod b{pub(in crate::a) fn f(){}}fn main(){}",
            "mod a{use crate::a as alias;mod b{pub(in crate::a::alias) fn f(){}}}fn main(){}",
            "mod a{pub(in self::super) fn f(){}}fn main(){}",
            "pub(super) fn f(){}fn main(){}",
            "trait T{pub(self) fn f(self);}fn main(){}",
            "trait T{fn f(self);}struct S{x:i32}impl T for S{pub(self) fn f(self){}}fn main(){}",
            "enum E{A{pub(self) x:i32}}fn main(){}",
        ] {
            assert!(parse_unresolved(source).is_err(), "accepted {source}");
        }
        let oversized = format!(
            "pub(in crate::{}) fn f(){{}}fn main(){{}}",
            "a::".repeat(1024) + "a"
        );
        assert!(
            parse_unresolved(&oversized)
                .unwrap_err()
                .contains("visibility path nesting")
        );
    }
    #[test]
    fn restricted_visibility_scope_heap_is_charged_before_program_clones() {
        let public = parse("pub fn main(){}").unwrap();
        let baseline = ir::PlanBudget::default();
        preflight_program_clone(&public, &baseline).unwrap();
        let bytes = baseline.usage_snapshot().bytes;
        let mut restricted = public.clone();
        restricted.functions[0].visibility = ir::Visibility::Restricted("a".repeat(4096));
        let budget = ir::PlanBudget::new(ir::PlanLimits {
            bytes: bytes + 4095,
            ..Default::default()
        });
        assert!(preflight_program_clone(&restricted, &budget).is_err());
    }
    #[test]
    fn module_parser_depth_limit_uses_an_ordinary_stack() {
        for depth in [128, 129, 300] {
            let source = format!(
                "{}{}fn main(){{}}",
                "mod a{".repeat(depth),
                "}".repeat(depth)
            );
            let result = parse_unresolved(&source);
            if depth == 128 {
                assert!(result.is_ok(), "{result:?}");
            } else {
                assert!(
                    result
                        .unwrap_err()
                        .contains("module nesting exceeds limit 128")
                );
            }
        }
    }
    #[test]
    fn public_reexports_validate_unused_originals_and_aliases() {
        for source in [
            "enum E{A}pub use E::A;fn main(){}",
            "mod a{pub struct Mixed{pub x:i32}fn Mixed(){}pub use self::Mixed as Alias;}fn main(){a::Alias();}",
            "fn f(){}pub use f as alias;fn main(){}",
            "struct S{x:i32}pub use S as Alias;fn main(){}",
            "enum E{A}pub use E as Alias;fn main(){}",
            "trait T{fn f(self);}pub use T as Alias;fn main(){}",
            "mod a{}pub use a as alias;fn main(){}",
            "pub fn f(){}use f as private;pub use private as alias;fn main(){}",
            "mod a{pub fn f(){}use self::f as private;pub use self::private as alias;}fn main(){}",
            "mod a{pub fn f(){}use self::f as private;}pub use a::private as alias;fn main(){}",
            "pub use later as first;use f as later;pub fn f(){}fn main(){}",
            "pub use second as first;pub use first as second;fn main(){}",
            "pub mod a{}pub use a as alias;pub use a as alias;fn main(){}",
            "pub fn f(){}pub use f::{self as alias};fn main(){}",
            "mod a{pub use self as alias;}fn main(){}",
            "mod a{pub mod b{pub use super as alias;}}fn main(){}",
        ] {
            bad(source);
        }
    }
    #[test]
    fn legal_imported_main_is_an_explicit_unsupported_boundary() {
        for source in [
            "mod a{pub fn f(){}}pub use a::f as main;",
            "mod a{pub fn f(){}}use a::f as main;",
        ] {
            assert!(
                parse(source)
                    .unwrap_err()
                    .contains("imported main entry points are unsupported")
            );
        }
    }
    #[test]
    fn unsupported_module_forms_are_rejected_explicitly() {
        for (source, message) in [
            ("mod a; fn main(){}", "external module"),
            (
                "type Alias<T> where T:Copy=T;fn main(){}",
                "type alias bounds and where clauses are unsupported",
            ),
            ("use crate::a::*; fn main(){}", "glob imports"),
            ("pub use crate::a::*; fn main(){}", "glob imports"),
            ("mod a{} mod a{} fn main(){}", "duplicate item"),
        ] {
            assert!(parse(source).unwrap_err().contains(message), "{source}");
        }
        bad("mod a{} fn main(){a();}");
        bad("fn f(){} fn main(){f::g();}");
        bad("fn main(){use crate::a;}");
        let nested = format!("{}{}fn main(){{}}", "mod a{".repeat(300), "}".repeat(300));
        assert!(parse(&nested).unwrap_err().contains("nesting"));
    }
    #[test]
    fn source_aliases_erase_binders_without_nominal_or_value_declarations() {
        for source in [
            "type A<T>=B<(T,T)>;type B<T>=T;fn f<U>(x:A<U>)->(U,U){x}fn main(){let p=f::<i32>((2,3));}",
            "type T=A<i32>;type A<T>=T;fn main(){let n:T=7;}",
            "mod a{mod hidden{pub struct S{pub x:i32}}pub type A=hidden::S;}fn main(){let a=a::A{x:7};let a::A{x}=a;}",
            "struct S<T>{x:T}type A=S<i32>;fn main(){let a=A{x:7};let A{x}=a;}",
            "enum E<T>{V(T),N}type A<T>=E<T>;fn main(){let a=A::<i32>::V(7);match a{A::<i32>::V(x)=>{let n=x;},A::<i32>::N=>{}}}",
            "struct S{x:i32}type A=S;fn A()->i32{7}fn main(){let n=A();let A=8;let n=A;let s=crate::A{x:n};}",
        ] {
            let program = parse(source).unwrap_or_else(|e| panic!("{source}: {e}"));
            check(&program).unwrap_or_else(|e| panic!("{source}: {e}"));
        }
    }
    #[test]
    fn unused_aliases_validate_targets_arity_parameters_and_cycles() {
        for (source, expected) in [
            ("type A=Missing;fn main(){}", "unresolved type"),
            ("struct S<T>{x:T}type A=S;fn main(){}", "E0107"),
            ("type A<T>=i32;fn main(){}", "E0091"),
            ("type A<T>=A<(T,T)>;fn main(){}", "E0391"),
            ("type A=B;type B=A;fn main(){}", "E0391"),
            ("type A=(_,(i32,_));fn main(){}", "E0121"),
            (
                "struct S{x:i32}type A=S;use A::{self as B};fn main(){}",
                "not a module",
            ),
        ] {
            let error = parse(source).unwrap_err();
            assert!(error.contains(expected), "{source}: {error}");
        }
    }
    #[test]
    fn alias_resolution_is_bounded_on_a_normal_128k_stack() {
        let source = (0..40)
            .map(|i| format!("type A{i}=A{};", i + 1))
            .collect::<String>()
            + "type A40=i32;fn main(){}";
        let parsed = parse_unresolved(&source).unwrap();
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(move || {
                resolve(&parsed).unwrap();
            })
            .unwrap()
            .join()
            .unwrap();
        let source = "type A<T>=B<(T,T)>;type B<T>=A<(T,T)>;fn main(){}";
        let parsed = parse_unresolved(source).unwrap();
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(move || {
                assert!(resolve(&parsed).unwrap_err().contains("E0391"));
            })
            .unwrap()
            .join()
            .unwrap();
    }
    fn good(s: &str) {
        let p = parse(s).unwrap();
        check(&p).unwrap();
    }
    fn bad(s: &str) {
        assert!(parse(s).and_then(|p| check(&p)).is_err(), "accepted {s}");
    }
    #[test]
    fn escapes_comparisons_and_reserved() {
        let p = parse(r#"fn main(){println!("\u{1f980}\x21");}"#).unwrap();
        assert!(
            matches!(&p.functions[0].body.statements[0], Stmt::Expr(Expr::Println(s)) if s == "🦀!")
        );
        good("fn main(){let x = (1 < 2) == true; let y = 1<2 && 3<4;}");
        bad("fn main(){let x = true == 1 < 2;}");
        bad("fn try(){} fn main(){}");
        bad("fn main(){let x = 1; let x = true; x=2;}");
        for s in [
            r#"fn main(){println!("\u{d800}");}"#,
            r#"fn main(){println!("\x80");}"#,
            r#"fn main(){println!("\u{}");}"#,
        ] {
            bad(s);
        }
    }
    #[test]
    fn nesting_limits() {
        let nested = format!(
            "fn main(){{let x={}1{};}}",
            "(".repeat(300),
            ")".repeat(300)
        );
        assert!(parse(&nested).unwrap_err().contains("nesting"));
        let chain = format!("fn main(){{let x=1{};}}", "+1".repeat(10000));
        assert!(parse(&chain).unwrap_err().contains("nesting"));
        let blocks = format!("fn main(){{{}{} }}", "{".repeat(300), "}".repeat(300));
        assert!(parse(&blocks).unwrap_err().contains("nesting"));
    }
    #[test]
    fn independent_checks() {
        let p = parse("fn wrong()->i32 {true} fn main(){}").unwrap();
        check_signatures(&p).unwrap();
        assert!(check_function(&p, 0).is_err());
        check_function(&p, 1).unwrap();
        assert!(check_function(&p, 2).is_err());
    }
    #[test]
    fn recursion_and_flow() {
        good(
            "fn fib(n:i32)->i32 { if n < 2 { return n; } fib(n-1)+fib(n-2) } fn main(){ let mut x = fib(4); while x > 0 { x = x - 1; } println!(\"ok\\n{{done}}\"); }",
        );
    }
    #[test]
    fn scopes() {
        good("fn main(){let x=1; let x=x+1; {let x=true; let y=x && !false;} let y=x+1;}");
        bad("fn main(){let x=1; x=2;}");
        bad("fn main(){{let x=1;} let y=x;}");
        bad("fn f(x:i32,x:i32){} fn main(){}");
    }
    #[test]
    fn return_flow() {
        good("fn f(b:bool)->i32 {if b {return 1;} else {return 2;}} fn main(){}");
        bad("fn f(b:bool)->i32 {if b {return 1;}} fn main(){}");
        bad("fn f()->i32 {while true {return 1;}} fn main(){}");
        bad("fn f()->i32 {false && {return 1;};} fn main(){}");
        good("fn f()->i32 {return 1; false;} fn main(){}");
    }
    #[test]
    fn rejects_invalid() {
        for s in [
            "fn main(x:i32){}",
            "fn main()->i32{0}",
            "fn main(){foo();}",
            "fn f(x:i32){} fn main(){f();}",
            "fn main(){let x:bool=1;}",
            "fn main(){let café=1;}",
            "fn main(){println!(\"{}\");}",
            "fn main(){let x=2147483648;}",
            "fn main(){let x=1 < 2 < 3;}",
            "fn main(){} fn main(){}",
            "fn main(){let x=1; x();}",
        ] {
            bad(s);
        }
    }
    #[test]
    fn literals_and_comments() {
        good(
            "/* a /* b */ */ fn main(){let x=-2147483648; let y:()=(); println!(\"a\\t\\\"\\\\\");}",
        );
        bad("fn main(){println!(\"\\q\");}");
        assert!(parse("\nfn main(){@}").unwrap_err().starts_with("2:11:"));
    }
}

fn canonical_trait_spelling(path: &str) -> Option<&'static str> {
    match path {
        "Copy" | "core::marker::Copy" | "std::marker::Copy" => Some("core::marker::Copy"),
        "Clone" | "core::clone::Clone" | "std::clone::Clone" => Some("core::clone::Clone"),
        "Sized" | "core::marker::Sized" | "std::marker::Sized" => Some("core::marker::Sized"),
        "Drop" | "core::ops::Drop" | "std::ops::Drop" => Some("core::ops::Drop"),
        _ => None,
    }
}
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
            module: info.module.clone(),
            item: entry.name.rsplit("::").next().unwrap_or(entry.name).into(),
            kind: info.kind,
            associated: None,
        },
        source_index: usize::MAX,
        param_env: info.own_env.clone(),
        type_params: (0..info.own_env.arity())
            .map(|i| format!("__catalog{i}"))
            .collect::<Vec<_>>()
            .into(),
        env: info.env.clone(),
        visibility: info.visibility.clone(),
    })
}
fn clone_text(text: &str, budget: &ir::PlanBudget) -> Result<(), String> {
    budget.charge_work(
        text.len()
            .checked_add(1)
            .ok_or("source text work overflow")?,
    )?;
    budget.retain_bytes(
        text.len()
            .checked_add(32)
            .ok_or("source text bytes overflow")?,
    )
}
fn clone_vector<T>(values: &[T], budget: &ir::PlanBudget) -> Result<(), String> {
    budget.charge_work(
        values
            .len()
            .checked_add(1)
            .ok_or("source collection work overflow")?,
    )?;
    budget.retain_bytes(
        std::mem::size_of_val(values)
            .checked_mul(2)
            .and_then(|n| n.checked_add(128))
            .ok_or("source collection bytes overflow")?,
    )
}
fn clone_names(names: &[String], budget: &ir::PlanBudget) -> Result<(), String> {
    clone_vector(names, budget)?;
    for name in names {
        clone_text(name, budget)?;
    }
    Ok(())
}
fn clone_regions(regions: &RegionDecls, budget: &ir::PlanBudget) -> Result<(), String> {
    clone_names(&regions.params, budget)?;
    if let Some(receiver) = &regions.receiver {
        clone_text(receiver, budget)?;
    }
    clone_vector(&regions.outlives, budget)?;
    for (a, b) in &regions.outlives {
        clone_text(a, budget)?;
        clone_text(b, budget)?;
    }
    Ok(())
}
fn clone_bounds(bounds: &[TypeBound], budget: &ir::PlanBudget) -> Result<(), String> {
    clone_vector(bounds, budget)?;
    for bound in bounds {
        clone_text(&bound.parameter, budget)?;
        clone_names(&bound.traits, budget)?;
    }
    Ok(())
}
fn clone_parameters(params: &[(String, Type)], budget: &ir::PlanBudget) -> Result<(), String> {
    clone_vector(params, budget)?;
    for (name, ty) in params {
        clone_text(name, budget)?;
        walk_depth(DepthNode::Type(ty), Some(budget))?;
    }
    Ok(())
}
fn clone_fields(fields: &[FieldDecl], budget: &ir::PlanBudget) -> Result<(), String> {
    clone_vector(fields, budget)?;
    for field in fields {
        clone_visibility(&field.visibility, budget)?;
        clone_text(&field.name, budget)?;
        walk_depth(DepthNode::Type(&field.ty), Some(budget))?;
    }
    Ok(())
}
fn clone_selector(selector: &FieldSelector, budget: &ir::PlanBudget) -> Result<(), String> {
    if let FieldSelector::Named(name) = selector {
        clone_text(name, budget)?;
    }
    Ok(())
}
fn clone_place(place: &Place, budget: &ir::PlanBudget) -> Result<(), String> {
    clone_text(&place.local, budget)?;
    clone_vector(&place.fields, budget)?;
    for field in &place.fields {
        clone_selector(field, budget)?;
    }
    Ok(())
}
fn clone_path(path: &GenericPath, budget: &ir::PlanBudget) -> Result<(), String> {
    clone_vector(&path.segments, budget)?;
    for segment in &path.segments {
        clone_text(&segment.name, budget)?;
        clone_vector(&segment.args, budget)?;
    }
    Ok(())
}
fn preflight_clone_node(node: &DepthNode<'_>, budget: &ir::PlanBudget) -> Result<(), String> {
    budget.charge_work(1)?;
    budget.retain_bytes(1024)?;
    match node {
        DepthNode::Type(ty) => match ty {
            Type::Named(name) | Type::Param(name) => clone_text(name, budget)?,
            Type::Ref {
                region: Some(name), ..
            } => clone_text(name, budget)?,
            Type::Tuple(fields) => clone_vector(fields, budget)?,
            Type::Apply(path) => clone_path(path, budget)?,
            _ => {}
        },
        DepthNode::Pattern(pattern) => match pattern {
            Pattern::Apply { path, .. } => clone_path(path, budget)?,
            Pattern::Path(name) | Pattern::Binding { name, .. } => clone_text(name, budget)?,
            Pattern::Struct { name, fields, .. } => {
                clone_text(name, budget)?;
                clone_vector(fields, budget)?;
                for (name, _) in fields {
                    clone_text(name, budget)?;
                }
            }
            Pattern::Variant { name, fields, .. } => {
                clone_text(name, budget)?;
                clone_vector(fields, budget)?;
                for (field, _) in fields {
                    clone_selector(field, budget)?;
                }
            }
            Pattern::Tuple(fields) => clone_vector(fields, budget)?,
            _ => {}
        },
        DepthNode::Expr(expr) => match expr {
            Expr::Var(name) | Expr::Println(name) => clone_text(name, budget)?,
            Expr::Call { name, args } => {
                clone_text(name, budget)?;
                clone_vector(args, budget)?;
            }
            Expr::MethodCall {
                name,
                type_args,
                args,
                ..
            }
            | Expr::AssociatedCall {
                name,
                type_args,
                args,
                ..
            } => {
                clone_text(name, budget)?;
                clone_vector(type_args, budget)?;
                clone_vector(args, budget)?;
            }
            Expr::TraitCall {
                trait_path,
                name,
                args,
                ..
            } => {
                clone_text(trait_path, budget)?;
                clone_text(name, budget)?;
                clone_vector(args, budget)?;
            }
            Expr::Apply { path, .. } => clone_path(path, budget)?,
            Expr::Borrow { place, .. } => clone_place(place, budget)?,
            Expr::Array(fields) | Expr::Tuple(fields) => clone_vector(fields, budget)?,
            Expr::EnumVariant { name, fields, .. } => {
                clone_text(name, budget)?;
                clone_vector(fields, budget)?;
                for (field, _) in fields {
                    clone_selector(field, budget)?;
                }
            }
            Expr::Struct { name, fields } => {
                clone_text(name, budget)?;
                clone_vector(fields, budget)?;
                for (field, _) in fields {
                    clone_text(field, budget)?;
                }
            }
            Expr::Field { field, .. } => clone_selector(field, budget)?,
            Expr::Match { arms, .. } => clone_vector(arms, budget)?,
            _ => {}
        },
        DepthNode::Block(block) => {
            clone_vector(&block.statements, budget)?;
            for statement in &block.statements {
                budget.charge_work(1)?;
                match statement {
                    Stmt::Declare { name, .. }
                    | Stmt::Let { name, .. }
                    | Stmt::Assign { name, .. } => clone_text(name, budget)?,
                    Stmt::AssignPlace { place, .. } | Stmt::AssignOp { place, .. } => {
                        clone_place(place, budget)?
                    }
                    Stmt::AssignIndex { array, .. } | Stmt::AssignIndexOp { array, .. } => {
                        clone_place(array, budget)?
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}
fn preflight_program_clone(program: &Program, budget: &ir::PlanBudget) -> Result<(), String> {
    budget.retain_bytes(std::mem::size_of::<Program>())?;
    clone_vector(&program.functions, budget)?;
    for f in &program.functions {
        preflight_function_clone(f, budget)?;
    }
    clone_vector(&program.impls, budget)?;
    for implementation in &program.impls {
        clone_text(&implementation.module, budget)?;
        if let Some(path) = &implementation.trait_path {
            clone_text(path, budget)?;
        }
        clone_names(&implementation.type_params, budget)?;
        clone_bounds(&implementation.bounds, budget)?;
        walk_depth(DepthNode::Type(&implementation.self_ty), Some(budget))?;
        clone_vector(&implementation.items, budget)?;
        for item in &implementation.items {
            clone_visibility(&item.visibility, budget)?;
            preflight_function_clone(&item.function, budget)?;
        }
    }
    clone_vector(&program.structs, budget)?;
    for declaration in &program.structs {
        clone_visibility(&declaration.visibility, budget)?;
        clone_text(&declaration.name, budget)?;
        clone_names(&declaration.type_params, budget)?;
        clone_fields(&declaration.fields, budget)?;
    }
    clone_vector(&program.enums, budget)?;
    for declaration in &program.enums {
        clone_visibility(&declaration.visibility, budget)?;
        clone_text(&declaration.name, budget)?;
        clone_names(&declaration.type_params, budget)?;
        clone_vector(&declaration.variants, budget)?;
        for variant in &declaration.variants {
            clone_text(&variant.name, budget)?;
            clone_fields(&variant.fields, budget)?;
        }
    }
    clone_vector(&program.traits, budget)?;
    for declaration in &program.traits {
        clone_visibility(&declaration.visibility, budget)?;
        clone_text(&declaration.name, budget)?;
        clone_names(&declaration.supertraits, budget)?;
        clone_vector(&declaration.items, budget)?;
        for item in &declaration.items {
            let head = &item.signature;
            clone_text(&head.name, budget)?;
            clone_names(&head.type_params, budget)?;
            clone_bounds(&head.bounds, budget)?;
            clone_regions(&head.regions, budget)?;
            clone_parameters(&head.params, budget)?;
            walk_depth(DepthNode::Type(&head.result), Some(budget))?;
            if let Some(default) = &item.default {
                preflight_function_clone(default, budget)?;
            }
        }
    }
    clone_vector(&program.trait_imports, budget)?;
    for (module, path) in &program.trait_imports {
        clone_text(module, budget)?;
        clone_text(path, budget)?;
    }
    Ok(())
}
fn preflight_function_clone(function: &Function, budget: &ir::PlanBudget) -> Result<(), String> {
    clone_visibility(&function.visibility, budget)?;
    clone_text(&function.name, budget)?;
    clone_names(&function.type_params, budget)?;
    clone_bounds(&function.bounds, budget)?;
    clone_regions(&function.regions, budget)?;
    clone_parameters(&function.params, budget)?;
    walk_depth(DepthNode::Type(&function.result), Some(budget))?;
    walk_depth(DepthNode::Block(&function.body), Some(budget))
}

fn clone_visibility(visibility: &ir::Visibility, budget: &ir::PlanBudget) -> Result<(), String> {
    if let ir::Visibility::Restricted(scope) = visibility {
        clone_text(scope, budget)?;
    }
    Ok(())
}

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
