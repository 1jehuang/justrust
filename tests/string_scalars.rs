//! Executable UTF-8 boundary and Unicode White_Space String contracts.
use justrust_engine::compiler;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

#[test]
fn string_unicode_trim_and_utf8_prefix_suffix_match_stock() {
    let mut cases = Vec::new();
    let whitespace = [
        '\t', '\n', '\u{b}', '\u{c}', '\r', ' ', '\u{85}', '\u{a0}', '\u{1680}', '\u{2000}',
        '\u{2001}', '\u{2002}', '\u{2003}', '\u{2004}', '\u{2005}', '\u{2006}', '\u{2007}',
        '\u{2008}', '\u{2009}', '\u{200a}', '\u{2028}', '\u{2029}', '\u{202f}', '\u{205f}',
        '\u{3000}',
    ];
    for ch in whitespace {
        for text in [
            format!("{ch}é日🦀{ch}"),
            format!("{ch}{ch}"),
            format!("é{ch}日"),
        ] {
            cases.push(format!(
                "{{ let text = String::from({text:?}); println!(\"[{{}}]\", text.trim()); }}\n"
            ));
        }
    }
    for text in [
        "",
        "é",
        "日🦀",
        "\u{180e}é\u{180e}",
        "\u{200b}é\u{200b}",
        "\u{feff}é\u{feff}",
        " \u{200b}é\u{200b} ",
    ] {
        cases.push(format!(
            "{{ let text = String::from({text:?}); println!(\"[{{}}]\", text.trim()); }}\n"
        ));
    }
    for text in ["", "é", "日", "🦀", "é日🦀", "aé", "éa"] {
        for pattern in ["", "a", "é", "日", "🦀", "é日", "日🦀", "é日🦀x"] {
            cases.push(format!("{{ let text = String::from({text:?}); println!(\"{{}} {{}}\", text.starts_with({pattern:?}), text.ends_with({pattern:?})); }}\n"));
        }
    }
    cases.push(
        r#"{
        let empty = String::new();
        println!("{} {}", empty.as_ptr() as usize == 0usize, empty.as_str().len());
        let mut text = String::with_capacity(0);
        println!("{} {}", text.as_ptr() as usize == 0usize, text.trim().len());
        text.push_str("é"); text.clear();
        println!("{} {}", text.as_ptr() as usize == 0usize, text.trim().len());
        text.push_str("日"); println!("{}", text.as_str());
    }
"#
        .to_string(),
    );
    let mut source = String::new();
    let mut entry = String::from("fn main() {\n");
    for (index, group) in cases.chunks(8).enumerate() {
        source.push_str(&format!("fn scalar_cases_{index}() {{\n"));
        for case in group {
            source.push_str(case);
        }
        source.push_str("}\n");
        entry.push_str(&format!("scalar_cases_{index}();\n"));
    }
    entry.push_str("}\n");
    source.push_str(&entry);
    let base = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let root = base.join(format!("jr-string-scalars-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let input = root.join("main.rs");
    fs::write(&input, &source).unwrap();
    let rustc = std::env::var_os("JUSTRUST_REAL_RUSTC").unwrap_or_else(|| "rustc".into());
    let stock = Command::new(&rustc)
        .args(["--edition", "2024", "-C", "panic=abort"])
        .arg(&input)
        .arg("-o")
        .arg(root.join("stock"))
        .output()
        .unwrap();
    assert!(
        stock.status.success(),
        "{}",
        String::from_utf8_lossy(&stock.stderr)
    );
    let expected = Command::new(root.join("stock")).output().unwrap();
    assert!(expected.status.success());
    for workers in [1, 8] {
        compiler::compile(source.clone(), workers, true).unwrap();
        let result = compiler::compile(source.clone(), workers, false).unwrap();
        let binary = root.join(format!("native-{workers}"));
        fs::write(&binary, result.executable.unwrap()).unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let actual = Command::new(&binary).output().unwrap();
        assert!(actual.status.success(), "{actual:?}");
        assert_eq!(actual.stdout, expected.stdout, "workers={workers}");
    }
    fs::remove_dir_all(root).unwrap();
}
