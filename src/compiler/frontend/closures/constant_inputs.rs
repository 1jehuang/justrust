//! Graph-sealed constants for the original stored body, never its empty adapter.
use super::super::*;
use std::cell::RefCell;

impl super::super::type_check::Checker<'_> {
    pub(in crate::compiler::frontend) fn source_constant_type(
        &self,
        expression: &Expr,
        missing: &str,
    ) -> Result<ir::Ty, String> {
        if let Some(source) = self.canonical.as_ref() {
            source.constant_type(expression)
        } else if let Some(source) = self.stored_constant_access.as_ref() {
            source.constant_type(expression)
        } else {
            Err(missing.to_owned())
        }
    }

    pub(in crate::compiler::frontend) fn source_constant_kind(
        &self,
        expression: &Expr,
        missing: &str,
    ) -> Result<ir::ExprKind, String> {
        if let Some(source) = self.canonical.as_ref() {
            source.constant_kind(expression)
        } else if let Some(source) = self.stored_constant_access.as_ref() {
            source.constant_kind(expression)
        } else {
            Err(missing.to_owned())
        }
    }

    pub(in crate::compiler::frontend) fn record_source_constant(
        &self,
        expression: &Expr,
        id: ir::ExprId,
        missing: &str,
    ) -> Result<(), String> {
        if let Some(source) = self.canonical.as_ref() {
            source.record_constant(expression, id)
        } else if let Some(source) = self.stored_constant_access.as_ref() {
            source.record(expression, id)
        } else {
            Err(missing.to_owned())
        }
    }
}

pub(in crate::compiler) struct StoredConstantInputs {
    parent: ir::DeclId,
    provider: ir::DeclId,
    site: u32,
    body: usize,
    facts: Arc<tr::ValidatedFacts>,
    constants: const_inputs::SealedBodyConstants,
    pub(in crate::compiler) children: Arc<[Arc<StoredConstantInputs>]>,
}

impl PlanCtx {
    pub(in crate::compiler) fn attach_stored_constant_inputs(
        &self,
        mut constants: const_inputs::SealedBodyConstants,
        inputs: Vec<Arc<StoredConstantInputs>>,
        budget: &ir::PlanBudget,
    ) -> Result<const_inputs::SealedBodyConstants, FactsError> {
        let prepared = self.prepare_body_constants(constants.declaration, budget)?;
        if prepared.stored().len() != inputs.len() || !constants.stored_inputs.is_empty() {
            return Err(FactsError::invalid(
                budget,
                "stored constant source frontier changed",
            ));
        }
        for (provider, input) in prepared.stored().iter().zip(&inputs) {
            if *provider != input.provider || input.parent != constants.declaration {
                return Err(FactsError::invalid(
                    budget,
                    "stored constant graph input changed parent/provider",
                ));
            }
            input
                .validate(self, budget)
                .map_err(|error| source_error(budget, &error))?;
        }
        constants.stored_inputs = inputs;
        Ok(constants)
    }

    fn stored_constant_source(
        &self,
        provider: ir::DeclId,
    ) -> Result<(ir::DeclId, &closures::StoredClosureSyntax), String> {
        let (parent, site) = self
            .stored_closure_owner(provider)?
            .ok_or("constant inputs require an original stored provider")?;
        let function = self.callable_body(&self.program, provider)?;
        let syntax = self
            .stored_closure_syntax(&function.name)?
            .ok_or("stored constant body disappeared")?;
        if syntax.site != site {
            return Err("stored constant source changed canonical site".into());
        }
        Ok((parent, syntax))
    }

    pub(in crate::compiler::frontend) fn stored_constant_inventory(
        &self,
        provider: ir::DeclId,
        identity: sr::OriginalBodyId,
        budget: &ir::PlanBudget,
    ) -> Result<SourceInventory, FactsError> {
        let (_, syntax) = self
            .stored_constant_source(provider)
            .map_err(|e| source_error(budget, &e))?;
        let function = self
            .callable_body(&self.program, provider)
            .map_err(|e| source_error(budget, &e))?;
        // Only constant/value occurrence rows escape this walk. No local,
        // binding or call evidence from the adapter's ABI is authorized here.
        SourceInventory::build_body(
            &self.program,
            function,
            &syntax.body,
            None,
            identity,
            budget,
        )
    }

