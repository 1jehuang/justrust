//! One item expansion owns an explicit inventory of every emitted callable body.
//! Generic/associated identity is retained by ordinary declarations, never
//! translated into free functions. Each body gets its own lexical product proof.
use super::*;

impl Parser<'_, '_> {
    pub(super) fn emitted_callable_body(
        &mut self,
        declaration: AstIdent,
        params: &[(BindingName, Type)],
    ) -> Result<(Block, Option<super::super::semantic_scope::ScopeId>), ParseError> {
        let Some(proof) = self.item_expansion else {
            return self.function_body(params);
        };
        let receiver = self.lexical.pending_receiver.take();
        let scopes = self.lexical.scopes.as_mut().ok_or_else(|| {
            native_adapter::transport_error(super::super::token_session::TokenError::InvalidHandle)
        })?;
        let frontier = scopes
            .generated_receiver_function_root(
                self.names,
                &*self.modules,
                &proof,
                declaration,
                receiver,
            )
            .map_err(scope_parse_error)?;
        let owner = scopes.function(frontier).map_err(scope_parse_error)?;
        let previous = self.lexical.function.replace(FunctionMacroState {
            frontier,
            driver: None,
        });
        let result = (|| {
            self.bind_receiver_lexical(receiver)?;
            for (name, _) in params {
                if let BindingName::Named(name) = name {
                    self.bind_lexical(*name)?;
                }
            }
            self.block()
        })();
        self.lexical.function = previous;
        Ok((result?, Some(owner)))
    }

    pub(super) fn seal_emitted_callables(
        &mut self,
        product: &ProgramDeclarations,
        proof: &super::super::semantic_scope::ItemExpansionAdmission,
    ) -> Result<(), ParseError> {
        let functions = product
            .functions
            .iter()
            .chain(
                product
                    .impls
                    .iter()
                    .flat_map(|owner| owner.items.iter().map(|item| &item.function)),
            )
            .chain(
                product
                    .traits
                    .iter()
                    .flat_map(|owner| owner.items.iter().filter_map(|item| item.default.as_ref())),
            );
        let scopes = self.lexical.scopes.as_mut().ok_or_else(|| {
            native_adapter::transport_error(super::super::token_session::TokenError::InvalidHandle)
        })?;
        let mut count = 0usize;
        for generated in functions {
            self.names
                .budget()
                .charge(super::super::expansion_budget::Counter::Work, 1)
                .map_err(|error| native_adapter::transport_error(error.into()))?;
            let declaration = *generated.name.segments().last().ok_or_else(|| {
                native_adapter::transport_error(
                    super::super::token_session::TokenError::InvalidHandle,
                )
            })?;
            let function = generated.lexical_scope.ok_or_else(|| {
                native_adapter::transport_error(
                    super::super::token_session::TokenError::InvalidHandle,
                )
            })?;
            scopes
                .seal_item_product(self.names, &*self.modules, proof, function, declaration)
                .map_err(scope_parse_error)?;
            count = count.checked_add(1).ok_or_else(|| {
                native_adapter::transport_error(
                    super::super::token_session::TokenError::IdExhausted,
                )
            })?;
        }
        scopes
            .complete_item_expansion(self.names, &*self.modules, proof, count)
            .map_err(scope_parse_error)
    }
}
