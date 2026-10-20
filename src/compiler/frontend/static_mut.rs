//! Fail-closed source admission for mutable statics.
//!
//! Mutable statics need declaration-authenticated storage identity, constant
//! initialization and process-wide storage semantics. A lazily initialized
//! fixed-address accessor is not that contract, even when the item is unused.
use super::*;

impl Parser<'_, '_> {
    pub(super) fn static_mut_item(&mut self, _module: ModuleId) -> Result<Function, ParseError> {
        Err(self.unsupported("mutable static storage and initialization are unsupported"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutable_statics_reject_before_synthetic_accessor_publication() {
        for source in [
            "static mut X:u8=1;fn main(){}",
            "pub static mut X:u8=1;fn main(){}",
            "mod inner{pub static mut X:u8=1;}fn main(){}",
            "static mut X:u8=1;fn main(){unsafe{X=2;}}",
            "static mut X:u8=1;fn main(){let _x=unsafe{X};}",
        ] {
            let error = parse_syntax_unprepared(source).unwrap_err();
            assert_eq!(
                error.kind,
                super::super::super::source::SourceErrorKind::Unsupported
            );
            assert!(error.message.contains("mutable static storage"), "{error}");
        }
    }
}
