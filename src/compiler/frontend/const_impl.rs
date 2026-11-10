//! Declaration-owned mixed type/const slots in source impl headers.
use super::*;

impl Parser<'_, '_> {
    pub(super) fn impl_parameters(&mut self) -> Result<Vec<AstIdent>, ParseError> {
        self.type_params.clear();
        self.impl_const_params.clear();
        self.pending_self_defaults.clear();
        self.pending_type_defaults.clear();
        let mut regions = RegionDecls::default();
        let parameters = self.callable_generics(&mut regions)?;
        self.pending_regions = regions;
        let mut constants = std::mem::take(&mut self.impl_const_params);
        self.reserve_parser_items(&mut constants, self.const_params.len())?;
        constants.extend(self.const_params.iter().cloned());
        self.impl_const_params = constants;
        Ok(parameters)
    }
}

/// Publish only this source impl's final slot metadata during registration.
pub(super) fn install(
    registry: &mut DeclRegistry,
    program: &Program,
    index: usize,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    let block = program
        .impls
        .get(index)
        .ok_or("missing source const impl")?;
    if block.const_params.is_empty() {
        return Ok(());
    }
    let declaration = *registry
        .impl_ids
        .get(index)
        .ok_or("missing source const impl owner")?;
    let info = registry.declaration(declaration)?;
    validate_owner(info, block, index, declaration)?;
    budget.charge_work(block.type_params.len().saturating_add(1))?;
    budget.retain_bytes(block.type_params.len() * std::mem::size_of::<ir::GenericParamKind>())?;
    let mut kinds = Vec::new();
    kinds
        .try_reserve_exact(block.type_params.len())
        .map_err(|_| "impl const slot allocation failed")?;
    kinds.resize(block.type_params.len(), ir::GenericParamKind::Type);
    for parameter in &block.const_params {
        let slot = usize::from(parameter.slot);
        let written = block
            .type_params
            .get(slot)
            .ok_or("impl const slot outside source header")?;
        if program
            .names
            .key(*written, LookupClass::TypeParameter)
            .map_err(native_adapter::name_diagnostic)?
            != program
                .names
                .key(parameter.name, LookupClass::TypeParameter)
                .map_err(native_adapter::name_diagnostic)?
        {
            return Err("impl const slot changed source header order".into());
        }
        let kind = kinds
            .get_mut(slot)
            .ok_or("impl const slot outside source header")?;
        if *kind != ir::GenericParamKind::Type {
            return Err("duplicate source impl const slot".into());
        }
        *kind = ir::GenericParamKind::Const(parameter.ty);
    }
    let info = decl_mut(&registry.slots, &mut registry.declarations, declaration)?;
    info.param_env = Arc::new(info.param_env.with_slot_kinds(&kinds, budget)?);
    info.env = Arc::new(ir::GenericEnv::new(
        std::slice::from_ref(&info.param_env),
        budget,
    )?);
    Ok(())
}

fn validate_owner(
    info: &DeclInfo,
    block: &ImplDecl,
    index: usize,
    declaration: ir::DeclId,
) -> Result<(), String> {
    let expected_kind = if block.trait_path.is_some() {
        ir::DeclKind::TraitImpl
    } else {
        ir::DeclKind::Impl
    };
    if info.id != declaration || info.source_index != index || info.kind != expected_kind {
        return Err("const impl changed its source declaration owner".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn const_impl_registration_rejects_changed_owner_slot_and_const_kind() {
        let source = "trait A{fn len(&self)->usize;}impl<T,const N:usize>A for [T;N]{fn len(&self)->usize{N}}impl A for i32{fn len(&self)->usize{0}}fn main(){}";
        let program = parse(source).unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let expected = plan.registry.impl_ids[0];
        let world = plan.trait_world().unwrap();
        let implementation = world
            .impl_headers()
            .iter()
            .find(|header| header.declaration == expected)
            .unwrap();
        for predicate in implementation.predicates.iter() {
            if let ir::Ty::Param(parameter) = &predicate.trait_ref().self_ty {
                assert_eq!(
                    implementation.schema_env.parameter_kind(*parameter),
                    Some(ir::GenericParamKind::Type)
                );
            }
        }
        let foreign = plan
            .registry
            .declaration(plan.registry.impl_ids[1])
            .unwrap();
        assert!(
            validate_owner(foreign, &program.impls[0], 0, expected)
                .unwrap_err()
                .contains("owner")
        );

        let mut changed = program.clone();
        changed.impls[0].const_params[0].slot = 0;
        assert!(
            PlanCtx::from_program(&changed, &ir::PlanBudget::default())
                .err()
                .unwrap()
                .contains("source header order")
        );

        let mut changed = program.clone();
        changed.impls[0].const_params[0].ty = ir::ConstType::Bool;
        assert!(PlanCtx::from_program(&changed, &ir::PlanBudget::default()).is_err());

        // These signatures need per-element source lifetime selectors. Keep
        // both valid and invalid unused bodies outside executable admission
        // instead of erasing the difference between their source lifetimes.
        for source in [
            "fn keep<'a,const N:usize>(x:[&'a i32;N])->[&'a i32;N]{x}fn main(){}",
            "fn cross<'a,'b,const N:usize>(x:[&'a i32;N],y:[&'b i32;N])->[&'a i32;N]{y}fn main(){}",
        ] {
            let program = parse(source).unwrap();
            let error = PlanCtx::from_program(&program, &ir::PlanBudget::default())
                .err()
                .unwrap();
            assert!(
                error.contains(
                    "unsupported: explicit source reference regions in dependent array elements"
                ),
                "{error}"
            );
        }
    }
}
