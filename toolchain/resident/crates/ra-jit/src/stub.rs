//! ra-jit as cargo's `RUSTC_WORKSPACE_WRAPPER`: rustc builds the crates of the registry, and of
//! the workspace only what runs during the build. A library or program of the workspace is
//! built from a few lines that name the crates it depends on, so that rustc links the
//! program against them as it would have, and the code is ra-jit's to compile.
//!
//! What runs during the build is told from the rest by `--target`, which cargo passes for
//! the program's crates alone when it is given one itself.

use std::{ffi::OsString, path::Path, process::Command};

use anyhow::Context as _;

/// The flags of rustc that cargo passes with their value as the next argument.
pub(crate) const VALUED: &[&str] = &[
    "-C",
    "-L",
    "-Z",
    "-l",
    "-A",
    "-W",
    "-D",
    "-F",
    "--cfg",
    "--check-cfg",
    "--extern",
    "--out-dir",
    "--target",
    "--crate-name",
    "--crate-type",
    "--cap-lints",
    "--emit",
    "--edition",
    "--error-format",
    "--json",
    "--color",
    "--diagnostic-width",
    "--remap-path-prefix",
];

/// A `main` for cargo's link of a stand-in program to find. It is weak and of no code: the
/// link of what runs has ra-jit's.
const WEAK_MAIN: &str = r#"#[cfg(not(target_vendor = "apple"))]
core::arch::global_asm!(".weak main", "main:");
#[cfg(target_vendor = "apple")]
core::arch::global_asm!(".globl _main", ".weak_definition _main", "_main:");
"#;

/// Runs rustc as cargo asked, or on what stands in for a crate of the workspace.
pub(crate) fn command(mut args: impl Iterator<Item = OsString>) -> anyhow::Result<()> {
    let rustc = args.next().context("cargo names rustc first")?;
    let mut args: Vec<OsString> = args.collect();
    if let Some((source, text)) = stand_in(&args) {
        let out_dir = valued(&args, "--out-dir").context("cargo names where the output goes")?;
        let name = valued(&args, "--crate-name").unwrap_or_default();
        let extra = args
            .iter()
            .filter_map(|it| it.to_str()?.strip_prefix("extra-filename="))
            .next()
            .unwrap_or_default();
        let path = Path::new(out_dir).join(format!("{name}{extra}.ra-jit.rs"));
        std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
        args[source] = path.into();
    }
    run(Command::new(rustc).args(args))
}

/// Becomes rustc.
#[cfg(unix)]
fn run(rustc: &mut Command) -> anyhow::Result<()> {
    use std::os::unix::process::CommandExt as _;
    Err(rustc.exec().into())
}

/// Windows has no process become another: rustc runs below this one, which ends as it did.
#[cfg(windows)]
fn run(rustc: &mut Command) -> anyhow::Result<()> {
    let status = rustc.status()?;
    std::process::exit(status.code().unwrap_or(1))
}

fn valued<'a>(args: &'a [OsString], flag: &str) -> Option<&'a str> {
    let at = args.iter().position(|it| it == flag)?;
    args.get(at + 1)?.to_str()
}

/// Where the source of the crate is among the arguments and what to build in its place, for
/// a library or a program of the workspace that is built for the target.
fn stand_in(args: &[OsString]) -> Option<(usize, String)> {
    let args: Vec<&str> = args.iter().map(|it| it.to_str()).collect::<Option<_>>()?;
    valued_in(&args, "--target")?;
    let program = is_program(&args)?;
    if args.contains(&"--test") || valued_in(&args, "--crate-name")?.starts_with("build_script_") {
        return None;
    }
    let source = (0..args.len())
        .find(|&at| args[at].ends_with(".rs") && !(at > 0 && VALUED.contains(&args[at - 1])))?;
    Some((source, text_of(&args, program, Path::new(args[source]))))
}

/// Whether rustc is asked for a program and nothing else. `None` when it is asked for
/// neither that nor a Rust library alone: nothing stands in for the crate then.
pub(crate) fn is_program<S: AsRef<str>>(args: &[S]) -> Option<bool> {
    let kinds = args.windows(2).filter(|it| it[0].as_ref() == "--crate-type");
    match *kinds.map(|it| it[1].as_ref()).collect::<Vec<_>>() {
        ["bin"] => Some(true),
        ["lib" | "rlib"] => Some(false),
        _ => None,
    }
}

/// How a module of [`FORWARDING_ALLOCATOR`] starts.
const FORWARDING_MODULE: &str = "mod __ra_jit_global_allocator_";

