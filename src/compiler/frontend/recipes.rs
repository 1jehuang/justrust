//! One fixed recipe event stream feeds both resource counting and real checked IR.
use super::*;
use crate::compiler::ownership::RecipePlanCounts;

#[derive(Clone, Copy)]
enum Event {
    Read,
    Load,
    Borrow { index: usize, array: bool },
    Clone,
    Product { array: bool, len: usize },
}
fn walk(
    descriptor: tr::RecipeDescriptor,
    mut emit: impl FnMut(Event) -> Result<(), String>,
) -> Result<(), String> {
    match descriptor {
        tr::RecipeDescriptor::CloneTuple { arity } => {
            for index in 0..usize::from(arity) {
                emit(Event::Borrow {
                    index,
                    array: false,
                })?;
                emit(Event::Clone)?;
            }
            emit(Event::Product {
                array: false,
                len: usize::from(arity),
            })
        }
        tr::RecipeDescriptor::CloneArray { len } => {
            for index in 0..usize::from(len) {
                emit(Event::Borrow { index, array: true })?;
                emit(Event::Clone)?;
            }
            emit(Event::Product {
                array: true,
                len: usize::from(len),
            })
        }
        tr::RecipeDescriptor::CloneNever => Ok(()),
        _ => {
            emit(Event::Read)?;
            emit(Event::Load)
        }
    }
}
pub(super) fn counts(
    descriptor: tr::RecipeDescriptor,
    budget: &ir::PlanBudget,
) -> Result<RecipePlanCounts, String> {
    let components = match descriptor {
        tr::RecipeDescriptor::CloneTuple { arity } => usize::from(arity),
        tr::RecipeDescriptor::CloneArray { len } => usize::from(len),
        _ => 0,
    };
    let mut c = RecipePlanCounts {
        binders: match descriptor {
            tr::RecipeDescriptor::CloneTuple { arity } => usize::from(arity),
            tr::RecipeDescriptor::CloneArray { .. } | tr::RecipeDescriptor::CloneSharedRef => 1,
            _ => 0,
        },
        locals: 2,
        blocks: 1,
        formal_origins: if descriptor == tr::RecipeDescriptor::CloneSharedRef {
            3
        } else {
            1 + components
        },
        carrier_slots: if descriptor == tr::RecipeDescriptor::CloneSharedRef {
            5
        } else {
            1 + 2 * components
        },
        path_nodes: match descriptor {
            tr::RecipeDescriptor::CloneTuple { arity } => usize::from(arity) + 4,
            tr::RecipeDescriptor::CloneArray { len } => usize::from(len) + 4,
            _ => 4,
        },
        ..RecipePlanCounts::default()
    };
    walk(descriptor, |event| {
        budget.charge_work(1)?;
        c.expressions = c
            .expressions
            .checked_add(1)
            .ok_or("recipe expression count overflow")?;
        c.locals = c
            .locals
            .checked_add(1)
            .ok_or("recipe local count overflow")?;
        c.path_nodes = c
            .path_nodes
            .checked_add(2)
            .ok_or("recipe path count overflow")?;
        match event {
            Event::Borrow { .. } => {
                c.loan_sites += 1;
                c.carrier_slots += 2;
                c.target_projection_steps += 2;
            }
            Event::Clone => {
                c.blocks += 1;
                c.carrier_slots += 1;
            }
            Event::Read => {
                c.carrier_slots += if descriptor == tr::RecipeDescriptor::CloneSharedRef {
                    3
                } else {
                    1
                }
            }
            Event::Load => {
                if descriptor == tr::RecipeDescriptor::CloneSharedRef {
                    c.carrier_slots += 2;
                }
            }
            Event::Product { len, .. } => c.carrier_slots += len,
        }
        Ok(())
    })?;
    // Derived footprints saturate: an oversized shape is an inventoried
    // intrinsic limit (reported only if that Clone recipe is ever demanded),
    // never a hard error for programs that merely mention the array type.
    c.points = c
        .expressions
        .saturating_mul(5)
        .saturating_add(c.blocks.saturating_mul(3).saturating_add(8));
    let origins = c.loan_sites.saturating_add(c.formal_origins);
    c.origin_memberships = c
        .points
        .saturating_mul(c.carrier_slots)
        .saturating_mul(origins);
    c.parent_memberships = origins.saturating_mul(origins);
    c.lifetime_dependency_memberships = c.parent_memberships;
    c.target_memberships = c.origin_memberships;
    c.target_allocations = c.origin_memberships;
    c.target_projection_steps = c.target_allocations.saturating_mul(2);
    c.target_authority_entries = c.target_allocations.saturating_mul(origins);
    c.target_overhead_words = c.origin_memberships.saturating_mul(3);
    c.operations = c.expressions.saturating_mul(32).saturating_add(64);
    c.ir_retained_bytes = c
        .expressions
        .saturating_mul(1024)
        .saturating_add(c.locals.saturating_mul(512).saturating_add(4096));
    c.cfg_retained_bytes = c.points.saturating_mul(1024);
    c.construction_peak_bytes = c.ir_retained_bytes;
    c.verification_scratch_bytes = c.cfg_retained_bytes;
    c.work_upper_bound = c
        .operations
        .saturating_add(c.origin_memberships.saturating_mul(8));
    Ok(c)
}