    pub(in crate::compiler) fn prepare_stored_constants(
        &self,
        provider: ir::DeclId,
        budget: &ir::PlanBudget,
    ) -> Result<const_inputs::PreparedBodyConstants, FactsError> {
        self.check_budget(budget)
            .map_err(|e| source_error(budget, &e))?;
        let inventory =
            self.stored_constant_inventory(provider, sr::OriginalBodyId::new(budget)?, budget)?;
        self.prepare_constant_inventory(provider, Some(inventory), budget)
    }

    pub(in crate::compiler) fn seal_stored_constants(
        &self,
        provider: ir::DeclId,
        prepared: const_inputs::PreparedBodyConstants,
        values: Vec<Arc<super::super::super::const_check::ValueFact>>,
        children: Vec<Arc<StoredConstantInputs>>,
        budget: &ir::PlanBudget,
    ) -> Result<StoredConstantInputs, FactsError> {
        let (parent, syntax) = self
            .stored_constant_source(provider)
            .map_err(|e| source_error(budget, &e))?;
        if prepared.stored().len() != children.len() {
            return Err(FactsError::invalid(
                budget,
                "stored constant child frontier coverage changed",
            ));
        }
        for (expected, child) in prepared.stored().iter().zip(&children) {
            if child.parent != provider || child.provider != *expected {
                return Err(FactsError::invalid(
                    budget,
                    "stored constant child changed exact provider/parent",
                ));
            }
            child
                .validate(self, budget)
                .map_err(|e| source_error(budget, &e))?;
        }
        let constants = self.seal_body_constants(prepared, values, budget)?;
        if constants.declaration != provider {
            return Err(FactsError::invalid(
                budget,
                "stored constant sealing changed source declaration",
            ));
        }
        let facts = self
            .callable_facts(provider)
            .map_err(|e| source_error(budget, &e))?;
        sr::retain(
            budget,
            std::mem::size_of::<StoredConstantInputs>()
                + children.len() * std::mem::size_of::<Arc<StoredConstantInputs>>(),
        )?;
        let inputs = StoredConstantInputs {
            parent,
            provider,
            site: syntax.site,
            body: &syntax.body as *const Block as usize,
            facts,
            constants,
            children: children.into(),
        };
        inputs
            .validate(self, budget)
            .map_err(|error| source_error(budget, &error))?;
        Ok(inputs)
    }
}

impl StoredConstantInputs {
    fn validate(&self, plan: &PlanCtx, budget: &ir::PlanBudget) -> Result<(), String> {
        plan.check_budget(budget)?;
        budget.charge_work(1)?;
        let (parent, syntax) = plan.stored_constant_source(self.provider)?;
        let facts = plan.callable_facts(self.provider)?;
        if parent != self.parent
            || syntax.site != self.site
            || &syntax.body as *const Block as usize != self.body
            || self.constants.declaration != self.provider
            || self.facts.declaration() != self.provider
            || !Arc::ptr_eq(self.facts.world(), facts.world())
            || !self
                .facts
                .same_body_dispatch(&facts)
                .map_err(|e| e.into_legacy_diagnostic(budget))?
        {
            return Err("stored constant inputs changed original provider/body/facts".into());
        }
        let inventory = self
            .constants
            .inventory
            .as_ref()
            .ok_or("stored constant inventory missing")?;
        if inventory.root_block_pointer != self.body
            || inventory.constants.len() + inventory.associated_constants.len()
                != self.constants.uses.len() + self.constants.symbolic.len()
        {
            return Err("stored constant original occurrence coverage changed".into());
        }
        let fresh = plan
            .stored_constant_inventory(self.provider, inventory.owner.clone(), budget)
            .map_err(|error| error.into_legacy_diagnostic(budget))?;
        if inventory.constants != fresh.constants
            || inventory.constant_pointers != fresh.constant_pointers
            || inventory.associated_constant_pointers != fresh.associated_constant_pointers
            || inventory.associated_constants.len() != fresh.associated_constants.len()
        {
            return Err("stored constant rows changed original source occurrences".into());
        }
        for (index, (node, source)) in fresh.associated_constants.iter().enumerate() {
            if inventory.associated_constants[index].0 != *node {
                return Err("stored associated constant changed original source path".into());
            }
            if let Some(row) = self.constants.symbolic.get(index) {
                let original = plan.associated_constant_operand(
                    self.provider,
                    source,
                    node.path.clone(),
                    &facts,
                    budget,
                )?;
                if row.node != *node || row.operand.as_ref() != &original {
                    return Err(
                        "stored associated constant changed canonical source member proof".into(),
                    );
                }
            }
        }
        let prepared = plan
            .prepare_constant_inventory(self.provider, Some(fresh), budget)
            .map_err(|error| error.into_legacy_diagnostic(budget))?;
        if prepared.stored().len() != self.children.len()
            || prepared
                .stored()
                .iter()
                .zip(self.children.iter())
                .any(|(provider, child)| {
                    *provider != child.provider || child.parent != self.provider
                })
        {
            return Err("stored constant nested source frontier changed".into());
        }
        Ok(())
    }

