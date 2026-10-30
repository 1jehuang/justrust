//! wasmtime's `HasData`: `fn(&mut T) -> D::Data<'_>` is `for<'a> fn(&'a mut T) -> D::Data<'a>`,
//! an alias that names the lifetime of the binder around it. Monomorphization normalized it
//! as if it stood alone, the solver panicked on the escaping bound variable, and the worker
//! went down with it, so nothing of the workspace was the daemon's.

use std::marker::PhantomData;

pub trait HasData {
    type Data<'a>;
}

pub struct HasSelf<T>(PhantomData<T>);

impl<T: 'static> HasData for HasSelf<T> {
    type Data<'a> = &'a mut T;
}

#[inline(never)]
fn with<T, D: HasData>(
    state: &mut T,
    get: fn(&mut T) -> D::Data<'_>,
    then: fn(D::Data<'_>) -> usize,
) -> usize {
    then(get(state))
}

fn whole(table: &mut Vec<u32>) -> &mut Vec<u32> {
    table
}

fn grown(table: &mut Vec<u32>) -> usize {
    table.push(3);
    table.len()
}

pub fn run() -> String {
    let mut table = vec![1, 2];
    let len = with::<Vec<u32>, HasSelf<Vec<u32>>>(&mut table, whole, grown);
    format!("{len} {table:?}")
}
