//! Higher-ranked trait bounds `for<'a, ..> Bound`.
//!
//! Admitted exactly when the binder is equivalent to lifetime elision in
//! parenthesized callable sugar: `for<'a> Fn(&'a T) -> R` is `Fn(&T) -> R`,
//! because every elided input lifetime of `Fn` sugar is its own late-bound
//! region. Each bound lifetime may therefore appear at most once, only as the
//! region of a reference in the inputs, and never in the output (whose
//! elision would bind it differently). Every other higher-ranked bound gets
//! an explicit unsupported diagnostic.
use super::*;

const UNSUPPORTED: &str = "higher-ranked bounds other than elision-equivalent `for<'a> Fn(&'a T) -> R` callable sugar are unsupported";

impl Parser<'_, '_> {
    /// A trait bound, optionally preceded by a `for<..>` binder.
    pub(in crate::compiler::frontend) fn bound_trait_path(
        &mut self,
    ) -> Result<TraitPath, ParseError> {
        if !self.is("for") {
            return self.trait_path();
        }
        self.cursor.bump();
        self.expect("<")?;
        let mut binders = Vec::new();
        while !self.is(">") {
            if binders.len() >= 16 {
                return Err(self.limit("higher-ranked binder limit 16"));
            }
            if !self.token().is_lifetime() || self.token().string {
                return Err(self.unsupported(UNSUPPORTED));
            }
            let RegionName::Named(ident) = self.lifetime()? else {
                return Err(self.error("expected a named higher-ranked lifetime"));
            };
            let spelling = self
                .names
                .spelling(ident)
                .map_err(native_adapter::name_error)?
                .to_owned();
            for outer in self
                .pending_regions
                .params
                .iter()
                .chain(&self.impl_regions.params)
                .chain(&self.callable_regions)
                .chain(&binders)
            {
                if self
                    .names
                    .spelling(*outer)
                    .map_err(native_adapter::name_error)?
                    == spelling
                {
                    return Err(self.error(
                        "E0496: lifetime name shadows a lifetime name that is already in scope",
                    ));
                }
            }
            self.reserve_parser_items(&mut binders, 1)?;
            binders.push(ident);
            if !self.eat(",") {
                break;
            }
        }
        self.expect(">")?;
        let mut path = self.trait_path()?;
        if binders.is_empty() {
            return Ok(path);
        }
        let TraitPath::Source(generic) = &mut path else {
            return Err(self.unsupported(UNSUPPORTED));
        };
        let Some(last) = generic.segments.last_mut() else {
            return Err(self.unsupported(UNSUPPORTED));
        };
        let sugar = matches!(
            self.names
                .spelling(last.name)
                .map_err(native_adapter::name_error)?,
            "Fn" | "FnMut" | "FnOnce"
        );
        if !sugar
            || !last.region_args.is_empty()
            || last.args.len() != 1
            || last.associated_types.len() != 1
        {
            return Err(self.unsupported(UNSUPPORTED));
        }
        let mut uses = vec![0usize; binders.len()];
        let mut eraser = Eraser {
            names: &*self.names,
            binders: &binders,
            uses: &mut uses,
            erase: true,
        };
        eraser
            .ty(&mut last.args[0], 0)
            .map_err(|e| self.unsupported(&e))?;
        if uses.iter().any(|count| *count > 1) {
            return Err(self.unsupported(UNSUPPORTED));
        }
        let mut output_uses = vec![0usize; binders.len()];
        let mut output = Eraser {
            names: &*self.names,
            binders: &binders,
            uses: &mut output_uses,
            erase: false,
        };
        output
            .ty(&mut last.associated_types[0].value, 0)
            .map_err(|e| self.unsupported(&e))?;
        if output_uses.iter().any(|count| *count > 0) {
            return Err(self.unsupported(UNSUPPORTED));
        }
        // The sugar segment itself must not mention the binders elsewhere.
        for segment in &generic.segments[..generic.segments.len() - 1] {
            if !segment.region_args.is_empty()
                || !segment.args.is_empty()
                || !segment.associated_types.is_empty()
            {
                return Err(self.unsupported(UNSUPPORTED));
            }
        }
        Ok(path)
    }
}

struct Eraser<'n, N: NameAccess> {
    names: &'n N,
    binders: &'n [AstIdent],
    uses: &'n mut [usize],
    erase: bool,
}

impl<N: NameAccess> Eraser<'_, N> {
    fn bound(&mut self, region: &RegionName) -> Result<Option<usize>, String> {
        let RegionName::Named(name) = region else {
            return Ok(None);
        };
        for (index, binder) in self.binders.iter().enumerate() {
            if name_scope::ident_eq(self.names, *binder, *name, LookupClass::LifetimeParameter)? {
                return Ok(Some(index));
            }
        }
        Ok(None)
    }

    fn ty(&mut self, ty: &mut Type, depth: usize) -> Result<(), String> {
        if depth > 64 {
            return Err(UNSUPPORTED.into());
        }
        match ty {
            Type::Ref {
                region, referent, ..
            } => {
                if let Some(r) = region
                    && let Some(index) = self.bound(r)?
                {
                    self.uses[index] += 1;
                    if self.erase {
                        *region = None;
                    }
                }
                self.ty(referent, depth + 1)
            }
            Type::Tuple(items) => {
                for item in items {
                    self.ty(item, depth + 1)?;
                }
                Ok(())
            }
            Type::Array(item, _) | Type::Slice(item) => self.ty(item, depth + 1),
            Type::RawPtr { pointee, .. } => self.ty(pointee, depth + 1),
            Type::Apply(path) => {
                for segment in &mut path.segments {
                    for region in &segment.region_args {
                        if self.bound(region)?.is_some() {
                            return Err(UNSUPPORTED.into());
                        }
                    }
                    for arg in &mut segment.args {
                        self.ty(arg, depth + 1)?;
                    }
                    if !segment.associated_types.is_empty() {
                        return Err(UNSUPPORTED.into());
                    }
                }
                Ok(())
            }
            Type::Int(_)
            | Type::Float(_)
            | Type::Char
            | Type::Str
            | Type::Bool
            | Type::Unit
            | Type::Never
            | Type::SelfType
            | Type::Param(_)
            | Type::Named(_) => Ok(()),
            // Binders could hide inside these; never guess their meaning.
            _ => Err(UNSUPPORTED.into()),
        }
    }
}