pub(super) fn inventory(
    program: &Program,
    budget: &ir::PlanBudget,
) -> Result<tr::RecipeShapeInventory, String> {
    // Inventory walks source before publishing recipes, including cold panic sites.
    program.validate_name_handles()?;
    inventory_validated(program, budget)
}

/// `inventory` for a program whose name handles the caller has already
/// validated in this same computation (the borrow is immutable, so the
/// result cannot change in between).
pub(super) fn inventory_validated(
    program: &Program,
    budget: &ir::PlanBudget,
) -> Result<tr::RecipeShapeInventory, String> {
    let mut descriptors = Vec::new();
    fn add(
        ds: &mut Vec<tr::RecipeDescriptor>,
        d: tr::RecipeDescriptor,
        b: &ir::PlanBudget,
    ) -> Result<(), String> {
        if matches!(d, tr::RecipeDescriptor::CloneTuple { arity: 0 }) {
            return add(ds, tr::RecipeDescriptor::CloneUnit, b);
        }
        for old in ds.iter() {
            b.charge_work(1)?;
            if *old == d {
                return Ok(());
            }
        }
        b.retain_bytes(4 * std::mem::size_of::<tr::RecipeDescriptor>() + 64)?;
        ds.push(d);
        Ok(())
    }
    for int in [
        ir::IntTy::I8,
        ir::IntTy::I16,
        ir::IntTy::I32,
        ir::IntTy::I64,
        ir::IntTy::Isize,
        ir::IntTy::U8,
        ir::IntTy::U16,
        ir::IntTy::U32,
        ir::IntTy::U64,
        ir::IntTy::Usize,
        ir::IntTy::I128,
        ir::IntTy::U128,
    ] {
        add(
            &mut descriptors,
            tr::RecipeDescriptor::CloneInt(int),
            budget,
        )?;
    }
    for d in [
        tr::RecipeDescriptor::CloneFloat(ir::FloatTy::F32),
        tr::RecipeDescriptor::CloneFloat(ir::FloatTy::F64),
        tr::RecipeDescriptor::CloneBool,
        tr::RecipeDescriptor::CloneChar,
        tr::RecipeDescriptor::CloneUnit,
        tr::RecipeDescriptor::CloneSharedRef,
        tr::RecipeDescriptor::CloneNever,
    ] {
        add(&mut descriptors, d, budget)?;
    }
    fn ty(
        t: &Type,
        ds: &mut Vec<tr::RecipeDescriptor>,
        b: &ir::PlanBudget,
        depth: usize,
    ) -> Result<(), String> {
        b.check_depth(depth)?;
        b.charge_work(1)?;
        match t {
            Type::Tuple(ts) => {
                add(
                    ds,
                    tr::RecipeDescriptor::CloneTuple {
                        arity: u16::try_from(ts.len())
                            .map_err(|_| "tuple recipe arity overflow")?,
                    },
                    b,
                )?;
                for t in ts {
                    ty(t, ds, b, depth + 1)?;
                }
            }
            Type::Array(t, n) => {
                // Clone recipes expand per element. Longer arrays get no
                // seeded recipe: cloning one is an explicit unsupported
                // boundary at the Clone obligation, not a parse failure.
                if let Ok(len) = u16::try_from(*n) {
                    add(ds, tr::RecipeDescriptor::CloneArray { len }, b)?;
                }
                ty(t, ds, b, depth + 1)?;
            }
            Type::Ref { referent, .. } => ty(referent, ds, b, depth + 1)?,
            Type::Projection(projection) => match projection.as_ref() {
                SourceProjection::Qualified {
                    self_ty,
                    trait_path: path,
                    ..
                } => {
                    ty(self_ty, ds, b, depth + 1)?;
                    trait_path(path, ds, b, depth + 1)?;
                }
                SourceProjection::SelfItem { .. } => {}
            },
            // The slice itself has no Clone recipe, but every written element
            // shape remains part of the source inventory, even in a boundary.
            Type::Slice(element) => ty(element, ds, b, depth + 1)?,
            Type::Apply(path) => generic_path(path, ds, b, depth)?,
            _ => {}
        }
        Ok(())
    }
    fn generic_path(
        path: &GenericPath,
        ds: &mut Vec<tr::RecipeDescriptor>,
        b: &ir::PlanBudget,
        depth: usize,
    ) -> Result<(), String> {
        b.check_depth(depth)?;
        b.charge_work(path.segments.len())?;
        for segment in &path.segments {
            b.charge_work(segment.args.len())?;
            b.charge_work(segment.associated_types.len())?;
            for actual in &segment.args {
                ty(actual, ds, b, depth + 1)?;
            }
            for binding in &segment.associated_types {
                ty(&binding.value, ds, b, depth + 1)?;
            }
        }
        Ok(())
    }
    fn trait_path(
        path: &TraitPath,
        ds: &mut Vec<tr::RecipeDescriptor>,
        b: &ir::PlanBudget,
        depth: usize,
    ) -> Result<(), String> {
        b.check_depth(depth)?;
        b.charge_work(1)?;
        if let TraitPath::Source(path) = path {
            generic_path(path, ds, b, depth)?;
        }
        Ok(())
    }
    fn bounds(
        values: &[TypeBound],
        ds: &mut Vec<tr::RecipeDescriptor>,
        b: &ir::PlanBudget,
        depth: usize,
    ) -> Result<(), String> {
        b.check_depth(depth)?;
        b.charge_work(values.len())?;
        for bound in values {
            b.charge_work(bound.traits.len())?;
            for path in &bound.traits {
                trait_path(path, ds, b, depth + 1)?;
            }
        }
        Ok(())
    }
    fn pattern(
        value: &Pattern,
        ds: &mut Vec<tr::RecipeDescriptor>,
        b: &ir::PlanBudget,
        depth: usize,
    ) -> Result<(), String> {
        b.check_depth(depth)?;
        b.charge_work(1)?;
        match value {
            Pattern::Apply {
                path,
                pattern: child,
            } => {
                generic_path(path, ds, b, depth)?;
                pattern(child, ds, b, depth + 1)?;
            }
            Pattern::Ref { pattern: child, .. } | Pattern::At { pattern: child, .. } => {
                pattern(child, ds, b, depth + 1)?
            }
            Pattern::Tuple(children)
            | Pattern::Array(children)
            | Pattern::Alternatives(children) => {
                b.charge_work(children.len())?;
                for child in children {
                    pattern(child, ds, b, depth + 1)?;
                }
            }
            Pattern::TupleRest(children)
            | Pattern::ArrayRest(children)
            | Pattern::VariantRest {
                fields: children, ..
            } => {
                b.charge_work(children.prefix.len())?;
                b.charge_work(children.suffix.len())?;
                for child in children.prefix.iter().chain(&children.suffix) {
                    pattern(child, ds, b, depth + 1)?;
                }
            }
            Pattern::Variant { fields, .. } => {
                b.charge_work(fields.len())?;
                for (_, child) in fields {
                    pattern(child, ds, b, depth + 1)?;
                }
            }
            Pattern::Struct { fields, .. } => {
                b.charge_work(fields.len())?;
                for (_, child) in fields {
                    pattern(child, ds, b, depth + 1)?;
                }
            }
            Pattern::Path(_)
            | Pattern::Bool(_)
            | Pattern::Int { .. }
            | Pattern::Char(_)
            | Pattern::Range { .. }
            | Pattern::Wildcard
            | Pattern::Binding { .. } => {}
        }
        Ok(())
    }
    fn block(
        value: &Block,
        ds: &mut Vec<tr::RecipeDescriptor>,
        b: &ir::PlanBudget,
        d: usize,
    ) -> Result<(), String> {
        b.check_depth(d)?;
        for s in &value.statements {
            b.charge_work(1)?;
            match s {
                Stmt::Declare { ty: t, .. } => ty(t, ds, b, d + 1)?,
                Stmt::Let { ty: t, value, .. } => {
                    ty(t, ds, b, d + 1)?;
                    expr(value, ds, b, d + 1)?;
                }
                Stmt::LetPattern {
                    pattern: child,
                    ty: t,
                    value,
                } => {
                    pattern(child, ds, b, d + 1)?;
                    ty(t, ds, b, d + 1)?;
                    expr(value, ds, b, d + 1)?;
                }
                Stmt::LetElse {
                    pattern: child,
                    ty: t,
                    value,
                    else_block,
                } => {
                    pattern(child, ds, b, d + 1)?;
                    ty(t, ds, b, d + 1)?;
                    expr(value, ds, b, d + 1)?;
                    block(else_block, ds, b, d + 1)?;
                }
                Stmt::AssignNestedIndex { target, value, .. } => {
                    expr(target, ds, b, d + 1)?;
                    expr(value, ds, b, d + 1)?;
                }
                Stmt::AssignDeref {
                    carrier,
                    index,
                    value,
                    ..
                } => {
                    expr(carrier, ds, b, d + 1)?;
                    if let Some(index) = index {
                        expr(index, ds, b, d + 1)?;
                    }
                    expr(value, ds, b, d + 1)?;
                }
                Stmt::AssignIndex { index, value, .. }
                | Stmt::AssignIndexOp { index, value, .. } => {
                    expr(index, ds, b, d + 1)?;
                    expr(value, ds, b, d + 1)?;
                }
                Stmt::Assign { value, .. }
                | Stmt::AssignOp { value, .. }
                | Stmt::AssignPlace { value, .. }
                | Stmt::Expr(value)
                | Stmt::ImplicitExpr(value)
                | Stmt::Return(Some(value)) => expr(value, ds, b, d + 1)?,
                Stmt::While {
                    condition, body, ..
                } => {
                    expr(condition, ds, b, d + 1)?;
                    block(body, ds, b, d + 1)?;
                }
                Stmt::WhileLet {
                    pattern: child,
                    scrutinee,
                    body,
                    ..
                } => {
                    pattern(child, ds, b, d + 1)?;
                    expr(scrutinee, ds, b, d + 1)?;
                    block(body, ds, b, d + 1)?;
                }
                Stmt::For {
                    pattern: child,
                    source,
                    body,
                    ..
                } => {
                    pattern(child, ds, b, d + 1)?;
                    match &source.iterable {
                        ForIterable::Range { start, end, .. } => {
                            expr(start, ds, b, d + 1)?;
                            expr(end, ds, b, d + 1)?;
                        }
                        ForIterable::Value(value) => expr(value, ds, b, d + 1)?,
                    }
                    block(body, ds, b, d + 1)?;
                }
                Stmt::Return(None) => {}
            }
        }
        if let Some(e) = &value.tail {
            expr(e, ds, b, d + 1)?;
        }
        Ok(())
    }
    fn expr(
        e: &Expr,
        ds: &mut Vec<tr::RecipeDescriptor>,
        b: &ir::PlanBudget,
        d: usize,
    ) -> Result<(), String> {
        b.check_depth(d)?;
        b.charge_work(1)?;
        match e {
            Expr::NativeSyscall(arguments)
            | Expr::RawSlice { arguments, .. }
            | Expr::Format { arguments, .. } => {
                for argument in arguments {
                    expr(argument, ds, b, d + 1)?;
                }
            }
            Expr::FormatInto(source) => {
                for argument in source.arguments.iter().chain(source.rest.iter().flatten()) {
                    expr(argument, ds, b, d + 1)?;
                }
            }
            Expr::Tuple(es) => {
                add(
                    ds,
                    tr::RecipeDescriptor::CloneTuple {
                        arity: u16::try_from(es.len())
                            .map_err(|_| "tuple recipe arity overflow")?,
                    },
                    b,
                )?;
                for e in es {
                    expr(e, ds, b, d + 1)?;
                }
            }
            Expr::Array(es) => {
                if let Ok(len) = u16::try_from(es.len()) {
                    add(ds, tr::RecipeDescriptor::CloneArray { len }, b)?;
                }
                for e in es {
                    expr(e, ds, b, d + 1)?;
                }
            }
            Expr::Repeat { value, len } => {
                if let Ok(len) = u16::try_from(*len) {
                    add(ds, tr::RecipeDescriptor::CloneArray { len }, b)?;
                }
                expr(value, ds, b, d + 1)?;
            }
            Expr::Apply { path, value } => {
                generic_path(path, ds, b, d)?;
                expr(value, ds, b, d + 1)?;
            }
            Expr::Invoke { receiver, args } => {
                expr(receiver, ds, b, d + 1)?;
                for argument in args {
                    expr(argument, ds, b, d + 1)?;
                }
            }
            Expr::MethodCall {
                receiver,
                type_args,
                args,
                ..
            } => {
                expr(receiver, ds, b, d + 1)?;
                for t in type_args {
                    ty(t, ds, b, d + 1)?;
                }
                for e in args {
                    expr(e, ds, b, d + 1)?;
                }
            }
            Expr::AssociatedCall {
                owner,
                type_args,
                args,
                ..
            } => {
                ty(owner, ds, b, d + 1)?;
                for t in type_args {
                    ty(t, ds, b, d + 1)?;
                }
                for e in args {
                    expr(e, ds, b, d + 1)?;
                }
            }
            Expr::TraitCall {
                owner,
                trait_path: path,
                type_args,
                args,
                ..
            } => {
                ty(owner, ds, b, d + 1)?;
                trait_path(path, ds, b, d + 1)?;
                for actual in type_args {
                    ty(actual, ds, b, d + 1)?;
                }
                for e in args {
                    expr(e, ds, b, d + 1)?;
                }
            }
            Expr::Call { args, .. } | Expr::TupleStruct { fields: args, .. } => {
                for e in args {
                    expr(e, ds, b, d + 1)?;
                }
            }
            Expr::Cast { value, target } => {
                ty(target, ds, b, d + 1)?;
                expr(value, ds, b, d + 1)?;
            }
            Expr::TypeAlignment(target) => ty(target, ds, b, d + 1)?,
            Expr::SliceDataPointer(value)
            | Expr::Deref { value }
            | Expr::BorrowValue { value, .. }
            | Expr::Unary { value, .. }
            | Expr::Field { base: value, .. }
            | Expr::Index { base: value, .. } => expr(value, ds, b, d + 1)?,
            Expr::RangeIndex {
                base, start, end, ..
            } => {
                expr(base, ds, b, d + 1)?;
                for bound in [start, end].into_iter().flatten() {
                    expr(bound, ds, b, d + 1)?;
                }
            }
            Expr::DynamicIndex { base, index, .. }
            | Expr::Binary {
                left: base,
                right: index,
                ..
            } => {
                expr(base, ds, b, d + 1)?;
                expr(index, ds, b, d + 1)?;
            }
            Expr::Struct { fields, .. } => {
                for (_, e) in fields {
                    expr(e, ds, b, d + 1)?;
                }
            }
            Expr::EnumVariant { fields, .. } => {
                for (_, e) in fields {
                    expr(e, ds, b, d + 1)?;
                }
            }
            Expr::Match { scrutinee, arms } => {
                expr(scrutinee, ds, b, d + 1)?;
                for a in arms {
                    pattern(&a.pattern, ds, b, d + 1)?;
                    if let Some(g) = &a.guard {
                        expr(g, ds, b, d + 1)?;
                    }
                    expr(&a.body, ds, b, d + 1)?;
                }
            }
            Expr::IfLet {
                pattern: child,
                scrutinee,
                then_block,
                else_branch,
            } => {
                pattern(child, ds, b, d + 1)?;
                expr(scrutinee, ds, b, d + 1)?;
                block(then_block, ds, b, d + 1)?;
                if let Some(branch) = else_branch {
                    expr(branch, ds, b, d + 1)?;
                }
            }
            Expr::If {
                condition,
                then_block,
                else_block,
            } => {
                expr(condition, ds, b, d + 1)?;
                block(then_block, ds, b, d + 1)?;
                block(else_block, ds, b, d + 1)?;
            }
            Expr::Block(v)
            | Expr::Unsafe(v)
            | Expr::Loop { body: v, .. }
            | Expr::LabeledBlock { body: v, .. } => block(v, ds, b, d + 1)?,
            Expr::Break { value: Some(v), .. } | Expr::Try(v) => expr(v, ds, b, d + 1)?,
            _ => {}
        }
        Ok(())
    }
    // Local functions share the same scan implementation without source mutation.
    fn function(
        f: &Function,
        ds: &mut Vec<tr::RecipeDescriptor>,
        b: &ir::PlanBudget,
    ) -> Result<(), String> {
        b.charge_work(f.type_params.len())?;
        bounds(&f.bounds, ds, b, 0)?;
        for (_, t) in &f.params {
            ty(t, ds, b, 0)?;
        }
        ty(&f.result, ds, b, 0)?;
        block(&f.body, ds, b, 0)
    }
    for f in &program.functions {
        function(f, &mut descriptors, budget)?;
    }
    for i in &program.impls {
        budget.charge_work(i.type_params.len())?;
        bounds(&i.bounds, &mut descriptors, budget, 0)?;
        if let Some(path) = &i.trait_path {
            trait_path(path, &mut descriptors, budget, 0)?;
        }
        ty(&i.self_ty, &mut descriptors, budget, 0)?;
        budget.charge_work(i.associated_types.len())?;
        for associated in &i.associated_types {
            ty(&associated.value, &mut descriptors, budget, 0)?;
        }
        for m in &i.items {
            function(&m.function, &mut descriptors, budget)?;
        }
    }
    for s in &program.structs {
        for f in &s.fields {
            ty(&f.ty, &mut descriptors, budget, 0)?;
        }
    }
    for e in &program.enums {
        for v in &e.variants {
            for f in &v.fields {
                ty(&f.ty, &mut descriptors, budget, 0)?;
            }
        }
    }
    for t in &program.traits {
        budget.charge_work(t.type_params.len())?;
        bounds(&t.bounds, &mut descriptors, budget, 0)?;
        for path in &t.supertraits {
            trait_path(path, &mut descriptors, budget, 0)?;
        }
        budget.charge_work(t.associated_types.len())?;
        for associated in &t.associated_types {
            budget.charge_work(associated.bounds.len())?;
            for path in &associated.bounds {
                trait_path(path, &mut descriptors, budget, 0)?;
            }
        }
        for i in &t.items {
            budget.charge_work(i.signature.type_params.len())?;
            bounds(&i.signature.bounds, &mut descriptors, budget, 0)?;
            for (_, t) in &i.signature.params {
                ty(t, &mut descriptors, budget, 0)?;
            }
            ty(&i.signature.result, &mut descriptors, budget, 0)?;
            if let Some(default) = &i.default {
                function(default, &mut descriptors, budget)?;
            }
        }
    }
    for (_, path) in &program.trait_imports {
        trait_path(path, &mut descriptors, budget, 0)?;
    }
    let mut inventory = tr::RecipeShapeInventory::new();
    for descriptor in descriptors {
        let counts = counts(descriptor, budget)?;
        inventory
            .record(descriptor, &counts, budget)
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
    }
    Ok(inventory)
}

