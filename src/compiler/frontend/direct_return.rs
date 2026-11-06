//! Original, complete direct-return syntax, separate from declaration envelopes.
use super::*;
use crate::compiler::token_session::OriginalSpan;

#[derive(Clone, Debug)]
pub(in crate::compiler) struct ParsedSource {
    source: OriginalSpan,
    module: ModuleId,
    scope: Option<super::super::semantic_scope::ScopeId>,
    name: AstIdent,
    generic: AstIdent,
    binding: AstIdent,
    reference: AstIdent,
    field: AstIdent,
    input: Type,
    output: Type,
    _storage: super::super::expansion_budget::RetainedBytes,
}

// This grammar has no statements, implicit calls, dereference selectors, or
// control flow. A receipt is not issued for merely similar return syntax.
fn shape(function: &Function, parsed: bool) -> Option<(AstIdent, AstIdent, AstIdent)> {
    if function.unsafety
        || function.static_mut
        || function.constness != Constness::Runtime
        || function.type_params.len() != 1
        || !function.const_params.is_empty()
        || !function.bounds.is_empty()
        || function.impl_params != 0
        || !function.regions.params.is_empty()
        || !function.regions.outlives.is_empty()
        || function.regions.receiver.is_some()
        || function.params.len() != 1
        || function.param_mut.iter().any(|mutable| *mutable)
        || function.suspended_source.is_some()
        || !function.body.statements.is_empty()
    {
        return None;
    }
    let (
        BindingName::Named(binding),
        Type::Ref {
            kind: ir::BorrowKind::Mut,
            region: None,
            referent,
        },
    ) = &function.params[0]
    else {
        return None;
    };
    let Type::Apply(application) = referent.as_ref() else {
        return None;
    };
    let mut applications = application
        .segments
        .iter()
        .filter(|segment| !segment.args.is_empty());
    let argument = applications.next()?;
    if (parsed && (application.segments.len() != 1 || application.anchor != PathAnchor::Relative))
        || applications.next().is_some()
        || argument.args.len() != 1
        || application
            .segments
            .iter()
            .any(|segment| !segment.region_args.is_empty() || !segment.associated_types.is_empty())
        || !matches!(argument.args[0], Type::Param(_))
        || !matches!(&function.result, Type::Ref { kind: ir::BorrowKind::Mut, region: None, referent } if matches!(referent.as_ref(), Type::Param(_)))
    {
        return None;
    }
    let Expr::Borrow {
        kind: ir::BorrowKind::Mut,
        place,
    } = function.body.tail.as_deref()?
    else {
        return None;
    };
    let LocalName::Named(reference) = place.local else {
        return None;
    };
    let [FieldSelector::Named(field)] = place.fields.as_slice() else {
        return None;
    };
    Some((*binding, reference, *field))
}

pub(super) fn issue(
    parser: &Parser<'_, '_>,
    start: Option<usize>,
    function: &Function,
    associated: bool,
    external: bool,
) -> Result<Option<Arc<ParsedSource>>, String> {
    if associated || external {
        return Ok(None);
    }
    let Some((binding, reference, field)) = shape(function, true) else {
        return Ok(None);
    };
    let (Some(start), Some(end)) = (start, parser.cursor.index()) else {
        return Ok(None);
    };
    let scope = function.lexical_scope;
    let Some(tokens) = parser.input.get(start..end) else {
        return Ok(None);
    };
    let Some(source) = function_source::original_token_range(parser.names, tokens)? else {
        return Ok(None);
    };
    // The complete statement-free original grammar has one formal and no
    // shadows. Root-context lookup proves this use denotes that formal.
    // Expanded tokens declined above cannot project a nonroot context here.
    if parser
        .names
        .key(binding, LookupClass::Local)
        .map_err(native_adapter::name_diagnostic)?
        != parser
            .names
            .key(reference, LookupClass::Local)
            .map_err(native_adapter::name_diagnostic)?
    {
        return Ok(None);
    }
    let Some(&name) = function.name.segments().last() else {
        return Ok(None);
    };
    // The finite grammar above bounds the cloned type payloads: two reference
    // boxes, one application segment and one parameter actual. Paths retain
    // their existing immutable occurrence storage.
    use crate::compiler::expansion_budget::Storage;
    let bytes = std::mem::size_of::<ParsedSource>()
        + 4 * std::mem::size_of::<Type>()
        + std::mem::size_of::<PathSegment>();
    let storage = parser
        .names
        .budget()
        .reserve(Storage::Retained, bytes as u64)
        .and_then(|lease| lease.retain())
        .map_err(|error| error.to_string())?;
    Ok(Some(Arc::new(ParsedSource {
        source,
        module: function.module,
        scope,
        name,
        generic: function.type_params[0],
        binding,
        reference,
        field,
        input: function.params[0].1.clone(),
        output: function.result.clone(),
        _storage: storage,
    })))
}

