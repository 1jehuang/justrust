//! Executable UTF-8 boundary and Unicode White_Space String contracts.
use justrust_engine::compiler;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

#[test]
fn string_split_patterns_match_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "string-split-patterns",
        &[
            Case {
                name: "owned_generic_aggregate_preserves_mutable_capture_components",
                source: r#"struct Wrap<'a,F>{text:&'a str,f:F}fn wrap<'a,F:FnMut()->i32>(text:&'a str,f:F)->Wrap<'a,F>{Wrap{text,f}}fn main(){let mut a=0;let mut b=0;{let mut w=wrap("ok",||{a+=1;b+=2;a+b});println!("{} {} {}",(w.f)(),(w.f)(),w.text);}println!("{} {}",a,b);}"#,
                expect: Expect::Run("3 6 ok\n2 4\n"),
            },
            Case {
                name: "unused_owned_family_duplication",
                source: "struct Wrap<T>{value:T}fn bad<T>(value:T)->(Wrap<T>,Wrap<T>){(Wrap{value},Wrap{value})}fn main(){}",
                expect: Expect::Invalid("E0382"),
            },
            Case {
                name: "owned_generic_aggregate_keeps_captured_borrow_live",
                source: "struct Wrap<F>{f:F}fn wrap<F:FnMut()->i32>(f:F)->Wrap<F>{Wrap{f}}fn main(){let mut a=0;let mut b=0;let mut w=wrap(||{a+=1;b+=2;a+b});a=9;println!(\"{}\",(w.f)());}",
                expect: Expect::Invalid("E0506"),
            },
            Case {
                name: "unicode_char_substring_and_empty_patterns",
                source: r#"fn main(){
    println!("{:?}", "é,日,,🦀,".split(',').collect::<Vec<_>>());
    println!("{:?}", "é::日::🦀::".split("::").collect::<Vec<_>>());
    println!("{:?}", "é日🦀".split("").collect::<Vec<_>>());
    println!("{:?}", "".split("").collect::<Vec<_>>());
    println!("{:?}", "".split(',').collect::<Vec<_>>());
    println!("{:?}", "aaaaa".split("aa").collect::<Vec<_>>());
    println!("{:?}", "é日é".split("é").collect::<Vec<_>>());
}"#,
                expect: Expect::Run(
                    "[\"é\", \"日\", \"\", \"🦀\", \"\"]\n[\"é\", \"日\", \"🦀\", \"\"]\n[\"\", \"é\", \"日\", \"🦀\", \"\"]\n[\"\", \"\"]\n[\"\"]\n[\"\", \"\", \"a\"]\n[\"\", \"日\", \"\"]\n",
                ),
            },
            Case {
                name: "stateful_predicate_and_generic_forwarding",
                source: r#"fn pieces<P:FnMut(char)->bool>(text:&str,p:P)->usize{text.split(p).count()}
fn main(){
    let mut seen=String::new();
    let mut calls=0usize;
    {
        let mut it="é,!日".split(|c:char|{calls+=1;seen.push(c);c==','||c=='!'});
        println!("{:?} {:?} {:?}",it.next(),it.next(),it.next());
        println!("{:?} {:?}",it.next(),it.next());
    }
    println!("{} {}",calls,seen);
    println!("{}",pieces("a!b,c",|c:char|c=='!'||c==','));
    println!("{:?}","é日🦀".find(|c:char|c=='日'));
    println!("{}","é日🦀".contains(|c:char|c=='🦀'));
    let owned=String::from("é日🦀");
    println!("{} {:?}",owned.contains('日'),owned.find(|c:char|c=='🦀'));
}"#,
                expect: Expect::Run(
                    "Some(\"é\") Some(\"\") Some(\"日\")\nNone None\n4 é,!日\n3\nSome(2)\ntrue\ntrue Some(5)\n",
                ),
            },
            Case {
                name: "reverse_and_first_match_patterns",
                source: r#"fn main(){
    println!("{:?} {:?}","é::日::🦀".split_once("::"),"é日".split_once(""));
    println!("{:?} {:?}","é,日".split_once(','),"é日".split_once('!'));
    println!("{:?} {:?} {:?}","é日é".rfind('é'),"é::日::🦀".rfind("::"),"é日".rfind(""));
    let mut seen=String::new();
    let found="é!日,🦀".rfind(|c:char|{seen.push(c);c=='!'||c==','});
    println!("{:?} {}",found,seen);
    let mut calls=0usize;
    let pair="é!日,🦀".split_once(|c:char|{calls+=1;c=='!'||c==','});
    println!("{:?} {}",pair,calls);
}"#,
                expect: Expect::Run(
                    "Some((\"é\", \"日::🦀\")) Some((\"\", \"é日\"))\nSome((\"é\", \"日\")) None\nSome(5) Some(7) Some(5)\nSome(6) 🦀,\nSome((\"é\", \"日,🦀\")) 2\n",
                ),
            },
            Case {
                name: "unused_split_once_borrow_escape",
                source: "fn bad()->&'static str{let text=String::from(\"a,b\");text.split_once(',').unwrap().1}fn main(){}",
                expect: Expect::Invalid("E0515"),
            },
            Case {
                name: "scalar_trim_patterns_preserve_predicate_order",
                source: r#"fn main(){
    println!("{:?} {:?} {:?}","éé日é".trim_matches('é'),"éé".trim_matches('é'),"".trim_matches('é'));
    let mut seen=String::new();
    let trimmed="!!é日??".trim_matches(|c:char|{seen.push(c);c=='!'||c=='?'});
    println!("{} {}",trimmed,seen);
    let mut calls=0usize;
    let one="é".trim_matches(|_:char|{calls+=1;false});
    println!("{} {}",one,calls);
}"#,
                expect: Expect::Run("\"日\" \"\" \"\"\né日 !!é??日\né 1\n"),
            },
            Case {
                name: "unused_substring_is_not_double_ended_trim_pattern",
                source: "fn bad(s:&str){let _=s.trim_matches(\"xx\");}fn main(){}",
                expect: Expect::Invalid("E0277"),
            },
            Case {
                name: "unused_predicate_wrong_result",
                source: "fn bad(s:&str){let _=s.split(|_:char|1u8);}fn main(){}",
                expect: Expect::Invalid("E0271"),
            },
            Case {
                name: "unused_predicate_wrong_parameter",
                source: "fn bad(s:&str){let _=s.split(|_:u8|true);}fn main(){}",
                expect: Expect::Invalid("E0631"),
            },
            Case {
                name: "unused_split_borrow_escape",
                source: "fn bad()->&'static str{let text=String::from(\"a,b\");text.split(',').next().unwrap()}fn main(){}",
                expect: Expect::Invalid("E0515"),
            },
            Case {
                name: "prefix_suffix_char_str_and_predicate_patterns",
                source: r#"struct P<'a>{src:&'a str,pos:usize}impl<'a> P<'a>{fn skip_ws(&mut self){while self.src[self.pos..].starts_with(' '){self.pos+=1;}}}fn main(){
    let mut p=P{src:"  é日 x",pos:0};p.skip_ws();println!("{}",p.pos);
    let s=String::from("é日🦀");
    println!("{} {} {} {}",s.starts_with('é'),s.starts_with('日'),s.ends_with('🦀'),s.ends_with('日'));
    println!("{} {} {}","é日🦀".starts_with("é日"),"é日🦀".ends_with("日🦀"),"".ends_with('a'));
    println!("{} {}","abc".starts_with(|c:char|c.is_alphabetic()),"ab1".ends_with(char::is_numeric));
    let owned=String::from("é");let pre="é日";
    println!("{} {} {}","é日🦀".starts_with(&owned),"é日🦀".starts_with(&pre),s.ends_with(&format!("{}",'🦀')));
    let text="ab cd";let at=3usize;println!("{} {} {}",text[at..].len(),text[..2].to_uppercase(),text[1..at].trim().is_empty());
}"#,
                expect: Expect::Run(
                    "2\ntrue false true false\ntrue true false\ntrue true\ntrue true true\n2 AB false\n",
                ),
            },
            Case {
                name: "unused_str_range_is_not_string",
                source: "fn bad(s:&str)->usize{let t:&String=&s[1..];t.len()}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "unused_prefix_pattern_wrong_type",
                source: "fn bad(s:&str)->bool{s.starts_with(1u8)}fn main(){}",
                expect: Expect::Invalid("E0277"),
            },
        ],
    );
}

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
                name: "char_indices_byte_offsets_and_mixed_ends",
                source: r#"fn main(){let s="aé日🦀z";let mut it=s.char_indices();println!("{:?} {:?}",it.next_back(),it.next());println!("{:?} {:?} {:?}",it.next_back(),it.next(),it.next_back());println!("{:?} {:?}",it.next(),it.next_back());println!("{:?}",s.char_indices().filter(|(_,c)|!c.is_ascii()).collect::<Vec<_>>());println!("{} {}","".char_indices().count(),"é日🦀".char_indices().count());}"#,
                expect: Expect::Run(
                    "Some((10, 'z')) Some((0, 'a'))\nSome((6, '🦀')) Some((1, 'é')) Some((3, '日'))\nNone None\n[(1, 'é'), (3, '日'), (6, '🦀')]\n0 3\n",
                ),
            },
            Case {
                name: "unused_char_indices_wrong_item",
                source: "fn bad(s:&str)->char{let mut it=s.char_indices();it.next().unwrap()}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "unused_char_indices_owner_escape",
                source: r#"fn bad(){let mut it;{let s=String::from("é");it=s.as_str().char_indices();}let _=it.next();}fn main(){}"#,
                expect: Expect::Invalid("E0597"),
            },
            Case {
                name: "unused_chars_wrong_item",
                source: "fn unused(s: &str) -> u8 { let mut it = s.chars(); match it.next() { Some(c) => c, None => 0 } }\nfn main() {}",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

#[test]
fn str_bytes_matches_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "str-bytes",
        &[Case {
            name: "raw_utf8_and_interleaved_ends",
            source: r#"fn main(){let owned=String::from("aé🦀z");let mut it=owned.as_str().bytes();println!("{:?} {:?} {:?}",it.next(),it.next_back(),it.next());println!("{:?}",it.collect::<Vec<u8>>());let mut e="".bytes();println!("{:?} {:?}",e.next(),e.next_back());println!("{:?}","é日🦀".bytes().collect::<Vec<u8>>());let mut one="x".bytes();println!("{:?} {:?} {:?}",one.next_back(),one.next(),one.next_back());}"#,
            expect: Expect::Run(
                "Some(97) Some(122) Some(195)\n[169, 240, 159, 166, 128]\nNone None\n[195, 169, 230, 151, 165, 240, 159, 166, 128]\nSome(120) None None\n",
            ),
        }],
    );
}