    pub(in crate::compiler::frontend) fn find(
        inputs: &[Arc<Self>],
        provider: ir::DeclId,
    ) -> Option<Arc<Self>> {
        inputs
            .iter()
            .find(|input| input.provider == provider)
            .cloned()
    }
}

pub(in crate::compiler::frontend) struct StoredConstantAccess {
    plan: PlanCtx,
    inputs: Arc<StoredConstantInputs>,
    emissions: RefCell<Vec<(usize, ir::ExprId)>>,
}

#[derive(Clone)]
pub(in crate::compiler::frontend) struct StoredConstantReceipt {
    inputs: Arc<StoredConstantInputs>,
    emissions: Arc<[(usize, ir::ExprId)]>,
}

impl std::fmt::Debug for StoredConstantReceipt {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StoredConstantReceipt")
            .field("parent", &self.inputs.parent)
            .field("provider", &self.inputs.provider)
            .field("site", &self.inputs.site)
            .field("emissions", &self.emissions)
            .finish()
    }
}

impl StoredConstantAccess {
    pub(in crate::compiler::frontend) fn provider(&self) -> ir::DeclId {
        self.inputs.provider
    }

    pub(in crate::compiler::frontend) fn children(&self) -> &[Arc<StoredConstantInputs>] {
        &self.inputs.children
    }

    pub(in crate::compiler::frontend) fn new(
        plan: &PlanCtx,
        inputs: Arc<StoredConstantInputs>,
        parent: ir::DeclId,
        provider: ir::DeclId,
        body: &Block,
    ) -> Result<Self, String> {
        inputs.validate(plan, &plan.budget)?;
        if inputs.parent != parent
            || inputs.provider != provider
            || inputs.body != body as *const Block as usize
        {
            return Err("stored constant capability changed exact parent/provider/body".into());
        }
        Ok(Self {
            plan: plan.clone(),
            inputs,
            emissions: RefCell::new(Vec::new()),
        })
    }

    fn index(&self, expression: &Expr) -> Result<usize, String> {
        self.plan.budget.charge_work(1)?;
        let inventory = self
            .inputs
            .constants
            .inventory
            .as_ref()
            .ok_or("stored constant inventory missing")?;
        let pointer = expression as *const Expr as usize;
        if matches!(expression, Expr::AssociatedConst(_)) {
            inventory
                .associated_constant_pointers
                .get(&pointer)
                .map(|index| inventory.constants.len() + *index)
        } else {
            inventory.constant_pointers.get(&pointer).copied()
        }
        .ok_or_else(|| "stored constant lacks exact original body occurrence".into())
    }

