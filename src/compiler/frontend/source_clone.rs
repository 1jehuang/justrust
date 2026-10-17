//! Conservative preflight for the allocations performed by source AST clones.
//! Name occurrences, module IDs and visibility restrictions are inline handles.
//! Immutable path/name/module arena payloads remain shared, not copied here.
use super::*;

pub(super) fn clone_text(text: &str, budget: &ir::PlanBudget) -> Result<(), String> {
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

pub(super) fn clone_vector<T>(values: &[T], budget: &ir::PlanBudget) -> Result<(), String> {
    budget.charge_work(
        values
            .len()
            .checked_add(1)
            .ok_or("source collection work overflow")?,
    )?;
    budget.retain_bytes(
        std::mem::size_of_val(values)
            .checked_mul(2)
            .and_then(|bytes| bytes.checked_add(128))
            .ok_or("source collection bytes overflow")?,
    )
}

pub(super) fn clone_names(names: &[AstIdent], budget: &ir::PlanBudget) -> Result<(), String> {
    clone_vector(names, budget)
}

fn clone_ast_path(_path: &AstPath, budget: &ir::PlanBudget) -> Result<(), String> {
    // Only the Arc handle is cloned. The actual shared payload keeps its original
    // expansion-budget lease until its last owner, independently of this clone.
    budget.charge_work(1)
}

fn clone_trait_path(path: &TraitPath, budget: &ir::PlanBudget) -> Result<(), String> {
    match path {
        TraitPath::Source(path) => clone_ast_path(path, budget),
        TraitPath::Builtin(_) => budget.charge_work(1),
    }
}

pub(super) fn clone_regions(regions: &RegionDecls, budget: &ir::PlanBudget) -> Result<(), String> {
    clone_names(&regions.params, budget)?;
    clone_vector(&regions.outlives, budget)
}

pub(super) fn clone_bounds(bounds: &[TypeBound], budget: &ir::PlanBudget) -> Result<(), String> {
    clone_vector(bounds, budget)?;
    for bound in bounds {
        clone_vector(&bound.traits, budget)?;
        for path in &bound.traits {
            clone_trait_path(path, budget)?;
        }
    }
    Ok(())
}

pub(super) fn clone_parameters(
    params: &[(BindingName, Type)],
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    clone_vector(params, budget)?;
    for (_, ty) in params {
        walk_depth(DepthNode::Type(ty), Some(budget))?;
    }
    Ok(())
}

pub(super) fn clone_fields(fields: &[FieldDecl], budget: &ir::PlanBudget) -> Result<(), String> {
    clone_vector(fields, budget)?;
    for field in fields {
        clone_visibility(&field.visibility, budget)?;
        walk_depth(DepthNode::Type(&field.ty), Some(budget))?;
    }
    Ok(())
}

pub(super) fn clone_selector(
    _selector: &FieldSelector,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    // Every selector, including a named occurrence, is now an inline value.
    budget.charge_work(1)
}

pub(super) fn clone_place(place: &Place, budget: &ir::PlanBudget) -> Result<(), String> {
    clone_vector(&place.fields, budget)?;
    for field in &place.fields {
        clone_selector(field, budget)?;
    }
    Ok(())
}

pub(super) fn clone_path(path: &GenericPath, budget: &ir::PlanBudget) -> Result<(), String> {
    clone_vector(&path.segments, budget)?;
    for segment in &path.segments {
        clone_vector(&segment.args, budget)?;
    }
    Ok(())
}

pub(super) fn preflight_clone_node(
    node: &DepthNode<'_>,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    budget.charge_work(1)?;
    budget.retain_bytes(1024)?;
    match node {
        DepthNode::Type(ty) => match ty {
            Type::Named(path) => clone_ast_path(path, budget)?,
            Type::Tuple(fields) => clone_vector(fields, budget)?,
            Type::Apply(path) => clone_path(path, budget)?,
            _ => {}
        },
        DepthNode::Pattern(pattern) => match pattern {
            Pattern::Apply { path, .. } => clone_path(path, budget)?,
            Pattern::Path(path) => clone_ast_path(path, budget)?,
            Pattern::Struct { name, fields, .. } => {
                clone_ast_path(name, budget)?;
                clone_vector(fields, budget)?;
            }
            Pattern::Variant { name, fields, .. } => {
                clone_ast_path(name, budget)?;
                clone_vector(fields, budget)?;
                for (field, _) in fields {
                    clone_selector(field, budget)?;
                }
            }
            Pattern::Tuple(fields) => clone_vector(fields, budget)?,
            Pattern::TupleRest(fields) => {
                clone_vector(&fields.prefix, budget)?;
                clone_vector(&fields.suffix, budget)?;
            }
            Pattern::VariantRest { name, fields } => {
                clone_ast_path(name, budget)?;
                clone_vector(&fields.prefix, budget)?;
                clone_vector(&fields.suffix, budget)?;
            }
            _ => {}
        },
        DepthNode::Expr(expr) => match expr {
            Expr::UnexpandedMacro(_) => {
                return Err("unexpanded macro syntax cannot enter active AST cloning".into());
            }
            Expr::Var(path) => clone_ast_path(path, budget)?,
            Expr::Println(text) => clone_text(text, budget)?,
            Expr::Float(literal) => clone_text(&literal.digits, budget)?,
            Expr::Call { name, args } | Expr::TupleStruct { name, fields: args } => {
                clone_ast_path(name, budget)?;
                clone_vector(args, budget)?;
            }
            Expr::MethodCall {
                type_args, args, ..
            }
            | Expr::AssociatedCall {
                type_args, args, ..
            } => {
                clone_vector(type_args, budget)?;
                clone_vector(args, budget)?;
            }
            Expr::TraitCall {
                trait_path, args, ..
            } => {
                clone_trait_path(trait_path, budget)?;
                clone_vector(args, budget)?;
            }
            Expr::Apply { path, .. } => clone_path(path, budget)?,
            Expr::Borrow { place, .. } => clone_place(place, budget)?,
            Expr::Array(fields) | Expr::Tuple(fields) => clone_vector(fields, budget)?,
            Expr::EnumVariant { name, fields, .. } => {
                clone_ast_path(name, budget)?;
                clone_vector(fields, budget)?;
                for (field, _) in fields {
                    clone_selector(field, budget)?;
                }
            }
            Expr::Struct { name, fields } => {
                clone_ast_path(name, budget)?;
                clone_vector(fields, budget)?;
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

pub(super) fn preflight_program_clone(
    program: &Program,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    // Program's Arc fields are included inline. Their name/module/symbol/path
    // payloads, including an optional lexical-scope arena, are shared under
    // their original leases, never deep-cloned here.
    budget.retain_bytes(std::mem::size_of::<Program>())?;
    clone_vector(&program.functions, budget)?;
    for function in &program.functions {
        preflight_function_clone(function, budget)?;
    }
    clone_vector(&program.impls, budget)?;
    for implementation in &program.impls {
        if let Some(path) = &implementation.trait_path {
            clone_trait_path(path, budget)?;
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
        clone_ast_path(&declaration.name, budget)?;
        clone_names(&declaration.type_params, budget)?;
        clone_fields(&declaration.fields, budget)?;
    }
    clone_vector(&program.enums, budget)?;
    for declaration in &program.enums {
        clone_visibility(&declaration.visibility, budget)?;
        clone_ast_path(&declaration.name, budget)?;
        clone_names(&declaration.type_params, budget)?;
        clone_vector(&declaration.variants, budget)?;
        for variant in &declaration.variants {
            clone_fields(&variant.fields, budget)?;
        }
    }
    clone_vector(&program.traits, budget)?;
    for declaration in &program.traits {
        clone_visibility(&declaration.visibility, budget)?;
        clone_ast_path(&declaration.name, budget)?;
        clone_vector(&declaration.supertraits, budget)?;
        for path in &declaration.supertraits {
            clone_trait_path(path, budget)?;
        }
        clone_vector(&declaration.items, budget)?;
        for item in &declaration.items {
            let head = &item.signature;
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
    for (_, path) in &program.trait_imports {
        clone_trait_path(path, budget)?;
    }
    Ok(())
}

pub(super) fn preflight_function_clone(
    function: &Function,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    clone_visibility(&function.visibility, budget)?;
    clone_ast_path(&function.name, budget)?;
    clone_names(&function.type_params, budget)?;
    clone_bounds(&function.bounds, budget)?;
    clone_regions(&function.regions, budget)?;
    clone_parameters(&function.params, budget)?;
    walk_depth(DepthNode::Type(&function.result), Some(budget))?;
    walk_depth(DepthNode::Block(&function.body), Some(budget))
}

pub(super) fn clone_visibility(
    _visibility: &ir::Visibility,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    // Both visibility alternatives contain only inline checked module handles.
    budget.charge_work(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::expansion_budget::ExpansionBudget;
    use crate::compiler::module_identity::ModuleArenaBuilder;
    use std::path::Path;

    #[test]
    fn visibility_clone_is_inline_and_does_not_retain_scope_text() {
        let modules = ModuleArenaBuilder::for_catalog(ExpansionBudget::default())
            .unwrap()
            .freeze()
            .unwrap();
        let budget = ir::PlanBudget::new(ir::PlanLimits {
            bytes: 0,
            ..Default::default()
        });
        clone_visibility(&ir::Visibility::Public, &budget).unwrap();
        clone_visibility(&ir::Visibility::Restricted(modules.core_root()), &budget).unwrap();
        assert_eq!(budget.usage_snapshot().bytes, 0);
        assert_eq!(budget.usage_snapshot().work, 2);
    }

    #[test]
    fn large_identifier_path_clone_shares_its_existing_arena_lease() {
        let source_budget = ExpansionBudget::default();
        let mut builder =
            ProgramBuilder::new(Path::new("clone-control.rs"), "", &source_budget).unwrap();
        let identifier = builder.ident(&"a".repeat(16_384)).unwrap();
        let path = AstPath::new(builder.names(), PathAnchor::Crate, &[identifier]).unwrap();
        let program = builder.finish().unwrap();
        let before = source_budget.snapshot();
        let budget = ir::PlanBudget::new(ir::PlanLimits {
            bytes: 0,
            ..Default::default()
        });
        clone_ast_path(&path, &budget).unwrap();
        let copied = path.clone();
        assert_eq!(path.key(Namespace::Type), copied.key(Namespace::Type));
        assert_eq!(budget.usage_snapshot().bytes, 0);
        assert_eq!(budget.usage_snapshot().work, 1);
        assert_eq!(
            source_budget.snapshot().retained_bytes,
            before.retained_bytes
        );
        assert_eq!(
            program.names.spelling(copied.segments()[0]).unwrap().len(),
            16_384
        );
    }

    #[test]
    fn program_clone_keeps_exact_name_module_and_closed_symbol_owners() {
        let source_budget = ExpansionBudget::default();
        let mut program = ProgramBuilder::new(Path::new("clone-owners.rs"), "", &source_budget)
            .unwrap()
            .finish()
            .unwrap();
        let scopes =
            crate::compiler::semantic_scope::ScopeArenaBuilder::for_names(program.names.as_ref())
                .unwrap()
                .freeze()
                .unwrap();
        program.scopes = Some(scopes);
        let before = source_budget.snapshot();
        let budget = ir::PlanBudget::new(ir::PlanLimits::default());
        preflight_program_clone(&program, &budget).unwrap();
        let copied = program.clone();
        assert!(Arc::ptr_eq(&program.names, &copied.names));
        assert!(Arc::ptr_eq(&program.modules, &copied.modules));
        assert!(Arc::ptr_eq(&program.symbols, &copied.symbols));
        assert!(Arc::ptr_eq(
            program.scopes.as_ref().unwrap(),
            copied.scopes.as_ref().unwrap()
        ));
        assert_eq!(
            source_budget.snapshot().retained_bytes,
            before.retained_bytes
        );
        assert!(budget.usage_snapshot().bytes >= std::mem::size_of::<Program>());
    }

    #[test]
    fn tuple_constructor_argument_storage_is_admitted_before_clone() {
        let source_budget = ExpansionBudget::default();
        let mut builder =
            ProgramBuilder::new(Path::new("tuple-clone.rs"), "", &source_budget).unwrap();
        let name = builder.path("crate::Pair").unwrap();
        let expr = Expr::TupleStruct {
            name,
            fields: vec![Expr::Bool(true), Expr::Bool(false)],
        };
        let Expr::TupleStruct { fields, .. } = &expr else {
            unreachable!()
        };
        let required = std::mem::size_of_val(fields.as_slice()) * 2 + 128;
        let budget = ir::PlanBudget::new(ir::PlanLimits {
            bytes: 1024 + required - 1,
            ..Default::default()
        });
        assert!(preflight_clone_node(&DepthNode::Expr(&expr), &budget).is_err());
        let failure = budget.first_resource_failure().unwrap();
        assert_eq!(failure.kind, ir::ResourceBudgetKind::Bytes);
        assert_eq!(failure.used, 1024);
        assert_eq!(failure.requested, required);
        assert_eq!(budget.usage_snapshot().bytes, 1024);
        assert!(!failure.arithmetic_overflow);
    }

    #[test]
    fn float_literal_text_heap_is_admitted_before_clone() {
        let digits = "1".repeat(4096);
        let required = digits.len() + 32;
        let literal = Expr::Float(FloatLiteral {
            digits,
            suffix: None,
        });
        let budget = ir::PlanBudget::new(ir::PlanLimits {
            bytes: 1024 + required - 1,
            ..Default::default()
        });
        assert!(preflight_clone_node(&DepthNode::Expr(&literal), &budget).is_err());
        let failure = budget.first_resource_failure().unwrap();
        assert_eq!(failure.kind, ir::ResourceBudgetKind::Bytes);
        assert_eq!(failure.requested, required);
        assert_eq!(failure.used, 1024);
        assert_eq!(budget.usage_snapshot().bytes, 1024);
        assert!(!failure.arithmetic_overflow);
    }

    #[test]
    fn positional_rest_clone_precharges_both_written_child_vectors() {
        let fields = PositionalRest {
            prefix: vec![Pattern::Wildcard],
            suffix: vec![Pattern::Wildcard, Pattern::Wildcard],
        };
        let prefix_bytes = std::mem::size_of_val(fields.prefix.as_slice()) * 2 + 128;
        let suffix_bytes = std::mem::size_of_val(fields.suffix.as_slice()) * 2 + 128;
        let pattern = Pattern::TupleRest(fields);
        let budget = ir::PlanBudget::new(ir::PlanLimits {
            bytes: 1024 + prefix_bytes + suffix_bytes - 1,
            ..Default::default()
        });
        assert!(preflight_clone_node(&DepthNode::Pattern(&pattern), &budget).is_err());
        let failure = budget.first_resource_failure().unwrap();
        assert_eq!(failure.kind, ir::ResourceBudgetKind::Bytes);
        assert_eq!(failure.used, 1024 + prefix_bytes);
        assert_eq!(failure.requested, suffix_bytes);
        assert_eq!(budget.usage_snapshot().bytes, 1024 + prefix_bytes);
        assert!(!failure.arithmetic_overflow);
    }

    #[test]
    fn lexical_scope_storage_survives_program_clone_until_last_owner() {
        let source_budget = ExpansionBudget::default();
        let mut program = ProgramBuilder::new(Path::new("clone-scope.rs"), "", &source_budget)
            .unwrap()
            .finish()
            .unwrap();
        let baseline = source_budget.snapshot().retained_bytes;
        let mut scopes =
            crate::compiler::semantic_scope::ScopeArenaBuilder::for_names(program.names.as_ref())
                .unwrap();
        scopes.function_root(program.names.as_ref()).unwrap();
        program.scopes = Some(scopes.freeze().unwrap());
        let retained = source_budget.snapshot().retained_bytes;
        assert!(retained > baseline);
        let copied = program.clone();
        assert_eq!(source_budget.snapshot().retained_bytes, retained);
        drop(program);
        assert_eq!(source_budget.snapshot().retained_bytes, retained);
        copied
            .scopes
            .as_ref()
            .unwrap()
            .check_names(copied.names.as_ref())
            .unwrap();
        drop(copied);
        assert_eq!(source_budget.snapshot().retained_bytes, 0);
    }
}
