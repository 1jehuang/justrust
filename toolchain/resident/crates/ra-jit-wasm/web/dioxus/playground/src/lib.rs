//! The page, a playground: a picker of programs, the source of one in an editor, what it
//! shows when it runs, and how its last build went.
//!
//! This app does not compile anything. The compiler is a worker, and `playground.js` runs it
//! and the programs it builds: this app tells it of the program to show and of every edit,
//! through `window.ra-jit`, and hears how builds went through [`ra_jit_state`] and [`ra_jit_line`].
//!
//! It is also what a Dioxus program is linked into. This module is linked whole, with
//! everything exported, and the program ra-jit compiles calls its Dioxus and its `std`: the
//! program's `App` is a function in the table they share, which [`ra_jit_launch`] hands over
//! and this app renders as a component of its own.

use std::cell::RefCell;

use dioxus::{dioxus_core::Runtime, prelude::*};
use dioxus_code::{CodeTheme, Theme};
use dioxus_code_editor::{CodeEditor, Language};
use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures_util::StreamExt;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn error(text: &str);
}

/// Starts the page, called by `playground.js` once the module is loaded.
#[unsafe(no_mangle)]
pub extern "C" fn ra_jit_main() {
    std::panic::set_hook(Box::new(|info| error(&info.to_string())));
    dioxus::LaunchBuilder::web()
        .with_cfg(dioxus::web::Config::new().rootname("playground"))
        .launch(Playground);
}

/// The component of a Dioxus program, a function ra-jit compiled: its slot in the table.
#[derive(Clone, Copy, PartialEq)]
struct App(usize);

/// The program ra-jit compiled, in a scope of its own for its hooks.
#[component]
fn Program(app: App) -> Element {
    // SAFETY: `playground.js` gave `ra_jit_launch` the slot of the program's `App`, which
    // ra-jit compiled as a function of this signature.
    let app = unsafe { std::mem::transmute::<usize, fn() -> Element>(app.0) };
    app()
}

