//! Loader-issued authority for the original native Box declaration.
//! Consumers must first replay a genuine nominal read in their current world.
use super::*;

#[derive(Clone, Debug)]
pub struct NativeBoxSource {
    declaration: ir::DeclId,
    parameter: ir::ParamId,
    module: ModuleId,
    visibility: ir::Visibility,
    field_visibility: ir::Visibility,
    _source: crate::compiler::token_session::OriginalSpan,
    _issuer: crate::compiler::source::EmbeddedOriginalFile,
}

/// Exact allocation replayed from the original native declaration and its
/// canonical ordered source actuals. This is not an arbitrary layout receipt.
#[derive(Clone, Debug)]
pub struct NativeBoxApplication {
    definition: ir::StructTy,
    source: Arc<NativeBoxSource>,
}

impl NativeBoxApplication {
    pub(in crate::compiler) fn authorize_definition(
        &self,
        ty: &ir::Ty,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        budget.charge_work(1)?;
        let ir::Ty::Struct(owner) = ty else {
            return Err("native Box application requires its exact struct allocation".into());
        };
        if owner.is_back_edge()
            || owner.realization().is_some()
            || owner.native_sequence_binding().is_some()
            || !std::ptr::eq(owner.def(), self.definition.def())
        {
            return Err("native Box application changed its exact source allocation".into());
        }
        self.source.authorize_definition(ty, budget)
    }
}

pub(super) fn issue_application(
    adapter: &SchemaAdapter,
    ty: &ir::Ty,
    facts: &tr::ValidatedFacts,
) -> Result<Option<Arc<NativeBoxApplication>>, tr::FactsError> {
    let budget = &adapter.budget;
    budget
        .charge_work(2)
        .map_err(|error| adapter.error(error))?;
    if !matches!(
        facts.phase(),
        tr::FactsPhase::Closed | tr::FactsPhase::Universal
    ) || !facts.budget().same_authority(budget)
        || facts.registry_snapshot() != adapter.catalog.snapshot()
        || !facts.world().schema_is(adapter)
        || !Arc::ptr_eq(&adapter.program, &adapter.registry.program)
    {
        return Err(adapter.error("native Box application changed source authority".into()));
    }
    let ir::Ty::Struct(owner) = ty else {
        return Ok(None);
    };
    // Weak cycle-closing handles carry no independently readable source
    // definition. This optional service never grants them application authority,
    // and must not dereference an unfinished back edge to classify its role.
    if owner.is_back_edge() {
        return Ok(None);
    }
    let definition = owner.def();
    let Some(source) = issue(&adapter.registry, definition.source_decl, budget)
        .map_err(|error| adapter.error(error))?
    else {
        return Ok(None);
    };
    if owner.realization().is_some() {
        return Err(adapter
            .error("native Box source application requires a strong source definition".into()));
    }
    source
        .authorize_definition(ty, budget)
        .map_err(|error| adapter.error(error))?;
    let [actual] = definition.type_args.as_ref() else {
        return Err(adapter.error("native Box application changed actual arity".into()));
    };
    // Universal checking may construct cleanup only for an independently
    // closed actual in this exact caller. Generic or opaque actuals still defer
    // to the freshly lowered closed instance and receive no application proof.
    if facts.phase() == tr::FactsPhase::Universal && !facts.native_box_actual_is_closed(actual)? {
        return Ok(None);
    }
    facts.validate_ty_with_budget(actual, budget)?;
    validate_argument(actual, ir::FactsCtx::Validated(facts), budget, 0)
        .map_err(|error| adapter.error(error))?;
    budget
        .charge_work(3)
        .map_err(|error| adapter.error(error))?;
    budget
        .retain_bytes(std::mem::size_of::<ir::Ty>())
        .map_err(|error| adapter.error(error))?;
    if definition.id.snapshot != facts.registry_snapshot()
        || definition.source_decl.snapshot != definition.id.snapshot
        || owner.native_sequence_binding().is_some()
    {
        return Err(adapter.error("noncanonical native Box source application identity".into()));
    }
    match definition.id.index {
        ir::NominalIndex::ParentLocal { .. } => {
            // A Box whose actual contains a closure is minted by its owning
            // body, not the source interner. Replay its original source schema
            // and private receipt under these exact facts. This authenticates
            // the parent, ordinal, ordered actuals and every normalized field.
            adapter.validate_nominal_tree(ty, facts.env(), budget, false, Some(facts))?;
        }
        ir::NominalIndex::SourceSlot(slot)
            if slot
                == nominal_schema::canonical_slot(&(
                    definition.source_decl,
                    vec![actual.clone()],
                )) =>
        {
            // Recompute the original declaration, not the supplied definition's fields.
            let canonical = adapter
                .scratch()
                .intern(
                    &adapter.registry,
                    definition.source_decl,
                    std::slice::from_ref(actual),
                    ir::FactsCtx::Validated(facts),
                    budget,
                    0,
                )
                .map_err(|error| adapter.error(error))?;
            let ir::Ty::Struct(canonical) = canonical else {
                return Err(
                    adapter.error("native Box canonical source changed nominal kind".into())
                );
            };
            bounded_same_definition(owner, &canonical, budget)
                .map_err(|error| adapter.error(error))?;
        }
        _ => {
            return Err(adapter.error("noncanonical native Box source application identity".into()));
        }
    }
    budget
        .charge_nodes(1)
        .map_err(|error| adapter.error(error))?;
    budget
        .retain_bytes(
            std::mem::size_of::<NativeBoxApplication>() + 2 * std::mem::size_of::<usize>(),
        )
        .map_err(|error| adapter.error(error))?;
    Ok(Some(Arc::new(NativeBoxApplication {
        definition: owner.clone(),
        source,
    })))
}

