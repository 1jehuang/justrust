//! Declaration-owned nominal defaults. Raw nominal interning remains full-arity.
use super::*;
use crate::compiler::token_session::SpanId;

#[derive(Clone, Debug)]
pub(super) struct SourceDefault {
    pub(super) ty: Type,
    syntax: [SpanId; 2],
}
impl SourceDefault {
    pub(super) fn parsed(ty: Type, start: SpanId, end: SpanId) -> Self {
        Self {
            ty,
            syntax: [start, end],
        }
    }
    pub(super) fn syntax(&self, names: &NameStore) -> Result<[SpanId; 2], String> {
        for span in self.syntax {
            names.session().span(span).map_err(|error| {
                super::super::source::SourceError::from_token(error).to_string()
            })?;
        }
        Ok(self.syntax)
    }
}

#[derive(Clone, Copy)]
pub(super) enum Owner {
    TraitSelf,
    Nominal,
    Forbidden,
}

pub(super) fn validate_type(
    names: &NameStore,
    params: &[AstIdent],
    index: usize,
    ty: &Type,
) -> Result<(), String> {
    let mut inspect = |node: &DepthNode<'_>, _| {
        names
            .budget()
            .charge(super::super::expansion_budget::Counter::Work, 1)
            .map_err(|error| {
                super::super::source::SourceError::from_token(error.into()).to_string()
            })?;
        if let DepthNode::Type(node) = node {
            match node {
                Type::Param(parameter) => {
                    let key = names
                        .key(*parameter, LookupClass::TypeParameter)
                        .map_err(native_adapter::name_diagnostic)?;
                    let mut position = None;
                    for (index, candidate) in params.iter().enumerate() {
                        names
                            .budget()
                            .charge(super::super::expansion_budget::Counter::Work, 1)
                            .map_err(|error| {
                                super::super::source::SourceError::from_token(error.into())
                                    .to_string()
                            })?;
                        if names
                            .key(*candidate, LookupClass::TypeParameter)
                            .map_err(native_adapter::name_diagnostic)?
                            == key
                        {
                            position = Some(index);
                            break;
                        }
                    }
                    let position =
                        position.ok_or("default type parameter is outside its declaration")?;
                    if position >= index {
                        return Err(
                            "generic defaults cannot reference their own or a later parameter"
                                .into(),
                        );
                    }
                }
                Type::SelfType => {
                    return Err("Self is not available in nominal type defaults".into());
                }
                Type::Ref { .. }
                | Type::RawPtr { .. }
                | Type::Slice(_)
                | Type::Str
                | Type::Projection(_)
                | Type::Opaque(_)
                | Type::Dyn(_)
                | Type::Infer => {
                    return Err(
                        "unsupported: this nominal default type requires broader source transport"
                            .into(),
                    );
                }
                _ => {}
            }
        }
        Ok(())
    };
    walk_depth_inspecting(DepthNode::Type(ty), None, Some(names), Some(&mut inspect))
}

pub(super) fn validate_declaration(
    names: &NameStore,
    params: &[AstIdent],
    defaults: &[Option<Box<SourceDefault>>],
) -> Result<(), String> {
    if params.len() != defaults.len() {
        return Err(
            "contract: nominal default rows differ from the original parameter list".into(),
        );
    }
    let mut seen = false;
    for (index, default) in defaults.iter().enumerate() {
        names
            .budget()
            .charge(super::super::expansion_budget::Counter::Work, 1)
            .map_err(|error| {
                super::super::source::SourceError::from_token(error.into()).to_string()
            })?;
        if let Some(default) = default {
            default.syntax(names)?;
            validate_type(names, params, index, &default.ty)?;
            seen = true;
        } else if seen {
            return Err("generic parameters with a default must be trailing".into());
        }
    }
    Ok(())
}

pub(super) fn preflight_clone(
    defaults: &[Option<Box<SourceDefault>>],
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    source_clone::clone_vector(defaults, budget)?;
    for default in defaults.iter().flatten() {
        budget.charge_work(1)?;
        budget.retain_bytes(std::mem::size_of::<SourceDefault>())?;
        walk_depth(DepthNode::Type(&default.ty), Some(budget))?;
    }
    Ok(())
}

