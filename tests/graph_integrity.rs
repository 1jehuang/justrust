//! Graph integrity gates. See GRAPH-INTEGRITY.md.
//!
//! The architecture (ARCHITECTURE.md section 17) requires that every compiler
//! fact and dependency lives in the graph engine: no hidden shared state, no
//! whole-program phase barriers. These tests make violations fail the build.
//! Existing violations are recorded as debt that may only shrink. Never raise a
//! recorded number or add an entry without explicit user approval.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use justrust_engine::compiler;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

const SHRINK_ENV: &str = "GRAPH_INTEGRITY_SHRINK";
const RULE: &str = "Every compiler fact must be a graph output read through engine handles, \
with no hidden shared state and no whole-program barrier. See GRAPH-INTEGRITY.md. \
Do not raise recorded debt without explicit user approval.";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

// ---------------------------------------------------------------------------
// Static gate: out-of-graph state primitives in compiler code.
// ---------------------------------------------------------------------------

const SCANNED: &[&str] = &["src/compiler", "src/project"];
const BASELINE: &str = "tests/graph_integrity/out_of_graph_state.baseline";

/// Replace comments and string/char literal contents with spaces, preserving
/// byte offsets and newlines so positions still map onto the original text.
fn strip(src: &str) -> Vec<u8> {
    let b = src.as_bytes();
    let n = b.len();
    let mut out = b.to_vec();
    let blank = |out: &mut Vec<u8>, from: usize, to: usize| {
        for byte in &mut out[from..to.min(n)] {
            if *byte != b'\n' {
                *byte = b' ';
            }
        }
    };
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80;
    let utf8_len = |c: u8| match c {
        0..=0x7f => 1,
        0x80..=0xdf => 2,
        0xe0..=0xef => 3,
        _ => 4,
    };
    let mut i = 0;
    while i < n {
        let c = b[i];
        let next = b.get(i + 1).copied();
        let prev_ident = i > 0 && ident(b[i - 1]);
        if c == b'/' && next == Some(b'/') {
            let start = i;
            while i < n && b[i] != b'\n' {
                i += 1;
            }
            blank(&mut out, start, i);
            continue;
        }
        if c == b'/' && next == Some(b'*') {
            let start = i;
            let mut depth = 0usize;
            while i < n {
                if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
                    depth += 1;
                    i += 2;
                } else if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            blank(&mut out, start, i);
            continue;
        }
        if !prev_ident && (c == b'r' || ((c == b'b' || c == b'c') && next == Some(b'r'))) {
            let mut j = if c == b'r' { i + 1 } else { i + 2 };
            let mut hashes = 0;
            while j < n && b[j] == b'#' {
                hashes += 1;
                j += 1;
            }
            if j < n && b[j] == b'"' {
                j += 1;
                let mut end = n;
                while j < n {
                    if b[j] == b'"'
                        && j + 1 + hashes <= n
                        && b[j + 1..j + 1 + hashes].iter().all(|&x| x == b'#')
                    {
                        end = j + 1 + hashes;
                        break;
                    }
                    j += 1;
                }
                blank(&mut out, i, end);
                i = end;
                continue;
            }
        }
        if c == b'"' || (!prev_ident && (c == b'b' || c == b'c') && next == Some(b'"')) {
            let start = i;
            let mut j = if c == b'"' { i + 1 } else { i + 2 };
            while j < n {
                if b[j] == b'\\' {
                    j += 2;
                } else if b[j] == b'"' {
                    j += 1;
                    break;
                } else {
                    j += 1;
                }
            }
            blank(&mut out, start, j);
            i = j;
            continue;
        }
        if c == b'\'' || (!prev_ident && c == b'b' && next == Some(b'\'')) {
            let q = if c == b'\'' { i } else { i + 1 };
            if b.get(q + 1) == Some(&b'\\') {
                let mut j = q + 3;
                while j < n && b[j] != b'\'' {
                    j += 1;
                }
                blank(&mut out, i, j + 1);
                i = j + 1;
                continue;
            }
            if q + 1 < n {
                let len = utf8_len(b[q + 1]);
                if b.get(q + 1 + len) == Some(&b'\'') {
                    blank(&mut out, i, q + 2 + len);
                    i = q + 2 + len;
                    continue;
                }
            }
            // A lifetime such as 'static: keep it so it is never read as an item.
            i = q + 1;
            continue;
        }
        i += 1;
    }
    out
}

