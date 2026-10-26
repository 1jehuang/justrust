//! Recursive input evidence rebuilt from immutable source and lexical occurrences.
//! Checked types are comparison targets, never an inference source.
use super::*;

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

pub(super) fn supported(
    source: &Expr,
    node: &sr::SourceNode,
    function: &Function,
    inventory: &SourceInventory,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<bool, FactsError> {
    sr::depth(budget, depth)?;
    sr::work(budget, 1)?;
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
            if site.parameter.is_some() {
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
            budget,
            depth + 1,
        ),
        Expr::Repeat { value, .. } => supported(
            value,
            &node.child(11, 0, budget)?,
            function,
            inventory,
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
                    budget,
                    depth + 1,
                )? {
                    return Ok(false);
                }
            }
            Ok(true)
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
            self.expression(source, node, id, expected, depth)
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
                    trace.result_type(self.budget, &mut |decl, args| {
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
                }
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
        if let ir::ExprKind::Struct { ty: tag, .. } = &actual.kind {
            if !self.equal(&ty, &ir::Ty::Struct(tag.clone()))? {
                return Err(FactsError::invalid(
                    self.budget,
                    "input nominal constructor tag changed",
                ));
            }
        }
        if let ir::ExprKind::EnumVariant { ty: tag, .. } = &actual.kind {
            if !self.equal(&ty, &ir::Ty::Enum(tag.clone()))? {
                return Err(FactsError::invalid(
                    self.budget,
                    "input enum constructor tag changed",
                ));
            }
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
                hint.as_ref(),
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