    fn value(&self, index: usize) -> Result<(ir::Ty, ir::ExprKind), String> {
        let inventory = self
            .inputs
            .constants
            .inventory
            .as_ref()
            .ok_or("stored constant inventory missing")?;
        if index >= inventory.constants.len() && !self.inputs.constants.symbolic.is_empty() {
            let row = self
                .inputs
                .constants
                .symbolic
                .get(index - inventory.constants.len())
                .ok_or("stored symbolic constant input missing")?;
            let expected = &inventory
                .associated_constants
                .get(index - inventory.constants.len())
                .ok_or("stored symbolic source occurrence missing")?
                .0;
            if row.node != *expected
                || row.operand.caller() != self.inputs.provider
                || row.operand.path() != expected.path.as_ref()
            {
                return Err("stored symbolic constant changed provider/path".into());
            }
            row.operand
                .validate(&self.inputs.facts, &self.plan.budget)?;
            return Ok((
                row.operand.ty().clone(),
                ir::ExprKind::AssociatedConst(row.operand.clone()),
            ));
        }
        let row = self
            .inputs
            .constants
            .uses
            .get(index)
            .ok_or("stored constant sealed value missing")?;
        let expected = if index < inventory.constants.len() {
            &inventory.constants[index].0
        } else {
            &inventory
                .associated_constants
                .get(index - inventory.constants.len())
                .ok_or("stored source occurrence missing")?
                .0
        };
        if row.node != *expected
            || row.declaration != row.value.scalar().declaration()
            || row.value.body().source().declaration != row.declaration
        {
            return Err("stored constant changed source/value declaration".into());
        }
        row.value.body().source().validate_authority()?;
        let scalar = row.value.scalar().value();
        let kind = match scalar {
            crate::compiler::const_eval::ScalarValue::Int(value) => ir::ExprKind::Int(value),
            crate::compiler::const_eval::ScalarValue::Bool(value) => ir::ExprKind::Bool(value),
        };
        Ok((const_inputs::scalar_ir_type(scalar.ty()), kind))
    }

    pub(in crate::compiler::frontend) fn constant_type(
        &self,
        expression: &Expr,
    ) -> Result<ir::Ty, String> {
        self.value(self.index(expression)?).map(|row| row.0)
    }
    pub(in crate::compiler::frontend) fn constant_kind(
        &self,
        expression: &Expr,
    ) -> Result<ir::ExprKind, String> {
        self.value(self.index(expression)?).map(|row| row.1)
    }
    pub(in crate::compiler::frontend) fn record(
        &self,
        expression: &Expr,
        id: ir::ExprId,
    ) -> Result<(), String> {
        let index = self.index(expression)?;
        self.plan
            .budget
            .retain_bytes(std::mem::size_of::<(usize, ir::ExprId)>())?;
        let mut emissions = self.emissions.borrow_mut();
        if emissions
            .iter()
            .any(|(old, previous)| *old == index || *previous == id)
        {
            return Err("duplicate stored constant emission".into());
        }
        emissions.push((index, id));
        Ok(())
    }
    pub(in crate::compiler::frontend) fn finish(&self) -> Result<StoredConstantReceipt, String> {
        self.inputs.validate(&self.plan, &self.plan.budget)?;
        let inventory = self.inputs.constants.inventory.as_ref().unwrap();
        let emissions = self.emissions.borrow();
        if emissions.len() != inventory.constants.len() + inventory.associated_constants.len() {
            return Err("stored constant emissions omit an original occurrence".into());
        }
        self.plan
            .budget
            .retain_bytes(emissions.len() * std::mem::size_of::<(usize, ir::ExprId)>())?;
        Ok(StoredConstantReceipt {
            inputs: self.inputs.clone(),
            emissions: emissions.clone().into(),
        })
    }
}

