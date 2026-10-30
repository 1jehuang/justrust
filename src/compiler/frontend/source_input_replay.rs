//! Recursive input evidence rebuilt from immutable source and lexical occurrences.
//! Checked types are comparison targets, never an inference source.
use super::*;

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

fn precedes(binding: &sr::SourceNode, operand: &sr::SourceNode) -> bool {
    let Some((ordinal, parent)) = binding.path.split_last() else {
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
        Expr::Var(path) => return Ok(is_local_shaped_path(path)),
        Expr::Borrow { .. } | Expr::Integer(_) | Expr::Bool(_) | Expr::Char(_) => return Ok(true),
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
        } if type_args.is_empty()
            && args.is_empty()
            && matches!(receiver.as_ref(), Expr::Var(path) if is_local_shaped_path(path)) =>
        {
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

pub(super) fn supported(
    source: &Expr,
    node: &sr::SourceNode,
    function: &Function,
    inventory: &SourceInventory,
    original: Option<&OriginalSource>,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<bool, FactsError> {
    sr::depth(budget, depth)?;
    sr::work(budget, 1)?;
    if authored_float_width(source, budget, depth)?.is_some() {
        return Ok(true);
    }
    match source {
        Expr::Var(_) | Expr::Borrow { .. } => {
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
            let Some(Stmt::Let { value, .. }) =
                source_statement(function, inventory, &site.node, budget)?
            else {
                return Ok(false);
            };
            supported(
                value,
                &site.node.child(3, 0, budget)?,
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
            function,
            inventory,
            original,
            budget,
            depth + 1,
        ),
        Expr::Repeat { value, .. } => supported(
            value,
            &node.child(11, 0, budget)?,
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
        } if type_args.is_empty()
            && args.is_empty()
            && matches!(receiver.as_ref(), Expr::Var(path) if is_local_shaped_path(path)) =>
        {
            // Selection here is provisional. Only replay of the exact builtin
            // receipt and lexical receiver below grants SliceLen authority.
            sr::work(budget, inventory.calls.len())?;
            let Some(site) = inventory.calls.iter().find(|site| site.node == *node) else {
                return Ok(false);
            };
            if let Some(original) = original {
                let index = inventory
                    .calls
                    .iter()
                    .position(|site| site.node == *node)
                    .ok_or_else(|| {
                        FactsError::invalid(budget, "method producer occurrence missing")
                    })?;
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
                if !target.type_args.is_empty()
                    || view.generic_env().total_arity() != 0
                    || !original.facts.is_source_callable(target.declaration)?
                {
                    return Ok(false);
                }
                let declaration = original
                    .plan
                    .registry
                    .declaration(target.declaration)
                    .map_err(|e| source_error(budget, &e))?;
                if declaration.path.associated.is_none()
                    || !matches!(
                        declaration.receiver,
                        ReceiverKind::Borrowed {
                            kind: ir::BorrowKind::Shared
                        }
                    )
                {
                    return Ok(false);
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
            let receiver = node.child(10, 0, budget)?;
            Ok(inventory.operand_nodes.contains_key(&receiver.path))
        }
        Expr::Integer(_) | Expr::Bool(_) | Expr::Char(_) => Ok(true),
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
        self.original.facts.validate_signature_with(
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
        // A written annotation is the binding's authoritative type: the
        // checker already required the initializer to conform to it, so the
        // root contributes the annotation without replaying the initializer.
        // Only reference-free annotations qualify: no loan can flow through.
        if let Some(annotation) = annotation.as_ref()
            && !syntax(source, self.budget, depth)?
            && ir::carrier_locations(annotation, self.budget)
                .map_err(|e| source_error(self.budget, &e))?
                .is_empty()
        {
            if !self.equal(annotation, &declaration.ty)? {
                return Err(FactsError::invalid(
                    self.budget,
                    "input annotated binding type changed",
                ));
            }
            return Ok((local, annotation.clone()));
        }
        let ty = self.expression(
            source,
            &site.node.child(3, 0, self.budget)?,
            value,
            annotation.as_ref(),
            depth + 1,
        )?;
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
        if site.parameter.is_none() && !precedes(&site.node, node) {
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
        sr::work(self.budget, self.inventory.calls.len())?;
        let source_index = self
            .inventory
            .calls
            .iter()
            .position(|site| site.node == *node)
            .ok_or_else(|| {
                FactsError::invalid(self.budget, "method producer source occurrence missing")
            })?;
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
        sr::work(self.budget, self.inventory.calls.len())?;
        let index = self
            .inventory
            .calls
            .iter()
            .position(|site| site.node == *node)
            .ok_or_else(|| FactsError::invalid(self.budget, "len source occurrence missing"))?;
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
                sr::work(self.budget, self.inventory.calls.len())?;
                let source_index = self
                    .inventory
                    .calls
                    .iter()
                    .position(|site| site.node == *node)
                    .ok_or_else(|| {
                        FactsError::invalid(self.budget, "producer source occurrence missing")
                    })?;
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
            ) => self.concrete_method_result(source, node, id, target, signature, args)?,
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
                let resolved = resolve_integer(literal, false, expected)
                    .map_err(|e| source_error(self.budget, &e))?;
                if resolved != *value {
                    return Err(FactsError::invalid(self.budget, "input integer changed"));
                }
                ir::Ty::Int(resolved.ty)
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
