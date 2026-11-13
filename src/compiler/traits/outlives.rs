//! Declaration-owned static obligations used by trait implementation selection.
//! Erased reference lifetimes are never inferred from their runtime payload.
use super::*;

impl ValidatedFacts {
    pub(in crate::compiler) fn parameter_outlives_static(
        &self,
        parameter: ParamId,
    ) -> Result<bool> {
        self.world.type_outlives_static(
            &Ty::Param(parameter),
            self.env(),
            self.predicates().predicates(),
            0,
        )
    }
}

impl ValidatedTraitWorld {
    fn trait_has_static_self(&self, declaration: DeclId, depth: usize) -> Result<bool> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        let header = self.trait_header(declaration)?;
        if header.env.outlives_static(header.self_param) {
            return Ok(true);
        }
        for parent in header.supertraits.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            if matches!(&parent.self_ty, Ty::Param(parameter) if *parameter == header.self_param)
                && self.trait_has_static_self(parent.declaration, depth + 1)?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(super) fn type_outlives_static(
        &self,
        ty: &Ty,
        env: &GenericEnv,
        predicates: &[Predicate],
        depth: usize,
    ) -> Result<bool> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        if let Ty::Param(parameter) = ty {
            if env.outlives_static(*parameter) {
                return Ok(true);
            }
            // A declared superbound entails static for the exact subject.
            // Callers checking an impl pass only its written predicates, never
            // the impl's own membership, which would justify itself circularly.
            for predicate in predicates {
                charged(&self.budget, self.budget.charge_work(1))?;
                if let Predicate::Implements(goal) = predicate
                    && self.eq_ty(ty, &goal.self_ty, 0)?
                    && self.trait_has_static_self(goal.declaration, depth + 1)?
                {
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        let child = |ty: &Ty| self.type_outlives_static(ty, env, predicates, depth + 1);
        match ty {
            // The reference lifetime is carried separately from Ty. Its
            // absence here is not evidence for static, including under a raw
            // pointer, PhantomData, a function item, or a nominal field.
            Ty::Ref { .. } => Err(FactsError::unproved(
                &self.budget,
                "static lifetime obligation for a reference payload is not proved by the caller",
            )),
            // A projection whose family declares `'static` outlives it in
            // every implementation; each assignment is checked at its impl.
            Ty::AssociatedType(projection)
                if self.associated_header(projection.item)?.outlives_static =>
            {
                Ok(true)
            }
            Ty::Opaque(_) | Ty::AssociatedType(_) => Err(FactsError::unsupported(
                &self.budget,
                "unsupported: static trait selection requires a declared opaque or projection outlives proof",
            )),
            Ty::Dyn(object) => {
                for ty in object.types() {
                    if !child(ty)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Ty::RawPtr { pointee, .. } => child(pointee),
            Ty::PhantomData(value) => child(value.argument()),
            Ty::Cell(value) => child(value.argument()),
            Ty::ManuallyDrop(value) => child(value.argument()),
            Ty::FnPointer(value) => {
                for ty in value.types() {
                    if !child(ty)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Ty::FnItem(value) => {
                for ty in value.type_args.iter() {
                    if !child(ty)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Ty::Closure(value) => {
                for ty in value.captures() {
                    if !child(ty)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Ty::Tuple(fields) => {
                for ty in fields.iter() {
                    if !child(ty)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Ty::Array { element, .. } | Ty::Slice { element } => child(element),
            Ty::DependentArray { .. } => Ok(false),
            Ty::Struct(value) => {
                if value.is_back_edge() {
                    return Ok(false);
                }
                for ty in value.def().type_args.iter() {
                    if !child(ty)? {
                        return Ok(false);
                    }
                }
                let fields = value
                    .def()
                    .fields
                    .read(&self.budget)
                    .map_err(|error| FactsError::from_message(&self.budget, &error))?;
                for field in fields.iter() {
                    if !child(&field.ty)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Ty::Enum(value) => {
                if value.is_back_edge() {
                    return Ok(false);
                }
                for ty in value.def().type_args.iter() {
                    if !child(ty)? {
                        return Ok(false);
                    }
                }
                for variant in value.def().variants.iter() {
                    let fields = variant
                        .fields
                        .read(&self.budget)
                        .map_err(|error| FactsError::from_message(&self.budget, &error))?;
                    for field in fields.iter() {
                        if !child(&field.ty)? {
                            return Ok(false);
                        }
                    }
                }
                Ok(true)
            }
            Ty::Unit
            | Ty::Bool
            | Ty::Char
            | Ty::Int(_)
            | Ty::Float(_)
            | Ty::Str
            | Ty::Never
            | Ty::Const(_) => Ok(true),
            Ty::Param(_) => unreachable!(),
        }
    }

    pub(super) fn static_arguments_hold(
        &self,
        env: &GenericEnv,
        arguments: &[Ty],
        caller: &GenericEnv,
        predicates: &[Predicate],
    ) -> Result<bool> {
        if arguments.len() != env.total_arity() {
            return Err(FactsError::invalid(
                &self.budget,
                "static obligation argument arity changed",
            ));
        }
        for (parameter, actual) in env.ordered_params().zip(arguments) {
            charged(&self.budget, self.budget.charge_work(1))?;
            if env.outlives_static(parameter)
                && !self.type_outlives_static(actual, caller, predicates, 0)?
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub(super) fn require_trait_static_self(
        &self,
        goal: &TraitRef,
        env: &GenericEnv,
        predicates: &[Predicate],
    ) -> Result<()> {
        if self.trait_has_static_self(goal.declaration, 0)?
            && !self.type_outlives_static(&goal.self_ty, env, predicates, 0)?
        {
            return Err(FactsError::unproved(
                &self.budget,
                "trait requires a static Self type, but its source lifetime obligation is not proved",
            ));
        }
        Ok(())
    }
}