#[test]
fn str_bytes_forwarding_inline() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "str-bytes-forwarding-inline",
        &[Case {
            name: "inline",
            source: r#"fn sum<I:IntoIterator<Item=u8>>(it:I)->u32{let mut n=0u32;for b in it{n+=b as u32;}n}fn main(){println!("{}",sum("é".bytes()));}"#,
            expect: Expect::Run("364\n"),
        }],
    );
}

#[test]
fn str_bytes_forwarding_local() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "str-bytes-forwarding-local",
        &[Case {
            name: "local",
            source: r#"fn sum<I:IntoIterator<Item=u8>>(it:I)->u32{let mut n=0u32;for b in it{n+=b as u32;}n}fn main(){let text="é";let it=text.bytes();println!("{}",sum(it));}"#,
            expect: Expect::Run("364\n"),
        }],
    );
}

#[test]
fn str_bytes_forwarding_owned() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "str-bytes-forwarding-owned",
        &[Case {
            name: "owned",
            source: r#"fn sum<I:IntoIterator<Item=u8>>(it:I)->u32{let mut n=0u32;for b in it{n+=b as u32;}n}fn main(){let owned=String::from("aé");println!("{}",sum(owned.as_str().bytes()));}"#,
            expect: Expect::Run("461\n"),
        }],
    );
}