/// End of an attribute starting at `start` (pointing at `#`), exclusive.
fn attribute_end(s: &[u8], start: usize) -> Option<usize> {
    let open = s[start..].iter().position(|&c| c == b'[')? + start;
    let mut depth = 0usize;
    for (k, &c) in s.iter().enumerate().skip(open) {
        match c {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(k + 1);
                }
            }
            _ => {}
        }
    }
    None
}

fn is_test_cfg(attribute: &str) -> bool {
    let compact: String = attribute.chars().filter(|c| !c.is_whitespace()).collect();
    let inner = compact
        .trim_start_matches("#!")
        .trim_start_matches('#')
        .trim_start_matches('[');
    if !inner.starts_with("cfg(") || compact.contains("not(test") {
        return false;
    }
    compact
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .any(|token| token == "test")
}

struct Scanned {
    /// Stripped code with test-only items blanked.
    code: Vec<u8>,
    /// Whole file is test-only (`#![cfg(test)]`).
    test_file: bool,
    /// Out-of-line modules declared under a test cfg: (name, explicit path).
    test_modules: Vec<(String, Option<String>)>,
    /// All out-of-line module declarations in non-test code.
    modules: Vec<(String, Option<String>)>,
}

fn path_attribute(original: &str, from: usize, to: usize) -> Option<String> {
    let text = &original[from..to];
    let at = text.find("#[path")?;
    let rest = &text[at..];
    let open = rest.find('"')? + 1;
    let close = rest[open..].find('"')? + open;
    Some(rest[open..close].to_string())
}

fn module_declaration(code: &[u8], from: usize, to: usize) -> Option<String> {
    let header = String::from_utf8_lossy(&code[from..to]);
    let words: Vec<&str> = header
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .collect();
    let at = words.iter().position(|w| *w == "mod")?;
    words.get(at + 1).map(|w| w.to_string())
}

fn scan(original: &str) -> Scanned {
    let mut code = strip(original);
    let mut test_modules = Vec::new();
    let mut test_file = false;
    let mut i = 0;
    while i + 1 < code.len() {
        if code[i] != b'#' || !(code[i + 1] == b'[' || code[i + 1] == b'!') {
            i += 1;
            continue;
        }
        let Some(end) = attribute_end(&code, i) else {
            break;
        };
        let attribute = String::from_utf8_lossy(&code[i..end]).to_string();
        if !is_test_cfg(&attribute) {
            i = end;
            continue;
        }
        if code[i + 1] == b'!' {
            test_file = true;
            break;
        }
        // Skip following attributes, then the item itself.
        let mut j = end;
        loop {
            while j < code.len() && code[j].is_ascii_whitespace() {
                j += 1;
            }
            if j + 1 < code.len() && code[j] == b'#' && code[j + 1] == b'[' {
                j = attribute_end(&code, j).unwrap_or(code.len());
            } else {
                break;
            }
        }
        let item_start = j;
        let mut depth = 0isize;
        let mut item_end = code.len();
        let mut k = j;
        while k < code.len() {
            match code[k] {
                b'(' | b'[' => depth += 1,
                b')' | b']' => depth -= 1,
                b';' if depth == 0 => {
                    if let Some(name) = module_declaration(&code, item_start, k) {
                        test_modules.push((name, path_attribute(original, i, k)));
                    }
                    item_end = k + 1;
                    break;
                }
                b'{' if depth == 0 => {
                    let mut braces = 0usize;
                    while k < code.len() {
                        if code[k] == b'{' {
                            braces += 1;
                        } else if code[k] == b'}' {
                            braces -= 1;
                            if braces == 0 {
                                break;
                            }
                        }
                        k += 1;
                    }
                    item_end = k + 1;
                    break;
                }
                _ => {}
            }
            k += 1;
        }
        let item_end = item_end.min(code.len());
        for byte in &mut code[i..item_end] {
            if *byte != b'\n' {
                *byte = b' ';
            }
        }
        i = item_end;
    }
    let modules = declared_modules(original, &code);
    Scanned {
        code,
        test_file,
        test_modules,
        modules,
    }
}

