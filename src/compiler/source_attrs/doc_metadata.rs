//! Grammar-checked inert rustdoc settings. These never grant source identity,
//! enable native attributes, or change the owner to which an attribute belongs.
use super::{AttributeError, AttributeErrorKind, Meta, MetaValue, Owner, error};

pub(super) fn validate(meta: &Meta, owner: Owner) -> Result<(), AttributeError> {
    match &meta.value {
        MetaValue::String(_) => Ok(()),
        MetaValue::List(settings) => {
            for setting in settings {
                match setting.name.as_str() {
                    "hidden" if matches!(setting.value, MetaValue::Word) => {}
                    "html_favicon_url"
                    | "html_logo_url"
                    | "html_root_url"
                    | "html_playground_url"
                    | "issue_tracker_base_url" => {
                        if owner != Owner::Crate {
                            return Err(error(
                                AttributeErrorKind::Unsupported,
                                setting.range.clone(),
                                "crate rustdoc settings require a crate owner",
                            ));
                        }
                        if !matches!(setting.value, MetaValue::String(_)) {
                            return Err(error(
                                AttributeErrorKind::Syntax,
                                setting.range.clone(),
                                "rustdoc URL metadata requires a string value",
                            ));
                        }
                    }
                    "test" => validate_test(setting)?,
                    "hidden" => {
                        return Err(error(
                            AttributeErrorKind::Syntax,
                            setting.range.clone(),
                            "doc(hidden) takes no arguments",
                        ));
                    }
                    _ => {
                        return Err(error(
                            AttributeErrorKind::Unsupported,
                            setting.range.clone(),
                            "unsupported rustdoc setting",
                        ));
                    }
                }
            }
            Ok(())
        }
        _ => Err(error(
            AttributeErrorKind::Unsupported,
            meta.range.clone(),
            "doc requires a supported string or parenthesized metadata list",
        )),
    }
}

fn validate_test(meta: &Meta) -> Result<(), AttributeError> {
    let MetaValue::List(settings) = &meta.value else {
        return Err(error(
            AttributeErrorKind::Syntax,
            meta.range.clone(),
            "doc(test) requires a parenthesized settings list",
        ));
    };
    for setting in settings {
        match (&*setting.name, &setting.value) {
            ("no_crate_inject", MetaValue::Word) => {}
            ("attr", MetaValue::List(_)) => {
                // The shared meta parser already checks bounded, fully consumed
                // attribute paths, delimiters, literals and nested token grammar.
                // These attributes belong to future doctests, not this program.
                // In particular deny/allow/cfg here must not affect native code.
            }
            ("attr" | "no_crate_inject", _) => {
                return Err(error(
                    AttributeErrorKind::Syntax,
                    setting.range.clone(),
                    "expected doc(test(attr(...))) or doc(test(no_crate_inject))",
                ));
            }
            _ => {
                return Err(error(
                    AttributeErrorKind::Unsupported,
                    setting.range.clone(),
                    "unsupported rustdoc test setting",
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "doc_metadata/tests.rs"]
mod tests;