// Canonical resolution may add path prefixes. Only its certified original use
// can replay a parser occurrence. Actual arguments remain exact source syntax.
fn same_type(
    names: &NameStore,
    a: &Type,
    b: &Type,
    budget: &ir::PlanBudget,
) -> Result<bool, String> {
    budget.charge_work(1)?;
    Ok(match (a, b) {
        (Type::Param(a), Type::Param(b)) => exact_source_ident(names, *a, *b)?,
        (
            Type::Ref {
                kind: ak,
                region: None,
                referent: a,
            },
            Type::Ref {
                kind: bk,
                region: None,
                referent: b,
            },
        ) => ak == bk && same_type(names, a, b, budget)?,
        (Type::Apply(a), Type::Apply(b)) => {
            let original = |path: &GenericPath| {
                path.path(names)?
                    .original_use(names)
                    .map_err(native_adapter::name_diagnostic)
            };
            let ap = original(a)?;
            let bp = original(b)?;
            let mut equal = ap.anchor() == bp.anchor() && ap.len() == bp.len();
            for (a, b) in ap.segments().iter().zip(bp.segments()) {
                budget.charge_work(1)?;
                equal &= exact_source_ident(names, *a, *b)?;
            }
            budget.charge_work(a.segments.len().saturating_add(b.segments.len()))?;
            let mut aa = a.segments.iter().filter(|segment| !segment.args.is_empty());
            let mut ba = b.segments.iter().filter(|segment| !segment.args.is_empty());
            if let (Some(a), Some(b)) = (aa.next(), ba.next()) {
                equal &= aa.next().is_none() && ba.next().is_none();
                equal &= a.args.len() == 1
                    && b.args.len() == 1
                    && a.region_args.is_empty()
                    && b.region_args.is_empty()
                    && a.associated_types.is_empty()
                    && b.associated_types.is_empty();
                if let ([a], [b]) = (a.args.as_slice(), b.args.as_slice()) {
                    equal &= same_type(names, a, b, budget)?;
                }
            } else {
                equal = false;
            }
            equal
        }
        _ => false,
    })
}

impl ParsedSource {
    fn replay(
        &self,
        program: &Program,
        function: &Function,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        budget.charge_work(8)?;
        program
            .names
            .session()
            .sources()
            .render_span(self.source)
            .map_err(|error| error.to_string())?;
        // Resolution can qualify the input application, but cannot change the
        // complete body or any formal/header occurrence authenticated here.
        let Some((binding, reference, field)) = shape(function, false) else {
            return Err("direct return source changed complete original body/header".into());
        };
        let names = program.names.as_ref();
        if function.module != self.module
            || function.lexical_scope != self.scope
            || !exact_source_ident(
                names,
                *function
                    .name
                    .segments()
                    .last()
                    .ok_or("direct return function has no original name")?,
                self.name,
            )?
            || !exact_source_ident(names, function.type_params[0], self.generic)?
            || !exact_source_ident(names, binding, self.binding)?
            || !exact_source_ident(names, reference, self.reference)?
            || !exact_source_ident(names, field, self.field)?
            || !same_type(names, &self.input, &function.params[0].1, budget)?
            || !same_type(names, &self.output, &function.result, budget)?
        {
            return Err("direct return source changed original declaration/binding/field".into());
        }
        if names
            .key(binding, LookupClass::Local)
            .map_err(native_adapter::name_diagnostic)?
            != names
                .key(reference, LookupClass::Local)
                .map_err(native_adapter::name_diagnostic)?
        {
            return Err("direct return source changed original parameter use".into());
        }
        if let Some(scope) = self.scope {
            let scopes = program
                .scopes
                .as_ref()
                .ok_or("direct return source has no original lexical scope")?;
            let binding = scopes
                .declaration_for(names, binding)
                .map_err(|error| error.to_string())?;
            if scopes
                .reference_for(names, reference)
                .map_err(|error| error.to_string())?
                != Some(binding)
                || scopes
                    .binding_function(binding)
                    .map_err(|error| error.to_string())?
                    != scope
            {
                return Err("direct return source changed original parameter binding".into());
            }
        }
        Ok(())
    }
}