fn declared_modules(original: &str, code: &[u8]) -> Vec<(String, Option<String>)> {
    let text = String::from_utf8_lossy(code);
    let mut found = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        let words: Vec<&str> = trimmed.trim_end_matches(';').split_whitespace().collect();
        if trimmed.ends_with(';')
            && words.len() >= 2
            && words[words.len() - 2] == "mod"
            && words[..words.len() - 2]
                .iter()
                .all(|w| w.starts_with("pub"))
        {
            let start = original[..offset]
                .rfind("\n\n")
                .unwrap_or(0)
                .max(offset.saturating_sub(200));
            found.push((
                words[words.len() - 1].to_string(),
                path_attribute(original, start, offset + line.len()),
            ));
        }
        offset += line.len();
    }
    found
}

fn module_files(declaring: &Path, name: &str, explicit: &Option<String>) -> Vec<PathBuf> {
    let dir = declaring.parent().unwrap();
    if let Some(path) = explicit {
        return vec![dir.join(path)];
    }
    let stem = declaring.file_stem().unwrap().to_string_lossy();
    let base = if matches!(stem.as_ref(), "mod" | "lib" | "main") {
        dir.to_path_buf()
    } else {
        dir.join(stem.as_ref())
    };
    vec![
        base.join(format!("{name}.rs")),
        base.join(name).join("mod.rs"),
    ]
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.map(|e| e.unwrap().path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Primitives that can carry state across graph tasks or workers. Single-thread
/// `Cell`/`RefCell` are not listed: they are `!Sync`, so they cannot be shared
/// between graph tasks and stay local to the computation that owns them.
fn classify(identifier: &str) -> Option<&'static str> {
    Some(match identifier {
        "OnceLock" => "OnceLock",
        "LazyLock" => "LazyLock",
        "UnsafeCell" => "UnsafeCell",
        "Condvar" => "Condvar",
        "thread_local" => "thread_local",
        id if id.starts_with("Mutex") => "Mutex",
        id if id.starts_with("RwLock") => "RwLock",
        id if id.starts_with("Atomic") && id[6..].starts_with(|c: char| c.is_ascii_uppercase()) => {
            "Atomic"
        }
        _ => return None,
    })
}

fn count_primitives(code: &[u8]) -> BTreeMap<&'static str, usize> {
    let text = String::from_utf8_lossy(code);
    let bytes = text.as_bytes();
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut tokens: Vec<(usize, &str)> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if (bytes[i].is_ascii_alphabetic() || bytes[i] == b'_') && (i == 0 || !ident(bytes[i - 1]))
        {
            let start = i;
            while i < bytes.len() && ident(bytes[i]) {
                i += 1;
            }
            tokens.push((start, &text[start..i]));
        } else {
            i += 1;
        }
    }
    let mut counts = BTreeMap::new();
    for (index, &(start, token)) in tokens.iter().enumerate() {
        if let Some(kind) = classify(token) {
            *counts.entry(kind).or_default() += 1;
        }
        if token == "static" && (start == 0 || bytes[start - 1] != b'\'') {
            let mut k = index + 1;
            if tokens.get(k).is_some_and(|t| t.1 == "mut") {
                k += 1;
            }
            if let Some(&(name_start, name)) = tokens.get(k) {
                let after = text[name_start + name.len()..].trim_start();
                if after.starts_with(':') && !after.starts_with("::") {
                    *counts.entry("static").or_default() += 1;
                }
            }
        }
    }
    counts
}

