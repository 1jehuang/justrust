//! Sealed native sequence roles from loader-issued original source.
//! A path selects a role only after an embedded original-file issuer matches.
use super::*;

mod slot;
mod swap;
pub use slot::NativeSequenceSlot;
pub(super) use slot::issue_slot;
pub use swap::NativeSequenceSwap;
pub(super) use swap::issue_swap;

#[derive(Clone, Debug)]
pub(in crate::compiler) struct NativeSequenceBinding {
    declaration: ir::DeclId,
    parameter: ir::ParamId,
    original_template: Option<ir::StructTy>,
    source_substitution: Option<Arc<NativeSourceSubstitution>>,
    module: ModuleId,
    visibility: ir::Visibility,
    _source: crate::compiler::token_session::OriginalSpan,
    _issuer: crate::compiler::source::EmbeddedOriginalFile,
}

/// Canonical source-schema application, never an executable nominal read.
/// The retained allocation is the exact product of checked binder substitution.
#[derive(Clone, Debug)]
struct NativeSourceSubstitution {
    definition: ir::StructTy,
    _environment: Arc<[ir::ParamId]>,
}

impl NativeSequenceBinding {
    pub(in crate::compiler) fn matches_source_substitution(
        &self,
        definition: &ir::StructDef,
    ) -> bool {
        self.matches_declaration(definition)
            && self
                .source_substitution
                .as_ref()
                .is_some_and(|proof| std::ptr::eq(proof.definition.def(), definition))
    }
    pub(in crate::compiler) fn matches_declaration(&self, definition: &ir::StructDef) -> bool {
        definition.source_decl == self.declaration
            && definition.module == self.module
            && definition.visibility == self.visibility
    }
    pub(in crate::compiler) fn matches_original_template(
        &self,
        definition: &ir::StructDef,
    ) -> bool {
        self.matches_declaration(definition)
            && self
                .original_template
                .as_ref()
                .is_some_and(|template| std::ptr::eq(template.def(), definition))
            && definition.type_args.as_ref() == [ir::Ty::Param(self.parameter)]
    }
}

pub(super) fn issue(
    registry: &DeclRegistry,
    declaration: ir::DeclId,
    source: &StructDecl,
    value: &ir::StructTy,
    budget: &ir::PlanBudget,
) -> Result<Option<Arc<NativeSequenceBinding>>, String> {
    let definition = value.def();
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
    // These spellings choose a protocol in authenticated embedded source. They
    // are never compared as nominal identity and cannot authorize raw fixtures.
    let expected = [
        crate::compiler::source::source_prelude::PRELUDE_MODULE,
        "vec",
        "Vec",
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
    if source.type_params.len() != 1
        || !source.region_params.is_empty()
        || source.fields.len() != 3
        || source.shape != StructShape::Record
    {
        return Err("native Vec source role changed its declared shape".into());
    }
    let parameters = &registry.declaration(declaration)?.param_env;
    if parameters.arity() != 1 {
        return Err("native Vec source role changed binder arity".into());
    }
    let parameter = ir::ParamId {
        owner: parameters.binder(),
        index: 0,
    };
    let original_template =
        (definition.type_args.as_ref() == [ir::Ty::Param(parameter)]).then(|| value.clone());
    budget.retain_bytes(std::mem::size_of::<NativeSequenceBinding>())?;
    Ok(Some(Arc::new(NativeSequenceBinding {
        declaration,
        parameter,
        original_template,
        source_substitution: None,
        module: source.module,
        visibility: source.visibility.clone(),
        _source: original,
        _issuer: issuer.clone(),
    })))
}

/// Called only after the schema interner has completed its canonical field
/// substitution and ordinary argument validation, never on structural transport.
pub(super) fn bind_source_substitution(
    value: ir::Ty,
    caller: ir::FactsCtx<'_>,
    budget: &ir::PlanBudget,
) -> Result<ir::Ty, String> {
    let ir::Ty::Struct(owner) = value else {
        return Ok(value);
    };
    let Some(binding) = owner.native_sequence_binding() else {
        return Ok(ir::Ty::Struct(owner));
    };
    if !binding.matches_declaration(owner.def()) {
        return Err("native source substitution changed declaration authority".into());
    }
    let [element] = owner.def().type_args.as_ref() else {
        return Err("native source substitution changed actual arity".into());
    };
    // This is a second replay at the issuing boundary, not a caller-supplied
    // success bit. Foreign binders and forged graph reads remain rejected.
    validate_argument(element, caller, budget, 0)?;
    let fields = owner.def().fields.read(budget)?;
    let [pointer, length, capacity] = fields.as_ref() else {
        return Err("native source substitution changed field arity".into());
    };
    if pointer.name != "ptr"
        || !matches!(&pointer.ty, ir::Ty::RawPtr {kind: ir::BorrowKind::Mut, pointee} if pointee.as_ref() == element)
        || length.name != "len"
        || length.ty != ir::Ty::Int(ir::IntTy::Usize)
        || capacity.name != "cap"
        || capacity.ty != ir::Ty::Int(ir::IntTy::Usize)
    {
        return Err("native source substitution changed canonical fields".into());
    }
    let env = match caller {
        ir::FactsCtx::Concrete => None,
        ir::FactsCtx::Universal(env) => Some(env),
        ir::FactsCtx::Validated(facts) => Some(facts.env().as_ref()),
    };
    let mut parameters = Vec::new();
    if let Some(env) = env {
        for parameter in env.ordered_params() {
            sr::push(&mut parameters, parameter, budget)
                .map_err(|error| error.into_legacy_diagnostic(budget))?;
        }
    }
    // The vector buffer and its immutable transfer coexist. The shared helper
    // pays vector capacity, while this separately prepays all three Arc roots.
    let environment_bytes = parameters
        .len()
        .checked_mul(std::mem::size_of::<ir::ParamId>())
        .and_then(|bytes| bytes.checked_add(2 * std::mem::size_of::<usize>()))
        .ok_or("native source substitution environment storage overflow")?;
    let root_bytes = std::mem::size_of::<NativeSequenceBinding>()
        .checked_add(std::mem::size_of::<NativeSourceSubstitution>())
        .and_then(|bytes| bytes.checked_add(4 * std::mem::size_of::<usize>()))
        .and_then(|bytes| bytes.checked_add(environment_bytes))
        .ok_or("native source substitution root storage overflow")?;
    budget.charge_nodes(3)?;
    budget.retain_bytes(root_bytes)?;
    let mut bound = binding.as_ref().clone();
    bound.source_substitution = Some(Arc::new(NativeSourceSubstitution {
        definition: owner.clone(),
        _environment: parameters.into(),
    }));
    Ok(ir::Ty::Struct(
        owner.with_native_sequence_binding(Some(Arc::new(bound))),
    ))
}