pub(super) fn checked_body(
    plan: &PlanCtx,
    target: &ir::CallTarget,
    facts: &tr::ValidatedFacts,
    descriptor: tr::RecipeDescriptor,
    budget: &ir::PlanBudget,
) -> Result<ir::CheckedFunction, String> {
    let signature = plan.function_schema(target.declaration)?.signature;
    let footprint = counts(descriptor, budget)?;
    budget.retain_bytes(footprint.ir_retained_bytes)?;
    let mut f = ir::CheckedFunction {
        runtime_sites: Default::default(),
        name: "clone".into(),
        ref_contract: signature.ref_contract.clone(),
        params: vec![ir::LocalId(0)],
        result: signature.result.clone(),
        locals: vec![ir::LocalDecl {
            name: "self".into(),
            ty: signature.params[0].clone(),
            mutable: false,
        }],
        exprs: Vec::new(),
        call_evidence: Arc::from([]),
        body_substitution: None,
        body: ir::TypedBlock {
            statements: Vec::new(),
            tail: None,
        },
    };
    let mut children = Vec::new();
    let mut call_evidence = Vec::new();
    let mut last = None;
    let mut emit = |kind, ty| -> Result<ir::ExprId, String> {
        budget.charge_work(1)?;
        budget.retain_bytes(1024)?;
        let id = ir::ExprId(f.exprs.len());
        f.exprs.push(ir::TypedExpr {
            kind,
            ty,
            completion: ir::Completion::Returns,
        });
        Ok(id)
    };
    let mut child_ty = None;
    walk(descriptor, |event| {
        match event {
            Event::Read => {
                last = Some(emit(
                    ir::ExprKind::Local(ir::LocalId(0)),
                    signature.params[0].clone(),
                )?)
            }
            Event::Load => {
                last = Some(emit(
                    ir::ExprKind::Deref {
                        value: last.ok_or("recipe load input missing")?,
                    },
                    signature.result.clone(),
                )?)
            }
            Event::Borrow { index, array } => {
                let t = match &signature.result {
                    ir::Ty::Tuple(ts) => ts.get(index).cloned(),
                    ir::Ty::Array { element, .. } => Some(element.as_ref().clone()),
                    _ => None,
                }
                .ok_or("recipe child type missing")?;
                let projection = vec![
                    ir::Projection::Deref,
                    if array {
                        ir::Projection::ConstantIndex(index)
                    } else {
                        ir::Projection::Field(index)
                    },
                ];
                let ty = ir::Ty::Ref {
                    kind: ir::BorrowKind::Shared,
                    referent: Arc::new(t.clone()),
                };
                last = Some(emit(
                    ir::ExprKind::Borrow {
                        kind: ir::BorrowKind::Shared,
                        place: ir::Place {
                            local: ir::LocalId(0),
                            projection,
                        },
                        timing: ir::BorrowTiming::Immediate,
                    },
                    ty,
                )?);
                child_ty = Some(t);
            }
            Event::Clone => {
                let t = child_ty.take().ok_or("recipe child missing")?;
                let call = ir::TraitCall {
                    trait_ref: ir::TraitRef {
                        declaration: plan.catalog.id(tr::CatalogItem::Clone),
                        self_ty: t.clone(),
                        args: Arc::from([]),
                    },
                    item: plan.catalog.id(tr::CatalogItem::CloneItem),
                    method_args: Arc::from([]),
                };
                let sig = facts
                    .service()
                    .instantiate_trait_signature(&call, facts)
                    .map_err(|e| e.into_legacy_diagnostic(budget))?;
                let argument = last.ok_or("recipe child borrow missing")?;
                let expectation = match sig.params.first() {
                    Some(ir::Ty::Ref { kind, .. }) => {
                        ir::CallExpectation::FirmReference { kind: *kind }
                    }
                    _ => ir::CallExpectation::NoFirmReference,
                };
                let id = emit(
                    ir::ExprKind::Call {
                        target: ir::Callee::TraitItem(call),
                        name: "clone".into(),
                        args: vec![argument],
                        signature: sig,
                    },
                    t,
                )?;
                budget.retain_bytes(
                    4 * std::mem::size_of::<ir::CallEvidence>()
                        + 4 * std::mem::size_of::<ir::CallArgumentTrace>()
                        + 128,
                )?;
                call_evidence.push(ir::CallEvidence {
                    call: id,
                    arguments: Arc::from([ir::CallArgumentTrace {
                        argument: 0,
                        value: argument,
                        expectation,
                    }]),
                    type_actual_regions: Arc::from([]),
                });
                children.push(id);
                last = Some(id);
            }
            Event::Product { array, len } => {
                if children.len() != len {
                    return Err("recipe child count mismatch".into());
                }
                last = Some(emit(
                    if array {
                        ir::ExprKind::Array(std::mem::take(&mut children))
                    } else {
                        ir::ExprKind::Tuple(std::mem::take(&mut children))
                    },
                    signature.result.clone(),
                )?);
            }
        }
        Ok(())
    })?;
    f.body.tail = last;
    f.call_evidence = call_evidence.into();
    let mut actual_loans = 0usize;
    let mut actual_calls = 0usize;
    let mut actual_carriers = ir::carrier_locations(&f.result, budget)?.len();
    for local in &f.locals {
        budget.charge_work(1)?;
        actual_carriers += ir::carrier_locations(&local.ty, budget)?.len();
    }
    for expression in &f.exprs {
        budget.charge_work(1)?;
        actual_carriers += ir::carrier_locations(&expression.ty, budget)?.len();
        actual_loans += usize::from(matches!(expression.kind, ir::ExprKind::Borrow { .. }));
        actual_calls += usize::from(matches!(expression.kind, ir::ExprKind::Call { .. }));
    }
    if f.exprs.len() != footprint.expressions
        || f.locals.len() + f.exprs.len() + 1 != footprint.locals
        || actual_loans != footprint.loan_sites
        || actual_calls + 1 != footprint.blocks
        || actual_carriers != footprint.carrier_slots
    {
        return Err("recipe footprint/emission mismatch".into());
    }
    let mut cx = plan.clone();
    ir::validate_call_evidence(&f, facts, budget)?;
    ir::canonicalize_checked_calls(&f, facts, &mut cx, budget)
        .map_err(|e| e.into_legacy_diagnostic(budget))
}