type Debt = BTreeMap<(String, String), usize>;

fn measure_out_of_graph_state() -> Debt {
    let root = root();
    let mut files = Vec::new();
    for dir in SCANNED {
        rust_files(&root.join(dir), &mut files);
    }
    let mut scanned: BTreeMap<PathBuf, Scanned> = BTreeMap::new();
    for path in &files {
        let text = fs::read_to_string(path).unwrap();
        scanned.insert(path.clone(), scan(&text));
    }
    // Test-only files: declared under a test cfg, transitively.
    let mut test_only: HashSet<PathBuf> = HashSet::new();
    let mut frontier: Vec<PathBuf> = Vec::new();
    for (path, scan) in &scanned {
        if scan.test_file {
            frontier.push(path.clone());
        }
        for (name, explicit) in &scan.test_modules {
            frontier.extend(module_files(path, name, explicit));
        }
    }
    while let Some(path) = frontier.pop() {
        let path = normalize(&path);
        if !scanned.contains_key(&path) || !test_only.insert(path.clone()) {
            continue;
        }
        let scan = &scanned[&path];
        for (name, explicit) in scan.modules.iter().chain(&scan.test_modules) {
            frontier.extend(module_files(&path, name, explicit));
        }
    }
    let mut debt = Debt::new();
    for (path, scan) in &scanned {
        if test_only.contains(path) {
            continue;
        }
        let relative = path
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .to_string();
        for (kind, count) in count_primitives(&scan.code) {
            debt.insert((relative.clone(), kind.to_string()), count);
        }
    }
    debt
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

fn read_baseline() -> Debt {
    let text = fs::read_to_string(root().join(BASELINE)).unwrap_or_default();
    let mut debt = Debt::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let count: usize = parts.next().unwrap().parse().unwrap();
        let kind = parts.next().unwrap().to_string();
        let path = parts.next().unwrap().to_string();
        debt.insert((path, kind), count);
    }
    debt
}

fn write_baseline(debt: &Debt) {
    let mut text = String::from(
        "# Out-of-graph state debt: shared mutable state, global locks, lazy statics and\n\
         # process-global counters in compiler code. Format: COUNT PRIMITIVE PATH.\n\
         # These numbers may only go down. Raising one, or adding a line, requires\n\
         # explicit user approval. See GRAPH-INTEGRITY.md. Shrink after removing debt:\n\
         # GRAPH_INTEGRITY_SHRINK=1 justrust test --test graph_integrity\n",
    );
    for ((path, kind), count) in debt {
        text.push_str(&format!("{count} {kind} {path}\n"));
    }
    fs::write(root().join(BASELINE), text).unwrap();
}

#[test]
fn compiler_code_adds_no_out_of_graph_state() {
    let current = measure_out_of_graph_state();
    let baseline = read_baseline();
    let mut increases = Vec::new();
    let mut decreases = Vec::new();
    for (key, &count) in &current {
        let allowed = baseline.get(key).copied().unwrap_or(0);
        if count > allowed {
            increases.push(format!("  {} {}: {allowed} -> {count}", key.0, key.1));
        } else if count < allowed {
            decreases.push(format!("  {} {}: {allowed} -> {count}", key.0, key.1));
        }
    }
    for (key, &allowed) in &baseline {
        if !current.contains_key(key) {
            decreases.push(format!("  {} {}: {allowed} -> 0", key.0, key.1));
        }
    }
    assert!(
        increases.is_empty(),
        "new out-of-graph state in compiler code:\n{}\n{RULE}",
        increases.join("\n")
    );
    if !decreases.is_empty() {
        if std::env::var_os(SHRINK_ENV).is_some() {
            write_baseline(&current);
            return;
        }
        panic!(
            "out-of-graph state debt shrank. Lock in the progress with \
             `{SHRINK_ENV}=1 justrust test --test graph_integrity` and commit {BASELINE}:\n{}",
            decreases.join("\n")
        );
    }
}

