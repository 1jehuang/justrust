//! Bounded filesystem snapshot and external-module expansion for the native compiler.
//!
//! This is an interim text expansion, not a Rust source map. Frontend positions
//! refer to the expanded text, not necessarily the named input file. Visibility
//! and module boundaries are retained and resolved by the ordinary frontend.
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

const MAX_DEPTH: usize = 64;
const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_FILES: usize = 1024;

#[derive(Debug)]
pub struct LoadedSource {
    pub text: String,
    /// Total UTF-8 input bytes read, before expansion.
    pub source_bytes: usize,
    /// File occurrences loaded, including the crate root.
    pub files: usize,
}

/// Load a crate root without invoking any compiler or other subprocess.
/// Paths follow the lexical source-file location, including when symlinked.
/// Canonical paths are used only to detect active inclusion cycles. This is
/// not a filesystem sandbox. `#[path]` is deliberately unsupported.
pub fn load(path: &Path) -> Result<LoadedSource, String> {
    let mut loader = Loader {
        active: Vec::new(),
        bytes: 0,
        files: 0,
    };
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    let text = loader.file(path, directory, 0)?;
    Ok(LoadedSource {
        text,
        source_bytes: loader.bytes,
        files: loader.files,
    })
}

struct Loader {
    active: Vec<PathBuf>,
    bytes: usize,
    files: usize,
}

