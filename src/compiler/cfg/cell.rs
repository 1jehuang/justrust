//! Exact source-owned scalar interior operations. No payload place is exposed.

use super::*;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CellOperation {
    New,
    Get,
    Set,
}
impl CellOperation {
    fn declaration(self, catalog: &super::super::traits::CoreCatalog) -> ir::DeclId {
        use super::super::traits::CatalogItem;
        catalog.id(match self {
            Self::New => CatalogItem::CellNew,
            Self::Get => CatalogItem::CellGet,
            Self::Set => CatalogItem::CellSet,
        })
    }
}
#[derive(Clone, Copy)]
pub(super) struct CellOperands<'a> {
    pub operation: CellOperation,
    pub reference: Option<&'a Place>,
    pub value: Option<&'a Operand>,
    pub origin: Option<ExprId>,
}
struct OriginalCell<'a> {
    operation: CellOperation,
    descriptor: &'a Arc<ir::CellTy>,
    source: &'a Arc<super::super::frontend::cell::CellSourceOperation>,
    receiver: Option<ExprId>,
    value: Option<ExprId>,
}
#[derive(Clone, Debug)]
struct ClosureCellProof {
    substitution: Option<super::super::source_replay::OriginalBodyId>,
    original_descriptor: Arc<ir::CellTy>,
    receiver: Option<ExprId>,
    value: Option<ExprId>,
}
#[derive(Clone, Debug)]
pub struct CellOperationProof {
    closure: Option<ClosureCellProof>,
    publication: super::super::source_replay::OriginalBodyId,
    site: ExprId,
    operation: CellOperation,
    descriptor: Arc<ir::CellTy>,
    source: Arc<super::super::frontend::cell::CellSourceOperation>,
    reference: Option<Place>,
    value: Option<Operand>,
}
impl PartialEq for CellOperationProof {
    fn eq(&self, other: &Self) -> bool {
        let closure_equal = match (&self.closure, &other.closure) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                a.substitution == b.substitution
                    && Arc::ptr_eq(&a.original_descriptor, &b.original_descriptor)
                    && a.receiver == b.receiver
                    && a.value == b.value
            }
            _ => false,
        };
        closure_equal
            && self.publication == other.publication
            && self.site == other.site
            && self.operation == other.operation
            && Arc::ptr_eq(&self.descriptor, &other.descriptor)
            && Arc::ptr_eq(&self.source, &other.source)
    }
}
impl Eq for CellOperationProof {}
impl CellOperationProof {
    pub(in crate::compiler) fn descriptor(&self) -> &Arc<ir::CellTy> {
        &self.descriptor
    }
    fn original(
        publication: &Arc<super::super::source_replay::CanonicalSourcePublication>,
        site: ExprId,
    ) -> Result<OriginalCell<'_>, String> {
        Self::original_in(publication.source(), site)
    }
    fn original_in(source: &ir::CheckedFunction, site: ExprId) -> Result<OriginalCell<'_>, String> {
        let expression = source
            .exprs
            .get(site.0)
            .ok_or("Cell original source occurrence is missing")?;
        Ok(match &expression.kind {
            ir::ExprKind::CellNew { ty, value, source } => OriginalCell {
                operation: CellOperation::New,
                descriptor: ty,
                source,
                receiver: None,
                value: Some(*value),
            },
            ir::ExprKind::CellGet {
                ty,
                receiver,
                source,
            } => OriginalCell {
                operation: CellOperation::Get,
                descriptor: ty,
                source,
                receiver: Some(*receiver),
                value: None,
            },
            ir::ExprKind::CellSet {
                ty,
                receiver,
                value,
                source,
            } => OriginalCell {
                operation: CellOperation::Set,
                descriptor: ty,
                source,
                receiver: Some(*receiver),
                value: Some(*value),
            },
            _ => {
                return Err(
                    "Cell receipt does not refer to an original canonical operation".into(),
                );
            }
        })
    }
    fn same_reference(
        a: Option<&Place>,
        b: Option<&Place>,
        budget: &ir::PlanBudget,
    ) -> Result<bool, String> {
        match (a, b) {
            (None, None) => Ok(true),
            (Some(a), Some(b)) => ManuallyDropTakeProof::same_place(a, b, budget),
            _ => Ok(false),
        }
    }
    fn same_value(
        a: Option<&Operand>,
        b: Option<&Operand>,
        budget: &ir::PlanBudget,
    ) -> Result<bool, String> {
        match (a, b) {
            (None, None) => Ok(true),
            (Some(a), Some(b)) => ManuallyDropIntoInnerProof::same_value(a, b, budget),
            _ => Ok(false),
        }
    }
    pub(super) fn same_original(
        &self,
        other: &Self,
        budget: &ir::PlanBudget,
    ) -> Result<bool, String> {
        budget.charge_work(1)?;
        Ok(self == other
            && Self::same_reference(self.reference.as_ref(), other.reference.as_ref(), budget)?
            && Self::same_value(self.value.as_ref(), other.value.as_ref(), budget)?)
    }
    pub(super) fn seal(
        publication: &Arc<super::super::source_replay::CanonicalSourcePublication>,
        site: ExprId,
        reference: Option<&Place>,
        value: Option<&Operand>,
        budget: &ir::PlanBudget,
    ) -> Result<Arc<Self>, String> {
        let OriginalCell {
            operation,
            descriptor,
            source,
            ..
        } = Self::original(publication, site)?;
        let value = value
            .map(|operand| match operand {
                Operand::Copy(place) => Ok(Operand::Copy(ManuallyDropTakeProof::copy_place(
                    place, 0, budget,
                )?)),
                Operand::Move(place) => Ok(Operand::Move(ManuallyDropTakeProof::copy_place(
                    place, 0, budget,
                )?)),
                _ => Err("Cell scalar operand requires its exact evaluated temporary".to_string()),
            })
            .transpose()?;
        let reference = reference
            .map(|place| ManuallyDropTakeProof::copy_place(place, 0, budget))
            .transpose()?;
        budget.retain_bytes(std::mem::size_of::<Self>() + 2 * std::mem::size_of::<usize>())?;
        let proof = Arc::new(Self {
            closure: None,
            publication: publication.identity().clone(),
            site,
            operation,
            descriptor: descriptor.clone(),
            source: source.clone(),
            reference,
            value,
        });
        proof.validate(
            publication,
            CellOperands {
                operation,
                reference: proof.reference.as_ref(),
                value: proof.value.as_ref(),
                origin: Some(site),
            },
            budget,
        )?;
        Ok(proof)
    }
    pub(super) fn validate(
        &self,
        publication: &Arc<super::super::source_replay::CanonicalSourcePublication>,
        operands: CellOperands<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        let CellOperands {
            operation,
            reference,
            value,
            origin: current_origin,
        } = operands;
        budget.charge_work(1)?;
        if self.closure.is_some() {
            return Err("Cell closure receipt cannot impersonate a source publication".into());
        }
        if current_origin != Some(self.site)
            || self.publication != *publication.identity()
            || operation != self.operation
            || !Self::same_reference(reference, self.reference.as_ref(), budget)?
            || !Self::same_value(value, self.value.as_ref(), budget)?
        {
            return Err("Cell operation changed exact publication, current site, operation or evaluated operands".into());
        }
        let OriginalCell {
            operation: original,
            descriptor,
            source,
            receiver: receiver_id,
            value: value_id,
        } = Self::original(publication, self.site)?;
        if original != operation
            || !Arc::ptr_eq(descriptor, &self.descriptor)
            || !Arc::ptr_eq(source, &self.source)
            || receiver_id.is_some() != reference.is_some()
            || value_id.is_some() != value.is_some()
        {
            return Err(
                "Cell operation changed original descriptor, source or operand presence".into(),
            );
        }
        let catalog = publication.facts().world().canonical_catalog();
        source.validate_site(
            publication.facts().declaration(),
            operation.declaration(catalog),
            self.site,
            receiver_id,
            value_id,
        )?;
        descriptor.validate(catalog, budget)?;
        if !descriptor.is_concrete() {
            return Err("unsupported: Cell runtime requires a concrete primitive scalar".into());
        }
        publication
            .validate_source(publication.source(), publication.facts(), budget)
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}
impl Function {
    pub(super) fn cell_operation_type(
        &self,
        operands: CellOperands<'_>,
        proof: &CellOperationProof,
        facts: ir::FactsCtx<'_>,
        scratch: &mut TraitScratch,
    ) -> Result<Ty, String> {
        let CellOperands {
            operation,
            reference,
            value,
            ..
        } = operands;
        let authority = facts
            .validated()
            .ok_or("Cell operation requires exact validated source facts")?;
        let shape = self
            .lowering_proof
            .as_ref()
            .ok_or("Cell operation lacks exact complete lowering shape")?;
        if proof.closure.is_some() {
            let identity = shape
                .closure_origin
                .as_ref()
                .ok_or("Cell closure lost its original verified origin token")?;
            let context = if let Some(body) = self.body_substitution.as_ref() {
                let template = body.template();
                let original = template
                    .function()
                    .lowering_proof
                    .as_ref()
                    .and_then(|shape| shape.closure_origin.as_ref());
                if original != Some(identity) {
                    return Err("Cell closure changed original template token".into());
                }
                ClosureCellContext {
                    identity,
                    original_source: Some(template.source()),
                    original_facts: template.facts(),
                    substitution: Some(body),
                }
            } else {
                ClosureCellContext {
                    identity,
                    original_source: None,
                    original_facts: authority,
                    substitution: None,
                }
            };
            proof.validate_closure(context, authority, operands, authority.budget())?;
        } else {
            let publication = shape
                .original_publication
                .as_ref()
                .ok_or("unsupported: Cell operation requires original source publication")?;
            if self.body_substitution.is_some() {
                return Err(
                    "unsupported: Cell generated nonclosure operation receipt transport".into(),
                );
            }
            if !Arc::ptr_eq(authority.world(), publication.facts().world())
                || authority.declaration() != publication.facts().declaration()
            {
                return Err("Cell operation changed original validated world authority".into());
            }
            proof.validate(publication, operands, authority.budget())?;
        }
        let cell = Ty::Cell(proof.descriptor.clone());
        authority
            .validate_ty(&cell)
            .map_err(|error| error.to_string())?;
        if let Some(reference) = reference {
            let actual = self.place_ty_with_budget(reference, Some(authority.budget()))?;
            let Ty::Ref {
                kind: ir::BorrowKind::Shared,
                referent,
            } = actual
            else {
                return Err("Cell operation requires an exact shared Cell reference".into());
            };
            if !authority
                .world()
                .types_equal(
                    std::slice::from_ref(referent.as_ref()),
                    std::slice::from_ref(&cell),
                )
                .map_err(|error| error.to_string())?
            {
                return Err("Cell operation changed canonical receiver descriptor".into());
            }
        }
        if let Some(value) = value {
            self.require_operand_with_facts(value, proof.descriptor.argument(), facts, scratch)?;
        }
        Ok(match operation {
            CellOperation::New => cell,
            CellOperation::Get => proof.descriptor.argument().clone(),
            CellOperation::Set => Ty::Unit,
        })
    }
}