#[cfg(test)]
mod wide_tests {
    use super::*;

    #[test]
    fn equality_inventory_keeps_nested_and_unused_shapes_in_every_path_position() {
        let parsed = super::super::parse_unresolved(
            "trait Family{type Out;type Other;}\
             fn unused<T:Family<Out=(Boxed<[bool;2]>,<[bool;3] as Family<Other=[bool;4]>>::Out,[&[bool;5];0]),Other=[[bool;6]]>>(){}fn main(){}",
        )
        .unwrap();
        let TraitPath::Source(path) = &parsed.program.functions[0].bounds[0].traits[0] else {
            panic!("expected actual source equality path")
        };
        for site in 0..4 {
            let mut program = parsed.program.clone();
            if site != 0 {
                program.functions[0].bounds.clear();
            }
            match site {
                0 => {}
                1 => program.functions[0].result = Type::Apply(path.clone()),
                2 => {
                    program.functions[0].body.tail = Some(Box::new(Expr::Apply {
                        path: path.clone(),
                        value: Box::new(Expr::Bool(true)),
                    }))
                }
                _ => program.functions[0].body.statements.push(Stmt::LetPattern {
                    pattern: Pattern::Apply {
                        path: path.clone(),
                        pattern: Box::new(Pattern::Wildcard),
                    },
                    ty: Type::Infer,
                    value: Expr::Bool(true),
                }),
            }
            let budget = ir::PlanBudget::default();
            let shapes = inventory(&program, &budget).unwrap();
            let builder = tr::CoreCatalogBuilder::new(
                ir::SnapshotId::standalone().unwrap(),
                program.modules.clone(),
                &budget,
            )
            .unwrap();
            let source_start = builder.source_start();
            let catalog = builder.finish(source_start, shapes, &budget).unwrap();
            for descriptor in [
                tr::RecipeDescriptor::CloneTuple { arity: 3 },
                tr::RecipeDescriptor::CloneArray { len: 0 },
                tr::RecipeDescriptor::CloneArray { len: 2 },
                tr::RecipeDescriptor::CloneArray { len: 3 },
                tr::RecipeDescriptor::CloneArray { len: 4 },
                tr::RecipeDescriptor::CloneArray { len: 5 },
                tr::RecipeDescriptor::CloneArray { len: 6 },
            ] {
                match catalog.recipe(descriptor) {
                    Ok(_) => {}
                    Err(error) => {
                        let footprint = counts(descriptor, &budget).unwrap();
                        let admission = crate::compiler::ownership::preflight_recipe_counts(
                            &footprint, &budget,
                        )
                        .unwrap();
                        let crate::compiler::ownership::RecipeAdmission::IntrinsicLimit(reason) =
                            admission
                        else {
                            panic!("site {site}: missing inventoried {descriptor:?}: {error}");
                        };
                        assert_eq!(error.kind(), tr::FactsErrorKind::Resource);
                        assert_eq!(error.to_string(), reason.message());
                    }
                }
            }
            assert!(!budget.has_resource_failure());
        }
    }

