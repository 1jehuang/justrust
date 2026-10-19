//! Linux x86-64 System V C interoperability. Private carriers are never C layout.
mod bridge;
pub mod classify;
pub mod elf;
pub mod layout;
pub use bridge::{export, import};

pub(crate) fn attach_export(
    object: &mut crate::compiler::backend::Object,
    wrapper: crate::compiler::backend::Object,
    symbol: String,
) -> Result<(), String> {
    if symbol == "main" || symbol == "_start" || symbol.starts_with("__jr_") {
        return Err("unsupported: reserved native export symbol".into());
    }
    let offset = object.code.len();
    object.exports.push((offset, symbol));
    object.calls.extend(
        wrapper
            .calls
            .into_iter()
            .map(|(at, name)| (at + offset, name)),
    );
    object.strings.extend(
        wrapper
            .strings
            .into_iter()
            .map(|(at, bytes)| (at + offset, bytes)),
    );
    object.vtables.extend(
        wrapper
            .vtables
            .into_iter()
            .map(|(at, value)| (at + offset, value)),
    );
    object.code.extend(wrapper.code);
    Ok(())
}

/// Immutable, validated transport description for one closed C declaration.
#[derive(Clone, Debug)]
pub struct Signature {
    pub params: Vec<layout::Layout>,
    pub result: layout::Layout,
    pub plan: classify::CallPlan,
}
impl Signature {
    pub fn new(params: Vec<layout::Layout>, result: layout::Layout) -> Result<Self, String> {
        let plan = classify::plan(&params, &result)?;
        Ok(Self {
            params,
            result,
            plan,
        })
    }
}
