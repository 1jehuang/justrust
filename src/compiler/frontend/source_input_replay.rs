//! Recursive input evidence rebuilt from immutable source and lexical occurrences.
//! Checked types are comparison targets, never an inference source.
use super::*;

mod associated_method;
mod borrow_coercion;
mod closure_value;
mod generic_method;
mod indexed_borrow;
mod lexical_block;
mod projected_borrow;
mod shared_deref;

pub(super) fn lexical_result_context(
    original: &OriginalSource,
    inventory: &SourceInventory,
    target: &sr::SourceNode,
    signatures: &Signatures,
    budget: &ir::PlanBudget,
) -> Result<Option<ir::Ty>, FactsError> {
    lexical_block::result_context(original, inventory, target, signatures, budget)
}

// This bounded arithmetic slice obtains its width only from authored suffixes,
// never from checked expression types or a caller's inferred actuals.
fn authored_float_width(
    source: &Expr,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<Option<ir::FloatTy>, FactsError> {
    sr::depth(budget, depth)?;
    sr::work(budget, 1)?;
    match source {
        Expr::Float(literal) => Ok(literal.suffix),
        Expr::Binary {
            op: BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem,
            left,
            right,
            ..
        } => {
            let left = authored_float_width(left, budget, depth + 1)?;
            let right = authored_float_width(right, budget, depth + 1)?;
            Ok(left.filter(|width| Some(*width) == right))
        }
        _ => Ok(None),
    }
}

// Navigate the immutable AST through the same general walker used by source
// validation. The transient pointer index only selects an exact SourceNode,
// never a spelling or an assumed enclosing-expression shape.
fn source_statement<'a>(
    function: &'a Function,
    inventory: &SourceInventory,
    node: &sr::SourceNode,
    budget: &ir::PlanBudget,
) -> Result<Option<&'a Stmt>, FactsError> {
    let mut found = None;
    walk_depth_inspecting(
        DepthNode::Block(&function.body),
        None,
        Some(&inventory.names),
        Some(&mut |part, depth| {
            budget.check_depth(depth)?;
            budget.charge_work(1)?;
            if let DepthNode::Block(block) = part {
                for statement in &block.statements {
                    budget.charge_work(1)?;
                    if let Some(index) = inventory
                        .statement_pointers
                        .get(&(statement as *const Stmt as usize))
                        && inventory.bindings[*index].node == *node
                    {
                        found = Some(statement);
                    }
                }
            }
            Ok(())
        }),
    )
    .map_err(|e| source_error(budget, &e))?;
    Ok(found)
}

fn checked_blocks(
    checked: &ir::CheckedFunction,
    budget: &ir::PlanBudget,
    mut visit: impl FnMut(&ir::TypedBlock) -> Result<(), FactsError>,
) -> Result<(), FactsError> {
    let mut blocks = Vec::new();
    sr::push(&mut blocks, &checked.body, budget)?;
    // Expressions live in one arena, so no expression-edge traversal (and no
    // repeated visits through shared expression IDs) is needed.
    for expression in &checked.exprs {
        sr::work(budget, 1)?;
        match &expression.kind {
            ir::ExprKind::Block(block)
            | ir::ExprKind::LabeledBlock { body: block, .. }
            | ir::ExprKind::Loop { body: block, .. } => sr::push(&mut blocks, block, budget)?,
            ir::ExprKind::If {
                then_block,
                else_block,
                ..
            } => {
                sr::push(&mut blocks, then_block, budget)?;
                sr::push(&mut blocks, else_block, budget)?;
            }
            _ => {}
        }
    }
    while let Some(block) = blocks.pop() {
        visit(block)?;
        for statement in &block.statements {
            sr::work(budget, 1)?;
            if let ir::TypedStmt::While { body, .. } = statement {
                sr::push(&mut blocks, body, budget)?;
            }
        }
    }
    Ok(())
}

pub(super) fn capture_scoped_initializers(
    cap: &CanonicalSourceCapability,
    checked: &ir::CheckedFunction,
    budget: &ir::PlanBudget,
) -> Result<Vec<(usize, ir::LocalId, ir::ExprId)>, FactsError> {
    let bindings = cap.scoped_bindings.borrow();
    if bindings.is_empty() {
        return Ok(Vec::new());
    }
    let mut locals = HashMap::new();
    for (index, local) in bindings.iter() {
        reserve_source_map(&mut locals, budget)?;
        if locals.insert(*local, *index).is_some() {
            return Err(FactsError::invalid(budget, "duplicate scoped source local"));
        }
    }
    let mut result = Vec::new();
    checked_blocks(checked, budget, |block| {
        for statement in &block.statements {
            sr::work(budget, 1)?;
            if let ir::TypedStmt::Let { local, value } = statement
                && let Some(index) = locals.remove(local)
            {
                sr::push(&mut result, (index, *local, *value), budget)?;
            }
        }
        Ok(())
    })?;
    if !locals.is_empty() {
        return Err(FactsError::invalid(
            budget,
            "scoped source initializer missing",
        ));
    }
    Ok(result)
}

#[derive(Clone, Debug)]
pub(super) struct ArrayBorrowReceipt {
    binding: usize,
    node: sr::SourceNode,
    initializer: ir::ExprId,
    reference: ir::ExprId,
    temporary: ir::LocalId,
    value: ir::ExprId,
    borrowed: ir::ExprId,
}

// This slice deliberately covers only authored shared byte-slice annotations
// and closed integer arrays. Storage identities are captured before publication.
fn shared_byte_array<'a>(source: &'a Expr, annotation: Option<&Type>) -> Option<&'a Expr> {
    let Some(Type::Ref {
        kind: ir::BorrowKind::Shared,
        referent,
        ..
    }) = annotation
    else {
        return None;
    };
    if !matches!(referent.as_ref(), Type::Slice(element) if matches!(element.as_ref(), Type::Int(ir::IntTy::U8)))
    {
        return None;
    }
    let Expr::BorrowValue {
        kind: ir::BorrowKind::Shared,
        value,
    } = source
    else {
        return None;
    };
    match value.as_ref() {
        Expr::Array(elements)
            if elements
                .iter()
                .all(|element| matches!(element, Expr::Integer(_))) =>
        {
            Some(value)
        }
        _ => None,
    }
}

pub(super) fn capture_array_borrows(
    cap: &CanonicalSourceCapability,
    checked: &ir::CheckedFunction,
    root_positions: &[Option<usize>],
    budget: &ir::PlanBudget,
) -> Result<Vec<ArrayBorrowReceipt>, FactsError> {
    let function = cap
        .plan
        .callable_body(&cap.plan.program, cap.declaration)
        .map_err(|e| source_error(budget, &e))?;
    let mut result = Vec::new();
    for (binding, site) in cap.inventory.bindings.iter().enumerate() {
        sr::work(budget, 1)?;
        // Nested/scoped initializer capture is intentionally not generalized.
        let Some(position) = root_positions.get(binding).copied().flatten() else {
            continue;
        };
        let Some(Stmt::Let { value: source, .. }) =
            source_statement(function, &cap.inventory, &site.node, budget)?
        else {
            continue;
        };
        if let Expr::BorrowValue { value, .. } = source
            && let Expr::Array(elements) = value.as_ref()
        {
            sr::work(budget, elements.len())?;
        }
        if shared_byte_array(source, site.annotation.as_ref()).is_none() {
            continue;
        }
        let Some(ir::TypedStmt::Let {
            value: initializer, ..
        }) = checked.body.statements.get(position)
        else {
            return Err(FactsError::invalid(
                budget,
                "original byte-array initializer missing",
            ));
        };
        let Some(ir::TypedExpr {
            kind: ir::ExprKind::ArrayToSlice { reference },
            ..
        }) = checked.exprs.get(initializer.0)
        else {
            return Err(FactsError::invalid(
                budget,
                "original byte-array unsizing missing",
            ));
        };
        let Some(ir::TypedExpr {
            kind: ir::ExprKind::Block(block),
            ..
        }) = checked.exprs.get(reference.0)
        else {
            return Err(FactsError::invalid(
                budget,
                "original byte-array temporary block missing",
            ));
        };
        let ([ir::TypedStmt::Assign { local, value }], Some(borrowed)) =
            (block.statements.as_slice(), block.tail)
        else {
            return Err(FactsError::invalid(
                budget,
                "original byte-array temporary shape changed",
            ));
        };
        sr::push(
            &mut result,
            ArrayBorrowReceipt {
                binding,
                node: site.node.child(3, 0, budget)?,
                initializer: *initializer,
                reference: *reference,
                temporary: *local,
                value: *value,
                borrowed,
            },
            budget,
        )?;
    }
    Ok(result)
}

fn precedes(binding: &sr::SourceNode, operand: &sr::SourceNode) -> bool {
    precedes_path(&binding.path, binding, operand)
}

/// As `precedes`, for a binder whose statement occurrence is the given path
/// (a tuple-pattern binder's node extends its `let` statement path).
fn precedes_path(path: &[u32], binding: &sr::SourceNode, operand: &sr::SourceNode) -> bool {
    let Some((ordinal, parent)) = path.split_last() else {
        return false;
    };
    let Some((2, block)) = parent.split_last() else {
        return false;
    };
    if operand.owner != binding.owner {
        return false;
    }
    match operand.path.strip_prefix(block) {
        // The initializer executes before its own binding enters scope. A
        // later statement (including its nested blocks) may use the binding.
        Some([2, next, ..]) => next > ordinal,
        Some([6, ..]) => true, // enclosing block tail
        _ => false,
    }
}

pub(super) fn initializer(
    original: &OriginalSource,
    index: usize,
    checked: &ir::CheckedFunction,
    budget: &ir::PlanBudget,
) -> Result<(ir::LocalId, ir::ExprId), FactsError> {
    let site = original
        .bindings
        .get(index)
        .ok_or_else(|| FactsError::invalid(budget, "input binding missing"))?;
    if site.statement.is_some() {
        let local = mapped_source_binding(original, index, budget)?;
        let position = original
            .root_positions
            .get(index)
            .copied()
            .flatten()
            .ok_or_else(|| FactsError::invalid(budget, "input let mapping missing"))?;
        if let Some(ir::TypedStmt::Let {
            local: actual,
            value,
        }) = checked.body.statements.get(position)
            && *actual == local
        {
            return Ok((local, *value));
        }
        return Err(FactsError::invalid(budget, "input checked let changed"));
    }
    let mut mapping = None;
    for (source, local, value) in &original.scoped_initializers {
        sr::work(budget, 1)?;
        if *source == index && mapping.replace((*local, *value)).is_some() {
            return Err(FactsError::invalid(
                budget,
                "duplicate scoped initializer mapping",
            ));
        }
    }
    let (local, value) =
        mapping.ok_or_else(|| FactsError::invalid(budget, "scoped initializer mapping missing"))?;
    let mut matches = 0;
    checked_blocks(checked, budget, |block| {
        for statement in &block.statements {
            sr::work(budget, 1)?;
            if let ir::TypedStmt::Let {
                local: actual,
                value: actual_value,
            } = statement
                && *actual == local
            {
                if *actual_value != value {
                    return Err(FactsError::invalid(budget, "scoped initializer changed"));
                }
                matches += 1;
            }
        }
        Ok(())
    })?;
    if matches != 1 {
        return Err(FactsError::invalid(
            budget,
            "scoped initializer occurrence changed",
        ));
    }
    Ok((local, value))
}

