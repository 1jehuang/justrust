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
//! module-level named/grouped/glob imports have separate type/value namespaces. Named
//! structs and enums are nominal and non-Copy. Checked pattern decisions retain
//! ordered scalar/tag observations and initialize bindings only after all tests
//! succeed. Scoped acceptance and remaining pattern boundaries are documented
//! in PATTERN-DECISIONS.md and PATTERN-USEFULNESS.md.
//! Move checking belongs to CFG analysis, not
//! this semantic checker. Restricted visibility names lexical ancestor modules.
//! External modules and block-local items are not
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
mod assignment_expr;
mod break_prefix;
mod c_abi;
mod carrier_regions;
mod char_literal;
pub(super) mod closures;
mod extern_crate;
mod local_consts;
pub use closures::shape_seal::ClosureShapeSeal;
#[cfg(test)]
mod trait_method_source_tests;

mod function_qualifiers;
mod function_source;
pub use function_source::packed_query::PackedReserveAuthority;
mod generated_callables;
mod generated_debug;
mod generated_derive;
mod generated_metadata;
mod generated_owners;
mod invocation;
#[cfg(test)]
mod invocation_authority;
mod let_chains;
mod literal_call_inputs;
mod method_argument_selection;
mod name_scope;
mod name_validation;
mod pattern_usefulness;
mod program_symbols;
mod source_inventory_walk;
mod source_schemas;
#[cfg(test)]
mod trait_method_arguments;
mod type_alignment;
use source_schemas::{SourceSchemas, SourceSchemasBuilder, StoredSchema};
mod token_cursor;
use char_literal::{parse_byte_literal, parse_char_literal};
mod deref_coercion;
mod format_args;
mod format_macros;
mod int_consts;
mod item_module;
mod let_statement;
pub(super) mod module_declarations;
pub use format_macros::SourceFormatInto;
mod associated_constants;
mod format_spec;
mod inherited_associated;
pub use associated_constants::{AssociatedConstDecl, AssociatedConstUse};
pub(in crate::compiler) use associated_constants::{
    AuthorizedAssociatedConstant, InitializerMemberInput,
};
mod iterator_source;
mod iterator_traits;
mod nominal_bounds;
#[cfg(test)]
mod nominal_intern_tests;
mod opaque;
mod operators;
pub use opaque::SourceOpaqueType;
mod panic_format;
mod param_projection;
mod runtime_sites;
mod trait_selection;
mod vec_macro;
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

type ParsedPathArguments = (Vec<RegionName>, Vec<Type>, Vec<AssociatedTypeBinding>);

/// Node-local publication attempt. A suspension returns the original sealed
/// constant inputs intact, not a partially published body or reconstructed proof.
pub(super) enum NominalCallablePublication {
    Ready(
        PublishedCallableBody,
        Vec<closures::check::CheckedClosure>,
        Vec<ir::OpaqueHiddenType>,
    ),
    Blocked {
        preparation: NominalPreparation,
        constants: Box<const_inputs::SealedBodyConstants>,
    },
}

