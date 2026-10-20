//! Target selection for the native project's pinned Linux x86-64 platform.
//!
//! Dependency conditions are target facts, not crate features or build-profile
//! facts. Keep this distinct from source `cfg` expansion and reuse only its
//! bounded parser and Boolean evaluator.
use crate::compiler::source::source_cfg::{self, CfgSet, Meta, MetaValue};

pub const NATIVE_TARGET: &str = "x86_64-unknown-linux-gnu";

pub fn matches(selector: &str) -> Result<bool, String> {
    if selector.len() > 64 * 1024 {
        return Err("target dependency selector byte limit exceeded".into());
    }
    let selector = selector.trim();
    if !selector
        .strip_prefix("cfg")
        .is_some_and(|suffix| suffix.trim_start().starts_with('('))
    {
        if selector.is_empty()
            || !selector
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err("invalid target dependency selector".into());
        }
        return Ok(selector == NATIVE_TARGET);
    }
    let meta = source_cfg::parse_meta(selector, 0).map_err(|error| error.message)?;
    let MetaValue::List(arguments) = &meta.value else {
        return Err("target cfg requires exactly one predicate".into());
    };
    let [predicate] = arguments.as_slice() else {
        return Err("target cfg requires exactly one predicate".into());
    };
    if meta.name != "cfg" {
        return Err("invalid target cfg selector".into());
    }
    validate_target_predicate(predicate)?;
    source_cfg::evaluate(predicate, &CfgSet::native_x86_64_linux()).map_err(|error| error.message)
}

fn validate_target_predicate(meta: &Meta) -> Result<(), String> {
    if matches!(
        meta.name.as_str(),
        "feature" | "debug_assertions" | "test" | "proc_macro" | "panic"
    ) {
        return Err(format!(
            "unsupported: target dependencies cannot use crate or profile predicate `{}`",
            meta.name
        ));
    }
    if let MetaValue::List(children) = &meta.value {
        for child in children {
            validate_target_predicate(child)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_platform_and_boolean_predicates() {
        for selector in [
            NATIVE_TARGET,
            "cfg(unix)",
            "cfg (unix)",
            "cfg(not(miri))",
            "cfg(all(target_os = \"linux\", target_arch = \"x86_64\"))",
            "cfg(any(windows, target_pointer_width = \"64\"))",
            "cfg(all())",
        ] {
            assert!(matches(selector).unwrap(), "{selector}");
        }
        for selector in [
            "aarch64-unknown-linux-gnu",
            "cfg(windows)",
            "cfg(target_os = \"macos\")",
            "cfg(any())",
            "cfg(all(unix, miri))",
        ] {
            assert!(!matches(selector).unwrap(), "{selector}");
        }
    }

    #[test]
    fn malformed_and_crate_dependent_conditions_reject() {
        for selector in [
            "",
            "cfg()",
            "cfg(unix, windows)",
            "cfg(not())",
            "cfg(not(unix, windows))",
            "cfg(any(unix, not()))",
            "cfg(feature = \"enabled\")",
            "cfg(not(debug_assertions))",
            "cfg(test)",
            "cfg(proc_macro)",
            "cfg(panic = \"abort\")",
        ] {
            assert!(matches(selector).is_err(), "{selector}");
        }
    }
}
