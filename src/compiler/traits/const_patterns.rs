//! Const coordinates in impl heads use the same ordered declaration slots as
//! types. Array extents infer a scalar argument, never a synthetic value type.
use super::*;

impl ValidatedTraitWorld {
    pub(super) fn match_const_extent(
        &self,
        parameter: ir::ConstParamId,
        actual: &Ty,
        env: &GenericEnv,
        slots: &mut [Option<Ty>],
        depth: usize,
    ) -> Result<bool> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(
            &self.budget,
            self.budget.charge_work(env.total_arity().saturating_add(1)),
        )?;
        let parameter = ir::ParamId {
            owner: parameter.owner,
            index: parameter.index,
        };
        if env.parameter_kind(parameter)
            != Some(ir::GenericParamKind::Const(ir::ConstType::Int(
                ir::IntTy::Usize,
            )))
        {
            return Err(FactsError::invalid(
                &self.budget,
                "impl array extent lacks a declaration-owned usize slot",
            ));
        }
        let argument = match actual {
            Ty::Array { len, .. } => {
                if *len > ir::MAX_ARRAY_TYPE_LEN {
                    return Err(FactsError::resource(
                        &self.budget,
                        "array length limit exceeded",
                    ));
                }
                Ty::Const(ir::ConstArgument::Integer {
                    ty: ir::IntTy::Usize,
                    bits: *len as u128,
                })
            }
            Ty::DependentArray { length, .. } => Ty::Param(ir::ParamId {
                owner: length.owner,
                index: length.index,
            }),
            _ => return Ok(false),
        };
        let index = env
            .ordered_params()
            .position(|candidate| candidate == parameter)
            .ok_or_else(|| FactsError::invalid(&self.budget, "missing impl const slot"))?;
        if slots.len() != env.total_arity() {
            return Err(FactsError::invalid(
                &self.budget,
                "impl inference slots do not match declaration arity",
            ));
        }
        match &slots[index] {
            Some(previous) => self.eq_ty(previous, &argument, depth + 1),
            None => {
                slots[index] = Some(argument);
                Ok(true)
            }
        }
    }
}
