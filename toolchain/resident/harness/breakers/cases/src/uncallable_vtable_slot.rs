//! tests/ui/where-clauses/obligation-error-propagation.rs: a trait method with a `where`
//! clause of its own that the type does not meet can never be called, and rustc leaves its
//! slot in the vtable empty. ra-jit compiled the method for the slot, found no `partial_cmp`
//! to call and refused the function that made the trait object.

#[inline(never)]
fn name_through_object() -> &'static str {
    struct Unordered;

    trait Compare<T> {
        fn smaller(&self, value: &Option<T>) -> bool
        where
            Option<T>: Ord,
        {
            *value < *value
        }

        fn name(&self) -> &'static str {
            "compare"
        }
    }

    impl Compare<Unordered> for () {}

    let object = &() as &dyn Compare<Unordered>;
    object.name()
}

pub fn run() -> &'static str {
    name_through_object()
}
