//! Executable UTF-8 boundary and Unicode White_Space String contracts.
use justrust_engine::compiler;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

#[test]
fn uppercase_mappings_match_stock() {
    let text: String = (0..=0x10ffff)
        .filter_map(char::from_u32)
        .filter(|c| c.to_uppercase().ne(std::iter::once(*c)))
        .chain("Az09 日🦀".chars())
        .collect();
    let source = format!("fn main(){{let text={text:?};println!(\"{{}}\",text.to_uppercase());}}");
    let expected = format!("{}\n", text.to_uppercase());
    closure_oracle::check(
        "uppercase-mappings",
        &[closure_oracle::Case {
            name: "all_changed_unicode_scalars",
            source: Box::leak(source.into_boxed_str()),
            expect: closure_oracle::Expect::Run(Box::leak(expected.into_boxed_str())),
        }],
    );
}

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

#[test]
fn string_implementation_helpers_remain_private() {
    for call in [
        "std::string::str_starts_with(\"é\", \"é\")",
        "std::string::str_ends_with(\"é\", \"é\")",
        "std::string::char_is_whitespace(' ')",
    ] {
        let source = format!("fn main() {{ let _value = {call}; }}");
        let error = compiler::compile(source, 1, true)
            .err()
            .expect("private implementation helper was exposed")
            .to_string();
        assert!(
            error.contains("private") || error.contains("visible"),
            "{call}: {error}"
        );
    }
    let source = "fn main() { let text = String::from(\"\\u{3000}é\\u{a0}\"); println!(\"[{}] {} {}\", text.trim(), text.starts_with(\"\\u{3000}\"), text.ends_with(\"\\u{a0}\")); }";
    compiler::compile(source.into(), 1, true).unwrap();
    let built = compiler::compile(source.into(), 1, false).unwrap();
    let root = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("jr-string-private-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let binary = root.join("native");
    fs::write(&binary, built.executable.unwrap()).unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    let actual = Command::new(binary).output().unwrap();
    assert!(actual.status.success(), "{actual:?}");
    assert_eq!(actual.stdout, "[é] true true\n".as_bytes());
    fs::remove_dir_all(root).unwrap();
}

#[path = "support/closure_oracle.rs"]
mod closure_oracle;

#[test]
fn str_as_bytes_matches_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "str-as-bytes",
        &[
            Case {
                name: "empty_nul_and_multibyte_utf8_views",
                source: r#"fn view<'a>(text: &'a str) -> &'a [u8] { text.as_bytes() }
                fn main() {
                    for text in ["", "a\0b", "é日🦀"] {
                        let bytes = view(text);
                        println!("len {}", bytes.len());
                        let mut i = 0usize;
                        while i < bytes.len() { println!("{}", bytes[i]); i += 1usize; }
                    }
                }"#,
                expect: Expect::Run(
                    "len 0\nlen 3\n97\n0\n98\nlen 9\n195\n169\n230\n151\n165\n240\n159\n166\n128\n",
                ),
            },
            Case {
                name: "owned_string_view_aliases_storage_and_borrow_ends",
                source: r#"fn main() {
                    let mut owner = String::from("é\0");
                    let bytes = owner.as_bytes();
                    println!("{} {} {}", bytes.len(), bytes[0], bytes[2]);
                    println!("{}", (&bytes[0] as *const u8 as usize) == (owner.as_ptr() as usize));
                    owner.push_str("日");
                    let bytes = owner.as_str().as_bytes();
                    println!("{} {} {} {}", bytes.len(), bytes[3], bytes[4], bytes[5]);
                    let empty = String::new();
                    println!("{}", empty.as_bytes().len());
                }"#,
                expect: Expect::Run("3 195 0\ntrue\n6 230 151 165\n0\n"),
            },
            Case {
                name: "unused_local_owner_view_cannot_escape",
                source: "fn invalid<'a>() -> &'a [u8] { let owner = String::from(\"abc\"); owner.as_bytes() } fn main() {}",
                expect: Expect::Invalid("E0515"),
            },
            Case {
                name: "unused_borrowed_view_cannot_gain_static_lifetime",
                source: "fn invalid(text: &str) -> &'static [u8] { text.as_bytes() } fn main() {}",
                expect: Expect::Invalid("lifetime may not live long enough"),
            },
            Case {
                name: "shared_bytes_cannot_mutate_utf8",
                source: "fn main() { let bytes = \"é\".as_bytes(); bytes[0] = 0u8; }",
                expect: Expect::Invalid("E0594"),
            },
            Case {
                name: "owner_cannot_mutate_while_bytes_live",
                source: "fn main() { let mut owner = String::from(\"abc\"); let bytes = owner.as_bytes(); owner.push_str(\"d\"); println!(\"{}\", bytes[0]); }",
                expect: Expect::Invalid("E0502"),
            },
            Case {
                name: "owner_cannot_move_while_bytes_live",
                source: "fn main() { let owner = String::from(\"abc\"); let bytes = owner.as_bytes(); let moved = owner; println!(\"{}\", bytes[0]); }",
                expect: Expect::Invalid("E0505"),
            },
            Case {
                name: "unused_shared_view_cannot_be_strengthened_to_mutable",
                source: "fn invalid(text: &str) -> &mut [u8] { text.as_bytes() } fn main() {}",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

#[test]
fn str_as_bytes_retains_typed_raw_slice_graph_without_check_codegen() {
    use justrust_engine::compiler::cfg;
    let source = "fn view<'a>(text: &'a str) -> &'a [u8] { text.as_bytes() } fn main() { let owner = String::from(\"é\"); let bytes = view(owner.as_str()); println!(\"{} {}\", bytes.len(), bytes[0]); }";
    for workers in [1, 8] {
        let checked = compiler::compile(source.into(), workers, true).unwrap();
        assert!(checked.executable.is_none());
        assert!(checked.report.capture_complete);
        assert!(checked.analyses.iter().all(|analysis| analysis.passed()));
        assert!(
            checked
                .report
                .node_evaluations
                .iter()
                .all(|(key, _)| { !matches!(key.kind, 103 | 105 | 130 | 131 | 133) })
        );
        // CHECK exposes user SOURCE_ADMISSION CFGs, not prelude helper CFGs.
        // The user's view must retain its borrowed slice result and typed call.
        assert!(checked.control_flow.iter().any(|function| {
            function.result.slice_ref_element().is_some()
                && function.blocks.iter().any(|block| {
                    matches!(&block.terminator.value, cfg::Terminator::Call { signature, .. }
                        if signature.result.slice_ref_element().is_some())
                })
        }));
        // BUILD reports demanded LOWER instances, including the ordinary
        // from_raw_parts helper that owns the real RawSlice operation.
        let built = compiler::compile(source.into(), workers, false).unwrap();
        assert!(built.executable.is_some());
        assert!(built.report.capture_complete);
        assert!(built.analyses.iter().all(|analysis| analysis.passed()));
        assert!(built.control_flow.iter().any(|function| {
            function.result.slice_ref_element().is_some()
                && function.blocks.iter().any(|block| {
                    block.statements.iter().any(|statement| {
                        matches!(
                            statement.value,
                            cfg::Statement::Assign {
                                value: cfg::Rvalue::RawSlice { .. },
                                ..
                            }
                        )
                    })
                })
        }));
    }
}

#[test]
fn str_ordering_and_tuple_comparator_match_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "str-ordering",
        &[
            Case {
                name: "bytewise_empty_prefix_unicode_and_nul",
                source: r#"fn main() {
    println!("{:?} {:?} {:?}", "".cmp(""), "".cmp("a"), "a".cmp(""));
    println!("{:?} {:?} {:?}", "a".cmp("aa"), "aa".cmp("a"), "abc".cmp("abc"));
    println!("{:?} {:?} {:?}", "é".cmp("日"), "日".cmp("🦀"), "é".cmp("e"));
    println!("{:?} {:?}", "a\0b".cmp("a\0c"), "a\0".cmp("a"));
    println!("{:?}", "日".partial_cmp("é"));
    let mut words: Vec<(&str, usize)> = Vec::new();
    words.push(("z", 1)); words.push(("é", 2));
    words.push(("a", 1)); words.push(("a", 3));
    words.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    for (word, count) in words { println!("{}: {}", word, count); }
}"#,
                expect: Expect::Run(
                    "Equal Less Greater\nLess Greater Equal\nLess Less Greater\nLess Greater\nSome(Greater)\na: 3\né: 2\na: 1\nz: 1\n",
                ),
            },
            Case {
                name: "unused_string_order_wrong_operand",
                source: "fn unused() { let _ = \"a\".cmp(&1u32); } fn main() {}",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

/// `str::chars` decodes UTF-8 (1-4 byte scalars) through the prelude `Chars`
/// iterator: `next`, `count`, empty input. CHECK/BUILD at workers 1 and 8.
#[test]
fn str_chars_decodes_utf8_like_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "str-chars",
        &[
            Case {
                name: "chars_next_count",
                source: r#"fn main() {
    let s = "héllo €😀";
    let mut it = s.chars();
    while let Some(c) = it.next() {
        println!("{} {}", c, c as u32);
    }
    println!("{}", s.chars().count());
    let owned = String::from("日本");
    println!("{}", owned.as_str().chars().count());
    println!("{}", "".chars().count());
    let mut e = "".chars();
    println!("{}", e.next().is_none());
}"#,
                expect: Expect::Run(
                    "h 104\né 233\nl 108\nl 108\no 111\n  32\n€ 8364\n😀 128512\n8\n2\n0\ntrue\n",
                ),
            },
            Case {
                name: "unused_chars_wrong_item",
                source: "fn unused(s: &str) -> u8 { let mut it = s.chars(); match it.next() { Some(c) => c, None => 0 } }\nfn main() {}",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}
