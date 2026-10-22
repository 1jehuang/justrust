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
        142 => "instance",
        114 => "concrete_proofs",
        115 => "universal_admission",
        116 => "body_lexical",
        117 => "capture_environment",
        130 => "ssa",
        131 => "local_opt",
        132 => "summary",
        133 => "ipo_opt",
        141 => "item_inventory",
        143 => "signature",
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
const WHOLE_PROGRAM_DEBT: &[&str] = &["parse", "resolve", "signatures"];

/// Ceiling on how much the transitive dependency closure of an independent leaf
/// function grows when the program doubles from 16 to 32 functions. A value
/// above 1.0 means unrelated functions are hidden or explicit prerequisites,
/// which is a whole-program barrier. Target: 1.0. May only go down.
const LEAF_CLOSURE_GROWTH_CEILING: f64 = 1.02;

/// Ceiling on planning work growth from 16 to 32 functions. Linear compilation
/// is about 2.0; quadratic is 4.0. Target: at most 2.2. May only go down.
// Source-library methods add fixed universal work. The lower ratio is not an
// algorithmic speedup, but the measured growth ceiling must still only shrink.
const WORK_GROWTH_CEILING: f64 = 1.08;
/// Optimizer BUILD growth keeps the prior shared ceiling; may only shrink.
const OPTIMIZER_WORK_GROWTH_CEILING: f64 = 1.81;

/// The largest generated program that must compile. Raise as fixed; the
/// architectural target is no fixed program-size ceiling at all.
const REQUIRED_FUNCTIONS: usize = 2000;
/// Known to fail today because whole-compilation budgets cap total program
/// size. When it compiles, this test fails until the constant is raised.
/// None: budgets are per-node ledgers (no shared plan/expansion ledger) and
/// scale with each node's own input. Larger counts hit the per-function
/// ownership state cap of the single generated `main`, a per-task limit.
const KNOWN_FAILING_FUNCTIONS: Option<usize> = None;

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
    build_graph_with(count, compiler::ssa::OptMode::Off)
}