#[cfg(test)]
mod canonical_source_replay_tests {
    use super::*;
    #[test]
    fn closed_literal_inputs_reject_forged_values_and_storage() {
        let (_, _, budget, publication) = published(
            "struct Pair<T>{a:T,b:T}fn consume<T>(x:T)->bool{true}fn main(){consume(&Pair{a:\"x\",b:\"y\"});}",
            1,
        );
        let source = publication.source();
        let original = publication.original().source_for_test();
        publication
            .validate_source(source, publication.facts(), &budget)
            .unwrap();
        assert_eq!(original.literal_temporaries.len(), 1);
        for mode in 0..5 {
            let mut changed = source.as_ref().clone();
            match mode {
                0 => {
                    let expr = changed
                        .exprs
                        .iter_mut()
                        .find(|e| matches!(e.kind, ir::ExprKind::Str(_)))
                        .unwrap();
                    expr.kind = ir::ExprKind::Str("forged".into());
                }
                1 => {
                    let expr = changed
                        .exprs
                        .iter_mut()
                        .find(|e| matches!(e.kind, ir::ExprKind::Struct { .. }))
                        .unwrap();
                    let ir::ExprKind::Struct { fields, .. } = &mut expr.kind else {
                        unreachable!()
                    };
                    fields.swap(0, 1);
                }
                2 | 3 => {
                    let expr = changed
                        .exprs
                        .iter_mut()
                        .find(|e| matches!(e.kind, ir::ExprKind::Borrow { .. }))
                        .unwrap();
                    let ir::ExprKind::Borrow { kind, place, .. } = &mut expr.kind else {
                        unreachable!()
                    };
                    if mode == 2 {
                        *kind = ir::BorrowKind::Mut;
                    } else {
                        place.projection.push(ir::Projection::Deref);
                    }
                }
                _ => {
                    let (_, local) = original.literal_temporaries.iter().next().unwrap();
                    let replacement = ir::LocalId(changed.locals.len());
                    changed.locals.push(changed.locals[local.0].clone());
                    for expr in &mut changed.exprs {
                        match &mut expr.kind {
                            ir::ExprKind::Borrow { place, .. } if place.local == *local => {
                                place.local = replacement
                            }
                            ir::ExprKind::Block(block) => {
                                for statement in &mut block.statements {
                                    if let ir::TypedStmt::Assign { local: slot, .. } = statement
                                        && *slot == *local
                                    {
                                        *slot = replacement;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            assert_eq!(
                kind_error(replay_source_body(
                    &original,
                    &Arc::new(changed),
                    publication.facts(),
                    &budget
                )),
                tr::FactsErrorKind::InvalidMetadata,
                "literal forgery {mode}"
            );
        }
    }

    #[test]
    fn root_or_seal_binds_order_tests_projections_body_and_scrutinee() {
        let text = "fn pick(p:(bool,u8))->u8{match p{(true,0)|(false,1)=>7,_=>0}}fn main(){}";
        let (_, _, budget, publication) = published(text, 0);
        publication
            .validate_source(publication.source(), publication.facts(), &budget)
            .unwrap();
        for mutation in 0..7 {
            let mut changed = publication.source().as_ref().clone();
            let index = changed
                .exprs
                .iter()
                .position(|value| matches!(value.kind, ir::ExprKind::Match { .. }))
                .unwrap();
            let ir::ExprKind::Match {
                scrutinee, arms, ..
            } = &mut changed.exprs[index].kind
            else {
                unreachable!()
            };
            let ir::MatchPattern::Alternatives { rows, .. } = &mut arms[0].pattern else {
                panic!("missing original alternatives")
            };
            match mutation {
                0 => rows.swap(0, 1),
                1 => {
                    rows.pop();
                }
                2 => rows[0].clear(),
                3 => rows[0][0].projection = vec![ir::Projection::Field(1)],
                4 => rows[0][0].kind = ir::PatternTestKind::Bool(false),
                5 => arms[0].body = arms[1].body,
                6 => *scrutinee = arms[0].body,
                _ => unreachable!(),
            }
            assert_eq!(
                kind_error(publication.validate_source(
                    &Arc::new(changed),
                    publication.facts(),
                    &budget
                )),
                tr::FactsErrorKind::InvalidMetadata,
                "mutation {mutation}"
            );
        }
        let (_, _, _, foreign) = published(text, 0);
        assert_eq!(
            kind_error(publication.validate_source(foreign.source(), publication.facts(), &budget)),
            tr::FactsErrorKind::InvalidMetadata
        );
    }
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
        // Associated omitted arguments now use the same sealed source replay.
        let (program, plan, budget, associated) = published(
            "struct S{x:i32}impl S{fn id<T>(x:T)->T{x}}fn f(r:&i32){let y=S::id(r);}",
            0,
        );
        ir::validate_published_source(&associated, &budget).unwrap();
        let (call, target, argument) = associated
            .source()
            .exprs
            .iter()
            .enumerate()
            .find_map(|(index, expr)| match &expr.kind {
                ir::ExprKind::Call {
                    target: ir::Callee::Direct(target),
                    args,
                    ..
                } => Some((index, target, args[0])),
                _ => None,
            })
            .unwrap();
        let path = plan.semantic_path(target.declaration).unwrap();
        assert!(path.associated.is_some());
        let DeclItemName::Source(name) = path.item else {
            panic!("expected source method")
        };
        assert_eq!(program.names.spelling(name).unwrap(), "id");
        assert_eq!(
            target.type_args.as_ref(),
            &[associated.source().exprs[argument.0].ty.clone()]
        );
        assert!(matches!(
            associated.source().exprs[argument.0].kind,
            ir::ExprKind::Local(_)
        ));
        let mut forged = associated.source().as_ref().clone();
        let ir::ExprKind::Call {
            target: ir::Callee::Direct(target),
            ..
        } = &mut forged.exprs[call].kind
        else {
            unreachable!()
        };
        target.declaration = plan.function_decl(0).unwrap();
        assert_eq!(
            kind_error(associated.validate_source(&Arc::new(forged), associated.facts(), &budget)),
            tr::FactsErrorKind::InvalidMetadata
        );
        super::parser_tests::stock_native_run(
            "struct S{x:i32}impl S{fn id<T>(x:T)->T{x}}fn main(){let value=7i32;let r=&value;let y=S::id(r);assert_eq!(*y,7);}",
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
            let stock =
                super::parser_tests::stock_compile(source, std::path::Path::new("/dev/null"), true);
            assert!(
                !stock.status.success()
                    && String::from_utf8_lossy(&stock.stderr).contains("error[E"),
                "stock did not produce a Rust semantic rejection: {source}: {}",
                String::from_utf8_lossy(&stock.stderr)
            );
            for workers in [1, 8] {
                for check in [true, false] {
                    let failure = super::super::compile_observed(source.into(), workers, check)
                        .err()
                        .unwrap();
                    if source.contains("let value=1i32") {
                        let diagnostic = failure.error.to_string();
                        assert!(diagnostic.contains("ImmutableReassignment"), "{diagnostic}");
                        assert!(diagnostic.contains("source declaration 1"), "{diagnostic}");
                        let report = failure.report.as_ref().unwrap();
                        let admission = report
                            .node_evaluations
                            .iter()
                            .zip(&report.node_ids)
                            .find_map(|((key, _), id)| {
                                (key.kind == super::super::SOURCE_ADMISSION
                                    && key.params.as_slice() == [1])
                                .then_some(id)
                            })
                            .expect(
                                "main must fail its source admission, before instance analysis",
                            );
                        assert!(report.node_errors.iter().any(|(id, error)| id == admission
                            && error.to_string().contains("ImmutableReassignment")));
                    } else {
                        let report = failure.report.as_ref().unwrap();
                        let source_rejection =
                            report.node_evaluations.iter().zip(&report.node_ids).any(
                                |((key, _), id)| {
                                    key.kind == super::super::SOURCE_ADMISSION
                                        && key.params.as_slice() == [1]
                                        && report.node_errors.iter().any(|(failed_id, error)| {
                                            failed_id == id
                                                && error
                                                    .to_string()
                                                    .contains("initialization/move analysis")
                                        })
                                },
                            );
                        assert!(
                            source_rejection
                                || failure
                                    .analyses
                                    .iter()
                                    .any(|(_, analysis)| analysis.complete && !analysis.passed()),
                            "{source}: {}",
                            failure.error
                        );
                        let diagnostic = failure.error.to_string();
                        assert!(
                            diagnostic.contains("BorrowConflict")
                                || diagnostic.contains("ReadNotDefinitelyInitialized")
                                || diagnostic.contains("InvalidReference"),
                            "{source}: {diagnostic}"
                        );
                    }
                    let report = failure.report.as_ref().unwrap();
                    assert!(report.capture_complete);
                    assert!(!report.published_kind(super::super::BUILD));
                    if check {
                        assert!(
                            report
                                .node_evaluations
                                .iter()
                                .all(|(key, _)| key.kind != super::super::OBJECT)
                        );
                    }
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
                let report = failure.report.unwrap();
                assert!(report.capture_complete);
                // BUILD may demand an object before instance planning finishes,
                // but this rejected body must never publish code or a binary.
                assert!(!report.published_kind(103));
                assert!(!report.published_kind(105));
                if check {
                    assert!(
                        report
                            .node_evaluations
                            .iter()
                            .all(|(key, _)| key.kind != 103)
                    );
                }
            }
        }
    }

    #[test]
    fn root_borrow_unsupported_origins_have_typed_rejections() {
        for (source, expected) in [
            (
                "fn renamed<T>(x:T)->T{x}struct S{x:i32}fn main(){let value=S{x:1};let r=renamed(&value.x);}",
                "inferred reference requires original shared local Var",
            ),
            (
                "fn renamed<T>(x:T)->T{x}fn runtime()->i32{1}fn main(){let value=runtime();let r=renamed(&value);}",
                "unsupported: fresh borrow root requires an annotation or suffixed integer/Bool/char literal",
            ),
            (
                "fn renamed<T>(x:T)->T{x}fn main(){let value=1i32;let nested=&value;let r=renamed(&nested);}",
                "unsupported: fresh borrow root requires an annotation or suffixed integer/Bool/char literal",
            ),
        ] {
            let stock =
                super::parser_tests::stock_compile(source, std::path::Path::new("/dev/null"), true);
            assert!(
                stock.status.success(),
                "stock: {}",
                String::from_utf8_lossy(&stock.stderr)
            );
            let program = fixture_program(source);
            let budget = ir::PlanBudget::default();
            let plan = PlanCtx::from_program(&program, &budget).unwrap();
            let main = program
                .functions
                .iter()
                .position(|function| {
                    function.name.anchor() == PathAnchor::Crate
                        && function.name.len() == 1
                        && program.names.spelling(function.name.segments()[0]).unwrap() == "main"
                })
                .unwrap();
            let error = plan
                .publish_callable_body(plan.function_decl(main).unwrap(), &budget)
                .err()
                .expect("bounded origin must remain unsupported");
            assert_eq!(error.kind(), tr::FactsErrorKind::Unsupported);
            let failure = super::super::compile_observed(source.into(), 1, true)
                .err()
                .unwrap();
            assert!(
                failure.error.to_string().contains(expected),
                "{source}: {}",
                failure.error
            );
            let report = failure.report.unwrap();
            assert!(report.capture_complete);
            assert!(!report.published_kind(super::super::BUILD));
            assert!(
                report
                    .node_evaluations
                    .iter()
                    .all(|(key, _)| key.kind != super::super::OBJECT)
            );
        }
    }

    #[test]
    fn root_borrow_replay_rejects_kind_place_capability_and_initializer_mutations() {
        assert_root_borrow_replay_rejects_mutations(
            "fn renamed<T>(x:T)->T{x}fn main(){let mut left=1i32;let mut right=2i32;let r=renamed(&mut left);*r=3;}",
        );
    }

    #[test]
    fn scalar_input_replay_rejects_kind_place_capability_and_initializer_mutations() {
        assert_root_borrow_replay_rejects_mutations(
            "fn consume<T>(x:T)->i32{let _=x;7}fn main(){let mut left=1i32;let mut right=2i32;let r=consume(&mut left);let out=r;}",
        );
    }

    #[test]
    fn scalar_input_replay_preserves_move_expectation() {
        let (_, _, budget, publication) = published(
            "fn consume<T>(x:T)->i32{let _=x;7}fn main(){let mut n=1i32;let moved:&mut i32=&mut n;let result=consume(moved);}",
            1,
        );
        let original = publication.original().source_for_test();
        let source = publication.source();
        let (call, argument) = source
            .exprs
            .iter()
            .enumerate()
            .find_map(|(index, expr)| match &expr.kind {
                ir::ExprKind::Call { args, .. } => Some((ir::ExprId(index), args[0])),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            publication.replay().expected(call, 0, &budget).unwrap(),
            Some(ir::CallExpectation::NoFirmReference)
        );
        let ir::ExprKind::Local(local) = source.exprs[argument.0].kind else {
            panic!("inferred generic argument must move the reference");
        };
        let mut changed = source.as_ref().clone();
        let carrier = ir::ExprId(changed.exprs.len());
        changed.exprs.push(changed.exprs[argument.0].clone());
        changed.exprs[argument.0].kind = ir::ExprKind::Borrow {
            kind: ir::BorrowKind::Mut,
            place: ir::Place {
                local,
                projection: vec![ir::Projection::Deref],
            },
            timing: ir::BorrowTiming::TwoPhase {
                activation: call,
                argument: 0,
                reason: ir::ImplicitBorrowKind::MutableArgumentReborrow { carrier },
            },
        };
        assert_eq!(
            kind_error(replay_source_body(
                &original,
                &Arc::new(changed),
                publication.facts(),
                &budget
            )),
            tr::FactsErrorKind::InvalidMetadata
        );
    }

    fn assert_root_borrow_replay_rejects_mutations(text: &str) {
        let (_, _, budget, publication) = published(text, 1);
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
        let ir::TypedStmt::Let {
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
    fn borrowed_callable_replay_rejects_place_and_producer_mutations() {
        for source in [
            "trait Marker{}struct S{}impl Marker for S{}fn apply<F>(f:F,x:i32)->i32{x}fn add(n:i32)->impl Marker{S{}}fn main(){let a=add(5);let b=add(6);let result=apply(&a,10);}",
            "trait Marker{}struct S{}impl Marker for S{}fn apply<F>(f:F,x:i32)->i32{x}fn add(n:i32)->impl Marker{S{}}fn main(){let a=add(5);let b=add(6);let first=&a;let second=&first;let result=apply(second,10);}",
            "trait Marker{}struct S{}impl Marker for S{}fn apply<F>(f:F,x:i32)->i32{x}fn add(n:i32)->impl Marker{S{}}fn main(){let mut a=add(5);let b=add(6);let first=&mut a;let result=apply(first,10);}",
        ] {
            let (_, _, budget, publication) = published(source, 2);
            let original = publication.original().source_for_test();
            let source = publication.source();
            replay_source_body(&original, source, publication.facts(), &budget).unwrap();
            let borrow = source
                .exprs
                .iter()
                .position(|e| matches!(e.kind, ir::ExprKind::Borrow { .. }))
                .unwrap();
            let sibling = source
                .locals
                .iter()
                .position(|local| local.name == "b")
                .unwrap();
            for mutation in 0..4 {
                let mut changed = source.as_ref().clone();
                let ir::ExprKind::Borrow { kind, place, .. } = &mut changed.exprs[borrow].kind
                else {
                    unreachable!()
                };
                match mutation {
                    0 => place.local = ir::LocalId(sibling),
                    1 => {
                        *kind = if *kind == ir::BorrowKind::Mut {
                            ir::BorrowKind::Shared
                        } else {
                            ir::BorrowKind::Mut
                        }
                    }
                    2 => place.projection.push(ir::Projection::Deref),
                    _ => {
                        let ir::TypedStmt::Let {
                            value: sibling_value,
                            ..
                        } = source.body.statements[1]
                        else {
                            unreachable!()
                        };
                        let ir::TypedStmt::Let { value, .. } = &mut changed.body.statements[0]
                        else {
                            unreachable!()
                        };
                        *value = sibling_value;
                    }
                }
                assert_eq!(
                    kind_error(replay_source_body(
                        &original,
                        &Arc::new(changed),
                        publication.facts(),
                        &budget
                    )),
                    tr::FactsErrorKind::InvalidMetadata
                );
            }
        }
    }

    #[test]
    fn borrowed_callable_replay_rejects_reborrow_timing_mutations() {
        let (_, _, budget, publication) = published(
            "trait Marker{}struct S{}impl Marker for S{}fn apply<F>(f:F,x:i32)->i32{x}fn add(n:i32)->impl Marker{S{}}fn main(){let mut a=add(5);let first=&mut a;let result=apply(first,10);}",
            2,
        );
        let original = publication.original().source_for_test();
        let source = publication.source();
        replay_source_body(&original, source, publication.facts(), &budget).unwrap();
        let index = source
            .exprs
            .iter()
            .position(|expr| {
                matches!(
                    expr.kind,
                    ir::ExprKind::Borrow {
                        timing: ir::BorrowTiming::TwoPhase { .. },
                        ..
                    }
                )
            })
            .unwrap();
        for mutation in 0..3 {
            let mut changed = source.as_ref().clone();
            let ir::ExprKind::Borrow {
                timing:
                    ir::BorrowTiming::TwoPhase {
                        activation,
                        argument,
                        reason,
                    },
                ..
            } = &mut changed.exprs[index].kind
            else {
                unreachable!()
            };
            match mutation {
                0 => *activation = ir::ExprId(index),
                1 => *argument = 1,
                _ => {
                    let ir::ImplicitBorrowKind::MutableArgumentReborrow { carrier } = reason else {
                        unreachable!()
                    };
                    *carrier = ir::ExprId(index);
                }
            }
            assert_eq!(
                kind_error(replay_source_body(
                    &original,
                    &Arc::new(changed),
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
            runtime_sites: Default::default(),
            plan: original.plan.clone(),
            declaration: original.declaration,
            facts: original.facts.clone(),
            inventory,
            constants: Vec::new(),
            symbolic_constants: Vec::new(),
            symbolic_constant_emissions: Default::default(),
            constant_emissions: Default::default(),
            calls: Default::default(),
            nominal_call_realizations: Default::default(),
            builtin_leading_zeros: Default::default(),
            builtin_lens: Default::default(),
            array_iters: Default::default(),
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
    for_iterable: bool,
    /// Exact direct `.iter()` occurrence of this for iterable, never a
    /// name-only grant of builtin method authority.
    array_iter_source: Option<usize>,
    leading_zeros_candidate: bool,
    /// Syntactic `receiver.name(args)` with an admitted primitive method name.
    intrinsic_candidate: bool,
    /// Operator expression or compound assignment that may lower to an
    /// overloaded operator trait call; primitive operands map to no call.
    operator_candidate: bool,
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
    // Reconstructed from immutable source on each inventory walk, never published.
    literal_arguments: HashMap<usize, Expr>,
    constants: Vec<(sr::SourceNode, PathKey)>,
    associated_constants: Vec<(sr::SourceNode, AssociatedConstUse)>,
    associated_constant_pointers: HashMap<usize, usize>,
    constant_pointers: HashMap<usize, usize>,
    bindings: Vec<SourceBindingSite>,
    call_pointers: HashMap<usize, usize>,
    for_pointers: HashMap<usize, usize>,
    iterator_pointers: HashMap<usize, usize>,
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
                    receiver_authority::local(path)
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
            // Keep indexing as an expression until the receiver type is known.
            // A literal slice index is a runtime check, unlike a fixed array
            // projection, and must retain its original source occurrence.
            Expr::Index { .. } => return None,
            _ => return None,
        }
    }
}
/// A path that can name a local binding: one relative segment or bare `self`.
fn is_local_shaped_path(path: &AstPath) -> bool {
    matches!(
        (path.anchor(), path.segments().len()),
        (PathAnchor::Relative, 1) | (PathAnchor::SelfModule, 0)
    )
}
fn source_local_name(path: &AstPath, names: &NameStore) -> Result<LocalName, String> {
    path.check(names).map_err(native_adapter::name_diagnostic)?;
    match (path.anchor(), path.segments()) {
        (PathAnchor::Relative, [name]) => Ok(LocalName::Named(*name)),
        (PathAnchor::SelfModule, []) => Ok(receiver_authority::local(path)),
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
        Type::Opaque(opaque) => {
            for bound in &opaque.bounds {
                sr::work(budget, 1)?;
                if let TraitPath::Source(path) = bound {
                    compare_source_path_work(path, budget, at + 1)?;
                }
            }
        }
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
        (ir::Callee::TraitItem(a), ir::Callee::TraitItem(b))
        | (ir::Callee::Untupled(a), ir::Callee::Untupled(b))
        // A checked callable trait call on a function item canonicalizes to
        // builtin untupled dispatch of the same trait item.
        | (ir::Callee::TraitItem(a), ir::Callee::Untupled(b))
        | (
            ir::Callee::Virtual(ir::VirtualCall { call: a }),
            ir::Callee::Virtual(ir::VirtualCall { call: b }),
        ) => {
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
        sr::work(budget, segment.region_args.len())?;
        budget
            .retain_bytes(
                segment
                    .region_args
                    .len()
                    .checked_mul(std::mem::size_of::<RegionName>())
                    .ok_or_else(|| {
                        FactsError::resource(budget, "source lifetime argument allocation overflow")
                    })?,
            )
            .map_err(|error| FactsError::from_message(budget, &error))?;
        let mut region_args = Vec::new();
        region_args
            .try_reserve_exact(segment.region_args.len())
            .map_err(|_| FactsError::resource(budget, "source lifetime argument allocation"))?;
        region_args.extend_from_slice(&segment.region_args);
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
            region_args,
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
        Type::Opaque(opaque) => Type::Opaque(Box::new(opaque.copy(budget, at + 1)?)),
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
        Type::Dyn(trait_path) => Type::Dyn(Box::new(match trait_path.as_ref() {
            TraitPath::Source(path) => TraitPath::Source(copy_source_path(path, budget, at + 1)?),
            TraitPath::Builtin(kind) => TraitPath::Builtin(*kind),
        })),
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
        Type::Opaque(opaque) => opaque.retain(budget, depth + 1)?,
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
/// Whether a written signature type mentions `Self` (dyn compatibility).
/// `Self::Item` projections also count; `&Self` receivers are not params.
fn source_type_mentions_self(
    ty: &Type,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<bool, String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    Ok(match ty {
        Type::Opaque(opaque) => {
            let mut found = false;
            for ty in opaque.types() {
                found |= source_type_mentions_self(ty, budget, depth + 1)?;
            }
            found
        }
        Type::SelfType => true,
        Type::Projection(projection) => match projection.as_ref() {
            SourceProjection::SelfItem { .. } => false,
            SourceProjection::Qualified {
                self_ty,
                trait_path,
                ..
            } => {
                let mut mentions = !matches!(self_ty, Type::SelfType)
                    && source_type_mentions_self(self_ty, budget, depth + 1)?;
                if let TraitPath::Source(path) = trait_path {
                    for segment in &path.segments {
                        for argument in &segment.args {
                            mentions |= source_type_mentions_self(argument, budget, depth + 1)?;
                        }
                    }
                }
                mentions
            }
        },
        Type::Ref { referent, .. } => source_type_mentions_self(referent, budget, depth + 1)?,
        Type::Array(element, _)
        | Type::Slice(element)
        | Type::RawPtr {
            pointee: element, ..
        } => source_type_mentions_self(element, budget, depth + 1)?,
        Type::Tuple(fields) => {
            let mut found = false;
            for field in fields {
                found |= source_type_mentions_self(field, budget, depth + 1)?;
            }
            found
        }
        Type::Apply(path) => {
            let mut found = matches!(path.anchor, PathAnchor::SelfType);
            for segment in &path.segments {
                for arg in segment.args.iter().chain(
                    segment
                        .associated_types
                        .iter()
                        .map(|binding| &binding.value),
                ) {
                    found |= source_type_mentions_self(arg, budget, depth + 1)?;
                }
            }
            found
        }
        Type::Named(path) => matches!(path.anchor(), PathAnchor::SelfType),
        Type::Dyn(_)
        | Type::Str
        | Type::Param(_)
        | Type::Int(_)
        | Type::Float(_)
        | Type::Char
        | Type::Bool
        | Type::Unit
        | Type::Never
        | Type::Infer => false,
    })
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
            literal_arguments: HashMap::new(),
            constants: Vec::new(),
            associated_constants: Vec::new(),
            associated_constant_pointers: HashMap::new(),
            constant_pointers: HashMap::new(),
            bindings: Vec::new(),
            call_pointers: HashMap::new(),
            for_pointers: HashMap::new(),
            iterator_pointers: HashMap::new(),
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
        if matches!(
            name.checked_key(&self.names)
                .map_err(|error| source_name_error(&self.names, budget, &error))?,
            Some(LocalKey::Receiver)
        ) {
            let occurrence = name
                .occurrence(&self.names)
                .map_err(|error| source_name_error(&self.names, budget, &error))?;
            return receiver_authority::key(scopes, &self.names, function, occurrence, declaration)
                .map(Some)
                .map_err(|error| source_scope_error(budget, error));
        }
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
            Pattern::At { name, pattern, .. } => {
                self.bind(name, node.clone(), None, None, None, budget)?;
                self.shadow_pattern(pattern, &node.child(9, 0, budget)?, budget, depth + 1)?
            }
            Pattern::Path(path) => {
                if let Some(name) = path.single() {
                    self.bind(name, node.clone(), None, None, None, budget)?;
                }
            }
            Pattern::Apply { pattern, .. } | Pattern::Ref { pattern, .. } => {
                self.shadow_pattern(pattern, node, budget, depth + 1)?
            }
            Pattern::Tuple(fields) | Pattern::Array(fields) | Pattern::Alternatives(fields) => {
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
    /// Inventory an operator occurrence that may become an overloaded call.
    fn operator_site(
        &mut self,
        pointer: usize,
        node: &sr::SourceNode,
        budget: &ir::PlanBudget,
    ) -> Result<(), FactsError> {
        let index = self.calls.len();
        sr::push(
            &mut self.calls,
            SourceCallSite {
                node: node.clone(),
                len_candidate: false,
                for_iterable: false,
                array_iter_source: None,
                leading_zeros_candidate: false,
                intrinsic_candidate: false,
                operator_candidate: true,
                ordinary: None,
                application: None,
                argument_binding: None,
                argument_borrow: None,
                direct_let: None,
                args_pointer: 0,
            },
            budget,
        )?;
        insert_source_pointer(&mut self.call_pointers, pointer, index, budget)
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
                Stmt::AssignDeref {
                    carrier,
                    index,
                    value,
                    ..
                } => {
                    if type_check::genuine_call(carrier) {
                        // `*guard() (OP)= v` may call DerefMut on the owned
                        // result; recorded at the carrier's receiver step.
                        self.operator_site(
                            deref_coercion::site(carrier as *const Expr as usize, 1),
                            &here.child(4, 0, budget)?,
                            budget,
                        )?;
                    }
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
                    if let Some(index) = index {
                        self.expr(
                            index,
                            &here.child(4, 2, budget)?,
                            None,
                            None,
                            budget,
                            depth + 1,
                        )?;
                    }
                }
                Stmt::AssignNestedIndex { target, value, .. } => {
                    self.expr(
                        target,
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
                    self.operator_site(
                        statement as *const Stmt as usize | 1,
                        &here.child(41, 0, budget)?,
                        budget,
                    )?;
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
                Stmt::AssignOp { value, .. } => {
                    // A statement may share its address with its inline
                    // value expression; tag statement keys with bit 0.
                    self.operator_site(
                        statement as *const Stmt as usize | 1,
                        &here.child(41, 0, budget)?,
                        budget,
                    )?;
                    self.expr(
                        value,
                        &here.child(4, 0, budget)?,
                        None,
                        None,
                        budget,
                        depth + 1,
                    )?
                }
                Stmt::Assign { value, .. }
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
                Stmt::AssignPlace { value, .. } => {
                    self.operator_site(
                        statement as *const Stmt as usize | 1,
                        &here.child(41, 0, budget)?,
                        budget,
                    )?;
                    self.expr(
                        value,
                        &here.child(4, 0, budget)?,
                        None,
                        None,
                        budget,
                        depth + 1,
                    )?;
                }
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
                    if matches!(source.iterable, ForIterable::Value(_)) {
                        self.for_iterator_sites(source, &here, budget)?;
                        let index = self.calls.len();
                        sr::push(
                            &mut self.calls,
                            SourceCallSite {
                                node: here.child(24, 4, budget)?,
                                len_candidate: false,
                                for_iterable: true,
                                array_iter_source: if iterator_source::array_iter_candidate(
                                    first,
                                    &self.names,
                                )
                                .map_err(|e| source_error(budget, &e))?
                                .is_some()
                                {
                                    self.call_pointers
                                        .get(&(first as *const Expr as usize))
                                        .copied()
                                } else {
                                    None
                                },
                                leading_zeros_candidate: false,
                                intrinsic_candidate: false,
                                operator_candidate: true,
                                ordinary: None,
                                application: None,
                                argument_binding: None,
                                argument_borrow: None,
                                direct_let: None,
                                args_pointer: 0,
                            },
                            budget,
                        )?;
                        insert_source_pointer(
                            &mut self.for_pointers,
                            source.as_ref() as *const ForSource as usize,
                            index,
                            budget,
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
    fn expr_branch(
        &mut self,
        expr: &Expr,
        node: &sr::SourceNode,
        application: Option<&GenericPath>,
        direct_let: Option<sr::SourceNode>,
        budget: &ir::PlanBudget,
        depth: usize,
    ) -> Result<(), FactsError> {
        match expr {
            Expr::AssociatedConst(value) => {
                let index = self.associated_constants.len();
                sr::push(
                    &mut self.associated_constants,
                    (node.clone(), value.as_ref().clone()),
                    budget,
                )?;
                insert_source_pointer(
                    &mut self.associated_constant_pointers,
                    expr as *const Expr as usize,
                    index,
                    budget,
                )?;
            }
            // A function item value has no runtime operands or call site.
            Expr::AssociatedFn(_) => {}
            Expr::UnexpandedMacro(_) => {
                return Err(source_error(
                    budget,
                    "unexpanded macro syntax cannot enter active source inventory",
                ));
            }
            Expr::Try(inner) => {
                // A differing residual error type lowers to a `From::from`
                // call at this occurrence (operand pointer tagged 3).
                self.operator_site(
                    inner.as_ref() as *const Expr as usize | 3,
                    &node.child(44, 0, budget)?,
                    budget,
                )?;
                self.expr(
                    inner,
                    &node.child(40, 0, budget)?,
                    None,
                    None,
                    budget,
                    depth + 1,
                )?
            }
            Expr::Output(output) => validate_output_site(output, &self.names)
                .map_err(|message| source_error(budget, &message))?,
            Expr::FormatInto(source) => {
                for (index, argument) in source.arguments.iter().enumerate() {
                    self.operator_site(
                        argument as *const Expr as usize | 2,
                        &node.child(42, index, budget)?,
                        budget,
                    )?;
                    self.expr(
                        argument,
                        &node.child(21, index, budget)?,
                        None,
                        None,
                        budget,
                        depth + 1,
                    )?;
                }
                for (index, row) in source.rest.iter().enumerate() {
                    for (column, argument) in row.iter().enumerate() {
                        self.expr(
                            argument,
                            &node.child(43, index * 16 + column, budget)?,
                            None,
                            None,
                            budget,
                            depth + 1,
                        )?;
                    }
                }
            }
            Expr::Format { arguments, .. } => {
                for (index, argument) in arguments.iter().enumerate() {
                    self.operator_site(
                        argument as *const Expr as usize | 2,
                        &node.child(42, index, budget)?,
                        budget,
                    )?;
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
                    // Only local-shaped paths name bindings. Generated item
                    // paths (an inline closure lowered to a crate item) are
                    // not local references.
                    [Expr::Var(name), ..] if is_local_shaped_path(name) => {
                        (self.lookup(name, budget)?, None)
                    }
                    [Expr::Borrow { kind, place }, ..] if place.fields.is_empty() => {
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
                        for_iterable: false,
                        array_iter_source: None,
                        leading_zeros_candidate: false,
                        intrinsic_candidate: false,
                        operator_candidate: false,
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
                if let [argument] = args.as_slice()
                    && let Some(literal) = literal_call_inputs::copy_literal(argument, budget, 0)?
                {
                    reserve_source_map(&mut self.literal_arguments, budget)?;
                    self.literal_arguments.insert(index, literal);
                }
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
            Expr::Invoke { args, .. }
            | Expr::MethodCall { args, .. }
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
                        for_iterable: false,
                        array_iter_source: None,
                        intrinsic_candidate: intrinsic_candidate(expr, &self.names)
                            .map_err(|e| source_error(budget, &e))?,
                        operator_candidate: false,
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
                if let Expr::Invoke { receiver, .. } | Expr::MethodCall { receiver, .. } = expr {
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
            Expr::Deref { value } => {
                // May become an overloaded `*Deref::deref(&value)`.
                self.operator_site(expr as *const Expr as usize, node, budget)?;
                self.expr(
                    value,
                    &node.child(11, 0, budget)?,
                    None,
                    None,
                    budget,
                    depth + 1,
                )?
            }
            Expr::Borrow { .. } => {
                self.operator_site(expr as *const Expr as usize, node, budget)?;
            }
            Expr::BorrowValue { value, .. } => {
                self.operator_site(expr as *const Expr as usize, node, budget)?;
                self.expr(
                    value,
                    &node.child(11, 0, budget)?,
                    None,
                    None,
                    budget,
                    depth + 1,
                )?;
            }
            Expr::Cast { value, .. }
            | Expr::Repeat { value, .. }
            | Expr::SliceDataPointer(value) => self.expr(
                value,
                &node.child(11, 0, budget)?,
                None,
                None,
                budget,
                depth + 1,
            )?,
            Expr::NativeSyscall(fields)
            | Expr::RawSlice {
                arguments: fields, ..
            }
            | Expr::Array(fields)
            | Expr::Tuple(fields)
            | Expr::TupleStruct { fields, .. } => {
                let tag = if matches!(expr, Expr::NativeSyscall(_) | Expr::RawSlice { .. }) {
                    23
                } else if matches!(expr, Expr::TupleStruct { .. }) {
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
            Expr::Binary {
                left: base,
                right: index,
                op,
                ..
            } if !matches!(op, BinaryOp::And | BinaryOp::Or) => {
                self.operator_site(expr as *const Expr as usize, node, budget)?;
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
            Expr::Unary { value, .. } => {
                self.operator_site(expr as *const Expr as usize, node, budget)?;
                self.expr(
                    value,
                    &node.child(11, 0, budget)?,
                    None,
                    None,
                    budget,
                    depth + 1,
                )?;
            }
            Expr::DynamicIndex { base, index, .. }
            | Expr::Binary {
                left: base,
                right: index,
                ..
            } => {
                if matches!(expr, Expr::DynamicIndex { .. }) {
                    // May become an overloaded `*Index::index(&base, index)`.
                    self.operator_site(expr as *const Expr as usize, node, budget)?;
                }
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
            Expr::RangeIndex {
                base, start, end, ..
            } => {
                if matches!(base.as_ref(), Expr::Var(_) | Expr::Field { .. }) {
                    // A Vec place base may become `base.as_slice()` /
                    // `base.as_mut_slice()`, recorded at the base occurrence.
                    self.operator_site(base.as_ref() as *const Expr as usize, node, budget)?;
                }
                self.expr(
                    base,
                    &node.child(13, 0, budget)?,
                    None,
                    None,
                    budget,
                    depth + 1,
                )?;
                for (ordinal, bound) in [(1, start), (2, end)] {
                    if let Some(bound) = bound {
                        self.expr(
                            bound,
                            &node.child(13, ordinal, budget)?,
                            None,
                            None,
                            budget,
                            depth + 1,
                        )?;
                    }
                }
            }
            Expr::Index { base, .. } | Expr::Field { base, .. } => {
                if matches!(expr, Expr::Index { .. }) {
                    // May become an overloaded `*Index::index(&base, N)`.
                    self.operator_site(expr as *const Expr as usize, node, budget)?;
                }
                self.expr(
                    base,
                    &node.child(13, 0, budget)?,
                    None,
                    None,
                    budget,
                    depth + 1,
                )?
            }
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
                    if let Some(guard) = &arm.guard {
                        self.expr(
                            guard,
                            &here.child(41, 0, budget)?,
                            None,
                            None,
                            budget,
                            depth + 1,
                        )?;
                    }
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
            Expr::ConstItem(path) => {
                let key = name_scope::path_key(self.names.as_ref(), path, Namespace::Value)
                    .map_err(|error| source_error(budget, &error))?;
                let index = self.constants.len();
                sr::push(&mut self.constants, (node.clone(), key), budget)?;
                insert_source_pointer(
                    &mut self.constant_pointers,
                    expr as *const Expr as usize,
                    index,
                    budget,
                )?;
            }
            Expr::TypeAlignment(_)
            | Expr::Integer(_)
            | Expr::Float(_)
            | Expr::Char(_)
            | Expr::Str(_)
            | Expr::ByteStr(_)
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
        slice_len_method_candidate(*name, type_args.len(), args.len(), names)
    } else {
        Ok(false)
    }
}

/// Lexical hint only; receiver typing and canonical source replay still grant
/// builtin authority. Shared by body constraints and checked lowering.
fn slice_len_method_candidate(
    name: AstIdent,
    type_arguments: usize,
    arguments: usize,
    names: &NameStore,
) -> Result<bool, String> {
    Ok(names
        .spelling(name)
        .map_err(native_adapter::name_diagnostic)?
        == "len"
        && type_arguments == 0
        && arguments == 0)
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
        leading_zeros_method_candidate(*name, type_args.len(), args.len(), names)
    } else {
        Ok(false)
    }
}

fn leading_zeros_method_candidate(
    name: AstIdent,
    type_arguments: usize,
    arguments: usize,
    names: &NameStore,
) -> Result<bool, String> {
    Ok(names
        .spelling(name)
        .map_err(native_adapter::name_diagnostic)?
        == "leading_zeros"
        && type_arguments == 0
        && arguments == 0)
}

/// Exact builtin signature shared by body inference and checked lowering.
/// Neither the method spelling nor an expected result grants receiver authority.
fn leading_zeros_signature(
    receiver: &ir::Ty,
    name: AstIdent,
    type_arguments: usize,
    arguments: usize,
    names: &NameStore,
) -> Result<Option<ir::IntrinsicSignature>, String> {
    if !matches!(receiver, ir::Ty::Int(_)) || !leading_zeros_method_candidate(name, 0, 0, names)? {
        return Ok(None);
    }
    if type_arguments != 0 {
        return Err("E0107: leading_zeros takes no type arguments".into());
    }
    if arguments != 0 {
        return Err("E0061: leading_zeros takes no arguments".into());
    }
    Ok(Some(ir::IntrinsicSignature {
        params: vec![receiver.clone()],
        result: ir::Ty::Int(ir::IntTy::U32),
    }))
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
    runtime_sites: std::cell::RefCell<Vec<(ir::ExprId, Arc<ir::RuntimeLocation>)>>,
    plan: PlanCtx,
    declaration: ir::DeclId,
    facts: Arc<tr::ValidatedFacts>,
    inventory: SourceInventory,
    constants: Vec<const_inputs::SourceConstantUse>,
    symbolic_constants: Vec<associated_constants::SourceSymbolicConstantUse>,
    constant_emissions: std::cell::RefCell<Vec<(usize, ir::ExprId)>>,
    symbolic_constant_emissions: std::cell::RefCell<Vec<(usize, ir::ExprId)>>,
    calls: std::cell::RefCell<Vec<(usize, ir::ExprId, ir::Callee)>>,
    nominal_call_realizations: std::cell::RefCell<Option<Vec<nominal_call_receipts::Receipt>>>,
    builtin_leading_zeros: std::cell::RefCell<Vec<BuiltinLeadingZerosOccurrence>>,
    builtin_lens: std::cell::RefCell<Vec<BuiltinLenOccurrence>>,
    array_iters: std::cell::RefCell<Vec<iterator_source::ArrayIterOccurrence>>,
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
    constants: Vec<const_inputs::SourceConstantUse>,
    symbolic_constants: Vec<associated_constants::SourceSymbolicConstantUse>,
    constant_emissions: Vec<(usize, ir::ExprId)>,
    symbolic_constant_emissions: Vec<(usize, ir::ExprId)>,
    calls: Vec<SourceCallSite>,
    bindings: Vec<SourceBindingSite>,
    literal_temporaries: HashMap<ir::ExprId, ir::LocalId>,
    call_mapping: Vec<(usize, ir::ExprId, ir::Callee)>,
    nominal_call_realizations: Option<Vec<nominal_call_receipts::Receipt>>,
    builtin_leading_zeros_mapping: Vec<BuiltinLeadingZerosOccurrence>,
    builtin_len_mapping: Vec<BuiltinLenOccurrence>,
    array_iter_mapping: Vec<iterator_source::ArrayIterOccurrence>,
    builtin_intrinsic_mapping: Vec<BuiltinIntrinsicOccurrence>,
    unsize_mapping: Vec<UnsizeOccurrence>,
    binding_mapping: Vec<(usize, ir::LocalId)>,
    narrow: Vec<RawSharedOccurrence>,
    root_positions: Vec<Option<usize>>,
}

impl OriginalSource {
    pub(super) fn authored_identity(&self) -> &sr::OriginalBodyId {
        &self.identity
    }
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
        self.record_call_at(expr as *const Expr as usize, id, kind)
    }
    /// Record a call at an inventoried source pointer (an expression, or the
    /// statement of an overloaded compound assignment).
    pub(super) fn record_call_at(
        &self,
        pointer: usize,
        id: ir::ExprId,
        kind: &ir::ExprKind,
    ) -> Result<(), String> {
        let budget = &self.plan.budget;
        sr::work(budget, 1).map_err(|e| self.render(e))?;
        let index = self
            .inventory
            .call_pointers
            .get(&pointer)
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
    /// Descriptor length generated by one genuine source for iterable.
    /// It is not a source method call and cannot consume a `.len()` occurrence.
    fn record_for_len(
        &self,
        source: &ForSource,
        id: ir::ExprId,
        iterable: ir::ExprId,
        checked: &[ir::TypedExpr],
    ) -> Result<(), String> {
        let budget = &self.plan.budget;
        sr::work(budget, 1).map_err(|e| self.render(e))?;
        let source_index = *self
            .inventory
            .for_pointers
            .get(&(source as *const ForSource as usize))
            .ok_or_else(|| {
                self.render(FactsError::invalid(
                    budget,
                    "for length has no original iterable occurrence",
                ))
            })?;
        let receiver = match checked.get(id.0).map(|value| &value.kind) {
            Some(ir::ExprKind::SliceLen { slice }) => *slice,
            Some(ir::ExprKind::Int(_)) => iterable,
            _ => {
                return Err(self.render(FactsError::invalid(
                    budget,
                    "for iterable length has wrong checked operation",
                )));
            }
        };
        if !self.inventory.calls[source_index].for_iterable {
            return Err(self.render(FactsError::invalid(
                budget,
                "for length requires an original iterable",
            )));
        }
        sr::push(
            &mut self.builtin_lens.borrow_mut(),
            BuiltinLenOccurrence {
                source_index,
                expr: id,
                receiver,
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
            [Expr::Var(name)] if is_local_shaped_path(name) => {
                (source_local_name(name, &self.plan.program.names)?, None)
            }
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

    fn inferred_scalar_input(
        &self,
        args: &[Expr],
        target: &ir::Callee,
        signature: &ir::Signature,
    ) -> Result<bool, String> {
        let budget = &self.plan.budget;
        let ir::Callee::Direct(target) = target else {
            return Ok(false);
        };
        if args.len() != 1 || target.type_args.len() != 1 {
            return Ok(false);
        }
        if !matches!(&target.type_args[0], ir::Ty::Ref { referent, .. }
            if matches!(referent.as_ref(), ir::Ty::Int(_) | ir::Ty::Bool | ir::Ty::Char))
            || !ir::carrier_locations(&signature.result, budget)?.is_empty()
        {
            return Ok(false);
        }
        sr::work(budget, self.inventory.calls.len()).map_err(|e| self.render(e))?;
        let Some(site) = self
            .inventory
            .calls
            .iter()
            .find(|site| site.args_pointer == args.as_ptr() as usize)
        else {
            return Ok(false);
        };
        if site.ordinary.is_none() || site.application.is_some() {
            return Ok(false);
        }
        let view = self
            .facts
            .original_callable_schema(tr::OriginalCallableSubject::Direct(target.declaration))
            .map_err(|e| self.render(e))?;
        let scopes = view.generic_env().scopes();
        Ok(scopes.len() == 1
            && scopes[0].arity() == 1
            && view.signature().params.first()
                == Some(&ir::Ty::Param(ir::ParamId {
                    owner: scopes[0].binder(),
                    index: 0,
                })))
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
                Some(ir::TypedStmt::Let { local: actual, .. }) => {
                    !source.is_declare && actual == local
                }
                Some(ir::TypedStmt::Declare { local: actual }) => {
                    source.is_declare && actual == local
                }
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
        let Some(ir::TypedStmt::Let { local: actual, .. }) = checked.body.statements.get(statement)
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
                        "unsupported: fresh borrow root requires an annotation or suffixed integer/Bool/char literal",
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
        let Some(ir::TypedStmt::Let {
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
        if (!site.len_candidate && !site.for_iterable)
            || site.ordinary.is_some()
            || site.application.is_some()
        {
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
        let shape_valid = match &value.kind {
            ir::ExprKind::SliceLen { slice } => *slice == row.receiver && checked.exprs.get(slice.0).is_some_and(|receiver| {
                if site.for_iterable { receiver.ty.shared_slice_element().is_some() }
                else { receiver.ty.shared_descriptor_element().is_some() }
            }),
            ir::ExprKind::Int(bound) if site.for_iterable => checked.exprs.get(row.receiver.0).is_some_and(|receiver| {
                let ty = match &receiver.ty { ir::Ty::Ref { referent, .. } => referent.as_ref(), ty => ty };
                matches!(ty, ir::Ty::Array { len, .. } if *bound == ir::IntValue::usize(*len as u64))
            }),
            _ => false,
        };
        if !shape_valid || value.ty != ir::Ty::Int(ir::IntTy::Usize) {
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
        if !matches!(value.kind, ir::ExprKind::ArrayToSlice { reference } | ir::ExprKind::RefToDyn { reference } if reference == row.reference)
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
        if matches!(
            expr.kind,
            ir::ExprKind::ArrayToSlice { .. } | ir::ExprKind::RefToDyn { .. }
        ) && !covered[index]
        {
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
// Recover only source-authenticated callable roots and local reference chains.
// Checked local types are comparisons, never the authority for inferred roots.
fn borrowed_callable_source_binding(
    original: &OriginalSource,
    inventory: &SourceInventory,
    index: usize,
    checked: &ir::CheckedFunction,
    signatures: &Signatures,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<(ir::LocalId, ir::Ty), FactsError> {
    sr::depth(budget, depth)?;
    sr::work(budget, 1)?;
    let site = original.bindings.get(index).ok_or_else(|| {
        FactsError::invalid(budget, "borrowed callable binding outside original source")
    })?;
    validate_original_occurrence(original, &site.node, budget)?;
    if site.annotation.is_some() || site.parameter.is_some() {
        return source_binding_type(original, index, checked, signatures, budget);
    }
    let statement = site.statement.ok_or_else(|| {
        FactsError::unsupported(budget, "borrowed callable requires root source binding")
    })?;
    let ordinal = u32::try_from(statement)
        .map_err(|_| FactsError::resource(budget, "borrowed callable statement overflow"))?;
    if site.node.path.as_ref() != [1, 0, 2, ordinal] {
        return Err(FactsError::invalid(
            budget,
            "borrowed callable root occurrence changed",
        ));
    }
    let function = original
        .plan
        .callable_body(&original.plan.program, original.declaration)
        .map_err(|e| source_error(budget, &e))?;
    let Some(Stmt::Let {
        value: source,
        mutable,
        ..
    }) = function.body.statements.get(statement)
    else {
        return Err(FactsError::invalid(
            budget,
            "borrowed callable source initializer changed",
        ));
    };
    let local = mapped_source_binding(original, index, budget)?;
    let position = original.root_positions[index].ok_or_else(|| {
        FactsError::invalid(budget, "borrowed callable initializer lost root mapping")
    })?;
    let Some(ir::TypedStmt::Let {
        local: actual,
        value,
    }) = checked.body.statements.get(position)
    else {
        return Err(FactsError::invalid(
            budget,
            "borrowed callable initializer changed",
        ));
    };
    if *actual != local {
        return Err(FactsError::invalid(
            budget,
            "borrowed callable local identity changed",
        ));
    }
    let expression = checked.exprs.get(value.0).ok_or_else(|| {
        FactsError::invalid(budget, "borrowed callable initializer outside checked body")
    })?;
    let source_local = |name: LocalName| -> Result<usize, FactsError> {
        let key = inventory
            .lexical_key(&name, false, budget)?
            .ok_or_else(|| {
                FactsError::invalid(budget, "borrowed callable source lacks lexical binding")
            })?;
        let mut found = None;
        for (candidate, binding) in original.bindings.iter().enumerate() {
            sr::work(budget, 1)?;
            let declaration_key = if let Some(parameter) = binding.parameter {
                let (name, _) = function.params.get(parameter).ok_or_else(|| {
                    FactsError::invalid(budget, "borrowed callable parameter outside source")
                })?;
                inventory.lexical_key(name, true, budget)?
            } else if let Some(prior) = binding.statement {
                // Only earlier root bindings can initialize this root binding.
                if prior >= statement {
                    continue;
                }
                let Some(Stmt::Let { name, .. }) = function.body.statements.get(prior) else {
                    continue;
                };
                inventory.lexical_key(name, true, budget)?
            } else {
                continue;
            };
            if declaration_key == Some(key) && found.replace(candidate).is_some() {
                return Err(FactsError::invalid(
                    budget,
                    "borrowed callable source binding ambiguous",
                ));
            }
        }
        found.ok_or_else(|| FactsError::invalid(budget, "borrowed callable source binding missing"))
    };
    let ty = match source {
        Expr::Call { .. } => {
            sr::work(budget, original.call_mapping.len())?;
            let matched = original.call_mapping.iter().any(|(source, expression, _)| {
                expression == value
                    && original
                        .calls
                        .get(*source)
                        .is_some_and(|call| call.direct_let.as_ref() == Some(&site.node))
            });
            let ir::ExprKind::Call {
                target, signature, ..
            } = &expression.kind
            else {
                return Err(FactsError::invalid(
                    budget,
                    "borrowed callable producer changed",
                ));
            };
            if !matched {
                return Err(FactsError::invalid(
                    budget,
                    "borrowed callable producer lacks original occurrence",
                ));
            }
            ir::validate_callee(target, signature, ir::FactsCtx::Validated(&original.facts))
                .map_err(|e| source_error(budget, &e))?;
            signature.result.clone()
        }
        Expr::Var(path)
            if original
                .plan
                .stored_closure_syntax(path)
                .map_err(|e| source_error(budget, &e))?
                .is_some() =>
        {
            let syntax = original
                .plan
                .stored_closure_syntax(path)
                .map_err(|e| source_error(budget, &e))?
                .unwrap();
            let key =
                name_scope::path_key(original.plan.program.names.as_ref(), path, Namespace::Value)
                    .map_err(|e| source_error(budget, &e))?;
            let provider = original.plan.registry.functions.get(&key).ok_or_else(|| {
                FactsError::invalid(budget, "borrowed callable source provider missing")
            })?;
            let (ir::Ty::Closure(closure), ir::ExprKind::Tuple(_)) =
                (&expression.ty, &expression.kind)
            else {
                return Err(FactsError::invalid(
                    budget,
                    "borrowed callable closure initializer changed",
                ));
            };
            if closure.provider() != *provider
                || closure.owner() != original.declaration
                || closure.site() != syntax.site
                || original
                    .plan
                    .stored_closure_owner(*provider)
                    .map_err(|e| source_error(budget, &e))?
                    != Some((original.declaration, syntax.site))
            {
                return Err(FactsError::invalid(
                    budget,
                    "borrowed callable closure source identity changed",
                ));
            }
            let recipe = original
                .plan
                .registry
                .closure_recipes
                .get(provider)
                .ok_or_else(|| {
                    FactsError::invalid(budget, "borrowed callable closure recipe missing")
                })?;
            let template = closure
                .shape_seal()
                .and_then(|seal| seal.template_for(recipe))
                .ok_or_else(|| {
                    FactsError::invalid(budget, "borrowed callable closure source seal changed")
                })?;
            let unsealed = closure.as_ref().clone().with_shape_seal(None);
            if !template.try_eq(&unsealed, |left, right| {
                replay_type_equal(left, right, budget)
            })? {
                return Err(FactsError::invalid(
                    budget,
                    "borrowed callable closure shape changed",
                ));
            }
            expression.ty.clone()
        }
        Expr::Var(path) if is_local_shaped_path(path) => {
            let name = source_local_name(path, original.plan.program.names.as_ref())
                .map_err(|e| source_error(budget, &e))?;
            let source_index = source_local(name)?;
            let (source_local, ty) = borrowed_callable_source_binding(
                original,
                inventory,
                source_index,
                checked,
                signatures,
                budget,
                depth + 1,
            )?;
            if !matches!(expression.kind, ir::ExprKind::Local(actual) if actual == source_local) {
                return Err(FactsError::invalid(
                    budget,
                    "borrowed callable alias changed source local",
                ));
            }
            ty
        }
        Expr::Borrow { kind, place } if place.fields.is_empty() => {
            let source_index = source_local(place.local)?;
            let (source_local, referent) = borrowed_callable_source_binding(
                original,
                inventory,
                source_index,
                checked,
                signatures,
                budget,
                depth + 1,
            )?;
            if !matches!(&expression.kind, ir::ExprKind::Borrow { kind: actual_kind, place: actual_place, timing: ir::BorrowTiming::Immediate }
                if actual_kind == kind && actual_place.local == source_local && actual_place.projection.is_empty())
            {
                return Err(FactsError::invalid(
                    budget,
                    "borrowed callable reference initializer changed",
                ));
            }
            sr::retain(budget, std::mem::size_of::<ir::Ty>() + 32)?;
            ir::Ty::Ref {
                kind: *kind,
                referent: Arc::new(referent),
            }
        }
        _ => {
            return Err(FactsError::unsupported(
                budget,
                "borrowed callable requires source callable or reference initializer",
            ));
        }
    };
    let actual = checked.locals.get(local.0).ok_or_else(|| {
        FactsError::invalid(budget, "borrowed callable local outside checked body")
    })?;
    if actual.mutable != *mutable
        || !replay_type_equal(&actual.ty, &ty, budget)?
        || !replay_type_equal(&expression.ty, &ty, budget)?
    {
        return Err(FactsError::invalid(
            budget,
            "borrowed callable initializer type or mutability changed",
        ));
    }
    Ok((local, ty))
}

fn borrowed_callable_leaf(
    ty: &ir::Ty,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<bool, FactsError> {
    sr::depth(budget, depth)?;
    sr::work(budget, 1)?;
    match ty {
        ir::Ty::Ref { referent, .. } => borrowed_callable_leaf(referent, budget, depth + 1),
        ir::Ty::Opaque(_) | ir::Ty::Closure(_) => Ok(true),
        _ => Ok(false),
    }
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
    const_inputs::validate_constants(original, &fresh, checked, budget)?;
    associated_constants::validate_symbolic_constants(original, &fresh, checked, budget)?;
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
            || fresh.for_iterable != retained.for_iterable
            || fresh.array_iter_source != retained.array_iter_source
            || fresh.intrinsic_candidate != retained.intrinsic_candidate
            || fresh.operator_candidate != retained.operator_candidate
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
    nominal_call_receipts::replay(original, checked, facts, budget)?;
    let mut decisions = Vec::new();
    let mut identity_decisions = 0usize;
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
    let array_iter_sources =
        iterator_source::replay_array_iters(original, checked, &positions, budget)?;
    iterator_source::validate_for_alternatives(
        original,
        &positions,
        checked,
        &builtin_sources,
        budget,
    )?;
    let intrinsic_sources =
        replay_builtin_intrinsics(original, checked, &positions, &mut seen, budget)?;
    replay_unsizes(original, checked, budget)?;
    let mut call_scratch = tr::TraitScratch::default();
    for (index, site) in original.calls.iter().enumerate() {
        sr::work(budget, 1)?;
        if builtin_sources[index]
            || leading_zero_sources[index]
            || intrinsic_sources[index]
            || array_iter_sources[index]
        {
            continue;
        }
        if site.operator_candidate && positions[index].is_none() {
            // A primitive operator: no overloaded call was emitted.
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
            ir::validate_callee_scratch(
                target,
                signature,
                ir::FactsCtx::Validated(facts),
                &mut call_scratch,
            )
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
            let (_, written) = path.application().map_err(|e| source_error(budget, &e))?;
            let anonymous = original
                .plan
                .registry
                .source_callable(expected_decl)
                .map_or(0, |f| f.impl_params);
            if anonymous > 0
                || written
                    .iter()
                    .any(type_check::partial_annotation::has_nested_infer)
            {
                if written.len() + anonymous != target.type_args.len() {
                    return Err(FactsError::invalid(
                        budget,
                        "partial source actual count changed",
                    ));
                }
                for (written, actual) in written.iter().zip(target.type_args.iter()) {
                    type_check::partial_annotation::matches_partial(written, actual, &signatures)
                        .map_err(|e| source_error(budget, &e))?;
                }
                let key = name_scope::path_key(
                    original.plan.program.names.as_ref(),
                    name,
                    Namespace::Value,
                )
                .map_err(|e| source_error(budget, &e))?;
                let regions = checker
                    .type_actual_regions(written, 0)
                    .map_err(|e| source_error(budget, &e))?;
                checker
                    .applications
                    .borrow_mut()
                    .push((key, target.type_args.to_vec(), regions));
            } else {
                checker
                    .push_application(path, Namespace::Value)
                    .map_err(|e| source_error(budget, &e))?;
            }
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
        // Borrowed callable values and scalar roots consumed by a call with a
        // reference-free result have no returned loan to infer. Authenticate
        // the source place and exact instantiated signature instead of applying
        // the identity-return rule. Scalar actuals additionally replay the
        // original one-slot input constraint, without a result context.
        // A freshly borrowed source root (`f(&mut r)`) passed as the only
        // argument of a callable whose result carries no reference likewise
        // has no returned loan; the ordinary borrow analysis checks the loan.
        if target.type_args.len() == 1
            && let ir::Ty::Ref { kind, referent } = &target.type_args[0]
            && (borrowed_callable_leaf(referent, budget, 0)?
                || (signature.params.len() == 1
                    && matches!(
                        referent.as_ref(),
                        ir::Ty::Int(_) | ir::Ty::Bool | ir::Ty::Char
                    ))
                || (signature.params.len() == 1 && site.argument_borrow == Some(*kind)))
            && signature
                .params
                .first()
                .is_some_and(|ty| ty == &target.type_args[0])
            && ir::carrier_locations(&signature.result, budget)
                .map_err(|e| source_error(budget, &e))?
                .is_empty()
        {
            let binding = site.argument_binding.as_ref().ok_or_else(|| {
                FactsError::unsupported(budget, "borrowed callable requires original local source")
            })?;
            let binding_index = source_binding_index(&original.bindings, binding, budget)?
                .ok_or_else(|| FactsError::invalid(budget, "borrowed callable source missing"))?;
            let scalar = matches!(
                referent.as_ref(),
                ir::Ty::Int(_) | ir::Ty::Bool | ir::Ty::Char
            );
            let plain_borrow = !scalar
                && !borrowed_callable_leaf(referent, budget, 0)?
                && site.argument_borrow == Some(*kind);
            let (local, source_ty) = if plain_borrow {
                // The root's checked type is authoritative here: the exact
                // borrow below must name this local with this referent type.
                let local = mapped_source_binding(original, binding_index, budget)?;
                let actual = checked.locals.get(local.0).ok_or_else(|| {
                    FactsError::invalid(budget, "borrow root outside checked locals")
                })?;
                if *kind == ir::BorrowKind::Mut && !actual.mutable {
                    return Err(FactsError::invalid(
                        budget,
                        "mutable borrow of an immutable source root",
                    ));
                }
                (local, actual.ty.clone())
            } else if scalar {
                if site.argument_borrow.is_some() {
                    source_root_borrow_type(original, binding_index, checked, &signatures, budget)?
                } else {
                    source_binding_type(original, binding_index, checked, &signatures, budget)?
                }
            } else {
                borrowed_callable_source_binding(
                    original,
                    &fresh,
                    binding_index,
                    checked,
                    &signatures,
                    budget,
                    0,
                )?
            };
            let first = args
                .first()
                .and_then(|arg| checked.exprs.get(arg.0))
                .ok_or_else(|| FactsError::invalid(budget, "borrowed callable argument missing"))?;
            let exact = if let Some(source_kind) = site.argument_borrow {
                source_kind == *kind
                    && replay_type_equal(&source_ty, referent, budget)?
                    && matches!(&first.kind, ir::ExprKind::Borrow { kind: actual_kind, place, timing: ir::BorrowTiming::Immediate }
                        if actual_kind == kind && place.local == local && place.projection.is_empty())
            } else {
                let source_value = match &first.kind {
                    ir::ExprKind::Local(actual) => *actual == local,
                    ir::ExprKind::Borrow {
                        kind: ir::BorrowKind::Mut,
                        place,
                        timing:
                            ir::BorrowTiming::TwoPhase {
                                activation,
                                argument: 0,
                                reason: ir::ImplicitBorrowKind::MutableArgumentReborrow { carrier },
                            },
                    } if !scalar
                        && *kind == ir::BorrowKind::Mut
                        && *activation == id
                        && place.local == local
                        && place.projection.as_slice() == [ir::Projection::Deref] =>
                    {
                        let carrier = checked.exprs.get(carrier.0).ok_or_else(|| {
                            FactsError::invalid(
                                budget,
                                "borrowed callable reborrow carrier missing",
                            )
                        })?;
                        matches!(carrier.kind, ir::ExprKind::Local(actual) if actual == local)
                            && replay_type_equal(&carrier.ty, &source_ty, budget)?
                    }
                    _ => false,
                };
                replay_type_equal(&source_ty, &target.type_args[0], budget)? && source_value
            };
            if !exact || !replay_type_equal(&first.ty, &target.type_args[0], budget)? {
                return Err(FactsError::invalid(
                    budget,
                    "borrowed callable changed source place or type",
                ));
            }
            if scalar {
                let input = if let Some(kind) = site.argument_borrow {
                    sr::retain(budget, std::mem::size_of::<ir::Ty>() + 32)?;
                    ir::Ty::Ref {
                        kind,
                        referent: Arc::new(source_ty.clone()),
                    }
                } else {
                    source_ty.clone()
                };
                let trace = sr::infer_single_input_trace(&view, &input, budget)?;
                if !replay_type_equal(trace.actual(), &target.type_args[0], budget)? {
                    return Err(FactsError::invalid(
                        budget,
                        "scalar input changed inferred actual",
                    ));
                }
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
            for parameter in signature.params.iter().skip(1) {
                if !ir::carrier_locations(parameter, budget)
                    .map_err(|e| source_error(budget, &e))?
                    .is_empty()
                {
                    return Err(FactsError::unsupported(
                        budget,
                        "additional borrowed callable argument carriers",
                    ));
                }
            }
            ir::validate_callee(
                &ir::Callee::Direct(target.clone()),
                signature,
                ir::FactsCtx::Validated(facts),
            )
            .map_err(|e| source_error(budget, &e))?;
            continue;
        }
        if let Some(literal) = fresh.literal_arguments.get(&index)
            && let [argument] = args.as_slice()
        {
            literal_call_inputs::validate(
                literal,
                checked,
                *argument,
                target,
                signature,
                &view,
                &signatures,
                caller_context.module,
                facts,
                &original.literal_temporaries,
                budget,
            )?;
            continue;
        }
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
        if !matches!(checked.body.statements.get(statement),Some(ir::TypedStmt::Let{local,value}) if *local==output && *value==id)
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
        identity_decisions += 1;
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
    if original.narrow.len() != identity_decisions {
        return Err(FactsError::invalid(
            budget,
            "unused or missing original shared frontier",
        ));
    }
    // Operator sites that stayed primitive have no call correspondence.
    let mut mapped = vec![false; original.calls.len()];
    for (index, _, _) in &original.call_mapping {
        if let Some(slot) = mapped.get_mut(*index) {
            *slot = true;
        }
    }
    for row in &original.builtin_len_mapping {
        if let Some(slot) = mapped.get_mut(row.source_index) {
            *slot = true;
        }
    }
    let primitive_operators = original
        .calls
        .iter()
        .zip(&mapped)
        .filter(|(site, mapped)| site.operator_candidate && !**mapped)
        .count();
    if original
        .call_mapping
        .len()
        .checked_add(original.builtin_len_mapping.len())
        .and_then(|n| n.checked_add(original.builtin_intrinsic_mapping.len()))
        .and_then(|n| n.checked_add(original.builtin_leading_zeros_mapping.len()))
        .and_then(|n| n.checked_add(original.array_iter_mapping.len()))
        .and_then(|n| n.checked_add(primitive_operators))
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
    /// Read-only declaration listing for the semantic core adapter.
    pub fn sema_declarations(&self) -> &[DeclInfo] {
        &self.registry.declarations
    }
    /// The declaration namespace this context issued.
    pub(in crate::compiler) fn registry_snapshot_id(&self) -> Result<ir::SnapshotId, String> {
        self.registry
            .declarations
            .first()
            .map(|info| info.id.snapshot)
            .ok_or_else(|| "missing declaration inventory".into())
    }
    /// Read-only resolved program for the semantic core adapter.
    pub fn sema_program(&self) -> &Arc<Program> {
        &self.program
    }
    /// Validated generic schema of a callable, if it has one.
    pub fn sema_schema(&self, id: ir::DeclId) -> Option<&ir::GenericSignature> {
        self.validated.get(&id)
    }
    /// Spelling of a declaration's own item name, if it has one.
    pub fn sema_item_name(&self, info: &DeclInfo) -> Option<String> {
        match info.path.item {
            DeclItemName::Source(ident) => {
                self.program.names.spelling(ident).ok().map(str::to_owned)
            }
            _ => Some(
                info.name
                    .rsplit("::")
                    .next()
                    .unwrap_or(&info.name)
                    .to_owned(),
            ),
        }
    }
    pub(super) fn publish_callable_body(
        &self,
        declaration: ir::DeclId,
        budget: &ir::PlanBudget,
    ) -> Result<PublishedCallableBody, FactsError> {
        let prepared = self.prepare_body_constants(declaration, budget)?;
        if !prepared.direct().is_empty() {
            return Err(FactsError::invalid(
                budget,
                "source constants require graph input handles",
            ));
        }
        let sealed = self.seal_body_constants(prepared, Vec::new(), budget)?;
        self.publish_callable_body_with_constants(declaration, sealed, budget)
    }
    pub(super) fn publish_callable_body_with_constants(
        &self,
        declaration: ir::DeclId,
        constants: const_inputs::SealedBodyConstants,
        budget: &ir::PlanBudget,
    ) -> Result<PublishedCallableBody, FactsError> {
        self.publish_callable_body_with_closures(declaration, constants, budget)
            .map(|(body, _)| body)
    }
    pub(super) fn publish_callable_body_with_closures(
        &self,
        declaration: ir::DeclId,
        constants: const_inputs::SealedBodyConstants,
        budget: &ir::PlanBudget,
    ) -> Result<(PublishedCallableBody, Vec<closures::check::CheckedClosure>), FactsError> {
        self.publish_callable_body_outputs(declaration, constants, budget)
            .map(|(body, closures, _)| (body, closures))
    }
    pub(super) fn publish_callable_body_outputs(
        &self,
        declaration: ir::DeclId,
        constants: const_inputs::SealedBodyConstants,
        budget: &ir::PlanBudget,
    ) -> Result<
        (
            PublishedCallableBody,
            Vec<closures::check::CheckedClosure>,
            Vec<ir::OpaqueHiddenType>,
        ),
        FactsError,
    > {
        match self.publish_callable_body_inner(declaration, constants, budget, None)? {
            NominalCallablePublication::Ready(body, closures, hidden) => {
                Ok((body, closures, hidden))
            }
            NominalCallablePublication::Blocked { .. } => Err(FactsError::invalid(
                budget,
                "ordinary publication suspended without nominal graph inputs",
            )),
        }
    }
    pub(super) fn publish_callable_body_with_nominal_inputs(
        &self,
        declaration: ir::DeclId,
        constants: const_inputs::SealedBodyConstants,
        budget: &ir::PlanBudget,
        receiver: Option<&NominalShape>,
        inputs: &NominalPreparationInputs<'_>,
    ) -> Result<NominalCallablePublication, FactsError> {
        self.publish_callable_body_inner(declaration, constants, budget, Some((receiver, inputs)))
    }
    fn publish_callable_body_inner(
        &self,
        declaration: ir::DeclId,
        mut constants: const_inputs::SealedBodyConstants,
        budget: &ir::PlanBudget,
        nominal: Option<(Option<&NominalShape>, &NominalPreparationInputs<'_>)>,
    ) -> Result<NominalCallablePublication, FactsError> {
        if constants.declaration != declaration {
            return Err(FactsError::invalid(
                budget,
                "sealed constant inputs changed callable owner",
            ));
        }
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
                if nominal.is_some() {
                    return Err(FactsError::unsupported(
                        budget,
                        "nominal source publication requires a source provider",
                    ));
                }
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
                Ok(NominalCallablePublication::Ready(
                    PublishedCallableBody::Recipe(Arc::new(checked)),
                    Vec::new(),
                    Vec::new(),
                ))
            }
            CallableProvider::UnavailableProvided { .. } => Err(FactsError::unsupported(
                budget,
                "unavailable canonical body",
            )),
            CallableProvider::Source { .. } => {
                let inventory = constants.inventory.take().ok_or_else(|| {
                    FactsError::invalid(budget, "sealed source inventory missing")
                })?;
                let abstract_source = self
                    .source_inventory_uses_opaque(declaration, &inventory, budget)
                    .map_err(|e| source_error(budget, &e))?;
                let facts = if env.is_generic() || abstract_source {
                    self.callable_facts(declaration)
                } else {
                    self.instance_facts(&ir::CallTarget {
                        declaration,
                        type_args: Arc::from([]),
                    })
                }
                .map_err(|e| source_error(budget, &e))?;
                let identity = inventory.owner.clone();
                let mut cap = CanonicalSourceCapability {
                    runtime_sites: Default::default(),
                    plan: self.clone(),
                    declaration,
                    facts: facts.clone(),
                    inventory,
                    constants: constants.uses,
                    symbolic_constants: constants.symbolic,
                    symbolic_constant_emissions: Default::default(),
                    constant_emissions: Default::default(),
                    calls: Default::default(),
                    nominal_call_realizations: Default::default(),
                    builtin_leading_zeros: Default::default(),
                    builtin_lens: Default::default(),
                    array_iters: Default::default(),
                    builtin_intrinsics: Default::default(),
                    unsizes: Default::default(),
                    bindings: Default::default(),
                    narrow: Default::default(),
                    emissions: Default::default(),
                    error: Default::default(),
                };
                if nominal.is_some() {
                    *cap.nominal_call_realizations.borrow_mut() = Some(Vec::new());
                }
                let checked = match nominal {
                    Some((receiver, inputs)) => {
                        type_check::type_callable_source_with_nominal_inputs(
                            &self.program,
                            declaration,
                            self,
                            budget,
                            Some(&cap),
                            receiver,
                            inputs,
                        )
                    }
                    None => type_check::type_callable_source_outputs(
                        &self.program,
                        declaration,
                        self,
                        budget,
                        Some(&cap),
                    )
                    .map(|(body, closures, hidden)| {
                        NominalCallableOutput::Ready(Box::new(body), closures, hidden)
                    }),
                };
                let (checked, closures, hidden) = match checked {
                    Ok(NominalCallableOutput::Ready(body, closures, hidden)) => {
                        (*body, closures, hidden)
                    }
                    Ok(NominalCallableOutput::Blocked(preparation)) => {
                        // The immutable inventory and typed constant facts are
                        // returned with their original owner/body identity.
                        // Partial local emissions are discarded, never sealed.
                        let constants = const_inputs::SealedBodyConstants {
                            declaration,
                            inventory: Some(cap.inventory),
                            uses: cap.constants,
                            symbolic: cap.symbolic_constants,
                        };
                        return Ok(NominalCallablePublication::Blocked {
                            preparation,
                            constants: Box::new(constants),
                        });
                    }
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
                let mut literal_temporaries = HashMap::new();
                for (index, id, _) in cap.calls.borrow().iter() {
                    sr::work(budget, 1)?;
                    if cap.inventory.literal_arguments.contains_key(index)
                        && let Some(ir::TypedExpr {
                            kind: ir::ExprKind::Call { args, .. },
                            ..
                        }) = checked.exprs.get(id.0)
                        && let [argument] = args.as_slice()
                    {
                        literal_call_inputs::capture_temporaries(
                            &checked,
                            *argument,
                            &mut literal_temporaries,
                            budget,
                            0,
                        )?;
                    }
                }
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
                    constants: std::mem::take(&mut cap.constants),
                    symbolic_constants: std::mem::take(&mut cap.symbolic_constants),
                    symbolic_constant_emissions: cap.symbolic_constant_emissions.take(),
                    constant_emissions: cap.constant_emissions.take(),
                    calls,
                    bindings: std::mem::take(&mut cap.inventory.bindings),
                    literal_temporaries,
                    call_mapping: cap.calls.take(),
                    nominal_call_realizations: cap.nominal_call_realizations.borrow().clone(),
                    builtin_leading_zeros_mapping: cap.builtin_leading_zeros.take(),
                    builtin_len_mapping: cap.builtin_lens.take(),
                    array_iter_mapping: cap.array_iters.take(),
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
                Ok(NominalCallablePublication::Ready(
                    PublishedCallableBody::Source(publication),
                    closures,
                    hidden,
                ))
            }
        }
    }
}

mod array_len;
mod array_len_receiver;
#[cfg(test)]
mod associated_bounds;
mod associated_declarations;
mod const_inputs;
pub(super) mod nominal_call_receipts;
pub(in crate::compiler) use const_inputs::SealedBodyConstants;
mod const_items;
#[cfg(test)]
mod const_source_tests;
mod index_bounds;
mod indexed_borrow;
#[cfg(test)]
mod leading_zero_tests;
mod mut_slice;
#[cfg(test)]
mod mutable_range_source_tests;
#[cfg(test)]
mod name_tests;
mod or_patterns;
mod primitive_constants;
mod range_expr;
mod raw_slices;
mod receiver_authority;
mod recipes;
#[cfg(test)]
mod slice_tests;
mod static_mut;
pub use tr::CallableProvider;

#[derive(Clone, Debug)]
pub struct Program {
    struct_sources: Arc<name_validation::StructSources>,
    embedded_issuers: Arc<[super::source::EmbeddedOriginalFile]>,
    no_std_roots: Arc<[super::token_session::SourceFileHandle]>,
    core_prelude_scope: Option<ModuleId>,
    core_library_module: Option<ModuleId>,
    generated_owners: Arc<[generated_owners::Certificate]>,
    generated_metadata: Arc<[generated_metadata::Attachment]>,
    c_abi: Arc<c_abi::Declarations>,
    pub names: Arc<NameStore>,
    pub modules: Arc<ModuleArena>,
    pub symbols: Arc<ProgramSymbols>,
    pub scopes: Option<Arc<super::semantic_scope::ScopeArena>>,
    pub functions: Vec<Function>,
    pub constants: Vec<ConstDecl>,
    pub structs: Vec<StructDecl>,
    pub enums: Vec<EnumDecl>,
    pub impls: Vec<ImplDecl>,
    pub traits: Vec<TraitDecl>,
    pub trait_imports: Vec<(ModuleId, TraitPath)>,
    /// Hidden crate-root module holding the embedded native prelude library,
    /// when loaded. `std::`/`core::` paths and prelude names resolve into it.
    pub prelude_module: Option<ModuleId>,
    /// The `prelude::v1` module inside the prelude library. Its public names
    /// (types, variants, traits) are in scope in every module unless shadowed.
    pub prelude_scope: Option<ModuleId>,
    /// Hidden crate-root modules holding mounted dependency crates
    /// (`__justrust_extern_<name>`). `<name>::` paths resolve into them and
    /// `crate` inside one names its mount.
    pub extern_mounts: Vec<ModuleId>,
    /// Crate-root `extern crate core|std as alias;` items: (crate root, alias
    /// path). They join that crate's extern prelude.
    extern_crate_aliases: Arc<[(ModuleId, AstPath)]>,
    // Resolver-issued source authority. Shared unchanged by public AST clones.
    associated_bound_sources: Arc<[AssociatedBoundSource]>,
    /// Immutable parser/resolver output, never a mutable callable side slot.
    closure_sources: Arc<HashMap<PathKey, closures::ClosureSource>>,
    closure_owners: Arc<HashMap<PathKey, Arc<[PathKey]>>>,
}
#[derive(Clone, Debug)]
struct AssociatedBoundSource {
    trait_index: usize,
    member_index: usize,
    module: ModuleId,
    trait_name: AstPath,
    member_name: AstIdent,
    bounds: Arc<[(GenericPath, TraitPath)]>,
}
/// Syntax-only result. Resolve this once before publishing callable signatures.
#[derive(Clone, Debug)]
pub struct ParsedProgram {
    pub program: Program,
    items: HashMap<PathKey, Vec<Item>>,
    empty_use_prefixes: Vec<(ModuleId, AstPath)>,
    /// `use prefix::*;` declarations: importing module, its canonical path,
    /// the glob target path and the import's visibility.
    glob_imports: Vec<GlobImport>,
    /// Child file entries of a single-file view parse, in child-site order.
    child_entries: Vec<ChildEntry>,
}
/// Where a child file's items mount (PARSE-SPLIT.md step C1): its module in
/// the parent file's arena and the textual macro scope at the declaration.
#[derive(Clone)]
pub(super) struct ChildEntry {
    pub(super) module: ModuleId,
    pub(super) path: AstPath,
    pub(super) macros: Option<super::source_item_macro::ModuleMacroCheckpoint>,
    /// The parent's textual scope point at the declaration.
    pub(super) frontier: Option<super::semantic_scope::ModuleFrontierId>,
}
impl ParsedProgram {
    /// Child file entries of a single-file view parse, in child-site order.
    pub(super) fn child_entries(&self) -> &[ChildEntry] {
        &self.child_entries
    }
}
impl std::fmt::Debug for ChildEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChildEntry")
            .field("module", &self.module)
            .field("macros", &self.macros.is_some())
            .finish()
    }
}
#[derive(Clone, Debug)]
struct GlobImport {
    module: ModuleId,
    module_path: AstPath,
    target: AstPath,
    visibility: ir::Visibility,
}
/// Parser construction state has no detached or placeholder name store. The
/// final program is published only after its one mutable owner is frozen.
#[derive(Default)]
struct ProgramDeclarations {
    generated_metadata: Vec<generated_metadata::Attachment>,
    no_std_roots: Arc<[super::token_session::SourceFileHandle]>,
    core_prelude_scope: Option<ModuleId>,
    core_library_module: Option<ModuleId>,
    c_abi: c_abi::Declarations,
    functions: Vec<Function>,
    constants: Vec<ConstDecl>,
    structs: Vec<StructDecl>,
    enums: Vec<EnumDecl>,
    impls: Vec<ImplDecl>,
    traits: Vec<TraitDecl>,
    trait_imports: Vec<(ModuleId, TraitPath)>,
    prelude_module: Option<ModuleId>,
    prelude_scope: Option<ModuleId>,
    extern_mounts: Vec<ModuleId>,
    extern_crate_aliases: Vec<(ModuleId, AstPath)>,
    closure_sources: HashMap<PathKey, closures::ClosureSource>,
}
struct ParsedDeclarations {
    program: ProgramDeclarations,
    items: HashMap<PathKey, Vec<Item>>,
    empty_use_prefixes: Vec<(ModuleId, AstPath)>,
    glob_imports: Vec<GlobImport>,
}
impl ProgramDeclarations {
    fn publish(
        self,
        names: Arc<NameStore>,
        modules: Arc<ModuleArena>,
        symbols: Arc<ProgramSymbols>,
        scopes: Option<Arc<super::semantic_scope::ScopeArena>>,
    ) -> Result<Program, ParseError> {
        let struct_sources = name_validation::capture_struct_sources(&names, &self.structs)?;
        Ok(Program {
            struct_sources,
            embedded_issuers: Arc::from([]),
            no_std_roots: self.no_std_roots,
            core_prelude_scope: self.core_prelude_scope,
            core_library_module: self.core_library_module,
            generated_owners: Arc::from([]),
            generated_metadata: self.generated_metadata.into(),
            c_abi: Arc::new(self.c_abi),
            names,
            modules,
            symbols,
            scopes,
            functions: self.functions,
            constants: self.constants,
            structs: self.structs,
            enums: self.enums,
            impls: self.impls,
            traits: self.traits,
            trait_imports: self.trait_imports,
            prelude_module: self.prelude_module,
            prelude_scope: self.prelude_scope,
            extern_mounts: self.extern_mounts,
            extern_crate_aliases: self.extern_crate_aliases.into(),
            associated_bound_sources: Arc::from([]),
            closure_sources: Arc::new(self.closure_sources),
            closure_owners: Arc::new(HashMap::new()),
        })
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
    pub constants: Vec<ConstDecl>,
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
        let input = super::source::PreparedLexInput::identity(
            name,
            source,
            budget,
            super::owner_path::standalone_issued(
                budget,
                super::owner_path::OwnerDomain::SourceSnapshot,
            )
            .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        // Direct fixture builder, never reached from a graph node.
        let owner = super::owner_path::standalone_issued(
            budget,
            super::owner_path::OwnerDomain::TokenSession,
        )
        .map_err(|error| error.to_string())?;
        let (mut session, cap) = super::token_session::TokenSession::with_owner(
            input.snapshot().clone(),
            budget.clone(),
            owner,
        )
        .map_err(|error| error.to_string())?;
        let fixture_anchor = session
            .original_span(input.eof().map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
        let mut names = NameStoreBuilder::new(session, cap, super::token::Edition::E2024)
            .map_err(|error| error.to_string())?;
        let symbols = ProgramSymbols::new(&mut names).map_err(native_adapter::name_diagnostic)?;
        let modules = ModuleArenaBuilder::for_builder(&mut names).map_err(module_diagnostic)?;
        Ok(Self {
            names,
            modules,
            symbols,
            fixture_anchor,
            functions: Vec::new(),
            constants: Vec::new(),
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
    #[cfg(test)]
    pub(crate) fn names_mut(&mut self) -> &mut NameStoreBuilder {
        &mut self.names
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
            c_abi: c_abi::Declarations::default(),
            functions: self.functions,
            constants: self.constants,
            structs: self.structs,
            enums: self.enums,
            impls: self.impls,
            traits: self.traits,
            trait_imports: self.trait_imports,
            prelude_module: None,
            prelude_scope: None,
            no_std_roots: Arc::from([]),
            core_prelude_scope: None,
            core_library_module: None,
            extern_mounts: Vec::new(),
            extern_crate_aliases: Vec::new(),
            closure_sources: HashMap::new(),
            generated_metadata: Vec::new(),
        }
        .publish(names, modules, self.symbols, None)
        .map_err(|error| error.to_string())?;
        program.validate_name_handles()?;
        Ok(program)
    }
}
impl Program {
    fn no_std_module(&self, mut module: ModuleId) -> Result<bool, String> {
        if self.no_std_roots.is_empty() {
            return Ok(false);
        }
        let source = self.modules.source_root().map_err(module_diagnostic)?;
        while let Some(parent) = self.modules.parent(module).map_err(module_diagnostic)? {
            if parent == source {
                break;
            }
            module = parent;
        }
        let root = if module == source {
            self.names.session().sources().file(0)
        } else {
            let name = self
                .modules
                .name(module)
                .map_err(module_diagnostic)?
                .ok_or("crate module has no original name")?;
            let span = self
                .names
                .span(name)
                .map_err(native_adapter::name_diagnostic)?;
            super::source_macro_binding::invocation_root(self.names.session(), span)
        }
        .map_err(|e| super::source::SourceError::from_token(e).to_string())?;
        for issued in self.no_std_roots.iter() {
            self.names
                .session()
                .sources()
                .text(*issued)
                .map_err(|e| super::source::SourceError::from_token(e).to_string())?;
        }
        Ok(self.no_std_roots.contains(&root))
    }
    fn implicit_prelude(&self, module: ModuleId) -> Result<Option<ModuleId>, String> {
        Ok(if self.no_std_module(module)? {
            self.core_prelude_scope
        } else {
            self.prelude_scope
        })
    }
    pub(super) fn in_prelude(&self, module: ModuleId) -> bool {
        self.prelude_module
            .is_some_and(|prelude| self.modules.is_descendant(module, prelude).unwrap_or(false))
    }
    /// User-crate struct count, excluding the embedded prelude library.
    pub fn user_structs(&self) -> usize {
        self.structs
            .iter()
            .filter(|s| !self.in_prelude(s.module))
            .count()
    }
    /// User-crate enum count, excluding the embedded prelude library.
    pub fn user_enums(&self) -> usize {
        self.enums
            .iter()
            .filter(|e| !self.in_prelude(e.module))
            .count()
    }
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
    Const,
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
/// Original-source immutable constant declaration. This parsed AST remains in
/// the source graph only, never in the published constant fact payloads.
#[derive(Clone, Debug)]
pub struct ConstDecl {
    pub module: ModuleId,
    pub visibility: ir::Visibility,
    pub name: AstPath,
    pub ty: Type,
    pub initializer: Expr,
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
    pub(super) embedded_source: Option<Arc<function_source::FunctionSource>>,
    pub lexical_scope: Option<super::semantic_scope::ScopeId>,
    pub module: ModuleId,
    pub constness: Constness,
    /// `unsafe fn`: calls require an unsafe context (E0133).
    pub unsafety: bool,
    /// Synthetic accessor of a `static mut` item (see static_mut.rs): uses
    /// of the item name are rewritten to `*accessor()`.
    pub static_mut: bool,
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
    pub regions: RegionDecls,
    pub(super) original_source: Option<super::token_session::OriginalSpan>,
    primitive_source: Option<Arc<primitive_constants::SourceImpl>>,
    pub trait_path: Option<TraitPath>,
    pub bounds: Vec<TypeBound>,
    pub module: ModuleId,
    pub type_params: Vec<AstIdent>,
    pub self_ty: Type,
    pub items: Vec<AssociatedFunction>,
    pub associated_types: Vec<AssociatedTypeValue>,
    pub associated_constants: Vec<AssociatedConstDecl>,
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
    pub subject: Type,
    pub traits: Vec<TraitPath>,
    /// Written `?Sized`: the parameter has no implicit `Sized` predicate.
    pub maybe_unsized: bool,
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
    pub associated_constants: Vec<AssociatedConstDecl>,
    /// Trailing type parameters (indices) defaulted to `Self` when omitted.
    pub self_defaults: Vec<usize>,
}
/// A genuine type member owned by its declaring trait.
#[derive(Clone, Debug)]
pub struct AssociatedTypeDecl {
    pub name: AstIdent,
    pub bounds: Vec<TraitPath>,
    /// Written `?Sized`: the associated type has no implicit `Sized` bound.
    pub relaxed_sized: bool,
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
    pub unsafety: bool,
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
    /// Declared bounds, as for [`StructDecl::bounds`].
    pub bounds: Vec<TypeBound>,
    pub region_params: Vec<AstIdent>,
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
    /// Declared bounds (`struct S<T: Ord>` / `where T: Ord`), proven for
    /// every concrete application built by a constructor.
    pub bounds: Vec<TypeBound>,
    pub region_params: Vec<AstIdent>,
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
    pub region_args: Vec<RegionName>,
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
        let mut applications = self
            .segments
            .iter()
            .filter(|s| !s.region_args.is_empty() || !s.args.is_empty());
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
    Opaque(Box<SourceOpaqueType>),
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
    /// Unsized trait object `dyn Trait`. Admitted only behind a reference.
    Dyn(Box<TraitPath>),
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
    /// Actual source `self` token, never a synthetic receiver identity.
    ReceiverOccurrence(super::semantic_path::ReceiverOccurrence),
}
#[derive(Clone, Debug)]
pub struct Place {
    pub local: LocalName,
    pub fields: Vec<FieldSelector>,
}
#[derive(Clone, Debug)]
pub enum Pattern {
    /// Original match-arm root alternatives, never a nested pattern grammar.
    Alternatives(Vec<Pattern>),
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
    /// `name @ pattern`: binds the whole matched place and tests `pattern`.
    At {
        name: AstIdent,
        mutable: bool,
        by_ref: Option<ir::BorrowKind>,
        pattern: Box<Pattern>,
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
    /// Assignment through a reference produced by a genuine call:
    /// `*call() OP= value`, `call().f OP= value`, `call().a[i] OP= value`.
    /// Stock evaluates `value`, then `carrier`, then `index`, exactly once.
    /// `fields` follow the implicit/explicit carrier Deref; `op` is None for `=`.
    AssignDeref {
        site: Option<super::expansion_span::SpanId>,
        carrier: Expr,
        fields: Vec<FieldSelector>,
        index: Option<Expr>,
        op: Option<BinaryOp>,
        value: Expr,
    },
    AssignIndex {
        site: Option<super::expansion_span::SpanId>,
        array: Place,
        index: Expr,
        value: Expr,
    },
    /// `m[i][j] OP= value` whose array base itself contains a runtime index
    /// rooted at a local (NESTED-INDEX.md). `target` is the full written
    /// `DynamicIndex` expression; `op` is None for `=`.
    AssignNestedIndex {
        target: Expr,
        op: Option<BinaryOp>,
        value: Expr,
    },
    AssignOp {
        site: Option<super::expansion_span::SpanId>,
        place: Place,
        op: BinaryOp,
        value: Expr,
    },
    AssignIndexOp {
        site: Option<super::expansion_span::SpanId>,
        array: Place,
        index: Expr,
        op: BinaryOp,
        value: Expr,
    },
    AssignPlace {
        site: Option<super::expansion_span::SpanId>,
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
                site: None,
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
    /// The diagnostic was already written to stderr by a preceding
    /// formatted output (`panic!("{}", x)`); the abort writes nothing more.
    pub preformatted: bool,
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
    TypeAlignment(Type),
    /// Source callee-expression invocation, resolved to the canonical callable
    /// trait method before active checking. Arguments retain one tuple value.
    Invoke {
        receiver: Box<Expr>,
        args: Vec<Expr>,
    },
    /// Bootstrap-only Linux x86-64 ABI primitive, never an allocator policy.
    NativeSyscall(Vec<Expr>),
    RawSlice {
        kind: ir::BorrowKind,
        arguments: Vec<Expr>,
    },
    /// Extract the data address from an exclusive slice descriptor.
    SliceDataPointer(Box<Expr>),
    UnexpandedMacro(Arc<UnexpandedMacroSyntax>),
    TraitCall {
        owner: Type,
        trait_path: Box<TraitPath>,
        name: AstIdent,
        type_args: Vec<Type>,
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
    /// Immutable program bytes, with exact type `&'static [u8; N]`.
    ByteStr(Vec<u8>),
    DynamicIndex {
        base: Box<Expr>,
        index: Box<Expr>,
        site: Option<super::expansion_span::SpanId>,
    },
    /// `base[start..end]` / `..=` / open bounds (SUBSLICE.md).
    RangeIndex {
        base: Box<Expr>,
        start: Option<Box<Expr>>,
        end: Option<Box<Expr>>,
        inclusive: bool,
    },
    Array(Vec<Expr>),
    Repeat {
        value: Box<Expr>,
        len: usize,
    },
    Index {
        base: Box<Expr>,
        index: usize,
        site: Option<super::expansion_span::SpanId>,
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
    /// Resolved module constant, never a local binding or mutable place.
    ConstItem(AstPath),
    AssociatedConst(Box<AssociatedConstUse>),
    /// `Owner::name` naming an associated function as a value. Checking
    /// selects the exact declaration and types it as `Ty::FnItem`.
    AssociatedFn(Box<AssociatedConstUse>),
    Unary {
        op: UnaryOp,
        value: Box<Expr>,
        site: Option<super::expansion_span::SpanId>,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
        /// Original source occurrence, retained across Pratt restructuring.
        /// Generated arithmetic has no site and cannot invent one later.
        site: Option<super::expansion_span::SpanId>,
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
        modes: Vec<ir::FormatMode>,
        specs: Vec<format_spec::FormatSpec>,
        stream: ir::OutputStream,
        newline: bool,
    },
    Output(Box<SourceOutput>),
    /// format!/write! placeholder calls (format_macros.rs).
    FormatInto(Box<SourceFormatInto>),
    Panic(Box<SourcePanic>),
    /// Postfix `?` on an Option or Result operand (TRY.md).
    Try(Box<Expr>),
}
#[derive(Clone, Debug)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub body: Expr,
    /// A builtin matches! guard lowered into the body's If condition.
    /// Retained so type checking can enforce the bounded guard binding rules.
    pub guarded: bool,
    /// Ordinary `pat if cond =>` match-arm guard (MATCH-GUARDS).
    pub guard: Option<Box<Expr>>,
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
    Byte,
    ByteString,
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
/// An assignment target with a runtime index strictly inside its place path
/// (`m[i][j]`, `m[i][0]`), rooted at a local through indexes and fields.
/// A single outermost runtime index (`a[i] = v`) keeps `AssignIndex`.
fn nested_runtime_target(target: &Expr) -> bool {
    match target {
        Expr::DynamicIndex { base, .. } | Expr::Index { base, .. } | Expr::Field { base, .. } => {
            nested_runtime_base(base)
        }
        _ => false,
    }
}
/// An array base that contains a runtime index and is rooted at a local
/// through indexes and fields only, e.g. the `m[i]` of `m[i][j] = v`.
fn nested_runtime_base(mut base: &Expr) -> bool {
    let mut runtime = false;
    for _ in 0..128 {
        match base {
            Expr::Var(_) => return runtime,
            Expr::DynamicIndex { base: inner, .. } => {
                runtime = true;
                base = inner;
            }
            Expr::Index { base: inner, .. } | Expr::Field { base: inner, .. } => base = inner,
            _ => return false,
        }
    }
    false
}
/// Split `*call()`, `call().f`, `(*call()).f`, `call().a[K]` and
/// `call().a[i]` assignment targets into the genuine call carrier, its
/// written selectors and an optional runtime index. Any other target is
/// returned unchanged for the existing local-rooted place grammar.
fn call_rooted_target(
    target: Expr,
    op: Option<BinaryOp>,
    value: Expr,
) -> Result<Stmt, Box<(Expr, Expr)>> {
    fn rooted(mut expr: &Expr) -> bool {
        for _ in 0..128 {
            match expr {
                Expr::Deref { value } => expr = value,
                Expr::Field { base, .. } | Expr::Index { base, .. } => expr = base,
                _ => return genuine_call(expr),
            }
        }
        false
    }
    let base = match &target {
        Expr::DynamicIndex { base, .. } => base.as_ref(),
        other => other,
    };
    // A projection-free call is not a place expression.
    if genuine_call(base) || !rooted(base) {
        return Err(Box::new((target, value)));
    }
    let site = match &target {
        Expr::DynamicIndex { site, .. } | Expr::Index { site, .. } => *site,
        _ => None,
    };
    let (mut expr, index) = match target {
        Expr::DynamicIndex { base, index, .. } => (*base, Some(*index)),
        other => (other, None),
    };
    let mut fields = Vec::new();
    loop {
        expr = match expr {
            Expr::Deref { value: carrier } if genuine_call(&carrier) => {
                fields.push(FieldSelector::Deref);
                fields.reverse();
                return Ok(Stmt::AssignDeref {
                    site,
                    carrier: *carrier,
                    fields,
                    index,
                    op,
                    value,
                });
            }
            Expr::Field { base, field } if genuine_call(&base) => {
                fields.push(field);
                fields.reverse();
                return Ok(Stmt::AssignDeref {
                    site,
                    carrier: *base,
                    fields,
                    index,
                    op,
                    value,
                });
            }
            Expr::Index {
                base,
                index: constant,
                ..
            } if genuine_call(&base) => {
                fields.push(FieldSelector::ConstantIndex(constant));
                fields.reverse();
                return Ok(Stmt::AssignDeref {
                    site,
                    carrier: *base,
                    fields,
                    index,
                    op,
                    value,
                });
            }
            Expr::Deref { value } => {
                fields.push(FieldSelector::Deref);
                *value
            }
            Expr::Field { base, field } => {
                fields.push(field);
                *base
            }
            Expr::Index {
                base,
                index: constant,
                ..
            } => {
                fields.push(FieldSelector::ConstantIndex(constant));
                *base
            }
            _ => unreachable!("call-rooted target checked above"),
        };
    }
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
#[cfg(test)]
fn lex(source: &str) -> Result<NativeParserInput, ParseError> {
    let budget = super::expansion_budget::ExpansionBudget::default();
    let projection = super::source::PreparedLexInput::identity(
        std::path::Path::new("<syntax>"),
        source,
        &budget,
        crate::compiler::owner_path::standalone_issued(
            &budget,
            crate::compiler::owner_path::OwnerDomain::SourceSnapshot,
        )
        .unwrap(),
    )
    .map_err(native_adapter::source_error)?;
    let owner =
        super::owner_path::standalone_issued(&budget, super::owner_path::OwnerDomain::TokenSession)
            .map_err(native_adapter::transport_error)?;
    native_input(&projection, budget, owner)
}
#[cfg(test)]
fn native_input(
    projection: &super::source::PreparedLexInput<'_>,
    budget: super::expansion_budget::ExpansionBudget,
    owner: super::owner_path::Issued,
) -> Result<NativeParserInput, ParseError> {
    native_input_for_edition(projection, budget, owner, super::token::Edition::E2024)
}
fn native_input_for_edition(
    projection: &super::source::PreparedLexInput<'_>,
    budget: super::expansion_budget::ExpansionBudget,
    owner: super::owner_path::Issued,
    edition: super::token::Edition,
) -> Result<NativeParserInput, ParseError> {
    native_input_with(projection, budget, owner, edition, false)
}
fn native_input_with(
    projection: &super::source::PreparedLexInput<'_>,
    budget: super::expansion_budget::ExpansionBudget,
    owner: super::owner_path::Issued,
    edition: super::token::Edition,
    inherits_macros: bool,
) -> Result<NativeParserInput, ParseError> {
    let (mut session, cap) = super::token_session::TokenSession::with_owner(
        projection.snapshot().clone(),
        budget,
        owner,
    )
    .map_err(native_adapter::transport_error)?;
    let lexed = super::lexer::lex_prepared(projection, edition, &mut session)
        .map_err(native_adapter::lex_error)?;
    if inherits_macros {
        return NativeParserInput::new_macro_bearing(lexed, session, cap, projection);
    }
    NativeParserInput::new(lexed, session, cap, projection)
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
    #[test]
    fn root_or_span_validation_work_is_independent_of_line_prefix() {
        use super::super::expansion_budget::{Counter, ExpansionBudget};
        fn measured(prefix: usize) -> u64 {
            let source = format!("{}0 | 1", " ".repeat(prefix));
            let budget = ExpansionBudget::default();
            let projection = super::super::source::PreparedLexInput::identity(
                std::path::Path::new("or.rs"),
                &source,
                &budget,
                crate::compiler::owner_path::standalone_issued(
                    &budget,
                    crate::compiler::owner_path::OwnerDomain::SourceSnapshot,
                )
                .unwrap(),
            )
            .unwrap();
            let (tokens, session, owner) = native_input(
                &projection,
                budget.clone(),
                super::super::owner_path::standalone_issued(
                    &budget,
                    super::super::owner_path::OwnerDomain::TokenSession,
                )
                .unwrap(),
            )
            .unwrap()
            .into_parts();
            let mut names =
                NameStoreBuilder::new(session, owner, super::super::token::Edition::E2024).unwrap();
            let mut parser = Parser::new(&tokens, &mut names).unwrap();
            let before = budget.snapshot().counters[Counter::Work as usize];
            assert!(
                matches!(parser.arm_pattern().unwrap(), Pattern::Alternatives(rows) if rows.len() == 2)
            );
            budget.snapshot().counters[Counter::Work as usize] - before
        }
        let short = measured(0);
        assert!(short > 0);
        assert_eq!(measured(128 * 1024), short);
    }
    fn syntax_with_budget(
        source: &str,
        budget: &super::super::expansion_budget::ExpansionBudget,
    ) -> Result<ParsedProgram, ParseError> {
        let input = super::super::source::PreparedLexInput::identity(
            std::path::Path::new("nested.rs"),
            source,
            budget,
            crate::compiler::owner_path::standalone_issued(
                &budget,
                crate::compiler::owner_path::OwnerDomain::SourceSnapshot,
            )
            .unwrap(),
        )
        .map_err(native_adapter::source_error)?;
        parse_syntax_tokens(native_input(
            &input,
            budget.clone(),
            super::super::owner_path::standalone_issued(
                budget,
                super::super::owner_path::OwnerDomain::TokenSession,
            )
            .unwrap(),
        )?)
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
        let receipt = super::super::owner_path::published_owner_receipt(&result.program.names)
            + result.program.struct_sources.receipt_bytes();
        drop(result);
        // Only the exact immutable name and struct authority receipts remain.
        assert_eq!(budget.snapshot().retained_bytes, receipt);
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
        let receipt = super::super::owner_path::published_owner_receipt(&program.names)
            + program.struct_sources.receipt_bytes();
        let owner = program.names.clone();
        drop(program);
        assert!(budget.snapshot().retained_bytes >= charged);
        drop(owner);
        // Format storage returns to the exact immutable authority receipts.
        assert_eq!(budget.snapshot().retained_bytes, receipt);
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
                        ..
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
        let (buffer, _session, _session_cap) = lex("x").unwrap().into_parts();
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
    pending_receiver: Option<AstIdent>,
    scopes: Option<super::semantic_scope::ScopeArenaBuilder>,
    function: Option<FunctionMacroState>,
    item_macros: Option<super::source_item_macro::SourceItemMacroDriver>,
    inactive_items: Option<super::token_session::BudgetVec<Arc<UnexpandedMacroSyntax>>>,
    /// Anonymous bodies remain independent checked callable declarations.
    closure_functions: Vec<Function>,
    closure_serial: usize,
    closure_depth: usize,
    closure_parent: Option<closures::ClosureParent>,
    closure_sources: Vec<closures::ClosureSource>,
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
    /// Prepared offsets just inside each external `mod child;` group, in
    /// file order (PARSE-SPLIT.md step C1). Empty for whole-crate parses.
    child_sites: Arc<[usize]>,
    /// Entry for each child site reached so far, recorded in file order.
    child_entries: Vec<ChildEntry>,
    forbid_unsafe_roots: Arc<[super::token_session::SourceFileHandle]>,
    embedded_issuers: Arc<[super::source::EmbeddedOriginalFile]>,
    no_std_roots: Arc<[super::token_session::SourceFileHandle]>,
    c_abi: c_abi::Declarations,
    pending_c_abi: bool,
    pending_c_export: bool,
    pending_repr_c: bool,
    foreign_c: bool,
    debug_assertions: bool,
    input: &'input NativeTokenBuffer,
    names: &'names mut NameStoreBuilder,
    modules: ParserState<'names, ModuleArenaBuilder>,
    lexical: ParserState<'names, ParserLexicalState>,
    expansion: Option<super::semantic_scope::ExpansionAdmission>,
    expansion_depth: u32,
    purpose: ParserPurpose,
    item_expansion: Option<super::semantic_scope::ItemExpansionAdmission>,
    generated_metadata: Vec<generated_metadata::Attachment>,
    module_frontier: Option<super::semantic_scope::ModuleFrontierId>,
    module: ModuleId,
    alias_target: bool,
    pending_bounds: Vec<TypeBound>,
    /// Bounds of the enclosing impl and of the current parameter list, read
    /// by `Param::Item` shorthand projections.
    visible_bounds: Vec<TypeBound>,
    /// Indices of type parameters written `= Self` by the last parameter list.
    pending_self_defaults: Vec<usize>,
    /// Lifetime parameters written by the last nominal/impl parameter list.
    pending_regions: RegionDecls,
    /// Lifetime parameters of the enclosing impl, inherited by its methods.
    impl_regions: RegionDecls,
    /// Anonymous parameters minted for argument-position `impl Trait` while a
    /// function parameter list is parsed. `None` forbids `impl Trait` (E0562).
    impl_trait_params: Option<Vec<AstIdent>>,
    opaque_return_sites: Option<u32>,
    cursor: TokenCursor<'input>,
    nesting: usize,
    allow_struct: bool,
    condition_purpose: ConditionPurpose,
    type_params: Vec<AstIdent>,
    use_expansions: usize,
    use_expansion_bytes: usize,
    /// Block-local `const` items, innermost block last (local_consts.rs).
    local_consts: local_consts::LocalConsts,
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
            child_sites: Arc::from([]),
            child_entries: Vec::new(),
            embedded_issuers: self.embedded_issuers.clone(),
            no_std_roots: self.no_std_roots.clone(),
            forbid_unsafe_roots: self.forbid_unsafe_roots.clone(),
            c_abi: c_abi::Declarations::default(),
            pending_c_abi: false,
            pending_c_export: false,
            pending_repr_c: false,
            foreign_c: false,
            debug_assertions: self.debug_assertions,
            input: &buffer,
            names: &mut *self.names,
            modules: ParserState::Borrowed(&mut *self.modules),
            lexical: ParserState::Borrowed(&mut *self.lexical),
            expansion: None,
            expansion_depth: 0,
            purpose: ParserPurpose::Active,
            item_expansion: Some(proof),
            generated_metadata: Vec::new(),
            module_frontier: self.module_frontier,
            module,
            alias_target: false,
            pending_bounds: Vec::new(),
            pending_self_defaults: Vec::new(),
            pending_regions: RegionDecls::default(),
            impl_regions: RegionDecls::default(),
            visible_bounds: Vec::new(),
            impl_trait_params: None,
            opaque_return_sites: None,
            cursor: TokenCursor::new(&buffer),
            nesting: self.nesting,
            allow_struct: true,
            condition_purpose: ConditionPurpose::Ordinary,
            type_params: Vec::new(),
            use_expansions: self.use_expansions,
            use_expansion_bytes: self.use_expansion_bytes,
            local_consts: local_consts::LocalConsts::default(),
        };
        let mut product = ParsedDeclarations {
            program: ProgramDeclarations::default(),
            items: HashMap::new(),
            empty_use_prefixes: Vec::new(),
            glob_imports: Vec::new(),
        };
        while !child.is("") {
            child.reserve_item_publication(&mut product)?;
            let start = child.cursor.checkpoint();
            let previous_count = product.program.functions.len();
            let metadata_start = child.generated_metadata.len();
            let nominal_start = product.program.structs.len();
            child.retain_generated_item_metadata()?;
            let metadata_end = child.generated_metadata.len();
            let (visibility, explicit_visibility) = child.visibility(module)?;
            if child.is("unsafe") {
                child.reject_forbidden_unsafe()?;
            }
            let item_start = [
                "fn", "trait", "impl", "struct", "enum", "type", "const", "static", "pub",
            ]
            .iter()
            .any(|token| child.is(token));
            if !item_start {
                let valid_unadmitted_item = [
                    "macro_rules",
                    "extern",
                    "unsafe",
                    "async",
                    "mod",
                    "use",
                    "#",
                ]
                .iter()
                .any(|token| child.is(token))
                    || child.token().is_ident()
                        && child.cursor.peek(1).is_some_and(|token| token.matches("!"));
                return Err(if valid_unadmitted_item {
                    child.unsupported("item macro output must contain only nongeneric free functions; generated attributes and other items are unsupported")
                } else if native_adapter::punctuation(child.token().text) && !child.is("@") {
                    child.error("expected a complete free function or end of item macro output")
                } else {
                    child.unsupported("expected an item in item macro output")
                });
            }
            child.nonmodule_item(
                module,
                module_path,
                &mut product,
                visibility,
                explicit_visibility,
                None,
            )?;
            child.finish_generated_metadata(
                &mut product.program,
                metadata_start..metadata_end,
                nominal_start,
            )?;
            if child.cursor.is_at(start) {
                return Err(native_adapter::transport_error(
                    super::token_session::TokenError::InvalidHandle,
                ));
            }
            let _ = previous_count;
        }
        // No function or empty expansion is published until the entire native
        // token buffer has been consumed. The shared expansion is provenance,
        // not a substitute for each function's exact occurrence and fresh scope.
        product
            .program
            .generated_metadata
            .append(&mut child.generated_metadata);
        drop(child);
        self.seal_emitted_callables(&product.program, &proof)?;
        for key in product.items.keys() {
            if parsed.items.contains_key(key) {
                return Err(self.error("duplicate item name"));
            }
        }
        for (key, items) in product.items.drain() {
            self.reserve_item_publication(parsed)?;
            parsed.items.insert(key, items);
        }
        let mut generated = product.program;
        generated_metadata::rebase(
            &mut generated.generated_metadata,
            parsed.program.structs.len(),
            parsed.program.impls.len(),
        )?;
        parsed.program.functions.extend(generated.functions);
        parsed.program.constants.extend(generated.constants);
        parsed.program.structs.extend(generated.structs);
        parsed.program.enums.extend(generated.enums);
        parsed.program.impls.extend(generated.impls);
        parsed.program.traits.extend(generated.traits);
        parsed
            .program
            .generated_metadata
            .extend(generated.generated_metadata);
        parsed.program.trait_imports.extend(generated.trait_imports);
        parsed
            .program
            .closure_sources
            .extend(generated.closure_sources);
        parsed.empty_use_prefixes.extend(product.empty_use_prefixes);
        parsed.glob_imports.extend(product.glob_imports);
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
        let file = super::source_macro_binding::definition_root(self.names.session(), span)
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
    #[inline(never)]
    fn local_macro_expression(&mut self) -> Result<Expr, ParseError> {
        if self.purpose == ParserPurpose::InactiveSyntax {
            let span = self.token().span;
            let path = self.path()?;
            self.expect("!")?;
            return Ok(Expr::UnexpandedMacro(self.inactive_invocation(path, span)?));
        }
        if self.builtin_macro_ahead("__justrust_align_of")?
            || self.builtin_macro_ahead("__justrust_storage_stride")?
        {
            return self.type_alignment_expression();
        }
        if self.builtin_macro_ahead("__justrust_linux_syscall6")? {
            return self.native_syscall_expression();
        }
        if self.builtin_macro_ahead("__justrust_slice_as_mut_ptr")? {
            return self.slice_data_pointer_expression();
        }
        for (name, kind) in [
            ("__justrust_slice_from_raw_parts", ir::BorrowKind::Shared),
            ("__justrust_slice_from_raw_parts_mut", ir::BorrowKind::Mut),
        ] {
            if self.builtin_macro_ahead(name)? {
                return self.raw_slice_expression(kind);
            }
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
        for spelling in ["format", "write", "writeln"] {
            if self.builtin_macro_ahead(spelling)? {
                return self.format_into_expression(spelling);
            }
        }
        if self.builtin_macro_ahead("vec")? {
            return self.vec_expression();
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
    // Syscall operands are unrelated to recursive user-macro expansion. Keep
    // their Expr/Result and allocation scratch out of each expansion frame.
    #[inline(never)]
    fn native_syscall_expression(&mut self) -> Result<Expr, ParseError> {
        self.ident()?;
        self.expect("!")?;
        self.expect("(")?;
        let mut arguments = Vec::new();
        for index in 0..7 {
            if index != 0 {
                self.expect(",")?;
            }
            let argument = self.expr(0)?;
            self.names
                .retain_parser_text(std::mem::size_of::<Expr>())
                .map_err(native_adapter::name_error)?;
            arguments.try_reserve_exact(1).map_err(|_| {
                native_adapter::transport_error(self.names.budget().poison_allocation().into())
            })?;
            arguments.push(argument);
        }
        self.eat(",");
        self.expect(")")?;
        Ok(Expr::NativeSyscall(arguments))
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
        let builtin = !self.known_module_macro(name)?;
        if builtin
            && matches!(
                spelling,
                "print"
                    | "println"
                    | "eprint"
                    | "eprintln"
                    | "vec"
                    | "format"
                    | "write"
                    | "writeln"
            )
        {
            let root =
                super::source_macro_binding::invocation_root(self.names.session(), token.span)
                    .map_err(native_adapter::transport_error)?;
            if self.no_std_roots.contains(&root) {
                return Err(self.unsupported("std/alloc macro is unavailable in this no_std crate; explicit library opt-in is unsupported"));
            }
        }
        Ok(builtin)
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
        let pattern = self.arm_pattern()?;
        let guarded = self.eat("if");
        let mut arm_guard = None;
        let body = if guarded {
            let frontier = self.enter_lexical()?;
            let guard = (|| {
                self.bind_pattern_lexical(&pattern)?;
                self.nested_expr()
            })();
            self.restore_frontier(frontier);
            let guard = guard?;
            if or_patterns::has_nested_alternatives(&pattern, false) {
                // An or-pattern guard retries each matching alternative. A
                // boolean-valued body would return false after the first one.
                arm_guard = Some(
                    self.names
                        .try_parser_box(guard)
                        .map_err(native_adapter::name_error)?,
                );
                Expr::Bool(true)
            } else {
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
            guard: arm_guard,
        });
        arms.push(MatchArm {
            pattern: Pattern::Wildcard,
            body: Expr::Bool(false),
            guarded: false,
            guard: None,
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
        let (mut pieces, holders) =
            format_spec::parse_format(&text).map_err(|message| self.error(&message))?;
        let (arguments, modes, specs) = self.format_arguments(close, &holders)?;
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
                    + arguments.len() * std::mem::size_of::<Expr>()
                    + modes.len() * std::mem::size_of::<ir::FormatMode>(),
            )
            .map_err(native_adapter::name_error)?;
        Ok(Expr::Format {
            pieces,
            arguments,
            modes,
            specs,
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
        let legacy = self.legacy_panic_literal(close, 0)?;
        if legacy.is_none() && !self.is(close) && self.panic_has_format() {
            return self.formatted_panic(close, spelling, default, site);
        }
        let message = if let Some(text) = legacy {
            if spelling == "panic" {
                text
            } else {
                self.prefixed_panic_message(default, Some(&text))?
            }
        } else if self.is(close) {
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
            .try_parser_box(SourcePanic {
                message,
                site,
                preformatted: false,
            })
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
                site: None,
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
        let legacy = if self.is(",") && comparison.is_none() {
            self.legacy_panic_literal(close, 1)?
        } else {
            None
        };
        let message = if let Some(text) = legacy {
            text
        } else if self.eat(",") && !self.is(close) {
            if self.panic_has_format() {
                // Stock prints the formatted message (assert!) or the
                // comparison header plus message (assert_eq!/assert_ne!).
                let (spelling, header) = match comparison {
                    Some(BinaryOp::Eq) => ("assert_eq", "assertion `left == right` failed"),
                    Some(BinaryOp::Ne) => ("assert_ne", "assertion `left != right` failed"),
                    _ => ("panic", ""),
                };
                let failure = self.formatted_panic(close, spelling, header, site)?;
                let condition = self
                    .names
                    .try_parser_box(condition)
                    .map_err(native_adapter::name_error)?;
                let failure = self
                    .names
                    .try_parser_box(failure)
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
                return Ok(expression);
            }
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
            .try_parser_box(SourcePanic {
                message,
                site,
                preformatted: false,
            })
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
    /// Registers the visible module-level `macro_rules!` definition of `name`
    /// with this function's local driver, so expression/statement-position
    /// invocations expand through the ordinary local path.
    fn import_module_macro(
        &mut self,
        name: AstIdent,
        call: super::semantic_scope::FrontierId,
    ) -> Result<Option<super::source_macro::CheckedDef>, ParseError> {
        let (Some(items), Some(module_call), Some(scopes)) = (
            &self.lexical.item_macros,
            self.module_frontier,
            &self.lexical.scopes,
        ) else {
            return Ok(None);
        };
        let Some((body, site, origin)) = items
            .visible_import(name, module_call, scopes, self.names)
            .map_err(|error| self.driver_error(error))?
        else {
            return Ok(None);
        };
        let definition = self
            .lexical
            .scopes
            .as_mut()
            .ok_or_else(|| {
                native_adapter::transport_error(super::token_session::TokenError::InvalidHandle)
            })?
            .module_definition_frontier(origin, call)
            .map_err(scope_parse_error)?;
        let function = self.lexical.function.as_mut().ok_or_else(|| {
            native_adapter::transport_error(super::token_session::TokenError::InvalidHandle)
        })?;
        let driver = function.driver.as_mut().ok_or_else(|| {
            native_adapter::transport_error(super::token_session::TokenError::InvalidHandle)
        })?;
        let defined = driver.define(name, body, site, definition, self.names);
        defined.map_err(|error| self.driver_error(error))?;
        let scopes = self.lexical.scopes.as_ref().ok_or_else(|| {
            native_adapter::transport_error(super::token_session::TokenError::InvalidHandle)
        })?;
        let driver = self
            .lexical
            .function
            .as_ref()
            .and_then(|function| function.driver.as_ref())
            .ok_or_else(|| {
                native_adapter::transport_error(super::token_session::TokenError::InvalidHandle)
            })?;
        let found = driver.lookup(name, call, scopes, self.names);
        found.map_err(|error| self.driver_error(error))
    }
    fn probe_module_macro(&mut self, probe: &TokenCursor<'input>) -> Result<bool, ParseError> {
        let mut at = TokenCursor::new(self.input);
        if !at.restore(probe.checkpoint()) {
            return Ok(false);
        }
        let saved = std::mem::replace(&mut self.cursor, at);
        let name = self.ident();
        self.cursor = saved;
        match name {
            Ok(name) => self.known_module_macro(name),
            Err(_) => Ok(false),
        }
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
            None => match self.import_module_macro(name, call)? {
                Some(selected) => selected,
                None => {
                    return Err(self.error("unknown function-local macro at this lexical point"));
                }
            },
        };
        let driver = self
            .lexical
            .function
            .as_ref()
            .and_then(|function| function.driver.as_ref())
            .ok_or_else(|| self.unsupported("function-local macro context required"))?;
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
            child_sites: Arc::from([]),
            child_entries: Vec::new(),
            embedded_issuers: self.embedded_issuers.clone(),
            no_std_roots: self.no_std_roots.clone(),
            forbid_unsafe_roots: self.forbid_unsafe_roots.clone(),
            c_abi: c_abi::Declarations::default(),
            pending_c_abi: false,
            pending_c_export: false,
            pending_repr_c: false,
            foreign_c: false,
            debug_assertions: self.debug_assertions,
            input: buffer,
            names: &mut *self.names,
            modules: ParserState::Borrowed(&mut *self.modules),
            lexical: ParserState::Borrowed(&mut *self.lexical),
            expansion,
            expansion_depth,
            purpose: self.purpose,
            item_expansion: self.item_expansion,
            generated_metadata: Vec::new(),
            module_frontier: self.module_frontier,
            module: self.module,
            alias_target: false,
            pending_bounds: Vec::new(),
            pending_self_defaults: Vec::new(),
            pending_regions: RegionDecls::default(),
            impl_regions: RegionDecls::default(),
            visible_bounds: Vec::new(),
            impl_trait_params: None,
            opaque_return_sites: None,
            cursor: TokenCursor::new(buffer),
            nesting: self.nesting,
            allow_struct: self.allow_struct,
            condition_purpose: self.condition_purpose,
            type_params: std::mem::take(&mut self.type_params),
            use_expansions: self.use_expansions,
            use_expansion_bytes: self.use_expansion_bytes,
            // Items are unhygienic: expanded expressions see enclosing
            // block-local consts like any other item.
            local_consts: std::mem::take(&mut self.local_consts),
        };
        let result = child.expr(0).and_then(|value| {
            if !child.is("") {
                Err(child.unsupported("macro output must be exactly one complete expression"))
            } else {
                Ok(value)
            }
        });
        self.type_params = std::mem::take(&mut child.type_params);
        self.local_consts = std::mem::take(&mut child.local_consts);
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
        self.reference_lexical_binding(ident).map(|_| ())
    }
    /// Records the reference and reports whether it names a local binding.
    fn reference_lexical_binding(&mut self, ident: AstIdent) -> Result<bool, ParseError> {
        if let Some(frontier) = self.frontier() {
            return Ok(self
                .lexical
                .scopes
                .as_mut()
                .ok_or_else(|| {
                    native_adapter::transport_error(super::token_session::TokenError::InvalidInput(
                        "missing parser lexical owner",
                    ))
                })?
                .resolve_reference(self.names, frontier, ident)
                .map_err(scope_parse_error)?
                .is_some());
        }
        Ok(false)
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
                Pattern::At { name, pattern, .. } => {
                    self.bind_lexical(*name)?;
                    pending
                        .push(pattern)
                        .map_err(native_adapter::transport_error)?;
                }
                Pattern::Tuple(fields) | Pattern::Alternatives(fields) => {
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
        self.function_body_parameters(params, None)
    }
    fn function_body_parameters(
        &mut self,
        params: &[(BindingName, Type)],
        patterns: Option<&[(Pattern, Type)]>,
    ) -> Result<(Block, Option<super::semantic_scope::ScopeId>), ParseError> {
        let receiver = self.lexical.pending_receiver.take();
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
        let module_macros = self
            .lexical
            .item_macros
            .as_ref()
            .is_some_and(|driver| !driver.is_empty());
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
            if module_macros
                && probe.peek(1).is_some_and(|token| token.matches("!"))
                && self.probe_module_macro(&probe)?
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
                super::semantic_scope::ScopeArenaBuilder::for_builder(self.names)
                    .map_err(scope_parse_error)?,
            );
        }
        let scopes = self.lexical.scopes.as_mut().ok_or_else(|| {
            native_adapter::transport_error(super::token_session::TokenError::InvalidInput(
                "missing parser lexical owner",
            ))
        })?;
        let frontier = scopes
            .receiver_function_root(self.names, receiver)
            .map_err(scope_parse_error)?;
        let owner = scopes.function(frontier).map_err(scope_parse_error)?;
        let driver = super::source_macro::LocalMacroDriver::new(source, owner, self.names)
            .map_err(|error| self.driver_error(error))?;
        let previous = self.lexical.function.replace(FunctionMacroState {
            frontier,
            driver: Some(driver),
        });
        self.bind_receiver_lexical(receiver)?;
        for (name, _) in params {
            if let BindingName::Named(name) = name {
                self.bind_lexical(*name)?;
            }
        }
        if let Some(patterns) = patterns {
            for (pattern, _) in patterns {
                self.bind_pattern_lexical(pattern)?;
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
        let modules = ModuleArenaBuilder::for_builder(names).map_err(module_parse_error)?;
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
            child_sites: Arc::from([]),
            child_entries: Vec::new(),
            embedded_issuers: Arc::from([]),
            forbid_unsafe_roots: Arc::from([]),
            no_std_roots: Arc::from([]),
            debug_assertions: super::source::CfgSet::DEFAULT_DEBUG_ASSERTIONS,
            c_abi: c_abi::Declarations::default(),
            pending_c_abi: false,
            pending_c_export: false,
            pending_repr_c: false,
            foreign_c: false,
            input: tokens,
            names,
            modules,
            lexical,
            expansion: None,
            expansion_depth: 0,
            purpose,
            item_expansion: None,
            generated_metadata: Vec::new(),
            module_frontier: None,
            module,
            alias_target: false,
            pending_bounds: Vec::new(),
            pending_self_defaults: Vec::new(),
            pending_regions: RegionDecls::default(),
            impl_regions: RegionDecls::default(),
            visible_bounds: Vec::new(),
            impl_trait_params: None,
            opaque_return_sites: None,
            cursor: TokenCursor::new(tokens),
            nesting: 0,
            allow_struct: true,
            condition_purpose: ConditionPurpose::Ordinary,
            type_params: Vec::new(),
            use_expansions: 0,
            use_expansion_bytes: 0,
            local_consts: local_consts::LocalConsts::default(),
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
            "mod", "impl", "trait", "self", "Self", "as", "move", "unsafe", "where", "type", "ref",
            "extern", "crate", "super", "abstract", "become", "box", "do", "final", "macro",
            "override", "priv", "typeof", "unsized", "virtual", "yield",
        ];
        t.is_ident()
            && t.text != "_"
            && t.text
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && !reserved.contains(&t.text)
            && (!matches!(t.text, "gen" | "async" | "await" | "dyn" | "try")
                || self
                    .input
                    .edition_keyword_is_reserved(&t)
                    .is_ok_and(|reserved| !reserved))
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
        self.ty_at_position(self.opaque_return_sites.is_some())
    }
    pub(super) fn ty_at_return(&mut self) -> Result<Type, ParseError> {
        self.ty_at_position(true)
    }
    fn ty_at_position(&mut self, direct_return: bool) -> Result<Type, ParseError> {
        if self.nesting >= 128 {
            return Err(self.limit("type nesting exceeds limit 128"));
        }
        let start = self.token();
        self.nesting += 1;
        let result = self.ty_inner();
        self.nesting -= 1;
        let result = result?;
        // Stable Rust permits a directly written return type `!`, but not a
        // parenthesized type, parameter, field, argument or nested return
        // type containing it. Internal Never values do not pass this parser.
        if direct_return && start.text == "!" && matches!(result, Type::Never) {
            return Ok(result);
        }
        use super::expansion_budget::{Counter, Storage};
        let budget = self.names.budget();
        let mut pending = super::token_session::BudgetVec::new(budget, Storage::Temporary)
            .map_err(native_adapter::transport_error)?;
        pending
            .push((&result, 0usize))
            .map_err(native_adapter::transport_error)?;
        while let Some((ty, depth)) = pending.pop() {
            budget
                .charge(Counter::Work, 1)
                .map_err(|error| native_adapter::transport_error(error.into()))?;
            if depth > 128 {
                return Err(self.limit("type nesting exceeds limit 128"));
            }
            let mut push = |child| {
                pending
                    .push((child, depth + 1))
                    .map_err(native_adapter::transport_error)
            };
            let path =
                match ty {
                    Type::Never => return Err(self.error_at(
                        &start,
                        "E0658: the never type is experimental outside function return position",
                    )),
                    Type::Ref { referent, .. }
                    | Type::RawPtr {
                        pointee: referent, ..
                    }
                    | Type::Array(referent, _)
                    | Type::Slice(referent) => {
                        push(referent.as_ref())?;
                        None
                    }
                    Type::Tuple(fields) => {
                        for field in fields {
                            push(field)?;
                        }
                        None
                    }
                    Type::Opaque(opaque) => {
                        for child in opaque.types() {
                            push(child)?;
                        }
                        None
                    }
                    Type::Apply(path) => Some(path),
                    Type::Dyn(trait_path) => match trait_path.as_ref() {
                        TraitPath::Source(path) => Some(path),
                        _ => None,
                    },
                    Type::Projection(projection) => match projection.as_ref() {
                        SourceProjection::Qualified {
                            self_ty,
                            trait_path,
                            ..
                        } => {
                            push(self_ty)?;
                            match trait_path {
                                TraitPath::Source(path) => Some(path),
                                _ => None,
                            }
                        }
                        SourceProjection::SelfItem { .. } => None,
                    },
                    _ => None,
                };
            if let Some(path) = path {
                for segment in &path.segments {
                    for child in segment.args.iter().chain(
                        segment
                            .associated_types
                            .iter()
                            .map(|binding| &binding.value),
                    ) {
                        push(child)?;
                    }
                }
            }
        }
        Ok(result)
    }
    fn ty_inner(&mut self) -> Result<Type, ParseError> {
        if self.is("fn") {
            return Err(self.unsupported("function pointer types are unsupported"));
        }
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
        if self.eat("_") {
            return Ok(Type::Infer);
        }
        if self.eat("!") {
            return Ok(Type::Never);
        }
        if self.eat("&") {
            let region = if self.token().is_lifetime() && self.token().text == "'static" {
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
            return if self.opaque_return_sites.is_some() {
                self.opaque_return_type()
            } else {
                self.impl_trait_param()
            };
        }
        if self.eat("dyn") {
            if self.token().is_lifetime() || self.is("?") || self.is("(") {
                return Err(self.unsupported("unsupported trait object bound syntax"));
            }
            let path = self.trait_path()?;
            if self.is("+") {
                return Err(self.unsupported(
                    "trait objects with auto-trait or lifetime bounds (`dyn A + B`) are unsupported",
                ));
            }
            return Ok(Type::Dyn(Box::new(path)));
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
                return self.param_shorthand_projection(name);
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
            let mut path = self.generic_path(true)?;
            self.rewrite_local_item_generic(&mut path)?;
            if path.segments.iter().any(|s| {
                !s.region_args.is_empty() || !s.args.is_empty() || !s.associated_types.is_empty()
            }) {
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
        let name = self
            .names
            .anonymous_impl_parameter(site, minted.len())
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
            subject: Type::Param(name),
            traits,
            maybe_unsized: false,
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
        let (_, args, associated_types) = self.path_arguments()?;
        if !associated_types.is_empty() {
            return Err(
                self.unsupported("associated equalities are unsupported in method type arguments")
            );
        }
        Ok(args)
    }
    #[inline(never)]
    fn path_arguments(&mut self) -> Result<ParsedPathArguments, ParseError> {
        // Split the first `<` of a nested qualified associated projection.
        if !self.cursor.consume_type_lt() {
            return Err(self.error("expected \"<\""));
        }
        let mut args = Vec::new();
        let mut region_args = Vec::new();
        let mut associated_types = Vec::new();
        loop {
            if region_args.len() + args.len() + associated_types.len() >= 32 {
                return Err(self.limit("generic argument resource limit 32"));
            }
            if self.token().is_lifetime() && !self.token().string {
                if !args.is_empty() || !associated_types.is_empty() {
                    return Err(self.error("lifetime arguments must precede type arguments"));
                }
                let region = if self.eat("'static") {
                    RegionName::Static
                } else if self.eat("'_") {
                    RegionName::Infer
                } else {
                    self.lifetime()?
                };
                self.reserve_parser_items(&mut region_args, 1)?;
                region_args.push(region);
                if !self.eat(",") || self.token().text.starts_with('>') {
                    break;
                }
                continue;
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
        Ok((region_args, args, associated_types))
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
            let sugar_site = self.token().span;
            let callable_sugar = type_context
                && matches!(self.token().text, "Fn" | "FnMut" | "FnOnce")
                && self.cursor.peek(1).is_some_and(|token| token.matches("("));
            let name = self.ident()?;
            let (region_args, args, associated_types) = if callable_sugar {
                let opaque_context = self.opaque_return_sites.take();
                let sugar = self.callable_sugar_arguments(sugar_site);
                self.opaque_return_sites = opaque_context;
                let (args, associated_types) = sugar?;
                (Vec::new(), args, associated_types)
            } else if type_context && (self.is("<") || self.is("<<")) {
                self.path_arguments()?
            } else {
                (Vec::new(), Vec::new(), Vec::new())
            };
            self.reserve_parser_items(&mut segments, 1)?;
            segments.push(PathSegment {
                name,
                region_args,
                args,
                associated_types,
            });
            if !self.eat("::") {
                break;
            }
            if self.is("<") {
                let (region_args, args, associated_types) = self.path_arguments()?;
                if !segments.last().unwrap().region_args.is_empty()
                    || !segments.last().unwrap().args.is_empty()
                    || !segments.last().unwrap().associated_types.is_empty()
                {
                    return Err(self.error("duplicate generic arguments"));
                }
                segments.last_mut().unwrap().args = args;
                segments.last_mut().unwrap().region_args = region_args;
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
        self.pending_self_defaults.clear();
        self.pending_regions = RegionDecls::default();
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
                if !params.is_empty() {
                    return Err(self.error("lifetime parameters must precede type parameters"));
                }
                if self.pending_regions.params.len() >= 64 {
                    return Err(self.limit("declared lifetime limit64"));
                }
                let name = self.lifetime()?;
                let RegionName::Named(ident) = name else {
                    return Err(self.error("expected declared lifetime"));
                };
                let mut regions = std::mem::take(&mut self.pending_regions);
                regions.params.push(ident);
                let bounded = self.region_bounds(name, &mut regions);
                self.pending_regions = regions;
                bounded?;
                if !self.eat(",") || self.token().text.starts_with('>') {
                    break;
                }
                continue;
            }
            let name = self.predeclared_parameter(&declarations, params.len())?;
            if self.contains_name(&params, name, LookupClass::TypeParameter)? {
                return Err(self.error("duplicate type parameter"));
            }
            self.reserve_parser_items(&mut params, 1)?;
            params.push(name);
            if self.eat(":") {
                let mut traits = Vec::new();
                let mut maybe_unsized = false;
                loop {
                    if self.relaxed_sized()? {
                        maybe_unsized = true;
                    } else {
                        let path = self.trait_path()?;
                        self.reserve_parser_items(&mut traits, 1)?;
                        traits.push(path);
                    }
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
                    subject: Type::Param(name),
                    traits,
                    maybe_unsized,
                });
                self.pending_bounds = bounds;
            }
            if self.eat("=") {
                // `Rhs = Self` style defaults (trait declarations only).
                if !self.eat("Self") {
                    return Err(
                        self.unsupported("generic defaults other than `Self` are unsupported")
                    );
                }
                self.pending_self_defaults.push(params.len() - 1);
            }
            if !self.eat(",") || self.token().text.starts_with('>') {
                break;
            }
        }
        self.close_angle()?;
        Ok(params)
    }
    /// `?Sized` in a bound list. Any other `?Trait` is rejected like stock.
    fn relaxed_sized(&mut self) -> Result<bool, ParseError> {
        if !self.is("?") {
            return Ok(false);
        }
        self.cursor.bump();
        if !self.eat("Sized") {
            return Err(self.error("relaxing a default bound only does something for `?Sized`"));
        }
        Ok(true)
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
                region_args: Vec::new(),
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
        if self.eat("*") {
            if prefix.anchor() == PathAnchor::Relative && prefix.is_empty() {
                return Err(
                    self.unsupported("glob imports of the crate root namespace are unsupported")
                );
            }
            imports.push((None, prefix.clone(), true));
            return Ok(());
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
                    // Edition 2015: a non-anchored `use` path is relative to
                    // the crate root. `std`/`core`/`alloc` are crate-root
                    // `extern crate` items there, so they keep resolving
                    // through the extern prelude identically.
                    if self.names.edition() == super::token::Edition::E2015
                        && path.anchor() == PathAnchor::Relative
                        && path.is_empty()
                        && !matches!(
                            self.names
                                .spelling(name)
                                .map_err(native_adapter::name_error)?,
                            "std" | "core" | "alloc"
                        )
                    {
                        path = path.with_anchor(PathAnchor::Crate);
                    }
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
                if self.eat("*") {
                    imports.push((None, path, true));
                    return Ok(());
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
        // The supported length bound is a semantic limit. Inactive items are
        // validated for syntax only and never lowered.
        let inactive = self.purpose == ParserPurpose::InactiveSyntax;
        if value.bits > ir::MAX_ARRAY_TYPE_LEN as u128 && !inactive {
            return Err(self.limit("array length/index exceeds supported limit"));
        }
        let number = value.bits.min(ir::MAX_ARRAY_TYPE_LEN as u128) as usize;
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
        if let Some(start) = self.cursor.index() {
            let tokens: &[Token] = self.input;
            if let Some(end) = array_len::length_end(tokens, start) {
                let table = array_len::scan(tokens);
                if let Some(value) = array_len::evaluate(tokens, &table, start..end) {
                    if value > ir::MAX_ARRAY_TYPE_LEN as u128
                        && self.purpose != ParserPurpose::InactiveSyntax
                    {
                        return Err(self.limit("array length/index exceeds supported limit"));
                    }
                    while self.cursor.index().is_some_and(|at| at < end) {
                        self.cursor.bump();
                    }
                    return Ok(value.min(ir::MAX_ARRAY_TYPE_LEN as u128) as usize);
                }
            }
        }
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
    // Construction and destruction of drivers involve large fallible values.
    // Keep them out of every recursive module frame, and retain only a paid
    // boxed parent while descending through an ordinary 2 MiB thread stack.
    #[inline(never)]
    fn enter_module_macros(
        &mut self,
        module: ModuleId,
        previous_frontier: Option<super::semantic_scope::ModuleFrontierId>,
    ) -> Result<Option<Box<super::source_item_macro::SourceItemMacroDriver>>, ParseError> {
        if self.purpose != ParserPurpose::Active || self.input.macro_source().is_none() {
            return Ok(None);
        }
        if self.lexical.scopes.is_none() {
            self.lexical.scopes = Some(
                super::semantic_scope::ScopeArenaBuilder::for_builder(self.names)
                    .map_err(scope_parse_error)?,
            );
        }
        let source = self.input.macro_source().cloned().ok_or_else(|| {
            native_adapter::transport_error(super::token_session::TokenError::InvalidHandle)
        })?;
        let parent = if let Some(mut parent) = self.lexical.item_macros.take() {
            let checkpoint = parent
                .checkpoint(self.names)
                .map_err(|error| self.driver_error(error))?;
            self.lexical.item_macros = Some(
                super::source_item_macro::SourceItemMacroDriver::from_checkpoint(
                    source,
                    &checkpoint,
                    self.names,
                )
                .map_err(|error| self.driver_error(error))?,
            );
            Some(
                self.names
                    .try_parser_box(parent)
                    .map_err(native_adapter::name_error)?,
            )
        } else {
            self.lexical.item_macros = Some(
                super::source_item_macro::SourceItemMacroDriver::new(source, self.names)
                    .map_err(|error| self.driver_error(error))?,
            );
            None
        };
        let scopes = self.lexical.scopes.as_mut().ok_or_else(|| {
            native_adapter::transport_error(super::token_session::TokenError::InvalidHandle)
        })?;
        self.module_frontier = Some(
            scopes
                .module_root(self.names, &*self.modules, module, previous_frontier)
                .map_err(scope_parse_error)?,
        );
        Ok(parent)
    }
    #[inline(never)]
    fn leave_module_macros(
        &mut self,
        parent: Option<Box<super::source_item_macro::SourceItemMacroDriver>>,
    ) {
        if let Some(parent) = parent {
            self.lexical.item_macros = Some(*parent);
        }
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
        let parent_macros = self.enter_module_macros(module, previous_frontier)?;
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
            let function_source_start = self.cursor.index();
            self.c_attributes()?;
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
                let (child, full) =
                    self.inline_module_header(module, module_path, parsed, visibility)?;
                self.items(child, &full, parsed)?;
                self.expect("}")?;
                continue;
            }
            self.nonmodule_item(
                module,
                module_path,
                parsed,
                visibility,
                explicit_visibility,
                function_source_start,
            )?;
            self.publish_local_consts(module, module_path, parsed)?;
        }
        self.module = previous_module;
        self.module_frontier = previous_frontier;
        self.leave_module_macros(parent_macros);
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
        function_source_start: Option<usize>,
    ) -> Result<(), ParseError> {
        // `extern crate` is not a foreign block: require `extern {` or `extern "ABI" {`.
        let legacy_foreign = self.is("extern")
            && (self.cursor.peek(1).is_some_and(|t| t.matches("{"))
                || (self.cursor.peek(1).is_some_and(|t| t.string)
                    && self.cursor.peek(2).is_some_and(|t| t.matches("{"))));
        if (legacy_foreign
            || (self.is("unsafe") && self.cursor.peek(1).is_some_and(|t| t.matches("extern"))))
            && !self.starts_qualified_function()
        {
            if explicit_visibility || self.pending_c_export || self.pending_repr_c {
                return Err(
                    self.unsupported("attributes or visibility on extern blocks are unsupported")
                );
            }
            return self.c_foreign_block(module, module_path, parsed);
        }
        if self.pending_repr_c && !self.is("struct") {
            return Err(self.unsupported("repr(C) currently requires a struct"));
        }
        if self.pending_c_export && !self.starts_qualified_function() {
            return Err(self.unsupported("no_mangle requires a free extern C function"));
        }
        if self.is("extern") && self.cursor.peek(1).is_some_and(|t| t.matches("crate")) {
            return self.extern_crate_item(module, module_path, parsed, visibility);
        }
        if (!self.token().string
            && matches!(
                self.token().text,
                "async" | "extern" | "union" | "macro_rules"
            )
            || (self.is("unsafe") && !self.cursor.peek(1).is_some_and(|t| t.text == "fn")))
            && !self.starts_qualified_function()
        {
            return Err(self.unsupported("item syntax is unsupported"));
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
                    if type_only {
                        parsed.glob_imports.push(GlobImport {
                            module,
                            module_path: module_path.clone(),
                            target: path,
                            visibility: visibility.clone(),
                        });
                    } else {
                        parsed.empty_use_prefixes.push((module, path));
                    }
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
            let self_defaults = std::mem::take(&mut self.pending_self_defaults);
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
            let mut associated_constants = Vec::new();
            let mut item_names = std::collections::HashSet::new();
            while !self.is("}") {
                if items
                    .len()
                    .saturating_add(associated_types.len())
                    .saturating_add(associated_constants.len())
                    >= 4096
                {
                    return Err(self.limit("trait item limit exceeded"));
                }
                self.retain_generated_item_metadata()?;
                if self.is("pub") {
                    return Err(self.error("trait items cannot declare visibility"));
                }
                if self.eat("type") {
                    let name = self.ident()?;
                    if self.is("<") || self.is("where") {
                        return Err(self.unsupported("generic associated types are unsupported"));
                    }
                    let mut associated_bounds = Vec::new();
                    let mut relaxed_sized = false;
                    if self.eat(":") {
                        loop {
                            if self.is("?") {
                                // Only `?Sized`; any other relaxation is rejected
                                // like stock. Repeating it is also an error.
                                self.relaxed_sized()?;
                                if relaxed_sized {
                                    return Err(self.error("duplicate relaxed `Sized` bounds"));
                                }
                                relaxed_sized = true;
                                if !self.eat("+") {
                                    break;
                                }
                                continue;
                            }
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
                        relaxed_sized,
                    });
                    continue;
                }
                if self.is("const") && !self.starts_qualified_function() {
                    let constant =
                        self.associated_constant(true, visibility.clone(), &type_params)?;
                    if !item_names.insert(self.name_key(constant.name, LookupClass::Member)?) {
                        return Err(self.error("duplicate trait value member"));
                    }
                    self.reserve_parser_items(&mut associated_constants, 1)?;
                    associated_constants.push(constant);
                    continue;
                }
                let (signature, receiver) = self.function_head(&type_params, true)?;
                if !item_names.insert(self.name_key(signature.name, LookupClass::Member)?) {
                    return Err(self.error("duplicate required trait item"));
                }
                let default = if self.is("{") {
                    let (body, lexical_scope) =
                        self.emitted_callable_body(signature.name, &signature.params)?;
                    Some(Function {
                        embedded_source: None,
                        lexical_scope,
                        module,
                        constness: signature.constness,
                        unsafety: signature.unsafety,
                        static_mut: false,
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
                associated_constants,
                self_defaults,
            });
            (name, ItemKind::Trait)
        } else if self.eat("enum") {
            let name = self.ident()?;
            let type_params = self.type_parameters()?;
            let region_params = self.pending_regions.params.clone();
            self.type_params = type_params.clone();
            let bounds = self.nominal_bounds()?;
            self.set_visible_bounds(&bounds)?;
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
                bounds,
                region_params,
                name: module_path
                    .push(self.names, name)
                    .map_err(native_adapter::name_error)?,
                visibility: visibility.clone(),
                variants,
            });
            (name, ItemKind::Enum)
        } else if self.eat("struct") {
            let name = self.ident()?;
            if std::mem::take(&mut self.pending_repr_c) {
                self.names
                    .retain_parser_text(128)
                    .map_err(native_adapter::name_error)?;
                self.c_abi
                    .repr_c
                    .try_reserve(1)
                    .map_err(|_| self.limit("repr(C) metadata allocation"))?;
                let key = self
                    .names
                    .key(name, LookupClass::Item(Namespace::Type))
                    .map_err(native_adapter::name_error)?;
                self.c_abi.repr_c.insert((module, key));
            }
            let type_params = self.type_parameters()?;
            let region_params = self.pending_regions.params.clone();
            self.type_params = type_params.clone();
            let mut bounds = self.nominal_bounds()?;
            self.set_visible_bounds(&bounds)?;
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
                // Tuple structs put `where` after the field list.
                bounds.extend(self.nominal_bounds()?);
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
                // Empty braced declarations remain records, never unit
                // constructors. Source-shape authority is retained separately.
                StructShape::Record
            };
            parsed.program.structs.push(StructDecl {
                shape,
                module,
                type_params,
                bounds,
                region_params,
                name: module_path
                    .push(self.names, name)
                    .map_err(native_adapter::name_error)?,
                visibility: visibility.clone(),
                fields,
            });
            (name, ItemKind::Struct)
        } else if self.is("impl") {
            let original_start = primitive_constants::original_token(self);
            self.expect("impl")?;
            if explicit_visibility {
                return Err(self.error("pub impl is invalid"));
            }
            let type_params = self.type_parameters()?;
            self.impl_regions = std::mem::take(&mut self.pending_regions);
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
            self.set_visible_bounds(&bounds)?;
            self.expect("{")?;
            let mut items = Vec::new();
            let mut associated_types = Vec::new();
            let mut associated_constants = Vec::new();
            while !self.is("}") {
                if items
                    .len()
                    .saturating_add(associated_types.len())
                    .saturating_add(associated_constants.len())
                    >= 4096
                {
                    return Err(self.limit("associated item limit exceeded"));
                }
                self.retain_generated_item_metadata()?;
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
                if self.is("const") && !self.starts_qualified_function() {
                    let constant =
                        self.associated_constant(false, visibility.clone(), &type_params)?;
                    self.reserve_parser_items(&mut associated_constants, 1)?;
                    associated_constants.push(constant);
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
            let original_source = primitive_constants::original_impl_range(self, original_start);
            self.expect("}")?;
            let mut block = ImplDecl {
                regions: self.impl_regions.clone(),
                original_source,
                primitive_source: None,
                trait_path,
                bounds,
                module,
                type_params,
                self_ty,
                items,
                associated_types,
                associated_constants,
            };
            block.primitive_source = primitive_constants::capture(
                &block,
                parsed.program.impls.len(),
                &self.embedded_issuers,
                self.names.session().sources(),
                self.names.budget(),
            )
            .map_err(|message| self.error(&message))?;
            parsed.program.impls.push(block);
            self.type_params.clear();
            self.visible_bounds.clear();
            self.impl_regions = RegionDecls::default();
            return Ok(());
        } else if self.is("static") && self.cursor.peek(1).is_some_and(|t| t.text == "mut") {
            let function = self.static_mut_item(module)?;
            let name = *function
                .name
                .segments()
                .last()
                .ok_or_else(|| self.error("empty static name"))?;
            let mut function = function;
            function.visibility = visibility.clone();
            function.name = module_path
                .push(self.names, name)
                .map_err(native_adapter::name_error)?;
            parsed.program.functions.push(function);
            (name, ItemKind::Function)
        } else if self.is("static") || (self.is("const") && !self.starts_qualified_function()) {
            if self.item_expansion.is_some() {
                return Err(self.unsupported("generated const item authority is unsupported"));
            }
            // Immutable statics share constant semantics in this draft: every
            // admitted initializer is pure, so each use observes the same value.
            if !self.eat("static") {
                self.expect("const")?;
            }
            let name = self.ident()?;
            self.expect(":")?;
            let ty = self.ty()?;
            self.expect("=")?;
            let initializer = self.nested_expr()?;
            self.expect(";")?;
            self.reserve_parser_items(&mut parsed.program.constants, 1)?;
            parsed.program.constants.push(ConstDecl {
                module,
                visibility: visibility.clone(),
                name: module_path
                    .push(self.names, name)
                    .map_err(native_adapter::name_error)?,
                ty,
                initializer,
            });
            (name, ItemKind::Const)
        } else if self.starts_qualified_function() {
            let mut function = self.function()?;
            function.visibility = visibility.clone();
            let name = function
                .name
                .single()
                .ok_or_else(|| self.error("function declaration must have a single name"))?;
            function.name = module_path
                .push(self.names, name)
                .map_err(native_adapter::name_error)?;
            function.embedded_source = function_source::issue_parsed(
                self,
                function_source_start,
                parsed.program.functions.len(),
                &function,
            )
            .map_err(|message| self.error(&message))?;
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
        self.visible_bounds.clear();
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
                    | (ItemKind::Function, ItemKind::Const)
                    | (ItemKind::Const, ItemKind::Function)
                    | (ItemKind::Const, ItemKind::Const)
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
            "where", "while", "abstract", "become", "box", "do", "final", "macro", "override",
            "priv", "typeof", "unsized", "virtual", "yield",
        ]
        .contains(&name)
            || (matches!(name, "gen" | "async" | "await" | "dyn" | "try")
                && self.input.edition_keyword_is_reserved(&self.token())?)
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
    /// Inline parameter bounds plus an optional `where` clause on a struct
    /// or enum declaration.
    fn nominal_bounds(&mut self) -> Result<Vec<TypeBound>, ParseError> {
        let mut bounds = std::mem::take(&mut self.pending_bounds);
        if self.eat("where") {
            loop {
                if self.token().is_lifetime() {
                    return Err(self.unsupported(
                        "lifetime where predicates on nominal declarations are unsupported",
                    ));
                }
                self.where_type_predicate(&mut bounds)?;
                if !self.eat(",") || self.is("{") || self.is(";") {
                    break;
                }
            }
        }
        Ok(bounds)
    }
    fn where_type_predicate(&mut self, bounds: &mut Vec<TypeBound>) -> Result<(), ParseError> {
        if self.is("for") {
            return Err(self.unsupported("higher-ranked where predicates are unsupported"));
        }
        let subject = self.ty()?;
        self.expect(":")?;
        let mut traits = Vec::new();
        let mut maybe_unsized = false;
        while !(self.is(",") || self.is("{") || self.is(";")) {
            if self.relaxed_sized()? {
                maybe_unsized = true;
                if !self.eat("+") {
                    break;
                }
                continue;
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
        if maybe_unsized && !matches!(subject, Type::Param(_)) {
            return Err(self.error("this relaxed bound is not permitted here"));
        }
        if bounds.len() >= 4096 {
            return Err(self.limit("predicate limit exceeded"));
        }
        self.reserve_parser_items(bounds, 1)?;
        bounds.push(TypeBound {
            subject,
            traits,
            maybe_unsized,
        });
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
        let enclosing_bounds = self.visible_bounds.clone();
        let (head, receiver) = self.function_head(inherited, associated)?;
        let mut body_bounds = enclosing_bounds.clone();
        self.reserve_parser_items(&mut body_bounds, head.bounds.len())?;
        body_bounds.extend(head.bounds.iter().cloned());
        self.set_visible_bounds(&body_bounds)?;
        let external = std::mem::take(&mut self.pending_c_abi) || self.foreign_c;
        let exported = std::mem::take(&mut self.pending_c_export);
        if external && associated {
            return Err(self.unsupported("associated extern C functions are unsupported"));
        }
        let previous_parent = self.lexical.closure_parent.take();
        if !associated {
            self.lexical.closure_parent = Some(closures::ClosureParent {
                name: self.closure_path(head.name)?,
                params: head.params.clone(),
            });
        }
        let result = (|| {
            let (body, lexical_scope) = if self.foreign_c {
                self.expect(";")?;
                // Foreign declarations have no Rust body. The uninhabited
                // source carrier traverses normal signature/CFG authority,
                // but OBJECT emits only the issued native import bridge.
                (
                    Block {
                        statements: Vec::new(),
                        tail: Some(Box::new(Expr::Loop {
                            label: None,
                            body: Block {
                                statements: Vec::new(),
                                tail: None,
                            },
                        })),
                    },
                    None,
                )
            } else {
                self.emitted_callable_body(head.name, &head.params)?
            };
            Ok((
                Function {
                    embedded_source: None,
                    lexical_scope,
                    module: self.module,
                    constness: head.constness,
                    unsafety: head.unsafety || self.foreign_c,
                    static_mut: false,
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
        })();
        let result = result.and_then(|(function, receiver)| {
            self.record_c_function(&function, external, exported)?;
            Ok((function, receiver))
        });
        self.lexical.closure_parent = previous_parent;
        self.visible_bounds = enclosing_bounds;
        result
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
                    let mut maybe_unsized = false;
                    loop {
                        if traits.len() >= 256 {
                            return Err(self.limit("predicate limit exceeded"));
                        }
                        if self.relaxed_sized()? {
                            maybe_unsized = true;
                        } else {
                            let path = self.trait_path()?;
                            self.reserve_parser_items(&mut traits, 1)?;
                            traits.push(path);
                        }
                        if !self.eat("+") {
                            break;
                        }
                    }
                    let mut bounds = std::mem::take(&mut self.pending_bounds);
                    self.reserve_parser_items(&mut bounds, 1)?;
                    bounds.push(TypeBound {
                        subject: Type::Param(name),
                        traits,
                        maybe_unsized,
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
        // `const fn` is callable at runtime with identical semantics; constant
        // initializers that call one are inlined at their uses (const_inline),
        // so the qualifier carries no further obligation in this draft.
        // Associated `const fn`s are ordinary methods here: their qualifier
        // only matters inside constant initializers, which are inlined.
        self.lexical.pending_receiver = None;
        let (constness, unsafety) = self.function_qualifiers(associated)?;
        self.expect("fn")?;
        let name = self.ident()?;
        let mut regions = if associated {
            self.impl_regions.clone()
        } else {
            RegionDecls::default()
        };
        self.type_params.clear();
        let mut visible = std::mem::take(&mut self.type_params);
        self.reserve_parser_items(&mut visible, inherited.len())?;
        visible.extend_from_slice(inherited);
        self.type_params = visible;
        let type_params = self.callable_generics(&mut regions)?;
        let mut bounds = std::mem::take(&mut self.pending_bounds);
        let enclosing_bounds = std::mem::take(&mut self.visible_bounds);
        {
            let mut visible = enclosing_bounds.clone();
            self.reserve_parser_items(&mut visible, bounds.len())?;
            visible.extend(bounds.iter().cloned());
            self.visible_bounds = visible;
        }
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
            self.receiver_binding_token()?;
            receiver = ReceiverKind::Borrowed { kind };
            if !self.is(")") {
                self.expect(",")?;
            }
        } else if associated
            && (self.is("self")
                || (self.is("mut")
                    && self
                        .cursor
                        .peek(1)
                        .is_some_and(|token| token.matches("self"))))
        {
            let mutable = self.eat("mut");
            self.receiver_binding_token()?;
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
                if (self.foreign_c || self.pending_c_abi) && (self.is("...") || self.is("..")) {
                    return Err(self.unsupported("C variadic functions are unsupported"));
                }
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
                if self.names.edition() == super::token::Edition::E2015
                    && !self.is(":")
                    && (self.is(",") || self.is(")") || self.is("<") || self.is("::"))
                {
                    return Err(self.unsupported(
                        "anonymous trait method parameters (edition 2015) are unsupported; name the parameter (`_: T`)",
                    ));
                }
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
        let result = self.opaque_function_result()?;
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
        self.visible_bounds = enclosing_bounds;
        Ok((
            FunctionHead {
                constness,
                unsafety,
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
        let result = self.enter_local_const_scope().and_then(|()| {
            let result = self.block_inner();
            self.leave_local_const_scope();
            result
        });
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
        let result = result.and_then(|first| {
            // Nested or-patterns; the checker flattens them into root rows.
            if !self.is("|") {
                return Ok(first);
            }
            let mut rows = vec![first];
            while self.eat("|") {
                self.names
                    .budget()
                    .charge(super::expansion_budget::Counter::Work, 1)
                    .map_err(|error| native_adapter::transport_error(error.into()))?;
                self.reserve_rest_patterns(&mut rows, 1)?;
                rows.push(self.pattern_inner()?);
            }
            Ok(Pattern::Alternatives(rows))
        });
        let result = result.and_then(|pattern| {
            if self.is("@") {
                Err(self.unsupported("@ patterns are unsupported"))
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
    fn arm_pattern(&mut self) -> Result<Pattern, ParseError> {
        let first = self.pattern_no_alternatives()?;
        if !self.is("|") {
            if self.is("@") {
                return Err(self.unsupported("@ patterns are unsupported"));
            }
            if self.is("..") || self.is("..=") {
                return Err(
                    self.unsupported("path and constant range pattern bounds are unsupported")
                );
            }
            return Ok(first);
        }
        // Generated match owners have not been granted original OR authority.
        let site = self.token().span;
        let span = self
            .names
            .session()
            .span(site)
            .map_err(native_adapter::transport_error)?;
        if span.expansion.is_some()
            || !matches!(span.origin, super::expansion_span::SpanOrigin::Original(_))
        {
            return Err(self.unsupported("generated or-pattern owners are unsupported"));
        }
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
            .budget()
            .charge(super::expansion_budget::Counter::Work, 1)
            .map_err(|error| native_adapter::transport_error(error.into()))?;
        self.names
            .session()
            .sources()
            .render_span(original)
            .map_err(native_adapter::transport_error)?;
        let mut rows = Vec::new();
        self.reserve_rest_patterns(&mut rows, 1)?;
        rows.push(first);
        while self.eat("|") {
            self.names
                .budget()
                .charge(super::expansion_budget::Counter::Work, 1)
                .map_err(|error| native_adapter::transport_error(error.into()))?;
            self.reserve_rest_patterns(&mut rows, 1)?;
            rows.push(self.pattern_no_alternatives()?);
        }
        if self.is("@") {
            return Err(self.unsupported("@ patterns are unsupported"));
        }
        Ok(Pattern::Alternatives(rows))
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
                || self.token().is_byte()
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
    /// Bytes reuse the rigid u8 integer path, including inference and patterns.
    fn byte_integer_literal(&mut self) -> Result<IntegerLiteral, ParseError> {
        self.names
            .budget()
            .charge(
                super::expansion_budget::Counter::Work,
                self.token().text.len() as u64,
            )
            .map_err(|error| native_adapter::transport_error(error.into()))?;
        let value = parse_byte_literal(self.token().text).map_err(|e| self.error(e))?;
        self.cursor.bump();
        Ok(IntegerLiteral {
            magnitude: u128::from(value),
            suffix: Some(ir::IntTy::U8),
        })
    }
    /// One literal range or scalar pattern endpoint: char, byte, or optionally negated integer.
    fn range_bound(&mut self) -> Result<RangeBound, ParseError> {
        if self.token().is_byte() {
            return Ok(RangeBound::Int {
                literal: self.byte_integer_literal()?,
                negative: false,
            });
        }
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
        let pattern = self.pattern_atom()?;
        if !self.is("@") {
            return Ok(pattern);
        }
        let Pattern::Binding {
            name,
            mutable,
            by_ref,
        } = pattern
        else {
            return Err(self.error("left-hand side of `@` must be a binding"));
        };
        if self.cursor.peek(1).is_some_and(|token| token.matches("..")) {
            return Err(self.unsupported("@ rest (subslice) bindings are unsupported"));
        }
        self.cursor.bump();
        let inner = self.pattern_no_alternatives()?;
        let pattern = self
            .names
            .try_parser_box(inner)
            .map_err(native_adapter::name_error)?;
        Ok(Pattern::At {
            name,
            mutable,
            by_ref,
            pattern,
        })
    }
    fn pattern_atom(&mut self) -> Result<Pattern, ParseError> {
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
        self.pattern_named_atom()
    }
    /// Constructor parsing owns substantial path/field temporaries. Keep it
    /// off the recursive frame for parenthesized and tuple patterns.
    #[inline(never)]
    fn pattern_named_atom(&mut self) -> Result<Pattern, ParseError> {
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
    /// Parse the remainder of `TARGET = VALUE;`. Kept out of line so the
    /// recursive block parser frame does not grow with assignment temporaries.
    #[inline(never)]
    fn plain_assignment_statement(&mut self, target: Expr) -> Result<Stmt, ParseError> {
        let value = self.expr(0)?;
        self.expect(";")?;
        let (target, value) = match call_rooted_target(target, None, value) {
            Ok(statement) => return Ok(statement),
            Err(unchanged) => *unchanged,
        };
        if nested_runtime_target(&target) {
            return Ok(Stmt::AssignNestedIndex {
                target,
                op: None,
                value,
            });
        }
        if let Expr::DynamicIndex { base, index, site } = target {
            return Ok(Stmt::AssignIndex {
                site,
                array: self.assignment_place(*base)?,
                index: *index,
                value,
            });
        }
        let site = match &target {
            Expr::Index { site, .. } => *site,
            _ => None,
        };
        let place = self.assignment_place(target)?;
        Ok(if place.fields.is_empty() {
            Stmt::Assign {
                name: place.local,
                value,
            }
        } else {
            Stmt::AssignPlace { site, place, value }
        })
    }
    /// After `base[` and an optional start, at `..` or `..=`: parse the rest
    /// of a range index through the closing `]`.
    #[inline(never)]
    fn range_index(&mut self, base: Expr, start: Option<Expr>) -> Result<Expr, ParseError> {
        let inclusive = self.eat("..=");
        if !inclusive {
            self.expect("..")?;
        }
        let end = if self.is("]") {
            if inclusive {
                return Err(self.error("inclusive range index requires an end bound"));
            }
            None
        } else {
            Some(Box::new(self.nested_expr()?))
        };
        self.expect("]")?;
        Ok(Expr::RangeIndex {
            base: Box::new(base),
            start: start.map(Box::new),
            end,
            inclusive,
        })
    }
    #[inline(never)]
    fn compound_statement(&mut self, target: Expr, op: BinaryOp) -> Result<Stmt, ParseError> {
        let value = self.expr(0)?;
        self.expect(";")?;
        let (target, value) = match call_rooted_target(target, Some(op), value) {
            Ok(statement) => return Ok(statement),
            Err(unchanged) => *unchanged,
        };
        if nested_runtime_target(&target) {
            return Ok(Stmt::AssignNestedIndex {
                target,
                op: Some(op),
                value,
            });
        }
        if let Expr::DynamicIndex { base, index, site } = target {
            return Ok(Stmt::AssignIndexOp {
                site,
                array: self.assignment_place(*base)?,
                index: *index,
                op,
                value,
            });
        }
        let site = match &target {
            Expr::Index { site, .. } => *site,
            _ => None,
        };
        Ok(Stmt::AssignOp {
            site,
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
    /// Inactive code is validated for syntax only. The item is parsed into a
    /// discarded declaration set, never published. Kept off the recursive
    /// block frame: the scratch declaration set is large.
    #[inline(never)]
    fn inactive_block_item(&mut self) -> Result<(), ParseError> {
        let mut scratch = Box::new(ParsedDeclarations {
            program: ProgramDeclarations::default(),
            items: HashMap::new(),
            empty_use_prefixes: Vec::new(),
            glob_imports: Vec::new(),
        });
        let module = self.module;
        let path =
            AstPath::new(self.names, PathAnchor::Crate, &[]).map_err(native_adapter::name_error)?;
        let start = self.cursor.index();
        let (visibility, explicit) = self.visibility(module)?;
        self.nonmodule_item(module, &path, &mut scratch, visibility, explicit, start)
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
            if self.is("const")
                && self.cursor.peek(1).is_some_and(|t| t.is_ident())
                && self.cursor.peek(2).is_some_and(|t| t.matches(":"))
            {
                self.local_const_item()?;
                continue;
            }
            if self.is("mod") || self.is("use") || self.is("fn") || self.is("pub") {
                if self.purpose == ParserPurpose::InactiveSyntax && !self.is("mod") {
                    self.inactive_block_item()?;
                    continue;
                }
                if self.is("fn") && self.cursor.peek(1).is_some_and(|t| t.is_ident()) {
                    self.local_fn_item()?;
                    continue;
                }
                if self.is("use") {
                    self.local_use_item()?;
                    continue;
                }
                return Err(self.unsupported("block-local items are unsupported"));
            }
            if self.try_for_statement(&mut statements)? {
                continue;
            }
            if self.eat("let") {
                statements.push(self.let_statement()?);
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
                if self.at_chain_let() {
                    let first = let_chains::ChainCond::Bool(condition);
                    statements.push(self.while_chain_statement(label, first)?);
                    continue;
                }
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
                    statements.push(self.plain_assignment_statement(expr)?);
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
                    let local = if path.anchor() == PathAnchor::SelfModule && path.is_empty() { receiver_authority::local(&path) } else { LocalName::Named(path.single().ok_or_else(|| self.unsupported("assignment requires a local-rooted place; destructuring and temporary assignment are unsupported"))?) };
                    fields.reverse(); return Ok(Place { local, fields });
                }
                Expr::Deref {value}=>{fields.push(FieldSelector::Deref);expr = *value;}
                Expr::Field { base, field } => { fields.push(field); expr = *base; }
                Expr::Index { base,index, .. } => {fields.push(FieldSelector::ConstantIndex(index));expr = *base;}
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
        if let Some(limit) = self.int_const_expr()? {
            return Ok(limit);
        }
        // The legacy println parser is a builtin consumer, not a spelling-based
        // bypass of the same checked binding policy used by every other macro.
        let println_syntax = self.is("println")
            && self.cursor.peek(1).is_some_and(|token| token.matches("!"))
            && self.builtin_macro_ahead("println")?;
        if !println_syntax && self.cursor.peek(1).is_some_and(|token| token.matches("!")) {
            return self.local_macro_expression();
        }
        self.ordinary_named_expr(println_syntax)
    }
    #[inline(never)]
    fn ordinary_named_expr(&mut self, println_syntax: bool) -> Result<Expr, ParseError> {
        if self.is("self") && !self.cursor.peek(1).is_some_and(|token| token.matches("::")) {
            return self.receiver_expression();
        }
        let mut generic_path = self.generic_path(false)?;
        let mut name = generic_path
            .checked_path(self.names)
            .map_err(native_adapter::name_error)?;
        if let Some(head) = name.segments().first() {
            let local = self.reference_lexical_binding(*head)?;
            // A local binding shadows items; otherwise an enclosing
            // block-local const shadows module items of the same name.
            if !local
                && let Some(single) = name.single()
                && name.anchor() == PathAnchor::Relative
                && !self.cursor.current().matches("(")
                && !self.cursor.current().matches("!")
                && !self.cursor.current().matches("{")
                && let Some(hidden) = self.local_const_path(single)?
            {
                name = hidden;
            } else if !local && let Some(hidden) = self.local_item_path(&name)? {
                name = hidden;
                self.rewrite_local_item_generic(&mut generic_path)?;
            }
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
            // Retain the original occurrence and its recorded lexical
            // association. Resolution distinguishes captured/local callees
            // from genuine module items before checking callable bounds.
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
        if min == 0 && !self.token().string && (self.is("..") || self.is("..=")) {
            return self.range_expression(None);
        }
        let site = self.token().span;
        let left = self.expr_prefix(statement)?;
        let left = self.expr_continuation(left, min, statement_blocklike, site)?;
        if min == 0 && !self.token().string && (self.is("..") || self.is("..=")) {
            return self.range_expression(Some(left));
        }
        if min == 0 && !statement {
            return self.assignment_expression(left);
        }
        Ok(left)
    }
    #[inline(never)]
    fn labeled_expression(&mut self, min: u8, statement: bool) -> Result<Expr, ParseError> {
        let site = self.token().span;
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
        self.expr_continuation(left, min, statement, site)
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
        let type_args = if self.is("::") && self.cursor.peek(1).is_some_and(|t| t.text == "<") {
            self.expect("::")?;
            self.type_arguments()?
        } else {
            Vec::new()
        };
        if !self.is("(") {
            if !type_args.is_empty() {
                return Err(
                    self.unsupported("associated constants cannot take method type arguments")
                );
            }
            let value = AssociatedConstUse {
                owner,
                trait_path: Some(*trait_path),
                name,
            };
            return Ok(Expr::AssociatedConst(
                self.names
                    .try_parser_box(value)
                    .map_err(native_adapter::name_error)?,
            ));
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
            type_args,
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
        if self.eat("break") {
            return self.break_prefix();
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
            self.reject_forbidden_unsafe()?;
            self.cursor.bump();
            return Ok(Expr::Unsafe(self.block()?));
        }
        self.ordinary_expr_prefix(statement)
    }
    #[inline(never)]
    fn ordinary_expr_prefix(&mut self, statement: bool) -> Result<Expr, ParseError> {
        let site = self.token().span;
        if self.is("|") || self.is("||") || self.is("move") {
            return self.stored_closure();
        }
        if self.is("while")
            && self
                .cursor
                .peek(1)
                .is_some_and(|token| token.matches("let"))
        {
            return Err(self.unsupported("while let expressions are unsupported"));
        }
        if self.is("const") && self.cursor.peek(1).is_some_and(|t| t.text == "{") {
            self.expect("const")?;
            return Ok(Expr::Block(self.block()?));
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
                site: Some(site),
            }
        } else if self.eat("!") {
            Expr::Unary {
                op: UnaryOp::Not,
                value: Box::new(self.expr(11)?),
                site: Some(site),
            }
        } else if self.token().string {
            let text = self.string_literal_value(self.token().text)?;
            self.cursor.bump();
            Expr::Str(text)
        } else if self.token().is_byte_string() {
            let value = super::literal_values::decode(self.token().text, self.names.budget())
                .map_err(|e| {
                    native_adapter::lex_error(super::lexer::literal_error(self.token().text, e))
                })?;
            self.names
                .retain_parser_storage(value.bytes().len())
                .map_err(native_adapter::name_error)?;
            let bytes = value.bytes().to_vec();
            self.cursor.bump();
            Expr::ByteStr(bytes)
        } else if self.token().is_byte() {
            Expr::Integer(self.byte_integer_literal()?)
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
        if self.is("|") || self.is("||") || self.is("move") {
            return self.immediate_closure();
        }
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
        if self.at_chain_let() {
            return self.if_chain(let_chains::ChainCond::Bool(*condition), statement);
        }
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
        if self.is("&&") {
            let first = let_chains::ChainCond::Let(
                self.names
                    .try_parser_box(pattern)
                    .map_err(native_adapter::name_error)?,
                self.names
                    .try_parser_box(scrutinee)
                    .map_err(native_adapter::name_error)?,
            );
            return self.if_chain(first, statement);
        }
        if self.is("||") {
            return Err(self.error("`||` operators are not supported in let chain conditions"));
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
            let start = self.nested_expr_no_range()?;
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
            let start = self.condition_with(1, ConditionPurpose::Ordinary)?;
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
                ForIterable::Value(start)
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
        _adapters: &mut Vec<ForAdapter>,
    ) -> Result<ForIterable, ParseError> {
        let value = self.condition_with(1, ConditionPurpose::Ordinary)?;
        if self.is("..") || self.is("..=") {
            return Err(self.unsupported("unsupported: parenthesized range start in for loop"));
        }
        // Methods remain ordinary original source occurrences, including their
        // actual names, generic arguments and closure-bearing argument lists.
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
        if self.is("&&") {
            let first = let_chains::ChainCond::Let(
                self.names
                    .try_parser_box(pattern)
                    .map_err(native_adapter::name_error)?,
                self.names
                    .try_parser_box(scrutinee)
                    .map_err(native_adapter::name_error)?,
            );
            return self.while_chain_statement(label, first);
        }
        if self.is("||") {
            return Err(self.error("`||` operators are not supported in let chain conditions"));
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
            let pattern = self.arm_pattern()?;
            if self.is("|") || self.is("@") {
                return Err(self.unsupported("or-patterns and @ patterns are unsupported here"));
            }
            let frontier = self.enter_lexical()?;
            self.bind_pattern_lexical(&pattern)?;
            let guard = if self.eat("if") {
                let condition = self.nested_expr()?;
                Some(
                    self.names
                        .try_parser_box(condition)
                        .map_err(native_adapter::name_error)?,
                )
            } else {
                None
            };
            self.expect("=>")?;
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
                guard,
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
        site: super::expansion_span::SpanId,
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
                        site: Some(site),
                    };
                } else {
                    if self.is("..") || self.is("..=") {
                        left = self.range_index(left, None)?;
                        validate_depth(DepthNode::Expr(&left)).map_err(|e| self.depth_error(&e))?;
                        continue;
                    }
                    let index = self.nested_expr_no_range()?;
                    if self.is("..") || self.is("..=") {
                        left = self.range_index(left, Some(index))?;
                        validate_depth(DepthNode::Expr(&left)).map_err(|e| self.depth_error(&e))?;
                        continue;
                    }
                    self.expect("]")?;
                    left = Expr::DynamicIndex {
                        base: Box::new(left),
                        index: Box::new(index),
                        site: Some(site),
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
            if self.is("(")
                && statement_blocklike
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
            if self.eat("(") {
                left = self.invocation_after_open(left)?;
                validate_depth(DepthNode::Expr(&left)).map_err(|e| self.depth_error(&e))?;
                continue;
            }
            if !self.token().string && self.eat("?") {
                left = Expr::Try(Box::new(left));
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
                // A let-chain operand: the chain parser takes over.
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
                site: Some(site),
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
    child_entries: Vec<ChildEntry>,
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
#[cfg(test)]
fn parse_syntax_tokens_configured(
    tokens: NativeParserInput,
    debug_assertions: bool,
) -> Result<ParsedProgram, ParseError> {
    parse_syntax_tokens_policy(
        tokens,
        debug_assertions,
        Arc::from([]),
        Arc::from([]),
        Arc::from([]),
        Arc::from([]),
        None,
    )
}
fn parse_syntax_tokens_policy(
    tokens: NativeParserInput,
    debug_assertions: bool,
    no_std_roots: Arc<[super::token_session::SourceFileHandle]>,
    embedded_issuers: Arc<[super::source::EmbeddedOriginalFile]>,
    forbid_unsafe_roots: Arc<[super::token_session::SourceFileHandle]>,
    child_sites: Arc<[usize]>,
    mount: Option<ChildMount<'_>>,
) -> Result<ParsedProgram, ParseError> {
    let unit = parse_syntax_unit(
        tokens,
        ParserPurpose::Active,
        debug_assertions,
        no_std_roots,
        embedded_issuers.clone(),
        forbid_unsafe_roots,
        child_sites,
        mount,
    )?;
    let mut program =
        unit.parsed
            .program
            .publish(unit.names, unit.modules, unit.symbols, unit.scopes)?;
    program.embedded_issuers = embedded_issuers;
    program.generated_owners =
        generated_owners::capture(&program).map_err(|message| ParseError {
            kind: super::source::SourceErrorKind::Contract,
            has_location: false,
            bytes: 0..0,
            primary: None,
            line: 1,
            column: 1,
            message,
            _diagnostic_lease: None,
        })?;
    Ok(ParsedProgram {
        program,
        items: unit.parsed.items,
        empty_use_prefixes: unit.parsed.empty_use_prefixes,
        glob_imports: unit.parsed.glob_imports,
        child_entries: unit.child_entries,
    })
}
fn parse_syntax_unit(
    tokens: NativeParserInput,
    purpose: ParserPurpose,
    debug_assertions: bool,
    no_std_roots: Arc<[super::token_session::SourceFileHandle]>,
    embedded_issuers: Arc<[super::source::EmbeddedOriginalFile]>,
    forbid_unsafe_roots: Arc<[super::token_session::SourceFileHandle]>,
    child_sites: Arc<[usize]>,
    mount: Option<ChildMount<'_>>,
) -> Result<ParsedSyntaxUnit, ParseError> {
    let edition = tokens.edition();
    let (tokens, session, cap) = tokens.into_parts();
    let mut names =
        NameStoreBuilder::new(session, cap, edition).map_err(native_adapter::name_error)?;
    if let Some(mount) = &mount {
        // The parent and every ancestor it links (the crate root included),
        // so inherited handles from any ancestor stay admissible.
        let mut stores = mount.parent.names.session().linked_stores();
        stores.push(mount.parent.names.clone());
        let root = crate_root_store(&stores)?;
        names
            .link_crate_files(stores)
            .and_then(|()| names.join_crate(&root))
            .map_err(native_adapter::name_error)?;
    }
    let symbols = ProgramSymbols::new(&mut names).map_err(native_adapter::name_error)?;
    let mut parser = match purpose {
        ParserPurpose::Active => Parser::new(&tokens, &mut names)?,
        ParserPurpose::InactiveSyntax => Parser::with_purpose(&tokens, &mut names, purpose)?,
    };
    parser.embedded_issuers = embedded_issuers;
    parser.no_std_roots = no_std_roots;
    parser.forbid_unsafe_roots = forbid_unsafe_roots;
    parser.child_sites = child_sites;
    parser.debug_assertions = debug_assertions;
    let mut parsed = ParsedDeclarations {
        program: ProgramDeclarations::default(),
        items: HashMap::new(),
        empty_use_prefixes: Vec::new(),
        glob_imports: Vec::new(),
    };
    let (root, root_path) = match &mount {
        Some(mount) => {
            parser.mount_child(mount)?;
            (mount.entry.module, mount.entry.path.clone())
        }
        None => (
            parser.modules.source_root().map_err(module_parse_error)?,
            AstPath::new(parser.names, PathAnchor::Crate, &[])
                .map_err(native_adapter::name_error)?,
        ),
    };
    parser.items(root, &root_path, &mut parsed)?;
    parsed.program.no_std_roots = parser.no_std_roots.clone();
    parsed.program.c_abi = std::mem::take(&mut parser.c_abi);
    if !parser.is("") {
        return Err(parser.error("unexpected module closing brace"));
    }
    parsed
        .program
        .functions
        .append(&mut parser.lexical.closure_functions);
    let sources = std::mem::take(&mut parser.lexical.closure_sources);
    parsed
        .program
        .closure_sources
        .try_reserve(sources.len())
        .map_err(|_| parser.limit("closure source index allocation"))?;
    for source in sources {
        parser
            .names
            .budget()
            .charge(super::expansion_budget::Counter::Work, 1)
            .map_err(|error| native_adapter::transport_error(error.into()))?;
        if parsed
            .program
            .closure_sources
            .insert(source.name.key(Namespace::Type), source)
            .is_some()
        {
            return Err(parser.error("duplicate anonymous source declaration"));
        }
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
    if parser.child_entries.len() != parser.child_sites.len() {
        return Err(parser.error("external module site was not reached by the parser"));
    }
    let child_entries = std::mem::take(&mut parser.child_entries);
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
        child_entries,
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
        super::owner_path::standalone_issued(
            &budget,
            super::owner_path::OwnerDomain::SourceSnapshot,
        )
        .map_err(native_adapter::transport_error)?,
    )
    .map_err(native_adapter::source_error)?;
    let owner =
        super::owner_path::standalone_issued(&budget, super::owner_path::OwnerDomain::TokenSession)
            .map_err(native_adapter::transport_error)?;
    let (mut session, cap) = super::token_session::TokenSession::with_owner(
        projection.snapshot().clone(),
        budget,
        owner,
    )
    .map_err(native_adapter::transport_error)?;
    let lexed = super::lexer::lex_prepared(&projection, super::token::Edition::E2024, &mut session)
        .map_err(native_adapter::lex_error)?;
    let input = NativeParserInput::new_inactive(lexed, session, cap, &projection)?;
    Ok(InactiveSyntaxCertificate {
        _unit: parse_syntax_unit(
            input,
            ParserPurpose::InactiveSyntax,
            super::source::CfgSet::DEFAULT_DEBUG_ASSERTIONS,
            Arc::from([]),
            Arc::from([]),
            Arc::from([]),
            Arc::from([]),
            None,
        )?,
    })
}

#[cfg(test)]
mod inactive_item_syntax_tests {
    use super::*;

    /// A forced canonical-key collision (two declarations minted with the
    /// same module key, kind and ordinal) is rejected, never merged.
    #[test]
    fn canonical_decl_id_collision_is_rejected_not_merged() {
        let program = resolve(&parse_unresolved("struct S;fn main(){}").unwrap()).unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let registry = &plan.registry;
        let key = name_scope::path_key(
            program.names.as_ref(),
            &program.structs[0].name,
            Namespace::Type,
        )
        .unwrap();
        let existing = registry.nominals[&key];
        assert!(existing.is_source());
        let module_key =
            canonical_module_key(&program.modules, &program.names, program.structs[0].module)
                .unwrap();
        let forged = ir::DeclId {
            snapshot: existing.snapshot,
            index: ir::DeclId::source_index(module_key, ir::DeclKind::Struct, 0).unwrap(),
        };
        assert_eq!(forged, existing, "canonical identity must be reproducible");
        // A fresh registry holding the original row, then a second row minted
        // under the same canonical key with different metadata.
        let original = registry.declaration(existing).unwrap().clone();
        let mut colliding = original.clone();
        colliding.kind = ir::DeclKind::Enum;
        let mut fresh = DeclRegistry {
            declarations: Vec::new(),
            slots: HashMap::new(),
            ..registry.empty_like()
        };
        fresh.push_declaration(original).unwrap();
        let error = fresh.push_declaration(colliding).unwrap_err();
        assert!(error.contains("duplicate declaration identity"), "{error}");
        assert_eq!(fresh.declarations.len(), 1);
        assert_eq!(
            fresh.declaration(existing).unwrap().kind,
            ir::DeclKind::Struct
        );
    }

    fn client_main(program: &Program) -> &Function {
        let root = program.modules.source_root().unwrap();
        let mut matches = program.functions.iter().filter(|function| {
            function.module == root
                && function.name.anchor() == PathAnchor::Crate
                && function.name.len() == 1
                && program.names.spelling(function.name.segments()[0]).unwrap() == "main"
        });
        let main = matches.next().expect("client root main");
        assert!(matches.next().is_none(), "duplicate client root main");
        main
    }

    fn exact_call_inventory(inventory: &SourceInventory, references: &[&Expr], calls: &[&Expr]) {
        let mut expected = std::collections::HashSet::new();
        for expression in references {
            assert!(deref_coercion::candidate(expression));
            let mut source_node = None;
            for step in 1..=deref_coercion::MAX_DEREF_STEPS {
                let pointer = deref_coercion::site(*expression as *const Expr as usize, step);
                assert!(expected.insert(pointer));
                let site = &inventory.calls[*inventory.call_pointers.get(&pointer).unwrap()];
                assert!(site.operator_candidate);
                assert!(site.ordinary.is_none());
                assert_eq!(site.node.owner, inventory.owner);
                if let Some(node) = &source_node {
                    assert_eq!(&site.node, node);
                } else {
                    source_node = Some(site.node.clone());
                }
            }
        }
        for expression in calls {
            let Expr::Call { name, .. } = expression else {
                panic!("expected written call")
            };
            let pointer = *expression as *const Expr as usize;
            assert!(expected.insert(pointer));
            let site = &inventory.calls[*inventory.call_pointers.get(&pointer).unwrap()];
            assert!(!site.operator_candidate);
            assert_eq!(
                site.ordinary.as_ref().unwrap().key(Namespace::Value),
                name.key(Namespace::Value)
            );
            assert_eq!(site.node.owner, inventory.owner);
        }
        assert_eq!(inventory.calls.len(), expected.len());
        assert_eq!(
            inventory
                .call_pointers
                .keys()
                .copied()
                .collect::<std::collections::HashSet<_>>(),
            expected
        );
    }

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
        let applications_before = budget.usage_snapshot().applications;
        let (_, ty) = plan.nominal_schema(declaration, &budget).unwrap();
        assert!(matches!(ty, ir::Ty::Struct(_)), "{ty:?}");
        assert_eq!(nominal_parts(&ty).unwrap().0, declaration);
        // The canonical application was sealed during plan construction.
        // Repeated schema reads preserve it without interning a new one.
        assert_eq!(plan.nominal_schema(declaration, &budget).unwrap().1, ty);
        assert_eq!(budget.usage_snapshot().applications, applications_before);
        assert_eq!(budget.usage_snapshot().obligations, 0);
        let error = parse("struct S;type A=S;fn main(){let x=A;}").unwrap_err();
        assert!(!error.contains("unsupported"), "{error}");
        assert!(
            error.contains("type alias is not a value constructor"),
            "{error}"
        );
        let record = parse_unresolved("struct S{}fn main(){}").unwrap();
        assert_eq!(record.program.structs[0].shape, StructShape::Record);
        assert!(record.program.structs[0].fields.is_empty());
        assert!(
            parse_unresolved("struct S();fn main(){}")
                .unwrap_err()
                .contains("empty tuple structs are unsupported")
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
    fn if_let_chains_parse_and_or_boundary_is_typed_at_written_tokens() {
        for source in [
            "fn main(){if let true=true && true {}}",
            "fn main(){if true && let true=true {}}",
        ] {
            parse_syntax_unprepared(source).unwrap();
        }
        {
            let (source, token) = ("fn main(){if let true=true || false {}}", "||");
            let error = parse_syntax_unprepared(source).unwrap_err();
            assert_eq!(error.kind, super::super::source::SourceErrorKind::Syntax);
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
    fn while_let_chains_parse_and_expression_boundaries_remain_explicit() {
        for source in [
            "fn main(){while let true=true && true {}}",
            "fn main(){while true && let true=true {}}",
        ] {
            parse_syntax_unprepared(source).unwrap();
        }
        for (source, token) in [
            ("fn main(){while let true=true || false {}}", "||"),
            ("fn main(){let value=while let true=false {};}", "while"),
            (
                "fn main(){let value='outer: while let true=false {};}",
                "while",
            ),
        ] {
            let error = parse_syntax_unprepared(source).unwrap_err();
            let expected = if token == "||" {
                super::super::source::SourceErrorKind::Syntax
            } else {
                super::super::source::SourceErrorKind::Unsupported
            };
            assert_eq!(error.kind, expected);
            let at = source.find(token).unwrap();
            assert_eq!(error.bytes, at..at + token.len());
        }
        parse_unresolved("fn main(){while let true=(true && false) {break;}}").unwrap();
    }

    #[test]
    fn while_let_scope_inventory_keeps_outer_scrutinee_and_one_real_callsite() {
        let program = parse("fn probe()->bool{false}fn main(){macro_rules! marker{()=>{42}}let x=40;while let x=x{let y=x;break;}let z=x;while let true=probe(){break;}}").unwrap();
        let function = client_main(&program);
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
        let Stmt::Let {
            value: body_value, ..
        } = &body.statements[0]
        else {
            panic!("body binding")
        };
        assert_eq!(reference(body_value), Some(inner_id));
        let Stmt::Let {
            value: after_value, ..
        } = &function.body.statements[2]
        else {
            panic!("after binding")
        };
        assert_eq!(reference(after_value), Some(outer_id));
        let budget = ir::PlanBudget::default();
        let identity = sr::OriginalBodyId::new(&budget).unwrap();
        let inventory = SourceInventory::build(&program, function, identity, &budget).unwrap();
        assert_eq!(inventory.bindings.len(), 4);
        let Stmt::WhileLet {
            scrutinee: probe, ..
        } = &function.body.statements[3]
        else {
            panic!("probe")
        };
        exact_call_inventory(&inventory, &[scrutinee, body_value, after_value], &[probe]);
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
        let parsed = parse_syntax_unprepared(source).unwrap();
        let Stmt::LetElse { pattern, .. } = &parsed.program.functions[0].body.statements[0] else {
            panic!("let-else")
        };
        assert!(matches!(pattern.as_ref(), Pattern::Alternatives(rows) if rows.len() == 2));
        let stock =
            super::parser_tests::stock_compile(source, std::path::Path::new("unused"), true);
        assert!(
            stock.status.success(),
            "{}",
            String::from_utf8_lossy(&stock.stderr)
        );
        // Preserve the syntax without claiming unsupported semantic lowering.
        for workers in [1, 8] {
            for check in [true, false] {
                let failure = super::super::compile(source.into(), workers, check)
                    .err()
                    .expect("unsupported let-else alternatives accepted");
                assert!(
                    failure
                        .to_string()
                        .contains("or-patterns are only admitted at match arm roots"),
                    "{failure}"
                );
            }
        }
        let invalid =
            "enum E{A(i32),B(i32)}fn main(){let (E::A(x)|E::B(_))=E::A(42)else{return;};}";
        let stock =
            super::parser_tests::stock_compile(invalid, std::path::Path::new("unused"), true);
        assert!(!stock.status.success());
        assert!(String::from_utf8_lossy(&stock.stderr).contains("E0408"));
        for workers in [1, 8] {
            assert!(super::super::compile(invalid.into(), workers, true).is_err());
        }
    }

    #[test]
    fn let_else_binding_escapes_only_after_old_frontier_else_and_initializer() {
        let program = parse("fn probe()->bool{false}fn main(){macro_rules! marker{()=>{42}}let x=40;let x=x else{let y=x;return;};let z=x;let true=probe()else{return;};}").unwrap();
        let function = client_main(&program);
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
        let Stmt::Let {
            value: else_value, ..
        } = &else_block.statements[0]
        else {
            panic!("else binding")
        };
        assert_eq!(reference(else_value), Some(outer_id));
        let Stmt::Let {
            value: after_value, ..
        } = &function.body.statements[2]
        else {
            panic!("after binding")
        };
        assert_eq!(reference(after_value), Some(inner_id));
        let budget = ir::PlanBudget::default();
        let identity = sr::OriginalBodyId::new(&budget).unwrap();
        let inventory = SourceInventory::build(&program, function, identity, &budget).unwrap();
        assert_eq!(inventory.bindings.len(), 4);
        let Stmt::LetElse { value: probe, .. } = &function.body.statements[3] else {
            panic!("probe")
        };
        exact_call_inventory(&inventory, &[value, else_value, after_value], &[probe]);
        assert_ne!(inventory.bindings[0].node, inventory.bindings[2].node);
    }

    #[test]
    fn if_let_scrutinee_and_else_keep_outer_binding_and_inventory_identity() {
        let program = parse("fn main(){macro_rules! marker{()=>{42}}let x=40;if let x=x {let y=x;} else {let z=x;}let w=x;}").unwrap();
        let function = client_main(&program);
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
        let Stmt::Let {
            value: then_value, ..
        } = &then_block.statements[0]
        else {
            panic!("then binding")
        };
        assert_eq!(reference(then_value), Some(inner_id));
        let Expr::Block(block) = else_branch.as_deref().unwrap() else {
            panic!("else block")
        };
        let Stmt::Let {
            value: else_value, ..
        } = &block.statements[0]
        else {
            panic!("else binding")
        };
        assert_eq!(reference(else_value), Some(outer_id));
        let Stmt::Let {
            value: after_value, ..
        } = &function.body.statements[2]
        else {
            panic!("after binding")
        };
        assert_eq!(reference(after_value), Some(outer_id));
        let budget = ir::PlanBudget::default();
        let identity = sr::OriginalBodyId::new(&budget).unwrap();
        let inventory = SourceInventory::build(&program, function, identity, &budget).unwrap();
        assert_eq!(inventory.bindings.len(), 5);
        exact_call_inventory(
            &inventory,
            &[scrutinee, then_value, else_value, after_value],
            &[],
        );
        assert_ne!(inventory.bindings[0].node, inventory.bindings[1].node);
    }

    #[test]
    fn item_publication_denies_before_container_allocation_and_releases() {
        use super::super::expansion_budget::{BudgetResource, Storage};
        let (tokens, session, session_cap) = lex("fn main(){}").unwrap().into_parts();
        let mut names =
            NameStoreBuilder::new(session, session_cap, super::super::token::Edition::E2024)
                .unwrap();
        let budget = names.budget().clone();
        let mut parser = Parser::new(&tokens, &mut names).unwrap();
        let mut parsed = ParsedDeclarations {
            program: ProgramDeclarations::default(),
            items: HashMap::new(),
            empty_use_prefixes: Vec::new(),
            glob_imports: Vec::new(),
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
        let program = unit
            .parsed
            .program
            .publish(unit.names, unit.modules, unit.symbols, unit.scopes)
            .unwrap();
        assert!(
            PlanCtx::from_program(&program, &ir::PlanBudget::default())
                .unwrap_err()
                .contains("unexpanded macro")
        );
    }

    #[test]
    fn inactive_array_lengths_skip_the_semantic_length_limit() {
        for source in [
            "fn unused(){ let a = [0u8; 1048576]; }",
            "fn unused(){ let a = [0xffu8; 1024 * 1024 - 1024]; }",
        ] {
            validate_inactive_syntax(source).unwrap();
            let active = format!("{source} fn main(){{}}");
            assert!(parse(&active).is_err(), "{active}");
        }
        assert!(validate_inactive_syntax("fn unused(){ let a = [0u8; ]; }").is_err());
    }

    #[test]
    fn inactive_block_local_items_are_syntax_checked_only() {
        validate_inactive_syntax(
            "fn unused(){ use std::io::BufReader; fn inner(x: &[u8]) -> usize { x.len() } inner(&[]); }",
        )
        .unwrap();
        for bad in ["fn unused(){ use ; }", "fn unused(){ fn inner( { } }"] {
            assert!(validate_inactive_syntax(bad).is_err(), "{bad}");
        }
        assert!(parse("fn main(){ use std::io::BufReader; }").is_err());
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
/// Direct, non-graph entry: mints a fresh standalone owner root per call.
pub fn parse_prepared(
    source: &super::source::PreparedSource,
) -> Result<ParsedProgram, super::source::SourceError> {
    let budget = super::expansion_budget::ExpansionBudget::default();
    let snapshot = super::owner_path::standalone_issued(
        &budget,
        super::owner_path::OwnerDomain::SourceSnapshot,
    )
    .map_err(super::source::SourceError::from_token)?;
    let owner =
        super::owner_path::standalone_issued(&budget, super::owner_path::OwnerDomain::TokenSession)
            .map_err(super::source::SourceError::from_token)?;
    parse_prepared_owned(source, budget, snapshot, owner)
}

/// Graph entry: the session owner is a tag issued by the calling producer.
/// Never mints a root, so a graph PARSE cannot fall back to standalone.
pub fn parse_prepared_owned(
    source: &super::source::PreparedSource,
    budget: super::expansion_budget::ExpansionBudget,
    snapshot: super::owner_path::Issued,
    owner: super::owner_path::Issued,
) -> Result<ParsedProgram, super::source::SourceError> {
    let projection = super::source::PreparedLexInput::prepared(source, &budget, snapshot)?;
    parse_projection(source, projection, 0, Arc::from([]), None, budget, owner)
}

/// Parse one file's shallow view in its own session, rendered at its crate
/// assembly base (PARSE-SPLIT.md step B). Not yet scheduled as PARSE_FILE.
pub fn parse_file_view_owned(
    view: &super::source::FileView,
    budget: super::expansion_budget::ExpansionBudget,
    snapshot: super::owner_path::Issued,
    owner: super::owner_path::Issued,
) -> Result<ParsedProgram, super::source::SourceError> {
    let projection = super::source::PreparedLexInput::file_view(view, &budget, snapshot)?;
    let sites = Arc::from(view.child_sites.as_slice());
    parse_projection(
        &view.prepared,
        projection,
        view.base,
        sites,
        None,
        budget,
        owner,
    )
}

/// Parse a child file's view mounted at its parent's entry (PARSE-SPLIT.md
/// step C2): the child links the parent's frozen name, module and scope
/// stores, joins its crate, starts at the parent's module for `mod child;`
/// and sees exactly the textual macros in effect at that declaration.
pub fn parse_child_view_owned(
    view: &super::source::FileView,
    parent: &ParsedProgram,
    index: usize,
    budget: super::expansion_budget::ExpansionBudget,
    snapshot: super::owner_path::Issued,
    owner: super::owner_path::Issued,
) -> Result<ParsedProgram, super::source::SourceError> {
    let entry = parent.child_entries.get(index).ok_or_else(|| {
        super::source::SourceError::from_token(super::token_session::TokenError::InvalidInput(
            "no such child entry",
        ))
    })?;
    let mount = ChildMount {
        parent: &parent.program,
        entry,
    };
    let projection = super::source::PreparedLexInput::child_file_view(
        view,
        parent.program.names.session().sources(),
        &budget,
        snapshot,
    )?;
    debug_assert!(parent.child_entries.len() > index);
    let sites = Arc::from(view.child_sites.as_slice());
    parse_projection(
        &view.prepared,
        projection,
        view.base,
        sites,
        Some(mount),
        budget,
        owner,
    )
}
/// The store among a crate's linked file stores that is its own crate.
fn crate_root_store(stores: &[Arc<NameStore>]) -> Result<Arc<NameStore>, ParseError> {
    stores
        .iter()
        .find(|s| s.session().crate_key() == s.session().id())
        .cloned()
        .ok_or_else(|| {
            native_adapter::transport_error(super::token_session::TokenError::InvalidInput(
                "no crate root among linked files",
            ))
        })
}
/// A child parse's mount point in its parent's frozen parse.
pub(super) struct ChildMount<'a> {
    parent: &'a Program,
    entry: &'a ChildEntry,
}

/// Merge a crate's per-file parses, in depth-first assembly order with the
/// crate root first, into one syntax result (PARSE-SPLIT.md step E1). The
/// merged program reads every file through fresh crate-level name, module
/// and scope stores that link all file stores and join the root's crate, so
/// structural keys from every file compare equal. Declarations append in
/// file order; nothing is re-parsed.
pub fn merge_crate_files(
    files: &[ParsedProgram],
    budget: super::expansion_budget::ExpansionBudget,
    owner: super::owner_path::Issued,
) -> Result<ParsedProgram, String> {
    use super::semantic_scope::ScopeArenaBuilder;
    let (root, rest) = files.split_first().ok_or("no crate files to merge")?;
    let siblings = rest
        .iter()
        .map(|f| f.program.names.session().sources().clone())
        .collect();
    let sources = root
        .program
        .names
        .session()
        .sources()
        .linked_with(siblings, &budget)
        .map_err(|e| e.to_string())?;
    let (session, cap) = super::token_session::TokenSession::with_owner(sources, budget, owner)
        .map_err(|e| e.to_string())?;
    let mut names = NameStoreBuilder::new(session, cap, root.program.names.edition())
        .map_err(|e| e.to_string())?;
    names
        .link_crate_files(files.iter().map(|f| f.program.names.clone()).collect())
        .and_then(|()| names.join_crate(&root.program.names))
        .map_err(|e| e.to_string())?;
    let mut modules = super::module_identity::ModuleArenaBuilder::for_builder(&mut names)
        .map_err(|e| e.to_string())?;
    modules
        .link_crate_files(files.iter().map(|f| f.program.modules.clone()).collect())
        .and_then(|()| modules.join_crate(&root.program.modules))
        .map_err(|e| e.to_string())?;
    let scopes = match files.iter().any(|f| f.program.scopes.is_some()) {
        true => {
            let mut scopes =
                ScopeArenaBuilder::for_builder(&mut names).map_err(|e| e.to_string())?;
            scopes
                .link_crate_files(
                    files
                        .iter()
                        .filter_map(|f| f.program.scopes.clone())
                        .collect(),
                )
                .map_err(|e| e.to_string())?;
            Some(scopes.freeze().map_err(|e| e.to_string())?)
        }
        false => None,
    };
    let symbols = ProgramSymbols::new(&mut names).map_err(|e| e.to_string())?;
    let modules = modules.freeze().map_err(|e| e.to_string())?;
    let names = names.freeze().map_err(|e| e.to_string())?;
    let mut program = root.program.clone();
    let mut items = root.items.clone();
    let mut empty_use_prefixes = root.empty_use_prefixes.clone();
    let mut glob_imports = root.glob_imports.clone();
    let mut closure_sources = (*root.program.closure_sources).clone();
    let mut c_abi = (*root.program.c_abi).clone();
    let mut generated_metadata = root.program.generated_metadata.to_vec();
    let mut generated_owners = root.program.generated_owners.to_vec();
    for file in rest {
        let p = &file.program;
        if p.prelude_module.is_some()
            || p.prelude_scope.is_some()
            || !p.extern_mounts.is_empty()
            || !p.embedded_issuers.is_empty()
        {
            return Err("unsupported: crate-root declarations in a child file".into());
        }
        program.functions.extend(p.functions.iter().cloned());
        program.constants.extend(p.constants.iter().cloned());
        program.structs.extend(p.structs.iter().cloned());
        program.enums.extend(p.enums.iter().cloned());
        program.impls.extend(p.impls.iter().cloned());
        program.traits.extend(p.traits.iter().cloned());
        program
            .trait_imports
            .extend(p.trait_imports.iter().cloned());
        for (key, list) in &file.items {
            items
                .entry(key.clone())
                .or_default()
                .extend(list.iter().cloned());
        }
        empty_use_prefixes.extend(file.empty_use_prefixes.iter().cloned());
        glob_imports.extend(file.glob_imports.iter().cloned());
        for (key, source) in p.closure_sources.iter() {
            if closure_sources
                .insert(key.clone(), source.clone())
                .is_some()
            {
                return Err("duplicate anonymous source declaration across files".into());
            }
        }
        c_abi.functions.extend(
            p.c_abi
                .functions
                .iter()
                .map(|(k, v)| (k.clone(), v.clone())),
        );
        c_abi.repr_c.extend(p.c_abi.repr_c.iter().cloned());
        generated_metadata.extend(p.generated_metadata.iter().cloned());
        generated_owners.extend(p.generated_owners.iter().cloned());
    }
    program.struct_sources =
        name_validation::capture_struct_sources(&names, &program.structs).map_err(|e| e.message)?;
    program.names = names;
    program.modules = modules;
    program.symbols = symbols;
    program.scopes = scopes;
    program.closure_sources = Arc::new(closure_sources);
    program.c_abi = Arc::new(c_abi);
    program.generated_metadata = generated_metadata.into();
    program.generated_owners = generated_owners.into();
    Ok(ParsedProgram {
        program,
        items,
        empty_use_prefixes,
        glob_imports,
        child_entries: Vec::new(),
    })
}

fn parse_projection(
    source: &super::source::PreparedSource,
    projection: super::source::PreparedLexInput<'_>,
    base: u32,
    child_sites: Arc<[usize]>,
    mount: Option<ChildMount<'_>>,
    budget: super::expansion_budget::ExpansionBudget,
    owner: super::owner_path::Issued,
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
        // Spans render at the crate assembly FileId; the view's own map
        // holds that file at local index `id - base`.
        let rendered = diagnostic.primary;
        if let Some(span) = diagnostic.primary.as_mut() {
            span.file.0 = span.file.0.wrapping_sub(base);
        }
        let mut diagnostic = diagnostic.with_sources(&source.sources);
        diagnostic.primary = rendered;
        diagnostic
    };
    let mut no_std_roots = Vec::new();
    let mut forbid_unsafe_roots = Vec::new();
    for policy in &source.crate_policies {
        if let Some(root) = policy.forbid_unsafe_root(source)? {
            forbid_unsafe_roots.push(
                projection
                    .snapshot()
                    .file(root)
                    .map_err(super::source::SourceError::from_token)?,
            );
        }
        if let Some(root) = policy.no_std_root(source)? {
            no_std_roots.push(
                projection
                    .snapshot()
                    .file(root)
                    .map_err(super::source::SourceError::from_token)?,
            );
        }
    }
    let issuers = source.embedded_issuers(projection.snapshot(), &budget)?;
    let inherits = mount.as_ref().is_some_and(|m| m.entry.macros.is_some());
    let tokens = native_input_with(&projection, budget, owner, source.edition(), inherits)
        .map_err(mapped)?;
    parse_syntax_tokens_policy(
        tokens,
        source.debug_assertions(),
        no_std_roots.into(),
        issuers.into(),
        forbid_unsafe_roots.into(),
        child_sites,
        mount,
    )
    .map_err(mapped)
}

/// Parse syntax with the same preparation policy as file inputs, retaining the
/// legacy line/column String diagnostic format for existing direct callers.
pub fn parse_unresolved(source: &str) -> Result<ParsedProgram, String> {
    let prepared =
        super::source::prepare_text("<text>", source, &super::source::SourceOptions::default())
            .map_err(|e| e.to_string())?;
    parse_prepared(&prepared).map_err(|e| e.to_string())
}

/// Convenience frontend for callers without separate parse/signatures phases.
/// Performs name resolution, but not function body type checks.
pub fn parse(source: &str) -> Result<Program, String> {
    resolve(&parse_unresolved(source)?)
}

mod provider_source_context;
mod provider_source_domain;
mod root_resolution;
pub mod sema_lower;
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
        let calls = inventory
            .calls
            .iter()
            .filter(|call| !call.operator_candidate)
            .collect::<Vec<_>>();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].ordinary.is_none());
    }

    #[test]
    fn source_trait_argument_placement_and_invalid_generic_boundaries_stay_closed() {
        assert!(parse_syntax_unprepared("trait T{fn value<U>(x:U);}fn main(){}").is_ok());
        for source in [
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
        let (tokens, session, session_cap) = lex("<T:Bound<U>,U>").unwrap().into_parts();
        let mut names =
            NameStoreBuilder::new(session, session_cap, super::super::token::Edition::E2024)
                .unwrap();
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
            DepthNode::Expr(Expr::AssociatedConst(value) | Expr::AssociatedFn(value)) => {
                if let Some(names) = names {
                    names
                        .check(value.name)
                        .map_err(native_adapter::name_diagnostic)?;
                }
                push(DepthNode::Type(&value.owner))?;
                if let Some(TraitPath::Source(path)) = &value.trait_path {
                    for segment in &path.segments {
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
            DepthNode::Type(Type::Opaque(opaque)) => {
                if let Some(names) = names {
                    opaque.validate_names(names)?;
                }
                for ty in opaque.types() {
                    push(DepthNode::Type(ty))?;
                }
            }
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
                    Pattern::Tuple(fields)
                    | Pattern::Array(fields)
                    | Pattern::Alternatives(fields) => {
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
                    Pattern::Ref { pattern, .. } | Pattern::At { pattern, .. } => {
                        push(DepthNode::Pattern(pattern))?
                    }
                    _ => {}
                }
            }
            DepthNode::Expr(expr) => match expr {
                Expr::Invoke { receiver, args } => {
                    push(DepthNode::Expr(receiver))?;
                    for arg in args {
                        push(DepthNode::Expr(arg))?;
                    }
                }
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
                    type_args,
                    args,
                    ..
                } => {
                    push(DepthNode::Type(owner))?;
                    for actual in type_args {
                        push(DepthNode::Type(actual))?;
                    }
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
                Expr::Deref { value }
                | Expr::BorrowValue { value, .. }
                | Expr::SliceDataPointer(value) => push(DepthNode::Expr(value))?,
                Expr::Repeat { value, .. } => push(DepthNode::Expr(value))?,
                Expr::DynamicIndex { base, index, .. } => {
                    push(DepthNode::Expr(base))?;
                    push(DepthNode::Expr(index))?;
                }
                Expr::RangeIndex {
                    base, start, end, ..
                } => {
                    push(DepthNode::Expr(base))?;
                    for bound in [start, end].into_iter().flatten() {
                        push(DepthNode::Expr(bound))?;
                    }
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
                        if let Some(guard) = &arm.guard {
                            push(DepthNode::Expr(guard))?;
                        }
                    }
                }
                Expr::TypeAlignment(ty) => push(DepthNode::Type(ty))?,
                Expr::NativeSyscall(fields)
                | Expr::RawSlice {
                    arguments: fields, ..
                }
                | Expr::Tuple(fields)
                | Expr::TupleStruct { fields, .. } => {
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
                Expr::FormatInto(source) => {
                    for arg in source.arguments.iter().chain(source.rest.iter().flatten()) {
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
                Expr::Try(value) => push(DepthNode::Expr(value))?,
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
                        Stmt::AssignNestedIndex { target, value, .. } => {
                            push(DepthNode::Expr(value))?;
                            push(DepthNode::Expr(target))?;
                        }
                        Stmt::AssignDeref {
                            carrier,
                            index,
                            value,
                            ..
                        } => {
                            if let Some(index) = index {
                                push(DepthNode::Expr(index))?;
                            }
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
        budget.charge_work(a.region_args.len().saturating_add(b.region_args.len()))?;
        equal &= a.region_args.len() == b.region_args.len();
        for (a, b) in a.region_args.iter().zip(&b.region_args) {
            equal &= match (a, b) {
                (RegionName::Named(a), RegionName::Named(b)) => exact_source_ident(names, *a, *b)?,
                (RegionName::Static, RegionName::Static)
                | (RegionName::Infer, RegionName::Infer) => true,
                _ => false,
            };
        }
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
        (Type::Opaque(a), Type::Opaque(b)) => a.exact(b, names, budget, depth + 1)?,
        (Type::Dyn(a), Type::Dyn(b)) => match (a.as_ref(), b.as_ref()) {
            (TraitPath::Builtin(a), TraitPath::Builtin(b)) => a == b,
            (TraitPath::Source(a), TraitPath::Source(b)) => {
                exact_source_generic_path(names, a, b, budget, depth + 1)?
            }
            _ => false,
        },
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
        found |= retained_type_projection(names, &bound.subject, projection, budget)?;
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
use std::sync::Arc;

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
    pub(super) embedded_function: Option<Arc<function_source::BoundFunctionSource>>,
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
    /// Storage slot of each declaration identity. Identities are not storage
    /// positions, so they may be canonical per-module keys.
    slots: HashMap<ir::DeclId, usize>,
    functions: HashMap<PathKey, ir::DeclId>,
    constants: HashMap<PathKey, ir::DeclId>,
    const_ids: Vec<ir::DeclId>,
    nominals: HashMap<PathKey, ir::DeclId>,
    function_ids: Vec<ir::DeclId>,
    callable_sources: Vec<CallableSource>,
    impl_ids: Vec<ir::DeclId>,
    /// Inherent associated callables per nominal (item-local overlap peers).
    inherent_peers: HashMap<ir::DeclId, Vec<ir::DeclId>>,
    associated_ids: Vec<Vec<ir::DeclId>>,
    traits: HashMap<TraitLookupKey, ir::DeclId>,
    trait_ids: Vec<ir::DeclId>,
    /// Canonical embedded iter module declarations, indexed during registration.
    native_iterator_traits: [Option<ir::DeclId>; 2],
    trait_items: Vec<Vec<ir::DeclId>>,
    trait_default_callables: HashMap<(usize, usize), ir::DeclId>,
    associated_constants: associated_constants::Registry,
    closure_recipes: HashMap<ir::DeclId, ir::ClosureRecipeId>,
    trait_associated_types: Vec<Vec<ir::DeclId>>,
    associated_type_bounds: HashMap<ir::DeclId, FrozenAssociatedBounds>,
    trait_imports: Vec<(ModuleId, TraitPath)>,
    /// First position of each callable in `function_ids`, built once when the
    /// registry is sealed. Replaces O(callables) scans per lookup.
    function_positions: HashMap<ir::DeclId, usize>,
    function_positions_sealed: bool,
    /// Canonical owner of each stored closure provider, authenticated once at
    /// seal time by `closure_source_owner` (immutable thereafter).
    closure_owner_sites: HashMap<ir::DeclId, ir::DeclId>,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum TraitLookupKey {
    Source(PathKey),
    Builtin(BuiltinTrait),
}
#[derive(Debug)]
struct FrozenAssociatedBounds {
    ids: Arc<[ir::DeclId]>,
    relaxed_sized: bool,
    predicates: Arc<[ir::Predicate]>,
    source_index: Option<usize>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum GenericBindingKey {
    Param(NameKey),
    SelfType,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    /// Size of one callable's own source input (parameters plus statements).
    pub fn source_callable_input_size(&self, decl: ir::DeclId) -> usize {
        self.source_callable(decl).map_or(0, |function| {
            function
                .params
                .len()
                .saturating_add(function.body.statements.len())
                .saturating_add(1)
        })
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
        if written != explicit {
            return Err(format!(
                "E0107: function takes {explicit} generic arguments but {written} were supplied"
            ));
        }
        Ok(())
    }
    fn source_type_context(&self, declaration: ir::DeclId) -> Result<SourceTypeContext, String> {
        let info = self.declaration(declaration)?;
        self.modules
            .check(info.path.module)
            .map_err(module_diagnostic)?;
        let owner = match info.kind {
            ir::DeclKind::Trait | ir::DeclKind::TraitImpl => Some(info.id),
            ir::DeclKind::TraitItem
            | ir::DeclKind::AssociatedType
            | ir::DeclKind::AssociatedConst => info.owner,
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
            ir::DeclKind::Struct => {
                let declaration = self
                    .program
                    .structs
                    .get(info.source_index)
                    .ok_or("missing retained struct projection source")?;
                if declaration.module != context.module {
                    return Err("nominal projection retained module changed".into());
                }
                let mut found = false;
                for field in &declaration.fields {
                    budget.charge_work(1)?;
                    found |= retained_type_projection(names, &field.ty, projection, budget)?;
                }
                found
            }
            ir::DeclKind::Enum => {
                let declaration = self
                    .program
                    .enums
                    .get(info.source_index)
                    .ok_or("missing retained enum projection source")?;
                if declaration.module != context.module {
                    return Err("nominal projection retained module changed".into());
                }
                let mut found = false;
                for variant in &declaration.variants {
                    for field in &variant.fields {
                        budget.charge_work(1)?;
                        found |= retained_type_projection(names, &field.ty, projection, budget)?;
                    }
                }
                found
            }
            ir::DeclKind::AssociatedType => {
                associated_declarations::contains_projection(self, info.id, projection, budget)?
            }
            ir::DeclKind::AssociatedConst => {
                associated_constants::retains_projection(self, info, projection, budget)?
            }
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
                for bound in &head.bounds {
                    found |= retained_type_projection(names, &bound.subject, projection, budget)?;
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
                found |= retained_type_projection(names, &bound.subject, projection, budget)?;
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
            ir::DeclKind::Struct => {
                inspect_bounds(&self.program.structs[info.source_index].bounds)?
            }
            ir::DeclKind::Enum => inspect_bounds(&self.program.enums[info.source_index].bounds)?,
            ir::DeclKind::Trait => inspect_bounds(&self.program.traits[info.source_index].bounds)?,
            ir::DeclKind::TraitItem => {
                let owner =
                    self.declaration(info.owner.ok_or("trait source item missing owner")?)?;
                let head = &self
                    .program
                    .traits
                    .get(owner.source_index)
                    .and_then(|retained| retained.items.get(info.source_index))
                    .ok_or("missing retained trait bound projection item")?
                    .signature;
                inspect_bounds(&head.bounds)?;
            }
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
            ir::DeclKind::Struct => inspect(&self.program.structs[info.source_index].bounds)?,
            ir::DeclKind::Enum => inspect(&self.program.enums[info.source_index].bounds)?,
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
    /// Source trait declaration behind a trait DeclId.
    pub(super) fn source_trait(&self, principal: ir::DeclId) -> Result<&TraitDecl, String> {
        let info = self.declaration(principal)?;
        if info.kind != ir::DeclKind::Trait || info.source_index == usize::MAX {
            return Err("E0038: builtin marker traits are not dyn compatible".into());
        }
        self.program
            .traits
            .get(info.source_index)
            .ok_or_else(|| "missing source trait declaration".into())
    }
    fn check_dyn_compatible(
        &self,
        principal: ir::DeclId,
        budget: &ir::PlanBudget,
        depth: usize,
    ) -> Result<(), String> {
        budget.check_depth(depth)?;
        budget.charge_work(1)?;
        let declaration = self.source_trait(principal)?;
        let not_compatible = |why: &str| -> Result<(), String> {
            Err(format!(
                "E0038: the trait `{}` is not dyn compatible: {why}",
                self.declaration(principal)
                    .map(|i| i.name.clone())
                    .unwrap_or_default()
            ))
        };
        for supertrait in &declaration.supertraits {
            match supertrait {
                TraitPath::Builtin(BuiltinTrait::Sized) => {
                    return not_compatible("it requires `Self: Sized`");
                }
                TraitPath::Builtin(_) => {
                    return not_compatible("a supertrait mentions `Self` (Copy/Clone/Drop)");
                }
                TraitPath::Source(_) => {
                    let parent = self.resolve_trait_path(supertrait)?;
                    self.check_dyn_compatible(parent, budget, depth + 1)?;
                }
            }
        }
        for item in &declaration.items {
            budget.charge_work(1)?;
            let head = &item.signature;
            let names = self.names.as_ref();
            let method = names
                .spelling(head.name)
                .map_err(native_adapter::name_diagnostic)?
                .to_owned();
            // `where Self: Sized` excludes the method from the vtable, so it
            // does not affect dyn compatibility (stock rustc rule).
            if head.bounds.iter().any(|bound| {
                matches!(bound.subject, Type::SelfType)
                    && bound
                        .traits
                        .iter()
                        .any(|t| matches!(t, TraitPath::Builtin(BuiltinTrait::Sized)))
            }) {
                continue;
            }
            if matches!(item.receiver, ReceiverKind::None) {
                return not_compatible(&format!(
                    "associated function `{method}` has no `self` parameter"
                ));
            }
            if !head.type_params.is_empty() {
                return not_compatible(&format!("method `{method}` has generic type parameters"));
            }
            for (_, ty) in &head.params {
                if source_type_mentions_self(ty, budget, 0)? {
                    return not_compatible(&format!(
                        "method `{method}` references the `Self` type in its parameters"
                    ));
                }
            }
            if source_type_mentions_self(&head.result, budget, 0)? {
                return not_compatible(&format!(
                    "method `{method}` references the `Self` type in its return type"
                ));
            }
        }
        Ok(())
    }
    fn parameter_key(&self, ident: AstIdent) -> Result<GenericBindingKey, String> {
        Ok(GenericBindingKey::Param(
            self.names
                .key(ident, LookupClass::TypeParameter)
                .map_err(native_adapter::name_diagnostic)?,
        ))
    }
    pub fn declaration(&self, id: ir::DeclId) -> Result<&DeclInfo, String> {
        self.slots
            .get(&id)
            .and_then(|slot| self.declarations.get(*slot))
            .filter(|d| d.id == id)
            .ok_or_else(|| "foreign generic declaration identity".into())
    }
    /// Appends a declaration under its identity. Duplicate identities are
    /// rejected; storage order carries no meaning.
    /// An empty registry over the same program, for identity tests.
    #[cfg(test)]
    fn empty_like(&self) -> Self {
        Self {
            program: self.program.clone(),
            names: self.names.clone(),
            modules: self.modules.clone(),
            symbols: self.symbols.clone(),
            declarations: Vec::new(),
            slots: HashMap::new(),
            functions: HashMap::new(),
            constants: HashMap::new(),
            const_ids: Vec::new(),
            nominals: HashMap::new(),
            function_ids: Vec::new(),
            callable_sources: Vec::new(),
            impl_ids: Vec::new(),
            inherent_peers: HashMap::new(),
            associated_ids: Vec::new(),
            traits: HashMap::new(),
            trait_ids: Vec::new(),
            native_iterator_traits: [None; 2],
            trait_items: Vec::new(),
            trait_default_callables: HashMap::new(),
            associated_constants: associated_constants::Registry::default(),
            closure_recipes: HashMap::new(),
            trait_associated_types: Vec::new(),
            associated_type_bounds: HashMap::new(),
            trait_imports: Vec::new(),
            function_positions: HashMap::new(),
            function_positions_sealed: false,
            closure_owner_sites: HashMap::new(),
        }
    }
    fn push_declaration(&mut self, info: DeclInfo) -> Result<(), String> {
        let slot = self.declarations.len();
        if self.slots.contains_key(&info.id) {
            return Err("duplicate declaration identity".into());
        }
        self.slots.insert(info.id, slot);
        self.declarations.push(info);
        Ok(())
    }
    /// Identity-checked mutable access during registration. Callers never
    /// index storage by `DeclId.index` directly, so the identity scheme can
    /// change without touching them.
    fn declaration_mut(&mut self, id: ir::DeclId) -> Result<&mut DeclInfo, String> {
        decl_mut(&self.slots, &mut self.declarations, id)
    }
}
/// Stable key of a module's canonical resolved path (FNV-1a over segment
/// spellings, root to leaf). It depends only on the path, never on arena
/// handles or the order other modules were declared in.
fn canonical_module_key(
    modules: &ModuleArena,
    names: &NameStore,
    module: ModuleId,
) -> Result<u64, String> {
    let mut segments = Vec::new();
    let mut current = Some(module);
    while let Some(id) = current {
        segments.push(modules.segment(id).map_err(module_diagnostic)?);
        current = modules.parent(id).map_err(module_diagnostic)?;
        if segments.len() > 4096 {
            return Err("module path depth limit exceeded".into());
        }
    }
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut mix = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    };
    for segment in segments.iter().rev() {
        match segment {
            super::module_identity::ModuleSegment::SourceRoot => mix(b"\0crate"),
            super::module_identity::ModuleSegment::Catalog(item) => {
                mix(format!("\0core{item:?}").as_bytes())
            }
            super::module_identity::ModuleSegment::Source(ident) => mix(names
                .spelling(*ident)
                .map_err(native_adapter::name_diagnostic)?
                .as_bytes()),
        }
    }
    Ok(hash)
}
/// Identity-checked mutable registry access that borrows only the
/// declaration storage, so callers may hold other registry fields.
fn decl_mut<'a>(
    slots: &HashMap<ir::DeclId, usize>,
    declarations: &'a mut [DeclInfo],
    id: impl std::borrow::Borrow<ir::DeclId>,
) -> Result<&'a mut DeclInfo, String> {
    let id = *id.borrow();
    slots
        .get(&id)
        .and_then(|slot| declarations.get_mut(*slot))
        .filter(|d| d.id == id)
        .ok_or_else(|| "foreign generic declaration identity".into())
}
#[derive(Clone, Debug)]
enum NominalSchema {
    Struct(StructDecl),
    Enum(EnumDecl),
}
#[derive(Debug, Default)]
pub struct NominalInterner {
    /// Immutable canonical applications sealed at plan construction, shared
    /// read-only by every per-call scratch interner.
    base: Arc<CanonicalNominals>,
    applications: HashMap<(ir::DeclId, Vec<ir::Ty>), ir::Ty>,
    /// Canonical slot (see `canonical_slot`) to its application.
    by_slot: HashMap<usize, ir::Ty>,
    active: Vec<ir::DeclId>,
    /// Applications under construction, keyed by exact application, with the
    /// cycle-closing back edge handed to recursive references to them.
    building: Vec<((ir::DeclId, Vec<ir::Ty>), ir::Ty)>,
}
#[derive(Clone, Copy, Debug)]
struct SourceTypeContext {
    declaration: ir::DeclId,
    module: ModuleId,
    owner: Option<ir::DeclId>,
}
/// Canonical nominal applications interned while the plan was constructed
/// (every declaration's symbolic application and what it reaches). Immutable.
#[derive(Debug, Default)]
struct CanonicalNominals {
    applications: HashMap<(ir::DeclId, Vec<ir::Ty>), ir::Ty>,
    by_slot: HashMap<usize, ir::Ty>,
}
/// Per-call lowering scratch: the interning memo, current source context and
/// byte reservations of one computation. Owned by the caller, never stored in
/// a plan or fact; reservations are released when the scratch is dropped.
#[derive(Debug)]
struct PlanState {
    source_context: Option<SourceTypeContext>,
    schemas: Arc<HashMap<ir::DeclId, Arc<NominalSchema>>>,
    nominals: NominalInterner,
    reservations: Vec<ir::PlanReservation>,
}
impl PlanState {
    fn scratch(
        schemas: &Arc<HashMap<ir::DeclId, Arc<NominalSchema>>>,
        base: &Arc<CanonicalNominals>,
    ) -> Self {
        Self {
            source_context: None,
            schemas: schemas.clone(),
            nominals: NominalInterner {
                base: base.clone(),
                ..NominalInterner::default()
            },
            reservations: Vec::new(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct PlanCtx {
    program: Arc<Program>,
    catalog: Arc<tr::CoreCatalog>,
    world: Option<Arc<tr::ValidatedTraitWorld>>,
    validated: Arc<HashMap<ir::DeclId, ir::GenericSignature>>,
    registry: Arc<DeclRegistry>,
    /// Immutable nominal source schemas, sealed at construction.
    nominal_schemas: Arc<HashMap<ir::DeclId, Arc<NominalSchema>>>,
    /// Immutable canonical applications, sealed at construction.
    nominals: Arc<CanonicalNominals>,
    /// Immutable per-callable source contexts, sealed at construction.
    contexts: Arc<HashMap<ir::DeclId, CallableContext>>,
    source_schemas: Arc<SourceSchemas>,
    /// Immutable whole-program signature and nominal lookup maps, built once
    /// when the trait world is published and shared by every body check.
    /// Previously each body copied every signature, which made checking
    /// quadratic in program size in both time and retained bytes.
    shared_env: Option<Arc<SharedTypeEnv>>,
    /// Per-module item ownership, in first-declaration order. Each entry is
    /// the input of one keyed ITEM_INVENTORY node.
    module_items: Arc<[ModuleItems]>,
    pub budget: ir::PlanBudget,
}
/// Declarations owned by one module: the per-module inventory CHECK fans
/// out over instead of scanning whole-program item counts.
#[derive(Clone, Debug, Default)]
pub struct ModuleItems {
    pub callables: Vec<usize>,
    pub constants: Vec<ir::DeclId>,
    pub nominals: Vec<ir::DeclId>,
    pub associated_constants: Vec<ir::DeclId>,
    /// Source implementation blocks (per-impl coherence inputs).
    pub impls: Vec<ir::DeclId>,
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
        Type::Opaque(opaque) => {
            add(std::mem::size_of::<SourceOpaqueType>())?;
            add(opaque.bounds.len() * std::mem::size_of::<TraitPath>())?;
            add(opaque.regions.len() * std::mem::size_of::<RegionName>())?;
            for bound in &opaque.bounds {
                if let TraitPath::Source(path) = bound {
                    budget.charge_work(path.segments.len())?;
                    for segment in &path.segments {
                        add(std::mem::size_of::<PathSegment>())?;
                        add(segment.region_args.len() * std::mem::size_of::<RegionName>())?;
                        for argument in segment.args.iter().chain(
                            segment
                                .associated_types
                                .iter()
                                .map(|binding| &binding.value),
                        ) {
                            add(source_type_storage(argument, budget, depth + 1)?)?;
                        }
                        add(segment.associated_types.len()
                            * std::mem::size_of::<AssociatedTypeBinding>())?;
                    }
                }
            }
        }
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
/// Qualify one complete const body against the sealed declaration identities.
/// This is shared by signature publication and public per-callable checking.
fn validate_const_forwarding(
    registry: &DeclRegistry,
    function: &Function,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    budget.charge_work(1)?;
    if function.constness == Constness::Runtime {
        return Ok(());
    }
    let names = registry.program.names.as_ref();
    let owner = function
        .name
        .display(names)
        .map_err(native_adapter::name_diagnostic)?;
    walk_depth_inspecting(
        DepthNode::Block(&function.body),
        None,
        Some(names),
        Some(&mut |node, _| {
            budget.charge_work(1)?;
            match node {
                DepthNode::Expr(Expr::Call { name, .. }) => {
                    let key = name_scope::path_key(names, name, Namespace::Value)?;
                    let declaration = registry.functions.get(&key).ok_or_else(|| {
                        format!(
                            "in const function {owner}: unresolved callable in constant context"
                        )
                    })?;
                    let callee = registry.source_callable(*declaration)
                        .ok_or_else(|| format!("unsupported: in const function {owner}: callable has no const-qualified source body"))?;
                    if callee.constness != Constness::Const {
                        return Err(format!(
                            "E0015: in const function {owner}: cannot call non-const function in constant context"
                        ));
                    }
                }
                DepthNode::Expr(
                    Expr::Println(_)
                    | Expr::Format { .. }
                    | Expr::Output(_)
                    | Expr::FormatInto(_)
                    | Expr::NativeSyscall(_),
                ) => {
                    return Err(format!(
                        "E0015: in const function {owner}: cannot call non-const operation in constant context"
                    ));
                }
                DepthNode::Expr(
                    Expr::MethodCall { .. }
                    | Expr::AssociatedCall { .. }
                    | Expr::TraitCall { .. }
                    | Expr::Invoke { .. },
                ) => {
                    return Err(format!(
                        "unsupported: in const function {owner}: indirect or method calls in constant contexts"
                    ));
                }
                _ => {}
            }
            Ok(())
        }),
    )
}
fn validate_const_typed_calls(
    registry: &DeclRegistry,
    function: &Function,
    checked: &ir::CheckedFunction,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    if function.constness == Constness::Runtime {
        return Ok(());
    }
    for expression in &checked.exprs {
        budget.charge_work(1)?;
        if let ir::ExprKind::Call { target, .. } = &expression.kind {
            match target {
                ir::Callee::Direct(target) => {
                    let source = registry
                        .source_callable(target.declaration)
                        .ok_or("unsupported: const call has no qualified source declaration")?;
                    if source.constness != Constness::Const {
                        return Err("E0015: non-const call in typed constant body".into());
                    }
                }
                ir::Callee::TraitItem(_) | ir::Callee::Virtual(_) | ir::Callee::Untupled(_) => {
                    return Err("E0015: non-const trait call in typed constant body".into());
                }
            }
        }
    }
    Ok(())
}
impl PlanCtx {
    /// Fresh per-call lowering scratch over the immutable canonical base.
    fn scratch(&self) -> PlanState {
        PlanState::scratch(&self.nominal_schemas, &self.nominals)
    }
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
            let mut state = self.scratch();
            if let ir::FactsCtx::Validated(facts) = caller {
                state.source_context =
                    Some(self.registry.source_type_context(facts.declaration())?);
            }
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
        let owner = |domain| {
            super::owner_path::standalone_issued(
                &super::expansion_budget::ExpansionBudget::default(),
                domain,
            )
            .map_err(|error| error.to_string())
        };
        let catalog = owner(super::owner_path::OwnerDomain::DeclarationSnapshot)?;
        let world = owner(super::owner_path::OwnerDomain::TraitWorld)?;
        Self::from_shared_program_with(shared, budget, true, catalog, world)
    }
    /// `eager = false` is the graph constructor: impl coherence, inherent
    /// overlap and C ABI checks are left to per-item graph nodes
    /// (`impl_coherence`, `item_coherence`) that CHECK demands.
    pub fn from_shared_program_with(
        shared: Arc<Program>,
        budget: &ir::PlanBudget,
        eager: bool,
        catalog_owner: super::owner_path::Issued,
        world_owner: super::owner_path::Issued,
    ) -> Result<Self, String> {
        let program = shared.as_ref();
        program.validate_name_handles()?;
        program.validate_c_source()?;
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
        let snapshot = ir::SnapshotId::issued(catalog_owner);
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
            slots: HashMap::new(),
            functions: HashMap::new(),
            constants: HashMap::new(),
            const_ids: Vec::new(),
            nominals: HashMap::new(),
            function_ids: Vec::new(),
            callable_sources: Vec::new(),
            impl_ids: Vec::new(),
            inherent_peers: HashMap::new(),
            associated_ids: Vec::new(),
            traits: HashMap::new(),
            trait_ids: Vec::new(),
            native_iterator_traits: [None; 2],
            trait_items: Vec::new(),
            trait_default_callables: HashMap::new(),
            associated_constants: associated_constants::Registry::default(),
            closure_recipes: HashMap::new(),
            trait_associated_types: Vec::new(),
            associated_type_bounds: HashMap::new(),
            trait_imports: program.trait_imports.clone(),
            function_positions: HashMap::new(),
            function_positions_sealed: false,
            closure_owner_sites: HashMap::new(),
        };
        reserve_registry_map(&mut registry.functions, program.functions.len(), budget)?;
        reserve_registry_map(
            &mut registry.closure_recipes,
            program.closure_sources.len(),
            budget,
        )?;
        reserve_registry_map(&mut registry.constants, program.constants.len(), budget)?;
        budget.retain_bytes(
            program
                .constants
                .len()
                .checked_mul(std::mem::size_of::<ir::DeclId>())
                .ok_or("constant identity inventory overflow")?,
        )?;
        registry
            .const_ids
            .try_reserve_exact(program.constants.len())
            .map_err(|_| "constant identity inventory allocation")?;
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
        let mut source_schemas = SourceSchemasBuilder::default();
        let mut state = PlanState::scratch(&Arc::default(), &Arc::default());
        let mut contexts: HashMap<ir::DeclId, CallableContext> = HashMap::new();
        for entry in catalog_builder.base_declarations() {
            let info = &entry.info;
            budget.charge_work(1)?;
            budget.retain_bytes(1024 + entry.name.len() * 4)?;
            if entry.declaration.index as usize != registry.declarations.len() {
                return Err("catalog prefix identity mismatch".into());
            }
            registry.push_declaration(catalog_decl_info(entry, budget)?)?;
            if let Some(schema) = &entry.schema {
                source_schemas.functions.insert(
                    source_schemas::decl_slot(entry.declaration, &registry, budget)?,
                    StoredSchema::copy(schema, budget)?,
                    budget,
                )?;
            }
            source_schemas.set_region_mode(
                entry.declaration,
                info.region_contract_mode,
                &registry,
                budget,
            )?;
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
        let mut default_callable_count = 0usize;
        let declarations = program
            .functions
            .len()
            .checked_add(program.structs.len())
            .and_then(|n| n.checked_add(program.constants.len()))
            .and_then(|n| n.checked_add(program.enums.len()))
            .and_then(|n| {
                program.traits.iter().try_fold(n, |n, t| {
                    let defaults = t.items.iter().filter(|i| i.default.is_some()).count();
                    default_callable_count = default_callable_count.checked_add(defaults)?;
                    n.checked_add(
                        1 + t.items.len()
                            + t.associated_types.len()
                            + t.associated_constants.len()
                            + defaults,
                    )
                })
            })
            .and_then(|n| {
                program.impls.iter().try_fold(n, |n, b| {
                    n.checked_add(1 + b.items.len() + b.associated_constants.len())
                })
            })
            .ok_or("generic declaration resource overflow")?;
        reserve_registry_map(
            &mut registry.trait_default_callables,
            default_callable_count,
            budget,
        )?;
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
        let mut module_ordinals: HashMap<(u64, u8), u64> = HashMap::new();
        let mut insert = |path: Option<&AstPath>,
                          module: ModuleId,
                          item: DeclItemName,
                          params: &[AstIdent],
                          bounds: &[TypeBound],
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
            // Canonical identity: (module path, kind, ordinal among the
            // module's same-kind items), never whole-program insertion order.
            let module_key = canonical_module_key(&program.modules, &program.names, module)?;
            let ordinal = module_ordinals
                .entry((module_key, kind as u8))
                .or_insert(0u64);
            let id = ir::DeclId {
                snapshot,
                index: ir::DeclId::source_index(module_key, kind, *ordinal)?,
            };
            *ordinal += 1;
            let arity = params
                .len()
                .checked_add(usize::from(kind == ir::DeclKind::Trait))
                .ok_or("generic parameter resource overflow")?;
            if arity > budget.limits().params {
                return Err("generic parameter resource limit exceeded".into());
            }
            // `?Sized` on a parameter of this binder removes its implicit
            // `Sized` row everywhere the environment is rebuilt.
            let first = usize::from(kind == ir::DeclKind::Trait);
            let mut relaxed = 0u32;
            for bound in bounds.iter().filter(|bound| bound.maybe_unsized) {
                budget.charge_work(params.len())?;
                let Type::Param(subject) = &bound.subject else {
                    continue;
                };
                let subject = program
                    .names
                    .key(*subject, LookupClass::TypeParameter)
                    .map_err(native_adapter::name_diagnostic)?;
                for (index, parameter) in params.iter().enumerate() {
                    let key = program
                        .names
                        .key(*parameter, LookupClass::TypeParameter)
                        .map_err(native_adapter::name_diagnostic)?;
                    if key == subject {
                        relaxed |= 1u32 << (index + first);
                    }
                }
            }
            let param_env = Arc::new(ir::ParamEnv::with_relaxed_sized(
                ir::BinderId { declaration: id },
                arity,
                relaxed,
            )?);
            if matches!(
                kind,
                ir::DeclKind::Function
                    | ir::DeclKind::Const
                    | ir::DeclKind::Struct
                    | ir::DeclKind::Enum
            ) {
                let path = path.ok_or("contract: missing declaration path")?;
                let namespace = if matches!(kind, ir::DeclKind::Function | ir::DeclKind::Const) {
                    Namespace::Value
                } else {
                    Namespace::Type
                };
                let key = name_scope::path_key(program.names.as_ref(), path, namespace)?;
                let previous = if matches!(kind, ir::DeclKind::Function | ir::DeclKind::Const) {
                    if registry.functions.contains_key(&key)
                        || registry.constants.contains_key(&key)
                    {
                        return Err(format!("duplicate value declaration {name}"));
                    }
                    if kind == ir::DeclKind::Function {
                        registry.functions.insert(key, id)
                    } else {
                        registry.constants.insert(key, id)
                    }
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
            if registry.slots.contains_key(&id) {
                return Err("duplicate declaration identity".into());
            }
            registry.slots.insert(id, registry.declarations.len());
            registry.declarations.push(DeclInfo {
                embedded_function: None,
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
                &d.bounds,
                ir::DeclKind::Struct,
                index,
            )?;
            clone_visibility(&d.visibility, budget)?;
            Arc::make_mut(&mut state.schemas)
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
                &d.bounds,
                ir::DeclKind::Enum,
                index,
            )?;
            clone_visibility(&d.visibility, budget)?;
            Arc::make_mut(&mut state.schemas).insert(id, Arc::new(NominalSchema::Enum(d.clone())));
        }
        // User callables keep their indices; native prelude callables follow.
        let mut prelude_callables: Vec<(ir::DeclId, CallableSource)> = Vec::new();
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
                &f.bounds,
                ir::DeclKind::Function,
                index,
            )?;
            let key = name_scope::path_key(program.names.as_ref(), &f.name, Namespace::Type)?;
            if program
                .closure_sources
                .get(&key)
                .is_some_and(|source| source.stored.is_some())
            {
                budget.charge_nodes(1)?;
                budget.retain_bytes(std::mem::size_of::<ir::ClosureRecipeId>() + 32)?;
                registry
                    .closure_recipes
                    .insert(id, ir::ClosureRecipeId::new());
            }
            if program.in_prelude(f.module) {
                prelude_callables.push((id, CallableSource::Free(index)));
            } else {
                registry.function_ids.push(id);
                registry.callable_sources.push(CallableSource::Free(index));
            }
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
                &declaration.bounds,
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
            if let Some(slot) = iterator_traits::native_iterator_slot(program, declaration)?
                && registry.native_iterator_traits[slot].replace(id).is_some()
            {
                return Err("duplicate canonical native iterator trait".into());
            }
            let mut items = Vec::new();
            for (item, method) in declaration.items.iter().enumerate() {
                items.push(insert(
                    None,
                    declaration.module,
                    DeclItemName::Source(method.signature.name),
                    &method.signature.type_params,
                    &method.signature.bounds,
                    ir::DeclKind::TraitItem,
                    item,
                )?);
                if method.default.is_some() {
                    let id = insert(
                        None,
                        declaration.module,
                        DeclItemName::Source(method.signature.name),
                        &method.signature.type_params,
                        &method.signature.bounds,
                        ir::DeclKind::AssociatedFunction,
                        item,
                    )?;
                    let source = CallableSource::TraitDefault {
                        trait_index: index,
                        item,
                    };
                    if registry
                        .trait_default_callables
                        .insert((index, item), id)
                        .is_some()
                    {
                        return Err("duplicate canonical trait default callable".into());
                    }
                    if program.in_prelude(declaration.module) {
                        prelude_callables.push((id, source));
                    } else {
                        registry.function_ids.push(id);
                        registry.callable_sources.push(source);
                    }
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
                    &[],
                    ir::DeclKind::AssociatedType,
                    item,
                )?);
            }
            let mut constants = Vec::new();
            for (item, constant) in declaration.associated_constants.iter().enumerate() {
                constants.push(insert(
                    None,
                    declaration.module,
                    DeclItemName::Source(constant.name),
                    &[],
                    &[],
                    ir::DeclKind::AssociatedConst,
                    item,
                )?);
            }
            registry.associated_constants.traits.push(constants);
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
                &block.bounds,
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
                    &method.function.bounds,
                    ir::DeclKind::AssociatedFunction,
                    item,
                )?;
                let source = CallableSource::Associated {
                    implementation,
                    item,
                };
                if program.in_prelude(block.module) {
                    prelude_callables.push((id, source));
                } else {
                    registry.function_ids.push(id);
                    registry.callable_sources.push(source);
                }
                ids.push(id);
            }
            let mut constants = Vec::new();
            for (item, constant) in block.associated_constants.iter().enumerate() {
                constants.push(insert(
                    None,
                    block.module,
                    DeclItemName::Source(constant.name),
                    &[],
                    &[],
                    ir::DeclKind::AssociatedConst,
                    item,
                )?);
            }
            registry.associated_constants.impls.push(constants);
            registry.associated_ids.push(ids);
        }
        for (index, declaration) in program.constants.iter().enumerate() {
            let item = *declaration
                .name
                .segments()
                .last()
                .ok_or("empty constant path")?;
            let id = insert(
                Some(&declaration.name),
                declaration.module,
                DeclItemName::Source(item),
                &[],
                &[],
                ir::DeclKind::Const,
                index,
            )?;
            registry.const_ids.push(id);
        }
        for (id, source) in prelude_callables {
            registry.function_ids.push(id);
            registry.callable_sources.push(source);
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
        for declaration in &program.constants {
            clone_visibility(&declaration.visibility, budget)?;
            let key =
                name_scope::path_key(program.names.as_ref(), &declaration.name, Namespace::Value)?;
            let id = registry.constants[&key];
            decl_mut(&registry.slots, &mut registry.declarations, id)?.visibility =
                declaration.visibility.clone();
        }
        for declaration in &program.structs {
            clone_visibility(&declaration.visibility, budget)?;
            let id = registry.nominals[&name_scope::path_key(
                program.names.as_ref(),
                &declaration.name,
                Namespace::Type,
            )?];
            decl_mut(&registry.slots, &mut registry.declarations, id)?.visibility =
                declaration.visibility.clone();
        }
        for declaration in &program.enums {
            clone_visibility(&declaration.visibility, budget)?;
            let id = registry.nominals[&name_scope::path_key(
                program.names.as_ref(),
                &declaration.name,
                Namespace::Type,
            )?];
            decl_mut(&registry.slots, &mut registry.declarations, id)?.visibility =
                declaration.visibility.clone();
        }
        for (position, source) in registry.callable_sources.iter().enumerate() {
            let CallableSource::Free(index) = *source else {
                continue;
            };
            let function = &program.functions[index];
            clone_visibility(&function.visibility, budget)?;
            let id = registry.function_ids[position];
            decl_mut(&registry.slots, &mut registry.declarations, id)?.visibility =
                function.visibility.clone();
            if let Some(receipt) = &function.embedded_source {
                let bound = function_source::bind_registration(
                    receipt,
                    program,
                    registry.declaration(id)?,
                    index,
                    id,
                    budget,
                )?;
                decl_mut(&registry.slots, &mut registry.declarations, id)?.embedded_function =
                    Some(bound);
            }
        }
        associated_constants::seal_registry_owners(&mut registry, program, budget)?;
        for (index, declaration) in program.traits.iter().enumerate() {
            let id = registry.trait_ids[index];
            clone_visibility(&declaration.visibility, budget)?;
            decl_mut(&registry.slots, &mut registry.declarations, id)?.visibility =
                declaration.visibility.clone();
            let trait_env = registry.declaration(id)?.param_env.clone();
            let inherited_env = registry.declaration(id)?.env.clone();
            for item in &registry.trait_associated_types[index] {
                clone_visibility(&declaration.visibility, budget)?;
                let info = decl_mut(&registry.slots, &mut registry.declarations, item)?;
                info.owner = Some(id);
                info.visibility = declaration.visibility.clone();
                info.env = inherited_env.clone();
            }
            for (ii, item) in registry.trait_items[index].iter().enumerate() {
                clone_visibility(&declaration.visibility, budget)?;
                let info = decl_mut(&registry.slots, &mut registry.declarations, item)?;
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
            decl_mut(&registry.slots, &mut registry.declarations, id)?
                .path
                .module = block.module;
        }
        for (index, source) in registry.callable_sources.iter().enumerate() {
            let CallableSource::TraitDefault { trait_index, item } = *source else {
                continue;
            };
            let owner = registry.trait_ids[trait_index];
            let trait_env = registry.declaration(owner)?.param_env.clone();
            let module = registry.declaration(owner)?.path.module;
            let id = registry.function_ids[index];
            let info = decl_mut(&registry.slots, &mut registry.declarations, id)?;
            info.owner = Some(owner);
            info.receiver = program.traits[trait_index].items[item].receiver;
            clone_visibility(&program.traits[trait_index].visibility, budget)?;
            info.visibility = program.traits[trait_index].visibility.clone();
            info.env = Arc::new(ir::GenericEnv::new(
                &[trait_env.clone(), info.param_env.clone()],
                budget,
            )?);
            contexts.insert(
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
            let impl_env = registry
                .declaration(registry.impl_ids[implementation])?
                .param_env
                .clone();
            for id in ids {
                let info = decl_mut(&registry.slots, &mut registry.declarations, id)?;
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
                    for (_, path) in source.bounds.iter() {
                        ids.push(registry.resolve_trait_path(path)?);
                    }
                }
                registry.associated_type_bounds.insert(
                    declaration,
                    FrozenAssociatedBounds {
                        ids: ids.into(),
                        relaxed_sized: member.relaxed_sized,
                        predicates: Arc::from([]),
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
                // Trait impls may target reference types (`impl Tr for &T`).
                block.trait_path.is_some(),
            )?;
            if raw_type_has_projection(&self_ty, budget, 0)? {
                return Err("unsupported: projection-bearing impl heads".into());
            }
            let nominal = match &self_ty {
                ir::Ty::Struct(t) => t.def().source_decl,
                ir::Ty::Enum(t) => t.def().source_decl,
                // Trait impls on structural/primitive types: the impl itself is
                // the owning identity. Selection goes through trait headers.
                _ if block.trait_path.is_some() => impl_id,
                _ => {
                    let receipt = block.primitive_source.as_ref().ok_or("E0390: cannot define inherent impl for primitive types outside their authenticated source issuer")?;
                    receipt.replay_registration(
                        program,
                        &registry,
                        implementation,
                        impl_id,
                        &self_ty,
                        budget,
                    )?;
                    impl_id
                }
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
            // Trait impl constraints include associated equalities and are
            // validated against the typed header by the trait world.
            if block.trait_path.is_none()
                && block
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
                let source = source_schemas::assignment_source(
                    (impl_id, associated),
                    (&assignment.value, &rhs),
                    (&block.regions, program.names.as_ref()),
                    info.env.clone(),
                    budget,
                )?;
                source_schemas.assignment(impl_id, associated, rhs, source, budget)?;
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
                                    let item_name = &registry.declaration(*item_id)?.path.item;
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
                    .declaration_mut(*id)?
                    .path
                    .associated = Some(owner);
                budget.retain_bytes(512)?;
                contexts.insert(
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
        Arc::get_mut(&mut registry)
            .ok_or("closure provider domains shared before source schema completion")?
            .install_closure_provider_domains(budget)?;
        provider_source_context::install_closure_source_contexts(&mut contexts, &registry, budget)?;
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
            let context = contexts
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
            // Additional exact conservative admission, not a refund or carveout
            // from the existing broad declaration reservation.
            let capacity = f
                .params
                .len()
                .checked_add(usize::from(context.receiver != ReceiverKind::None))
                .ok_or("signature params overflow")?;
            let (mut params, params_storage) = StoredSchema::params(capacity, budget)?;
            budget.charge_work(f.params.len())?;
            for (_, ty) in &f.params {
                params.push(state.source_ty(&registry, ty, &bindings, caller, budget, 0, true)?);
            }
            if context.receiver != ReceiverKind::None {
                budget.charge_work(params.len() + 1)?;
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
            source_schemas.set_region_mode(id, region_mode, &registry, budget)?;
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
            source_schemas.source_regions.insert(
                source_schemas::decl_slot(id, &registry, budget)?,
                carrier_regions::attach(
                    &registry,
                    &f.regions,
                    &f.params,
                    &f.result,
                    match registry.callable_sources[index] {
                        CallableSource::Associated { implementation, .. } => {
                            Some(&program.impls[implementation])
                        }
                        _ => None,
                    },
                    &signature,
                    context.receiver,
                    original_source_regions(
                        (program.names.as_ref(), budget),
                        id,
                        &f.regions,
                        &f.params,
                        &f.result,
                        &signature.ref_contract,
                        context.receiver,
                    )?,
                    budget,
                )?,
                budget,
            )?;
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
            source_schemas.functions.insert(
                source_schemas::decl_slot(id, &registry, budget)?,
                StoredSchema {
                    value: tr::GenericSchema {
                        env: info.env.clone(),
                        signature,
                        predicates: rows.into(),
                    },
                    params_storage,
                },
                budget,
            )?;
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
                let capacity = h
                    .params
                    .len()
                    .checked_add(usize::from(item.receiver != ReceiverKind::None))
                    .ok_or("signature params overflow")?;
                let (mut params, params_storage) = StoredSchema::params(capacity, budget)?;
                budget.charge_work(h.params.len())?;
                for (_, ty) in &h.params {
                    params
                        .push(state.source_ty(&registry, ty, &bindings, caller, budget, 0, true)?);
                }
                if item.receiver != ReceiverKind::None {
                    budget.charge_work(params.len() + 1)?;
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
                let signature = ir::Signature {
                    ref_contract,
                    params,
                    result,
                };
                source_schemas.source_regions.insert(
                    source_schemas::decl_slot(id, &registry, budget)?,
                    carrier_regions::attach(
                        &registry,
                        &h.regions,
                        &h.params,
                        &h.result,
                        None,
                        &signature,
                        item.receiver,
                        original_source_regions(
                            (program.names.as_ref(), budget),
                            id,
                            &h.regions,
                            &h.params,
                            &h.result,
                            &signature.ref_contract,
                            item.receiver,
                        )?,
                        budget,
                    )?,
                    budget,
                )?;
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
                source_schemas.functions.insert(
                    source_schemas::decl_slot(id, &registry, budget)?,
                    StoredSchema {
                        value: tr::GenericSchema {
                            env: info.env.clone(),
                            signature,
                            predicates: rows.into(),
                        },
                        params_storage,
                    },
                    budget,
                )?;
            }
        }
        associated_declarations::seal(&mut registry, &mut state, budget)?;
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
        // Recipe declarations follow the catalog prefix in the catalog's own
        // (non-source) index range; source identities carry SOURCE_BIT.
        let source_end = catalog_builder.source_start();
        // `program` was name-validated on entry to this constructor.
        let inventory = recipes::inventory_validated(program, budget)?;
        let catalog = catalog_builder
            .finish(source_end, inventory, budget)
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
        let registry_mut =
            Arc::get_mut(&mut registry).ok_or("registry shared before catalog seal")?;
        for entry in catalog.declarations() {
            if entry.declaration.index < u64::from(source_end) {
                continue;
            }
            budget.charge_work(1)?;
            budget.retain_bytes(1024 + entry.name.len() * 4)?;
            registry_mut.push_declaration(catalog_decl_info(entry, budget)?)?;
            if let Some(schema) = &entry.schema {
                source_schemas.functions.insert(
                    source_schemas::decl_slot(entry.declaration, registry_mut, budget)?,
                    StoredSchema::copy(schema, budget)?,
                    budget,
                )?;
            }
            source_schemas.set_region_mode(
                entry.declaration,
                entry.info.region_contract_mode,
                registry_mut,
                budget,
            )?;
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
        let mut peers: HashMap<ir::DeclId, Vec<ir::DeclId>> = HashMap::new();
        for id in &registry_mut.function_ids {
            budget.charge_work(1)?;
            if let Some(owner) = &registry_mut.declaration(*id)?.path.associated
                && owner.trait_decl.is_none()
            {
                peers.entry(owner.nominal).or_default().push(*id);
            }
        }
        registry_mut.inherent_peers = peers;
        registry_mut.function_positions_sealed = true;
        let mut owner_sites = HashMap::new();
        for provider in registry_mut.closure_recipes.keys() {
            budget.charge_work(1)?;
            let info = registry_mut.declaration(*provider)?;
            let Some(function) = program.functions.get(info.source_index) else {
                continue;
            };
            let key =
                name_scope::path_key(program.names.as_ref(), &function.name, Namespace::Type)?;
            if let Some(source) = registry_mut.program.closure_sources.get(&key)
                && source.stored.is_some()
            {
                owner_sites.insert(*provider, registry_mut.closure_source_owner(source)?);
            }
        }
        registry_mut.closure_owner_sites = owner_sites;
        // Qualify every declaration before graph-facing publication. The sealed
        // callable index resolves each call without scanning other bodies.
        for function in &program.functions {
            validate_const_forwarding(&registry, function, budget)?;
        }
        // Consume construction authority once before graph-facing adapters.
        source_schemas.complete_closure_predicates(&registry, budget)?;
        let source_schemas = source_schemas.seal_from_registry(&registry, budget)?;
        // Construction storage stays charged for the plan's life; the
        // interned applications become the immutable canonical base.
        for reservation in std::mem::take(&mut state.reservations) {
            reservation.retain()?;
        }
        let PlanState {
            schemas: nominal_schemas,
            nominals,
            ..
        } = state;
        let nominals = Arc::new(CanonicalNominals {
            applications: nominals.applications,
            by_slot: nominals.by_slot,
        });
        let mut plan = Self {
            program: shared.clone(),
            catalog,
            world: None,
            validated: Arc::new(HashMap::new()),
            registry,
            nominal_schemas,
            nominals,
            contexts: Arc::new(contexts),
            source_schemas,
            shared_env: None,
            module_items: Arc::from([]),
            budget: budget.clone(),
        };
        if eager {
            plan.validate_inherent_overlaps(budget)?;
        }
        plan.publish_trait_world(program, eager, world_owner)?;
        if eager {
            plan.validate_c_abi()?;
        }
        plan.module_items = plan.compute_module_items()?;
        Ok(plan)
    }
    fn compute_module_items(&self) -> Result<Arc<[ModuleItems]>, String> {
        let mut slots: HashMap<ModuleId, usize> = HashMap::new();
        let mut modules: Vec<ModuleItems> = Vec::new();
        let mut slot = |decl: ir::DeclId| -> Result<usize, String> {
            self.budget.charge_work(1)?;
            let module = self.declaration(decl)?.path.module;
            Ok(*slots.entry(module).or_insert_with(|| {
                modules.push(ModuleItems::default());
                modules.len() - 1
            }))
        };
        let mut callables = Vec::new();
        for index in 0..self.callable_count() {
            let decl = self.callable_decl(index)?;
            let at = slot(decl)?;
            // Embedded prelude bodies are trusted library code checked and
            // instantiated on demand, like stock's precompiled std. Eagerly
            // admitting every one made a trivial CHECK cost ~1300 bodies.
            if !self.is_prelude_decl(decl)? {
                callables.push((at, index));
            }
        }
        let mut constants = Vec::new();
        for index in 0..self.constant_count() {
            let decl = self.constant_decl(index)?;
            constants.push((slot(decl)?, decl));
        }
        let mut nominals = Vec::new();
        for index in 0..self.nominal_declaration_count() {
            let decl = self.nominal_declaration(index)?;
            nominals.push((slot(decl)?, decl));
        }
        let mut associated = Vec::new();
        for decl in self.associated_constant_declarations().collect::<Vec<_>>() {
            associated.push((slot(decl)?, decl));
        }
        let mut impls = Vec::new();
        for &decl in &self.registry.impl_ids {
            // Prelude impls are trusted; user impls check against them.
            if self.is_prelude_decl(decl)? {
                continue;
            }
            impls.push((slot(decl)?, decl));
        }
        for (at, index) in callables {
            modules[at].callables.push(index);
        }
        for (at, decl) in constants {
            modules[at].constants.push(decl);
        }
        for (at, decl) in nominals {
            modules[at].nominals.push(decl);
        }
        for (at, decl) in impls {
            modules[at].impls.push(decl);
        }
        for (at, decl) in associated {
            modules[at].associated_constants.push(decl);
        }
        Ok(modules.into())
    }
    /// Number of per-module item inventories.
    pub fn module_inventory_count(&self) -> usize {
        self.module_items.len()
    }
    /// Items owned by the module with inventory ordinal `ordinal`.
    pub fn module_inventory(&self, ordinal: usize) -> Result<&ModuleItems, String> {
        self.module_items
            .get(ordinal)
            .ok_or_else(|| "invalid module inventory ordinal".into())
    }
    /// Nongeneric Source callable indices in callable order. Test and
    /// diagnostic helper only; graph nodes use per-item `is_closed_root`.
    pub fn closed_roots(&self) -> Result<Vec<usize>, String> {
        let mut roots = Vec::new();
        for index in 0..self.callable_count() {
            self.budget.charge_work(1)?;
            if self.is_closed_root(index)? {
                roots.push(index);
            }
        }
        Ok(roots)
    }
    /// Whether callable `index` is a closed source root: a nongeneric,
    /// non-prelude Source callable. Closed-root instance nodes are keyed by
    /// this callable index, a per-item fact, not by a whole-program slot.
    pub fn is_closed_root(&self, index: usize) -> Result<bool, String> {
        let declaration = self.callable_decl(index)?;
        Ok(matches!(
            self.callable_provider(declaration)?,
            CallableProvider::Source { .. }
        ) && !self.callable_env(declaration)?.is_generic()
            && !self.is_prelude_decl(declaration)?)
    }
    fn publish_trait_world(
        &mut self,
        program: &Program,
        eager: bool,
        world_owner: super::owner_path::Issued,
    ) -> Result<(), String> {
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
                let mut state = self.scratch();
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
                    relaxed_sized: self
                        .registry
                        .associated_type_bounds
                        .get(declaration)
                        .is_some_and(|bounds| bounds.relaxed_sized),
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
                let mut state = self.scratch();
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
                    let mut rows = Vec::new();
                    for row in self.source_schemas.assignment_rows(budget)? {
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
        let mut constant_headers = Vec::new();
        for declaration in self
            .registry
            .associated_constants
            .traits
            .iter()
            .chain(&self.registry.associated_constants.impls)
            .flatten()
        {
            budget.charge_work(1)?;
            budget.retain_bytes(std::mem::size_of::<tr::AssociatedConstHeader>())?;
            constant_headers
                .try_reserve_exact(1)
                .map_err(|_| "associated constant header allocation")?;
            constant_headers.push(self.associated_constant_schema(*declaration, budget)?);
        }
        self.validate_constant_completeness(&impls, budget)?;
        headers.constants = constant_headers.into();
        headers.traits = traits.into();
        headers.items = items.into();
        headers.associated_types = associated_types.into();
        headers.impls = impls.into();
        headers.callables = callables.into();
        let adapter = Arc::new(SchemaAdapter {
            program: self.program.clone(),
            catalog: self.catalog.clone(),
            registry: self.registry.clone(),
            nominal_schemas: self.nominal_schemas.clone(),
            nominals: self.nominals.clone(),
            contexts: self.contexts.clone(),
            source_schemas: self.source_schemas.clone(),
            budget: budget.clone(),
        });
        let world =
            tr::ValidatedTraitWorld::build_with(headers, adapter, budget, eager, world_owner)
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
    /// Whether a declaration lives in the embedded native prelude library.
    /// Prelude items are library code: not user telemetry, and nongeneric
    /// prelude bodies are instantiated only on demand.
    pub fn is_prelude_decl(&self, id: ir::DeclId) -> Result<bool, String> {
        let Some(prelude) = self.program.prelude_module else {
            return Ok(false);
        };
        let module = self.declaration(id)?.path.module;
        self.program
            .modules
            .is_descendant(module, prelude)
            .map_err(module_diagnostic)
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
    pub fn callable_constness(&self, declaration: ir::DeclId) -> Result<Constness, String> {
        self.declaration(declaration)?;
        Ok(self
            .registry
            .source_callable(declaration)
            .map_or(Constness::Runtime, |function| function.constness))
    }
    pub fn callable_count(&self) -> usize {
        self.registry.function_ids.len()
    }
    pub fn source_callable_input_size(&self, decl: ir::DeclId) -> usize {
        self.registry.source_callable_input_size(decl)
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
        self.contexts
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
        self.declaration(id)?;
        self.source_schemas.schema(id, &self.budget)
    }
    pub fn source_region_env(&self, id: ir::DeclId) -> Result<Arc<ir::SourceRegionEnv>, String> {
        self.declaration(id)?;
        self.source_schemas
            .region_env(id, self.catalog.declaration(id).is_some(), &self.budget)
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
        let ty = self
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
            let mut state = self.scratch();
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
            .find(|(_, s)| !s.region_args.is_empty() || !s.args.is_empty())
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
        let structural = match caller {
            ir::FactsCtx::Validated(facts) => {
                if !facts.budget().same_authority(budget)
                    || !Arc::ptr_eq(facts.world(), &self.trait_world()?)
                {
                    return Err("nominal interning changed validated caller authority".into());
                }
                ir::FactsCtx::Universal(facts.env())
            }
            other => other,
        };
        for argument in args {
            caller.validate_ty_metadata_with_budget(argument, budget)?;
        }
        // Metadata validation can ask SchemaAdapter to authenticate a nested
        // nominal through this same state. Intern structurally while locked,
        // then replay the original caller's authority after releasing the guard.
        let out = {
            self.scratch()
                .intern(&self.registry, decl, args, structural, budget, 0)?
        };
        caller.validate_ty_metadata_with_budget(&out, budget)?;
        Ok(out)
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
        let mut environment_bytes = 0usize;
        for info in &self.registry.declarations {
            self.budget.charge_work(1)?;
            let params = self.source_schemas.signature_arity(info.id, &self.budget)?;
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
            match self.nominals.applications.get(&(*id, Vec::new())) {
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
        if let Some(decl) = decl {
            let info = self.declaration(decl)?;
            env.params = generic_bindings(&self.registry, &info.env, &self.budget)?;
            env.self_ty = self.contexts.get(&decl).and_then(|c| c.self_ty.clone());
            if let Some(ty) = &env.self_ty {
                env.params.insert(GenericBindingKey::SelfType, ty.clone());
            }
            if info.env.is_generic() {
                env.param_env = Some(info.env.clone());
            }
        }
        if let Some(decl) = decl {
            env.facts = Some(self.callable_facts(decl)?);
        }
        Ok(env)
    }
}
impl ir::SubstitutionCx for PlanCtx {
    fn nominal_application(&self, id: ir::NominalAppId) -> Result<ir::NominalAppInfo, String> {
        let ty = self
            .nominals
            .by_slot
            .get(&id.index.source_slot().map_err(|error| {
                format!(
                    "{error}; nominal_application id={id:?}, requested_at={}",
                    std::panic::Location::caller()
                )
            })?)
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
            // Schema substitution may carry raw associated projections
            // (`Option<<C as Iter>::Item>`); they normalize later through
            // the trait world, so only metadata is checked here.
            caller.validate_ty_metadata_with_budget(arg, budget)?;
        }
        let mut state = self.scratch();
        let declaration = request.source_decl();
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
                    // Membership is resolved below against the instantiated
                    // principal and its canonical supertrait applications.
                    registry
                        .names
                        .check(binding.name)
                        .map_err(|e| e.to_string())?;
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
        if args.len() < expected
            && let Ok(source) = registry.source_trait(declaration)
        {
            while args.len() < expected && source.self_defaults.contains(&args.len()) {
                budget.retain_bytes(std::mem::size_of::<ir::Ty>())?;
                args.push(owner.clone());
            }
        }
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
                    let (binding_owner, item) = self
                        .inherited_associated_owner(
                            registry,
                            trait_ref.clone(),
                            binding.name,
                            caller,
                            budget,
                            &mut Vec::new(),
                        )?
                        .ok_or("extra or unknown associated type item")?;
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
                            trait_ref: binding_owner,
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
        let ty = state.source_ty(registry, &row.subject, bindings, caller, budget, 0, true)?;
        if row.maybe_unsized {
            // `T: ?Sized` drops the implicit `T: Sized` row.
            let sized = *registry
                .traits
                .get(&TraitLookupKey::Builtin(BuiltinTrait::Sized))
                .ok_or("missing canonical Sized identity")?;
            budget.charge_work(out.len())?;
            out.retain(|predicate| {
                !matches!(predicate, ir::Predicate::Implements(goal)
                    if goal.declaration == sized && goal.self_ty == ty)
            });
        }
        for path in &row.traits {
            budget.charge_work(1)?;
            if out.len() >= tr::MAX_PREDICATES {
                return Err("predicate environment limit exceeded".into());
            }
            budget.retain_bytes(256)?;
            let application =
                state.source_trait_application(registry, path, &ty, bindings, caller, budget)?;
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
/// Structural payload equality of two definitions for one canonical
/// application. Nested nominals compare by identity and are validated by
/// their own walk step.
fn same_fields(a: &[ir::FieldDef], b: &[ir::FieldDef]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(a, b)| {
            a.name == b.name
                && a.ty == b.ty
                && a.visibility == b.visibility
                && a.lifetime_bound == b.lifetime_bound
        })
}
fn same_struct_def(a: &ir::StructDef, b: &ir::StructDef) -> bool {
    a.id == b.id
        && a.source_decl == b.source_decl
        && a.type_args == b.type_args
        && a.name == b.name
        && a.module == b.module
        && a.visibility == b.visibility
        && same_fields(&a.fields, &b.fields)
}
fn same_enum_def(a: &ir::EnumDef, b: &ir::EnumDef) -> bool {
    a.id == b.id
        && a.source_decl == b.source_decl
        && a.type_args == b.type_args
        && a.name == b.name
        && a.module == b.module
        && a.visibility == b.visibility
        && a.variants.len() == b.variants.len()
        && a.variants.iter().zip(b.variants.iter()).all(|(a, b)| {
            a.name == b.name && a.shape == b.shape && same_fields(&a.fields, &b.fields)
        })
}
#[derive(Debug)]
struct SchemaAdapter {
    program: Arc<Program>,
    catalog: Arc<tr::CoreCatalog>,
    registry: Arc<DeclRegistry>,
    nominal_schemas: Arc<HashMap<ir::DeclId, Arc<NominalSchema>>>,
    nominals: Arc<CanonicalNominals>,
    contexts: Arc<HashMap<ir::DeclId, CallableContext>>,
    source_schemas: Arc<SourceSchemas>,
    budget: ir::PlanBudget,
}
impl SchemaAdapter {
    fn scratch(&self) -> PlanState {
        PlanState::scratch(&self.nominal_schemas, &self.nominals)
    }
    fn raw_plan(&self) -> Result<PlanCtx, String> {
        self.budget.retain_bytes(128)?;
        Ok(PlanCtx {
            program: self.program.clone(),
            catalog: self.catalog.clone(),
            registry: self.registry.clone(),
            nominal_schemas: self.nominal_schemas.clone(),
            nominals: self.nominals.clone(),
            contexts: self.contexts.clone(),
            source_schemas: self.source_schemas.clone(),
            budget: self.budget.clone(),
            world: None,
            validated: Arc::new(HashMap::new()),
            shared_env: None,
            module_items: Arc::from([]),
        })
    }
    fn error(&self, e: String) -> tr::FactsError {
        tr::FactsError::from_message(&self.budget, &e)
    }
}
impl tr::SchemaCx for SchemaAdapter {
    fn replay_packed_reserve_authority(
        &self,
        authority: &PackedReserveAuthority,
        declaration: ir::DeclId,
    ) -> Result<(), tr::FactsError> {
        authority
            .replay(&self.program, &self.registry, declaration, &self.budget)
            .map_err(|error| self.error(error))
    }

    fn closure_recipe(
        &self,
        provider: ir::DeclId,
    ) -> Result<Option<ir::ClosureRecipeId>, tr::FactsError> {
        self.source_closure_owner_site(provider)?;
        Ok(self.registry.closure_recipes.get(&provider).cloned())
    }
    fn closure_trait_kind(
        &self,
        declaration: ir::DeclId,
    ) -> Result<Option<ir::ClosureKind>, tr::FactsError> {
        self.source_closure_trait_kind(declaration)
    }
    fn closure_owner_site(
        &self,
        provider: ir::DeclId,
    ) -> Result<Option<(ir::DeclId, u32)>, tr::FactsError> {
        self.source_closure_owner_site(provider)
    }
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
        if let Some(info) = self
            .source_schemas
            .declaration_info(id, &self.budget)
            .map_err(|e| self.error(e))?
        {
            return Ok(info);
        }
        // Nonfunction source declarations retain the authoritative registry
        // path and their original fixed Positioned contract.
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
            region_contract_mode: tr::RegionContractMode::Positioned,
        })
    }
    fn declaration_schema(&self, id: ir::DeclId) -> Result<tr::GenericSchema, tr::FactsError> {
        self.registry.declaration(id).map_err(|e| self.error(e))?;
        self.source_schemas
            .schema(id, &self.budget)
            .map_err(|e| self.error(e))
    }
    fn associated_constant_schema(
        &self,
        declaration: ir::DeclId,
    ) -> Result<tr::AssociatedConstHeader, tr::FactsError> {
        self.raw_plan()
            .map_err(|e| self.error(e))?
            .associated_constant_schema(declaration, &self.budget)
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
        self.source_schemas
            .associated_rhs(implementation, associated, &self.budget)
            .map_err(|e| self.error(e))
    }
    fn associated_type_source(
        &self,
        implementation: ir::DeclId,
        associated: ir::DeclId,
    ) -> Result<Arc<tr::AssociatedTypeSource>, tr::FactsError> {
        self.associated_type_rhs(implementation, associated)?;
        self.source_schemas
            .associated_source(implementation, associated, &self.budget)
            .map_err(|e| self.error(e))
    }
    fn instantiate_constant_metadata(
        &self,
        declaration: ir::DeclId,
        args: &[ir::Ty],
        caller: &ir::GenericEnv,
        budget: &ir::PlanBudget,
    ) -> Result<(Option<ir::Ty>, ir::Ty, Arc<[ir::Predicate]>), tr::FactsError> {
        self.substitute_constant_metadata(declaration, args, caller, budget)
    }
    fn instantiate_constant_type(
        &self,
        declaration: ir::DeclId,
        args: &[ir::Ty],
        caller: &ir::GenericEnv,
        budget: &ir::PlanBudget,
    ) -> Result<ir::Ty, tr::FactsError> {
        if !budget.same_authority(&self.budget) {
            return Err(tr::FactsError::invalid(
                budget,
                "constant schema budget authority mismatch",
            ));
        }
        let mut plan = self.raw_plan().map_err(|e| self.error(e))?;
        let header = plan
            .associated_constant_schema(declaration, budget)
            .map_err(|e| self.error(e))?;
        ir::substitute_schema_metadata_ty(
            &header.ty,
            &header.env,
            args,
            ir::FactsCtx::Universal(caller),
            &mut plan,
            self,
            budget,
        )
        .map_err(|e| self.error(e))
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
    fn associated_type_predicates(
        &self,
        associated: ir::DeclId,
    ) -> Result<Arc<[ir::Predicate]>, tr::FactsError> {
        self.associated_type_bounds(associated)?;
        self.registry
            .associated_type_bounds
            .get(&associated)
            .map(|bounds| bounds.predicates.clone())
            .ok_or_else(|| self.error("missing source associated bound applications".into()))
    }
    fn source_region_env(
        &self,
        id: ir::DeclId,
    ) -> Result<Arc<ir::SourceRegionEnv>, tr::FactsError> {
        self.registry.declaration(id).map_err(|e| self.error(e))?;
        self.source_schemas
            .region_env(id, self.catalog.declaration(id).is_some(), &self.budget)
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
        let mut projection_actuals = false;
        for argument in args {
            projection_actuals |= ir::contains_associated_types(argument, budget)
                .map_err(|error| self.error(error))?;
        }
        let mut sub = |ty: &ir::Ty| {
            if projection_actuals {
                ir::substitute_schema_metadata_ty(
                    ty,
                    &schema.env,
                    args,
                    facts,
                    &mut cx,
                    self,
                    budget,
                )
            } else if schema_only_item {
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
    /// Remint `original`'s source declaration with normalized actuals
    /// through the canonical interner, never reusing the original identity.
    fn reapply_nominal(
        &self,
        original: &ir::Ty,
        arguments: &[ir::Ty],
        env: &ir::GenericEnv,
        budget: &ir::PlanBudget,
    ) -> Result<ir::Ty, tr::FactsError> {
        if !budget.same_authority(&self.budget) {
            return Err(tr::FactsError::invalid(
                budget,
                "schema budget authority mismatch",
            ));
        }
        tr::SchemaCx::validate_nominal(self, original, env, budget)?;
        let declaration = match original {
            ir::Ty::Struct(t) => t.def().source_decl,
            ir::Ty::Enum(t) => t.def().source_decl,
            _ => {
                return Err(tr::FactsError::invalid(
                    budget,
                    "nominal reapplication requires a nominal application",
                ));
            }
        };
        let caller = if env.total_arity() == 0 {
            ir::FactsCtx::Concrete
        } else {
            ir::FactsCtx::Universal(env)
        };
        self.scratch()
            .intern(&self.registry, declaration, arguments, caller, budget, 0)
            .map_err(|e| self.error(e))
    }
    fn nominal_declaration_schema(
        &self,
        declaration: ir::DeclId,
    ) -> Result<tr::NominalDeclarationSchema, tr::FactsError> {
        self.nominal_source_schema(declaration)
            .map_err(|error| self.error(error))
    }
    fn validate_nominal(
        &self,
        ty: &ir::Ty,
        env: &ir::GenericEnv,
        budget: &ir::PlanBudget,
    ) -> Result<(), tr::FactsError> {
        self.validate_nominal_tree(ty, env, budget, true)
    }
    fn validate_nominal_node(
        &self,
        ty: &ir::Ty,
        env: &ir::GenericEnv,
        budget: &ir::PlanBudget,
    ) -> Result<(), tr::FactsError> {
        self.validate_nominal_tree(ty, env, budget, false)
    }
}
impl SchemaAdapter {
    fn validate_nominal_tree(
        &self,
        ty: &ir::Ty,
        env: &ir::GenericEnv,
        budget: &ir::PlanBudget,
        descend: bool,
    ) -> Result<(), tr::FactsError> {
        if !budget.same_authority(&self.budget) {
            return Err(tr::FactsError::invalid(
                budget,
                "schema budget authority mismatch",
            ));
        }
        #[allow(clippy::too_many_arguments)]
        fn walk(
            cx: &SchemaAdapter,
            ty: &ir::Ty,
            env: &ir::GenericEnv,
            b: &ir::PlanBudget,
            depth: usize,
            memo: &mut HashMap<(ir::NominalAppId, usize), usize>,
            state: &mut PlanState,
            descend: bool,
        ) -> Result<usize, String> {
            b.check_depth(depth)?;
            b.charge_work(1)?;
            if ty.is_nominal_back_edge() {
                // Cycle-closing edge: its definition is the strong outer one.
                return Ok(0);
            }
            let nominal = match ty {
                ir::Ty::Struct(t) => Some((t.def().id, t.def() as *const _ as usize)),
                ir::Ty::Enum(t) => Some((t.def().id, t.def() as *const _ as usize)),
                _ => None,
            };
            if let Some((id, pointer)) = nominal {
                // Only an already authenticated immutable definition allocation
                // can skip replay. Equal IDs on a different payload still replay.
                if let Some(height) = memo.get(&(id, pointer)) {
                    b.check_depth(depth + height)?;
                    return Ok(*height);
                }
                if let ir::NominalIndex::ParentLocal { owner, .. } = id.index {
                    // Minted by the owning parent from its own closure actuals;
                    // children (actuals, fields) are walked below.
                    if id.snapshot != cx.registry.declarations[0].id.snapshot
                        || owner.snapshot != id.snapshot
                        || cx.registry.declaration(owner)?.kind != ir::DeclKind::Function
                    {
                        return Err("foreign parent-local nominal".into());
                    }
                } else if matches!(id.index, ir::NominalIndex::GraphRealization(_)) {
                    let capability = match ty {
                        ir::Ty::Struct(n) => n.realization(),
                        ir::Ty::Enum(n) => n.realization(),
                        _ => None,
                    }
                    .ok_or("graph nominal lacks its typed read capability")?;
                    capability.check_definition(ty).map_err(|e| e.to_string())?;
                    if id.snapshot != cx.registry.declarations[0].id.snapshot {
                        return Err("foreign graph nominal source snapshot".into());
                    }
                } else {
                    // The slot is a canonical digest of the definition's own
                    // resolved declaration and ordered actuals: recompute it.
                    let (decl, args) = match ty {
                        ir::Ty::Struct(t) => (t.def().source_decl, t.def().type_args.to_vec()),
                        ir::Ty::Enum(t) => (t.def().source_decl, t.def().type_args.to_vec()),
                        _ => return Err("noncanonical nominal application".into()),
                    };
                    if !matches!(
                        cx.registry.declaration(decl)?.kind,
                        ir::DeclKind::Struct | ir::DeclKind::Enum
                    ) || nominal_schema::canonical_slot(&(decl, args))
                        != id.index.source_slot()?
                        || id.snapshot != cx.registry.declarations[0].id.snapshot
                        || decl.snapshot != id.snapshot
                    {
                        return Err("noncanonical nominal application".into());
                    }
                    // The definition payload must equal the canonical one:
                    // the immutable sealed base by pointer, otherwise a
                    // deterministic re-interning in per-call scratch.
                    let canonical = match cx.nominals.by_slot.get(&id.index.source_slot()?) {
                        Some(base) => base.clone(),
                        None => {
                            let caller = if env.total_arity() == 0 {
                                ir::FactsCtx::Concrete
                            } else {
                                ir::FactsCtx::Universal(env)
                            };
                            let (decl, args) = match ty {
                                ir::Ty::Struct(t) => {
                                    (t.def().source_decl, t.def().type_args.to_vec())
                                }
                                ir::Ty::Enum(t) => {
                                    (t.def().source_decl, t.def().type_args.to_vec())
                                }
                                _ => unreachable!(),
                            };
                            state.intern(&cx.registry, decl, &args, caller, b, 0)?
                        }
                    };
                    let same = match (&canonical, ty) {
                        (ir::Ty::Struct(c), ir::Ty::Struct(t)) => {
                            c.def() as *const _ as usize == pointer
                                || same_struct_def(c.def(), t.def())
                        }
                        (ir::Ty::Enum(c), ir::Ty::Enum(t)) => {
                            c.def() as *const _ as usize == pointer
                                || same_enum_def(c.def(), t.def())
                        }
                        _ => false,
                    };
                    if !same {
                        return Err("noncanonical nominal application".into());
                    }
                }
            }
            if !descend {
                return Ok(0);
            }
            let mut height = 0usize;
            let mut child = |t: &ir::Ty| -> Result<(), String> {
                height = height.max(walk(cx, t, env, b, depth + 1, memo, state, true)? + 1);
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
                    if *len > ir::MAX_ARRAY_TYPE_LEN {
                        return Err("array length limit exceeded".into());
                    }
                    child(element)?;
                }
                ir::Ty::Ref { referent, .. } => child(referent)?,
                _ => {}
            }
            if let Some((id, pointer)) = nominal {
                b.retain_bytes(128)?;
                memo.insert((id, pointer), height);
            }
            Ok(height)
        }
        // Raw schema types may hold associated projections inside nominal
        // actuals (`Option<Self::Item>`); `walk` validates them structurally.
        let mut inner = ty;
        let mut layers = 0usize;
        while let ir::Ty::Ref { referent, .. } = inner {
            layers += 1;
            if layers > 128 {
                return Err(self.error("reference depth resource limit exceeded".into()));
            }
            inner = referent;
        }
        walk(
            self,
            inner,
            env,
            budget,
            layers,
            &mut HashMap::new(),
            &mut self.scratch(),
            descend,
        )
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
    if ty.is_nominal_back_edge() {
        return Ok(false);
    }
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
        (ir::Ty::Dyn(a), ir::Ty::Dyn(b)) => {
            if a.principal != b.principal
                || a.args.len() != b.args.len()
                || a.associated_types.len() != b.associated_types.len()
            {
                return Ok(false);
            }
            for (left, right) in a.args.iter().zip(b.args.iter()) {
                if !metered_type_equal(left, right, budget, depth + 1)? {
                    return Ok(false);
                }
            }
            for ((left_item, left), (right_item, right)) in
                a.associated_types.iter().zip(b.associated_types.iter())
            {
                if left_item != right_item || !metered_type_equal(left, right, budget, depth + 1)? {
                    return Ok(false);
                }
            }
            Ok(true)
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
        (
            ir::Ty::RawPtr {
                kind: a,
                pointee: x,
            },
            ir::Ty::RawPtr {
                kind: b,
                pointee: y,
            },
        ) if a == b => match_impl_type(x, y, env, bindings, budget, depth + 1),
        (ir::Ty::Slice { element: x }, ir::Ty::Slice { element: y }) => {
            match_impl_type(x, y, env, bindings, budget, depth + 1)
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
        ) if a == b => match_impl_type(x, y, env, bindings, budget, depth + 1),
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
    /// Item-local coherence: this callable's inherent overlap against same-named
    /// inherent items of its nominal, and its C ABI contract.
    pub fn validate_item_coherence(
        &self,
        index: usize,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        self.check_budget(budget)?;
        let id = self.callable_decl(index)?;
        let a = self.declaration(id)?;
        if let Some(ao) = &a.path.associated
            && ao.trait_decl.is_none()
        {
            for b in self
                .registry
                .inherent_peers
                .get(&ao.nominal)
                .map(|v| v.as_slice())
                .unwrap_or_default()
            {
                budget.charge_work(1)?;
                if *b == id {
                    continue;
                }
                let b = self.declaration(*b)?;
                let Some(bo) = &b.path.associated else {
                    continue;
                };
                if !self.registry.source_items_equal(a.path.item, b.path.item)? {
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
        if !self.program.c_abi.functions.is_empty() {
            self.c_abi_for(id)?;
        }
        Ok(())
    }
    /// Trait implemented by a source impl header, if any.
    pub fn impl_trait(&self, implementation: ir::DeclId) -> Result<Option<ir::DeclId>, String> {
        Ok(self.trait_world()?.impl_trait(implementation))
    }
    /// Coherence of one implementation against its trait's other impls
    /// (`peers`, the trait's TRAIT_IMPLS completeness fact).
    pub fn validate_impl_coherence(
        &self,
        implementation: ir::DeclId,
        peers: &[ir::DeclId],
    ) -> Result<(), String> {
        let world = self.trait_world()?;
        world
            .check_impl_coherence(implementation, peers)
            .map_err(|e| e.into_legacy_diagnostic(&self.budget))
    }
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
        if let Some(selected) = self.select_method_with_arguments(
            receiver,
            name,
            caller,
            (module, budget),
            |_, _, _| {
                Ok(method_argument_selection::MethodArguments::Ready(
                    std::borrow::Cow::Borrowed(args),
                ))
            },
            |_, _| Ok(None),
        )? {
            return Ok(selected);
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
            if scope.relaxed_sized(ir::ParamId {
                owner: scope.binder(),
                index: index as u16,
            }) {
                continue;
            }
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
    if ty.is_nominal_back_edge() {
        return Ok(0);
    }
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
        ir::Ty::Ref { referent, .. } => child(referent)?,
        ir::Ty::Slice { element } => child(element)?,
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
        Type::Opaque(opaque) => {
            opaque.validate_names(names)?;
            for ty in opaque.types() {
                collect_source_params(names, ty, used, budget, depth + 1)?;
            }
        }
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
mod dynamic_objects;
mod nominal_declarations;
mod nominal_realization;
mod nominal_schema;
mod nominal_source_shapes;
/// One immutable per-body view with a shared canonical application service.
mod type_check;
use type_check::*;
pub(in crate::compiler) use type_check::{
    NominalCallableOutput, NominalDeclarationFields, NominalDeclarationRecipe, NominalDemand,
    NominalPreparation, NominalPreparationInputs, NominalShape,
};
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
                    region_args: Vec::new(),
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
            nominal_schemas: plan.nominal_schemas.clone(),
            nominals: plan.nominals.clone(),
            contexts: plan.contexts.clone(),
            source_schemas: plan.source_schemas.clone(),
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
        assert_eq!(
            plan.source_schemas
                .assignment_rows(&plan.budget)
                .unwrap()
                .len(),
            2
        );
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
            nominal_schemas: plan.nominal_schemas.clone(),
            nominals: plan.nominals.clone(),
            contexts: plan.contexts.clone(),
            source_schemas: plan.source_schemas.clone(),
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
        assert!(
            plan.nominal_source_shape(function, &original.result)
                .is_ok()
        );
        assert!(plan.nominal_source_shape(function, &transplanted).is_err());
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
    fn trait_use_projection_retains_its_original_source_occurrence() {
        let program = parse("trait A{type Out;type Other;}trait Sink<T>{}fn f<T:A,U:Sink<T::Out>>(x:T::Other){}fn main(){}").unwrap();
        let plan = PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap();
        let declaration = plan.function_decl(0).unwrap();
        let env = plan.environment(Some(declaration)).unwrap();
        let facts = plan.callable_facts(declaration).unwrap();
        let owner = ir::Ty::Param(ir::ParamId {
            owner: plan.param_env(declaration).unwrap().binder(),
            index: 1,
        });
        let path = &plan.program.functions[0].bounds[1].traits[0];
        plan.resolve_trait_use(
            &owner,
            path,
            &env,
            ir::FactsCtx::Validated(&facts),
            &plan.budget,
        )
        .unwrap();
        let Type::Projection(donor) = &plan.program.functions[0].params[0].1 else {
            panic!("missing donor projection")
        };
        let mut forged = path.clone();
        let TraitPath::Source(forged_path) = &mut forged else {
            panic!("missing source path")
        };
        let Type::Projection(projection) = &mut forged_path.segments.last_mut().unwrap().args[0]
        else {
            panic!("missing bound projection")
        };
        match (projection.as_mut(), donor.as_ref()) {
            (
                SourceProjection::Qualified { item, .. },
                SourceProjection::Qualified { item: other, .. },
            ) => *item = *other,
            _ => panic!("missing qualified projection"),
        }
        assert!(
            plan.resolve_trait_use(
                &owner,
                &forged,
                &env,
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
        let Type::Param(parameter) = &bound.subject else {
            panic!("parameter bound")
        };
        let owner = bindings
            .get(&plan.registry.parameter_key(*parameter).unwrap())
            .unwrap();
        let application = plan
            .scratch()
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
        let Type::Param(parameter) = &bound.subject else {
            panic!("parameter bound")
        };
        let owner = bindings
            .get(&plan.registry.parameter_key(*parameter).unwrap())
            .unwrap();
        let mut path = bound.traits[0].clone();
        let TraitPath::Source(source) = &mut path else {
            panic!("source bound")
        };
        let segment = source.segments.last_mut().unwrap();
        segment.args[0] = plan.program.functions[1].params[0].1.clone();
        segment.associated_types[1].name = *parameter;
        let error = plan
            .scratch()
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
    fn const_fn_bodies_are_ordinary_checked_bodies() {
        for declaration in [
            "const fn f(x:u8)->u8{let y=x;y}",
            "const fn f(x:u8)->u8{{x}}",
            "const fn f(x:u8,y:u8)->u8{x+y}",
        ] {
            let source = format!("{declaration}fn main(){{}}");
            check(&parse(&source).unwrap()).unwrap();
        }
        // Type errors in unused const fn bodies still reject.
        assert!(
            super::super::compile("const fn f(x:u8)->u64{x}fn main(){}".into(), 1, true).is_err()
        );
    }

    #[test]
    fn const_execution_contexts_and_associated_qualifiers_remain_unsupported() {
        for source in [
            "const fn id(x:u8)->u8{x}fn main(){let x=[0u8;id(1u8)];}",
            "const fn id(x:u8)->u8{x}fn main(){let x:[u8;id(1u8)]=[0u8];}",
            "fn f<const N:usize>(){}fn main(){}",
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
        ] {
            assert_eq!(
                parse_syntax_unprepared(source).unwrap_err().kind,
                SourceErrorKind::Unsupported,
                "{source}"
            );
        }
        super::parser_tests::stock_native_run(
            "fn main(){let x=[14u8;1+2];let y:[u8;1+2]=[14u8;3];println!(\"{} {}\",x[2]*3,y[2]*3);}",
        );
        let mismatch = "fn main(){let x:[u8;1+2]=[0u8];}";
        let stock =
            super::parser_tests::stock_compile(mismatch, std::path::Path::new("unused"), true);
        assert!(!stock.status.success());
        assert!(String::from_utf8_lossy(&stock.stderr).contains("E0308"));
        for workers in [1, 8] {
            assert!(super::super::compile(mismatch.into(), workers, true).is_err());
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
        // items use this seam too.
        assert!(parse_syntax_unprepared(effectful).is_ok());
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
mod ord_max_tests;

#[cfg(test)]
mod prepared_location_tests {
    use super::*;

    fn parser_parts(tokens: NativeParserInput) -> (NativeTokenBuffer, NameStoreBuilder) {
        let (tokens, session, session_cap) = tokens.into_parts();
        let names =
            NameStoreBuilder::new(session, session_cap, super::super::token::Edition::E2024)
                .unwrap();
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
        let tokens = lex("1+2]").unwrap();
        let (tokens, mut names) = parser_parts(tokens);
        let mut parser = Parser::new(&tokens, &mut names).unwrap();
        parser.use_expansions = 7;
        parser.use_expansion_bytes = 19;
        assert_eq!(parser.array_number("array length").unwrap(), 3);
        assert_eq!(parser.token().bytes, 3..4);
        assert_eq!(parser.token().text, "]");
        assert_eq!(parser.use_expansions, 7);
        assert_eq!(parser.use_expansion_bytes, 19);
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
            format!("<text>:{}", error),
        );
    }

    #[test]
    fn prepared_diagnostics_preserve_syntax_unsupported_and_limit_categories() {
        use super::super::source::{SourceErrorKind, SourceOptions, prepare_text};
        let syntax = parse_syntax_unprepared("fn f() { let = 1; }")
            .err()
            .unwrap();
        assert_eq!(syntax.kind, SourceErrorKind::Syntax);
        for never in [
            "fn f(x: !) {}fn main(){}",
            "fn f(x: &!) {}fn main(){}",
            "struct S{x:!}fn main(){}",
            "struct S<T>{x:T}fn f(x:S<!>){}fn main(){}",
            "fn f()->&'static !{loop{}}fn main(){}",
            "fn f()->(!){loop{}}fn main(){}",
            "fn main(){let f=||->(!){loop{}};}",
            "fn f(){let x:! = loop{};}fn main(){}",
        ] {
            let syntax = parse_syntax_unprepared(never).unwrap_err();
            assert_eq!(syntax.kind, SourceErrorKind::Syntax);
            assert!(syntax.message.contains("E0658"));
            let stock =
                super::parser_tests::stock_compile(never, std::path::Path::new("unused"), true);
            assert!(!stock.status.success());
            assert!(String::from_utf8_lossy(&stock.stderr).contains("E0658"));
            for workers in [1, 8] {
                for check in [true, false] {
                    let failure = super::super::compile(never.into(), workers, check)
                        .err()
                        .expect("unstable never type accepted");
                    assert!(failure.to_string().contains("E0658"), "{failure}");
                }
            }
        }
        super::parser_tests::stock_native_run(
            "fn never()->!{loop{}}fn main(){println!(\"{}\",42);}",
        );
        let closure = "fn main(){let f=||->!{loop{}};}";
        parse_syntax_unprepared(closure).expect("stored closure preserves direct never return");
        let immediate = "fn main(){let f=(||->!{loop{}})();}";
        let stock =
            super::parser_tests::stock_compile(immediate, std::path::Path::new("unused"), true);
        assert!(
            stock.status.success(),
            "{}",
            String::from_utf8_lossy(&stock.stderr)
        );
        let raw = parse_syntax_unprepared(immediate).unwrap_err();
        assert_eq!(raw.kind, SourceErrorKind::Unsupported);
        assert!(
            raw.message
                .contains("non-scalar closure return types are unsupported")
        );
        // Stored closures preserve their direct never return through preparation.
        super::parser_tests::stock_native_run(
            "fn main(){let f=||->!{loop{}};println!(\"{}\",42);}",
        );
        let unsupported = parse_syntax_unprepared("fn f<const N:usize>() {}")
            .err()
            .unwrap();
        assert_eq!(unsupported.kind, SourceErrorKind::Unsupported);
        {
            let text = "async fn unused() {}fn main(){}";
            let stock =
                super::parser_tests::stock_compile(text, std::path::Path::new("unused"), true);
            assert!(
                stock.status.success(),
                "{}",
                String::from_utf8_lossy(&stock.stderr)
            );
            assert_eq!(
                parse_syntax_unprepared(text).err().unwrap().kind,
                SourceErrorKind::Unsupported,
                "{text}",
            );
        }
        super::parser_tests::stock_native_run(
            "fn main(){let value=r\"raw\";println!(\"{}\",value);}",
        );
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
        embedded_function: None,
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
    // Selection uses the realized signature, but final call publication must
    // authenticate and retain the original declaration-owned signature too.
    original_signature: Option<ir::Signature>,
    mode: ReceiverKind,
    depth: usize,
    adjustment: ReceiverAdjustment,
}

impl crate::engine::Fact for ParsedProgram {}

impl crate::engine::Fact for PlanCtx {}

impl crate::engine::Fact for Program {}
