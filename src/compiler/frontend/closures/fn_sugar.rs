//! `Fn(A, B) -> R` parenthesized callable sugar: `Fn<(A, B), Output = R>`.
use super::*;

impl Parser<'_, '_> {
    pub(in crate::compiler::frontend) fn callable_sugar_arguments(
        &mut self,
        token_site: super::super::super::expansion_span::SpanId,
    ) -> Result<(Vec<Type>, Vec<AssociatedTypeBinding>), ParseError> {
        self.expect("(")?;
        let mut inputs = Vec::new();
        while !self.is(")") {
            if inputs.len() >= 32 {
                return Err(self.limit("callable sugar argument limit 32"));
            }
            self.reserve_parser_items(&mut inputs, 1)?;
            inputs.push(self.ty()?);
            if !self.eat(",") {
                break;
            }
        }
        self.expect(")")?;
        let output = if self.eat("->") {
            self.ty()?
        } else {
            Type::Unit
        };
        let args = if inputs.is_empty() {
            Type::Unit
        } else {
            Type::Tuple(inputs)
        };
        let site = panic_callsite(self.names.session(), token_site)
            .map_err(native_adapter::transport_error)?;
        let name = self
            .names
            .intern_root("Output", site)
            .map_err(native_adapter::name_error)?;
        Ok((
            vec![args],
            vec![AssociatedTypeBinding {
                name,
                value: output,
            }],
        ))
    }
}
