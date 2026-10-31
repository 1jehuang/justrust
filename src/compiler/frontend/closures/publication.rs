//! Correspondence between authored closure shapes and a finalized parent.
//! All retained fields are immutable tokens or pure typed payload receipts.
use super::check::{CheckedClosure, CheckedClosureBody};
use super::*;

#[derive(Clone, Debug)]
struct ModeShape {
    mode: ir::ClosureKind,
    body: ir::ClosureBodyShape,
}
#[derive(Clone, Debug)]
pub(super) struct CheckedClosureAuthority {
    seal: Arc<ClosureShapeSeal>,
    modes: Arc<[ModeShape]>,
}
impl CheckedClosureAuthority {
    /// Private to the parent checker implementation, after every mode is final.
    pub(super) fn capture(
        plan: &PlanCtx,
        closure: &Arc<ir::ClosureTy>,
        modes: &[CheckedClosureBody],
    ) -> Result<Arc<Self>, String> {
        let seal = closure
            .shape_seal()
            .ok_or("checked closure has no final shape seal")?
            .clone();
        let world = plan.trait_world()?;
        let mut shapes = Vec::new();
        for mode in modes {
            if shapes
                .iter()
                .any(|shape: &ModeShape| shape.mode == mode.mode)
            {
                return Err("checked closure has duplicate receiver modes".into());
            }
            let facts = world
                .closure_callable_facts_for(closure.clone(), mode.mode)
                .map_err(|e| e.into_legacy_diagnostic(&plan.budget))?;
            facts
                .validate_closure_body_source(closure, mode.mode, &mode.body)
                .map_err(|e| e.into_legacy_diagnostic(&plan.budget))?;
            cell::replay_checked_body(plan, closure.provider(), &mode.body, &plan.budget)?;
            let body = ir::ClosureBodyShape::capture(&mode.body, &facts, &plan.budget)?;
            plan.budget.retain_bytes(std::mem::size_of::<ModeShape>())?;
            shapes.push(ModeShape {
                mode: mode.mode,
                body,
            });
        }
        plan.budget.retain_bytes(std::mem::size_of::<Self>() + 32)?;
        Ok(Arc::new(Self {
            seal,
            modes: shapes.into(),
        }))
    }
    pub(super) fn validate(&self, plan: &PlanCtx, checked: &CheckedClosure) -> Result<(), String> {
        let seal = checked
            .ty
            .shape_seal()
            .ok_or("closure lost its parent-minted shape seal")?;
        if !Arc::ptr_eq(&self.seal, seal) || checked.modes.len() != self.modes.len() {
            return Err("closure changed its parent-minted shape or receiver mode set".into());
        }
        let world = plan.trait_world()?;
        for mode in [
            ir::ClosureKind::Fn,
            ir::ClosureKind::FnMut,
            ir::ClosureKind::FnOnce,
        ] {
            if !checked.ty.supports(mode) {
                if self.modes.iter().any(|shape| shape.mode == mode) {
                    return Err("closure receipt grants an unsupported receiver mode".into());
                }
                continue;
            }
            let shape = self
                .modes
                .iter()
                .find(|shape| shape.mode == mode)
                .ok_or("closure receipt omits a supported receiver mode")?;
            let body = checked.body_for(mode)?;
            let facts = world
                .closure_callable_facts_for(checked.ty.clone(), mode)
                .map_err(|e| e.into_legacy_diagnostic(&plan.budget))?;
            facts
                .validate_closure_body_source(&checked.ty, mode, &body)
                .map_err(|e| e.into_legacy_diagnostic(&plan.budget))?;
            cell::replay_checked_body(plan, checked.ty.provider(), &body, &plan.budget)?;
            shape.body.validate(&body, &facts, &plan.budget)?;
        }
        Ok(())
    }
}

/// Graph output payload. Unlike CheckedClosure this cannot retain an optional
/// instantiation/CFG/world capability through a CheckedFunction field.
#[derive(Clone, Debug)]
pub(in crate::compiler) struct PublishedClosure {
    pub(in crate::compiler) ty: Arc<ir::ClosureTy>,
    authority: Arc<CheckedClosureAuthority>,
}
impl PublishedClosure {
    pub(in crate::compiler) fn capture(
        plan: &PlanCtx,
        checked: &CheckedClosure,
    ) -> Result<Self, String> {
        checked.authority.validate(plan, checked)?;
        Ok(Self {
            ty: checked.ty.clone(),
            authority: checked.authority.clone(),
        })
    }

    fn restore(&self, plan: &PlanCtx) -> Result<CheckedClosure, String> {
        let seal = self
            .ty
            .shape_seal()
            .ok_or("published closure has no final shape")?;
        if !Arc::ptr_eq(seal, &self.authority.seal) {
            return Err("published closure changed its source shape receipt".into());
        }
        let world = plan.trait_world()?;
        let mut modes = Vec::new();
        for shape in self.authority.modes.iter() {
            if !self.ty.supports(shape.mode)
                || modes
                    .iter()
                    .any(|mode: &CheckedClosureBody| mode.mode == shape.mode)
            {
                return Err("published closure changed its authorized receiver mode set".into());
            }
            let facts = world
                .closure_callable_facts_for(self.ty.clone(), shape.mode)
                .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?;
            let body = shape.body.restore(&facts, &plan.budget)?;
            facts
                .validate_closure_body_source(&self.ty, shape.mode, &body)
                .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?;
            modes.push(CheckedClosureBody {
                mode: shape.mode,
                body: Arc::new(body),
            });
        }
        let body = modes
            .iter()
            .find(|body| body.mode == self.ty.kind())
            .ok_or("published closure omitted its canonical primary body")?
            .body
            .clone();
        let checked = CheckedClosure {
            ty: self.ty.clone(),
            body,
            modes: modes.into(),
            authority: self.authority.clone(),
        };
        checked.authority.validate(plan, &checked)?;
        Ok(checked)
    }
}

