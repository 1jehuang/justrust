//! Source const arguments retain contextual integer inference until the
//! declaration-owned const slot supplies its scalar type.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceConstArgument {
    Closed(ir::ConstArgument),
    Integer {
        magnitude: u128,
        suffix: Option<ir::IntTy>,
        negative: bool,
    },
}

impl From<ir::ConstArgument> for SourceConstArgument {
    fn from(value: ir::ConstArgument) -> Self {
        Self::Closed(value)
    }
}

impl SourceConstArgument {
    pub(super) fn resolve(&self, expected: ir::ConstType) -> Result<ir::ConstArgument, String> {
        let value = match self {
            Self::Closed(value) => {
                value.closed_key()?;
                if value.closed_type() != Some(expected) {
                    return Err("E0308: const argument has the wrong scalar type".into());
                }
                value.clone()
            }
            Self::Integer {
                magnitude,
                suffix,
                negative,
            } => {
                let ir::ConstType::Int(ty) = expected else {
                    return Err(
                        "E0308: integer const argument requires an integer parameter".into(),
                    );
                };
                if suffix.is_some_and(|suffix| suffix != ty) {
                    return Err("E0308: const argument suffix does not match its parameter".into());
                }
                let literal = IntegerLiteral {
                    magnitude: *magnitude,
                    suffix: *suffix,
                };
                let value = resolve_integer(&literal, *negative, Some(&ir::Ty::Int(ty)))?;
                ir::ConstArgument::Integer {
                    ty: value.ty,
                    bits: value.bits,
                }
            }
        };
        value.closed_key()?;
        Ok(value)
    }
}

impl PlanCtx {
    pub(super) fn source_generic_argument(
        &self,
        argument: &Type,
        expected: ir::GenericParamKind,
        bindings: &HashMap<GenericBindingKey, ir::Ty>,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<ir::Ty, String> {
        budget.charge_work(1)?;
        let ir::GenericParamKind::Const(expected) = expected else {
            if matches!(argument, Type::ConstArgument(_)) {
                return Err("E0747: constant provided when a type was expected".into());
            }
            if let Type::Param(name) = argument {
                let key = self.registry.parameter_key(*name)?;
                if let Some(ir::Ty::Param(parameter)) = bindings.get(&key) {
                    let kind = match caller {
                        ir::FactsCtx::Universal(env) => env.parameter_kind(*parameter),
                        ir::FactsCtx::Validated(facts) => facts.env().parameter_kind(*parameter),
                        ir::FactsCtx::Concrete => None,
                    };
                    if matches!(kind, Some(ir::GenericParamKind::Const(_))) {
                        return Err("E0747: constant provided when a type was expected".into());
                    }
                }
            }
            return self.source_ty(argument, bindings, caller, budget);
        };
        match argument {
            Type::ConstArgument(value) => Ok(ir::Ty::Const(value.resolve(expected)?)),
            Type::Param(name) => {
                let key = self.registry.parameter_key(*name)?;
                let Some(ir::Ty::Param(parameter)) = bindings.get(&key) else {
                    return Err("E0747: type provided when a constant was expected".into());
                };
                let kind = match caller {
                    ir::FactsCtx::Universal(env) => env.parameter_kind(*parameter),
                    ir::FactsCtx::Validated(facts) => facts.env().parameter_kind(*parameter),
                    ir::FactsCtx::Concrete => None,
                };
                match kind {
                    Some(ir::GenericParamKind::Const(actual)) if actual == expected => {
                        Ok(ir::Ty::Param(*parameter))
                    }
                    Some(ir::GenericParamKind::Const(_)) => {
                        Err("E0308: forwarded const argument has the wrong scalar type".into())
                    }
                    _ => Err("E0747: type provided when a constant was expected".into()),
                }
            }
            _ => Err("E0747: type provided when a constant was expected".into()),
        }
    }
}

impl Parser<'_, '_> {
    pub(super) fn const_generic_argument(
        &mut self,
    ) -> Result<Option<SourceConstArgument>, ParseError> {
        let starts = self.is("true")
            || self.is("false")
            || self.token().is_char()
            || self.is("{")
            || self.is("-")
            || self
                .token()
                .text
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_digit);
        if !starts {
            return Ok(None);
        }
        let braced = self.eat("{");
        let value = if self.eat("true") {
            SourceConstArgument::Closed(ir::ConstArgument::Bool(true))
        } else if self.eat("false") {
            SourceConstArgument::Closed(ir::ConstArgument::Bool(false))
        } else if self.token().is_char() {
            let value = parse_char_literal(self.token().text).map_err(|error| self.error(error))?;
            self.cursor.bump();
            SourceConstArgument::Closed(ir::ConstArgument::Char(value))
        } else {
            let negative = self.eat("-");
            if !self
                .token()
                .text
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_digit)
            {
                return Err(self.unsupported(
                    "const generic expressions other than scalar literals are unsupported",
                ));
            }
            let literal =
                parse_integer_literal(self.token().text).map_err(|error| self.error(&error))?;
            self.cursor.bump();
            SourceConstArgument::Integer {
                magnitude: literal.magnitude,
                suffix: literal.suffix,
                negative,
            }
        };
        if braced {
            if !self.eat("}") {
                return Err(self.unsupported(
                    "const generic expressions other than scalar literals are unsupported",
                ));
            }
        } else if !self.is(",") && !self.token().text.starts_with('>') {
            return Err(
                self.unsupported("const generic expressions require a supported scalar literal")
            );
        }
        Ok(Some(value))
    }
}