/// A frozen original-source precision proof, not a replacement header contract.
#[derive(Debug)]
pub struct DirectReturnPlace {
    declaration: ir::DeclId,
    environment: Arc<ir::GenericEnv>,
    nominal: ir::StructTy,
    parameter: ir::ParamId,
    contract: Arc<ir::RefContract>,
    path: ir::InputPath,
    _source: Arc<ParsedSource>,
    _storage: ir::RetainedStorage,
}

pub(super) fn seal(
    registry: &DeclRegistry,
    declaration: ir::DeclId,
    schema: &tr::GenericSchema,
    budget: &ir::PlanBudget,
) -> Result<Option<Arc<DirectReturnPlace>>, String> {
    let Some(function) = registry.source_callable(declaration) else {
        return Ok(None);
    };
    let Some(source) = &function.direct_return_source else {
        return Ok(None);
    };
    source.replay(&registry.program, function, budget)?;
    let info = registry.declaration(declaration)?;
    if info.kind != ir::DeclKind::Function
        || !Arc::ptr_eq(&info.env, &schema.env)
        || schema.env.total_arity() != 1
    {
        return Err("direct return source changed canonical declaration environment".into());
    }
    let (
        [
            ir::Ty::Ref {
                kind: ir::BorrowKind::Mut,
                referent: input,
            },
        ],
        ir::Ty::Ref {
            kind: ir::BorrowKind::Mut,
            referent: output,
        },
    ) = (schema.signature.params.as_slice(), &schema.signature.result)
    else {
        return Err("direct return source changed typed reference header".into());
    };
    let (ir::Ty::Struct(nominal), ir::Ty::Param(parameter)) = (input.as_ref(), output.as_ref())
    else {
        return Ok(None);
    };
    if nominal.def().type_args.as_ref() != [ir::Ty::Param(*parameter)]
        || !schema.env.contains(*parameter)
    {
        return Ok(None);
    }
    let owner = registry.declaration(nominal.def().source_decl)?;
    if owner.kind != ir::DeclKind::Struct {
        return Ok(None);
    }
    let original = registry
        .program
        .structs
        .get(owner.source_index)
        .ok_or("direct return source nominal missing")?;
    let names = registry.program.names.as_ref();
    let key = names
        .key(source.field, LookupClass::Member)
        .map_err(native_adapter::name_diagnostic)?;
    let mut selected = None;
    for (index, field) in original.fields.iter().enumerate() {
        budget.charge_work(1)?;
        if let FieldName::Named(name) = field.name
            && names
                .key(name, LookupClass::Member)
                .map_err(native_adapter::name_diagnostic)?
                == key
            && selected.replace(index).is_some()
        {
            return Err("duplicate original return field".into());
        }
    }
    let field = selected.ok_or("direct return source field is not in original nominal")?;
    let fields = nominal.def().fields.read(budget)?;
    if fields
        .get(field)
        .is_none_or(|field| field.ty != ir::Ty::Param(*parameter))
    {
        return Err("direct return source field does not have declared result type".into());
    }
    let storage = budget
        .reserve_bytes(
            std::mem::size_of::<DirectReturnPlace>()
                + 2 * std::mem::size_of::<usize>()
                + std::mem::size_of::<ir::Projection>(),
        )?
        .retain()?;
    Ok(Some(Arc::new(DirectReturnPlace {
        declaration,
        environment: schema.env.clone(),
        nominal: nominal.clone(),
        parameter: *parameter,
        contract: schema.signature.ref_contract.clone(),
        path: ir::InputPath {
            input: ir::InputReference {
                param: 0,
                position: ir::ReferencePosition { depth: 0 },
            },
            projection: Arc::from([ir::Projection::Field(field)]),
        },
        _source: source.clone(),
        _storage: storage,
    })))
}

