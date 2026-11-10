//! Declaration-owned static requirements for implementation selection.
use super::*;

impl ValidatedTraitWorld {
    fn type_outlives_static(
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
}
