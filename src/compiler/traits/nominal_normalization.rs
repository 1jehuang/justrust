//! Normalize nominal actuals through their canonical source-application authority.
//! A field rewrite is never permission to reuse the original nominal identity.
use super::*;

impl ValidatedTraitWorld {
    pub(super) fn normalize_nominal_actuals(
        &self,
        original: &Ty,
        env: &GenericEnv,
        depth: usize,
        mut normalize: impl FnMut(&Ty) -> Result<Ty>,
    ) -> Result<Ty> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        match original {
            Ty::Struct(_) | Ty::Enum(_) => {}
            _ => {
                return Err(FactsError::invalid(
                    &self.budget,
                    "nominal normalization requires an original nominal application",
                ));
            }
        }
        let arguments = type_arguments(original);
        // Preserve the existing no-projection fast path. Callers validate raw
        // schema metadata independently before requesting normalization.
        if !self.type_contains_projection(original, depth + 1)? {
            return Ok(original.clone());
        }
        self.schema.validate_nominal(original, env, &self.budget)?;
        let mut normalized = Vec::new();
        for argument in arguments.iter() {
            push(&self.budget, &mut normalized, normalize(argument)?)?;
        }
        // This callback must remint from the immutable source declaration. It
        // receives the original canonical application, not a reusable numeric
        // ID or caller-supplied replacement field metadata.
        let result = self
            .schema
            .reapply_nominal(original, &normalized, env, &self.budget)?;
        self.validate_reapplied_nominal(original, &normalized, &result, env, depth)?;
        if self.type_contains_projection(&result, depth + 1)? {
            return Err(FactsError::unsupported(
                &self.budget,
                "unsupported: residual projection in source nominal fields after actual normalization",
            ));
        }
        Ok(result)
    }

    fn validate_reapplied_nominal(
        &self,
        original: &Ty,
        normalized: &[Ty],
        result: &Ty,
        env: &GenericEnv,
        depth: usize,
    ) -> Result<()> {
        self.schema.validate_nominal(original, env, &self.budget)?;
        self.schema.validate_nominal(result, env, &self.budget)?;
        let declaration = match original {
            Ty::Struct(value) => value.def().source_decl,
            Ty::Enum(value) => value.def().source_decl,
            _ => {
                return Err(FactsError::invalid(
                    &self.budget,
                    "nominal reapplication lost original application",
                ));
            }
        };
        let (result_declaration, actuals) = match (original, &result) {
            (Ty::Struct(_), Ty::Struct(value)) => (value.def().source_decl, &value.def().type_args),
            (Ty::Enum(_), Ty::Enum(value)) => (value.def().source_decl, &value.def().type_args),
            _ => {
                return Err(FactsError::invalid(
                    &self.budget,
                    "nominal normalization changed the source nominal kind",
                ));
            }
        };
        if result_declaration != declaration || actuals.len() != normalized.len() {
            return Err(FactsError::invalid(
                &self.budget,
                "nominal normalization changed source declaration or argument arity",
            ));
        }
        for (actual, expected) in actuals.iter().zip(normalized) {
            if !self.eq_ty(actual, expected, depth + 1)? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "nominal normalization changed a normalized actual argument",
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::frontend::{self, PlanCtx};

    fn source_applications() -> (Arc<ValidatedTraitWorld>, Vec<Ty>, Arc<GenericEnv>) {
        let program = frontend::parse(
            "struct Wrap<T>(T);struct Other<T>(T);enum Choice<T>{Value(T)}\
             fn first(x:Wrap<i32>){}fn second(x:Wrap<bool>){}\
             fn third(x:Other<bool>){}fn fourth(x:Choice<bool>){}fn main(){}",
        )
        .unwrap();
        let plan = PlanCtx::from_program(&program, &PlanBudget::default()).unwrap();
        let world = plan.trait_world().unwrap();
        let applications = (0..4)
            .map(|index| {
                world
                    .callable_header(plan.callable_decl(index).unwrap())
                    .unwrap()
                    .schema
                    .signature
                    .params[0]
                    .clone()
            })
            .collect();
        let env = world
            .callable_header(plan.callable_decl(0).unwrap())
            .unwrap()
            .schema
            .env
            .clone();
        (world, applications, env)
    }

    #[test]
    fn reapplication_requires_exact_source_kind_and_normalized_actuals() {
        let (world, applications, env) = source_applications();
        let original = &applications[0];
        let normalized = [Ty::Bool];
        world
            .validate_reapplied_nominal(original, &normalized, &applications[1], &env, 0)
            .unwrap();
        for result in [&applications[0], &applications[2], &applications[3]] {
            let error = world
                .validate_reapplied_nominal(original, &normalized, result, &env, 0)
                .unwrap_err();
            assert_eq!(error.kind(), FactsErrorKind::InvalidMetadata);
        }
        let error = world
            .validate_reapplied_nominal(original, &[], &applications[1], &env, 0)
            .unwrap_err();
        assert_eq!(error.kind(), FactsErrorKind::InvalidMetadata);
    }

    #[test]
    fn reapplication_rejects_foreign_and_rewritten_application_payloads() {
        let (world, applications, env) = source_applications();
        let (_, foreign, _) = source_applications();
        assert!(
            world
                .validate_reapplied_nominal(&applications[0], &[Ty::Bool], &foreign[1], &env, 0)
                .is_err()
        );
        let Ty::Struct(value) = &applications[1] else {
            panic!("source fixture is not a struct");
        };
        let Ty::Struct(other) = &applications[2] else {
            panic!("source fixture is not a struct");
        };
        let mut rebound = value.def().clone();
        rebound.source_decl = other.def().source_decl;
        let rebound = Ty::Struct(ir::StructTy::new(rebound));
        assert!(
            world
                .validate_reapplied_nominal(&applications[0], &[Ty::Bool], &rebound, &env, 0)
                .is_err()
        );
        let mut rewritten = value.def().clone();
        rewritten.fields = Arc::from([ir::FieldDef {
            name: "0".into(),
            ty: Ty::Int(ir::IntTy::I32),
            visibility: rewritten.fields[0].visibility.clone(),
            lifetime_bound: false,
        }]);
        let forged = Ty::Struct(ir::StructTy::new(rewritten));
        assert!(
            world
                .validate_reapplied_nominal(&applications[0], &[Ty::Bool], &forged, &env, 0)
                .is_err()
        );
    }

    #[test]
    fn projection_free_path_preserves_application_and_rejects_non_nominals() {
        let (world, applications, env) = source_applications();
        let result = world
            .normalize_nominal_actuals(&applications[0], &env, 0, |_| {
                panic!("projection-free application must not need reminting")
            })
            .unwrap();
        assert_eq!(result, applications[0]);
        assert!(
            world
                .normalize_nominal_actuals(&Ty::Bool, &env, 0, |_| unreachable!())
                .is_err()
        );
    }
}