fn syntax(source: &Expr, budget: &ir::PlanBudget, depth: usize) -> Result<bool, FactsError> {
    sr::depth(budget, depth)?;
    sr::work(budget, 1)?;
    let children: Vec<&Expr> = match source {
        Expr::Block(block) => return lexical_block::syntax(block, budget, depth + 1),
        Expr::AssociatedCall {
            type_args, args, ..
        } if type_args.is_empty() => {
            source_clone::clone_vector(args, budget).map_err(|e| source_error(budget, &e))?;
            args.iter().collect()
        }
        // This only retains syntax. Rooted anonymous providers are authenticated
        // by supported/replay before they may supply any inferred input type.
        Expr::Var(path) => {
            return Ok(is_local_shaped_path(path) || path.anchor() == PathAnchor::Crate);
        }
        Expr::Borrow { .. } | Expr::Integer(_) | Expr::Bool(_) | Expr::Char(_) => return Ok(true),
        Expr::BorrowValue { .. } if indexed_borrow::authored_index_borrow(source).is_some() => {
            return Ok(true);
        }
        Expr::BorrowValue { .. } => {
            return explicit_field_borrow::authored_static_borrow(source, budget)
                .map(|source| source.is_some())
                .map_err(|e| source_error(budget, &e));
        }
        Expr::Str(text) => {
            source_clone::clone_text(text, budget).map_err(|e| source_error(budget, &e))?;
            return Ok(true);
        }
        Expr::Tuple(values)
        | Expr::Array(values)
        | Expr::TupleStruct { fields: values, .. }
        | Expr::Call { args: values, .. } => {
            source_clone::clone_vector(values, budget).map_err(|e| source_error(budget, &e))?;
            values.iter().collect()
        }
        Expr::MethodCall {
            receiver,
            type_args,
            args,
            ..
        } if type_args.is_empty() => {
            if !syntax(receiver, budget, depth + 1)? {
                return Ok(false);
            }
            for argument in args {
                if !syntax(argument, budget, depth + 1)? {
                    return Ok(false);
                }
            }
            return Ok(true);
        }
        Expr::Struct { fields, .. } => {
            source_clone::clone_vector(fields, budget).map_err(|e| source_error(budget, &e))?;
            fields.iter().map(|(_, value)| value).collect()
        }
        Expr::EnumVariant { fields, .. } => {
            source_clone::clone_vector(fields, budget).map_err(|e| source_error(budget, &e))?;
            fields.iter().map(|(_, value)| value).collect()
        }
        Expr::Repeat { value, .. } | Expr::Apply { value, .. } => {
            return syntax(value, budget, depth + 1);
        }
        _ => return Ok(false),
    };
    for value in children {
        if !syntax(value, budget, depth + 1)? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn copy_arguments(
    args: &[Expr],
    budget: &ir::PlanBudget,
) -> Result<Option<Vec<Expr>>, FactsError> {
    for arg in args {
        if !syntax(arg, budget, 0)? {
            return Ok(None);
        }
    }
    source_clone::clone_vector(args, budget).map_err(|e| source_error(budget, &e))?;
    let mut copied = Vec::new();
    copied
        .try_reserve_exact(args.len())
        .map_err(|_| FactsError::resource(budget, "input syntax allocation"))?;
    for arg in args {
        walk_depth(DepthNode::Expr(arg), Some(budget)).map_err(|e| source_error(budget, &e))?;
        copied.push(arg.clone());
    }
    Ok(Some(copied))
}

// Prepublication selection must not suppress a source method's existing
// inference path. Only authored shared slice/str receiver syntax establishes
// this candidate class without consulting checked or inferred types. Lexical
// selection is occurrence-based, so a shadowing Vec/nominal local cannot inherit
// an outer descriptor annotation. Canonical replay still proves the operation.
fn authored_len_receiver(
    inventory: &SourceInventory,
    node: &sr::SourceNode,
    budget: &ir::PlanBudget,
) -> Result<bool, FactsError> {
    let receiver = node.child(10, 0, budget)?;
    sr::work(budget, receiver.path.len())?;
    let Some(binding) = inventory.operand_nodes.get(&receiver.path) else {
        return Ok(false);
    };
    let Some(index) = source_binding_index(&inventory.bindings, binding, budget)? else {
        return Ok(false);
    };
    Ok(matches!(
        inventory.bindings[index].annotation.as_ref(),
        Some(Type::Ref { kind: ir::BorrowKind::Shared, referent, .. })
            if matches!(referent.as_ref(), Type::Slice(_) | Type::Str)
    ))
}

// Receiver autoderef placeholders deliberately share their source node with
// an authored nested call. Only the non-operator role owns that call.
fn authored_call_index(
    inventory: &SourceInventory,
    node: &sr::SourceNode,
    budget: &ir::PlanBudget,
) -> Result<usize, FactsError> {
    sr::work(budget, inventory.calls.len())?;
    let mut found = None;
    for (index, site) in inventory.calls.iter().enumerate() {
        if site.node == *node && !site.operator_candidate && found.replace(index).is_some() {
            return Err(FactsError::invalid(
                budget,
                "duplicate authored call source role",
            ));
        }
    }
    found.ok_or_else(|| FactsError::invalid(budget, "authored call source role missing"))
}

fn original_nonstored_field(
    original: &OriginalSource,
    inventory: &SourceInventory,
    node: &sr::SourceNode,
    field: &FieldSelector,
    budget: &ir::PlanBudget,
) -> Result<bool, FactsError> {
    validate_original_occurrence(original, node, budget)?;
    let binding = inventory
        .operand_nodes
        .get(&node.path)
        .ok_or_else(|| FactsError::invalid(budget, "projected source operand binding missing"))?;
    validate_original_occurrence(original, binding, budget)?;
    let index = source_binding_index(&inventory.bindings, binding, budget)?
        .ok_or_else(|| FactsError::invalid(budget, "projected source lexical binding missing"))?;
    let site = &inventory.bindings[index];
    let function = original
        .plan
        .callable_body(&original.plan.program, original.declaration)
        .map_err(|e| source_error(budget, &e))?;
    let annotation = match site.parameter {
        Some(parameter) => Some(
            &function
                .params
                .get(parameter)
                .ok_or_else(|| FactsError::invalid(budget, "projected source parameter missing"))?
                .1,
        ),
        None => site.annotation.as_ref(),
    };
    let path = if let Some(annotation) = annotation {
        match annotation {
            Type::Named(path) => path.clone(),
            Type::Apply(path) => path
                .path(inventory.names.as_ref())
                .map_err(|e| source_error(budget, &e))?,
            _ => return Ok(false),
        }
    } else {
        let Some(Stmt::Let { value, .. }) = source_statement(function, inventory, binding, budget)?
        else {
            return Ok(false);
        };
        let mut value = value;
        let mut depth = 0;
        while let Expr::Apply { value: inner, .. } = value {
            sr::depth(budget, depth)?;
            sr::work(budget, 1)?;
            depth += 1;
            value = inner;
        }
        match value {
            Expr::Struct { name, .. } | Expr::TupleStruct { name, .. } => name.clone(),
            _ => return Ok(false),
        }
    };
    let key = name_scope::path_key(original.plan.program.names.as_ref(), &path, Namespace::Type)
        .map_err(|e| source_error(budget, &e))?;
    let Some(declaration) = original.plan.registry.nominals.get(&key) else {
        return Ok(false);
    };
    let Some(source) = original.plan.nominal_schemas.get(declaration) else {
        return Err(FactsError::invalid(
            budget,
            "projected source nominal schema missing",
        ));
    };
    let NominalSchema::Struct(source) = source.as_ref() else {
        return Ok(false);
    };
    let stored = match (source.shape, field) {
        (StructShape::Tuple, FieldSelector::Tuple(index)) => source.fields.get(*index).is_some_and(
            |field| matches!(field.name, FieldName::Tuple(actual) if actual == *index),
        ),
        (StructShape::Record, FieldSelector::Named(name)) => {
            let class = crate::compiler::semantic_name::LookupClass::Member;
            let expected = inventory
                .names
                .key(*name, class)
                .map_err(|e| source_error(budget, &native_adapter::name_diagnostic(e)))?;
            let mut stored = false;
            for field in &source.fields {
                sr::work(budget, 1)?;
                if let FieldName::Named(name) = field.name {
                    stored |=
                        inventory.names.key(name, class).map_err(|e| {
                            source_error(budget, &native_adapter::name_diagnostic(e))
                        })? == expected;
                }
            }
            stored
        }
        _ => false,
    };
    Ok(!stored)
}

pub(super) fn supported(
    source: &Expr,
    node: &sr::SourceNode,
    plan: &PlanCtx,
    function: &Function,
    inventory: &SourceInventory,
    original: Option<&OriginalSource>,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<bool, FactsError> {
    sr::depth(budget, depth)?;
    sr::work(budget, 1)?;
    if authored_float_width(source, budget, depth)?.is_some()
        || closure_value::provider(plan, source, budget)?.is_some()
    {
        return Ok(true);
    }
    if indexed_borrow::authored_index_borrow(source).is_some() {
        return supported(
            match source {
                Expr::BorrowValue { value, .. } => match value.as_ref() {
                    Expr::Index { base, .. } => base,
                    _ => unreachable!(),
                },
                _ => unreachable!(),
            },
            &indexed_borrow::root_node(node, budget)?,
            plan,
            function,
            inventory,
            original,
            budget,
            depth + 1,
        );
    }
    if matches!(source, Expr::BorrowValue { .. }) {
        let Some((kind, place)) = explicit_field_borrow::authored_static_borrow(source, budget)
            .map_err(|e| source_error(budget, &e))?
        else {
            return Ok(false);
        };
        let Some(original) = original else {
            return Ok(false);
        };
        let LocalName::Named(root) = place.local else {
            return Ok(false);
        };
        Replay::borrow_coercion_field_site(
            original,
            inventory,
            node,
            kind,
            root,
            (&place.fields, true),
            budget,
        )?;
        let Some(first) = place.fields.first() else {
            return Ok(false);
        };
        if !original_nonstored_field(original, inventory, node, first, budget)? {
            return Ok(false);
        }
    }
    match source {
        Expr::Block(block) => lexical_block::supported(
            block,
            node,
            plan,
            function,
            inventory,
            original,
            budget,
            depth + 1,
        ),
        Expr::Var(_) | Expr::Borrow { .. } | Expr::BorrowValue { .. } => {
            sr::work(budget, node.path.len())?;
            let Some(binding) = inventory.operand_nodes.get(&node.path) else {
                return Ok(false);
            };
            let Some(index) = source_binding_index(&inventory.bindings, binding, budget)? else {
                return Ok(false);
            };
            let site = &inventory.bindings[index];
            if site.parameter.is_some() || site.annotation.is_some() {
                return Ok(true);
            }
            if site.pattern.is_some() {
                let Some(leaf) = pattern_source_leaf(function, site, budget)? else {
                    return Ok(false);
                };
                if leaf.annotation.is_some() {
                    return Ok(true);
                }
                return supported(
                    leaf.value,
                    &leaf.node,
                    plan,
                    function,
                    inventory,
                    original,
                    budget,
                    depth + 1,
                );
            }
            let Some(Stmt::Let { value, .. }) =
                source_statement(function, inventory, &site.node, budget)?
            else {
                return Ok(false);
            };
            supported(
                value,
                &site.node.child(3, 0, budget)?,
                plan,
                function,
                inventory,
                original,
                budget,
                depth + 1,
            )
        }
        Expr::Tuple(values) | Expr::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                if !supported(
                    value,
                    &node.child(12, index, budget)?,
                    plan,
                    function,
                    inventory,
                    original,
                    budget,
                    depth + 1,
                )? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        Expr::TupleStruct { fields, .. } => {
            for (index, value) in fields.iter().enumerate() {
                if !supported(
                    value,
                    &node.child(14, index, budget)?,
                    plan,
                    function,
                    inventory,
                    original,
                    budget,
                    depth + 1,
                )? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        Expr::Struct { fields, .. } => {
            for (index, (_, value)) in fields.iter().enumerate() {
                if !supported(
                    value,
                    &node.child(14, index, budget)?,
                    plan,
                    function,
                    inventory,
                    original,
                    budget,
                    depth + 1,
                )? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        Expr::EnumVariant { fields, .. } => {
            for (index, (_, value)) in fields.iter().enumerate() {
                if !supported(
                    value,
                    &node.child(14, index, budget)?,
                    plan,
                    function,
                    inventory,
                    original,
                    budget,
                    depth + 1,
                )? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        Expr::Apply { value, .. } => supported(
            value,
            &node.child(7, 0, budget)?,
            plan,
            function,
            inventory,
            original,
            budget,
            depth + 1,
        ),
        Expr::ParamRepeat { .. } => Ok(false),
        Expr::Repeat { value, .. } => supported(
            value,
            &node.child(11, 0, budget)?,
            plan,
            function,
            inventory,
            original,
            budget,
            depth + 1,
        ),
        Expr::Call { args, .. } => {
            for (index, value) in args.iter().enumerate() {
                if !supported(
                    value,
                    &node.child(9, index, budget)?,
                    plan,
                    function,
                    inventory,
                    original,
                    budget,
                    depth + 1,
                )? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        Expr::AssociatedCall {
            type_args, args, ..
        } if type_args.is_empty() => {
            for (index, argument) in args.iter().enumerate() {
                if !supported(
                    argument,
                    &node.child(9, index, budget)?,
                    plan,
                    function,
                    inventory,
                    original,
                    budget,
                    depth + 1,
                )? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        Expr::MethodCall {
            receiver,
            type_args,
            args,
            ..
        } if type_args.is_empty() => {
            // Selection here is provisional. Only replay of the exact builtin
            // receipt and lexical receiver below grants SliceLen authority.
            let index = authored_call_index(inventory, node, budget)?;
            let site = &inventory.calls[index];
            if let Some(original) = original {
                sr::work(budget, original.call_mapping.len())?;
                sr::work(
                    budget,
                    original.builtin_len_mapping.len()
                        + original.builtin_leading_zeros_mapping.len()
                        + original.builtin_intrinsic_mapping.len(),
                )?;
                if original
                    .builtin_len_mapping
                    .iter()
                    .any(|row| row.source_index == index)
                {
                    return Ok(site.len_candidate
                        && !site.for_iterable
                        && authored_len_receiver(inventory, node, budget)?);
                }
                if original
                    .builtin_leading_zeros_mapping
                    .iter()
                    .any(|row| row.source_index == index)
                    || original
                        .builtin_intrinsic_mapping
                        .iter()
                        .any(|row| row.source_index == index)
                {
                    return Ok(false);
                }
                let (_, _, target) = original
                    .call_mapping
                    .iter()
                    .find(|(source, _, _)| *source == index)
                    .ok_or_else(|| {
                        FactsError::invalid(budget, "method producer original receipt missing")
                    })?;
                let ir::Callee::Direct(target) = target else {
                    return Ok(false);
                };
                let view = original.facts.original_callable_schema(
                    tr::OriginalCallableSubject::Direct(target.declaration),
                )?;
                view.validate_for(&original.facts)?;
                if !original.facts.is_source_callable(target.declaration)? {
                    return Ok(false);
                }
                let declaration = original
                    .plan
                    .registry
                    .declaration(target.declaration)
                    .map_err(|e| source_error(budget, &e))?;
                if (!args.is_empty()
                    || matches!(
                        declaration.receiver,
                        ReceiverKind::Borrowed {
                            kind: ir::BorrowKind::Mut
                        }
                    ))
                    && (!matches!(declaration.receiver, ReceiverKind::Borrowed { .. })
                        || !matches!(receiver.as_ref(), Expr::Var(path) if is_local_shaped_path(path)))
                {
                    return Ok(false);
                }
                for (index, argument) in args.iter().enumerate() {
                    if !supported(
                        argument,
                        &node.child(9, index, budget)?,
                        plan,
                        function,
                        inventory,
                        Some(original),
                        budget,
                        depth + 1,
                    )? {
                        return Ok(false);
                    }
                }
                if declaration.path.associated.is_none()
                    || !matches!(
                        declaration.receiver,
                        ReceiverKind::Borrowed {
                            kind: ir::BorrowKind::Shared | ir::BorrowKind::Mut
                        } | ReceiverKind::Value { .. }
                    )
                {
                    return Ok(false);
                }
                if !target.type_args.is_empty()
                    || view.generic_env().total_arity() != 0
                    || matches!(declaration.receiver, ReceiverKind::Value { .. })
                    || !matches!(receiver.as_ref(), Expr::Var(path) if is_local_shaped_path(path))
                {
                    if declaration.param_env.arity() != 0 {
                        return Ok(false);
                    }
                    return supported(
                        receiver,
                        &node.child(10, 0, budget)?,
                        plan,
                        function,
                        inventory,
                        Some(original),
                        budget,
                        depth + 1,
                    );
                }
            } else if site.len_candidate {
                // Do not route Vec/custom len methods away from their existing
                // fallback just because their spelling is a builtin candidate.
                return authored_len_receiver(inventory, node, budget);
            } else if site.leading_zeros_candidate || site.intrinsic_candidate {
                return Ok(false);
            }
            // Source method replay still authenticates the exact declaration,
            // original source occurrence and lexical receiver root.
            if matches!(receiver.as_ref(), Expr::Var(path) if is_local_shaped_path(path)) {
                let receiver = node.child(10, 0, budget)?;
                Ok(inventory.operand_nodes.contains_key(&receiver.path))
            } else {
                supported(
                    receiver,
                    &node.child(10, 0, budget)?,
                    plan,
                    function,
                    inventory,
                    original,
                    budget,
                    depth + 1,
                )
            }
        }
        Expr::Integer(_) | Expr::Bool(_) | Expr::Char(_) | Expr::Str(_) => Ok(true),
        _ => Ok(false),
    }
}

pub(super) struct Replay<'a> {
    pub original: &'a OriginalSource,
    pub inventory: &'a SourceInventory,
    pub checked: &'a ir::CheckedFunction,
    pub signatures: &'a Signatures,
    pub budget: &'a ir::PlanBudget,
}
impl Replay<'_> {
    /// Preserve unresolved numeric source terms through aggregate structure.
    /// Nonliteral leaves contribute only after ordinary source replay has
    /// authenticated them. No checked expression type seeds these constraints.
    #[allow(clippy::too_many_arguments)]
    fn input_term(
        &self,
        source: &Expr,
        node: &sr::SourceNode,
        id: ir::ExprId,
        checker: &Checker<'_>,
        state: &sr::CallInferenceState,
        depth: usize,
    ) -> Result<sr::InferTerm, FactsError> {
        sr::depth(self.budget, depth)?;
        sr::work(self.budget, 1)?;
        let actual = self
            .checked
            .exprs
            .get(id.0)
            .ok_or_else(|| FactsError::invalid(self.budget, "array source operand missing"))?;
        match (source, &actual.kind) {
            (Expr::Integer(_), ir::ExprKind::Int(_)) => checker
                .inference_evidence(source, depth)
                .map_err(|e| source_error(self.budget, &e)),
            (Expr::Tuple(fields), ir::ExprKind::Tuple(values)) if fields.len() == values.len() => {
                let mut terms = Vec::new();
                for (index, (source, value)) in fields.iter().zip(values).enumerate() {
                    sr::push(
                        &mut terms,
                        self.input_term(
                            source,
                            &node.child(12, index, self.budget)?,
                            *value,
                            checker,
                            state,
                            depth + 1,
                        )?,
                        self.budget,
                    )?;
                }
                state.node(sr::InferKind::Tuple(terms.into()))
            }
            (Expr::Array(fields), ir::ExprKind::Array(values)) if fields.len() == values.len() => {
                let element =
                    self.array_element_term(fields, values, node, checker, state, None, depth + 1)?;
                state.node(sr::InferKind::Array(element, fields.len()))
            }
            (
                Expr::Repeat { value, len },
                ir::ExprKind::Repeat {
                    value: actual,
                    len: actual_len,
                },
            ) if len == actual_len => {
                let element = self.input_term(
                    value,
                    &node.child(11, 0, self.budget)?,
                    *actual,
                    checker,
                    state,
                    depth + 1,
                )?;
                state.node(sr::InferKind::Array(element, *len))
            }
            _ => {
                let ty = self.expression(source, node, id, None, depth + 1)?;
                state.type_term(&ty, None, depth + 1)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn array_element_term(
        &self,
        fields: &[Expr],
        values: &[ir::ExprId],
        node: &sr::SourceNode,
        checker: &Checker<'_>,
        state: &sr::CallInferenceState,
        expected: Option<&ir::Ty>,
        depth: usize,
    ) -> Result<sr::InferTerm, FactsError> {
        sr::depth(self.budget, depth)?;
        let mut element = expected
            .map(|ty| state.type_term(ty, None, depth))
            .transpose()?;
        for (index, (source, value)) in fields.iter().zip(values).enumerate() {
            let next = self.input_term(
                source,
                &node.child(12, index, self.budget)?,
                *value,
                checker,
                state,
                depth + 1,
            )?;
            element = Some(match element {
                Some(previous) => state.merge(&previous, &next, depth + 1)?,
                None => next,
            });
        }
        match element {
            Some(element) => Ok(element),
            None => state.node(sr::InferKind::Unknown(false)),
        }
    }

    fn array_element_type(
        &self,
        fields: &[Expr],
        values: &[ir::ExprId],
        node: &sr::SourceNode,
        expected: Option<&ir::Ty>,
        depth: usize,
    ) -> Result<ir::Ty, FactsError> {
        let module = self
            .original
            .plan
            .callable_context(self.original.declaration)
            .map_err(|e| source_error(self.budget, &e))?
            .module;
        let checker = Checker::new(self.signatures, ir::Ty::Unit, module);
        let state = sr::CallInferenceState::new(None, self.budget)?;
        let expected = match expected {
            Some(ir::Ty::Array { element, .. }) => Some(element.as_ref()),
            _ => None,
        };
        let element =
            self.array_element_term(fields, values, node, &checker, &state, expected, depth)?;
        // Rust's integer fallback applies after all sibling constraints have
        // merged, never while visiting the first unsuffixed element.
        state
            .resolve(&element, true, depth, &mut |decl, args| {
                self.original
                    .plan
                    .intern_nominal(
                        decl,
                        args,
                        ir::FactsCtx::Validated(&self.original.facts),
                        self.budget,
                    )
                    .map_err(|e| source_error(self.budget, &e))
            })?
            .ok_or_else(|| {
                FactsError::unsupported(self.budget, "array element source type is unresolved")
            })
    }

    #[allow(clippy::too_many_arguments)]
    fn explicit_producer(
        &self,
        application: &GenericPath,
        inputs: &[Expr],
        node: &sr::SourceNode,
        id: ir::ExprId,
        arguments: &[ir::ExprId],
        target: &ir::Callee,
        signature: &ir::Signature,
        canonical: &ir::Signature,
        evidence: &ir::CallEvidence,
        depth: usize,
    ) -> Result<ir::Ty, FactsError> {
        sr::depth(self.budget, depth)?;
        let (_, written) = application
            .application()
            .map_err(|e| source_error(self.budget, &e))?;
        if written
            .iter()
            .any(type_check::partial_annotation::has_nested_infer)
        {
            return Err(FactsError::unsupported(
                self.budget,
                "producer requires fully authored type arguments",
            ));
        }
        let ir::Callee::Direct(direct) = target else {
            return Err(FactsError::invalid(
                self.budget,
                "explicit producer changed target role",
            ));
        };
        let context = self
            .original
            .plan
            .callable_context(self.original.declaration)
            .map_err(|e| source_error(self.budget, &e))?;
        let checker = Checker::new(self.signatures, ir::Ty::Unit, context.module);
        checker
            .push_application(application, Namespace::Value)
            .map_err(|e| source_error(self.budget, &e))?;
        let (name, authored, regions) =
            checker.applications.borrow_mut().pop().ok_or_else(|| {
                FactsError::invalid(self.budget, "producer authored application missing")
            })?;
        if self.original.plan.registry.functions.get(&name) != Some(&direct.declaration)
            || authored.len() != direct.type_args.len()
        {
            return Err(FactsError::invalid(
                self.budget,
                "producer authored target or arity changed",
            ));
        }
        for (source, actual) in authored.iter().zip(direct.type_args.iter()) {
            if !self.equal(source, actual)? {
                return Err(FactsError::invalid(
                    self.budget,
                    "producer authored type argument changed",
                ));
            }
        }
        // This replay authenticates the authored target, ordered arguments and
        // source call evidence. It does not issue an executable DirectCallProof
        // or require projected source fields to have physical graph identities.
        self.original
            .facts
            .validate_source_flow_signature_with_scratch(
                ir::SignatureSubject::Call(target),
                canonical,
                &mut tr::TraitScratch::default(),
            )?;
        if inputs.len() != arguments.len()
            || arguments.len() != canonical.params.len()
            || evidence.arguments.len() != arguments.len()
        {
            return Err(FactsError::invalid(
                self.budget,
                "explicit producer input arity changed",
            ));
        }
        for (index, (source, argument)) in inputs.iter().zip(arguments).enumerate() {
            // Explicit application instantiates the signature before checking
            // arguments. In contrast to inferred calls, T = &mut U supplies
            // a firm mutable-reference expectation here.
            let expectation = match &canonical.params[index] {
                ir::Ty::Ref { kind, .. } => ir::CallExpectation::FirmReference { kind: *kind },
                _ => ir::CallExpectation::NoFirmReference,
            };
            let input = self.argument(
                source,
                &node.child(9, index, self.budget)?,
                *argument,
                id,
                index,
                expectation,
                Some(&canonical.params[index]),
                depth + 1,
            )?;
            let row = &evidence.arguments[index];
            if !self.equal(&input, &canonical.params[index])?
                || row.argument != index
                || row.value != *argument
                || row.expectation != expectation
            {
                return Err(FactsError::invalid(
                    self.budget,
                    "explicit producer source input changed",
                ));
            }
        }
        // Preserve authored region occurrences, including elided nested ones.
        let mut expected_regions = Vec::new();
        for row in regions.iter() {
            sr::push(&mut expected_regions, row.clone(), self.budget)?;
        }
        for (argument, ty) in authored.iter().enumerate() {
            for location in ir::carrier_locations(ty, self.budget)
                .map_err(|e| source_error(self.budget, &e))?
                .iter()
            {
                sr::work(self.budget, expected_regions.len() + 1)?;
                if matches!(location.channel, ir::CarrierChannel::Fixed(_))
                    && !expected_regions
                        .iter()
                        .any(|row| row.argument == argument && row.location == *location)
                {
                    sr::push(
                        &mut expected_regions,
                        ir::TypeActualRegion {
                            argument,
                            location: location.clone(),
                            source: ir::ActualRegion::Elided,
                        },
                        self.budget,
                    )?;
                }
            }
        }
        if expected_regions.as_slice() != evidence.type_actual_regions.as_ref()
            || !self.equal(&canonical.result, &signature.result)?
        {
            return Err(FactsError::invalid(
                self.budget,
                "explicit producer regions or result changed",
            ));
        }
        Ok(canonical.result.clone())
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn argument(
        &self,
        source: &Expr,
        node: &sr::SourceNode,
        id: ir::ExprId,
        call: ir::ExprId,
        index: usize,
        expectation: ir::CallExpectation,
        expected: Option<&ir::Ty>,
        depth: usize,
    ) -> Result<ir::Ty, FactsError> {
        if matches!(source, Expr::Var(_))
            && matches!(
                expectation,
                ir::CallExpectation::FirmReference {
                    kind: ir::BorrowKind::Mut
                }
            )
        {
            let (local, ty) = self.operand(node, depth)?;
            let value = self
                .checked
                .exprs
                .get(id.0)
                .ok_or_else(|| FactsError::invalid(self.budget, "input reborrow missing"))?;
            let ir::ExprKind::Borrow {
                kind: ir::BorrowKind::Mut,
                place,
                timing:
                    ir::BorrowTiming::TwoPhase {
                        activation,
                        argument,
                        reason: ir::ImplicitBorrowKind::MutableArgumentReborrow { carrier },
                    },
            } = &value.kind
            else {
                return Err(FactsError::invalid(
                    self.budget,
                    "input lost authored mutable reborrow",
                ));
            };
            let carrier = self.checked.exprs.get(carrier.0).ok_or_else(|| {
                FactsError::invalid(self.budget, "input reborrow carrier missing")
            })?;
            if *activation != call
                || *argument != index
                || place.local != local
                || place.projection != [ir::Projection::Deref]
                || !matches!(carrier.kind, ir::ExprKind::Local(actual) if actual == local)
                || carrier.completion != ir::Completion::Returns
                || value.completion != ir::Completion::Returns
                || !self.equal(&ty, &carrier.ty)?
                || !self.equal(&ty, &value.ty)?
            {
                return Err(FactsError::invalid(
                    self.budget,
                    "input reborrow changed source identity or type",
                ));
            }
            Ok(ty)
        } else {
            // Inferred calls still provide concrete parameter context, such
            // as the usize argument of `split<T>(slice, index)`. Recover it
            // from the original declaration, never the checked call's actuals.
            let declared = if expected.is_none() {
                sr::work(self.budget, self.original.call_mapping.len())?;
                let (source_index, _, target) = self
                    .original
                    .call_mapping
                    .iter()
                    .find(|(_, expression, _)| *expression == call)
                    .ok_or_else(|| {
                        FactsError::invalid(self.budget, "input call source mapping missing")
                    })?;
                let site = self.inventory.calls.get(*source_index).ok_or_else(|| {
                    FactsError::invalid(self.budget, "input call source occurrence missing")
                })?;
                if site.node.child(9, index, self.budget)? != *node {
                    return Err(FactsError::invalid(
                        self.budget,
                        "input argument source occurrence changed",
                    ));
                }
                let ir::Callee::Direct(target) = target else {
                    return Err(FactsError::invalid(
                        self.budget,
                        "input call source target is not direct",
                    ));
                };
                let view = self.original.facts.original_callable_schema(
                    tr::OriginalCallableSubject::Direct(target.declaration),
                )?;
                let parameter = view.signature().params.get(index).ok_or_else(|| {
                    FactsError::invalid(self.budget, "input source parameter missing")
                })?;
                // Rigid declaration parameters are not inferred actuals. In
                // particular E<T> must not become an expected E<actual> while
                // reconstructing a generic aggregate argument.
                parameter.is_closed_shallow().then(|| parameter.clone())
            } else {
                None
            };
            self.expression(source, node, id, expected.or(declared.as_ref()), depth)
        }
    }
    fn equal(&self, a: &ir::Ty, b: &ir::Ty) -> Result<bool, FactsError> {
        self.original
            .facts
            .validate_ty_metadata_with_budget(a, self.budget)?;
        self.original
            .facts
            .validate_ty_metadata_with_budget(b, self.budget)?;
        sr::replay_types_equal(a, b, self.budget)
    }
    #[allow(clippy::too_many_arguments)]
    fn array_borrow_initializer(
        &self,
        index: usize,
        source: &Expr,
        node: &sr::SourceNode,
        id: ir::ExprId,
        annotation: Option<&Type>,
        depth: usize,
    ) -> Result<Option<ir::Ty>, FactsError> {
        sr::depth(self.budget, depth)?;
        if let Expr::BorrowValue { value, .. } = source
            && let Expr::Array(elements) = value.as_ref()
        {
            sr::work(self.budget, elements.len())?;
        }
        let Some(array) = shared_byte_array(source, annotation) else {
            return Ok(None);
        };
        validate_original_occurrence(self.original, node, self.budget)?;
        let mut receipt = None;
        for row in &self.original.array_borrows {
            sr::work(self.budget, 1)?;
            if row.binding == index && receipt.replace(row).is_some() {
                return Err(FactsError::invalid(
                    self.budget,
                    "duplicate byte-array borrow receipt",
                ));
            }
        }
        let row = receipt.ok_or_else(|| {
            FactsError::invalid(self.budget, "original byte-array borrow receipt missing")
        })?;
        sr::work(self.budget, row.node.path.len().max(node.path.len()))?;
        if row.node != *node || row.initializer != id {
            return Err(FactsError::invalid(
                self.budget,
                "byte-array borrow source occurrence changed",
            ));
        }
        let get = |id: ir::ExprId| {
            self.checked.exprs.get(id.0).ok_or_else(|| {
                FactsError::invalid(self.budget, "byte-array borrow expression missing")
            })
        };
        let wrapper = get(id)?;
        if !matches!(wrapper.kind, ir::ExprKind::ArrayToSlice { reference } if reference == row.reference)
        {
            return Err(FactsError::invalid(
                self.budget,
                "byte-array borrow unsizing changed",
            ));
        }
        sr::work(self.budget, self.original.unsize_mapping.len())?;
        if !self
            .original
            .unsize_mapping
            .iter()
            .any(|entry| entry.wrapper == id && entry.reference == row.reference)
        {
            return Err(FactsError::invalid(
                self.budget,
                "byte-array borrow lacks original unsizing",
            ));
        }
        let reference = get(row.reference)?;
        let ir::ExprKind::Block(block) = &reference.kind else {
            return Err(FactsError::invalid(
                self.budget,
                "byte-array temporary block changed",
            ));
        };
        if !matches!(block.statements.as_slice(), [ir::TypedStmt::Assign { local, value }] if *local == row.temporary && *value == row.value)
            || block.tail != Some(row.borrowed)
        {
            return Err(FactsError::invalid(
                self.budget,
                "byte-array temporary identities changed",
            ));
        }
        let borrowed = get(row.borrowed)?;
        if !matches!(&borrowed.kind, ir::ExprKind::Borrow { kind: ir::BorrowKind::Shared, place, timing: ir::BorrowTiming::Immediate }
            if place.local == row.temporary && place.projection.is_empty())
        {
            return Err(FactsError::invalid(
                self.budget,
                "byte-array temporary borrow changed",
            ));
        }
        let Expr::Array(elements) = array else {
            unreachable!();
        };
        sr::retain(self.budget, std::mem::size_of::<ir::Ty>() + 32)?;
        let array_type = ir::Ty::Array {
            element: Arc::new(ir::Ty::Int(ir::IntTy::U8)),
            len: elements.len(),
        };
        let actual_array = self.expression(
            array,
            &node.child(11, 0, self.budget)?,
            row.value,
            Some(&array_type),
            depth + 1,
        )?;
        let temporary = self.checked.locals.get(row.temporary.0).ok_or_else(|| {
            FactsError::invalid(self.budget, "byte-array temporary local missing")
        })?;
        let reference_type = self.reference(ir::BorrowKind::Shared, array_type.clone())?;
        sr::retain(self.budget, std::mem::size_of::<ir::Ty>() + 32)?;
        let slice = self.reference(
            ir::BorrowKind::Shared,
            ir::Ty::Slice {
                element: Arc::new(ir::Ty::Int(ir::IntTy::U8)),
            },
        )?;
        if !self.equal(&actual_array, &array_type)?
            || !self.equal(&temporary.ty, &array_type)?
            || !temporary.mutable
            || !self.equal(&reference.ty, &reference_type)?
            || !self.equal(&borrowed.ty, &reference_type)?
            || !self.equal(&wrapper.ty, &slice)?
            || [
                wrapper.completion,
                reference.completion,
                borrowed.completion,
            ]
            .iter()
            .any(|completion| *completion != ir::Completion::Returns)
        {
            return Err(FactsError::invalid(
                self.budget,
                "byte-array temporary type or completion changed",
            ));
        }
        Ok(Some(slice))
    }

    pub fn binding(&self, index: usize, depth: usize) -> Result<(ir::LocalId, ir::Ty), FactsError> {
        sr::depth(self.budget, depth)?;
        sr::work(self.budget, 1)?;
        let site = self
            .original
            .bindings
            .get(index)
            .ok_or_else(|| FactsError::invalid(self.budget, "input binding missing"))?;
        validate_original_occurrence(self.original, &site.node, self.budget)?;
        if let Some(parameter) = site.parameter {
            let (local, ty) = source_binding_type(
                self.original,
                index,
                self.checked,
                self.signatures,
                self.budget,
            )?;
            let function = self
                .original
                .plan
                .callable_body(&self.original.plan.program, self.original.declaration)
                .map_err(|e| source_error(self.budget, &e))?;
            let actual = self
                .checked
                .locals
                .get(local.0)
                .ok_or_else(|| FactsError::invalid(self.budget, "input parameter missing"))?;
            if actual.mutable != function.param_mut.get(parameter).copied().unwrap_or(false) {
                return Err(FactsError::invalid(
                    self.budget,
                    "input parameter mutability changed",
                ));
            }
            return Ok((local, ty));
        }
        let function = self
            .original
            .plan
            .callable_body(&self.original.plan.program, self.original.declaration)
            .map_err(|e| source_error(self.budget, &e))?;
        if let Some((_, path)) = &site.pattern {
            return self.pattern_binding(function, index, path, depth + 1);
        }
        let Some(Stmt::Let {
            value: source,
            mutable,
            ..
        }) = source_statement(function, self.inventory, &site.node, self.budget)?
        else {
            return Err(FactsError::unsupported(
                self.budget,
                "input evidence requires initialized source let",
            ));
        };
        let (local, value) = initializer(self.original, index, self.checked, self.budget)?;
        let declaration = self
            .checked
            .locals
            .get(local.0)
            .ok_or_else(|| FactsError::invalid(self.budget, "input checked local missing"))?;
        if declaration.mutable != *mutable {
            return Err(FactsError::invalid(
                self.budget,
                "input binding identity or mutability changed",
            ));
        }
        let annotation = site
            .annotation
            .as_ref()
            .map(|ty| {
                checked_local_ty(ty, self.signatures).map_err(|e| source_error(self.budget, &e))
            })
            .transpose()?;
        let node = site.node.child(3, 0, self.budget)?;
        // A written annotation is the binding's authoritative type: the
        // checker already required the initializer to conform to it, so the
        // root contributes the annotation without replaying the initializer.
        // Only reference-free annotations qualify: no loan can flow through.
        // Retained syntax alone does not establish a replay producer, notably
        // for trait-default methods without a nominal owner. Use the same
        // original-source admission proof as recursive argument selection.
        if let Some(annotation) = annotation.as_ref()
            && ir::carrier_locations(annotation, self.budget)
                .map_err(|e| source_error(self.budget, &e))?
                .is_empty()
            && !supported(
                source,
                &node,
                &self.original.plan,
                function,
                self.inventory,
                Some(self.original),
                self.budget,
                depth + 1,
            )?
        {
            if !self.equal(annotation, &declaration.ty)? {
                return Err(FactsError::invalid(
                    self.budget,
                    "input annotated binding type changed",
                ));
            }
            return Ok((local, annotation.clone()));
        }
        let ty = match self.array_borrow_initializer(
            index,
            source,
            &node,
            value,
            site.annotation.as_ref(),
            depth + 1,
        )? {
            Some(ty) => ty,
            None => self.expression(source, &node, value, annotation.as_ref(), depth + 1)?,
        };
        let annotation_matches = match annotation.as_ref() {
            Some(annotation) => self.equal(annotation, &ty)?,
            None => true,
        };
        if !self.equal(&ty, &declaration.ty)? || !annotation_matches {
            return Err(FactsError::invalid(
                self.budget,
                "input initializer type changed",
            ));
        }
        Ok((local, ty))
    }
    /// A root tuple-pattern binder: the root type is the replayed initializer
    /// element (or written annotation element) selected by the binder's exact
    /// tuple path, checked against the authenticated pattern correspondence.
    fn pattern_binding(
        &self,
        function: &Function,
        index: usize,
        path: &[usize],
        depth: usize,
    ) -> Result<(ir::LocalId, ir::Ty), FactsError> {
        sr::depth(self.budget, depth)?;
        let site = &self.original.bindings[index];
        let leaf = pattern_source_leaf(function, site, self.budget)?.ok_or_else(|| {
            FactsError::unsupported(
                self.budget,
                "input evidence requires a tuple initializer for a pattern binder",
            )
        })?;
        let local = mapped_source_binding(self.original, index, self.budget)?;
        let declaration = self
            .checked
            .locals
            .get(local.0)
            .ok_or_else(|| FactsError::invalid(self.budget, "input checked local missing"))?;
        if declaration.mutable != leaf.mutable {
            return Err(FactsError::invalid(
                self.budget,
                "input pattern binding mutability changed",
            ));
        }
        let value = checked_pattern_leaf_id(self.checked, local, path, self.budget)?;
        let annotation = leaf
            .annotation
            .map(|ty| {
                checked_local_ty(ty, self.signatures).map_err(|e| source_error(self.budget, &e))
            })
            .transpose()?;
        let ty = match annotation {
            Some(annotation)
                if ir::carrier_locations(&annotation, self.budget)
                    .map_err(|e| source_error(self.budget, &e))?
                    .is_empty() =>
            {
                annotation
            }
            annotation => self.expression(
                leaf.value,
                &leaf.node,
                value,
                annotation.as_ref(),
                depth + 1,
            )?,
        };
        if !self.equal(&ty, &declaration.ty)? {
            return Err(FactsError::invalid(
                self.budget,
                "input pattern initializer type changed",
            ));
        }
        Ok((local, ty))
    }
    fn operand(
        &self,
        node: &sr::SourceNode,
        depth: usize,
    ) -> Result<(ir::LocalId, ir::Ty), FactsError> {
        sr::work(self.budget, node.path.len())?;
        let binding = self
            .inventory
            .operand_nodes
            .get(&node.path)
            .ok_or_else(|| {
                FactsError::invalid(self.budget, "input lacks lexical operand authority")
            })?;
        validate_original_occurrence(self.original, node, self.budget)?;
        validate_original_occurrence(self.original, binding, self.budget)?;
        let index = source_binding_index(&self.original.bindings, binding, self.budget)?
            .ok_or_else(|| FactsError::invalid(self.budget, "input lexical binding missing"))?;
        let site = &self.original.bindings[index];
        let statement_path = match &site.pattern {
            Some(_) if site.node.path.len() >= 4 => &site.node.path[..4],
            _ => &site.node.path[..],
        };
        if site.parameter.is_none() && !precedes_path(statement_path, &site.node, node) {
            return Err(FactsError::invalid(
                self.budget,
                "input initializer refers to a later binding",
            ));
        }
        self.binding(index, depth + 1)
    }
    fn reference(&self, kind: ir::BorrowKind, referent: ir::Ty) -> Result<ir::Ty, FactsError> {
        sr::retain(self.budget, std::mem::size_of::<ir::Ty>() + 32)?;
        Ok(ir::Ty::Ref {
            kind,
            referent: Arc::new(referent),
        })
    }
    /// A closed source method header determines this result, not its checked
    /// receiver type or inferred call actuals. The retained source-call receipt
    /// authenticates dispatch, and lexical authority authenticates the root.
    #[allow(clippy::too_many_arguments)]
    fn concrete_method_result(
        &self,
        source: &Expr,
        node: &sr::SourceNode,
        id: ir::ExprId,
        target: &ir::Callee,
        signature: &ir::Signature,
        args: &[ir::ExprId],
        depth: usize,
    ) -> Result<ir::Ty, FactsError> {
        let Expr::MethodCall {
            receiver,
            name,
            type_args,
            args: source_args,
        } = source
        else {
            return Err(FactsError::invalid(
                self.budget,
                "method producer lost source syntax",
            ));
        };
        let ir::Callee::Direct(direct) = target else {
            return Err(FactsError::unsupported(
                self.budget,
                "input method requires a direct source declaration",
            ));
        };
        // A reference local can be the receiver value itself, rather than an
        // autoref of its local storage. Select that replay from the original
        // lexical binding, never from the checked receiver's inferred type.
        let shared_reference_local = if matches!(receiver.as_ref(), Expr::Var(path) if is_local_shaped_path(path))
            && matches!(
                self.original
                    .plan
                    .registry
                    .declaration(direct.declaration)
                    .map_err(|e| source_error(self.budget, &e))?
                    .receiver,
                ReceiverKind::Borrowed {
                    kind: ir::BorrowKind::Shared
                }
            ) {
            let receiver_node = node.child(10, 0, self.budget)?;
            matches!(
                self.operand(&receiver_node, depth + 1)?.1,
                ir::Ty::Ref {
                    kind: ir::BorrowKind::Shared,
                    ..
                }
            )
        } else {
            false
        };
        if shared_reference_local
            || !source_args.is_empty()
            || matches!(
                self.original
                    .plan
                    .registry
                    .declaration(direct.declaration)
                    .map_err(|e| source_error(self.budget, &e))?
                    .receiver,
                ReceiverKind::Borrowed {
                    kind: ir::BorrowKind::Mut
                }
            )
            || !direct.type_args.is_empty()
            || !matches!(receiver.as_ref(), Expr::Var(path) if is_local_shaped_path(path))
            || matches!(
                self.original
                    .plan
                    .registry
                    .declaration(direct.declaration)
                    .map_err(|e| source_error(self.budget, &e))?
                    .receiver,
                ReceiverKind::Value { .. }
            )
        {
            return self.generic_method_result(source, node, id, direct, signature, args, depth);
        }
        if !type_args.is_empty() || !source_args.is_empty() || !direct.type_args.is_empty() {
            return Err(FactsError::unsupported(
                self.budget,
                "input method requires a closed zero-argument declaration",
            ));
        }
        if !matches!(receiver.as_ref(), Expr::Var(path) if is_local_shaped_path(path)) {
            return Err(FactsError::unsupported(
                self.budget,
                "input method requires an original local receiver",
            ));
        }
        validate_original_occurrence(self.original, node, self.budget)?;
        let source_index = authored_call_index(self.inventory, node, self.budget)?;
        sr::work(self.budget, self.original.call_mapping.len())?;
        let (position, (_, expression, original_target)) = self
            .original
            .call_mapping
            .iter()
            .enumerate()
            .find(|(_, (index, _, _))| *index == source_index)
            .ok_or_else(|| {
                FactsError::invalid(self.budget, "method producer original receipt missing")
            })?;
        if *expression != id {
            return Err(FactsError::invalid(
                self.budget,
                "method producer expression identity changed",
            ));
        }
        validate_original_callee(original_target, target, self.budget)?;
        let view = self
            .original
            .facts
            .original_callable_schema(tr::OriginalCallableSubject::Direct(direct.declaration))?;
        view.validate_for(&self.original.facts)?;
        if view.generic_env().total_arity() != 0
            || !self.original.facts.is_source_callable(direct.declaration)?
        {
            return Err(FactsError::unsupported(
                self.budget,
                "input method requires a nongeneric source header",
            ));
        }
        let info = self
            .original
            .plan
            .registry
            .declaration(direct.declaration)
            .map_err(|e| source_error(self.budget, &e))?;
        if info.path.associated.is_none()
            || !self
                .original
                .plan
                .registry
                .source_member_equal(info.path.item, *name)
                .map_err(|e| source_error(self.budget, &e))?
            || !matches!(
                info.receiver,
                ReceiverKind::Borrowed {
                    kind: ir::BorrowKind::Shared
                }
            )
        {
            return Err(FactsError::unsupported(
                self.budget,
                "input method requires an exact shared source receiver",
            ));
        }
        let canonical = nominal_call_receipts::original_call_signature(
            self.original,
            position,
            id,
            target,
            signature,
            &self.original.facts,
            self.budget,
            &mut tr::TraitScratch::default(),
        )?;
        let declared = view.signature();
        if declared.params.len() != 1
            || canonical.params.len() != 1
            || args.len() != 1
            || !self.equal(&declared.params[0], &canonical.params[0])?
            || !self.equal(&declared.result, &canonical.result)?
            || declared.ref_contract != canonical.ref_contract
        {
            return Err(FactsError::invalid(
                self.budget,
                "method producer declaration signature changed",
            ));
        }
        let ir::Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent,
        } = &declared.params[0]
        else {
            return Err(FactsError::invalid(
                self.budget,
                "method producer source receiver is not shared",
            ));
        };
        let receiver_node = node.child(10, 0, self.budget)?;
        validate_original_occurrence(self.original, &receiver_node, self.budget)?;
        let binding = self
            .inventory
            .operand_nodes
            .get(&receiver_node.path)
            .ok_or_else(|| {
                FactsError::invalid(self.budget, "method receiver lacks lexical authority")
            })?;
        validate_original_occurrence(self.original, binding, self.budget)?;
        let binding_index = source_binding_index(&self.original.bindings, binding, self.budget)?
            .ok_or_else(|| {
                FactsError::invalid(self.budget, "method receiver source binding missing")
            })?;
        let site = &self.original.bindings[binding_index];
        if site.parameter.is_none() && !precedes(&site.node, &receiver_node) {
            return Err(FactsError::invalid(
                self.budget,
                "method receiver binding does not precede use",
            ));
        }
        let local = if site.parameter.is_some() || site.statement.is_some() {
            mapped_source_binding(self.original, binding_index, self.budget)?
        } else {
            // Nested lets have a distinct occurrence-and-initializer receipt,
            // not a root statement mapping. Validate that receipt exactly.
            initializer(self.original, binding_index, self.checked, self.budget)?.0
        };
        let actual = self.checked.exprs.get(args[0].0).ok_or_else(|| {
            FactsError::invalid(self.budget, "method receiver outside checked arena")
        })?;
        if !matches!(&actual.kind, ir::ExprKind::Borrow {
            kind: ir::BorrowKind::Shared, place, timing: ir::BorrowTiming::Immediate,
        } if place.local == local && place.projection.is_empty())
            || !self.equal(&declared.params[0], &actual.ty)?
        {
            return Err(FactsError::invalid(
                self.budget,
                "method receiver changed its original root or adjustment",
            ));
        }
        let local = self
            .checked
            .locals
            .get(local.0)
            .ok_or_else(|| FactsError::invalid(self.budget, "method receiver local missing"))?;
        if !self.equal(referent, &local.ty)? {
            return Err(FactsError::invalid(
                self.budget,
                "method receiver differs from its source declaration header",
            ));
        }
        Ok(declared.result.clone())
    }

    /// Replay the exact declaration-owned transfer of one shared reference
    /// actual from one root input into one result component. No spelling, raw
    /// checked actual, or independently supplied contract authorizes this path.
    #[allow(clippy::too_many_arguments)]
    fn shared_static_result_transfer(
        &self,
        target: &ir::Callee,
        schema: &tr::OriginalCallableView,
        env: &ir::ParamEnv,
        actuals: &[ir::Ty],
        inputs: &[ir::Ty],
        results: &[ir::CarrierLocation],
        canonical: &ir::Signature,
        depth: usize,
    ) -> Result<(), FactsError> {
        sr::depth(self.budget, depth)?;
        if env.arity() != 1
            || actuals.len() != 1
            || results.len() != 1
            || !matches!(
                &actuals[0],
                ir::Ty::Ref {
                    kind: ir::BorrowKind::Shared,
                    ..
                }
            )
        {
            return Err(FactsError::unsupported(
                self.budget,
                "generic static result requires one authenticated shared carrier",
            ));
        }
        let actual_locations = ir::carrier_locations(&actuals[0], self.budget)
            .map_err(|e| source_error(self.budget, &e))?;
        let root = ir::CarrierLocation {
            path: ir::CarrierPath(Arc::from([])),
            channel: ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 0 }),
        };
        if actual_locations.as_ref() != std::slice::from_ref(&root) {
            return Err(FactsError::unsupported(
                self.budget,
                "generic static result requires a single-layer shared actual",
            ));
        }
        // The target was authenticated against the immutable original call
        // receipt, and its actuals were independently inferred and compared.
        let transfer = self
            .original
            .facts
            .carrier_contract(ir::SignatureSubject::Call(target))?;
        transfer.validate_for(&self.original.facts)?;
        let ir::Callee::Direct(direct) = target else {
            return Err(FactsError::invalid(
                self.budget,
                "static transfer lost direct target",
            ));
        };
        sr::work(
            self.budget,
            schema.generic_env().total_arity() + actuals.len(),
        )?;
        if transfer.declaration() != direct.declaration
            || transfer.arguments().len() != actuals.len()
            || transfer.original_env().total_arity() != schema.generic_env().total_arity()
            || !transfer
                .original_env()
                .ordered_params()
                .eq(schema.generic_env().ordered_params())
        {
            return Err(FactsError::invalid(
                self.budget,
                "generic static result transfer changed original identity or binder",
            ));
        }
        for (actual, retained) in actuals.iter().zip(transfer.arguments()) {
            if !self.equal(actual, retained)? {
                return Err(FactsError::invalid(
                    self.budget,
                    "generic static result transfer changed independently inferred actual",
                ));
            }
        }
        let original = transfer.original_contract();
        let mapped = transfer.contract();
        if original.inputs.len() != 1
            || original.results.len() != 1
            || original.results[0].sources.len() != 1
            || mapped.inputs.len() != 1
            || mapped.results.len() != 1
            || mapped.results[0].sources.len() != 1
            || !mapped.writes.is_empty()
            || transfer.component_mappings().len() != 1
            || transfer.source_region_env().region_count != 0
            || !transfer.source_region_env().carriers.is_empty()
            || !transfer.source_region_env().outlives.is_empty()
            || !transfer.region_schema().constraints.is_empty()
        {
            return Err(FactsError::unsupported(
                self.budget,
                "generic static result requires exact single-source declaration transfer",
            ));
        }
        let parameter = ir::ParamId {
            owner: env.binder(),
            index: 0,
        };
        let tail = ir::CarrierChannel::ParamTail {
            parameter,
            prefix: 0,
        };
        let original_input = &original.inputs[0];
        let original_result = &original.results[0];
        let mapped_input = &mapped.inputs[0];
        let mapped_result = &mapped.results[0];
        let component = &transfer.component_mappings()[0];
        sr::work(
            self.budget,
            original_result.location.path.0.len()
                + mapped_result.location.path.0.len()
                + component.original.path.0.len()
                + component.substituted.path.0.len()
                + inputs.len(),
        )?;
        if !original_input.location.path.0.is_empty()
            || original_input.location.channel != tail
            || original_result.location.channel != tail
            || original_result.location.path.0.is_empty()
            || original_result.sources[0] != *original_input
            || mapped_input.param != original_input.param
            || mapped_input.location != root
            || mapped_result.location != results[0]
            || mapped_result.location.path != original_result.location.path
            || mapped_result.location.channel != root.channel
            || mapped_result.sources[0] != *mapped_input
            || component.original_family != ir::CarrierFamily::Parameter(parameter)
            || component.original != original_input.location
            || component.substituted != root
            || transfer.ref_contract().as_ref() != canonical.ref_contract.as_ref()
        {
            return Err(FactsError::invalid(
                self.budget,
                "generic static result changed exact original carrier transfer",
            ));
        }
        let source = inputs.get(mapped_input.param).ok_or_else(|| {
            FactsError::invalid(self.budget, "generic static transfer source input missing")
        })?;
        if schema.signature().params.get(mapped_input.param) != Some(&ir::Ty::Param(parameter))
            || !self.equal(source, &actuals[0])?
        {
            return Err(FactsError::invalid(
                self.budget,
                "generic static transfer source differs from authenticated input",
            ));
        }
        for (index, input) in inputs.iter().enumerate() {
            let locations = ir::carrier_locations(input, self.budget)
                .map_err(|e| source_error(self.budget, &e))?;
            if (index == mapped_input.param && locations.as_ref() != std::slice::from_ref(&root))
                || (index != mapped_input.param && !locations.is_empty())
            {
                return Err(FactsError::unsupported(
                    self.budget,
                    "generic static transfer contains additional input carriers",
                ));
            }
        }
        Ok(())
    }

    /// Infer an omitted generic owner only from authored result context and
    /// recursively authenticated original inputs. Reference results additionally
    /// require the narrow declaration-owned carrier transfer above.
    #[allow(clippy::too_many_arguments)]
    fn generic_static_result(
        &self,
        owner: &Type,
        name: &AstIdent,
        node: &sr::SourceNode,
        inputs: &[Expr],
        id: ir::ExprId,
        direct: &ir::CallTarget,
        signature: &ir::Signature,
        arguments: &[ir::ExprId],
        view: &tr::OriginalCallableView,
        info: &DeclInfo,
        source_index: usize,
        position: usize,
        declared_expected: Option<&ir::Ty>,
        depth: usize,
    ) -> Result<ir::Ty, FactsError> {
        sr::depth(self.budget, depth)?;
        let associated = info.path.associated.as_ref().ok_or_else(|| {
            FactsError::invalid(self.budget, "generic static source owner missing")
        })?;
        if info.param_env.arity() != 0 {
            return Err(FactsError::unsupported(
                self.budget,
                "generic static producer requires an empty own binder",
            ));
        }
        if inputs.len() != arguments.len()
            || inputs.len() != view.signature().params.len()
            || inputs.len() != signature.params.len()
        {
            return Err(FactsError::invalid(
                self.budget,
                "generic static producer changed original input arity",
            ));
        }
        let Type::Apply(path) = owner else {
            return Err(FactsError::unsupported(
                self.budget,
                "generic static producer requires original omitted owner actuals",
            ));
        };
        let (_, written) = path
            .application()
            .map_err(|e| source_error(self.budget, &e))?;
        sr::work(self.budget, written.len())?;
        if written.is_empty() || written.iter().any(|ty| !matches!(ty, Type::Infer)) {
            return Err(FactsError::unsupported(
                self.budget,
                "generic static producer requires exclusively omitted owner actuals",
            ));
        }
        let context = self
            .original
            .plan
            .callable_context(self.original.declaration)
            .map_err(|e| source_error(self.budget, &e))?;
        let checker = Checker::new(self.signatures, ir::Ty::Unit, context.module);
        let owner_path = path
            .path(checker.names().map_err(|e| source_error(self.budget, &e))?)
            .map_err(|e| source_error(self.budget, &e))?;
        let owner_key = checker
            .path_key(&owner_path, Namespace::Type)
            .map_err(|e| source_error(self.budget, &e))?;
        if self.original.plan.registry.nominals.get(&owner_key) != Some(&associated.nominal)
            || !self
                .original
                .plan
                .registry
                .source_member_equal(info.path.item, *name)
                .map_err(|e| source_error(self.budget, &e))?
        {
            return Err(FactsError::invalid(
                self.budget,
                "generic static producer changed original owner or member",
            ));
        }
        let mut env = None;
        for scope in view.generic_env().scopes() {
            sr::work(self.budget, 1)?;
            if scope.arity() == 0 {
                continue;
            }
            if scope.binder().declaration != associated.impl_decl
                || env.replace(scope.clone()).is_some()
            {
                return Err(FactsError::unsupported(
                    self.budget,
                    "generic static producer requires exactly the original owner binder",
                ));
            }
        }
        let env = env.ok_or_else(|| {
            FactsError::invalid(self.budget, "generic static producer owner binder missing")
        })?;
        // Omitted owner slots follow the nominal's own parameters (including
        // defaulted ones), while the impl binder may fix some of them
        // (`impl<K, V> HashMap<K, V, RandomState>`). Each arity is exact.
        let nominal_arity = self
            .original
            .plan
            .param_env(associated.nominal)
            .map_err(|e| source_error(self.budget, &e))?
            .arity();
        if nominal_arity != written.len() || env.arity() != direct.type_args.len() {
            return Err(FactsError::invalid(
                self.budget,
                "generic static producer owner arity changed",
            ));
        }
        let site = self.inventory.calls.get(source_index).ok_or_else(|| {
            FactsError::invalid(
                self.budget,
                "generic static producer source occurrence missing",
            )
        })?;
        let expected = source_result_context(
            self.original,
            site,
            id,
            self.checked,
            self.signatures,
            self.budget,
        )?;
        let target = ir::Callee::Direct(direct.clone());
        let canonical = nominal_call_receipts::original_call_signature(
            self.original,
            position,
            id,
            &target,
            signature,
            &self.original.facts,
            self.budget,
            &mut tr::TraitScratch::default(),
        )?;
        sr::work(self.budget, self.checked.call_evidence.len())?;
        let evidence = self
            .checked
            .call_evidence
            .iter()
            .find(|row| row.call == id)
            .ok_or_else(|| {
                FactsError::invalid(
                    self.budget,
                    "generic static producer argument trace missing",
                )
            })?;
        if canonical.params.len() != inputs.len() || evidence.arguments.len() != inputs.len() {
            return Err(FactsError::invalid(
                self.budget,
                "generic static producer original input arity changed",
            ));
        }
        let mut state = sr::CallInferenceState::new(Some(env.clone()), self.budget)?;
        let result_schema =
            state.type_term(&view.signature().result, Some(env.binder()), depth + 1)?;
        if let Some(expected) = declared_expected {
            // Recursive constructor replay supplies original declared field
            // types, never a checked constructor tag or inferred call result.
            let expected = state.type_term(expected, None, depth + 1)?;
            state.constrain(&result_schema, &expected, depth + 1)?;
        }
        if let Some(expected) = &expected {
            // Context may intentionally contain unresolved receiver terms.
            // Constrain that authored term directly, never materialize it or
            // seed it from the checked call result or inferred actuals.
            state.constrain(&result_schema, expected, depth + 1)?;
        }
        let mut input_types = Vec::new();
        for (index, (source, argument)) in inputs.iter().zip(arguments).enumerate() {
            let parameter = &view.signature().params[index];
            let expectation = match parameter {
                ir::Ty::Ref { kind, .. } => ir::CallExpectation::FirmReference { kind: *kind },
                _ => ir::CallExpectation::NoFirmReference,
            };
            let input = self.argument(
                source,
                &node.child(9, index, self.budget)?,
                *argument,
                id,
                index,
                expectation,
                parameter.is_closed_shallow().then_some(parameter),
                depth + 1,
            )?;
            let row = &evidence.arguments[index];
            if !self.equal(&input, &canonical.params[index])?
                || !self.equal(&input, &signature.params[index])?
                || row.argument != index
                || row.value != *argument
            {
                return Err(FactsError::invalid(
                    self.budget,
                    "generic static producer changed original input identity or type",
                ));
            }
            state.constrain_input(parameter, &input)?;
            sr::push(&mut input_types, input, self.budget)?;
        }
        let mut materialize = |decl, args: &[ir::Ty]| {
            self.original
                .plan
                .intern_nominal(
                    decl,
                    args,
                    ir::FactsCtx::Validated(&self.original.facts),
                    self.budget,
                )
                .map_err(|e| source_error(self.budget, &e))
        };
        let actuals = state.finish(false, &mut materialize)?.ok_or_else(|| {
            FactsError::unsupported(
                self.budget,
                "generic static source actuals remain unresolved",
            )
        })?;
        for (authored, checked) in actuals.iter().zip(direct.type_args.iter()) {
            if !self.equal(authored, checked)? {
                return Err(FactsError::invalid(
                    self.budget,
                    "generic static producer changed independently inferred actuals",
                ));
            }
        }
        // Schema Slots are not concrete types. Instantiate the original
        // declaration only with the independently inferred actual vector.
        sr::retain(
            self.budget,
            actuals.len().saturating_mul(std::mem::size_of::<ir::Ty>()) + 32,
        )?;
        let inferred_target = ir::CallTarget {
            declaration: direct.declaration,
            type_args: actuals.into(),
        };
        let inferred = self
            .original
            .plan
            .instantiate_signature(
                &inferred_target,
                ir::FactsCtx::Validated(&self.original.facts),
                self.budget,
            )
            .map_err(|e| source_error(self.budget, &e))?;
        if inferred.params.len() != inputs.len() || inferred.ref_contract != canonical.ref_contract
        {
            return Err(FactsError::invalid(
                self.budget,
                "generic static producer changed independently instantiated contract",
            ));
        }
        // Associated-call checking infers and instantiates the owner before
        // checking arguments. Unlike an inferred ordinary free call, an owner
        // slot instantiated to a reference supplies a firm reference context.
        // Reconstruct that context only AFTER independent source inference.
        for (index, (source, argument)) in inputs.iter().zip(arguments).enumerate() {
            let parameter = &inferred.params[index];
            let expectation = match parameter {
                ir::Ty::Ref { kind, .. } => ir::CallExpectation::FirmReference { kind: *kind },
                _ => ir::CallExpectation::NoFirmReference,
            };
            let input = self.argument(
                source,
                &node.child(9, index, self.budget)?,
                *argument,
                id,
                index,
                expectation,
                Some(parameter),
                depth + 1,
            )?;
            let row = &evidence.arguments[index];
            if !self.equal(&input, &input_types[index])?
                || !self.equal(&input, parameter)?
                || !self.equal(parameter, &canonical.params[index])?
                || !self.equal(parameter, &signature.params[index])?
                || row.argument != index
                || row.value != *argument
                || row.expectation != expectation
            {
                return Err(FactsError::invalid(
                    self.budget,
                    "generic static producer changed independently reconstructed input expectation",
                ));
            }
        }
        let result = inferred.result;
        let actuals = inferred_target.type_args.as_ref();
        if !self.equal(&result, &canonical.result)? {
            return Err(FactsError::invalid(
                self.budget,
                "generic static producer changed independently inferred result",
            ));
        }
        let result_locations = ir::carrier_locations(&result, self.budget)
            .map_err(|e| source_error(self.budget, &e))?;
        let mut expected_regions = Vec::new();
        let mut actual_has_carriers = false;
        for (argument, actual) in actuals.iter().enumerate() {
            let locations = ir::carrier_locations(actual, self.budget)
                .map_err(|e| source_error(self.budget, &e))?;
            actual_has_carriers |= !locations.is_empty();
            for location in locations.iter() {
                sr::work(self.budget, 1)?;
                if matches!(location.channel, ir::CarrierChannel::Fixed(_)) {
                    // Both owner and own actuals are omitted in this route.
                    // No authored lifetime occurrence is erased or fabricated.
                    sr::push(
                        &mut expected_regions,
                        ir::TypeActualRegion {
                            argument,
                            location: location.clone(),
                            source: ir::ActualRegion::Elided,
                        },
                        self.budget,
                    )?;
                }
            }
        }
        if expected_regions.as_slice() != evidence.type_actual_regions.as_ref() {
            return Err(FactsError::invalid(
                self.budget,
                "generic static producer changed original actual-region occurrences",
            ));
        }
        let mut inputs_carry_references = false;
        for input in &input_types {
            inputs_carry_references |= !ir::carrier_locations(input, self.budget)
                .map_err(|e| source_error(self.budget, &e))?
                .is_empty();
        }
        // With no reference-carrying input (`HashMap::<&'static str, _>::new()`)
        // the result's reference carriers cannot borrow from the caller: the
        // borrow analysis issues them no loan. Only the declaration's exact
        // contract is replayed, as for a reference-free result.
        if (result_locations.is_empty()
            && !actual_has_carriers
            && !contains_source_ref(&result, self.budget, depth + 1)?)
            || !inputs_carry_references
        {
            if view.signature().ref_contract != canonical.ref_contract {
                return Err(FactsError::invalid(
                    self.budget,
                    "generic static producer changed reference-free result contract",
                ));
            }
        } else {
            self.shared_static_result_transfer(
                &target,
                view,
                &env,
                actuals,
                &input_types,
                &result_locations,
                canonical,
                depth + 1,
            )?;
        }
        ir::validate_callee(
            &target,
            signature,
            ir::FactsCtx::Validated(&self.original.facts),
        )
        .map_err(|e| source_error(self.budget, &e))?;
        Ok(result)
    }

    /// Closed static inherent calls are producers, not inference binders. Their
    /// authored owner/member, declaration header and original call receipt must
    /// all agree before their recursively authenticated arguments are consumed.
    #[allow(clippy::too_many_arguments)]
    fn closed_static_result(
        &self,
        source: &Expr,
        node: &sr::SourceNode,
        id: ir::ExprId,
        target: &ir::Callee,
        signature: &ir::Signature,
        arguments: &[ir::ExprId],
        expected: Option<&ir::Ty>,
        depth: usize,
    ) -> Result<ir::Ty, FactsError> {
        let Expr::AssociatedCall {
            owner,
            name,
            type_args,
            args,
        } = source
        else {
            return Err(FactsError::invalid(
                self.budget,
                "static producer lost source syntax",
            ));
        };
        let ir::Callee::Direct(direct) = target else {
            return Err(FactsError::unsupported(
                self.budget,
                "static input producer requires direct source declaration",
            ));
        };
        if !type_args.is_empty() {
            return Err(FactsError::unsupported(
                self.budget,
                "static input producer requires a closed source header",
            ));
        }
        validate_original_occurrence(self.original, node, self.budget)?;
        let source_index = authored_call_index(self.inventory, node, self.budget)?;
        sr::work(self.budget, self.original.call_mapping.len())?;
        let (position, (_, expression, original_target)) = self
            .original
            .call_mapping
            .iter()
            .enumerate()
            .find(|(_, (index, _, _))| *index == source_index)
            .ok_or_else(|| {
                FactsError::invalid(self.budget, "static producer original receipt missing")
            })?;
        if *expression != id {
            return Err(FactsError::invalid(
                self.budget,
                "static producer expression identity changed",
            ));
        }
        validate_original_callee(original_target, target, self.budget)?;
        let view = self
            .original
            .facts
            .original_callable_schema(tr::OriginalCallableSubject::Direct(direct.declaration))?;
        view.validate_for(&self.original.facts)?;
        let info = self
            .original
            .plan
            .registry
            .declaration(direct.declaration)
            .map_err(|e| source_error(self.budget, &e))?;
        let associated = info.path.associated.as_ref().ok_or_else(|| {
            FactsError::invalid(self.budget, "static producer lacks associated source owner")
        })?;
        // A direct trait-impl associated function (`Dog::from("a")` through
        // `impl From<&str> for Dog`) is a closed producer like an inherent
        // one: the owner, member, header and receipt checks below are the
        // same. Only the original direct callee is replayed, never dispatch.
        if info.kind != ir::DeclKind::AssociatedFunction
            || !matches!(info.receiver, ReceiverKind::None)
            || !self.original.facts.is_source_callable(direct.declaration)?
        {
            return Err(FactsError::unsupported(
                self.budget,
                "static input producer requires a closed inherent value result",
            ));
        }
        if view.generic_env().total_arity() != 0 {
            return self.generic_static_result(
                owner,
                name,
                node,
                args,
                id,
                direct,
                signature,
                arguments,
                &view,
                info,
                source_index,
                position,
                expected,
                depth + 1,
            );
        }
        if !direct.type_args.is_empty()
            || contains_source_ref(&view.signature().result, self.budget, depth + 1)?
        {
            return Err(FactsError::unsupported(
                self.budget,
                "static input producer requires a closed inherent value result",
            ));
        }
        let owner =
            checked_local_ty(owner, self.signatures).map_err(|e| source_error(self.budget, &e))?;
        if !self
            .original
            .plan
            .registry
            .source_member_equal(info.path.item, *name)
            .map_err(|e| source_error(self.budget, &e))?
            || !self.equal(&owner, &associated.self_pattern)?
        {
            return Err(FactsError::invalid(
                self.budget,
                "static producer changed original source owner or member",
            ));
        }
        let canonical = nominal_call_receipts::original_call_signature(
            self.original,
            position,
            id,
            target,
            signature,
            &self.original.facts,
            self.budget,
            &mut tr::TraitScratch::default(),
        )?;
        let declared = view.signature();
        if args.len() != arguments.len()
            || args.len() != declared.params.len()
            || args.len() != canonical.params.len()
            || args.len() != signature.params.len()
            || !self.equal(&declared.result, &canonical.result)?
            || declared.ref_contract != canonical.ref_contract
        {
            return Err(FactsError::invalid(
                self.budget,
                "static producer declaration signature changed",
            ));
        }
        sr::work(self.budget, self.checked.call_evidence.len())?;
        let evidence = self
            .checked
            .call_evidence
            .iter()
            .find(|row| row.call == id)
            .ok_or_else(|| {
                FactsError::invalid(self.budget, "static producer argument trace missing")
            })?;
        if evidence.arguments.len() != arguments.len() || !evidence.type_actual_regions.is_empty() {
            return Err(FactsError::invalid(
                self.budget,
                "static producer changed closed argument evidence",
            ));
        }
        for (index, (source, argument)) in args.iter().zip(arguments).enumerate() {
            let parameter = &declared.params[index];
            let expectation = match parameter {
                ir::Ty::Ref { kind, .. } => ir::CallExpectation::FirmReference { kind: *kind },
                _ => ir::CallExpectation::NoFirmReference,
            };
            let input = self.argument(
                source,
                &node.child(9, index, self.budget)?,
                *argument,
                id,
                index,
                expectation,
                Some(parameter),
                depth + 1,
            )?;
            let row = &evidence.arguments[index];
            if !self.equal(&input, parameter)?
                || !self.equal(parameter, &canonical.params[index])?
                || !self.equal(parameter, &signature.params[index])?
                || row.argument != index
                || row.value != *argument
                || row.expectation != expectation
            {
                return Err(FactsError::invalid(
                    self.budget,
                    "static producer changed original argument or parameter",
                ));
            }
        }
        ir::validate_callee(
            target,
            signature,
            ir::FactsCtx::Validated(&self.original.facts),
        )
        .map_err(|e| source_error(self.budget, &e))?;
        Ok(declared.result.clone())
    }

    /// The result is the builtin's fixed usize, never a checked inference seed.
    /// This deliberately supports only an unadjusted shared descriptor local.
    /// Other len receiver shapes stay on their existing canonical proof path.
    fn builtin_len_result(
        &self,
        node: &sr::SourceNode,
        id: ir::ExprId,
        receiver: ir::ExprId,
        depth: usize,
    ) -> Result<ir::Ty, FactsError> {
        validate_original_occurrence(self.original, node, self.budget)?;
        let index = authored_call_index(self.inventory, node, self.budget)?;
        let site = &self.inventory.calls[index];
        if !site.len_candidate
            || site.for_iterable
            || site.ordinary.is_some()
            || site.application.is_some()
        {
            return Err(FactsError::invalid(
                self.budget,
                "len source candidate changed",
            ));
        }
        sr::work(self.budget, self.original.builtin_len_mapping.len())?;
        let mut rows = self
            .original
            .builtin_len_mapping
            .iter()
            .filter(|row| row.source_index == index);
        let row = rows.next().ok_or_else(|| {
            FactsError::invalid(self.budget, "len input lacks canonical builtin receipt")
        })?;
        if rows.next().is_some() || row.expr != id || row.receiver != receiver {
            return Err(FactsError::invalid(
                self.budget,
                "len input occurrence changed",
            ));
        }
        let receiver_node = node.child(10, 0, self.budget)?;
        let (local, source_ty) = self.operand(&receiver_node, depth + 1)?;
        if source_ty.shared_descriptor_element().is_none() {
            return Err(FactsError::unsupported(
                self.budget,
                "len input requires an unadjusted shared descriptor source",
            ));
        }
        let actual = self
            .checked
            .exprs
            .get(receiver.0)
            .ok_or_else(|| FactsError::invalid(self.budget, "len input receiver missing"))?;
        if !matches!(actual.kind, ir::ExprKind::Local(root) if root == local)
            || actual.completion != ir::Completion::Returns
            || !self.equal(&source_ty, &actual.ty)?
        {
            return Err(FactsError::invalid(
                self.budget,
                "len input lexical receiver changed",
            ));
        }
        Ok(ir::Ty::Int(ir::IntTy::Usize))
    }

    pub(super) fn expression(
        &self,
        source: &Expr,
        node: &sr::SourceNode,
        id: ir::ExprId,
        expected: Option<&ir::Ty>,
        depth: usize,
    ) -> Result<ir::Ty, FactsError> {
        sr::depth(self.budget, depth)?;
        sr::work(self.budget, 1)?;
        let actual = self
            .checked
            .exprs
            .get(id.0)
            .ok_or_else(|| FactsError::invalid(self.budget, "input expression missing"))?;
        let ty = match (source, &actual.kind) {
            (Expr::Block(source), ir::ExprKind::Block(block)) => {
                self.lexical_block(source, node, block, expected, depth + 1)?
            }
            (Expr::Var(_), _)
                if closure_value::provider(&self.original.plan, source, self.budget)?.is_some() =>
            {
                closure_value::replay(self.original, source, actual, self.checked, self.budget)?
            }
            (Expr::Float(literal), ir::ExprKind::Float(value)) => {
                let width = literal.suffix.ok_or_else(|| {
                    FactsError::unsupported(self.budget, "input float requires authored width")
                })?;
                let resolved = literal
                    .value(width, false)
                    .map_err(|e| source_error(self.budget, &e))?;
                if resolved != *value {
                    return Err(FactsError::invalid(
                        self.budget,
                        "input float literal changed",
                    ));
                }
                ir::Ty::Float(width)
            }
            (
                Expr::Binary {
                    op, left, right, ..
                },
                ir::ExprKind::Binary {
                    op: actual_op,
                    left: actual_left,
                    right: actual_right,
                },
            ) if authored_float_width(source, self.budget, depth)?.is_some() => {
                if op != actual_op {
                    return Err(FactsError::invalid(
                        self.budget,
                        "input float operator changed",
                    ));
                }
                let left = self.expression(
                    left,
                    &node.child(13, 0, self.budget)?,
                    *actual_left,
                    None,
                    depth + 1,
                )?;
                let right = self.expression(
                    right,
                    &node.child(13, 1, self.budget)?,
                    *actual_right,
                    None,
                    depth + 1,
                )?;
                if !self.equal(&left, &right)? {
                    return Err(FactsError::invalid(
                        self.budget,
                        "input float operand width changed",
                    ));
                }
                left
            }
            (Expr::Apply { value, .. }, _) => self.expression(
                value,
                &node.child(7, 0, self.budget)?,
                id,
                expected,
                depth + 1,
            )?,
            (_, ir::ExprKind::ArrayToSlice { reference }) => {
                sr::work(self.budget, self.original.unsize_mapping.len())?;
                if !self
                    .original
                    .unsize_mapping
                    .iter()
                    .any(|row| row.wrapper == id && row.reference == *reference)
                {
                    return Err(FactsError::invalid(
                        self.budget,
                        "input slice coercion lacks original correspondence",
                    ));
                }
                let source_ty = self.expression(source, node, *reference, None, depth + 1)?;
                let Some((kind, element, _)) = source_ty.array_ref_element_len() else {
                    return Err(FactsError::invalid(
                        self.budget,
                        "input slice coercion source is not an array reference",
                    ));
                };
                self.reference(
                    kind,
                    ir::Ty::Slice {
                        element: Arc::new(element.clone()),
                    },
                )?
            }
            (_, ir::ExprKind::RefToDyn { reference }) => {
                // Reference unsizing at an argument coercion site. The wrapper
                // must be the recorded original occurrence, its operand must
                // replay to a reference of the same kind, and the concrete
                // referent must again prove the dynamic principal.
                sr::work(self.budget, self.original.unsize_mapping.len())?;
                if !self
                    .original
                    .unsize_mapping
                    .iter()
                    .any(|row| row.wrapper == id && row.reference == *reference)
                {
                    return Err(FactsError::invalid(
                        self.budget,
                        "input object coercion lacks original correspondence",
                    ));
                }
                let Some((want_kind, object)) = super::type_check::trait_object_target(&actual.ty)
                else {
                    return Err(FactsError::unsupported(
                        self.budget,
                        "input object coercion requires a reference target",
                    ));
                };
                let source_ty = self.expression(source, node, *reference, None, depth + 1)?;
                let ir::Ty::Ref { kind, referent } = &source_ty else {
                    return Err(FactsError::invalid(
                        self.budget,
                        "input object coercion source is not a reference",
                    ));
                };
                if *kind != want_kind || matches!(referent.as_ref(), ir::Ty::Dyn(_)) {
                    return Err(FactsError::invalid(
                        self.budget,
                        "input object coercion changed reference kind or source",
                    ));
                }
                self.original
                    .facts
                    .prove_dynamic_unsize(referent, &object)?;
                self.reference(want_kind, ir::Ty::Dyn(object.clone()))?
            }
            (source @ Expr::Borrow { place, .. }, ir::ExprKind::Block(block))
                if !place.fields.is_empty() =>
            {
                self.projected_borrow(source, node, id, block, depth + 1)?
            }
            (source @ Expr::BorrowValue { .. }, _)
                if indexed_borrow::authored_index_borrow(source).is_some() =>
            {
                self.index_borrow(source, node, id, depth + 1)?
            }
            (source @ Expr::BorrowValue { .. }, ir::ExprKind::Block(block)) => {
                self.projected_borrow(source, node, id, block, depth + 1)?
            }
            (source @ Expr::Borrow { place, .. }, ir::ExprKind::Call { .. })
                if place.fields.is_empty() =>
            {
                self.explicit_borrow_coercion(source, node, id, depth + 1)?
            }
            (Expr::Borrow { place, .. }, ir::ExprKind::Call { .. }) if !place.fields.is_empty() => {
                return Err(FactsError::invalid(
                    self.budget,
                    "projected borrow cannot omit its carrier block and projected tail",
                ));
            }
            (
                Expr::Call { .. },
                ir::ExprKind::Call {
                    target,
                    signature,
                    args,
                    ..
                },
            ) => {
                let ir::Callee::Direct(direct) = target else {
                    return Err(FactsError::unsupported(
                        self.budget,
                        "input producer requires direct source callable",
                    ));
                };
                let source_index = authored_call_index(self.inventory, node, self.budget)?;
                sr::work(self.budget, self.original.call_mapping.len())?;
                let (position, (_, expression, original_target)) = self
                    .original
                    .call_mapping
                    .iter()
                    .enumerate()
                    .find(|(_, (index, _, _))| *index == source_index)
                    .ok_or_else(|| {
                        FactsError::invalid(self.budget, "producer original call mapping missing")
                    })?;
                if *expression != id {
                    return Err(FactsError::invalid(
                        self.budget,
                        "producer expression identity changed",
                    ));
                }
                validate_original_callee(original_target, target, self.budget)?;
                require_ordinary_free(&self.original.plan, direct.declaration, self.budget)?;
                let site = &self.inventory.calls[source_index];
                if site.ordinary.is_none() {
                    return Err(FactsError::unsupported(
                        self.budget,
                        "producer requires ordinary source call",
                    ));
                }
                let inputs = self
                    .inventory
                    .argument_syntax
                    .get(&source_index)
                    .ok_or_else(|| {
                        FactsError::unsupported(self.budget, "producer input syntax is unsupported")
                    })?;
                let view = self.original.facts.original_callable_schema(
                    tr::OriginalCallableSubject::Direct(direct.declaration),
                )?;
                let canonical = nominal_call_receipts::original_call_signature(
                    self.original,
                    position,
                    id,
                    target,
                    signature,
                    &self.original.facts,
                    self.budget,
                    &mut tr::TraitScratch::default(),
                )?;
                sr::work(self.budget, self.checked.call_evidence.len())?;
                let evidence = self
                    .checked
                    .call_evidence
                    .iter()
                    .find(|row| row.call == id)
                    .ok_or_else(|| {
                        FactsError::invalid(self.budget, "producer argument trace missing")
                    })?;
                if let Some(application) = &site.application {
                    self.explicit_producer(
                        application,
                        inputs,
                        node,
                        id,
                        args,
                        target,
                        signature,
                        canonical,
                        evidence,
                        depth + 1,
                    )?
                } else {
                    let trace = replay_shared_local_inputs(
                        id,
                        self.original,
                        self.inventory,
                        inputs,
                        site,
                        self.checked,
                        args,
                        direct,
                        signature,
                        canonical,
                        &view,
                        self.signatures,
                        evidence,
                        &self.original.facts,
                        self.budget,
                        depth + 1,
                    )?;
                    let result = trace.result_type(self.budget, &mut |decl, args| {
                        self.original
                            .plan
                            .intern_nominal(
                                decl,
                                args,
                                ir::FactsCtx::Validated(&self.original.facts),
                                self.budget,
                            )
                            .map_err(|e| source_error(self.budget, &e))
                    })?;
                    self.original
                        .facts
                        .normalize_substituted_projection_type(&result, self.budget)?
                }
            }
            (
                source @ Expr::AssociatedCall { .. },
                ir::ExprKind::Call {
                    target,
                    signature,
                    args,
                    ..
                },
            ) => {
                let explicit_method = if let ir::Callee::Direct(direct) = target {
                    let info = self
                        .original
                        .plan
                        .registry
                        .declaration(direct.declaration)
                        .map_err(|e| source_error(self.budget, &e))?;
                    info.path.associated.is_some()
                        && matches!(info.receiver, ReceiverKind::Borrowed { .. })
                } else {
                    false
                };
                if explicit_method {
                    let ir::Callee::Direct(direct) = target else {
                        unreachable!()
                    };
                    self.generic_method_result(
                        source,
                        node,
                        id,
                        direct,
                        signature,
                        args,
                        depth + 1,
                    )?
                } else {
                    self.closed_static_result(
                        source, node, id, target, signature, args, expected, depth,
                    )?
                }
            }
            (
                Expr::MethodCall {
                    receiver,
                    type_args,
                    args,
                    ..
                },
                ir::ExprKind::SliceLen { slice },
            ) if type_args.is_empty()
                && args.is_empty()
                && matches!(receiver.as_ref(), Expr::Var(path) if is_local_shaped_path(path)) =>
            {
                self.builtin_len_result(node, id, *slice, depth + 1)?
            }
            (
                source @ Expr::MethodCall { .. },
                ir::ExprKind::Call {
                    target,
                    signature,
                    args,
                    ..
                },
            ) => {
                self.concrete_method_result(source, node, id, target, signature, args, depth + 1)?
            }
            (Expr::Var(path), ir::ExprKind::ConstParameter(actual_operand)) => {
                let expected = super::const_parameters::parameter_path_value(
                    self.signatures,
                    path,
                    self.budget,
                )
                .map_err(|error| source_error(self.budget, &error))?
                .ok_or_else(|| FactsError::invalid(self.budget, "const value source changed"))?;
                if &expected != actual_operand {
                    return Err(FactsError::invalid(
                        self.budget,
                        "const value declaration changed",
                    ));
                }
                expected.value_type()
            }
            (Expr::Var(path), ir::ExprKind::Local(local)) if is_local_shaped_path(path) => {
                let (root, ty) = self.operand(node, depth)?;
                if root != *local {
                    return Err(FactsError::invalid(self.budget, "input alias root changed"));
                }
                ty
            }
            (
                Expr::Borrow { kind, place },
                ir::ExprKind::Borrow {
                    kind: actual_kind,
                    place: actual_place,
                    timing: ir::BorrowTiming::Immediate,
                },
            ) => {
                let (root, mut ty) = self.operand(node, depth)?;
                let checker = Checker::new(
                    self.signatures,
                    ir::Ty::Unit,
                    self.original
                        .plan
                        .callable_context(self.original.declaration)
                        .map_err(|e| source_error(self.budget, &e))?
                        .module,
                );
                let mut projections = Vec::new();
                for selector in &place.fields {
                    sr::work(self.budget, 1)?;
                    let (projection, next) = match selector {
                        FieldSelector::Deref => {
                            let ir::Ty::Ref { referent, .. } = &ty else {
                                return Err(FactsError::unsupported(
                                    self.budget,
                                    "input dereference requires a reference",
                                ));
                            };
                            (ir::Projection::Deref, referent.as_ref().clone())
                        }
                        _ => {
                            let (index, next) = checker
                                .field_type(&ty, selector)
                                .map_err(|e| source_error(self.budget, &e))?;
                            (ir::Projection::Field(index), next)
                        }
                    };
                    sr::push(&mut projections, projection, self.budget)?;
                    ty = next;
                }
                if root != actual_place.local
                    || projections != actual_place.projection
                    || kind != actual_kind
                {
                    return Err(FactsError::invalid(
                        self.budget,
                        "input borrow place or kind changed",
                    ));
                }
                self.reference(*kind, ty)?
            }
            (Expr::Integer(literal), ir::ExprKind::Int(value)) => {
                // An unsuffixed literal with no replay context (a generic
                // argument such as `vec![1]`'s seed element) took its final
                // type from later inference: replay against that type, which
                // re-validates the literal's range.
                let inferred = ir::Ty::Int(value.ty);
                let context = match (expected, literal.suffix) {
                    (None, None) => Some(&inferred),
                    _ => expected,
                };
                let resolved = resolve_integer(literal, false, context)
                    .map_err(|e| source_error(self.budget, &e))?;
                if resolved != *value {
                    return Err(FactsError::invalid(self.budget, "input integer changed"));
                }
                ir::Ty::Int(resolved.ty)
            }
            (Expr::Str(source), ir::ExprKind::Str(value)) => {
                // Static string syntax fixes both the descriptor type and its
                // bytes. In particular, a checked call result or inferred
                // nominal field cannot supply this recursive inference leaf.
                sr::work(self.budget, source.len().max(value.len()))?;
                if source != value {
                    return Err(FactsError::invalid(
                        self.budget,
                        "input string changed original source bytes",
                    ));
                }
                self.reference(ir::BorrowKind::Shared, ir::Ty::Str)?
            }
            (Expr::Bool(a), ir::ExprKind::Bool(b)) if a == b => ir::Ty::Bool,
            (Expr::Char(a), ir::ExprKind::Char(b)) if u32::from(*a) == b.value => ir::Ty::Char,
            (Expr::Tuple(fields), ir::ExprKind::Tuple(values))
            | (Expr::Array(fields), ir::ExprKind::Array(values))
                if fields.len() == values.len() =>
            {
                let array_element = if matches!(source, Expr::Array(_)) {
                    Some(self.array_element_type(fields, values, node, expected, depth + 1)?)
                } else {
                    None
                };
                let mut types = Vec::new();
                for (index, (field, value)) in fields.iter().zip(values).enumerate() {
                    let hint = match expected {
                        Some(ir::Ty::Tuple(types)) => types.get(index),
                        _ => array_element.as_ref(),
                    };
                    sr::push(
                        &mut types,
                        self.expression(
                            field,
                            &node.child(12, index, self.budget)?,
                            *value,
                            hint,
                            depth + 1,
                        )?,
                        self.budget,
                    )?;
                }
                if matches!(source, Expr::Tuple(_)) {
                    ir::Ty::Tuple(types.into())
                } else {
                    let element = types
                        .first()
                        .cloned()
                        .or_else(|| match expected {
                            Some(ir::Ty::Array { element, .. }) => Some(element.as_ref().clone()),
                            _ => None,
                        })
                        .ok_or_else(|| {
                            FactsError::unsupported(
                                self.budget,
                                "empty array requires source annotation",
                            )
                        })?;
                    for ty in &types {
                        if !self.equal(&element, ty)? {
                            return Err(FactsError::invalid(
                                self.budget,
                                "input array elements differ",
                            ));
                        }
                    }
                    sr::retain(self.budget, std::mem::size_of::<ir::Ty>() + 32)?;
                    ir::Ty::Array {
                        element: Arc::new(element),
                        len: fields.len(),
                    }
                }
            }
            (
                Expr::Repeat { value, len },
                ir::ExprKind::Repeat {
                    value: actual_value,
                    len: actual_len,
                },
            ) if len == actual_len => {
                let hint = match expected {
                    Some(ir::Ty::Array { element, .. }) => Some(element.as_ref()),
                    _ => None,
                };
                let element = self.expression(
                    value,
                    &node.child(11, 0, self.budget)?,
                    *actual_value,
                    hint,
                    depth + 1,
                )?;
                sr::retain(self.budget, std::mem::size_of::<ir::Ty>() + 32)?;
                ir::Ty::Array {
                    element: Arc::new(element),
                    len: *len,
                }
            }
            (Expr::Struct { name, fields }, ir::ExprKind::Struct { fields: values, .. }) => {
                source_clone::clone_vector(fields, self.budget)
                    .map_err(|e| source_error(self.budget, &e))?;
                self.nominal(
                    name,
                    None,
                    fields
                        .iter()
                        .map(|(name, value)| (FieldSelector::Named(*name), value))
                        .collect(),
                    values,
                    node,
                    expected,
                    depth,
                )?
            }
            (Expr::TupleStruct { name, fields }, ir::ExprKind::Struct { fields: values, .. }) => {
                source_clone::clone_vector(fields, self.budget)
                    .map_err(|e| source_error(self.budget, &e))?;
                self.nominal(
                    name,
                    None,
                    fields
                        .iter()
                        .enumerate()
                        .map(|(index, value)| (FieldSelector::Tuple(index), value))
                        .collect(),
                    values,
                    node,
                    expected,
                    depth,
                )?
            }
            (
                Expr::EnumVariant {
                    name,
                    shape,
                    fields,
                },
                ir::ExprKind::EnumVariant {
                    variant,
                    fields: values,
                    ..
                },
            ) => {
                let owner = name
                    .semantic_parent(&*self.original.plan.program.names)
                    .map_err(native_adapter::name_diagnostic)
                    .map_err(|e| source_error(self.budget, &e))?;
                let member = *name.segments().last().ok_or_else(|| {
                    FactsError::invalid(self.budget, "input enum variant name missing")
                })?;
                source_clone::clone_vector(fields, self.budget)
                    .map_err(|e| source_error(self.budget, &e))?;
                self.nominal(
                    &owner,
                    Some((member, *shape, *variant)),
                    fields
                        .iter()
                        .map(|(selector, value)| (selector.clone(), value))
                        .collect(),
                    values,
                    node,
                    expected,
                    depth,
                )?
            }
            _ => {
                if syntax(source, self.budget, depth)? {
                    return Err(FactsError::invalid(
                        self.budget,
                        "checked expression changed authenticated source shape",
                    ));
                }
                return Err(FactsError::unsupported(
                    self.budget,
                    "input initializer has no recursive source evidence",
                ));
            }
        };
        if let ir::ExprKind::Struct { ty: tag, .. } = &actual.kind
            && !self.equal(&ty, &ir::Ty::Struct(tag.clone()))?
        {
            return Err(FactsError::invalid(
                self.budget,
                "input nominal constructor tag changed",
            ));
        }
        if let ir::ExprKind::EnumVariant { ty: tag, .. } = &actual.kind
            && !self.equal(&ty, &ir::Ty::Enum(tag.clone()))?
        {
            return Err(FactsError::invalid(
                self.budget,
                "input enum constructor tag changed",
            ));
        }
        if actual.completion != ir::Completion::Returns || !self.equal(&ty, &actual.ty)? {
            return Err(FactsError::invalid(
                self.budget,
                "input source expression type or completion changed",
            ));
        }
        Ok(ty)
    }
    #[allow(clippy::too_many_arguments)]
    fn nominal(
        &self,
        name: &AstPath,
        variant: Option<(AstIdent, ir::VariantShape, usize)>,
        fields: Vec<(FieldSelector, &Expr)>,
        values: &[(usize, ir::ExprId)],
        node: &sr::SourceNode,
        expected: Option<&ir::Ty>,
        depth: usize,
    ) -> Result<ir::Ty, FactsError> {
        sr::retain(
            self.budget,
            fields
                .len()
                .saturating_mul(std::mem::size_of::<(FieldSelector, &Expr)>()),
        )?;
        if fields.len() != values.len() {
            return Err(FactsError::invalid(
                self.budget,
                "input nominal field count changed",
            ));
        }
        let checker = Checker::new(
            self.signatures,
            ir::Ty::Unit,
            self.original
                .plan
                .callable_context(self.original.declaration)
                .map_err(|e| source_error(self.budget, &e))?
                .module,
        );
        let key = checker
            .path_key(name, Namespace::Type)
            .map_err(|e| source_error(self.budget, &e))?;
        let decl = *self
            .original
            .plan
            .registry
            .nominals
            .get(&key)
            .ok_or_else(|| {
                FactsError::invalid(self.budget, "input nominal source declaration missing")
            })?;
        let (env, schema) = self
            .original
            .plan
            .nominal_schema(decl, self.budget)
            .map_err(|e| source_error(self.budget, &e))?;
        let selected_variant = match variant {
            Some((member, shape, actual_index)) => {
                let index = checker
                    .variant_index(decl, member)
                    .map_err(|e| source_error(self.budget, &e))?;
                let ir::Ty::Enum(enumeration) = &schema else {
                    return Err(FactsError::invalid(
                        self.budget,
                        "input variant owner is not enum",
                    ));
                };
                let definition = enumeration.def().variants.get(index).ok_or_else(|| {
                    FactsError::invalid(self.budget, "input variant definition missing")
                })?;
                if index != actual_index
                    || (shape != definition.shape
                        && !(definition.shape == ir::VariantShape::Unit
                            && shape == ir::VariantShape::Struct
                            && fields.is_empty()))
                {
                    return Err(FactsError::invalid(
                        self.budget,
                        "input enum variant or shape changed",
                    ));
                }
                if definition.fields.len() != fields.len() {
                    return Err(FactsError::invalid(
                        self.budget,
                        "input enum field arity changed",
                    ));
                }
                Some(index)
            }
            None => None,
        };
        let mut state = sr::CallInferenceState::new(Some(env), self.budget)?;
        if let Some(expected) = expected {
            state.constrain_input(&schema, expected)?;
        }
        let mut source_types = Vec::new();
        for (position, ((selector, source), (actual_index, value))) in
            fields.iter().zip(values).enumerate()
        {
            let (index, formal) =
                self.nominal_field(&checker, &schema, selected_variant, selector)?;
            if index != *actual_index {
                return Err(FactsError::invalid(
                    self.budget,
                    "input nominal field identity changed",
                ));
            }
            let hint = expected
                .map(|ty| {
                    self.nominal_field(&checker, ty, selected_variant, selector)
                        .map(|(_, ty)| ty)
                })
                .transpose()?;
            let input = self.expression(
                source,
                &node.child(14, position, self.budget)?,
                *value,
                // A concrete field type is source-authenticated context even
                // when the constructor itself has no written annotation.
                // Never use the checked constructor tag to recover this hint.
                hint.as_ref()
                    .or_else(|| formal.is_closed_shallow().then_some(&formal)),
                depth + 1,
            )?;
            state.constrain_input(&formal, &input)?;
            sr::push(&mut source_types, (selector.clone(), input), self.budget)?;
        }
        let mut materialize = |decl, args: &[ir::Ty]| {
            self.original
                .plan
                .intern_nominal(
                    decl,
                    args,
                    ir::FactsCtx::Validated(&self.original.facts),
                    self.budget,
                )
                .map_err(|e| source_error(self.budget, &e))
        };
        let args = state.finish(false, &mut materialize)?.ok_or_else(|| {
            FactsError::unsupported(self.budget, "input nominal actuals remain unresolved")
        })?;
        let ty = materialize(decl, &args)?;
        for (selector, input) in source_types {
            let (_, field) = self.nominal_field(&checker, &ty, selected_variant, &selector)?;
            if !self.equal(&field, &input)? {
                return Err(FactsError::invalid(
                    self.budget,
                    "input nominal field type changed",
                ));
            }
        }
        Ok(ty)
    }
    fn nominal_field(
        &self,
        checker: &Checker<'_>,
        ty: &ir::Ty,
        variant: Option<usize>,
        selector: &FieldSelector,
    ) -> Result<(usize, ir::Ty), FactsError> {
        let Some(variant) = variant else {
            return checker
                .field_type(ty, selector)
                .map_err(|e| source_error(self.budget, &e));
        };
        let ir::Ty::Enum(enumeration) = ty else {
            return Err(FactsError::invalid(
                self.budget,
                "input enum field owner changed",
            ));
        };
        let definition =
            enumeration.def().variants.get(variant).ok_or_else(|| {
                FactsError::invalid(self.budget, "input enum field variant missing")
            })?;
        let index = match selector {
            FieldSelector::Tuple(index) if definition.shape == ir::VariantShape::Tuple => *index,
            FieldSelector::Named(name) if definition.shape == ir::VariantShape::Struct => checker
                .member_index(enumeration.def().source_decl, Some(variant), *name)
                .map_err(|e| source_error(self.budget, &e))?,
            _ => {
                return Err(FactsError::invalid(
                    self.budget,
                    "input enum field selector changed",
                ));
            }
        };
        let fields = definition
            .fields
            .read(self.budget)
            .map_err(|e| source_error(self.budget, &e))?;
        let field = fields
            .get(index)
            .ok_or_else(|| FactsError::invalid(self.budget, "input enum field missing"))?;
        Ok((index, field.ty.clone()))
    }
}

#[cfg(test)]
mod literal_context_tests {
    use super::*;

    #[test]
    fn lexical_integer_context_comes_from_authored_result_binding() {
        for body in [
            "let bag:Bag<u8>={let mut inside=seed(7);inside.set(2);inside};consume(&bag);",
            "let bag:Bag<u8>={let inside=seed(7);{let inside=inside;inside}};consume(&bag);",
        ] {
            let source = format!(
                "struct Bag<T>{{value:T}}impl<T> Bag<T>{{fn set(&mut self,value:T){{self.value=value;}}}}fn seed<T>(value:T)->Bag<T>{{Bag{{value}}}}fn consume<T>(value:&T){{}}fn main(){{{body}}}"
            );
            let program = parse(&source).unwrap();
            let budget = ir::PlanBudget::default();
            let plan = PlanCtx::from_program(&program, &budget).unwrap();
            let declaration = plan.function_decl(program.functions.len() - 1).unwrap();
            let PublishedCallableBody::Source(publication) =
                plan.publish_callable_body(declaration, &budget).unwrap()
            else {
                panic!("expected lexical integer source");
            };
            let original = publication.original().source_for_test();
            let checked = publication.source();
            assert!(
                !replay_source_body(&original, checked, publication.facts(), &budget)
                    .unwrap()
                    .is_empty(),
                "numeric authority fixture must select borrowed-input replay"
            );
            let function = plan.callable_body(&plan.program, declaration).unwrap();
            let Stmt::Let {
                value: Expr::Block(source_block),
                ..
            } = &function.body.statements[0]
            else {
                panic!("expected authored result block")
            };
            let Stmt::Let {
                value: Expr::Call {
                    args: source_args, ..
                },
                ..
            } = &source_block.statements[0]
            else {
                panic!("expected authored seed initializer")
            };
            assert!(
                matches!(&source_args[0], Expr::Integer(literal) if literal.magnitude == 7 && literal.suffix.is_none())
            );
            let ir::TypedStmt::Let { value: block, .. } = checked.body.statements[0] else {
                panic!("expected checked result block")
            };
            let ir::ExprKind::Block(block) = &checked.exprs[block.0].kind else {
                panic!("expected checked lexical block")
            };
            let ir::TypedStmt::Let { value: seed, .. } = block.statements[0] else {
                panic!("expected checked seed initializer")
            };
            let ir::ExprKind::Call { args, .. } = &checked.exprs[seed.0].kind else {
                panic!("expected checked seed call")
            };
            let integer = args[0].0;
            assert!(
                matches!(&checked.exprs[integer].kind, ir::ExprKind::Int(value) if value.ty == ir::IntTy::U8 && value.bits == 7)
            );
            for mutation in 0..2 {
                let mut forged = checked.as_ref().clone();
                let ir::ExprKind::Int(value) = &mut forged.exprs[integer].kind else {
                    unreachable!()
                };
                if mutation == 0 {
                    value.ty = ir::IntTy::I32;
                    forged.exprs[integer].ty = ir::Ty::Int(ir::IntTy::I32);
                } else {
                    *value = resolve_integer(
                        &IntegerLiteral {
                            magnitude: 9,
                            suffix: Some(ir::IntTy::U8),
                        },
                        false,
                        None,
                    )
                    .unwrap();
                }
                assert!(
                    replay_source_body(&original, &Arc::new(forged), publication.facts(), &budget)
                        .is_err(),
                    "forged lexical integer mutation {mutation} passed"
                );
            }
        }
    }

    #[test]
    fn annotation_admission_rejects_changed_type_and_source() {
        let program = parse("trait Source:Sized{fn value(self)->i32{7}}struct Maker;impl Source for Maker{}fn consume<T>(value:&T){}fn main(){let value:i32=Maker.value();consume(&value);}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let declaration = plan.function_decl(program.functions.len() - 1).unwrap();
        let PublishedCallableBody::Source(publication) =
            plan.publish_callable_body(declaration, &budget).unwrap()
        else {
            panic!("expected original source");
        };
        let original = publication.original().source_for_test();
        let checked = publication.source();
        assert!(
            !replay_source_body(&original, checked, publication.facts(), &budget)
                .unwrap()
                .is_empty()
        );
        let ir::TypedStmt::Let { local, value } = checked.body.statements[0] else {
            panic!("expected source annotation");
        };
        let mut changed_type = checked.as_ref().clone();
        changed_type.locals[local.0].ty = ir::Ty::Bool;
        changed_type.exprs[value.0].ty = ir::Ty::Bool;
        assert!(
            replay_source_body(
                &original,
                &Arc::new(changed_type),
                publication.facts(),
                &budget
            )
            .is_err()
        );
        let mut changed_source = original.clone();
        let binding = changed_source
            .bindings
            .iter_mut()
            .find(|binding| binding.annotation.is_some())
            .unwrap();
        binding.annotation = Some(Type::Bool);
        assert!(
            replay_source_body(&changed_source, checked, publication.facts(), &budget).is_err()
        );
    }

    #[test]
    fn lexical_block_producers_reject_changed_effects_and_scope() {
        let source = "struct Bag<T>{value:T}impl<T> Bag<T>{fn set(&mut self,value:T){self.value=value;}}fn seed<T>(value:T)->Bag<T>{Bag{value}}fn consume<T>(value:T){}fn main(){let sibling=\"sibling\";let mut bag={let mut inside=seed(\"first\");inside.set(\"second\");inside};consume(&mut bag);}";
        let program = parse(source).unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let declaration = plan.function_decl(program.functions.len() - 1).unwrap();
        let PublishedCallableBody::Source(publication) =
            plan.publish_callable_body(declaration, &budget).unwrap()
        else {
            panic!("expected lexical source block");
        };
        let original = publication.original().source_for_test();
        let checked = publication.source();
        assert!(
            !replay_source_body(&original, checked, publication.facts(), &budget)
                .unwrap()
                .is_empty()
        );
        let block_id = checked.exprs.iter().position(|expression| matches!(&expression.kind, ir::ExprKind::Block(block) if block.statements.len() == 2)).unwrap();
        for mutation in 0..4 {
            let mut forged = checked.as_ref().clone();
            let ir::ExprKind::Block(block) = &mut forged.exprs[block_id].kind else {
                unreachable!()
            };
            match mutation {
                0 => {
                    block.statements.pop();
                }
                1 => block.statements.swap(0, 1),
                2 => block.tail = None,
                3 => {
                    let ir::TypedStmt::Let { local, .. } = &mut block.statements[0] else {
                        unreachable!()
                    };
                    *local = ir::LocalId(usize::MAX);
                }
                _ => unreachable!(),
            }
            assert!(
                replay_source_body(&original, &Arc::new(forged), publication.facts(), &budget)
                    .is_err(),
                "forged lexical block mutation {mutation} passed"
            );
        }
    }

    #[test]
    fn generic_method_chains_require_unique_authored_role_not_deref_placeholders() {
        for source in [
            "struct A{n:i32}impl A{fn copy(&self)->A{A{n:self.n}}fn get(self)->i32{self.n}}fn main(){let a=A{n:7};let n=a.copy().get();}",
            "struct Slot<T>{v:T}impl<T> Slot<T>{fn take(self)->T{self.v}}fn make()->Slot<i32>{Slot{v:7}}fn main(){let n=make().take();}",
            "struct Slot{v:i32}impl Slot{fn new()->Self{Slot{v:7}}fn take(self)->i32{self.v}}fn main(){let n=Slot::new().take();}",
        ] {
            let program = parse(source).unwrap();
            let budget = ir::PlanBudget::default();
            let identity = sr::OriginalBodyId::new(&budget).unwrap();
            let build = || {
                SourceInventory::build(
                    &program,
                    program.functions.last().unwrap(),
                    identity.clone(),
                    &budget,
                )
                .unwrap()
            };
            let inventory = build();
            let index = inventory
                .calls
                .iter()
                .position(|site| {
                    !site.operator_candidate
                        && inventory
                            .calls
                            .iter()
                            .any(|other| other.node == site.node && other.operator_candidate)
                })
                .unwrap();
            let node = inventory.calls[index].node.clone();
            assert!(
                inventory
                    .calls
                    .iter()
                    .position(|site| site.node == node)
                    .unwrap()
                    < index
            );
            assert_eq!(
                authored_call_index(&inventory, &node, &budget).unwrap(),
                index
            );
            for mode in 0..3 {
                let mut forged = build();
                match mode {
                    0 => forged.calls[index].operator_candidate = true,
                    1 => {
                        forged.calls.remove(index);
                    }
                    _ => forged.calls.push(forged.calls[index].clone()),
                }
                assert_eq!(
                    authored_call_index(&forged, &node, &budget)
                        .unwrap_err()
                        .kind(),
                    tr::FactsErrorKind::InvalidMetadata,
                    "mode {mode}"
                );
            }
        }
    }

    #[test]
    fn projected_borrow_value_keeps_builtin_stored_field_route() {
        let source = "struct Owner{values:[i32;2]}fn take<T>(value:&T){}fn main(){let owner=Owner{values:[3i32,7i32]};take(&owner.values[1]);}";
        let prepared = crate::compiler::source::prepare_text_with_prelude(
            "stored-index-borrow-source.rs",
            source,
            &crate::compiler::source::SourceOptions::default(),
        )
        .unwrap();
        let program = resolve(&parse_prepared(&prepared).unwrap()).unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let declaration = plan.function_decl(1).unwrap();
        let PublishedCallableBody::Source(publication) =
            plan.publish_callable_body(declaration, &budget).unwrap()
        else {
            panic!("stored indexed borrow source body");
        };
        let original = publication.original().source_for_test();
        assert!(
            replay_source_body(
                &original,
                publication.source(),
                publication.facts(),
                &budget
            )
            .unwrap()
            .is_empty()
        );
    }

    #[test]
    fn projected_borrow_replays_original_fields_and_rejects_forgeries() {
        for (formal, borrow, kind) in [
            ("&[T;2]", "&owner.values.0", ir::BorrowKind::Shared),
            ("&mut[T;2]", "&mut owner.values.0", ir::BorrowKind::Mut),
            ("&T", "&owner.values.0[1]", ir::BorrowKind::Shared),
            ("&mut T", "&mut owner.values.0[1]", ir::BorrowKind::Mut),
        ] {
            let source = format!(
                "use std::ops::{{Deref,DerefMut}};struct Target<T>{{values:([T;2],[T;2]),other:([T;2],[T;2])}}struct Owner<T>{{inner:Target<T>}}impl<T> Deref for Owner<T>{{type Target=Target<T>;fn deref(&self)->&Target<T>{{&self.inner}}}}impl<T> DerefMut for Owner<T>{{fn deref_mut(&mut self)->&mut Target<T>{{&mut self.inner}}}}fn take<T>(v:{formal}){{}}fn main(){{let mut owner:Owner<i32>=Owner{{inner:Target{{values:([3i32,7i32],[5i32,9i32]),other:([11i32,13i32],[17i32,19i32])}}}};take({borrow});}}"
            );
            let prepared = crate::compiler::source::prepare_text_with_prelude(
                "projected-borrow-source.rs",
                &source,
                &crate::compiler::source::SourceOptions::default(),
            )
            .unwrap();
            let program = resolve(&parse_prepared(&prepared).unwrap()).unwrap();
            let budget = ir::PlanBudget::default();
            let plan = PlanCtx::from_program(&program, &budget).unwrap();
            let declaration = plan.function_decl(1).unwrap();
            let PublishedCallableBody::Source(publication) =
                plan.publish_callable_body(declaration, &budget).unwrap()
            else {
                panic!("original projected borrow body");
            };
            let original = publication.original().source_for_test();
            let checked = publication.source();
            assert!(
                !replay_source_body(&original, checked, publication.facts(), &budget)
                    .unwrap()
                    .is_empty()
            );
            let (index, operator, _) = original
                .call_mapping
                .iter()
                .find(|(index, _, _)| original.calls[*index].operator_candidate)
                .unwrap();
            let operator = *operator;
            let operand = match &checked.exprs[operator.0].kind {
                ir::ExprKind::Call { args, .. } => args[0],
                _ => unreachable!(),
            };
            let owner_local = match &checked.exprs[operand.0].kind {
                ir::ExprKind::Borrow { place, .. } => place.local,
                _ => unreachable!(),
            };
            let (wrapper, carrier, tail) = checked
                .exprs
                .iter()
                .enumerate()
                .find_map(|(index, expr)| {
                    let ir::ExprKind::Block(block) = &expr.kind else {
                        return None;
                    };
                    let ([ir::TypedStmt::Assign { local, value }], Some(tail)) =
                        (block.statements.as_slice(), block.tail)
                    else {
                        return None;
                    };
                    (*value == operator).then_some((ir::ExprId(index), *local, tail))
                })
                .unwrap();
            for mutation in 0..24 {
                let mut forged = checked.as_ref().clone();
                match mutation {
                    0..=3 => {
                        let ir::ExprKind::Block(block) = &mut forged.exprs[wrapper.0].kind else {
                            unreachable!()
                        };
                        match mutation {
                            0 => block.statements.push(ir::TypedStmt::Assign {
                                local: carrier,
                                value: operator,
                            }),
                            1 => block.tail = None,
                            2 => {
                                block.statements = vec![ir::TypedStmt::Assign {
                                    local: carrier,
                                    value: operand,
                                }]
                            }
                            _ => {
                                block.statements = vec![ir::TypedStmt::Assign {
                                    local: owner_local,
                                    value: operator,
                                }]
                            }
                        }
                    }
                    4 => forged.locals[carrier.0].ty = ir::Ty::Bool,
                    5 => forged.locals[carrier.0].mutable = true,
                    6..=9 => {
                        let ir::ExprKind::Borrow {
                            kind: actual_kind,
                            place,
                            timing,
                        } = &mut forged.exprs[tail.0].kind
                        else {
                            unreachable!()
                        };
                        match mutation {
                            6 => *place.projection.last_mut().unwrap() = ir::Projection::Field(1),
                            7 => place.local = owner_local,
                            8 => {
                                *actual_kind = if kind == ir::BorrowKind::Mut {
                                    ir::BorrowKind::Shared
                                } else {
                                    ir::BorrowKind::Mut
                                }
                            }
                            _ => {
                                *timing = ir::BorrowTiming::TwoPhase {
                                    activation: operator,
                                    argument: 0,
                                    reason: ir::ImplicitBorrowKind::MutableReceiver,
                                }
                            }
                        }
                    }
                    10 => forged.exprs[tail.0].ty = ir::Ty::Bool,
                    11..=14 => {
                        let ir::ExprKind::Call {
                            target: ir::Callee::Direct(target),
                            signature,
                            ..
                        } = &mut forged.exprs[operator.0].kind
                        else {
                            unreachable!()
                        };
                        match mutation {
                            11 => target.declaration = declaration,
                            12 => target.type_args = vec![ir::Ty::Bool].into(),
                            13 => signature.params.clear(),
                            _ => signature.ref_contract = Arc::new(ir::RefContract::default()),
                        }
                    }
                    15 | 16 => {
                        let mut evidence = forged.call_evidence.to_vec();
                        let row = evidence
                            .iter_mut()
                            .find(|row| row.call == operator)
                            .unwrap();
                        if mutation == 15 {
                            row.arguments = Arc::from([]);
                        } else {
                            row.type_actual_regions = vec![ir::TypeActualRegion {
                                argument: 0,
                                location: ir::CarrierLocation {
                                    path: ir::CarrierPath(Arc::from([])),
                                    channel: ir::CarrierChannel::Fixed(ir::ReferencePosition {
                                        depth: 0,
                                    }),
                                },
                                source: ir::ActualRegion::Elided,
                            }]
                            .into();
                        }
                        forged.call_evidence = evidence.into();
                    }
                    17..=19 => {
                        let ir::ExprKind::Borrow {
                            kind: actual_kind,
                            place,
                            ..
                        } = &mut forged.exprs[operand.0].kind
                        else {
                            unreachable!()
                        };
                        match mutation {
                            17 => place.projection.push(ir::Projection::Field(0)),
                            18 => place.local = carrier,
                            _ => {
                                *actual_kind = if kind == ir::BorrowKind::Mut {
                                    ir::BorrowKind::Shared
                                } else {
                                    ir::BorrowKind::Mut
                                }
                            }
                        }
                    }
                    20 => forged.exprs[operator.0].completion = ir::Completion::Never,
                    21 => forged.exprs[tail.0].completion = ir::Completion::Never,
                    22 => forged.exprs[wrapper.0].ty = ir::Ty::Bool,
                    _ => forged.exprs[wrapper.0] = forged.exprs[operator.0].clone(),
                }
                assert_eq!(
                    replay_source_body(&original, &Arc::new(forged), publication.facts(), &budget)
                        .expect_err(&format!("projected borrow checked mutation {mutation}"))
                        .kind(),
                    tr::FactsErrorKind::InvalidMetadata
                );
            }
            let function = plan.callable_body(&plan.program, declaration).unwrap();
            let inventory =
                SourceInventory::build(&plan.program, function, original.identity.clone(), &budget)
                    .unwrap();
            let (pointer, root, fields, indexed_value) = function
                .body
                .statements
                .iter()
                .find_map(|statement| {
                    let Stmt::Expr(Expr::Call { args, .. }) = statement else {
                        return None;
                    };
                    let (_, place) =
                        explicit_field_borrow::authored_static_borrow(&args[0], &budget)
                            .unwrap()?;
                    let LocalName::Named(root) = place.local else {
                        return None;
                    };
                    Some((
                        &args[0] as *const Expr as usize,
                        root,
                        place.fields.clone(),
                        matches!(&args[0], Expr::BorrowValue { .. }),
                    ))
                })
                .unwrap();
            let second = inventory.call_pointers[&deref_coercion::site(pointer, 2)];
            for mutation in 0..4 {
                let mut forged = original.clone();
                match mutation {
                    0 => forged
                        .call_mapping
                        .retain(|(_, value, _)| *value != operator),
                    1 => {
                        forged
                            .call_mapping
                            .iter_mut()
                            .find(|(_, value, _)| *value == operator)
                            .unwrap()
                            .0 = second
                    }
                    2 => forged.call_mapping.push(
                        original
                            .call_mapping
                            .iter()
                            .find(|(site, _, _)| site == index)
                            .unwrap()
                            .clone(),
                    ),
                    _ => {
                        forged
                            .binding_mapping
                            .iter_mut()
                            .find(|(_, local)| *local == owner_local)
                            .unwrap()
                            .1 = carrier
                    }
                }
                assert_eq!(
                    replay_source_body(&forged, checked, publication.facts(), &budget)
                        .unwrap_err()
                        .kind(),
                    tr::FactsErrorKind::InvalidMetadata,
                    "projected original mutation {mutation}"
                );
            }
            for mutation in 0..10 {
                let mut forged = SourceInventory::build(
                    &plan.program,
                    function,
                    original.identity.clone(),
                    &budget,
                )
                .unwrap();
                let key = deref_coercion::site(pointer, 1);
                match mutation {
                    0 => {
                        forged.call_pointers.remove(&key);
                    }
                    1 => {
                        forged.call_pointers.insert(key, second);
                    }
                    2 => forged.calls[*index].operator_candidate = false,
                    3 => {
                        forged.calls[*index].node.owner = sr::OriginalBodyId::new(&budget).unwrap()
                    }
                    4 => {
                        forged
                            .call_pointers
                            .insert(deref_coercion::site(pointer, 3), *index);
                    }
                    _ => {}
                }
                let wrong = [FieldSelector::Tuple(0)];
                let wrong_tail = [fields[0].clone(), FieldSelector::Tuple(1)];
                let FieldName::Named(other) = program.structs[0].fields[1].name else {
                    unreachable!()
                };
                let wrong_member = [FieldSelector::Named(other), fields[1].clone()];
                let mut wrong_index = fields.clone();
                *wrong_index.last_mut().unwrap() = FieldSelector::ConstantIndex(0);
                let expected_fields: &[FieldSelector] = match mutation {
                    5 => &wrong,
                    6 => &wrong_tail,
                    7 => &wrong_member,
                    8 => &wrong_index,
                    _ => &fields,
                };
                assert_eq!(
                    Replay::borrow_coercion_field_site(
                        &original,
                        &forged,
                        &original.calls[*index].node,
                        kind,
                        root,
                        (
                            expected_fields,
                            if mutation == 9 {
                                !indexed_value
                            } else {
                                indexed_value
                            },
                        ),
                        &budget
                    )
                    .unwrap_err()
                    .kind(),
                    tr::FactsErrorKind::InvalidMetadata,
                    "projected inventory mutation {mutation}"
                );
            }
            let foreign_program = parse("fn main(){}").unwrap();
            let foreign_budget = ir::PlanBudget::default();
            let foreign = PlanCtx::from_program(&foreign_program, &foreign_budget).unwrap();
            let facts = foreign
                .callable_facts(foreign.function_decl(0).unwrap())
                .unwrap();
            assert_eq!(
                replay_source_body(&original, checked, &facts, &budget)
                    .unwrap_err()
                    .kind(),
                tr::FactsErrorKind::InvalidMetadata
            );
        }
    }

    #[test]
    fn explicit_borrow_coercion_replays_original_owner_and_rejects_forgeries() {
        for (formal, borrow, kind) in [
            ("&[T]", "&owner", ir::BorrowKind::Shared),
            ("&mut[T]", "&mut owner", ir::BorrowKind::Mut),
        ] {
            let source = format!(
                "use std::ops::{{Deref,DerefMut}};struct Wrapper<T>{{values:[T;2]}}impl<T> Deref for Wrapper<T>{{type Target=[T];fn deref(&self)->&[T]{{&self.values}}}}impl<T> DerefMut for Wrapper<T>{{fn deref_mut(&mut self)->&mut[T]{{&mut self.values}}}}fn take<T>(v:{formal}){{}}fn main(){{let mut owner:Wrapper<i32>=Wrapper{{values:[3i32,7i32]}};take({borrow});}}"
            );
            let prepared = crate::compiler::source::prepare_text_with_prelude(
                "borrow-coercion-source.rs",
                &source,
                &crate::compiler::source::SourceOptions::default(),
            )
            .unwrap();
            let program = resolve(&parse_prepared(&prepared).unwrap()).unwrap();
            let budget = ir::PlanBudget::default();
            let plan = PlanCtx::from_program(&program, &budget).unwrap();
            let declaration = plan.function_decl(1).unwrap();
            let PublishedCallableBody::Source(publication) =
                plan.publish_callable_body(declaration, &budget).unwrap()
            else {
                panic!("original explicit-borrow body");
            };
            let original = publication.original().source_for_test();
            let checked = publication.source();
            assert!(
                !replay_source_body(&original, checked, publication.facts(), &budget)
                    .unwrap()
                    .is_empty()
            );
            let (index, operator, _) = original
                .call_mapping
                .iter()
                .find(|(index, _, _)| original.calls[*index].operator_candidate)
                .unwrap();
            let operator = *operator;
            let operand = match &checked.exprs[operator.0].kind {
                ir::ExprKind::Call { args, .. } => args[0],
                _ => unreachable!(),
            };
            for mutation in 0..7 {
                let mut forged = checked.as_ref().clone();
                match mutation {
                    0 => {
                        let ir::ExprKind::Borrow { kind: actual, .. } =
                            &mut forged.exprs[operand.0].kind
                        else {
                            unreachable!()
                        };
                        *actual = if kind == ir::BorrowKind::Mut {
                            ir::BorrowKind::Shared
                        } else {
                            ir::BorrowKind::Mut
                        };
                    }
                    1 => {
                        let ir::ExprKind::Borrow { place, .. } = &mut forged.exprs[operand.0].kind
                        else {
                            unreachable!()
                        };
                        place.local = ir::LocalId(usize::MAX);
                    }
                    2 | 3 => {
                        let ir::ExprKind::Call {
                            target: ir::Callee::Direct(target),
                            ..
                        } = &mut forged.exprs[operator.0].kind
                        else {
                            unreachable!()
                        };
                        if mutation == 2 {
                            target.declaration = declaration;
                        } else {
                            target.type_args = vec![ir::Ty::Bool].into();
                        }
                    }
                    4 => forged.exprs[operator.0].ty = ir::Ty::Bool,
                    _ => {
                        let mut rows = forged.call_evidence.to_vec();
                        let row = rows.iter_mut().find(|row| row.call == operator).unwrap();
                        if mutation == 5 {
                            row.arguments = Arc::from([]);
                        } else {
                            row.type_actual_regions = vec![ir::TypeActualRegion {
                                argument: 0,
                                location: ir::CarrierLocation {
                                    path: ir::CarrierPath(Arc::from([])),
                                    channel: ir::CarrierChannel::Fixed(ir::ReferencePosition {
                                        depth: 0,
                                    }),
                                },
                                source: ir::ActualRegion::Elided,
                            }]
                            .into();
                        }
                        forged.call_evidence = rows.into();
                    }
                }
                assert_eq!(
                    replay_source_body(&original, &Arc::new(forged), publication.facts(), &budget)
                        .expect_err(&format!("explicit borrow mutation {mutation}"))
                        .kind(),
                    tr::FactsErrorKind::InvalidMetadata
                );
            }
            let function = plan.callable_body(&plan.program, declaration).unwrap();
            let inventory =
                SourceInventory::build(&plan.program, function, original.identity.clone(), &budget)
                    .unwrap();
            let (pointer, root) = function
                .body
                .statements
                .iter()
                .find_map(|statement| {
                    let Stmt::Expr(Expr::Call { args, .. }) = statement else {
                        return None;
                    };
                    let Expr::Borrow { place, .. } = &args[0] else {
                        return None;
                    };
                    let LocalName::Named(root) = place.local else {
                        return None;
                    };
                    Some((&args[0] as *const Expr as usize, root))
                })
                .unwrap();
            let second = inventory.call_pointers[&deref_coercion::site(pointer, 2)];
            for mutation in 0..3 {
                let mut forged = original.clone();
                match mutation {
                    0 => forged
                        .call_mapping
                        .retain(|(_, expression, _)| *expression != operator),
                    1 => {
                        forged
                            .call_mapping
                            .iter_mut()
                            .find(|(_, expression, _)| *expression == operator)
                            .unwrap()
                            .0 = second;
                    }
                    _ => forged.call_mapping.push(
                        original
                            .call_mapping
                            .iter()
                            .find(|(source, _, _)| source == index)
                            .unwrap()
                            .clone(),
                    ),
                }
                assert_eq!(
                    replay_source_body(&forged, checked, publication.facts(), &budget)
                        .unwrap_err()
                        .kind(),
                    tr::FactsErrorKind::InvalidMetadata
                );
            }
            for mutation in 0..4 {
                let mut inventory = SourceInventory::build(
                    &plan.program,
                    function,
                    original.identity.clone(),
                    &budget,
                )
                .unwrap();
                let key = deref_coercion::site(pointer, 1);
                match mutation {
                    0 => {
                        inventory.call_pointers.remove(&key);
                    }
                    1 => {
                        inventory.call_pointers.insert(key, second);
                    }
                    2 => inventory.calls[*index].operator_candidate = false,
                    _ => {
                        inventory.calls[*index].node.owner =
                            sr::OriginalBodyId::new(&budget).unwrap()
                    }
                }
                assert_eq!(
                    Replay::borrow_coercion_site(
                        &original,
                        &inventory,
                        &original.calls[*index].node,
                        kind,
                        root,
                        &budget
                    )
                    .unwrap_err()
                    .kind(),
                    tr::FactsErrorKind::InvalidMetadata
                );
            }
            let foreign_program = parse("fn main(){}").unwrap();
            let foreign_budget = ir::PlanBudget::default();
            let foreign = PlanCtx::from_program(&foreign_program, &foreign_budget).unwrap();
            let facts = foreign
                .callable_facts(foreign.function_decl(0).unwrap())
                .unwrap();
            assert_eq!(
                replay_source_body(&original, checked, &facts, &budget)
                    .unwrap_err()
                    .kind(),
                tr::FactsErrorKind::InvalidMetadata
            );
            assert!(budget.charge_work(usize::MAX).is_err());
            assert_eq!(
                replay_source_body(&original, checked, publication.facts(), &budget)
                    .unwrap_err()
                    .kind(),
                tr::FactsErrorKind::Resource
            );
        }
    }

    #[test]
    fn projected_borrow_does_not_guess_named_actual_regions() {
        let source = "use std::ops::Deref;struct Target<T>{value:T}struct Owner<T>{target:Target<T>}impl<T> Deref for Owner<T>{type Target=Target<T>;fn deref(&self)->&Target<T>{&self.target}}fn take<T>(v:&T){}fn use_it<'a>(value:&'a i32){let owner:Owner<&'a i32>=Owner{target:Target{value}};take(&owner.value);}fn main(){let value=7i32;use_it(&value);}";
        let prepared = crate::compiler::source::prepare_text_with_prelude(
            "projected-borrow-regions.rs",
            source,
            &crate::compiler::source::SourceOptions::default(),
        )
        .unwrap();
        let program = resolve(&parse_prepared(&prepared).unwrap()).unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let error = match plan.publish_callable_body(plan.function_decl(1).unwrap(), &budget) {
            Err(error) => error,
            Ok(_) => panic!("projected named actuals must not acquire guessed regions"),
        };
        assert_eq!(error.kind(), tr::FactsErrorKind::Unsupported);
        assert!(
            error
                .to_string()
                .contains("independently reference-free generic actuals")
        );
        assert!(!budget.has_resource_failure());
    }

    #[test]
    fn explicit_borrow_coercion_does_not_guess_named_actual_regions() {
        let source = "use std::ops::Deref;struct Wrapper<T>{values:[T;1]}impl<T> Deref for Wrapper<T>{type Target=[T];fn deref(&self)->&[T]{&self.values}}fn take<T>(v:&[T]){}fn use_it<'a>(value:&'a i32){let owner:Wrapper<&'a i32>=Wrapper{values:[value]};take(&owner);}fn main(){let value=7i32;use_it(&value);}";
        let prepared = crate::compiler::source::prepare_text_with_prelude(
            "borrow-coercion-regions.rs",
            source,
            &crate::compiler::source::SourceOptions::default(),
        )
        .unwrap();
        let program = resolve(&parse_prepared(&prepared).unwrap()).unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let error = match plan.publish_callable_body(plan.function_decl(1).unwrap(), &budget) {
            Err(error) => error,
            Ok(_) => panic!("named generic regions must not acquire guessed coercion authority"),
        };
        assert_eq!(error.kind(), tr::FactsErrorKind::Unsupported);
        assert!(
            error
                .to_string()
                .contains("independently reference-free generic actuals")
        );
        assert!(!budget.has_resource_failure());
    }

    #[test]
    fn shared_local_deref_reconstructs_owner_and_rejects_forged_evidence() {
        for receiver in ["wrapper", "borrowed"] {
            let source = format!(
                "use std::ops::Deref;struct Owner<T>{{n:T}}struct Wrapper<T>{{v:Owner<T>}}struct View<'a,T>{{r:&'a T}}impl<T> Deref for Wrapper<T>{{type Target=Owner<T>;fn deref(&self)->&Owner<T>{{&self.v}}}}impl<T> Owner<T>{{fn view<'a>(&'a self)->View<'a,T>{{View{{r:&self.n}}}}}}fn consume<T>(x:T){{}}fn main(){{let wrapper=Wrapper{{v:Owner{{n:7i32}}}};let borrowed=&wrapper;let row={receiver}.view();consume(row);}}"
            );
            let prepared = crate::compiler::source::prepare_text_with_prelude(
                "shared-receiver.rs",
                &source,
                &crate::compiler::source::SourceOptions::default(),
            )
            .unwrap();
            let program = resolve(&parse_prepared(&prepared).unwrap()).unwrap();
            let budget = ir::PlanBudget::default();
            let plan = PlanCtx::from_program(&program, &budget).unwrap();
            let declaration = plan.function_decl(1).unwrap();
            let PublishedCallableBody::Source(publication) =
                plan.publish_callable_body(declaration, &budget).unwrap()
            else {
                panic!("expected original shared receiver source");
            };
            let original = publication.original().source_for_test();
            let checked = publication.source();
            assert!(
                !replay_source_body(&original, checked, publication.facts(), &budget)
                    .unwrap()
                    .is_empty()
            );
            let (source_index, operator, _) = original
                .call_mapping
                .iter()
                .find(|(index, _, _)| original.calls[*index].operator_candidate)
                .unwrap();
            let operator = *operator;
            let operand = match &checked.exprs[operator.0].kind {
                ir::ExprKind::Call { args, .. } => args[0],
                _ => unreachable!(),
            };
            for mutation in 0..7 {
                let mut forged = checked.as_ref().clone();
                match mutation {
                    0 => {
                        let ir::ExprKind::Call {
                            target: ir::Callee::Direct(target),
                            ..
                        } = &mut forged.exprs[operator.0].kind
                        else {
                            unreachable!()
                        };
                        target.declaration = declaration;
                    }
                    1 => match &mut forged.exprs[operand.0].kind {
                        ir::ExprKind::Borrow { place, .. } => place.local = ir::LocalId(usize::MAX),
                        ir::ExprKind::Local(local) => *local = ir::LocalId(usize::MAX),
                        _ => unreachable!(),
                    },
                    2 => forged.exprs[operator.0].ty = ir::Ty::Bool,
                    3 => {
                        let ir::ExprKind::Call {
                            target: ir::Callee::Direct(target),
                            ..
                        } = &mut forged.exprs[operator.0].kind
                        else {
                            unreachable!()
                        };
                        target.type_args = vec![ir::Ty::Bool].into();
                    }
                    _ => {
                        let mut rows = forged.call_evidence.to_vec();
                        let row = rows.iter_mut().find(|row| row.call == operator).unwrap();
                        if mutation == 4 {
                            row.arguments = Arc::from([]);
                        } else if mutation == 5 {
                            let mut args = row.arguments.to_vec();
                            args[0].expectation = ir::CallExpectation::NoFirmReference;
                            row.arguments = args.into();
                        } else {
                            row.type_actual_regions = vec![ir::TypeActualRegion {
                                argument: 0,
                                location: ir::CarrierLocation {
                                    path: ir::CarrierPath(Arc::from([])),
                                    channel: ir::CarrierChannel::Fixed(ir::ReferencePosition {
                                        depth: 0,
                                    }),
                                },
                                source: ir::ActualRegion::Elided,
                            }]
                            .into();
                        }
                        forged.call_evidence = rows.into();
                    }
                }
                assert_eq!(
                    replay_source_body(&original, &Arc::new(forged), publication.facts(), &budget)
                        .expect_err(&format!("forged shared Deref mutation={mutation}"))
                        .kind(),
                    tr::FactsErrorKind::InvalidMetadata
                );
            }
            let function = plan.callable_body(&plan.program, declaration).unwrap();
            let inventory =
                SourceInventory::build(&plan.program, function, original.identity.clone(), &budget)
                    .unwrap();
            let pointer = function
                .body
                .statements
                .iter()
                .find_map(|statement| {
                    let Stmt::Let {
                        value: Expr::MethodCall { receiver, .. },
                        ..
                    } = statement
                    else {
                        return None;
                    };
                    Some(receiver.as_ref() as *const Expr as usize)
                })
                .unwrap();
            let second = inventory.call_pointers[&deref_coercion::site(pointer, 2)];
            for mutation in 0..3 {
                let mut forged = original.clone();
                match mutation {
                    0 => forged
                        .call_mapping
                        .retain(|(_, expression, _)| *expression != operator),
                    1 => {
                        let row = forged
                            .call_mapping
                            .iter_mut()
                            .find(|(_, expression, _)| *expression == operator)
                            .unwrap();
                        row.0 = second;
                    }
                    _ => forged.call_mapping.push(
                        original
                            .call_mapping
                            .iter()
                            .find(|(index, _, _)| index == source_index)
                            .unwrap()
                            .clone(),
                    ),
                }
                assert_eq!(
                    replay_source_body(&forged, checked, publication.facts(), &budget)
                        .unwrap_err()
                        .kind(),
                    tr::FactsErrorKind::InvalidMetadata
                );
            }
            let node = &inventory.calls[*source_index].node;
            for mutation in 0..3 {
                let mut inventory = SourceInventory::build(
                    &plan.program,
                    function,
                    original.identity.clone(),
                    &budget,
                )
                .unwrap();
                let first_key = deref_coercion::site(pointer, 1);
                match mutation {
                    0 => {
                        inventory.call_pointers.remove(&first_key);
                    }
                    1 => {
                        inventory.call_pointers.insert(first_key, second);
                    }
                    _ => {
                        inventory.calls[*source_index].operator_candidate = false;
                    }
                }
                assert_eq!(
                    Replay::shared_deref_site(&original, &inventory, node, &budget)
                        .unwrap_err()
                        .kind(),
                    tr::FactsErrorKind::InvalidMetadata
                );
            }
            let foreign_program = parse("fn main(){}").unwrap();
            let foreign_budget = ir::PlanBudget::default();
            let foreign = PlanCtx::from_program(&foreign_program, &foreign_budget).unwrap();
            let facts = foreign
                .callable_facts(foreign.function_decl(0).unwrap())
                .unwrap();
            assert_eq!(
                replay_source_body(&original, checked, &facts, &budget)
                    .unwrap_err()
                    .kind(),
                tr::FactsErrorKind::InvalidMetadata
            );
        }
    }

    #[test]
    fn closed_method_arguments_reconstruct_mut_receiver_and_reject_forged_evidence() {
        for result in [
            "struct Output{n:i32}impl Owner{fn make(&mut self,n:i32)->Output{self.n=n;Output{n:self.n}}}",
            "struct Output<'a>{r:&'a i32}impl Owner{fn make<'a>(&'a mut self,n:i32)->Output<'a>{self.n=n;Output{r:&self.n}}}",
        ] {
            let source = format!(
                "struct Owner{{n:i32}}{result}fn consume<T>(x:&T){{}}fn main(){{let mut owner=Owner{{n:0i32}};let mut sibling=Owner{{n:9i32}};let row=owner.make(7i32);consume(&row);}}"
            );
            let program = parse(&source).unwrap();
            let budget = ir::PlanBudget::default();
            let plan = PlanCtx::from_program(&program, &budget).unwrap();
            let declaration = plan.function_decl(program.functions.len() - 1).unwrap();
            let PublishedCallableBody::Source(publication) =
                plan.publish_callable_body(declaration, &budget).unwrap()
            else {
                panic!("expected original method source");
            };
            let original = publication.original().source_for_test();
            let checked = publication.source();
            assert!(
                !replay_source_body(&original, checked, publication.facts(), &budget)
                    .unwrap()
                    .is_empty(),
                "closed method control must demand source reconstruction"
            );
            let producer = checked
                .exprs
                .iter()
                .enumerate()
                .find_map(|(index, expression)| {
                    let ir::ExprKind::Call {
                        target: ir::Callee::Direct(target),
                        args,
                        ..
                    } = &expression.kind
                    else {
                        return None;
                    };
                    let info = plan.registry.declaration(target.declaration).unwrap();
                    (args.len() == 2
                        && matches!(
                            info.receiver,
                            ReceiverKind::Borrowed {
                                kind: ir::BorrowKind::Mut
                            }
                        ))
                    .then_some(ir::ExprId(index))
                })
                .unwrap();
            let receiver = match &checked.exprs[producer.0].kind {
                ir::ExprKind::Call { args, .. } => args[0],
                _ => unreachable!(),
            };
            for mutation in 0..6 {
                let mut forged = checked.as_ref().clone();
                match mutation {
                    0 => {
                        let ir::ExprKind::Call { args, .. } = &mut forged.exprs[producer.0].kind
                        else {
                            unreachable!()
                        };
                        args.pop();
                    }
                    1 => {
                        let ir::ExprKind::Call { args, .. } = &mut forged.exprs[producer.0].kind
                        else {
                            unreachable!()
                        };
                        args[1] = receiver;
                    }
                    2 => {
                        let ir::ExprKind::Borrow { place, .. } = &mut forged.exprs[receiver.0].kind
                        else {
                            unreachable!()
                        };
                        place.local = ir::LocalId(usize::MAX);
                    }
                    3 => {
                        let ir::ExprKind::Borrow { timing, .. } =
                            &mut forged.exprs[receiver.0].kind
                        else {
                            unreachable!()
                        };
                        *timing = ir::BorrowTiming::Immediate;
                    }
                    4 => {
                        let ir::ExprKind::Call {
                            target: ir::Callee::Direct(target),
                            ..
                        } = &mut forged.exprs[producer.0].kind
                        else {
                            unreachable!()
                        };
                        target.declaration = declaration;
                    }
                    _ => {
                        let mut rows = forged.call_evidence.to_vec();
                        let row = rows.iter_mut().find(|r| r.call == producer).unwrap();
                        let mut arguments = row.arguments.to_vec();
                        arguments[1].expectation = ir::CallExpectation::FirmReference {
                            kind: ir::BorrowKind::Shared,
                        };
                        row.arguments = arguments.into();
                        forged.call_evidence = rows.into();
                    }
                }
                let error =
                    replay_source_body(&original, &Arc::new(forged), publication.facts(), &budget)
                        .expect_err(&format!(
                            "forged closed method mutation={mutation} must fail"
                        ));
                assert_eq!(
                    error.kind(),
                    tr::FactsErrorKind::InvalidMetadata,
                    "mutation={mutation}"
                );
            }
            let mut forged = original.clone();
            forged
                .call_mapping
                .retain(|(_, expression, _)| *expression != producer);
            let error = replay_source_body(&forged, checked, publication.facts(), &budget)
                .expect_err("missing closed method source receipt");
            assert_eq!(error.kind(), tr::FactsErrorKind::InvalidMetadata);
            let foreign_plan = PlanCtx::from_program(&program, &budget).unwrap();
            let foreign = foreign_plan
                .callable_facts(
                    foreign_plan
                        .function_decl(program.functions.len() - 1)
                        .unwrap(),
                )
                .unwrap();
            assert_eq!(
                replay_source_body(&original, checked, &foreign, &budget)
                    .unwrap_err()
                    .kind(),
                tr::FactsErrorKind::InvalidMetadata
            );
        }
    }

    #[test]
    fn generic_method_chains_reconstruct_owner_and_reject_forged_evidence() {
        let source = "struct Owner<T>{n:T}struct View<'a,T>{r:&'a T}struct Slot<T>{v:T}impl<T> Owner<T>{fn slot<'a>(&'a self)->Slot<View<'a,T>>{Slot{v:View{r:&self.n}}}}impl<T> Slot<T>{fn take(self)->T{self.v}fn other(self)->T{self.v}}fn consume<T>(x:T){}fn main(){let owner=Owner{n:7i32};let sibling=Owner{n:9i32};let row=owner.slot().take();consume(row);}";
        let program = parse(source).unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let declaration = plan.function_decl(program.functions.len() - 1).unwrap();
        let PublishedCallableBody::Source(publication) =
            plan.publish_callable_body(declaration, &budget).unwrap()
        else {
            panic!("expected generic method source");
        };
        let original = publication.original().source_for_test();
        let checked = publication.source();
        replay_source_body(&original, checked, publication.facts(), &budget).unwrap();
        let (method, receiver) = checked
            .exprs
            .iter()
            .enumerate()
            .find_map(|(index, expr)| {
                let ir::ExprKind::Call {
                    target: ir::Callee::Direct(target),
                    args,
                    ..
                } = &expr.kind
                else {
                    return None;
                };
                let info = plan.declaration(target.declaration).unwrap();
                (matches!(
                    info.receiver,
                    ReceiverKind::Borrowed {
                        kind: ir::BorrowKind::Shared
                    }
                ) && args.len() == 1)
                    .then_some((ir::ExprId(index), args[0]))
            })
            .unwrap();
        let sibling = ir::LocalId(
            checked
                .locals
                .iter()
                .position(|local| local.name == "sibling")
                .unwrap(),
        );
        for mode in 0..5 {
            let mut payload = checked.as_ref().clone();
            match mode {
                0 => {
                    let ir::ExprKind::Call {
                        target: ir::Callee::Direct(target),
                        ..
                    } = &mut payload.exprs[method.0].kind
                    else {
                        unreachable!()
                    };
                    target.type_args = vec![ir::Ty::Bool].into();
                }
                1 => {
                    let ir::ExprKind::Borrow { place, .. } = &mut payload.exprs[receiver.0].kind
                    else {
                        unreachable!()
                    };
                    place.local = sibling;
                }
                2 => payload.exprs[method.0].ty = ir::Ty::Bool,
                3 => {
                    let ir::ExprKind::Call {
                        target: ir::Callee::Direct(target),
                        ..
                    } = &mut payload.exprs[method.0].kind
                    else {
                        unreachable!()
                    };
                    target.declaration = declaration;
                }
                _ => {
                    let ir::ExprKind::Call { args, .. } = &mut payload.exprs[method.0].kind else {
                        unreachable!()
                    };
                    args.clear();
                }
            }
            assert_eq!(
                replay_source_body(&original, &Arc::new(payload), publication.facts(), &budget)
                    .unwrap_err()
                    .kind(),
                tr::FactsErrorKind::InvalidMetadata,
                "mode {mode}"
            );
        }
        let mapping = original
            .call_mapping
            .iter()
            .find(|(_, expression, _)| *expression == method)
            .unwrap()
            .clone();
        let wrong_role = original
            .calls
            .iter()
            .position(|site| site.operator_candidate && site.node == original.calls[mapping.0].node)
            .unwrap();
        for mode in 0..3 {
            let mut source = original.clone();
            match mode {
                0 => source
                    .call_mapping
                    .retain(|(_, expression, _)| *expression != method),
                1 => {
                    source
                        .call_mapping
                        .iter_mut()
                        .find(|(_, expression, _)| *expression == method)
                        .unwrap()
                        .0 = wrong_role
                }
                _ => source.call_mapping.push(mapping.clone()),
            }
            assert_eq!(
                replay_source_body(&source, checked, publication.facts(), &budget)
                    .unwrap_err()
                    .kind(),
                tr::FactsErrorKind::InvalidMetadata,
                "original mapping mode {mode}"
            );
        }
        let other = PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap();
        let foreign = other
            .callable_facts(other.function_decl(program.functions.len() - 1).unwrap())
            .unwrap();
        assert_eq!(
            replay_source_body(&original, checked, &foreign, &budget)
                .unwrap_err()
                .kind(),
            tr::FactsErrorKind::InvalidMetadata
        );
    }

    #[test]
    fn thread_panicking_transplanted_source_fails_original_replay_and_publication() {
        let program =
            parse("fn query()->bool{std::thread::panicking()}fn ordinary()->bool{true}fn main(){}")
                .unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let query = plan.function_decl(0).unwrap();
        let ordinary = plan.function_decl(1).unwrap();
        let PublishedCallableBody::Source(query_publication) =
            plan.publish_callable_body(query, &budget).unwrap()
        else {
            panic!("expected canonical query caller source")
        };
        let PublishedCallableBody::Source(publication) =
            plan.publish_callable_body(ordinary, &budget).unwrap()
        else {
            panic!("expected canonical ordinary source")
        };
        publication
            .validate_source(publication.source(), publication.facts(), &budget)
            .unwrap();
        sr::replay_original_calls(
            publication.source().clone(),
            publication.original(),
            publication.facts(),
            &budget,
        )
        .unwrap();
        assert!(
            !publication
                .source()
                .exprs
                .iter()
                .any(|expression| matches!(expression.kind, ir::ExprKind::Call { .. }))
        );

        let mut transplanted = program.clone();
        transplanted.functions[1].body = program.functions[0].body.clone();
        // This low-level API types an explicitly supplied AST. It does not
        // authenticate that AST as the retained original declaration body.
        let typed =
            Arc::new(type_callable_with_plan(&transplanted, ordinary, &plan, &budget).unwrap());
        assert!(
            typed
                .exprs
                .iter()
                .any(|expression| matches!(expression.kind, ir::ExprKind::Call { .. }))
        );
        typed
            .validate_with_facts(ir::FactsCtx::Validated(publication.facts()))
            .unwrap();
        // Exercise the same immutable-original replay used by the private
        // publication factory, not merely an independently typed-body check.
        let error = sr::replay_original_calls(
            typed.clone(),
            publication.original(),
            publication.facts(),
            &budget,
        )
        .err()
        .expect("transplanted canonical call must not match the ordinary original source");
        assert_eq!(error.kind(), tr::FactsErrorKind::InvalidMetadata);
        for candidate in [&typed, query_publication.source()] {
            let error = publication
                .validate_source(candidate, publication.facts(), &budget)
                .expect_err("canonical source publication must reject a transplanted body");
            assert_eq!(error.kind(), tr::FactsErrorKind::InvalidMetadata);
        }
        publication
            .validate_source(publication.source(), publication.facts(), &budget)
            .unwrap();
    }

    #[test]
    fn builtin_method_producers_keep_canonical_occurrence_authority() {
        let program = parse("fn consume(a:&usize){}fn main(){let values=[1i32,2];let slice:&[i32]=&values;let n=slice.len();consume(&n);}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let declaration = plan.function_decl(1).unwrap();
        let PublishedCallableBody::Source(publication) =
            plan.publish_callable_body(declaration, &budget).unwrap()
        else {
            panic!("expected source")
        };
        let original = publication.original().source_for_test();
        let checked = publication.source();
        let function = plan.callable_body(&plan.program, declaration).unwrap();
        let inventory =
            SourceInventory::build(&plan.program, function, original.identity.clone(), &budget)
                .unwrap();
        let call = inventory
            .calls
            .iter()
            .position(|site| site.ordinary.is_some())
            .unwrap();
        assert!(
            recursive_argument_sources_supported(&plan, declaration, &inventory, call, &budget)
                .unwrap()
        );
        assert!(
            recursive_argument_sources_supported_with_original(
                &plan,
                declaration,
                &inventory,
                call,
                Some(&original),
                &budget,
            )
            .unwrap()
        );
        replay_source_body(&original, checked, publication.facts(), &budget).unwrap();
        let length = checked
            .exprs
            .iter()
            .position(|expr| matches!(expr.kind, ir::ExprKind::SliceLen { .. }))
            .unwrap();
        for mutation in 0..2 {
            let mut forged = checked.as_ref().clone();
            if mutation == 0 {
                forged.exprs[length].ty = ir::Ty::Bool;
            } else {
                forged.exprs[length].kind = ir::ExprKind::Bool(true);
            }
            let error =
                replay_source_body(&original, &Arc::new(forged), publication.facts(), &budget)
                    .expect_err("forged builtin length must fail");
            assert_eq!(error.kind(), tr::FactsErrorKind::InvalidMetadata);
        }
    }

    #[test]
    fn provisional_len_selection_keeps_nominal_method_fallbacks() {
        // These source-only fixtures do not load the canonical prelude. Real
        // Vec::len fallback coverage stays in typed_slices' differential suite.
        for (source, eligible) in [
            (
                "fn consume(a:&usize){}fn main(){let values=[1i32,2];let receiver:&[i32]=&values;let n=receiver.len();consume(&n);}",
                true,
            ),
            (
                "struct Buffer{value:usize}impl Buffer{fn len(&self)->usize{self.value}}fn consume(a:&usize){}fn main(){let receiver:Buffer=Buffer{value:2usize};let n=receiver.len();consume(&n);}",
                false,
            ),
            (
                "struct Owner<T>{value:T}impl<T> Owner<T>{fn len(&self)->usize{2usize}}fn consume(a:&usize){}fn main(){let receiver=Owner{value:1i32};let n=receiver.len();consume(&n);}",
                false,
            ),
            (
                "struct Owner{value:usize}impl Owner{fn len(&self)->usize{self.value}}fn consume(a:&usize){}fn main(){let values=[1i32,2];let receiver:&[i32]=&values;{let receiver=Owner{value:3usize};let n=receiver.len();consume(&n);}}",
                false,
            ),
        ] {
            let program = parse(source).unwrap();
            let budget = ir::PlanBudget::default();
            let plan = PlanCtx::from_program(&program, &budget).unwrap();
            let declaration = plan.function_decl(program.functions.len() - 1).unwrap();
            let PublishedCallableBody::Source(publication) =
                plan.publish_callable_body(declaration, &budget).unwrap()
            else {
                panic!("expected source")
            };
            let original = publication.original().source_for_test();
            let function = plan.callable_body(&plan.program, declaration).unwrap();
            let inventory =
                SourceInventory::build(&plan.program, function, original.identity.clone(), &budget)
                    .unwrap();
            let length = inventory
                .calls
                .iter()
                .find(|site| site.len_candidate)
                .unwrap();
            assert_eq!(
                authored_len_receiver(&inventory, &length.node, &budget).unwrap(),
                eligible,
                "{source}"
            );
            let consumer = inventory
                .calls
                .iter()
                .position(|site| {
                    site.ordinary.is_some() && site.argument_borrow == Some(ir::BorrowKind::Shared)
                })
                .unwrap();
            assert_eq!(
                recursive_argument_sources_supported(
                    &plan,
                    declaration,
                    &inventory,
                    consumer,
                    &budget,
                )
                .unwrap(),
                eligible,
                "prepublication route: {source}"
            );
        }
    }

    #[test]
    fn generic_len_inputs_reject_forged_receipts_and_receivers() {
        let program = parse("fn consume<T>(a:T,b:T){}fn main(){let values=[1i32,2];let slice:&[i32]=&values;let sibling:&[i32]=&values;let n=slice.len();let other=sibling.len();let m=2usize;consume(&n,&m);consume(&other,&m);}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let declaration = plan.function_decl(1).unwrap();
        let PublishedCallableBody::Source(publication) =
            plan.publish_callable_body(declaration, &budget).unwrap()
        else {
            panic!("expected source")
        };
        let original = publication.original().source_for_test();
        let checked = publication.source();
        replay_source_body(&original, checked, publication.facts(), &budget).unwrap();
        assert_eq!(original.builtin_len_mapping.len(), 2);
        let first = original.builtin_len_mapping[0];
        let second = original.builtin_len_mapping[1];
        for mutation in 0..7 {
            let mut source = original.clone();
            let mut payload = checked.as_ref().clone();
            match mutation {
                0 => {
                    source.builtin_len_mapping.remove(0);
                }
                1 => {
                    source.builtin_len_mapping[0].receiver = second.receiver;
                }
                2 => {
                    payload.exprs[first.expr.0].ty = ir::Ty::Bool;
                }
                3 => {
                    payload.exprs[first.expr.0].kind = ir::ExprKind::Int(ir::IntValue::usize(2));
                }
                4 => {
                    payload.exprs[first.expr.0].kind = ir::ExprKind::SliceLen {
                        slice: second.receiver,
                    };
                }
                5 => {
                    payload.exprs[first.receiver.0].kind =
                        payload.exprs[second.receiver.0].kind.clone();
                }
                6 => {
                    source.builtin_len_mapping[0].source_index = second.source_index;
                }
                _ => unreachable!(),
            }
            let error =
                replay_source_body(&source, &Arc::new(payload), publication.facts(), &budget)
                    .expect_err("generic input must not launder a forged len receipt");
            assert_eq!(
                error.kind(),
                tr::FactsErrorKind::InvalidMetadata,
                "mutation {mutation}"
            );
        }
    }

    #[test]
    fn source_concrete_method_producer_rejects_forged_authority() {
        for source in [
            "struct Owner{n:i32}impl Owner{fn get(&self)->&i32{&self.n}}fn first<T:Copy>(s:&[T])->T{s[0]}fn main(){let owner=Owner{n:7};let sibling=Owner{n:9};let row;{let rows=[(owner.get(),3usize)];row=first(&rows);}}",
            "struct Owner{n:i32}impl Owner{fn get(&self)->&i32{&self.n}}fn first<T:Copy>(s:&[T])->T{s[0]}fn main(){let sibling=Owner{n:9};let row;{let owner=Owner{n:7};let rows=[(owner.get(),3usize)];row=first(&rows);if row.1==3usize{let value=*row.0;}}}",
        ] {
            let program = parse(source).unwrap();
            let budget = ir::PlanBudget::default();
            let plan = PlanCtx::from_program(&program, &budget).unwrap();
            let declaration = plan.function_decl(program.functions.len() - 1).unwrap();
            let PublishedCallableBody::Source(publication) =
                plan.publish_callable_body(declaration, &budget).unwrap()
            else {
                panic!("expected original method producer source");
            };
            let original = publication.original().source_for_test();
            let checked = publication.source();
            replay_source_body(&original, checked, publication.facts(), &budget).unwrap();
            let (method, receiver) = checked.exprs.iter().enumerate().find_map(|(index, expression)| {
            let ir::ExprKind::Call { args, signature, .. } = &expression.kind else { return None };
            (matches!(&signature.result, ir::Ty::Ref { referent, .. } if **referent == ir::Ty::Int(ir::IntTy::I32))
                && args.len() == 1).then_some((ir::ExprId(index), args[0]))
        }).unwrap();
            let sibling = ir::LocalId(
                checked
                    .locals
                    .iter()
                    .position(|local| local.name == "sibling")
                    .unwrap(),
            );
            for mutation in 0..3 {
                let mut forged = checked.as_ref().clone();
                match mutation {
                    0 => {
                        let ir::ExprKind::Borrow { place, .. } = &mut forged.exprs[receiver.0].kind
                        else {
                            panic!("expected autoref");
                        };
                        place.local = sibling;
                    }
                    1 => {
                        forged.exprs[method.0].ty = ir::Ty::Ref {
                            kind: ir::BorrowKind::Shared,
                            referent: Arc::new(ir::Ty::Bool),
                        };
                    }
                    _ => {
                        let ir::ExprKind::Call { args, .. } = &mut forged.exprs[method.0].kind
                        else {
                            unreachable!()
                        };
                        args.clear();
                    }
                }
                let error =
                    replay_source_body(&original, &Arc::new(forged), publication.facts(), &budget)
                        .expect_err("forged method evidence must fail");
                assert_eq!(error.kind(), tr::FactsErrorKind::InvalidMetadata);
            }
            let mut forged = original.clone();
            forged
                .call_mapping
                .retain(|(_, expression, _)| *expression != method);
            let error = replay_source_body(&forged, checked, publication.facts(), &budget)
                .expect_err("missing original method receipt must fail");
            assert_eq!(error.kind(), tr::FactsErrorKind::InvalidMetadata);
        }
    }

    #[test]
    fn shared_identity_method_receiver_rejects_forged_source_evidence() {
        for receiver in ["\"left\"", "left"] {
            let source = format!(
                "struct View<'a>{{text:&'a str}}trait Source{{fn view(&self)->View<'_>;}}impl Source for str{{fn view(&self)->View<'_>{{View{{text:self}}}}}}fn consume<T>(value:T){{}}fn main(){{let left=\"left\";let sibling=\"right\";let row={receiver}.view();consume(row);}}"
            );
            let program = parse(&source).unwrap();
            let budget = ir::PlanBudget::default();
            let plan = PlanCtx::from_program(&program, &budget).unwrap();
            let declaration = plan.function_decl(program.functions.len() - 1).unwrap();
            let PublishedCallableBody::Source(publication) =
                plan.publish_callable_body(declaration, &budget).unwrap()
            else {
                panic!("expected original shared identity method source");
            };
            let original = publication.original().source_for_test();
            let checked = publication.source();
            assert!(
                !replay_source_body(&original, checked, publication.facts(), &budget)
                    .unwrap()
                    .is_empty()
            );
            let (method, receiver) = checked
                .exprs
                .iter()
                .enumerate()
                .find_map(|(index, expression)| {
                    let ir::ExprKind::Call {
                        target: ir::Callee::Direct(target),
                        args,
                        ..
                    } = &expression.kind
                    else {
                        return None;
                    };
                    let info = plan.declaration(target.declaration).unwrap();
                    (matches!(
                        info.receiver,
                        ReceiverKind::Borrowed {
                            kind: ir::BorrowKind::Shared
                        }
                    ) && args.len() == 1)
                        .then_some((ir::ExprId(index), args[0]))
                })
                .unwrap();
            let sibling = ir::LocalId(
                checked
                    .locals
                    .iter()
                    .position(|local| local.name == "sibling")
                    .unwrap(),
            );
            for mutation in 0..5 {
                let mut forged = checked.as_ref().clone();
                match mutation {
                    0 => match &mut forged.exprs[receiver.0].kind {
                        ir::ExprKind::Local(local) => *local = sibling,
                        ir::ExprKind::Str(text) => *text = "right".into(),
                        _ => panic!("expected original identity receiver value"),
                    },
                    1 => {
                        forged.exprs[receiver.0].ty = ir::Ty::Ref {
                            kind: ir::BorrowKind::Shared,
                            referent: Arc::new(ir::Ty::Bool),
                        }
                    }
                    2 => {
                        forged.exprs[receiver.0].kind = ir::ExprKind::Borrow {
                            kind: ir::BorrowKind::Shared,
                            place: ir::Place {
                                local: sibling,
                                projection: Vec::new(),
                            },
                            timing: ir::BorrowTiming::Immediate,
                        }
                    }
                    3 => {
                        let evidence = Arc::make_mut(&mut forged.call_evidence)
                            .iter_mut()
                            .find(|row| row.call == method)
                            .unwrap();
                        Arc::make_mut(&mut evidence.arguments)[0].expectation =
                            ir::CallExpectation::NoFirmReference;
                    }
                    _ => {
                        let ir::ExprKind::Call { args, .. } = &mut forged.exprs[method.0].kind
                        else {
                            unreachable!()
                        };
                        args.clear();
                    }
                }
                let error =
                    replay_source_body(&original, &Arc::new(forged), publication.facts(), &budget)
                        .expect_err("forged shared identity receiver must fail");
                assert_eq!(error.kind(), tr::FactsErrorKind::InvalidMetadata);
            }
            let mut forged = original.clone();
            forged
                .call_mapping
                .retain(|(_, expression, _)| *expression != method);
            assert_eq!(
                replay_source_body(&forged, checked, publication.facts(), &budget)
                    .unwrap_err()
                    .kind(),
                tr::FactsErrorKind::InvalidMetadata
            );
        }
    }

    #[test]
    fn generic_empty_static_producer_rejects_forged_authority() {
        // Result contexts are invariant, not call-input coercion sites.
        let invariant_budget = ir::PlanBudget::default();
        let mut state = sr::CallInferenceState::new(None, &invariant_budget).unwrap();
        let raw = state
            .type_term(
                &ir::Ty::RawPtr {
                    kind: ir::BorrowKind::Shared,
                    pointee: Arc::new(ir::Ty::Int(ir::IntTy::U32)),
                },
                None,
                0,
            )
            .unwrap();
        let reference = state
            .type_term(
                &ir::Ty::Ref {
                    kind: ir::BorrowKind::Shared,
                    referent: Arc::new(ir::Ty::Int(ir::IntTy::U32)),
                },
                None,
                0,
            )
            .unwrap();
        assert!(state.constrain(&raw, &reference, 0).is_err());
        for source in [
            "enum Slot<T>{Empty,Value(T)}impl<T> Slot<T>{fn empty()->Slot<T>{Slot::Empty}}fn take<T>(s:&Slot<T>){}fn main(){let slot:Slot<u32>=Slot::empty();take(&slot);}",
            "enum Slot<T>{Empty,Value(T)}impl<T> Slot<T>{fn empty()->Slot<T>{Slot::Empty}}struct Owner{slot:Slot<u32>}fn take<T>(s:&T){}fn main(){let owner=Owner{slot:Slot::empty()};take(&owner);}",
        ] {
            let program = parse(source).unwrap();
            let budget = ir::PlanBudget::default();
            let plan = PlanCtx::from_program(&program, &budget).unwrap();
            let declaration = plan.function_decl(program.functions.len() - 1).unwrap();
            let PublishedCallableBody::Source(publication) =
                plan.publish_callable_body(declaration, &budget).unwrap()
            else {
                panic!("expected original generic static producer source");
            };
            let original = publication.original().source_for_test();
            let checked = publication.source();
            replay_source_body(&original, checked, publication.facts(), &budget).unwrap();
            let producer = checked
                .exprs
                .iter()
                .enumerate()
                .find_map(|(index, expression)| {
                    let ir::ExprKind::Call {
                        target: ir::Callee::Direct(target),
                        args,
                        ..
                    } = &expression.kind
                    else {
                        return None;
                    };
                    (args.is_empty() && target.type_args.len() == 1).then_some(ir::ExprId(index))
                })
                .expect("generic static producer call");
            for mutation in 0..3 {
                let mut forged = checked.as_ref().clone();
                let ir::ExprKind::Call {
                    target: ir::Callee::Direct(target),
                    signature,
                    args,
                    ..
                } = &mut forged.exprs[producer.0].kind
                else {
                    unreachable!();
                };
                match mutation {
                    0 => target.type_args = vec![ir::Ty::Bool].into(),
                    1 => signature.result = ir::Ty::Bool,
                    _ => args.push(producer),
                }
                let error =
                    replay_source_body(&original, &Arc::new(forged), publication.facts(), &budget)
                        .expect_err("forged generic static producer must fail");
                assert_eq!(error.kind(), tr::FactsErrorKind::InvalidMetadata);
            }
            let mut forged = original.clone();
            forged
                .call_mapping
                .retain(|(_, expression, _)| *expression != producer);
            let error = replay_source_body(&forged, checked, publication.facts(), &budget)
                .expect_err("missing generic static source receipt must fail");
            assert_eq!(error.kind(), tr::FactsErrorKind::InvalidMetadata);
        }
    }

    #[test]
    fn generic_shared_static_producer_rejects_forged_authority() {
        let program = parse(
            "struct Wrap<T>{inner:T}impl<T> Wrap<T>{fn new(inner:T)->Wrap<T>{Wrap{inner}}}fn take<T>(s:&T){}fn main(){let n:u32=7;let r:&u32=&n;let wrap=Wrap::new(r);take(&wrap);}",
        ).unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let declaration = plan.function_decl(program.functions.len() - 1).unwrap();
        let PublishedCallableBody::Source(publication) =
            plan.publish_callable_body(declaration, &budget).unwrap()
        else {
            panic!("expected original shared static producer source");
        };
        let original = publication.original().source_for_test();
        let checked = publication.source();
        replay_source_body(&original, checked, publication.facts(), &budget).unwrap();
        let producer = checked
            .exprs
            .iter()
            .enumerate()
            .find_map(|(index, expression)| {
                let ir::ExprKind::Call {
                    target: ir::Callee::Direct(target),
                    args,
                    ..
                } = &expression.kind
                else {
                    return None;
                };
                let info = plan.registry.declaration(target.declaration).unwrap();
                (args.len() == 1 && info.path.associated.is_some() && target.type_args.len() == 1)
                    .then_some(ir::ExprId(index))
            })
            .expect("generic shared static producer call");
        for mutation in 0..5 {
            let mut forged = checked.as_ref().clone();
            match mutation {
                0 => {
                    let ir::ExprKind::Call {
                        target: ir::Callee::Direct(target),
                        ..
                    } = &mut forged.exprs[producer.0].kind
                    else {
                        unreachable!();
                    };
                    target.type_args = vec![ir::Ty::Ref {
                        kind: ir::BorrowKind::Shared,
                        referent: Arc::new(ir::Ty::Bool),
                    }]
                    .into();
                }
                1 => forged.exprs[producer.0].ty = ir::Ty::Bool,
                2 => {
                    let mut evidence = forged.call_evidence.to_vec();
                    let row = evidence
                        .iter_mut()
                        .find(|row| row.call == producer)
                        .unwrap();
                    row.type_actual_regions = Arc::from([]);
                    forged.call_evidence = evidence.into();
                }
                3 => {
                    let mut evidence = forged.call_evidence.to_vec();
                    let row = evidence
                        .iter_mut()
                        .find(|row| row.call == producer)
                        .unwrap();
                    let mut arguments = row.arguments.to_vec();
                    arguments[0].expectation = ir::CallExpectation::NoFirmReference;
                    row.arguments = arguments.into();
                    forged.call_evidence = evidence.into();
                }
                _ => {
                    let ir::ExprKind::Call { signature, .. } = &mut forged.exprs[producer.0].kind
                    else {
                        unreachable!();
                    };
                    signature.ref_contract = Arc::new(ir::RefContract::default());
                    signature.params[0] = ir::Ty::Bool;
                }
            }
            let error =
                replay_source_body(&original, &Arc::new(forged), publication.facts(), &budget)
                    .expect_err("forged shared static producer must fail");
            assert_eq!(error.kind(), tr::FactsErrorKind::InvalidMetadata);
        }
        let mut forged = original.clone();
        forged
            .call_mapping
            .retain(|(_, expression, _)| *expression != producer);
        let error = replay_source_body(&forged, checked, publication.facts(), &budget)
            .expect_err("missing shared static source receipt must fail");
        assert_eq!(error.kind(), tr::FactsErrorKind::InvalidMetadata);
    }

    #[test]
    fn associated_shared_method_producer_rejects_forged_authority() {
        let program = parse(
            "trait Duplicate{fn duplicate(&self)->Self;}struct Wrap<T>{inner:T}impl<T> Duplicate for Wrap<T>{fn duplicate(&self)->Self{loop{}}}fn take<T>(s:&T){}fn main(){let wrap=Wrap{inner:7u32};let copy=Wrap::duplicate(&wrap);take(&copy);}",
        ).unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let declaration = plan.function_decl(program.functions.len() - 1).unwrap();
        let PublishedCallableBody::Source(publication) =
            plan.publish_callable_body(declaration, &budget).unwrap()
        else {
            panic!("expected associated shared method source");
        };
        let original = publication.original().source_for_test();
        let checked = publication.source();
        replay_source_body(&original, checked, publication.facts(), &budget).unwrap();
        let producer = checked
            .exprs
            .iter()
            .enumerate()
            .find_map(|(index, expression)| {
                let ir::ExprKind::Call {
                    target: ir::Callee::Direct(target),
                    args,
                    ..
                } = &expression.kind
                else {
                    return None;
                };
                let info = plan.registry.declaration(target.declaration).unwrap();
                (args.len() == 1
                    && info
                        .path
                        .associated
                        .as_ref()
                        .is_some_and(|owner| owner.trait_decl.is_some()))
                .then_some(ir::ExprId(index))
            })
            .expect("associated shared method producer");
        for mutation in 0..4 {
            let mut forged = checked.as_ref().clone();
            match mutation {
                0 => {
                    let ir::ExprKind::Call {
                        target: ir::Callee::Direct(target),
                        ..
                    } = &mut forged.exprs[producer.0].kind
                    else {
                        unreachable!()
                    };
                    target.type_args = vec![ir::Ty::Bool].into();
                }
                1 => {
                    let ir::ExprKind::Call { signature, .. } = &mut forged.exprs[producer.0].kind
                    else {
                        unreachable!()
                    };
                    signature.result = ir::Ty::Bool;
                }
                2 => {
                    let ir::ExprKind::Call { args, .. } = &forged.exprs[producer.0].kind else {
                        unreachable!()
                    };
                    let receiver = args[0];
                    let ir::ExprKind::Borrow { kind, .. } = &mut forged.exprs[receiver.0].kind
                    else {
                        panic!("expected explicit shared receiver borrow")
                    };
                    *kind = ir::BorrowKind::Mut;
                }
                _ => {
                    let mut evidence = forged.call_evidence.to_vec();
                    let row = evidence
                        .iter_mut()
                        .find(|row| row.call == producer)
                        .unwrap();
                    let mut arguments = row.arguments.to_vec();
                    arguments[0].expectation = ir::CallExpectation::NoFirmReference;
                    row.arguments = arguments.into();
                    forged.call_evidence = evidence.into();
                }
            }
            let error =
                replay_source_body(&original, &Arc::new(forged), publication.facts(), &budget)
                    .expect_err("forged associated receiver producer must fail");
            assert_eq!(
                error.kind(),
                tr::FactsErrorKind::InvalidMetadata,
                "mutation {mutation}"
            );
        }
        let mut forged = original.clone();
        forged
            .call_mapping
            .retain(|(_, expression, _)| *expression != producer);
        let error = replay_source_body(&forged, checked, publication.facts(), &budget)
            .expect_err("associated producer requires original source correspondence");
        assert_eq!(error.kind(), tr::FactsErrorKind::InvalidMetadata);
    }

    #[test]
    fn annotated_byte_array_borrow_rejects_forged_temporary_evidence() {
        let program = parse("struct Wrap<T>{inner:T} impl<T> Wrap<T>{fn new(inner:T)->Wrap<T>{Wrap{inner}}} fn take<T>(s:&T){} fn main(){let data:&[u8]=&[3,4,5];let wrap=Wrap::new(data);take(&wrap);}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let declaration = plan.function_decl(program.functions.len() - 1).unwrap();
        let PublishedCallableBody::Source(publication) =
            plan.publish_callable_body(declaration, &budget).unwrap()
        else {
            panic!("expected source");
        };
        let original = publication.original().source_for_test();
        let checked = publication.source();
        replay_source_body(&original, checked, publication.facts(), &budget).unwrap();
        assert_eq!(original.array_borrows.len(), 1);
        let row = original.array_borrows[0].clone();
        let ir::ExprKind::Array(elements) = &checked.exprs[row.value.0].kind else {
            panic!("expected array");
        };
        let first = elements[0];
        for mutation in 0..12 {
            let mut source = original.clone();
            let mut payload = checked.as_ref().clone();
            match mutation {
                0 => source.array_borrows.clear(),
                1 => source.array_borrows[0].node = row.node.child(11, 0, &budget).unwrap(),
                2 => source.array_borrows[0].temporary = ir::LocalId(usize::MAX),
                3 => {
                    let ir::ExprKind::Block(block) = &mut payload.exprs[row.reference.0].kind
                    else {
                        unreachable!();
                    };
                    block.statements.clear();
                }
                4 => {
                    let ir::ExprKind::Block(block) = &mut payload.exprs[row.reference.0].kind
                    else {
                        unreachable!();
                    };
                    block.tail = Some(row.value);
                }
                5 => {
                    let ir::ExprKind::Borrow { place, .. } =
                        &mut payload.exprs[row.borrowed.0].kind
                    else {
                        unreachable!();
                    };
                    place.local = ir::LocalId(usize::MAX);
                }
                6 => {
                    let ir::ExprKind::Borrow { kind, .. } = &mut payload.exprs[row.borrowed.0].kind
                    else {
                        unreachable!();
                    };
                    *kind = ir::BorrowKind::Mut;
                }
                7 => payload.exprs[row.reference.0].completion = ir::Completion::Never,
                8 => payload.exprs[row.borrowed.0].ty = ir::Ty::Bool,
                9 => {
                    let ir::ExprKind::Int(value) = &mut payload.exprs[first.0].kind else {
                        unreachable!();
                    };
                    value.bits = 9;
                }
                10 => payload.exprs[first.0].ty = ir::Ty::Bool,
                11 => payload.locals[row.temporary.0].ty = ir::Ty::Bool,
                _ => unreachable!(),
            }
            let error =
                replay_source_body(&source, &Arc::new(payload), publication.facts(), &budget)
                    .expect_err("forged byte-array temporary must not replay");
            assert_eq!(
                error.kind(),
                tr::FactsErrorKind::InvalidMetadata,
                "mutation {mutation}"
            );
        }
    }

    #[test]
    fn source_float_arithmetic_rejects_forged_replay() {
        let program = parse(
            "fn consume<T>(a:T,b:T){} fn main(){let a=0.0f64/0.0f64;let b=1.0f64;consume(&a,&b);}",
        )
        .unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let declaration = plan.function_decl(1).unwrap();
        let PublishedCallableBody::Source(publication) =
            plan.publish_callable_body(declaration, &budget).unwrap()
        else {
            panic!("expected original source");
        };
        let checked = publication.source();
        sr::replay_original_calls(
            checked.clone(),
            publication.original(),
            publication.facts(),
            &budget,
        )
        .unwrap();
        let binary = checked
            .exprs
            .iter()
            .position(|expr| matches!(expr.kind, ir::ExprKind::Binary { .. }))
            .unwrap();
        let literal = checked
            .exprs
            .iter()
            .position(|expr| matches!(expr.kind, ir::ExprKind::Float(_)))
            .unwrap();
        for mutation in 0..3 {
            let mut forged = checked.as_ref().clone();
            if mutation == 0 {
                let ir::ExprKind::Binary { op, .. } = &mut forged.exprs[binary].kind else {
                    unreachable!()
                };
                *op = BinaryOp::Add;
            } else {
                let expression = &mut forged.exprs[literal];
                let ir::ExprKind::Float(value) = &mut expression.kind else {
                    unreachable!()
                };
                if mutation == 1 {
                    value.bits = 1.0f64.to_bits();
                } else {
                    *value = ir::FloatValue::f32(0.0);
                    expression.ty = ir::Ty::Float(ir::FloatTy::F32);
                }
            }
            let error = sr::replay_original_calls(
                Arc::new(forged),
                publication.original(),
                publication.facts(),
                &budget,
            )
            .err()
            .expect("forged float source must fail replay");
            assert_eq!(error.kind(), tr::FactsErrorKind::InvalidMetadata);
        }
    }

    #[test]
    fn source_field_integer_context_rejects_forged_literal() {
        let program = parse("struct Count<'a> { n: usize, r: &'a i32 } fn consume<T>(value: T) {} fn main() { let n = 7i32; consume(Count { n: 0, r: &n }); }").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let declaration = plan.function_decl(1).unwrap();
        let PublishedCallableBody::Source(publication) =
            plan.publish_callable_body(declaration, &budget).unwrap()
        else {
            panic!("expected original source");
        };
        sr::replay_original_calls(
            publication.source().clone(),
            publication.original(),
            publication.facts(),
            &budget,
        )
        .unwrap();
        let checked = publication.source();
        let constructor = checked
            .exprs
            .iter()
            .find_map(|expression| match &expression.kind {
                ir::ExprKind::Call { args, .. } => args.first().copied(),
                _ => None,
            })
            .unwrap();
        let ir::ExprKind::Struct { fields, .. } = &checked.exprs[constructor.0].kind else {
            panic!("expected Count")
        };
        let integer = fields[0].1.0;
        for change_type in [false, true] {
            let mut forged = checked.as_ref().clone();
            let expression = &mut forged.exprs[integer];
            let ir::ExprKind::Int(value) = &mut expression.kind else {
                panic!("expected integer")
            };
            assert_eq!(value.ty, ir::IntTy::Usize);
            if change_type {
                value.ty = ir::IntTy::U32;
                expression.ty = ir::Ty::Int(ir::IntTy::U32);
            } else {
                value.bits = 1;
            }
            let error = sr::replay_original_calls(
                Arc::new(forged),
                publication.original(),
                publication.facts(),
                &budget,
            )
            .err()
            .expect("forged literal must fail source replay");
            assert_eq!(error.kind(), tr::FactsErrorKind::InvalidMetadata);
        }
    }
}