#[test]
fn str_bytes_forwarding_ordinary() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "str-bytes-forwarding-ordinary",
        &[Case {
            name: "ordinary",
            source: r#"struct View<'a>{text:&'a str}trait Source{fn view(&self)->View<'_>;}impl Source for str{fn view(&self)->View<'_>{View{text:self}}}fn size<T>(_:T)->usize{1usize}fn main(){let text="left";println!("{} {}",size("right".view()),size(text.view()));}"#,
            expect: Expect::Run("1 1\n"),
        }],
    );
}

#[test]
fn str_bytes_forwarding_escape() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "str-bytes-forwarding-escape",
        &[Case {
            name: "escape",
            source: r#"fn sum<I:IntoIterator<Item=u8>>(it:I)->u32{let mut n=0u32;for b in it{n+=b as u32;}n}fn bad(){let it;{let s=String::from("é");it=s.as_str().bytes();}let _=sum(it);}fn main(){}"#,
            expect: Expect::Invalid("E0597"),
        }],
    );
}

#[test]
fn str_bytes_forwarding_mutation() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "str-bytes-forwarding-mutation",
        &[Case {
            name: "mutation",
            source: r#"fn sum<I:IntoIterator<Item=u8>>(it:I)->u32{let mut n=0u32;for b in it{n+=b as u32;}n}fn bad(){let mut s=String::from("é");let it=s.as_str().bytes();s.push('x');let _=sum(it);}fn main(){}"#,
            expect: Expect::Invalid("E0502"),
        }],
    );
}