fn bounded_same_definition(
    supplied: &ir::StructTy,
    canonical: &ir::StructTy,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    budget.charge_work(8)?;
    let a = supplied.def();
    let b = canonical.def();
    budget.charge_work(a.name.len().max(b.name.len()))?;
    if a.id != b.id
        || a.source_decl != b.source_decl
        || a.name != b.name
        || a.module != b.module
        || a.visibility != b.visibility
        || supplied.realization().is_some()
        || canonical.realization().is_some()
        || supplied.native_sequence_binding().is_some()
        || canonical.native_sequence_binding().is_some()
        || supplied.is_back_edge()
        || canonical.is_back_edge()
    {
        return Err("native Box canonical source definition payload changed".into());
    }
    let ([a_actual], [b_actual]) = (a.type_args.as_ref(), b.type_args.as_ref()) else {
        return Err("native Box canonical source actual arity changed".into());
    };
    if !ir::identical_type(a_actual, b_actual, budget)? {
        return Err("native Box canonical source actual changed".into());
    }
    let a_fields = a.fields.read(budget)?;
    let b_fields = b.fields.read(budget)?;
    let ([a], [b]) = (a_fields.as_ref(), b_fields.as_ref()) else {
        return Err("native Box canonical source field arity changed".into());
    };
    budget.charge_work(4)?;
    budget.charge_work(a.name.len().max(b.name.len()))?;
    let (
        ir::Ty::RawPtr {
            kind: a_kind,
            pointee: a_ty,
        },
        ir::Ty::RawPtr {
            kind: b_kind,
            pointee: b_ty,
        },
    ) = (&a.ty, &b.ty)
    else {
        return Err("native Box canonical source pointer payload changed".into());
    };
    if a.name != b.name
        || a.visibility != b.visibility
        || a.lifetime_bound != b.lifetime_bound
        || a_kind != b_kind
        || !ir::identical_type(a_ty, b_ty, budget)?
    {
        return Err("native Box canonical source field payload changed".into());
    }
    Ok(())
}

impl NativeBoxSource {
    pub(in crate::compiler) fn authorize_definition(
        &self,
        ty: &ir::Ty,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        budget.charge_work(3)?;
        let ir::Ty::Struct(owner) = ty else {
            return Err("native Box source authority requires a struct definition".into());
        };
        let definition = owner.def();
        if definition.source_decl != self.declaration
            || self.parameter.owner.declaration != self.declaration
            || definition.module != self.module
            || definition.visibility != self.visibility
        {
            return Err("native Box source authority changed its original owner".into());
        }
        let [actual] = definition.type_args.as_ref() else {
            return Err("native Box source authority changed actual arity".into());
        };
        let fields = definition.fields.read(budget)?;
        let [field] = fields.as_ref() else {
            return Err("native Box source authority changed field arity".into());
        };
        if field.name != "ptr"
            || field.visibility != self.field_visibility
            || !matches!(&field.ty, ir::Ty::RawPtr {kind: ir::BorrowKind::Mut, pointee} if ir::identical_type(pointee, actual, budget)?)
        {
            return Err("native Box source authority changed its exact pointer argument".into());
        }
        Ok(())
    }
}