    #[test]
    fn unused_associated_rhs_inventory_includes_projection_self_actuals_and_siblings() {
        let parsed = super::super::parse_unresolved(
            "trait Family<T>{type Out;}struct S;\
             impl Family<bool> for S{type Out=(<([bool;2],bool) as Family<[bool;3]>>::Out,[bool;4]);}\
             fn main(){}",
        )
        .unwrap();
        let budget = ir::PlanBudget::default();
        let shapes = inventory(&parsed.program, &budget).unwrap();
        let builder = tr::CoreCatalogBuilder::new(
            ir::SnapshotId::standalone().unwrap(),
            parsed.program.modules.clone(),
            &budget,
        )
        .unwrap();
        let source_start = builder.source_start();
        let catalog = builder.finish(source_start, shapes, &budget).unwrap();
        for descriptor in [
            tr::RecipeDescriptor::CloneTuple { arity: 2 },
            tr::RecipeDescriptor::CloneArray { len: 2 },
            tr::RecipeDescriptor::CloneArray { len: 3 },
            tr::RecipeDescriptor::CloneArray { len: 4 },
        ] {
            if let Err(error) = catalog.recipe(descriptor) {
                let footprint = counts(descriptor, &budget).unwrap();
                let admission =
                    crate::compiler::ownership::preflight_recipe_counts(&footprint, &budget)
                        .unwrap();
                let crate::compiler::ownership::RecipeAdmission::IntrinsicLimit(reason) = admission
                else {
                    panic!("written associated RHS recipe is missing: {error}");
                };
                assert_eq!(error.kind(), tr::FactsErrorKind::Resource);
                assert_eq!(error.to_string(), reason.message());
            }
        }
        let absent = catalog
            .recipe(tr::RecipeDescriptor::CloneArray { len: 5 })
            .unwrap_err();
        assert_eq!(absent.kind(), tr::FactsErrorKind::InvalidMetadata);
        assert!(!budget.has_resource_failure());

        let budget = ir::PlanBudget::new(ir::PlanLimits {
            depth: 1,
            ..Default::default()
        });
        assert!(inventory(&parsed.program, &budget).is_err());
        let failure = budget.first_resource_failure().unwrap();
        assert_eq!(failure.kind, ir::ResourceBudgetKind::Depth);
        assert_eq!(failure.requested, 2);
    }