#[test]
fn str_bytes_named_lifetime() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "str-bytes-named",
        &[Case {
            name: "named_borrowed_bytes",
            source: r#"use std::str::Bytes;fn bytes<'a>(s:&'a str)->Bytes<'a>{s.bytes()}fn main(){let owned=String::from("é");let mut it:Bytes<'_>=bytes(owned.as_str());println!("{:?} {:?} {:?}",it.next(),it.next_back(),it.next());}"#,
            expect: Expect::Run("Some(195) Some(169) None\n"),
        }],
    );
}

#[test]
fn str_bytes_rev_inline() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "str-bytes-rev_inline",
        &[Case {
            name: "rev_inline",
            source: r#"fn main(){println!("{:?}","aé".bytes().rev().collect::<Vec<u8>>());}"#,
            expect: Expect::Run("[169, 195, 97]\n"),
        }],
    );
}

#[test]
fn str_bytes_rev_local() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "str-bytes-rev_local",
        &[Case {
            name: "rev_local",
            source: r#"fn main(){let text="aé";let it=text.bytes();println!("{:?}",it.rev().collect::<Vec<u8>>());}"#,
            expect: Expect::Run("[169, 195, 97]\n"),
        }],
    );
}

#[test]
fn str_bytes_chain_inline() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "str-bytes-chain_inline",
        &[Case {
            name: "chain_inline",
            source: r#"fn main(){println!("{:?}","a".bytes().chain("é".bytes()).collect::<Vec<u8>>());}"#,
            expect: Expect::Run("[97, 195, 169]\n"),
        }],
    );
}

#[test]
fn str_bytes_chain_local() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "str-bytes-chain_local",
        &[Case {
            name: "chain_local",
            source: r#"fn main(){let a="a";let b="é";let left=a.bytes();let right=b.bytes();println!("{:?}",left.chain(right).collect::<Vec<u8>>());}"#,
            expect: Expect::Run("[97, 195, 169]\n"),
        }],
    );
}