#[derive(Clone, Debug)]
pub(in crate::compiler) struct ClosurePublicationProof {
    finalized: sr::OriginalBodyId,
    authored: sr::OriginalBodyId,
    parent: ir::DeclId,
    seals: Arc<[Arc<ClosureShapeSeal>]>,
}
impl ClosurePublicationProof {
    pub(in crate::compiler) fn restore_closure(
        &self,
        plan: &PlanCtx,
        publication: &sr::CanonicalSourcePublication,
        published: &PublishedClosure,
        budget: &ir::PlanBudget,
    ) -> Result<CheckedClosure, String> {
        publication
            .validate_source(publication.source(), publication.facts(), budget)
            .map_err(|error| error.into_legacy_diagnostic(budget))?;
        let seal = published
            .ty
            .shape_seal()
            .ok_or("published closure has no source shape")?;
        if &self.finalized != publication.identity()
            || &self.authored != publication.authored_identity()
            || self.parent != publication.facts().declaration()
            || seal.authored_source() != Some(&self.authored)
            || !self
                .seals
                .iter()
                .any(|original| Arc::ptr_eq(original, seal))
        {
            return Err("closure reconstruction belongs to another finalized parent".into());
        }
        let checked = published.restore(plan)?;
        self.validate(plan, publication, &checked, budget)?;
        Ok(checked)
    }

    pub(in crate::compiler) fn capture(
        plan: &PlanCtx,
        publication: &sr::CanonicalSourcePublication,
        bodies: &[CheckedClosure],
        budget: &ir::PlanBudget,
    ) -> Result<Self, String> {
        plan.check_budget(budget)?;
        publication
            .validate_source(publication.source(), publication.facts(), budget)
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
        let parent = publication.facts().declaration();
        let mut seals = Vec::new();
        for checked in bodies {
            checked.authority.validate(plan, checked)?;
            let seal = checked
                .ty
                .shape_seal()
                .ok_or("published closure has no shape seal")?;
            if seal.authored_source() != Some(publication.authored_identity()) {
                return Err(
                    "closure source recipe is not the finalized parent's authored body".into(),
                );
            }
            // A source token alone cannot transplant a previously checked
            // environment into another finalized typed parent. Replay the
            // actual construction occurrence in its immediate lexical owner.
            let owner_body = if checked.ty.owner() == parent {
                publication.source().as_ref()
            } else {
                let mut lexical = None;
                for candidate in bodies {
                    budget.charge_work(1)?;
                    if candidate.ty.provider() == checked.ty.owner() {
                        lexical = Some(candidate.body.as_ref());
                        break;
                    }
                }
                lexical.ok_or("nested closure has no checked lexical owner body")?
            };
            let mut constructed = false;
            for expression in &owner_body.exprs {
                budget.charge_work(1)?;
                let (ir::Ty::Closure(actual), ir::ExprKind::Tuple(_)) =
                    (&expression.ty, &expression.kind)
                else {
                    continue;
                };
                if actual.provider() == checked.ty.provider()
                    && actual
                        .shape_seal()
                        .is_some_and(|actual_seal| Arc::ptr_eq(actual_seal, seal))
                {
                    constructed = true;
                    break;
                }
            }
            if !constructed {
                return Err(
                    "closure seal has no exact construction in its finalized lexical owner".into(),
                );
            }
            // Nested lexical providers must form an actual source-owner chain,
            // not an arbitrary declaration with compatible generic arguments.
            let mut owner = checked.ty.owner();
            for depth in 0..=ir::MAX_PATTERN_PROJECTION_DEPTH {
                budget.check_depth(depth)?;
                if owner == parent {
                    break;
                }
                owner = plan
                    .stored_closure_owner(owner)?
                    .ok_or("closure source is outside the finalized parent body")?
                    .0;
                if depth == ir::MAX_PATTERN_PROJECTION_DEPTH {
                    return Err("closure source owner chain exceeds depth limit".into());
                }
            }
            if plan.stored_closure_owner(checked.ty.provider())?
                != Some((checked.ty.owner(), checked.ty.site()))
            {
                return Err("published closure changed its exact provider, owner or site".into());
            }
            budget.charge_work(seals.len())?;
            if seals.iter().any(|prior| Arc::ptr_eq(prior, seal)) {
                return Err("parent publication repeats a finalized closure shape".into());
            }
            budget.retain_bytes(std::mem::size_of::<Arc<ClosureShapeSeal>>())?;
            seals.push(seal.clone());
        }
        Ok(Self {
            finalized: publication.identity().clone(),
            authored: publication.authored_identity().clone(),
            parent,
            seals: seals.into(),
        })
    }
    pub(in crate::compiler) fn validate(
        &self,
        plan: &PlanCtx,
        publication: &sr::CanonicalSourcePublication,
        checked: &CheckedClosure,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        plan.check_budget(budget)?;
        publication
            .validate_source(publication.source(), publication.facts(), budget)
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
        if &self.finalized != publication.identity()
            || &self.authored != publication.authored_identity()
            || self.parent != publication.facts().declaration()
        {
            return Err("closure correspondence belongs to another finalized publication".into());
        }
        let seal = checked
            .ty
            .shape_seal()
            .ok_or("closure correspondence has no finalized shape")?;
        budget.charge_work(self.seals.len())?;
        if seal.authored_source() != Some(&self.authored)
            || !self
                .seals
                .iter()
                .any(|original| Arc::ptr_eq(original, seal))
        {
            return Err(
                "closure shape is absent from its exact finalized parent publication".into(),
            );
        }
        checked.authority.validate(plan, checked)
    }
}
