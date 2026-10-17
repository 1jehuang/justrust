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
                add(
                    ds,
                    tr::RecipeDescriptor::CloneArray {
                        len: u16::try_from(*n).map_err(|_| "array recipe length overflow")?,
                    },
                    b,
                )?;
                ty(t, ds, b, depth + 1)?;
            }
            Type::Ref { referent, .. } => ty(referent, ds, b, depth + 1)?,
            // `[T]` is unsized and not Clone: no recipe descriptor. Admission of
            // slice types is decided by the typed frontend, not recipe discovery.
            Type::Slice(_) => {}
            Type::Apply(p) => {
                for s in &p.segments {
                    for t in &s.args {
                        ty(t, ds, b, depth + 1)?;
                    }
                }
            }
            _ => {}
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
                Stmt::Let { ty: t, value, .. } | Stmt::LetPattern { ty: t, value, .. } => {
                    ty(t, ds, b, d + 1)?;
                    expr(value, ds, b, d + 1)?;
                }
                Stmt::AssignDeref { carrier, value } => {
                    expr(carrier, ds, b, d + 1)?;
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
                add(
                    ds,
                    tr::RecipeDescriptor::CloneArray {
                        len: u16::try_from(es.len()).map_err(|_| "array recipe length overflow")?,
                    },
                    b,
                )?;
                for e in es {
                    expr(e, ds, b, d + 1)?;
                }
            }
            Expr::Repeat { value, len } => {
                add(
                    ds,
                    tr::RecipeDescriptor::CloneArray {
                        len: u16::try_from(*len).map_err(|_| "array recipe length overflow")?,
                    },
                    b,
                )?;
                expr(value, ds, b, d + 1)?;
            }
            Expr::Apply { path, value } => {
                for s in &path.segments {
                    for t in &s.args {
                        ty(t, ds, b, d + 1)?;
                    }
                }
                expr(value, ds, b, d + 1)?;
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
            Expr::TraitCall { owner, args, .. } => {
                ty(owner, ds, b, d + 1)?;
                for e in args {
                    expr(e, ds, b, d + 1)?;
                }
            }
            Expr::Call { args, .. } => {
                for e in args {
                    expr(e, ds, b, d + 1)?;
                }
            }
            Expr::Cast { value, target } => {
                ty(target, ds, b, d + 1)?;
                expr(value, ds, b, d + 1)?;
            }
            Expr::Deref { value }
            | Expr::Unary { value, .. }
            | Expr::Field { base: value, .. }
            | Expr::Index { base: value, .. } => expr(value, ds, b, d + 1)?,
            Expr::DynamicIndex { base, index }
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
                    expr(&a.body, ds, b, d + 1)?;
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
            Expr::Block(v) | Expr::Loop { body: v, .. } | Expr::LabeledBlock { body: v, .. } => {
                block(v, ds, b, d + 1)?
            }
            Expr::Break { value: Some(v), .. } => expr(v, ds, b, d + 1)?,
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
        ty(&i.self_ty, &mut descriptors, budget, 0)?;
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
        for i in &t.items {
            for (_, t) in &i.signature.params {
                ty(t, &mut descriptors, budget, 0)?;
            }
            ty(&i.signature.result, &mut descriptors, budget, 0)?;
            if let Some(default) = &i.default {
                function(default, &mut descriptors, budget)?;
            }
        }
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
        body: ir::Block {
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