#[test]
fn str_bytes_map_inline() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "str-bytes-map_inline",
        &[Case {
            name: "map_inline",
            source: r#"fn main(){println!("{:?}","é".bytes().map(|b|b as u32).collect::<Vec<u32>>());}"#,
            expect: Expect::Run("[195, 169]\n"),
        }],
    );
}

#[test]
fn str_bytes_map_local() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "str-bytes-map_local",
        &[Case {
            name: "map_local",
            source: r#"fn main(){let text="é";let it=text.bytes();println!("{:?}",it.map(|b|b as u32).collect::<Vec<u32>>());}"#,
            expect: Expect::Run("[195, 169]\n"),
        }],
    );
}

#[test]
fn str_bytes_for_inline() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "str-bytes-for_inline",
        &[Case {
            name: "for_inline",
            source: r#"fn main(){let mut n=0u32;for b in "ab".bytes(){n+=b as u32;}println!("{}",n);}"#,
            expect: Expect::Run("195\n"),
        }],
    );
}

#[test]
fn str_bytes_for_local() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "str-bytes-for_local",
        &[Case {
            name: "for_local",
            source: r#"fn main(){let text="ab";let it=text.bytes();let mut n=0u32;for b in it{n+=b as u32;}println!("{}",n);}"#,
            expect: Expect::Run("195\n"),
        }],
    );
}

#[test]
fn str_bytes_count_inline() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "str-bytes-count_inline",
        &[Case {
            name: "count_inline",
            source: r#"fn main(){println!("{}","".bytes().count());}"#,
            expect: Expect::Run("0\n"),
        }],
    );
}

#[test]
fn str_bytes_count_local() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "str-bytes-count_local",
        &[Case {
            name: "count_local",
            source: r#"fn main(){let text="";let it=text.bytes();println!("{}",it.count());}"#,
            expect: Expect::Run("0\n"),
        }],
    );
}

#[test]
fn str_bytes_unused_invalids() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "str-bytes-invalid",
        &[
            Case {
                name: "unused_wrong_byte_item",
                source: "fn bad(s:&str)->char{s.bytes().next().unwrap()}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "unused_bytes_owner_escape",
                source: "fn bad(){let mut it;{let s=String::from(\"é\");it=s.as_str().bytes();}let _=it.next();}fn main(){}",
                expect: Expect::Invalid("E0597"),
            },
            Case {
                name: "unused_bytes_owner_mutation",
                source: "fn bad(){let mut s=String::from(\"é\");let mut it=s.as_str().bytes();s.push('x');let _=it.next();}fn main(){}",
                expect: Expect::Invalid("E0502"),
            },
        ],
    );
}

#[test]
fn string_from_utf8_lossy_rejects_unused_escape() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "string-from-utf8-lossy-escape",
        &[Case {
            name: "unused_lossy_borrow_escape",
            source: "use std::borrow::Cow;fn bad()->Cow<'static,str>{let bytes=vec![72u8,105];String::from_utf8_lossy(&bytes)}fn main(){}",
            expect: Expect::Invalid("E0515"),
        }],
    );
}

#[test]
fn string_from_utf8_lossy_rejects_unused_wrong_result() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "string-from-utf8-lossy-result",
        &[Case {
            name: "unused_lossy_wrong_result",
            source: "fn bad(bytes:&[u8])->String{String::from_utf8_lossy(bytes)}fn main(){}",
            expect: Expect::Invalid("E0308"),
        }],
    );
}

#[test]
fn string_from_utf8_lossy_matches_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "string-from-utf8-lossy",
        &[Case {
            name: "unchanged_m2_string_formatting",
            source: include_str!("milestones/m2/07_string_formatting.rs"),
            expect: Expect::Run(include_str!("milestones/m2/07_string_formatting.stdout")),
        }],
    );
}