/// A `#[global_allocator]` that calls the functions rust-analyzer's expansion of the crate's
/// own defines, which ra-jit compiles: rustc's allocator shim calls the allocator of the stand-in.
/// `{cfg}` is the `#[cfg]`s of the crate's, `{n}` tells one from another.
const FORWARDING_ALLOCATOR: &str = r#"{cfg}
mod __ra_jit_global_allocator_{n} {
    extern crate core;
    use core::alloc::{GlobalAlloc, Layout};
    struct Allocator;
    unsafe extern "C" {
        fn __ra_jit_global_alloc(size: usize, align: usize) -> *mut u8;
        fn __ra_jit_global_dealloc(ptr: *mut u8, size: usize, align: usize);
        fn __ra_jit_global_realloc(ptr: *mut u8, size: usize, align: usize, new_size: usize) -> *mut u8;
        fn __ra_jit_global_alloc_zeroed(size: usize, align: usize) -> *mut u8;
    }
    unsafe impl GlobalAlloc for Allocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            unsafe { __ra_jit_global_alloc(layout.size(), layout.align()) }
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            unsafe { __ra_jit_global_dealloc(ptr, layout.size(), layout.align()) }
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            unsafe { __ra_jit_global_realloc(ptr, layout.size(), layout.align(), new_size) }
        }
        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            unsafe { __ra_jit_global_alloc_zeroed(layout.size(), layout.align()) }
        }
    }
    #[global_allocator]
    static ALLOCATOR: Allocator = Allocator;
{weak}}
"#;

/// The functions of ra-jit's that the forwarding allocator calls.
pub(crate) const ALLOCATOR_FUNCTIONS: &[&str] = &[
    "__ra_jit_global_alloc",
    "__ra_jit_global_dealloc",
    "__ra_jit_global_realloc",
    "__ra_jit_global_alloc_zeroed",
];

/// Weak definitions of `names` for cargo's link of the stand-in to find, as [`WEAK_MAIN`];
/// these trap, as ra-jit's are the ones that run.
fn weak_traps(names: &[&str]) -> String {
    let asm = |apple: bool, trap: Option<&str>| {
        let mut lines = Vec::new();
        for name in names {
            if apple {
                lines.extend([
                    format!(".globl _{name}"),
                    format!(".weak_definition _{name}"),
                    format!("_{name}:"),
                ]);
            } else {
                lines.extend([format!(".weak {name}"), format!("{name}:")]);
            }
        }
        lines.extend(trap.map(str::to_owned));
        let lines: Vec<String> = lines.iter().map(|it| format!("{it:?}")).collect();
        format!("core::arch::global_asm!({});\n", lines.join(", "))
    };
    let mut text = String::new();
    for (apple, vendor) in
        [(true, "target_vendor = \"apple\""), (false, "not(target_vendor = \"apple\")")]
    {
        for (arch, trap) in
            [("target_arch = \"aarch64\"", "brk #1"), ("target_arch = \"x86_64\"", "ud2")]
        {
            text.push_str(&format!(
                "    #[cfg(all({vendor}, {arch}))]\n    {}",
                asm(apple, Some(trap))
            ));
        }
        let other = "not(any(target_arch = \"aarch64\", target_arch = \"x86_64\"))";
        text.push_str(&format!("    #[cfg(all({vendor}, {other}))]\n    {}", asm(apple, None)));
    }
    text
}

/// The source that stands in for the crate rustc is given `args` for: it names the
/// libraries the crate is linked against and has no code, but for a `#[global_allocator]` of
/// the crate's `source`, which has to be one of the stand-in's for rustc to call it.
pub(crate) fn text_of<S: AsRef<str>>(args: &[S], program: bool, source: &Path) -> String {
    let mut text = String::from("#![allow(warnings)]\n");
    if program {
        text.push_str("#![no_main]\n");
    }
    for (n, cfg) in global_allocators(source).iter().enumerate() {
        let forwarding = FORWARDING_ALLOCATOR.replace("{cfg}", cfg).replace("{n}", &n.to_string());
        text.push_str(&forwarding.replace("{weak}", &weak_traps(ALLOCATOR_FUNCTIONS)));
    }
    // Cargo can name a crate more than once (nightly's names its `.rlib` and its `.rmeta`),
    // and a second `extern crate` of a name is an error.
    let mut named = std::collections::HashSet::new();
    for pair in args.windows(2).filter(|it| it[0].as_ref() == "--extern") {
        let Some((name, path)) = pair[1].as_ref().split_once('=') else { continue };
        let name = name.rsplit_once(':').map_or(name, |it| it.1);
        // A proc macro is not linked into the program.
        if (path.ends_with(".rlib") || path.ends_with(".rmeta")) && named.insert(name) {
            text.push_str(&format!("extern crate {name};\n"));
        }
    }
    if program {
        text.push_str(WEAK_MAIN);
    }
    text
}

