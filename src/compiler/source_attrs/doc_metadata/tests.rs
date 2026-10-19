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