    #[test]
    fn trait_actual_recipes_include_unused_bounds_supertraits_impls_and_default_ufcs() {
        let parsed = super::super::parse_unresolved(
            "trait Bound<T>{}trait Parent<T>{}\
             trait Decl<T:Bound<[bool;2]>>:Parent<[bool;3]>{\
             fn provided(&self){let _=<S as Act<[bool;7]>>::go(S);}}\
             struct Wrap<T>(T);impl<T:Bound<[bool;4]>>Bound<[bool;5]> for Wrap<T>{}\
             trait Act<T>{fn go(self)->i32;}struct S;\
             impl Act<[bool;7]> for S{fn go(self)->i32{7}}\
             fn unused<U:Bound<[bool;6]>>(){}fn main(){}",
        )
        .unwrap();
        let budget = ir::PlanBudget::default();
        let shapes = inventory(&parsed.program, &budget).unwrap();
        let builder = tr::CoreCatalogBuilder::new(
            ir::SnapshotId::standalone().unwrap(),
            parsed.program.modules.clone(),
            &budget,
        )
        .unwrap();
        let source_start = builder.source_start();
        let catalog = builder.finish(source_start, shapes, &budget).unwrap();
        for len in 2..=7 {
            let descriptor = tr::RecipeDescriptor::CloneArray { len };
            match catalog.recipe(descriptor) {
                Ok(_) => {}
                Err(error) => {
                    let footprint = counts(descriptor, &budget).unwrap();
                    let admission =
                        crate::compiler::ownership::preflight_recipe_counts(&footprint, &budget)
                            .unwrap();
                    let crate::compiler::ownership::RecipeAdmission::IntrinsicLimit(reason) =
                        admission
                    else {
                        panic!(
                            "missing actually written trait argument array length {len}: {error}"
                        );
                    };
                    assert_eq!(error.kind(), tr::FactsErrorKind::Resource);
                    assert_eq!(error.to_string(), reason.message());
                }
            }
            assert!(!budget.has_resource_failure());
        }
        let absent = catalog
            .recipe(tr::RecipeDescriptor::CloneArray { len: 8 })
            .unwrap_err();
        assert_eq!(absent.kind(), tr::FactsErrorKind::InvalidMetadata);
        assert_eq!(absent.to_string(), "Clone recipe shape was not inventoried");
        assert!(!budget.has_resource_failure());
    }