impl Loader {
    fn file(&mut self, path: &Path, directory: &Path, depth: usize) -> Result<String, String> {
        if depth > MAX_DEPTH {
            return Err(format!(
                "module depth limit ({MAX_DEPTH}) exceeded at {}",
                path.display()
            ));
        }
        if self.files >= MAX_FILES {
            return Err(format!("module file limit ({MAX_FILES}) exceeded"));
        }
        let canonical = path
            .canonicalize()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if self.active.contains(&canonical) {
            return Err(format!(
                "module inclusion cycle: {} -> {}",
                self.active
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(" -> "),
                canonical.display()
            ));
        }
        let metadata = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if !metadata.is_file() {
            return Err(format!("{}: source must be a regular file", path.display()));
        }
        let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if !file
            .metadata()
            .map_err(|e| format!("{}: {e}", path.display()))?
            .is_file()
        {
            return Err(format!("{}: source must be a regular file", path.display()));
        }
        let mut bytes = Vec::new();
        file.take((MAX_BYTES - self.bytes + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        self.bytes += bytes.len();
        if self.bytes > MAX_BYTES {
            return Err(format!("source size limit ({MAX_BYTES} bytes) exceeded"));
        }
        self.files += 1;
        let text = String::from_utf8(bytes)
            .map_err(|e| format!("{}: source is not UTF-8: {e}", path.display()))?;
        self.active.push(canonical);
        let result = self
            .expand(&text, directory, depth)
            .map_err(|e| format!("{}: {e}", path.display()));
        self.active.pop();
        result
    }

    fn expand(&mut self, text: &str, directory: &Path, depth: usize) -> Result<String, String> {
        if depth > MAX_DEPTH {
            return Err(format!("module depth limit ({MAX_DEPTH}) exceeded"));
        }
        let tokens = tokens(text)?;
        let mut delimiters = Vec::new();
        for token in &tokens {
            match token.text(text) {
                "{" => delimiters.push("}"),
                "[" => delimiters.push("]"),
                "(" => delimiters.push(")"),
                closing @ ("}" | "]" | ")") => {
                    if delimiters.pop() != Some(closing) {
                        return Err("unbalanced delimiter in source file".into());
                    }
                }
                _ => {}
            }
        }
        if !delimiters.is_empty() {
            return Err("unclosed delimiter in source file".into());
        }
        let mut result = String::new();
        let mut copied = 0;
        let mut i = 0;
        while i < tokens.len() {
            let token = tokens[i].text(text);
            if token == "#" {
                let mut end = i + 1;
                if tokens.get(end).is_some_and(|t| t.text(text) == "!") {
                    end += 1;
                }
                if tokens.get(end).is_some_and(|t| t.text(text) == "[") {
                    let mut brackets = 1;
                    end += 1;
                    while end < tokens.len() && brackets > 0 {
                        match tokens[end].text(text) {
                            "path" => {
                                return Err(
                                    "#[path] attributes are unsupported by the source loader"
                                        .into(),
                                );
                            }
                            "[" => brackets += 1,
                            "]" => brackets -= 1,
                            _ => {}
                        }
                        end += 1;
                    }
                    i = end;
                    continue;
                }
            }
            if token == "mod" && i + 2 < tokens.len() {
                let name = tokens[i + 1].text(text);
                if !identifier(name) {
                    i += 1;
                    continue;
                }
                let delimiter = tokens[i + 2].text(text);
                if delimiter == ";" {
                    let flat = directory.join(format!("{name}.rs"));
                    let nested = directory.join(name).join("mod.rs");
                    let flat_exists = flat
                        .try_exists()
                        .map_err(|e| format!("{}: {e}", flat.display()))?;
                    let nested_exists = nested
                        .try_exists()
                        .map_err(|e| format!("{}: {e}", nested.display()))?;
                    let path = match (flat_exists, nested_exists) {
                        (true, false) => flat,
                        (false, true) => nested,
                        (true, true) => {
                            return Err(format!(
                                "ambiguous module {name}: both {} and {} exist",
                                flat.display(),
                                nested.display()
                            ));
                        }
                        (false, false) => {
                            return Err(format!(
                                "module {name} not found: expected {} or {}",
                                flat.display(),
                                nested.display()
                            ));
                        }
                    };
                    let body = self.file(&path, &directory.join(name), depth + 1)?;
                    result.push_str(&text[copied..tokens[i + 2].start]);
                    result.push_str("{\n");
                    result.push_str(&body);
                    result.push_str("\n}");
                    copied = tokens[i + 2].end;
                    i += 3;
                    continue;
                }
                if delimiter == "{" {
                    let end = close_brace(&tokens, text, i + 2)?;
                    let body = self.expand(
                        &text[tokens[i + 2].end..tokens[end].start],
                        &directory.join(name),
                        depth + 1,
                    )?;
                    result.push_str(&text[copied..tokens[i + 2].end]);
                    result.push_str(&body);
                    copied = tokens[end].start;
                    i = end + 1;
                    continue;
                }
            }
            if token == "{" {
                i = close_brace(&tokens, text, i)? + 1;
            } else {
                i += 1;
            }
        }
        result.push_str(&text[copied..]);
        if result.len() > MAX_BYTES {
            return Err(format!(
                "expanded source size limit ({MAX_BYTES} bytes) exceeded"
            ));
        }
        Ok(result)
    }
}

fn identifier(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}
struct Token {
    start: usize,
    end: usize,
}
impl Token {
    fn text<'a>(&self, s: &'a str) -> &'a str {
        &s[self.start..self.end]
    }
}
fn close_brace(tokens: &[Token], text: &str, start: usize) -> Result<usize, String> {
    let mut depth = 0;
    for (i, t) in tokens.iter().enumerate().skip(start) {
        match t.text(text) {
            "{" => depth += 1,
            "}" => {
                depth -= 1;
                if depth == 0 {
                    return Ok(i);
                }
            }
            _ => {}
        }
    }
    Err("unclosed brace in source".into())
}

