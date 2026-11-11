//! Declaration-owned const slots on source structs and enums
//! (`struct M<const R: usize, T>`). Slots share the source order of type
//! parameters, so `ParamEnv` kinds address them like impl and fn const slots.
use super::*;

impl Parser<'_, '_> {
    pub(super) fn nominal_const_parameter(
        &mut self,
        params: &mut Vec<AstIdent>,
    ) -> Result<(), ParseError> {
        self.expect("const")?;
        let name = self.ident()?;
        self.expect(":")?;
        let ty = match self.ty()? {
            Type::Int(ty) => ir::ConstType::Int(ty),
            Type::Bool => ir::ConstType::Bool,
            Type::Char => ir::ConstType::Char,
            _ => {
                return Err(self.unsupported("const parameter type is not a supported scalar"));
            }
        };
        if self.is("=") {
            return Err(self.unsupported("const parameter defaults are unsupported"));
        }
        if self.pending_type_defaults.iter().any(Option::is_some)
            || !self.pending_self_defaults.is_empty()
        {
            return Err(self.error("generic parameters with a default must be trailing"));
        }
        if self.contains_name(params, name, LookupClass::TypeParameter)? {
            return Err(self.error("duplicate generic parameter"));
        }
        let slot = u16::try_from(params.len()).map_err(|_| self.limit("generic slot overflow"))?;
        self.reserve_parser_items(params, 1)?;
        params.push(name);
        let mut values = std::mem::take(&mut self.const_params);
        self.reserve_parser_items(&mut values, 1)?;
        values.push(SourceConstParameter { name, ty, slot });
        self.const_params = values;
        let mut rows = std::mem::take(&mut self.pending_type_defaults);
        self.reserve_parser_items(&mut rows, 1)?;
        rows.push(None);
        self.pending_type_defaults = rows;
        Ok(())
    }
}

/// Publish a source nominal's const slot kinds during registration.
pub(super) fn install(
    registry: &mut DeclRegistry,
    program: &Program,
    declaration: ir::DeclId,
    type_params: &[AstIdent],
    const_params: &[SourceConstParameter],
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    if const_params.is_empty() {
        return Ok(());
    }
    budget.charge_work(type_params.len().saturating_add(1))?;
    budget.retain_bytes(type_params.len() * std::mem::size_of::<ir::GenericParamKind>())?;
    let mut kinds = Vec::new();
    kinds
        .try_reserve_exact(type_params.len())
        .map_err(|_| "nominal const slot allocation failed")?;
    kinds.resize(type_params.len(), ir::GenericParamKind::Type);
    for parameter in const_params {
        let slot = usize::from(parameter.slot);
        let written = type_params
            .get(slot)
            .ok_or("nominal const slot outside source header")?;
        if program
            .names
            .key(*written, LookupClass::TypeParameter)
            .map_err(native_adapter::name_diagnostic)?
            != program
                .names
                .key(parameter.name, LookupClass::TypeParameter)
                .map_err(native_adapter::name_diagnostic)?
        {
            return Err("nominal const slot changed source header order".into());
        }
        let kind = kinds
            .get_mut(slot)
            .ok_or("nominal const slot outside source header")?;
        if *kind != ir::GenericParamKind::Type {
            return Err("duplicate nominal const slot".into());
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

impl Parser<'_, '_> {
    /// `N` in `[value; N]` when it names a visible const generic parameter
    /// and is the whole extent. Other names remain ordinary const evaluation.
    pub(super) fn const_parameter_extent(&mut self) -> Result<Option<AstIdent>, ParseError> {
        if !self.token().is_ident() || !self.cursor.peek(1).is_some_and(|next| next.matches("]")) {
            return Ok(None);
        }
        let token = self.input.mint_name(&self.token(), self.names)?;
        for declared in self.const_params.iter().chain(&self.impl_const_params) {
            if self.contains_name(&[declared.name], token, LookupClass::TypeParameter)? {
                self.cursor.bump();
                return Ok(Some(token));
            }
        }
        Ok(None)
    }
}