/// The `#[cfg]`s of each `#[global_allocator]` in the files of the crate whose root is `root`,
/// found through its `mod name;` items, with those of the modules it is in, inline or not. Read
/// as text, not parsed: a `#[path]`, a module written by a macro, the files of a `mod name;` in
/// an inline module and `cfg_attr` aren't seen.
fn global_allocators(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut files = vec![(root.to_path_buf(), true, String::new())];
    let mut seen = std::collections::HashSet::new();
    while let Some((file, owns_dir, outer)) = files.pop() {
        if !seen.insert(file.clone()) || seen.len() > 10_000 {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&file) else { continue };
        let kept = blanked(&text, false);
        let code = blanked(&text, true);
        // A stand-in, which a second wrapper is given in place of the source, has the `#[cfg]`s
        // on the module the allocator is in.
        let marker = match code.contains(FORWARDING_MODULE) {
            true => FORWARDING_MODULE,
            false => "#[global_allocator]",
        };
        for (start, _) in code.match_indices(marker) {
            let around = cfgs_around(&kept, start, start + marker.len());
            found.push(format!("{outer}{}{around}", enclosing_cfgs(&code, &kept, start)));
        }
        // A module of `a.rs` is in `a/`, one of `main.rs`, `lib.rs` or `mod.rs` beside it.
        let Some(parent) = file.parent() else { continue };
        let dir = if owns_dir {
            parent.to_path_buf()
        } else {
            parent.join(file.file_stem().unwrap_or_default())
        };
        for (name, statement) in out_of_line_modules(&code) {
            let enclosing = enclosing_cfgs(&code, &kept, statement.start);
            let outer = format!("{outer}{enclosing}{}", leading_cfgs(&kept[statement]));
            let flat = dir.join(format!("{name}.rs"));
            let nested = dir.join(&name).join("mod.rs");
            if flat.exists() {
                files.push((flat, false, outer));
            } else if nested.exists() {
                files.push((nested, true, outer));
            }
        }
    }
    found
}

/// The text with its comments blanked out, at the same offsets, and with `strings` the
/// insides of its string and character literals as well.
fn blanked(text: &str, strings: bool) -> String {
    let bytes = text.as_bytes();
    let mut out = bytes.to_vec();
    let mut blank = |range: std::ops::Range<usize>| {
        for it in &mut out[range] {
            if *it != b'\n' {
                *it = b' ';
            }
        }
    };
    let ident = |at: usize| bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_';
    let find = |from: usize, what: &[u8]| {
        let rest = bytes.get(from..).unwrap_or_default();
        rest.windows(what.len()).position(|it| it == what).map_or(bytes.len(), |it| from + it)
    };
    let mut at = 0;
    while at < bytes.len() {
        let rest = &bytes[at..];
        if rest.starts_with(b"//") {
            let end = find(at, b"\n");
            blank(at..end);
            at = end;
        } else if rest.starts_with(b"/*") {
            let (mut depth, mut end) = (0usize, at);
            while end < bytes.len() {
                if bytes[end..].starts_with(b"/*") {
                    (depth, end) = (depth + 1, end + 2);
                } else if bytes[end..].starts_with(b"*/") {
                    (depth, end) = (depth - 1, end + 2);
                    if depth == 0 {
                        break;
                    }
                } else {
                    end += 1;
                }
            }
            blank(at..end);
            at = end;
        } else if rest[0] == b'r'
            && (at == 0
                || !ident(at - 1)
                || matches!(bytes[at - 1], b'b' | b'c') && (at < 2 || !ident(at - 2)))
            && let hashes = rest[1..].iter().take_while(|&&it| it == b'#').count()
            && rest.get(1 + hashes) == Some(&b'"')
        {
            let open = at + 2 + hashes;
            let close = find(open, &[b"\"".as_slice(), &vec![b'#'; hashes]].concat());
            if strings {
                blank(open..close);
            }
            at = (close + 1 + hashes).min(bytes.len());
        } else if rest[0] == b'"' {
            let mut end = at + 1;
            while end < bytes.len() && bytes[end] != b'"' {
                end += if bytes[end] == b'\\' { 2 } else { 1 };
            }
            let end = end.min(bytes.len());
            if strings {
                blank(at + 1..end);
            }
            at = end + 1;
        } else if rest[0] == b'\'' {
            // A character, or the lifetime or label that starts the same.
            let inside = match rest.get(1) {
                Some(b'\\') => Some(find(at + 3, b"'")),
                Some(_) => text[at + 1..]
                    .chars()
                    .next()
                    .map(|it| at + 1 + it.len_utf8())
                    .filter(|&end| bytes.get(end) == Some(&b'\'')),
                None => None,
            };
            match inside {
                Some(end) => {
                    if strings {
                        blank(at + 1..end);
                    }
                    at = end + 1;
                }
                None => at += 1,
            }
        } else {
            at += 1;
        }
    }
    String::from_utf8(out).expect("whole characters are blanked")
}