#[component]
fn Playground() -> Element {
    let mut demo = use_signal(Demo::of_address);
    let mut source = use_signal(|| demo.peek().source.to_owned());
    let mut dark = use_signal(stored_dark);
    let mut state = use_signal(|| "building".to_owned());
    let mut log = use_signal(Vec::<Line>::new);
    let mut program = use_signal(|| None::<App>);

    // The program to show, once what it is shown in is in the page.
    use_effect(move || {
        let demo = demo();
        set_address(demo.name);
        show(demo.name, demo.source);
    });
    use_future(move || async move {
        let Some(mut receiver) = SAID.with_borrow_mut(|(_, receiver)| receiver.take()) else {
            return;
        };
        while let Some(next) = receiver.next().await {
            match next {
                Said::State(kind) => state.set(kind),
                Said::App(app) => program.set(Some(app)),
                // The functions in the table are others now, which nothing here can tell.
                Said::Swapped => Runtime::current().force_all_dirty(),
                Said::Line(line) => {
                    let mut log = log.write();
                    let extra = (log.len() + 1).saturating_sub(LINES);
                    log.drain(..extra);
                    log.push(line);
                }
            }
        }
    });

    let shown = demo();
    let app = shown.output == Output::App;
    let theme = if dark() { Theme::TOKYO_NIGHT } else { Theme::GITHUB_LIGHT };
    rsx! {
        div { class: "playground", "data-theme": if dark() { "dark" } else { "light" },
            header {
                h1 { b { "ra-jit" } " playground" }
                p { "A Rust compiler running in this page as WebAssembly, emitting WebAssembly." }
                button {
                    r#type: "button",
                    onclick: move |_| {
                        let now = !dark();
                        store_dark(now);
                        dark.set(now);
                    },
                    if dark() { "light mode" } else { "dark mode" }
                }
            }
            nav {
                for (kind, title) in [(Output::Canvas, "canvas, no_std"), (Output::App, "dioxus, std")] {
                    h2 { "{title}" }
                    for it in DEMOS.iter().copied().filter(|it| it.output == kind) {
                        button {
                            r#type: "button",
                            class: if it.name == shown.name { "picked" },
                            onclick: move |_| {
                                if it.name != demo.peek().name {
                                    source.set(it.source.to_owned());
                                    demo.set(it);
                                }
                            },
                            b { "{it.title}" }
                            span { "{it.about}" }
                            small {
                                for tag in it.tags {
                                    i { "{tag}" }
                                }
                            }
                        }
                    }
                }
            }
            section { class: "pane",
                h2 { "src/main.rs" span { "compiled in this page on every edit" } }
                div { id: "editor",
                    // Another editor for another program: the text of one that was typed
                    // in is no longer what its `value` says. A key is only looked at in a list.
                    for name in [shown.name] {
                        CodeEditor {
                            key: "{name}",
                            value: source(),
                            language: Language::Rust,
                            theme: CodeTheme::fixed(theme),
                            line_numbers: true,
                            aria_label: "Rust source",
                            oninput: move |text: String| {
                                edit(&text);
                                source.set(text);
                            },
                        }
                    }
                }
            }
            // Both are always there: a program keeps what it has drawn into while another
            // is shown.
            section { class: "pane",
                h2 { if app { "app" } else { "canvas" } }
                // The canvas is as large as this is, whatever size it draws at.
                div { class: "stage",
                    canvas { id: "canvas", hidden: app }
                    div { id: "main", hidden: !app,
                        if let Some(app) = program() {
                            Program { app }
                        }
                    }
                }
            }
            section { class: "pane builds",
                h2 {
                    "builds"
                    span { id: "state", class: "{state}", "{state}" }
                    button { r#type: "button", onclick: move |_| log.write().clear(), "clear" }
                }
                div { id: "status",
                    for line in log.read().iter() {
                        div { class: "line {line.level}",
                            time { "{line.time}" }
                            p {
                                b { "{line.head}" }
                                for part in line.parts.iter() {
                                    span { "{part}" }
                                }
                                for note in line.notes.iter() {
                                    pre { "{note}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// What a program shows in.
#[derive(Clone, Copy, PartialEq)]
enum Output {
    /// It draws, with the functions `playground.js` gives it.
    Canvas,
    /// It is a Dioxus app, linked into this one.
    App,
}

/// A program of the picker.
#[derive(Clone, Copy, PartialEq)]
struct Demo {
    name: &'static str,
    title: &'static str,
    about: &'static str,
    /// What of the language it leans on.
    tags: &'static [&'static str],
    source: &'static str,
    output: Output,
}

const DEMOS: &[Demo] = &[
    Demo {
        name: "fountain",
        title: "Fountain",
        about: "Drops under gravity. Change the speed, the wind, the color.",
        tags: &["closures", "iterators", "consts", "f64"],
        source: include_str!("../../../demos/fountain.rs"),
        output: Output::Canvas,
    },
    Demo {
        name: "balls",
        title: "Bouncing balls",
        about: "Click to drop a ball. Change the gravity while they fall.",
        tags: &["structs", "traits", "statics", "f64"],
        source: include_str!("../../../demos/balls.rs"),
        output: Output::Canvas,
    },
    Demo {
        name: "life",
        title: "Game of life",
        about: "Conway's cells on a grid. Click to bring one to life.",
        tags: &["2D arrays", "iterators", "static mut"],
        source: include_str!("../../../demos/life.rs"),
        output: Output::Canvas,
    },
    Demo {
        name: "sort",
        title: "Sorting",
        about: "Three algorithms behind one trait object, a step a frame.",
        tags: &["dyn Trait", "slices", "vtables"],
        source: include_str!("../../../demos/sort.rs"),
        output: Output::Canvas,
    },
    Demo {
        name: "mandelbrot",
        title: "Mandelbrot set",
        about: "Complex numbers with operators of their own. Click to zoom.",
        tags: &["operators", "closures", "Option"],
        source: include_str!("../../../demos/mandelbrot.rs"),
        output: Output::Canvas,
    },
    Demo {
        name: "dioxus",
        title: "Dioxus counter",
        about: "Linked into the Dioxus and std rustc built. Edits keep its state.",
        tags: &["rsx!", "signals", "#[component]", "std"],
        source: include_str!("../../app/src/main.rs"),
        output: Output::App,
    },
];

impl Demo {
    fn named(name: &str) -> Option<Demo> {
        DEMOS.iter().copied().find(|it| it.name == name)
    }

    /// The one the address of the page names after its `#`, or the first.
    fn of_address() -> Demo {
        let hash = web_sys::window().and_then(|it| it.location().hash().ok()).unwrap_or_default();
        Demo::named(hash.trim_start_matches('#')).unwrap_or(DEMOS[0])
    }
}

fn set_address(name: &str) {
    if let Some(window) = web_sys::window() {
        _ = window.location().set_hash(name);
    }
}

const DARK_KEY: &str = "ra-jit-theme";

/// The theme picked before, or else the one of the system.
fn stored_dark() -> bool {
    let Some(window) = web_sys::window() else { return true };
    let stored = window.local_storage().ok().flatten().and_then(|it| it.get_item(DARK_KEY).ok());
    match stored.flatten().as_deref() {
        Some(name) => name != "light",
        None => !window
            .match_media("(prefers-color-scheme: light)")
            .ok()
            .flatten()
            .is_some_and(|it| it.matches()),
    }
}

fn store_dark(dark: bool) {
    let storage = web_sys::window().and_then(|it| it.local_storage().ok().flatten());
    if let Some(storage) = storage {
        _ = storage.set_item(DARK_KEY, if dark { "dark" } else { "light" });
    }
}

/// How many lines of builds are kept.
const LINES: usize = 200;

/// One thing a build said. `level` is `info`, `good` or `bad`.
#[derive(Clone, PartialEq)]
struct Line {
    time: String,
    level: String,
    head: String,
    /// Measurements, shown after the head.
    parts: Vec<String>,
    /// Longer text, shown under it: what did not compile, and why.
    notes: Vec<String>,
}

/// What `playground.js` says.
enum Said {
    /// `building`, `running` or `error`.
    State(String),
    Line(Line),
    /// The Dioxus program was built for the first time.
    App(App),
    /// It was built again, and functions of it were swapped.
    Swapped,
}

type Channel = (UnboundedSender<Said>, Option<UnboundedReceiver<Said>>);

thread_local! {
    /// What `playground.js` said, for the app to take.
    static SAID: RefCell<Channel> = RefCell::new({
        let (sender, receiver) = unbounded();
        (sender, Some(receiver))
    });
}

fn say(said: Said) {
    SAID.with_borrow(|(sender, _)| _ = sender.unbounded_send(said));
}

/// Called by `playground.js` with what the program shown is doing.
#[wasm_bindgen]
pub fn ra_jit_state(kind: String) {
    say(Said::State(kind));
}

/// Called by `playground.js` with a line about a build.
#[wasm_bindgen]
pub fn ra_jit_line(
    time: String,
    level: String,
    head: String,
    parts: Vec<String>,
    notes: Vec<String>,
) {
    say(Said::Line(Line { time, level, head, parts, notes }));
}

/// Called by `playground.js` with the `App` of the Dioxus program, once it is linked in.
#[unsafe(no_mangle)]
pub extern "C" fn ra_jit_launch(app: usize) {
    say(Said::App(App(app)));
}

/// Called by `playground.js` once it has swapped functions of the Dioxus program.
#[unsafe(no_mangle)]
pub extern "C" fn ra_jit_rerender() {
    say(Said::Swapped);
}

#[wasm_bindgen]
extern "C" {
    /// Builds and runs another program, in place of the one shown.
    #[wasm_bindgen(js_namespace = ["window", "ra-jit"])]
    fn show(name: &str, source: &str);

    /// Builds the program shown again, from this source.
    #[wasm_bindgen(js_namespace = ["window", "ra-jit"])]
    fn edit(source: &str);
}

// Linked whole, the page takes in what the C of the highlighting's parser never calls here
// but names: two things of libc.
#[unsafe(no_mangle)]
static stderr: usize = 0;

#[unsafe(no_mangle)]
extern "C" fn isprint(character: i32) -> i32 {
    (0x20..0x7f).contains(&character) as i32
}