    #[test]
    fn unused_trait_argument_recipe_depth_failure_is_not_silently_ignored() {
        let parsed = super::super::parse_unresolved(
            "trait Bound<T>{}fn unused<U:Bound<[[bool;2];3]>>(){}fn main(){}",
        )
        .unwrap();
        let budget = ir::PlanBudget::new(ir::PlanLimits {
            depth: 1,
            ..ir::PlanLimits::default()
        });
        assert!(inventory(&parsed.program, &budget).is_err());
        let failure = budget.first_resource_failure().unwrap();
        assert_eq!(failure.kind, ir::ResourceBudgetKind::Depth);
        assert_eq!(failure.requested, 2);
    }

    #[test]
    fn wide_clone_recipe_is_scalar_read_load_without_aggregate_origins() {
        for kind in [ir::IntTy::I128, ir::IntTy::U128] {
            let mut events = Vec::new();
            walk(tr::RecipeDescriptor::CloneInt(kind), |event| {
                events.push(event);
                Ok(())
            })
            .unwrap();
            assert!(matches!(events.as_slice(), [Event::Read, Event::Load]));
            let budget = ir::PlanBudget::new(ir::PlanLimits::default());
            let wide = counts(tr::RecipeDescriptor::CloneInt(kind), &budget).unwrap();
            let narrow = counts(tr::RecipeDescriptor::CloneInt(ir::IntTy::I64), &budget).unwrap();
            assert_eq!(wide, narrow);
            assert_eq!(wide.call_result_origins, 0);
            assert_eq!(wide.loan_sites, 0);
            assert_eq!(wide.binders, 0);
        }
    }
}
