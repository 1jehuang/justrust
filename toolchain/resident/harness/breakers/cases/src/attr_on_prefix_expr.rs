//! Found in utoipa-gen 5.5.0 (`Schema::to_tokens`; #598): a `filter` closure whose tail is
//! `#[cfg(not(feature = "config"))] !is_inline`. The attribute is the first child of the
//! prefix expression, where the operator was looked for, so the expression had no operator
//! and was lowered as a missing one (`IncompleteExpr`).

struct Item<'a>(&'a str, bool);

fn not_inlined<'a>(items: &[Item<'a>]) -> Vec<&'a str> {
    items
        .iter()
        .filter(|Item(_, is_inline)| {
            #[cfg(feature = "config")]
            {
                *is_inline
            }
            #[cfg(not(feature = "config"))]
            !is_inline
        })
        .map(|Item(name, _)| *name)
        .collect()
}

fn negated(x: i32) -> i32 {
    #[allow(unused_parens)]
    -(x)
}

pub fn run() -> (Vec<&'static str>, i32) {
    (not_inlined(&[Item("a", true), Item("b", false)]), negated(3))
}