impl DirectReturnPlace {
    pub(in crate::compiler) fn validate(
        &self,
        declaration: ir::DeclId,
        schema: &tr::GenericSchema,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        budget.charge_work(4)?;
        if self.declaration != declaration
            || !Arc::ptr_eq(&self.environment, &schema.env)
            || self.contract != schema.signature.ref_contract
            || schema.signature.params.len() != 1
            || !matches!(&schema.signature.result, ir::Ty::Ref { kind: ir::BorrowKind::Mut, referent } if referent.as_ref() == &ir::Ty::Param(self.parameter))
        {
            return Err("direct return proof changed original schema authority".into());
        }
        let ir::Ty::Ref {
            kind: ir::BorrowKind::Mut,
            referent,
        } = &schema.signature.params[0]
        else {
            return Err("direct return proof changed original input".into());
        };
        let ir::Ty::Struct(nominal) = referent.as_ref() else {
            return Err("direct return proof lost original nominal".into());
        };
        if !std::ptr::eq(nominal.def(), self.nominal.def()) {
            return Err("direct return proof changed original nominal identity".into());
        }
        Ok(())
    }
    pub(in crate::compiler) fn path(&self) -> ir::InputPath {
        self.path.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const SOURCE: &str = "struct Slot<T>{value:T,spare:T,tag:i32} fn field<T>(slot:&mut Slot<T>)->&mut T{&mut slot.value} fn main(){}";

    #[test]
    fn direct_return_original_body_issues_typed_projection_without_changing_envelope() {
        let program = parse(SOURCE).unwrap();
        assert!(
            program.functions[0].direct_return_source.is_some(),
            "{:?}",
            program.functions[0]
        );
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let declaration = plan.function_decl(0).unwrap();
        let schema = plan.source_schemas.schema(declaration, &budget).unwrap();
        let proof = plan
            .source_schemas
            .direct_return(declaration, &budget)
            .unwrap()
            .expect("typed direct return proof");
        proof.validate(declaration, &schema, &budget).unwrap();
        assert_eq!(proof.path.projection.as_ref(), &[ir::Projection::Field(0)]);
        assert!(matches!(
            schema.signature.ref_contract.results[0].returns,
            ir::ReturnContract::MutInputEnvelopes(_)
        ));
    }

    #[test]
    fn direct_return_rejects_changed_body_field_binding_and_foreign_owner() {
        let program = parse("struct Slot<T>{value:T,spare:T} fn field<T>(slot:&mut Slot<T>)->&mut T{&mut slot.value} fn other<T>(slot:&mut Slot<T>)->&mut T{&mut slot.spare} fn effect(){let changed=1i32;} fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let mut changed = program.clone();
        changed.functions[0].body.statements = program.functions[2].body.statements.clone();
        assert!(!changed.functions[0].body.statements.is_empty());
        assert!(
            PlanCtx::from_program(&changed, &budget)
                .unwrap_err()
                .contains("direct return source changed complete original body/header")
        );

        let Expr::Borrow { place: other, .. } = program.functions[1].body.tail.as_deref().unwrap()
        else {
            panic!("original other projection")
        };
        let mut changed = program.clone();
        let Expr::Borrow { place, .. } = changed.functions[0].body.tail.as_deref_mut().unwrap()
        else {
            panic!("original value projection")
        };
        place.fields.clone_from(&other.fields);
        assert!(
            PlanCtx::from_program(&changed, &budget)
                .unwrap_err()
                .contains("direct return source changed original declaration/binding/field")
        );

        let mut changed = program.clone();
        changed.functions[0].params[0].0 = program.functions[1].params[0].0;
        assert!(
            PlanCtx::from_program(&changed, &budget)
                .unwrap_err()
                .contains("direct return source changed original declaration/binding/field")
        );

        let mut changed = program.clone();
        changed.functions[1].direct_return_source =
            program.functions[0].direct_return_source.clone();
        assert!(
            PlanCtx::from_program(&changed, &budget)
                .unwrap_err()
                .contains("direct return source changed original declaration/binding/field")
        );

        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let declaration = plan.function_decl(0).unwrap();
        let proof = plan
            .source_schemas
            .direct_return(declaration, &budget)
            .unwrap()
            .unwrap();
        let foreign = parse(SOURCE).unwrap();
        let foreign_plan = PlanCtx::from_program(&foreign, &budget).unwrap();
        let foreign_schema = foreign_plan
            .source_schemas
            .schema(foreign_plan.function_decl(0).unwrap(), &budget)
            .unwrap();
        assert!(
            proof
                .validate(declaration, &foreign_schema, &budget)
                .unwrap_err()
                .contains("original schema authority")
        );
        let mut changed = program.clone();
        changed.functions[0].direct_return_source =
            foreign.functions[0].direct_return_source.clone();
        assert!(PlanCtx::from_program(&changed, &budget).is_err());
    }

    #[test]
    fn direct_return_does_not_refine_effects_branches_calls_or_raw_access() {
        for body in [
            "{let changed=1i32; &mut slot.value}",
            "{if true {&mut slot.value} else {&mut slot.spare}}",
            "{other(slot)}",
            "{unsafe {&mut *(&mut slot.value as *mut T)}}",
        ] {
            let source = format!(
                "struct Slot<T>{{value:T,spare:T}} fn field<T>(slot:&mut Slot<T>)->&mut T{body} fn other<T>(slot:&mut Slot<T>)->&mut T{{&mut slot.spare}} fn main(){{}}"
            );
            let program = parse(&source).unwrap();
            assert!(
                program.functions[0].direct_return_source.is_none(),
                "{body}"
            );
        }
    }
}
