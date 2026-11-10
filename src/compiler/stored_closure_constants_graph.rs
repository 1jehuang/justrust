//! Independent constant prerequisites for one original stored closure body.
use super::*;
use frontend::closures::constant_inputs::StoredConstantInputs;

impl crate::engine::Fact for StoredConstantInputs {}

pub(super) async fn produce(
    compiler: &CompilerProvider,
    cx: &Context,
    key: &Key,
) -> Result<(), Error> {
    let [index] = key.params.as_slice() else {
        return Err(Error::new(
            "stored constant inputs require one canonical callable coordinate",
        ));
    };
    let index = usize::try_from(*index)
        .map_err(|_| Error::new("stored constant provider coordinate overflow"))?;
    let types = Facts { cx }.types().await?;
    let provider = types.callable_decl(index).map_err(Error::new)?;
    if types.callable_index(provider).map_err(Error::new)? != index {
        return Err(Error::new("stored constant key changed canonical provider"));
    }
    let prepared = types
        .prepare_stored_constants(provider, &compiler.0.budget)
        .map_err(|error| source_fact_error(error, &compiler.0.budget))?;
    let (mut values, _value_storage) = compiler.scratch_vec(prepared.direct().len())?;
    let (mut children, _child_storage) = compiler.scratch_vec(prepared.stored().len())?;
    for declaration in prepared.direct() {
        values.push(
            cx.request(keys::item(CONST_VALUE, declaration.index as usize))
                .get(0),
        );
    }
    for child in prepared.stored() {
        let coordinate = types.callable_index(*child).map_err(Error::new)?;
        children.push(
            cx.request(keys::item(STORED_CLOSURE_CONSTANT_INPUTS, coordinate))
                .get(0),
        );
    }
    // All source-independent prerequisites are requested before any is awaited.
    let (mut resolved_values, _resolved_values_storage) = compiler.scratch_vec(values.len())?;
    let (mut resolved_children, _resolved_children_storage) =
        compiler.scratch_vec(children.len())?;
    for handle in values {
        resolved_values.push(cx.read_fact(handle).await?);
    }
    for handle in children {
        resolved_children.push(cx.read_fact(handle).await?);
    }
    let inputs = types
        .seal_stored_constants(
            provider,
            prepared,
            resolved_values,
            resolved_children,
            &compiler.0.budget,
        )
        .map_err(|error| source_fact_error(error, &compiler.0.budget))?;
    cx.publish_fact(0, inputs)
}