#[test]
fn scanner_ignores_comments_strings_tests_and_lifetimes() {
    let source = r##"
use std::sync::{Arc, OnceLock};
// Mutex in a comment
/* RefCell /* nested */ Mutex */
fn f() -> &'static str { "Mutex OnceLock static X: u8" }
const C: char = '\'';
const R: &str = r#"Mutex "quoted" RwLock"#;
static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
fn g<T: 'static>() {}
#[cfg(test)]
mod tests { use std::sync::Mutex; static Y: u8 = 0; }
#[cfg(test)]
mod more;
#[cfg(not(test))]
fn h(_: std::sync::Mutex<u8>) {}
"##;
    let scanned = scan(source);
    let counts = count_primitives(&scanned.code);
    assert_eq!(counts.get("OnceLock"), Some(&1));
    assert_eq!(
        counts.get("Mutex"),
        Some(&1),
        "only the cfg(not(test)) Mutex counts"
    );
    assert_eq!(counts.get("Atomic"), Some(&2));
    assert_eq!(counts.get("static"), Some(&1));
    assert_eq!(counts.get("UnsafeCell"), None);
    assert_eq!(counts.get("RwLock"), None);
    assert_eq!(scanned.test_modules, vec![("more".to_string(), None)]);
}

// ---------------------------------------------------------------------------
// Dynamic gates: graph shape and scaling of an actual compilation.
// ---------------------------------------------------------------------------

/// Same generator as experiments/compiler.py `source_for`: independent leaf
/// functions plus one main calling each. Leaf work must not depend on N.
fn flat_source(count: usize) -> String {
    let mut lines: Vec<String> = (0..count)
        .map(|i| format!("fn f{i}(x:i32)->i32 {{ if x < 0 {{ -x }} else {{ x + {i} }} }}"))
        .collect();
    lines.push("fn fail(x:i32) { let y=1/x; }".into());
    let calls: Vec<String> = (0..count)
        .map(|i| format!("if f{i}(2) != {} {{ fail(0); }}", i + 2))
        .collect();
    lines.push(format!(
        "fn main(){{{}println!(\"compiled correctly\");}}",
        calls.join("\n")
    ));
    lines.join("\n")
}

fn kind_name(kind: u32) -> String {
    match kind {
        100 => "parse",
        101 => "signatures",
        102 => "body",
        103 => "object",
        104 => "check",
        105 => "build",
        106 => "resolve",
        107 => "source",
        108 => "lower",
        109 => "analyze",
        110 => "template_body",
        111 => "universal_lower",
        112 => "universal_analyze",
        113 => "mono_plan",
        114 => "concrete_proofs",
        115 => "effects_ready",
        _ => return format!("kind{kind}"),
    }
    .to_string()
}

/// Requested roots. They are the only whole-program facts that may remain.
/// `check` is the explicit "every required check passed" completeness fact and
/// `build` gates the linked artifact on it, as ARCHITECTURE.md section 17 allows.
const PERMANENT_WHOLE_PROGRAM: &[&str] = &["check", "build"];

/// Whole-program nodes that exist today and must be split into per-file or
/// per-item facts. This list may only shrink.
const WHOLE_PROGRAM_DEBT: &[&str] = &[
    "source",
    "parse",
    "resolve",
    "signatures",
    "effects_ready",
    "mono_plan",
    "concrete_proofs",
];

/// Ceiling on how much the transitive dependency closure of an independent leaf
/// function grows when the program doubles from 16 to 32 functions. A value
/// above 1.0 means unrelated functions are hidden or explicit prerequisites,
/// which is a whole-program barrier. Target: 1.0. May only go down.
const LEAF_CLOSURE_GROWTH_CEILING: f64 = 1.50;

