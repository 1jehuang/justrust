//! Original initializer-node member typing, separate from runtime call sites.
use super::*;

#[derive(Clone, Debug)]
pub(in crate::compiler) struct InitializerMemberInput {
    declaration: ir::DeclId,
    path: Arc<[u32]>,
    schema: tr::ConstantMemberType,
}
impl InitializerMemberInput {
    pub(in crate::compiler) fn ty(&self) -> &ir::Ty {
        self.schema.ty()
    }
    pub(in crate::compiler) fn path(&self) -> &[u32] {
        &self.path
    }
    pub(in crate::compiler) fn validate(
        &self,
        plan: &PlanCtx,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        let replay = plan.initializer_member_input(self.declaration, &self.path, budget)?;
        if self.schema != replay.schema {
            return Err(
                "constant initializer member type changed original declaration authority".into(),
            );
        }
        Ok(())
    }
}

impl PlanCtx {
    fn initializer_member_source(
        &self,
        declaration: ir::DeclId,
        path: &[u32],
        budget: &ir::PlanBudget,
    ) -> Result<&AssociatedConstUse, String> {
        self.check_budget(budget)?;
        budget.check_depth(path.len() / 2)?;
        if path.len() % 2 != 0 {
            return Err("constant initializer path has a partial coordinate".into());
        }
        let mut expression = self
            .associated_constant_source(declaration)?
            .initializer
            .as_ref()
            .ok_or("associated constant declaration has no original initializer")?;
        for step in path.chunks_exact(2) {
            budget.charge_work(1)?;
            expression = match (expression, step[0], step[1]) {
                (Expr::Unary { value, .. }, 1, 0) => value,
                (Expr::Binary { left, .. }, 2, 0) => left,
                (Expr::Binary { right, .. }, 2, 1) => right,
                (Expr::Call { args, .. }, 3, index) => args.get(index as usize).ok_or(
                    "constant initializer argument coordinate is outside its original call",
                )?,
                _ => {
                    return Err(
                        "constant initializer path does not match original expression shape".into(),
                    );
                }
            };
        }
        match expression {
            Expr::AssociatedConst(source) => Ok(source),
            _ => Err("constant initializer coordinate is not an original member use".into()),
        }
    }
    fn initializer_member_reference(
        &self,
        declaration: ir::DeclId,
        path: &[u32],
        actuals: Option<&[ir::Ty]>,
        facts: &tr::ValidatedFacts,
        budget: &ir::PlanBudget,
    ) -> Result<super::inherent::ConstantReference, String> {
        let source = self.initializer_member_source(declaration, path, budget)?;
        let header = self.associated_constant_header(declaration, budget)?;
        let mut bindings = generic_bindings(&self.registry, &header.env, budget)?;
        if let Some(owner) = header.owner_ty {
            bindings.insert(GenericBindingKey::SelfType, owner);
        }
        if let Some(actuals) = actuals {
            if actuals.len() != header.env.total_arity() {
                return Err("constant initializer selected argument arity changed".into());
            }
            let mut context = self.clone();
            for value in bindings.values_mut() {
                *value = ir::substitute_ty(
                    value,
                    &header.env,
                    actuals,
                    ir::FactsCtx::from_validated(facts),
                    &mut context,
                    budget,
                )?;
            }
        }
        self.associated_constant_application(declaration, source, &bindings, facts, actuals, budget)
    }
    pub(in crate::compiler) fn initializer_member_input(
        &self,
        declaration: ir::DeclId,
        path: &[u32],
        budget: &ir::PlanBudget,
    ) -> Result<InitializerMemberInput, String> {
        self.check_budget(budget)?;
        let facts = self
            .trait_world()?
            .associated_constant_facts(declaration)
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
        let reference =
            self.initializer_member_reference(declaration, path, None, &facts, budget)?;
        let schema = reference.schema(&facts, budget)?;
        schema
            .validate_for(&facts)
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
        budget.retain_bytes(
            std::mem::size_of::<InitializerMemberInput>() + std::mem::size_of_val(path),
        )?;
        Ok(InitializerMemberInput {
            declaration,
            path: Arc::from(path),
            schema,
        })
    }
    pub(in crate::compiler) fn select_initializer_member(
        &self,
        parent: &tr::SelectedAssociatedConstant,
        path: &[u32],
        facts: &tr::ValidatedFacts,
        budget: &ir::PlanBudget,
    ) -> Result<tr::SelectedAssociatedConstant, String> {
        parent
            .validate_for(facts)
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
        self.initializer_member_reference(
            parent.declaration(),
            path,
            Some(parent.arguments()),
            facts,
            budget,
        )?
        .value(facts, budget)
    }
}
