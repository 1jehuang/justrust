use super::super::{AttributeErrorKind, prepare};
use crate::compiler::source::CfgSet;

#[test]
fn original_time_core_doc_metadata_preserves_bytes_and_records() {
    let source = "//! Core items for time.\r\n#![doc(html_favicon_url = \"https://avatars0.githubusercontent.com/u/55999857\")]\n#![doc(html_logo_url = \"https://avatars0.githubusercontent.com/u/55999857\")]\n#![doc(test(attr(deny(warnings))))]\nfn main(){}\n";
    let out = prepare(source, &CfgSet::default()).unwrap();
    assert_eq!(source.len(), out.text.len());
    for (original, filtered) in source.bytes().zip(out.text.bytes()) {
        if matches!(original, b'\r' | b'\n') {
            assert_eq!(original, filtered);
        }
    }
    assert_eq!(out.attributes.len(), 4);
    assert!(
        out.attributes
            .iter()
            .all(|a| a.name == "doc" && a.effective)
    );
    assert!(out.text.contains("fn main(){}"));
}

#[test]
fn parsed_doctest_attributes_do_not_configure_native_code() {
    let source = "#![doc(html_logo_url=r#\"https://example.invalid/α\"#, html_favicon_url=\"\",)]\n#![doc(test(no_crate_inject,attr(deny(warnings),allow(dead_code),cfg(any())),))]\n#[doc(hidden, test(attr(allow(unused))))] fn main(){}";
    let out = prepare(source, &CfgSet::default()).unwrap();
    assert!(out.text.contains("fn main(){}"));
    assert!(out.inactive_items.is_empty());
    assert!(out.attributes.iter().all(|a| a.effective));
    let inactive = "#![cfg_attr(any(),doc(test(attr(deny(=))))) ] fn main(){}";
    assert!(prepare(inactive, &CfgSet::default()).is_ok());
}

#[test]
fn malformed_known_doc_settings_and_nested_grammar_are_not_ignored() {
    for source in [
        "#![doc(html_logo_url)] fn main(){}",
        "#![doc(html_favicon_url(\"x\"))] fn main(){}",
        "#![doc(html_logo_url=7)] fn main(){}",
        "#![doc(test)] fn main(){}",
        "#![doc(test(attr=\"deny(warnings)\"))] fn main(){}",
        "#![doc(test(no_crate_inject=\"yes\"))] fn main(){}",
        "#![doc(test(attr(deny(warnings) allow(dead_code))))] fn main(){}",
        "#![doc(test(attr(deny(=))))] fn main(){}",
        "#![doc(html_logo_url=\"x\" trailing)] fn main(){}",
    ] {
        assert!(
            prepare(source, &CfgSet::default()).is_err(),
            "accepted {source}"
        );
    }
    for source in [
        "#![doc(unknown_setting(foo))] fn main(){}",
        "#![doc(test(unknown_setting(foo)))] fn main(){}",
        "#[doc(html_logo_url=\"x\")] fn main(){}",
        "#[doc=42] fn main(){}",
    ] {
        assert_eq!(
            prepare(source, &CfgSet::default()).unwrap_err().kind,
            AttributeErrorKind::Unsupported,
            "{source}"
        );
    }
}

#[test]
fn original_associated_type_and_constant_docs_keep_owner_spans() {
    let text = "trait Per<T>{/// smallest output\n type Output;/** ratio */ const VALUE:Self::Output;}impl Per<u8> for u8{#[doc=\"output\"]type Output=u8;#[doc(hidden)]const VALUE:u8={7};}impl S{/// inherent\n const N:u8=1;}";
    let out = prepare(text, &CfgSet::default()).unwrap();
    assert_eq!(out.text.len(), text.len());
    assert_eq!(out.attributes.len(), 5);
    assert!(out.text.contains("type Output;"));
    assert!(out.text.contains("const VALUE:Self::Output;"));
    assert!(out.text.contains("const VALUE:u8={7};"));
    for record in out.attributes {
        assert!(record.effective);
        assert_eq!(record.name, "doc");
        for (original, masked) in text[record.range.clone()]
            .bytes()
            .zip(out.text[record.range].bytes())
        {
            assert_eq!(
                masked,
                if matches!(original, b'\r' | b'\n') {
                    original
                } else {
                    b' '
                }
            );
        }
    }
}

#[test]
fn associated_declaration_docs_do_not_activate_other_attributes_or_nested_docs() {
    for item in ["type A;", "const N:u8;", "const N:u8={1};"] {
        for attribute in [
            "inline",
            "cfg(any())",
            "cfg_attr(any(),doc=\"x\")",
            "derive(Copy)",
        ] {
            let text = format!("trait T{{#[doc=\"prefix\"]#[{attribute}]{item}}}");
            assert_eq!(
                prepare(&text, &CfgSet::default()).unwrap_err().kind,
                AttributeErrorKind::Unsupported,
                "{text}"
            );
        }
    }
    for item in ["type A;", "const N:u8;", "const N:u8={1};"] {
        for attribute in ["allow(dead_code)", "warn(unused)", "expect(dead_code)"] {
            let text = format!("trait T{{#[doc=\"prefix\"]#[{attribute}]{item}}}");
            let out = prepare(&text, &CfgSet::default()).unwrap();
            assert_eq!(out.text.len(), text.len());
            assert_eq!(out.attributes.len(), 2);
            assert!(out.attributes.iter().all(|record| record.effective));
        }
        for attribute in [
            "deny(dead_code)",
            "forbid(unused)",
            "allow(foo::bar)",
            "allow(dead_code reason)",
        ] {
            let text = format!("trait T{{#[doc=\"prefix\"]#[{attribute}]{item}}}");
            assert!(prepare(&text, &CfgSet::default()).is_err(), "{text}");
        }
    }
    for text in [
        "trait T{#[doc=\"dangling\"]}",
        "trait T{type #[doc=\"header\"]A;}",
        "impl S{#[doc=\"outer\"]const N:u8={#[doc=\"nested\"]1};}",
        "trait T{#![doc=\"inner\"]type A;}",
    ] {
        assert!(prepare(text, &CfgSet::default()).is_err(), "{text}");
    }
}

#[test]
fn nested_doc_metadata_uses_existing_parser_depth_limit() {
    let source = format!(
        "#![doc(test(attr({}x{})))] fn main(){{}}",
        "a(".repeat(70),
        ")".repeat(70)
    );
    assert_eq!(
        prepare(&source, &CfgSet::default()).unwrap_err().kind,
        AttributeErrorKind::Limit
    );
}
