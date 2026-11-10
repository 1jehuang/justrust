//! Retain owning-object output context without forcing a concrete payload to dyn.
use super::*;

impl Checker<'_> {
    pub(super) fn owning_object_source_expectation<'a>(
        &self,
        source: &Expr,
        expected: &'a ir::Ty,
    ) -> Result<Expectation<'a>, String> {
        let Expr::MethodCall {
            receiver,
            name,
            type_args,
            args,
        } = source
        else {
            return Ok(Expectation::None);
        };
        let Some(receiver) = self.method_receiver_hint(receiver)? else {
            return Ok(Expectation::None);
        };
        // A known concrete result owns its sized payload, which must be
        // inferred before unsizing. A deferred result application instead
        // needs the exact target to select its authored trait output.
        if self
            .method_result_for_hint(&receiver, name, type_args, args)?
            .is_some()
        {
            Ok(Expectation::None)
        } else {
            // This is inference context, not a new coercion/reborrow site.
            // Final ordinary method selection still proves the application.
            Ok(Expectation::Inferred(expected))
        }
    }
}
