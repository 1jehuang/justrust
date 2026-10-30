//! The list of a proc-macro crate's macros, which rustc keeps in the crate's metadata and the
//! loader of ra-jit's proc-macro server reads from the library ra-jit links instead
//! (docs/no-rustc.md, step J2). ra-jit adds it to the root file of a proc-macro crate it links
//! as a shared library, as a module of its own as it does a test harness: a static exported
//! by name with each macro's kind, name, helper attributes and the client the server expands
//! it through. A proc macro compiled to WebAssembly gets exports instead, which call the
//! macros through the `proc_macro` that stands in for the standard library's there
//! (`crates/ra-jit-macro-wasm`, [`shim`]).

use std::fmt::Write as _;

use ide_db::RootDatabase;

/// The module's name in the crate's root.
pub(crate) const MODULE: &str = "__ra_jit_proc_macros";

/// What the loader looks the list up by: a slice of the macros, as rustc's
/// `#[rustc_proc_macro_decls]` static is.
pub(crate) const SYMBOL: &str = "RA_JIT_PROC_MACROS";

/// What the module starts with, after the text of the root file.
pub(crate) const MARK: &str =
    "\n// ra-jit's list of the crate's proc macros, which is not in the file on disk.\n";

/// The module for the macros of `krate`, a proc-macro crate, for a WebAssembly module when
/// `wasm`.
pub(crate) fn module(db: &RootDatabase, krate: hir::Crate, wasm: bool) -> String {
    let edition = krate.edition(db);
    let mut macros = Vec::new();
    for def in krate.root_module(db).declarations(db) {
        let hir::ModuleDef::Function(function) = def else { continue };
        let Some(mac) = function.as_proc_macro(db) else { continue };
        let path = function.name(db).display(db, edition).to_string();
        // The kinds as ra-jit-expand numbers them.
        let (kind, helpers) = match mac.kind(db) {
            hir::MacroKind::Derive => {
                let id = hir_def::MacroId::from(mac);
                let helpers = hir_def::attrs::AttrFlags::derive_info(db, id)
                    .map(|it| it.helpers.iter().map(|it| it.as_str().to_owned()).collect())
                    .unwrap_or_default();
                (0, helpers)
            }
            hir::MacroKind::Attr => (1, Vec::new()),
            _ => (2, Vec::new()),
        };
        macros.push((kind, mac.name(db).as_str().to_owned(), helpers, path));
    }
    // rustc lists them as the crate declares them; the server finds them by name.
    macros.sort_by(|a, b| a.1.cmp(&b.1));
    if wasm { wasm_text(&macros) } else { text(&macros) }
}

/// The `proc_macro` crate `krate` links, a proc-macro crate compiled to WebAssembly, and the
/// text of its root file there: ra-jit's `proc_macro` of `crates/ra-jit-macro-wasm`, which
/// asks the host for what the standard library's asks the compiler's bridge.
pub(crate) fn shim(db: &RootDatabase, krate: hir::Crate) -> Option<(vfs::FileId, String)> {
    use ide_db::base_db::{CrateOrigin, LangCrateOrigin};
    let data = ide_db::base_db::Crate::from(krate).data(db);
    let proc_macro =
        data.dependencies.iter().map(|dep| dep.crate_id).find(|it| {
            matches!(it.data(db).origin, CrateOrigin::Lang(LangCrateOrigin::ProcMacro))
        })?;
    Some((hir::Crate::from(proc_macro).root_file(db), ra_jit_macro_wasm::GUEST.to_owned()))
}

/// What a WebAssembly module of proc macros exports (`crates/ra-jit-macro-wasm`): room for
/// the host to write an input in, the list of the macros as text, and the expansion of the
/// macro at an index of that list.
fn wasm_text(macros: &[(u32, String, Vec<String>, String)]) -> String {
    let mut list = String::new();
    for (kind, name, helpers, _) in macros {
        _ = writeln!(list, "{kind}\t{name}\t{}", helpers.join(","));
    }
    let bytes: Vec<String> = list.bytes().map(|it| it.to_string()).collect();
    let mut text = String::from(MARK);
    _ = write!(
        text,
        "#[doc(hidden)]\n#[allow(warnings, clippy::all)]\npub mod {MODULE} {{\n    \
         extern crate proc_macro;\n\n    \
         #[unsafe(no_mangle)]\n    \
         pub extern \"C\" fn {ALLOC}(len: usize) -> *mut u8 {{\n        \
         proc_macro::__blaze::alloc(len)\n    \
         }}\n\n    \
         #[unsafe(link_section = \"{SECTION}\")]\n    \
         static LISTED: [u8; {len}] = [{bytes}];\n\n    \
         #[unsafe(no_mangle)]\n    \
         pub extern \"C\" fn {LIST}() {{\n        \
         proc_macro::__blaze::list_bytes(&LISTED)\n    \
         }}\n\n    \
         #[unsafe(no_mangle)]\n    \
         pub extern \"C\" fn {EXPAND}(\n        \
         index: u32,\n        \
         attr: *const u8,\n        \
         attr_len: usize,\n        \
         input: *const u8,\n        \
         input_len: usize,\n    \
         ) {{\n        \
         match index {{\n",
        ALLOC = ra_jit_macro_wasm::exports::ALLOC,
        LIST = ra_jit_macro_wasm::exports::LIST,
        EXPAND = ra_jit_macro_wasm::exports::EXPAND,
        SECTION = ra_jit_macro_wasm::exports::LIST_SECTION,
        len = list.len(),
        bytes = bytes.join(", "),
    );
    for (index, (kind, _, _, path)) in macros.iter().enumerate() {
        let call = match kind {
            1 => format!(
                "proc_macro::__blaze::expand2(crate::{path}, attr, attr_len, input, input_len)"
            ),
            _ => format!("proc_macro::__blaze::expand1(crate::{path}, input, input_len)"),
        };
        _ = writeln!(text, "            {index} => {call},");
    }
    text.push_str("            _ => {}\n        }\n    }\n}\n");
    text
}

fn text(macros: &[(u32, String, Vec<String>, String)]) -> String {
    let mut text = String::from(MARK);
    _ = write!(
        text,
        "#[doc(hidden)]\n#[allow(warnings, clippy::all)]\npub mod {MODULE} {{\n    \
         extern crate proc_macro;\n\n    \
         use proc_macro::bridge::client::Client;\n\n    \
         #[repr(C)]\n    \
         pub struct Macro {{\n        \
         pub kind: u32,\n        \
         pub name: &'static str,\n        \
         pub helpers: &'static [&'static str],\n        \
         pub client: Client,\n    \
         }}\n\n    \
         #[unsafe(no_mangle)]\n    \
         pub static {SYMBOL}: &[Macro] = &[\n",
    );
    for (kind, name, helpers, path) in macros {
        let expand = if *kind == 1 { "expand2" } else { "expand1" };
        _ = writeln!(
            text,
            "        Macro {{ kind: {kind}, name: {name:?}, helpers: &{helpers:?}, \
             client: Client::{expand}(crate::{path}) }},"
        );
    }
    text.push_str("    ];\n}\n");
    text
}
