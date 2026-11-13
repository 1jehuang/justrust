//! `I::Item` shorthand associated-type projections on a type parameter.
//!
//! Rust resolves `I::Item` against the traits bounding `I` that declare an
//! associated type `Item`. This parser desugars the shorthand to the qualified
//! form `<I as Trait>::Item`, which the resolver and checker already handle.
//! Selection here is syntactic: when exactly one written bound of `I` is in
//! scope, it is the trait. Zero or several written bounds are rejected
//! explicitly. The source spelling remains attached so canonical resolution
//! may search supertraits for shorthand without accepting an invalid explicit
//! qualifier naming a trait that does not declare the member.
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
        let declared = self
            .visible_bounds
            .iter()
            .chain(self.pending_bounds.iter())
            .any(|bound| {
                matches!(&bound.subject, Type::Param(p) if self.names.key(*p, LookupClass::TypeParameter).ok() == Some(key))
            });
        // A bound written later in the same function's `where` clause is in
        // scope for the whole signature; use it only if nothing earlier is.
        let forward: &[TypeBound] = if declared {
            &[]
        } else {
            &self.forward_where_bounds
        };
        for bound in self
            .visible_bounds
            .iter()
            .chain(self.pending_bounds.iter())
            .chain(forward.iter())
        {
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
                    let mut application = path.clone();
                    let TraitPath::Source(source) = &mut application else {
                        unreachable!()
                    };
                    // The bound is retained unchanged above, including every
                    // equality obligation and its original source occurrence.
                    // A shorthand qualifier denotes only its trait application:
                    // copying bound constraints would fabricate the invalid
                    // explicit syntax `<I as Trait<Item = T>>::Other`.
                    // Keep the original anchor, names, ordered type and region
                    // arguments. Canonical resolution still checks the bound
                    // and the shorthand's own parameter/member occurrences.
                    for segment in &mut source.segments {
                        segment.associated_types.clear();
                    }
                    found = Some(application);
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
            syntax: ProjectionSyntax::ParameterShorthand,
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
    fn shorthand_qualifier_does_not_duplicate_original_bound_equalities() {
        let parsed = parse_syntax_unprepared(
            "trait Has<A>{type Out;}fn id<I:Has<bool,Out=u8>>(x:I::Out)->I::Out{x}fn main(){}",
        )
        .unwrap();
        let function = &parsed.program.functions[0];
        let TraitPath::Source(bound) = &function.bounds[0].traits[0] else {
            panic!("source bound required")
        };
        let Type::Projection(projection) = &function.params[0].1 else {
            panic!("projection required")
        };
        let SourceProjection::Qualified {
            trait_path: TraitPath::Source(qualifier),
            ..
        } = projection.as_ref()
        else {
            panic!("source qualifier required")
        };
        let original = bound.segments.last().unwrap();
        let projected = qualifier.segments.last().unwrap();
        let names = parsed.program.names.as_ref();
        assert_eq!(
            crate::compiler::semantic_name::occurrence_index(names, original.name).unwrap(),
            crate::compiler::semantic_name::occurrence_index(names, projected.name).unwrap()
        );
        assert!(original.region_args.is_empty());
        assert!(projected.region_args.is_empty());
        assert!(matches!(original.args.as_slice(), [Type::Bool]));
        assert!(matches!(projected.args.as_slice(), [Type::Bool]));
        assert_eq!(original.associated_types.len(), 1);
        assert!(matches!(
            original.associated_types[0].value,
            Type::Int(ir::IntTy::U8)
        ));
        assert!(projected.associated_types.is_empty());
    }

    #[test]
    fn projection_syntax_is_retained_by_exact_source_authority() {
        let program =
            parse("trait Has{type Item;}fn id<I:Has>(x:I::Item)->I::Item{x}fn main(){}").unwrap();
        let plan = PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap();
        let declaration = plan.function_decl(0).unwrap();
        let context = plan.registry.source_type_context(declaration).unwrap();
        let Type::Projection(original) = &program.functions[0].params[0].1 else {
            panic!("missing shorthand projection")
        };
        let SourceProjection::Qualified { syntax, .. } = original.as_ref() else {
            panic!("missing parameter qualifier")
        };
        assert_eq!(*syntax, ProjectionSyntax::ParameterShorthand);
        plan.registry
            .validate_source_projection(context, original, &plan.budget)
            .unwrap();
        let mut forged = original.clone();
        let SourceProjection::Qualified { syntax, .. } = forged.as_mut() else {
            unreachable!()
        };
        *syntax = ProjectionSyntax::Explicit;
        assert!(
            plan.registry
                .validate_source_projection(context, &forged, &plan.budget)
                .is_err()
        );

        let explicit = parse_syntax_unprepared(
            "trait Has{type Item;}fn id<I:Has>(x:<I as Has>::Item){}fn main(){}",
        )
        .unwrap();
        assert!(matches!(
            &explicit.program.functions[0].params[0].1,
            Type::Projection(projection)
                if matches!(projection.as_ref(), SourceProjection::Qualified {
                    syntax: ProjectionSyntax::Explicit, ..
                })
        ));
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
