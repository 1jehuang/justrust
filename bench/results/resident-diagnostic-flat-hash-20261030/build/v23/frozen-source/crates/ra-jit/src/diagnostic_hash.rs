//! Diagnostic text hashing without materializing text for flat syntax trees.

use std::hash::BuildHasher;

/// Preserve the existing diagnostic hash, borrowing flat-tree text when available.
pub(crate) fn of(node: &syntax::SyntaxNode) -> u64 {
    match flat_text(node) {
        Some(text) => rustc_hash::FxBuildHasher.hash_one(text),
        None => rustc_hash::FxBuildHasher.hash_one(node.text().to_string()),
    }
}

fn flat_text(node: &syntax::SyntaxNode) -> Option<&str> {
    let (tree, at) = node.flat()?;
    let range = tree.range(at);
    Some(&tree.text()[usize::from(range.start())..usize::from(range.end())])
}

#[cfg(test)]
mod tests {
    use super::{flat_text, of};
    use std::hash::BuildHasher;
    use syntax::{Edition, SourceFile, SyntaxNode};

    const SOURCES: &[(&str, &str)] = &[
        ("empty", ""),
        ("trivia", " \t\r\n// only a comment\n/* trailing */\n"),
        (
            "utf8",
            "// café 🦀\nfn café() { let 名 = \"λ🦀\"; /* 日本語 */ }\n",
        ),
        (
            "nested",
            "/// docs\nmod outer { /* a /* nested */ comment */\n    fn f() { let x = { (1 + 2) }; if x > 0 { f(); } }\n}\n",
        ),
        ("empty blocks", "mod m {} fn f() {} struct S {}\n"),
        (
            "recovery",
            "fn broken( { let x = ; /* kept */\n mod inner {",
        ),
    ];

    fn check_subtrees(root: &SyntaxNode, source: &str, flat: bool) {
        assert_eq!(root.text().to_string(), source);
        let mut count = 0;
        // Rowan's descendants includes the root, including an empty source file.
        for node in root.descendants() {
            count += 1;
            let range = node.text_range();
            let oracle = &source[usize::from(range.start())..usize::from(range.end())];
            let old_text = node.text().to_string();
            assert_eq!(old_text, oracle, "{:?} at {range:?}", node.kind());
            assert_eq!(of(&node), rustc_hash::FxBuildHasher.hash_one(old_text));
            assert_eq!(of(&node), rustc_hash::FxBuildHasher.hash_one(oracle));
            if flat {
                let borrowed = flat_text(&node).expect("must exercise the flat branch");
                assert_eq!(borrowed, oracle);
                let (tree, at) = node.flat().expect("must remain flat");
                let range = tree.range(at);
                let exact = &tree.text()[usize::from(range.start())..usize::from(range.end())];
                assert_eq!(borrowed.as_ptr(), exact.as_ptr());
                assert_eq!(borrowed.len(), exact.len());
            } else {
                assert!(node.flat().is_none(), "must exercise the green fallback");
                assert!(flat_text(&node).is_none());
            }
        }
        assert!(count > 0, "must check the source-file root");
    }

    #[test]
    fn flat_hash_matches_old_hash_and_source_substrings() {
        for &(name, source) in SOURCES {
            let parsed = SourceFile::parse(source, Edition::CURRENT);
            if name == "recovery" {
                assert!(!parsed.errors().is_empty());
            }
            // Explicit conversion guarantees branch coverage without changing
            // the process-global GREEN_TREES setting used by parallel tests.
            let root = parsed.syntax_node().to_flat();
            assert!(flat_text(&root).is_some(), "{name}");
            check_subtrees(&root, source, true);
        }
    }

    #[test]
    fn green_hash_matches_old_hash_and_source_substrings() {
        for &(_, source) in SOURCES {
            let parsed_root = SourceFile::parse(source, Edition::CURRENT).syntax_node();
            let root = SyntaxNode::new_root(parsed_root.green().to_owned());
            assert!(flat_text(&root).is_none());
            check_subtrees(&root, source, false);
        }
    }
}