impl NominalSchema {
    pub(super) fn defaults(&self) -> &[Option<Box<SourceDefault>>] {
        match self {
            Self::Struct(source) => &source.type_defaults,
            Self::Enum(source) => &source.type_defaults,
        }
    }
}

impl DeclRegistry {
    pub(super) fn nominal_source_arity(
        &self,
        declaration: ir::DeclId,
        written: usize,
    ) -> Result<(), String> {
        let info = self.declaration(declaration)?;
        let defaults = match info.kind {
            ir::DeclKind::Struct => {
                &self
                    .program
                    .structs
                    .get(info.source_index)
                    .ok_or("missing nominal default declaration")?
                    .type_defaults
            }
            ir::DeclKind::Enum => {
                &self
                    .program
                    .enums
                    .get(info.source_index)
                    .ok_or("missing nominal default declaration")?
                    .type_defaults
            }
            _ => return Err("defaults require a nominal declaration".into()),
        };
        if defaults.len() != info.type_params.len()
            || written > defaults.len()
            || defaults[written..].iter().any(Option::is_none)
        {
            return Err("nominal type argument arity mismatch".into());
        }
        Ok(())
    }
}

impl PlanState {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn default_type_obligations(
        &mut self,
        registry: &DeclRegistry,
        ty: &ir::Ty,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
        output: &mut Vec<ir::Predicate>,
        depth: usize,
    ) -> Result<(), String> {
        budget.check_depth(depth)?;
        budget.charge_work(1)?;
        let (declaration, actuals) = match ty {
            ir::Ty::Struct(value) => (value.def().source_decl, value.def().type_args.as_ref()),
            ir::Ty::Enum(value) => (value.def().source_decl, value.def().type_args.as_ref()),
            ir::Ty::Tuple(parts) => {
                for part in parts.iter() {
                    self.default_type_obligations(
                        registry,
                        part,
                        caller,
                        budget,
                        output,
                        depth + 1,
                    )?;
                }
                return Ok(());
            }
            ir::Ty::Array { element, .. } => {
                return self.default_type_obligations(
                    registry,
                    element,
                    caller,
                    budget,
                    output,
                    depth + 1,
                );
            }
            _ => return Ok(()),
        };
        for actual in actuals {
            self.default_type_obligations(registry, actual, caller, budget, output, depth + 1)?;
        }
        let info = registry.declaration(declaration)?;
        let schema = self
            .schemas
            .get(&declaration)
            .cloned()
            .ok_or("missing nested default nominal schema")?;
        let bounds = match schema.as_ref() {
            NominalSchema::Struct(source) => &source.bounds,
            NominalSchema::Enum(source) => &source.bounds,
        };
        let (bindings, _storage) = self.schema_param_bindings(registry, info, actuals, budget)?;
        let previous = self.source_context;
        let context = registry.source_type_context(declaration)?;
        self.source_context = Some(context);
        let result =
            append_source_bounds(self, registry, &bindings, bounds, output, caller, budget);
        self.source_context = previous;
        result?;
        let sized = *registry
            .traits
            .get(&TraitLookupKey::Builtin(BuiltinTrait::Sized))
            .ok_or("missing canonical Sized identity")?;
        for (index, actual) in actuals.iter().enumerate() {
            budget.charge_work(1)?;
            let parameter = ir::ParamId {
                owner: info.param_env.binder(),
                index: index as u16,
            };
            if !info.param_env.relaxed_sized(parameter) {
                budget.retain_bytes(std::mem::size_of::<ir::Predicate>())?;
                output
                    .try_reserve_exact(1)
                    .map_err(|_| "infrastructure: default predicate allocation failed")?;
                output.push(ir::Predicate::Implements(ir::TraitRef {
                    declaration: sized,
                    self_ty: actual.clone(),
                    args: Arc::from([]),
                }));
            }
        }
        Ok(())
    }

