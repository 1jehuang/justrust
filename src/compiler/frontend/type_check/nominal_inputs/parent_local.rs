//! Parent-local realization of nominal applications whose ordered actuals hold
//! closure types (`W<{closure}>`, adapter structs).
//!
//! A closure type is final only inside the parent body that defines it, so a
//! graph realization keyed by layout-free shapes cannot name it, and reading the
//! parent's published closure facts from a separate node would wait on the
//! parent itself. Instead the parent realizes such an application from its own
//! finalized `ClosureTy` actuals:
//!
//! * identity is `(parent declaration, application ordinal)`; the ordinal is
//!   assigned in deterministic first-use order inside this checking node, and
//!   each ordinal records the exact ordered actuals it was minted for. The
//!   closure actuals themselves carry `(owner, site, provider, owner_args)`, so
//!   distinct closures (or one closure under different parent arguments) never
//!   share an ordinal;
//! * fields are the canonical source schema with the parent's actuals
//!   substituted structurally. No interner, lock or other node is consulted.
//!
//! The ordinal table is local `RefCell` scratch of one checking computation,
//! never part of a published fact or reachable from another node.
use super::*;

mod replay;
pub(in crate::compiler::frontend) use replay::ParentLocalReplayMemo;

/// Whether any ordered actual structurally holds a closure type.
pub(in crate::compiler::frontend) fn contains_closure(
    types: &[ir::Ty],
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<bool, String> {
    for ty in types {
        if closure_in(ty, budget, depth)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn closure_in(ty: &ir::Ty, budget: &ir::PlanBudget, depth: usize) -> Result<bool, String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    Ok(match ty {
        ir::Ty::Closure(_) => true,
        ir::Ty::Struct(n) if !n.is_back_edge() => {
            contains_closure(&n.def().type_args, budget, depth + 1)?
        }
        ir::Ty::Enum(n) if !n.is_back_edge() => {
            contains_closure(&n.def().type_args, budget, depth + 1)?
        }
        ir::Ty::Tuple(types) => contains_closure(types, budget, depth + 1)?,
        ir::Ty::Cell(cell) => closure_in(cell.argument(), budget, depth + 1)?,
        ir::Ty::ManuallyDrop(wrapper) => closure_in(wrapper.argument(), budget, depth + 1)?,
        ir::Ty::PhantomData(marker) => closure_in(marker.argument(), budget, depth + 1)?,
        ir::Ty::FnPointer(pointer) => {
            contains_closure(&pointer.inputs, budget, depth + 1)?
                || closure_in(&pointer.output, budget, depth + 1)?
        }
        ir::Ty::Ref { referent: t, .. }
        | ir::Ty::RawPtr { pointee: t, .. }
        | ir::Ty::Array { element: t, .. }
        | ir::Ty::Slice { element: t } => closure_in(t, budget, depth + 1)?,
        _ => false,
    })
}

/// One minted application: exact declaration, ordered actuals and the shared
/// definition handed to every later use of the same application.
pub(in crate::compiler::frontend) struct ParentLocalApplication {
    context: tr::ParentLocalContext,
    declaration: ir::DeclId,
    actuals: Arc<[ir::Ty]>,
    definition: ir::Ty,
}

#[derive(Clone, Debug)]
pub(in crate::compiler) struct ParentLocalReceipt {
    context: tr::ParentLocalContext,
    template: ir::Ty,
    env: Arc<ir::ParamEnv>,
    id: ir::NominalAppId,
    actuals: Arc<[ir::Ty]>,
    fields: Arc<[tr::ParentLocalField]>,
}

impl ParentLocalReceipt {
    pub(in crate::compiler) fn declaration(&self) -> ir::DeclId {
        self.env.binder().declaration
    }

    pub(in crate::compiler) fn matches_owning_context(
        &self,
        facts: &tr::ValidatedFacts,
    ) -> Result<bool, String> {
        self.context
            .matches(facts)
            .map_err(|error| error.into_legacy_diagnostic(facts.budget()))
    }

    fn validate_context(
        &self,
        facts: &tr::ValidatedFacts,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        if !facts.budget().same_authority(budget) {
            return Err("parent-local receipt changed planning authority".into());
        }
        self.context
            .validate_for(facts)
            .map_err(|error| error.into_legacy_diagnostic(budget))
    }

    pub(in crate::compiler) fn replay(
        &self,
        definition: &ir::Ty,
        original: &ir::Ty,
        facts: &tr::ValidatedFacts,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        self.replay_with_scratch(
            definition,
            original,
            facts,
            budget,
            &mut tr::TraitScratch::default(),
        )
    }

    fn replay_with_scratch(
        &self,
        definition: &ir::Ty,
        original: &ir::Ty,
        facts: &tr::ValidatedFacts,
        budget: &ir::PlanBudget,
        scratch: &mut tr::TraitScratch,
    ) -> Result<(), String> {
        self.validate_context(facts, budget)?;
        let source_matches = match (&self.template, original) {
            (ir::Ty::Struct(a), ir::Ty::Struct(b)) => same_struct_def(a.def(), b.def(), budget)?,
            (ir::Ty::Enum(a), ir::Ty::Enum(b)) => same_enum_def(a.def(), b.def(), budget)?,
            _ => false,
        };
        if !source_matches {
            return Err("parent-local receipt changed original source template".into());
        }
        for actual in self.actuals.iter() {
            facts
                .validate_ty_metadata_scratch(actual, budget, scratch)
                .map_err(|error| error.into_legacy_diagnostic(budget))?;
        }
        for field in self.fields.iter() {
            field
                .replay(&self.template, &self.env, &self.actuals, facts, scratch)
                .map_err(|error| error.into_legacy_diagnostic(budget))?;
        }
        let expected = self.definition(budget)?;
        let matches = match (definition, &expected) {
            (ir::Ty::Struct(a), ir::Ty::Struct(b)) => same_struct_def(a.def(), b.def(), budget)?,
            (ir::Ty::Enum(a), ir::Ty::Enum(b)) => same_enum_def(a.def(), b.def(), budget)?,
            _ => false,
        };
        if !matches {
            return Err(
                "parent-local receipt changed ordinal, actuals or normalized fields".into(),
            );
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::compiler) fn remint_for_source_substitution(
        &self,
        definition: &ir::Ty,
        source_facts: &tr::ValidatedFacts,
        current_facts: &tr::ValidatedFacts,
        env: &ir::GenericEnv,
        parent_arguments: &[ir::Ty],
        mapped_actuals: &[ir::Ty],
        budget: &ir::PlanBudget,
    ) -> Result<ir::Ty, String> {
        let ir::NominalIndex::ParentLocal { owner, .. } = self.id.index else {
            return Err("source closing substitution requires a parent-local application".into());
        };
        if self.actuals.len() != mapped_actuals.len()
            || !self.actuals.iter().all(ir::Ty::is_closed_shallow)
            || !mapped_actuals.iter().all(ir::Ty::is_closed_shallow)
            || !same_actuals(&self.actuals, mapped_actuals, budget)?
        {
            return Err(
                "unsupported: closing parent-local substitution changed canonical ordered actuals"
                    .into(),
            );
        }
        let mut scratch = tr::TraitScratch::default();
        for actual in mapped_actuals {
            current_facts
                .validate_ty_metadata_scratch(actual, budget, &mut scratch)
                .map_err(|error| error.into_legacy_diagnostic(budget))?;
        }
        if self
            .context
            .matches(current_facts)
            .map_err(|error| error.into_legacy_diagnostic(budget))?
        {
            self.replay(definition, &self.template, current_facts, budget)?;
            return Ok(definition.clone());
        }
        current_facts
            .validate_parent_local_closing_source(source_facts, owner, env, parent_arguments)
            .map_err(|error| error.into_legacy_diagnostic(budget))?;
        self.replay(definition, &self.template, source_facts, budget)?;
        budget.retain_bytes(std::mem::size_of_val(mapped_actuals))?;
        let actuals: Arc<[ir::Ty]> = mapped_actuals.into();
        let mut fields = Vec::new();
        let mut collect = |variant, source: &ir::NominalFields| -> Result<(), String> {
            let source = source.read(budget)?;
            for field in 0..source.len() {
                budget.charge_work(1)?;
                budget.retain_bytes(std::mem::size_of::<tr::ParentLocalField>())?;
                fields
                    .try_reserve_exact(1)
                    .map_err(|_| "closed parent-local field evidence allocation failed")?;
                fields.push(
                    current_facts
                        .parent_local_field(&self.template, &self.env, &actuals, variant, field)
                        .map_err(|error| error.into_legacy_diagnostic(budget))?,
                );
            }
            Ok(())
        };
        match &self.template {
            ir::Ty::Struct(value) => collect(None, &value.def().fields)?,
            ir::Ty::Enum(value) => {
                for (variant, source) in value.def().variants.iter().enumerate() {
                    collect(Some(variant), &source.fields)?;
                }
            }
            _ => return Err("closed parent-local substitution lost nominal template".into()),
        }
        budget.retain_bytes(
            std::mem::size_of::<ParentLocalReceipt>() + 2 * std::mem::size_of::<usize>(),
        )?;
        let receipt = Arc::new(Self {
            context: current_facts
                .parent_local_context()
                .map_err(|error| error.into_legacy_diagnostic(budget))?,
            template: self.template.clone(),
            env: self.env.clone(),
            id: self.id,
            actuals,
            fields: fields.into(),
        });
        let closed = match receipt.definition(budget)? {
            ir::Ty::Struct(value) => {
                ir::Ty::Struct(value.with_parent_local_receipt(receipt.clone()))
            }
            ir::Ty::Enum(value) => ir::Ty::Enum(value.with_parent_local_receipt(receipt.clone())),
            _ => unreachable!("canonical nominal receipt template"),
        };
        receipt.replay(&closed, &self.template, current_facts, budget)?;
        Ok(closed)
    }

    fn definition(&self, budget: &ir::PlanBudget) -> Result<ir::Ty, String> {
        let mut cursor = 0usize;
        let mut fields = |source: &ir::NominalFields| -> Result<ir::NominalFields, String> {
            let source = source.read(budget)?;
            budget.charge_work(source.len())?;
            budget.retain_bytes(std::mem::size_of_val(source.as_ref()))?;
            let mut result = Vec::new();
            result
                .try_reserve_exact(source.len())
                .map_err(|_| "parent-local field allocation failed")?;
            for original in source.iter() {
                budget.retain_bytes(original.name.len())?;
                let mut field = original.clone();
                field.ty = self
                    .fields
                    .get(cursor)
                    .ok_or("parent-local receipt omitted field")?
                    .ty()
                    .clone();
                cursor += 1;
                result.push(field);
            }
            Ok(Arc::<[ir::FieldDef]>::from(result).into())
        };
        let result = match &self.template {
            ir::Ty::Struct(value) => {
                let mut definition = value.def().clone();
                definition.id = self.id;
                definition.type_args = self.actuals.clone();
                definition.fields = fields(&definition.fields)?;
                ir::Ty::Struct(ir::StructTy::new_parent_local(definition))
            }
            ir::Ty::Enum(value) => {
                let mut definition = value.def().clone();
                definition.id = self.id;
                definition.type_args = self.actuals.clone();
                let mut variants = Vec::new();
                budget.retain_bytes(std::mem::size_of_val(definition.variants.as_ref()))?;
                variants
                    .try_reserve_exact(definition.variants.len())
                    .map_err(|_| "parent-local variant allocation failed")?;
                for original in definition.variants.iter() {
                    let mut variant = original.clone();
                    budget.retain_bytes(original.name.len())?;
                    variant.fields = fields(&variant.fields)?;
                    variants.push(variant);
                }
                definition.variants = variants.into();
                ir::Ty::Enum(ir::EnumTy::new_parent_local(definition))
            }
            _ => return Err("parent-local receipt lost original nominal kind".into()),
        };
        if cursor != self.fields.len() {
            return Err("parent-local receipt added fields".into());
        }
        Ok(result)
    }
}

impl Checker<'_> {
    pub(in crate::compiler::frontend) fn parent_local_invocation_inputs(
        &self,
        callee: &ir::Callee,
    ) -> Result<Vec<ir::Ty>, String> {
        if self.nominal_inputs.is_none() {
            return Ok(Vec::new());
        }
        let facts = self
            .facts()
            .validated()
            .ok_or("parent-local invocation requires owning facts")?;
        let source = facts
            .parent_local_source_signature(callee)
            .map_err(|error| error.into_legacy_diagnostic(facts.budget()))?;
        let mut definitions = facts
            .parent_local_signature_inputs(&source)
            .map_err(|error| error.into_legacy_diagnostic(facts.budget()))?;
        for ty in source.params.iter().chain(std::iter::once(&source.result)) {
            self.prepare_parent_local_source(ty, &mut definitions, 0)?;
        }
        Ok(definitions)
    }

    fn prepare_parent_local_source(
        &self,
        ty: &ir::Ty,
        definitions: &mut Vec<ir::Ty>,
        depth: usize,
    ) -> Result<(), String> {
        self.infer_work(depth)?;
        let nominal = match ty {
            ir::Ty::Struct(value) => Some((
                value.def().source_decl,
                &value.def().type_args,
                value.parent_local_receipt().is_some(),
            )),
            ir::Ty::Enum(value) => Some((
                value.def().source_decl,
                &value.def().type_args,
                value.parent_local_receipt().is_some(),
            )),
            _ => None,
        };
        if let Some((declaration, arguments, sealed)) = nominal {
            for argument in arguments.iter() {
                self.prepare_parent_local_source(argument, definitions, depth + 1)?;
            }
            if !sealed && contains_closure(arguments, &self.signatures.plan()?.budget, depth + 1)? {
                let definition = self.mint_parent_local_nominal(declaration, arguments)?;
                self.infer_storage(1)?;
                definitions.push(definition);
            }
            return Ok(());
        }
        match ty {
            ir::Ty::Tuple(parts) => {
                for part in parts.iter() {
                    self.prepare_parent_local_source(part, definitions, depth + 1)?;
                }
            }
            ir::Ty::Ref {
                referent: child, ..
            }
            | ir::Ty::RawPtr { pointee: child, .. }
            | ir::Ty::Array { element: child, .. }
            | ir::Ty::Slice { element: child } => {
                self.prepare_parent_local_source(child, definitions, depth + 1)?
            }
            ir::Ty::Cell(wrapper) => {
                self.prepare_parent_local_source(wrapper.argument(), definitions, depth + 1)?
            }
            ir::Ty::ManuallyDrop(wrapper) => {
                self.prepare_parent_local_source(wrapper.argument(), definitions, depth + 1)?
            }
            ir::Ty::PhantomData(wrapper) => {
                self.prepare_parent_local_source(wrapper.argument(), definitions, depth + 1)?
            }
            ir::Ty::FnPointer(pointer) => {
                for child in pointer
                    .inputs
                    .iter()
                    .chain(std::iter::once(&pointer.output))
                {
                    self.prepare_parent_local_source(child, definitions, depth + 1)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(in crate::compiler::frontend) fn mint_parent_local_nominal(
        &self,
        declaration: ir::DeclId,
        actuals: &[ir::Ty],
    ) -> Result<ir::Ty, String> {
        let plan = self.signatures.plan()?;
        let budget = &plan.budget;
        let owner = self
            .declaration
            .ok_or("unsupported: closure-bearing nominal outside a parent callable")?;
        let facts = self
            .facts()
            .validated()
            .ok_or("parent-local source mint requires validated owning facts")?;
        let (env, template) = plan.nominal_schema(declaration, budget)?;
        for actual in actuals {
            facts
                .validate_ty_metadata_with_budget(actual, budget)
                .map_err(|error| error.into_legacy_diagnostic(budget))?;
        }
        {
            let table = self.parent_local_nominals.borrow();
            for entry in table.iter() {
                self.infer_work(0)?;
                if entry
                    .context
                    .matches(facts)
                    .map_err(|error| error.into_legacy_diagnostic(budget))?
                    && entry.declaration == declaration
                    && entry.actuals.len() == actuals.len()
                    && same_actuals(&entry.actuals, actuals, budget)?
                {
                    let receipt = match &entry.definition {
                        ir::Ty::Struct(value) => value.parent_local_receipt(),
                        ir::Ty::Enum(value) => value.parent_local_receipt(),
                        _ => None,
                    }
                    .ok_or("parent-local cache lost source receipt")?;
                    receipt.replay(&entry.definition, &template, facts, budget)?;
                    return Ok(entry.definition.clone());
                }
            }
        }
        if env.arity() != actuals.len() {
            return Err("invalid metadata: parent-local nominal actual arity".into());
        }
        let ordinal = u32::try_from(self.parent_local_nominals.borrow().len())
            .map_err(|_| "parent-local nominal ordinal overflow")?;
        let id = ir::NominalAppId {
            snapshot: match &template {
                ir::Ty::Struct(n) => n.id().snapshot,
                ir::Ty::Enum(n) => n.id().snapshot,
                _ => return Err("invalid metadata: nominal schema is not nominal".into()),
            },
            index: ir::NominalIndex::ParentLocal { owner, ordinal },
        };
        let actuals: Arc<[ir::Ty]> = actuals.into();
        let mut fields = Vec::new();
        let mut collect = |variant, original: &ir::NominalFields| -> Result<(), String> {
            let original = original.read(budget)?;
            for field in 0..original.len() {
                budget.charge_work(1)?;
                budget.retain_bytes(std::mem::size_of::<tr::ParentLocalField>())?;
                fields
                    .try_reserve_exact(1)
                    .map_err(|_| "parent-local evidence allocation failed")?;
                fields.push(
                    facts
                        .parent_local_field(&template, &env, &actuals, variant, field)
                        .map_err(|error| error.into_legacy_diagnostic(budget))?,
                );
            }
            Ok(())
        };
        match &template {
            ir::Ty::Struct(value) => collect(None, &value.def().fields)?,
            ir::Ty::Enum(value) => {
                for (index, variant) in value.def().variants.iter().enumerate() {
                    collect(Some(index), &variant.fields)?;
                }
            }
            _ => return Err("invalid metadata: nominal schema is not nominal".into()),
        }
        budget.retain_bytes(
            std::mem::size_of::<ParentLocalReceipt>() + 2 * std::mem::size_of::<usize>(),
        )?;
        let receipt = Arc::new(ParentLocalReceipt {
            context: facts
                .parent_local_context()
                .map_err(|error| error.into_legacy_diagnostic(budget))?,
            template,
            env,
            id,
            actuals: actuals.clone(),
            fields: fields.into(),
        });
        let definition = match receipt.definition(budget)? {
            ir::Ty::Struct(value) => ir::Ty::Struct(value.with_parent_local_receipt(receipt)),
            ir::Ty::Enum(value) => ir::Ty::Enum(value.with_parent_local_receipt(receipt)),
            _ => unreachable!("sealed original nominal"),
        };
        self.infer_storage(1)?;
        self.parent_local_nominals
            .borrow_mut()
            .push(ParentLocalApplication {
                context: facts
                    .parent_local_context()
                    .map_err(|error| error.into_legacy_diagnostic(budget))?,
                declaration,
                actuals,
                definition: definition.clone(),
            });
        Ok(definition)
    }
}

fn same_actuals(a: &[ir::Ty], b: &[ir::Ty], budget: &ir::PlanBudget) -> Result<bool, String> {
    for (a, b) in a.iter().zip(b) {
        budget.charge_work(1)?;
        // Structural Ty equality: closures compare by owner, site, provider,
        // owner arguments and every type; nominals by application identity.
        if a != b {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Structural substitution of the declaration's own parameters. Nested generic
/// nominals in field types need their own realization and are rejected
/// explicitly rather than approximated.
#[cfg(test)]
fn substitute(
    ty: &ir::Ty,
    env: &ir::ParamEnv,
    actuals: &[ir::Ty],
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<ir::Ty, String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    let child = |t: &ir::Ty| substitute(t, env, actuals, budget, depth + 1).map(Arc::new);
    Ok(match ty {
        ir::Ty::Param(p) => {
            if !env.contains(*p) {
                return Err("invalid metadata: foreign parameter in nominal field".into());
            }
            actuals
                .get(usize::from(p.index))
                .cloned()
                .ok_or("invalid metadata: parameter outside nominal actuals")?
        }
        ir::Ty::Ref { kind, referent } => ir::Ty::Ref {
            kind: *kind,
            referent: child(referent)?,
        },
        ir::Ty::RawPtr { kind, pointee } => ir::Ty::RawPtr {
            kind: *kind,
            pointee: child(pointee)?,
        },
        ir::Ty::DependentArray { element, length } => ir::Ty::DependentArray {
            element: child(element)?,
            length: *length,
        },
        ir::Ty::Array { element, len } => ir::Ty::Array {
            element: child(element)?,
            len: *len,
        },
        ir::Ty::Slice { element } => ir::Ty::Slice {
            element: child(element)?,
        },
        ir::Ty::Tuple(types) => ir::Ty::Tuple(
            types
                .iter()
                .map(|t| substitute(t, env, actuals, budget, depth + 1))
                .collect::<Result<Vec<_>, _>>()?
                .into(),
        ),
        ir::Ty::Cell(cell) => ir::Ty::Cell(cell.try_map(
            budget,
            |error| error,
            |ty| substitute(ty, env, actuals, budget, depth + 1),
        )?),
        ir::Ty::ManuallyDrop(wrapper) => ir::Ty::ManuallyDrop(wrapper.try_map(
            budget,
            |error| error,
            |ty| substitute(ty, env, actuals, budget, depth + 1),
        )?),
        ir::Ty::PhantomData(marker) => ir::Ty::PhantomData(marker.try_map(
            budget,
            |error| error,
            |ty| substitute(ty, env, actuals, budget, depth + 1),
        )?),
        ir::Ty::FnPointer(pointer) => ir::Ty::FnPointer(pointer.try_map(
            budget,
            |error| error,
            |ty| substitute(ty, env, actuals, budget, depth + 1),
        )?),
        ir::Ty::Struct(_)
        | ir::Ty::Enum(_)
        | ir::Ty::AssociatedType(_)
        | ir::Ty::Closure(_)
        | ir::Ty::Opaque(_) => {
            if mentions_param(ty, budget, depth)? {
                return Err(
                    "unsupported: closure-bearing nominal field with a nested generic application"
                        .into(),
                );
            }
            ty.clone()
        }
        other => other.clone(),
    })
}

#[cfg(test)]
fn mentions_param(ty: &ir::Ty, budget: &ir::PlanBudget, depth: usize) -> Result<bool, String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    let any = |types: &[ir::Ty]| -> Result<bool, String> {
        for t in types {
            if mentions_param(t, budget, depth + 1)? {
                return Ok(true);
            }
        }
        Ok(false)
    };
    Ok(match ty {
        ir::Ty::Param(_) | ir::Ty::AssociatedType(_) | ir::Ty::Opaque(_) => true,
        ir::Ty::Closure(c) => any(&c.types().cloned().collect::<Vec<_>>())?,
        ir::Ty::Struct(n) if !n.is_back_edge() => any(&n.def().type_args)?,
        ir::Ty::Enum(n) if !n.is_back_edge() => any(&n.def().type_args)?,
        ir::Ty::Tuple(types) => any(types)?,
        ir::Ty::Cell(cell) => mentions_param(cell.argument(), budget, depth + 1)?,
        ir::Ty::ManuallyDrop(wrapper) => mentions_param(wrapper.argument(), budget, depth + 1)?,
        ir::Ty::PhantomData(marker) => mentions_param(marker.argument(), budget, depth + 1)?,
        ir::Ty::FnPointer(pointer) => {
            any(&pointer.inputs)? || mentions_param(&pointer.output, budget, depth + 1)?
        }
        ir::Ty::Ref { referent: t, .. }
        | ir::Ty::RawPtr { pointee: t, .. }
        | ir::Ty::Array { element: t, .. }
        | ir::Ty::Slice { element: t } => mentions_param(t, budget, depth + 1)?,
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projected_parent_local_receipt_dispatch_replays_explicit_inputs() {
        let text = "trait Has{type Out;}impl Has for i32{type Out=u8;}struct Storage<T:Has>{value:T::Out}trait Marker{fn mark(&self);}impl Marker for i32{fn mark(&self){}}trait StorageMarker{}impl<T:Has> StorageMarker for Storage<T>{}trait ReceiverMarker{fn receive(&self);}impl<T> ReceiverMarker for T{fn receive(&self){}}trait GenericReceiver{fn echo<U>(&self,value:U)->U;}impl<T> GenericReceiver for T{fn echo<U>(&self,value:U)->U{value}}trait ProjectedReceiver{fn project(&self)->Self::Out;type Out;}impl<T> ProjectedReceiver for T{fn project(&self)->u8{1}type Out=u8;}fn main(){}fn other(){}fn bounded<T:Has>(value:Storage<T>)where Storage<T>:StorageMarker{}fn shared<T:Has>(value:Storage<T>)where Storage<T>:Sync{}trait StorageView<A>{type Out:Carries<A,Item=A>;}impl<T:Has> StorageView<Storage<T>> for Storage<T>{type Out=u8;}trait Carries<A>{type Item;}impl<A> Carries<A> for u8{type Item=A;}fn source_generic<U>(){}";
        let program = parse(text).unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        for wrong_rhs in ["u16", "bool"] {
            let invalid =
                parse(&text.replace("type Item=A;", &format!("type Item={wrong_rhs};"))).unwrap();
            assert!(
                PlanCtx::from_program(&invalid, &ir::PlanBudget::default()).is_err(),
                "unused implementation must reject a mismatched associated assignment"
            );
        }
        let owner = plan.function_decl(0).unwrap();
        let target = ir::CallTarget {
            declaration: owner,
            type_args: Arc::from([]),
        };
        let mut signatures = plan.environment(Some(owner)).unwrap();
        signatures.facts = Some(plan.instance_facts(&target).unwrap());
        let function = plan.callable_body(&plan.program, owner).unwrap();
        let mut checker = Checker::new(&signatures, ir::Ty::Unit, function.module);
        checker.declaration = Some(owner);
        let definition = checker
            .mint_parent_local_nominal(
                plan.nominal_declaration(0).unwrap(),
                &[ir::Ty::Int(ir::IntTy::I32)],
            )
            .unwrap();
        let facts = checker.facts().validated().unwrap();
        let call = ir::TraitCall {
            trait_ref: ir::TraitRef {
                declaration: plan.registry.trait_ids[1],
                self_ty: ir::Ty::Int(ir::IntTy::I32),
                args: Arc::from([]),
            },
            item: plan.registry.trait_items[1][0],
            method_args: Arc::from([]),
        };
        let mut scratch = tr::TraitScratch::default();
        let rows = [definition.clone()];
        let bounded = ir::CallTarget {
            declaration: plan.function_decl(2).unwrap(),
            type_args: Arc::from([ir::Ty::Int(ir::IntTy::I32)]),
        };
        let entry = facts.world().instance_facts(&bounded).unwrap();
        entry.validate_instance_entry(&bounded, &budget).unwrap();
        let shared = ir::CallTarget {
            declaration: plan.function_decl(3).unwrap(),
            type_args: bounded.type_args.clone(),
        };
        let shared_entry = facts.world().instance_facts(&shared).unwrap();
        shared_entry
            .validate_instance_entry(&shared, &budget)
            .unwrap();
        assert!(
            facts
                .world()
                .instance_facts(&ir::CallTarget {
                    declaration: shared.declaration,
                    type_args: Arc::from([ir::Ty::Bool]),
                })
                .is_err()
        );
        {
            use crate::compiler::nominal_realization::substitution::{
                substitute_source_schema_ty_with_scratch, substitute_source_ty_with_scratch,
            };
            let mut source_plan = PlanCtx::from_program(&program, &budget).unwrap();
            let declaration = source_plan.function_decl(2).unwrap();
            let schema = source_plan.function_schema(declaration).unwrap();
            let source = source_plan.callable_facts(declaration).unwrap();
            let closed_target = ir::CallTarget {
                declaration,
                type_args: bounded.type_args.clone(),
            };
            let closed = source_plan.instance_facts(&closed_target).unwrap();
            let ty = &schema.signature.params[0];
            let mut original_scratch = tr::TraitScratch::default();
            let mut current_scratch = tr::TraitScratch::default();
            let mapped = substitute_source_schema_ty_with_scratch(
                ty,
                &source,
                source.env(),
                &closed_target.type_args,
                &closed,
                &mut source_plan,
                &budget,
                None,
                &mut original_scratch,
                &mut current_scratch,
            )
            .unwrap();
            closed.validate_ty_metadata(&mapped).unwrap();
            let source_call = ir::Callee::Direct(ir::CallTarget {
                declaration: source_plan.function_decl(4).unwrap(),
                type_args: Arc::from([ty.clone()]),
            });
            for _ in 0..2 {
                let closed_call = ir::substitute_source_callee_metadata(
                    &source_call,
                    source.env(),
                    &closed_target.type_args,
                    &source,
                    &closed,
                    &mut source_plan,
                    &budget,
                    &mut original_scratch,
                    &mut current_scratch,
                )
                .unwrap();
                let ir::Callee::Direct(target) = closed_call else {
                    panic!("callee kind changed")
                };
                assert_eq!(target.type_args.as_ref(), &[mapped.clone()]);
                let classified = closed.world().instance_facts(&target).unwrap();
                assert!(classified.is_source_callable(target.declaration).unwrap());
                assert!(
                    classified
                        .original_callable_schema(tr::OriginalCallableSubject::Direct(
                            target.declaration
                        ),)
                        .is_err()
                );
                let generic_call = ir::TraitCall {
                    trait_ref: ir::TraitRef {
                        declaration: source_plan.registry.trait_ids[4],
                        self_ty: ir::Ty::Int(ir::IntTy::I32),
                        args: Arc::from([]),
                    },
                    item: source_plan.registry.trait_items[4][0],
                    method_args: Arc::from([mapped.clone()]),
                };
                let selected = closed
                    .source_callee_entry(
                        &ir::Callee::TraitItem(generic_call.clone()),
                        &[],
                        &mut current_scratch,
                    )
                    .unwrap();
                let ir::Callee::Direct(method_target) = selected else {
                    panic!("generic source method did not select its canonical provider")
                };
                assert_eq!(
                    method_target.type_args.as_ref(),
                    &[ir::Ty::Int(ir::IntTy::I32), mapped.clone()]
                );
                closed
                    .source_call_entry_with_nominals(&method_target, &[], &mut current_scratch)
                    .unwrap();
                let source_signature = closed
                    .source_direct_call_signature_with_scratch(&method_target, &mut current_scratch)
                    .unwrap();
                assert!(
                    ir::identical_type(source_signature.params.last().unwrap(), &mapped, &budget,)
                        .unwrap()
                );
                closed
                    .validate_source_flow_signature_with_scratch(
                        ir::SignatureSubject::Call(&ir::Callee::Direct(method_target.clone())),
                        &source_signature,
                        &mut current_scratch,
                    )
                    .unwrap();
                let mut missing_actual = method_target.clone();
                missing_actual.type_args = Arc::from([]);
                assert!(
                    closed
                        .source_direct_call_signature_with_scratch(
                            &missing_actual,
                            &mut current_scratch,
                        )
                        .is_err()
                );
                assert!(
                    closed
                        .service()
                        .resolve_trait_call_with_nominals(
                            &generic_call,
                            &closed,
                            &[],
                            &mut current_scratch,
                        )
                        .is_err()
                );
                let missing_method_actual = ir::TraitCall {
                    method_args: Arc::from([]),
                    ..generic_call.clone()
                };
                assert!(
                    closed
                        .source_callee_entry(
                            &ir::Callee::TraitItem(missing_method_actual),
                            &[],
                            &mut current_scratch,
                        )
                        .is_err()
                );
                let wrong_item = ir::TraitCall {
                    item: source_plan.registry.trait_items[1][0],
                    ..generic_call.clone()
                };
                assert!(
                    closed
                        .source_callee_entry(
                            &ir::Callee::TraitItem(wrong_item),
                            &[],
                            &mut current_scratch,
                        )
                        .is_err()
                );
                assert!(
                    closed
                        .source_callee_entry(
                            &ir::Callee::Untupled(generic_call),
                            &[],
                            &mut current_scratch,
                        )
                        .is_err()
                );
                let flow_call = ir::Callee::Direct(target.clone());
                let flow = closed
                    .source_carrier_contract(ir::SignatureSubject::Call(&flow_call))
                    .unwrap();
                flow.validate_for(&closed).unwrap();
                assert!(flow.validate_for(&entry).is_err());
                assert!(
                    closed
                        .carrier_contract(ir::SignatureSubject::Call(&flow_call))
                        .is_err()
                );
                let source_signature = ir::Signature {
                    params: Vec::new(),
                    result: ir::Ty::Unit,
                    ref_contract: Arc::new(ir::RefContract::default()),
                };
                closed
                    .validate_source_flow_signature(
                        ir::SignatureSubject::Call(&flow_call),
                        &source_signature,
                    )
                    .unwrap();
                let forged_source_signature = ir::Signature {
                    result: ir::Ty::Bool,
                    ..source_signature
                };
                assert!(
                    closed
                        .validate_source_flow_signature(
                            ir::SignatureSubject::Call(&flow_call),
                            &forged_source_signature
                        )
                        .is_err()
                );
                let admission = closed
                    .source_call_entry_with_nominals(&target, &[], &mut current_scratch)
                    .unwrap();
                admission
                    .validate(&target, &closed, &mut current_scratch)
                    .unwrap();
                assert!(
                    admission
                        .validate(&target, &entry, &mut current_scratch)
                        .is_err()
                );
                let changed = ir::CallTarget {
                    declaration: target.declaration,
                    type_args: Arc::from([ir::Ty::Bool]),
                };
                assert!(
                    admission
                        .validate(&changed, &closed, &mut current_scratch)
                        .is_err()
                );
                assert!(
                    closed
                        .source_call_entry_with_nominals(
                            &ir::CallTarget {
                                declaration: target.declaration,
                                type_args: Arc::from([])
                            },
                            &[],
                            &mut current_scratch,
                        )
                        .is_err()
                );
                let invalid_bound = ir::CallTarget {
                    declaration: closed_target.declaration,
                    type_args: Arc::from([ir::Ty::Bool]),
                };
                assert!(
                    closed
                        .source_call_entry_with_nominals(&invalid_bound, &[], &mut current_scratch,)
                        .is_err()
                );
                // Source inventory does not authenticate executable field layout.
                assert!(
                    ir::substitute_callee_with_scratch(
                        &source_call,
                        source.env(),
                        &closed_target.type_args,
                        &closed,
                        &mut source_plan,
                        &budget,
                        &mut current_scratch,
                    )
                    .is_err()
                );
            }
            assert!(
                ir::substitute_source_callee_metadata(
                    &source_call,
                    source.env(),
                    &[],
                    &source,
                    &closed,
                    &mut source_plan,
                    &budget,
                    &mut original_scratch,
                    &mut current_scratch,
                )
                .is_err()
            );
            assert!(
                ir::substitute_source_callee_metadata(
                    &source_call,
                    source.env(),
                    &closed_target.type_args,
                    &source,
                    &entry,
                    &mut source_plan,
                    &budget,
                    &mut original_scratch,
                    &mut current_scratch,
                )
                .is_err()
            );
            let logical_projection =
                ir::Ty::AssociatedType(Arc::new(ir::AssociatedTypeProjection {
                    trait_ref: ir::TraitRef {
                        declaration: source_plan.registry.trait_ids[6],
                        self_ty: mapped.clone(),
                        args: Arc::from([mapped.clone()]),
                    },
                    item: source_plan.registry.trait_associated_types[6][0],
                }));
            for _ in 0..2 {
                let normal = closed
                    .normalize_substituted_source_type(
                        &logical_projection,
                        &source,
                        source.env(),
                        &closed_target.type_args,
                        &budget,
                        &mut current_scratch,
                    )
                    .unwrap();
                assert!(matches!(normal, ir::Ty::Int(ir::IntTy::U8)));
                assert!(
                    closed
                        .normalize_substituted_projection_type(&logical_projection, &budget,)
                        .is_err()
                );
            }
            let changed_projection =
                ir::Ty::AssociatedType(Arc::new(ir::AssociatedTypeProjection {
                    trait_ref: ir::TraitRef {
                        declaration: source_plan.registry.trait_ids[6],
                        self_ty: mapped.clone(),
                        args: Arc::from([ir::Ty::Bool]),
                    },
                    item: source_plan.registry.trait_associated_types[6][0],
                }));
            assert!(
                closed
                    .normalize_substituted_source_type(
                        &changed_projection,
                        &source,
                        source.env(),
                        &closed_target.type_args,
                        &budget,
                        &mut current_scratch,
                    )
                    .is_err()
            );
            assert!(
                closed
                    .source_normal_drop_targets(&mapped, &mut current_scratch)
                    .unwrap()
                    .is_empty()
            );
            assert!(closed.normal_drop_plan(&mapped).is_err());
            // Logical source closure cannot publish executable field layout.
            assert!(
                substitute_source_ty_with_scratch(
                    ty,
                    &source,
                    source.env(),
                    &closed_target.type_args,
                    &closed,
                    &mut source_plan,
                    &budget,
                    None,
                    &mut original_scratch,
                    &mut current_scratch,
                )
                .is_err()
            );
            assert!(
                substitute_source_schema_ty_with_scratch(
                    ty,
                    &source,
                    source.env(),
                    &[],
                    &closed,
                    &mut source_plan,
                    &budget,
                    None,
                    &mut original_scratch,
                    &mut current_scratch,
                )
                .is_err()
            );
            assert!(
                substitute_source_schema_ty_with_scratch(
                    ty,
                    &source,
                    source.env(),
                    &closed_target.type_args,
                    &entry,
                    &mut source_plan,
                    &budget,
                    None,
                    &mut original_scratch,
                    &mut current_scratch,
                )
                .is_err()
            );
        }
        assert!(entry.validate_instance_entry(&target, &budget).is_err());
        assert!(
            entry
                .validate_instance_entry(&bounded, &ir::PlanBudget::default())
                .is_err()
        );
        assert!(
            entry
                .service()
                .discharge_direct_with(&bounded, &entry, &mut tr::TraitScratch::default())
                .is_err()
        );
        assert!(
            facts
                .world()
                .instance_facts(&ir::CallTarget {
                    declaration: bounded.declaration,
                    type_args: Arc::from([ir::Ty::Bool]),
                })
                .is_err()
        );
        assert!(
            facts
                .world()
                .instance_facts(&ir::CallTarget {
                    declaration: bounded.declaration,
                    type_args: Arc::from([]),
                })
                .is_err()
        );
        assert!(
            facts
                .service()
                .discharge_direct_with(&bounded, facts, &mut scratch)
                .is_err()
        );
        let bounded_proof = facts
            .service()
            .discharge_direct_with_nominals(&bounded, facts, &rows, &mut scratch)
            .unwrap();
        assert_eq!(bounded_proof.signature().params, rows);
        let repeated = facts
            .service()
            .discharge_direct_with_nominals(&bounded, facts, &rows, &mut scratch)
            .unwrap();
        assert!(Arc::ptr_eq(
            &bounded_proof.signature().ref_contract,
            &repeated.signature().ref_contract,
        ));
        assert_eq!(repeated.signature().params, rows);
        let canonical_inputs = facts
            .parent_local_call_inputs(
                &ir::Callee::Direct(bounded.clone()),
                bounded_proof.signature(),
            )
            .unwrap();
        assert_eq!(canonical_inputs, rows);
        let mut collector = tr::TraitScratch::default();
        let callee = ir::Callee::Direct(bounded.clone());
        let before = budget.usage_snapshot().work;
        assert_eq!(
            facts
                .parent_local_call_inputs_with_scratch(
                    &callee,
                    bounded_proof.signature(),
                    &mut collector
                )
                .unwrap(),
            canonical_inputs
        );
        let cold = budget.usage_snapshot().work - before;
        let before = budget.usage_snapshot().work;
        assert_eq!(
            facts
                .parent_local_call_inputs_with_scratch(
                    &callee,
                    bounded_proof.signature(),
                    &mut collector
                )
                .unwrap(),
            canonical_inputs
        );
        let warm = budget.usage_snapshot().work - before;
        assert!(
            warm < cold,
            "signature transport discarded metadata arena: {warm}/{cold}"
        );
        let ir::Ty::Struct(sealed) = &definition else {
            unreachable!()
        };
        let mut forged_receipt = sealed.parent_local_receipt().unwrap().as_ref().clone();
        forged_receipt.actuals = Arc::from([ir::Ty::Bool]);
        let mut forged_signature = bounded_proof.signature().clone();
        forged_signature.params[0] = ir::Ty::Struct(
            sealed
                .clone()
                .with_parent_local_receipt(Arc::new(forged_receipt)),
        );
        assert!(
            facts
                .parent_local_call_inputs_with_scratch(&callee, &forged_signature, &mut collector)
                .is_err()
        );
        assert_eq!(
            facts
                .parent_local_call_inputs_with_scratch(
                    &callee,
                    bounded_proof.signature(),
                    &mut collector
                )
                .unwrap(),
            canonical_inputs
        );
        for _ in 0..2 {
            let warmed_inputs = facts
                .parent_local_call_inputs_with_scratch(
                    &ir::Callee::Direct(bounded.clone()),
                    bounded_proof.signature(),
                    &mut scratch,
                )
                .unwrap();
            assert_eq!(warmed_inputs, canonical_inputs);
        }
        facts
            .service()
            .discharge_direct_with_nominals(&bounded, facts, &canonical_inputs, &mut scratch)
            .unwrap();
        let repeated = facts
            .service()
            .discharge_direct_with_nominals(&bounded, facts, &rows, &mut scratch)
            .unwrap();
        assert_eq!(repeated.signature().params, rows);
        assert!(
            facts
                .service()
                .discharge_direct_with(&bounded, facts, &mut scratch)
                .is_err()
        );
        let receiver_inputs = facts.parent_local_receiver_inputs(&definition).unwrap();
        assert_eq!(receiver_inputs.as_slice(), rows.as_slice());
        let mut receiver_scratch = tr::TraitScratch::default();
        let before_receiver = budget.usage_snapshot().work;
        assert_eq!(
            facts
                .parent_local_receiver_inputs_with_scratch(&definition, &mut receiver_scratch)
                .unwrap(),
            receiver_inputs,
        );
        let receiver_cold = budget.usage_snapshot().work - before_receiver;
        let before_receiver = budget.usage_snapshot().work;
        assert_eq!(
            facts
                .parent_local_receiver_inputs_with_scratch(&definition, &mut receiver_scratch)
                .unwrap(),
            receiver_inputs,
        );
        let receiver_warm = budget.usage_snapshot().work - before_receiver;
        assert!(
            receiver_warm < receiver_cold,
            "receiver metadata arena was lost: cold={receiver_cold}, warm={receiver_warm}"
        );
        let missing_caller = facts
            .world()
            .schema_for_tests()
            .validate_nominal(&definition, facts.env(), &budget)
            .unwrap_err();
        assert!(
            missing_caller
                .to_string()
                .contains("parent-local source receipt requires current validated facts")
        );
        let receiver_call = ir::TraitCall {
            trait_ref: ir::TraitRef {
                declaration: plan.registry.trait_ids[3],
                self_ty: definition.clone(),
                args: Arc::from([]),
            },
            item: plan.registry.trait_items[3][0],
            method_args: Arc::from([]),
        };
        facts
            .service()
            .instantiate_trait_signature_with_nominals(
                &receiver_call,
                facts,
                &receiver_inputs,
                &mut scratch,
            )
            .unwrap();
        let source = plan
            .intern_nominal(
                plan.nominal_declaration(0).unwrap(),
                &[ir::Ty::Int(ir::IntTy::I32)],
                checker.facts(),
                &budget,
            )
            .unwrap();
        assert!(
            facts
                .parent_local_receiver_inputs(&source)
                .unwrap()
                .is_empty()
        );
        // Signature instantiation above already replayed these exact seals.
        // Measure membership cold/warm in a new operation arena rather than
        // relying on an unrelated empty invocation to evict those proofs.
        let mut scratch = tr::TraitScratch::default();
        let membership = ir::Predicate::Implements(ir::TraitRef {
            declaration: plan.registry.trait_ids[2],
            self_ty: definition.clone(),
            args: Arc::from([]),
        });
        // Storage<i32>: StorageMarker is a logical source-head fact, not
        // permission to execute with an unnormalized physical field layout.
        // Ordinary direct executable discharge is still rejected above.
        let tr::ProofOutcome::Proved(logical) = facts
            .service()
            .prove_goal_with(&membership, facts, &mut scratch)
            .unwrap()
        else {
            panic!("valid logical source membership was not proved")
        };
        let before_transport = budget.usage_snapshot().work;
        let tr::ProofOutcome::Proved(first) = facts
            .service()
            .prove_goal_with_nominals(&membership, facts, &receiver_inputs, &mut scratch)
            .unwrap()
        else {
            panic!("sealed source membership was not proved")
        };
        let cold_transport = budget.usage_snapshot().work - before_transport;
        let before_transport = budget.usage_snapshot().work;
        let tr::ProofOutcome::Proved(warm) = facts
            .service()
            .prove_goal_with_nominals(&membership, facts, &receiver_inputs, &mut scratch)
            .unwrap()
        else {
            panic!("warm sealed source membership was not proved")
        };
        let warm_transport = budget.usage_snapshot().work - before_transport;
        assert!(
            warm_transport < cold_transport,
            "sealed proof lost its row replay arena: {warm_transport}/{cold_transport}"
        );
        assert!(first.same_allocation(&warm));
        assert!(!logical.same_allocation(&first));
        let repeated_inputs = [receiver_inputs.as_slice(), receiver_inputs.as_slice()].concat();
        let tr::ProofOutcome::Proved(different_transport) = facts
            .service()
            .prove_goal_with_nominals(&membership, facts, &repeated_inputs, &mut scratch)
            .unwrap()
        else {
            panic!("independently validated repeated input was not proved")
        };
        assert!(!first.same_allocation(&different_transport));
        let tr::ProofOutcome::Proved(original_transport) = facts
            .service()
            .prove_goal_with_nominals(&membership, facts, &receiver_inputs, &mut scratch)
            .unwrap()
        else {
            panic!("original sealed transport was not retained")
        };
        assert!(first.same_allocation(&original_transport));
        // The explicit invocation must not replace the separately keyed
        // logical proof, nor grant an executable signature to that proof.
        let tr::ProofOutcome::Proved(logical_warm) = facts
            .service()
            .prove_goal_with(&membership, facts, &mut scratch)
            .unwrap()
        else {
            panic!("warm logical source membership was not proved")
        };
        assert!(logical.same_allocation(&logical_warm));
        assert!(
            entry
                .service()
                .discharge_direct_with(&bounded, &entry, &mut tr::TraitScratch::default(),)
                .is_err()
        );
        facts
            .service()
            .instantiate_trait_signature_with_nominals(&call, facts, &rows, &mut scratch)
            .unwrap();
        facts
            .service()
            .resolve_trait_call_with_nominals(&call, facts, &rows, &mut scratch)
            .unwrap();
        let ir::Ty::Struct(value) = &definition else {
            panic!("source kind changed")
        };
        let stripped = ir::Ty::Struct(ir::StructTy::new_parent_local(value.def().clone()));
        assert!(facts.parent_local_receiver_inputs(&stripped).is_err());
        let stripped_call = ir::TraitCall {
            trait_ref: ir::TraitRef {
                self_ty: stripped.clone(),
                ..receiver_call.trait_ref.clone()
            },
            ..receiver_call.clone()
        };
        assert!(
            facts
                .service()
                .instantiate_trait_signature_with_nominals(
                    &stripped_call,
                    facts,
                    &receiver_inputs,
                    &mut scratch,
                )
                .is_err()
        );
        assert!(
            facts
                .service()
                .prove_goal_with_nominals(
                    &membership,
                    facts,
                    std::slice::from_ref(&stripped),
                    &mut scratch,
                )
                .is_err()
        );
        assert!(
            facts
                .service()
                .resolve_trait_call_with_nominals(&call, facts, &[stripped], &mut scratch)
                .is_err()
        );
        let mut changed = value.def().clone();
        let mut fields = changed.fields.read(&budget).unwrap().to_vec();
        fields[0].ty = ir::Ty::Bool;
        changed.fields = Arc::<[ir::FieldDef]>::from(fields).into();
        let changed = ir::Ty::Struct(
            ir::StructTy::new_parent_local(changed)
                .with_parent_local_receipt(value.parent_local_receipt().unwrap().clone()),
        );
        assert!(facts.parent_local_receiver_inputs(&changed).is_err());
        assert!(
            facts
                .parent_local_receiver_inputs_with_scratch(&changed, &mut receiver_scratch)
                .is_err()
        );
        assert!(
            facts
                .service()
                .prove_goal_with_nominals(
                    &membership,
                    facts,
                    std::slice::from_ref(&changed),
                    &mut scratch,
                )
                .is_err()
        );
        assert!(
            facts
                .service()
                .resolve_trait_call_with_nominals(&call, facts, &[changed], &mut scratch)
                .is_err()
        );
        let universal = plan.callable_facts(owner).unwrap();
        assert!(universal.parent_local_receiver_inputs(&definition).is_err());
        assert!(
            facts
                .service()
                .instantiate_trait_signature_with_nominals(
                    &receiver_call,
                    &universal,
                    &receiver_inputs,
                    &mut scratch,
                )
                .is_err()
        );
        assert!(
            facts
                .service()
                .prove_goal_with_nominals(&membership, &universal, &rows, &mut scratch)
                .is_err()
        );
        assert!(
            facts
                .service()
                .resolve_trait_call_with_nominals(&call, &universal, &rows, &mut scratch)
                .is_err()
        );
        let other = plan
            .instance_facts(&ir::CallTarget {
                declaration: plan.function_decl(1).unwrap(),
                type_args: Arc::from([]),
            })
            .unwrap();
        assert!(other.parent_local_receiver_inputs(&definition).is_err());
        assert!(
            other
                .parent_local_receiver_inputs_with_scratch(&definition, &mut receiver_scratch)
                .is_err()
        );
        assert!(
            facts
                .service()
                .instantiate_trait_signature_with_nominals(
                    &receiver_call,
                    &other,
                    &receiver_inputs,
                    &mut scratch,
                )
                .is_err()
        );
        assert!(
            facts
                .service()
                .prove_goal_with_nominals(&membership, &other, &rows, &mut scratch)
                .is_err()
        );
        assert!(
            facts
                .service()
                .resolve_trait_call_with_nominals(&call, &other, &rows, &mut scratch)
                .is_err()
        );
        let foreign = PlanCtx::from_program(&parse(text).unwrap(), &budget).unwrap();
        let foreign_facts = foreign
            .instance_facts(&ir::CallTarget {
                declaration: foreign.function_decl(0).unwrap(),
                type_args: Arc::from([]),
            })
            .unwrap();
        assert!(
            foreign_facts
                .parent_local_receiver_inputs(&definition)
                .is_err()
        );
        assert!(
            foreign_facts
                .service()
                .instantiate_trait_signature_with_nominals(
                    &receiver_call,
                    &foreign_facts,
                    &receiver_inputs,
                    &mut scratch,
                )
                .is_err()
        );
        assert!(
            foreign_facts
                .service()
                .prove_goal_with_nominals(&membership, &foreign_facts, &rows, &mut scratch)
                .is_err()
        );
        assert!(
            foreign_facts
                .service()
                .resolve_trait_call_with_nominals(&call, &foreign_facts, &rows, &mut scratch)
                .is_err()
        );
        for (trait_index, method_args, result) in [
            (
                4,
                Arc::<[ir::Ty]>::from([ir::Ty::Int(ir::IntTy::I32)]),
                ir::Ty::Int(ir::IntTy::I32),
            ),
            (5, Arc::from([]), ir::Ty::Int(ir::IntTy::U8)),
        ] {
            let required_call = ir::TraitCall {
                trait_ref: ir::TraitRef {
                    declaration: plan.registry.trait_ids[trait_index],
                    self_ty: definition.clone(),
                    args: Arc::from([]),
                },
                item: plan.registry.trait_items[trait_index][0],
                method_args,
            };
            let signature = facts
                .service()
                .instantiate_trait_signature_with_nominals(
                    &required_call,
                    facts,
                    &receiver_inputs,
                    &mut scratch,
                )
                .unwrap_or_else(|error| panic!("required trait {trait_index}: {error}"));
            assert_eq!(signature.result, result);
            let stripped_call = ir::TraitCall {
                trait_ref: ir::TraitRef {
                    self_ty: ir::Ty::Struct(ir::StructTy::new_parent_local(value.def().clone())),
                    ..required_call.trait_ref.clone()
                },
                ..required_call.clone()
            };
            assert!(
                facts
                    .service()
                    .instantiate_trait_signature_with_nominals(
                        &stripped_call,
                        facts,
                        &receiver_inputs,
                        &mut scratch,
                    )
                    .is_err()
            );
            for changed_facts in [&universal, &other, &foreign_facts] {
                assert!(
                    changed_facts
                        .service()
                        .instantiate_trait_signature_with_nominals(
                            &required_call,
                            changed_facts,
                            &receiver_inputs,
                            &mut scratch,
                        )
                        .is_err()
                );
            }
        }
        let source_entry_target = ir::CallTarget {
            declaration: plan.function_decl(4).unwrap(),
            type_args: Arc::from([definition.clone()]),
        };
        let source_entry = facts
            .source_call_entry_with_nominals(&source_entry_target, &receiver_inputs, &mut scratch)
            .unwrap();
        source_entry
            .validate(&source_entry_target, facts, &mut scratch)
            .unwrap();
        assert!(
            source_entry
                .validate(&source_entry_target, &other, &mut scratch)
                .is_err()
        );
        assert!(
            source_entry
                .validate(&source_entry_target, &foreign_facts, &mut scratch)
                .is_err()
        );
        assert!(
            facts
                .source_call_entry_with_nominals(
                    &source_entry_target,
                    &[ir::Ty::Struct(ir::StructTy::new_parent_local(
                        value.def().clone()
                    ))],
                    &mut scratch,
                )
                .is_err()
        );
        assert!(budget.charge_work(usize::MAX).is_err());
        assert!(
            facts
                .parent_local_receiver_inputs_with_scratch(&definition, &mut receiver_scratch)
                .unwrap_err()
                .is_resource()
        );
        assert!(
            source_entry
                .validate(&source_entry_target, facts, &mut scratch)
                .is_err()
        );
        assert!(
            facts
                .source_call_entry_with_nominals(
                    &source_entry_target,
                    &receiver_inputs,
                    &mut scratch,
                )
                .is_err()
        );
        assert!(
            facts
                .parent_local_call_inputs_with_scratch(
                    &callee,
                    bounded_proof.signature(),
                    &mut collector
                )
                .is_err()
        );
        assert!(
            facts
                .service()
                .prove_goal_with_nominals(&membership, facts, &receiver_inputs, &mut scratch)
                .is_err()
        );
    }

    #[test]
    fn projected_parent_local_receipt_replays_source_fields_and_exact_context() {
        let text = "trait Has{type Out;}impl Has for i32{type Out=u8;}struct Storage<T:Has>{inner:StorageSlot<T::Out>}struct Other<T:Has>{inner:StorageSlot<T::Out>}enum StorageSlot<T>{Empty,Full(T)}fn main(){}fn other(){}";
        let program = parse(text).unwrap();
        let budget = ir::PlanBudget::default();
        let mut plan = PlanCtx::from_program(&program, &budget).unwrap();
        let owner = plan.function_decl(0).unwrap();
        let declaration = plan.nominal_declaration(0).unwrap();
        let signatures = plan.environment(Some(owner)).unwrap();
        let function = plan.callable_body(&plan.program, owner).unwrap();
        let mut checker = Checker::new(&signatures, ir::Ty::Unit, function.module);
        checker.declaration = Some(owner);
        let original = checker
            .mint_parent_local_nominal(declaration, &[ir::Ty::Int(ir::IntTy::I32)])
            .unwrap();
        let facts = checker.facts().validated().unwrap();
        facts.validate_ty_metadata(&original).unwrap();
        assert_eq!(
            facts.world().normalize_ty(&original, facts).unwrap(),
            original
        );
        let ir::Ty::Struct(value) = &original else {
            panic!("source kind changed")
        };
        let receipt = value.parent_local_receipt().unwrap().clone();
        let adapter = crate::compiler::frontend::SchemaAdapter {
            program: plan.program.clone(),
            catalog: plan.catalog.clone(),
            registry: plan.registry.clone(),
            nominal_schemas: plan.nominal_schemas.clone(),
            nominals: plan.nominals.clone(),
            contexts: plan.contexts.clone(),
            source_schemas: plan.source_schemas.clone(),
            budget: budget.clone(),
        };
        let mut source_session = tr::SchemaCx::nominal_validation_session(&adapter)
            .unwrap()
            .unwrap();
        for _ in 0..2 {
            source_session
                .validate_node(&original, facts.env(), Some(facts), &budget)
                .unwrap();
        }
        let mut altered_receipt = receipt.as_ref().clone();
        altered_receipt.actuals = Arc::from([ir::Ty::Bool]);
        let altered_seal = ir::Ty::Struct(
            value
                .clone()
                .with_parent_local_receipt(Arc::new(altered_receipt)),
        );
        let ir::Ty::Struct(altered_value) = &altered_seal else {
            unreachable!()
        };
        assert!(std::ptr::eq(value.def(), altered_value.def()));
        assert!(
            source_session
                .validate_node(&altered_seal, facts.env(), Some(facts), &budget)
                .is_err()
        );
        source_session
            .validate_node(&original, facts.env(), Some(facts), &budget)
            .unwrap();
        let closing_target = ir::CallTarget {
            declaration: owner,
            type_args: Arc::from([]),
        };
        let closed_facts = facts.world().instance_facts(&closing_target).unwrap();
        assert_eq!(facts.phase(), tr::FactsPhase::Universal);
        let mut source_scratch = tr::TraitScratch::default();
        let mut closing_scratch = tr::TraitScratch::default();
        use crate::compiler::nominal_realization::substitution::{
            substitute_source_schema_ty_with_scratch, substitute_source_ty_with_scratch,
        };
        let mut schema_original_scratch = tr::TraitScratch::default();
        let mut schema_closed_scratch = tr::TraitScratch::default();
        let before = budget.usage_snapshot().work;
        let first_schema = substitute_source_schema_ty_with_scratch(
            &original,
            facts,
            facts.env(),
            &closing_target.type_args,
            &closed_facts,
            &mut plan,
            &budget,
            None,
            &mut schema_original_scratch,
            &mut schema_closed_scratch,
        )
        .unwrap();
        let cold_work = budget.usage_snapshot().work - before;
        let before = budget.usage_snapshot().work;
        let warm_schema = substitute_source_schema_ty_with_scratch(
            &original,
            facts,
            facts.env(),
            &closing_target.type_args,
            &closed_facts,
            &mut plan,
            &budget,
            None,
            &mut schema_original_scratch,
            &mut schema_closed_scratch,
        )
        .unwrap();
        let warm_work = budget.usage_snapshot().work - before;
        assert!(warm_work < cold_work, "cold={cold_work} warm={warm_work}");
        assert_eq!(first_schema, warm_schema);
        closed_facts.validate_ty_metadata(&warm_schema).unwrap();
        assert!(
            substitute_source_schema_ty_with_scratch(
                &original,
                &closed_facts,
                closed_facts.env(),
                &closing_target.type_args,
                &closed_facts,
                &mut plan,
                &budget,
                None,
                &mut schema_original_scratch,
                &mut schema_closed_scratch,
            )
            .is_err()
        );
        assert!(
            substitute_source_schema_ty_with_scratch(
                &altered_seal,
                facts,
                facts.env(),
                &closing_target.type_args,
                &closed_facts,
                &mut plan,
                &budget,
                None,
                &mut schema_original_scratch,
                &mut schema_closed_scratch,
            )
            .is_err()
        );
        for _ in 0..2 {
            let mapped = substitute_source_ty_with_scratch(
                &original,
                facts,
                facts.env(),
                &closing_target.type_args,
                &closed_facts,
                &mut plan,
                &budget,
                None,
                &mut source_scratch,
                &mut closing_scratch,
            )
            .unwrap();
            let ir::Ty::Struct(mapped_struct) = &mapped else {
                panic!("mapped source struct changed kind")
            };
            assert_eq!(mapped_struct.id(), value.id());
            assert!(!Arc::ptr_eq(
                &receipt,
                mapped_struct.parent_local_receipt().unwrap()
            ));
            closed_facts.validate_ty_metadata(&mapped).unwrap();
            assert!(closed_facts.validate_ty_metadata(&original).is_err());
            assert!(facts.validate_ty_metadata(&mapped).is_err());
            let current = substitute_source_ty_with_scratch(
                &mapped,
                &closed_facts,
                closed_facts.env(),
                &closing_target.type_args,
                &closed_facts,
                &mut plan,
                &budget,
                None,
                &mut source_scratch,
                &mut closing_scratch,
            )
            .unwrap();
            let ir::Ty::Struct(current) = &current else {
                panic!("current source struct changed kind")
            };
            assert!(Arc::ptr_eq(
                current.parent_local_receipt().unwrap(),
                mapped_struct.parent_local_receipt().unwrap()
            ));
            assert!(
                substitute_source_ty_with_scratch(
                    &original,
                    &closed_facts,
                    closed_facts.env(),
                    &closing_target.type_args,
                    &closed_facts,
                    &mut plan,
                    &budget,
                    None,
                    &mut source_scratch,
                    &mut closing_scratch,
                )
                .is_err()
            );
            assert!(
                substitute_source_ty_with_scratch(
                    &mapped,
                    &closed_facts,
                    closed_facts.env(),
                    &[ir::Ty::Bool],
                    &closed_facts,
                    &mut plan,
                    &budget,
                    None,
                    &mut source_scratch,
                    &mut closing_scratch,
                )
                .is_err()
            );
        }
        let copied_env = ir::GenericEnv::new(facts.env().scopes(), &budget).unwrap();
        assert!(
            substitute_source_ty_with_scratch(
                &original,
                facts,
                &copied_env,
                &closing_target.type_args,
                &closed_facts,
                &mut plan,
                &budget,
                None,
                &mut source_scratch,
                &mut closing_scratch,
            )
            .is_err()
        );
        assert!(
            substitute_source_ty_with_scratch(
                &original,
                facts,
                facts.env(),
                &[ir::Ty::Bool],
                &closed_facts,
                &mut plan,
                &budget,
                None,
                &mut source_scratch,
                &mut closing_scratch,
            )
            .is_err()
        );
        let closed = receipt
            .remint_for_source_substitution(
                &original,
                facts,
                &closed_facts,
                facts.env(),
                &[],
                &[ir::Ty::Int(ir::IntTy::I32)],
                &budget,
            )
            .unwrap();
        closed_facts.validate_ty_metadata(&closed).unwrap();
        assert!(closed_facts.validate_ty_metadata(&original).is_err());
        assert!(facts.validate_ty_metadata(&closed).is_err());
        let ir::Ty::Struct(closed_value) = &closed else {
            panic!("closed receipt changed nominal kind")
        };
        assert_eq!(value.id(), closed_value.id());
        assert!(!Arc::ptr_eq(
            &receipt,
            closed_value.parent_local_receipt().unwrap()
        ));
        let repeated_closed = closed_value
            .parent_local_receipt()
            .unwrap()
            .remint_for_source_substitution(
                &closed,
                facts,
                &closed_facts,
                facts.env(),
                &[],
                &[ir::Ty::Int(ir::IntTy::I32)],
                &budget,
            )
            .unwrap();
        let ir::Ty::Struct(repeated_value) = repeated_closed else {
            panic!("closed receipt changed kind")
        };
        assert!(Arc::ptr_eq(
            closed_value.parent_local_receipt().unwrap(),
            repeated_value.parent_local_receipt().unwrap()
        ));
        assert!(
            receipt
                .remint_for_source_substitution(
                    &original,
                    facts,
                    &closed_facts,
                    facts.env(),
                    &[],
                    &[ir::Ty::Bool],
                    &budget
                )
                .is_err()
        );
        assert!(
            receipt
                .remint_for_source_substitution(
                    &original,
                    &closed_facts,
                    &closed_facts,
                    facts.env(),
                    &[],
                    &[ir::Ty::Int(ir::IntTy::I32)],
                    &budget
                )
                .is_err()
        );
        let foreign_target = ir::CallTarget {
            declaration: plan.function_decl(1).unwrap(),
            type_args: Arc::from([]),
        };
        let foreign_closed = facts.world().instance_facts(&foreign_target).unwrap();
        assert!(
            receipt
                .remint_for_source_substitution(
                    &original,
                    facts,
                    &foreign_closed,
                    facts.env(),
                    &[],
                    &[ir::Ty::Int(ir::IntTy::I32)],
                    &budget
                )
                .is_err()
        );
        let (_, template) = plan.nominal_schema(declaration, &budget).unwrap();
        let mut replay_memo = ParentLocalReplayMemo::default();
        let mut schema_metadata_scratch = tr::TraitScratch::default();
        for _ in 0..2 {
            facts
                .validate_ty_metadata_scratch(&original, &budget, &mut schema_metadata_scratch)
                .unwrap();
            replay_memo
                .replay(&receipt, &original, &template, facts, &budget)
                .unwrap();
            receipt
                .replay_with_scratch(
                    &original,
                    &template,
                    facts,
                    &budget,
                    &mut schema_metadata_scratch,
                )
                .unwrap();
        }
        let source = plan
            .intern_nominal(
                declaration,
                &[ir::Ty::Int(ir::IntTy::I32)],
                checker.facts(),
                &budget,
            )
            .unwrap();
        assert!(facts.world().normalize_ty(&source, facts).is_err());
        assert!(
            receipt
                .replay_with_scratch(
                    &original,
                    &source,
                    facts,
                    &budget,
                    &mut schema_metadata_scratch,
                )
                .is_err()
        );
        assert!(
            replay_memo
                .replay(&receipt, &original, &source, facts, &budget)
                .is_err()
        );
        let rows = [original.clone()];
        let validation = std::cell::RefCell::default();
        let transport = tr::ParentLocalTransport::new(&rows).with_validation(&validation);
        let mut selection_replays = tr::NominalReplays::default();
        let before = budget.usage_snapshot().work;
        assert_eq!(
            transport
                .select_with_replays(facts.world(), &source, facts, &mut selection_replays)
                .unwrap(),
            Some(original.clone())
        );
        let cold = budget.usage_snapshot().work - before;
        let before = budget.usage_snapshot().work;
        assert_eq!(
            transport
                .select_with_replays(facts.world(), &source, facts, &mut selection_replays)
                .unwrap(),
            Some(original.clone())
        );
        let warm = budget.usage_snapshot().work - before;
        assert!(
            warm < cold,
            "selection discarded source metadata: {warm}/{cold}"
        );
        for _ in 0..2 {
            assert_eq!(
                transport.select(facts.world(), &source, facts).unwrap(),
                Some(original.clone())
            );
        }
        {
            let _occupied = validation.borrow_mut();
            assert_eq!(
                transport.select(facts.world(), &source, facts).unwrap(),
                Some(original.clone())
            );
        }
        assert!(
            tr::ParentLocalTransport::default()
                .select(facts.world(), &source, facts)
                .unwrap()
                .is_none()
        );
        let target = ir::CallTarget {
            declaration: owner,
            type_args: Arc::from([]),
        };
        let mut invocation_scratch = tr::TraitScratch::default();
        facts
            .service()
            .discharge_direct_with_nominals(&target, facts, &rows, &mut invocation_scratch)
            .unwrap();
        let stripped = ir::Ty::Struct(ir::StructTy::new_parent_local(value.def().clone()));
        assert!(facts.validate_ty_metadata(&stripped).is_err());
        assert!(facts.world().normalize_ty(&stripped, facts).is_err());
        assert!(
            tr::ParentLocalTransport::default()
                .select(facts.world(), &stripped, facts)
                .is_err()
        );
        assert!(
            facts
                .service()
                .discharge_direct_with_nominals(
                    &target,
                    facts,
                    &[stripped.clone()],
                    &mut invocation_scratch
                )
                .is_err()
        );
        assert!(transport.select(facts.world(), &stripped, facts).is_err());
        assert!(
            transport
                .select_with_replays(facts.world(), &stripped, facts, &mut selection_replays)
                .is_err()
        );
        let stripped = ir::Ty::Struct(ir::StructTy::from_ref(ir::NominalRef::Strong(Arc::new(
            value.def().clone(),
        ))));
        assert!(facts.validate_ty_metadata(&stripped).is_err());
        assert!(value.clone().into_def().is_err());
        let mut changed = value.def().clone();
        let mut fields = changed.fields.read(&budget).unwrap().to_vec();
        fields[0].ty = ir::Ty::Bool;
        changed.fields = Arc::<[ir::FieldDef]>::from(fields).into();
        let changed = ir::Ty::Struct(
            ir::StructTy::new_parent_local(changed).with_parent_local_receipt(receipt.clone()),
        );
        assert!(
            replay_memo
                .replay(&receipt, &changed, &template, facts, &budget)
                .is_err()
        );
        assert!(facts.validate_ty_metadata(&changed).is_err());
        assert!(
            receipt
                .remint_for_source_substitution(
                    &changed,
                    facts,
                    &closed_facts,
                    facts.env(),
                    &[],
                    &[ir::Ty::Int(ir::IntTy::I32)],
                    &budget
                )
                .is_err()
        );
        assert!(
            facts
                .validate_ty_metadata_scratch(&changed, &budget, &mut schema_metadata_scratch)
                .is_err()
        );
        assert!(facts.world().normalize_ty(&changed, facts).is_err());
        assert!(
            tr::ParentLocalTransport::default()
                .select(facts.world(), &changed, facts)
                .is_err()
        );
        assert!(
            facts
                .service()
                .discharge_direct_with_nominals(
                    &target,
                    facts,
                    &[changed.clone()],
                    &mut invocation_scratch
                )
                .is_err()
        );
        assert!(
            tr::ParentLocalTransport::new(&[changed])
                .with_validation(&validation)
                .select(facts.world(), &source, facts)
                .is_err()
        );
        let mut changed = value.def().clone();
        let ir::NominalIndex::ParentLocal { owner, ordinal } = changed.id.index else {
            panic!("source identity changed")
        };
        changed.id.index = ir::NominalIndex::ParentLocal {
            owner,
            ordinal: ordinal + 1,
        };
        let changed = ir::Ty::Struct(
            ir::StructTy::new_parent_local(changed).with_parent_local_receipt(receipt.clone()),
        );
        assert!(facts.validate_ty_metadata(&changed).is_err());
        let mut changed = value.def().clone();
        changed.source_decl = plan.nominal_declaration(1).unwrap();
        let changed = ir::Ty::Struct(
            ir::StructTy::new_parent_local(changed).with_parent_local_receipt(receipt.clone()),
        );
        assert!(facts.validate_ty_metadata(&changed).is_err());
        assert!(
            tr::ParentLocalTransport::new(&[original.clone(), changed])
                .with_validation(&validation)
                .select(facts.world(), &source, facts)
                .is_err()
        );
        let mut changed = receipt.as_ref().clone();
        let mut binder = changed.env.binder();
        binder.declaration.index += 1;
        changed.env = Arc::new(ir::ParamEnv::new(binder, 1).unwrap());
        assert!(
            replay_memo
                .replay(
                    &Arc::new(changed.clone()),
                    &original,
                    &template,
                    facts,
                    &budget
                )
                .is_err()
        );
        assert!(
            changed
                .replay(&original, &template, facts, &budget)
                .is_err()
        );
        let other = plan.callable_facts(plan.function_decl(1).unwrap()).unwrap();
        assert!(
            replay_memo
                .replay(&receipt, &original, &template, &other, &budget)
                .is_err()
        );
        assert!(transport.select(facts.world(), &source, &other).is_err());
        assert!(facts.world().normalize_ty(&original, &other).is_err());
        assert!(
            facts
                .service()
                .discharge_direct_with_nominals(&target, &other, &rows, &mut invocation_scratch)
                .is_err()
        );
        let mut memo = tr::TraitScratch::default();
        facts
            .validate_ty_metadata_scratch(&original, &budget, &mut memo)
            .unwrap();
        assert!(
            other
                .validate_ty_metadata_scratch(&original, &budget, &mut memo)
                .is_err()
        );
        checker.parent_local_nominals.borrow_mut()[0].context =
            other.parent_local_context().unwrap();
        let reminted = checker
            .mint_parent_local_nominal(declaration, &[ir::Ty::Int(ir::IntTy::I32)])
            .unwrap();
        let conflicting = [original.clone(), reminted.clone()];
        assert!(
            tr::ParentLocalTransport::new(&conflicting)
                .with_validation(&validation)
                .select(facts.world(), &source, facts)
                .is_err()
        );
        let ir::Ty::Struct(reminted) = reminted else {
            panic!("remint changed source kind")
        };
        assert_ne!(reminted.id(), value.id());
        assert_eq!(checker.parent_local_nominals.borrow().len(), 2);
        assert!(
            receipt
                .replay(&original, &template, &other, &budget)
                .is_err()
        );
        let closed = plan
            .instance_facts(&ir::CallTarget {
                declaration: owner,
                type_args: Arc::from([]),
            })
            .unwrap();
        assert!(
            replay_memo
                .replay(&receipt, &original, &template, &closed, &budget)
                .is_err()
        );
        assert!(
            receipt
                .replay(&original, &template, &closed, &budget)
                .is_err()
        );
        let foreign_budget = ir::PlanBudget::default();
        let foreign = PlanCtx::from_program(&parse(text).unwrap(), &foreign_budget).unwrap();
        let foreign_facts = foreign
            .callable_facts(foreign.function_decl(0).unwrap())
            .unwrap();
        assert!(
            replay_memo
                .replay(
                    &receipt,
                    &original,
                    &template,
                    &foreign_facts,
                    &foreign_budget
                )
                .is_err()
        );
        assert!(
            replay_memo
                .replay(&receipt, &original, &template, facts, &foreign_budget)
                .is_err()
        );
        assert!(
            receipt
                .replay(&original, &template, &foreign_facts, &foreign_budget)
                .is_err()
        );
        assert!(
            receipt
                .replay(&original, &template, facts, &foreign_budget)
                .is_err()
        );
        assert!(transport.select(facts.world(), &source, &closed).is_err());
        assert!(
            transport
                .select_with_replays(facts.world(), &source, &closed, &mut selection_replays)
                .is_err()
        );
        assert!(
            transport
                .select(facts.world(), &source, &foreign_facts)
                .is_err()
        );
        assert!(
            transport
                .select_with_replays(
                    facts.world(),
                    &source,
                    &foreign_facts,
                    &mut selection_replays
                )
                .is_err()
        );
        assert_eq!(
            transport.select(facts.world(), &source, facts).unwrap(),
            Some(original.clone())
        );
        assert!(budget.charge_work(usize::MAX).is_err());
        let failure = budget.first_resource_failure().unwrap();
        assert!(
            transport
                .select_with_replays(facts.world(), &source, facts, &mut selection_replays)
                .unwrap_err()
                .is_resource()
        );
        assert!(transport.select(facts.world(), &source, facts).is_err());
        assert!(
            facts
                .validate_ty_metadata_scratch(&original, &budget, &mut schema_metadata_scratch)
                .is_err()
        );
        assert!(
            replay_memo
                .replay(&receipt, &original, &template, facts, &budget)
                .is_err()
        );
        assert!(facts.parent_local_receiver_inputs(&original).is_err());
        assert!(
            receipt
                .replay(&original, &template, facts, &budget)
                .is_err()
        );
        assert!(
            substitute_source_schema_ty_with_scratch(
                &original,
                facts,
                facts.env(),
                &closing_target.type_args,
                &closed_facts,
                &mut plan,
                &budget,
                None,
                &mut schema_original_scratch,
                &mut schema_closed_scratch,
            )
            .is_err()
        );
        assert_eq!(budget.first_resource_failure(), Some(failure));
    }

    fn function_marker(budget: &ir::PlanBudget) -> (ir::ParamEnv, ir::Ty) {
        let declaration = ir::DeclId {
            snapshot: ir::SnapshotId::standalone().unwrap(),
            index: 1,
        };
        let binder = ir::BinderId { declaration };
        let env = ir::ParamEnv::new(binder, 1).unwrap();
        let parameter = ir::Ty::Param(ir::ParamId {
            owner: binder,
            index: 0,
        });
        let ty = ir::Ty::FnPointer(
            ir::FnPointerTy::new(vec![parameter.clone()], parameter, budget).unwrap(),
        );
        (env, ty)
    }

    #[test]
    fn parent_local_function_marker_substitution_keeps_exact_binder() {
        let budget = ir::PlanBudget::default();
        let (env, ty) = function_marker(&budget);
        assert!(mentions_param(&ty, &budget, 0).unwrap());
        let mapped = substitute(&ty, &env, &[ir::Ty::Bool], &budget, 0).unwrap();
        let ir::Ty::FnPointer(pointer) = &mapped else {
            panic!("function marker kind changed")
        };
        assert!(matches!(pointer.inputs.as_ref(), [ir::Ty::Bool]));
        assert!(matches!(pointer.output, ir::Ty::Bool));
        assert!(!mentions_param(&mapped, &budget, 0).unwrap());
        let mut foreign = env.binder();
        foreign.declaration.index += 1;
        let foreign = ir::ParamEnv::new(foreign, 1).unwrap();
        assert!(
            substitute(&ty, &foreign, &[ir::Ty::Bool], &budget, 0)
                .unwrap_err()
                .contains("foreign parameter")
        );
        assert!(
            substitute(&ty, &env, &[], &budget, 0)
                .unwrap_err()
                .contains("outside nominal actuals")
        );
    }

    #[test]
    fn parent_local_wrapper_substitution_preserves_first_resource_failure() {
        let setup = ir::PlanBudget::default();
        let (env, ty) = function_marker(&setup);
        let budget = ir::PlanBudget::new(ir::PlanLimits {
            work: 1,
            ..ir::PlanLimits::default()
        });
        assert!(
            substitute(&ty, &env, &[ir::Ty::Bool], &budget, 0)
                .unwrap_err()
                .contains("resource")
        );
        let failure = budget.first_resource_failure().unwrap();
        assert!(
            mentions_param(&ty, &budget, 0)
                .unwrap_err()
                .contains("resource")
        );
        assert_eq!(budget.first_resource_failure(), Some(failure));
    }
}