impl StoredConstantReceipt {
    pub(in crate::compiler::frontend) fn validate_for(
        &self,
        plan: &PlanCtx,
        closure: &ir::ClosureTy,
        body: &ir::CheckedFunction,
    ) -> Result<(), String> {
        if self.inputs.parent != closure.owner()
            || self.inputs.provider != closure.provider()
            || self.inputs.site != closure.site()
        {
            return Err("stored constant receipt changed exact closure identity".into());
        }
        self.validate(plan, body)
    }
    pub(in crate::compiler::frontend) fn validate(
        &self,
        plan: &PlanCtx,
        body: &ir::CheckedFunction,
    ) -> Result<(), String> {
        let access = StoredConstantAccess {
            plan: plan.clone(),
            inputs: self.inputs.clone(),
            emissions: RefCell::new(Vec::new()),
        };
        self.inputs.validate(plan, &plan.budget)?;
        for (position, (index, id)) in self.emissions.iter().enumerate() {
            plan.budget.charge_work(position + 1)?;
            if self.emissions[..position]
                .iter()
                .any(|(old, previous)| old == index || previous == id)
            {
                return Err("stored constant receipt repeats an occurrence/emission".into());
            }
            let expression = body
                .exprs
                .get(id.0)
                .ok_or("stored constant checked emission disappeared")?;
            let (ty, kind) = access.value(*index)?;
            let matches = match (&kind, &expression.kind) {
                (ir::ExprKind::Int(a), ir::ExprKind::Int(b)) => a == b,
                (ir::ExprKind::Bool(a), ir::ExprKind::Bool(b)) => a == b,
                (ir::ExprKind::AssociatedConst(a), ir::ExprKind::AssociatedConst(b)) => a == b,
                _ => false,
            };
            if !matches || expression.ty != ty || expression.completion != ir::Completion::Returns {
                return Err(
                    "stored constant receipt changed exact type/value/source operand".into(),
                );
            }
        }
        let inventory = self.inputs.constants.inventory.as_ref().unwrap();
        if self.emissions.len() != inventory.constants.len() + inventory.associated_constants.len()
        {
            return Err("stored constant receipt coverage changed".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_constant_inputs_replay_original_provider_body_and_occurrences() {
        let program = parse(
            "trait Zero{const ZERO:i32;}fn form<T:Zero>(){let f=||T::ZERO;}fn other(){}fn main(){}",
        )
        .unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let (provider, owner) = (0..plan.callable_count())
            .find_map(|index| {
                let provider = plan.callable_decl(index).ok()?;
                plan.stored_closure_owner(provider)
                    .ok()
                    .flatten()
                    .map(|(owner, _)| (provider, owner))
            })
            .unwrap();
        let make = || {
            let prepared = plan.prepare_stored_constants(provider, &budget).unwrap();
            plan.seal_stored_constants(provider, prepared, Vec::new(), Vec::new(), &budget)
                .unwrap()
        };
        let inputs = make();
        assert_eq!(inputs.constants.symbolic.len(), 1);
        let (_, syntax) = plan.stored_constant_source(provider).unwrap();
        let expression = syntax.body.tail.as_deref().unwrap();
        let access =
            StoredConstantAccess::new(&plan, Arc::new(inputs), owner, provider, &syntax.body)
                .unwrap();
        assert!(matches!(
            access.constant_kind(expression).unwrap(),
            ir::ExprKind::AssociatedConst(_)
        ));
        assert!(access.constant_kind(&expression.clone()).is_err());
        assert!(access.finish().is_err());
        access.record(expression, ir::ExprId(0)).unwrap();
        assert!(access.record(expression, ir::ExprId(1)).is_err());
        assert_eq!(access.finish().unwrap().emissions.len(), 1);

        let mut changed = make();
        changed.parent = provider;
        assert!(changed.validate(&plan, &budget).is_err());
        let mut changed = make();
        changed.provider = owner;
        assert!(changed.validate(&plan, &budget).is_err());
        let mut changed = make();
        changed.site += 1;
        assert!(changed.validate(&plan, &budget).is_err());
        let mut changed = make();
        changed.body ^= 1;
        assert!(changed.validate(&plan, &budget).is_err());
        let mut changed = make();
        changed.facts = plan.callable_facts(owner).unwrap();
        assert!(changed.validate(&plan, &budget).is_err());
        let mut changed = make();
        changed.constants.symbolic[0].node.path = Arc::from([999]);
        assert!(changed.validate(&plan, &budget).is_err());
        let mut changed = make();
        changed
            .constants
            .inventory
            .as_mut()
            .unwrap()
            .associated_constant_pointers
            .clear();
        assert!(changed.validate(&plan, &budget).is_err());

        let prepared = plan.prepare_body_constants(owner, &budget).unwrap();
        let constants = plan
            .seal_body_constants(prepared, Vec::new(), &budget)
            .unwrap();
        let constants = plan
            .attach_stored_constant_inputs(constants, vec![Arc::new(make())], &budget)
            .unwrap();
        let (_, closures) = plan
            .publish_callable_body_with_closures(owner, constants, &budget)
            .unwrap();
        let checked = closures
            .iter()
            .find(|checked| checked.ty.provider() == provider)
            .unwrap();
        checked.authority.validate(&plan, checked).unwrap();
        let mut changed = checked.clone();
        let body = Arc::make_mut(&mut changed.body);
        let constant = body
            .exprs
            .iter_mut()
            .find(|expression| matches!(expression.kind, ir::ExprKind::AssociatedConst(_)))
            .unwrap();
        constant.kind = ir::ExprKind::Bool(false);
        assert!(changed.authority.validate(&plan, &changed).is_err());
    }
}