/// The same flat program with an explicit optimizer mode, so the gates also
/// measure the optimizer nodes (SSA, LOCAL_OPT, SUMMARY, IPO_OPT).
fn build_graph_with(count: usize, opt: compiler::ssa::OptMode) -> Graph {
    let options = compiler::source::SourceOptions {
        opt,
        ..compiler::source::SourceOptions::default()
    };
    let compilation =
        compiler::compile_observed_with_options(flat_source(count), 1, false, options)
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

/// CHECK fans out through per-module item inventories and per-item checks.
/// It must not scan every callable's TEMPLATE_BODY to discover which ones are
/// universal: non-universal callables reach CHECK only through admission.
#[test]
fn check_root_reads_item_inventories_not_template_bodies() {
    let graph = build_graph(16);
    let (check, _) = graph
        .keys
        .iter()
        .find(|(_, key)| key.0 == "check" && key.1.is_empty())
        .expect("check root");
    let direct: Vec<&(String, Vec<u64>)> = graph.deps[check]
        .iter()
        .map(|dep| &graph.keys[dep])
        .collect();
    assert!(
        direct.iter().all(|key| key.0 != "template_body"),
        "CHECK depends directly on TEMPLATE_BODY: {direct:?}. {RULE}"
    );
    assert!(
        direct.iter().any(|key| key.0 == "item_inventory"),
        "CHECK no longer reads per-module item inventories: {direct:?}"
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
        |failure| {
            if let (Some(resource), Some(report)) = (&failure.resource_usage.first_failure, &failure.report)
                && let Some(task) = resource.task_id
                && let Some(node) = report.events.iter().find(|event| event.task == Some(task) && event.node.is_some()).and_then(|event| event.node)
                && let Some(index) = report.node_ids.iter().position(|id| *id == node)
            {
                eprintln!("resource task key: {:?}", report.node_evaluations[index].0);
            }
            panic!("{REQUIRED_FUNCTIONS} generated functions must compile: {failure}; first resource failure: {:?}", failure.resource_usage.first_failure)
        },
    );
    if let Some(count) = KNOWN_FAILING_FUNCTIONS {
        match compiler::compile_observed(flat_source(count), 1, true) {
            Ok(_) => panic!(
                "{count} functions now compile. Raise REQUIRED_FUNCTIONS to {count} and \
                 KNOWN_FAILING_FUNCTIONS to the next failing size (or None) in \
                 tests/graph_integrity.rs."
            ),
            Err(failure) => {
                eprintln!(
                    "size probe {count}: {:?}",
                    failure.resource_usage.first_failure
                );
                assert!(
                    failure.resource_usage.first_failure.is_some(),
                    "{count} functions failed for a reason other than resource limits: {failure}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Reachable-fact gate: mutable state reachable from published graph facts.
// ---------------------------------------------------------------------------
//
// Graph facts must be immutable. A fact that carries a `Mutex`, `OnceLock` or
// atomic (directly or through any field, alias, wrapper or generic argument)
// is shared mutable state with a graph-shaped disguise: consumers read it
// through a handle, but mutations through it are invisible to the graph. The
// lexical scan above cannot see this when the primitive is declared in a
// different file from the fact. This gate builds a type-reference graph of
// compiler declarations and reports every mutable primitive reachable from a
// fact root, recorded in a shrink-only debt inventory.

const FACT_DEBT: &str = "tests/graph_integrity/fact_reachable_state.debt";

/// Interior-mutability primitives that make a reachable value mutable across
/// graph tasks. `Cell`/`RefCell` are `!Sync`, so `publish_fact` (which requires
/// `Send + Sync`) cannot carry them; they are omitted.
const FACT_MUTABLE: &[&str] = &[
    "Mutex",
    "RwLock",
    "OnceLock",
    "LazyLock",
    "Condvar",
    "UnsafeCell",
    "AtomicBool",
    "AtomicU8",
    "AtomicU16",
    "AtomicU32",
    "AtomicU64",
    "AtomicUsize",
    "AtomicI8",
    "AtomicI16",
    "AtomicI32",
    "AtomicI64",
    "AtomicIsize",
    "AtomicPtr",
];

/// One declaration: its type-name references, from fields, variants, alias
/// right-hand sides and generic bounds.
#[derive(Debug, Default, Clone)]
struct TypeDecl {
    file: String,
    refs: BTreeSet<String>,
}

fn identifiers(text: &str) -> Vec<&str> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| w.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_'))
        .collect()
}

/// Matching close for the `{`, `(` or `<` at `open`, ignoring `->`/`=>`.
fn matching(code: &[u8], open: usize) -> Option<usize> {
    let (o, c) = match code[open] {
        b'{' => (b'{', b'}'),
        b'(' => (b'(', b')'),
        b'<' => (b'<', b'>'),
        _ => return None,
    };
    let mut depth = 0usize;
    let mut k = open;
    while k < code.len() {
        let b = code[k];
        let arrow = b == b'>' && k > 0 && (code[k - 1] == b'-' || code[k - 1] == b'=');
        if b == o {
            depth += 1;
        } else if b == c && !arrow {
            depth -= 1;
            if depth == 0 {
                return Some(k);
            }
        }
        k += 1;
    }
    None
}

/// Type-name references in one type expression. `dyn Trait` becomes the
/// pseudo-declaration `dyn Trait`, which references every implementor.
fn type_refs(ty: &str) -> Vec<String> {
    let words = identifiers(ty);
    let mut out = Vec::new();
    let mut i = 0;
    while i < words.len() {
        if words[i] == "dyn" && i + 1 < words.len() {
            // Path segments: `dyn tr::SchemaCx` takes the last segment before
            // a non-path boundary. identifiers() splits on `::`, so walk while
            // the source has `::` between words; approximate with next word
            // that starts uppercase.
            let mut j = i + 1;
            while j + 1 < words.len() && words[j].starts_with(|c: char| c.is_ascii_lowercase()) {
                j += 1;
            }
            out.push(format!("dyn {}", words[j]));
            i = j + 1;
            continue;
        }
        out.push(words[i].to_string());
        i += 1;
    }
    out
}

/// Split `text` at top-level `sep`, ignoring separators nested in `()`, `[]`,
/// `{}` or `<>`. `->` and `=>` do not close an angle bracket.
fn split_top(text: &str, sep: u8) -> Vec<&str> {
    let b = text.as_bytes();
    let mut depth = 0isize;
    let mut parts = Vec::new();
    let mut start = 0;
    for (k, &c) in b.iter().enumerate() {
        match c {
            b'(' | b'[' | b'{' | b'<' => depth += 1,
            b'>' if k > 0 && (b[k - 1] == b'-' || b[k - 1] == b'=') => {}
            b')' | b']' | b'}' | b'>' => depth -= 1,
            _ if c == sep && depth == 0 => {
                parts.push(&text[start..k]);
                start = k + 1;
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
}

/// Drop leading `#[...]` attributes, whitespace and a `pub`/`pub(...)`
/// visibility from one field or variant.
fn strip_field_prefix(mut piece: &str) -> &str {
    loop {
        piece = piece.trim_start();
        if piece.starts_with("#[") || piece.starts_with("#![") {
            match matching(piece.as_bytes(), piece.find('[').unwrap()) {
                Some(close) => piece = &piece[close + 1..],
                None => return "",
            }
        } else if let Some(rest) = piece.strip_prefix("pub") {
            if rest.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_') {
                return piece;
            }
            let rest = rest.trim_start();
            piece = if rest.starts_with('(') {
                match matching(rest.as_bytes(), 0) {
                    Some(close) => &rest[close + 1..],
                    None => return "",
                }
            } else {
                rest
            };
        } else {
            return piece;
        }
    }
}

/// Type references of a field list: `name: Type, ...` when `named`, else
/// `Type, ...`. Field names are never references.
fn field_type_refs(inner: &str, named: bool, refs: &mut BTreeSet<String>) {
    for piece in split_top(inner, b',') {
        let piece = strip_field_prefix(piece);
        let ty = if named {
            let b = piece.as_bytes();
            let colon = (0..b.len()).find(|&k| {
                b[k] == b':' && b.get(k + 1) != Some(&b':') && (k == 0 || b[k - 1] != b':')
            });
            match colon {
                Some(k) => &piece[k + 1..],
                None => continue,
            }
        } else {
            piece
        };
        refs.extend(type_refs(ty));
    }
}

/// Type references of enum variants: variant names and discriminant
/// expressions are skipped, payload field types are kept.
fn variant_type_refs(inner: &str, refs: &mut BTreeSet<String>) {
    for piece in split_top(inner, b',') {
        let piece = strip_field_prefix(piece);
        let name_end = piece
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(piece.len());
        let rest = piece[name_end..].trim_start();
        let named = rest.starts_with('{');
        if (named || rest.starts_with('('))
            && let Some(close) = matching(rest.as_bytes(), 0)
        {
            field_type_refs(&rest[1..close], named, refs);
        }
    }
}

/// Parse `struct`, `enum`, `union` and `type` declarations out of stripped,
/// test-blanked code. Only type positions count as references: field and
/// variant names, discriminants, generic parameters and bounds do not.
/// Names are keyed by identifier (last path segment); a name declared more
/// than once merges its references conservatively.
fn parse_type_decls(code: &[u8], file: &str, out: &mut BTreeMap<String, TypeDecl>) {
    let text = String::from_utf8_lossy(code);
    let bytes = text.as_bytes();
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i < bytes.len() {
        let at_word = i == 0 || !ident(bytes[i - 1]);
        let keyword = ["struct", "enum", "union", "type"].into_iter().find(|k| {
            at_word && text[i..].starts_with(k) && !ident(*bytes.get(i + k.len()).unwrap_or(&b' '))
        });
        let Some(keyword) = keyword else {
            i += 1;
            continue;
        };
        let mut j = i + keyword.len();
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        let start = j;
        while j < bytes.len() && ident(bytes[j]) {
            j += 1;
        }
        if start == j {
            i = j.max(i + 1);
            continue;
        }
        let name = text[start..j].to_string();
        let mut refs = BTreeSet::new();
        let mut k = j;
        while k < bytes.len() && bytes[k].is_ascii_whitespace() {
            k += 1;
        }
        // Generic parameters and bounds are not owned fields.
        if bytes.get(k) == Some(&b'<') {
            k = matching(bytes, k).map_or(bytes.len(), |c| c + 1);
        }
        let mut end = bytes.len();
        while k < bytes.len() {
            match bytes[k] {
                b'(' if keyword == "struct" => {
                    // Tuple struct fields, then an optional where clause and `;`.
                    let close = matching(bytes, k).unwrap_or(bytes.len() - 1);
                    field_type_refs(&text[k + 1..close], false, &mut refs);
                    k = close + 1;
                    while k < bytes.len() && bytes[k] != b';' {
                        k += 1;
                    }
                    end = k + 1;
                    break;
                }
                b'(' | b'<' | b'[' => {
                    // Inside a where clause: skip balanced groups.
                    k = matching(bytes, k).map_or(bytes.len(), |c| c + 1);
                }
                b'=' if keyword == "type" => {
                    let rhs_end = (k..bytes.len())
                        .find(|&e| bytes[e] == b';')
                        .unwrap_or(bytes.len());
                    refs.extend(type_refs(&text[k + 1..rhs_end]));
                    end = rhs_end + 1;
                    break;
                }
                b'{' if keyword != "type" => {
                    let close = matching(bytes, k).unwrap_or(bytes.len() - 1);
                    let inner = &text[k + 1..close];
                    if keyword == "enum" {
                        variant_type_refs(inner, &mut refs);
                    } else {
                        field_type_refs(inner, true, &mut refs);
                    }
                    end = close + 1;
                    break;
                }
                b';' => {
                    end = k + 1;
                    break;
                }
                _ => k += 1,
            }
        }
        refs.remove(&name);
        let entry = out.entry(name).or_default();
        if entry.file.is_empty() {
            entry.file = file.to_string();
        }
        entry.refs.extend(refs);
        i = end.min(bytes.len()).max(i + 1);
    }
}

/// `impl [path::]Trait[<..>] for Type` edges: a `dyn Trait` field reaches every
/// implementing type, since any of them may sit behind the trait object.
/// Recorded as a pseudo-declaration named `dyn Trait` referencing implementors.
fn parse_trait_impls(code: &[u8], out: &mut BTreeMap<String, TypeDecl>, file: &str) {
    let text = String::from_utf8_lossy(code);
    for line in text.lines() {
        let l = line.trim_start();
        let Some(rest) = l.strip_prefix("impl") else {
            continue;
        };
        let rest = rest.trim_start();
        let rest = if rest.starts_with('<') {
            match matching(rest.as_bytes(), 0) {
                Some(close) => &rest[close + 1..],
                None => continue,
            }
        } else {
            rest
        };
        let Some(for_at) = rest.find(" for ") else {
            continue;
        };
        let trait_part = &rest[..for_at];
        let trait_name = trait_part
            .split('<')
            .next()
            .unwrap_or("")
            .rsplit("::")
            .next()
            .unwrap_or("")
            .trim();
        let ty_part = rest[for_at + 5..].trim();
        let ty_name = ty_part
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == ':'))
            .next()
            .unwrap_or("")
            .rsplit("::")
            .next()
            .unwrap_or("");
        if trait_name.is_empty() || ty_name.is_empty() || trait_name.starts_with('!') {
            continue;
        }
        let entry = out.entry(format!("dyn {trait_name}")).or_default();
        if entry.file.is_empty() {
            entry.file = file.to_string();
        }
        entry.refs.insert(ty_name.to_string());
    }
}

/// Fact roots: every type named in a typed fact read or publication.
/// `facts.get` / `read_fact` / `fact_of` results are annotated `Arc<T>` or
/// turbofished; `publish_fact(i, value)` arguments are struct literals or
/// expressions whose type also appears as a read. Helper methods returning
/// `Result<Arc<T>, _>` are covered by their declared return types.
fn fact_roots(code: &str) -> BTreeSet<String> {
    let mut roots = BTreeSet::new();
    let mut take = |fragment: &str| {
        for word in identifiers(fragment) {
            if !matches!(
                word,
                "Arc" | "Result" | "Error" | "Option" | "Vec" | "dyn" | "Any" | "Send" | "Sync"
            ) {
                roots.insert(word.to_string());
            }
        }
    };
    for line in code.lines() {
        let l = line.trim();
        // `let name: Arc<T> = facts.get(...)` / `= cx.read_fact(...)`
        if let Some(colon) = l.find(": Arc<") {
            let rest = &l[colon + 2..];
            let next_line_read =
                rest.ends_with('=') || rest.contains("facts.get") || rest.contains("read_fact");
            if next_line_read {
                if let Some(eq) = rest.find('=') {
                    take(&rest[..eq]);
                } else {
                    take(rest);
                }
            }
        }
        // `fact_of::<T>(` / `read_fact::<T>(` / `.fact::<T>(`
        for marker in ["fact_of::<", "read_fact::<", ".fact::<"] {
            if let Some(at) = l.find(marker) {
                let rest = &l[at + marker.len()..];
                if let Some(end) = rest.find(">(")
                    && rest[..end].trim() != "T"
                {
                    take(&rest[..end]);
                }
            }
        }
        // Typed helpers: `async fn x(&self ...) -> Result<Arc<T>, Error>`
        if l.contains("async fn") && l.contains("-> Result<Arc<") {
            let rest = &l[l.find("-> Result<Arc<").unwrap() + 14..];
            if let Some(end) = rest.find(">,") {
                take(&rest[..end]);
            }
        }
        // `publish_fact(N, Name { ... })`
        if let Some(at) = l.find("publish_fact(") {
            let rest = &l[at + 13..];
            if let Some(comma) = rest.find(',') {
                let arg = rest[comma + 1..].trim();
                if let Some(brace) = arg.find(" {") {
                    take(&arg[..brace]);
                }
            }
        }
    }
    roots
}

/// Every `(root, carrier, primitive)` where `carrier` is a declaration
/// reachable from fact `root` that directly holds mutable `primitive`, with a
/// shortest witness path. Keying by carrier means a second mutable field on a
/// different type is new debt even when the root already reaches a `Mutex`.
type Reach = BTreeMap<(String, String, String), Vec<String>>;

fn reachable_mutable_state(decls: &BTreeMap<String, TypeDecl>, roots: &BTreeSet<String>) -> Reach {
    let mut found = Reach::new();
    for root in roots {
        let mut parent: HashMap<String, String> = HashMap::new();
        let mut seen: HashSet<String> = HashSet::from([root.clone()]);
        let mut queue = std::collections::VecDeque::from([root.clone()]);
        while let Some(name) = queue.pop_front() {
            let refs: Vec<String> = decls
                .get(&name)
                .map(|d| d.refs.iter().cloned().collect())
                .unwrap_or_default();
            for next in refs {
                if FACT_MUTABLE.contains(&next.as_str()) {
                    let key = (root.clone(), name.clone(), next.clone());
                    found.entry(key).or_insert_with(|| {
                        let mut path = vec![next.clone(), name.clone()];
                        let mut at = name.clone();
                        while let Some(p) = parent.get(&at) {
                            path.push(p.clone());
                            at = p.clone();
                        }
                        path.reverse();
                        path
                    });
                    continue;
                }
                if decls.contains_key(&next) && seen.insert(next.clone()) {
                    parent.insert(next.clone(), name.clone());
                    queue.push_back(next);
                }
            }
        }
    }
    found
}

fn scanned_type_decls() -> (BTreeMap<String, TypeDecl>, String) {
    let root = root();
    let mut files = Vec::new();
    for dir in SCANNED {
        rust_files(&root.join(dir), &mut files);
    }
    let mut scanned: BTreeMap<PathBuf, Scanned> = BTreeMap::new();
    let mut mod_rs = String::new();
    for path in &files {
        let text = fs::read_to_string(path).unwrap();
        if path.ends_with("src/compiler/mod.rs") {
            mod_rs = String::from_utf8_lossy(&scan(&text).code).to_string();
        }
        scanned.insert(path.clone(), scan(&text));
    }
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
    let mut decls = BTreeMap::new();
    for (path, scan) in &scanned {
        if test_only.contains(path) {
            continue;
        }
        let relative = path
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .to_string();
        parse_type_decls(&scan.code, &relative, &mut decls);
        parse_trait_impls(&scan.code, &mut decls, &relative);
    }
    (decls, mod_rs)
}

fn read_fact_debt() -> BTreeSet<(String, String, String)> {
    let text = fs::read_to_string(root().join(FACT_DEBT)).unwrap_or_default();
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let mut parts = l.split_whitespace();
            let mut next = || parts.next().unwrap().to_string();
            (next(), next(), next())
        })
        .collect()
}

fn write_fact_debt(found: &Reach) {
    let mut text = String::from(
        "# Mutable state reachable from published graph facts.\n\
         # Format: ROOT CARRIER PRIMITIVE  # witness path. ROOT is a type read or\n\
         # published as a graph fact. CARRIER is a type reachable from ROOT through\n\
         # fields, aliases or generic arguments that directly holds PRIMITIVE.\n\
         # Shrink-only forever: removing a line is progress, adding one requires\n\
         # explicit user approval. See GRAPH-INTEGRITY.md. Shrink after removal:\n\
         # GRAPH_INTEGRITY_SHRINK=1 justrust test --test graph_integrity\n",
    );
    for ((root, carrier, primitive), path) in found {
        text.push_str(&format!(
            "{root} {carrier} {primitive}  # {}\n",
            path.join(" -> ")
        ));
    }
    fs::write(root().join(FACT_DEBT), text).unwrap();
}

#[test]
fn graph_facts_reach_no_new_mutable_state() {
    let (decls, mod_rs) = scanned_type_decls();
    let roots = fact_roots(&mod_rs);
    assert!(
        roots.len() >= 8,
        "fact root discovery found only {roots:?}; the gate would be vacuous. \
         Update fact_roots() for the new fact API."
    );
    let found = reachable_mutable_state(&decls, &roots);
    let current: BTreeSet<(String, String, String)> = found.keys().cloned().collect();
    let recorded = read_fact_debt();
    let added: Vec<String> = found
        .iter()
        .filter(|(k, _)| !recorded.contains(*k))
        .map(|((r, c, p), path)| format!("  {r} reaches {p} in {c}: {}", path.join(" -> ")))
        .collect();
    assert!(
        added.is_empty(),
        "graph facts now reach new mutable state:\n{}\nFacts must be immutable values. \
         Move the mutation into its own graph node or make the field an immutable \
         value built when the fact is published. {RULE}",
        added.join("\n")
    );
    let removed: Vec<String> = recorded
        .difference(&current)
        .map(|(r, c, p)| format!("  {r} {c} {p}"))
        .collect();
    if !removed.is_empty() {
        // Progress means a carrier lost its mutable field. A recorded root
        // that is no longer discovered at all, while its carriers still hold
        // their primitives, is a blind spot in root discovery, not progress.
        let still_mutable: BTreeSet<&String> = decls
            .iter()
            .filter(|(_, d)| d.refs.iter().any(|r| FACT_MUTABLE.contains(&r.as_str())))
            .map(|(name, _)| name)
            .collect();
        let vanished: Vec<String> = recorded
            .difference(&current)
            .filter(|(r, c, _)| !roots.contains(r) && still_mutable.contains(c))
            .map(|(r, c, p)| format!("  {r} {c} {p}"))
            .collect();
        assert!(
            vanished.is_empty(),
            "recorded fact roots are no longer discovered, but their carriers still hold \
             mutable state. This is lost coverage, not progress. Keep the fact read/publish in \
             a shape fact_roots() recognizes, or extend fact_roots():\n{}",
            vanished.join("\n")
        );
        if std::env::var_os(SHRINK_ENV).is_some() {
            write_fact_debt(&found);
            return;
        }
        panic!(
            "reachable-fact mutable state shrank. Lock in the progress with \
             `{SHRINK_ENV}=1 justrust test --test graph_integrity` and commit {FACT_DEBT}:\n{}",
            removed.join("\n")
        );
    }
}

/// Builder-only owner types: refundable leases and minting capabilities.
/// No published fact may reach them by type, regardless of runtime variant.
const BUILDER_ONLY_OWNER_TYPES: &[&str] = &[
    "BuilderOwnedPath",
    "OwnerIssuer",
    "Issued",
    "TagMint",
    "SessionOwner",
];

/// Every builder-only owner type reachable by type from `root`, with a path.
fn reachable_builder_types(decls: &BTreeMap<String, TypeDecl>, root: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut parent: HashMap<String, String> = HashMap::new();
    let mut seen: HashSet<String> = HashSet::from([root.to_string()]);
    let mut queue = std::collections::VecDeque::from([root.to_string()]);
    while let Some(name) = queue.pop_front() {
        for next in decls.get(&name).map(|d| d.refs.clone()).unwrap_or_default() {
            if !seen.insert(next.clone()) {
                continue;
            }
            parent.insert(next.clone(), name.clone());
            if BUILDER_ONLY_OWNER_TYPES.contains(&next.as_str()) {
                let mut path = vec![next.clone()];
                let mut at = next.clone();
                while let Some(p) = parent.get(&at) {
                    path.push(p.clone());
                    at = p.clone();
                }
                path.reverse();
                found.push(path.join(" -> "));
            } else if decls.contains_key(&next) {
                queue.push_back(next);
            }
        }
    }
    found
}

/// Published name stores and parsed programs must not reach a builder-only
/// owner type at all (PARSE-SPLIT.md). A runtime "published" enum variant is
/// not enough: the builder variant would still be reachable by type.
#[test]
fn published_stores_reach_no_builder_owner_types() {
    let (decls, _) = scanned_type_decls();
    for root in ["NameStore", "ParsedProgram", "Program", "OwnerPath"] {
        assert!(decls.contains_key(root), "{root} declaration not scanned");
        let found = reachable_builder_types(&decls, root);
        assert!(
            found.is_empty(),
            "published {root} reaches builder-only owner types:\n  {}",
            found.join("\n  ")
        );
    }
}

/// Owner paths will be carried inside published facts (PARSE-SPLIT.md), so
/// they must reach no mutable state at all, before any fact names them.
#[test]
fn owner_path_reaches_no_mutable_state() {
    let (decls, _) = scanned_type_decls();
    assert!(
        decls.contains_key("OwnerPath"),
        "OwnerPath declaration not scanned"
    );
    let roots = BTreeSet::from(["OwnerPath".to_string()]);
    let found = reachable_mutable_state(&decls, &roots);
    assert!(
        found.is_empty(),
        "OwnerPath reaches mutable state:\n{}",
        found
            .iter()
            .map(|((_, c, p), path)| format!("  {p} in {c}: {}", path.join(" -> ")))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

// ---------------------------------------------------------------------------
// Standalone owner roots: allowed only in exact direct (non-graph) entries.
// ---------------------------------------------------------------------------

/// The only production functions that may mint a fresh standalone owner root
/// (PARSE-SPLIT.md). Graph routes receive tags from their producer's issuer.
/// Each entry is (file, enclosing function). May only shrink.
const STANDALONE_OWNER_ENTRIES: &[(&str, &str)] = &[
    ("src/compiler/owner_path.rs", "standalone_issued"),
    ("src/compiler/token_session.rs", "standalone"),
    ("src/compiler/frontend.rs", "new"),
    ("src/compiler/frontend.rs", "validate_inactive_syntax"),
    ("src/compiler/frontend.rs", "parse_prepared"),
];
const STANDALONE_MARKERS: &[&str] = &[
    "standalone_issued(",
    "TokenSession::standalone(",
    "OwnerRoot::standalone(",
];

/// (enclosing fn, byte offset) of every standalone-mint call in non-test code.
fn standalone_calls(original: &str) -> Vec<String> {
    let scanned = scan(original);
    let code = String::from_utf8_lossy(&scanned.code).to_string();
    let mut found = Vec::new();
    for marker in STANDALONE_MARKERS {
        let mut from = 0;
        while let Some(at) = code[from..].find(marker) {
            let at = from + at;
            from = at + marker.len();
            // The definition itself is not a call.
            let line_start = code[..at].rfind('\n').map_or(0, |i| i + 1);
            if code[line_start..at].trim_end().ends_with("fn") {
                continue;
            }
            let before = &code[..at];
            let name = before
                .rmatch_indices("fn ")
                .find_map(|(i, _)| {
                    let rest = &before[i + 3..];
                    let end = rest.find(|c: char| !(c.is_alphanumeric() || c == '_'))?;
                    (end > 0).then(|| rest[..end].to_string())
                })
                .unwrap_or_default();
            found.push(name);
        }
    }
    found
}

#[test]
fn standalone_owner_roots_only_in_allowed_direct_entries() {
    let mut files = Vec::new();
    for dir in SCANNED {
        rust_files(&root().join(dir), &mut files);
    }
    let mut violations = Vec::new();
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    for file in files {
        let rel = file
            .strip_prefix(root())
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let text = fs::read_to_string(&file).unwrap();
        for function in standalone_calls(&text) {
            let entry = (rel.clone(), function.clone());
            if !STANDALONE_OWNER_ENTRIES
                .iter()
                .any(|(f, n)| *f == rel && *n == function)
            {
                violations.push(format!("  {rel}: fn {function}"));
            }
            seen.insert(entry);
        }
    }
    assert!(
        violations.is_empty(),
        "standalone owner roots minted outside the allowed direct entries:\n{}\n\
         Graph code must receive an issued tag from its producer (TokenSession::with_owner).",
        violations.join("\n")
    );
    let stale: Vec<_> = STANDALONE_OWNER_ENTRIES
        .iter()
        .filter(|(f, n)| !seen.contains(&(f.to_string(), n.to_string())))
        .collect();
    assert!(
        stale.is_empty(),
        "allowed standalone entries no longer mint: {stale:?}. Remove them from \
         STANDALONE_OWNER_ENTRIES to lock in the progress."
    );
}

#[test]
fn standalone_owner_scan_rejects_injected_graph_fallback() {
    // A graph-route function that falls back to a standalone mint is caught;
    // comments, strings and test code are not calls.
    let source = r#"
fn graph_parse(cx: &Context) { let owner = standalone_issued(&budget); }
fn direct() { /* standalone_issued( */ let s = "TokenSession::standalone("; }
#[cfg(test)]
mod tests { fn t() { let _ = TokenSession::standalone(a, b); } }
"#;
    assert_eq!(standalone_calls(source), vec!["graph_parse".to_string()]);
}

#[test]
fn reachable_fact_gate_follows_aliases_wrappers_and_generics() {
    let source = r#"
pub struct Fact { inner: Arc<Middle>, plain: u64 }
struct Middle { list: Vec<Option<Leaf>>, alias: Shared }
type Shared = Arc<Cache>;
struct Cache { slots: HashMap<u32, Guarded> }
pub(super) struct Guarded(std::sync::Mutex<Vec<u8>>);
enum Choice { A(Leaf), B { once: std::sync::OnceLock<u8> } }
struct Leaf { n: u32 }
struct Generic<T> where T: Bound { value: T, count: AtomicUsize }
struct UsesGeneric { g: Generic<Leaf> }
struct Cycle { next: Option<Box<Cycle>>, leaf: Leaf }
struct Clean { a: Arc<[u8]>, b: Box<Leaf>, f: fn(u8) -> u8 }
struct Budget { lock: std::sync::Mutex<u8> }
trait Schema { fn get(&self) -> u8; }
struct Adapter { state: std::sync::Arc<std::sync::Mutex<u8>> }
impl tr::Schema for Adapter { fn get(&self) -> u8 { 0 } }
struct PureSchema { n: u8 }
impl<'a> Schema for PureSchema { fn get(&self) -> u8 { self.n } }
struct World { schema: Arc<dyn tr::Schema + Send + Sync> }
trait Unused {}
struct NoImpls { u: Box<dyn Unused> }
struct BudgetFailure { n: u32 }
enum TokenError { Budget(BudgetFailure), Exhausted = 3, Named { why: Leaf } }
struct HoldsError { error: TokenError, Guarded: u8 }
struct Bounded<G: Into<Guarded>> where G: AsRef<Guarded> { g: u8 }
struct Attributed { #[allow(dead_code)] pub(crate) inner: Leaf, #[doc = "Guarded"] other: u8 }
#[cfg(test)]
struct TestOnly { m: Mutex<u8> }
// struct Commented { m: Mutex<u8> }
const S: &str = "struct InString { m: Mutex<u8> }";
"#;
    let code = scan(source).code;
    let mut decls = BTreeMap::new();
    parse_type_decls(&code, "fixture.rs", &mut decls);
    assert!(
        !decls.contains_key("TestOnly"),
        "cfg(test) items are excluded"
    );
    assert!(!decls.contains_key("Commented") && !decls.contains_key("InString"));
    let roots: BTreeSet<String> = [
        "Fact",
        "Choice",
        "UsesGeneric",
        "Cycle",
        "Clean",
        "HoldsError",
        "Bounded",
        "Attributed",
        "World",
        "NoImpls",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    let found = reachable_mutable_state(&decls, &roots);
    let get = |r: &str, p: &str| {
        found
            .iter()
            .find(|((root, _, prim), _)| root == r && prim == p)
            .map(|(_, path)| path.clone())
    };
    assert_eq!(
        get("Fact", "Mutex").unwrap(),
        ["Fact", "Middle", "Shared", "Cache", "Guarded", "Mutex"],
        "through Arc, Vec/Option generics, an alias, a map and a tuple struct"
    );
    assert!(
        get("Choice", "OnceLock").is_some(),
        "enum struct-variant fields"
    );
    assert!(
        get("UsesGeneric", "AtomicUsize").is_some(),
        "generic struct with where clause"
    );
    assert!(
        found.keys().all(|(r, _, _)| r != "Cycle"),
        "cycles terminate, no false positive"
    );
    assert!(
        found.keys().all(|(r, _, _)| r != "Clean"),
        "fn pointers and slices are clean"
    );
    // A second mutable carrier under an already-indebted root is distinct debt.
    let carriers: BTreeSet<&str> = found
        .keys()
        .filter(|(r, _, _)| r == "Fact")
        .map(|(_, c, _)| c.as_str())
        .collect();
    assert_eq!(carriers, BTreeSet::from(["Guarded"]));

    let mod_rs = r#"
        let a: Arc<FactA> = facts.get(key).await?;
        let b: Arc<Arc<frontend::FactB>> =
            facts.get(root(X)).await?;
        let c = engine.fact_of::<module::FactC>(&key, 0);
        async fn types(&self) -> Result<Arc<frontend::FactD>, Error> {
        cx.publish_fact(0, FactE { inner })
        self.cx.read_fact::<T>(handle).await
    "#;
    let roots = fact_roots(mod_rs);
    for expected in ["FactA", "FactB", "FactC", "FactD", "FactE"] {
        assert!(
            roots.contains(expected),
            "{expected} missing from {roots:?}"
        );
    }
    assert!(
        !roots.contains("T"),
        "generic helper parameter is not a root"
    );
}

/// Flat program shaped for the optimizer: independent scalar leaves plus a
/// scalar driver that calls every leaf, so IPO_OPT really inlines and its
/// fan-in is exercised. `main` only prints the result.
fn optimizer_source(count: usize) -> String {
    let mut lines: Vec<String> = (0..count)
        .map(|i| format!("fn f{i}(x:i32)->i32 {{ if x < 0 {{ -x }} else {{ x + {i} }} }}"))
        .collect();
    let calls: Vec<String> = (0..count).map(|i| format!("f{i}(x)")).collect();
    lines.push(format!("fn drive(x:i32)->i32 {{ {} }}", calls.join(" + ")));
    let expected: i64 = (0..count as i64).map(|i| 2 + i).sum();
    lines.push(format!(
        "fn main(){{ if drive(2) == {expected} {{ println!(\"compiled correctly\"); }} }}"
    ));
    lines.join("\n")
}

fn optimizer_graph(source: String) -> Graph {
    let options = compiler::source::SourceOptions {
        opt: compiler::ssa::OptMode::Adaptive,
        ..compiler::source::SourceOptions::default()
    };
    let compilation = compiler::compile_observed_with_options(source, 1, false, options)
        .unwrap_or_else(|failure| panic!("optimizer build failed: {failure}"));
    assert!(compilation.optimizer.optimized > 0, "optimizer did not run");
    Graph::from_report(&compilation.report)
}

/// The same shape gates as `no_new_whole_program_barriers`, with the optimizer
/// on, so SSA/LOCAL_OPT/SUMMARY/IPO_OPT are inside the measured graph. Uses
/// the existing constants unchanged; adds no recorded debt.
#[test]
fn optimizer_nodes_add_no_whole_program_barriers() {
    for (label, small, large) in [
        (
            "flat",
            build_graph_with(16, compiler::ssa::OptMode::Adaptive),
            build_graph_with(32, compiler::ssa::OptMode::Adaptive),
        ),
        (
            "inlining driver",
            optimizer_graph(optimizer_source(16)),
            optimizer_graph(optimizer_source(32)),
        ),
    ] {
        let optimizer_kinds = ["ssa", "local_opt", "summary", "ipo_opt"];
        let present: BTreeSet<&str> = large
            .keys
            .values()
            .filter_map(|(kind, _)| optimizer_kinds.iter().find(|k| **k == kind).copied())
            .collect();
        for kind in ["ssa", "local_opt", "ipo_opt"] {
            assert!(
                present.contains(kind),
                "{label}: optimizer kind {kind} not measured"
            );
        }
        // No optimizer node may be whole-program.
        for (kind, params) in large.keys.values() {
            if optimizer_kinds.contains(&kind.as_str()) {
                assert_eq!(
                    params.len(),
                    1,
                    "{label}: {kind}{params:?} is not per-item. {RULE}"
                );
            }
        }
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
            "{label}: new whole-program nodes {added:?}. {RULE}"
        );
        // Leaf facts (indices below 16 are the same leaf in both programs)
        // keep bounded fan-in and closure. The driver legitimately reads one
        // summary per direct callee, so it is excluded from the leaf checks.
        let small_keys = small.by_key();
        let mut worst = (1.0f64, String::new());
        for (id, key) in &large.keys {
            if key.1.is_empty() || key.1[0] >= 16 {
                continue;
            }
            let Some(&small_id) = small_keys.get(key) else {
                continue;
            };
            let (a, b) = (small.fan_in(small_id), large.fan_in(*id));
            assert!(
                b <= a,
                "{label}: {}{:?} fan-in grew {a} -> {b} with program size. {RULE}",
                key.0,
                key.1
            );
            let (ca, cb) = (small.closure(small_id), large.closure(*id));
            if ca > 0 && cb as f64 / ca as f64 > worst.0 {
                worst = (cb as f64 / ca as f64, format!("{}{:?}", key.0, key.1));
            }
        }
        eprintln!(
            "{label}: optimizer leaf closure growth 16->32: {:.3} {}",
            worst.0, worst.1
        );
        assert!(
            worst.0 <= LEAF_CLOSURE_GROWTH_CEILING,
            "{label}: leaf closure grew {:.3}x with the optimizer on ({}). {RULE}",
            worst.0,
            worst.1
        );
    }
}

/// Build-mode work growth with the optimizer on stays within the existing
/// work ceiling (the base gate measures CHECK, which never runs it).
#[test]
fn optimizer_build_work_scales_linearly() {
    let work = |count: usize| {
        let options = compiler::source::SourceOptions {
            opt: compiler::ssa::OptMode::Adaptive,
            ..compiler::source::SourceOptions::default()
        };
        compiler::compile_observed_with_options(optimizer_source(count), 1, false, options)
            .unwrap_or_else(|failure| panic!("{count} functions failed: {failure}"))
            .resource_usage
            .work
    };
    let growth = work(32) as f64 / work(16) as f64;
    eprintln!("optimizer build work growth 16->32: {growth:.3}");
    assert!(
        growth <= OPTIMIZER_WORK_GROWTH_CEILING,
        "optimizer build work grew {growth:.3}x when the program doubled. {RULE}"
    );
}

/// Caller-side IPO_OPT fan-in is pinned exactly: doubling the driver's direct
/// leaf callees from 16 to 32 adds exactly one SUMMARY read per added callee
/// plus one LOCAL_OPT body read per added inlined callee, and nothing else.
/// A planted join reading any extra fact per callee fails this.
#[test]
fn optimizer_caller_fan_in_grows_exactly_per_direct_callee() {
    let measure = |count: usize| {
        let graph = optimizer_graph(optimizer_source(count));
        let drive = count as u64; // drive is the slot after the leaves
        let (id, _) = graph
            .keys
            .iter()
            .find(|(_, key)| key.0 == "ipo_opt" && key.1 == vec![drive])
            .expect("driver IPO_OPT measured");
        let deps = graph.deps.get(id).cloned().unwrap_or_default();
        let kind_of = |dep: &u64| graph.keys[dep].0.clone();
        let summaries = deps.iter().filter(|d| kind_of(d) == "summary").count();
        let bodies = deps
            .iter()
            .filter(|d| kind_of(d) == "local_opt" && graph.keys[*d].1 != vec![drive])
            .count();
        (deps.len(), summaries, bodies)
    };
    let (a, a_summaries, a_bodies) = measure(16);
    let (b, b_summaries, b_bodies) = measure(32);
    eprintln!(
        "driver IPO_OPT fan-in: 16 leaves {a} ({a_summaries} summaries, {a_bodies} bodies), \
         32 leaves {b} ({b_summaries} summaries, {b_bodies} bodies)"
    );
    assert_eq!(
        (a_summaries, b_summaries),
        (16, 32),
        "one SUMMARY per direct callee"
    );
    // Every leaf is inlinable today, so each callee also contributes its body.
    assert_eq!(
        (a_bodies, b_bodies),
        (16, 32),
        "one LOCAL_OPT per inlined callee"
    );
    assert_eq!(
        b - a,
        (b_summaries - a_summaries) + (b_bodies - a_bodies),
        "caller fan-in grew by more than added callee summaries and bodies. {RULE}"
    );
}

/// Raw engine keys (`Key::new`, `Key { .. }` literals) may only be built in
/// `src/compiler/keys.rs`. Everything else uses typed `Item`/`Params`/
/// `RootKey` constructors, so a whole-program key cannot be written.
#[test]
fn compiler_builds_no_raw_engine_keys() {
    let root = root();
    let mut files = Vec::new();
    rust_files(&root.join("src/compiler"), &mut files);
    let mut found = Vec::new();
    for path in &files {
        if path.ends_with("src/compiler/keys.rs") {
            continue;
        }
        let text = fs::read_to_string(path).unwrap();
        let code = strip(&text);
        let code = String::from_utf8_lossy(&code);
        for (line, l) in code.lines().enumerate() {
            let raw = raw_key_sites(l);
            if raw > 0 {
                let relative = path.strip_prefix(&root).unwrap().display();
                found.push(format!("  {relative}:{}", line + 1));
            }
        }
    }
    assert!(
        found.is_empty(),
        "raw engine Key construction outside src/compiler/keys.rs:\n{}\nUse keys::Item, \
         keys::Params, keys::params(kind, head, tail) or a sealed RootKey. {RULE}",
        found.join("\n")
    );
}

fn raw_key_sites(line: &str) -> usize {
    let b = line.as_bytes();
    let mut count = 0;
    for pattern in ["Key::new(", "Key {"] {
        let mut from = 0;
        while let Some(at) = line[from..].find(pattern) {
            let at = from + at;
            let prev = at.checked_sub(1).map(|i| b[i]);
            let ident = prev.is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_');
            let return_type = line[..at].trim_end().ends_with("->");
            if !ident && !return_type {
                count += 1;
            }
            from = at + pattern.len();
        }
    }
    count
}

#[test]
fn raw_key_scan_rejects_injected_construction() {
    assert_eq!(raw_key_sites("let k = Key::new(BODY, vec![]);"), 1);
    assert_eq!(raw_key_sites("crate::engine::Key::new(1, v)"), 1);
    assert_eq!(raw_key_sites("Key { kind: 1, version: 1, params }"), 1);
    assert_eq!(raw_key_sites("struct NormalizationKey { a: u8 }"), 0);
    assert_eq!(raw_key_sites("keys::params(BODY, 0, vec![])"), 0);
    assert_eq!(raw_key_sites("fn source_key(d: DeclId) -> Key {"), 0);
}

/// Every type implementing `Fact` must be a root of the reachable-mutable-
/// state scan, so a `Fact` impl cannot bypass the immutability gate.
///
/// Ignored pending approval: it exposes pre-existing debt that `fact_roots()`
/// never discovered because these facts are published outside mod.rs
/// (CanonicalSourcePublication, FileUnit, UnitFailure, ValidatedTraitWorld
/// reach budget/PlanCtx Mutexes). Recording that debt needs user approval.
#[test]
#[ignore = "exposes undiscovered pre-existing fact debt; needs approval to record"]
fn every_fact_impl_is_scanned_for_mutable_state() {
    let (decls, mod_rs) = scanned_type_decls();
    let discovered = fact_roots(&mod_rs);
    let impls = fact_impl_types();
    assert!(impls.len() >= 30, "found only {} Fact impls", impls.len());
    let roots: BTreeSet<String> = discovered.union(&impls).cloned().collect();
    let found = reachable_mutable_state(&decls, &roots);
    let recorded = read_fact_debt();
    let added: Vec<String> = found
        .iter()
        .filter(|(k, _)| !recorded.contains(*k))
        .map(|((r, c, p), path)| format!("  {r} reaches {p} in {c}: {}", path.join(" -> ")))
        .collect();
    assert!(
        added.is_empty(),
        "types implementing Fact reach unrecorded mutable state:\n{}\n{RULE}",
        added.join("\n")
    );
}

fn fact_impl_types() -> BTreeSet<String> {
    let root = root();
    let mut files = Vec::new();
    rust_files(&root.join("src/compiler"), &mut files);
    let mut types = BTreeSet::new();
    for path in &files {
        let text = fs::read_to_string(path).unwrap();
        let code = String::from_utf8_lossy(&strip(&text)).to_string();
        for l in code.lines() {
            let l = l.trim();
            for marker in ["impl crate::engine::Fact for ", "impl Fact for "] {
                if let Some(rest) = l.strip_prefix(marker) {
                    let name = rest.trim_end_matches("{}").trim();
                    if let Some(last) = identifiers(name).last() {
                        types.insert(last.to_string());
                    }
                }
            }
        }
    }
    types
}