/// The `#[cfg(..)]` attributes next to the one at `start..end`: those before it back to the
/// end of the previous item, and those after it up to the item.
fn cfgs_around(text: &str, start: usize, end: usize) -> String {
    let mut cfgs = Vec::new();
    let mut before = text[..start].trim_end();
    while before.ends_with(']') {
        let Some(open) = matching_open(before) else { break };
        let attribute = &before[open..];
        let Some(prefix) = before[..open].strip_suffix('#') else { break };
        cfgs.push(attribute);
        before = prefix.trim_end();
    }
    cfgs.reverse();
    let mut after = text[end..].trim_start();
    while let Some(attribute) = after.strip_prefix('#') {
        let Some(len) = matching_close(attribute) else { break };
        cfgs.push(&attribute[..len]);
        after = attribute[len..].trim_start();
    }
    let cfg =
        |it: &&str| it.strip_prefix('[').is_some_and(|it| it.trim_start().starts_with("cfg("));
    cfgs.into_iter().filter(cfg).map(|it| format!("#{it}\n")).collect()
}

/// Where the `[` is that the `]` ending `text` closes.
fn matching_open(text: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (at, it) in text.char_indices().rev() {
        match it {
            ']' => depth += 1,
            '[' => {
                depth -= 1;
                if depth == 0 {
                    return Some(at);
                }
            }
            _ => {}
        }
    }
    None
}

