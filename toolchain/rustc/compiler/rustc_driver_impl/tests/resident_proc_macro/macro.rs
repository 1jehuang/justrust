extern crate proc_macro;

use proc_macro::TokenStream;
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

static GLOBAL: AtomicUsize = AtomicUsize::new(0);
thread_local! { static LOCAL: Cell<usize> = const { Cell::new(0) }; }

#[proc_macro]
pub fn counter(_input: TokenStream) -> TokenStream {
    let global = GLOBAL.fetch_add(1, Ordering::SeqCst) + 1;
    let local = LOCAL.with(|value| {
        value.set(value.get() + 1);
        value.get()
    });
    format!("({global}usize, {local}usize)").parse().unwrap()
}
