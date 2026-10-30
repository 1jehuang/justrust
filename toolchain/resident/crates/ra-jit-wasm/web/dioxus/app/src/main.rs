//! A Dioxus app, compiled in this page by ra-jit and linked into the Dioxus rustc built.
//! Edit the `rsx!`, add a `#[component]`: it re-renders and keeps its state.

use dioxus::prelude::*;

fn main() {
    dioxus::launch(App);
}

fn App() -> Element {
    let mut count = use_signal(|| 0);
    rsx! {
        h1 { "High-Five counter: {count}" }
        button { onclick: move |_| count += 1, "Up high!" }
        button { onclick: move |_| count -= 1, "Down low!" }
    }
}