/// The length of the `[..]` that `text` starts with.
fn matching_close(text: &str) -> Option<usize> {
    if !text.starts_with('[') {
        return None;
    }
    let mut depth = 0usize;
    for (at, it) in text.char_indices() {
        match it {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(at + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// The names of the `mod name;` items of a file, each with where its statement is.
fn out_of_line_modules(text: &str) -> Vec<(String, std::ops::Range<usize>)> {
    let mut names = Vec::new();
    let mut from = 0;
    for (at, end) in text.match_indices([';', '{', '}']) {
        let statement = from..at;
        from = at + 1;
        let words: Vec<&str> = text[statement.clone()].split_whitespace().collect();
        if end == ";"
            && let [.., "mod", name] = words[..]
            && name.chars().all(|it| it.is_alphanumeric() || it == '_')
        {
            names.push((name.to_owned(), statement));
        }
    }
    names
}

/// The `#[cfg(..)]` attributes of the inline modules (`mod name { .. }`) that `at` is in,
/// outermost first: a `#[cfg(test)] mod tests` can have an allocator of its own.
fn enclosing_cfgs(code: &str, kept: &str, at: usize) -> String {
    let mut open = Vec::new();
    let mut from = 0;
    for (brace, it) in code[..at].match_indices([';', '{', '}']) {
        let statement = from..brace;
        from = brace + 1;
        match it {
            "{" => {
                let words: Vec<&str> = code[statement.clone()].split_whitespace().collect();
                let module = matches!(words[..], [.., "mod", _]);
                open.push(if module { leading_cfgs(&kept[statement]) } else { String::new() });
            }
            "}" => {
                open.pop();
            }
            _ => {}
        }
    }
    open.concat()
}

/// The `#[cfg(..)]` attributes that `statement` starts with.
fn leading_cfgs(statement: &str) -> String {
    let mut cfgs = String::new();
    let mut rest = statement.trim_start();
    while let Some(attribute) = rest.strip_prefix('#') {
        let Some(len) = matching_close(attribute) else { break };
        if attribute[1..].trim_start().starts_with("cfg(") {
            cfgs += &format!("#{}\n", &attribute[..len]);
        }
        rest = attribute[len..].trim_start();
    }
    cfgs
}

fn valued_in<'a>(args: &[&'a str], flag: &str) -> Option<&'a str> {
    let at = args.iter().position(|&it| it == flag)?;
    args.get(at + 1).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ra-jit-stub-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn allocators_with_their_cfgs_through_modules() {
        let dir = scratch("modules");
        let file = |path: &str, text: &str| {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        file(
            "src/main.rs",
            "mod alloc;\nmod inline { fn f() {} }\n// #[global_allocator]\nfn main() {}\n",
        );
        file(
            "src/alloc.rs",
            "mod deeper;\n#[cfg(all(\n    not(windows),\n    feature = \"jemalloc\"\n))]\n#[global_allocator]\n\
             #[allow(unused)]\nstatic A: J = J;\n",
        );
        file("src/alloc/deeper.rs", "#[global_allocator]\npub static B: S = S;\n");
        file("src/inline.rs", "#[global_allocator]\nstatic C: S = S;\n");
        let mut found = global_allocators(&dir.join("src/main.rs"));
        found.sort();
        assert_eq!(found, ["", "#[cfg(all(\n    not(windows),\n    feature = \"jemalloc\"\n))]\n"]);
    }

    #[test]
    fn allocators_under_the_cfgs_of_inline_modules() {
        let dir = scratch("inline");
        std::fs::write(
            dir.join("lib.rs"),
            "fn f() { let _ = '{'; }\n#[cfg(feature = \"a\")]\npub mod outer {\n    impl S { fn g() {} }\n\
             \n    #[cfg(test)]\n    mod test {\n        use super::*;\n\n        #[global_allocator]\n\
             \x20       static A: D = D;\n    }\n}\n#[global_allocator]\nstatic B: S = S;\n",
        )
        .unwrap();
        assert_eq!(
            global_allocators(&dir.join("lib.rs")),
            ["#[cfg(feature = \"a\")]\n#[cfg(test)]\n", ""]
        );
    }

    #[test]
    fn allocators_not_in_strings_and_under_the_cfgs_of_their_modules() {
        let dir = scratch("strings");
        let file = |path: &str, text: &str| std::fs::write(dir.join(path), text).unwrap();
        file(
            "lib.rs",
            "#![doc = \"backed by a pool rather than `#[global_allocator]` // or /*\"]\n\
             const A: &str = r##\"a \"#[global_allocator]\" b\"##;\nconst B: char = '\"';\n\
             fn f<'a>(_: &'a str) -> char { '\\'' }\n/* /* */ #[global_allocator] */\n\
             #[cfg(feature = \"jemalloc\")]\n#[allow(unused)] mod alloc;\n",
        );
        file("alloc.rs", "#[cfg(not(windows))]\n#[global_allocator]\nstatic A: J = J;\n");
        assert_eq!(
            global_allocators(&dir.join("lib.rs")),
            ["#[cfg(feature = \"jemalloc\")]\n#[cfg(not(windows))]\n"]
        );
        let text = "é \"// '\" 'é' x";
        assert_eq!(blanked(text, false), text);
        assert_eq!(blanked(text, true), "é \"    \" '  ' x");
    }

    #[test]
    fn a_crate_named_twice_is_one_extern_crate() {
        let dir = scratch("named-twice");
        let args = [
            "--crate-type",
            "bin",
            "--extern",
            "dep=/t/out/libdep-1.rlib",
            "--extern",
            "dep=/t/out/libdep-1.rmeta",
            "--extern",
            "noprelude:other=/t/out/libother-2.rlib",
            "--extern",
            "derive=/t/out/libderive-3.dylib",
        ];
        let text = text_of(&args, true, &dir.join("main.rs"));
        assert_eq!(text.matches("extern crate dep;").count(), 1, "{text}");
        assert_eq!(text.matches("extern crate other;").count(), 1, "{text}");
        assert!(!text.contains("extern crate derive;"), "{text}");
    }

    #[test]
    fn a_stand_in_forwards_to_ra_jits_allocator() {
        let dir = scratch("forwards");
        let main = dir.join("main.rs");
        std::fs::write(
            &main,
            "#[cfg(unix)]\n#[global_allocator]\nstatic A: S = S;\nfn main() {}\n",
        )
        .unwrap();
        let text = text_of(&["--crate-type", "bin"], true, &main);
        assert!(text.contains("#[cfg(unix)]\n\nmod __ra_jit_global_allocator_0 {"), "{text}");
        assert!(
            text.contains("__ra_jit_global_realloc(ptr, layout.size(), layout.align(), new_size)")
        );
        assert!(!text_of(&["--crate-type", "bin"], true, &dir.join("none.rs")).contains("mod "));
        // A second wrapper is given the stand-in as the source.
        let stand_in = dir.join("main.ra-jit.rs");
        std::fs::write(&stand_in, &text).unwrap();
        assert_eq!(text_of(&["--crate-type", "bin"], true, &stand_in), text);
    }
}