/// A boundary scanner only. The frontend still owns syntax and semantics.
fn tokens(text: &str) -> Result<Vec<Token>, String> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let start = i;
        if b[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if b[i..].starts_with(b"//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b[i..].starts_with(b"/*") {
            i += 2;
            let mut depth = 1usize;
            while i < b.len() && depth > 0 {
                if b[i..].starts_with(b"/*") {
                    depth += 1;
                    i += 2;
                } else if b[i..].starts_with(b"*/") {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if depth != 0 {
                return Err("unterminated block comment".into());
            }
            continue;
        }
        // Raw strings must hide all their braces and apparent module tokens.
        let raw_start = if b[i] == b'r' {
            Some(i + 1)
        } else if b[i..].starts_with(b"br") {
            Some(i + 2)
        } else {
            None
        };
        if let Some(mut quote) = raw_start {
            let hashes_start = quote;
            while quote < b.len() && b[quote] == b'#' {
                quote += 1;
            }
            if quote < b.len() && b[quote] == b'"' {
                let hashes = quote - hashes_start;
                i = quote + 1;
                loop {
                    if i >= b.len() {
                        return Err("unterminated raw string".into());
                    }
                    if b[i] == b'"'
                        && b.get(i + 1..i + 1 + hashes)
                            .is_some_and(|s| s.iter().all(|c| *c == b'#'))
                    {
                        i += 1 + hashes;
                        break;
                    }
                    i += 1;
                }
                out.push(Token { start, end: i });
                continue;
            }
        }
        // A named lifetime has no closing quote. Keep the apostrophe in the
        // token so names such as 'mod cannot masquerade as module keywords.
        // A closing quote immediately after the identifier instead denotes a
        // character literal, whose delimiters must remain hidden below.
        if b[i] == b'\''
            && b.get(i + 1)
                .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
        {
            let mut end = i + 2;
            while end < b.len() && (b[end].is_ascii_alphanumeric() || b[end] == b'_') {
                end += 1;
            }
            if b.get(end) != Some(&b'\'') {
                i = end;
                out.push(Token { start, end: i });
                continue;
            }
        }
        if b[i] == b'"' || b[i] == b'\'' {
            let quote = b[i];
            i += 1;
            let mut closed = false;
            while i < b.len() {
                if b[i] == b'\\' {
                    i = (i + 2).min(b.len());
                } else if b[i] == quote {
                    i += 1;
                    closed = true;
                    break;
                } else {
                    i += 1;
                }
            }
            if !closed {
                return Err("unterminated string or character literal".into());
            }
        } else if b[i].is_ascii_alphabetic() || b[i] == b'_' {
            i += 1;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
        } else {
            i += text[i..].chars().next().unwrap().len_utf8();
        }
        out.push(Token { start, end: i });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Tree(PathBuf);
    impl Tree {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "jr-source-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&root).unwrap();
            Self(root)
        }
        fn put(&self, name: &str, text: &str) {
            let path = self.0.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        fn load(&self) -> Result<LoadedSource, String> {
            load(&self.0.join("main.rs"))
        }
    }
    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn mixed_layout_preserves_visibility_and_counts() {
        let tree = Tree::new();
        let root = "pub mod a; mod b; fn main(){}";
        let a = "pub mod inline { mod child; }";
        let child = "fn private(){}";
        let b = "pub fn public(){}";
        tree.put("main.rs", root);
        tree.put("a.rs", a);
        tree.put("a/inline/child/mod.rs", child);
        tree.put("b/mod.rs", b);
        let loaded = tree.load().unwrap();
        assert_eq!(loaded.files, 4);
        assert_eq!(
            loaded.source_bytes,
            root.len() + a.len() + child.len() + b.len()
        );
        assert!(loaded.text.contains("pub mod a{"));
        assert!(loaded.text.contains("mod child{"));
        assert!(loaded.text.contains("fn private(){}"));
        assert!(loaded.text.contains("pub fn public(){}"));
    }

    #[test]
    fn comments_and_literals_do_not_load_phantom_files() {
        let tree = Tree::new();
        let root = r###"/* mod phantom; /* } */ */ // mod ghost;
fn main(){println!("mod absent; }"); let x = r##"mod raw; {"##; let c = '}';}
"###;
        tree.put("main.rs", root);
        let loaded = tree.load().unwrap();
        assert_eq!(loaded.text, root);
        assert_eq!(loaded.files, 1);
    }

    #[test]
    fn named_lifetimes_survive_external_module_expansion() {
        let tree = Tree::new();
        let root = "mod child; fn id<'a>(x:&'a i32)->&'a i32{x} fn main(){}";
        let child = "pub fn shorten<'long:'short,'short>(x:&'long i32)->&'short i32{x}";
        tree.put("main.rs", root);
        tree.put("child.rs", child);
        let loaded = tree.load().unwrap();
        assert_eq!(loaded.files, 2);
        assert_eq!(loaded.source_bytes, root.len() + child.len());
        assert!(loaded.text.contains("fn id<'a>(x:&'a i32)->&'a i32{x}"));
        assert!(loaded.text.contains(child));
        super::super::frontend::parse(&loaded.text).unwrap();
    }

    #[test]
    fn lifetime_tokens_do_not_hide_delimiters_or_expose_literal_contents() {
        let text = "fn f<'mod>(x:&'mod i32){let a='}';let b='a';let c='\\'';}";
        let actual: Vec<_> = tokens(text).unwrap().iter().map(|t| t.text(text)).collect();
        assert_eq!(actual.iter().filter(|t| **t == "'mod").count(), 2);
        assert_eq!(actual.iter().filter(|t| **t == "}").count(), 1);
        assert!(actual.contains(&"'}'"));
        assert!(actual.contains(&"'a'"));
        assert!(actual.contains(&"'\\''"));
    }

    #[test]
    fn missing_ambiguous_and_path_attribute_are_explicit() {
        let tree = Tree::new();
        tree.put("main.rs", "mod child;");
        assert!(tree.load().unwrap_err().contains("not found"));
        tree.put("child.rs", "");
        tree.put("child/mod.rs", "");
        assert!(tree.load().unwrap_err().contains("ambiguous"));
        tree.put("main.rs", "#[path = \"other.rs\"] mod child;");
        assert!(tree.load().unwrap_err().contains("#[path]"));
        tree.put(
            "main.rs",
            "#[cfg_attr(any(), path = \"other.rs\")] mod child;",
        );
        assert!(tree.load().unwrap_err().contains("#[path]"));
    }

    #[test]
    fn nested_flat_file_uses_stem_directory() {
        let tree = Tree::new();
        tree.put("main.rs", "mod a;");
        tree.put("a.rs", "mod b;");
        tree.put("a/b.rs", "mod c;");
        tree.put("a/b/c.rs", "fn leaf(){}");
        assert_eq!(tree.load().unwrap().files, 4);
    }

    #[test]
    fn depth_and_size_are_bounded() {
        let tree = Tree::new();
        tree.put(
            "main.rs",
            &format!(
                "{}{}",
                "mod a {".repeat(MAX_DEPTH + 1),
                "}".repeat(MAX_DEPTH + 1)
            ),
        );
        assert!(tree.load().unwrap_err().contains("depth limit"));
        tree.put("main.rs", &" ".repeat(MAX_BYTES + 1));
        assert!(tree.load().unwrap_err().contains("size limit"));
        let mut loader = Loader {
            active: Vec::new(),
            bytes: 0,
            files: MAX_FILES,
        };
        assert!(
            loader
                .file(&tree.0.join("main.rs"), &tree.0, 0)
                .unwrap_err()
                .contains("file limit")
        );
    }

    #[cfg(unix)]
    #[test]
    fn canonical_active_stack_rejects_symlink_cycle() {
        let tree = Tree::new();
        tree.put("main.rs", "mod a;");
        std::os::unix::fs::symlink(tree.0.join("main.rs"), tree.0.join("a.rs")).unwrap();
        assert!(tree.load().unwrap_err().contains("cycle"));
    }

    #[test]
    fn nonregular_sources_are_rejected() {
        let tree = Tree::new();
        assert!(load(&tree.0).unwrap_err().contains("regular file"));
        tree.put("main.rs", "mod child;");
        std::fs::create_dir(tree.0.join("child.rs")).unwrap();
        assert!(tree.load().unwrap_err().contains("regular file"));
    }

    #[test]
    fn malformed_and_non_utf8_sources_fail() {
        let tree = Tree::new();
        for text in [
            "/*",
            "fn main(){",
            "fn main(){\"unterminated}",
            "fn main(){r###\"unfinished}",
        ] {
            tree.put("main.rs", text);
            assert!(tree.load().is_err(), "{text}");
        }
        std::fs::write(tree.0.join("main.rs"), [0xff]).unwrap();
        assert!(tree.load().unwrap_err().contains("UTF-8"));
    }
}