/// Ceiling on planning work growth from 16 to 32 functions. Linear compilation
/// is about 2.0; quadratic is 4.0. Target: at most 2.2. May only go down.
const WORK_GROWTH_CEILING: f64 = 2.05;

/// The largest generated program that must compile. Raise as fixed; the
/// architectural target is no fixed program-size ceiling at all.
const REQUIRED_FUNCTIONS: usize = 350;
/// Known to fail today because whole-compilation budgets cap total program
/// size. When it compiles, this test fails until the constant is raised.
const KNOWN_FAILING_FUNCTIONS: Option<usize> = Some(400);

struct Graph {
    keys: HashMap<u64, (String, Vec<u64>)>,
    deps: HashMap<u64, BTreeSet<u64>>,
}

impl Graph {
    fn from_report(report: &justrust_engine::Report) -> Self {
        assert!(report.capture_complete, "graph capture must be complete");
        let mut keys = HashMap::new();
        for ((key, _), id) in report.node_evaluations.iter().zip(&report.node_ids) {
            keys.insert(*id, (kind_name(key.kind), key.params.clone()));
        }
        let mut task_node = HashMap::new();
        for event in &report.events {
            if let (Some(task), Some(node)) = (event.task, event.node) {
                task_node.entry(task).or_insert(node);
            }
        }
        let mut deps: HashMap<u64, BTreeSet<u64>> = HashMap::new();
        for &(task, producer, _) in &report.dependency_edges {
            let consumer = task_node[&task];
            deps.entry(consumer).or_default().insert(producer);
        }
        Self { keys, deps }
    }

    fn by_key(&self) -> HashMap<(String, Vec<u64>), u64> {
        self.keys
            .iter()
            .map(|(id, key)| (key.clone(), *id))
            .collect()
    }

    fn fan_in(&self, node: u64) -> usize {
        self.deps.get(&node).map_or(0, BTreeSet::len)
    }

    fn closure(&self, node: u64) -> usize {
        let mut seen = HashSet::new();
        let mut stack = vec![node];
        while let Some(n) = stack.pop() {
            for &dep in self.deps.get(&n).into_iter().flatten() {
                if seen.insert(dep) {
                    stack.push(dep);
                }
            }
        }
        seen.len()
    }
}

fn build_graph(count: usize) -> Graph {
    let compilation = compiler::compile_observed(flat_source(count), 1, false)
        .unwrap_or_else(|failure| panic!("{count} functions failed: {failure}"));
    Graph::from_report(&compilation.report)
}

