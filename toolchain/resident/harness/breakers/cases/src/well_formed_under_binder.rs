//! tests/ui/regions/regions-relate-bound-regions-on-closures-to-inference-variables.rs: a
//! struct with a bound between its lifetimes, named under a `for<'b>`. What makes the struct
//! well formed then speaks of `'b`, which means nothing outside the `for`; rustc passes over
//! such a type, and ra-jit, asking for the bound to hold, stopped with an index out of bounds.

struct Context<'tcx> {
    numbers: &'tcx Vec<isize>,
}

struct Walk<'a, 'tcx: 'a> {
    context: &'a Context<'tcx>,
}

impl<'a, 'tcx> Walk<'a, 'tcx> {
    fn first(&mut self) -> isize {
        self.with(Box::new(|this| this.context.numbers[0]))
    }

    fn with(&mut self, mut each: Box<dyn for<'b> FnMut(&mut Walk<'b, 'tcx>) -> isize>) -> isize {
        each(self)
    }
}

pub fn run() -> isize {
    let numbers = vec![22];
    let context = Context { numbers: &numbers };
    Walk { context: &context }.first()
}