#[test]
fn string_from_utf8_lossy_sequences_match_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "string-from-utf8-lossy-sequences",
        &[Case {
            name: "utf8_valid_and_malformed_sequences",
            source: "use std::borrow::Cow;\nfn show(bytes:&[u8]) { let value=String::from_utf8_lossy(bytes); let borrowed=matches!(&value,Cow::Borrowed(_)); println!(\"{} {:?}\",borrowed,&*value); println!(\"{:?}\",value.into_owned()); }\nfn main(){show(&[]);show(&[72,105]);show(\"é日🦀\".as_bytes());show(&[0x66,0x80,0x6f]);show(&[0xe2,0x82]);show(&[0xe2,0x82,0x41]);show(&[0xe0,0x80,0x80]);show(&[0xed,0xa0,0x80]);show(&[0xf4,0x90,0x80,0x80]);show(&[0xf0,0x9f,0xa6,0x80,0xff,0x61]);}",
            expect: Expect::Run(
                "true \"\"\n\"\"\ntrue \"Hi\"\n\"Hi\"\ntrue \"é日🦀\"\n\"é日🦀\"\nfalse \"f�o\"\n\"f�o\"\nfalse \"�\"\n\"�\"\nfalse \"�A\"\n\"�A\"\nfalse \"���\"\n\"���\"\nfalse \"���\"\n\"���\"\nfalse \"����\"\n\"����\"\nfalse \"🦀�a\"\n\"🦀�a\"\n",
            ),
        }],
    );
}

#[test]
fn cow_borrowed_unsizes_array_reference_field() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "cow-borrowed-array-unsize",
        &[
            Case {
                name: "cow_borrowed_array_reference",
                source: "use std::borrow::Cow;\nfn main(){let bytes=[7u8,8,9];let data:Cow<'_,[u8]>=Cow::Borrowed(&bytes);println!(\"{:?} {}\",data,data.len());let owned:Vec<u8>=data.into_owned();println!(\"{:?}\",owned);}",
                expect: Expect::Run("[7, 8, 9] 3\n[7, 8, 9]\n"),
            },
            Case {
                name: "user_deref_to_slice_and_str_methods",
                source: "struct C<'a,B:?Sized>{b:&'a B}\nimpl<'a,B:?Sized> std::ops::Deref for C<'a,B>{type Target=B;fn deref(&self)->&B{self.b}}\nfn main(){let a=[1u8,2,3];let c:C<'_,[u8]>=C{b:&a[..]};println!(\"{} {:?} {:?}\",c.len(),c.first(),&*c);let s:&[u8]=&c;println!(\"{:?}\",s);let t:C<'_,str>=C{b:\"abc\"};println!(\"{} {}\",t.len(),t.to_uppercase());let o:std::borrow::Cow<'_,[u8]>=std::borrow::Cow::Owned(vec![4,5]);println!(\"{} {:?}\",o.len(),o.first());}",
                expect: Expect::Run("3 Some(1) [1, 2, 3]\n[1, 2, 3]\n3 ABC\n2 Some(4)\n"),
            },
            Case {
                name: "sized_parameter_rejects_slice_actual",
                source: "trait T{fn go(&self)->usize;}struct C<'a,B:?Sized>{b:&'a B}impl<'a,B> T for C<'a,B>{fn go(&self)->usize{1}}fn bad()->usize{let a=[1u8];let c=C{b:&a[..]};c.go()}fn main(){}",
                expect: Expect::Invalid("E0599"),
            },
            Case {
                name: "deref_target_lacks_method",
                source: "struct C<'a,B:?Sized>{b:&'a B}impl<'a,B:?Sized> std::ops::Deref for C<'a,B>{type Target=B;fn deref(&self)->&B{self.b}}fn bad(){let a=[1u8];let c=C{b:&a[..]};c.nope();}fn main(){}",
                expect: Expect::Invalid("E0599"),
            },
            Case {
                name: "cow_borrowed_scalar_reference_for_slice",
                source: "use std::borrow::Cow;fn bad()->usize{let data=Cow::<[u8]>::Borrowed(&5u8);data.len()}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}