    /// Fill only omitted source actuals, never raw intern or IR arguments.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn nominal_source_arguments(
        &mut self,
        registry: &DeclRegistry,
        declaration: ir::DeclId,
        mut actuals: Vec<ir::Ty>,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
        depth: usize,
        transport: Option<&ir::NominalSubstitutionRequest<'_>>,
    ) -> Result<Vec<ir::Ty>, String> {
        budget.check_depth(depth)?;
        registry.nominal_source_arity(declaration, actuals.len())?;
        let info = registry.declaration(declaration)?;
        if actuals.len() == info.type_params.len() {
            return Ok(actuals);
        }
        let schema = self
            .schemas
            .get(&declaration)
            .cloned()
            .ok_or("missing nominal defaults schema")?;
        let defaults = schema.defaults();
        budget.retain_bytes(
            info.type_params
                .len()
                .checked_mul(4 * std::mem::size_of::<(GenericBindingKey, ir::Ty)>())
                .ok_or("nominal default binding allocation overflow")?,
        )?;
        let mut bindings = HashMap::new();
        bindings
            .try_reserve(info.type_params.len())
            .map_err(|_| "infrastructure: nominal default binding allocation failed")?;
        // Reserve the complete replacement backing independently of the
        // source argument vector and the bindings HashMap admission above.
        budget.retain_bytes(
            info.type_params
                .len()
                .checked_mul(std::mem::size_of::<ir::Ty>())
                .ok_or("nominal default actual allocation overflow")?,
        )?;
        actuals
            .try_reserve_exact(info.type_params.len() - actuals.len())
            .map_err(|_| "infrastructure: nominal default actual allocation failed")?;
        // Exact unique source parameter keys bound the total number of
        // inserts to the capacity reserved above, including omitted slots.
        for (parameter, actual) in info.type_params.iter().zip(&actuals) {
            budget.charge_work(1)?;
            caller.validate_ty_metadata_with_budget(actual, budget)?;
            bindings.insert(registry.parameter_key(*parameter)?, actual.clone());
        }
        let context = registry.source_type_context(declaration)?;
        let previous_context = self.source_context;
        let previous_metadata = self.source_metadata.active.take();
        self.source_context = Some(context);
        let result = (|| {
            while actuals.len() < info.type_params.len() {
                budget.charge_work(1)?;
                let index = actuals.len();
                let default = defaults[index]
                    .as_ref()
                    .ok_or("missing original nominal default")?;
                for previous in &self.default_expansions {
                    budget.charge_work(1)?;
                    if *previous == (declaration, index) {
                        return Err("cyclic nominal generic default expansion".into());
                    }
                }
                budget.retain_bytes(std::mem::size_of::<(ir::DeclId, usize)>())?;
                self.default_expansions
                    .try_reserve_exact(1)
                    .map_err(|_| "infrastructure: nominal default cycle stack allocation failed")?;
                self.default_expansions.push((declaration, index));
                let actual = self.source_ty_with_transport(
                    registry,
                    &default.ty,
                    &bindings,
                    caller,
                    budget,
                    depth + 1,
                    false,
                    transport,
                );
                self.default_expansions.pop();
                let actual = actual?;
                caller.validate_ty_metadata_with_budget(&actual, budget)?;
                // Defaults do not synthesize reference/family source rows.
                // Rigid parameters are conservatively carrier-bearing too.
                if !ir::carrier_locations(&actual, budget)?.is_empty() {
                    return Err("unsupported: nominal defaults carrying references or rigid carrier families require original source region transport".into());
                }
                bindings.insert(
                    registry.parameter_key(info.type_params[index])?,
                    actual.clone(),
                );
                actuals.push(actual);
            }
            Ok(actuals)
        })();
        self.source_metadata.active = previous_metadata;
        self.source_context = previous_context;
        result
    }
}

impl PlanCtx {
    pub(super) fn complete_nominal_source_arguments(
        &self,
        declaration: ir::DeclId,
        actuals: Vec<ir::Ty>,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<Vec<ir::Ty>, String> {
        self.check_budget(budget)?;
        self.scratch().nominal_source_arguments(
            &self.registry,
            declaration,
            actuals,
            caller,
            budget,
            0,
            None,
        )
    }
}