#[test]
fn no_new_whole_program_barriers() {
    let small = build_graph(16);
    let large = build_graph(32);

    let whole: BTreeSet<String> = large
        .keys
        .values()
        .filter(|(_, params)| params.is_empty())
        .map(|(kind, _)| kind.clone())
        .collect();
    let added: Vec<_> = whole
        .iter()
        .filter(|k| !PERMANENT_WHOLE_PROGRAM.contains(&k.as_str()))
        .filter(|k| !WHOLE_PROGRAM_DEBT.contains(&k.as_str()))
        .collect();
    assert!(
        added.is_empty(),
        "new whole-program graph nodes {added:?}. Facts must be keyed per file or item. {RULE}"
    );
    let removed: Vec<_> = WHOLE_PROGRAM_DEBT
        .iter()
        .filter(|k| !whole.contains(**k))
        .collect();
    assert!(
        removed.is_empty(),
        "whole-program nodes {removed:?} are gone. Remove them from WHOLE_PROGRAM_DEBT \
         in tests/graph_integrity.rs to lock in the progress."
    );

    // A per-item fact reads a bounded set of facts. If its direct dependency
    // count grows with program size, it is a whole-program join in disguise.
    let small_keys = small.by_key();
    let mut joins = Vec::new();
    for (id, key) in &large.keys {
        // Indices below 16 are the same leaf function in both programs.
        if key.1.is_empty() || key.1[0] >= 16 {
            continue;
        }
        if let Some(&small_id) = small_keys.get(key) {
            let (a, b) = (small.fan_in(small_id), large.fan_in(*id));
            if b > a {
                joins.push(format!(
                    "{}{:?}: {a} -> {b} direct dependencies",
                    key.0, key.1
                ));
            }
        }
    }
    joins.sort();
    assert!(
        joins.is_empty(),
        "per-item graph facts whose direct dependencies grow with program size \
         (16 -> 32 functions):\n  {}\n{RULE}",
        joins.join("\n  ")
    );

    // Leaf functions are independent: their dependency closure must not grow
    // with the number of unrelated functions in the program.
    let mut worst = (0.0f64, String::new());
    for (id, key) in &large.keys {
        if key.1.is_empty() {
            continue;
        }
        let Some(&small_id) = small_keys.get(key) else {
            continue;
        };
        if key.1[0] >= 16 {
            continue;
        }
        let (a, b) = (small.closure(small_id), large.closure(*id));
        if a == 0 {
            continue;
        }
        let growth = b as f64 / a as f64;
        if growth > worst.0 {
            worst = (growth, format!("{}{:?}: {a} -> {b} nodes", key.0, key.1));
        }
    }
    eprintln!("leaf closure growth 16->32: {:.3} ({})", worst.0, worst.1);
    assert!(
        worst.0 <= LEAF_CLOSURE_GROWTH_CEILING,
        "leaf function dependency closure grew {:.3}x from 16 to 32 functions \
         (ceiling {LEAF_CLOSURE_GROWTH_CEILING}): {}. An independent function now waits \
         on unrelated ones. {RULE}",
        worst.0,
        worst.1
    );
    assert!(
        worst.0 > LEAF_CLOSURE_GROWTH_CEILING - 0.1,
        "leaf closure growth improved to {:.3}. Lower LEAF_CLOSURE_GROWTH_CEILING in \
         tests/graph_integrity.rs to {:.2} to lock in the progress.",
        worst.0,
        worst.0 + 0.02
    );
}

fn check_work(count: usize) -> usize {
    compiler::compile_observed(flat_source(count), 1, true)
        .unwrap_or_else(|failure| panic!("{count} functions failed: {failure}"))
        .resource_usage
        .work
}

#[test]
fn compile_work_scales_linearly_ratchet() {
    let growth = check_work(32) as f64 / check_work(16) as f64;
    eprintln!("planning work growth 16->32: {growth:.3}");
    assert!(
        growth <= WORK_GROWTH_CEILING,
        "compile work grew {growth:.3}x when the program doubled (ceiling \
         {WORK_GROWTH_CEILING}, linear is about 2.0). Some per-function step became \
         more superlinear. Profile with `jr-rust --profile` and fix the algorithm."
    );
    assert!(
        growth > WORK_GROWTH_CEILING - 0.15,
        "work growth improved to {growth:.3}. Lower WORK_GROWTH_CEILING in \
         tests/graph_integrity.rs to {:.2} to lock in the progress.",
        growth + 0.05
    );
}

#[test]
fn program_size_ceiling_ratchet() {
    compiler::compile_observed(flat_source(REQUIRED_FUNCTIONS), 1, true).unwrap_or_else(
        |failure| panic!("{REQUIRED_FUNCTIONS} generated functions must compile: {failure}"),
    );
    if let Some(count) = KNOWN_FAILING_FUNCTIONS {
        match compiler::compile_observed(flat_source(count), 1, true) {
            Ok(_) => panic!(
                "{count} functions now compile. Raise REQUIRED_FUNCTIONS to {count} and \
                 KNOWN_FAILING_FUNCTIONS to the next failing size (or None) in \
                 tests/graph_integrity.rs."
            ),
            Err(failure) => assert!(
                failure.resource_usage.first_failure.is_some(),
                "{count} functions failed for a reason other than resource limits: {failure}"
            ),
        }
    }
}
