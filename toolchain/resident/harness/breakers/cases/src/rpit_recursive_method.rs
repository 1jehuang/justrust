//! Found in utoipa-gen 5.5.0 (`TypeTree::convert_types`, `compose_generics`; #598): a method
//! called on the return-position `impl Trait` of a recursive call, inside the defining
//! function, went unresolved. There the opaque type's hidden type is still an inference
//! variable, and method probing lost its link to the opaque, so the item bounds that give
//! `Iterator::collect` were never looked at.

fn doubled(depth: u32) -> Result<impl Iterator<Item = u32>, String> {
    let inner: Vec<u32> = if depth == 0 {
        Vec::new()
    } else {
        match doubled(depth - 1) {
            Ok(values) => values.collect(),
            Err(error) => return Err(error),
        }
    };
    Ok(inner.into_iter().chain(std::iter::once(depth * 2)))
}

fn summed(depth: u32) -> impl Iterator<Item = u32> + Clone {
    let below: u32 = if depth == 0 { 0 } else { summed(depth - 1).clone().sum() };
    std::iter::once(below + depth)
}

pub fn run() -> (Vec<u32>, Vec<u32>) {
    (doubled(3).unwrap().collect(), summed(4).collect())
}
