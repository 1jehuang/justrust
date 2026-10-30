//! A generic trait method called on closures, a `Signal` and a `&str`, through a dependency's
//! `impl<T, U: Into<T>, F: Fn() -> U> IntoReactive<T> for F` and its impls for concrete types,
//! from a generic function of the dependency and from one of this crate: the shape of nsh-core's
//! `IntoReactive`, which an app's patches were refused on (ra-jit #224) with "call to
//! `into_reactive`, which has no body and no rustc symbol".

use reactive::{IntoReactive, Reactive, Signal, bind_value};

fn label<V: IntoReactive<String>>(value: V) -> String {
    value.into_reactive().get()
}

pub fn run() -> Vec<String> {
    let count = Signal::new(1u32);
    let name = Signal::new("hexuria".to_owned());
    let shown = {
        let count = count.clone();
        move || format!("count {}", count.get())
    };
    let greeting = {
        let name = name.clone();
        move || format!("hello {}", name.get())
    };
    let bound = bind_value(shown);
    // The closure's `&str` becomes the `String` the trait is asked for.
    let fixed = bind_value(|| "fixed");
    let wide: Reactive<u64> = {
        let count = count.clone();
        (move || count.get() * 10).into_reactive()
    };
    let mut seen = vec![bound.get(), fixed.get(), label(greeting), label(name.clone())];
    seen.push(label("literal"));
    count.set(2);
    name.set("again".to_owned());
    seen.push(bound.get());
    seen.push(wide.get().to_string());
    seen.push(bind_value(name).get());
    seen.push(label(String::from("owned")));
    seen
}