struct ClosureCellContext<'a> {
    identity: &'a super::super::source_replay::OriginalBodyId,
    original_source: Option<&'a ir::CheckedFunction>,
    original_facts: &'a super::super::traits::ValidatedFacts,
    substitution: Option<&'a Arc<ir::BodySubstitutionProof>>,
}
impl CellOperationProof {
    fn seal_closure(
        context: ClosureCellContext<'_>,
        current: &ir::CheckedFunction,
        facts: &super::super::traits::ValidatedFacts,
        site: ExprId,
        reference: Option<&Place>,
        value: Option<&Operand>,
        budget: &ir::PlanBudget,
    ) -> Result<Arc<Self>, String> {
        let original = context
            .original_source
            .ok_or("Cell closure mint lacks original checked source")?;
        let row = Self::original_in(original, site)?;
        let actual = Self::original_in(current, site)?;
        if row.operation != actual.operation
            || row.receiver != actual.receiver
            || row.value != actual.value
            || !Arc::ptr_eq(row.source, actual.source)
        {
            return Err(
                "Cell closure changed original source operation or argument occurrences".into(),
            );
        }
        if let Some(body) = context.substitution {
            body.validate(current, facts, budget)?;
            if !std::ptr::eq(body.template().source().as_ref(), original) {
                return Err("Cell closure changed exact original template source".into());
            }
        } else if !std::ptr::eq(current, original) {
            return Err("Cell closure mint changed exact original checked body".into());
        }
        if !facts
            .world()
            .types_equal(
                &[Ty::Cell(row.descriptor.clone())],
                &[Ty::Cell(actual.descriptor.clone())],
            )
            .map_err(|error| error.to_string())?
        {
            return Err("Cell closure changed exact canonical primitive descriptor".into());
        }
        let value = value
            .map(|value| match value {
                Operand::Copy(place) => Ok(Operand::Copy(ManuallyDropTakeProof::copy_place(
                    place, 0, budget,
                )?)),
                Operand::Move(place) => Ok(Operand::Move(ManuallyDropTakeProof::copy_place(
                    place, 0, budget,
                )?)),
                _ => Err("Cell closure value lacks evaluated temporary".to_string()),
            })
            .transpose()?;
        let reference = reference
            .map(|place| ManuallyDropTakeProof::copy_place(place, 0, budget))
            .transpose()?;
        budget.retain_bytes(std::mem::size_of::<Self>() + 2 * std::mem::size_of::<usize>())?;
        let proof = Arc::new(Self {
            publication: context.identity.clone(),
            site,
            operation: row.operation,
            descriptor: actual.descriptor.clone(),
            source: row.source.clone(),
            reference,
            value,
            closure: Some(ClosureCellProof {
                substitution: context.substitution.map(|body| body.identity().clone()),
                original_descriptor: row.descriptor.clone(),
                receiver: row.receiver,
                value: row.value,
            }),
        });
        proof.validate_closure(
            context,
            facts,
            CellOperands {
                operation: proof.operation,
                reference: proof.reference.as_ref(),
                value: proof.value.as_ref(),
                origin: Some(site),
            },
            budget,
        )?;
        Ok(proof)
    }
    fn validate_closure(
        &self,
        context: ClosureCellContext<'_>,
        facts: &super::super::traits::ValidatedFacts,
        operands: CellOperands<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        budget.charge_work(1)?;
        let original = self
            .closure
            .as_ref()
            .ok_or("Cell source publication receipt cannot impersonate closure authority")?;
        if self.publication != *context.identity
            || operands.origin != Some(self.site)
            || operands.operation != self.operation
            || original.substitution.as_ref() != context.substitution.map(|body| body.identity())
            || !Self::same_reference(operands.reference, self.reference.as_ref(), budget)?
            || !Self::same_value(operands.value, self.value.as_ref(), budget)?
        {
            return Err("Cell closure changed original token, exact current body, site or evaluated operands".into());
        }
        let (original_closure, original_mode) = context
            .original_facts
            .closure_lowering_identity()
            .ok_or("Cell closure lacks exact original provider authority")?;
        if facts.declaration() != original_closure.provider()
            || context.original_facts.declaration() != original_closure.provider()
            || !Arc::ptr_eq(facts.world(), context.original_facts.world())
        {
            return Err("Cell closure changed original provider or world".into());
        }
        if let Some(body) = context.substitution {
            if facts.phase() != super::super::traits::FactsPhase::Closed
                || !std::ptr::eq(body.template().facts().as_ref(), context.original_facts)
                || body.target().declaration != original_closure.provider()
                || !context
                    .original_source
                    .is_some_and(|source| std::ptr::eq(source, body.template().source().as_ref()))
            {
                return Err("Cell closed provider changed exact original closure template".into());
            }
        } else {
            let (closure, mode) = facts
                .closure_lowering_identity()
                .ok_or("Cell closure lacks canonical current provider authority")?;
            if mode != original_mode
                || !facts
                    .world()
                    .types_equal(&[Ty::Closure(closure)], &[Ty::Closure(original_closure)])
                    .map_err(|error| error.to_string())?
            {
                return Err(
                    "Cell original closure changed canonical source identity or mode".into(),
                );
            }
        }
        if let Some(source) = context.original_source {
            let row = Self::original_in(source, self.site)?;
            if row.operation != self.operation
                || !Arc::ptr_eq(row.descriptor, &original.original_descriptor)
                || !Arc::ptr_eq(row.source, &self.source)
                || row.receiver != original.receiver
                || row.value != original.value
            {
                return Err("Cell closure changed original retained source occurrence".into());
            }
        }
        let catalog = facts.world().canonical_catalog();
        self.source.validate(catalog, budget)?;
        self.source.validate_site(
            facts.declaration(),
            self.operation.declaration(catalog),
            self.site,
            original.receiver,
            original.value,
        )?;
        self.descriptor.validate(catalog, budget)?;
        if !self.descriptor.is_concrete()
            || !facts
                .world()
                .types_equal(
                    &[Ty::Cell(self.descriptor.clone())],
                    &[Ty::Cell(original.original_descriptor.clone())],
                )
                .map_err(|error| error.to_string())?
        {
            return Err("Cell closure changed canonical concrete scalar descriptor".into());
        }
        if original.receiver.is_some() != operands.reference.is_some()
            || original.value.is_some() != operands.value.is_some()
        {
            return Err("Cell closure changed exact operand presence".into());
        }
        if let Some(body) = context.substitution {
            let target = facts
                .target()
                .ok_or("Cell generated closure lacks exact current target")?;
            if target.declaration != body.target().declaration
                || !facts
                    .world()
                    .types_equal(&target.type_args, &body.target().type_args)
                    .map_err(|error| error.to_string())?
            {
                return Err("Cell generated closure changed ordered owner actuals".into());
            }
            let expected = match self.operation {
                CellOperation::New => Ty::Cell(self.descriptor.clone()),
                CellOperation::Get => self.descriptor.argument().clone(),
                CellOperation::Set => Ty::Unit,
            };
            if !facts
                .world()
                .types_equal(
                    std::slice::from_ref(body.expression_type(self.site)?),
                    std::slice::from_ref(&expected),
                )
                .map_err(|error| error.to_string())?
            {
                return Err(
                    "Cell generated closure changed exact original expression result mapping"
                        .into(),
                );
            }
        }
        Ok(())
    }
}
impl Lowerer<'_> {
    pub(super) fn seal_cell_operation(
        &self,
        site: ExprId,
        reference: Option<&Place>,
        value: Option<&Operand>,
        budget: &ir::PlanBudget,
    ) -> Result<Arc<CellOperationProof>, String> {
        if let Some(publication) = self.publication {
            return CellOperationProof::seal(publication, site, reference, value, budget);
        }
        let facts = self
            .facts
            .validated()
            .ok_or("Cell closure lowering requires validated facts")?;
        let context = if let Some(identity) = self.closure_origin {
            ClosureCellContext {
                identity,
                original_source: Some(self.source),
                original_facts: facts,
                substitution: None,
            }
        } else {
            let body = self.source.body_substitution.as_ref().ok_or(
                "unsupported: Cell operation lacks original publication or verified closure origin",
            )?;
            let template = body.template();
            let identity = template
                .function()
                .lowering_proof
                .as_ref()
                .and_then(|shape| shape.closure_origin.as_ref())
                .ok_or("unsupported: Cell generated operation lacks verified closure origin")?;
            ClosureCellContext {
                identity,
                original_source: Some(template.source()),
                original_facts: template.facts(),
                substitution: Some(body),
            }
        };
        CellOperationProof::seal_closure(
            context,
            self.source,
            facts,
            site,
            reference,
            value,
            budget,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::{frontend, source};
    #[test]
    fn scalar_cell_receipts_bind_original_owner_site_receiver_value_and_descriptor() {
        let budget = ir::PlanBudget::default();
        let prepared = source::prepare_text_with_prelude("cell-cfg.rs","use core::cell::Cell;fn f(){let a=Cell::new(1usize);let b=Cell::new(2usize);a.set(3usize);b.set(4usize);let x=a.get();}fn g(){let a=Cell::new(1usize);a.set(3usize);}fn main(){}",&source::SourceOptions::default()).unwrap();
        let parsed = frontend::parse_prepared(&prepared).unwrap();
        let program = frontend::resolve(&parsed).unwrap();
        let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let publish = |index| {
            let frontend::PublishedCallableBody::Source(publication) = plan
                .publish_callable_body(plan.function_decl(index).unwrap(), &budget)
                .unwrap()
            else {
                panic!("original publication required")
            };
            publication
        };
        let publication = publish(0);
        let foreign = publish(1);
        let function = lower_source_closed(&publication, &budget).unwrap();
        let facts = ir::FactsCtx::from_validated(publication.facts());
        function.validate_with_facts(facts).unwrap();
        function
            .validate_provenance(publication.facts(), &budget)
            .unwrap();
        let sets: Vec<_> = function
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .filter_map(|statement| {
                if let Statement::Assign {
                    value:
                        Rvalue::CellSet {
                            reference,
                            value,
                            proof,
                        },
                    ..
                } = &statement.value
                {
                    Some((reference, value, proof, statement.origin))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(sets.len(), 2);
        let (reference, value, proof, origin) = sets[0];
        let request = CellOperands {
            operation: CellOperation::Set,
            reference: Some(reference),
            value: Some(value),
            origin,
        };
        proof.validate(&publication, request, &budget).unwrap();
        assert!(proof.validate(&foreign, request, &budget).is_err());
        assert!(
            proof
                .validate(
                    &publication,
                    CellOperands {
                        origin: sets[1].3,
                        ..request
                    },
                    &budget
                )
                .is_err()
        );
        assert!(
            proof
                .validate(
                    &publication,
                    CellOperands {
                        reference: Some(sets[1].0),
                        ..request
                    },
                    &budget
                )
                .is_err()
        );
        assert!(
            proof
                .validate(
                    &publication,
                    CellOperands {
                        value: Some(sets[1].1),
                        ..request
                    },
                    &budget
                )
                .is_err()
        );
        assert!(
            proof
                .validate(
                    &publication,
                    CellOperands {
                        operation: CellOperation::Get,
                        ..request
                    },
                    &budget
                )
                .is_err()
        );
        let mut wrong = proof.as_ref().clone();
        wrong.descriptor = ir::CellTy::new(
            publication.facts().world().canonical_catalog(),
            Ty::Bool,
            &budget,
        )
        .unwrap();
        assert!(wrong.validate(&publication, request, &budget).is_err());
        for change in 0..4 {
            let mut forged = function.clone();
            let statement = forged
                .blocks
                .iter_mut()
                .flat_map(|block| &mut block.statements)
                .find(|statement| {
                    matches!(
                        &statement.value,
                        Statement::Assign {
                            value: Rvalue::CellSet { .. },
                            ..
                        }
                    )
                })
                .unwrap();
            match change {
                0 => statement.origin = sets[1].3,
                1 => {
                    let Statement::Assign {
                        value: Rvalue::CellSet { reference, .. },
                        ..
                    } = &mut statement.value
                    else {
                        unreachable!()
                    };
                    *reference = sets[1].0.clone();
                }
                2 => {
                    let Statement::Assign {
                        value: Rvalue::CellSet { value, .. },
                        ..
                    } = &mut statement.value
                    else {
                        unreachable!()
                    };
                    *value = sets[1].1.clone();
                }
                _ => forged.lowering_proof = None,
            }
            assert!(
                forged.validate_with_facts(facts).is_err(),
                "forged Cell effect {change}"
            );
            assert!(
                forged
                    .validate_provenance(publication.facts(), &budget)
                    .is_err(),
                "forged Cell provenance {change}"
            );
        }
        let denied = ir::PlanBudget::new(ir::PlanLimits {
            bytes: 0,
            ..Default::default()
        });
        let error = CellOperationProof::seal(
            &publication,
            proof.site,
            Some(reference),
            Some(value),
            &denied,
        )
        .unwrap_err();
        let first = denied.first_resource_failure().unwrap();
        assert_eq!(first.kind, ir::ResourceBudgetKind::Bytes);
        assert_eq!(first.limit, 0);
        assert!(first.requested > 0);
        assert_eq!(
            crate::compiler::traits::FactsError::from_message(&denied, &error).kind(),
            crate::compiler::traits::FactsErrorKind::Resource,
        );
        // A rejection latches its evidence, not every independent counter.
        // Even a zero-byte admission cannot clear the original byte failure.
        let _ = denied.retain_bytes(0);
        assert_eq!(denied.first_resource_failure(), Some(first));
        assert!(denied.retain_bytes(1).is_err());
        assert_eq!(denied.first_resource_failure(), Some(first));
    }
    #[test]
    fn scalar_cell_closure_receipts_bind_provider_token_site_and_operands() {
        let budget = ir::PlanBudget::default();
        let prepared = source::prepare_text_with_prelude("cell-closure-cfg.rs", "use core::cell::Cell;fn main(){let c=Cell::new(1usize);let write=||{c.set(2usize);c.set(3usize);};let read=||c.get();write();let n=read();}", &source::SourceOptions::default()).unwrap();
        let parsed = frontend::parse_prepared(&prepared).unwrap();
        let program = frontend::resolve(&parsed).unwrap();
        let mut plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let owner = plan.function_decl(0).unwrap();
        let constants = plan.prepare_body_constants(owner, &budget).unwrap();
        assert!(constants.direct().is_empty());
        let constants = plan
            .seal_body_constants(constants, Vec::new(), &budget)
            .unwrap();
        let (publication, closures) = plan
            .publish_callable_body_with_closures(owner, constants, &budget)
            .unwrap();
        let frontend::PublishedCallableBody::Source(publication) = publication else {
            panic!("original caller publication required")
        };
        let closure_target = |provider| {
            publication
                .source()
                .exprs
                .iter()
                .find_map(|expression| {
                    let ir::ExprKind::Call { target, .. } = &expression.kind else {
                        return None;
                    };
                    let resolved = match target {
                        ir::Callee::Direct(target) => target.clone(),
                        ir::Callee::TraitItem(call) => {
                            let call = publication
                                .facts()
                                .service()
                                .resolve_trait_call(call, publication.facts())
                                .unwrap();
                            let ir::Callee::Direct(target) = call.callee() else {
                                return None;
                            };
                            target.clone()
                        }
                        _ => return None,
                    };
                    (resolved.declaration == provider).then_some(resolved)
                })
                .expect("actual original caller dispatch to closure provider")
        };
        let mut lowered = Vec::new();
        for original in &closures {
            let facts = plan
                .trait_world()
                .unwrap()
                .closure_callable_facts_for(original.ty.clone(), ir::ClosureKind::Fn)
                .unwrap();
            let source = original.body_for(ir::ClosureKind::Fn).unwrap();
            let template = super::super::lower_closure_template(
                source,
                facts.clone(),
                original,
                ir::ClosureKind::Fn,
                &budget,
            )
            .unwrap();
            template
                .function()
                .validate_provenance(&facts, &budget)
                .unwrap();
            lowered.push((template, facts));
        }
        assert_eq!(lowered.len(), 2);
        let (template, facts) = lowered
            .iter()
            .find(|(template, _)| {
                template.function().blocks.iter().any(|block| {
                    block.statements.iter().any(|statement| {
                        matches!(
                            statement.value,
                            Statement::Assign {
                                value: Rvalue::CellSet { .. },
                                ..
                            }
                        )
                    })
                })
            })
            .unwrap();
        let original = template.function();
        for change in 0..4 {
            let mut forged = original.as_ref().clone();
            let statement = forged
                .blocks
                .iter_mut()
                .flat_map(|block| block.statements.iter_mut())
                .find(|statement| {
                    matches!(
                        statement.value,
                        Statement::Assign {
                            value: Rvalue::CellSet { .. },
                            ..
                        }
                    )
                })
                .unwrap();
            let Statement::Assign {
                value:
                    Rvalue::CellSet {
                        reference,
                        value,
                        proof,
                    },
                ..
            } = &mut statement.value
            else {
                unreachable!()
            };
            match change {
                0 => statement.origin = None,
                1 => {
                    let mut receipt = proof.as_ref().clone();
                    receipt.publication =
                        super::super::super::source_replay::OriginalBodyId::new(&budget).unwrap();
                    *proof = Arc::new(receipt);
                }
                2 => reference.local = original.return_local,
                _ => *value = Operand::Constant(Constant::Unit),
            }
            assert!(
                forged.validate_provenance(facts, &budget).is_err(),
                "closure receipt forgery {change}"
            );
        }
        let foreign = lowered
            .iter()
            .find(|(_, other)| other.declaration() != facts.declaration())
            .unwrap();
        assert!(original.validate_provenance(&foreign.1, &budget).is_err());
        let verified = crate::compiler::ownership::verify_template(template.clone()).unwrap();
        let target = closure_target(facts.declaration());
        assert_eq!(verified.env().total_arity(), target.type_args.len());
        let (actual_closure, actual_mode) = facts
            .world()
            .closure_adapter_environment(&target, publication.facts().env())
            .unwrap();
        assert_eq!(actual_closure.provider(), facts.declaration());
        assert_eq!(actual_mode, ir::ClosureKind::Fn);
        let closed = facts.world().instance_facts(&target).unwrap();
        assert!(closed.closure_lowering_identity().is_none());
        let instance =
            ir::instantiate_checked(&verified, &target.type_args, &closed, &mut plan, &budget)
                .unwrap();
        let generated = super::super::lower_with_facts(&instance, &closed, &budget).unwrap();
        generated.validate_provenance(&closed, &budget).unwrap();
        let other_instance =
            ir::instantiate_checked(&verified, &target.type_args, &closed, &mut plan, &budget)
                .unwrap();
        let other_body = other_instance.body_substitution.as_ref().unwrap();
        for change in 0..3 {
            let mut forged = generated.clone();
            match change {
                0 => {
                    let statement = forged
                        .blocks
                        .iter_mut()
                        .flat_map(|block| block.statements.iter_mut())
                        .find(|statement| {
                            matches!(
                                statement.value,
                                Statement::Assign {
                                    value: Rvalue::CellSet { .. },
                                    ..
                                }
                            )
                        })
                        .unwrap();
                    statement.origin = None;
                }
                1 => forged.body_substitution = Some(other_body.clone()),
                _ => {
                    let mut shape = forged.lowering_proof.as_ref().unwrap().as_ref().clone();
                    shape.closure_origin = Some(
                        super::super::super::source_replay::OriginalBodyId::new(&budget).unwrap(),
                    );
                    forged.lowering_proof = Some(Arc::new(shape));
                }
            }
            assert!(
                forged.validate_provenance(&closed, &budget).is_err(),
                "closed Cell receipt forgery {change}"
            );
        }
        let foreign_target = closure_target(foreign.1.declaration());
        let foreign_closed = facts.world().instance_facts(&foreign_target).unwrap();
        assert!(
            generated
                .validate_provenance(&foreign_closed, &budget)
                .is_err()
        );
    }
}
