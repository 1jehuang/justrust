//! `I::Item` shorthand associated-type projections on a type parameter.
//!
//! Rust resolves `I::Item` against the traits bounding `I` that declare an
//! associated type `Item`. This parser desugars the shorthand to the qualified
//! form `<I as Trait>::Item`, which the resolver and checker already handle.
//! Selection here is syntactic: when exactly one written bound of `I` is in
//! scope, it is the trait. Zero or several written bounds are rejected
//! explicitly (rustc additionally consults supertraits and declared item
//! names, which this desugaring does not).
use super::*;

impl Parser<'_, '_> {
    /// Called with the parameter token consumed and `::` current.
    pub(super) fn param_shorthand_projection(
        &mut self,
        name: AstIdent,
    ) -> Result<Type, ParseError> {
        self.expect("::")?;
        let item = self.ident()?;
        let key = self
            .names
            .key(name, LookupClass::TypeParameter)
            .map_err(native_adapter::name_error)?;
        let mut found: Option<TraitPath> = None;
        let mut ambiguous = false;
        for bound in self.visible_bounds.iter().chain(self.pending_bounds.iter()) {
            let Type::Param(parameter) = &bound.subject else {
                continue;
            };
            let same = self
                .names
                .key(*parameter, LookupClass::TypeParameter)
                .map_err(native_adapter::name_error)?
                == key;
            if !same {
                continue;
            }
            for path in &bound.traits {
                if !matches!(path, TraitPath::Source(_)) {
                    continue;
                }
                if found.is_some() {
                    ambiguous = true;
                } else {
                    found = Some(path.clone());
                }
            }
        }
        if ambiguous {
            return Err(self.unsupported(
                "associated type shorthand on a parameter with several trait bounds; write <P as Trait>::Item",
            ));
        }
        let Some(trait_path) = found else {
            return Err(self.error(
                "E0220: associated type shorthand on a type parameter without a trait bound",
            ));
        };
        self.box_type_projection(SourceProjection::Qualified {
            self_ty: Type::Param(name),
            trait_path,
            item,
        })
    }

    /// Bounds visible to shorthand projections inside an item body.
    pub(super) fn set_visible_bounds(&mut self, bounds: &[TypeBound]) -> Result<(), ParseError> {
        let mut visible = std::mem::take(&mut self.visible_bounds);
        visible.clear();
        self.reserve_parser_items(&mut visible, bounds.len())?;
        visible.extend(bounds.iter().cloned());
        self.visible_bounds = visible;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shorthand_tracks_enclosing_and_method_bounds_without_leaking_items() {
        for source in [
            "trait Has{type Out;}fn id<I:Has>(x:I::Out)->I::Out{x}fn main(){}",
            "trait Has{type Out;}struct Holder<I:Has>{item:I::Out}fn main(){}",
            "trait Has{type Out;}struct Holder<I>{item:I}impl<I:Has> Holder<I>{fn both<U:Has>(x:I::Out,y:U::Out)->I::Out{x}}fn main(){}",
            "trait Has{type Out;}enum Holder<I:Has>{Value(I::Out)}fn main(){}",
        ] {
            parse_syntax_unprepared(source).unwrap_or_else(|error| panic!("{source}: {error}"));
        }
        let error = parse_syntax_unprepared(
            "trait Has{type Out;}fn first<I:Has>(x:I::Out){}fn second<I>(x:I::Out){}fn main(){}",
        )
        .unwrap_err();
        assert!(error.message.contains("without a trait bound"), "{error}");
    }

    #[test]
    fn ambiguous_shorthand_is_explicit_and_qualified_nested_syntax_is_retained() {
        let error = parse_syntax_unprepared(
            "trait A{type Out;}trait B{type Out;}fn f<I:A+B>(x:I::Out){}fn main(){}",
        )
        .unwrap_err();
        assert_eq!(
            error.kind,
            crate::compiler::source::SourceErrorKind::Unsupported
        );
        assert!(error.message.contains("several trait bounds"));
        parse_syntax_unprepared(
            "trait A{type Out;}trait B{type Out;}enum O<T>{Value(T)}fn f<I:A+B>(x:O<<I as A>::Out>){}fn main(){}",
        ).unwrap();
    }
}