pub(super) fn issue(
    registry: &DeclRegistry,
    declaration: ir::DeclId,
    budget: &ir::PlanBudget,
) -> Result<Option<Arc<NativeBoxSource>>, String> {
    budget.charge_work(2)?;
    let info = registry.declaration(declaration)?;
    if info.kind != ir::DeclKind::Struct {
        return Ok(None);
    }
    let source = registry
        .program
        .structs
        .get(info.source_index)
        .ok_or("native Box source authority lost its original declaration")?;
    let names = registry.program.names.as_ref();
    let Some(&member) = source.name.segments().last() else {
        return Ok(None);
    };
    let span = names
        .span(member)
        .map_err(native_adapter::name_diagnostic)?;
    let original = match names
        .session()
        .span(span)
        .map_err(|e| e.to_string())?
        .origin
    {
        crate::compiler::expansion_span::SpanOrigin::Original(original) => original,
        _ => return Ok(None),
    };
    let mut matching_issuer = None;
    for issuer in registry.program.embedded_issuers.iter() {
        budget.charge_work(1)?;
        if issuer.authorizes(names.session().sources(), original) {
            matching_issuer = Some(issuer);
            break;
        }
    }
    let Some(issuer) = matching_issuer else {
        return Ok(None);
    };
    // A protocol selector only after original embedded-file authentication.
    let expected = [
        crate::compiler::source::source_prelude::PRELUDE_MODULE,
        "boxed",
        "Box",
    ];
    if source.name.segments().len() != expected.len() {
        return Ok(None);
    }
    for (&segment, expected) in source.name.segments().iter().zip(expected) {
        budget.charge_work(1)?;
        if names
            .spelling(segment)
            .map_err(native_adapter::name_diagnostic)?
            != expected
        {
            return Ok(None);
        }
    }
    budget.charge_work(2)?;
    if info.owner.is_some()
        || info.path.module != source.module
        || info.path.kind != ir::DeclKind::Struct
        || info.path.associated.is_some()
        || !matches!(info.path.item, DeclItemName::Source(item) if exact_source_ident(names, item, member)?)
        || info.visibility != source.visibility
        || source.shape != StructShape::Record
        || source.type_params.len() != 1
        || !source.region_params.is_empty()
        || source.fields.len() != 1
        || info.param_env.arity() != 1
        || info.param_env.binder().declaration != declaration
        || info.type_params.len() != 1
        || !exact_source_ident(names, info.type_params[0], source.type_params[0])?
        || info.env.scopes().len() != 1
        || !Arc::ptr_eq(&info.env.scopes()[0], &info.param_env)
    {
        return Err("native Box original source owner or binder changed".into());
    }
    let parameter = ir::ParamId {
        owner: info.param_env.binder(),
        index: 0,
    };
    let original_parameter = source.type_params[0];
    let parameter_key = registry.parameter_key(original_parameter)?;
    let field = &source.fields[0];
    let FieldName::Named(field_name) = field.name else {
        return Err("native Box original field is not named".into());
    };
    let Type::RawPtr {
        kind: ir::BorrowKind::Mut,
        pointee,
    } = &field.ty
    else {
        return Err("native Box original field is not a mutable raw pointer".into());
    };
    let Type::Param(pointee_parameter) = pointee.as_ref() else {
        return Err("native Box original pointer lost its parameter".into());
    };
    if names
        .spelling(field_name)
        .map_err(native_adapter::name_diagnostic)?
        != "ptr"
        || registry.parameter_key(*pointee_parameter)? != parameter_key
        || field.visibility != ir::Visibility::Restricted(source.module)
    {
        return Err("native Box original pointer parameter or visibility changed".into());
    }
    // Bind all role-bearing occurrences to this same original embedded file.
    for occurrence in [original_parameter, field_name, *pointee_parameter] {
        budget.charge_work(1)?;
        let span = names
            .span(occurrence)
            .map_err(native_adapter::name_diagnostic)?;
        let origin = names
            .session()
            .span(span)
            .map_err(|e| e.to_string())?
            .origin;
        if !matches!(origin, crate::compiler::expansion_span::SpanOrigin::Original(original)
            if issuer.authorizes(names.session().sources(), original))
        {
            return Err("native Box source occurrence changed original issuer".into());
        }
    }
    let mut original_relaxed = false;
    for bound in &source.bounds {
        budget.charge_work(1)?;
        if let Type::Param(subject) = &bound.subject
            && registry.parameter_key(*subject)? == parameter_key
        {
            original_relaxed |= bound.maybe_unsized;
            if !bound.traits.is_empty() {
                return Err("native Box original parameter acquired extra bounds".into());
            }
        }
    }
    if !original_relaxed
        || !info.param_env.relaxed_sized(parameter)
        || !info.env.relaxed_sized(parameter)
    {
        return Err("native Box original relaxed parameter evidence changed".into());
    }
    budget.charge_nodes(1)?;
    budget
        .retain_bytes(std::mem::size_of::<NativeBoxSource>() + 2 * std::mem::size_of::<usize>())?;
    Ok(Some(Arc::new(NativeBoxSource {
        declaration,
        parameter,
        module: source.module,
        visibility: source.visibility.clone(),
        field_visibility: field.visibility.clone(),
        _source: original,
        _issuer: issuer.clone(),
    })))
}
